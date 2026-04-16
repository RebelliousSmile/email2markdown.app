//! WebView review window for sort results.
//!
//! Opens a wry WebView in a dedicated thread so the user can inspect and
//! modify sort decisions before they are applied.

use std::path::PathBuf;
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

use crate::sort_emails::{apply_report, EmailSummary, SortReport};
use crate::tray_actions::ActionResult;

/// IPC payload sent from the review window when the user clicks "Appliquer".
#[derive(Debug, serde::Deserialize)]
struct IpcDecisions {
    decisions: Vec<IpcDecision>,
}

#[derive(Debug, serde::Deserialize)]
struct IpcDecision {
    file: String,
    action: String,
}

/// Open a review window for the given sort report.
///
/// This function blocks the calling thread until the window is closed.
/// All errors are sent as `ActionResult::Error` through `sender`.
pub fn open(report_path: PathBuf, account: String, sender: Sender<ActionResult>) {
    if let Err(e) = run_window(report_path, account, sender.clone()) {
        let _ = sender.send(ActionResult::Error(format!("Fenêtre de révision : {}", e)));
    }
}

fn run_window(report_path: PathBuf, account: String, sender: Sender<ActionResult>) -> anyhow::Result<()> {
    // Load the sort report.
    let json = std::fs::read_to_string(&report_path)
        .with_context(|| format!("failed to read report: {}", report_path.display()))?;
    let report: SortReport = serde_json::from_str(&json).context("failed to parse sort report")?;

    // Inject report data into the HTML template.
    let html_template = include_str!("../assets/sort_review.html");
    let report_json =
        serde_json::to_string(&report.categories).context("failed to serialize report")?;
    let html = html_template.replace("__REPORT_JSON__", &report_json);

    // Shared flag: IPC handler signals "exit after apply" to the event loop.
    let should_exit = Arc::new(AtomicBool::new(false));
    let should_exit_ipc = Arc::clone(&should_exit);

    // Capture values for the IPC closure.
    let sender_ipc = sender.clone();
    let report_path_ipc = report_path.clone();
    let account_ipc = account.clone();

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
        .with_title(format!("Révision du tri — {}", account))
        .with_inner_size(LogicalSize::new(900.0f64, 620.0f64))
        .build(&event_loop)
        .context("failed to create review window")?;

    let _webview = WebViewBuilder::new(&window)
        .with_html(html)
        .with_ipc_handler(move |req: wry::http::Request<String>| {
            let body = req.body().clone();
            match handle_ipc_message(&body, &report_path_ipc, &account_ipc) {
                Ok(result) => {
                    let _ = sender_ipc.send(result);
                }
                Err(e) => {
                    let _ = sender_ipc
                        .send(ActionResult::Error(format!("Erreur d'application : {}", e)));
                }
            }
            should_exit_ipc.store(true, Ordering::Release);
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

/// Parse the IPC JSON, update the report, write it back, and apply it.
fn handle_ipc_message(body: &str, report_path: &PathBuf, account: &str) -> anyhow::Result<ActionResult> {
    let payload: IpcDecisions =
        serde_json::from_str(body).context("failed to parse IPC decisions")?;

    // Rebuild the categories map from the decisions.
    let mut new_categories: std::collections::HashMap<String, Vec<EmailSummary>> =
        std::collections::HashMap::new();
    new_categories.insert("delete".to_string(), Vec::new());
    new_categories.insert("summarize".to_string(), Vec::new());
    new_categories.insert("keep".to_string(), Vec::new());

    // Load current report to preserve email metadata.
    let json = std::fs::read_to_string(report_path)
        .context("failed to re-read report for apply")?;
    let mut report: SortReport =
        serde_json::from_str(&json).context("failed to re-parse report for apply")?;

    // Build a flat index of file -> EmailSummary from all current categories.
    let all_emails: std::collections::HashMap<String, EmailSummary> = report
        .categories
        .values()
        .flatten()
        .map(|e| (e.file.clone(), e.clone()))
        .collect();

    // Distribute according to the decisions received from the UI.
    // Reject unknown action values to prevent corrupt categories.
    for decision in &payload.decisions {
        match decision.action.as_str() {
            "delete" | "summarize" | "keep" => {}
            other => {
                return Err(anyhow::anyhow!(
                    "unknown action '{}' in IPC decision for file '{}'",
                    other,
                    decision.file
                ));
            }
        }
        if let Some(email) = all_emails.get(&decision.file) {
            let target = new_categories
                .entry(decision.action.clone())
                .or_default();
            target.push(email.clone());
        }
    }

    report.categories = new_categories;

    // Persist the updated report.
    let updated_json =
        serde_json::to_string_pretty(&report).context("failed to serialize updated report")?;
    std::fs::write(report_path, updated_json)
        .with_context(|| format!("failed to write updated report to {}", report_path.display()))?;

    // Apply the decisions.
    let stats = apply_report(&report).context("failed to apply sort report")?;

    Ok(ActionResult::Success(
        format!("Tri appliqué — {}", account),
        format!(
            "Supprimés : {} | Résumés : {} | Conservés : {}",
            stats.deleted, stats.moved, stats.skipped
        ),
    ))
}
