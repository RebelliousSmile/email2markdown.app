//! System tray module for Email to Markdown.
//!
//! This module provides a system tray icon with a context menu
//! and owns the application's single GUI event loop on the main
//! thread. All windows (progress, sort review, settings) live in
//! this loop and are routed by `WindowId`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, OnceLock};
use std::thread;

use anyhow::{Context, Result};
use tao::dpi::LogicalSize;
use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget};
use tao::window::{Window, WindowBuilder, WindowId};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu, accelerator::Accelerator},
    TrayIcon, TrayIconBuilder,
};
use wry::{WebView, WebViewBuilder};

use crate::config::{self, AccountBehavior, RawAccount, Settings};
use crate::progress::ProgressUpdate;
use crate::tray_actions::{self, action_open_config, ActionResult};
use crate::updater;

type CloseCb = Box<dyn FnOnce() + Send>;
type ActionCb = Box<dyn FnOnce() + Send>;

/// Commands routed through the main event loop's user-event channel.
pub enum AppCommand {
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

enum WState {
    Progress(ProgressState),
    Config(#[allow(dead_code)] ConfigState),
    Update(UpdateState),
}

/// Prevents duplicate config windows from opening simultaneously.
static CONFIG_WINDOW_OPEN: AtomicBool = AtomicBool::new(false);

/// Prevents duplicate update windows from opening simultaneously.
static UPDATE_WINDOW_OPEN: AtomicBool = AtomicBool::new(false);

static APP_PROXY: OnceLock<EventLoopProxy<AppCommand>> = OnceLock::new();

/// Send a command to the main event loop. Returns Err if the loop is not running yet.
pub fn send_command(cmd: AppCommand) -> Result<()> {
    APP_PROXY
        .get()
        .context("tray event loop not initialised")?
        .send_event(cmd)
        .map_err(|_| anyhow::anyhow!("tray event loop closed"))
}

/// Menu item identifiers.
mod menu_ids {
    pub const IMPORT_THUNDERBIRD: &str = "import_thunderbird";
    pub const CHOOSE_EXPORT_DIR: &str = "choose_export_dir";
    pub const CHOOSE_NOTES_DIR: &str = "choose_notes_dir";
    pub const OPEN_CONFIG: &str = "open_config";
    pub const OPEN_DOCUMENTATION: &str = "open_documentation";
    pub const UPDATE: &str = "update";
    pub const QUIT: &str = "quit";
    pub const EXPORT_PREFIX: &str = "export_";
    pub const FIXHTML_PREFIX: &str = "fixhtml_";
}

/// Run the system tray application.
pub fn run_tray() -> Result<()> {
    let event_loop = EventLoopBuilder::<AppCommand>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    APP_PROXY
        .set(proxy.clone())
        .map_err(|_| anyhow::anyhow!("APP_PROXY already initialised"))?;

    let (result_sender, result_receiver) = mpsc::channel::<ActionResult>();
    let menu_channel = MenuEvent::receiver();

    let mut tray_icon: Option<TrayIcon> = None;
    let mut windows: HashMap<WindowId, WState> = HashMap::new();

    event_loop.run(move |event, target, control_flow| {
        *control_flow = ControlFlow::Poll;

        if let Event::NewEvents(StartCause::Init) = event {
            match create_tray_icon() {
                Ok(icon) => {
                    tray_icon = Some(icon);
                    println!("Tray icon created successfully");
                }
                Err(e) => {
                    eprintln!("Failed to create tray icon: {}", e);
                }
            }
        }

        if let Ok(menu_event) = menu_channel.try_recv() {
            handle_menu_event(&menu_event.id.0, result_sender.clone());
        }

        if let Ok(result) = result_receiver.try_recv() {
            match &result {
                ActionResult::Imported(_) => {
                    if let Some(ref icon) = tray_icon {
                        match create_menu() {
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

        match event {
            Event::UserEvent(AppCommand::OpenProgress {
                action_name,
                warning,
                progress_rx,
                on_close,
                error_action,
                sender,
                cancel_token,
            }) => match build_progress_window(target, &proxy, &action_name, warning.as_deref(), cancel_token) {
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
                            let _ = bridge_proxy.send_event(AppCommand::ProgressUpdate {
                                window_id,
                                update,
                            });
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
            },
            Event::UserEvent(AppCommand::OpenConfig { sender }) => {
                if CONFIG_WINDOW_OPEN
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
                {
                    // Already open — ignore.
                } else {
                    match build_config_window(target, &proxy, sender.clone()) {
                        Ok((window, webview, window_id)) => {
                            windows.insert(window_id, WState::Config(ConfigState { webview, window }));
                        }
                        Err(e) => {
                            CONFIG_WINDOW_OPEN.store(false, Ordering::Release);
                            let _ = sender.send(ActionResult::Error(format!(
                                "Fenêtre de paramètres : {:#}",
                                e
                            )));
                        }
                    }
                }
            }
            Event::UserEvent(AppCommand::OpenUpdate) => {
                if UPDATE_WINDOW_OPEN
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
                {
                    // Already open — ignore.
                } else {
                    match build_update_window(target, &proxy) {
                        Ok((window, webview, window_id)) => {
                            windows.insert(window_id, WState::Update(UpdateState { webview, window }));
                        }
                        Err(e) => {
                            UPDATE_WINDOW_OPEN.store(false, Ordering::Release);
                            eprintln!("Fenêtre de mise à jour : {:#}", e);
                        }
                    }
                }
            }
            Event::UserEvent(AppCommand::UpdateMsg(msg)) => {
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
            Event::UserEvent(AppCommand::ProgressUpdate { window_id, update }) => {
                if let Some(WState::Progress(state)) = windows.get(&window_id) {
                    let js = format_progress_js(&update);
                    let _ = state.webview.evaluate_script(&js);
                }
            }
            Event::UserEvent(AppCommand::EvalScript { window_id, js: _ }) => {
                // M7: EvalScript will be used by the route review window.
                // No-op until M7 wires it up.
                let _ = window_id;
            }
            Event::UserEvent(AppCommand::ActionRequested { window_id }) => {
                if let Some(WState::Progress(mut state)) = windows.remove(&window_id) {
                    if let Some(f) = state.error_action.take() {
                        f();
                    }
                }
            }
            Event::UserEvent(AppCommand::CloseWindow { window_id }) => {
                match windows.remove(&window_id) {
                    Some(WState::Config(_)) => {
                        CONFIG_WINDOW_OPEN.store(false, Ordering::Release);
                    }
                    Some(WState::Update(_)) => {
                        UPDATE_WINDOW_OPEN.store(false, Ordering::Release);
                    }
                    _ => {}
                }
            }
            Event::WindowEvent {
                window_id,
                event: WindowEvent::CloseRequested,
                ..
            } => match windows.remove(&window_id) {
                Some(WState::Progress(mut state)) => {
                    if let Some(f) = state.on_close.take() {
                        f();
                    }
                }
                Some(WState::Config(_)) => {
                    CONFIG_WINDOW_OPEN.store(false, Ordering::Release);
                }
                Some(WState::Update(_)) => {
                    UPDATE_WINDOW_OPEN.store(false, Ordering::Release);
                }
                None => {}
            },
            _ => {}
        }

        let _ = &tray_icon;
    });
}

/// Build a progress window inline on the main event loop thread.
fn build_progress_window(
    target: &EventLoopWindowTarget<AppCommand>,
    proxy: &EventLoopProxy<AppCommand>,
    action_name: &str,
    warning: Option<&str>,
    cancel_token: Option<Arc<AtomicBool>>,
) -> Result<(Window, WebView, WindowId)> {
    let cancel_html = if cancel_token.is_some() {
        r#"<button id="cancel" onclick="window.ipc.postMessage('cancel')">Annuler</button>"#
    } else {
        ""
    };
    let warning_html = match warning {
        Some(msg) => format!(r#"<div class="warning-badge">⚠ {}</div>"#, msg),
        None => String::new(),
    };
    let html_template = include_str!("../assets/progress_window.html");
    let html = html_template
        .replace("__ACTION_NAME__", action_name)
        .replace("__WARNING__", &warning_html)
        .replace("__HAS_CANCEL__", cancel_html);

    let window = WindowBuilder::new()
        .with_title(format!("En cours — {}", action_name))
        .with_inner_size(LogicalSize::new(500.0f64, 220.0f64))
        .build(target)
        .context("failed to create progress window")?;
    window.set_focus();
    let window_id = window.id();

    let proxy_ipc = proxy.clone();
    let webview = WebViewBuilder::new(&window)
        .with_html(html)
        .with_ipc_handler(move |msg| {
            match msg.body().as_str() {
                "action" => { let _ = proxy_ipc.send_event(AppCommand::ActionRequested { window_id }); }
                "close" => { let _ = proxy_ipc.send_event(AppCommand::CloseWindow { window_id }); }
                "cancel" => {
                    if let Some(token) = &cancel_token {
                        token.store(true, Ordering::Relaxed);
                    }
                    let _ = proxy_ipc.send_event(AppCommand::CloseWindow { window_id });
                }
                _ => {}
            }
        })
        .build()
        .context("failed to create progress webview")?;

    Ok((window, webview, window_id))
}

// ── Config window ────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct ConfigIpcMessage {
    action: String,
    data: Option<serde_json::Value>,
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
    organize_by_type: Option<bool>,
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
    organize_by_type: Option<bool>,
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

/// Build a config window inline on the main event loop thread.
fn build_config_window(
    target: &EventLoopWindowTarget<AppCommand>,
    proxy: &EventLoopProxy<AppCommand>,
    sender: Sender<ActionResult>,
) -> Result<(Window, WebView, WindowId)> {
    let settings_path = config::settings_path();
    let settings = Settings::load(&settings_path).unwrap_or_default();
    let accounts_path = config::accounts_yaml_path();
    let raw_accounts = config::load_raw_accounts(&accounts_path).unwrap_or_default();

    let html_template = include_str!("../assets/config_window.html");
    let settings_json =
        serde_json::to_string(&settings).context("failed to serialize settings")?;
    let accounts_json =
        serde_json::to_string(&raw_accounts).context("failed to serialize accounts")?;
    let html = html_template
        .replace("__SETTINGS_JSON__", &settings_json)
        .replace("__ACCOUNTS_JSON__", &accounts_json);

    let window = WindowBuilder::new()
        .with_title("Email to Markdown \u{2014} Param\u{00e8}tres")
        .with_inner_size(LogicalSize::new(700.0f64, 500.0f64))
        .build(target)
        .context("failed to create config window")?;
    window.set_focus();
    let window_id = window.id();

    let proxy_ipc = proxy.clone();
    let webview = WebViewBuilder::new(&window)
        .with_html(html)
        .with_ipc_handler(move |req: wry::http::Request<String>| {
            let body = req.body().clone();
            let (result, should_close) = handle_config_ipc(&body);
            if let Some(r) = result {
                let _ = sender.send(r);
            }
            if should_close {
                let _ = proxy_ipc.send_event(AppCommand::CloseWindow { window_id });
            }
        })
        .build()
        .context("failed to create config webview")?;

    Ok((window, webview, window_id))
}

// ── Update window ─────────────────────────────────────────────────────────────

/// Build an update window inline on the main event loop thread.
fn build_update_window(
    target: &EventLoopWindowTarget<AppCommand>,
    proxy: &EventLoopProxy<AppCommand>,
) -> Result<(Window, WebView, WindowId)> {
    let html = include_str!("../assets/update_window.html");

    let window = WindowBuilder::new()
        .with_title("Email to Markdown \u{2014} Mise \u{00e0} jour")
        .with_inner_size(LogicalSize::new(700.0f64, 500.0f64))
        .build(target)
        .context("failed to create update window")?;
    window.set_focus();
    let window_id = window.id();

    let proxy_ipc = proxy.clone();
    let webview = WebViewBuilder::new(&window)
        .with_html(html)
        .with_ipc_handler(move |req: wry::http::Request<String>| {
            let body = req.body().clone();
            let parsed: serde_json::Value = match serde_json::from_str(&body) {
                Ok(v) => v,
                Err(_) => return,
            };
            if parsed.get("action").and_then(|v| v.as_str()) == Some("update-confirm") {
                if let Some(asset_url) = parsed.get("asset_url").and_then(|v| v.as_str()) {
                    let asset_url = asset_url.to_string();
                    let proxy_dl = proxy_ipc.clone();
                    thread::spawn(move || {
                        let result = updater::download_and_apply(&asset_url, |msg| {
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
                                    "type": "msg",
                                    "text": format!("Erreur : {:#}", e)
                                })
                                .to_string();
                                let _ = proxy_dl.send_event(AppCommand::UpdateMsg(json));
                            }
                        }
                    });
                }
            }
        })
        .build()
        .context("failed to create update webview")?;

    let proxy_check = proxy.clone();
    let current_version = env!("CARGO_PKG_VERSION").to_string();
    thread::spawn(move || {
        match updater::check_update(&current_version) {
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
                    "body": release.body,
                    "asset_url": release.asset_url
                })
                .to_string();
                let _ = proxy_check.send_event(AppCommand::UpdateMsg(json));
            }
            Err(e) => {
                let json = serde_json::json!({
                    "type": "msg",
                    "text": format!("Erreur : {:#}", e)
                })
                .to_string();
                let _ = proxy_check.send_event(AppCommand::UpdateMsg(json));
            }
        }
    });

    Ok((window, webview, window_id))
}

/// Parse a config IPC message and act on it synchronously.
///
/// Returns `(Option<ActionResult>, bool)` — the bool is `should_close`.
fn handle_config_ipc(body: &str) -> (Option<ActionResult>, bool) {
    let msg: ConfigIpcMessage = match serde_json::from_str(body) {
        Ok(m) => m,
        Err(e) => {
            return (
                Some(ActionResult::Error(format!("failed to parse IPC message: {}", e))),
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
                    organize_by_type: data.defaults.organize_by_type,
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
                    Some(ActionResult::Error(format!("Erreur de sauvegarde : {:#}", e))),
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

                let mut behavior =
                    settings.accounts.get(&canonical_key).cloned().unwrap_or_default();
                behavior.organize_by_type = data.organize_by_type;
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
                    Some(ActionResult::Error(format!("Erreur de sauvegarde : {:#}", e))),
                    false,
                ),
            }
        }
        "open_raw" => {
            if let Err(e) =
                action_open_config().context("failed to open settings file in editor")
            {
                return (Some(ActionResult::Error(format!("Erreur : {:#}", e))), false);
            }
            (None, false)
        }
        other => (
            Some(ActionResult::Error(format!("unknown IPC action '{}'", other))),
            true,
        ),
    }
}

/// Format a JS call for the progress webview.
fn format_progress_js(update: &ProgressUpdate) -> String {
    match update {
        ProgressUpdate::Step { current, total, message } => {
            format!("step({},{},{:?})", current, total, message)
        }
        ProgressUpdate::Indeterminate { message } => {
            format!("indeterminate({:?})", message)
        }
        ProgressUpdate::Done { summary } => {
            format!("finish({:?})", summary)
        }
        ProgressUpdate::Error { message, action_label } => {
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

/// Create the system tray icon with menu.
fn create_tray_icon() -> Result<TrayIcon> {
    let menu = create_menu()?;
    let icon = load_icon()?;
    let tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("Email to Markdown")
        .with_icon(icon)
        .build()
        .context("Failed to create tray icon")?;
    Ok(tray_icon)
}

/// Create the tray menu.
fn create_menu() -> Result<Menu> {
    let menu = Menu::new();

    let accounts = tray_actions::get_account_names().unwrap_or_default();
    let has_accounts = !accounts.is_empty();

    let no_accel: Option<Accelerator> = None;

    let export_submenu = Submenu::new("Export compte", has_accounts);
    for account in &accounts {
        let id = format!("{}{}", menu_ids::EXPORT_PREFIX, account);
        let _ = export_submenu.append(&MenuItem::with_id(
            id,
            account,
            true,
            no_accel.clone(),
        ));
    }
    menu.append(&export_submenu)?;

    let outils_submenu = Submenu::new("Outils", true);

    let fixhtml_submenu = Submenu::new("Fix HTML→Markdown", has_accounts);
    for account in &accounts {
        let id = format!("{}{}", menu_ids::FIXHTML_PREFIX, account);
        let _ = fixhtml_submenu.append(&MenuItem::with_id(
            id,
            account,
            true,
            no_accel.clone(),
        ));
    }
    let _ = outils_submenu.append(&fixhtml_submenu);

    let _ = outils_submenu.append(&PredefinedMenuItem::separator());

    let _ = outils_submenu.append(&MenuItem::with_id(
        menu_ids::IMPORT_THUNDERBIRD,
        "Import Thunderbird",
        true,
        no_accel.clone(),
    ));

    let _ = outils_submenu.append(&MenuItem::with_id(
        menu_ids::CHOOSE_EXPORT_DIR,
        "Choisir répertoire d'export…",
        true,
        no_accel.clone(),
    ));

    let _ = outils_submenu.append(&MenuItem::with_id(
        menu_ids::CHOOSE_NOTES_DIR,
        "Choisir répertoire de notes…",
        true,
        no_accel.clone(),
    ));

    let _ = outils_submenu.append(&MenuItem::with_id(
        menu_ids::OPEN_CONFIG,
        "Paramètres…",
        true,
        no_accel.clone(),
    ));

    let _ = outils_submenu.append(&PredefinedMenuItem::separator());

    let _ = outils_submenu.append(&MenuItem::with_id(
        menu_ids::UPDATE,
        "Mise à jour…",
        true,
        no_accel.clone(),
    ));

    let _ = outils_submenu.append(&MenuItem::with_id(
        menu_ids::OPEN_DOCUMENTATION,
        "Documentation",
        true,
        no_accel.clone(),
    ));

    menu.append(&outils_submenu)?;

    menu.append(&PredefinedMenuItem::separator())?;

    menu.append(&MenuItem::with_id(
        menu_ids::QUIT,
        "Quitter",
        true,
        no_accel,
    ))?;

    Ok(menu)
}

/// Handle menu item clicks.
fn handle_menu_event(id: &str, result_sender: mpsc::Sender<ActionResult>) {
    match id {
        menu_ids::IMPORT_THUNDERBIRD => {
            tray_actions::action_import_thunderbird(result_sender);
        }
        menu_ids::CHOOSE_EXPORT_DIR => {
            tray_actions::action_choose_export_dir(result_sender);
        }
        menu_ids::CHOOSE_NOTES_DIR => {
            tray_actions::action_choose_notes_dir(result_sender);
        }
        menu_ids::OPEN_CONFIG => {
            if let Err(e) = send_command(AppCommand::OpenConfig {
                sender: result_sender.clone(),
            }) {
                eprintln!("Failed to open config window: {:#}", e);
            }
        }
        menu_ids::UPDATE => {
            if let Err(e) = send_command(AppCommand::OpenUpdate) {
                eprintln!("Failed to open update window: {:#}", e);
            }
        }
        menu_ids::OPEN_DOCUMENTATION => {
            if let Err(e) = tray_actions::action_open_documentation() {
                let _ = result_sender.send(ActionResult::Error(format!(
                    "Failed to open documentation: {}",
                    e
                )));
            }
        }
        menu_ids::QUIT => {
            std::process::exit(0);
        }
        id if id.starts_with(menu_ids::EXPORT_PREFIX) => {
            if let Some(account_name) = id.strip_prefix(menu_ids::EXPORT_PREFIX) {
                tray_actions::action_export(account_name.to_string(), result_sender);
            }
        }
        id if id.starts_with(menu_ids::FIXHTML_PREFIX) => {
            if let Some(account_name) = id.strip_prefix(menu_ids::FIXHTML_PREFIX) {
                tray_actions::action_fix_html(account_name.to_string(), result_sender);
            }
        }
        _ => {}
    }
}

/// Load the tray icon.
fn load_icon() -> Result<tray_icon::Icon> {
    let icon_paths = [
        "assets/icon.ico",
        "assets/icon.png",
    ];

    for path in &icon_paths {
        if std::path::Path::new(path).exists() {
            if let Ok(icon) = load_icon_from_file(path) {
                return Ok(icon);
            }
        }
    }

    create_default_icon()
}

fn load_icon_from_file(path: &str) -> Result<tray_icon::Icon> {
    let img = image::open(path).context("Failed to load icon image")?;
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();

    tray_icon::Icon::from_rgba(rgba.into_raw(), width, height)
        .context("Failed to create icon from image")
}

fn create_default_icon() -> Result<tray_icon::Icon> {
    let size = 16u32;
    let mut rgba = vec![0u8; (size * size * 4) as usize];

    for chunk in rgba.chunks_exact_mut(4) {
        chunk.copy_from_slice(&[30u8, 136, 229, 255]);
    }

    let set = |buf: &mut Vec<u8>, x: u32, y: u32| {
        if x < size && y < size {
            let i = ((y * size + x) * 4) as usize;
            buf[i..i + 4].copy_from_slice(&[255u8, 255, 255, 255]);
        }
    };

    for x in 1u32..15 {
        set(&mut rgba, x, 2);
        set(&mut rgba, x, 13);
    }
    for y in 3u32..13 {
        set(&mut rgba, 1, y);
        set(&mut rgba, 14, y);
    }

    for i in 0u32..6 {
        set(&mut rgba, 2 + i, 3 + i);
        set(&mut rgba, 13 - i, 3 + i);
    }

    tray_icon::Icon::from_rgba(rgba, size, size).context("Failed to create default icon")
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

