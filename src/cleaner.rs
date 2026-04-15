use std::collections::BTreeMap;
use std::sync::LazyLock;

use regex::Regex;
use url::Url;

/// Result of running the cleaner pipeline on an email body.
///
/// `body` is the cleaned text. `social_links` holds any social-network
/// links extracted from a trailing footer block, to be merged into the
/// caller's frontmatter. Phase 0 always returns `None`.
pub struct CleanResult {
    pub body: String,
    pub social_links: Option<BTreeMap<String, String>>,
}

/// Run the cleaner pipeline on an email body.
///
/// Pipeline order (final architecture):
/// 1. Decode fidelity: residual quoted-printable, HTML entities, strip
///    invisible chars, mojibake warning.
/// 2. URL reattachment — must run before any line-structure pass so that
///    URLs broken across two physical lines are seen as single tokens by
///    later stages.
/// 3. Social footer extraction — must run before `unwrap_lines` collapses
///    the vertical layout that the footer detector relies on.
/// 4. Unwrap 80-char wrapping into flowing paragraphs.
/// 5. Inline link extraction → numbered references, then tracker
///    decontamination of the generated reference URLs.
/// 6. Whitespace hygiene: collapse runs of horizontal whitespace and
///    strip trailing whitespace per line.
pub fn clean(body: &str) -> CleanResult {
    // 1. Decode fidelity
    let body = decode_residual_qp(body);
    let body = decode_html_entities(&body);
    let body = strip_invisible_chars(&body);
    if detect_mojibake(&body) {
        eprintln!("warning: possible mojibake detected in email body");
    }

    // 2. Social footer extraction.
    //
    //    NOTE: the original phase plan put `reattach_urls` first, but that
    //    interacts badly with stacked single-link social lines whose URL-safe
    //    trailing `)` would otherwise match a dangling-URL tail. Running the
    //    footer detector first (which keys on lines matching
    //    `^\[.+\]\(.+\)$`) keeps the verbatim line layout intact so the four
    //    social links are recognised as a block, and any wrapped URLs *inside*
    //    the surviving body are still repaired in step 3.
    let (body, social_links) = extract_social_footer(&body);

    // 3. URL reattachment. The detector is dangling-URL-aware: it only joins
    //    two lines when the previous line's tail actually looks like a URL
    //    (`https?://…` or `www.…`). Plain prose boundaries are left alone, so
    //    wrapped paragraphs survive intact for `unwrap_lines` to handle next.
    let body = reattach_urls(&body);

    // 4. Unwrap 80-char wrapping
    let body = unwrap_lines(&body);

    // 5. Inline link extraction → numbered refs, then tracker decontamination
    //    of the reference URLs. We post-process the generated reference lines
    //    rather than threading a transform closure through `extract_links`.
    let body = extract_links(&body);
    let body = decontaminate_ref_urls(&body);

    // 6. Whitespace hygiene
    let body = collapse_whitespace(&body);
    let body = trim_trailing(&body);

    CleanResult { body, social_links }
}

/// Collapse runs of horizontal whitespace (ASCII space, tab, U+00A0) into a
/// single ASCII space. Newlines are preserved verbatim and runs are NOT
/// collapsed across line boundaries.
pub fn collapse_whitespace(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    let is_collapsible = |c: char| c == ' ' || c == '\t' || c == '\u{00A0}';

    let mut out = String::with_capacity(s.len());
    let mut prev_was_ws = false;
    for c in s.chars() {
        if c == '\n' {
            out.push('\n');
            prev_was_ws = false;
        } else if is_collapsible(c) {
            if !prev_was_ws {
                out.push(' ');
                prev_was_ws = true;
            }
        } else {
            out.push(c);
            prev_was_ws = false;
        }
    }
    out
}

/// Strip trailing whitespace from each line. The final newline of the input,
/// if present, is preserved.
pub fn trim_trailing(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    let had_trailing_newline = s.ends_with('\n');
    let mut out: Vec<String> = s
        .split('\n')
        .map(|l| l.trim_end().to_string())
        .collect();
    // `split('\n')` on a string ending with '\n' produces a trailing empty
    // slot. Drop it so we can re-append exactly one newline.
    if had_trailing_newline {
        out.pop();
    }
    let mut joined = out.join("\n");
    if had_trailing_newline {
        joined.push('\n');
    }
    joined
}

/// Walk lines and rewrite any numbered-reference line `[N]: <url>` by
/// passing its URL through `decontaminate_trackers`. Non-matching lines
/// are left untouched. A trailing newline in the input is preserved.
fn decontaminate_ref_urls(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    let re = Regex::new(r"^\[\d+\]: (.+)$").expect("static regex");
    let had_trailing_newline = s.ends_with('\n');

    let mut out: Vec<String> = Vec::new();
    let lines: Vec<&str> = s.split('\n').collect();
    let working: &[&str] = if had_trailing_newline {
        &lines[..lines.len() - 1]
    } else {
        &lines[..]
    };

    for line in working {
        if let Some(caps) = re.captures(line) {
            if let Some(url_match) = caps.get(1) {
                let url = url_match.as_str();
                let cleaned = decontaminate_trackers(url);
                // Preserve the `[N]: ` prefix length by reconstructing.
                let prefix_end = url_match.start();
                let prefix = &line[..prefix_end];
                out.push(format!("{}{}", prefix, cleaned));
                continue;
            }
        }
        out.push((*line).to_string());
    }

    let mut joined = out.join("\n");
    if had_trailing_newline {
        joined.push('\n');
    }
    joined
}

/// Defensive fallback for surviving quoted-printable byte sequences.
///
/// When the upstream MIME parser fails to decode a quoted-printable body
/// (unusual `Content-Transfer-Encoding` header, malformed boundary, etc.),
/// the body string still contains literal `=XX` sequences. This function
/// regex-replaces any surviving `=[0-9A-F]{2}` runs by interpreting them
/// as UTF-8 byte sequences.
///
/// Two-byte sequences (`=C2=A0`) are matched first to avoid the single-byte
/// pattern eating their first half. Invalid UTF-8 is left untouched.
pub fn decode_residual_qp(s: &str) -> String {
    // Match 1..=4 consecutive =XX hex bytes (UTF-8 max 4 bytes per codepoint),
    // longest first thanks to the +-greediness inside a single match.
    let re = Regex::new(r"(?:=[0-9A-Fa-f]{2}){1,4}").expect("static regex");
    re.replace_all(s, |caps: &regex::Captures| {
        let m = &caps[0];
        // Each =XX is 3 chars, so byte count = m.len() / 3
        let count = m.len() / 3;
        let mut bytes = Vec::with_capacity(count);
        let mut chars = m.chars();
        while let (Some(_eq), Some(h1), Some(h2)) = (chars.next(), chars.next(), chars.next()) {
            let hex: String = [h1, h2].iter().collect();
            if let Ok(b) = u8::from_str_radix(&hex, 16) {
                bytes.push(b);
            }
        }
        match std::str::from_utf8(&bytes) {
            Ok(decoded) => decoded.to_string(),
            // Not valid UTF-8 — keep the original literal so we don't lose data.
            Err(_) => m.to_string(),
        }
    })
    .into_owned()
}

