use crate::config::SortConfig;
use anyhow::{Context, Result};
use chrono::{DateTime, FixedOffset, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use walkdir::WalkDir;

static UNSUBSCRIBE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)https?://[^\s)>\]]*unsubscribe[^\s)>\]]*").expect("static regex"));

/// Email sorting category.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Delete,
    Summarize,
    Keep,
}

impl std::fmt::Display for Category {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Category::Delete => write!(f, "delete"),
            Category::Summarize => write!(f, "summarize"),
            Category::Keep => write!(f, "keep"),
        }
    }
}

/// Scope for a category change: single email or all emails from the same sender.
#[derive(Debug, Clone, PartialEq)]
pub enum CategoryScope {
    Single,
    BySender,
}

/// Email type classification.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmailSortType {
    Newsletter,
    MailingList,
    Group,
    Direct,
    Unknown,
}

impl std::fmt::Display for EmailSortType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EmailSortType::Newsletter => write!(f, "newsletter"),
            EmailSortType::MailingList => write!(f, "mailing_list"),
            EmailSortType::Group => write!(f, "group"),
            EmailSortType::Direct => write!(f, "direct"),
            EmailSortType::Unknown => write!(f, "unknown"),
        }
    }
}

/// Analyzed email data.
#[derive(Debug, Clone, Serialize)]
pub struct EmailData {
    pub file_path: PathBuf,
    pub file_name: String,
    pub file_size: u64,
    pub body_length: usize,
    pub has_attachments: bool,
    pub attachment_count: usize,
    pub date: Option<DateTime<FixedOffset>>,
    pub age_days: Option<i64>,
    pub sender: String,
    pub recipients: Vec<String>,
    pub subject: String,
    pub tags: Vec<String>,
    pub email_type: EmailSortType,
    pub sender_count: usize,
    pub score: i32,
    pub score_breakdown: Vec<(String, i32)>,
    pub category: Category,
}

/// Sorting statistics.
#[derive(Debug, Default, Serialize)]
pub struct SortStats {
    pub total_emails: usize,
    pub by_category: HashMap<String, usize>,
    pub by_type: HashMap<String, usize>,
    pub by_sender: HashMap<String, usize>,
    pub by_date: HashMap<String, usize>,
}

