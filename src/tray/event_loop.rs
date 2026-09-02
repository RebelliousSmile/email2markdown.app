//! Main GUI event loop: tray icon lifecycle, window registry, and the
//! `AppCommand` dispatch table for `run_tray`.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;

use anyhow::Result;
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget};
use tao::window::WindowId;
use tray_icon::{menu::MenuEvent, TrayIcon};

use crate::progress::ProgressUpdate;
use crate::route::RouteDecision;
use crate::tray_actions::ActionResult;

use super::{
    ActionCb, AppCommand, CloseCb, ConfigState, DestGuiState, ProgressState, RouteState,
    UpdateState, WState, APP_PROXY,
};

/// Identifies each single-instance GUI window kind tracked by [`WINDOW_REGISTRY`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum WindowKind {
    #[allow(dead_code)]
    Contextual,
    #[allow(dead_code)]
    Progress,
    Config,
    Update,
    DestGui,
    Route,
}

/// Tracks which single-instance windows are currently open, keyed by [`WindowKind`].
/// Only ever accessed from the `tao` event loop thread (single-threaded — no
/// `store`/`load` happens outside `run`), so a plain `Mutex` is used purely
/// as the simplest container for shared named state, not for cross-thread safety.
static WINDOW_REGISTRY: LazyLock<Mutex<HashMap<WindowKind, bool>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Atomically claims `kind` if it is not already open. Returns `false` (and leaves
/// the registry unchanged) when a window of that kind is already open — mirrors the
/// previous `AtomicBool::compare_exchange(false, true, ...)` single-instance guard.
fn try_claim_window(kind: WindowKind) -> bool {
    match WINDOW_REGISTRY.lock() {
        Ok(mut registry) => {
            if registry.get(&kind).copied().unwrap_or(false) {
                false
            } else {
                registry.insert(kind, true);
                true
            }
        }
        Err(_) => {
            eprintln!("Registre des fenêtres : mutex empoisonné");
            false
        }
    }
}

/// Marks `kind` as closed, freeing its registry entry without affecting any other kind.
fn close_window_kind(kind: WindowKind) {
    match WINDOW_REGISTRY.lock() {
        Ok(mut registry) => {
            registry.insert(kind, false);
        }
        Err(_) => eprintln!("Registre des fenêtres : mutex empoisonné"),
    }
}

/// Run the system tray application.
pub(super) fn run() -> Result<()> {
    let event_loop = EventLoopBuilder::<AppCommand>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    APP_PROXY
        .set(proxy.clone())
        .map_err(|_| anyhow::anyhow!("APP_PROXY already initialised"))?;

    let (result_sender, result_receiver) = mpsc::channel::<ActionResult>();
    let menu_channel = MenuEvent::receiver();

    // Bridge threads: block on the external channels and push their payload
    // through the proxy so the event loop can run under `ControlFlow::Wait`
    // instead of busy-polling both channels every tick.
    let menu_bridge_proxy = proxy.clone();
    thread::spawn(move || {
        while let Ok(menu_event) = menu_channel.recv() {
            let _ = menu_bridge_proxy.send_event(AppCommand::MenuEventReceived(menu_event.id.0));
        }
    });

    let result_bridge_proxy = proxy.clone();
    thread::spawn(move || {
        while let Ok(result) = result_receiver.recv() {
            let _ = result_bridge_proxy.send_event(AppCommand::ActionResultReceived(result));
        }
    });

    let mut tray_icon: Option<TrayIcon> = None;
    let mut windows: HashMap<WindowId, WState> = HashMap::new();

    event_loop.run(move |event, target, control_flow| {
        *control_flow = ControlFlow::Wait;

        if let Event::NewEvents(StartCause::Init) = event {
            match super::menu::create_tray_icon() {
                Ok(icon) => {
                    tray_icon = Some(icon);
                    println!("Tray icon created successfully");
                }
                Err(e) => {
                    eprintln!("Failed to create tray icon: {}", e);
                }
            }
        }

        match event {
            Event::UserEvent(AppCommand::OpenProgress {
                action_name,
                warning,
                progress_rx,
                on_close,
                error_action,
                sender,
                cancel_token,
            }) => handle_open_progress(
                target,
                &proxy,
                &mut windows,
                action_name,
                warning,
                progress_rx,
                on_close,
                error_action,
                sender,
                cancel_token,
            ),
            Event::UserEvent(AppCommand::OpenConfig { sender }) => {
                handle_open_config(target, &proxy, &mut windows, sender)
            }
            Event::UserEvent(AppCommand::OpenUpdate) => {
                handle_open_update(target, &proxy, &mut windows)
            }
            Event::UserEvent(AppCommand::UpdateMsg(msg)) => handle_update_msg(&windows, msg),
            Event::UserEvent(AppCommand::ProgressUpdate { window_id, update }) => {
                handle_progress_update(&windows, window_id, update)
            }
            Event::UserEvent(AppCommand::EvalScript { window_id, js }) => {
                handle_eval_script(&windows, window_id, js)
            }
            Event::UserEvent(AppCommand::PersistRoutingRule {
                window_id,
                dest_path,
                attr_kind,
                attr_value,
            }) => handle_persist_routing_rule(&windows, window_id, dest_path, attr_kind, attr_value),
            Event::UserEvent(AppCommand::OpenRouteReview(decisions)) => {
                handle_open_route_review(target, &proxy, &mut windows, decisions)
            }
            Event::UserEvent(AppCommand::OpenDestGui { dest_file }) => {
                handle_open_dest_gui(target, &proxy, &mut windows, dest_file)
            }
            Event::UserEvent(AppCommand::PushDestState { window_id, json }) => {
                handle_push_dest_state(&windows, window_id, json)
            }
            Event::UserEvent(AppCommand::ContextualDestinationSaved { window_id }) => {
                handle_contextual_destination_saved(&mut windows, window_id)
            }
            Event::UserEvent(AppCommand::ActionRequested { window_id }) => {
                handle_action_requested(&mut windows, window_id)
            }
            Event::UserEvent(AppCommand::CloseWindow { window_id }) => {
                handle_close_window(&mut windows, window_id)
            }
            Event::WindowEvent {
                window_id,
                event: WindowEvent::CloseRequested,
                ..
            } => handle_close_requested(&mut windows, window_id),
            Event::UserEvent(AppCommand::MenuEventReceived(id)) => {
                handle_menu_event_received(&id, result_sender.clone())
            }
            Event::UserEvent(AppCommand::ActionResultReceived(result)) => {
                handle_action_result_received(&tray_icon, result)
            }
            _ => {}
        }

        let _ = &tray_icon;
    });
}

#[allow(clippy::too_many_arguments)]
fn handle_open_progress(
    target: &EventLoopWindowTarget<AppCommand>,
    proxy: &EventLoopProxy<AppCommand>,
    windows: &mut HashMap<WindowId, WState>,
    action_name: String,
    warning: Option<String>,
    progress_rx: mpsc::Receiver<ProgressUpdate>,
    on_close: Option<CloseCb>,
    error_action: Option<ActionCb>,
    sender: Sender<ActionResult>,
    cancel_token: Option<Arc<AtomicBool>>,
) {
    match super::windows::build_progress_window(
        target,
        proxy,
        &action_name,
        warning.as_deref(),
        cancel_token,
    ) {
        Ok((window, webview, window_id)) => {
            windows.insert(
                window_id,
                WState::Progress(ProgressState {
                    on_close,
                    error_action,
                    webview,
                    window,
                }),
            );
            let bridge_proxy = proxy.clone();
            thread::spawn(move || {
                for update in progress_rx {
                    if matches!(update, ProgressUpdate::AutoClose) {
                        let _ = bridge_proxy.send_event(AppCommand::CloseWindow { window_id });
                        break;
                    }
                    let terminal = matches!(
                        update,
                        ProgressUpdate::Done { .. } | ProgressUpdate::Error { .. }
                    );
                    let _ =
                        bridge_proxy.send_event(AppCommand::ProgressUpdate { window_id, update });
                    if terminal {
                        break;
                    }
                }
            });
        }
        Err(e) => {
            let _ = sender.send(ActionResult::Error(format!(
                "Fenêtre de progression : {}",
                e
            )));
        }
    }
}

fn handle_open_config(
    target: &EventLoopWindowTarget<AppCommand>,
    proxy: &EventLoopProxy<AppCommand>,
    windows: &mut HashMap<WindowId, WState>,
    sender: Sender<ActionResult>,
) {
    if !try_claim_window(WindowKind::Config) {
        // Already open — ignore.
        return;
    }
    match super::windows::build_config_window(target, proxy, sender.clone()) {
        Ok((window, webview, window_id)) => {
            windows.insert(window_id, WState::Config(ConfigState { webview, window }));
        }
        Err(e) => {
            close_window_kind(WindowKind::Config);
            let _ = sender.send(ActionResult::Error(format!(
                "Fenêtre de paramètres : {:#}",
                e
            )));
        }
    }
}

fn handle_open_update(
    target: &EventLoopWindowTarget<AppCommand>,
    proxy: &EventLoopProxy<AppCommand>,
    windows: &mut HashMap<WindowId, WState>,
) {
    if !try_claim_window(WindowKind::Update) {
        // Already open — ignore.
        return;
    }
    match super::windows::build_update_window(target, proxy) {
        Ok((window, webview, window_id)) => {
            windows.insert(window_id, WState::Update(UpdateState { webview, window }));
        }
        Err(e) => {
            close_window_kind(WindowKind::Update);
            eprintln!("Fenêtre de mise à jour : {:#}", e);
        }
    }
}

fn handle_update_msg(windows: &HashMap<WindowId, WState>, msg: String) {
    for state in windows.values() {
        if let WState::Update(update_state) = state {
            // Serialize the JSON string as a JS string literal (handles all escapes).
            if let Ok(js_str) = serde_json::to_string(&msg) {
                let js = format!("window_msg({})", js_str);
                let _ = update_state.webview.evaluate_script(&js);
            }
            break;
        }
    }
}

fn handle_progress_update(
    windows: &HashMap<WindowId, WState>,
    window_id: WindowId,
    update: ProgressUpdate,
) {
    if let Some(WState::Progress(state)) = windows.get(&window_id) {
        let js = format_progress_js(&update);
        let _ = state.webview.evaluate_script(&js);
    }
}

fn handle_eval_script(windows: &HashMap<WindowId, WState>, window_id: WindowId, js: String) {
    if let Some(WState::Route(state)) = windows.get(&window_id) {
        let _ = state.webview.evaluate_script(&js);
    }
}

fn handle_persist_routing_rule(
    windows: &HashMap<WindowId, WState>,
    window_id: WindowId,
    dest_path: String,
    attr_kind: String,
    attr_value: String,
) {
    // Resolve destinations.yaml path from settings (shared resolver).
    let dest_file = crate::route::destinations_path();

    // Reject subject/account; only domain/from are surfaced in the UI.
    let rule_opt = match attr_kind.as_str() {
        "domain" => Some(crate::route::MatchRule::Domain(attr_value.clone())),
        "from" => Some(crate::route::MatchRule::From(attr_value.clone())),
        _ => None,
    };

    let Some(rule) = rule_opt else {
        let msg = format!(
            "unsupported attr_kind {:?} — only domain/from allowed",
            attr_kind
        );
        if let (Ok(js_str), Some(WState::Route(state))) =
            (serde_json::to_string(&msg), windows.get(&window_id))
        {
            let _ = state
                .webview
                .evaluate_script(&format!("route_review_error({})", js_str));
        }
        return;
    };

    match crate::route::upsert_rule(&dest_file, &dest_path, rule) {
        Err(e) => {
            let msg = format!("{:#}", e);
            if let (Ok(js_str), Some(WState::Route(state))) =
                (serde_json::to_string(&msg), windows.get(&window_id))
            {
                let _ = state
                    .webview
                    .evaluate_script(&format!("route_review_error({})", js_str));
            }
        }
        Ok(()) => {
            // Re-read destinations and inject the updated path list.
            let known_paths: Vec<String> = crate::route::load_destinations()
                .into_iter()
                .map(|d| d.path)
                .collect();
            if let (Ok(json), Some(WState::Route(state))) =
                (serde_json::to_string(&known_paths), windows.get(&window_id))
            {
                let escaped = super::windows::escape_json_for_script(&json);
                let _ = state
                    .webview
                    .evaluate_script(&format!("route_review_set_tree({})", escaped));
            }
        }
    }
}

fn handle_open_route_review(
    target: &EventLoopWindowTarget<AppCommand>,
    proxy: &EventLoopProxy<AppCommand>,
    windows: &mut HashMap<WindowId, WState>,
    decisions: Vec<(std::path::PathBuf, RouteDecision)>,
) {
    if !try_claim_window(WindowKind::Route) {
        // Already open — ignore. The previous window should be closed first.
        return;
    }
    match super::windows::build_route_window(target, proxy, decisions) {
        Ok((window, webview, window_id)) => {
            windows.insert(window_id, WState::Route(RouteState { webview, window }));
        }
        Err(e) => {
            close_window_kind(WindowKind::Route);
            eprintln!("Fenêtre de revue de routage : {:#}", e);
        }
    }
}

fn handle_open_dest_gui(
    target: &EventLoopWindowTarget<AppCommand>,
    proxy: &EventLoopProxy<AppCommand>,
    windows: &mut HashMap<WindowId, WState>,
    dest_file: std::path::PathBuf,
) {
    if !try_claim_window(WindowKind::DestGui) {
        // Already open — ignore.
        return;
    }
    match super::windows::build_dest_gui_window(target, proxy, &dest_file, None, false) {
        Ok((window, webview, window_id, cfg)) => {
            windows.insert(
                window_id,
                WState::DestGui(DestGuiState {
                    cfg,
                    dest_file,
                    webview,
                    window,
                }),
            );
        }
        Err(e) => {
            close_window_kind(WindowKind::DestGui);
            eprintln!("Fenêtre destinations : {:#}", e);
        }
    }
}

fn handle_push_dest_state(windows: &HashMap<WindowId, WState>, window_id: WindowId, json: String) {
    if let Some(WState::DestGui(state)) = windows.get(&window_id) {
        if let Ok(js_str) = serde_json::to_string(&json) {
            let js = format!("window_msg({})", js_str);
            let _ = state.webview.evaluate_script(&js);
        }
    }
}

fn handle_contextual_destination_saved(windows: &mut HashMap<WindowId, WState>, window_id: WindowId) {
    // This command is only emitted by the standalone contextual
    // editor. Keep the regular tray loop defensive if received.
    if let Some(WState::DestGui(_)) = windows.remove(&window_id) {
        close_window_kind(WindowKind::DestGui);
    }
}

fn handle_action_requested(windows: &mut HashMap<WindowId, WState>, window_id: WindowId) {
    if let Some(WState::Progress(mut state)) = windows.remove(&window_id) {
        if let Some(f) = state.error_action.take() {
            f();
        }
    }
}

fn handle_close_window(windows: &mut HashMap<WindowId, WState>, window_id: WindowId) {
    match windows.remove(&window_id) {
        Some(WState::Config(_)) => close_window_kind(WindowKind::Config),
        Some(WState::Update(_)) => close_window_kind(WindowKind::Update),
        Some(WState::Route(_)) => close_window_kind(WindowKind::Route),
        Some(WState::DestGui(_)) => close_window_kind(WindowKind::DestGui),
        _ => {}
    }
}

fn handle_close_requested(windows: &mut HashMap<WindowId, WState>, window_id: WindowId) {
    match windows.remove(&window_id) {
        Some(WState::Progress(mut state)) => {
            if let Some(f) = state.on_close.take() {
                f();
            }
        }
        Some(WState::Config(_)) => close_window_kind(WindowKind::Config),
        Some(WState::Update(_)) => close_window_kind(WindowKind::Update),
        Some(WState::Route(_)) => close_window_kind(WindowKind::Route),
        Some(WState::DestGui(_)) => close_window_kind(WindowKind::DestGui),
        None => {}
    }
}

fn handle_menu_event_received(id: &str, result_sender: Sender<ActionResult>) {
    super::menu::handle_menu_event(id, result_sender);
}

fn handle_action_result_received(tray_icon: &Option<TrayIcon>, result: ActionResult) {
    match &result {
        ActionResult::Imported(_) => {
            if let Some(ref icon) = tray_icon {
                match super::menu::create_menu() {
                    Ok(new_menu) => icon.set_menu(Some(Box::new(new_menu))),
                    Err(e) => eprintln!("Failed to rebuild menu: {}", e),
                }
            }
        }
        _ => {
            show_notification(&result);
        }
    }
}

fn format_progress_js(update: &ProgressUpdate) -> String {
    match update {
        ProgressUpdate::Step {
            current,
            total,
            message,
        } => {
            format!("step({},{},{:?})", current, total, message)
        }
        ProgressUpdate::Indeterminate { message } => {
            format!("indeterminate({:?})", message)
        }
        ProgressUpdate::Done { summary } => {
            format!("finish({:?})", summary)
        }
        ProgressUpdate::Error {
            message,
            action_label,
        } => {
            format!(
                "error({:?}, {:?})",
                message,
                action_label.as_deref().unwrap_or("")
            )
        }
        ProgressUpdate::StatusLine { text } => {
            format!("statusLine({:?})", text)
        }
        // AutoClose is consumed by the bridge thread before reaching here.
        ProgressUpdate::AutoClose => String::new(),
    }
}

/// Show a notification to the user (spawns a thread to avoid blocking the event loop).
fn show_notification(result: &ActionResult) {
    let (title, description, level) = match result {
        ActionResult::Success(title, m) => (title.clone(), m.clone(), rfd::MessageLevel::Info),
        ActionResult::Imported(m) => (
            "Import Thunderbird".to_string(),
            m.clone(),
            rfd::MessageLevel::Info,
        ),
        ActionResult::Error(m) => (
            "Email to Markdown - Erreur".to_string(),
            m.clone(),
            rfd::MessageLevel::Error,
        ),
    };

    thread::spawn(move || {
        rfd::MessageDialog::new()
            .set_title(&title)
            .set_description(&description)
            .set_level(level)
            .show();
    });
}
