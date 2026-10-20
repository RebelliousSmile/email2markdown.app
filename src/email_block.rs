//! The ```email block a note carries in its body.
//!
//! The note's frontmatter describes the *note* (tags, proof of origin, linked
//! files); the message's own header (subject, from, to, cc, date) and body live in
//! a fenced block whose content is TOML, rendered by the Obsidian "Notebook"
//! plugin. Readers that used to look at `from:`/`to:`/`subject:`/`date:` in the
//! frontmatter fall back to this block via [`read_header`].

use regex::Regex;
use serde::Deserialize;
use std::sync::LazyLock;

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

/// Images and links found in a Markdown body, as `(label, target)` pairs.
#[derive(Debug, Default, PartialEq)]
pub struct BodyMedia {
    pub images: Vec<(String, String)>,
    pub links: Vec<(String, String)>,
}

static MEDIA_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        // An image destination may be `<my photo.png>` (what `rewrite_cid_images`
        // writes for a name with spaces); `regex` forbids reusing a group name,
        // hence the `_a` siblings.
        r"\[!\[(?P<ialt>[^\]]*)\]\((?:<(?P<iurl_a>[^>]+)>|(?P<iurl>[^)\s]+))[^)]*\)\]\((?P<lurl>[^)\s]+)[^)]*\)",
        r"|!\[(?P<alt>[^\]]*)\]\((?:<(?P<src_a>[^>]+)>|(?P<src>[^)\s]+))[^)]*\)",
        r"|\[(?P<text>[^\[\]]+)\]\((?P<href>[^)\s]+)[^)]*\)",
    ))
    .expect("static regex")
});

static REF_DEF_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\[(?P<id>[^\]]+)\]:[ \t]*(?P<url>\S+)[ \t]*$").expect("static regex")
});
static REF_USE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?P<bang>!?)\[(?P<text>[^\[\]]*)\]\[(?P<id>[^\]]+)\]").expect("static regex")
});
static REF_WRAPPED_IMAGE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[(?P<image>!\[[^\]]*\]\([^)]*\))\]\[(?P<id>[^\]]+)\]").expect("static regex")
});

/// Turn reference-style images and links (`[text][1]` + `[1]: url`, what the
/// HTML conversion emits) into inline ones, so one pattern reads both.
fn inline_references(body: &str) -> String {
    let defs: Vec<(String, String)> = REF_DEF_RE
        .captures_iter(body)
        .map(|c| (c["id"].to_string(), c["url"].to_string()))
        .collect();
    if defs.is_empty() {
        return body.to_string();
    }
    let url_of = |id: &str| defs.iter().find(|(d, _)| d == id).map(|(_, u)| u.clone());
    let first = REF_USE_RE.replace_all(body, |c: &regex::Captures| match url_of(&c["id"]) {
        Some(url) => format!("{}[{}]({})", &c["bang"], &c["text"], url),
        None => c[0].to_string(),
    });
    REF_WRAPPED_IMAGE_RE
        .replace_all(&first, |c: &regex::Captures| match url_of(&c["id"]) {
            Some(url) => format!("[{}]({})", &c["image"], url),
            None => c[0].to_string(),
        })
        .into_owned()
}

/// The block body is shown as plain text by the plugin, so Markdown images and
/// links inside it do not render. Collect them (deduplicated by target) to be
/// listed after the block.
pub fn body_media(body: &str) -> BodyMedia {
    let body = inline_references(body);
    let mut media = BodyMedia::default();
    let push_image = |media: &mut BodyMedia, alt: &str, src: &str| {
        if !src.starts_with("data:") && !media.images.iter().any(|(_, s)| s == src) {
            media.images.push((alt.trim().to_string(), src.to_string()));
        }
    };
    let push_link = |media: &mut BodyMedia, text: &str, href: &str| {
        if is_remote(href) && !media.links.iter().any(|(_, h)| h == href) {
            media
                .links
                .push((text.trim().to_string(), href.to_string()));
        }
    };
    for caps in MEDIA_RE.captures_iter(&body) {
        let iurl = caps.name("iurl_a").or_else(|| caps.name("iurl"));
        let src = caps.name("src_a").or_else(|| caps.name("src"));
        if let (Some(alt), Some(src), Some(href)) = (caps.name("ialt"), iurl, caps.name("lurl")) {
            push_image(&mut media, alt.as_str(), src.as_str());
            push_link(&mut media, alt.as_str(), href.as_str());
        } else if let (Some(alt), Some(src)) = (caps.name("alt"), src) {
            push_image(&mut media, alt.as_str(), src.as_str());
        } else if let (Some(text), Some(href)) = (caps.name("text"), caps.name("href")) {
            push_link(&mut media, text.as_str(), href.as_str());
        }
    }
    media
}

