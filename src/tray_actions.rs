//! Action handlers for system tray menu items.
//!
//! This module provides the functions that are called when users
//! interact with the system tray menu.

use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::thread;

use anyhow::{Context, Result};
use rfd;

use crate::progress::ProgressUpdate;

use crate::config::{self, Config, SortConfig};
use crate::email_export::{self, ImapExporter};
use crate::fix_yaml;
use crate::sort_emails::EmailSorter;
use crate::thunderbird;

/// Result of an action, sent back to the main thread for notification.
#[derive(Debug, Clone)]
pub enum ActionResult {
    /// (title, message)
    Success(String, String),
    /// Import completed — the main thread should rebuild the tray menu.
    Imported(String),
    Error(String),
    /// Sort completed — the main thread should open the review window.
    SortCompleted {
        account: String,
        report_path: std::path::PathBuf,
    },
}

fn classify_error(e: &anyhow::Error) -> Option<String> {
    let msg = format!("{:#}", e).to_lowercase();
    if msg.contains("no password found")
        || msg.contains("not configured")
        || msg.contains("failed to load configuration")
        || (msg.contains("account") && msg.contains("not found"))
    {
        Some("Ouvrir la configuration".to_string())
    } else {
        None
    }
}

/// Export emails for a specific account.
///
/// Runs in a separate thread to avoid blocking the UI.
pub fn action_export(account_name: String, _result_sender: Sender<ActionResult>) {
    let (progress_tx, progress_rx) = mpsc::channel::<ProgressUpdate>();

    thread::spawn(move || {
        crate::tray_progress_window::open(
            "Export",
            progress_rx,
            None,
            Some(Box::new(|| { let _ = action_open_config(); })),
        );
    });

    thread::spawn(move || {
        let progress_tx_clone = progress_tx.clone();
        let on_progress = move |current: usize, total: usize, label: &str| {
            let _ = progress_tx_clone.send(ProgressUpdate::Step {
                current,
                total,
                message: label.to_string(),
            });
        };
        match run_export(&account_name, Some(&on_progress)) {
            Ok(summary) => {
                let _ = progress_tx.send(ProgressUpdate::Done { summary });
            }
            Err(e) => {
                let _ = progress_tx.send(ProgressUpdate::Error {
                    message: format!("Export error: {}", e),
                    action_label: classify_error(&e),
                });
            }
        }
    });
}

fn run_export(
    account_name: &str,
    on_progress: Option<&(dyn Fn(usize, usize, &str) + Send + Sync)>,
) -> Result<String> {
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
        .export_account(on_progress)
        .context("Export failed")?;

    exporter.disconnect().ok();

    let total_exported: usize = results.values().map(|s| s.exported).sum();
    let total_skipped: usize = results.values().map(|s| s.skipped).sum();
    let total_errors: usize = results.values().map(|s| s.errors).sum();

    Ok(format!(
        "Export terminé — {} exportés, {} ignorés, {} erreurs",
        total_exported, total_skipped, total_errors
    ))
}

/// Sort emails for a specific account.
///
/// Runs in a separate thread to avoid blocking the UI.
pub fn action_sort(account_name: String, result_sender: Sender<ActionResult>) {
    let (progress_tx, progress_rx) = mpsc::channel::<ProgressUpdate>();
    let (result_tx, result_rx) = mpsc::sync_channel::<ActionResult>(1);

    let on_close = Some(Box::new(move || {
        if let Ok(r) = result_rx.try_recv() {
            let _ = result_sender.send(r);
        }
    }) as Box<dyn FnOnce() + Send>);

    thread::spawn(move || {
        crate::tray_progress_window::open(
            "Sort",
            progress_rx,
            on_close,
            Some(Box::new(|| { let _ = action_open_config(); })),
        );
    });

    thread::spawn(move || {
        let progress_tx_clone = progress_tx.clone();
        let on_progress = move |current: usize, total: usize, label: &str| {
            let _ = progress_tx_clone.send(ProgressUpdate::Step {
                current,
                total,
                message: label.to_string(),
            });
        };
        match run_sort(&account_name, Some(&on_progress)) {
            Ok((report_path, email_count)) => {
                if email_count > 0 {
                    let _ = result_tx.send(ActionResult::SortCompleted {
                        account: account_name,
                        report_path,
                    });
                    let _ = progress_tx.send(ProgressUpdate::Done {
                        summary: format!("{} email(s) à réviser", email_count),
                    });
                } else {
                    let _ = progress_tx.send(ProgressUpdate::Done {
                        summary: "Tri terminé — rien à réviser".to_string(),
                    });
                }
            }
            Err(e) => {
                let _ = progress_tx.send(ProgressUpdate::Error {
                    message: format!("Sort error: {}", e),
                    action_label: classify_error(&e),
                });
            }
        }
    });
}