/// Sorting report.
#[derive(Debug, Serialize, Deserialize)]
pub struct SortReport {
    #[serde(default)]
    pub base_directory: String,
    #[serde(default)]
    pub organize_by_type: bool,
    pub summary: SortSummary,
    pub details: SortDetails,
    pub categories: HashMap<String, Vec<EmailSummary>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SortSummary {
    pub total_emails: usize,
    pub categories: HashMap<String, usize>,
    pub recommendations: HashMap<String, String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SortDetails {
    pub by_type: HashMap<String, usize>,
    pub by_sender: Vec<(String, usize)>,
    pub by_date: HashMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailSummary {
    pub file: String,
    pub subject: String,
    pub sender: String,
    pub date: String,
    pub score: i32,
    #[serde(rename = "type")]
    pub email_type: String,
    pub size: u64,
    pub attachments: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub breakdown: Vec<(String, i32)>,
}

/// Email sorter.
pub struct EmailSorter {
    base_directory: PathBuf,
    config: SortConfig,
    categories: HashMap<Category, Vec<EmailData>>,
    stats: SortStats,
}

impl EmailSorter {
    pub fn new(base_directory: PathBuf, config: SortConfig) -> Self {
        let mut stats = SortStats::default();
        stats.by_category.insert("delete".to_string(), 0);
        stats.by_category.insert("summarize".to_string(), 0);
        stats.by_category.insert("keep".to_string(), 0);

        EmailSorter {
            base_directory,
            config,
            categories: HashMap::new(),
            stats,
        }
    }

    /// Analyze a single email markdown file, returning the parsed data and body text.
    pub fn analyze_email_file(&self, file_path: &Path) -> Result<Option<(EmailData, String)>> {
        let content = fs::read_to_string(file_path)
            .context("Failed to read file")?;

        // Handle empty or very small files
        if content.trim().len() < 10 {
            println!("  Skipping empty file: {}", file_path.display());
            return Ok(None);
        }

        // Handle files with no frontmatter
        if !content.starts_with("---") {
            println!(
                "  Skipping file with no YAML frontmatter: {}",
                file_path.display()
            );
            return Ok(None);
        }

        // Extract frontmatter and body
        let (frontmatter, body) = match extract_frontmatter(&content) {
            Some(parts) => parts,
            None => {
                println!("  No valid frontmatter in: {}", file_path.display());
                return Ok(None);
            }
        };

        // Parse frontmatter
        let fm: Value = match serde_yaml::from_str(&frontmatter) {
            Ok(v) => v,
            Err(e) => {
                println!("  Could not parse frontmatter: {}...", &e.to_string()[..100.min(e.to_string().len())]);
                return Ok(None);
            }
        };

        let metadata = fs::metadata(file_path)?;

        // Extract fields with null checks
        let subject = fm
            .get("subject")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let sender = fm
            .get("from")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let date_str = fm
            .get("date")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let attachments = fm
            .get("attachments")
            .and_then(|v| v.as_sequence())
            .map(|s| s.len())
            .unwrap_or(0);

        let tags: Vec<String> = fm
            .get("tags")
            .and_then(|v| v.as_sequence())
            .map(|s| {
                s.iter()
                    .filter_map(|v| v.as_str())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();

        // Parse date
        let date = parse_date(date_str);
        let age_days = date.map(|d| {
            let now = Utc::now();
            (now.signed_duration_since(d.with_timezone(&Utc))).num_days()
        });

        // Determine email type
        let email_type = self.determine_email_type(&subject, &fm, &body);

        // Build email data
        let email_data = EmailData {
            file_path: file_path.to_path_buf(),
            file_name: file_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
            file_size: metadata.len(),
            body_length: body.len(),
            has_attachments: attachments > 0,
            attachment_count: attachments,
            date,
            age_days,
            sender,
            recipients: Vec::new(),
            subject,
            tags,
            email_type,
            sender_count: 0,
            score: 0,
            score_breakdown: Vec::new(),
            category: Category::Summarize,
        };

        Ok(Some((email_data, body)))
    }

    /// Determine email type from frontmatter field, subject, and body.
    fn determine_email_type(&self, subject: &str, fm: &Value, body: &str) -> EmailSortType {
        // Check frontmatter email_type field first (from export Phase 1)
        if let Some(et) = fm.get("email_type").and_then(|v| v.as_str()) {
            match et {
                "newsletter" => return EmailSortType::Newsletter,
                "mailing_list" => return EmailSortType::MailingList,
                "group" => return EmailSortType::Group,
                "direct" => return EmailSortType::Direct,
                _ => {}
            }
        }

        let subject_lower = subject.to_lowercase();

        if subject_lower.contains("newsletter")
            || subject_lower.contains("bulletin")
            || subject_lower.contains("digest")
            || UNSUBSCRIBE_RE.is_match(body)
        {
            EmailSortType::Newsletter
        } else {
            EmailSortType::Direct
        }
    }

    /// Calculate a score for the email, returning (score, breakdown).
    fn calculate_score(&self, email_data: &EmailData, body: &str) -> (i32, Vec<(String, i32)>) {
        let mut score: i32 = 0;
        let mut breakdown: Vec<(String, i32)> = Vec::new();

        // Type weight
        if self.config.use_type_weights {
            let type_key = email_data.email_type.to_string();
            if let Some(&weight) = self.config.type_weights.get(&type_key) {
                if weight != 0 {
                    breakdown.push(("type".to_string(), weight));
                }
                score += weight;
            }
        }

        // Age factors
        if self.config.use_age_scoring {
            if let Some(age) = email_data.age_days {
                if age <= self.config.recent_threshold_days {
                    breakdown.push(("age".to_string(), 2));
                    score += 2;
                } else if age >= self.config.old_threshold_days {
                    breakdown.push(("age".to_string(), -1));
                    score -= 1;
                }
            }
        }

        // Size factors
        if self.config.use_size_scoring {
            if email_data.body_length <= self.config.small_email_threshold {
                breakdown.push(("size".to_string(), -1));
                score -= 1;
            } else if email_data.body_length >= self.config.large_email_threshold {
                breakdown.push(("size".to_string(), 1));
                score += 1;
            }
        }

        // Attachment factors
        if email_data.has_attachments {
            if self.config.keep_with_attachments {
                breakdown.push(("attachments".to_string(), 1));
                score += 1;
            } else {
                breakdown.push(("attachments".to_string(), -1));
                score -= 1;
            }
        }

        // Folder scoring from tags
        if self.config.use_folder_score && !email_data.tags.is_empty() {
            let min_folder = email_data.tags.iter().map(|t| folder_score(t)).min().unwrap_or(0);
            if min_folder != 0 {
                breakdown.push(("folder".to_string(), min_folder));
                score += min_folder;
            }
        }

        // Subfolder bonus
        if self.config.use_subfolder_bonus && !email_data.tags.is_empty() {
            let all_user_folders = email_data.tags.iter().all(|t| !is_base_folder(t));
            if all_user_folders {
                breakdown.push(("subfolder".to_string(), 2));
                score += 2;
            }
        }

        // Subject analysis
        if self.config.use_subject_rules {
            let subject_lower = email_data.subject.to_lowercase();

            let delete_count = self
                .config
                .delete_keywords
                .iter()
                .filter(|k| subject_lower.contains(&k.to_lowercase()))
                .count() as i32;
            if delete_count > 0 {
                breakdown.push(("subject_delete".to_string(), -delete_count));
            }
            score -= delete_count;

            let keep_count = self
                .config
                .keep_keywords
                .iter()
                .filter(|k| subject_lower.contains(&k.to_lowercase()))
                .count() as i32;
            if keep_count > 0 {
                breakdown.push(("subject_keep".to_string(), keep_count * 2));
            }
            score += keep_count * 2;
        }

        // Sender analysis
        if self.config.use_sender_rules {
            let sender_lower = email_data.sender.to_lowercase();

            if self
                .config
                .delete_senders
                .iter()
                .any(|s| sender_lower.contains(&s.to_lowercase()))
            {
                breakdown.push(("sender_delete".to_string(), -3));
                score -= 3;
            }

            if self
                .config
                .keep_senders
                .iter()
                .any(|s| sender_lower.contains(&s.to_lowercase()))
            {
                breakdown.push(("sender_keep".to_string(), 3));
                score += 3;
            }
        }

        // Recurring sender malus
        if self.config.penalize_recurring && email_data.sender_count > 1 {
            let malus = -((email_data.sender_count as f64).ln() as i32);
            if malus != 0 {
                breakdown.push(("recurring".to_string(), malus));
            }
            score += malus;
        }

        // Body content analysis
        if self.config.use_body_keywords {
            let body_lower = body.to_lowercase();
            let important_keywords = [
                "contract",
                "invoice",
                "legal",
                "urgent",
                "important",
                "confidential",
                "agreement",
                "signature",
                "payment",
            ];

            if important_keywords
                .iter()
                .any(|&k| body_lower.contains(k))
            {
                breakdown.push(("body_keywords".to_string(), 2));
                score += 2;
            }
        }

        (score, breakdown)
    }

    /// Determine the category for an email.
    fn determine_category(&self, email_data: &EmailData, body: &str) -> Category {
        // Check whitelist first
        if self.config.is_whitelisted(&email_data.sender) {
            return Category::Keep;
        }

        let subject_lower = email_data.subject.to_lowercase();
        let sender_lower = email_data.sender.to_lowercase();
        let body_lower = body.to_lowercase();

        // Strong delete indicators
        let delete_indicators = email_data.email_type == EmailSortType::Newsletter
            || self
                .config
                .delete_keywords
                .iter()
                .any(|k| subject_lower.contains(&k.to_lowercase()))
            || self
                .config
                .delete_senders
                .iter()
                .any(|s| sender_lower.contains(&s.to_lowercase()));

        // Strong keep indicators
        let keep_indicators = self
            .config
            .keep_keywords
            .iter()
            .any(|k| subject_lower.contains(&k.to_lowercase()))
            || self
                .config
                .keep_senders
                .iter()
                .any(|s| sender_lower.contains(&s.to_lowercase()))
            || ["contract", "invoice", "legal", "urgent", "important"]
                .iter()
                .any(|&k| body_lower.contains(k));

        // Apply rules
        if keep_indicators {
            Category::Keep
        } else if self.config.delete_newsletters && email_data.email_type == EmailSortType::Newsletter {
            Category::Delete
        } else if delete_indicators || email_data.score <= -2 {
            Category::Delete
        } else if email_data.score >= 2
            || email_data.body_length > self.config.summarize_max_length
        {
            Category::Keep
        } else {
            Category::Summarize
        }
    }

    /// Sort all emails in the directory.
    pub fn sort_emails(&mut self) -> Result<()> {
        println!("Sorting emails in: {}", self.base_directory.display());

        let entries: Vec<PathBuf> = WalkDir::new(&self.base_directory)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path().extension().is_some_and(|ext| ext == "md")
                    && !e.path().to_string_lossy().contains("attachments")
            })
            .map(|e| e.path().to_path_buf())
            .collect();

        // Pass 1: collect emails and count senders
        let mut collected: Vec<(EmailData, String)> = Vec::new();
        let mut sender_counts: HashMap<String, usize> = HashMap::new();

        for file_path in &entries {
            if let Some((email_data, body)) = self.analyze_email_file(file_path)? {
                let sender_key = email_data.sender.to_lowercase();
                *sender_counts.entry(sender_key).or_insert(0) += 1;
                collected.push((email_data, body));
            }
        }

        // Pass 2: score and categorize with sender counts
        for (mut email_data, body) in collected {
            let sender_key = email_data.sender.to_lowercase();
            email_data.sender_count = *sender_counts.get(&sender_key).unwrap_or(&1);
            let (calc_score, breakdown) = self.calculate_score(&email_data, &body);
            email_data.score = calc_score;
            email_data.score_breakdown = breakdown;
            email_data.category = self.determine_category(&email_data, &body);

            self.stats.total_emails += 1;

            let category = email_data.category.clone();
            let category_key = category.to_string();
            *self
                .stats
                .by_category
                .entry(category_key)
                .or_insert(0) += 1;

            let type_key = email_data.email_type.to_string();
            *self.stats.by_type.entry(type_key).or_insert(0) += 1;

            *self
                .stats
                .by_sender
                .entry(email_data.sender.clone())
                .or_insert(0) += 1;

            if let Some(date) = &email_data.date {
                let date_key = date.format("%Y-%m").to_string();
                *self.stats.by_date.entry(date_key).or_insert(0) += 1;
            }

            self.categories
                .entry(category)
                .or_default()
                .push(email_data);
        }

        Ok(())
    }

    /// Generate a sorting report.
    pub fn generate_report(&self) -> SortReport {
        let total = self.stats.total_emails as f64;

        let mut recommendations = HashMap::new();
        if total > 0.0 {
            let delete_pct = (self.stats.by_category.get("delete").unwrap_or(&0) * 100) as f64 / total;
            let summarize_pct = (self.stats.by_category.get("summarize").unwrap_or(&0) * 100) as f64 / total;
            let keep_pct = (self.stats.by_category.get("keep").unwrap_or(&0) * 100) as f64 / total;

            recommendations.insert(
                "delete".to_string(),
                format!("{:.1}% of emails can be deleted", delete_pct),
            );
            recommendations.insert(
                "summarize".to_string(),
                format!("{:.1}% of emails can be summarized", summarize_pct),
            );
            recommendations.insert(
                "keep".to_string(),
                format!("{:.1}% of emails should be kept in full", keep_pct),
            );
        }

        // Get top senders
        let mut sender_counts: Vec<_> = self.stats.by_sender.iter().collect();
        sender_counts.sort_by(|a, b| b.1.cmp(a.1));
        let top_senders: Vec<(String, usize)> = sender_counts
            .into_iter()
            .take(10)
            .map(|(k, v)| (k.clone(), *v))
            .collect();

        // Build category details
        let mut categories = HashMap::new();
        for (category, emails) in &self.categories {
            let summaries: Vec<EmailSummary> = emails
                .iter()
                .map(|e| EmailSummary {
                    file: e
                        .file_path
                        .strip_prefix(&self.base_directory)
                        .unwrap_or(&e.file_path)
                        .to_string_lossy()
                        .to_string(),
                    subject: e.subject.clone(),
                    sender: e.sender.clone(),
                    date: e
                        .date
                        .map(|d| d.format("%Y-%m-%d").to_string())
                        .unwrap_or_else(|| "Unknown".to_string()),
                    score: e.score,
                    email_type: e.email_type.to_string(),
                    size: e.file_size,
                    attachments: e.attachment_count,
                    breakdown: e.score_breakdown.clone(),
                })
                .collect();

            categories.insert(category.to_string(), summaries);
        }

        SortReport {
            base_directory: self.base_directory.to_string_lossy().to_string(),
            organize_by_type: self.config.organize_by_type,
            summary: SortSummary {
                total_emails: self.stats.total_emails,
                categories: self.stats.by_category.clone(),
                recommendations,
            },
            details: SortDetails {
                by_type: self.stats.by_type.clone(),
                by_sender: top_senders,
                by_date: self.stats.by_date.clone(),
            },
            categories,
        }
    }

    /// Save report to JSON file.
    pub fn save_report(&self, report: &SortReport, output_file: &str) -> Result<PathBuf> {
        let output_path = self.base_directory.join(output_file);
        let content = serde_json::to_string_pretty(report)?;
        fs::write(&output_path, content)?;
        println!("Report saved to: {}", output_path.display());
        Ok(output_path)
    }

    /// Print summary of sorting results.
    pub fn print_summary(&self) {
        println!("\n==================================================");
        println!("EMAIL SORTING SUMMARY");
        println!("==================================================");

        println!("Total emails analyzed: {}", self.stats.total_emails);
        println!(
            "To delete: {}",
            self.stats.by_category.get("delete").unwrap_or(&0)
        );
        println!(
            "To summarize: {}",
            self.stats.by_category.get("summarize").unwrap_or(&0)
        );
        println!(
            "To keep: {}",
            self.stats.by_category.get("keep").unwrap_or(&0)
        );

        if self.stats.total_emails > 0 {
            let total = self.stats.total_emails as f64;
            let delete_pct = (self.stats.by_category.get("delete").unwrap_or(&0) * 100) as f64 / total;
            let summarize_pct = (self.stats.by_category.get("summarize").unwrap_or(&0) * 100) as f64 / total;
            let keep_pct = (self.stats.by_category.get("keep").unwrap_or(&0) * 100) as f64 / total;

            println!("\nPercentages:");
            println!("   Delete: {:.1}%", delete_pct);
            println!("   Summarize: {:.1}%", summarize_pct);
            println!("   Keep: {:.1}%", keep_pct);
        }

        println!("\nEmail types found:");
        let mut types: Vec<_> = self.stats.by_type.iter().collect();
        types.sort_by(|a, b| b.1.cmp(a.1));
        for (email_type, count) in types {
            println!("   {}: {}", email_type, count);
        }

        println!("\nTop senders:");
        let mut senders: Vec<_> = self.stats.by_sender.iter().collect();
        senders.sort_by(|a, b| b.1.cmp(a.1));
        for (sender, count) in senders.iter().take(5) {
            println!("   {}: {}", sender, count);
        }

        println!("==================================================");
    }

    /// Get reference to categories.
    pub fn categories(&self) -> &HashMap<Category, Vec<EmailData>> {
        &self.categories
    }

    /// Get reference to stats.
    pub fn stats(&self) -> &SortStats {
        &self.stats
    }
}

/// Strip provider prefix from IMAP folder tag (e.g. `[Gmail]/Corbeille` → `Corbeille`).
fn strip_provider_prefix(tag: &str) -> &str {
    if let Some(pos) = tag.find("]/") {
        &tag[pos + 2..]
    } else {
        tag
    }
}

/// Score a folder tag. Trash/Spam = -5, Drafts = -3, others = 0.
pub fn folder_score(tag: &str) -> i32 {
    let leaf = strip_provider_prefix(tag);
    let lower = leaf.to_lowercase();
    match lower.as_str() {
        "corbeille" | "trash" | "bin" => -5,
        "spam" | "junk" | "pourriel" => -5,
        "brouillons" | "drafts" => -3,
        _ => 0,
    }
}

/// Known base folder names (INBOX, Sent, etc.) that should not get a subfolder bonus.
const BASE_FOLDERS: &[&str] = &[
    "inbox",
    "sent",
    "messages envoyés",
    "messages envoyes",
    "all mail",
    "tous les messages",
    "starred",
    "suivis",
    "important",
    "corbeille",
    "trash",
    "bin",
    "spam",
    "junk",
    "pourriel",
    "brouillons",
    "drafts",
];

/// Returns true if the tag matches a known base folder name (case-insensitive).
pub fn is_base_folder(tag: &str) -> bool {
    let leaf = strip_provider_prefix(tag);
    let lower = leaf.to_lowercase();
    BASE_FOLDERS.contains(&lower.as_str())
}

/// Move an email (or all emails from the same sender) to a new category in the report.
pub fn apply_category_change(
    report: &mut SortReport,
    file: &str,
    new_category: &str,
    scope: CategoryScope,
) -> anyhow::Result<()> {
    // Locate the entry and capture its current category and sender.
    let mut found_category: Option<String> = None;
    let mut found_sender: Option<String> = None;

    'outer: for (cat_key, entries) in report.categories.iter() {
        for entry in entries.iter() {
            if entry.file == file {
                found_category = Some(cat_key.clone());
                found_sender = Some(entry.sender.clone());
                break 'outer;
            }
        }
    }

    let current_category = found_category
        .ok_or_else(|| anyhow::anyhow!("email not found: {file}"))?;
    let sender = found_sender.unwrap_or_default();

    match scope {
        CategoryScope::Single => {
            // Remove from current category.
            let entry = {
                let entries = report.categories.entry(current_category.clone()).or_default();
                let pos = entries.iter().position(|e| e.file == file)
                    .ok_or_else(|| anyhow::anyhow!("email not found: {file}"))?;
                entries.remove(pos)
            };
            // Insert into target category.
            report.categories.entry(new_category.to_string()).or_default().push(entry);
        }
        CategoryScope::BySender => {
            // Collect all entries matching the sender across all categories.
            let mut to_move: Vec<(String, usize)> = Vec::new(); // (category_key, index)

            for (cat_key, entries) in report.categories.iter() {
                for (idx, entry) in entries.iter().enumerate() {
                    if entry.sender == sender {
                        to_move.push((cat_key.clone(), idx));
                    }
                }
            }

            // Remove in reverse-index order per category to keep indices valid.
            let mut by_category: HashMap<String, Vec<usize>> = HashMap::new();
            for (cat_key, idx) in to_move {
                by_category.entry(cat_key).or_default().push(idx);
            }

            let mut moved_entries: Vec<EmailSummary> = Vec::new();
            for (cat_key, mut indices) in by_category {
                indices.sort_unstable_by(|a, b| b.cmp(a)); // descending
                let entries = report.categories.entry(cat_key).or_default();
                for idx in indices {
                    moved_entries.push(entries.remove(idx));
                }
            }

            report
                .categories
                .entry(new_category.to_string())
                .or_default()
                .extend(moved_entries);
        }
    }

    Ok(())
}

/// Interactively review the delete/summarize buckets and let the user reassign entries.
/// Returns `Ok(true)` when the user confirms with "apply", `Ok(false)` on quit.
pub fn review_report(report: &mut SortReport) -> anyhow::Result<bool> {
    // Clone entries for display (avoid borrow issues while mutating later).
    let delete_entries: Vec<EmailSummary> = report
        .categories
        .get("delete")
        .cloned()
        .unwrap_or_default();
    let summarize_entries: Vec<EmailSummary> = report
        .categories
        .get("summarize")
        .cloned()
        .unwrap_or_default();

    if delete_entries.is_empty() && summarize_entries.is_empty() {
        println!("Nothing to apply.");
        return Ok(false);
    }

    let print_list = |del: &[EmailSummary], sum: &[EmailSummary]| {
        println!("=== DELETE ({}) ===", del.len());
        for (i, e) in del.iter().enumerate() {
            println!("[{i}] {} | {} | score: {}", e.sender, e.subject, e.score);
        }
        println!("=== SUMMARIZE ({}) ===", sum.len());
        for (i, e) in sum.iter().enumerate() {
            println!("[{}] {} | {} | score: {}", i + del.len(), e.sender, e.subject, e.score);
        }
    };

    print_list(&delete_entries, &summarize_entries);

    // Build flat index: (file, current_category).
    // Rebuilt on each iteration after mutations.
    let build_index = |report: &SortReport| -> Vec<(String, String)> {
        let del = report.categories.get("delete").cloned().unwrap_or_default();
        let sum = report.categories.get("summarize").cloned().unwrap_or_default();
        let mut idx: Vec<(String, String)> = del
            .into_iter()
            .map(|e| (e.file, "delete".to_string()))
            .collect();
        idx.extend(sum.into_iter().map(|e| (e.file, "summarize".to_string())));
        idx
    };

    let stdin = io::stdin();
    loop {
        print!("> ");
        io::stdout().flush().context("failed to flush stdout")?;

        let mut line = String::new();
        stdin.lock().read_line(&mut line).context("failed to read stdin")?;
        let input = line.trim();

        if input == "apply" {
            return Ok(true);
        }

        if input == "q" || input == "quit" {
            return Ok(false);
        }

        if let Ok(n) = input.parse::<usize>() {
            let index = build_index(report);
            if n < index.len() {
                let (file, _current_cat) = &index[n];
                // Show the entry.
                let all: Vec<&EmailSummary> = report.categories.values().flatten().collect();
                if let Some(entry) = all.iter().find(|e| &e.file == file) {
                    println!("{} | {} | score: {}", entry.sender, entry.subject, entry.score);
                }
                // Prompt new category.
                print!("New category (delete/summarize/keep): ");
                io::stdout().flush().context("failed to flush stdout")?;
                let mut cat_line = String::new();
                stdin.lock().read_line(&mut cat_line).context("failed to read stdin")?;
                let new_cat = cat_line.trim().to_string();

                // Prompt scope.
                print!("Scope ([e]mail / [s]ender): ");
                io::stdout().flush().context("failed to flush stdout")?;
                let mut scope_line = String::new();
                stdin.lock().read_line(&mut scope_line).context("failed to read stdin")?;
                let scope = match scope_line.trim() {
                    "s" | "sender" => CategoryScope::BySender,
                    _ => CategoryScope::Single,
                };

                apply_category_change(report, file, &new_cat, scope)
                    .context("failed to apply category change")?;

                // Reprint updated lists.
                let del = report.categories.get("delete").cloned().unwrap_or_default();
                let sum = report.categories.get("summarize").cloned().unwrap_or_default();
                print_list(&del, &sum);
            } else {
                println!("Index out of range.");
            }
        } else {
            println!("Unknown command. Enter a number, 'apply', or 'q'.");
        }
    }
}

/// Statistics produced by `apply_report`.
pub struct ApplyStats {
    pub deleted: usize,
    pub moved: usize,
    pub skipped: usize,
}

/// Compute the relative path from `from_dir` to `to` using only stdlib.
/// Both paths must be absolute for a correct result.
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

/// Rewrite attachment paths in a .md file's YAML frontmatter so they are
/// relative to `new_parent_dir` instead of `base_dir`.
/// Both `base_dir` and `new_parent_dir` must be absolute paths.
fn rewrite_attachment_paths(
    md_path: &Path,
    base_dir: &Path,
    new_parent_dir: &Path,
) -> anyhow::Result<()> {
    let content = std::fs::read_to_string(md_path)
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
            // Absolute attachment path from base_dir
            let abs = base_dir.join(path_str.replace('/', std::path::MAIN_SEPARATOR_STR));
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
    std::fs::write(md_path, new_content)
        .with_context(|| format!("failed to write updated frontmatter to {}", md_path.display()))?;
    Ok(())
}

/// Apply the decisions from a `SortReport`: trash deletes, move summarize, count keeps.
pub fn apply_report(report: &SortReport) -> anyhow::Result<ApplyStats> {
    let mut deleted = 0usize;
    let mut moved = 0usize;
    let mut skipped = 0usize;

    let raw_base = PathBuf::from(&report.base_directory);
    // Canonicalize so that relative_path_from works correctly with absolute components.
    let base = if raw_base.is_absolute() {
        raw_base.clone()
    } else {
        raw_base
            .canonicalize()
            .unwrap_or_else(|_| raw_base.clone())
    };

    // --- DELETE ---
    let delete_entries: Vec<EmailSummary> = report
        .categories
        .get("delete")
        .cloned()
        .unwrap_or_default();

    for email in &delete_entries {
        let md_path = base.join(&email.file);
        if !md_path.exists() {
            continue;
        }
        trash::delete(&md_path)
            .with_context(|| format!("failed to trash {}", email.file))?;

        // Check for attachments directory
        let stem = md_path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let attachments_dir = md_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(format!("{}_attachments", stem));

        if attachments_dir.exists() {
            print!(
                "Trash attachments for \"{}\"? [y/N]: ",
                email.subject
            );
            io::stdout().flush().context("failed to flush stdout")?;
            let mut answer = String::new();
            io::stdin()
                .lock()
                .read_line(&mut answer)
                .context("failed to read stdin")?;
            if answer.trim().starts_with(['y', 'Y']) {
                trash::delete(&attachments_dir).with_context(|| {
                    format!("failed to trash attachments for {}", email.file)
                })?;
            }
        }

        deleted += 1;
    }

    // --- SUMMARIZE ---
    let summarize_entries: Vec<EmailSummary> = report
        .categories
        .get("summarize")
        .cloned()
        .unwrap_or_default();

    for email in &summarize_entries {
        let md_path = base.join(&email.file);
        if !md_path.exists() {
            continue;
        }
        let to_summarize_dir = base
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("to-summarize");
        fs::create_dir_all(&to_summarize_dir)
            .context("failed to create to-summarize directory")?;
        let dest = to_summarize_dir.join(
            md_path.file_name().unwrap_or_default(),
        );
        if fs::rename(&md_path, &dest).is_err() {
            fs::copy(&md_path, &dest)
                .with_context(|| format!("failed to copy {} to to-summarize/", email.file))?;
            fs::remove_file(&md_path)
                .with_context(|| format!("failed to remove {} after copy", email.file))?;
        }
        // Rewrite attachment paths relative to new location
        if let Err(e) = rewrite_attachment_paths(&dest, &base, &to_summarize_dir) {
            eprintln!("warning: could not update attachment paths in {}: {}", dest.display(), e);
        }
        moved += 1;
    }

    // --- KEEP ---
    let keep_entries: Vec<EmailSummary> = report.categories.get("keep").cloned().unwrap_or_default();

    if !report.organize_by_type {
        skipped += keep_entries.len();
    } else {
        for email in &keep_entries {
            let md_path = base.join(&email.file);
            if !md_path.exists() {
                continue;
            }
            let type_name = if email.email_type.is_empty() {
                "unknown"
            } else {
                email.email_type.as_str()
            };
            let type_dir = base.join(type_name);

            // Skip if already in the correct subfolder
            if md_path.parent().is_some_and(|p| p == type_dir) {
                skipped += 1;
                continue;
            }

            fs::create_dir_all(&type_dir)
                .with_context(|| format!("failed to create {}/", type_name))?;
            let dest = type_dir.join(md_path.file_name().unwrap_or_default());
            if fs::rename(&md_path, &dest).is_err() {
                fs::copy(&md_path, &dest)
                    .with_context(|| format!("failed to copy {} to {}/", email.file, type_name))?;
                fs::remove_file(&md_path)
                    .with_context(|| format!("failed to remove {} after copy", email.file))?;
            }
            // Rewrite attachment paths relative to new location
            if let Err(e) = rewrite_attachment_paths(&dest, &base, &type_dir) {
                eprintln!("warning: could not update attachment paths in {}: {}", dest.display(), e);
            }
            moved += 1;
        }
    }

    Ok(ApplyStats { deleted, moved, skipped })
}

/// Extract frontmatter and body from markdown content.
fn extract_frontmatter(content: &str) -> Option<(String, String)> {
    if !content.starts_with("---") {
        return None;
    }

    let lines: Vec<&str> = content.lines().collect();
    let mut frontmatter_lines = Vec::new();
    let mut body_start = 0;
    let mut found_end = false;

    for (i, line) in lines.iter().enumerate() {
        if i == 0 {
            continue;
        }
        if line.trim() == "---" {
            body_start = i + 1;
            found_end = true;
            break;
        }
        frontmatter_lines.push(*line);
    }

    if !found_end {
        return None;
    }

    let frontmatter = frontmatter_lines.join("\n");
    let body = lines[body_start..].join("\n");

    Some((frontmatter, body))
}

/// Parse date string into DateTime.
fn parse_date(date_str: &str) -> Option<DateTime<FixedOffset>> {
    if date_str.is_empty() {
        return None;
    }

    // Try ISO format first
    if let Ok(dt) = DateTime::parse_from_rfc3339(date_str) {
        return Some(dt);
    }

    // Try other common formats
    let formats = ["%Y-%m-%d", "%Y-%m-%d %H:%M:%S", "%d/%m/%Y", "%m/%d/%Y"];
    for fmt in &formats {
        if let Ok(naive) = chrono::NaiveDate::parse_from_str(date_str, fmt) {
            let dt = naive
                .and_hms_opt(0, 0, 0)?
                .and_local_timezone(FixedOffset::east_opt(0)?)
                .single()?;
            return Some(dt);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_frontmatter() {
        let content = "---\nfrom: test@example.com\nsubject: Test\n---\n\nBody content";
        let result = extract_frontmatter(content);
        assert!(result.is_some());

        let (frontmatter, body) = result.unwrap();
        assert!(frontmatter.contains("from:"));
        assert!(body.contains("Body content"));
    }

    #[test]
    fn test_parse_date_iso() {
        let result = parse_date("2024-01-15T10:30:00+00:00");
        assert!(result.is_some());
    }

    #[test]
    fn test_parse_date_simple() {
        let result = parse_date("2024-01-15");
        assert!(result.is_some());
    }

    #[test]
    fn test_category_display() {
        assert_eq!(Category::Delete.to_string(), "delete");
        assert_eq!(Category::Summarize.to_string(), "summarize");
        assert_eq!(Category::Keep.to_string(), "keep");
    }

    fn make_email_data() -> EmailData {
        EmailData {
            file_path: PathBuf::from("test.md"),
            file_name: "test.md".to_string(),
            file_size: 1000,
            body_length: 2000,
            has_attachments: false,
            attachment_count: 0,
            date: None,
            age_days: Some(60),
            sender: "user@example.com".to_string(),
            recipients: vec![],
            subject: "Hello".to_string(),
            tags: vec![],
            email_type: EmailSortType::Direct,
            sender_count: 0,
            score: 0,
            score_breakdown: Vec::new(),
            category: Category::Summarize,
        }
    }

    #[test]
    fn test_toggle_use_type_weights_off() {
        let mut config = SortConfig::default();
        config.use_type_weights = false;
        let sorter = EmailSorter::new(PathBuf::from("/tmp"), config);
        let email = make_email_data();
        let (score_off, _) = sorter.calculate_score(&email, "hello");

        let mut config2 = SortConfig::default();
        config2.use_type_weights = true;
        let sorter2 = EmailSorter::new(PathBuf::from("/tmp"), config2);
        let (score_on, _) = sorter2.calculate_score(&email, "hello");

        // Direct type weight is +1, so score_on should be higher
        assert!(score_on > score_off);
    }

    #[test]
    fn test_toggle_use_age_scoring_off() {
        let mut config = SortConfig::default();
        config.use_age_scoring = false;
        config.use_type_weights = false;
        config.use_size_scoring = false;
        let sorter = EmailSorter::new(PathBuf::from("/tmp"), config);
        let mut email = make_email_data();
        email.age_days = Some(5); // recent
        let (score, _) = sorter.calculate_score(&email, "hello");

        let mut config2 = SortConfig::default();
        config2.use_age_scoring = true;
        config2.use_type_weights = false;
        config2.use_size_scoring = false;
        let sorter2 = EmailSorter::new(PathBuf::from("/tmp"), config2);
        let (score_on, _) = sorter2.calculate_score(&email, "hello");

        assert!(score_on > score, "age scoring should add bonus for recent emails");
    }

    #[test]
    fn test_toggle_use_body_keywords_off() {
        let mut config = SortConfig::default();
        config.use_body_keywords = false;
        config.use_type_weights = false;
        config.use_size_scoring = false;
        config.use_age_scoring = false;
        let sorter = EmailSorter::new(PathBuf::from("/tmp"), config);
        let email = make_email_data();
        let (score, _) = sorter.calculate_score(&email, "this is an important contract");
        assert_eq!(score, 0);
    }

    #[test]
    fn test_toggle_use_sender_rules_off() {
        let mut config = SortConfig::default();
        config.use_sender_rules = false;
        config.use_type_weights = false;
        config.use_size_scoring = false;
        config.use_age_scoring = false;
        config.delete_senders = vec!["spam@example.com".to_string()];
        let sorter = EmailSorter::new(PathBuf::from("/tmp"), config);
        let mut email = make_email_data();
        email.sender = "spam@example.com".to_string();
        let (score, _) = sorter.calculate_score(&email, "hello");
        assert_eq!(score, 0);
    }

    #[test]
    fn test_folder_score_trash() {
        assert_eq!(folder_score("Corbeille"), -5);
        assert_eq!(folder_score("[Gmail]/Corbeille"), -5);
        assert_eq!(folder_score("Trash"), -5);
        assert_eq!(folder_score("[Gmail]/Trash"), -5);
    }

    #[test]
    fn test_folder_score_spam() {
        assert_eq!(folder_score("Spam"), -5);
        assert_eq!(folder_score("[Gmail]/Spam"), -5);
        assert_eq!(folder_score("Junk"), -5);
    }

    #[test]
    fn test_folder_score_drafts() {
        assert_eq!(folder_score("Brouillons"), -3);
        assert_eq!(folder_score("[Gmail]/Drafts"), -3);
    }

    #[test]
    fn test_folder_score_neutral() {
        assert_eq!(folder_score("INBOX"), 0);
        assert_eq!(folder_score("MyFolder"), 0);
        assert_eq!(folder_score("[Gmail]/All Mail"), 0);
    }

    #[test]
    fn test_is_base_folder() {
        assert!(is_base_folder("INBOX"));
        assert!(is_base_folder("[Gmail]/Sent"));
        assert!(is_base_folder("[Gmail]/Tous les messages"));
        assert!(is_base_folder("Drafts"));
        assert!(!is_base_folder("Projects"));
        assert!(!is_base_folder("Work/Reports"));
    }

    #[test]
    fn test_folder_score_applied_in_calculate() {
        let mut config = SortConfig::default();
        config.use_folder_score = true;
        config.use_type_weights = false;
        config.use_size_scoring = false;
        config.use_age_scoring = false;
        let sorter = EmailSorter::new(PathBuf::from("/tmp"), config);
        let mut email = make_email_data();
        email.tags = vec!["[Gmail]/Corbeille".to_string()];
        let (score, _) = sorter.calculate_score(&email, "hello");
        assert!(score < 0, "trash folder should give negative score, got {}", score);
    }

    #[test]
    fn test_subfolder_bonus_applied() {
        let mut config = SortConfig::default();
        config.use_subfolder_bonus = true;
        config.use_folder_score = false;
        config.use_type_weights = false;
        config.use_size_scoring = false;
        config.use_age_scoring = false;
        let sorter = EmailSorter::new(PathBuf::from("/tmp"), config);
        let mut email = make_email_data();
        email.tags = vec!["Projects/Important".to_string()];
        let (score, _) = sorter.calculate_score(&email, "hello");
        assert_eq!(score, 2, "subfolder bonus should be +2");
    }

    #[test]
    fn test_no_subfolder_bonus_for_inbox() {
        let mut config = SortConfig::default();
        config.use_subfolder_bonus = true;
        config.use_folder_score = false;
        config.use_type_weights = false;
        config.use_size_scoring = false;
        config.use_age_scoring = false;
        let sorter = EmailSorter::new(PathBuf::from("/tmp"), config);
        let mut email = make_email_data();
        email.tags = vec!["INBOX".to_string()];
        let (score, _) = sorter.calculate_score(&email, "hello");
        assert_eq!(score, 0, "base folder should not get subfolder bonus");
    }

    #[test]
    fn test_toggle_use_subject_rules_off() {
        let mut config = SortConfig::default();
        config.use_subject_rules = false;
        config.use_type_weights = false;
        config.use_size_scoring = false;
        config.use_age_scoring = false;
        let sorter = EmailSorter::new(PathBuf::from("/tmp"), config);
        let mut email = make_email_data();
        email.subject = "newsletter promotion".to_string();
        let (score, _) = sorter.calculate_score(&email, "hello");
        assert_eq!(score, 0);
    }

    #[test]
    fn test_newsletter_detection_via_frontmatter() {
        let config = SortConfig::default();
        let sorter = EmailSorter::new(PathBuf::from("/tmp"), config);
        let fm: Value = serde_yaml::from_str("email_type: newsletter\nsubject: Hello").unwrap();
        let email_type = sorter.determine_email_type("Hello", &fm, "plain body");
        assert_eq!(email_type, EmailSortType::Newsletter);
    }

    #[test]
    fn test_newsletter_detection_via_body_unsubscribe() {
        let config = SortConfig::default();
        let sorter = EmailSorter::new(PathBuf::from("/tmp"), config);
        let fm: Value = serde_yaml::from_str("subject: Hello").unwrap();
        let body = "Click here to https://example.com/unsubscribe?id=123";
        let email_type = sorter.determine_email_type("Hello", &fm, body);
        assert_eq!(email_type, EmailSortType::Newsletter);
    }

    #[test]
    fn test_newsletter_detection_direct_no_unsubscribe() {
        let config = SortConfig::default();
        let sorter = EmailSorter::new(PathBuf::from("/tmp"), config);
        let fm: Value = serde_yaml::from_str("subject: Hello").unwrap();
        let email_type = sorter.determine_email_type("Hello", &fm, "just a normal email");
        assert_eq!(email_type, EmailSortType::Direct);
    }

    #[test]
    fn test_delete_newsletters_toggle_on() {
        let mut config = SortConfig::default();
        config.delete_newsletters = true;
        let sorter = EmailSorter::new(PathBuf::from("/tmp"), config);
        let mut email = make_email_data();
        email.email_type = EmailSortType::Newsletter;
        let category = sorter.determine_category(&email, "body");
        assert_eq!(category, Category::Delete);
    }

    #[test]
    fn test_recurring_sender_1_email_no_malus() {
        let mut config = SortConfig::default();
        config.penalize_recurring = true;
        config.use_type_weights = false;
        config.use_size_scoring = false;
        config.use_age_scoring = false;
        let sorter = EmailSorter::new(PathBuf::from("/tmp"), config);
        let mut email = make_email_data();
        email.sender_count = 1;
        let (score, _) = sorter.calculate_score(&email, "hello");
        assert_eq!(score, 0, "1 email = 0 malus");
    }

    #[test]
    fn test_recurring_sender_3_emails_malus_1() {
        let mut config = SortConfig::default();
        config.penalize_recurring = true;
        config.use_type_weights = false;
        config.use_size_scoring = false;
        config.use_age_scoring = false;
        let sorter = EmailSorter::new(PathBuf::from("/tmp"), config);
        let mut email = make_email_data();
        email.sender_count = 3;
        let (score, _) = sorter.calculate_score(&email, "hello");
        assert_eq!(score, -1, "3 emails = -1 malus");
    }

    #[test]
    fn test_recurring_sender_8_emails_malus_2() {
        let mut config = SortConfig::default();
        config.penalize_recurring = true;
        config.use_type_weights = false;
        config.use_size_scoring = false;
        config.use_age_scoring = false;
        let sorter = EmailSorter::new(PathBuf::from("/tmp"), config);
        let mut email = make_email_data();
        email.sender_count = 8;
        let (score, _) = sorter.calculate_score(&email, "hello");
        assert_eq!(score, -2, "8 emails = -2 malus");
    }

    #[test]
    fn test_recurring_sender_25_emails_malus_3() {
        let mut config = SortConfig::default();
        config.penalize_recurring = true;
        config.use_type_weights = false;
        config.use_size_scoring = false;
        config.use_age_scoring = false;
        let sorter = EmailSorter::new(PathBuf::from("/tmp"), config);
        let mut email = make_email_data();
        email.sender_count = 25;
        let (score, _) = sorter.calculate_score(&email, "hello");
        assert_eq!(score, -3, "25 emails = -3 malus");
    }

    #[test]
    fn test_score_breakdown_contains_expected_entries() {
        let mut config = SortConfig::default();
        config.use_folder_score = true;
        config.use_type_weights = true;
        config.penalize_recurring = true;
        let sorter = EmailSorter::new(PathBuf::from("/tmp"), config);
        let mut email = make_email_data();
        email.tags = vec!["[Gmail]/Corbeille".to_string()];
        email.sender_count = 8;
        let (_, breakdown) = sorter.calculate_score(&email, "hello");

        let names: Vec<&str> = breakdown.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"type"), "breakdown should contain 'type', got {:?}", names);
        assert!(names.contains(&"folder"), "breakdown should contain 'folder', got {:?}", names);
        assert!(names.contains(&"recurring"), "breakdown should contain 'recurring', got {:?}", names);
    }

    #[test]
    fn test_score_breakdown_folder_value() {
        let mut config = SortConfig::default();
        config.use_folder_score = true;
        config.use_type_weights = false;
        config.use_size_scoring = false;
        config.use_age_scoring = false;
        let sorter = EmailSorter::new(PathBuf::from("/tmp"), config);
        let mut email = make_email_data();
        email.tags = vec!["[Gmail]/Corbeille".to_string()];
        let (_, breakdown) = sorter.calculate_score(&email, "hello");
        let folder_entry = breakdown.iter().find(|(n, _)| n == "folder");
        assert_eq!(folder_entry, Some(&("folder".to_string(), -5)));
    }

    #[test]
    fn test_delete_newsletters_toggle_off() {
        let mut config = SortConfig::default();
        config.delete_newsletters = false;
        let sorter = EmailSorter::new(PathBuf::from("/tmp"), config);
        let mut email = make_email_data();
        email.email_type = EmailSortType::Newsletter;
        email.score = 0;
        let category = sorter.determine_category(&email, "body");
        // Without delete_newsletters, newsletter still goes through normal flow
        assert_ne!(category, Category::Keep);
    }

    // --- apply_category_change / review_report tests ---

    fn make_test_report(entries: Vec<(&str, &str, &str)>) -> SortReport {
        let mut categories: HashMap<String, Vec<EmailSummary>> = HashMap::new();
        for (category, file, sender) in entries {
            categories
                .entry(category.to_string())
                .or_default()
                .push(EmailSummary {
                    file: file.to_string(),
                    subject: "Subject".to_string(),
                    sender: sender.to_string(),
                    date: "2024-01-01".to_string(),
                    score: 0,
                    email_type: "direct".to_string(),
                    size: 1000,
                    attachments: 0,
                    breakdown: vec![],
                });
        }
        SortReport {
            base_directory: "/tmp".to_string(),
            organize_by_type: false,
            summary: SortSummary {
                total_emails: 0,
                categories: HashMap::new(),
                recommendations: HashMap::new(),
            },
            details: SortDetails {
                by_type: HashMap::new(),
                by_sender: vec![],
                by_date: HashMap::new(),
            },
            categories,
        }
    }

    #[test]
    fn test_apply_category_change_single() {
        let mut report = make_test_report(vec![("delete", "email1.md", "alice@example.com")]);
        apply_category_change(&mut report, "email1.md", "keep", CategoryScope::Single).unwrap();
        assert!(report.categories.get("keep").is_some_and(|v| v.iter().any(|e| e.file == "email1.md")));
    }

    #[test]
    fn test_apply_category_change_single_removed_from_source() {
        let mut report = make_test_report(vec![("delete", "email1.md", "alice@example.com")]);
        apply_category_change(&mut report, "email1.md", "keep", CategoryScope::Single).unwrap();
        assert!(!report.categories.get("delete").is_some_and(|v| v.iter().any(|e| e.file == "email1.md")));
    }

    #[test]
    fn test_apply_category_change_by_sender() {
        let mut report = make_test_report(vec![
            ("delete", "email1.md", "bob@example.com"),
            ("delete", "email2.md", "bob@example.com"),
        ]);
        apply_category_change(&mut report, "email1.md", "keep", CategoryScope::BySender).unwrap();
        let keep = report.categories.get("keep").cloned().unwrap_or_default();
        assert!(keep.iter().any(|e| e.file == "email2.md"));
    }

    #[test]
    fn test_apply_category_change_not_found() {
        let mut report = make_test_report(vec![("delete", "email1.md", "alice@example.com")]);
        let result = apply_category_change(&mut report, "missing.md", "keep", CategoryScope::Single);
        assert!(result.is_err());
    }

    // --- relative_path_from tests ---

    #[test]
    fn test_relative_path_from_sibling_dir() {
        // /base/emails -> /base/attachments/file.pdf => ../attachments/file.pdf
        let from = Path::new("/base/emails");
        let to = Path::new("/base/attachments/file.pdf");
        let rel = relative_path_from(from, to);
        assert_eq!(rel, PathBuf::from("../attachments/file.pdf"));
    }

    #[test]
    fn test_relative_path_from_same_parent() {
        // /base/to-summarize -> /base/attachments/INBOX/doc.pdf => ../attachments/INBOX/doc.pdf
        let from = Path::new("/base/to-summarize");
        let to = Path::new("/base/attachments/INBOX/doc.pdf");
        let rel = relative_path_from(from, to);
        assert_eq!(rel, PathBuf::from("../attachments/INBOX/doc.pdf"));
    }

    #[test]
    fn test_relative_path_from_deeply_nested() {
        // /a/b/c -> /a/x/y.txt => ../../x/y.txt
        let from = Path::new("/a/b/c");
        let to = Path::new("/a/x/y.txt");
        let rel = relative_path_from(from, to);
        assert_eq!(rel, PathBuf::from("../../x/y.txt"));
    }

    // --- rewrite_attachment_paths tests ---

    #[test]
    fn test_rewrite_attachment_paths_no_frontmatter() {
        use tempfile::TempDir;
        let temp = TempDir::new().unwrap();
        let md = temp.path().join("email.md");
        std::fs::write(&md, "No frontmatter here").unwrap();
        // Should succeed silently (no frontmatter block)
        rewrite_attachment_paths(&md, temp.path(), temp.path()).unwrap();
        let content = std::fs::read_to_string(&md).unwrap();
        assert_eq!(content, "No frontmatter here");
    }

    #[test]
    fn test_rewrite_attachment_paths_no_attachments() {
        use tempfile::TempDir;
        let temp = TempDir::new().unwrap();
        let md = temp.path().join("email.md");
        std::fs::write(&md, "---\nsubject: Hello\n---\nBody").unwrap();
        // No attachments: content should be preserved as-is (no modification needed)
        rewrite_attachment_paths(&md, temp.path(), temp.path()).unwrap();
        let content = std::fs::read_to_string(&md).unwrap();
        assert!(content.contains("subject: Hello"));
    }

    #[test]
    fn test_rewrite_attachment_paths_updates_path() {
        use tempfile::TempDir;
        let temp = TempDir::new().unwrap();
        // Simulate: base_dir = temp/emails, file moved to temp/emails/to-summarize/
        let base_dir = temp.path().join("emails");
        let new_parent = base_dir.join("to-summarize");
        std::fs::create_dir_all(&new_parent).unwrap();
        // Create a dummy attachment path that would be referenced from base_dir
        let att_dir = base_dir.join("attachments").join("INBOX");
        std::fs::create_dir_all(&att_dir).unwrap();
        std::fs::write(att_dir.join("file.pdf"), b"dummy").unwrap();

        let md = new_parent.join("email.md");
        std::fs::write(
            &md,
            "---\nsubject: Hello\nattachments:\n  - attachments/INBOX/file.pdf\n---\nBody",
        )
        .unwrap();

        rewrite_attachment_paths(&md, &base_dir, &new_parent).unwrap();

        let content = std::fs::read_to_string(&md).unwrap();
        // Path should now be relative from to-summarize/ back up to attachments/
        assert!(
            content.contains("../attachments/INBOX/file.pdf"),
            "expected ../attachments/INBOX/file.pdf, got: {content}"
        );
        // Old path must be gone
        assert!(
            !content.contains("  - attachments/INBOX/file.pdf"),
            "old path should have been replaced, got: {content}"
        );
    }
}
