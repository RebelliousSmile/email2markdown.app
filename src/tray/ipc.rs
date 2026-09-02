//! IPC message decoding for the destinations-management and settings webviews.

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde_json::Value;
use tao::event_loop::EventLoopProxy;
use tao::window::WindowId;

use crate::config::{self, AccountBehavior, RawAccount, Settings};
use crate::tray_actions::{action_open_config, ActionResult};

use super::AppCommand;

/// Read a string field from a JSON object, `None` if missing or not a string.
fn field_str<'a>(data: &'a Value, field: &str) -> Option<&'a str> {
    data.get(field).and_then(|v| v.as_str())
}

#[derive(serde::Deserialize)]
struct DestGuiIpcMessage {
    action: String,
    data: Option<Value>,
}

pub(super) enum DestGuiIpcResult {
    StateChanged,
    Error(String),
    Suggestions(Vec<(String, usize)>),
    FolderSuggestions(Vec<String>),
    Saved,
    Close,
    Noop,
}

pub(super) fn state_json(cfg: &crate::destinations::DestinationsConfig) -> String {
    let json = serde_json::json!({ "type": "state", "destinations": cfg.destinations });
    serde_json::to_string(&json).unwrap_or_default()
}

/// Decode and dispatch one destinations-GUI IPC message.
pub(super) fn handle_dest_gui_ipc(
    body: &str,
    cfg: &mut crate::destinations::DestinationsConfig,
    dest_file: &Path,
) -> DestGuiIpcResult {
    let msg: DestGuiIpcMessage = match serde_json::from_str(body) {
        Ok(m) => m,
        Err(_) => return DestGuiIpcResult::Noop,
    };

    match msg.action.as_str() {
        "save" => handle_save(cfg, dest_file),
        "cancel" => DestGuiIpcResult::Close,
        "init" => DestGuiIpcResult::StateChanged,
        "add_entry" => handle_add_entry(cfg, msg.data),
        "remove_entry" => handle_remove_entry(cfg, msg.data),
        "set_default" => handle_set_default(cfg, msg.data),
        "set_note" => handle_set_note(cfg, msg.data),
        "add_rule" => handle_add_rule(cfg, msg.data),
        "remove_rule" => handle_remove_rule(cfg, msg.data),
        "reorder" => handle_reorder(cfg, msg.data),
        "remove_entries" => handle_remove_entries(cfg, msg.data),
        "add_entries" => handle_add_entries(cfg, msg.data),
        "scan_suggest" | "scan_folders" => handle_scan(cfg, &msg.action),
        "suggest_confirm" => handle_suggest_confirm(cfg, msg.data),
        _ => DestGuiIpcResult::Noop,
    }
}

fn handle_save(
    cfg: &crate::destinations::DestinationsConfig,
    dest_file: &Path,
) -> DestGuiIpcResult {
    if let Err(e) = crate::destinations::save_yaml(dest_file, cfg) {
        eprintln!("dest-gui: save failed: {:#}", e);
        return DestGuiIpcResult::Error(format!("Enregistrement impossible : {e:#}"));
    }
    DestGuiIpcResult::Saved
}

fn handle_add_entry(
    cfg: &mut crate::destinations::DestinationsConfig,
    data: Option<Value>,
) -> DestGuiIpcResult {
    let Some(data) = data else {
        return DestGuiIpcResult::Noop;
    };
    let path = match field_str(&data, "path") {
        Some(p) if !p.trim().is_empty() => p.trim().to_string(),
        _ => return DestGuiIpcResult::Noop,
    };
    if crate::route::join_safe_segments(Path::new(""), &path).is_err() {
        eprintln!("dest-gui: rejected invalid path {:?}", path);
        return DestGuiIpcResult::Noop;
    }
    let already_exists = cfg
        .destinations
        .iter()
        .any(|e| e.path.eq_ignore_ascii_case(&path));
    crate::destinations::upsert_entry(cfg, &path, &[]);
    if !already_exists {
        let note = field_str(&data, "note")
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.to_string());
        if note.is_some() {
            crate::destinations::set_note(cfg, &path, note);
        }
    }
    DestGuiIpcResult::StateChanged
}

fn handle_remove_entry(
    cfg: &mut crate::destinations::DestinationsConfig,
    data: Option<Value>,
) -> DestGuiIpcResult {
    let Some(data) = data else {
        return DestGuiIpcResult::Noop;
    };
    let Some(path) = field_str(&data, "path") else {
        return DestGuiIpcResult::Noop;
    };
    crate::destinations::remove_entry(cfg, path);
    DestGuiIpcResult::StateChanged
}

