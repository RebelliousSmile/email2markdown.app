use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::{Context, Result};
use tao::dpi::LogicalSize;
use tao::event_loop::{EventLoopProxy, EventLoopWindowTarget};
use tao::window::{Window, WindowBuilder, WindowId};
use wry::{WebView, WebViewBuilder};

use crate::route::RouteDecision;
use crate::tray_actions::{self, ActionResult};
use crate::updater;

use super::{AppCommand, ContextualIpcMessage};

fn build_gui_window(
    target: &EventLoopWindowTarget<AppCommand>,
    title: &str,
    size: (f64, f64),
) -> Result<(Window, WindowId)> {
    let window = WindowBuilder::new()
        .with_title(title)
        .with_inner_size(LogicalSize::new(size.0, size.1))
        .build(target)
        .with_context(|| format!("failed to create window {title:?}"))?;
    window.set_focus();
    let window_id = window.id();
    Ok((window, window_id))
}

/// Decide whether a webview may navigate to `url`.
///
/// Windows are loaded once from an HTML string and never navigate afterwards.
/// `about:blank` is always allowed: it is the document a webview starts from,
/// and on some platforms the URL reported for an HTML-string load. The first
/// other navigation is the initial load itself (WebView2 reports it under its
/// own URL); it claims `initial_load_seen` and every later one is refused.
fn allow_navigation(url: &str, initial_load_seen: &AtomicBool) -> bool {
    if url == "about:blank" {
        return true;
    }
    !initial_load_seen.swap(true, Ordering::SeqCst)
}

fn attach_webview(
    window: &Window,
    html: impl Into<std::borrow::Cow<'static, str>>,
    init_script: Option<&str>,
    ipc_handler: impl Fn(wry::http::Request<String>) + 'static,
) -> Result<WebView> {
    let html: std::borrow::Cow<'static, str> = html.into();
    let initial_load_seen = AtomicBool::new(false);
    let mut builder = WebViewBuilder::new(window)
        .with_html(html.into_owned())
        .with_navigation_handler(move |url| {
            let allowed = allow_navigation(&url, &initial_load_seen);
            if !allowed {
                // Only the scheme is logged: a refused URL may carry page data.
                let scheme = url.split(':').next().unwrap_or_default();
                eprintln!("navigation refusée (schéma {scheme})");
            }
            allowed
        })
        .with_ipc_handler(ipc_handler);
    if let Some(script) = init_script {
        builder = builder.with_initialization_script(script);
    }
    builder.build().context("failed to create webview")
}

pub(super) fn build_contextual_window(
    target: &EventLoopWindowTarget<AppCommand>,
    proxy: &EventLoopProxy<AppCommand>,
    launch: &tray_actions::ContextualLaunch,
) -> Result<(Window, WebView, WindowId)> {
    let (window, window_id) = build_gui_window(target, "Export contextuel", (900.0, 640.0))?;
    let html = include_str!("../../assets/contextual_export.html");
    let launch_json = serde_json::to_string(launch).unwrap_or_else(|_| "null".to_string());
    let init_script = format!(
        "window.__CONTEXTUAL_LAUNCH__ = {};",
        escape_json_for_script(&launch_json)
    );
    let ipc_proxy = proxy.clone();
    let webview = attach_webview(&window, html, Some(&init_script), move |req| {
        let body = req.body().as_str();
        let Ok(msg) = serde_json::from_str::<ContextualIpcMessage>(body) else {
            return;
        };
        match msg.action.as_str() {
            "search" => {
                if let Some(account) = msg.account {
                    let _ = ipc_proxy.send_event(AppCommand::ContextualSearchRequested {
                        window_id,
                        account,
                    });
                }
            }
            "convert" => {
                let _ = ipc_proxy.send_event(AppCommand::ContextualConvertRequested {
                    window_id,
                    keys: msg.keys,
                });
            }
            "retry_deletion" => {
                let _ =
                    ipc_proxy.send_event(AppCommand::ContextualRetryDeletionRequested { window_id });
            }
            "open_config" => {
                let _ = ipc_proxy.send_event(AppCommand::ContextualOpenConfig);
            }
            "create_rule" => {
                if let (Some(attr_kind), Some(attr_value)) = (msg.attr_kind, msg.attr_value) {
                    let _ = ipc_proxy.send_event(AppCommand::ContextualCreateRuleRequested {
                        window_id,
                        attr_kind,
                        attr_value,
                    });
                }
            }
            "cancel" => {
                let _ = ipc_proxy.send_event(AppCommand::CloseWindow { window_id });
            }
            _ => {}
        }
    })?;
    Ok((window, webview, window_id))
}

