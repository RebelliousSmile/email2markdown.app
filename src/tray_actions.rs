//! Action handlers for system tray menu items.
//!
//! This module provides the functions that are called when users
//! interact with the system tray menu.

use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::thread;

use anyhow::{Context, Result};

use crate::config::{Config, SortConfig};
use crate::email_export::ImapExporter;
use crate::sort_emails::{Category, EmailSorter};
use crate::thunderbird;

/// Result of an action, sent back to the main thread for notification.
#[derive(Debug, Clone)]
pub enum ActionResult {
    Success(String),
    /// Import completed — the main thread should rebuild the tray menu.
    Imported(String),
    Error(String),
}

/// Export emails for a specific account.
///
/// Runs in a separate thread to avoid blocking the UI.
pub fn action_export(account_name: String, result_sender: Sender<ActionResult>) {
    thread::spawn(move || {
        let result = run_export(&account_name);
        let action_result = match result {
            Ok(message) => ActionResult::Success(message),
            Err(e) => ActionResult::Error(format!("Export error: {}", e)),
        };
        let _ = result_sender.send(action_result);
    });
}

fn run_export(account_name: &str) -> Result<String> {
    // Load .env for passwords
    dotenv::dotenv().ok();

    let config_path = PathBuf::from("config/accounts.yaml");
    let config = Config::load(&config_path).context("Failed to load configuration")?;

    let account = config
        .get_account(account_name)
        .context(format!("Account '{}' not found", account_name))?
        .clone();

    if account.password.is_none() {
        return Err(anyhow::anyhow!(
            "No password found for {}. Check your .env file.",
            account_name
        ));
    }

    let mut exporter = ImapExporter::new(account.clone(), false);
    exporter.connect().context("Failed to connect to IMAP server")?;

    let results = exporter
        .export_account()
        .context("Export failed")?;

    exporter.disconnect().ok();

    let total_exported: usize = results.values().map(|s| s.exported).sum();
    let total_skipped: usize = results.values().map(|s| s.skipped).sum();
    let total_errors: usize = results.values().map(|s| s.errors).sum();

    Ok(format!(
        "{}: {} exported, {} skipped, {} errors",
        account_name, total_exported, total_skipped, total_errors
    ))
}

/// Sort emails for a specific account.
///
/// Runs in a separate thread to avoid blocking the UI.
pub fn action_sort(account_name: String, result_sender: Sender<ActionResult>) {
    thread::spawn(move || {
        let result = run_sort(&account_name);
        let action_result = match result {
            Ok(message) => ActionResult::Success(message),
            Err(e) => ActionResult::Error(format!("Sort error: {}", e)),
        };
        let _ = result_sender.send(action_result);
    });
}

fn run_sort(account_name: &str) -> Result<String> {
    dotenv::dotenv().ok();

    let config_path = PathBuf::from("config/accounts.yaml");
    let config = Config::load(&config_path).context("Failed to load configuration")?;

    let account = config
        .get_account(account_name)
        .context(format!("Account '{}' not found", account_name))?;

    let sort_directory = PathBuf::from(&account.export_directory);
    let sort_config = SortConfig::default();

    let mut sorter = EmailSorter::new(sort_directory.clone(), sort_config);
    sorter.sort_emails()?;

    let report = sorter.generate_report();
    let report_path = sort_directory.join("sort_report.json");
    sorter.save_report(&report, report_path.to_str().unwrap_or("sort_report.json"))?;

    let categories = sorter.categories();
    let delete_count = categories.get(&Category::Delete).map(|v| v.len()).unwrap_or(0);
    let summarize_count = categories.get(&Category::Summarize).map(|v| v.len()).unwrap_or(0);
    let keep_count = categories.get(&Category::Keep).map(|v| v.len()).unwrap_or(0);

    Ok(format!(
        "{}: {} delete, {} summarize, {} keep",
        account_name, delete_count, summarize_count, keep_count
    ))
}

/// Import accounts from Thunderbird.
///
/// Runs in a separate thread to avoid blocking the UI.
pub fn action_import_thunderbird(result_sender: Sender<ActionResult>) {
    thread::spawn(move || {
        let result = run_import_thunderbird();
        let action_result = match result {
            Ok(message) => ActionResult::Imported(message),
            Err(e) => ActionResult::Error(format!("Import error: {}", e)),
        };
        let _ = result_sender.send(action_result);
    });
}

fn run_import_thunderbird() -> Result<String> {
    let profiles = thunderbird::list_profiles().context("Could not find Thunderbird profiles")?;

    let profile = profiles
        .into_iter()
        .find(|p| p.is_default)
        .context("No default Thunderbird profile found")?;

    let accounts = thunderbird::extract_accounts(&profile)
        .context("Failed to extract accounts from Thunderbird")?;

    if accounts.is_empty() {
        return Ok("No IMAP accounts found in Thunderbird".to_string());
    }

    let yaml_content = thunderbird::generate_accounts_yaml(&accounts);
    let output_path = PathBuf::from("config/accounts.yaml");

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(&output_path, &yaml_content)?;

    Ok(format!(
        "Imported {} account(s) to config/accounts.yaml",
        accounts.len()
    ))
}

/// Open the documentation (README.md) in the default viewer.
pub fn action_open_documentation() -> Result<()> {
    let readme_paths = [
        "README.md",
        "docs/README.md",
    ];

    for path in &readme_paths {
        let readme_path = PathBuf::from(path);
        if readme_path.exists() {
            open::that(&readme_path).context("Failed to open documentation")?;
            return Ok(());
        }
    }

    Err(anyhow::anyhow!("README.md not found"))
}

/// Open the configuration file in the default editor.
pub fn action_open_config() -> Result<()> {
    let config_path = PathBuf::from("config/accounts.yaml");

    if !config_path.exists() {
        // Create a template if it doesn't exist
        std::fs::create_dir_all("config")?;
        let template = r#"# Email to Markdown - Accounts Configuration
# Add your IMAP accounts here

accounts:
  - name: Example
    server: imap.example.com
    port: 993
    username: user@example.com
    export_directory: ./exports/example
    quote_depth: 1
    skip_existing: true
    collect_contacts: false
    skip_signature_images: false
    delete_after_export: false
    ignored_folders:
      - Drafts
      - Trash
      - Spam
"#;
        std::fs::write(&config_path, template)?;
    }

    open::that(&config_path).context("Failed to open configuration file")?;
    Ok(())
}

/// Get the list of configured accounts.
pub fn get_account_names() -> Result<Vec<String>> {
    dotenv::dotenv().ok();

    let config_path = PathBuf::from("config/accounts.yaml");

    if !config_path.exists() {
        return Ok(Vec::new());
    }

    let config = Config::load(&config_path)?;
    Ok(config.list_accounts().into_iter().map(String::from).collect())
}