fn handle_set_default(
    cfg: &mut crate::destinations::DestinationsConfig,
    data: Option<Value>,
) -> DestGuiIpcResult {
    let Some(data) = data else {
        return DestGuiIpcResult::Noop;
    };
    let Some(path) = field_str(&data, "path") else {
        return DestGuiIpcResult::Noop;
    };
    crate::destinations::set_default(cfg, path);
    DestGuiIpcResult::StateChanged
}

fn handle_set_note(
    cfg: &mut crate::destinations::DestinationsConfig,
    data: Option<Value>,
) -> DestGuiIpcResult {
    let Some(data) = data else {
        return DestGuiIpcResult::Noop;
    };
    let Some(path) = field_str(&data, "path") else {
        return DestGuiIpcResult::Noop;
    };
    let note = field_str(&data, "note")
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string());
    crate::destinations::set_note(cfg, path, note);
    DestGuiIpcResult::StateChanged
}

fn handle_add_rule(
    cfg: &mut crate::destinations::DestinationsConfig,
    data: Option<Value>,
) -> DestGuiIpcResult {
    use crate::destinations::DestinationRule;

    let Some(data) = data else {
        return DestGuiIpcResult::Noop;
    };
    let Some(path) = field_str(&data, "path") else {
        return DestGuiIpcResult::Noop;
    };
    let Some(kind) = field_str(&data, "kind") else {
        return DestGuiIpcResult::Noop;
    };
    let Some(raw_value) = field_str(&data, "value") else {
        return DestGuiIpcResult::Noop;
    };
    let value = raw_value.trim();
    if value.is_empty() {
        return DestGuiIpcResult::Noop;
    }
    let rule = match kind {
        "domain" => DestinationRule::Domain(value.to_lowercase()),
        "from" => match crate::route::normalize_address(value) {
            Ok(_) => DestinationRule::From(value.to_string()),
            Err(e) => return DestGuiIpcResult::Error(e.to_string()),
        },
        "correspondent" => match crate::route::normalize_address(value) {
            Ok(_) => DestinationRule::Correspondent(value.to_string()),
            Err(e) => return DestGuiIpcResult::Error(e.to_string()),
        },
        "subject" => DestinationRule::Subject(value.to_string()),
        "account" => DestinationRule::Account(value.to_string()),
        _ => return DestGuiIpcResult::Noop,
    };
    crate::destinations::add_rule(cfg, path, rule);
    DestGuiIpcResult::StateChanged
}

fn handle_remove_rule(
    cfg: &mut crate::destinations::DestinationsConfig,
    data: Option<Value>,
) -> DestGuiIpcResult {
    use crate::destinations::DestinationRule;

    let Some(data) = data else {
        return DestGuiIpcResult::Noop;
    };
    let Some(path) = field_str(&data, "path") else {
        return DestGuiIpcResult::Noop;
    };
    let Some(rule_val) = data.get("rule") else {
        return DestGuiIpcResult::Noop;
    };
    let Ok(rule) = serde_json::from_value::<DestinationRule>(rule_val.clone()) else {
        return DestGuiIpcResult::Noop;
    };
    crate::destinations::remove_rule(cfg, path, &rule);
    DestGuiIpcResult::StateChanged
}

fn handle_reorder(
    cfg: &mut crate::destinations::DestinationsConfig,
    data: Option<Value>,
) -> DestGuiIpcResult {
    let Some(data) = data else {
        return DestGuiIpcResult::Noop;
    };
    let Some(order_arr) = data.get("order").and_then(|v| v.as_array()) else {
        return DestGuiIpcResult::Noop;
    };
    let order: Vec<&str> = order_arr.iter().filter_map(|v| v.as_str()).collect();
    crate::destinations::reorder_destinations(cfg, &order);
    DestGuiIpcResult::StateChanged
}

fn handle_remove_entries(
    cfg: &mut crate::destinations::DestinationsConfig,
    data: Option<Value>,
) -> DestGuiIpcResult {
    let Some(arr) = data.as_ref().and_then(|v| v.as_array()) else {
        return DestGuiIpcResult::Noop;
    };
    for item in arr {
        let Some(path) = field_str(item, "path") else {
            continue;
        };
        crate::destinations::remove_entry(cfg, path);
    }
    DestGuiIpcResult::StateChanged
}

