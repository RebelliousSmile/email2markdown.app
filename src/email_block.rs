//! The ```email block a note carries in its body.
//!
//! The note's frontmatter describes the *note* (tags, proof of origin, linked
//! files); the message's own header (subject, from, to, cc, date) and body live in
//! a fenced block whose content is TOML, rendered by the Obsidian "Notebook"
//! plugin. Readers that used to look at `from:`/`to:`/`subject:`/`date:` in the
//! frontmatter fall back to this block via [`read_header`].

use serde::Deserialize;

/// Header fields of a message, as written in the block.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BlockHeader {
    pub subject: String,
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    /// RFC 3339 when the exporter could parse the `Date` header, raw otherwise.
    pub date: String,
}

#[derive(Deserialize, Default)]
struct RawBlock {
    #[serde(default)]
    subject: Option<String>,
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    to: Option<OneOrMany>,
    #[serde(default)]
    cc: Option<OneOrMany>,
    #[serde(default)]
    date: Option<toml::Value>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    fn into_vec(self) -> Vec<String> {
        match self {
            OneOrMany::One(s) => vec![s],
            OneOrMany::Many(v) => v,
        }
    }
}

/// Escape a value for a TOML basic (single-line) string.
fn basic_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\u{:04X}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn string_array(values: &[String]) -> String {
    let items: Vec<String> = values.iter().map(|v| basic_string(v)).collect();
    format!("[{}]", items.join(", "))
}

/// A multi-line TOML string that reads back as `body` exactly.
///
/// A literal `'''` string needs no escaping; one holding `'''` itself falls back to
/// a basic `"""` string with the offending characters escaped.
fn multiline(body: &str) -> String {
    let body = body.replace("\r\n", "\n");

    let control = body
        .chars()
        .any(|c| (c.is_control() && c != '\n' && c != '\t') || c == '\u{7f}');

    if !body.contains("'''") && !control && !body.ends_with('\'') {
        return format!("'''\n{}\n'''", body);
    }

    let mut out = String::from("\"\"\"\n");
    for c in body.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' | '\t' => out.push(c),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push_str("\n\"\"\"");
    out
}

/// The fence must be longer than any backtick run of the body, or the body would
/// close the block early.
fn fence_for(body: &str) -> String {
    let longest = body
        .split(|c| c != '`')
        .map(str::len)
        .max()
        .unwrap_or_default();
    "`".repeat((longest + 1).max(3))
}

/// Render the fenced block for one message.
///
/// `date` is written as a TOML date-time when it is RFC 3339 (the plugin and
/// `tomllib` both read it back as a real date), as a string otherwise.
pub fn render(header: &BlockHeader, body: &str) -> String {
    let fence = fence_for(body);
    let mut out = String::new();

    out.push_str(&fence);
    out.push_str("email\n");
    out.push_str(&format!("subject = {}\n", basic_string(&header.subject)));
    out.push_str(&format!("from = {}\n", basic_string(&header.from)));
    out.push_str(&format!("to = {}\n", string_array(&header.to)));
    if !header.cc.is_empty() {
        out.push_str(&format!("cc = {}\n", string_array(&header.cc)));
    }
    if !header.date.is_empty() {
        if chrono::DateTime::parse_from_rfc3339(&header.date).is_ok() {
            out.push_str(&format!("date = {}\n", header.date));
        } else {
            out.push_str(&format!("date = {}\n", basic_string(&header.date)));
        }
    }
    out.push_str(&format!("body = {}\n", multiline(body.trim_matches('\n'))));
    out.push_str(&fence);
    out.push('\n');

    out
}

/// Content of the first ```email block of a note (without its fences).
fn block_source(content: &str) -> Option<String> {
    let mut lines = content.lines();

    while let Some(line) = lines.next() {
        let trimmed = line.trim_end();
        let ticks = trimmed.chars().take_while(|c| *c == '`').count();

        if ticks < 3 || trimmed[ticks..].trim() != "email" {
            continue;
        }

        let mut source = String::new();
        for inner in lines.by_ref() {
            let inner_trimmed = inner.trim_end();
            let closing = inner_trimmed.chars().take_while(|c| *c == '`').count();
            if closing >= ticks && inner_trimmed.chars().all(|c| c == '`') {
                return Some(source);
            }
            source.push_str(inner);
            source.push('\n');
        }

        return None;
    }

    None
}

