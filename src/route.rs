use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset};
use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// Safe path-segment shape — alphanumerics, space, dash, underscore, dot.
/// Used to validate user-typed notes destinations against path-traversal.
static SAFE_PATH_SEGMENT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9À-ſ _.\-]+$").expect("static regex"));

/// Join `dest` (user-typed `Travail/Projets/X`) onto `root` safely.
///
/// Rejects segments that:
/// - contain a Windows separator `\`
/// - are `..` or `.` (path-traversal)
/// - contain any character outside `SAFE_PATH_SEGMENT_RE`
///
/// Returns the joined `PathBuf` on success, or an error naming the bad segment.
pub fn join_safe_segments(root: &Path, dest: &str) -> Result<PathBuf> {
    let mut out = root.to_path_buf();
    for raw in dest.split('/') {
        let seg = raw.trim();
        if seg.is_empty() {
            continue;
        }
        if seg == ".." || seg == "." || seg.contains('\\') {
            anyhow::bail!("invalid path segment in notes destination: {:?}", seg);
        }
        if !SAFE_PATH_SEGMENT_RE.is_match(seg) {
            anyhow::bail!(
                "notes destination segment contains forbidden characters: {:?}",
                seg
            );
        }
        out = out.join(seg);
    }
    Ok(out)
}

/// Compute a relative path from `from_dir` to `to`.
/// Both paths should be absolute for correct results.
fn relative_path_from(from_dir: &Path, to: &Path) -> PathBuf {
    let mut from_parts: Vec<_> = from_dir.components().collect();
    let mut to_parts: Vec<_> = to.components().collect();

    // Strip common prefix
    let common = from_parts
        .iter()
        .zip(to_parts.iter())
        .take_while(|(a, b)| a == b)
        .count();
    from_parts.drain(..common);
    to_parts.drain(..common);

    let mut result = PathBuf::new();
    for _ in &from_parts {
        result.push("..");
    }
    for part in to_parts {
        result.push(part);
    }
    result
}

/// Rewrite attachment paths in a `.md` file's YAML frontmatter so they are
/// relative to `new_parent_dir` instead of `old_parent_dir`.
/// Both `old_parent_dir` and `new_parent_dir` must be absolute paths.
pub fn rewrite_attachment_paths(
    md_path: &Path,
    old_parent_dir: &Path,
    new_parent_dir: &Path,
) -> Result<()> {
    let content = fs::read_to_string(md_path)
        .with_context(|| format!("failed to read {}", md_path.display()))?;

    // Only process files that have a YAML frontmatter block.
    let Some(rest) = content.strip_prefix("---\n") else {
        return Ok(());
    };
    let Some(end) = rest.find("\n---") else {
        return Ok(());
    };
    let frontmatter = &rest[..end];
    let after_frontmatter = &rest[end + 4..]; // skip "\n---"

    // Rewrite each line that is an attachment list item: "  - <path>"
    // We look for lines inside an `attachments:` block.
    // TODO: unify with serde_yaml parse (see plan Phase 2) — rewrite_attachment_paths uses a
    // line-parser because it must emit rewritten YAML lines (different data shape from parsing
    // the list alone). Refactoring the enumeration step to use parse_frontmatter_attachments
    // would require a two-pass approach (parse then reserialize) — not done here.
    let mut in_attachments = false;
    let mut new_frontmatter = String::with_capacity(frontmatter.len());

    for line in frontmatter.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("attachments:") {
            in_attachments = true;
            new_frontmatter.push_str(line);
            new_frontmatter.push('\n');
            continue;
        }
        if in_attachments && trimmed.starts_with("- ") {
            // Extract the path after "- "
            let path_str = trimmed.trim_start_matches("- ");
            // Absolute attachment path from old_parent_dir
            let abs = old_parent_dir.join(path_str.replace('/', std::path::MAIN_SEPARATOR_STR));
            // Relative path from new_parent_dir to abs
            let rel = relative_path_from(new_parent_dir, &abs);
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            // Preserve original indentation
            let indent: String = line.chars().take_while(|c| *c == ' ').collect();
            new_frontmatter.push_str(&format!("{}- {}\n", indent, rel_str));
            continue;
        }
        // A non-list line ends the attachments block
        if in_attachments && !trimmed.starts_with('-') {
            in_attachments = false;
        }
        new_frontmatter.push_str(line);
        new_frontmatter.push('\n');
    }

    let new_content = format!("---\n{}---{}", new_frontmatter, after_frontmatter);
    fs::write(md_path, new_content)
        .with_context(|| format!("failed to write updated frontmatter to {}", md_path.display()))?;
    Ok(())
}

