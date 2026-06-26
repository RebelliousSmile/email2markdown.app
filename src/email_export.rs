use crate::config::Account;
use crate::network::{NetworkConfig, ProgressIndicator, with_retry};  // [3][4]
use crate::route::{route_email, Destination, EmailMeta, RouteDecision};
use crate::utils::{
    decode_imap_utf7, decode_mime_filename, extract_emails, get_short_name, hash_md5_prefix,
    is_signature_image, limit_quote_depth, normalize_line_breaks, sanitize_filename, subject_extract,
};
use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset};
use imap::{ImapConnection, Session};
use imap_proto::NameAttribute;
use mailparse::{self, MailHeaderMap, ParsedMail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailFrontmatter {
    pub from: String,
    pub to: String,
    pub date: String,
    pub subject: String,
    pub subject_hash: String,
    pub tags: Vec<String>,
    pub attachments: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub social_links: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone)]
pub struct EmailAnalysis {
    pub email_type: EmailType,
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub contacts: Vec<String>,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum EmailType {
    Direct,
    Group,
    Newsletter,
    MailingList,
    Unknown,
}

impl std::fmt::Display for EmailType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmailType::Direct => write!(f, "direct"),
            EmailType::Group => write!(f, "group"),
            EmailType::Newsletter => write!(f, "newsletter"),
            EmailType::MailingList => write!(f, "mailing_list"),
            EmailType::Unknown => write!(f, "unknown"),
        }
    }
}

pub struct ContactsCollector {
    pub direct: HashSet<String>,
    pub group: HashSet<String>,
    pub newsletter: HashSet<String>,
    pub mailing_list: HashSet<String>,
    pub unknown: HashSet<String>,
}

impl ContactsCollector {
    pub fn new() -> Self {
        ContactsCollector {
            direct: HashSet::new(),
            group: HashSet::new(),
            newsletter: HashSet::new(),
            mailing_list: HashSet::new(),
            unknown: HashSet::new(),
        }
    }

    pub fn add(&mut self, email_type: &EmailType, contact: String) {
        match email_type {
            EmailType::Direct => self.direct.insert(contact),
            EmailType::Group => self.group.insert(contact),
            EmailType::Newsletter => self.newsletter.insert(contact),
            EmailType::MailingList => self.mailing_list.insert(contact),
            EmailType::Unknown => self.unknown.insert(contact),
        };
    }

    pub fn generate_csv(&self, contacts_dir: &Path, account_name: &str) -> Result<PathBuf> {
        let safe_name = account_name.replace(['/', '\\', ':'], "_");
        let filepath = contacts_dir.join(format!("{}.csv", safe_name));

        // UTF-8 BOM required by Thunderbird on Windows for correct encoding detection
        let file = fs::File::create(&filepath)?;
        let mut bom_writer = std::io::BufWriter::new(file);
        std::io::Write::write_all(&mut bom_writer, b"\xEF\xBB\xBF")?;
        let mut writer = csv::Writer::from_writer(bom_writer);
        writer.write_record(["First Name", "Last Name", "Display Name", "Email", "Notes"])?;

        let categories = [
            (&self.direct, "Direct"),
            (&self.group, "Group"),
            (&self.newsletter, "Newsletter"),
            (&self.mailing_list, "Mailing List"),
            (&self.unknown, "Unknown"),
        ];

        for (contacts, contact_type) in categories {
            for contact in contacts {
                let display_name: String = contact
                    .split('@')
                    .next()
                    .unwrap_or("")
                    .replace('.', " ")
                    .split_whitespace()
                    .map(|w| {
                        let mut c = w.chars();
                        match c.next() {
                            None => String::new(),
                            Some(f) => f.to_uppercase().chain(c).collect(),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ");

                let mut parts = display_name.splitn(2, ' ');
                let first_name = parts.next().unwrap_or("").to_string();
                let last_name = parts.next().unwrap_or("").to_string();

                writer.write_record([
                    &first_name,
                    &last_name,
                    &display_name,
                    contact,
                    &format!("{} - {}", account_name, contact_type),
                ])?;
            }
        }

        writer.flush()?;
        Ok(filepath)
    }
}

impl Default for ContactsCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Analyze email type and extract contact information.
pub fn analyze_email_type(mail: &ParsedMail) -> EmailAnalysis {
    let from_field = mail.headers.get_first_value("From").unwrap_or_default();
    let to_field = mail.headers.get_first_value("To").unwrap_or_default();
    let cc_field = mail.headers.get_first_value("Cc").unwrap_or_default();
    let subject = mail.headers.get_first_value("Subject").unwrap_or_default();

    let from_emails = extract_emails(Some(&from_field));
    let to_emails = extract_emails(Some(&to_field));
    let cc_emails = extract_emails(Some(&cc_field));

    // Determine email type
    let email_type = if to_emails.len() > 1 || cc_emails.len() > 1 {
        EmailType::Group
    } else if subject.to_lowercase().contains("newsletter")
        || subject.to_lowercase().contains("bulletin")
        || subject.to_lowercase().contains("digest")
    {
        EmailType::Newsletter
    } else if mail.headers.get_first_value("List-Id").is_some()
        || mail.headers.get_first_value("List-Unsubscribe").is_some()
    {
        EmailType::MailingList
    } else if from_emails.len() == 1 && to_emails.len() == 1 {
        EmailType::Direct
    } else {
        EmailType::Unknown
    };

    // Collect all unique contacts
    let mut contacts: HashSet<String> = HashSet::new();
    for email in from_emails.iter().chain(to_emails.iter()).chain(cc_emails.iter()) {
        if !email.is_empty() {
            contacts.insert(email.clone());
        }
    }

    EmailAnalysis {
        email_type,
        from: from_emails.first().cloned().unwrap_or_default(),
        to: to_emails,
        cc: cc_emails,
        contacts: contacts.into_iter().collect(),
        subject,
    }
}

/// Check if an email has already been exported.
pub fn email_already_exported(
    date_str: &str,
    sender_short: &str,
    recipient_short: &str,
    subject_hash: &str,
    export_directory: &Path,
) -> bool {
    if !export_directory.exists() {
        return false;
    }

    let search_pattern = format!("email_{}_{}*to_{}*.md", date_str, sender_short, recipient_short);

    for entry in WalkDir::new(export_directory)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
    {
        let filename = entry.file_name().to_string_lossy();
        if glob::Pattern::new(&search_pattern)
            .map(|p| p.matches(filename.as_ref()))
            .unwrap_or(false)
        {
            if let Ok(content) = fs::read_to_string(entry.path()) {
                if content.contains(subject_hash) {
                    return true;
                }
            }
        }
    }

    false
}

/// Check if an email should be skipped based on its raw headers alone.
/// Also returns the email analysis so callers can collect contacts without re-parsing.
fn should_skip_from_headers(
    raw_headers: &[u8],
    export_dir: &Path,
) -> (bool, Option<EmailAnalysis>) {
    if raw_headers.is_empty() {
        return (false, None);
    }
    let mail = match mailparse::parse_mail(raw_headers) {
        Ok(m) => m,
        Err(_) => return (false, None),
    };
    let from_field = mail.headers.get_first_value("From").unwrap_or_default();
    let to_field = mail.headers.get_first_value("To").unwrap_or_default();
    let date_field = mail.headers.get_first_value("Date").unwrap_or_default();
    let subject = mail.headers.get_first_value("Subject").unwrap_or_default();

    let date_obj = parse_email_date(&date_field);
    let date_str = date_obj
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "unknown-date".to_string());
    let sender_short = get_short_name(Some(&from_field));
    let recipient_short = get_short_name(Some(&to_field));
    let subject_hash = if !subject.is_empty() {
        hash_md5_prefix(&subject, 6)
    } else {
        "no-subject".to_string()
    };

    let skip = email_already_exported(&date_str, &sender_short, &recipient_short, &subject_hash, export_dir);
    let analysis = analyze_email_type(&mail);
    (skip, Some(analysis))
}

/// Parse email date string to DateTime.
fn parse_email_date(date_str: &str) -> Option<DateTime<FixedOffset>> {
    mailparse::dateparse(date_str)
        .ok()
        .and_then(|ts| DateTime::from_timestamp(ts, 0))
        .map(|dt| dt.fixed_offset())
}

/// Session-level context shared by all `export_to_markdown` calls within a folder.
///
/// Groups the parameters that are constant across messages in the same export run,
/// reducing the argument count of `export_to_markdown` to four.
pub struct ExportContext<'a> {
    /// Target directory for this folder's exports.
    pub export_directory: &'a Path,
    /// Account-level base directory (used for duplicate detection).
    pub base_export_directory: &'a Path,
    /// Account configuration (name, flags, etc.).
    pub account: &'a Account,
    /// Emit extra diagnostic output when `true`.
    pub debug_mode: bool,
    /// Routing destinations parsed from `destinations.txt`.
    pub dests: &'a [Destination],
}

/// Export a single email to Markdown with frontmatter.
///
/// Returns `Ok(Some((filepath, decision)))` when the email was written, where
/// `decision` is the routing proposal (not yet applied — the `.md` stays in staging).
/// Returns `Ok(None)` when the email was skipped (already exported or filtered).
pub fn export_to_markdown(
    raw_email: &[u8],
    tags: Vec<String>,
    contacts_collector: Option<&mut ContactsCollector>,
    ctx: &mut ExportContext<'_>,
) -> Result<Option<(PathBuf, RouteDecision)>> {
    let export_directory = ctx.export_directory;
    let account = ctx.account;
    let debug_mode = ctx.debug_mode;
    let dests = ctx.dests;
    let mail = mailparse::parse_mail(raw_email)
        .context("Failed to parse email")?;

    let from_field = mail.headers.get_first_value("From").unwrap_or_default();
    let to_field = mail.headers.get_first_value("To").unwrap_or_default();
    let date_field = mail.headers.get_first_value("Date").unwrap_or_default();
    let subject = mail.headers.get_first_value("Subject").unwrap_or_default();

    // Parse date
    let date_obj = parse_email_date(&date_field);
    let date_str = date_obj
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "unknown-date".to_string());