/// Cancel button of the progress window, only present when the action can be cancelled.
const CANCEL_BUTTON_HTML: &str = r#"<button id="cancel" onclick="this.disabled = true; window.ipc.postMessage('cancel')">Annuler</button>"#;

/// `action_name` and `warning` are inserted into the page body without HTML
/// escaping. They must stay fixed labels chosen by the caller: never pass an
/// account name, a folder name or any other external data.
pub(super) fn build_progress_window(
    target: &EventLoopWindowTarget<AppCommand>,
    proxy: &EventLoopProxy<AppCommand>,
    action_name: &str,
    warning: Option<&str>,
    cancel_token: Option<Arc<AtomicBool>>,
) -> Result<(Window, WebView, WindowId)> {
    let (window, window_id) = build_gui_window(target, action_name, (480.0, 220.0))?;
    let html = include_str!("../../assets/progress_window.html")
        .replace("__ACTION_NAME__", action_name)
        .replace("__WARNING__", warning.unwrap_or(""))
        .replace(
            "__HAS_CANCEL__",
            if cancel_token.is_some() {
                CANCEL_BUTTON_HTML
            } else {
                ""
            },
        );
    let ipc_proxy = proxy.clone();
    let webview = attach_webview(&window, html, None, move |req| {
        let body = req.body().as_str();
        match body {
            "action" => {
                let _ = ipc_proxy.send_event(AppCommand::ActionRequested { window_id });
            }
            "close" => {
                let _ = ipc_proxy.send_event(AppCommand::CloseWindow { window_id });
            }
            "cancel" => {
                if let Some(token) = &cancel_token {
                    token.store(true, Ordering::SeqCst);
                }
            }
            _ => {}
        }
    })?;
    Ok((window, webview, window_id))
}

pub(super) fn build_config_window(
    target: &EventLoopWindowTarget<AppCommand>,
    proxy: &EventLoopProxy<AppCommand>,
    sender: Sender<ActionResult>,
) -> Result<(Window, WebView, WindowId)> {
    let settings_path = crate::config::settings_path();
    let settings = crate::config::Settings::load(&settings_path)
        .with_context(|| format!("failed to load settings from {}", settings_path.display()))?;
    let accounts_path = crate::config::accounts_yaml_path();
    let raw_accounts = crate::config::load_raw_accounts(&accounts_path)
        .with_context(|| format!("failed to load accounts from {}", accounts_path.display()))?;

    let html_template = include_str!("../../assets/config_window.html");
    let settings_json = serde_json::to_string(&settings).context("failed to serialize settings")?;
    let accounts_json =
        serde_json::to_string(&raw_accounts).context("failed to serialize accounts")?;
    let html = html_template
        .replace("__SETTINGS_JSON__", &escape_json_for_script(&settings_json))
        .replace("__ACCOUNTS_JSON__", &escape_json_for_script(&accounts_json));

    let (window, window_id) = build_gui_window(
        target,
        "Email to Markdown \u{2014} Param\u{00e8}tres",
        (700.0, 500.0),
    )?;

    let proxy_ipc = proxy.clone();
    let webview = attach_webview(
        &window,
        html,
        None,
        move |req: wry::http::Request<String>| {
            let body = req.body().clone();
            let outcome = super::ipc::handle_config_ipc(&body);
            if let Some(r) = outcome.result {
                let _ = sender.send(r);
            }
            if let Some(reply) = outcome.reply {
                let js = format!("config_result({})", escape_json_for_script(&reply));
                let _ = proxy_ipc.send_event(AppCommand::EvalScript { window_id, js });
            }
            if outcome.should_close {
                let _ = proxy_ipc.send_event(AppCommand::CloseWindow { window_id });
            }
        },
    )?;

    Ok((window, webview, window_id))
}

// ── Update window ─────────────────────────────────────────────────────────────