/// Extract the `attachments:` list from `.md` content's YAML frontmatter.
///
/// Uses `serde_yaml` deserialization so that YAML-quoted names (e.g.
/// `- 'invoice #5.pdf'`) are correctly unquoted — a line-parser would return
/// the quoted string verbatim, causing the file to go missing silently.
///
/// Returns `None` if no valid frontmatter delimiters are present.
/// Returns `Some(Err(_))` if the YAML block is malformed.
/// Returns `Some(Ok(vec))` on success (empty when `attachments:` is absent).
fn parse_frontmatter_attachments(content: &str) -> Option<Result<Vec<String>>> {
    #[derive(serde::Deserialize)]
    struct AttachmentsHead {
        #[serde(default)]
        attachments: Vec<String>,
    }

    let rest = content.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    let frontmatter = &rest[..end];

    Some(
        serde_yaml::from_str::<AttachmentsHead>(frontmatter)
            .map(|h| h.attachments)
            .map_err(|e| anyhow::anyhow!("failed to parse frontmatter YAML: {}", e)),
    )
}

/// Move a `.md` file and its sibling attachment files into `dest_dir`.
///
/// Attachments are identified by deserializing the `attachments:` list from the
/// YAML frontmatter via `serde_yaml`. Each link is resolved from the source directory;
/// only files actually inside that directory are moved. Links that resolve outside
/// (e.g. legacy centralized `attachments/` exports) are skipped with a warning.
///
/// Steps:
/// 1. Reject symlinks: if `md_path` is a symlink, return `Err` immediately (no FS mutation).
/// 2. Read the `.md` content and extract the attachment list via `parse_frontmatter_attachments`.
/// 3. Move each attachment from the source directory into `dest_dir`.
/// 4. Move the `.md` itself into `dest_dir`.
/// 5. Rewrite attachment paths in the moved `.md` (same-folder bare links stay unchanged).
///
/// The move is attempted with `fs::rename`; if that crosses device boundaries the
/// fallback is `fs::copy` + `fs::remove_file`.
pub fn move_email(md_path: &Path, dest_dir: &Path) -> Result<()> {
    // --- Symlink guard (project rule 02-rust-filesystem-safety) ---
    let meta = md_path
        .symlink_metadata()
        .with_context(|| format!("failed to stat {}", md_path.display()))?;
    if meta.file_type().is_symlink() {
        anyhow::bail!(
            "refusing to move symlink: {}",
            md_path.display()
        );
    }

    let old_parent = md_path
        .parent()
        .with_context(|| format!("md_path has no parent: {}", md_path.display()))?;

    // --- Read .md content and extract attachment list via serde_yaml ---
    // Graceful degradation: read or parse failure logs a warning and leaves the
    // attachment list empty; the .md move proceeds regardless (parity with the
    // existing warning tolerance around `rewrite_attachment_paths`).
    let attachments: Vec<String> = match fs::read_to_string(md_path) {
        Err(e) => {
            eprintln!(
                "warning: could not read {} to extract attachment list: {}; moving .md only",
                md_path.display(),
                e
            );
            vec![]
        }
        Ok(content) => match parse_frontmatter_attachments(&content) {
            // No frontmatter delimiters — normal for emails without attachments.
            None => vec![],
            Some(Err(e)) => {
                eprintln!(
                    "warning: could not parse frontmatter in {}: {}; moving .md only",
                    md_path.display(),
                    e
                );
                vec![]
            }
            Some(Ok(list)) => list,
        },
    };

    // --- Move each referenced attachment that lives in old_parent ---
    for link in &attachments {
        let att_src = old_parent.join(link.replace('/', std::path::MAIN_SEPARATOR_STR));

        // Only move files actually inside the source directory.
        // Links that resolve outside (legacy centralized exports) are skipped.
        let in_source = att_src.parent().map_or(false, |p| p == old_parent);
        if !in_source {
            eprintln!(
                "warning: attachment {:?} resolves outside source dir {}; skipping (no migration)",
                link,
                old_parent.display()
            );
            continue;
        }

        if !att_src.exists() {
            continue;
        }

        let file_name = match att_src.file_name() {
            Some(n) => n,
            None => {
                eprintln!("warning: attachment {:?} has no file name; skipping", link);
                continue;
            }
        };
        let att_dest = dest_dir.join(file_name);

        if fs::rename(&att_src, &att_dest).is_err() {
            // Cross-device fallback: copy then remove.
            fs::copy(&att_src, &att_dest).with_context(|| {
                format!(
                    "failed to copy attachment {} to {}",
                    att_src.display(),
                    att_dest.display()
                )
            })?;
            fs::remove_file(&att_src).with_context(|| {
                format!(
                    "failed to remove original attachment {} after copy",
                    att_src.display()
                )
            })?;
        }
    }

    // --- Move the .md file ---
    let md_dest = dest_dir.join(
        md_path
            .file_name()
            .with_context(|| format!("md_path has no file name: {}", md_path.display()))?,
    );
    if fs::rename(md_path, &md_dest).is_err() {
        fs::copy(md_path, &md_dest).with_context(|| {
            format!(
                "failed to copy {} to {}",
                md_path.display(),
                dest_dir.display()
            )
        })?;
        fs::remove_file(md_path).with_context(|| {
            format!("failed to remove {} after copy", md_path.display())
        })?;
    }

    // --- Rewrite attachment paths in the moved .md ---
    // The .md and its attachments are now co-located in dest_dir. Passing dest_dir as
    // both old and new parent means bare links stay unchanged (relative_path_from is
    // identity when old == new).
    if let Err(e) = rewrite_attachment_paths(&md_dest, dest_dir, dest_dir) {
        eprintln!(
            "warning: could not update attachment paths in {}: {}",
            md_dest.display(),
            e
        );
    }

    Ok(())
}

