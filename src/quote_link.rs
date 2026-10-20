//! Replace a message quoted in a reply by a wikilink to the note of that message.
//!
//! A reply carries the earlier messages as `>` quotes, each introduced by an
//! attribution line ("Le 14 oct. 2026 à 10:00, Mano a écrit :"). When the vault
//! holds exactly one note that matches the attribution (sender, minute) and the
//! start of the quoted text, the attribution and the quote become one line,
//! `Message cité : [[note]]`, and the link is listed after the block in a
//! `### Messages cités :` section (Obsidian does not index links in a code block).
//! Anything else is left untouched: a wrong link would lose text for good.

use crate::email_block::{note_header_field, read_body};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, NaiveDate};
use regex::Regex;
use std::collections::HashMap;
use std::path::Path;
use std::sync::LazyLock;
use walkdir::WalkDir;

const SECTION_TITLE: &str = "### Messages cités :";
const LINK_LABEL: &str = "Message cité : ";
/// Alphanumeric characters compared between a quote and a note body.
const TEXT_KEY_LEN: usize = 40;
/// Below this, a text start says too little to tell two messages apart.
const TEXT_KEY_MIN: usize = 12;

/// Why a quoted message was left as text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Left {
    /// Attribution without a date, or a reply whose own date has no offset.
    NoDate,
    /// No note matches.
    NoNote,
    /// Several notes match; no guess is made.
    Multiple,
    /// The reply writes between the quoted lines.
    Interleaved,
    /// The note has no literal `body = '''` to rewrite.
    NoBody,
}

/// Result of linking one note.
#[derive(Debug, Default)]
pub struct Outcome {
    /// The note text after linking; equal to the input when `linked` is empty.
    pub content: String,
    /// Wikilink targets written, in order.
    pub linked: Vec<String>,
    /// Quoted messages left as text, with the reason.
    pub left: Vec<Left>,
}

struct IndexedNote {
    /// Path of the note relative to the vault, `/`-separated, without `.md`.
    path: String,
    from: String,
    name: String,
    text_key: String,
}

/// The notes of a vault, findable by the minute they were sent.
#[derive(Default)]
pub struct Index {
    notes: Vec<IndexedNote>,
    by_minute: HashMap<i64, Vec<usize>>,
    /// Number of notes per file stem, to tell when a bare name is ambiguous.
    stems: HashMap<String, usize>,
}

impl Index {
    /// `notes` are `(path relative to the vault, full note text)`.
    pub fn build<'a>(notes: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let mut index = Index::default();
        for (rel_path, content) in notes {
            index.add(rel_path, content);
        }
        index
    }

    /// Index every `.md` note under `root`, never following symlinks.
    pub fn from_dir(root: &Path) -> Self {
        let mut index = Index::default();
        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.'))
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() || entry.path().extension().is_none_or(|e| e != "md") {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            if let Ok(rel) = entry.path().strip_prefix(root) {
                index.add(&rel.to_string_lossy(), &content);
            }
        }
        index
    }

    /// Add one note; a note without sender, date or readable body is only counted by name.
    pub fn add(&mut self, rel_path: &str, content: &str) {
        let path = rel_path
            .replace('\\', "/")
            .trim_end_matches(".md")
            .to_string();
        *self.stems.entry(stem(&path).to_string()).or_default() += 1;

        let Some(date) =
            note_header_field(content, "date").and_then(|d| DateTime::parse_from_rfc3339(&d).ok())
        else {
            return;
        };
        let Some(from) = note_header_field(content, "from") else {
            return;
        };
        let Some(body) = read_body(content).or_else(|| plain_body(content)) else {
            return;
        };
        let text_key = text_key(&body);
        if text_key.len() < TEXT_KEY_MIN {
            return;
        }
        let (name, address) = split_sender(&from);
        let slot = self.notes.len();
        self.by_minute
            .entry(date.timestamp().div_euclid(60))
            .or_default()
            .push(slot);
        self.notes.push(IndexedNote {
            path,
            from: address,
            name,
            text_key,
        });
    }

    /// `[[name]]` target: the file stem when no other note shares it, the path otherwise.
    fn link_target(&self, note: &IndexedNote) -> String {
        if self.stems.get(stem(&note.path)).copied().unwrap_or(0) > 1 {
            note.path.clone()
        } else {
            stem(&note.path).to_string()
        }
    }
}