/// Reuses `destinations::upsert_entry` instead of reconstructing entries inline
/// (fixes duplication with `handle_add_entry` flagged by the tray-refactor audit).
fn handle_add_entries(
    cfg: &mut crate::destinations::DestinationsConfig,
    data: Option<Value>,
) -> DestGuiIpcResult {
    let Some(arr) = data.as_ref().and_then(|v| v.as_array()) else {
        return DestGuiIpcResult::Noop;
    };
    for item in arr {
        let Some(path) = field_str(item, "path") else {
            continue;
        };
        let path = path.trim();
        if path.is_empty() {
            continue;
        }
        if crate::route::join_safe_segments(Path::new(""), path).is_err() {
            continue;
        }
        crate::destinations::upsert_entry(cfg, path, &[]);
    }
    DestGuiIpcResult::StateChanged
}

fn handle_scan(cfg: &crate::destinations::DestinationsConfig, action: &str) -> DestGuiIpcResult {
    let settings =
        crate::config::Settings::load(&crate::config::settings_path()).unwrap_or_default();
    let Some(notes_dir_str) = settings.notes_dir.as_deref() else {
        eprintln!("dest-gui: {}: notes_dir non configuré", action);
        return if action == "scan_suggest" {
            DestGuiIpcResult::Suggestions(vec![])
        } else {
            DestGuiIpcResult::FolderSuggestions(vec![])
        };
    };
    let notes_dir = std::path::PathBuf::from(notes_dir_str);
    let scan = match crate::dest_cmd::scan_notes(&notes_dir) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("dest-gui: scan_notes: {:#}", e);
            return if action == "scan_suggest" {
                DestGuiIpcResult::Suggestions(vec![])
            } else {
                DestGuiIpcResult::FolderSuggestions(vec![])
            };
        }
    };
    if action == "scan_suggest" {
        let candidates = crate::dest_cmd::uncovered_domains(scan.domains, cfg);
        DestGuiIpcResult::Suggestions(candidates)
    } else {
        let existing: std::collections::HashSet<String> = cfg
            .destinations
            .iter()
            .map(|e| e.path.to_lowercase())
            .collect();
        let folders = scan
            .folders
            .into_iter()
            .filter(|f| f.chars().filter(|&c| c == '/').count() < 3)
            .filter(|f| !existing.contains(&f.to_lowercase()))
            .collect();
        DestGuiIpcResult::FolderSuggestions(folders)
    }
}

fn handle_suggest_confirm(
    cfg: &mut crate::destinations::DestinationsConfig,
    data: Option<Value>,
) -> DestGuiIpcResult {
    use crate::destinations::DestinationRule;

    let Some(pairs_arr) = data.as_ref().and_then(|v| v.as_array()) else {
        return DestGuiIpcResult::Noop;
    };
    for pair in pairs_arr {
        let Some(domain) = field_str(pair, "domain") else {
            continue;
        };
        let Some(dest_path) = field_str(pair, "path") else {
            continue;
        };
        if dest_path.trim().is_empty() {
            continue;
        }
        if crate::route::join_safe_segments(Path::new(""), dest_path).is_err() {
            eprintln!(
                "dest-gui: suggest_confirm: rejected invalid path {:?}",
                dest_path
            );
            continue;
        }
        crate::destinations::upsert_entry(
            cfg,
            dest_path,
            &[DestinationRule::Domain(domain.to_lowercase())],
        );
    }
    DestGuiIpcResult::StateChanged
}

