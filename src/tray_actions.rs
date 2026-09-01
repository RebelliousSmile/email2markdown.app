//! Action handlers for system tray menu items.
//!
//! This module provides the functions that are called when users
//! interact with the system tray menu.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread;

use anyhow::{Context, Result};
use rfd;
use walkdir::WalkDir;

use crate::progress::ProgressUpdate;

use crate::config::{self, Config, Settings};
use crate::contextual_export::{
    build_deletion_batch, find_existing_proof, ContextualCandidate, DeletionOutcome,
    DeletionPreflight, DeletionRequest,
};
use crate::email_export::{self, ImapExporter};
use crate::route::{self, EmailMeta, RouteDecision};
use crate::thunderbird;

#[derive(Debug, Clone, serde::Serialize)]
pub struct ContextualAccountChoice {
    pub name: String,
    pub password_missing: bool,
    pub delete_after_export: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ContextualLaunch {
    pub target: PathBuf,
    pub relative_path: String,
    #[serde(skip)]
    pub rules: Vec<crate::route::MatchRule>,
    pub rule_labels: Vec<String>,
    pub accounts: Vec<ContextualAccountChoice>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ContextualCandidateRow {
    pub key: String,
    pub date: Option<chrono::DateTime<chrono::FixedOffset>>,
    pub correspondent: String,
    pub subject: String,
    pub folder: String,
    pub already_present: bool,
}

#[derive(Debug)]
pub struct ContextualSearchWork {
    pub candidates: Vec<ContextualCandidate>,
    pub rows: Vec<ContextualCandidateRow>,
    pub preflight: DeletionPreflight,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ContextualBatchSummary {
    pub converted: usize,
    pub already_present: usize,
    pub deleted: usize,
    pub retry_conversion: Vec<String>,
    pub retry_deletion_count: usize,
    pub stale_search: bool,
    pub message: String,
    #[serde(skip)]
    pub retry_deletion: Vec<DeletionRequest>,
}

impl ContextualBatchSummary {
    pub fn complete(&self) -> bool {
        self.retry_conversion.is_empty() && self.retry_deletion.is_empty() && !self.stale_search
    }
}

/// Validate the local target before any network connection and prepare the
/// serializable state injected into the standalone contextual window.
pub fn prepare_contextual_launch(target: &std::path::Path) -> Result<ContextualLaunch> {
    dotenvy::from_path(config::env_file_path()).ok();
    let cfg =
        Config::load(&config::accounts_yaml_path()).context("Failed to load configuration")?;
    let settings = Settings::load(&config::settings_path()).context("Failed to load settings")?;
    let notes_dir = settings
        .notes_dir
        .as_deref()
        .map(std::path::Path::new)
        .context("notes_dir is not configured")?;
    let destinations = route::load_destinations();
    let configured_names: Vec<String> = cfg
        .accounts
        .iter()
        .map(|account| account.name.clone())
        .collect();
    // With no configured account, still validate the exact local destination so
    // the window can offer its explicit configuration action.
    let validation_names = if configured_names.is_empty() {
        let mut names: Vec<String> = destinations
            .iter()
            .flat_map(|destination| destination.rules.iter())
            .filter_map(|rule| match rule {
                crate::route::MatchRule::Account(name) => Some(name.clone()),
                _ => None,
            })
            .collect();
        names.push("__unconfigured__".into());
        names
    } else {
        configured_names.clone()
    };
    let resolved =
        route::resolve_contextual_destination(notes_dir, target, &destinations, &validation_names)?;
    let accounts = cfg
        .accounts
        .iter()
        .filter(|account| resolved.allowed_accounts.contains(&account.name))
        .map(|account| ContextualAccountChoice {
            name: account.name.clone(),
            password_missing: account.password.is_none(),
            delete_after_export: account.delete_after_export,
        })
        .collect();
    let rule_labels = resolved
        .address_rules
        .iter()
        .map(|rule| match rule {
            crate::route::MatchRule::Correspondent(value) => format!("correspondant : {value}"),
            crate::route::MatchRule::From(value) => format!("expéditeur : {value}"),
            crate::route::MatchRule::Domain(value) => format!("domaine : {value}"),
            crate::route::MatchRule::Subject(value) => format!("objet : {value}"),
            crate::route::MatchRule::Account(value) => format!("compte : {value}"),
        })
        .collect();
    Ok(ContextualLaunch {
        target: resolved.target,
        relative_path: resolved.relative_path,
        rules: resolved.address_rules,
        rule_labels,
        accounts,
    })
}

pub fn run_contextual_search(
    launch: &ContextualLaunch,
    account_name: &str,
) -> Result<ContextualSearchWork> {
    dotenvy::from_path(config::env_file_path()).ok();
    let cfg =
        Config::load(&config::accounts_yaml_path()).context("Failed to load configuration")?;
    let account = cfg
        .get_account(account_name)
        .with_context(|| format!("Account '{}' not found", account_name))?
        .clone();
    if !launch
        .accounts
        .iter()
        .any(|choice| choice.name == account.name)
    {
        anyhow::bail!("account is not allowed for this destination");
    }
    if account.password.is_none() {
        anyhow::bail!("No password found for {}", account.name);
    }
    let mut exporter = ImapExporter::new(account, false);
    exporter
        .connect()
        .context("Failed to connect to IMAP server")?;
    let preflight = exporter.contextual_deletion_preflight()?;
    let candidates = exporter.search_contextual(&launch.rules)?;
    let rows = candidates
        .iter()
        .map(|candidate| -> Result<ContextualCandidateRow> {
            let correspondent = candidate
                .from
                .first()
                .or_else(|| candidate.to.first())
                .cloned()
                .unwrap_or_default();
            Ok(ContextualCandidateRow {
                key: candidate.logical_key(),
                date: candidate.date,
                correspondent,
                subject: candidate.subject.clone(),
                folder: candidate
                    .locations
                    .first()
                    .map(|location| location.folder_display.clone())
                    .unwrap_or_default(),
                already_present: find_existing_proof(&launch.target, candidate)?.is_some(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    exporter.disconnect().ok();
    Ok(ContextualSearchWork {
        candidates,
        rows,
        preflight,
    })
}

pub fn run_contextual_batch(
    target: &std::path::Path,
    account_name: &str,
    selected: &[ContextualCandidate],
) -> Result<ContextualBatchSummary> {
    dotenvy::from_path(config::env_file_path()).ok();
    let cfg =
        Config::load(&config::accounts_yaml_path()).context("Failed to load configuration")?;
    let account = cfg
        .get_account(account_name)
        .with_context(|| format!("Account '{}' not found", account_name))?
        .clone();
    let delete_after_export = account.delete_after_export;
    let mut exporter = ImapExporter::new(account, false);
    exporter
        .connect()
        .context("Failed to connect to IMAP server")?;
    let conversions = exporter.convert_contextual_selection(target, selected)?;
    let converted = conversions
        .iter()
        .filter(|result| {
            matches!(
                result.status,
                Some(crate::contextual_export::ConversionStatus::Written { .. })
            )
        })
        .count();
    let already_present = conversions
        .iter()
        .filter(|result| {
            matches!(
                result.status,
                Some(crate::contextual_export::ConversionStatus::AlreadyPresent { .. })
            )
        })
        .count();
    let retry_conversion: Vec<String> = conversions
        .iter()
        .filter(|result| result.status.is_none())
        .map(|result| result.candidate_key.clone())
        .collect();
    let mut deleted = 0;
    let mut stale_search = false;
    let mut retry_deletion = Vec::new();
    if delete_after_export {
        let batch = build_deletion_batch(&conversions);
        let deletion_results = exporter.delete_proved_messages(&batch)?;
        for (request, result) in batch.into_iter().zip(deletion_results) {
            match result.outcome {
                DeletionOutcome::Deleted | DeletionOutcome::AlreadyAbsent => deleted += 1,
                DeletionOutcome::StaleUidValidity { .. } => {
                    stale_search = true;
                    retry_deletion.push(request);
                }
                DeletionOutcome::RetryRequired(_) => retry_deletion.push(request),
            }
        }
    }
    exporter.disconnect().ok();
    let retry_deletion_count = retry_deletion.len();
    Ok(ContextualBatchSummary {
        converted,
        already_present,
        deleted,
        retry_conversion,
        retry_deletion_count,
        stale_search,
        message: if retry_deletion_count == 0 {
            "Traitement terminé".into()
        } else {
            "Conversion locale terminée, suppression serveur à réessayer".into()
        },
        retry_deletion,
    })
}

pub fn retry_contextual_deletions(
    account_name: &str,
    requests: &[DeletionRequest],
) -> Result<ContextualBatchSummary> {
    dotenvy::from_path(config::env_file_path()).ok();
    let cfg =
        Config::load(&config::accounts_yaml_path()).context("Failed to load configuration")?;
    let account = cfg
        .get_account(account_name)
        .with_context(|| format!("Account '{}' not found", account_name))?
        .clone();
    let mut exporter = ImapExporter::new(account, false);
    exporter
        .connect()
        .context("Failed to connect to IMAP server")?;
    let outcomes = exporter.delete_proved_messages(requests)?;
    let mut deleted = 0;
    let mut stale_search = false;
    let mut retry_deletion = Vec::new();
    for (request, result) in requests.iter().cloned().zip(outcomes) {
        match result.outcome {
            DeletionOutcome::Deleted | DeletionOutcome::AlreadyAbsent => deleted += 1,
            DeletionOutcome::StaleUidValidity { .. } => {
                stale_search = true;
                retry_deletion.push(request);
            }
            DeletionOutcome::RetryRequired(_) => retry_deletion.push(request),
        }
    }
    exporter.disconnect().ok();
    let retry_deletion_count = retry_deletion.len();
    Ok(ContextualBatchSummary {
        converted: 0,
        already_present: 0,
        deleted,
        retry_conversion: Vec::new(),
        retry_deletion_count,
        stale_search,
        message: if retry_deletion_count == 0 {
            "Suppression serveur terminée".into()
        } else {
            "Certaines suppressions restent à réessayer".into()
        },
        retry_deletion,
    })
}

/// Result of an action, sent back to the main thread for notification.
#[derive(Debug, Clone)]
pub enum ActionResult {
    /// (title, message)
    Success(String, String),
    /// Import completed — the main thread should rebuild the tray menu.
    Imported(String),
    Error(String),
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
pub fn action_export(account_name: String, result_sender: Sender<ActionResult>) {
    let (progress_tx, progress_rx) = mpsc::channel::<ProgressUpdate>();
    let cancel_token = Arc::new(AtomicBool::new(false));
    let cancel_token_worker = Arc::clone(&cancel_token);

    let delete_warning = {
        dotenvy::from_path(config::env_file_path()).ok();
        Config::load(&config::accounts_yaml_path())
            .ok()
            .and_then(|cfg| cfg.get_account(&account_name).cloned())
            .map(|acct| acct.delete_after_export)
            .unwrap_or(false)
    };

    if let Err(e) = crate::tray::send_command(crate::tray::AppCommand::OpenProgress {
        action_name: "Export".to_string(),
        warning: if delete_warning {
            Some("Les emails seront supprimés du serveur après export".to_string())
        } else {
            None
        },
        progress_rx,
        on_close: None,
        error_action: Some(Box::new(|| {
            let _ = action_open_config();
        })),
        sender: result_sender.clone(),
        cancel_token: Some(cancel_token),
    }) {
        let _ = result_sender.send(ActionResult::Error(format!(
            "Fenêtre de progression : {}",
            e
        )));
        return;
    }

    thread::spawn(move || {
        let progress_tx_clone = progress_tx.clone();
        let on_progress = move |current: usize, total: usize, label: &str| {
            let _ = progress_tx_clone.send(ProgressUpdate::Step {
                current,
                total,
                message: label.to_string(),
            });
        };
        let progress_tx_status = progress_tx.clone();
        let on_status = move |text: &str| {
            let _ = progress_tx_status.send(ProgressUpdate::StatusLine {
                text: text.to_string(),
            });
        };
        match run_export(
            &account_name,
            Some(&on_progress),
            Some(&on_status),
            cancel_token_worker,
        ) {
            Ok((summary, decisions)) => {
                let _ = progress_tx.send(ProgressUpdate::Done { summary });
                // D6: files stay in staging. Open the route review window so the
                // user can validate/reassign paths before any file is moved.
                // The AutoClose sent by ProgressUpdate::Done will close the progress
                // window; the route review window opens after that via the event loop.
                if !decisions.is_empty() {
                    if let Err(e) = crate::tray::send_command(
                        crate::tray::AppCommand::OpenRouteReview(decisions),
                    ) {
                        eprintln!("Failed to open route review window: {:#}", e);
                    }
                }
            }
            Err(e) => {
                let _ = progress_tx.send(ProgressUpdate::Error {
                    message: format!("Export error: {:#}", e),
                    action_label: classify_error(&e),
                });
            }
        }
    });
}

/// Returns `(summary_string, decisions)`.
/// Decisions are the `Vec<(PathBuf, RouteDecision)>` produced by `export_account`.
/// In GUI mode the caller opens the route review window; no files are moved here (D6).
fn run_export(
    account_name: &str,
    on_progress: Option<&(dyn Fn(usize, usize, &str) + Send + Sync)>,
    on_status: Option<&(dyn Fn(&str) + Send + Sync)>,
    cancel_token: Arc<AtomicBool>,
) -> Result<(String, Vec<(PathBuf, RouteDecision)>)> {
    dotenvy::from_path(config::env_file_path()).ok();

    let config =
        Config::load(&config::accounts_yaml_path()).context("Failed to load configuration")?;

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
    exporter
        .connect()
        .context("Failed to connect to IMAP server")?;

    let (results, decisions) = exporter
        .export_account(on_progress, on_status, Some(cancel_token.as_ref()))
        .context("Export failed")?;
    // `decisions` holds `Vec<(PathBuf, RouteDecision)>` — deferred move (D6).
    // GUI mode: files stay in staging; the route review window handles the move.

    exporter.disconnect().ok();

    let cancelled = cancel_token.load(Ordering::Relaxed);
    let total_exported: usize = results.values().map(|s| s.exported).sum();
    let total_skipped: usize = results.values().map(|s| s.skipped).sum();
    let total_errors: usize = results.values().map(|s| s.errors).sum();

    let prefix = if cancelled {
        "Export annulé"
    } else {
        "Export terminé"
    };
    Ok((
        format!(
            "{} — {} exportés, {} ignorés, {} erreurs",
            prefix, total_exported, total_skipped, total_errors
        ),
        decisions,
    ))
}

/// Resume sorting: re-open the route review window for a single account's emails
/// that are still sitting in staging (e.g. after a cancelled review).
///
/// Scans the account's `export_directory` for `.md` notes, recomputes a routing
/// proposal for each (same logic as the live export), and hands the list to the
/// route review window. Runs in a background thread — file IO only, no network.
pub fn action_resume_sort(account_name: String, result_sender: Sender<ActionResult>) {
    thread::spawn(move || match scan_staged_decisions(&account_name) {
        Ok(decisions) if decisions.is_empty() => {
            let _ = result_sender.send(ActionResult::Success(
                "Reprendre le tri".to_string(),
                format!("Aucun email à trier en attente pour {}", account_name),
            ));
        }
        Ok(decisions) => {
            if let Err(e) =
                crate::tray::send_command(crate::tray::AppCommand::OpenRouteReview(decisions))
            {
                let _ = result_sender.send(ActionResult::Error(format!(
                    "Ouverture de la revue : {:#}",
                    e
                )));
            }
        }
        Err(e) => {
            let _ = result_sender.send(ActionResult::Error(format!("Reprendre le tri : {:#}", e)));
        }
    });
}

/// Directories created by the export pipeline that must never be treated as
/// staged emails to re-sort: the delete bin, the failed-dump, and contacts.
fn is_excluded_staging_dir(name: &std::ffi::OsStr) -> bool {
    matches!(
        name.to_str(),
        Some("_deleted") | Some("_failed") | Some("contacts")
    )
}

/// Walk an account's `export_directory` and rebuild `(staging_path, RouteDecision)`
/// pairs for every `.md` note still there. Excludes `_deleted`/`_failed`/`contacts`.
fn scan_staged_decisions(account_name: &str) -> Result<Vec<(PathBuf, RouteDecision)>> {
    dotenvy::from_path(config::env_file_path()).ok();

    let config =
        Config::load(&config::accounts_yaml_path()).context("Failed to load configuration")?;
    let account = config
        .get_account(account_name)
        .context(format!("Account '{}' not found", account_name))?;

    let base = PathBuf::from(&account.export_directory);
    if !base.exists() {
        return Ok(Vec::new());
    }

    let dests = route::load_destinations();
    let mut decisions = Vec::new();

    let walker = WalkDir::new(&base)
        .into_iter()
        .filter_entry(|e| !(e.file_type().is_dir() && is_excluded_staging_dir(e.file_name())));

    for entry in walker.filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let meta = meta_from_frontmatter(&content, account_name);
        let decision = route::route_email(&meta, &dests);
        decisions.push((path.to_path_buf(), decision));
    }

    Ok(decisions)
}

/// Read a single scalar field from a `.md` file's YAML frontmatter block.
/// Matches the field name exactly (so `subject` never matches `subject_hash`).
fn frontmatter_field(content: &str, field: &str) -> Option<String> {
    let rest = content.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    let frontmatter = &rest[..end];
    let prefix = format!("{}:", field);
    for line in frontmatter.lines() {
        if let Some(value) = line.trim().strip_prefix(&prefix) {
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Rebuild the `EmailMeta` used for routing from a staged note's frontmatter.
/// Mirrors the export-time construction in `email_export::export_to_markdown`.
fn meta_from_frontmatter(content: &str, account_name: &str) -> EmailMeta {
    let from_raw = frontmatter_field(content, "from").unwrap_or_default();
    let to_raw = frontmatter_field(content, "to").unwrap_or_default();
    let subject = frontmatter_field(content, "subject").unwrap_or_default();
    let date_raw = frontmatter_field(content, "date").unwrap_or_default();

    let addresses = crate::utils::extract_emails(Some(&from_raw));
    let sender_addr = addresses.first().cloned().unwrap_or_default();
    let domain = sender_addr
        .rfind('@')
        .map(|i| sender_addr[i + 1..].to_string())
        .unwrap_or_default();

    // Frontmatter dates are written as RFC3339; epoch fallback on parse failure
    // (same fallback the export path uses for an unparseable Date header).
    let date = chrono::DateTime::parse_from_rfc3339(date_raw.trim()).unwrap_or_else(|_| {
        chrono::DateTime::from_timestamp(0, 0)
            .expect("epoch is valid")
            .fixed_offset()
    });

    EmailMeta {
        from: sender_addr,
        to: crate::utils::extract_emails(Some(&to_raw)),
        cc: Vec::new(),
        bcc: Vec::new(),
        domain,
        subject,
        account: account_name.to_string(),
        date,
    }
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

    let sender_for_error = result_sender.clone();
    let on_close = Some(Box::new(move || {
        if let Ok(r) = result_rx.try_recv() {
            let _ = result_sender.send(r);
        }
    }) as Box<dyn FnOnce() + Send>);

    if let Err(e) = crate::tray::send_command(crate::tray::AppCommand::OpenProgress {
        action_name: "Import Thunderbird".to_string(),
        warning: None,
        progress_rx,
        on_close,
        error_action: Some(Box::new(|| {
            let _ = action_open_config();
        })),
        sender: sender_for_error.clone(),
        cancel_token: None,
    }) {
        let _ = sender_for_error.send(ActionResult::Error(format!(
            "Fenêtre de progression : {}",
            e
        )));
        return;
    }

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
                    message: format!("Import error: {:#}", e),
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

/// Open a folder picker and store it as the notes directory in settings.yaml.
pub fn action_choose_notes_dir(result_sender: Sender<ActionResult>) {
    let folder = rfd::FileDialog::new()
        .set_title("Choisir le répertoire de notes")
        .pick_folder();

    let Some(base_dir) = folder else {
        return; // user cancelled
    };

    thread::spawn(move || {
        let result = set_notes_dir(&base_dir);
        let action_result = match result {
            Ok(msg) => ActionResult::Success("Répertoire de notes".to_string(), msg),
            Err(e) => ActionResult::Error(format!("Erreur répertoire de notes : {}", e)),
        };
        let _ = result_sender.send(action_result);
    });
}

fn set_notes_dir(base_dir: &std::path::Path) -> Result<String> {
    let settings_path = config::settings_path();
    let mut settings = Settings::load(&settings_path).unwrap_or_default();
    settings.notes_dir = Some(base_dir.to_string_lossy().replace('\\', "/"));
    settings.save(&settings_path)?;
    Ok(format!("Défini sur {}", base_dir.display()))
}

/// Open the documentation (README.md) in the default viewer.
pub fn action_open_documentation() -> Result<()> {
    let readme_paths = ["README.md", "docs/README.md"];

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

/// Fix HTML bodies to Markdown for a specific account's export directory.
pub fn action_fix_html(account_name: String, result_sender: Sender<ActionResult>) {
    let (progress_tx, progress_rx) = mpsc::channel::<ProgressUpdate>();

    if let Err(e) = crate::tray::send_command(crate::tray::AppCommand::OpenProgress {
        action_name: "Fix HTML".to_string(),
        warning: None,
        progress_rx,
        on_close: None,
        error_action: Some(Box::new(|| {
            let _ = action_open_config();
        })),
        sender: result_sender.clone(),
        cancel_token: None,
    }) {
        let _ = result_sender.send(ActionResult::Error(format!(
            "Fenêtre de progression : {}",
            e
        )));
        return;
    }

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
                    message: format!("Fix HTML error: {:#}", e),
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
    dotenvy::from_path(config::env_file_path()).ok();

    let config =
        Config::load(&config::accounts_yaml_path()).context("Failed to load configuration")?;

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
    dotenvy::from_path(config::env_file_path()).ok();

    let config_path = config::accounts_yaml_path();

    if !config_path.exists() {
        return Ok(Vec::new());
    }

    let config = Config::load(&config_path)?;
    Ok(config
        .list_accounts()
        .into_iter()
        .map(String::from)
        .collect())
}