/// Build an update window inline on the main event loop thread.
pub(super) fn build_update_window(
    target: &EventLoopWindowTarget<AppCommand>,
    proxy: &EventLoopProxy<AppCommand>,
) -> Result<(Window, WebView, WindowId)> {
    let html = include_str!("../../assets/update_window.html");

    let (window, window_id) = build_gui_window(
        target,
        "Email to Markdown \u{2014} Mise \u{00e0} jour",
        (700.0, 500.0),
    )?;

    // The release found by the check stays on the host side: the page only
    // confirms, it never chooses what gets downloaded.
    let pending_release: Arc<Mutex<Option<updater::Release>>> = Arc::new(Mutex::new(None));

    let proxy_ipc = proxy.clone();
    let pending_ipc = Arc::clone(&pending_release);
    let webview = attach_webview(
        &window,
        html,
        None,
        move |req: wry::http::Request<String>| {
            let body = req.body().clone();
            let parsed: serde_json::Value = match serde_json::from_str(&body) {
                Ok(v) => v,
                Err(_) => return,
            };
            if parsed.get("action").and_then(|v| v.as_str()) == Some("update-confirm") {
                // Any other field of the message is ignored.
                let release = pending_ipc.lock().ok().and_then(|guard| guard.clone());
                if let Some(release) = release {
                    let proxy_dl = proxy_ipc.clone();
                    thread::spawn(move || {
                        let result = updater::download_and_apply(&release, |msg| {
                            let json = serde_json::json!({ "type": "msg", "text": msg }).to_string();
                            let _ = proxy_dl.send_event(AppCommand::UpdateMsg(json));
                        });
                        match result {
                            Ok(()) => {
                                let json = serde_json::json!({
                                    "type": "msg",
                                    "text": "Mise à jour terminée — veuillez relancer l'application."
                                })
                                .to_string();
                                let _ = proxy_dl.send_event(AppCommand::UpdateMsg(json));
                                std::thread::sleep(std::time::Duration::from_millis(300));
                                std::process::exit(0);
                            }
                            Err(e) => {
                                let json = serde_json::json!({
                                    "type": "error",
                                    "text": format!("Erreur : {:#}", e)
                                })
                                .to_string();
                                let _ = proxy_dl.send_event(AppCommand::UpdateMsg(json));
                            }
                        }
                    });
                }
            }
        },
    )?;

    let proxy_check = proxy.clone();
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    thread::spawn(move || match updater::check_update(&current_version) {
        Ok(None) => {
            let json = serde_json::json!({
                "type": "check_result",
                "current": current_version,
                "latest": serde_json::Value::Null
            })
            .to_string();
            let _ = proxy_check.send_event(AppCommand::UpdateMsg(json));
        }
        Ok(Some(release)) => {
            let json = serde_json::json!({
                "type": "check_result",
                "current": current_version,
                "latest": release.tag_name,
                "body": release.body
            })
            .to_string();
            // Stored before the page learns about it, so a confirmation always
            // finds the release.
            if let Ok(mut pending) = pending_release.lock() {
                *pending = Some(release);
            }
            let _ = proxy_check.send_event(AppCommand::UpdateMsg(json));
        }
        Err(e) => {
            let json = serde_json::json!({
                "type": "error",
                "text": format!("Erreur : {:#}", e)
            })
            .to_string();
            let _ = proxy_check.send_event(AppCommand::UpdateMsg(json));
        }
    });

    Ok((window, webview, window_id))
}