/// Apply a batch of route decisions (move staged emails into their target folders).
///
/// `route.rs`/`destinations.rs` already own path validation and the actual file
/// move (`join_safe_segments`, `move_email`) — no logic is reimplemented here.
pub(super) fn apply_route_decisions(
    body: &str,
    notes_dir: &PathBuf,
    _window_id: WindowId,
    _proxy: &EventLoopProxy<AppCommand>,
) -> anyhow::Result<()> {
    #[derive(serde::Deserialize)]
    struct RouteApplyPayload {
        decisions: Vec<RouteDecisionRow>,
    }
    #[derive(serde::Deserialize)]
    struct RouteDecisionRow {
        file: String,
        dest_path: String,
    }

    let payload: RouteApplyPayload =
        serde_json::from_str(body).context("failed to parse route review IPC payload")?;

    for row in &payload.decisions {
        let staging_md = PathBuf::from(&row.file);
        // Normalize the destination to carry the email's <Year>/<Month>. The auto
        // proposal already ends with it; a manually reassigned path (cascade / free
        // entry / bulk) comes bare from destinations.txt — append it from the email
        // date so files always land under <dest>/<Year>/<Month> (no double suffix).
        let dest_path = match read_frontmatter_field(&staging_md, "date")
            .and_then(|d| chrono::DateTime::parse_from_rfc3339(d.trim()).ok())
        {
            Some(dt) => crate::route::ensure_year_month(
                &row.dest_path,
                &dt.format("%Y").to_string(),
                &dt.format("%m").to_string(),
            ),
            // Date unreadable/unparseable → keep the path as-is (no guess).
            None => row.dest_path.clone(),
        };
        // Anti-traversal validation — rejects "..", "\", absolute paths.
        let dest_dir =
            crate::route::join_safe_segments(notes_dir, &dest_path).with_context(|| {
                format!(
                    "invalid destination path {:?} for file {:?}",
                    dest_path, row.file
                )
            })?;
        // Create the directory tree (D4: mkdir -p).
        std::fs::create_dir_all(&dest_dir)
            .with_context(|| format!("failed to create directory {}", dest_dir.display()))?;
        // Move .md + its referenced attachment siblings.
        crate::route::move_email(&staging_md, &dest_dir).with_context(|| {
            format!(
                "failed to move {} to {}",
                staging_md.display(),
                dest_dir.display()
            )
        })?;
    }
    Ok(())
}

/// Delete staged emails (and their attachment sidecars) that were rejected during routing.
pub(super) fn delete_staged_emails(files: &[String]) -> (Vec<String>, Option<String>) {
    let mut deleted = Vec::new();
    let mut errors = Vec::new();
    for file in files {
        let path = PathBuf::from(file);
        match crate::route::delete_email(&path) {
            Ok(()) => deleted.push(file.clone()),
            Err(e) => errors.push(format!("{}: {:#}", file, e)),
        }
    }
    let err = if errors.is_empty() {
        None
    } else {
        Some(format!(
            "Suppression échouée pour {} fichier(s) :\n{}",
            errors.len(),
            errors.join("\n")
        ))
    };
    (deleted, err)
}