// ── Routing types ────────────────────────────────────────────────────────────

/// Metadata extracted from an email and used for deterministic routing.
#[derive(Debug, Clone)]
pub struct EmailMeta {
    /// Full `From:` address (e.g. `alice@example.com`).
    pub from: String,
    /// Domain portion of `from` (e.g. `example.com`).
    pub domain: String,
    /// `Subject:` header value.
    pub subject: String,
    /// Account name that received this email (IMAP account identifier).
    pub account: String,
    /// Parsed send date from the email.
    pub date: DateTime<FixedOffset>,
}

/// Outcome of `route_email` — a relative path to join onto `notes_dir`.
#[derive(Debug, Clone)]
pub struct RouteDecision {
    /// Relative path (e.g. `Perso/Finance/Banque/2026/06`).
    /// Joined with `notes_dir` at apply time.
    pub rel_path: String,
    /// Human-readable description of the matching rule, or `None` for default.
    pub matched_rule: Option<String>,
    /// `true` when no rule matched and the fallback path was used.
    pub is_default: bool,
}

/// A match condition inside a `destinations.txt` entry.
#[derive(Debug, Clone, PartialEq)]
pub enum MatchRule {
    /// Matches the sender domain (case-insensitive, suffix-safe).
    Domain(String),
    /// Matches the full `From:` address (case-insensitive exact match).
    From(String),
    /// Matches a keyword in the subject (case-insensitive substring).
    Subject(String),
    /// Matches the IMAP account name (exact, case-sensitive).
    Account(String),
}

/// A single entry from `destinations.txt`.
#[derive(Debug, Clone)]
pub struct Destination {
    /// Relative path under `notes_dir` (e.g. `Perso/Finance/Banque`).
    pub path: String,
    /// Rules that trigger routing to this destination (empty = AI-only or ignored).
    pub rules: Vec<MatchRule>,
    /// `true` if this entry was tagged with the `default` attribute.
    pub is_default: bool,
}

// ── Parser ───────────────────────────────────────────────────────────────────

