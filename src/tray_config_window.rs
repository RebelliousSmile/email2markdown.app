//! WebView settings window for editing settings.yaml.
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

use crate::config::{self, AccountBehavior, Settings};
use crate::tray_actions::{action_open_config, ActionResult};

/// Guard that prevents duplicate config windows from opening simultaneously.
static CONFIG_WINDOW_OPEN: AtomicBool = AtomicBool::new(false);

/// IPC payload sent from the config window.
#[derive(serde::Deserialize)]
struct IpcMessage {
    action: String,
    data: Option<SettingsData>,
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

    // Inject settings JSON into the HTML template.
    let html_template = include_str!("../assets/config_window.html");
    let settings_json =
        serde_json::to_string(&settings).context("failed to serialize settings")?;
    let html = html_template.replace("__SETTINGS_JSON__", &settings_json);

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
            match handle_ipc_message(&body) {
                Ok(Some(result)) => {
                    let _ = sender_ipc.send(result);
                    should_exit_ipc.store(true, Ordering::Release);
                }
                Ok(None) => {
                    // "open_raw" action — window stays open.
                }
                Err(e) => {
                    let _ = sender_ipc
                        .send(ActionResult::Error(format!("Erreur de sauvegarde : {}", e)));
                    should_exit_ipc.store(true, Ordering::Release);
                }
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
/// Returns `Ok(Some(result))` when the window should close (save completed),
/// `Ok(None)` for actions that keep the window open (open_raw),
/// and `Err` on I/O or parse failures.
fn handle_ipc_message(body: &str) -> anyhow::Result<Option<ActionResult>> {
    let msg: IpcMessage =
        serde_json::from_str(body).context("failed to parse IPC message")?;

    match msg.action.as_str() {
        "save" => {
            let data = msg
                .data
                .ok_or_else(|| anyhow::anyhow!("save action missing data field"))?;

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

            Ok(Some(ActionResult::Success(
                "Param\u{00e8}tres".to_string(),
                "Param\u{00e8}tres sauvegard\u{00e9}s".to_string(),
            )))
        }
        "open_raw" => {
            action_open_config()
                .context("failed to open settings file in editor")?;
            Ok(None)
        }
        other => Err(anyhow::anyhow!("unknown IPC action '{}'", other)),
    }
}
