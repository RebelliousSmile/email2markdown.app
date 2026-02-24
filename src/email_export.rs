use crate::config::Account;
use crate::network::{NetworkConfig, ProgressIndicator, with_retry};  // [3][4]
use crate::utils::{
    decode_imap_utf7, decode_mime_filename, extract_emails, get_short_name, hash_md5_prefix,
    is_signature_image, limit_quote_depth, normalize_line_breaks, sanitize_filename,
};
use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset, Utc};
use imap::{ImapConnection, Session};
use mailparse::{self, MailHeaderMap, ParsedMail};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailFrontmatter {
    pub from: String,
    pub to: String,
    pub date: String,
    pub subject: String,
    pub subject_hash: String,
    pub tags: Vec<String>,
    pub attachments: Vec<String>,
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

    pub fn generate_csv(&self, base_dir: &Path, account_name: &str) -> Result<PathBuf> {
        let date_str = Utc::now().format("%Y-%m-%d").to_string();
        let filename = format!("contacts_{}_{}.csv", account_name, date_str);
        let filepath = base_dir.join(&filename);

        let mut writer = csv::Writer::from_path(&filepath)?;
        writer.write_record(["Name", "Email", "Type", "Source", "Notes"])?;

        let categories = [
            (&self.direct, "Direct"),
            (&self.group, "Group"),
            (&self.newsletter, "Newsletter"),
            (&self.mailing_list, "Mailing List"),
            (&self.unknown, "Unknown"),
        ];

        for (contacts, contact_type) in categories {
            for contact in contacts {
                let name = contact
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

                writer.write_record([
                    &name,
                    contact,
                    contact_type,
                    account_name,
                    &format!("Collected from {} emails", account_name),
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

    if let Ok(entries) = fs::read_dir(export_directory) {
        for entry in entries.flatten() {
            let filename = entry.file_name().to_string_lossy().to_string();
            if glob::Pattern::new(&search_pattern)
                .map(|p| p.matches(&filename))
                .unwrap_or(false)
            {
                // Check if file contains the subject hash
                if let Ok(content) = fs::read_to_string(entry.path()) {
                    if content.contains(subject_hash) {
                        return true;
                    }
                }
            }
        }
    }

    false
}

/// Parse email date string to DateTime.
fn parse_email_date(date_str: &str) -> Option<DateTime<FixedOffset>> {
    mailparse::dateparse(date_str)
        .ok()
        .map(|ts| DateTime::from_timestamp(ts, 0))
        .flatten()
        .map(|dt| dt.with_timezone(&FixedOffset::east_opt(0).unwrap()))
}

/// Export a single email to Markdown with frontmatter.
pub fn export_to_markdown(
    raw_email: &[u8],
    export_directory: &Path,
    base_export_directory: &Path,
    tags: Vec<String>,
    account: &Account,
    contacts_collector: Option<&mut ContactsCollector>,
    debug_mode: bool,
) -> Result<Option<PathBuf>> {
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
        return Ok(None);
    }

    // Analyze email and collect contacts if enabled
    if let Some(collector) = contacts_collector {
        let analysis = analyze_email_type(&mail);
        for contact in analysis.contacts {
            collector.add(&analysis.email_type, contact);
        }
    }

    // Create export directory if needed
    fs::create_dir_all(export_directory)?;

    // Generate unique filename
    let base_filename = format!("email_{}_{}*to_{}", date_str, sender_short, recipient_short);
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

    // Handle attachments
    let relative_path = export_directory
        .strip_prefix(base_export_directory)
        .unwrap_or(export_directory);
    let attachments_dir = base_export_directory.join("attachments").join(relative_path);
    fs::create_dir_all(&attachments_dir)?;

    let mut attachments = Vec::new();
    let base_filename_for_attachments = base_filename.replace('*', "_");

    extract_attachments(
        &mail,
        &attachments_dir,
        &base_filename_for_attachments,
        base_export_directory,
        account.skip_signature_images,
        debug_mode,
        &mut attachments,
    )?;

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
    };

    // Normalize body and add attachments list
    let mut normalized_body = normalize_line_breaks(&body);

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

    Ok(Some(filepath))
}

/// Extract the body from a parsed email.
fn extract_body(mail: &ParsedMail) -> String {
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
                body = part.get_body().unwrap_or_default();
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
fn extract_attachments(
    mail: &ParsedMail,
    attachments_dir: &Path,
    base_filename: &str,
    base_export_directory: &Path,
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
                    let filename_hash = hash_md5_prefix(&decoded_filename, 8);
                    let full_filename =
                        format!("{}_{}_{}", base_filename, filename_hash, safe_filename);
                    let filepath = attachments_dir.join(&full_filename);

                    fs::write(&filepath, &payload)?;

                    // Calculate relative path from base export directory
                    let relative_path = filepath
                        .strip_prefix(base_export_directory)
                        .unwrap_or(&filepath)
                        .to_string_lossy()
                        .replace('\\', "/");

                    attachments.push(relative_path);
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
                base_filename,
                base_export_directory,
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

/// Extract filename parameter from a header value.
fn extract_filename_from_header(header: &str) -> Option<String> {
    // Look for filename="..." or filename=...
    let re = regex::Regex::new(r#"filename[*]?=(?:"([^"]+)"|([^;\s]+))"#).ok()?;
    if let Some(caps) = re.captures(header) {
        return caps
            .get(1)
            .or_else(|| caps.get(2))
            .map(|m| m.as_str().to_string());
    }

    // Look for name="..." or name=...
    let re_name = regex::Regex::new(r#"name[*]?=(?:"([^"]+)"|([^;\s]+))"#).ok()?;
    if let Some(caps) = re_name.captures(header) {
        return caps
            .get(1)
            .or_else(|| caps.get(2))
            .map(|m| m.as_str().to_string());
    }

    None
}

/// IMAP client for exporting emails.
pub struct ImapExporter {
    session: Option<Session<Box<dyn ImapConnection>>>,
    account: Account,
    debug_mode: bool,
    network_config: NetworkConfig,  // [4][5]
}

impl ImapExporter {
    pub fn new(account: Account, debug_mode: bool) -> Self {
        ImapExporter {
            session: None,
            account,
            debug_mode,
            network_config: NetworkConfig::default(),  // [4][5]
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
            .connect()?;

        if self.debug_mode {
            println!("Authenticating as {}...", self.account.username);
        }

        let session = client.login(&self.account.username, password).map_err(|e| e.0)?;

        if self.debug_mode {
            println!("Connected successfully!");
        }

        self.session = Some(session);
        Ok(())
    }

    /// List all folders.
    pub fn list_folders(&mut self) -> Result<Vec<String>> {
        let session = self.session.as_mut().context("Not connected")?;

        if self.debug_mode {
            println!("Listing folders...");
        }

        let folders = session.list(None, Some("*"))?;
        let folder_names: Vec<String> = folders
            .iter()
            .map(|f| decode_imap_utf7(f.name()))
            .collect();

        if self.debug_mode {
            println!("Found {} folders", folder_names.len());
        }

        Ok(folder_names)
    }

    /// Export a single folder.
    pub fn export_folder(
        &mut self,
        folder_name: &str,
        mut contacts_collector: Option<&mut ContactsCollector>,
    ) -> Result<ExportStats> {
        let base_export_directory = PathBuf::from(&self.account.export_directory);
        let export_directory = base_export_directory.join(folder_name.replace('.', "/"));

        let session = self.session.as_mut().context("Not connected")?;

        // Select folder
        let mailbox = session.select(folder_name)?;
        let message_count = mailbox.exists as usize;

        if self.debug_mode {
            println!("  {} messages in folder", message_count);
        }

        // Search for all messages
        let uids = session.search("ALL")?;
        let uids_vec: Vec<_> = uids.into_iter().collect();
        let total_messages = uids_vec.len();

        // [3] Progress indicator
        let mut progress = ProgressIndicator::new(folder_name, total_messages);
        let mut stats = ExportStats::default();

        for (_idx, uid) in uids_vec.into_iter().enumerate() {
            // [4] Retry logic for fetch
            let fetch_result = with_retry(&self.network_config, "fetch", || {
                session.fetch(uid.to_string(), "RFC822")
            });

            let messages = match fetch_result {
                Ok(m) => m,
                Err(e) => {
                    if self.debug_mode {
                        println!("  Failed to fetch message {}: {}", uid, e);
                    }
                    stats.errors += 1;
                    progress.inc();
                    continue;
                }
            };

            for message in messages.iter() {
                if let Some(body) = message.body() {
                    let result = export_to_markdown(
                        body,
                        &export_directory,
                        &base_export_directory,
                        vec![folder_name.to_string()],
                        &self.account,
                        contacts_collector.as_deref_mut(),
                        self.debug_mode,
                    );

                    match result {
                        Ok(Some(_)) => stats.exported += 1,
                        Ok(None) => stats.skipped += 1,
                        Err(e) => {
                            if self.debug_mode {
                                println!("  Error exporting message {}: {}", uid, e);
                            }
                            stats.errors += 1;
                        }
                    }
                }
            }

            // Delete after export if requested
            if self.account.delete_after_export {
                session.store(uid.to_string(), "+FLAGS (\\Deleted)")?;
            }

            // [3] Update progress
            progress.inc();
        }

        // [3] Finish progress indicator
        progress.finish_with_message(&format!(
            "{} exported, {} skipped, {} errors",
            stats.exported, stats.skipped, stats.errors
        ));

        // Expunge deleted messages
        if self.account.delete_after_export {
            session.expunge()?;
        }

        Ok(stats)
    }

    /// Export all folders for the account.
    pub fn export_account(&mut self) -> Result<HashMap<String, ExportStats>> {
        let mut results = HashMap::new();
        let mut contacts_collector = if self.account.collect_contacts {
            Some(ContactsCollector::new())
        } else {
            None
        };

        let folders = self.list_folders()?;

        for folder in folders {
            // Skip ignored folders
            if self.account.ignored_folders.contains(&folder) {
                println!("Ignored folder: {}", folder);
                continue;
            }

            println!("Exporting {} ...", folder);

            let stats = self.export_folder(&folder, contacts_collector.as_mut())?;
            println!(
                "  {} exported, {} skipped, {} errors",
                stats.exported, stats.skipped, stats.errors
            );

            results.insert(folder, stats);
        }

        // Generate contacts file if enabled
        if let Some(collector) = contacts_collector {
            let base_dir = PathBuf::from(&self.account.export_directory);
            let filepath = collector.generate_csv(&base_dir, &self.account.name)?;
            println!("Generated contacts file: {}", filepath.display());
        }

        Ok(results)
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_contacts_collector() {
        let mut collector = ContactsCollector::new();
        collector.add(&EmailType::Direct, "test@example.com".to_string());
        collector.add(&EmailType::Group, "group@example.com".to_string());

        assert!(collector.direct.contains("test@example.com"));
        assert!(collector.group.contains("group@example.com"));
    }
}