    let sender_short = get_short_name(Some(&from_field));
    let recipient_short = get_short_name(Some(&to_field));

    // Generate subject hash for uniqueness
    let subject_hash = if !subject.is_empty() {
        hash_md5_prefix(&subject, 6)
    } else {
        "no-subject".to_string()
    };

    // Check if email already exported
    if account.skip_existing
        && email_already_exported(&date_str, &sender_short, &recipient_short, &subject_hash, export_directory)
    {
        return Ok(None); // skipped — no (PathBuf, RouteDecision) to return
    }

    // Analyze email type and collect contacts if enabled
    let analysis = analyze_email_type(&mail);
    let email_type_str = analysis.email_type.to_string();
    if let Some(collector) = contacts_collector {
        for contact in analysis.contacts {
            collector.add(&analysis.email_type, contact);
        }
    }

    // Create export directory if needed
    fs::create_dir_all(export_directory)?;

    // Generate unique filename
    let extract = subject_extract(&subject);
    let base_filename = if extract.is_empty() {
        format!("email_{}_{}*to_{}", date_str, sender_short, recipient_short)
    } else {
        format!("email_{}_{}_{}*to_{}", date_str, sender_short, extract, recipient_short)
    };
    let mut counter = 1;
    let mut filename = format!("{}.md", base_filename.replace('*', "_"));
    while export_directory.join(&filename).exists() {
        counter += 1;
        filename = format!("{}_{}.md", base_filename.replace('*', "_"), counter);
    }

    // Extract body
    let body = extract_body(&mail);

    // Apply quote depth limiting
    let body = if account.quote_depth > 0 {
        limit_quote_depth(&body, account.quote_depth)
    } else {
        body
    };

    // Handle attachments — written into the same directory as the .md file,
    // named `<date>_<original-name>` for readability.
    let mut attachments = Vec::new();

    extract_attachments(
        &mail,
        export_directory,
        &date_str,
        account.skip_signature_images,
        debug_mode,
        &mut attachments,
    )?;

    // Normalize body and add attachments list
    let body = normalize_line_breaks(&body);
    let cleaned = crate::cleaner::clean(&body);
    let mut normalized_body = cleaned.body;
    let social_links = cleaned.social_links;

    // Create frontmatter
    let frontmatter = EmailFrontmatter {
        from: from_field,
        to: to_field,
        date: date_obj
            .map(|d| d.to_rfc3339())
            .unwrap_or_else(|| date_field.clone()),
        subject: subject.clone(),
        subject_hash,
        tags,
        attachments: attachments.clone(),
        email_type: Some(email_type_str),
        social_links,
    };

    if !attachments.is_empty() {
        normalized_body.push_str("\n\n### Pieces jointes :\n");
        for attachment in &attachments {
            let filename_only = Path::new(attachment)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            normalized_body.push_str(&format!("- [{}]({})\n", filename_only, attachment));
        }
    }

    // Write file
    let filepath = export_directory.join(&filename);
    let mut file = File::create(&filepath)?;

    writeln!(file, "---")?;
    let yaml = serde_yaml::to_string(&frontmatter)?;
    write!(file, "{}", yaml)?;
    writeln!(file, "---\n")?;
    write!(file, "{}", normalized_body)?;

    // Route the email — extract domain from the From address for matching.
    // Uses the first email address found; falls back to empty string on parse failure.
    let email_addresses = extract_emails(Some(&frontmatter.from));
    let sender_addr = email_addresses.first().map(|s| s.as_str()).unwrap_or("");
    let domain = sender_addr
        .rfind('@')
        .map(|i| sender_addr[i + 1..].to_string())
        .unwrap_or_default();

    let meta = EmailMeta {
        from: sender_addr.to_string(),
        domain,
        subject: frontmatter.subject.clone(),
        account: account.name.clone(),
        // date_obj is Option<DateTime<FixedOffset>>; use epoch on None
        date: date_obj.unwrap_or_else(|| {
            chrono::DateTime::from_timestamp(0, 0)
                .expect("epoch is valid")
                .fixed_offset()
        }),
    };

