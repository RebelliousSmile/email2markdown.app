//! System tray module for Email to Markdown.
//!
//! This module provides a system tray icon with a context menu
//! and owns the application's single GUI event loop on the main
//! thread. All windows (progress, sort review, settings) live in
//! this loop and are routed by `WindowId`.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;

use anyhow::{Context, Result};
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::{Window, WindowId};
use wry::WebView;

use crate::config;
use crate::progress::ProgressUpdate;
use crate::route::RouteDecision;
use crate::tray_actions::{self, ActionResult};

mod event_loop;
mod ipc;
mod menu;
mod windows;

type CloseCb = Box<dyn FnOnce() + Send>;
type ActionCb = Box<dyn FnOnce() + Send>;

/// Commands routed through the main event loop's user-event channel.
pub enum AppCommand {
    ContextualSearchRequested {
        window_id: WindowId,
        account: String,
    },
    ContextualSearchFinished {
        window_id: WindowId,
        account: String,
        result: std::result::Result<tray_actions::ContextualSearchWork, String>,
    },
    ContextualConvertRequested {
        window_id: WindowId,
        keys: Vec<String>,
    },
    ContextualConvertFinished {
        window_id: WindowId,
        result: std::result::Result<tray_actions::ContextualBatchSummary, String>,
    },
    ContextualRetryDeletionRequested {
        window_id: WindowId,
    },
    ContextualRetryDeletionFinished {
        window_id: WindowId,
        result: std::result::Result<tray_actions::ContextualBatchSummary, String>,
    },
    ContextualOpenConfig,
    /// Persist a new routing rule for the contextual export destination and
    /// re-inject the refreshed rule labels into the webview.
    ContextualCreateRuleRequested {
        window_id: WindowId,
        attr_kind: String,
        attr_value: String,
    },
    OpenProgress {
        action_name: String,
        warning: Option<String>,
        progress_rx: mpsc::Receiver<ProgressUpdate>,
        on_close: Option<CloseCb>,
        error_action: Option<ActionCb>,
        sender: Sender<ActionResult>,
        cancel_token: Option<Arc<AtomicBool>>,
    },
    OpenConfig {
        sender: Sender<ActionResult>,
    },
    OpenUpdate,
    UpdateMsg(String),
    /// Forwarded by the bridge thread that drains `progress_rx`.
    ProgressUpdate {
        window_id: WindowId,
        update: ProgressUpdate,
    },
    /// IPC "action" from a progress window → run `error_action` then close.
    ActionRequested {
        window_id: WindowId,
    },
    /// Programmatic close (e.g. sent by an IPC handler after a save).
    CloseWindow {
        window_id: WindowId,
    },
    /// Evaluate JS in the WebView of the given window.
    EvalScript {
        window_id: WindowId,
        js: String,
    },
    /// Open the route review window after an Export.
    /// Carries the list of (staging_path, RouteDecision) produced by export_account.
    OpenRouteReview(Vec<(PathBuf, RouteDecision)>),
    /// Persist a new routing rule into destinations.txt and re-inject the updated tree.
    /// IO is done in the event loop (not in the webview IPC callback).
    PersistRoutingRule {
        window_id: WindowId,
        dest_path: String,
        attr_kind: String,
        attr_value: String,
    },
    /// Open the destinations management GUI window.
    OpenDestGui {
        dest_file: PathBuf,
    },
    /// Push a serialized JSON state update to the destinations GUI webview.
    /// Dispatched via proxy so evaluate_script runs in the event loop, not the IPC closure.
    PushDestState {
        window_id: WindowId,
        json: String,
    },
    /// The contextual destination editor saved its configuration. Revalidate
    /// the clicked directory and continue directly to the email search.
    ContextualDestinationSaved {
        window_id: WindowId,
    },
    /// Forwarded by the bridge thread blocking on `MenuEvent::receiver()`.
    MenuEventReceived(String),
    /// Forwarded by the bridge thread blocking on `result_receiver`.
    ActionResultReceived(ActionResult),
}

/// Per-progress-window state. Fields declared in drop order:
/// callbacks first (cheap), then webview (must release WebView2 before
/// the parent HWND is destroyed), then window.
struct ProgressState {
    on_close: Option<CloseCb>,
    error_action: Option<ActionCb>,
    webview: WebView,
    // Kept alive for its Drop side-effect — webview must drop before window.
    #[allow(dead_code)]
    window: Window,
}

