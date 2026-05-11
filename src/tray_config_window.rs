//! WebView settings window for editing settings.yaml and accounts.yaml.
//!
//! Opens a wry WebView in a dedicated thread so the user can view and
//! modify application settings without leaving the tray.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use anyhow::Context;
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    platform::run_return::EventLoopExtRunReturn,
    window::WindowBuilder,
};
use wry::WebViewBuilder;

use crate::config::{self, AccountBehavior, RawAccount, Settings};
use crate::tray_actions::{action_open_config, ActionResult};

/// Guard that prevents duplicate config windows from opening simultaneously.
static CONFIG_WINDOW_OPEN: AtomicBool = AtomicBool::new(false);

/// IPC payload sent from the config window.
#[derive(serde::Deserialize)]
struct IpcMessage {
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

/// Account data sent from the "save_account" IPC action.
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

/// Open the settings window.
///
/// This function returns immediately; the window runs in a new OS thread.
/// Only one config window may be open at a time; duplicate calls are silently ignored.
pub fn open(sender: Sender<ActionResult>) {
    if CONFIG_WINDOW_OPEN.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
        // Window is already open — do nothing.
        return;
    }

    std::thread::spawn(move || {
        if let Err(e) = run_window(sender.clone()) {
            let _ = sender.send(ActionResult::Error(format!("Fenêtre de paramètres : {}", e)));
        }
        CONFIG_WINDOW_OPEN.store(false, Ordering::Release);
    });
}

fn run_window(sender: Sender<ActionResult>) -> anyhow::Result<()> {
    // Load current settings (falls back to default if file is absent).
    let settings_path = config::settings_path();
    let settings = Settings::load(&settings_path).unwrap_or_default();

    // Load current raw accounts (falls back to empty list if file is absent).
    let accounts_path = config::accounts_yaml_path();
    let raw_accounts =
        config::load_raw_accounts(&accounts_path).unwrap_or_default();

    // Inject settings and accounts JSON into the HTML template.
    let html_template = include_str!("../assets/config_window.html");
    let settings_json =
        serde_json::to_string(&settings).context("failed to serialize settings")?;
    let accounts_json =
        serde_json::to_string(&raw_accounts).context("failed to serialize accounts")?;
    let html = html_template
        .replace("__SETTINGS_JSON__", &settings_json)
        .replace("__ACCOUNTS_JSON__", &accounts_json);

    // Shared flag: IPC handler signals "save succeeded" → window should close.
    let should_exit = Arc::new(AtomicBool::new(false));
    let should_exit_ipc = Arc::clone(&should_exit);

    let sender_ipc = sender.clone();

    let mut event_loop = {
        let mut builder = EventLoopBuilder::<()>::new();
        #[cfg(target_os = "windows")]
        {
            use tao::platform::windows::EventLoopBuilderExtWindows;
            builder.with_any_thread(true);
        }
        builder.build()
    };

    let window = WindowBuilder::new()
        .with_title("Email to Markdown \u{2014} Param\u{00e8}tres")
        .with_inner_size(LogicalSize::new(700.0f64, 500.0f64))
        .build(&event_loop)
        .context("failed to create config window")?;
    window.set_focus();

    let _webview = WebViewBuilder::new(&window)
        .with_html(html)
        .with_ipc_handler(move |req: wry::http::Request<String>| {
            let body = req.body().clone();
            let (result, should_close) = handle_ipc_message(&body);
            if let Some(r) = result {
                let _ = sender_ipc.send(r);
            }
            if should_close {
                should_exit_ipc.store(true, Ordering::Release);
            }
        })
        .build()
        .context("failed to create webview")?;

    event_loop.run_return(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                *control_flow = ControlFlow::ExitWithCode(0);
            }
            Event::MainEventsCleared => {
                if should_exit.load(Ordering::Acquire) {
                    *control_flow = ControlFlow::ExitWithCode(0);
                }
            }
            _ => {}
        }
    });

    Ok(())
}

/// Parse the IPC JSON and act on it.
///
/// Returns `(Option<ActionResult>, bool)` where the bool is `should_close`.
/// - `save`: success → `(Some(Success), true)`; I/O error → `(Some(Error), true)`
/// - `save_account`: success → `(None, false)`; I/O error → `(Some(Error), false)`
/// - `open_raw`: `(None, false)`
/// - unknown action: `(Some(Error), true)`
fn handle_ipc_message(body: &str) -> (Option<ActionResult>, bool) {
    let msg: IpcMessage = match serde_json::from_str(body) {
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
                let data: SettingsData = serde_json::from_value(raw_data)
                    .context("failed to parse settings data")?;

                // Preserve per-account overrides that are not exposed in the GUI.
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
                    sort: settings.defaults.sort,
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
                Err(e) => (Some(ActionResult::Error(format!("Erreur de sauvegarde : {}", e))), true),
            }
        }
        "save_account" => {
            let result = (|| -> anyhow::Result<()> {
                let raw_data = msg
                    .data
                    .ok_or_else(|| anyhow::anyhow!("save_account action missing data field"))?;
                let data: AccountData = serde_json::from_value(raw_data)
                    .context("failed to parse account data")?;

                let accounts_path = config::accounts_yaml_path();
                let mut accounts =
                    config::load_raw_accounts(&accounts_path).unwrap_or_default();

                // Find and update the matching account by name (case-insensitive).
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

                config::save_accounts(&accounts, &accounts_path)
                    .with_context(|| {
                        format!("failed to save accounts to {}", accounts_path.display())
                    })?;

                let settings_path = config::settings_path();
                let mut settings = Settings::load(&settings_path).unwrap_or_default();

                // Find the canonical key using case-insensitive matching to avoid duplicate entries.
                let canonical_key = settings
                    .accounts
                    .keys()
                    .find(|k| k.eq_ignore_ascii_case(&data.account_name))
                    .cloned()
                    .unwrap_or_else(|| data.account_name.clone());

                let mut behavior = settings.accounts.get(&canonical_key).cloned().unwrap_or_default();

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
                // Window stays open — the JS side handles the UI transition.
                Ok(()) => (None, false),
                // I/O error — send notification but keep window open so user can retry.
                Err(e) => (Some(ActionResult::Error(format!("Erreur de sauvegarde : {}", e))), false),
            }
        }
        "open_raw" => {
            if let Err(e) = action_open_config().context("failed to open settings file in editor") {
                return (Some(ActionResult::Error(format!("Erreur : {}", e))), false);
            }
            (None, false)
        }
        other => (
            Some(ActionResult::Error(format!("unknown IPC action '{}'", other))),
            true,
        ),
    }
}