/// Decode HTML entities (`&amp;`, `&eacute;`, `&#x27;`, `&nbsp;`, …).
///
/// Wraps `html_escape::decode_html_entities` and converts the returned
/// `Cow<str>` into an owned `String`.
pub fn decode_html_entities(s: &str) -> String {
    html_escape::decode_html_entities(s).into_owned()
}

/// Strip invisible / zero-width characters that confuse LLMs and humans.
///
/// Removes:
/// - `U+200B` ZERO WIDTH SPACE
/// - `U+200C` ZERO WIDTH NON-JOINER
/// - `U+200D` ZERO WIDTH JOINER
/// - `U+FEFF` ZERO WIDTH NO-BREAK SPACE / BOM
/// - `U+00AD` SOFT HYPHEN
pub fn strip_invisible_chars(s: &str) -> String {
    s.chars()
        .filter(|c| {
            !matches!(
                *c,
                '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{FEFF}' | '\u{00AD}'
            )
        })
        .collect()
}

/// Heuristic mojibake detection: returns `true` if the input looks like
/// UTF-8 text that was mistakenly decoded as Latin-1 / Windows-1252.
///
/// Matches common French / European mojibake digraphs (`Ã©` for `é`,
/// `Â°` for `°`, …). Conservative: a single isolated `Ã` without a
/// matching second byte is not flagged.
pub fn detect_mojibake(s: &str) -> bool {
    const PATTERNS: &[&str] = &[
        "Ã©", "Ã¨", "Ã ", "Ã¢", "Ã´", "Ã»", "Ãª", "Ã®", "Ã¯", "Ã§", "Ã±", "Ã¼",
        "Â°", "Â«", "Â»", "Â ", "Â£", "Â§",
    ];
    PATTERNS.iter().any(|p| s.contains(p))
}

/// True if `c` is a character that can legitimately appear inside a URL.
///
/// Mirrors the regex class `[A-Za-z0-9._~:/?#\[\]@!$&'()*+,;=%-]` from RFC 3986
/// reserved + unreserved sets, used as a heuristic to detect URLs that were
/// hard-wrapped across two lines by an MTA.
fn is_url_safe_char(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(
            c,
            '.' | '_' | '~' | ':' | '/' | '?' | '#' | '[' | ']'
            | '@' | '!' | '$' | '&' | '\'' | '(' | ')' | '*'
            | '+' | ',' | ';' | '=' | '%' | '-'
        )
}

/// Rejoin URLs that were broken across two lines by 80-char wrapping.
///
/// Dangling-URL-aware: two physical lines are joined into one only when the
/// previous line's tail actually looks like a URL (`https?://…` or `www.…`)
/// AND the next line starts with a URL-safe char and no leading whitespace.
/// This avoids eating newline boundaries in wrapped French prose where both
/// sides happen to be ASCII alphanumerics.
///
/// The entire next line is appended with no separator. If the continuation
/// includes prose after the URL itself (e.g. `"path for details"`), a markdown
/// parser will still stop the URL at the first whitespace, so the recovered
/// URL is correct.
///
/// Trailing newline of the input is preserved.
pub fn reattach_urls(s: &str) -> String {
    // Dangling URL tail: starts from a URL hint (`http://`, `https://` or
    // `www.`), any run of URL-safe chars, anchored to end of line. The char
    // class mirrors `is_url_safe_char` so markdown-link tails ending in `)`
    // still match.
    static DANGLING_URL_RE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"(?:https?://|www\.)[A-Za-z0-9._~:/?#\[\]@!$&'()*+,;=%-]*$",
        )
        .expect("static regex")
    });

    if s.is_empty() {
        return String::new();
    }

    let had_trailing_newline = s.ends_with('\n');

    // Split on '\n' so we keep the same line-breaking semantics as the input.
    let lines: Vec<&str> = s.split('\n').collect();

    // Drop the empty trailing slot produced by a trailing '\n' so it does not
    // get fed into the join logic; we re-append it at the end.
    let working: &[&str] = if had_trailing_newline {
        &lines[..lines.len() - 1]
    } else {
        &lines[..]
    };

    if working.is_empty() {
        return s.to_string();
    }

    let mut out: Vec<String> = Vec::with_capacity(working.len());
    for line in working {
        if let Some(prev) = out.last_mut() {
            let next_first = line.chars().next();
            let next_starts_with_ws = line.starts_with(|c: char| c.is_whitespace());

            let should_join = match next_first {
                Some(n) => {
                    is_url_safe_char(n)
                        && !next_starts_with_ws
                        && DANGLING_URL_RE.is_match(prev)
                }
                None => false,
            };

            if should_join {
                prev.push_str(line);
                continue;
            }
        }
        out.push((*line).to_string());
    }

    let mut joined = out.join("\n");
    if had_trailing_newline {
        joined.push('\n');
    }
    joined
}

/// Set of social-network hosts whose links count as part of a footer block.
///
/// Each entry is the canonical (network, domain) pair. Subdomain matching is
/// performed by `host_matches_domain` so `www.instagram.com` also resolves to
/// `instagram`.
const SOCIAL_DOMAINS: &[(&str, &str)] = &[
    ("instagram", "instagram.com"),
    ("facebook", "facebook.com"),
    ("tiktok", "tiktok.com"),
    ("linkedin", "linkedin.com"),
    ("twitter", "twitter.com"),
    ("x", "x.com"),
    ("youtube", "youtube.com"),
];

/// True if `host` is `domain` or a subdomain of `domain`.
fn host_matches_domain(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{}", domain))
}

/// Return the canonical network name if `url_str` parses to a known social host.
fn classify_social_url(url_str: &str) -> Option<&'static str> {
    let url = Url::parse(url_str).ok()?;
    let host = url.host_str()?;
    for (name, domain) in SOCIAL_DOMAINS {
        if host_matches_domain(host, domain) {
            return Some(name);
        }
    }
    None
}