fn is_remote(target: &str) -> bool {
    target.starts_with("http://") || target.starts_with("https://")
}

/// Link to a file of the vault as Obsidian indexes it: `[[name]]`, or
/// `![[name]]` to embed it. Unlike `[name](name)`, it needs no escaping for
/// spaces or parentheses.
pub fn local_link(name: &str, embed: bool) -> String {
    format!("{}[[{name}]]", if embed { "!" } else { "" })
}

/// Sections listing the images then the links, each preceded by a blank line.
/// Empty when the body has neither.
pub fn render_media(media: &BodyMedia) -> String {
    let mut out = String::new();
    if !media.images.is_empty() {
        out.push_str("\n\n### Images :\n");
        for (alt, src) in &media.images {
            if is_remote(src) {
                out.push_str(&format!("\n![{alt}]({src})\n"));
            } else {
                // `![[file|alt]]` would read a numeric alt as a width: alt is dropped.
                out.push_str(&format!("\n{}\n", local_link(src, true)));
            }
        }
    }
    if !media.links.is_empty() {
        out.push_str("\n\n### Liens :\n");
        for (text, href) in &media.links {
            let label = if text.is_empty() { href } else { text };
            out.push_str(&format!("- [{label}]({href})\n"));
        }
    }
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
    fn test_body_media_collects_images_and_links_once() {
        let body = "[devis](https://a.test/d) et encore [devis](https://a.test/d)\n\
                    ![plan](https://a.test/p.png) ![](inline_x.png)\n\
                    [![logo](inline_l.png)](https://a.test/home) [mail](mailto:a@b.c)";
        let media = body_media(body);
        assert_eq!(
            media.images,
            vec![
                ("plan".to_string(), "https://a.test/p.png".to_string()),
                (String::new(), "inline_x.png".to_string()),
                ("logo".to_string(), "inline_l.png".to_string()),
            ]
        );
        assert_eq!(
            media.links,
            vec![
                ("devis".to_string(), "https://a.test/d".to_string()),
                ("logo".to_string(), "https://a.test/home".to_string()),
            ]
        );
    }

    #[test]
    fn test_body_media_resolves_reference_style() {
        let body = "Voir [le devis][1]\n\n[![logo][2]][3]\n\n![plan][2]\n\n\
                    [1]: https://a.test/devis\n[2]: https://a.test/logo.png\n[3]: https://a.test/home";
        let media = body_media(body);
        assert_eq!(
            media.images,
            vec![("logo".to_string(), "https://a.test/logo.png".to_string())]
        );
        assert_eq!(
            media.links,
            vec![
                ("le devis".to_string(), "https://a.test/devis".to_string()),
                ("logo".to_string(), "https://a.test/home".to_string()),
            ]
        );
    }

    #[test]
    fn test_render_media_is_empty_without_media() {
        assert_eq!(render_media(&body_media("Bonjour,\n\nMerci.")), "");
    }

    #[test]
    fn test_body_media_with_angle_bracket_destination() {
        let media = body_media("![x](<my photo.png>) [![y](<a (2).png>)](https://a.test/h)");
        assert_eq!(
            media.images,
            vec![
                ("x".to_string(), "my photo.png".to_string()),
                ("y".to_string(), "a (2).png".to_string()),
            ]
        );
        assert!(media.images.iter().all(|(_, src)| !src.starts_with('<')));
    }

    #[test]
    fn test_render_media_with_local_image_writes_wikilink() {
        let out = render_media(&body_media(
            "![vue](my photo.png) ![](<my photo.png>) ![logo](https://a.test/l.png)",
        ));
        assert!(out.contains("\n![[my photo.png]]\n"), "{out}");
        assert!(out.contains("\n![logo](https://a.test/l.png)\n"), "{out}");
        assert!(!out.contains("![vue]"), "{out}");
        assert!(!out.contains("![[https"), "{out}");
    }

    #[test]
    fn note_without_block_has_no_header() {
        assert_eq!(read_header("---\nsubject: x\n---\n\nbody"), None);
    }
}