/// Header of the note's ```email block, if it has a valid one.
pub fn read_header(content: &str) -> Option<BlockHeader> {
    let source = block_source(content)?;
    let raw: RawBlock = toml::from_str(&source).ok()?;

    let date = match raw.date {
        Some(toml::Value::Datetime(d)) => d.to_string(),
        Some(toml::Value::String(s)) => s,
        _ => String::new(),
    };

    Some(BlockHeader {
        subject: raw.subject.unwrap_or_default(),
        from: raw.from.unwrap_or_default(),
        to: raw.to.map(OneOrMany::into_vec).unwrap_or_default(),
        cc: raw.cc.map(OneOrMany::into_vec).unwrap_or_default(),
        date,
    })
}

/// Scalar value of a frontmatter key (`subject` never matches `subject_hash`).
fn frontmatter_scalar(content: &str, field: &str) -> Option<String> {
    let rest = content.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    let prefix = format!("{field}:");
    for line in rest[..end].lines() {
        if let Some(value) = line.trim().strip_prefix(&prefix) {
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// One header field (`subject`, `from`, `to`, `cc`, `date`) of a note.
///
/// The frontmatter wins (notes exported before the block existed), the
/// ```email block fills in otherwise. `to` / `cc` are joined with `", "`.
/// Every reader of a message header goes through here.
pub fn note_header_field(content: &str, field: &str) -> Option<String> {
    if let Some(value) = frontmatter_scalar(content, field) {
        return Some(value);
    }
    let header = read_header(content)?;
    let value = match field {
        "subject" => header.subject,
        "from" => header.from,
        "to" => header.to.join(", "),
        "cc" => header.cc.join(", "),
        "date" => header.date,
        _ => return None,
    };
    (!value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header() -> BlockHeader {
        BlockHeader {
            subject: "Devis \"salle de bain\"".to_string(),
            from: "Alice <alice@example.com>".to_string(),
            to: vec!["Bob <bob@example.com>".to_string()],
            cc: vec!["Carol <carol@example.com>".to_string()],
            date: "2026-07-02T10:18:19+00:00".to_string(),
        }
    }

    #[test]
    fn render_then_read_round_trips_the_header() {
        let block = render(&header(), "Bonjour,\n\nMerci.");
        assert_eq!(read_header(&block), Some(header()));
    }

    #[test]
    fn body_with_triple_quotes_and_backslashes_survives() {
        let body = "path C:\\tmp\n'''quoted'''\n\"\"\"also\"\"\"";
        let block = render(&header(), body);
        let source = block_source(&block).unwrap();
        let table: toml::Table = toml::from_str(&source).unwrap();
        // The closing delimiter sits on its own line, so the value ends with one newline.
        assert_eq!(table["body"].as_str().unwrap().trim_end_matches('\n'), body);
    }

    #[test]
    fn body_with_code_fence_gets_a_longer_fence() {
        let block = render(&header(), "```\ncode\n```");
        assert!(block.starts_with("````email\n"), "got {block:?}");
        assert_eq!(read_header(&block), Some(header()));
    }

    #[test]
    fn unparseable_date_is_kept_as_a_string() {
        let mut h = header();
        h.date = "Mon, 2 Jul".to_string();
        let block = render(&h, "x");
        assert_eq!(read_header(&block).unwrap().date, "Mon, 2 Jul");
    }

    #[test]
    fn test_note_header_field_reads_block_when_frontmatter_lacks_it() {
        let note = format!(
            "---\ntags:\n  - email\nsubject_hash: abc\n---\n\n{}",
            render(&header(), "x")
        );
        assert_eq!(
            note_header_field(&note, "from").as_deref(),
            Some("Alice <alice@example.com>")
        );
        assert_eq!(
            note_header_field(&note, "date").as_deref(),
            Some("2026-07-02T10:18:19+00:00")
        );
        assert_eq!(
            note_header_field(&note, "to").as_deref(),
            Some("Bob <bob@example.com>")
        );
    }

    #[test]
    fn test_note_header_field_frontmatter_wins_over_block() {
        let note = format!("---\nsubject: Ancien\n---\n\n{}", render(&header(), "x"));
        assert_eq!(
            note_header_field(&note, "subject").as_deref(),
            Some("Ancien")
        );
    }

    #[test]
    fn test_note_header_field_does_not_confuse_subject_hash() {
        let note = "---\nsubject_hash: abc\n---\nbody";
        assert_eq!(note_header_field(note, "subject"), None);
    }

    #[test]
    fn test_note_header_field_invalid_block_is_none() {
        let note = "---\ntags: []\n---\n\n```email\nsubject = \n```\n";
        assert_eq!(note_header_field(note, "subject"), None);
    }

    #[test]
    fn note_without_block_has_no_header() {
        assert_eq!(read_header("---\nsubject: x\n---\n\nbody"), None);
    }
}
