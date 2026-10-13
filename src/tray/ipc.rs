//! IPC message decoding for the destinations-management and settings webviews.

use std::collections::HashSet;
use std::path::Path;

use anyhow::Context;
use serde_json::Value;

use crate::config::{self, AccountBehavior, RawAccount, Settings};
use crate::tray_actions::{action_open_config, ActionResult};

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
    FolderSuggestions {
        new_folders: Vec<String>,
        orphans: Vec<String>,
    },
    /// The window asked for a folder picker; the dialog runs in the event loop.
    BrowseFolder,
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
        "browse_folder" => DestGuiIpcResult::BrowseFolder,
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
    let scanned = Settings::load(&config::settings_path())
        .context("failed to load settings")
        .and_then(|settings| {
            scan_destinations(settings.notes_dir.as_deref().map(Path::new), cfg, action)
        });
    match scanned {
        Ok(result) => result,
        Err(e) => DestGuiIpcResult::Error(format!("Analyse impossible : {:#}", e)),
    }
}

/// Scan `notes_dir` for the destinations window (`scan_suggest` or `scan_folders`).
///
/// Fails when the notes directory is not configured, does not exist or cannot be
/// read: an empty list would be indistinguishable from "nothing to suggest".
fn scan_destinations(
    notes_dir: Option<&Path>,
    cfg: &crate::destinations::DestinationsConfig,
    action: &str,
) -> anyhow::Result<DestGuiIpcResult> {
    let notes_dir = notes_dir
        .ok_or_else(|| anyhow::anyhow!("le dossier de notes n'est pas configur\u{00e9}"))?;
    if !notes_dir.is_dir() {
        anyhow::bail!(
            "le dossier de notes est introuvable : {}",
            notes_dir.display()
        );
    }
    let scan = crate::dest_cmd::scan_notes(notes_dir).context("failed to scan notes directory")?;
    Ok(if action == "scan_suggest" {
        let candidates = crate::dest_cmd::uncovered_domains(scan.domains, cfg);
        DestGuiIpcResult::Suggestions(candidates)
    } else {
        let existing: std::collections::HashSet<String> = cfg
            .destinations
            .iter()
            .map(|e| e.path.to_lowercase())
            .collect();
        let new_folders = scan
            .folders
            .into_iter()
            .filter(|f| f.chars().filter(|&c| c == '/').count() < 3)
            .filter(|f| !existing.contains(&f.to_lowercase()))
            .collect();
        let orphans = crate::dest_cmd::orphaned_entries(notes_dir, cfg);
        DestGuiIpcResult::FolderSuggestions {
            new_folders,
            orphans,
        }
    })
}

/// Open a folder picker rooted at `notes_dir` and build the reply for the
/// destinations window: the picked destination path, or an error message.
///
/// Returns `None` when the user cancels. Must run in the event loop, not in the
/// IPC closure: the dialog pumps its own message loop.
pub(super) fn browse_dest_folder() -> Option<String> {
    let notes_dir = match Settings::load(&config::settings_path())
        .context("failed to load settings")
        .and_then(|settings| {
            settings
                .notes_dir
                .ok_or_else(|| anyhow::anyhow!("le dossier de notes n'est pas configuré"))
        }) {
        Ok(dir) => std::path::PathBuf::from(dir),
        Err(e) => return Some(browse_error_json(&e)),
    };
    let picked = rfd::FileDialog::new()
        .set_title("Choisir un dossier de destination")
        .set_directory(&notes_dir)
        .pick_folder()?;
    Some(match relative_destination(&notes_dir, &picked) {
        Ok(path) => serde_json::json!({"type": "picked_folder", "path": path}).to_string(),
        Err(e) => browse_error_json(&e),
    })
}

fn browse_error_json(error: &anyhow::Error) -> String {
    serde_json::json!({"type": "error", "message": format!("Sélection impossible : {error:#}")})
        .to_string()
}