/// Parse the content of a `destinations.txt` file into a list of `Destination`s.
///
/// Syntax: `<path>  [ | <attr>, <attr>... ]`
/// Attributes: `domain:<d>`, `from:<addr>`, `subject:<kw>`, `account:<name>`, `default`.
/// - Empty lines and lines starting with `#` are silently skipped.
/// - Malformed attribute tokens are printed as warnings and skipped.
/// - More than one `default` entry is a **hard error** (returns `Err`).
pub fn parse_destinations(content: &str) -> Result<Vec<Destination>> {
    let mut destinations = Vec::new();
    let mut default_count: u32 = 0;

    for raw_line in content.lines() {
        let line = raw_line.trim();
        // Skip empty lines and comments.
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Split on first ` | ` (space-pipe-space) to separate path from attrs.
        let (path_part, attrs_part) = if let Some(idx) = line.find(" | ") {
            (&line[..idx], Some(&line[idx + 3..]))
        } else {
            (line, None)
        };

        let path = path_part.trim().to_string();
        if path.is_empty() {
            eprintln!("warning: destinations.txt — skipping line with empty path: {:?}", raw_line);
            continue;
        }

        let mut rules = Vec::new();
        let mut is_default = false;

        if let Some(attrs) = attrs_part {
            for token in attrs.split(',') {
                let token = token.trim();
                if token.is_empty() {
                    continue;
                }
                if token == "default" {
                    is_default = true;
                } else if let Some(d) = token.strip_prefix("domain:") {
                    if d.is_empty() {
                        eprintln!("warning: destinations.txt — empty domain value in {:?}", raw_line);
                    } else {
                        rules.push(MatchRule::Domain(d.to_string()));
                    }
                } else if let Some(a) = token.strip_prefix("from:") {
                    if a.is_empty() {
                        eprintln!("warning: destinations.txt — empty from value in {:?}", raw_line);
                    } else {
                        rules.push(MatchRule::From(a.to_string()));
                    }
                } else if let Some(k) = token.strip_prefix("subject:") {
                    if k.is_empty() {
                        eprintln!("warning: destinations.txt — empty subject value in {:?}", raw_line);
                    } else {
                        rules.push(MatchRule::Subject(k.to_string()));
                    }
                } else if let Some(a) = token.strip_prefix("account:") {
                    if a.is_empty() {
                        eprintln!("warning: destinations.txt — empty account value in {:?}", raw_line);
                    } else {
                        rules.push(MatchRule::Account(a.to_string()));
                    }
                } else {
                    eprintln!(
                        "warning: destinations.txt — unknown attribute token {:?} in line {:?}, skipping",
                        token, raw_line
                    );
                }
            }
        }

        if is_default {
            default_count += 1;
            if default_count > 1 {
                anyhow::bail!(
                    "destinations.txt: more than one `default` entry is not allowed \
                     (found a second one on path {:?})",
                    path
                );
            }
        }

        destinations.push(Destination { path, rules, is_default });
    }

    Ok(destinations)
}

// ── Router ───────────────────────────────────────────────────────────────────

/// Default fallback path (relative, without year/month — those are appended by the router).
const DEFAULT_BASE: &str = "Perso/Messy/Emails";