/// Per-config-window state. Same drop-order discipline as `ProgressState`.
struct ConfigState {
    #[allow(dead_code)]
    webview: WebView,
    #[allow(dead_code)]
    window: Window,
}

/// Per-update-window state. Same drop-order discipline as `ProgressState`.
struct UpdateState {
    webview: WebView,
    #[allow(dead_code)]
    window: Window,
}

/// Per-route-review-window state. Same drop-order discipline as `ProgressState`.
struct RouteState {
    #[allow(dead_code)]
    webview: WebView,
    #[allow(dead_code)]
    window: Window,
}

/// Per-destinations-gui-window state. Same drop-order discipline as `ProgressState`.
struct DestGuiState {
    #[allow(dead_code)]
    cfg: Arc<Mutex<crate::destinations::DestinationsConfig>>,
    #[allow(dead_code)]
    dest_file: PathBuf,
    webview: WebView,
    #[allow(dead_code)]
    window: Window,
}

struct ContextualState {
    launch: tray_actions::ContextualLaunch,
    account: Option<String>,
    candidates: Vec<crate::contextual_export::ContextualCandidate>,
    retry_deletion: Vec<crate::contextual_export::DeletionRequest>,
    busy: bool,
    webview: WebView,
    #[allow(dead_code)]
    window: Window,
}

enum ContextualStandaloneState {
    DestinationSetup(DestGuiState),
    Export(ContextualState),
}

impl ContextualStandaloneState {
    fn window_id(&self) -> WindowId {
        match self {
            Self::DestinationSetup(state) => state.window.id(),
            Self::Export(state) => state.window.id(),
        }
    }
}