    let decision = route_email(&meta, dests);

    Ok(Some((filepath, decision)))
}

/// Convert HTML to Markdown using htmd. Returns empty string on failure.
fn html_to_markdown(html: &str) -> String {
    htmd::convert(html).unwrap_or_default()
}

/// Stats returned by `fix_html_bodies`.
pub struct FixHtmlStats {
    pub fixed: usize,
    pub skipped: usize,
    pub errors: usize,
}

/// Walk `directory` recursively, find `.md` files whose body is raw HTML,
/// and convert them to Markdown in place.
///
/// When `dry_run` is true the files are detected but not modified.
pub fn fix_html_bodies(
    directory: &Path,
    dry_run: bool,
    on_progress: Option<&(dyn Fn(usize, usize, &str) + Send + Sync)>,
) -> anyhow::Result<FixHtmlStats> {
    let mut stats = FixHtmlStats { fixed: 0, skipped: 0, errors: 0 };

    let files: Vec<std::path::PathBuf> = WalkDir::new(directory)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map_or(false, |ext| ext == "md"))
        .map(|e| e.path().to_path_buf())
        .collect();

    let total = files.len();

    for (i, path) in files.iter().enumerate() {
        if let Some(cb) = on_progress {
            cb(i + 1, total, "Fix HTML");
        }
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => { stats.errors += 1; continue; }
        };

        // Find body: content after the second "---" separator line
        let body = extract_md_body(&content);
        let trimmed = body.trim_start();
        if !trimmed.starts_with("<!DOCTYPE html") && !trimmed.starts_with("<html") && !trimmed.starts_with("<HTML") {
            stats.skipped += 1;
            continue;
        }

        let converted = html_to_markdown(trimmed);
        if converted.trim().is_empty() {
            stats.errors += 1;
            continue;
        }

        if !dry_run {
            let front = &content[..content.len() - body.len()];
            let new_content = format!("{}{}", front, converted);
            if let Err(_) = fs::write(path, &new_content) {
                stats.errors += 1;
                continue;
            }
        }

        stats.fixed += 1;
    }

    Ok(stats)
}

/// Extract the body portion of a `.md` file (content after the closing `---`).
pub(crate) fn extract_md_body(content: &str) -> &str {
    // Skip the opening ---
    let after_open = content.strip_prefix("---\n").or_else(|| content.strip_prefix("---\r\n")).unwrap_or(content);
    // Find the closing ---
    if let Some(idx) = after_open.find("\n---\n") {
        &after_open[idx + 5..]
    } else if let Some(idx) = after_open.find("\n---\r\n") {
        &after_open[idx + 6..]
    } else {
        content
    }
}

/// Extract the body from a parsed email.
pub(crate) fn extract_body(mail: &ParsedMail) -> String {
    if mail.subparts.is_empty() {
        // Not multipart
        mail.get_body().unwrap_or_default()
    } else {
        // Multipart - look for text/plain or text/html
        let mut body = String::new();

        for part in &mail.subparts {
            let content_type = part
                .headers
                .get_first_value("Content-Type")
                .unwrap_or_default()
                .to_lowercase();

            if content_type.starts_with("text/plain") {
                body = part.get_body().unwrap_or_default();
                break;
            } else if content_type.starts_with("text/html") && body.is_empty() {
                let raw_html = part.get_body().unwrap_or_default();
                body = html_to_markdown(&raw_html);
            } else if content_type.starts_with("multipart/") {
                // Recurse into nested multipart
                let nested_body = extract_body(part);
                if !nested_body.is_empty() && body.is_empty() {
                    body = nested_body;
                }
            }
        }

        body
    }
}

/// Extract attachments from a parsed email.
///
/// Each attachment is written into `attachments_dir` (the same directory as the `.md` file)
/// using the flat filename scheme `<stem>__<hash>_<safe_name>`. When two attachments share the
/// same original name (identical hash), a numeric suffix `_2`, `_3`, … is appended to avoid
/// clobbering. The bare filename (no directory prefix) is pushed into `attachments`.
fn extract_attachments(
    mail: &ParsedMail,
    attachments_dir: &Path,
    name_prefix: &str,
    skip_signature_images: bool,
    debug_mode: bool,
    attachments: &mut Vec<String>,
) -> Result<()> {
    for part in &mail.subparts {
        let content_disposition = part
            .headers
            .get_first_value("Content-Disposition")
            .unwrap_or_default();

        if content_disposition.is_empty() && part.subparts.is_empty() {
            continue;
        }

        // Check if this is an attachment
        let has_attachment_disposition = content_disposition.to_lowercase().contains("attachment")
            || content_disposition.to_lowercase().contains("inline");

        if let Some(filename) = extract_attachment_filename(part) {
            let decoded_filename = decode_mime_filename(&filename);

            if has_attachment_disposition || !filename.is_empty() {
                let content_type = part
                    .headers
                    .get_first_value("Content-Type")
                    .unwrap_or_default();

                let payload = part.get_body_raw().unwrap_or_default();

                // Check if this is a signature image that should be skipped
                if skip_signature_images
                    && is_signature_image(
                        Some(&decoded_filename),
                        &content_type,
                        payload.len(),
                        Some(&content_disposition),
                    )
                {
                    if debug_mode {
                        println!(
                            "    Skipping signature image: '{}' ({} bytes)",
                            decoded_filename,
                            payload.len()
                        );
                    }
                    continue;
                }

                if !payload.is_empty() {
                    let safe_filename = sanitize_filename(&decoded_filename);
                    // Flat naming scheme: <date>_<original-name> — readable, with the
                    // email date as prefix. Collisions are disambiguated below; a second
                    // safety pass in `route::move_email` handles cross-email collisions
                    // when several emails are routed into the same destination folder.
                    let base_full_filename = format!("{}_{}", name_prefix, safe_filename);

                    // Numeric suffix loop on real path collision — suffix inserted before extension
                    // so `invoice.pdf` → `invoice_2.pdf`, not `invoice.pdf_2`.
                    let mut full_filename = base_full_filename.clone();
                    let mut suffix = 2u32;
                    while attachments_dir.join(&full_filename).exists() {
                        full_filename = match base_full_filename.rsplit_once('.') {
                            Some((stem, ext)) => format!("{}_{}.{}", stem, suffix, ext),
                            None => format!("{}_{}", base_full_filename, suffix),
                        };
                        suffix += 1;
                    }

                    let filepath = attachments_dir.join(&full_filename);
                    fs::write(&filepath, &payload)?;

                    // Store bare filename — same-folder relative link; normalize \ → / at write time
                    let bare_link = full_filename.replace('\\', "/");
                    attachments.push(bare_link);
                } else if debug_mode {
                    println!(
                        "    Skipping attachment '{}' with empty payload",
                        decoded_filename
                    );
                }
            }
        }

        // Recurse into nested parts
        if !part.subparts.is_empty() {
            extract_attachments(
                part,
                attachments_dir,
                name_prefix,
                skip_signature_images,
                debug_mode,
                attachments,
            )?;
        }
    }

    Ok(())
}