pub(super) fn build_dest_gui_window(
    target: &EventLoopWindowTarget<AppCommand>,
    proxy: &EventLoopProxy<AppCommand>,
    dest_file: &Path,
    initial_path: Option<&str>,
    contextual_on_save: bool,
) -> Result<(
    Window,
    WebView,
    WindowId,
    Arc<Mutex<crate::destinations::DestinationsConfig>>,
)> {
    let cfg = crate::destinations::load_yaml(dest_file)
        .with_context(|| format!("failed to load destinations from {}", dest_file.display()))?;
    let cfg_arc = Arc::new(Mutex::new(cfg));

    let initial_path_json = serde_json::to_string(&initial_path).unwrap_or_else(|_| "null".into());
    let html = include_str!("../../assets/destinations_window.html")
        .replace(
            "__INITIAL_PATH_JSON__",
            &escape_json_for_script(&initial_path_json),
        )
        .replace(
            "__CONTEXTUAL_SETUP_JSON__",
            if contextual_on_save { "true" } else { "false" },
        );

    let (window, window_id) = build_gui_window(target, "Email to Markdown \u{2014} Destinations", (820.0, 560.0))?;

    let proxy_ipc = proxy.clone();
    let cfg_ipc = Arc::clone(&cfg_arc);
    let dest_file_ipc = dest_file.to_path_buf();

    let webview = attach_webview(&window, html, None, move |req: wry::http::Request<String>| {
        let body = req.body();
        let mut cfg_guard = match cfg_ipc.lock() {
            Ok(g) => g,
            Err(_) => {
                eprintln!("dest-gui: mutex poisoned");
                return;
            }
        };
        let result = super::ipc::handle_dest_gui_ipc(body, &mut cfg_guard, &dest_file_ipc);
        match result {
            super::ipc::DestGuiIpcResult::StateChanged => {
                let json = super::ipc::state_json(&cfg_guard);
                drop(cfg_guard);
                let _ = proxy_ipc.send_event(AppCommand::PushDestState { window_id, json });
            }
            super::ipc::DestGuiIpcResult::Error(message) => {
                drop(cfg_guard);
                let json = serde_json::json!({"type": "error", "message": message}).to_string();
                let _ = proxy_ipc.send_event(AppCommand::PushDestState { window_id, json });
            }
            super::ipc::DestGuiIpcResult::Suggestions(items) => {
                drop(cfg_guard);
                let items_json = serde_json::json!({
                    "type": "suggestions",
                    "items": items.iter()
                        .map(|(d, c)| serde_json::json!({"domain": d, "count": c}))
                        .collect::<Vec<_>>()
                })
                .to_string();
                let _ = proxy_ipc.send_event(AppCommand::PushDestState {
                    window_id,
                    json: items_json,
                });
            }
            super::ipc::DestGuiIpcResult::FolderSuggestions {
                new_folders,
                orphans,
            } => {
                drop(cfg_guard);
                let json = serde_json::json!({
                    "type": "folder_suggestions",
                    "paths": new_folders,
                    "orphans": orphans,
                })
                .to_string();
                let _ = proxy_ipc.send_event(AppCommand::PushDestState { window_id, json });
            }
            super::ipc::DestGuiIpcResult::Saved => {
                drop(cfg_guard);
                let command = if contextual_on_save {
                    AppCommand::ContextualDestinationSaved { window_id }
                } else {
                    AppCommand::CloseWindow { window_id }
                };
                let _ = proxy_ipc.send_event(command);
            }
            super::ipc::DestGuiIpcResult::Close => {
                drop(cfg_guard);
                let _ = proxy_ipc.send_event(AppCommand::CloseWindow { window_id });
            }
            super::ipc::DestGuiIpcResult::Noop => {}
        }
    })?;

    Ok((window, webview, window_id, cfg_arc))
}

/// IPC discriminator — reads the `action` field (default `""`) without failing on unknown shapes.
#[derive(serde::Deserialize)]
struct IpcKind {
    #[serde(default)]
    action: String,
}

#[derive(serde::Deserialize)]
struct RuleCreatePayload {
    #[allow(dead_code)]
    action: String,
    path: String,
    attr_kind: String,
    attr_value: String,
}

fn extract_addr_and_domain(from_raw: &str) -> (String, String) {
    let trimmed = from_raw.trim();
    if trimmed.is_empty() {
        return (String::new(), String::new());
    }
    let addr: String = if let (Some(lt), Some(gt)) = (trimmed.find('<'), trimmed.rfind('>')) {
        if lt < gt {
            trimmed[lt + 1..gt].trim().to_string()
        } else {
            trimmed
                .split_whitespace()
                .find(|t| t.contains('@'))
                .unwrap_or("")
                .to_string()
        }
    } else {
        trimmed
            .split_whitespace()
            .find(|t| t.contains('@'))
            .unwrap_or("")
            .to_string()
    };
    if addr.is_empty() || !addr.contains('@') {
        return (String::new(), String::new());
    }
    if let Some(at_pos) = addr.rfind('@') {
        let domain = addr[at_pos + 1..].to_lowercase();
        (addr, domain)
    } else {
        (addr, String::new())
    }
}