/// Extract a trailing block of social-network links from the body.
///
/// Scans backwards from the end, skipping blank lines, and collects contiguous
/// lines whose trimmed content is a single markdown link `[text](url)`. If
/// every collected URL parses successfully **and** matches a known social host
/// **and** there are at least two such links, the block (plus the blank lines
/// preceding it) is removed from the body and returned alongside a map of
/// `network -> url`.
///
/// If any condition fails, the body is returned unchanged with `None`.
pub fn extract_social_footer(s: &str) -> (String, Option<BTreeMap<String, String>>) {
    if s.is_empty() {
        return (String::new(), None);
    }

    let link_re = Regex::new(r"^\[(.+)\]\((.+)\)$").expect("static regex");

    let lines: Vec<&str> = s.split('\n').collect();

    // Walk from the end, skipping any trailing blank lines.
    let mut idx = lines.len();
    while idx > 0 && lines[idx - 1].trim().is_empty() {
        idx -= 1;
    }
    let block_end = idx; // exclusive

    // Collect contiguous matching lines moving upwards.
    let mut block_start = block_end;
    let mut collected: Vec<(String, &'static str)> = Vec::new();
    while block_start > 0 {
        let candidate = lines[block_start - 1].trim();
        let Some(caps) = link_re.captures(candidate) else {
            break;
        };
        let Some(url_match) = caps.get(2) else {
            break;
        };
        let url_str = url_match.as_str();
        let Some(network) = classify_social_url(url_str) else {
            // Unknown domain mixed in → abort entirely (all-or-nothing).
            return (s.to_string(), None);
        };
        collected.push((url_str.to_string(), network));
        block_start -= 1;
    }

    if collected.len() < 2 {
        return (s.to_string(), None);
    }

    // Build the map (collected is in reverse order — order does not matter
    // for BTreeMap).
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    for (url, network) in collected {
        map.insert(network.to_string(), url);
    }

    // Strip the block plus any blank lines immediately preceding it.
    let mut new_end = block_start;
    while new_end > 0 && lines[new_end - 1].trim().is_empty() {
        new_end -= 1;
    }

    let new_body = lines[..new_end].join("\n");
    (new_body, Some(map))
}

/// Rewrite inline markdown links `[text](url)` to numbered reference form
/// `[text][N]` and append a `[N]: url` block at the bottom of the body.
///
/// Walks the input in document order, matching `\[([^\]]+)\]\(([^\)]+)\)`.
/// For each match:
/// - If the URL fails to parse via `url::Url::parse`, leave the inline link
///   UNCHANGED, do not increment the counter, and emit a warning to stderr.
/// - Otherwise allocate the next sequence number `N` (starting at 1) and
///   rewrite the match to `[text][N]`. Store `(N, url)` for the trailing
///   reference block.
///
/// After all substitutions, if any references were collected, append a blank
/// line followed by one `[N]: url` line per reference and a single trailing
/// newline. If no references were collected, return the input unchanged.
///
/// Reference-style markdown links (`[text][label]` or `[label]: url`) are not
/// rewritten — the `(...)` shape is required.
pub fn extract_links(s: &str) -> String {
    let re = Regex::new(r"\[([^\]]+)\]\(([^\)]+)\)").expect("static regex");

    let mut counter: usize = 0;
    let mut refs: Vec<(usize, String)> = Vec::new();

    let rewritten = re.replace_all(s, |caps: &regex::Captures| {
        let text = &caps[1];
        let url = &caps[2];
        match Url::parse(url) {
            Ok(_) => {
                counter += 1;
                let n = counter;
                refs.push((n, url.to_string()));
                format!("[{}][{}]", text, n)
            }
            Err(_) => {
                eprintln!("warning: malformed URL in markdown link: {url}");
                caps[0].to_string()
            }
        }
    });

    if refs.is_empty() {
        return rewritten.into_owned();
    }

    let mut out = rewritten.into_owned();
    out.push_str("\n\n");
    for (n, url) in &refs {
        out.push_str(&format!("[{}]: {}\n", n, url));
    }
    out
}

/// Strip tracker noise from a URL string and unwrap known click-tracker
/// redirects. Returns the cleaned URL string. If parsing fails at any step,
/// returns the input unchanged.
///
/// Cleaning rules, in order:
/// 1. Parse the URL via `url::Url::parse`. On failure: return input unchanged.
/// 2. If the host is a known tracker pattern (mailchimp `*.list-manage.com`,
///    sendgrid `*.sendgrid.net`, generic `click.*`, generic `email.*`), look
///    for a query param named `url`, `u`, or `r` whose value parses as an
///    HTTPS URL. If found, recursively clean and return that URL.
/// 3. Otherwise, strip every query param whose key starts with `utm_` and
///    return the rebuilt URL. If the resulting query is empty, the trailing
///    `?` is removed too.
/// 4. The URL fragment is preserved across all steps.
pub fn decontaminate_trackers(url_str: &str) -> String {
    let Ok(url) = Url::parse(url_str) else {
        return url_str.to_string();
    };

    // Step 1: try to unwrap a known tracker redirect
    if let Some(host) = url.host_str() {
        let is_tracker = host.ends_with(".list-manage.com")
            || host == "list-manage.com"
            || host.ends_with(".sendgrid.net")
            || host == "sendgrid.net"
            || host.starts_with("click.")
            || host.starts_with("email.");

        if is_tracker {
            for (key, value) in url.query_pairs() {
                if key == "url" || key == "u" || key == "r" {
                    // query_pairs already URL-decodes the value
                    if let Ok(inner) = Url::parse(&value) {
                        if inner.scheme() == "https" || inner.scheme() == "http" {
                            // Recursively clean the unwrapped URL (handles nested utm)
                            return decontaminate_trackers(inner.as_str());
                        }
                    }
                }
            }
        }
    }

    // Step 2: strip utm_* params
    strip_utm_params(&url)
}

/// Rebuild `url` with every `utm_*` query parameter removed. If the remaining
/// query is empty, the trailing `?` is dropped.
fn strip_utm_params(url: &Url) -> String {
    // Collect non-utm pairs first (clone into owned strings so we drop the
    // borrow on `url` before mutating).
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(k, _)| !k.starts_with("utm_"))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    let mut new_url = url.clone();
    if kept.is_empty() {
        new_url.set_query(None);
    } else {
        // Clear and re-extend to rewrite the query string from the kept pairs.
        new_url.query_pairs_mut().clear().extend_pairs(kept.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    }
    new_url.into()
}

/// Soft length gate for the unwrap heuristic: lines shorter than this are
/// considered "naturally short" (e.g. salutations, two-word labels) and never
/// merged with their successor, even when no terminal punctuation is present.
const UNWRAP_MIN_LEN: usize = 60;

/// Terminal characters that indicate the end of a sentence/clause. A line ending
/// (after trim) with one of these is treated as final and never glued to the
/// next line by the unwrap heuristic.
const UNWRAP_TERMINAL_CHARS: &[char] = &['.', '!', '?', ':', '\u{2026}', ')', '"', '\u{00BB}'];

/// True if `line` is a structural / preserved line that must NOT be joined
/// to its previous or next neighbour by `unwrap_lines`.
///
/// Covers markdown list items, blockquotes, table rows, headings, horizontal
/// rules, and lines whose trimmed content is a single markdown link
/// `[text](url)`. Fenced-code-block tracking and signature-block tracking are
/// stateful and handled directly inside `unwrap_lines`.
fn is_preserved_line(line: &str) -> bool {
    let trimmed = line.trim_start();

    // Markdown unordered list: "- ", "* ", "+ "
    if let Some(rest) = trimmed
        .strip_prefix('-')
        .or_else(|| trimmed.strip_prefix('*'))
        .or_else(|| trimmed.strip_prefix('+'))
    {
        if rest.starts_with(' ') {
            return true;
        }
    }

    // Markdown ordered list: "1. ", "12) ", etc.
    let digit_count = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
    if digit_count > 0 {
        let after_digits = &trimmed[digit_count..];
        if let Some(rest) = after_digits
            .strip_prefix('.')
            .or_else(|| after_digits.strip_prefix(')'))
        {
            if rest.starts_with(' ') {
                return true;
            }
        }
    }

    // Blockquote, table row, heading
    if trimmed.starts_with('>') || trimmed.starts_with('|') || trimmed.starts_with('#') {
        return true;
    }

    // Horizontal rule
    let trimmed_full = line.trim();
    if trimmed_full == "---" || trimmed_full == "***" || trimmed_full == "___" {
        return true;
    }

    // Lone markdown link line: `[text](url)`
    let link_re = Regex::new(r"^\[.+\]\(.+\)$").expect("static regex");
    if link_re.is_match(trimmed_full) {
        return true;
    }

    false
}

/// Reflow 80-char-wrapped prose into flowing paragraphs without breaking
/// structured blocks.
///
/// Walks lines in order. Joins a line to its predecessor with a single space
/// when ALL of the following hold:
/// - previous line (after trim) does NOT end with a sentence terminator
///   (`.`, `!`, `?`, `:`, `…`, `)`, `"`, `»`)
/// - previous line trimmed length ≥ `UNWRAP_MIN_LEN` (soft gate against
///   over-joining naturally short lines like greetings)
/// - neither the previous nor the current line is a preserved structural line
///   (list, quote, table, heading, hr, lone-link, fenced code, signature tail)
/// - current line is not empty
///
/// Otherwise lines are kept on separate lines.
///
/// State machines:
/// - Fenced code blocks: any line whose trimmed content starts with ` ``` `
///   toggles an "inside fence" flag. While inside, every line — including the
///   toggling line itself — is preserved verbatim.
/// - Signature separator: a line whose trimmed content is exactly `--` or
///   `-- ` (RFC 3676, lenient — many clients drop the trailing space) puts
///   the walker into "signature mode" for the rest of the document; every
///   subsequent line is preserved.
///
/// Pure `&str -> String`. A trailing `\n` on the input is preserved in the
/// output; empty input returns an empty string. Input is assumed to already
/// be `\n`-normalised by upstream pipeline stages.
pub fn unwrap_lines(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }

    let had_trailing_newline = s.ends_with('\n');

    let lines: Vec<&str> = s.split('\n').collect();
    let working: &[&str] = if had_trailing_newline {
        &lines[..lines.len() - 1]
    } else {
        &lines[..]
    };

    if working.is_empty() {
        return s.to_string();
    }

    let mut out: Vec<String> = Vec::with_capacity(working.len());
    let mut in_fence = false;
    let mut after_signature = false;

    // Tracks whether the most recently emitted output line was itself the
    // result of a "preserved" classification — used so the next iteration
    // knows not to glue onto it.
    let mut prev_was_preserved = true;

    for line in working {
        let trimmed_full = line.trim();

        // Detect a fence-toggling line BEFORE classifying preservation, since
        // the toggling line is itself preserved.
        let is_fence_toggle = trimmed_full.starts_with("```");

        // Detect signature separator (lenient: `--` or `-- `).
        let is_sig_separator = trimmed_full == "--" || trimmed_full == "-- ";

        // Decide preservation. Empty lines are paragraph separators — preserved
        // (never glued) and do not trigger join logic from the previous line.
        let preserved = in_fence
            || after_signature
            || is_fence_toggle
            || is_sig_separator
            || line.is_empty()
            || is_preserved_line(line);

        // Determine whether this line should be joined onto the previous out line.
        let should_join = !preserved
            && !prev_was_preserved
            && out.last().is_some_and(|prev| {
                let prev_trimmed = prev.trim_end();
                !prev_trimmed.is_empty()
                    && prev_trimmed.chars().count() >= UNWRAP_MIN_LEN
                    && prev_trimmed
                        .chars()
                        .last()
                        .is_some_and(|last_char| !UNWRAP_TERMINAL_CHARS.contains(&last_char))
            });

        if should_join {
            // Safe: should_join only true when out.last() is Some.
            if let Some(prev) = out.last_mut() {
                prev.push(' ');
                prev.push_str(line.trim_start());
            }
        } else {
            out.push((*line).to_string());
        }

        // Update state machines AFTER classification so the toggling/separator
        // line itself is preserved on the current iteration.
        if is_fence_toggle {
            in_fence = !in_fence;
        }
        if is_sig_separator {
            after_signature = true;
        }

        prev_was_preserved = preserved;
    }

    let mut joined = out.join("\n");
    if had_trailing_newline {
        joined.push('\n');
    }
    joined
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- collapse_whitespace ----------

    #[test]
    fn test_collapse_whitespace_runs_of_spaces() {
        assert_eq!(collapse_whitespace("Hello    world"), "Hello world");
    }

    #[test]
    fn test_collapse_whitespace_runs_of_nbsp() {
        assert_eq!(
            collapse_whitespace("Hello\u{00A0}\u{00A0}world"),
            "Hello world"
        );
    }

    #[test]
    fn test_collapse_whitespace_preserves_newlines() {
        assert_eq!(
            collapse_whitespace("Line one\n    indented"),
            "Line one\n indented"
        );
    }

    #[test]
    fn test_collapse_whitespace_no_change() {
        assert_eq!(collapse_whitespace("No change"), "No change");
    }

    #[test]
    fn test_collapse_whitespace_tabs() {
        assert_eq!(collapse_whitespace("a\t\tb"), "a b");
    }

    #[test]
    fn test_collapse_whitespace_mixed() {
        assert_eq!(collapse_whitespace("a \t \u{00A0} b"), "a b");
    }

    #[test]
    fn test_collapse_whitespace_empty() {
        assert_eq!(collapse_whitespace(""), "");
    }

    #[test]
    fn test_collapse_whitespace_newlines_untouched() {
        assert_eq!(collapse_whitespace("a\n\nb"), "a\n\nb");
    }

    // ---------- trim_trailing ----------

    #[test]
    fn test_trim_trailing_strips_per_line() {
        assert_eq!(trim_trailing("foo   \nbar\t\n"), "foo\nbar\n");
    }

    #[test]
    fn test_trim_trailing_no_trailing_newline() {
        assert_eq!(trim_trailing("foo\nbar"), "foo\nbar");
    }

    #[test]
    fn test_trim_trailing_single_line() {
        assert_eq!(trim_trailing("foo   "), "foo");
    }

    #[test]
    fn test_trim_trailing_whitespace_only_lines() {
        assert_eq!(trim_trailing("   \n   \n"), "\n\n");
    }

    #[test]
    fn test_trim_trailing_empty() {
        assert_eq!(trim_trailing(""), "");
    }

    // ---------- decontaminate_ref_urls ----------

    #[test]
    fn test_decontaminate_ref_urls_strips_utm() {
        let input = "[1]: https://example.com/?utm_source=x";
        assert_eq!(
            decontaminate_ref_urls(input),
            "[1]: https://example.com/"
        );
    }

    #[test]
    fn test_decontaminate_ref_urls_non_matching_left_alone() {
        let input = "Just some prose with [a link][1] in it.";
        assert_eq!(decontaminate_ref_urls(input), input);
    }

    #[test]
    fn test_decontaminate_ref_urls_mixed_body() {
        let input = "Body text [a][1] here.\n\n[1]: https://example.com/?utm_source=x&id=42\n";
        let expected = "Body text [a][1] here.\n\n[1]: https://example.com/?id=42\n";
        assert_eq!(decontaminate_ref_urls(input), expected);
    }

    // ---------- decode_residual_qp ----------

    #[test]
    fn test_decode_residual_qp_two_byte_nbsp() {
        // =C2=A0 → U+00A0 (NBSP)
        let input = "Bonjour=C2=A0world";
        let out = decode_residual_qp(input);
        assert_eq!(out, "Bonjour\u{00A0}world");
    }

    #[test]
    fn test_decode_residual_qp_two_byte_e_acute() {
        // =C3=A9 → é (U+00E9)
        let input = "caf=C3=A9";
        let out = decode_residual_qp(input);
        assert_eq!(out, "café");
    }

    #[test]
    fn test_decode_residual_qp_three_byte_em_dash() {
        // =E2=80=94 → — (U+2014 EM DASH)
        let input = "before=E2=80=94after";
        let out = decode_residual_qp(input);
        assert_eq!(out, "before\u{2014}after");
    }

    #[test]
    fn test_decode_residual_qp_no_qp() {
        let input = "already clean text";
        assert_eq!(decode_residual_qp(input), input);
    }

    #[test]
    fn test_decode_residual_qp_empty() {
        assert_eq!(decode_residual_qp(""), "");
    }

    #[test]
    fn test_decode_residual_qp_invalid_utf8_left_intact() {
        // =FF is not a valid standalone UTF-8 byte → keep literal
        let input = "weird=FFbyte";
        let out = decode_residual_qp(input);
        assert_eq!(out, "weird=FFbyte");
    }

    #[test]
    fn test_decode_residual_qp_lowercase_hex() {
        let input = "Bonjour=c2=a0world";
        let out = decode_residual_qp(input);
        assert_eq!(out, "Bonjour\u{00A0}world");
    }

    #[test]
    fn test_decode_residual_qp_mixed_with_text() {
        let input = "L=C3=A9o et Th=C3=A9o";
        let out = decode_residual_qp(input);
        assert_eq!(out, "Léo et Théo");
    }

    // ---------- decode_html_entities ----------

    #[test]
    fn test_decode_html_entities_amp() {
        assert_eq!(decode_html_entities("Tom &amp; Jerry"), "Tom & Jerry");
    }

    #[test]
    fn test_decode_html_entities_eacute() {
        assert_eq!(decode_html_entities("caf&eacute;"), "café");
    }

    #[test]
    fn test_decode_html_entities_numeric_hex() {
        // &#x27; → '
        assert_eq!(decode_html_entities("it&#x27;s"), "it's");
    }

    #[test]
    fn test_decode_html_entities_nbsp() {
        // &nbsp; → U+00A0
        assert_eq!(decode_html_entities("a&nbsp;b"), "a\u{00A0}b");
    }

    #[test]
    fn test_decode_html_entities_empty() {
        assert_eq!(decode_html_entities(""), "");
    }

    #[test]
    fn test_decode_html_entities_already_clean() {
        assert_eq!(decode_html_entities("plain text"), "plain text");
    }

    // ---------- strip_invisible_chars ----------

    #[test]
    fn test_strip_invisible_zero_width_space() {
        let input = "a\u{200B}b";
        assert_eq!(strip_invisible_chars(input), "ab");
    }

    #[test]
    fn test_strip_invisible_zero_width_non_joiner() {
        let input = "a\u{200C}b";
        assert_eq!(strip_invisible_chars(input), "ab");
    }

    #[test]
    fn test_strip_invisible_zero_width_joiner() {
        let input = "a\u{200D}b";
        assert_eq!(strip_invisible_chars(input), "ab");
    }

    #[test]
    fn test_strip_invisible_bom() {
        let input = "\u{FEFF}hello";
        assert_eq!(strip_invisible_chars(input), "hello");
    }

    #[test]
    fn test_strip_invisible_soft_hyphen() {
        let input = "long\u{00AD}word";
        assert_eq!(strip_invisible_chars(input), "longword");
    }

    #[test]
    fn test_strip_invisible_mixed() {
        let input = "vis\u{200B}ible\u{FEFF} text\u{00AD}";
        assert_eq!(strip_invisible_chars(input), "visible text");
    }

    #[test]
    fn test_strip_invisible_empty() {
        assert_eq!(strip_invisible_chars(""), "");
    }

    #[test]
    fn test_strip_invisible_already_clean() {
        assert_eq!(strip_invisible_chars("plain text"), "plain text");
    }

    #[test]
    fn test_strip_invisible_preserves_normal_nbsp() {
        // U+00A0 (regular NBSP) is NOT in the strip list — only U+00AD (soft hyphen)
        let input = "a\u{00A0}b";
        assert_eq!(strip_invisible_chars(input), "a\u{00A0}b");
    }

    // ---------- detect_mojibake ----------

    #[test]
    fn test_detect_mojibake_clean_french() {
        assert!(!detect_mojibake("Café et thé à Paris"));
    }

    #[test]
    fn test_detect_mojibake_e_acute() {
        // "café" mojibaked = "cafÃ©"
        assert!(detect_mojibake("cafÃ©"));
    }

    #[test]
    fn test_detect_mojibake_a_grave() {
        // "à Paris" mojibaked = "Ã  Paris"
        assert!(detect_mojibake("Ã  Paris"));
    }

    #[test]
    fn test_detect_mojibake_degree_sign() {
        // "20°C" mojibaked = "20Â°C"
        assert!(detect_mojibake("20Â°C"));
    }

    #[test]
    fn test_detect_mojibake_french_quotes() {
        // « » mojibaked = Â« Â»
        assert!(detect_mojibake("Â«hello Â»"));
    }

    #[test]
    fn test_detect_mojibake_empty() {
        assert!(!detect_mojibake(""));
    }

    #[test]
    fn test_detect_mojibake_isolated_a_tilde() {
        // A lone Ã without a following accent-byte is not mojibake
        assert!(!detect_mojibake("São Paulo"));
    }

    #[test]
    fn test_detect_mojibake_ascii_only() {
        assert!(!detect_mojibake("Hello world 123"));
    }

    // ---------- reattach_urls ----------

    #[test]
    fn test_reattach_urls_basic_two_line_split() {
        let input = "[text](https://example.com/\npath/to/page)";
        let out = reattach_urls(input);
        assert_eq!(out, "[text](https://example.com/path/to/page)");
    }

    #[test]
    fn test_reattach_urls_three_line_split() {
        let input = "https://example.com/very/\nlong/path/with/\nmany/segments";
        let out = reattach_urls(input);
        assert_eq!(out, "https://example.com/very/long/path/with/many/segments");
    }

    #[test]
    fn test_reattach_urls_empty_input() {
        assert_eq!(reattach_urls(""), "");
    }

    #[test]
    fn test_reattach_urls_single_line() {
        assert_eq!(reattach_urls("hello"), "hello");
    }

    #[test]
    fn test_reattach_urls_preserves_trailing_newline() {
        let input = "[link](https://example.com/\npath)\n";
        let out = reattach_urls(input);
        assert_eq!(out, "[link](https://example.com/path)\n");
    }

    #[test]
    fn test_reattach_urls_no_trailing_newline_preserved() {
        let input = "single line no newline";
        let out = reattach_urls(input);
        assert_eq!(out, "single line no newline");
    }

    #[test]
    fn test_reattach_urls_does_not_join_after_space() {
        // A line ending in a space (non-URL-safe char) is not joined
        let input = "hello \nworld";
        let out = reattach_urls(input);
        assert_eq!(out, "hello \nworld");
    }

    #[test]
    fn test_reattach_urls_does_not_join_when_next_starts_with_whitespace() {
        // Indented continuation should not be joined
        let input = "https://example.com/\n  indented";
        let out = reattach_urls(input);
        assert_eq!(out, "https://example.com/\n  indented");
    }

    #[test]
    fn test_reattach_urls_plain_prose_not_joined() {
        // D1 regression: alphanumeric boundary must NOT be joined. Only a
        // dangling URL on the previous line triggers reattachment.
        assert_eq!(reattach_urls("Hello\nWorld"), "Hello\nWorld");
    }

    #[test]
    fn test_reattach_urls_period_then_capital_not_joined() {
        // D1 regression: sentence boundary with a period must be preserved.
        assert_eq!(reattach_urls("End.\nStart"), "End.\nStart");
    }

    #[test]
    fn test_reattach_urls_does_not_corrupt_wrapped_prose() {
        // D1 regression against the exact failing fragment from JEVEUX_BODY.
        assert_eq!(
            reattach_urls("disponibles dans\nvotre secteur"),
            "disponibles dans\nvotre secteur"
        );
    }

    #[test]
    fn test_reattach_urls_joins_dangling_https_url() {
        // Primary use case: an actual dangling URL still gets stitched back.
        assert_eq!(
            reattach_urls("Click https://example.com/\npath/to/page for details"),
            "Click https://example.com/path/to/page for details"
        );
    }

    #[test]
    fn test_reattach_urls_joins_broken_markdown_link() {
        // Markdown-link URL broken across two lines. Tail ends in `/` which
        // is URL-safe; the regex matches through the `https://` prefix.
        assert_eq!(
            reattach_urls("[text](https://example.com/\npath)"),
            "[text](https://example.com/path)"
        );
    }

    #[test]
    fn test_reattach_urls_joins_www_prefix() {
        // `www.` prefix with no scheme is still recognised as a URL hint.
        assert_eq!(
            reattach_urls("Visit www.example.com/\npath for more"),
            "Visit www.example.com/path for more"
        );
    }

    #[test]
    fn test_reattach_urls_prose_word_not_joined() {
        // False-positive guard: "complete" has no URL scheme on the tail,
        // so the dangling-URL regex rejects it and prose is preserved.
        assert_eq!(
            reattach_urls("Work is complete\nNow we rest"),
            "Work is complete\nNow we rest"
        );
    }

    #[test]
    fn test_reattach_urls_does_not_join_blank_separator() {
        // Empty intermediate line breaks the chain
        let input = "https://example.com/\n\npath";
        let out = reattach_urls(input);
        assert_eq!(out, "https://example.com/\n\npath");
    }

    // ---------- extract_social_footer ----------

    #[test]
    fn test_extract_social_footer_nominal_four_networks() {
        let input = "Body content here.\n\n\
[Instagram](https://www.instagram.com/foo)\n\
[Facebook](https://www.facebook.com/foo)\n\
[TikTok](https://www.tiktok.com/@foo)\n\
[LinkedIn](https://www.linkedin.com/in/foo)";
        let (body, links) = extract_social_footer(input);
        assert_eq!(body, "Body content here.");
        let map = links.expect("expected social_links to be populated");
        assert_eq!(map.len(), 4);
        assert_eq!(map.get("instagram").map(|s| s.as_str()), Some("https://www.instagram.com/foo"));
        assert_eq!(map.get("facebook").map(|s| s.as_str()), Some("https://www.facebook.com/foo"));
        assert_eq!(map.get("tiktok").map(|s| s.as_str()), Some("https://www.tiktok.com/@foo"));
        assert_eq!(map.get("linkedin").map(|s| s.as_str()), Some("https://www.linkedin.com/in/foo"));
    }

    #[test]
    fn test_extract_social_footer_unknown_domain_aborts() {
        let input = "Body.\n\n\
[Instagram](https://www.instagram.com/foo)\n\
[Blog](https://example.com/blog)\n\
[Facebook](https://www.facebook.com/foo)";
        let (body, links) = extract_social_footer(input);
        assert_eq!(body, input);
        assert!(links.is_none());
    }

    #[test]
    fn test_extract_social_footer_no_footer() {
        let input = "Just some plain text body.\nWith two lines.";
        let (body, links) = extract_social_footer(input);
        assert_eq!(body, input);
        assert!(links.is_none());
    }

    #[test]
    fn test_extract_social_footer_single_link_below_threshold() {
        let input = "Body.\n\n[Instagram](https://www.instagram.com/foo)";
        let (body, links) = extract_social_footer(input);
        assert_eq!(body, input);
        assert!(links.is_none());
    }

    #[test]
    fn test_extract_social_footer_strips_trailing_blank_lines() {
        let input = "Body.\n\n\
[Instagram](https://www.instagram.com/foo)\n\
[Facebook](https://www.facebook.com/foo)\n\n\n";
        let (body, links) = extract_social_footer(input);
        assert_eq!(body, "Body.");
        assert!(links.is_some());
    }

    #[test]
    fn test_extract_social_footer_not_at_end_not_extracted() {
        let input = "Body.\n\n\
[Instagram](https://www.instagram.com/foo)\n\
[Facebook](https://www.facebook.com/foo)\n\n\
More text after.";
        let (body, links) = extract_social_footer(input);
        assert_eq!(body, input);
        assert!(links.is_none());
    }

    #[test]
    fn test_extract_social_footer_subdomains_match() {
        let input = "Body.\n\n\
[IG](https://www.instagram.com/foo)\n\
[FB](https://www.facebook.com/foo)";
        let (body, links) = extract_social_footer(input);
        assert_eq!(body, "Body.");
        let map = links.expect("expected social_links");
        assert_eq!(map.get("instagram").map(|s| s.as_str()), Some("https://www.instagram.com/foo"));
        assert_eq!(map.get("facebook").map(|s| s.as_str()), Some("https://www.facebook.com/foo"));
    }

    #[test]
    fn test_extract_social_footer_x_dot_com() {
        let input = "Body.\n\n\
[X](https://x.com/foo)\n\
[YouTube](https://www.youtube.com/foo)";
        let (body, links) = extract_social_footer(input);
        assert_eq!(body, "Body.");
        let map = links.expect("expected social_links");
        assert_eq!(map.get("x").map(|s| s.as_str()), Some("https://x.com/foo"));
        assert_eq!(map.get("youtube").map(|s| s.as_str()), Some("https://www.youtube.com/foo"));
    }

    #[test]
    fn test_extract_social_footer_empty_input() {
        let (body, links) = extract_social_footer("");
        assert_eq!(body, "");
        assert!(links.is_none());
    }

    // ---------- unwrap_lines ----------

    #[test]
    fn test_unwrap_lines_wrapped_paragraph_joins() {
        // First line is 76 chars, no terminal punctuation, well over the
        // 60-char soft length gate — should glue with the wrapped tail.
        let input = "This is a very long paragraph line that was wrapped by an email client\nhere at 80 characters.\n";
        let out = unwrap_lines(input);
        assert_eq!(
            out,
            "This is a very long paragraph line that was wrapped by an email client here at 80 characters.\n"
        );
    }

    #[test]
    fn test_unwrap_lines_short_lines_not_joined() {
        let input = "Hello\nAlice";
        let out = unwrap_lines(input);
        assert_eq!(out, "Hello\nAlice");
    }

    #[test]
    fn test_unwrap_lines_previous_ends_with_period_not_joined() {
        let input = "First sentence.\nSecond sentence.";
        let out = unwrap_lines(input);
        assert_eq!(out, "First sentence.\nSecond sentence.");
    }

    #[test]
    fn test_unwrap_lines_preserves_unordered_list() {
        let input = "Intro paragraph below this list which is long enough to trigger:\n- item one\n- item two\n";
        let out = unwrap_lines(input);
        assert_eq!(
            out,
            "Intro paragraph below this list which is long enough to trigger:\n- item one\n- item two\n"
        );
    }

    #[test]
    fn test_unwrap_lines_preserves_ordered_list() {
        let input = "Steps to follow which is a long enough intro to trigger unwrap:\n1. First step here\n2. Second step here\n";
        let out = unwrap_lines(input);
        assert_eq!(
            out,
            "Steps to follow which is a long enough intro to trigger unwrap:\n1. First step here\n2. Second step here\n"
        );
    }

    #[test]
    fn test_unwrap_lines_preserves_blockquote() {
        let input = "As he said in his famous speech which is long enough to unwrap:\n> I will not go gentle\n> into that good night\n";
        let out = unwrap_lines(input);
        assert_eq!(
            out,
            "As he said in his famous speech which is long enough to unwrap:\n> I will not go gentle\n> into that good night\n"
        );
    }

    #[test]
    fn test_unwrap_lines_preserves_fenced_code_block() {
        let input = "```rust\nfn main() {\n    println!(\"hi\");\n}\n```\n";
        let out = unwrap_lines(input);
        assert_eq!(
            out,
            "```rust\nfn main() {\n    println!(\"hi\");\n}\n```\n"
        );
    }

    #[test]
    fn test_unwrap_lines_preserves_signature_block() {
        let input = "A long paragraph that is definitely longer than sixty characters total\n-- \nJohn Doe\nCTO\nAcme";
        let out = unwrap_lines(input);
        assert_eq!(
            out,
            "A long paragraph that is definitely longer than sixty characters total\n-- \nJohn Doe\nCTO\nAcme"
        );
    }

    #[test]
    fn test_unwrap_lines_preserves_lone_markdown_link() {
        let input = "A paragraph of text here that is long enough to trigger unwrap normally.\n[Click me](https://example.com/path)\nMore text after the button here that is also long enough.\n";
        let out = unwrap_lines(input);
        assert_eq!(
            out,
            "A paragraph of text here that is long enough to trigger unwrap normally.\n[Click me](https://example.com/path)\nMore text after the button here that is also long enough.\n"
        );
    }

    #[test]
    fn test_unwrap_lines_preserves_heading() {
        let input = "Some intro text that is long enough to trigger unwrap normally.\n# Heading\nNext paragraph that is also long enough to trigger unwrap normally.\n";
        let out = unwrap_lines(input);
        assert_eq!(
            out,
            "Some intro text that is long enough to trigger unwrap normally.\n# Heading\nNext paragraph that is also long enough to trigger unwrap normally.\n"
        );
    }

    #[test]
    fn test_unwrap_lines_empty_lines_preserve_paragraph_breaks() {
        // Three paragraphs separated by blank lines. Within paragraph two
        // the wrapped continuation must glue back onto its predecessor while
        // the blank line above it is left untouched.
        let input = "First paragraph line which is definitely long enough to trigger.\n\nSecond paragraph line one that is long enough to trigger unwrap with no period\nand a continuation of second paragraph.\n";
        let out = unwrap_lines(input);
        assert_eq!(
            out,
            "First paragraph line which is definitely long enough to trigger.\n\nSecond paragraph line one that is long enough to trigger unwrap with no period and a continuation of second paragraph.\n"
        );
    }

    #[test]
    fn test_unwrap_lines_trailing_newline_preserved() {
        let input = "Short line\n";
        let out = unwrap_lines(input);
        assert_eq!(out, "Short line\n");
    }

    #[test]
    fn test_unwrap_lines_empty_input() {
        assert_eq!(unwrap_lines(""), "");
    }

    #[test]
    fn test_unwrap_lines_single_line() {
        assert_eq!(unwrap_lines("Just one line"), "Just one line");
    }

    #[test]
    fn test_unwrap_lines_length_gate_below_threshold() {
        // Previous line is exactly 59 chars → not joined.
        let prev = "a".repeat(59);
        assert_eq!(prev.len(), 59);
        let input = format!("{}\nnext line content", prev);
        let out = unwrap_lines(&input);
        assert_eq!(out, format!("{}\nnext line content", prev));
    }

    #[test]
    fn test_unwrap_lines_length_gate_at_threshold() {
        // Previous line is exactly 60 chars → joined.
        let prev = "a".repeat(60);
        assert_eq!(prev.len(), 60);
        let input = format!("{}\nnext line content", prev);
        let out = unwrap_lines(&input);
        assert_eq!(out, format!("{} next line content", prev));
    }

    // ---------- extract_links ----------

    #[test]
    fn test_extract_links_single_inline_link() {
        let input = "Click [here](https://example.com/page) to continue.";
        let out = extract_links(input);
        assert_eq!(
            out,
            "Click [here][1] to continue.\n\n[1]: https://example.com/page\n"
        );
    }

    #[test]
    fn test_extract_links_two_inline_links() {
        let input = "First [a](https://a.example.com/) then [b](https://b.example.com/).";
        let out = extract_links(input);
        assert_eq!(
            out,
            "First [a][1] then [b][2].\n\n[1]: https://a.example.com/\n[2]: https://b.example.com/\n"
        );
    }

    #[test]
    fn test_extract_links_no_links_returns_unchanged() {
        let input = "Plain text with no links at all.";
        let out = extract_links(input);
        assert_eq!(out, input);
    }

    #[test]
    fn test_extract_links_empty_input() {
        assert_eq!(extract_links(""), "");
    }

    #[test]
    fn test_extract_links_malformed_url_left_unchanged() {
        // "not a url" fails Url::parse — counter does not increment.
        let input = "See [docs](not a url) and [real](https://real.example.com/).";
        let out = extract_links(input);
        assert_eq!(
            out,
            "See [docs](not a url) and [real][1].\n\n[1]: https://real.example.com/\n"
        );
    }

    #[test]
    fn test_extract_links_preserves_text_with_spaces() {
        let input = "Read [the full guide](https://example.com/guide) here.";
        let out = extract_links(input);
        assert_eq!(
            out,
            "Read [the full guide][1] here.\n\n[1]: https://example.com/guide\n"
        );
    }

    #[test]
    fn test_extract_links_does_not_match_reference_style() {
        // `[text][label]` is reference-style — the regex requires `(...)` not `[...]`
        let input = "See [docs][label] and the definition [label]: https://example.com/";
        let out = extract_links(input);
        assert_eq!(out, input);
    }

    #[test]
    fn test_extract_links_numbering_in_document_order() {
        let input = "[one](https://1.example.com/)\n[two](https://2.example.com/)\n[three](https://3.example.com/)";
        let out = extract_links(input);
        assert_eq!(
            out,
            "[one][1]\n[two][2]\n[three][3]\n\n[1]: https://1.example.com/\n[2]: https://2.example.com/\n[3]: https://3.example.com/\n"
        );
    }

    // ---------- decontaminate_trackers ----------

    #[test]
    fn test_decontaminate_strips_utm_params() {
        let input = "https://example.com/page?utm_source=newsletter&utm_medium=email&foo=bar";
        let out = decontaminate_trackers(input);
        assert_eq!(out, "https://example.com/page?foo=bar");
    }

    #[test]
    fn test_decontaminate_strips_utm_only_no_trailing_question_mark() {
        let input = "https://example.com/page?utm_source=x";
        let out = decontaminate_trackers(input);
        assert_eq!(out, "https://example.com/page");
    }

    #[test]
    fn test_decontaminate_unwraps_mailchimp() {
        let input = "https://abc.list-manage.com/track/click?u=xyz&id=abc&url=https%3A%2F%2Freal.example.com%2Fpath";
        let out = decontaminate_trackers(input);
        assert_eq!(out, "https://real.example.com/path");
    }

    #[test]
    fn test_decontaminate_unwraps_generic_click_tracker() {
        let input = "https://click.example.com/ls/click?upn=foo&url=https%3A%2F%2Fdestination.com%2F";
        let out = decontaminate_trackers(input);
        assert_eq!(out, "https://destination.com/");
    }

    #[test]
    fn test_decontaminate_unwraps_sendgrid() {
        let input = "https://u123.sendgrid.net/wf/click?url=https%3A%2F%2Ftarget.example.com%2F&upn=foo";
        let out = decontaminate_trackers(input);
        assert_eq!(out, "https://target.example.com/");
    }

    #[test]
    fn test_decontaminate_unwraps_email_subdomain_tracker() {
        let input = "https://email.example.com/click?r=https%3A%2F%2Freal.example.org%2Fpage";
        let out = decontaminate_trackers(input);
        assert_eq!(out, "https://real.example.org/page");
    }

    #[test]
    fn test_decontaminate_malformed_url_unchanged() {
        let input = "not a url";
        let out = decontaminate_trackers(input);
        assert_eq!(out, "not a url");
    }

    #[test]
    fn test_decontaminate_no_query_params_unchanged() {
        let input = "https://example.com/path";
        let out = decontaminate_trackers(input);
        assert_eq!(out, "https://example.com/path");
    }

    #[test]
    fn test_decontaminate_mailchimp_with_nested_utm() {
        // The unwrapped URL also has utm params — both layers should clean.
        let input = "https://abc.list-manage.com/track/click?u=xyz&url=https%3A%2F%2Freal.example.com%2Fpath%3Futm_source%3Dnews%26foo%3Dbar";
        let out = decontaminate_trackers(input);
        assert_eq!(out, "https://real.example.com/path?foo=bar");
    }

    #[test]
    fn test_decontaminate_preserves_fragment() {
        let input = "https://example.com/page?utm_source=x#top";
        let out = decontaminate_trackers(input);
        assert_eq!(out, "https://example.com/page#top");
    }

    #[test]
    fn test_decontaminate_fragment_with_question_mark_left_alone() {
        // The `?` after `#section` is part of the fragment, not a query.
        let input = "https://example.com/page#section?utm_source=x";
        let out = decontaminate_trackers(input);
        assert_eq!(out, "https://example.com/page#section?utm_source=x");
    }

    #[test]
    fn test_decontaminate_keeps_non_utm_params() {
        let input = "https://example.com/?id=42&utm_campaign=spring&q=hello";
        let out = decontaminate_trackers(input);
        // url crate may reorder, but typically preserves insertion order.
        assert_eq!(out, "https://example.com/?id=42&q=hello");
    }

    #[test]
    fn test_extract_social_footer_twitter_canonical() {
        let input = "Body.\n\n\
[Tw](https://twitter.com/foo)\n\
[FB](https://facebook.com/foo)";
        let (_body, links) = extract_social_footer(input);
        let map = links.expect("expected social_links");
        assert_eq!(map.get("twitter").map(|s| s.as_str()), Some("https://twitter.com/foo"));
        assert_eq!(map.get("facebook").map(|s| s.as_str()), Some("https://facebook.com/foo"));
    }
}