/// Extract filename from an attachment part.
fn extract_attachment_filename(part: &ParsedMail) -> Option<String> {
    // Try Content-Disposition header first
    if let Some(disposition) = part.headers.get_first_value("Content-Disposition") {
        if let Some(filename) = extract_filename_from_header(&disposition) {
            return Some(filename);
        }
    }

    // Try Content-Type header
    if let Some(content_type) = part.headers.get_first_value("Content-Type") {
        if let Some(filename) = extract_filename_from_header(&content_type) {
            return Some(filename);
        }
    }

    None
}

static FILENAME_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r#"filename[*]?=(?:"([^"]+)"|([^;\s]+))"#).expect("static regex")
});

static NAME_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r#"name[*]?=(?:"([^"]+)"|([^;\s]+))"#).expect("static regex")
});

/// Extract filename parameter from a header value.
pub(crate) fn extract_filename_from_header(header: &str) -> Option<String> {
    if let Some(caps) = FILENAME_RE.captures(header) {
        return caps
            .get(1)
            .or_else(|| caps.get(2))
            .map(|m| m.as_str().to_string());
    }

    if let Some(caps) = NAME_RE.captures(header) {
        return caps
            .get(1)
            .or_else(|| caps.get(2))
            .map(|m| m.as_str().to_string());
    }

    None
}

fn is_gmail_server(host: &str) -> bool {
    let lower = host.to_lowercase();
    lower.contains("gmail.com") || lower.contains("googlemail.com")
}

/// IMAP client for exporting emails.
pub struct ImapExporter {
    session: Option<Session<Box<dyn ImapConnection>>>,
    account: Account,
    debug_mode: bool,
    network_config: NetworkConfig,  // [4][5]
    is_gmail: bool,
}

impl ImapExporter {
    pub fn new(account: Account, debug_mode: bool) -> Self {
        ImapExporter {
            session: None,
            account,
            debug_mode,
            network_config: NetworkConfig::default(),  // [4][5]
            is_gmail: false,
        }
    }

    /// [5] Set custom network configuration
    pub fn with_network_config(mut self, config: NetworkConfig) -> Self {
        self.network_config = config;
        self
    }

    /// Connect to the IMAP server.
    pub fn connect(&mut self) -> Result<()> {
        let password = self
            .account
            .password
            .as_ref()
            .context("No password found")?;

        if self.debug_mode {
            println!(
                "Connecting to {}:{}...",
                self.account.server, self.account.port
            );
        }

        let client = imap::ClientBuilder::new(&self.account.server, self.account.port)
            .connect()
            .context("connect to imap server")?;

        self.is_gmail = is_gmail_server(&self.account.server);

        if self.debug_mode {
            println!("Authenticating as {}...", self.account.username);
        }

        let session = match client.login(&self.account.username, password) {
            Ok(s) => s,
            Err((login_err, client)) => {
                if self.debug_mode {
                    println!("LOGIN failed ({}), trying AUTHENTICATE PLAIN...", login_err);
                }
                struct PlainAuth {
                    username: String,
                    password: String,
                }
                impl imap::Authenticator for PlainAuth {
                    type Response = Vec<u8>;
                    fn process(&self, _challenge: &[u8]) -> Self::Response {
                        let mut r = vec![0u8];
                        r.extend_from_slice(self.username.as_bytes());
                        r.push(0u8);
                        r.extend_from_slice(self.password.as_bytes());
                        r
                    }
                }
                let mut auth = PlainAuth {
                    username: self.account.username.clone(),
                    password: password.to_string(),
                };
                client.authenticate("PLAIN", &mut auth).map_err(|(e, _)| {
                    anyhow::anyhow!("Authentication failed (LOGIN: {login_err} / PLAIN: {e})")
                })?
            }
        };

        if self.debug_mode {
            println!("Connected successfully!");
        }

        self.session = Some(session);
        Ok(())
    }

    fn expunge_gmail_all_mail(&mut self) -> Result<()> {
        let raw_name = self.find_gmail_all_mail_folder()?;
        let session = self.session.as_mut().context("Not connected")?;
        session
            .select(&raw_name)
            .with_context(|| format!("select {}", raw_name))?;
        session
            .expunge()
            .with_context(|| format!("expunge {}", raw_name))?;
        Ok(())
    }

    /// Find the Gmail "All Mail" mailbox by SPECIAL-USE `\All` flag (RFC 6154),
    /// falling back to known localized names for servers that omit SPECIAL-USE.
    fn find_gmail_all_mail_folder(&mut self) -> Result<String> {
        let session = self.session.as_mut().context("Not connected")?;
        let folders = session
            .list(None, Some("*"))
            .context("list folders for \\All discovery")?;

        if let Some(f) = folders
            .iter()
            .find(|f| f.attributes().contains(&NameAttribute::All))
        {
            return Ok(f.name().to_string());
        }

        const KNOWN: &[&str] = &[
            "[Gmail]/All Mail",
            "[Gmail]/Tous les messages",
            "[Google Mail]/All Mail",
        ];
        for candidate in KNOWN {
            if folders.iter().any(|f| f.name() == *candidate) {
                return Ok((*candidate).to_string());
            }
        }

        anyhow::bail!(
            "Gmail \\All mailbox not found (SPECIAL-USE missing and no known name match)"
        )
    }

    /// List all folders.
    ///
    /// Returns the raw IMAP name (modified UTF-7, used for `SELECT`) and a decoded
    /// display name (used for local paths, `ignored_folders` matching and logging).
    /// Folders with the `\Noselect` attribute (e.g. Gmail's `[Gmail]` parent) are
    /// filtered out because they cannot be opened with `SELECT`.
    pub fn list_folders(&mut self) -> Result<Vec<FolderName>> {
        let session = self.session.as_mut().context("Not connected")?;

        if self.debug_mode {
            println!("Listing folders...");
        }

        let folders = session.list(None, Some("*"))?;
        let folder_names: Vec<FolderName> = folders
            .iter()
            .filter(|f| {
                let attrs = f.attributes();
                !attrs.contains(&NameAttribute::NoSelect)
                    && !attrs.contains(&NameAttribute::Junk)
                    && !attrs.contains(&NameAttribute::Trash)
                    && !attrs.contains(&NameAttribute::Drafts)
                    && !attrs.contains(&NameAttribute::All)
                    && !attrs.contains(&NameAttribute::Flagged)
                    && !attrs.iter().any(|a| {
                        matches!(a, NameAttribute::Extension(s) if s.eq_ignore_ascii_case("Important"))
                    })
                    // Gmail does not always declare \Important via SPECIAL-USE — filter by known names
                    && !matches!(f.name(), "[Gmail]/Important" | "[Google Mail]/Important")
            })
            .map(|f| {
                let raw = f.name().to_string();
                let display = decode_imap_utf7(f.name());
                FolderName { raw, display }
            })
            .collect();

        if self.debug_mode {
            println!("Found {} folders", folder_names.len());
        }

        Ok(folder_names)
    }