#[derive(serde::Deserialize)]
struct DeletePayload {
    #[allow(dead_code)]
    action: String,
    files: Vec<String>,
}

/// Escape a JSON string for safe embedding inside a `<script>` initialization
/// block (guards against `</script>` breakout in the WebView).
pub(super) fn escape_json_for_script(json: &str) -> String {
    json.replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
}

/// Build a route review window on the main event loop thread.
///
/// Loads `route_review.html`, injects the decisions JSON and the list of
/// known paths from `destinations.yaml`, and wires an IPC handler that
/// calls `apply_route_decisions` when the user clicks Apply.
///
/// The handler only moves or deletes the staging files sent to the page here.
pub(super) fn build_route_window(
    target: &EventLoopWindowTarget<AppCommand>,
    proxy: &EventLoopProxy<AppCommand>,
    decisions: Vec<(PathBuf, RouteDecision)>,
) -> Result<(Window, WebView, WindowId)> {
    let settings_path = crate::config::settings_path();
    let settings = crate::config::Settings::load(&settings_path).unwrap_or_default();
    let notes_dir: PathBuf = settings
        .notes_dir
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("notes"));

    // Collect known paths from destinations.yaml for the datalist autocomplete.
    let known_paths: Vec<String> = crate::route::load_destinations()
        .into_iter()
        .map(|d| d.path)
        .collect();

    // Build owned rows — extract frontmatter fields for display in the table.
    let owned_rows: Vec<(String, String, String, String, String, bool, String, String)> = decisions
        .iter()
        .map(|(staging_path, decision)| {
            let file = staging_path.to_string_lossy().into_owned();
            let subject = super::ipc::read_frontmatter_field(staging_path, "subject")
                .unwrap_or_else(|| {
                    staging_path
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default()
                });
            let from_raw =
                super::ipc::read_frontmatter_field(staging_path, "from").unwrap_or_default();
            let date = super::ipc::read_frontmatter_field(staging_path, "date").unwrap_or_default();
            let (sender_email, sender_domain) = extract_addr_and_domain(&from_raw);
            (
                file,
                subject,
                from_raw,
                date,
                decision.rel_path.clone(),
                decision.is_default,
                sender_email,
                sender_domain,
            )
        })
        .collect();

    // Staging files this window may act on, as the exact strings sent to the page
    // so the comparison never depends on path normalisation.
    let allowed_files: std::cell::RefCell<std::collections::HashSet<String>> =
        std::cell::RefCell::new(owned_rows.iter().map(|row| row.0.clone()).collect());

    let json_rows: Vec<serde_json::Value> = owned_rows
        .iter()
        .map(
            |(file, subject, sender, date, dest_path, is_default, sender_email, sender_domain)| {
                serde_json::json!({
                    "file":          file,
                    "subject":       subject,
                    "sender":        sender,
                    "date":          date,
                    "dest_path":     dest_path,
                    "is_default":    is_default,
                    "sender_email":  sender_email,
                    "sender_domain": sender_domain
                })
            },
        )
        .collect();

    let decisions_json =
        serde_json::to_string(&json_rows).context("failed to serialize decisions")?;
    let known_paths_json =
        serde_json::to_string(&known_paths).context("failed to serialize known paths")?;

    // Inject data via initialization script to avoid NavigateToString's ~2 MB limit.
    // AddScriptToExecuteOnDocumentCreated (called by with_initialization_script) has no
    // such size restriction and runs before any page script, so the globals are ready.
    let init_script = format!(
        "window.__DECISIONS_DATA__={};window.__KNOWN_PATHS__={};",
        escape_json_for_script(&decisions_json),
        escape_json_for_script(&known_paths_json)
    );

    let html_template = include_str!("../../assets/route_review.html");

    let (window, window_id) = build_gui_window(
        target,
        "Email to Markdown \u{2014} Revue du routage",
        (900.0, 600.0),
    )?;

    let proxy_ipc = proxy.clone();
    let webview = attach_webview(
        &window,
        html_template,
        Some(&init_script),
        move |req: wry::http::Request<String>| {
            let body = req.body().clone();

            // 1. Raw "cancel" string (not JSON) — close immediately.
            if body.trim() == "cancel" {
                let _ = proxy_ipc.send_event(AppCommand::CloseWindow { window_id });
                return;
            }

            // 2. Discriminate by `action` field (default `""` when absent).
            if let Ok(kind) = serde_json::from_str::<IpcKind>(&body) {
                if kind.action == "create_rule" {
                    match serde_json::from_str::<RuleCreatePayload>(&body) {
                        Ok(p) => {
                            let _ = proxy_ipc.send_event(AppCommand::PersistRoutingRule {
                                window_id,
                                dest_path: p.path,
                                attr_kind: p.attr_kind,
                                attr_value: p.attr_value,
                            });
                        }
                        Err(e) => {
                            let msg = format!("invalid create_rule payload: {:#}", e);
                            if let Ok(js_str) = serde_json::to_string(&msg) {
                                let js = format!("route_review_error({})", js_str);
                                let _ =
                                    proxy_ipc.send_event(AppCommand::EvalScript { window_id, js });
                            }
                        }
                    }
                    return;
                }
                if kind.action == "delete" {
                    match serde_json::from_str::<DeletePayload>(&body) {
                        Ok(p) => {
                            let (deleted, err) = super::ipc::delete_staged_emails(
                                &p.files,
                                &mut allowed_files.borrow_mut(),
                            );
                            // Remove the successfully-deleted rows from the table.
                            if let Ok(js_arr) = serde_json::to_string(&deleted) {
                                let js = format!(
                                    "route_review_deleted({})",
                                    escape_json_for_script(&js_arr)
                                );
                                let _ =
                                    proxy_ipc.send_event(AppCommand::EvalScript { window_id, js });
                            }
                            // Surface any failure without losing the successful deletions.
                            if let Some(msg) = err {
                                if let Ok(js_str) = serde_json::to_string(&msg) {
                                    let js = format!("route_review_error({})", js_str);
                                    let _ = proxy_ipc
                                        .send_event(AppCommand::EvalScript { window_id, js });
                                }
                            }
                        }
                        Err(e) => {
                            let msg = format!("invalid delete payload: {:#}", e);
                            if let Ok(js_str) = serde_json::to_string(&msg) {
                                let js = format!("route_review_error({})", js_str);
                                let _ =
                                    proxy_ipc.send_event(AppCommand::EvalScript { window_id, js });
                            }
                        }
                    }
                    return;
                }
            }

            // 3. Existing apply flow — payload `{ decisions: [...] }` (no `action` field).
            let organize_by_date = crate::config::Settings::load(&crate::config::settings_path())
                .unwrap_or_default()
                .organize_by_date;
            let outcome = super::ipc::parse_route_decisions(&body).and_then(|rows| {
                super::ipc::apply_route_decisions(
                    &rows,
                    &notes_dir,
                    &mut allowed_files.borrow_mut(),
                    organize_by_date,
                )
            });
            let error = match outcome {
                Ok(report) if report.is_complete() => {
                    let _ = proxy_ipc.send_event(AppCommand::CloseWindow { window_id });
                    return;
                }
                Ok(report) => {
                    // Drop the rows with nothing left to route, keep the failed ones.
                    if let Ok(js_arr) = serde_json::to_string(&report.settled_files()) {
                        let js = format!(
                            "route_review_applied({})",
                            escape_json_for_script(&js_arr)
                        );
                        let _ = proxy_ipc.send_event(AppCommand::EvalScript { window_id, js });
                    }
                    report.error_message().unwrap_or_default()
                }
                // Validation failed: nothing was moved.
                Err(e) => format!("{:#}", e),
            };
            // Surface the error back to the HTML without closing.
            if let Ok(js_str) = serde_json::to_string(&error) {
                let js = format!("route_review_error({})", escape_json_for_script(&js_str));
                let _ = proxy_ipc.send_event(AppCommand::EvalScript { window_id, js });
            }
        },
    )?;

    Ok((window, webview, window_id))
}