/// Destination path (`/`-separated, relative to `notes_dir`) of a picked folder.
///
/// Fails when `picked` is the notes directory itself, lies outside it, or has a
/// segment the router would reject.
fn relative_destination(notes_dir: &Path, picked: &Path) -> anyhow::Result<String> {
    let root = notes_dir.canonicalize().with_context(|| {
        format!(
            "le dossier de notes est introuvable : {}",
            notes_dir.display()
        )
    })?;
    let target = picked
        .canonicalize()
        .with_context(|| format!("le dossier choisi est introuvable : {}", picked.display()))?;
    let relative = target.strip_prefix(&root).map_err(|_| {
        anyhow::anyhow!(
            "le dossier choisi n'est pas dans le dossier de notes ({})",
            notes_dir.display()
        )
    })?;
    let segments: Vec<&str> = relative
        .components()
        .map(|c| c.as_os_str().to_str())
        .collect::<Option<_>>()
        .ok_or_else(|| anyhow::anyhow!("le nom du dossier choisi n'est pas de l'UTF-8 valide"))?;
    if segments.is_empty() {
        anyhow::bail!("choisissez un sous-dossier du dossier de notes");
    }
    let path = segments.join("/");
    crate::route::join_safe_segments(Path::new(""), &path)
        .context("le nom du dossier contient des caractères non pris en charge")?;
    Ok(path)
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

/// One row of the route review "apply" payload.
#[derive(serde::Deserialize)]
pub(super) struct RouteDecisionRow {
    pub(super) file: String,
    pub(super) dest_path: String,
}

/// Outcome of a route review "apply", file by file.
#[derive(Debug, Default, PartialEq)]
pub(super) struct RouteApplyReport {
    /// Files moved to their destination.
    pub(super) moved: Vec<String>,
    /// Files that were no longer in staging: nothing left to route.
    pub(super) missing: Vec<String>,
    /// Files that could not be moved, with the reason.
    pub(super) failed: Vec<(String, String)>,
    /// Moved notes carrying a source proof: their server copy can now be deleted.
    pub(super) deletions: Vec<crate::contextual_export::DeletionRequest>,
}

impl RouteApplyReport {
    /// `true` when every decision ended with its file moved.
    pub(super) fn is_complete(&self) -> bool {
        self.missing.is_empty() && self.failed.is_empty()
    }

    /// Files whose row has nothing left to route (moved or gone).
    pub(super) fn settled_files(&self) -> Vec<String> {
        self.moved.iter().chain(&self.missing).cloned().collect()
    }

    /// User-facing summary of what did not move, `None` when everything did.
    pub(super) fn error_message(&self) -> Option<String> {
        let mut parts = Vec::new();
        if !self.missing.is_empty() {
            parts.push(format!(
                "{} fichier(s) introuvable(s) dans le transit :\n{}",
                self.missing.len(),
                self.missing.join("\n")
            ));
        }
        if !self.failed.is_empty() {
            let lines: Vec<String> = self
                .failed
                .iter()
                .map(|(file, reason)| format!("{}: {}", file, reason))
                .collect();
            parts.push(format!(
                "Déplacement échoué pour {} fichier(s) :\n{}",
                self.failed.len(),
                lines.join("\n")
            ));
        }
        (!parts.is_empty()).then(|| parts.join("\n"))
    }
}

/// Decode the route review "apply" payload `{ decisions: [{ file, dest_path }] }`.
pub(super) fn parse_route_decisions(body: &str) -> anyhow::Result<Vec<RouteDecisionRow>> {
    #[derive(serde::Deserialize)]
    struct RouteApplyPayload {
        decisions: Vec<RouteDecisionRow>,
    }
    let payload: RouteApplyPayload =
        serde_json::from_str(body).context("failed to parse route review IPC payload")?;
    Ok(payload.decisions)
}

/// Final destination of a row, carrying the email's `<Year>/<Month>`.
///
/// The auto proposal already ends with it; a manually reassigned path (cascade /
/// free entry / bulk) comes bare from destinations.txt — append it from the email
/// date so files always land under `<dest>/<Year>/<Month>` (no double suffix).
/// Skipped entirely when the "organize by date" preference is off, so a bare
/// manual path stays flat instead of being silently re-dated.
fn dated_dest_path(row: &RouteDecisionRow, organize_by_date: bool) -> String {
    if !organize_by_date {
        return row.dest_path.clone();
    }
    match read_frontmatter_field(Path::new(&row.file), "date")
        .and_then(|d| chrono::DateTime::parse_from_rfc3339(d.trim()).ok())
    {
        Some(dt) => crate::route::ensure_year_month(
            &row.dest_path,
            &dt.format("%Y").to_string(),
            &dt.format("%m").to_string(),
        ),
        // Date unreadable/unparseable → keep the path as-is (no guess).
        None => row.dest_path.clone(),
    }
}

/// Apply a batch of route decisions (move staged emails into their target folders).
///
/// `allowed` holds the staging files the host sent to the review window: a row
/// naming any other file is rejected. The whole batch is validated before the
/// first move, so a bad row moves nothing; once moves start, a failure is
/// recorded and the remaining rows still run. Moved and missing files leave
/// `allowed`. The move itself is `route::apply_decision`.
pub(super) fn apply_route_decisions(
    rows: &[RouteDecisionRow],
    notes_dir: &Path,
    allowed: &mut HashSet<String>,
    organize_by_date: bool,
) -> anyhow::Result<RouteApplyReport> {
    // Pass 1 — validation only, no side effect.
    let mut dest_paths = Vec::with_capacity(rows.len());
    for row in rows {
        if !allowed.contains(&row.file) {
            anyhow::bail!("file {:?} is not part of this review", row.file);
        }
        if row.dest_path.trim().is_empty() {
            anyhow::bail!("empty destination path for file {:?}", row.file);
        }
        let dest_path = dated_dest_path(row, organize_by_date);
        // Anti-traversal validation — rejects "..", "\", absolute paths.
        crate::route::join_safe_segments(notes_dir, &dest_path).with_context(|| {
            format!(
                "invalid destination path {:?} for file {:?}",
                dest_path, row.file
            )
        })?;
        dest_paths.push(dest_path);
    }

    // Pass 2 — move; a failure does not stop the following rows.
    let mut report = RouteApplyReport::default();
    for (row, dest_path) in rows.iter().zip(&dest_paths) {
        let staging_md = Path::new(&row.file);
        // A file gone since the window opened must not block the batch.
        if staging_md.symlink_metadata().is_err() {
            allowed.remove(&row.file);
            report.missing.push(row.file.clone());
            continue;
        }
        // Read before the move: the proof travels in the note's frontmatter.
        let proof = crate::contextual_export::read_source_proof(staging_md)
            .ok()
            .flatten()
            .filter(crate::contextual_export::proof_is_complete);
        match crate::route::apply_decision(staging_md, dest_path, notes_dir) {
            Ok(()) => {
                allowed.remove(&row.file);
                report.moved.push(row.file.clone());
                if let (Some(proof), Some(name)) = (proof, staging_md.file_name()) {
                    if let Ok(dest_dir) = crate::route::join_safe_segments(notes_dir, dest_path) {
                        report
                            .deletions
                            .push(crate::contextual_export::DeletionRequest {
                                markdown: dest_dir.join(name),
                                proof,
                            });
                    }
                }
            }
            Err(e) => report.failed.push((row.file.clone(), format!("{:#}", e))),
        }
    }
    Ok(report)
}

/// Source proofs of the given staging files, read before they are deleted.
pub(super) fn staged_proofs(
    files: &[String],
) -> Vec<(String, crate::contextual_export::LocalSourceProof)> {
    files
        .iter()
        .filter_map(|file| {
            crate::contextual_export::read_source_proof(Path::new(file))
                .ok()
                .flatten()
                .filter(crate::contextual_export::proof_is_complete)
                .map(|proof| (file.clone(), proof))
        })
        .collect()
}

/// Delete staged emails (and their attachment sidecars) that were rejected during routing.
///
/// A file absent from `allowed` (the staging files the host sent to the review
/// window) is refused. Deleted files leave `allowed`.
pub(super) fn delete_staged_emails(
    files: &[String],
    allowed: &mut HashSet<String>,
) -> (Vec<String>, Option<String>) {
    let mut deleted = Vec::new();
    let mut errors = Vec::new();
    for file in files {
        if !allowed.contains(file) {
            errors.push(format!("{}: fichier hors de cette revue", file));
            continue;
        }
        match crate::route::delete_email(Path::new(file)) {
            Ok(()) => {
                allowed.remove(file);
                deleted.push(file.clone());
            }
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

/// Read a YAML frontmatter block list (`field:` followed by `- item` lines).
/// Returns an empty list for `field: []`, a missing field or an unreadable file.
pub(super) fn read_frontmatter_list(path: &std::path::Path, field: &str) -> Vec<String> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let content = raw.replace("\r\n", "\n");
    let Some(rest) = content.strip_prefix("---\n") else {
        return Vec::new();
    };
    let Some(end) = rest.find("\n---") else {
        return Vec::new();
    };
    let key = format!("{field}:");
    let mut items = Vec::new();
    let mut in_list = false;
    for line in rest[..end].lines() {
        if in_list {
            match line.trim_start().strip_prefix("- ") {
                Some(item) => {
                    let item = item.trim().trim_matches(|c| c == '"' || c == '\'');
                    if !item.is_empty() {
                        items.push(item.to_string());
                    }
                    continue;
                }
                None => break,
            }
        }
        if let Some(value) = line.strip_prefix(&key) {
            in_list = value.trim().is_empty();
        }
    }
    items
}

#[derive(serde::Deserialize)]
struct ConfigIpcMessage {
    action: String,
    data: Option<Value>,
}

#[derive(serde::Deserialize)]
struct SettingsData {
    export_base_dir: Option<String>,
    #[serde(default = "default_organize_by_date_field")]
    organize_by_date: bool,
    defaults: DefaultsData,
}

fn default_organize_by_date_field() -> bool {
    true
}

#[derive(serde::Deserialize)]
struct DefaultsData {
    quote_depth: Option<usize>,
    skip_existing: Option<bool>,
    collect_contacts: Option<bool>,
    skip_signature_images: Option<bool>,
    delete_after_export: Option<bool>,
    cleanup_empty_dirs: Option<bool>,
    #[serde(default)]
    batch_size: Option<usize>,
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
    #[serde(default)]
    batch_size: Option<usize>,
    /// `true` when the form creates the account: an existing name is refused.
    #[serde(default)]
    create: bool,
    /// New password for `.env`. Empty or absent keeps the current one.
    #[serde(default)]
    password: Option<String>,
}

/// Write the general settings to `settings_path`, keeping every field the form does not edit.
fn save_settings(data: SettingsData, settings_path: &Path) -> anyhow::Result<()> {
    let mut settings = Settings::load(settings_path)
        .with_context(|| format!("failed to load settings from {}", settings_path.display()))?;
    settings.export_base_dir = data.export_base_dir;
    settings.organize_by_date = data.organize_by_date;
    settings.defaults = AccountBehavior {
        folder_name: settings.defaults.folder_name,
        quote_depth: data.defaults.quote_depth,
        skip_existing: data.defaults.skip_existing,
        collect_contacts: data.defaults.collect_contacts,
        skip_signature_images: data.defaults.skip_signature_images,
        delete_after_export: data.defaults.delete_after_export,
        cleanup_empty_dirs: data.defaults.cleanup_empty_dirs,
        batch_size: data.defaults.batch_size,
    };
    settings
        .save(settings_path)
        .with_context(|| format!("failed to save settings to {}", settings_path.display()))?;
    Ok(())
}

/// Characters that cannot appear in an account name: it names an export folder.
const FORBIDDEN_NAME_CHARS: &[char] = &['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

/// Reject an account name that is empty, unusable as a folder name, already taken,
/// or that would share its `.env` variable prefix with another account.
fn validate_new_account_name(name: &str, accounts: &[RawAccount]) -> anyhow::Result<()> {
    if name.trim().is_empty() {
        anyhow::bail!("Le nom du compte est obligatoire.");
    }
    if name != name.trim() || name.contains(FORBIDDEN_NAME_CHARS) {
        anyhow::bail!(
            "Le nom du compte ne doit pas commencer ou finir par un espace, ni contenir / \\ : * ? \" < > |"
        );
    }
    let prefix = config::env_var_name(name);
    for existing in accounts {
        if existing.name.eq_ignore_ascii_case(name) {
            anyhow::bail!(
                "Un compte nomm\u{00e9} \u{00ab} {} \u{00bb} existe d\u{00e9}j\u{00e0}.",
                existing.name
            );
        }
        if config::env_var_name(&existing.name) == prefix {
            anyhow::bail!(
                "Le nom est trop proche de \u{00ab} {} \u{00bb} : ils partageraient le m\u{00ea}me mot de passe ({prefix}_PASSWORD).",
                existing.name
            );
        }
    }
    Ok(())
}

/// `.env` keys that can hold the password of `account_name`.
fn env_password_keys(account_name: &str) -> [String; 2] {
    let prefix = config::env_var_name(account_name);
    [
        format!("{prefix}_APPLICATION_PASSWORD"),
        format!("{prefix}_PASSWORD"),
    ]
}

fn read_env_lines(env_path: &Path) -> anyhow::Result<Vec<String>> {
    if !env_path.exists() {
        return Ok(Vec::new());
    }
    Ok(std::fs::read_to_string(env_path)
        .with_context(|| format!("failed to read {}", env_path.display()))?
        .lines()
        .map(str::to_string)
        .collect())
}

fn write_env_lines(env_path: &Path, lines: &[String]) -> anyhow::Result<()> {
    if let Some(parent) = env_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut content = lines.join("\n");
    if !content.is_empty() {
        content.push('\n');
    }
    std::fs::write(env_path, content)
        .with_context(|| format!("failed to write {}", env_path.display()))
}

fn env_line_has_key(line: &str, key: &str) -> bool {
    line.trim_start()
        .strip_prefix(key)
        .is_some_and(|rest| rest.starts_with('='))
}

/// Store `password` for `account_name` in `.env`.
///
/// `<NAME>_APPLICATION_PASSWORD` wins over `<NAME>_PASSWORD` when loading, so an
/// existing application password is the key to update.
fn set_env_password(env_path: &Path, account_name: &str, password: &str) -> anyhow::Result<()> {
    let mut lines = read_env_lines(env_path)?;
    let [app_key, plain_key] = env_password_keys(account_name);
    let key = if lines.iter().any(|l| env_line_has_key(l, &app_key)) {
        app_key
    } else {
        plain_key
    };
    let line = format!("{}='{}'", key, password.replace('\'', "'\\''"));
    match lines.iter().position(|l| env_line_has_key(l, &key)) {
        Some(pos) => lines[pos] = line,
        None => lines.push(line),
    }
    write_env_lines(env_path, &lines)
}

/// Drop every password line of `account_name` from `.env`. Returns how many were removed.
fn remove_env_passwords(env_path: &Path, account_name: &str) -> anyhow::Result<usize> {
    if !env_path.exists() {
        return Ok(0);
    }
    let lines = read_env_lines(env_path)?;
    let keys = env_password_keys(account_name);
    let kept: Vec<String> = lines
        .iter()
        .filter(|l| !keys.iter().any(|k| env_line_has_key(l, k)))
        .cloned()
        .collect();
    let removed = lines.len() - kept.len();
    if removed > 0 {
        write_env_lines(env_path, &kept)?;
    }
    Ok(removed)
}

/// Remove an account: its entry in `accounts.yaml`, its behaviour overrides in
/// `settings.yaml` and its password lines in `.env`. Exported emails stay on disk.
///
/// Both YAML files are loaded before the first write. Returns the number of
/// password lines removed from `.env`.
fn delete_account(
    account_name: &str,
    accounts_path: &Path,
    settings_path: &Path,
    env_path: &Path,
) -> anyhow::Result<usize> {
    let mut accounts = config::load_raw_accounts(accounts_path)
        .with_context(|| format!("failed to load accounts from {}", accounts_path.display()))?;
    let mut settings = Settings::load(settings_path)
        .with_context(|| format!("failed to load settings from {}", settings_path.display()))?;

    let before = accounts.len();
    accounts.retain(|a| !a.name.eq_ignore_ascii_case(account_name));
    if accounts.len() == before {
        anyhow::bail!("Compte introuvable : {account_name}");
    }
    settings
        .accounts
        .retain(|key, _| !key.eq_ignore_ascii_case(account_name));

    config::save_accounts(&accounts, accounts_path)
        .with_context(|| format!("failed to save accounts to {}", accounts_path.display()))?;
    settings
        .save(settings_path)
        .with_context(|| format!("failed to save settings to {}", settings_path.display()))?;
    remove_env_passwords(env_path, account_name)
}

/// Write one account: connection info to `accounts_path`, behaviour to `settings_path`.
///
/// Both files are loaded before the first write, so an unreadable `settings.yaml`
/// never leaves `accounts.yaml` half-updated.
fn save_account(
    data: AccountData,
    accounts_path: &Path,
    settings_path: &Path,
) -> anyhow::Result<()> {
    let mut accounts = config::load_raw_accounts(accounts_path)
        .with_context(|| format!("failed to load accounts from {}", accounts_path.display()))?;
    let mut settings = Settings::load(settings_path)
        .with_context(|| format!("failed to load settings from {}", settings_path.display()))?;

    if data.create {
        validate_new_account_name(&data.account_name, &accounts)?;
    }

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
    behavior.batch_size = data.batch_size;

    let is_empty = serde_json::to_value(&behavior)
        .map(|v| v.as_object().map(|o| o.is_empty()).unwrap_or(false))
        .unwrap_or(false);

    if is_empty {
        settings.accounts.remove(&canonical_key);
    } else {
        settings.accounts.insert(canonical_key, behavior);
    }

    config::save_accounts(&accounts, accounts_path)
        .with_context(|| format!("failed to save accounts to {}", accounts_path.display()))?;
    settings
        .save(settings_path)
        .with_context(|| format!("failed to save settings to {}", settings_path.display()))?;

    Ok(())
}

/// What the settings window must do after one IPC message.
pub(super) struct ConfigIpcOutcome {
    /// Notification for the tray, if any.
    pub(super) result: Option<ActionResult>,
    pub(super) should_close: bool,
    /// JSON answer for the page (`config_result`), sent for every save request.
    pub(super) reply: Option<String>,
}

impl ConfigIpcOutcome {
    fn new(result: Option<ActionResult>, should_close: bool) -> Self {
        Self {
            result,
            should_close,
            reply: None,
        }
    }
}

/// Decide what follows a save request (`save` or `save_account`) from its result.
///
/// No disk access. Only a successful `save` closes the window: a failed save keeps
/// it open so the page can show the cause and keep what was typed.
fn save_outcome(action: &str, saved: anyhow::Result<()>) -> ConfigIpcOutcome {
    match saved {
        Ok(()) => {
            let closes = action == "save";
            ConfigIpcOutcome {
                result: closes.then(|| {
                    ActionResult::Success(
                        "Param\u{00e8}tres".to_string(),
                        "Param\u{00e8}tres sauvegard\u{00e9}s".to_string(),
                    )
                }),
                should_close: closes,
                reply: Some(serde_json::json!({"action": action, "ok": true}).to_string()),
            }
        }
        Err(e) => {
            let message = format!("Erreur de sauvegarde : {:#}", e);
            ConfigIpcOutcome {
                result: Some(ActionResult::Error(message.clone())),
                should_close: false,
                reply: Some(
                    serde_json::json!({"action": action, "ok": false, "message": message})
                        .to_string(),
                ),
            }
        }
    }
}

/// Re-read `settings.yaml` and `accounts.yaml` from disk for the settings page,
/// so edits made directly in the files show up without reopening the window.
///
/// A file that no longer parses keeps the page as it was and reports the cause.
fn reload_outcome(
    settings_path: &Path,
    accounts_path: &Path,
    notice: Option<&str>,
) -> ConfigIpcOutcome {
    let loaded = (|| -> anyhow::Result<serde_json::Value> {
        let settings = config::Settings::load(settings_path)
            .with_context(|| format!("failed to load {}", settings_path.display()))?;
        let accounts = config::load_raw_accounts(accounts_path)
            .with_context(|| format!("failed to load {}", accounts_path.display()))?;
        Ok(serde_json::json!({
            "action": "reload",
            "ok": true,
            "settings": settings,
            "accounts": accounts,
            "notice": notice,
        }))
    })();
    let reply = match loaded {
        Ok(value) => value,
        Err(e) => serde_json::json!({
            "action": "reload",
            "ok": false,
            "message": format!("Rechargement impossible : {:#}", e),
        }),
    };
    ConfigIpcOutcome {
        result: None,
        should_close: false,
        reply: Some(reply.to_string()),
    }
}

/// Decode and dispatch one settings-window IPC message.
pub(super) fn handle_config_ipc(body: &str) -> ConfigIpcOutcome {
    let msg: ConfigIpcMessage = match serde_json::from_str(body) {
        Ok(m) => m,
        Err(e) => {
            return ConfigIpcOutcome::new(
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
            let result = (|| -> anyhow::Result<()> {
                let raw_data = msg
                    .data
                    .ok_or_else(|| anyhow::anyhow!("save action missing data field"))?;
                let data: SettingsData =
                    serde_json::from_value(raw_data).context("failed to parse settings data")?;
                save_settings(data, &config::settings_path())
            })();
            save_outcome("save", result)
        }
        "save_account" => {
            let result = (|| -> anyhow::Result<()> {
                let raw_data = msg
                    .data
                    .ok_or_else(|| anyhow::anyhow!("save_account action missing data field"))?;
                let data: AccountData =
                    serde_json::from_value(raw_data).context("failed to parse account data")?;
                let password = data.password.clone().filter(|p| !p.is_empty());
                let name = data.account_name.clone();
                save_account(
                    data,
                    &config::accounts_yaml_path(),
                    &config::settings_path(),
                )?;
                if let Some(password) = password {
                    set_env_password(&config::env_file_path(), &name, &password)
                        .context("compte enregistr\u{00e9}, mais mot de passe non \u{00e9}crit")?;
                }
                Ok(())
            })();
            save_outcome("save_account", result)
        }
        "open_raw" => {
            if let Err(e) = action_open_config().context("failed to open settings file in editor") {
                return ConfigIpcOutcome::new(
                    Some(ActionResult::Error(format!("Erreur : {:#}", e))),
                    false,
                );
            }
            ConfigIpcOutcome::new(None, false)
        }
        "open_dir" => {
            let dir = config::app_config_dir();
            let opened = std::fs::create_dir_all(&dir)
                .map_err(anyhow::Error::from)
                .and_then(|()| open::that(&dir).map_err(anyhow::Error::from))
                .context("failed to open config folder");
            match opened {
                Ok(()) => ConfigIpcOutcome::new(None, false),
                Err(e) => ConfigIpcOutcome::new(
                    Some(ActionResult::Error(format!("Erreur : {:#}", e))),
                    false,
                ),
            }
        }
        "reload" => reload_outcome(
            &config::settings_path(),
            &config::accounts_yaml_path(),
            None,
        ),
        "delete_account" => {
            let name = msg
                .data
                .as_ref()
                .and_then(|d| d.get("account_name"))
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let deleted = name
                .ok_or_else(|| anyhow::anyhow!("delete_account action missing account_name"))
                .and_then(|name| {
                    delete_account(
                        &name,
                        &config::accounts_yaml_path(),
                        &config::settings_path(),
                        &config::env_file_path(),
                    )
                    .map(|removed| (name, removed))
                });
            match deleted {
                Ok((name, removed)) => {
                    let notice = if removed > 0 {
                        format!("Compte \u{00ab} {name} \u{00bb} supprim\u{00e9}, mot de passe effac\u{00e9} de .env.")
                    } else {
                        format!("Compte \u{00ab} {name} \u{00bb} supprim\u{00e9} (aucun mot de passe dans .env).")
                    };
                    reload_outcome(
                        &config::settings_path(),
                        &config::accounts_yaml_path(),
                        Some(&notice),
                    )
                }
                Err(e) => ConfigIpcOutcome {
                    result: None,
                    should_close: false,
                    reply: Some(
                        serde_json::json!({
                            "action": "reload",
                            "ok": false,
                            "message": format!("Suppression impossible : {:#}", e),
                        })
                        .to_string(),
                    ),
                },
            }
        }
        other => ConfigIpcOutcome::new(
            Some(ActionResult::Error(format!(
                "unknown IPC action '{}'",
                other
            ))),
            true,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_route_decisions, delete_account, delete_staged_emails, read_frontmatter_list,
        relative_destination, reload_outcome, remove_env_passwords, save_account, save_outcome,
        save_settings, scan_destinations, set_env_password, validate_new_account_name, AccountData,
        DestGuiIpcResult, RouteApplyReport, RouteDecisionRow, SettingsData,
    };
    use crate::config::{load_raw_accounts, RawAccount, Settings};
    use crate::destinations::DestinationsConfig;
    use std::collections::HashSet;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    #[test]
    fn test_save_outcome_with_error() {
        let outcome = save_outcome("save", Err(anyhow::anyhow!("disk full")));
        assert!(!outcome.should_close);
    }

    #[test]
    fn test_save_outcome_with_error_reply() {
        let outcome = save_outcome("save", Err(anyhow::anyhow!("disk full")));
        let reply: serde_json::Value =
            serde_json::from_str(&outcome.reply.unwrap_or_default()).unwrap();
        assert_eq!(
            (reply["ok"].as_bool(), reply["message"].as_str()),
            (Some(false), Some("Erreur de sauvegarde : disk full"))
        );
    }

    #[test]
    fn test_save_outcome_with_settings_saved() {
        let outcome = save_outcome("save", Ok(()));
        assert!(outcome.should_close);
    }

    #[test]
    fn test_save_outcome_with_account_saved() {
        let outcome = save_outcome("save_account", Ok(()));
        assert!(!outcome.should_close);
    }

    #[test]
    fn test_scan_destinations_with_unconfigured_notes_dir() {
        let result = scan_destinations(None, &DestinationsConfig::default(), "scan_folders");
        assert!(result.is_err());
    }

    #[test]
    fn test_scan_destinations_with_missing_notes_dir() {
        let temp = TempDir::new().unwrap();
        let missing = temp.path().join("absent");
        let result = scan_destinations(
            Some(&missing),
            &DestinationsConfig::default(),
            "scan_folders",
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_scan_destinations_with_empty_notes_dir() {
        let temp = TempDir::new().unwrap();
        let result = scan_destinations(
            Some(temp.path()),
            &DestinationsConfig::default(),
            "scan_folders",
        );
        assert!(matches!(
            result,
            Ok(DestGuiIpcResult::FolderSuggestions { new_folders, orphans })
                if new_folders.is_empty() && orphans.is_empty()
        ));
    }

    #[test]
    fn test_relative_destination_with_nested_folder() {
        let temp = TempDir::new().unwrap();
        let picked = temp.path().join("Perso").join("Banque");
        fs::create_dir_all(&picked).unwrap();
        let result = relative_destination(temp.path(), &picked).unwrap();
        assert_eq!(result, "Perso/Banque");
    }

    #[test]
    fn test_relative_destination_with_notes_dir_itself() {
        let temp = TempDir::new().unwrap();
        assert!(relative_destination(temp.path(), temp.path()).is_err());
    }

    #[test]
    fn test_relative_destination_with_folder_outside_notes_dir() {
        let temp = TempDir::new().unwrap();
        let notes = temp.path().join("notes");
        let outside = temp.path().join("ailleurs");
        fs::create_dir_all(&notes).unwrap();
        fs::create_dir_all(&outside).unwrap();
        assert!(relative_destination(&notes, &outside).is_err());
    }

    /// Staging dir with three emails dated 2026-06, plus an empty notes dir.
    struct RouteFixture {
        _temp: TempDir,
        staging: PathBuf,
        notes: PathBuf,
        files: Vec<String>,
    }

    fn route_fixture() -> RouteFixture {
        let temp = TempDir::new().unwrap();
        let staging = temp.path().join("staging");
        let notes = temp.path().join("notes");
        fs::create_dir_all(&staging).unwrap();
        fs::create_dir_all(&notes).unwrap();
        let files = ["a.md", "b.md", "c.md"]
            .iter()
            .map(|name| {
                let path = staging.join(name);
                fs::write(
                    &path,
                    "---\ndate: 2026-06-15T10:00:00+00:00\nsubject: test\n---\nbody\n",
                )
                .unwrap();
                path.to_string_lossy().into_owned()
            })
            .collect();
        RouteFixture {
            _temp: temp,
            staging,
            notes,
            files,
        }
    }

    fn rows(pairs: &[(&str, &str)]) -> Vec<RouteDecisionRow> {
        pairs
            .iter()
            .map(|(file, dest_path)| RouteDecisionRow {
                file: file.to_string(),
                dest_path: dest_path.to_string(),
            })
            .collect()
    }

    fn allowed(files: &[String]) -> HashSet<String> {
        files.iter().cloned().collect()
    }

    /// Names of the `.md` files directly under `dir`, sorted.
    fn md_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    #[test]
    fn test_apply_route_decisions_valid_moves_all_files() {
        let fx = route_fixture();
        let decisions = rows(&[
            (&fx.files[0], "Perso/Banque"),
            (&fx.files[1], "Perso/Banque"),
            (&fx.files[2], "Travail"),
        ]);

        apply_route_decisions(&decisions, &fx.notes, &mut allowed(&fx.files), false).unwrap();

        assert_eq!(
            (
                md_names(&fx.notes.join("Perso").join("Banque")),
                md_names(&fx.notes.join("Travail")),
                md_names(&fx.staging),
            ),
            (
                vec!["a.md".to_string(), "b.md".to_string()],
                vec!["c.md".to_string()],
                Vec::<String>::new(),
            )
        );
    }

    #[test]
    fn test_apply_route_decisions_organize_by_date_single_suffix() {
        let fx = route_fixture();
        let decisions = rows(&[
            (&fx.files[0], "Perso/Banque"),
            (&fx.files[1], "Perso/Banque/2026/06"),
        ]);
        let dated = fx
            .notes
            .join("Perso")
            .join("Banque")
            .join("2026")
            .join("06");

        apply_route_decisions(&decisions, &fx.notes, &mut allowed(&fx.files), true).unwrap();

        assert_eq!(
            (md_names(&dated), dated.join("2026").exists()),
            (vec!["a.md".to_string(), "b.md".to_string()], false)
        );
    }

    #[test]
    fn test_apply_route_decisions_invalid_path_moves_nothing() {
        let fx = route_fixture();
        let decisions = rows(&[
            (&fx.files[0], "Perso"),
            (&fx.files[1], "../outside"),
            (&fx.files[2], "Travail"),
        ]);

        let err = apply_route_decisions(&decisions, &fx.notes, &mut allowed(&fx.files), false)
            .unwrap_err();

        assert!(format!("{:#}", err).contains("b.md"));
        assert_eq!(md_names(&fx.staging), ["a.md", "b.md", "c.md"]);
        assert!(!fx.notes.join("Perso").exists());
    }

    #[test]
    fn test_apply_route_decisions_unlisted_file_moves_nothing() {
        let fx = route_fixture();
        let decisions = rows(&[(&fx.files[0], "Perso"), (&fx.files[1], "Perso")]);
        let mut allowed = allowed(&fx.files[..1]);

        let err = apply_route_decisions(&decisions, &fx.notes, &mut allowed, false).unwrap_err();

        assert!(format!("{:#}", err).contains("b.md"));
        assert_eq!(md_names(&fx.staging), ["a.md", "b.md", "c.md"]);
        assert!(!fx.notes.join("Perso").exists());
    }

    #[test]
    fn test_apply_route_decisions_missing_file_reports_and_continues() {
        let fx = route_fixture();
        let decisions = rows(&[
            (&fx.files[0], "Perso"),
            (&fx.files[1], "Perso"),
            (&fx.files[2], "Perso"),
        ]);
        fs::remove_file(&fx.files[1]).unwrap();
        let mut allowed = allowed(&fx.files);

        let report = apply_route_decisions(&decisions, &fx.notes, &mut allowed, false).unwrap();

        assert_eq!(
            report,
            RouteApplyReport {
                moved: vec![fx.files[0].clone(), fx.files[2].clone()],
                missing: vec![fx.files[1].clone()],
                failed: vec![],
                deletions: vec![],
            }
        );
        assert_eq!(md_names(&fx.notes.join("Perso")), ["a.md", "c.md"]);
        assert!(allowed.is_empty());
    }

    #[test]
    fn test_apply_route_decisions_proof_yields_deletion_request_on_final_path() {
        let fx = route_fixture();
        let proof = "---\nsubject: s\nsource:\n  identity:\n    account: perso\n    message_id: m1@example.org\n    header_fingerprint: abc123\n  location:\n    folder_raw: INBOX\n    folder_display: INBOX\n    uid_validity: 7\n    uid: 42\n---\ncorps\n";
        fs::write(&fx.files[0], proof).unwrap();
        let decisions = rows(&[(&fx.files[0], "Perso")]);
        let mut allowed = allowed(&fx.files);

        let report = apply_route_decisions(&decisions, &fx.notes, &mut allowed, false).unwrap();

        assert_eq!(report.deletions.len(), 1);
        assert_eq!(
            report.deletions[0].markdown,
            fx.notes.join("Perso").join("a.md")
        );
        assert_eq!(report.deletions[0].proof.location.uid, 42);
    }

    #[test]
    fn test_staged_proofs_keeps_only_files_with_complete_proof() {
        let fx = route_fixture();
        let proof = "---\nsubject: s\nsource:\n  identity:\n    account: perso\n    message_id: m1@example.org\n    header_fingerprint: abc123\n  location:\n    folder_raw: INBOX\n    folder_display: INBOX\n    uid_validity: 7\n    uid: 42\n---\ncorps\n";
        fs::write(&fx.files[0], proof).unwrap();

        let proofs = super::staged_proofs(&fx.files);

        assert_eq!(proofs.len(), 1);
        assert_eq!(proofs[0].0, fx.files[0]);
    }

    #[test]
    fn test_apply_route_decisions_without_proof_yields_no_deletion() {
        let fx = route_fixture();
        let decisions = rows(&[(&fx.files[0], "Perso")]);
        let mut allowed = allowed(&fx.files);

        let report = apply_route_decisions(&decisions, &fx.notes, &mut allowed, false).unwrap();

        assert!(report.deletions.is_empty());
    }

    #[test]
    fn test_route_apply_report_error_message_names_missing_file_once() {
        let report = RouteApplyReport {
            moved: vec!["a.md".to_string()],
            missing: vec!["b.md".to_string()],
            failed: vec![],
            deletions: vec![],
        };

        let message = report.error_message().unwrap();

        assert_eq!(message.matches("b.md").count(), 1);
        assert!(!message.contains("a.md"));
    }

    #[test]
    fn test_route_apply_report_all_moved_is_complete() {
        let report = RouteApplyReport {
            moved: vec!["a.md".to_string()],
            ..Default::default()
        };

        assert_eq!((report.is_complete(), report.error_message()), (true, None));
    }

    #[test]
    fn test_delete_staged_emails_listed_file_deletes_it() {
        let fx = route_fixture();
        let mut allowed = allowed(&fx.files);

        let (deleted, err) = delete_staged_emails(&fx.files[..1], &mut allowed);

        assert_eq!((deleted, err), (vec![fx.files[0].clone()], None));
        assert_eq!(md_names(&fx.staging), ["b.md", "c.md"]);
        assert!(!allowed.contains(&fx.files[0]));
    }

    #[test]
    fn test_delete_staged_emails_unlisted_file_keeps_file() {
        let fx = route_fixture();
        let mut allowed = allowed(&fx.files[..1]);

        let (deleted, err) = delete_staged_emails(&fx.files[1..2], &mut allowed);

        assert!(deleted.is_empty());
        assert!(err.unwrap().contains("b.md"));
        assert_eq!(md_names(&fx.staging), ["a.md", "b.md", "c.md"]);
    }

    const INVALID_YAML: &str = "export_base_dir: [unclosed\naccounts: : :\n";
    const VALID_ACCOUNTS: &str =
        "accounts:\n  - name: Perso\n    server: imap.example.org\n    port: 993\n    username: perso@example.org\n";

    fn settings_data(export_base_dir: &str) -> SettingsData {
        serde_json::from_value(serde_json::json!({
            "export_base_dir": export_base_dir,
            "organize_by_date": true,
            "defaults": {}
        }))
        .unwrap()
    }

    fn account_data(name: &str) -> AccountData {
        serde_json::from_value(serde_json::json!({
            "account_name": name,
            "server": "imap.example.com",
            "port": 993,
            "username": "pro@example.com",
            "quote_depth": 2
        }))
        .unwrap()
    }

    fn account_files(temp: &TempDir) -> (PathBuf, PathBuf, PathBuf) {
        (
            temp.path().join("accounts.yaml"),
            temp.path().join("settings.yaml"),
            temp.path().join(".env"),
        )
    }

    #[test]
    fn test_delete_account_removes_entry_override_and_passwords() {
        let temp = TempDir::new().unwrap();
        let (accounts, settings, env) = account_files(&temp);
        fs::write(&accounts, VALID_ACCOUNTS).unwrap();
        let mut data = account_data("Perso");
        data.delete_after_export = Some(true);
        save_account(data, &accounts, &settings).unwrap();
        fs::write(
            &env,
            "PERSO_PASSWORD='a'\nPERSO_APPLICATION_PASSWORD='b'\nOTHER_PASSWORD='c'\n",
        )
        .unwrap();

        let removed = delete_account("perso", &accounts, &settings, &env).unwrap();

        assert_eq!(removed, 2);
        assert!(load_raw_accounts(&accounts).unwrap().is_empty());
        assert!(Settings::load(&settings).unwrap().accounts.is_empty());
        assert_eq!(fs::read_to_string(&env).unwrap(), "OTHER_PASSWORD='c'\n");
    }

    #[test]
    fn test_delete_account_unknown_name_changes_nothing() {
        let temp = TempDir::new().unwrap();
        let (accounts, settings, env) = account_files(&temp);
        fs::write(&accounts, VALID_ACCOUNTS).unwrap();
        fs::write(&env, "PERSO_PASSWORD='a'\n").unwrap();

        let result = delete_account("Inconnu", &accounts, &settings, &env);

        assert!(result.is_err());
        assert_eq!(load_raw_accounts(&accounts).unwrap().len(), 1);
        assert_eq!(fs::read_to_string(&env).unwrap(), "PERSO_PASSWORD='a'\n");
    }

    #[test]
    fn test_remove_env_passwords_without_env_file_is_noop() {
        let temp = TempDir::new().unwrap();
        let env = temp.path().join(".env");

        assert_eq!(remove_env_passwords(&env, "Perso").unwrap(), 0);
        assert!(!env.exists());
    }

    #[test]
    fn test_set_env_password_updates_application_password_key() {
        let temp = TempDir::new().unwrap();
        let env = temp.path().join(".env");
        fs::write(
            &env,
            "PERSO_APPLICATION_PASSWORD='old'\nPERSO_PASSWORD='x'\n",
        )
        .unwrap();

        set_env_password(&env, "Perso", "it's new").unwrap();

        assert_eq!(
            fs::read_to_string(&env).unwrap(),
            "PERSO_APPLICATION_PASSWORD='it'\\''s new'\nPERSO_PASSWORD='x'\n"
        );
    }

    #[test]
    fn test_set_env_password_appends_plain_key_when_absent() {
        let temp = TempDir::new().unwrap();
        let env = temp.path().join(".env");
        fs::write(&env, "OTHER_PASSWORD='c'\n").unwrap();

        set_env_password(&env, "Mon Compte", "secret").unwrap();

        assert_eq!(
            fs::read_to_string(&env).unwrap(),
            "OTHER_PASSWORD='c'\nMON_COMPTE_PASSWORD='secret'\n"
        );
    }

    #[test]
    fn test_save_account_create_refuses_existing_name() {
        let temp = TempDir::new().unwrap();
        let (accounts, settings, _) = account_files(&temp);
        fs::write(&accounts, VALID_ACCOUNTS).unwrap();
        let mut data = account_data("PERSO");
        data.create = true;

        assert!(save_account(data, &accounts, &settings).is_err());
        assert_eq!(load_raw_accounts(&accounts).unwrap().len(), 1);
    }

    #[test]
    fn test_validate_new_account_name_rejects_forbidden_chars_and_empty() {
        assert!(validate_new_account_name("", &[]).is_err());
        assert!(validate_new_account_name("a/b", &[]).is_err());
        assert!(validate_new_account_name(" pro", &[]).is_err());
        assert!(validate_new_account_name("Pro", &[]).is_ok());
    }

    #[test]
    fn test_validate_new_account_name_rejects_env_prefix_collision() {
        let existing = vec![RawAccount {
            name: "Mon Compte".to_string(),
            server: "imap.example.org".to_string(),
            port: 993,
            username: "u".to_string(),
            ignored_folders: Vec::new(),
        }];

        assert!(validate_new_account_name("Mon-Compte", &existing).is_err());
    }

    #[test]
    fn test_save_settings_valid_keeps_unedited_fields() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("settings.yaml");
        fs::write(&path, "notes_dir: C:/notes\nexport_base_dir: C:/old\n").unwrap();

        save_settings(settings_data("C:/new"), &path).unwrap();

        let saved = Settings::load(&path).unwrap();
        assert_eq!(
            (saved.notes_dir.as_deref(), saved.export_base_dir.as_deref()),
            (Some("C:/notes"), Some("C:/new"))
        );
    }

    #[test]
    fn test_save_settings_missing_file_creates_it() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("settings.yaml");

        save_settings(settings_data("C:/new"), &path).unwrap();

        let saved = Settings::load(&path).unwrap();
        assert_eq!(saved.export_base_dir.as_deref(), Some("C:/new"));
    }

    #[test]
    fn test_save_settings_invalid_yaml_keeps_file() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("settings.yaml");
        fs::write(&path, INVALID_YAML).unwrap();

        let err = save_settings(settings_data("C:/new"), &path).unwrap_err();

        assert!(format!("{:#}", err).contains("settings.yaml"));
        assert_eq!(fs::read(&path).unwrap(), INVALID_YAML.as_bytes());
    }

    #[test]
    fn test_reload_outcome_valid_files_returns_accounts_and_settings() {
        let temp = TempDir::new().unwrap();
        let accounts_path = temp.path().join("accounts.yaml");
        let settings_path = temp.path().join("settings.yaml");
        fs::write(&accounts_path, VALID_ACCOUNTS).unwrap();

        let outcome = reload_outcome(&settings_path, &accounts_path, None);

        let reply: serde_json::Value = serde_json::from_str(&outcome.reply.unwrap()).unwrap();
        assert_eq!(reply["ok"], true);
        assert!(reply["accounts"].as_array().is_some_and(|a| !a.is_empty()));
        assert!(!outcome.should_close);
    }

    #[test]
    fn test_reload_outcome_invalid_accounts_reports_error() {
        let temp = TempDir::new().unwrap();
        let accounts_path = temp.path().join("accounts.yaml");
        let settings_path = temp.path().join("settings.yaml");
        fs::write(&accounts_path, INVALID_YAML).unwrap();

        let outcome = reload_outcome(&settings_path, &accounts_path, None);

        let reply: serde_json::Value = serde_json::from_str(&outcome.reply.unwrap()).unwrap();
        assert_eq!(reply["ok"], false);
        assert!(reply["message"].as_str().unwrap().contains("accounts.yaml"));
    }

    #[test]
    fn test_save_account_valid_adds_account_and_keeps_others() {
        let temp = TempDir::new().unwrap();
        let accounts_path = temp.path().join("accounts.yaml");
        let settings_path = temp.path().join("settings.yaml");
        fs::write(&accounts_path, VALID_ACCOUNTS).unwrap();

        save_account(account_data("Pro"), &accounts_path, &settings_path).unwrap();

        let names: Vec<String> = load_raw_accounts(&accounts_path)
            .unwrap()
            .into_iter()
            .map(|a| a.name)
            .collect();
        assert_eq!(names, vec!["Perso".to_string(), "Pro".to_string()]);
        let settings = Settings::load(&settings_path).unwrap();
        assert_eq!(settings.accounts["Pro"].quote_depth, Some(2));
    }

    #[test]
    fn test_save_account_invalid_accounts_yaml_keeps_file() {
        let temp = TempDir::new().unwrap();
        let accounts_path = temp.path().join("accounts.yaml");
        let settings_path = temp.path().join("settings.yaml");
        fs::write(&accounts_path, INVALID_YAML).unwrap();

        let err = save_account(account_data("Pro"), &accounts_path, &settings_path).unwrap_err();

        assert!(format!("{:#}", err).contains("accounts.yaml"));
        assert_eq!(fs::read(&accounts_path).unwrap(), INVALID_YAML.as_bytes());
        assert!(!settings_path.exists());
    }

    #[test]
    fn test_save_account_invalid_settings_yaml_keeps_both_files() {
        let temp = TempDir::new().unwrap();
        let accounts_path = temp.path().join("accounts.yaml");
        let settings_path = temp.path().join("settings.yaml");
        fs::write(&accounts_path, VALID_ACCOUNTS).unwrap();
        fs::write(&settings_path, INVALID_YAML).unwrap();

        let err = save_account(account_data("Pro"), &accounts_path, &settings_path).unwrap_err();

        assert!(format!("{:#}", err).contains("settings.yaml"));
        assert_eq!(fs::read(&accounts_path).unwrap(), VALID_ACCOUNTS.as_bytes());
        assert_eq!(fs::read(&settings_path).unwrap(), INVALID_YAML.as_bytes());
    }

    #[test]
    fn test_read_frontmatter_list_with_items() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("note.md");
        fs::write(
            &path,
            "---\nsubject: Hi\nattachments:\n- a.pdf\n- \"b c.png\"\ntags: []\n---\nbody\n",
        )
        .unwrap();

        assert_eq!(
            read_frontmatter_list(&path, "attachments"),
            vec!["a.pdf".to_string(), "b c.png".to_string()]
        );
    }

    #[test]
    fn test_read_frontmatter_list_empty() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("note.md");
        fs::write(&path, "---\nattachments: []\n---\nbody\n").unwrap();

        assert!(read_frontmatter_list(&path, "attachments").is_empty());
    }
}