fn stem(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Lowercase alphanumerics only, so quote marks, links and wrapping do not matter.
fn alnum(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn text_key(text: &str) -> String {
    alnum(text).chars().take(TEXT_KEY_LEN).collect()
}

/// `(display name, address)` of a `From` value, both normalized by [`alnum`].
fn split_sender(from: &str) -> (String, String) {
    match from.split_once('<') {
        Some((name, rest)) => (
            alnum(name.trim_matches(|c: char| c == '"' || c.is_whitespace())),
            alnum(rest.split('>').next().unwrap_or_default()),
        ),
        None => (String::new(), alnum(from)),
    }
}

// ── Attribution lines ───────────────────────────────────────────────────────

struct Attribution {
    /// Local time as written (the replier's time zone).
    date: Option<chrono::NaiveDateTime>,
    /// Text between the date and "a écrit", normalized by [`alnum`].
    sender: String,
}

static FR_NUMERIC_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^Le (?:[[:alpha:]]+\.? )?(\d{1,2})/(\d{1,2})/(\d{2,4}) (?:à|a) (\d{1,2}):(\d{2})(?::\d{2})?,? (.*?) a écrit ?:?\s*$",
    )
    .expect("static regex")
});
static FR_TEXT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^Le (?:[[:alpha:]]+\.? )?(\d{1,2}) ([[:alpha:]\.]+) (\d{4}) (?:à|a) (\d{1,2}):(\d{2})(?::\d{2})?,? (.*?) a écrit ?:?\s*$",
    )
    .expect("static regex")
});
static EN_DAY_FIRST_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^On (?:[A-Za-z]{3,9},? )?(\d{1,2}) ([A-Za-z]{3,9})\.?,? (\d{4})(?:,? at)? (\d{1,2}):(\d{2})(?::\d{2})?,? (.*?) wrote ?:?\s*$",
    )
    .expect("static regex")
});
static EN_MONTH_FIRST_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^On (?:[A-Za-z]{3,9},? )?([A-Za-z]{3,9})\.? (\d{1,2}),? (\d{4}),? (?:at )?(\d{1,2}):(\d{2})(?::\d{2})? ?([AP]M)?,? (.*?) wrote ?:?\s*$",
    )
    .expect("static regex")
});
static UNDATED_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(.{0,160}?) (?:a écrit|wrote) ?:\s*$").expect("static regex"));
/// First line of an attribution that a mail client wrapped onto two lines.
static ATTRIBUTION_HEAD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:Le|On) .*\d{1,2}:\d{2}").expect("static regex"));

fn month_number(name: &str) -> Option<u32> {
    let lower = name.trim_end_matches('.').to_lowercase();
    let mut chars = lower.chars();
    let head: String = chars.by_ref().take(3).collect();
    Some(match head.as_str() {
        "jan" => 1,
        "fév" | "fev" | "feb" => 2,
        "mar" => 3,
        "avr" | "apr" => 4,
        "mai" | "may" => 5,
        "jui" | "jun" | "jul" => {
            if head == "jun" || (head == "jui" && chars.next() == Some('n')) {
                6
            } else {
                7
            }
        }
        "aoû" | "aou" | "aug" => 8,
        "sep" => 9,
        "oct" => 10,
        "nov" => 11,
        "déc" | "dec" => 12,
        _ => return None,
    })
}