    /// Export a single folder.
    ///
    /// Returns `(stats, decisions)` where `decisions` is the list of route proposals
    /// for every email written during this folder's export. The caller accumulates
    /// these and applies them after all folders are processed.
    pub fn export_folder(
        &mut self,
        folder: &FolderName,
        mut contacts_collector: Option<&mut ContactsCollector>,
        cancel_token: Option<&AtomicBool>,
        dests: &[Destination],
    ) -> Result<(ExportStats, Vec<(PathBuf, RouteDecision)>)> {
        let base_export_directory = PathBuf::from(&self.account.export_directory);
        let export_directory = base_export_directory.join(folder.display.replace('.', "/"));

        // Session borrow is scoped to a block so it ends before the gmail expunge dispatch,
        // which needs to re-borrow self.session via expunge_gmail_all_mail().
        let stats_and_decisions = {
            let session = self.session.as_mut().context("Not connected")?;

            // Select folder using the raw IMAP name (modified UTF-7)
            let mailbox = session.select(&folder.raw)?;
            let message_count = mailbox.exists as usize;

            if self.debug_mode {
                println!("  {} messages in folder", message_count);
            }

            // Search for all messages
            let uids = session.search("ALL")?;
            let uids_vec: Vec<_> = uids.into_iter().collect();

            // Pre-filter: batch fetch headers, skip already-exported without downloading body
            let (filtered_uids, pre_skipped, already_exported_uids) = if self.account.skip_existing && !uids_vec.is_empty() {
                let seq_set = uids_vec.iter().map(|u| u.to_string()).collect::<Vec<_>>().join(",");
                match session.fetch(&seq_set, "RFC822.HEADER") {
                    Ok(headers) => {
                        let mut skip_set = HashSet::new();
                        for message in headers.iter() {
                            if cancel_token.map_or(false, |t| t.load(Ordering::Relaxed)) {
                                break;
                            }
                            let (skip, analysis) = should_skip_from_headers(
                                message.header().unwrap_or(&[]),
                                &export_directory,
                            );
                            if skip {
                                skip_set.insert(message.message);
                                // Collect contacts from skipped emails too
                                if let (Some(collector), Some(a)) = (contacts_collector.as_deref_mut(), analysis) {
                                    for contact in a.contacts {
                                        collector.add(&a.email_type, contact);
                                    }
                                }
                            }
                        }
                        let skipped = skip_set.len();
                        let already_exported: Vec<u32> = skip_set.iter().copied().collect();
                        let filtered = uids_vec
                            .iter()
                            .filter(|u| !skip_set.contains(u))
                            .copied()
                            .collect::<Vec<_>>();
                        (filtered, skipped, already_exported)
                    }
                    Err(e) => {
                        if self.debug_mode {
                            eprintln!("  Header pre-fetch failed, falling back to full fetch: {:#}", e);
                        }
                        (uids_vec, 0, vec![])
                    }
                }
            } else {
                (uids_vec, 0, vec![])
            };

            // [3] Progress indicator
            let total_to_process = filtered_uids.len();
            let mut progress = ProgressIndicator::new(&folder.display, total_to_process);
            let mut stats = ExportStats::default();
            let mut folder_decisions: Vec<(PathBuf, RouteDecision)> = Vec::new();
            stats.skipped += pre_skipped;

            for (_idx, uid) in filtered_uids.into_iter().enumerate() {
                if cancel_token.map_or(false, |t| t.load(Ordering::Relaxed)) {
                    break;
                }

                // [4] Retry logic for fetch
                let fetch_result = with_retry(&self.network_config, "fetch", || {
                    session.fetch(uid.to_string(), "RFC822")
                });

                let messages = match fetch_result {
                    Ok(m) => m,
                    Err(e) => {
                        if self.debug_mode {
                            println!("  Failed to fetch message {}: {:#}", uid, e);
                        }
                        stats.errors += 1;
                        progress.inc();
                        continue;
                    }
                };

                for message in messages.iter() {
                    if let Some(body) = message.body() {
                        let mut ctx = ExportContext {
                            export_directory: &export_directory,
                            base_export_directory: &base_export_directory,
                            account: &self.account,
                            debug_mode: self.debug_mode,
                            dests,
                        };
                        let result = export_to_markdown(
                            body,
                            vec![folder.display.clone()],
                            contacts_collector.as_deref_mut(),
                            &mut ctx,
                        );

                        match result {
                            Ok(Some((path, decision))) => {
                                stats.exported += 1;
                                folder_decisions.push((path, decision));
                            }
                            Ok(None) => stats.skipped += 1,
                            Err(e) => {
                                // Malformed messages (RFC-invalid MIME, broken headers, etc.)
                                // are counted as skipped rather than errored: they cannot be
                                // exported by design and should not contribute to the error
                                // count that signals transient/recoverable failures.
                                let is_malformed =
                                    e.downcast_ref::<mailparse::MailParseError>().is_some();
                                if self.debug_mode {
                                    let label = if is_malformed {
                                        "Skipping malformed message"
                                    } else {
                                        "Error exporting message"
                                    };
                                    println!("  {} {}: {:#}", label, uid, e);
                                    let dump_dir = base_export_directory.join("_failed");
                                    if fs::create_dir_all(&dump_dir).is_ok() {
                                        let dump_path = dump_dir.join(format!(
                                            "{}_uid_{}.eml",
                                            sanitize_filename(&folder.display),
                                            uid
                                        ));
                                        let _ = fs::write(&dump_path, body);
                                        println!("  Raw message dumped to {}", dump_path.display());
                                    }
                                }
                                if is_malformed {
                                    stats.skipped += 1;
                                } else {
                                    stats.errors += 1;
                                }
                            }
                        }
                    }
                }

                // Delete after export if requested.
                // IMAP flag is set here (server-side); local `.md` files remain in staging
                // until route decisions are applied in the caller — the deferred move (D6)
                // ensures routing always precedes any local file removal.
                if self.account.delete_after_export {
                    session.store(uid.to_string(), "+FLAGS (\\Deleted)")?;
                }

                // [3] Update progress
                progress.inc();
            }

            // Mark already-exported (skipped) messages for deletion too.
            // They were safely archived in a previous run; with delete_after_export
            // the intent is to clean up the server, not just newly exported messages.
            if self.account.delete_after_export && !already_exported_uids.is_empty() {
                let seq_set = already_exported_uids
                    .iter()
                    .map(|u| u.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                session.store(&seq_set, "+FLAGS (\\Deleted)")?;
            }

            // [3] Finish progress indicator
            progress.finish_with_message(&format!(
                "{} exported, {} skipped, {} errors",
                stats.exported, stats.skipped, stats.errors
            ));

            (stats, folder_decisions)
            // session borrow ends here
        };

        let (stats, folder_decisions) = stats_and_decisions;

        // Expunge deleted messages
        if self.account.delete_after_export {
            if self.is_gmail {
                self.expunge_gmail_all_mail().context("gmail all mail expunge")?;
            } else {
                let session = self.session.as_mut().context("Not connected")?;
                session.expunge().context("expunge folder")?;
            }
        }

        Ok((stats, folder_decisions))
    }

    /// Export all folders for the account.
    ///
    /// Returns `(folder_stats, decisions)` where `decisions` accumulates all
    /// `(staging_path, RouteDecision)` pairs produced during the export.
    /// The `.md` files remain in staging (D6 — deferred move); the caller is
    /// responsible for applying the decisions via `route::apply_decision`.
    ///
    /// `destinations.txt` is parsed **once** here — before the folder loop — and
    /// the resulting `Vec<Destination>` is reused for every email in every folder.
    /// If the file is absent or unconfigured, an empty `Vec` is used and all
    /// emails fall through to the default path with a warning.
    pub fn export_account(
        &mut self,
        on_progress: Option<&(dyn Fn(usize, usize, &str) + Send + Sync)>,
        on_status: Option<&(dyn Fn(&str) + Send + Sync)>,
        cancel_token: Option<&AtomicBool>,
    ) -> Result<(HashMap<String, ExportStats>, Vec<(PathBuf, RouteDecision)>)> {
        // ── Parse destinations.txt ONCE (before the folder loop) ──────────────
        // Shared with the tray "Reprendre le tri" scan via `route::load_destinations`.
        let dests: Vec<Destination> = crate::route::load_destinations();

        // Run the existing body in an IIFE so cleanup can run on every exit path.
        let run_result: Result<(HashMap<String, ExportStats>, Vec<(PathBuf, RouteDecision)>)> = (|| {
            let mut results = HashMap::new();
            let mut all_decisions: Vec<(PathBuf, RouteDecision)> = Vec::new();
            let mut contacts_collector = if self.account.collect_contacts {
                Some(ContactsCollector::new())
            } else {
                None
            };

            let folders = self.list_folders()?;
            let total_folders = folders.len();
            let mut folder_index = 0usize;

            for folder in folders {
                // Skip ignored folders (matched against the decoded display name)
                if self.account.ignored_folders.contains(&folder.display) {
                    println!("Ignored folder: {}", folder.display);
                    continue;
                }

                folder_index += 1;
                if let Some(cb) = on_progress {
                    cb(folder_index, total_folders, &folder.display);
                }

                println!("Exporting {} ...", folder.display);

                let (stats, folder_decisions) = self.export_folder(
                    &folder,
                    contacts_collector.as_mut(),
                    cancel_token,
                    &dests,
                )?;
                if let Some(s) = on_status {
                    s(&format!(
                        "{} — {} exportés, {} ignorés, {} erreurs",
                        folder.display, stats.exported, stats.skipped, stats.errors
                    ));
                }
                all_decisions.extend(folder_decisions);
                results.insert(folder.display, stats);

                if cancel_token.map_or(false, |t| t.load(Ordering::Relaxed)) {
                    break;
                }
            }

            // Generate contacts file if enabled — centralized in _local/contacts/
            if let Some(collector) = contacts_collector {
                let export_dir = PathBuf::from(&self.account.export_directory);
                let contacts_dir = export_dir
                    .parent()
                    .unwrap_or(&export_dir)
                    .join("_local")
                    .join("contacts");
                fs::create_dir_all(&contacts_dir)?;
                let filepath = collector.generate_csv(&contacts_dir, &self.account.name)?;
                println!("Generated contacts file: {}", filepath.display());
            }

            Ok((results, all_decisions))
        })();

        if self.account.cleanup_empty_dirs {
            let _ = crate::utils::cleanup_empty_dirs(
                &PathBuf::from(&self.account.export_directory),
            );
        }

        run_result
    }

    /// Disconnect from the server.
    pub fn disconnect(&mut self) -> Result<()> {
        if let Some(mut session) = self.session.take() {
            session.logout()?;
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone)]
pub struct ExportStats {
    pub exported: usize,
    pub skipped: usize,
    pub errors: usize,
}

/// A mailbox name as returned by the IMAP `LIST` response.
///
/// `raw` is the modified UTF-7 name as sent by the server and must be used
/// for IMAP commands like `SELECT`. `display` is the decoded UTF-8 form used
/// for local paths, logging and matching against `ignored_folders`.
#[derive(Debug, Clone)]
pub struct FolderName {
    pub raw: String,
    pub display: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_gmail_server_gmail() {
        assert!(is_gmail_server("imap.gmail.com"));
    }

    #[test]
    fn test_is_gmail_server_googlemail() {
        assert!(is_gmail_server("imap.googlemail.com"));
    }

    #[test]
    fn test_is_gmail_server_non_gmail() {
        assert!(!is_gmail_server("mail.example.com"));
    }

    #[test]
    fn test_is_gmail_server_outlook() {
        assert!(!is_gmail_server("imap.outlook.com"));
    }

    #[test]
    fn test_is_gmail_server_uppercase() {
        assert!(is_gmail_server("IMAP.GMAIL.COM"));
    }

    #[test]
    fn test_analyze_email_type() {
        // Basic test with raw email bytes
        let raw_email = b"From: sender@example.com\r\nTo: recipient@example.com\r\nSubject: Test\r\n\r\nBody";
        let mail = mailparse::parse_mail(raw_email).unwrap();
        let analysis = analyze_email_type(&mail);

        assert_eq!(analysis.email_type, EmailType::Direct);
        assert_eq!(analysis.from, "sender@example.com");
    }

    #[test]
    fn test_email_type_newsletter() {
        let raw_email = b"From: news@example.com\r\nTo: user@example.com\r\nSubject: Weekly Newsletter\r\n\r\nBody";
        let mail = mailparse::parse_mail(raw_email).unwrap();
        let analysis = analyze_email_type(&mail);

        assert_eq!(analysis.email_type, EmailType::Newsletter);
    }

    #[test]
    fn test_email_type_group() {
        let raw_email = b"From: sender@example.com\r\nTo: a@example.com, b@example.com\r\nSubject: Test\r\n\r\nBody";
        let mail = mailparse::parse_mail(raw_email).unwrap();
        let analysis = analyze_email_type(&mail);

        assert_eq!(analysis.email_type, EmailType::Group);
    }

    #[test]
    fn test_email_already_exported_in_subfolder() {
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let subfolder = temp.path().join("direct");
        fs::create_dir_all(&subfolder).unwrap();

        let md_content = "---\nsubject_hash: abc123\ndate: 2024-01-15\n---\nBody";
        fs::write(subfolder.join("email_2024-01-15_alice_to_bob_abc123.md"), md_content).unwrap();

        assert!(email_already_exported(
            "2024-01-15",
            "alice",
            "bob",
            "abc123",
            temp.path(),
        ));
    }

    #[test]
    fn test_contacts_collector() {
        let mut collector = ContactsCollector::new();
        collector.add(&EmailType::Direct, "test@example.com".to_string());
        collector.add(&EmailType::Group, "group@example.com".to_string());

        assert!(collector.direct.contains("test@example.com"));
        assert!(collector.group.contains("group@example.com"));
    }

    #[test]
    fn test_html_to_markdown_heading() {
        let result = html_to_markdown("<h1>Hello</h1>");
        assert!(result.contains("Hello"));
        assert!(!result.contains("<h1>"));
    }

    #[test]
    fn test_html_to_markdown_paragraph() {
        let result = html_to_markdown("<p>World</p>");
        assert!(result.contains("World"));
        assert!(!result.contains("<p>"));
    }

    #[test]
    fn test_html_to_markdown_empty() {
        let result = html_to_markdown("");
        assert!(result.is_empty() || result.trim().is_empty());
    }

    // ── Helper ──────────────────────────────────────────────────────────────────

    /// Build a minimal valid RFC 2822 raw email.
    fn make_raw_email(from: &str, to: &str, subject: &str, content_type: &str, body: &str) -> Vec<u8> {
        format!(
            "From: {from}\r\nTo: {to}\r\nSubject: {subject}\r\nDate: Mon, 01 Jan 2024 12:00:00 +0000\r\nContent-Type: {content_type}\r\n\r\n{body}"
        )
        .into_bytes()
    }

    /// Build a multipart/alternative email with a text/plain and a text/html part.
    fn make_multipart_email(plain: &str, html: &str) -> Vec<u8> {
        let boundary = "TEST_BOUNDARY_42";
        format!(
            "From: sender@example.com\r\nTo: recv@example.com\r\nSubject: Test\r\nDate: Mon, 01 Jan 2024 12:00:00 +0000\r\nContent-Type: multipart/alternative; boundary=\"{boundary}\"\r\n\r\n--{boundary}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{plain}\r\n--{boundary}\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{html}\r\n--{boundary}--\r\n"
        )
        .into_bytes()
    }

    // ── Phase 1 — extract_body ───────────────────────────────────────────────────

    #[test]
    fn test_extract_body_prefers_text_plain_over_html() {
        let raw = make_multipart_email("Hello plain text", "<p>Hello HTML</p>");
        let mail = mailparse::parse_mail(&raw).unwrap();
        let body = extract_body(&mail);

        // Inclusive: must contain the plain-text content
        assert!(body.contains("Hello plain text"), "body should contain plain text: got {:?}", body);
        // Exclusive: must NOT contain the HTML tag (plain preferred, not HTML-converted)
        assert!(!body.contains("<p>"), "body should not contain raw HTML tags: got {:?}", body);
    }

    #[test]
    fn test_extract_body_html_fallback_when_no_plain() {
        let boundary = "BOUND_HTML_ONLY";
        let raw = format!(
            "From: a@b.com\r\nTo: c@d.com\r\nSubject: S\r\nDate: Mon, 01 Jan 2024 12:00:00 +0000\r\nContent-Type: multipart/alternative; boundary=\"{boundary}\"\r\n\r\n--{boundary}\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>Only HTML body</p>\r\n--{boundary}--\r\n"
        ).into_bytes();
        let mail = mailparse::parse_mail(&raw).unwrap();
        let body = extract_body(&mail);

        // Inclusive: HTML was converted — the text content should be present
        assert!(body.contains("Only HTML body"), "body should contain converted HTML text: got {:?}", body);
        // Exclusive: conversion means no raw HTML tags remain
        assert!(!body.contains("<p>"), "body should not contain raw HTML tags after conversion: got {:?}", body);
    }

    #[test]
    fn test_extract_body_simple_non_multipart() {
        let raw = make_raw_email(
            "a@b.com", "c@d.com", "Simple", "text/plain; charset=utf-8", "Simple body content",
        );
        let mail = mailparse::parse_mail(&raw).unwrap();
        let body = extract_body(&mail);

        assert!(body.contains("Simple body content"), "body should contain text: got {:?}", body);
        assert!(!body.contains("Content-Type"), "body should not contain header lines: got {:?}", body);
    }

    #[test]
    fn test_extract_body_nested_multipart() {
        let inner_boundary = "INNER";
        let outer_boundary = "OUTER";
        // Outer: multipart/mixed wrapping an inner multipart/alternative
        let raw = format!(
            "From: a@b.com\r\nTo: c@d.com\r\nSubject: Nested\r\nDate: Mon, 01 Jan 2024 12:00:00 +0000\r\nContent-Type: multipart/mixed; boundary=\"{outer_boundary}\"\r\n\r\n--{outer_boundary}\r\nContent-Type: multipart/alternative; boundary=\"{inner_boundary}\"\r\n\r\n--{inner_boundary}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nNested plain text body\r\n--{inner_boundary}\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>Nested HTML</p>\r\n--{inner_boundary}--\r\n--{outer_boundary}--\r\n"
        ).into_bytes();
        let mail = mailparse::parse_mail(&raw).unwrap();
        let body = extract_body(&mail);

        assert!(body.contains("Nested plain text body"), "nested body should be extracted: got {:?}", body);
        assert!(!body.contains("<p>"), "should not contain raw HTML in nested case: got {:?}", body);
    }

    // ── Phase 2 — export_to_markdown E2E ────────────────────────────────────────

    fn make_account(export_dir: &str) -> crate::config::Account {
        crate::config::Account {
            name: "test".to_string(),
            server: "imap.example.com".to_string(),
            port: 993,
            username: "user@example.com".to_string(),
            password: None,
            export_directory: export_dir.to_string(),
            ignored_folders: vec![],
            quote_depth: 0,
            skip_existing: false,
            collect_contacts: false,
            skip_signature_images: false,
            delete_after_export: false,
            cleanup_empty_dirs: false,
            organize_by_type: false,
        }
    }

    #[test]
    fn test_export_to_markdown_produces_valid_frontmatter() {
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let export_dir = temp.path().join("out");
        let account = make_account(&export_dir.to_string_lossy());

        let raw = make_raw_email(
            "Alice <alice@example.com>",
            "Bob <bob@example.com>",
            "Hello World",
            "text/plain; charset=utf-8",
            "Test body",
        );

        let mut ctx = ExportContext {
            export_directory: &export_dir,
            base_export_directory: temp.path(),
            account: &account,
            debug_mode: false,
            dests: &[],
        };
        let result = export_to_markdown(
            &raw,
            vec!["INBOX".to_string()],
            None,
            &mut ctx,
        );

        let (path, _decision) = result.unwrap().expect("should return a path");
        let content = fs::read_to_string(&path).unwrap();

        // Inclusive: YAML frontmatter markers
        assert!(content.starts_with("---\n"), "file should start with ---: got {:?}", &content[..50.min(content.len())]);
        assert!(content.contains("subject: Hello World"), "frontmatter should contain subject: got {:?}", &content[..200.min(content.len())]);
        assert!(content.contains("from: Alice <alice@example.com>"), "frontmatter should contain from");
        // Inclusive: subject_hash key must be present in frontmatter
        assert!(content.contains("subject_hash:"), "subject_hash key must be present");
        // Body present after closing ---
        assert!(content.contains("Test body"), "body should appear after frontmatter");
    }

    #[test]
    fn test_export_to_markdown_names_attachment_with_date_prefix() {
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let export_dir = temp.path().join("out");
        let account = make_account(&export_dir.to_string_lossy());

        // multipart/mixed: a text part + a PDF attachment named "Facture.pdf".
        let boundary = "BOUND_ATT";
        let raw = format!(
            "From: Alice <alice@example.com>\r\nTo: Bob <bob@example.com>\r\nSubject: Invoice\r\nDate: Mon, 01 Jan 2024 12:00:00 +0000\r\nContent-Type: multipart/mixed; boundary=\"{boundary}\"\r\n\r\n--{boundary}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nBody text\r\n--{boundary}\r\nContent-Type: application/pdf; name=\"Facture.pdf\"\r\nContent-Disposition: attachment; filename=\"Facture.pdf\"\r\n\r\n%PDF-1.4 fake pdf payload\r\n--{boundary}--\r\n"
        ).into_bytes();

        let mut ctx = ExportContext {
            export_directory: &export_dir,
            base_export_directory: temp.path(),
            account: &account,
            debug_mode: false,
            dests: &[],
        };
        let (md_path, _decision) = export_to_markdown(&raw, vec![], None, &mut ctx)
            .unwrap()
            .expect("export should produce a file");

        // Inclusive: the attachment is named `<date>_<original-name>`.
        let att = export_dir.join("2024-01-01_Facture.pdf");
        assert!(att.exists(), "attachment must be named with date prefix: {:?}", att);

        // Exclusive: no cryptic hash / double-underscore prefix remains.
        let md = fs::read_to_string(&md_path).unwrap();
        assert!(
            md.contains("2024-01-01_Facture.pdf"),
            "frontmatter/body must reference the dated attachment name"
        );
        assert!(
            !md.contains("__"),
            "attachment link must not carry the old `__<hash>_` scheme: got {:?}",
            md
        );
    }

    #[test]
    fn test_export_to_markdown_skip_existing_returns_none() {
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let export_dir = temp.path().join("out");
        let mut account = make_account(&export_dir.to_string_lossy());
        account.skip_existing = true;

        let raw = make_raw_email(
            "alice@example.com",
            "bob@example.com",
            "Duplicate Subject",
            "text/plain; charset=utf-8",
            "Body text",
        );

        // First export — should succeed
        let mut ctx = ExportContext {
            export_directory: &export_dir,
            base_export_directory: temp.path(),
            account: &account,
            debug_mode: false,
            dests: &[],
        };
        let (first_path, _decision) = export_to_markdown(&raw, vec![], None, &mut ctx)
            .unwrap()
            .expect("first export should produce a file");

        // Verify the file exists and contains the subject_hash
        let content = fs::read_to_string(&first_path).unwrap();
        assert!(content.contains("subject_hash:"), "first export must have subject_hash");

        // Second export — should be skipped
        let second = export_to_markdown(&raw, vec![], None, &mut ctx).unwrap();
        assert!(second.is_none(), "second export should return None when skip_existing is true");
    }

    // ── Phase 3 — fix_html_bodies / extract_md_body ─────────────────────────────

    #[test]
    fn test_extract_md_body_extracts_body() {
        let content = "---\nfrom: a@b.com\nsubject: Test\n---\n\nHello body here";
        let body = extract_md_body(content);

        assert!(body.contains("Hello body here"), "body should be extracted: got {:?}", body);
        assert!(!body.contains("from:"), "body should not contain frontmatter fields: got {:?}", body);
        assert!(!body.contains("---"), "body should not contain separators: got {:?}", body);
    }

    #[test]
    fn test_extract_md_body_crlf_separators() {
        let content = "---\r\nfrom: a@b.com\r\nsubject: Test\r\n---\r\n\r\nCRLF body";
        let body = extract_md_body(content);

        assert!(body.contains("CRLF body"), "CRLF body should be extracted: got {:?}", body);
        assert!(!body.contains("from:"), "should not contain frontmatter: got {:?}", body);
    }

    #[test]
    fn test_extract_md_body_no_frontmatter_returns_full_content() {
        let content = "No frontmatter here\nJust plain content";
        let body = extract_md_body(content);

        // When there is no frontmatter, the full content is returned
        assert!(body.contains("No frontmatter here"), "full content should be returned: got {:?}", body);
        assert!(body.contains("Just plain content"), "full content should be returned: got {:?}", body);
    }

    #[test]
    fn test_fix_html_bodies_converts_html_md_file() {
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let md_file = temp.path().join("email.md");
        let html_content = "---\nfrom: a@b.com\nsubject: Test\n---\n<!DOCTYPE html><html><body><p>Converted paragraph</p></body></html>";
        fs::write(&md_file, html_content).unwrap();

        let stats = fix_html_bodies(temp.path(), false, None).unwrap();

        assert_eq!(stats.fixed, 1, "one file should be fixed");
        assert_eq!(stats.errors, 0, "no errors expected");

        let after = fs::read_to_string(&md_file).unwrap();
        // Inclusive: HTML was converted
        assert!(after.contains("Converted paragraph"), "converted text should be present: got {:?}", &after);
        // Exclusive: raw HTML tags removed
        assert!(!after.contains("<p>"), "raw <p> tags should be gone: got {:?}", &after);
        assert!(!after.contains("<!DOCTYPE"), "DOCTYPE declaration should be gone: got {:?}", &after);
    }

    #[test]
    fn test_fix_html_bodies_dry_run_does_not_modify() {
        use tempfile::TempDir;

        let temp = TempDir::new().unwrap();
        let md_file = temp.path().join("email_dry.md");
        let original = "---\nfrom: a@b.com\nsubject: Test\n---\n<!DOCTYPE html><html><body><p>Dry run body</p></body></html>";
        fs::write(&md_file, original).unwrap();

        let stats = fix_html_bodies(temp.path(), true, None).unwrap();

        assert_eq!(stats.fixed, 1, "dry run still counts fixed");
        let after = fs::read_to_string(&md_file).unwrap();
        // Exclusive: file must not be modified
        assert_eq!(after, original, "dry_run must not modify the file");
    }

    // ── Phase 4 — extract_filename_from_header ───────────────────────────────────

    #[test]
    fn test_extract_filename_from_header_quoted() {
        let header = r#"attachment; filename="report.pdf""#;
        let result = extract_filename_from_header(header);

        assert_eq!(result.as_deref(), Some("report.pdf"), "quoted filename should be extracted");
    }

    #[test]
    fn test_extract_filename_from_header_star_form() {
        // RFC 5987 star-form: filename*=UTF-8''document.pdf
        let header = "attachment; filename*=UTF-8''document.pdf";
        let result = extract_filename_from_header(header);

        // The regex matches `filename*=` capturing the unquoted value
        assert!(result.is_some(), "star-form filename should be extracted");
        let val = result.unwrap();
        assert!(val.contains("document.pdf"), "extracted value should contain filename: got {:?}", val);
    }

    #[test]
    fn test_extract_filename_from_content_type_name() {
        let header = r#"application/pdf; name="invoice.pdf""#;
        let result = extract_filename_from_header(header);

        assert_eq!(result.as_deref(), Some("invoice.pdf"), "name= parameter should be extracted from Content-Type");
    }

    #[test]
    fn test_extract_filename_from_header_none_when_absent() {
        let header = "attachment; size=12345";
        let result = extract_filename_from_header(header);

        assert!(result.is_none(), "should return None when no filename parameter: got {:?}", result);
    }
}
