//! Action handlers for system tray menu items.
//!
//! This module provides the functions that are called when users
//! interact with the system tray menu.

use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::thread;

use anyhow::{Context, Result};
use rfd;

use crate::config::{self, Config, SortConfig};
use crate::email_export::ImapExporter;
use crate::sort_emails::{Category, EmailSorter};
use crate::thunderbird;

/// Result of an action, sent back to the main thread for notification.
#[derive(Debug, Clone)]
pub enum ActionResult {
    /// (title, message)
    Success(String, String),
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
            Ok(message) => ActionResult::Success("Export terminé".to_string(), message),
            Err(e) => ActionResult::Error(format!("Export error: {}", e)),
        };
        let _ = result_sender.send(action_result);
    });
}

fn run_export(account_name: &str) -> Result<String> {
    dotenv::from_path(config::env_file_path()).ok();

    let config = Config::load(&config::accounts_yaml_path()).context("Failed to load configuration")?;

    let account = config
        .get_account(account_name)
        .context(format!("Account '{}' not found", account_name))?
        .clone();

    if account.password.is_none() {
        return Err(anyhow::anyhow!(
            "No password found for {}. Check {}",
            account_name,
            config::env_file_path().display()
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
            Ok(message) => ActionResult::Success("Tri terminé".to_string(), message),
            Err(e) => ActionResult::Error(format!("Sort error: {}", e)),
        };
        let _ = result_sender.send(action_result);
    });
}

fn run_sort(account_name: &str) -> Result<String> {
    dotenv::from_path(config::env_file_path()).ok();

    let config = Config::load(&config::accounts_yaml_path()).context("Failed to load configuration")?;

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
/// Shows a Yes/No dialog asking whether to also extract passwords,
/// then runs the import in a background thread.
pub fn action_import_thunderbird(result_sender: Sender<ActionResult>) {
    let dialog_result = rfd::MessageDialog::new()
        .set_title("Import Thunderbird")
        .set_description(
            "Importer les comptes depuis Thunderbird ?\n\n\
             • Oui    — importer comptes + mots de passe\n\
             • Non    — importer les comptes uniquement\n\
             • Annuler — ne rien faire\n\n\
             (Thunderbird doit être fermé pour extraire les mots de passe)",
        )
        .set_buttons(rfd::MessageButtons::YesNoCancel)
        .show();

    let extract_passwords = match dialog_result {
        rfd::MessageDialogResult::Yes => true,
        rfd::MessageDialogResult::No => false,
        _ => return, // Annuler
    };

    thread::spawn(move || {
        let result = run_import_thunderbird(extract_passwords);
        let action_result = match result {
            Ok(message) => ActionResult::Imported(message),
            Err(e) => ActionResult::Error(format!("Import error: {}", e)),
        };
        let _ = result_sender.send(action_result);
    });
}

fn run_import_thunderbird(extract_passwords: bool) -> Result<String> {
    let profiles = thunderbird::list_profiles().context("Could not find Thunderbird profiles")?;

    // Same logic as CLI: prefer default profile that has prefs.js
    let has_prefs = |p: &thunderbird::ThunderbirdProfile| p.path.join("prefs.js").exists();
    let profile = profiles
        .iter()
        .find(|p| p.is_default && has_prefs(p))
        .or_else(|| profiles.iter().find(|p| has_prefs(p)))
        .cloned()
        .context("No usable Thunderbird profiles found (no prefs.js)")?;

    let accounts = thunderbird::extract_accounts(&profile)
        .context("Failed to extract accounts from Thunderbird")?;

    if accounts.is_empty() {
        return Ok("No IMAP accounts found in Thunderbird".to_string());
    }

    let yaml_content = thunderbird::generate_accounts_yaml(&accounts);
    let output_path = config::accounts_yaml_path();

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(&output_path, &yaml_content)?;

    let mut message = format!("Imported {} account(s)", accounts.len());

    if extract_passwords {
        match thunderbird::extract_passwords(&profile, None) {
            Ok(passwords) if !passwords.is_empty() => {
                let env_path = config::env_file_path();
                if let Some(parent) = env_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                match thunderbird::write_passwords_to_env(&accounts, &passwords, &env_path) {
                    Ok(n) => message.push_str(&format!("\n{} mot(s) de passe écrits", n)),
                    Err(e) => message.push_str(&format!("\nImpossible d'écrire .env : {}", e)),
                }
            }
            Ok(_) => message.push_str("\nAucun mot de passe trouvé dans Thunderbird"),
            Err(e) => message.push_str(&format!("\nExtraction des mots de passe échouée : {}", e)),
        }
    }

    Ok(message)
}

/// Open a folder picker and update export_directory for all accounts.
pub fn action_choose_export_dir(result_sender: Sender<ActionResult>) {
    // FileDialog must run on the main thread on some platforms — keep it here
    let folder = rfd::FileDialog::new()
        .set_title("Choisir le répertoire d'export")
        .pick_folder();

    let Some(base_dir) = folder else {
        return; // user cancelled
    };

    thread::spawn(move || {
        let result = set_export_dir(&base_dir);
        let action_result = match result {
            Ok(msg) => ActionResult::Success("Répertoire d'export".to_string(), msg),
            Err(e) => ActionResult::Error(format!("Erreur répertoire d'export : {}", e)),
        };
        let _ = result_sender.send(action_result);
    });
}

fn set_export_dir(base_dir: &std::path::Path) -> Result<String> {
    let config_path = config::accounts_yaml_path();

    if !config_path.exists() {
        return Err(anyhow::anyhow!(
            "Aucun compte configuré. Importez d'abord depuis Thunderbird."
        ));
    }

    // Manipulate raw YAML to avoid serialising passwords back to disk
    let content = std::fs::read_to_string(&config_path)?;
    let mut yaml: serde_yaml::Value = serde_yaml::from_str(&content)?;

    let mut count = 0usize;
    if let Some(accounts) = yaml
        .get_mut("accounts")
        .and_then(|a| a.as_sequence_mut())
    {
        for account in accounts.iter_mut() {
            if let Some(name) = account.get("name").and_then(|n| n.as_str()) {
                let export_dir = base_dir.join(name).to_string_lossy().replace('\\', "/");
                account["export_directory"] = serde_yaml::Value::String(export_dir);
                count += 1;
            }
        }
    }

    std::fs::write(&config_path, serde_yaml::to_string(&yaml)?)?;

    Ok(format!(
        "{} compte(s) mis à jour → {}",
        count,
        base_dir.display()
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
    let config_path = config::accounts_yaml_path();

    if !config_path.exists() {
        // Create a template if it doesn't exist
        if let Some(parent) = config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
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
    dotenv::from_path(config::env_file_path()).ok();

    let config_path = config::accounts_yaml_path();

    if !config_path.exists() {
        return Ok(Vec::new());
    }

    let config = Config::load(&config_path)?;
    Ok(config.list_accounts().into_iter().map(String::from).collect())
}