fn naive(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> Option<chrono::NaiveDateTime> {
    NaiveDate::from_ymd_opt(year, month, day)?.and_hms_opt(hour, minute, 0)
}

fn number<T: std::str::FromStr>(caps: &regex::Captures, group: usize) -> Option<T> {
    caps.get(group)?.as_str().parse().ok()
}

fn year(caps: &regex::Captures, group: usize) -> Option<i32> {
    let year: i32 = number(caps, group)?;
    Some(if year < 100 { 2000 + year } else { year })
}

fn parse_attribution(text: &str) -> Option<Attribution> {
    let text = text.trim().trim_start_matches('\\').trim();
    if let Some(c) = FR_NUMERIC_RE.captures(text) {
        let date = naive(
            year(&c, 3)?,
            number(&c, 2)?,
            number(&c, 1)?,
            number(&c, 4)?,
            number(&c, 5)?,
        );
        return Some(Attribution {
            date,
            sender: alnum(&c[6]),
        });
    }
    if let Some(c) = FR_TEXT_RE.captures(text) {
        let date = naive(
            year(&c, 3)?,
            month_number(&c[2])?,
            number(&c, 1)?,
            number(&c, 4)?,
            number(&c, 5)?,
        );
        return Some(Attribution {
            date,
            sender: alnum(&c[6]),
        });
    }
    if let Some(c) = EN_DAY_FIRST_RE.captures(text) {
        let date = naive(
            year(&c, 3)?,
            month_number(&c[2])?,
            number(&c, 1)?,
            number(&c, 4)?,
            number(&c, 5)?,
        );
        return Some(Attribution {
            date,
            sender: alnum(&c[6]),
        });
    }
    if let Some(c) = EN_MONTH_FIRST_RE.captures(text) {
        let mut hour: u32 = number(&c, 4)?;
        match c.get(6).map(|m| m.as_str()) {
            Some("PM") if hour < 12 => hour += 12,
            Some("AM") if hour == 12 => hour = 0,
            _ => {}
        }
        let date = naive(
            year(&c, 3)?,
            month_number(&c[1])?,
            number(&c, 2)?,
            hour,
            number(&c, 5)?,
        );
        return Some(Attribution {
            date,
            sender: alnum(&c[7]),
        });
    }
    UNDATED_RE.captures(text).map(|c| Attribution {
        date: None,
        sender: alnum(&c[1]),
    })
}

// ── Quoted lines ────────────────────────────────────────────────────────────

/// `(quote depth, text without the `>` marks)`.
fn split_quote(line: &str) -> (usize, &str) {
    let mut depth = 0;
    let mut rest = line.trim_start();
    while let Some(next) = rest.strip_prefix('>') {
        depth += 1;
        rest = next.trim_start();
    }
    (depth, rest)
}

/// What the quoted message starts with, as [`text_key`].
fn quoted_key(lines: &[&str]) -> String {
    let text: Vec<&str> = lines.iter().map(|l| split_quote(l).1).collect();
    text_key(&text.join(" "))
}

/// Lines of a message quoted after an attribution at `depth`.
///
/// Returns `(end of the quoted run, interleaved)`: `end` is exclusive, `interleaved`
/// tells that the reply writes between quoted lines, so the run is not one block.
fn quoted_run(lines: &[&str], from: usize, depth: usize) -> (usize, bool) {
    let mut end = from;
    let mut k = from;
    while k < lines.len() {
        let (d, text) = split_quote(lines[k]);
        if d > depth {
            end = k + 1;
        } else if !text.trim().is_empty() {
            break;
        }
        k += 1;
    }
    // Quoted lines again before the next attribution: the reply is written inline.
    let mut interleaved = false;
    for line in &lines[k..] {
        let (d, text) = split_quote(line);
        if parse_attribution(text).is_some() && d <= depth {
            break;
        }
        if d > depth {
            interleaved = true;
            break;
        }
    }
    (end, interleaved)
}

// ── Linking ─────────────────────────────────────────────────────────────────

/// Link the quoted messages of `content` that have a note in `index`.
///
/// `own_path` is the path of the note itself, relative to the vault (it is never
/// its own match).
pub fn link_note(content: &str, own_path: &str, index: &Index) -> Outcome {
    let unchanged = |left: Vec<Left>| Outcome {
        content: content.to_string(),
        linked: Vec::new(),
        left,
    };

    let own_path = own_path.replace('\\', "/");
    let own_path = own_path.trim_end_matches(".md");

    let lines: Vec<&str> = content.lines().collect();
    let Some((start, close, in_block)) = locate_body(&lines) else {
        return unchanged(vec![Left::NoBody]);
    };

    let mut out: Vec<String> = lines[..start].iter().map(|l| l.to_string()).collect();
    let mut linked: Vec<String> = Vec::new();
    let mut left: Vec<Left> = Vec::new();

    let body = &lines[..close];
    let mut i = start;
    while i < close {
        let (depth, text) = split_quote(body[i]);
        let mut consumed = 1;
        let mut attribution = parse_attribution(text);
        if attribution.is_none() && i + 1 < close && ATTRIBUTION_HEAD_RE.is_match(text.trim()) {
            let (next_depth, next) = split_quote(body[i + 1]);
            if next_depth == depth {
                attribution = parse_attribution(&format!("{} {}", text.trim(), next.trim()));
                consumed = 2;
            }
        }
        let Some(attribution) = attribution else {
            out.push(body[i].to_string());
            i += 1;
            continue;
        };

        let from = i + consumed;
        let (end, interleaved) = quoted_run(&body[..close], from, depth);
        if end == from {
            // Not a citation: nothing quoted follows.
            out.push(body[i].to_string());
            i += 1;
            continue;
        }
        let outcome = if interleaved {
            Err(Left::Interleaved)
        } else {
            find_note(index, &attribution, &quoted_key(&body[from..end]), own_path)
        };
        match outcome {
            Ok(target) => {
                let prefix = if depth > 0 {
                    format!("{} ", ">".repeat(depth))
                } else {
                    String::new()
                };
                out.push(format!("{prefix}{LINK_LABEL}[[{target}]]"));
                if !linked.contains(&target) {
                    linked.push(target);
                }
                i = end;
            }
            Err(reason) => {
                left.push(reason);
                out.push(body[i].to_string());
                i += 1;
            }
        }
    }

    if linked.is_empty() {
        return unchanged(left);
    }
    let close_out = out.len();
    out.extend(lines[close..].iter().map(|l| l.to_string()));
    let content_out = if in_block {
        add_section(out, &linked, close_out)
    } else {
        out.join("\n")
    };
    Outcome {
        content: if content.ends_with('\n') {
            format!("{content_out}\n")
        } else {
            content_out
        },
        linked,
        left,
    }
}

/// Body of a plain note (no ```email block): the text after the frontmatter.
fn plain_body(content: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    let (start, end, in_block) = locate_body(&lines)?;
    (!in_block).then(|| {
        lines[start..end].join(
            "
",
        )
    })
}

/// Offsets (minutes) a mail client may have written an attribution in, UTC-12 to UTC+14.
fn plausible_offsets() -> impl Iterator<Item = i64> {
    (-12 * 60..=14 * 60).step_by(15)
}

/// Where the body lives: `(first line, line after the last, inside an ```email block)`.
///
/// Either the literal TOML `body = '''` of the block, or, for a plain note with no
/// block, everything after the frontmatter.
fn locate_body(lines: &[&str]) -> Option<(usize, usize, bool)> {
    if let Some(open) = lines.iter().position(|l| l.trim_end() == "body = '''") {
        let close = lines[open + 1..]
            .iter()
            .position(|l| l.trim_end() == "'''")?;
        return Some((open + 1, open + 1 + close, true));
    }
    if lines.iter().any(|l| l.starts_with("```email")) || lines.first() != Some(&"---") {
        return None;
    }
    let end = lines[1..].iter().position(|l| l.trim_end() == "---")?;
    Some((end + 2, lines.len(), false))
}

fn find_note(
    index: &Index,
    attribution: &Attribution,
    key: &str,
    own_path: &str,
) -> Result<String, Left> {
    let Some(local) = attribution.date else {
        return Err(Left::NoDate);
    };
    if key.len() < TEXT_KEY_MIN {
        return Err(Left::NoNote);
    }
    // The attribution is in the time zone of whoever wrote the reply, which the note
    // does not keep reliably (exports often store UTC): try every plausible offset.
    let written = local.and_utc().timestamp().div_euclid(60);
    let mut slots: Vec<usize> = plausible_offsets()
        .filter_map(|offset| index.by_minute.get(&(written - offset)))
        .flatten()
        .copied()
        .collect();
    slots.sort_unstable();
    slots.dedup();
    let mut found = slots.iter().map(|&s| &index.notes[s]).filter(|n| {
        n.path != own_path
            && n.text_key == key
            && ((!n.from.is_empty() && attribution.sender.contains(&n.from))
                || (!n.name.is_empty() && attribution.sender.contains(&n.name)))
    });
    match (found.next(), found.next()) {
        (Some(note), None) => Ok(index.link_target(note)),
        (Some(_), Some(_)) => Err(Left::Multiple),
        _ => Err(Left::NoNote),
    }
}

/// Insert `linked` in the `### Messages cités :` section after the block's closing
/// fence, creating the section or completing the existing one.
fn add_section(mut lines: Vec<String>, linked: &[String], close: usize) -> String {
    let fence = lines[close + 1..]
        .iter()
        .position(|l| l.starts_with("```") && l.chars().all(|c| c == '`'))
        .map(|p| close + 1 + p);
    let Some(fence) = fence else {
        return lines.join("\n");
    };
    let items = |targets: Vec<&String>| -> Vec<String> {
        targets.into_iter().map(|t| format!("- [[{t}]]")).collect()
    };
    match lines[fence + 1..]
        .iter()
        .position(|l| l.trim_end() == SECTION_TITLE)
    {
        Some(p) => {
            let title = fence + 1 + p;
            let mut end = title + 1;
            while end < lines.len() && lines[end].starts_with("- ") {
                end += 1;
            }
            let missing: Vec<&String> = linked
                .iter()
                .filter(|t| {
                    !lines[title + 1..end]
                        .iter()
                        .any(|l| l == &format!("- [[{t}]]"))
                })
                .collect();
            for (offset, item) in items(missing).into_iter().enumerate() {
                lines.insert(end + offset, item);
            }
        }
        None => {
            let mut section = vec![String::new(), SECTION_TITLE.to_string()];
            section.extend(items(linked.iter().collect()));
            for (offset, line) in section.into_iter().enumerate() {
                lines.insert(fence + 1 + offset, line);
            }
        }
    }
    lines.join("\n")
}

// ── Whole vault ─────────────────────────────────────────────────────────────

/// What [`link_vault`] did, or would do in a dry run.
#[derive(Debug, Default)]
pub struct VaultReport {
    /// Notes scanned.
    pub scanned: usize,
    /// Notes with at least one quoted message linked, as paths relative to the vault.
    pub rewritten: Vec<String>,
    /// Quoted messages linked.
    pub linked: usize,
    /// Quoted messages left as text, counted by reason.
    pub left: HashMap<Left, usize>,
}

/// Canonical form of `path`, even when its last components do not exist yet.
fn resolve_missing(path: &Path) -> std::path::PathBuf {
    let mut tail = Vec::new();
    let mut head = path;
    while let Some(parent) = head.parent() {
        if let Ok(resolved) = head.canonicalize() {
            return tail.iter().rev().fold(resolved, |acc, name| acc.join(name));
        }
        if let Some(name) = head.file_name() {
            tail.push(name);
        }
        head = parent;
    }
    path.to_path_buf()
}

/// Link the quoted messages of every note under `root`.
///
/// Without `backup_dir` nothing is written. With it, each note is copied there
/// (same relative path) before being rewritten; `backup_dir` must be outside `root`
/// so Obsidian does not index the copies.
pub fn link_vault(root: &Path, backup_dir: Option<&Path>) -> Result<VaultReport> {
    if let Some(backup) = backup_dir {
        let root_abs = root.canonicalize().context("failed to resolve vault")?;
        let backup_abs = resolve_missing(backup);
        if backup_abs.starts_with(&root_abs) {
            bail!("backup directory must be outside the vault");
        }
    }
    let index = Index::from_dir(root);
    let mut report = VaultReport::default();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() || entry.path().extension().is_none_or(|e| e != "md") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let Ok(rel) = entry.path().strip_prefix(root) else {
            continue;
        };
        report.scanned += 1;
        let outcome = link_note(&content, &rel.to_string_lossy(), &index);
        for reason in &outcome.left {
            *report.left.entry(*reason).or_default() += 1;
        }
        if outcome.linked.is_empty() {
            continue;
        }
        report.linked += outcome.linked.len();
        report
            .rewritten
            .push(rel.to_string_lossy().replace('\\', "/"));
        if let Some(backup) = backup_dir {
            let copy = backup.join(rel);
            if let Some(parent) = copy.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            std::fs::write(&copy, &content)
                .with_context(|| format!("failed to back up {}", copy.display()))?;
            std::fs::write(entry.path(), &outcome.content)
                .with_context(|| format!("failed to write {}", entry.path().display()))?;
        }
    }
    Ok(report)
}
