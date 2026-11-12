//! IPC message decoding for the destinations-management and settings webviews.

use std::collections::HashSet;
use std::path::Path;

use anyhow::Context;
use serde_json::Value;

use crate::config::{self, AccountBehavior, RawAccount, Settings};
use crate::date_layout::DateLayout;
use crate::email_export::{describe_connection_error, probe_account, ListedFolder, PROBE_TIMEOUT};
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
    FolderSuggestions {
        /// Uncovered destination folders with their email count.
        new_folders: Vec<(String, usize)>,
        orphans: Vec<String>,
    },
    /// The window asked for a notes scan (`scan_folders`); it walks `notes_dir`,
    /// so it runs in a worker thread through [`handle_scan`].
    Scan,
    /// The window asked for per-year email counts (`dest_stats`); it reads every
    /// note, so it runs in a worker thread through [`handle_dest_stats`].
    Stats,
    /// Per-year email counts of each listed destination, keyed by path.
    DestStats(Vec<(String, crate::dest_cmd::EmailStats)>),
    /// The window asked for a folder picker; the dialog runs in the event loop.
    BrowseFolder,
    Saved,
    Close,
    Noop,
}

pub(super) fn state_json(cfg: &crate::destinations::DestinationsConfig) -> String {
    // Unreadable settings: defaults, so the list still opens.
    let settings = Settings::load(&config::settings_path()).unwrap_or_default();
    state_json_with(cfg, &settings)
}