#[cfg(test)]
mod tests {
    use super::{allow_navigation, escape_json_for_script};
    use std::sync::atomic::AtomicBool;
    use std::time::Instant;

    #[test]
    fn test_escape_json_for_script_decodes_to_original() {
        let original = serde_json::json!({
            "name": "A & B <admin>",
            "path": "C:/notes/</script><img src=x onerror=alert(1)>",
        });
        let escaped = escape_json_for_script(&original.to_string());
        let decoded: serde_json::Value = serde_json::from_str(&escaped).expect("valid json");
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_escape_json_for_script_no_markup_characters() {
        let payload = serde_json::json!({ "v": "</script><b>a & b</b>" }).to_string();
        let escaped = escape_json_for_script(&payload);
        assert!(
            !escaped.contains(['<', '>', '&']),
            "no <, > or & may remain: {escaped}"
        );
    }

    #[test]
    fn test_allow_navigation_initial_load_then_refused() {
        let seen = AtomicBool::new(false);
        let decisions = [
            allow_navigation("about:blank", &seen),
            allow_navigation("data:text/html,initial", &seen),
            allow_navigation("https://example.org/", &seen),
            allow_navigation("about:blank", &seen),
        ];
        assert_eq!(decisions, [true, true, false, true]);
    }

    #[test]
    fn test_templates_declare_content_security_policy() {
        let templates = [
            include_str!("../../assets/config_window.html"),
            include_str!("../../assets/contextual_export.html"),
            include_str!("../../assets/destinations_window.html"),
            include_str!("../../assets/progress_window.html"),
            include_str!("../../assets/route_review.html"),
            include_str!("../../assets/update_window.html"),
        ];
        let with_policy = templates
            .iter()
            .filter(|html| html.contains("Content-Security-Policy\" content=\"default-src 'none'"))
            .count();
        assert_eq!(with_policy, templates.len());
    }

    #[test]
    fn test_escape_json_for_script_script_tag_breakout() {
        // A value containing </script> must not appear literally after escaping.
        let payload = r#"{"subject":"</script><script>alert(1)</script>"}"#;
        let escaped = escape_json_for_script(payload);
        // Exclusive: no literal </script> in the output
        assert!(
            !escaped.contains("</script>"),
            "escaped output must not contain literal </script>: {escaped}"
        );
        // Inclusive: the < and > are replaced by unicode escapes
        assert!(
            escaped.contains("\\u003c") && escaped.contains("\\u003e"),
            "< and > must be escaped to \\u003c / \\u003e: {escaped}"
        );
    }

    #[test]
    fn test_escape_json_for_script_ampersand_escaped() {
        let payload = r#"{"name":"A & B"}"#;
        let escaped = escape_json_for_script(payload);
        assert!(
            !escaped.contains(" & "),
            "literal & must not appear: {escaped}"
        );
        assert!(
            escaped.contains("\\u0026"),
            "& must be escaped to \\u0026: {escaped}"
        );
    }

    #[test]
    fn test_escape_json_for_script_plain_ascii_unchanged() {
        // Regular JSON without dangerous chars must pass through verbatim.
        let payload = r#"{"key":"hello world"}"#;
        let escaped = escape_json_for_script(payload);
        assert_eq!(
            escaped, payload,
            "plain ASCII JSON must be unchanged by escaping"
        );
    }

    #[test]
    fn contextual_asset_keeps_large_lists_bounded_and_accessible() {
        let html = include_str!("../../assets/contextual_export.html");
        for required in [
            "aria-live=\"polite\"",
            "aria-live=\"assertive\"",
            ":focus-visible",
            "slice(0,state.shown)",
            "state.shown=200",
            "Tout sélectionner",
            "Tout effacer",
            "already_present",
            "retry_deletion",
            "retry-conversion",
        ] {
            assert!(
                html.contains(required),
                "missing contextual UI contract: {required}"
            );
        }
    }

    #[test]
    fn contextual_filter_reference_handles_ten_thousand_rows_under_500ms() {
        let rows: Vec<String> = (0..10_000)
            .map(|index| format!("2026-09-01 alice{index}@example.com projet {index} inbox"))
            .collect();
        let started = Instant::now();
        let filtered: Vec<_> = rows
            .iter()
            .filter(|row| row.to_lowercase().contains("projet 9999"))
            .collect();
        assert_eq!(filtered.len(), 1);
        assert!(
            started.elapsed().as_millis() < 500,
            "10,000-row reference filter exceeded 500 ms: {:?}",
            started.elapsed()
        );
    }
}