/// Read a single YAML frontmatter field from a markdown file, if present.
pub(super) fn read_frontmatter_field(path: &std::path::Path, field: &str) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let rest = content.strip_prefix("---\n")?;
    let end = rest.find("\n---")?;
    let frontmatter = &rest[..end];
    let prefix = format!("{}:", field);
    for line in frontmatter.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(&prefix) {
            let value = trimmed[prefix.len()..]
                .trim()
                .trim_matches('"')
                .trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

#[derive(serde::Deserialize)]
struct ConfigIpcMessage {
    action: String,
    data: Option<Value>,
}

#[derive(serde::Deserialize)]
struct SettingsData {
    export_base_dir: Option<String>,
    defaults: DefaultsData,
}

#[derive(serde::Deserialize)]
struct DefaultsData {
    quote_depth: Option<usize>,
    skip_existing: Option<bool>,
    collect_contacts: Option<bool>,
    skip_signature_images: Option<bool>,
    delete_after_export: Option<bool>,
    cleanup_empty_dirs: Option<bool>,
}

#[derive(serde::Deserialize)]
struct AccountData {
    account_name: String,
    server: String,
    port: u16,
    username: String,
    #[serde(default)]
    ignored_folders: Vec<String>,
    #[serde(default)]
    delete_after_export: Option<bool>,
    #[serde(default)]
    cleanup_empty_dirs: Option<bool>,
    #[serde(default)]
    skip_existing: Option<bool>,
    #[serde(default)]
    collect_contacts: Option<bool>,
    #[serde(default)]
    skip_signature_images: Option<bool>,
    #[serde(default)]
    quote_depth: Option<usize>,
}

/// Decode and dispatch one settings-window IPC message.
pub(super) fn handle_config_ipc(body: &str) -> (Option<ActionResult>, bool) {
    let msg: ConfigIpcMessage = match serde_json::from_str(body) {
        Ok(m) => m,
        Err(e) => {
            return (
                Some(ActionResult::Error(format!(
                    "failed to parse IPC message: {}",
                    e
                ))),
                true,
            );
        }
    };

    match msg.action.as_str() {
        "save" => {
            let result = (|| -> anyhow::Result<ActionResult> {
                let raw_data = msg
                    .data
                    .ok_or_else(|| anyhow::anyhow!("save action missing data field"))?;
                let data: SettingsData =
                    serde_json::from_value(raw_data).context("failed to parse settings data")?;

                let path = config::settings_path();
                let mut settings = Settings::load(&path).unwrap_or_default();
                settings.export_base_dir = data.export_base_dir;
                settings.defaults = AccountBehavior {
                    folder_name: settings.defaults.folder_name,
                    quote_depth: data.defaults.quote_depth,
                    skip_existing: data.defaults.skip_existing,
                    collect_contacts: data.defaults.collect_contacts,
                    skip_signature_images: data.defaults.skip_signature_images,
                    delete_after_export: data.defaults.delete_after_export,
                    cleanup_empty_dirs: data.defaults.cleanup_empty_dirs,
                };
                settings
                    .save(&path)
                    .with_context(|| format!("failed to save settings to {}", path.display()))?;

                Ok(ActionResult::Success(
                    "Param\u{00e8}tres".to_string(),
                    "Param\u{00e8}tres sauvegard\u{00e9}s".to_string(),
                ))
            })();
            match result {
                Ok(r) => (Some(r), true),
                Err(e) => (
                    Some(ActionResult::Error(format!(
                        "Erreur de sauvegarde : {:#}",
                        e
                    ))),
                    true,
                ),
            }
        }
        "save_account" => {
            let result = (|| -> anyhow::Result<()> {
                let raw_data = msg
                    .data
                    .ok_or_else(|| anyhow::anyhow!("save_account action missing data field"))?;
                let data: AccountData =
                    serde_json::from_value(raw_data).context("failed to parse account data")?;

                let accounts_path = config::accounts_yaml_path();
                let mut accounts = config::load_raw_accounts(&accounts_path).unwrap_or_default();

                let mut found = false;
                for acct in accounts.iter_mut() {
                    if acct.name.eq_ignore_ascii_case(&data.account_name) {
                        acct.server = data.server.clone();
                        acct.port = data.port;
                        acct.username = data.username.clone();
                        acct.ignored_folders = data.ignored_folders.clone();
                        found = true;
                        break;
                    }
                }
                if !found {
                    accounts.push(RawAccount {
                        name: data.account_name.clone(),
                        server: data.server.clone(),
                        port: data.port,
                        username: data.username.clone(),
                        ignored_folders: data.ignored_folders.clone(),
                    });
                }

                config::save_accounts(&accounts, &accounts_path).with_context(|| {
                    format!("failed to save accounts to {}", accounts_path.display())
                })?;

                let settings_path = config::settings_path();
                let mut settings = Settings::load(&settings_path).unwrap_or_default();

                let canonical_key = settings
                    .accounts
                    .keys()
                    .find(|k| k.eq_ignore_ascii_case(&data.account_name))
                    .cloned()
                    .unwrap_or_else(|| data.account_name.clone());

                let mut behavior = settings
                    .accounts
                    .get(&canonical_key)
                    .cloned()
                    .unwrap_or_default();
                behavior.delete_after_export = data.delete_after_export;
                behavior.cleanup_empty_dirs = data.cleanup_empty_dirs;
                behavior.skip_existing = data.skip_existing;
                behavior.collect_contacts = data.collect_contacts;
                behavior.skip_signature_images = data.skip_signature_images;
                behavior.quote_depth = data.quote_depth;

                let is_empty = serde_json::to_value(&behavior)
                    .map(|v| v.as_object().map(|o| o.is_empty()).unwrap_or(false))
                    .unwrap_or(false);

                if is_empty {
                    settings.accounts.remove(&canonical_key);
                } else {
                    settings.accounts.insert(canonical_key, behavior);
                }

                settings.save(&settings_path).with_context(|| {
                    format!("failed to save settings to {}", settings_path.display())
                })?;

                Ok(())
            })();
            match result {
                Ok(()) => (None, false),
                Err(e) => (
                    Some(ActionResult::Error(format!(
                        "Erreur de sauvegarde : {:#}",
                        e
                    ))),
                    false,
                ),
            }
        }
        "open_raw" => {
            if let Err(e) = action_open_config().context("failed to open settings file in editor") {
                return (
                    Some(ActionResult::Error(format!("Erreur : {:#}", e))),
                    false,
                );
            }
            (None, false)
        }
        other => (
            Some(ActionResult::Error(format!(
                "unknown IPC action '{}'",
                other
            ))),
            true,
        ),
    }
}