fn run_sort(
    account_name: &str,
    on_progress: Option<&(dyn Fn(usize, usize, &str) + Send + Sync)>,
) -> Result<(PathBuf, usize)> {
    dotenv::from_path(config::env_file_path()).ok();

    let config = Config::load(&config::accounts_yaml_path()).context("Failed to load configuration")?;

    let account = config
        .get_account(account_name)
        .context(format!("Account '{}' not found", account_name))?;

    let sort_directory = PathBuf::from(&account.export_directory);
    let sort_config = SortConfig::default();

    let mut sorter = EmailSorter::new(sort_directory.clone(), sort_config);
    sorter.sort_emails(on_progress)?;

    let report = sorter.generate_report();
    let email_count: usize = report.categories.values().map(|v| v.len()).sum();
    let report_path = sort_directory.join("sort_report.json");
    let path_str = report_path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("sort report path contains non-UTF-8 characters"))?;
    sorter.save_report(&report, path_str).context("failed to save sort report")?;

    Ok((report_path, email_count))
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

    let (progress_tx, progress_rx) = mpsc::channel::<ProgressUpdate>();
    let (result_tx, result_rx) = mpsc::sync_channel::<ActionResult>(1);

    let on_close = Some(Box::new(move || {
        if let Ok(r) = result_rx.try_recv() {
            let _ = result_sender.send(r);
        }
    }) as Box<dyn FnOnce() + Send>);

    thread::spawn(move || {
        crate::tray_progress_window::open(
            "Import Thunderbird",
            progress_rx,
            on_close,
            Some(Box::new(|| { let _ = action_open_config(); })),
        );
    });

    thread::spawn(move || {
        let _ = progress_tx.send(ProgressUpdate::Indeterminate {
            message: "Import Thunderbird en cours…".to_string(),
        });
        match run_import_thunderbird(extract_passwords) {
            Ok(message) => {
                let _ = result_tx.send(ActionResult::Imported(message.clone()));
                let _ = progress_tx.send(ProgressUpdate::Done { summary: message });
            }
            Err(e) => {
                let _ = progress_tx.send(ProgressUpdate::Error {
                    message: format!("Import error: {}", e),
                    action_label: classify_error(&e),
                });
            }
        }
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
    let settings_path = config::settings_path();
    let mut settings = config::Settings::load(&settings_path).unwrap_or_default();

    settings.export_base_dir = Some(base_dir.to_string_lossy().replace('\\', "/"));
    settings.save(&settings_path)?;

    // Count accounts to report
    let count = Config::load(&config::accounts_yaml_path())
        .map(|c| c.accounts.len())
        .unwrap_or(0);

    Ok(format!("{} compte(s) → {}", count, base_dir.display()))
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

/// Open settings.yaml in the default editor (creates a template if absent).
pub fn action_open_config() -> Result<()> {
    let settings_path = config::settings_path();

    if !settings_path.exists() {
        if let Some(parent) = settings_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let template = r#"# Email to Markdown — Application settings
# Set export_base_dir, then each account gets a sub-folder named after the account.

# Root directory for all exported emails
# export_base_dir: C:/Users/YourName/Documents/Emails

# Default behaviour for all accounts
defaults:
  quote_depth: 1
  skip_existing: true
  collect_contacts: false
  skip_signature_images: true
  delete_after_export: false

# Per-account overrides (optional)
# accounts:
#   Gmail:
#     folder_name: gmail          # custom sub-folder name (default: account name)
#     delete_after_export: false
#   Outlook:
#     collect_contacts: true
"#;
        std::fs::write(&settings_path, template)?;
    }

    open::that(&settings_path).context("Failed to open settings file")?;
    Ok(())
}

/// Fix YAML frontmatter for a specific account's export directory.
pub fn action_fix_yaml(account_name: String, _result_sender: Sender<ActionResult>) {
    let (progress_tx, progress_rx) = mpsc::channel::<ProgressUpdate>();

    thread::spawn(move || {
        crate::tray_progress_window::open(
            "Fix YAML",
            progress_rx,
            None,
            Some(Box::new(|| { let _ = action_open_config(); })),
        );
    });

    thread::spawn(move || {
        let progress_tx_clone = progress_tx.clone();
        let on_progress = move |current: usize, total: usize, label: &str| {
            let _ = progress_tx_clone.send(ProgressUpdate::Step {
                current,
                total,
                message: label.to_string(),
            });
        };
        match run_fix_yaml(&account_name, Some(&on_progress)) {
            Ok(summary) => {
                let _ = progress_tx.send(ProgressUpdate::Done { summary });
            }
            Err(e) => {
                let _ = progress_tx.send(ProgressUpdate::Error {
                    message: format!("Fix YAML error: {}", e),
                    action_label: classify_error(&e),
                });
            }
        }
    });
}

fn run_fix_yaml(
    account_name: &str,
    on_progress: Option<&(dyn Fn(usize, usize, &str) + Send + Sync)>,
) -> Result<String> {
    dotenv::from_path(config::env_file_path()).ok();

    let config = Config::load(&config::accounts_yaml_path()).context("Failed to load configuration")?;

    let account = config
        .get_account(account_name)
        .context(format!("Account '{}' not found", account_name))?;

    let dir = PathBuf::from(&account.export_directory);

    let stats = fix_yaml::scan_and_fix_directory(&dir, false, on_progress)
        .context("Failed to fix YAML frontmatter")?;

    Ok(format!(
        "{}: {} corrigés, {} réécrits, {} erreurs",
        account_name, stats.files_fixed, stats.files_rewritten, stats.errors
    ))
}

/// Fix HTML bodies to Markdown for a specific account's export directory.
pub fn action_fix_html(account_name: String, _result_sender: Sender<ActionResult>) {
    let (progress_tx, progress_rx) = mpsc::channel::<ProgressUpdate>();

    thread::spawn(move || {
        crate::tray_progress_window::open(
            "Fix HTML",
            progress_rx,
            None,
            Some(Box::new(|| { let _ = action_open_config(); })),
        );
    });

    thread::spawn(move || {
        let progress_tx_clone = progress_tx.clone();
        let on_progress = move |current: usize, total: usize, label: &str| {
            let _ = progress_tx_clone.send(ProgressUpdate::Step {
                current,
                total,
                message: label.to_string(),
            });
        };
        match run_fix_html(&account_name, Some(&on_progress)) {
            Ok(summary) => {
                let _ = progress_tx.send(ProgressUpdate::Done { summary });
            }
            Err(e) => {
                let _ = progress_tx.send(ProgressUpdate::Error {
                    message: format!("Fix HTML error: {}", e),
                    action_label: classify_error(&e),
                });
            }
        }
    });
}

fn run_fix_html(
    account_name: &str,
    on_progress: Option<&(dyn Fn(usize, usize, &str) + Send + Sync)>,
) -> Result<String> {
    dotenv::from_path(config::env_file_path()).ok();

    let config = Config::load(&config::accounts_yaml_path()).context("Failed to load configuration")?;

    let account = config
        .get_account(account_name)
        .context(format!("Account '{}' not found", account_name))?;

    let dir = PathBuf::from(&account.export_directory);

    let stats = email_export::fix_html_bodies(&dir, false, on_progress)
        .context("Failed to fix HTML bodies")?;

    Ok(format!(
        "{}: {} convertis, {} ignorés, {} erreurs",
        account_name, stats.fixed, stats.skipped, stats.errors
    ))
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