enum WState {
    Progress(ProgressState),
    Config(#[allow(dead_code)] ConfigState),
    Update(UpdateState),
    Route(#[allow(dead_code)] RouteState),
    DestGui(DestGuiState),
}

static APP_PROXY: OnceLock<EventLoopProxy<AppCommand>> = OnceLock::new();

/// Send a command to the main event loop. Returns Err if the loop is not running yet.
pub fn send_command(cmd: AppCommand) -> Result<()> {
    APP_PROXY
        .get()
        .context("tray event loop not initialised")?
        .send_event(cmd)
        .map_err(|_| anyhow::anyhow!("tray event loop closed"))
}

/// Run the system tray application.
pub fn run_tray() -> Result<()> {
    event_loop::run()
}

/// Run one contextual export window as a standalone process. This is the entry
/// point used by file-manager actions; closing the window exits the process.
fn contextual_launch_error_message(target_path: &Path, error: &anyhow::Error) -> String {
    let detail = format!("{error:#}");
    let guidance = if detail.contains("no usable address search rule") {
        "Aucune règle d’expéditeur ou de destinataire n’est définie pour ce dossier.\n\
         Ouvrez « Configurer les destinations » dans Email to Markdown, puis ajoutez au moins \
         une règle correspondant, expéditeur ou domaine."
    } else {
        "Vérifiez la configuration des comptes et des destinations dans Email to Markdown."
    };

    format!(
        "Impossible de préparer l’export des emails.\n\nDossier : {}\n\n{}\n\nDétail technique : {}",
        target_path.display(),
        guidance,
        detail
    )
}

pub fn run_contextual(target_path: PathBuf) -> Result<()> {
    let (initial_launch, setup_path) = match tray_actions::prepare_contextual_launch(&target_path) {
        Ok(launch) => (Some(launch), None),
        Err(error) => {
            if let Some(crate::route::ContextualDestinationError::MissingAddressRule { path }) =
                error.downcast_ref::<crate::route::ContextualDestinationError>()
            {
                (None, Some(path.clone()))
            } else {
                let message = contextual_launch_error_message(&target_path, &error);
                rfd::MessageDialog::new()
                    .set_level(rfd::MessageLevel::Error)
                    .set_title("Export contextuel impossible")
                    .set_description(&message)
                    .set_buttons(rfd::MessageButtons::Ok)
                    .show();
                return Ok(());
            }
        }
    };
    let event_loop = EventLoopBuilder::<AppCommand>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    APP_PROXY
        .set(proxy.clone())
        .map_err(|_| anyhow::anyhow!("APP_PROXY already initialised"))?;
    let mut state: Option<ContextualStandaloneState> = None;

    event_loop.run(move |event, target, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::NewEvents(StartCause::Init) => {
                if let Some(launch) = initial_launch.as_ref() {
                    match windows::build_contextual_window(target, &proxy, launch) {
                    Ok((window, webview, window_id)) => {
                        state = Some(ContextualStandaloneState::Export(ContextualState {
                            launch: launch.clone(),
                            account: None,
                            candidates: Vec::new(),
                            retry_deletion: Vec::new(),
                            busy: false,
                            webview,
                            window,
                        }));
                        let _ = window_id;
                    }
                    Err(error) => {
                        eprintln!("Fenêtre contextuelle : {error:#}");
                        *control_flow = ControlFlow::Exit;
                    }
                    }
                } else if let Some(path) = setup_path.as_deref() {
                    let dest_file = crate::route::destinations_path();
                    match windows::build_dest_gui_window(target, &proxy, &dest_file, Some(path), true) {
                        Ok((window, webview, _window_id, cfg)) => {
                            state = Some(ContextualStandaloneState::DestinationSetup(DestGuiState {
                                cfg,
                                dest_file,
                                webview,
                                window,
                            }));
                        }
                        Err(error) => {
                            eprintln!("Fenêtre de configuration contextuelle : {error:#}");
                            *control_flow = ControlFlow::Exit;
                        }
                    }
                }
            }
            Event::UserEvent(AppCommand::ContextualSearchRequested { window_id, account }) => {
                let Some(ContextualStandaloneState::Export(current)) = state.as_mut() else {
                    return;
                };
                if current.window.id() != window_id || current.busy {
                    return;
                }
                current.busy = true;
                current.account = Some(account.clone());
                current.candidates.clear();
                current.retry_deletion.clear();
                let launch = current.launch.clone();
                let worker_proxy = proxy.clone();
                thread::spawn(move || {
                    let result = tray_actions::run_contextual_search(&launch, &account)
                        .map_err(|error| format!("{error:#}"));
                    let _ = worker_proxy.send_event(AppCommand::ContextualSearchFinished {
                        window_id,
                        account,
                        result,
                    });
                });
            }
            Event::UserEvent(AppCommand::ContextualSearchFinished {
                window_id,
                account,
                result,
            }) => {
                let Some(ContextualStandaloneState::Export(current)) = state.as_mut() else {
                    return;
                };
                if current.window.id() != window_id || current.account.as_deref() != Some(&account)
                {
                    return;
                }
                current.busy = false;
                match result {
                    Ok(work) => {
                        current.candidates = work.candidates;
                        let payload = serde_json::json!({
                            "rows": work.rows,
                            "preflight": work.preflight,
                        });
                        if let Ok(json) = serde_json::to_string(&payload) {
                            let _ = current.webview.evaluate_script(&format!(
                                "contextual_search_done({})",
                                windows::escape_json_for_script(&json)
                            ));
                        }
                    }
                    Err(error) => contextual_eval_error(&current.webview, "search", &error),
                }
            }
            Event::UserEvent(AppCommand::ContextualConvertRequested { window_id, keys }) => {
                let Some(ContextualStandaloneState::Export(current)) = state.as_mut() else {
                    return;
                };
                if current.window.id() != window_id || current.busy || keys.is_empty() {
                    return;
                }
                let Some(account) = current.account.clone() else {
                    contextual_eval_error(
                        &current.webview,
                        "conversion",
                        "Choisissez une boîte aux lettres",
                    );
                    return;
                };
                let selected: Vec<_> = current
                    .candidates
                    .iter()
                    .filter(|candidate| keys.contains(&candidate.logical_key()))
                    .cloned()
                    .collect();
                if selected.is_empty() {
                    contextual_eval_error(
                        &current.webview,
                        "conversion",
                        "La sélection est vide ou périmée",
                    );
                    return;
                }
                current.busy = true;
                let target_path = current.launch.target.clone();
                let worker_proxy = proxy.clone();
                thread::spawn(move || {
                    let result =
                        tray_actions::run_contextual_batch(&target_path, &account, &selected)
                            .map_err(|error| format!("{error:#}"));
                    let _ = worker_proxy
                        .send_event(AppCommand::ContextualConvertFinished { window_id, result });
                });
            }
            Event::UserEvent(AppCommand::ContextualConvertFinished { window_id, result }) => {
                let Some(ContextualStandaloneState::Export(current)) = state.as_mut() else {
                    return;
                };
                if current.window.id() != window_id {
                    return;
                }
                current.busy = false;
                match result {
                    Ok(summary) if summary.complete() => *control_flow = ControlFlow::Exit,
                    Ok(summary) => {
                        current.retry_deletion = summary.retry_deletion.clone();
                        if let Ok(json) = serde_json::to_string(&summary) {
                            let _ = current.webview.evaluate_script(&format!(
                                "contextual_batch_partial({})",
                                windows::escape_json_for_script(&json)
                            ));
                        }
                    }
                    Err(error) => contextual_eval_error(&current.webview, "conversion", &error),
                }
            }
            Event::UserEvent(AppCommand::ContextualRetryDeletionRequested { window_id }) => {
                let Some(ContextualStandaloneState::Export(current)) = state.as_mut() else {
                    return;
                };
                if current.window.id() != window_id
                    || current.busy
                    || current.retry_deletion.is_empty()
                {
                    return;
                }
                let Some(account) = current.account.clone() else {
                    return;
                };
                current.busy = true;
                let requests = current.retry_deletion.clone();
                let worker_proxy = proxy.clone();
                thread::spawn(move || {
                    let result = tray_actions::retry_contextual_deletions(&account, &requests)
                        .map_err(|error| format!("{error:#}"));
                    let _ = worker_proxy.send_event(AppCommand::ContextualRetryDeletionFinished {
                        window_id,
                        result,
                    });
                });
            }
            Event::UserEvent(AppCommand::ContextualRetryDeletionFinished { window_id, result }) => {
                let Some(ContextualStandaloneState::Export(current)) = state.as_mut() else {
                    return;
                };
                if current.window.id() != window_id {
                    return;
                }
                current.busy = false;
                match result {
                    Ok(summary) if summary.complete() => *control_flow = ControlFlow::Exit,
                    Ok(summary) => {
                        current.retry_deletion = summary.retry_deletion.clone();
                        if let Ok(json) = serde_json::to_string(&summary) {
                            let _ = current.webview.evaluate_script(&format!(
                                "contextual_batch_partial({})",
                                windows::escape_json_for_script(&json)
                            ));
                        }
                    }
                    Err(error) => contextual_eval_error(&current.webview, "deletion", &error),
                }
            }
            Event::UserEvent(AppCommand::PushDestState { window_id, json }) => {
                if let Some(ContextualStandaloneState::DestinationSetup(current)) = state.as_ref() {
                    if current.window.id() == window_id {
                        if let Ok(js_str) = serde_json::to_string(&json) {
                            let _ = current
                                .webview
                                .evaluate_script(&format!("window_msg({js_str})"));
                        }
                    }
                }
            }
            Event::UserEvent(AppCommand::ContextualDestinationSaved { window_id }) => {
                if state.as_ref().map(ContextualStandaloneState::window_id) != Some(window_id) {
                    return;
                }
                match tray_actions::prepare_contextual_launch(&target_path) {
                    Ok(launch) => {
                        state = None;
                        match windows::build_contextual_window(target, &proxy, &launch) {
                            Ok((window, webview, _)) => {
                                state = Some(ContextualStandaloneState::Export(ContextualState {
                                    launch,
                                    account: None,
                                    candidates: Vec::new(),
                                    retry_deletion: Vec::new(),
                                    busy: false,
                                    webview,
                                    window,
                                }));
                            }
                            Err(error) => {
                                eprintln!("Fenêtre contextuelle : {error:#}");
                                *control_flow = ControlFlow::Exit;
                            }
                        }
                    }
                    Err(error)
                        if error
                            .downcast_ref::<crate::route::ContextualDestinationError>()
                            .is_some() =>
                    {
                        rfd::MessageDialog::new()
                            .set_level(rfd::MessageLevel::Warning)
                            .set_title("Une règle est nécessaire")
                            .set_description(
                                "Ajoutez au moins une règle « Correspondant », « Expéditeur » ou « Domaine » avant d’enregistrer.",
                            )
                            .set_buttons(rfd::MessageButtons::Ok)
                            .show();
                    }
                    Err(error) => {
                        let message = contextual_launch_error_message(&target_path, &error);
                        rfd::MessageDialog::new()
                            .set_level(rfd::MessageLevel::Error)
                            .set_title("Export contextuel impossible")
                            .set_description(&message)
                            .set_buttons(rfd::MessageButtons::Ok)
                            .show();
                    }
                }
            }
            Event::UserEvent(AppCommand::ContextualOpenConfig) => {
                if let Err(e) = open::that(config::accounts_yaml_path()) {
                    eprintln!("Ouverture accounts.yaml: {:#}", e);
                }
            }
            Event::UserEvent(AppCommand::ContextualCreateRuleRequested {
                window_id,
                attr_kind,
                attr_value,
            }) => {
                let Some(ContextualStandaloneState::Export(current)) = state.as_mut() else {
                    return;
                };
                if current.window.id() != window_id {
                    return;
                }
                let rule = match attr_kind.as_str() {
                    "domain" => Some(crate::route::MatchRule::Domain(attr_value.clone())),
                    "from" => Some(crate::route::MatchRule::From(attr_value.clone())),
                    _ => None,
                };
                let Some(rule) = rule else {
                    contextual_eval_error(
                        &current.webview,
                        "rule",
                        &format!("type de règle non pris en charge : {attr_kind}"),
                    );
                    return;
                };
                let dest_file = crate::route::destinations_path();
                if let Err(error) =
                    crate::route::upsert_rule(&dest_file, &current.launch.relative_path, rule)
                {
                    contextual_eval_error(&current.webview, "rule", &format!("{error:#}"));
                    return;
                }
                match tray_actions::prepare_contextual_launch(&current.launch.target) {
                    Ok(refreshed) => {
                        current.launch = refreshed;
                        if let Ok(json) = serde_json::to_string(&current.launch.rule_labels) {
                            let _ = current.webview.evaluate_script(&format!(
                                "contextual_rules_updated({})",
                                windows::escape_json_for_script(&json)
                            ));
                        }
                    }
                    Err(error) => {
                        contextual_eval_error(&current.webview, "rule", &format!("{error:#}"));
                    }
                }
            }
            Event::UserEvent(AppCommand::CloseWindow { window_id }) => {
                if state.as_ref().map(ContextualStandaloneState::window_id) == Some(window_id) {
                    state = None;
                    *control_flow = ControlFlow::Exit;
                }
            }
            Event::WindowEvent {
                window_id,
                event: WindowEvent::CloseRequested,
                ..
            } => {
                if state.as_ref().map(ContextualStandaloneState::window_id) == Some(window_id) {
                    state = None;
                    *control_flow = ControlFlow::Exit;
                }
            }
            _ => {}
        }
    });
}

#[cfg(test)]
mod contextual_launch_tests {
    use super::contextual_launch_error_message;
    use std::path::Path;

    #[test]
    fn launch_failure_message_names_the_directory_and_configuration_action() {
        let message = contextual_launch_error_message(
            Path::new(r"C:\Notes\Perso\Associations"),
            &anyhow::anyhow!("destination has no usable address search rule: Perso/Associations"),
        );

        assert!(message.contains(r"C:\Notes\Perso\Associations"));
        assert!(message.contains("Aucune règle d’expéditeur ou de destinataire"));
        assert!(message.contains("Configurer les destinations"));
    }
}

fn contextual_eval_error(webview: &WebView, kind: &str, message: &str) {
    if let (Ok(kind), Ok(message)) = (serde_json::to_string(kind), serde_json::to_string(message)) {
        let _ = webview.evaluate_script(&format!("contextual_error({kind},{message})"));
    }
}

#[derive(serde::Deserialize)]
struct ContextualIpcMessage {
    action: String,
    account: Option<String>,
    #[serde(default)]
    keys: Vec<String>,
    attr_kind: Option<String>,
    attr_value: Option<String>,
}