/// Route a single email deterministically using the rules from `destinations.txt`.
///
/// Matching order (first match wins): destinations are evaluated in the order they
/// appear in `destinations.txt` (first destination = highest priority). Within a
/// destination, rules are evaluated in the order they are declared on that line.
/// The first rule that matches wins, regardless of rule type (`Domain`, `From`,
/// `Subject`, `Account`). There is no priority hierarchy between rule types.
///
/// If no rule matches, the `default`-tagged entry is used; if none exists, the
/// hard-coded fallback `Perso/Messy/Emails/<Year>/<Month>` is returned.
///
/// The returned `rel_path` already includes `<Year>/<Month>` derived from `meta.date`.
///
/// # No Regex in this function
/// Subject matching uses `str::contains()` (case-insensitive substring). The keyword
/// `k` is dynamic (read from `destinations.txt`), so `static LazyLock<Regex>` is
/// inapplicable. `contains()` is correct and sufficient here.
pub fn route_email(meta: &EmailMeta, dests: &[Destination]) -> RouteDecision {
    let year = meta.date.format("%Y").to_string();
    let month = meta.date.format("%m").to_string();

    // Evaluate destinations in file order; within each destination, evaluate rules in
    // declaration order. First match wins — no priority hierarchy between rule types.
    for dest in dests {
        for rule in &dest.rules {
            let matched = match rule {
                MatchRule::Domain(d) => {
                    let meta_domain = meta.domain.to_lowercase();
                    let rule_domain = d.to_lowercase();
                    // Exact match OR subdomain (suffix ".{d}") — avoids false positives
                    // e.g. "notacme.com" must NOT match "acme.com".
                    meta_domain == rule_domain
                        || meta_domain.ends_with(&format!(".{}", rule_domain))
                }
                MatchRule::From(a) => meta.from.eq_ignore_ascii_case(a),
                // No Regex::new here — k is dynamic; str::contains is correct.
                MatchRule::Subject(k) => {
                    meta.subject.to_lowercase().contains(&k.to_lowercase())
                }
                MatchRule::Account(a) => meta.account == *a,
            };

            if matched {
                let rule_desc = format!("{:?}", rule);
                let rel_path = format!("{}/{}/{}", dest.path, year, month);
                return RouteDecision {
                    rel_path,
                    matched_rule: Some(rule_desc),
                    is_default: false,
                };
            }
        }
    }

    // No deterministic rule matched — look for a `default`-tagged entry.
    if let Some(default_dest) = dests.iter().find(|d| d.is_default) {
        let rel_path = format!("{}/{}/{}", default_dest.path, year, month);
        return RouteDecision {
            rel_path,
            matched_rule: None,
            is_default: true,
        };
    }

    // Hard-coded fallback.
    RouteDecision {
        rel_path: format!("{}/{}/{}", DEFAULT_BASE, year, month),
        matched_rule: None,
        is_default: true,
    }
}

// ── Apply ────────────────────────────────────────────────────────────────────

/// Apply a routing decision: create the target directory and move the `.md` file.
///
/// `rel_path` is joined onto `notes_dir` via `join_safe_segments` (anti-traversal).
/// Missing directories are created with `fs::create_dir_all` (D4).
/// `move_email` handles the `.md` + its referenced attachment siblings.
pub fn apply_decision(staging_md: &Path, rel_path: &str, notes_dir: &Path) -> Result<()> {
    let dest_dir = join_safe_segments(notes_dir, rel_path)
        .with_context(|| format!("invalid routing path {:?}", rel_path))?;
    fs::create_dir_all(&dest_dir)
        .with_context(|| format!("failed to create directory {}", dest_dir.display()))?;
    move_email(staging_md, &dest_dir)
        .with_context(|| format!("failed to move {} to {}", staging_md.display(), dest_dir.display()))
}

// ── AI extension point ────────────────────────────────────────────────────────

/// AI-assisted routing no-op.
///
/// Returns `None` when `ai_routing_enabled` is `false` (the default).
/// When AI is enabled (future work), this would call an LLM/classifier and return
/// a `RouteDecision` only if confidence ≥ `ai_confidence_threshold`.
pub fn ai_route(
    _meta: &EmailMeta,
    _dests: &[Destination],
    ai_routing_enabled: bool,
    _ai_confidence_threshold: f32,
) -> Option<RouteDecision> {
    if !ai_routing_enabled {
        return None;
    }
    // AI implementation is out of scope for M5.
    None
}

// ── Rule upsert ──────────────────────────────────────────────────────────────

/// Serialize a `MatchRule` to its `destinations.txt` token.
/// `Domain` value is lowercased — `route_email` compares via `to_lowercase()`.
fn match_rule_to_token(rule: &MatchRule) -> String {
    match rule {
        MatchRule::Domain(d)  => format!("domain:{}", d.to_lowercase()),
        MatchRule::From(a)    => format!("from:{}", a),
        MatchRule::Subject(k) => format!("subject:{}", k),
        MatchRule::Account(n) => format!("account:{}", n),
    }
}

/// Extract `(path_part, attrs)` from a raw destination line (no leading `#`).
/// `path_part` = text before the first ` | `, trimmed.
/// Returns `None` for `attrs` when there is no ` | ` separator.
fn extract_line_parts(line: &str) -> (&str, Option<&str>) {
    if let Some(idx) = line.find(" | ") {
        let path = line[..idx].trim();
        let attrs = line[idx + 3..].trim();
        (path, if attrs.is_empty() { None } else { Some(attrs) })
    } else {
        (line.trim(), None)
    }
}