fn state_json_with(cfg: &crate::destinations::DestinationsConfig, settings: &Settings) -> String {
    // Entries whose folder no longer exists under `notes_dir`. Empty when
    // `notes_dir` is unset or unreachable: nothing can be asserted then.
    let missing: Vec<String> = settings
        .notes_dir
        .as_deref()
        .map(std::path::Path::new)
        .filter(|dir| dir.is_dir())
        .map(|dir| crate::dest_cmd::orphaned_entries(dir, cfg))
        .unwrap_or_default();
    // Entries deeper than the configured maximum: kept, but flagged.
    let max_depth = settings.destination_max_depth();
    let too_deep: Vec<String> = cfg
        .destinations
        .iter()
        .filter(|e| crate::dest_cmd::exceeds_max_depth(&e.path, max_depth))
        .map(|e| e.path.clone())
        .collect();
    let json = serde_json::json!({
        "type": "state",
        "destinations": cfg.destinations,
        "missing": missing,
        "too_deep": too_deep,
        "max_depth": max_depth,
    });
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
        "scan_folders" => DestGuiIpcResult::Scan,
        "dest_stats" => DestGuiIpcResult::Stats,
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
    if data.get("move").and_then(Value::as_bool).unwrap_or(false) {
        crate::route::remove_rule_elsewhere(cfg, path, &crate::route::MatchRule::from(&rule));
    }
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

/// Per-year email counts of every listed destination under `notes_dir`.
///
/// Fails when the notes directory is not configured or does not exist.
pub(super) fn handle_dest_stats(cfg: &crate::destinations::DestinationsConfig) -> DestGuiIpcResult {
    let notes_dir = Settings::load(&config::settings_path())
        .ok()
        .and_then(|s| s.notes_dir)
        .map(std::path::PathBuf::from)
        .filter(|dir| dir.is_dir());
    let Some(notes_dir) = notes_dir else {
        return DestGuiIpcResult::DestStats(Vec::new());
    };
    DestGuiIpcResult::DestStats(dest_stats(&notes_dir, cfg))
}

/// [`crate::dest_cmd::email_stats`] of each listed destination; an unreadable
/// folder yields empty stats rather than failing the whole list.
fn dest_stats(
    notes_dir: &Path,
    cfg: &crate::destinations::DestinationsConfig,
) -> Vec<(String, crate::dest_cmd::EmailStats)> {
    cfg.destinations
        .iter()
        .map(|e| {
            let stats = crate::dest_cmd::email_stats(&notes_dir.join(&e.path)).unwrap_or_default();
            (e.path.clone(), stats)
        })
        .collect()
}

pub(super) fn handle_scan(cfg: &crate::destinations::DestinationsConfig) -> DestGuiIpcResult {
    let scanned = Settings::load(&config::settings_path())
        .context("failed to load settings")
        .and_then(|settings| {
            scan_destinations(
                settings.notes_dir.as_deref().map(Path::new),
                cfg,
                settings.destination_max_depth(),
            )
        });
    match scanned {
        Ok(result) => result,
        Err(e) => DestGuiIpcResult::Error(format!("Analyse impossible : {:#}", e)),
    }
}

/// Scan `notes_dir` for the destinations window (`scan_folders`): folders holding
/// emails but missing from the list, and listed destinations whose folder is gone.
///
/// Only the branches already used by the destination list are walked
/// (`dest_cmd::destination_branches`): `notes_dir` may hold unrelated trees.
///
/// Fails when the notes directory is not configured, does not exist or cannot be
/// read, or when no destination defines a branch: an empty list would be
/// indistinguishable from "nothing to suggest".
fn scan_destinations(
    notes_dir: Option<&Path>,
    cfg: &crate::destinations::DestinationsConfig,
    max_depth: usize,
) -> anyhow::Result<DestGuiIpcResult> {
    let notes_dir = notes_dir
        .ok_or_else(|| anyhow::anyhow!("le dossier de notes n'est pas configur\u{00e9}"))?;
    if !notes_dir.is_dir() {
        anyhow::bail!(
            "le dossier de notes est introuvable : {}",
            notes_dir.display()
        );
    }
    let branches = crate::dest_cmd::destination_branches(cfg);
    if branches.is_empty() {
        anyhow::bail!(
            "aucune destination : ajoutez-en une pour indiquer les branches \u{00e0} analyser"
        );
    }
    let new_folders = crate::dest_cmd::scan_folders(notes_dir, &branches, max_depth)
        .context("failed to scan notes directory")?
        .into_iter()
        .filter(|(f, _)| {
            !cfg.destinations
                .iter()
                .any(|e| e.path.eq_ignore_ascii_case(f))
        })
        .collect();
    let orphans = crate::dest_cmd::orphaned_entries(notes_dir, cfg);
    Ok(DestGuiIpcResult::FolderSuggestions {
        new_folders,
        orphans,
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
    /// Moved notes whose frontmatter holds the message on the server
    /// (`keep_on_server`): sorted, never deleted.
    pub(super) held: Vec<crate::contextual_export::LocalSourceProof>,
    /// Final paths of the moved notes, whose quoted messages are not linked yet.
    pub(super) routed: Vec<std::path::PathBuf>,
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

/// Final destination of a row, carrying the email's date folders per `layout`.
///
/// The auto proposal already ends with them; a manually reassigned path (cascade /
/// free entry / bulk) comes bare from destinations.txt — append them from the email
/// date so files always land under `<dest>/<date suffix>` (no double suffix).
/// Skipped entirely when the layout is [`DateLayout::None`], so a bare
/// manual path stays flat instead of being silently re-dated.
fn dated_dest_path(row: &RouteDecisionRow, layout: DateLayout) -> String {
    if layout == DateLayout::None {
        return row.dest_path.clone();
    }
    match read_note_header_field(Path::new(&row.file), "date")
        .and_then(|d| chrono::DateTime::parse_from_rfc3339(d.trim()).ok())
    {
        Some(dt) => layout.ensure(&row.dest_path, &dt),
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
/// `allowed`. The move itself is `route::move_decision`: quoted messages are
/// linked by the caller, once the moves are recorded (`report.routed`).
pub(super) fn apply_route_decisions(
    rows: &[RouteDecisionRow],
    notes_dir: &Path,
    allowed: &mut HashSet<String>,
    layout: DateLayout,
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
        let dest_path = dated_dest_path(row, layout);
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
        let head = crate::contextual_export::read_note_head(staging_md)
            .ok()
            .flatten()
            .unwrap_or_default();
        let proof = head
            .source
            .filter(crate::contextual_export::proof_is_complete);
        match crate::route::move_decision(staging_md, dest_path, notes_dir) {
            Ok(routed) => {
                allowed.remove(&row.file);
                report.moved.push(row.file.clone());
                report.routed.push(routed.clone());
                if head.keep_on_server.is_some() {
                    report.held.extend(proof);
                } else if let Some(proof) = proof {
                    // The routed path, not `dest_dir/<staging name>`: on a name collision
                    // the latter is another message's note.
                    report
                        .deletions
                        .push(crate::contextual_export::DeletionRequest {
                            markdown: routed,
                            proof,
                        });
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

/// Read a message header field (`subject`, `from`, `to`, `date`) of a note,
/// from its frontmatter or its ```email block.
pub(super) fn read_note_header_field(path: &std::path::Path, field: &str) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    crate::email_block::note_header_field(&content, field)
}

/// Read a YAML frontmatter block list (`field:` followed by `- item` lines).
/// Returns an empty list for `field: []` or a missing field.
pub(super) fn frontmatter_list(raw: &str, field: &str) -> Vec<String> {
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
    /// Required: the form is compiled into the binary and always sends it.
    date_layout: DateLayout,
    #[serde(default)]
    destination_max_depth: Option<usize>,
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
    #[serde(default)]
    batch_size: Option<usize>,
}

#[derive(serde::Deserialize)]
struct AccountData {
    account_name: String,
    server: String,
    port: u16,
    username: String,
    /// Ticked folders; `Some` replaces `ignored_folders` in `accounts.yaml`.
    #[serde(default)]
    included_folders: Option<Vec<String>>,
    /// `Option`, not `Vec`: an absent key must keep the saved list, not empty it.
    #[serde(default)]
    ignored_folders: Option<Vec<String>>,
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
    settings.set_date_layout(data.date_layout);
    // `0` is refused by `Settings::save`, before the file is touched.
    settings.destination_max_depth = data.destination_max_depth;
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
    config::write_env_file(env_path, lines)
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
    if !config::env_value_is_valid(password) {
        anyhow::bail!("Le mot de passe ne peut contenir ni retour à la ligne ni caractère NUL");
    }
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

/// Move `account_name` one place `"up"` or `"down"` in `accounts.yaml`.
///
/// The order drives the account list, the tray menu and the CLI export order.
/// Already at the edge: nothing is written.
fn move_account(account_name: &str, direction: &str, accounts_path: &Path) -> anyhow::Result<()> {
    let up = match direction {
        "up" => true,
        "down" => false,
        other => anyhow::bail!("direction inconnue : {other}"),
    };
    let mut accounts = config::load_raw_accounts(accounts_path)
        .with_context(|| format!("failed to load accounts from {}", accounts_path.display()))?;
    let idx = accounts
        .iter()
        .position(|a| a.name.eq_ignore_ascii_case(account_name))
        .ok_or_else(|| anyhow::anyhow!("Compte introuvable : {account_name}"))?;
    let neighbour = if up {
        idx.checked_sub(1)
    } else {
        Some(idx + 1).filter(|&n| n < accounts.len())
    };
    let Some(neighbour) = neighbour else {
        return Ok(());
    };
    accounts.swap(idx, neighbour);
    config::save_accounts(&accounts, accounts_path)
        .with_context(|| format!("failed to save accounts to {}", accounts_path.display()))?;
    Ok(())
}

/// Answer to a `move_account` request: the account list re-read from disk, or the cause.
fn move_outcome(
    account_name: Option<&str>,
    direction: Option<&str>,
    accounts_path: &Path,
) -> ConfigIpcOutcome {
    let moved = (|| -> anyhow::Result<Vec<RawAccount>> {
        let name =
            account_name.ok_or_else(|| anyhow::anyhow!("nom du compte absent de la demande"))?;
        let direction =
            direction.ok_or_else(|| anyhow::anyhow!("direction absente de la demande"))?;
        move_account(name, direction, accounts_path)?;
        config::load_raw_accounts(accounts_path)
            .with_context(|| format!("failed to load {}", accounts_path.display()))
    })();
    let reply = match moved {
        Ok(accounts) => serde_json::json!({
            "action": "move_account",
            "ok": true,
            "accounts": accounts,
        }),
        Err(e) => serde_json::json!({
            "action": "move_account",
            "ok": false,
            "message": format!("D\u{00e9}placement impossible : {:#}", e),
        }),
    };
    ConfigIpcOutcome {
        result: None,
        should_close: false,
        reply: Some(reply.to_string()),
    }
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
            if let Some(included) = &data.included_folders {
                acct.included_folders = Some(included.clone());
                acct.ignored_folders.clear();
            } else if let Some(ignored) = &data.ignored_folders {
                acct.ignored_folders = ignored.clone();
            }
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
            included_folders: data.included_folders.clone(),
            ignored_folders: match &data.included_folders {
                Some(_) => Vec::new(),
                None => data.ignored_folders.clone().unwrap_or_default(),
            },
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
            // The page shows the message itself: no tray notification on top.
            let message = format!("Erreur de sauvegarde : {:#}", e);
            ConfigIpcOutcome {
                result: None,
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

/// Password of `account_name` stored in `.env`, `_APPLICATION_PASSWORD` first.
///
/// Read from the file, not the process environment: the form must see the
/// value just saved. A missing file or an empty value counts as absent.
fn stored_password(env_path: &Path, account_name: &str) -> anyhow::Result<Option<String>> {
    if !env_path.exists() {
        return Ok(None);
    }
    let mut values = std::collections::HashMap::new();
    for item in dotenvy::from_path_iter(env_path)
        .with_context(|| format!("failed to open {}", env_path.display()))?
    {
        let (key, value) =
            item.with_context(|| format!("failed to read {}", env_path.display()))?;
        values.insert(key, value);
    }
    Ok(env_password_keys(account_name)
        .iter()
        .filter_map(|key| values.get(key))
        .find(|value| !value.is_empty())
        .cloned())
}

/// Password to probe with: the typed one, or the stored one when the form still
/// targets the saved account (same name, server and username).
///
/// The guard keeps a stored password from being sent to a server the user just typed.
fn probe_password(
    data: &AccountData,
    accounts_path: &Path,
    env_path: &Path,
) -> anyhow::Result<String> {
    if let Some(typed) = data.password.as_deref().filter(|p| !p.is_empty()) {
        return Ok(typed.to_string());
    }
    let missing = || {
        anyhow::anyhow!(
            "Saisissez le mot de passe pour tester la connexion \u{00e0} {}.",
            data.server
        )
    };
    if data.create {
        return Err(missing());
    }
    let accounts = config::load_raw_accounts(accounts_path)
        .with_context(|| format!("failed to load accounts from {}", accounts_path.display()))?;
    let unchanged = accounts.iter().any(|a| {
        a.name.eq_ignore_ascii_case(&data.account_name)
            && a.server.eq_ignore_ascii_case(data.server.trim())
            && a.username == data.username.trim()
    });
    if !unchanged {
        return Err(missing());
    }
    stored_password(env_path, &data.account_name)?.ok_or_else(missing)
}

type ProbeFn = dyn FnOnce(&str, u16, &str, &str) -> anyhow::Result<Vec<ListedFolder>>;

/// JSON answer to a `test_connection` request. Never carries the password.
fn test_connection_with(
    data: &AccountData,
    accounts_path: &Path,
    env_path: &Path,
    probe: Box<ProbeFn>,
) -> Value {
    let server = data.server.trim();
    let failure = |message: String| serde_json::json!({ "action": "test_connection", "ok": false, "message": message });
    let password = match probe_password(data, accounts_path, env_path) {
        Ok(password) => password,
        Err(e) => return failure(format!("{e:#}")),
    };
    match probe(server, data.port, data.username.trim(), &password) {
        Ok(folders) => serde_json::json!({
            "action": "test_connection",
            "ok": true,
            "folders": folders
                .iter()
                .map(|f| serde_json::json!({
                    "name": f.name.display,
                    "role": f.role,
                    "depth": f.depth,
                }))
                .collect::<Vec<_>>(),
        }),
        Err(e) => failure(describe_connection_error(server, &e)),
    }
}

/// Work of a settings-window message that must not run on the IPC handler.
///
/// The handler runs on the event loop thread: a network call there freezes
/// every tray window until it returns. `None` for every synchronous action.
/// The reply echoes `request_id`, so the form drops the answer of a test made
/// obsolete by an edit.
pub(super) fn background_config_job(body: &str) -> Option<Box<dyn FnOnce() -> String + Send>> {
    let msg: ConfigIpcMessage = serde_json::from_str(body).ok()?;
    if msg.action != "test_connection" {
        return None;
    }
    let request_id = msg.data.as_ref().and_then(|d| d.get("request_id")).cloned();
    let parsed = msg
        .data
        .ok_or_else(|| anyhow::anyhow!("donn\u{00e9}es absentes de la demande de test"))
        .and_then(|raw| {
            serde_json::from_value::<AccountData>(raw).context("Compte re\u{00e7}u illisible")
        });
    Some(Box::new(move || {
        let mut reply = match parsed {
            Ok(data) => test_connection_with(
                &data,
                &config::accounts_yaml_path(),
                &config::env_file_path(),
                Box::new(|server, port, username, password| {
                    probe_account(server, port, username, password, PROBE_TIMEOUT)
                }),
            ),
            Err(e) => serde_json::json!({
                "action": "test_connection",
                "ok": false,
                "message": format!("Test impossible : {:#}", e),
            }),
        };
        if let (Some(id), Some(fields)) = (request_id, reply.as_object_mut()) {
            fields.insert("request_id".to_string(), id);
        }
        reply.to_string()
    }))
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
                let raw_data = msg.data.ok_or_else(|| {
                    anyhow::anyhow!("données absentes de la demande d'enregistrement")
                })?;
                let data: SettingsData =
                    serde_json::from_value(raw_data).context("Réglages reçus illisibles")?;
                save_settings(data, &config::settings_path())
            })();
            save_outcome("save", result)
        }
        "save_account" => {
            let result = (|| -> anyhow::Result<()> {
                let raw_data = msg.data.ok_or_else(|| {
                    anyhow::anyhow!("données absentes de la demande d'enregistrement du compte")
                })?;
                let data: AccountData =
                    serde_json::from_value(raw_data).context("Compte reçu illisible")?;
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
            if let Err(e) =
                action_open_config().context("Ouverture du fichier de réglages impossible")
            {
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
                .context("Ouverture du dossier de configuration impossible");
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
        "move_account" => {
            let field = |key: &str| {
                msg.data
                    .as_ref()
                    .and_then(|d| d.get(key))
                    .and_then(|v| v.as_str())
            };
            move_outcome(
                field("account_name"),
                field("direction"),
                &config::accounts_yaml_path(),
            )
        }
        "delete_account" => {
            let name = msg
                .data
                .as_ref()
                .and_then(|d| d.get("account_name"))
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let deleted = name
                .ok_or_else(|| anyhow::anyhow!("nom du compte absent de la demande de suppression"))
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
                "Action IPC inconnue « {} »",
                other
            ))),
            true,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_route_decisions, background_config_job, dated_dest_path, delete_account,
        delete_staged_emails, frontmatter_list, move_account, move_outcome, relative_destination,
        reload_outcome, remove_env_passwords, save_account, save_outcome, save_settings,
        scan_destinations, set_env_password, state_json_with, stored_password,
        test_connection_with, validate_new_account_name, AccountData, DestGuiIpcResult,
        RouteApplyReport, RouteDecisionRow, SettingsData,
    };
    use crate::config::{load_raw_accounts, RawAccount, Settings};
    use crate::date_layout::DateLayout;
    use crate::destinations::{DestinationEntry, DestinationsConfig};
    use crate::email_export::{FolderName, FolderRole, ListedFolder};
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
    fn test_save_outcome_with_error_notifies_once() {
        let outcome = save_outcome("save", Err(anyhow::anyhow!("disk full")));
        assert!(outcome.result.is_none(), "the page already shows the error");
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
        let result = scan_destinations(None, &DestinationsConfig::default(), 2);
        assert!(result.is_err());
    }

    #[test]
    fn test_scan_destinations_with_missing_notes_dir() {
        let temp = TempDir::new().unwrap();
        let missing = temp.path().join("absent");
        let result = scan_destinations(Some(&missing), &DestinationsConfig::default(), 2);
        assert!(result.is_err());
    }

    /// Destination list with a single entry under `path`.
    fn cfg_with(path: &str) -> DestinationsConfig {
        DestinationsConfig {
            destinations: vec![DestinationEntry {
                path: path.into(),
                ..Default::default()
            }],
        }
    }

    /// Write an email note (frontmatter with `from`) under `root/dir`.
    fn write_email(root: &Path, dir: &str) {
        let dir = root.join(dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("n.md"),
            "---
from: a@x.com
---
Body
",
        )
        .unwrap();
    }

    #[test]
    fn test_scan_destinations_with_empty_notes_dir() {
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("Perso/Banque")).unwrap();
        let result = scan_destinations(Some(temp.path()), &cfg_with("Perso/Banque"), 2);
        assert!(matches!(
            result,
            Ok(DestGuiIpcResult::FolderSuggestions { new_folders, orphans })
                if new_folders.is_empty() && orphans.is_empty()
        ));
    }

    #[test]
    fn test_scan_destinations_without_destinations() {
        let temp = TempDir::new().unwrap();
        let result = scan_destinations(Some(temp.path()), &DestinationsConfig::default(), 2);
        assert!(result.is_err());
    }

    /// Only folders holding emails, inside the destination branches, are proposed.
    #[test]
    fn test_scan_destinations_with_unrelated_top_level_dirs() {
        let temp = TempDir::new().unwrap();
        write_email(temp.path(), "Perso/Banque/BNP/2025/06");
        write_email(temp.path(), "Perso/Sante/Dentiste/2025/06");
        write_email(temp.path(), "Code/Projet/App");
        let result = scan_destinations(Some(temp.path()), &cfg_with("Perso/Banque/BNP"), 2);
        let Ok(DestGuiIpcResult::FolderSuggestions { new_folders, .. }) = result else {
            panic!("expected folder suggestions");
        };
        assert_eq!(new_folders, vec![("Perso/Sante/Dentiste".to_string(), 1)]);
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

        apply_route_decisions(
            &decisions,
            &fx.notes,
            &mut allowed(&fx.files),
            DateLayout::None,
        )
        .unwrap();

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
    fn test_apply_route_decisions_year_month_single_suffix() {
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

        apply_route_decisions(
            &decisions,
            &fx.notes,
            &mut allowed(&fx.files),
            DateLayout::YearMonth,
        )
        .unwrap();

        assert_eq!(
            (md_names(&dated), dated.join("2026").exists()),
            (vec!["a.md".to_string(), "b.md".to_string()], false)
        );
    }

    #[test]
    fn test_dated_dest_path_reads_date_from_email_block() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("note.md");
        fs::write(
            &path,
            "---
tags:
  - email
---

```email
subject = \"x\"
from = \"a@b.c\"
to = []
date = 2026-07-02T10:18:19+00:00
body = '''
hi
'''
```
",
        )
        .unwrap();
        let row = RouteDecisionRow {
            file: path.to_string_lossy().into_owned(),
            dest_path: "Perso/Banque".to_string(),
        };

        let dated = dated_dest_path(&row, DateLayout::YearMonth);

        assert_eq!(dated, "Perso/Banque/2026/07");
    }

    #[test]
    fn test_apply_route_decisions_invalid_path_moves_nothing() {
        let fx = route_fixture();
        let decisions = rows(&[
            (&fx.files[0], "Perso"),
            (&fx.files[1], "../outside"),
            (&fx.files[2], "Travail"),
        ]);

        let err = apply_route_decisions(
            &decisions,
            &fx.notes,
            &mut allowed(&fx.files),
            DateLayout::None,
        )
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

        let err = apply_route_decisions(&decisions, &fx.notes, &mut allowed, DateLayout::None)
            .unwrap_err();

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

        let report =
            apply_route_decisions(&decisions, &fx.notes, &mut allowed, DateLayout::None).unwrap();

        assert_eq!(
            report,
            RouteApplyReport {
                moved: vec![fx.files[0].clone(), fx.files[2].clone()],
                missing: vec![fx.files[1].clone()],
                failed: vec![],
                deletions: vec![],
                held: vec![],
                routed: vec![
                    fx.notes.join("Perso").join("a.md"),
                    fx.notes.join("Perso").join("c.md"),
                ],
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

        let report =
            apply_route_decisions(&decisions, &fx.notes, &mut allowed, DateLayout::None).unwrap();

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

        let report =
            apply_route_decisions(&decisions, &fx.notes, &mut allowed, DateLayout::None).unwrap();

        assert!(report.deletions.is_empty());
    }

    #[test]
    fn test_route_apply_report_error_message_names_missing_file_once() {
        let report = RouteApplyReport {
            moved: vec!["a.md".to_string()],
            missing: vec!["b.md".to_string()],
            ..Default::default()
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
            "date_layout": "year_month",
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

    const ACCOUNT_WITH_FOLDERS: &str = "accounts:
  - name: Perso
    server: imap.example.org
    port: 993
    username: perso@example.org
    included_folders:
      - Projets
    ignored_folders:
      - Spam
";

    #[test]
    fn test_save_account_with_included_folders_clears_ignored() {
        let temp = TempDir::new().unwrap();
        let (accounts, settings, _) = account_files(&temp);
        fs::write(&accounts, ACCOUNT_WITH_FOLDERS).unwrap();
        let mut data = account_data("Perso");
        data.included_folders = Some(vec!["INBOX".to_string()]);

        save_account(data, &accounts, &settings).unwrap();

        let saved = &load_raw_accounts(&accounts).unwrap()[0];
        assert_eq!(saved.included_folders, Some(vec!["INBOX".to_string()]));
        assert!(saved.ignored_folders.is_empty());
    }

    #[test]
    fn test_save_account_without_selection_keeps_folder_keys() {
        let temp = TempDir::new().unwrap();
        let (accounts, settings, _) = account_files(&temp);
        fs::write(&accounts, ACCOUNT_WITH_FOLDERS).unwrap();

        save_account(account_data("Perso"), &accounts, &settings).unwrap();

        let saved = &load_raw_accounts(&accounts).unwrap()[0];
        assert_eq!(saved.included_folders, Some(vec!["Projets".to_string()]));
        assert_eq!(saved.ignored_folders, ["Spam"]);
    }

    #[test]
    fn test_save_account_with_ignored_folders_keeps_included() {
        let temp = TempDir::new().unwrap();
        let (accounts, settings, _) = account_files(&temp);
        fs::write(&accounts, ACCOUNT_WITH_FOLDERS).unwrap();
        let mut data = account_data("Perso");
        data.ignored_folders = Some(vec!["Trash".to_string()]);

        save_account(data, &accounts, &settings).unwrap();

        let saved = &load_raw_accounts(&accounts).unwrap()[0];
        assert_eq!(saved.ignored_folders, ["Trash"]);
        assert_eq!(saved.included_folders, Some(vec!["Projets".to_string()]));
    }

    #[test]
    fn test_save_account_with_new_account_and_included_folders() {
        let temp = TempDir::new().unwrap();
        let (accounts, settings, _) = account_files(&temp);
        fs::write(&accounts, VALID_ACCOUNTS).unwrap();
        let mut data = account_data("Pro");
        data.create = true;
        data.included_folders = Some(vec!["INBOX".to_string(), "Sent".to_string()]);

        save_account(data, &accounts, &settings).unwrap();

        let saved = load_raw_accounts(&accounts).unwrap();
        let pro = saved.iter().find(|a| a.name == "Pro").unwrap();
        assert_eq!(
            pro.included_folders,
            Some(vec!["INBOX".to_string(), "Sent".to_string()])
        );
        assert!(pro.ignored_folders.is_empty());
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

    const THREE_ACCOUNTS: &str = "accounts:\n  - name: A\n    server: a.example.org\n    port: 993\n    username: a\n  - name: B\n    server: b.example.org\n    port: 993\n    username: b\n  - name: C\n    server: c.example.org\n    port: 993\n    username: c\n";

    fn account_names(accounts: &std::path::Path) -> Vec<String> {
        load_raw_accounts(accounts)
            .unwrap()
            .into_iter()
            .map(|a| a.name)
            .collect()
    }

    #[test]
    fn test_move_account_up_swaps_with_previous() {
        let temp = TempDir::new().unwrap();
        let (accounts, _, _) = account_files(&temp);
        fs::write(&accounts, THREE_ACCOUNTS).unwrap();

        move_account("B", "up", &accounts).unwrap();

        assert_eq!(account_names(&accounts), ["B", "A", "C"]);
    }

    #[test]
    fn test_move_account_down_case_insensitive_swaps_with_next() {
        let temp = TempDir::new().unwrap();
        let (accounts, _, _) = account_files(&temp);
        fs::write(&accounts, THREE_ACCOUNTS).unwrap();

        move_account("b", "down", &accounts).unwrap();

        assert_eq!(account_names(&accounts), ["A", "C", "B"]);
    }

    #[test]
    fn test_move_account_at_edge_writes_nothing() {
        let temp = TempDir::new().unwrap();
        let (accounts, _, _) = account_files(&temp);
        fs::write(&accounts, THREE_ACCOUNTS).unwrap();

        move_account("A", "up", &accounts).unwrap();
        move_account("C", "down", &accounts).unwrap();

        assert_eq!(fs::read_to_string(&accounts).unwrap(), THREE_ACCOUNTS);
    }

    #[test]
    fn test_move_account_unknown_name_invalid() {
        let temp = TempDir::new().unwrap();
        let (accounts, _, _) = account_files(&temp);
        fs::write(&accounts, THREE_ACCOUNTS).unwrap();

        assert!(move_account("Z", "up", &accounts).is_err());
        assert_eq!(fs::read_to_string(&accounts).unwrap(), THREE_ACCOUNTS);
    }

    #[test]
    fn test_move_account_bad_direction_invalid() {
        let temp = TempDir::new().unwrap();
        let (accounts, _, _) = account_files(&temp);
        fs::write(&accounts, THREE_ACCOUNTS).unwrap();

        assert!(move_account("B", "sideways", &accounts).is_err());
        assert_eq!(fs::read_to_string(&accounts).unwrap(), THREE_ACCOUNTS);
    }

    #[test]
    fn test_move_outcome_valid_returns_new_order() {
        let temp = TempDir::new().unwrap();
        let (accounts, _, _) = account_files(&temp);
        fs::write(&accounts, THREE_ACCOUNTS).unwrap();

        let outcome = move_outcome(Some("C"), Some("up"), &accounts);

        let reply: serde_json::Value = serde_json::from_str(&outcome.reply.unwrap()).unwrap();
        assert_eq!(reply["action"], "move_account");
        assert_eq!(reply["ok"], true);
        let names: Vec<&str> = reply["accounts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["A", "C", "B"]);
    }

    #[test]
    fn test_move_outcome_missing_direction_reports_error() {
        let temp = TempDir::new().unwrap();
        let (accounts, _, _) = account_files(&temp);
        fs::write(&accounts, THREE_ACCOUNTS).unwrap();

        let outcome = move_outcome(Some("B"), None, &accounts);

        let reply: serde_json::Value = serde_json::from_str(&outcome.reply.unwrap()).unwrap();
        assert_eq!(reply["ok"], false);
        assert!(reply["message"]
            .as_str()
            .unwrap()
            .starts_with("D\u{00e9}placement impossible"));
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
    fn test_set_env_password_with_newline_invalid() {
        let temp = TempDir::new().unwrap();
        let env = temp.path().join(".env");
        fs::write(&env, "OTHER_PASSWORD='c'\n").unwrap();

        assert!(set_env_password(&env, "Perso", "pw'\nINJECTED=1").is_err());
        assert_eq!(fs::read_to_string(&env).unwrap(), "OTHER_PASSWORD='c'\n");
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
            included_folders: None,
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
    fn test_save_settings_writes_layout_and_depth() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("settings.yaml");
        fs::write(&path, "organize_by_date: false\n").unwrap();
        let mut data = settings_data("C:/new");
        data.date_layout = DateLayout::YearMonthDay;
        data.destination_max_depth = Some(3);

        save_settings(data, &path).unwrap();

        let yaml = fs::read_to_string(&path).unwrap();
        assert!(yaml.contains("date_layout: year_month_day"), "{yaml}");
        assert!(yaml.contains("destination_max_depth: 3"), "{yaml}");
        assert!(yaml.contains("organize_by_date: true"), "{yaml}");
    }

    #[test]
    fn test_save_settings_with_none_layout() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("settings.yaml");
        fs::write(&path, "date_layout: year_month_day\n").unwrap();
        let mut data = settings_data("C:/new");
        data.date_layout = DateLayout::None;

        save_settings(data, &path).unwrap();

        let yaml = fs::read_to_string(&path).unwrap();
        assert!(yaml.contains("date_layout: none"), "{yaml}");
        assert!(yaml.contains("organize_by_date: false"), "{yaml}");
    }

    #[test]
    fn test_save_settings_with_zero_depth_keeps_file() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("settings.yaml");
        fs::write(&path, "organize_by_date: false\n").unwrap();
        let mut data = settings_data("C:/new");
        data.destination_max_depth = Some(0);

        let err = format!("{:#}", save_settings(data, &path).unwrap_err());

        assert!(err.contains("destination_max_depth"), "{err}");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "organize_by_date: false\n"
        );
    }

    #[test]
    fn test_state_json_lists_too_deep_entries() {
        let cfg = DestinationsConfig {
            destinations: ["Pro/A/B/C", "Pro/A/B", "Pro/A/B/2026/10"]
                .into_iter()
                .map(|path| DestinationEntry {
                    path: path.into(),
                    ..Default::default()
                })
                .collect(),
        };
        let settings = Settings {
            destination_max_depth: Some(2),
            ..Settings::default()
        };

        let state: serde_json::Value =
            serde_json::from_str(&state_json_with(&cfg, &settings)).unwrap();

        assert_eq!(state["too_deep"], serde_json::json!(["Pro/A/B/C"]));
        assert_eq!(state["max_depth"], 2);
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
    fn test_frontmatter_list_with_items() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("note.md");
        fs::write(
            &path,
            "---\nsubject: Hi\nattachments:\n- a.pdf\n- \"b c.png\"\ntags: []\n---\nbody\n",
        )
        .unwrap();

        assert_eq!(
            frontmatter_list(&fs::read_to_string(&path).unwrap(), "attachments"),
            vec!["a.pdf".to_string(), "b c.png".to_string()]
        );
    }

    #[test]
    fn test_frontmatter_list_empty() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("note.md");
        fs::write(&path, "---\nattachments: []\n---\nbody\n").unwrap();

        assert!(frontmatter_list(&fs::read_to_string(&path).unwrap(), "attachments").is_empty());
    }

    fn perso_probe_data(server: &str, password: Option<&str>) -> AccountData {
        serde_json::from_value(serde_json::json!({
            "account_name": "Perso",
            "server": server,
            "port": 993,
            "username": "perso@example.org",
            "password": password,
        }))
        .unwrap()
    }

    fn perso_env(temp: &TempDir) -> (PathBuf, PathBuf) {
        let (accounts_path, _, env_path) = account_files(temp);
        fs::write(&accounts_path, VALID_ACCOUNTS).unwrap();
        fs::write(
            &env_path,
            "PERSO_PASSWORD='plain'\nPERSO_APPLICATION_PASSWORD='app'\n",
        )
        .unwrap();
        (accounts_path, env_path)
    }

    #[test]
    fn test_stored_password_with_application_password() {
        let temp = TempDir::new().unwrap();
        let (_, env_path) = perso_env(&temp);

        assert_eq!(
            stored_password(&env_path, "Perso").unwrap().as_deref(),
            Some("app")
        );
    }

    #[test]
    fn test_stored_password_without_env_file() {
        let temp = TempDir::new().unwrap();

        assert_eq!(
            stored_password(&temp.path().join(".env"), "Perso").unwrap(),
            None
        );
    }

    #[test]
    fn test_test_connection_with_changed_server_skips_probe() {
        let temp = TempDir::new().unwrap();
        let (accounts_path, env_path) = perso_env(&temp);
        let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = called.clone();

        let reply = test_connection_with(
            &perso_probe_data("imap.other.example", None),
            &accounts_path,
            &env_path,
            Box::new(move |_, _, _, _| {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(Vec::new())
            }),
        );

        assert!(!called.load(std::sync::atomic::Ordering::SeqCst), "{reply}");
        assert!(reply["message"]
            .as_str()
            .is_some_and(|m| m.contains("Saisissez le mot de passe")));
    }

    #[test]
    fn test_test_connection_with_unchanged_account_uses_stored_password() {
        let temp = TempDir::new().unwrap();
        let (accounts_path, env_path) = perso_env(&temp);
        let sent = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let seen = sent.clone();

        let reply = test_connection_with(
            &perso_probe_data("imap.example.org", Some("")),
            &accounts_path,
            &env_path,
            Box::new(move |_, _, _, password| {
                *seen.lock().unwrap() = password.to_string();
                Ok(vec![ListedFolder {
                    name: FolderName {
                        raw: "INBOX".into(),
                        display: "INBOX".into(),
                    },
                    role: FolderRole::Inbox,
                    depth: 0,
                }])
            }),
        );

        assert_eq!(*sent.lock().unwrap(), "app");
        assert_eq!(
            reply,
            serde_json::json!({
                "action": "test_connection",
                "ok": true,
                "folders": [{"name": "INBOX", "role": "inbox", "depth": 0}],
            })
        );
    }

    #[test]
    fn test_test_connection_with_failure_hides_password() {
        let temp = TempDir::new().unwrap();
        let (accounts_path, env_path) = perso_env(&temp);

        let reply = test_connection_with(
            &perso_probe_data("imap.example.org", Some("s3cret-typed")),
            &accounts_path,
            &env_path,
            Box::new(|_, _, _, password| anyhow::bail!("LOGIN refused ({} chars)", password.len())),
        );

        assert_eq!(reply["ok"], false);
        assert!(reply["message"].as_str().is_some_and(|m| !m.is_empty()));
        assert!(!reply.to_string().contains("s3cret-typed"));
    }

    #[test]
    fn test_background_config_job_with_sync_action() {
        assert!(background_config_job(r#"{"action":"reload"}"#).is_none());
    }

    #[test]
    fn test_background_config_job_with_request_id() {
        let job = background_config_job(
            r#"{"action":"test_connection","data":{"request_id":7,"account_name":"Perso"}}"#,
        )
        .unwrap();
        let reply: serde_json::Value = serde_json::from_str(&job()).unwrap();
        assert_eq!(
            (reply["ok"].clone(), reply["request_id"].clone()),
            (serde_json::json!(false), serde_json::json!(7))
        );
    }
}