/// Rebuild a destination line with `new_token` appended (dedup).
/// Returns `"path | attr1, attr2, ..."` or just `"path"` if the token list is empty.
fn merge_attrs(path: &str, existing_attrs: Option<&str>, new_token: &str) -> String {
    let mut tokens: Vec<String> = existing_attrs
        .map(|a| {
            a.split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect()
        })
        .unwrap_or_default();

    // Dedup: only append if the token is not already present.
    if !tokens.iter().any(|t| t == new_token) {
        tokens.push(new_token.to_string());
    }

    if tokens.is_empty() {
        path.to_string()
    } else {
        format!("{} | {}", path, tokens.join(", "))
    }
}

/// Upsert a routing rule into `destinations_file`, preserving all non-target lines verbatim.
///
/// Behaviour:
/// - File absent → treated as empty content; the new line is appended.
/// - Active line whose path matches `target_path` → merge new token (dedup).
/// - Commented line `# <path> [| attrs]` whose path matches `target_path` → uncomment + merge.
///   Free prose comments (non-matching path part) are left untouched.
/// - No match found → append `target_path | <token>` at end.
///
/// Order, blank lines, group headers, and all other comments are preserved byte-for-byte.
/// `Domain` values are lowercased to match `route_email`'s `to_lowercase()` comparison.
///
/// # Errors
/// Returns `Err` if `destinations_file` is a symlink (anti-symlink guard), or on I/O failure.
pub fn upsert_rule(destinations_file: &Path, target_path: &str, rule: MatchRule) -> Result<()> {
    // --- Read (absent file → empty content) ---
    let original: String = if destinations_file.exists() {
        fs::read_to_string(destinations_file)
            .with_context(|| format!("failed to read {}", destinations_file.display()))?
    } else {
        String::new()
    };

    // --- Anti-symlink guard before any write (rule 02-rust-filesystem-safety) ---
    if destinations_file.exists() {
        let meta = destinations_file
            .symlink_metadata()
            .with_context(|| format!("failed to stat {}", destinations_file.display()))?;
        if meta.file_type().is_symlink() {
            anyhow::bail!(
                "refusing to write to symlink: {}",
                destinations_file.display()
            );
        }
    }

    let new_token = match_rule_to_token(&rule);
    let had_trailing_newline = original.ends_with('\n');

    let mut output_lines: Vec<String> = Vec::new();
    let mut matched = false;

    for raw_line in original.lines() {
        let trimmed = raw_line.trim();

        if trimmed.is_empty() {
            // Blank line — preserve verbatim.
            output_lines.push(raw_line.to_string());
            continue;
        }

        if trimmed.starts_with('#') {
            // Could be a group header, free prose, or a commented destination.
            // Strip '#' + leading spaces and check if the path part matches target_path.
            let stripped = trimmed.trim_start_matches('#').trim_start();
            let (candidate_path, attrs_str) = extract_line_parts(stripped);

            if !matched && candidate_path.eq_ignore_ascii_case(target_path) {
                // Commented destination — uncomment and merge.
                let merged = merge_attrs(target_path, attrs_str, &new_token);
                output_lines.push(merged);
                matched = true;
            } else {
                // Free prose, group header, or different commented destination — verbatim.
                output_lines.push(raw_line.to_string());
            }
            continue;
        }

        // Active line — check if path matches target.
        let (line_path, attrs_str) = extract_line_parts(trimmed);
        if !matched && line_path.eq_ignore_ascii_case(target_path) {
            let merged = merge_attrs(target_path, attrs_str, &new_token);
            output_lines.push(merged);
            matched = true;
        } else {
            output_lines.push(raw_line.to_string());
        }
    }

    if !matched {
        // Target not found — append new line.
        output_lines.push(format!("{} | {}", target_path, new_token));
    }

    // --- Rebuild content ---
    let mut content = output_lines.join("\n");
    // Preserve trailing newline if original had one; also add one for a newly created line.
    if had_trailing_newline || !matched {
        content.push('\n');
    }

    // --- Write ---
    fs::write(destinations_file, content)
        .with_context(|| format!("failed to write {}", destinations_file.display()))?;

    Ok(())
}

