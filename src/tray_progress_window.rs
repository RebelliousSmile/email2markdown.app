use std::sync::mpsc::Receiver;
use std::thread;

use anyhow::Context;
use tao::{
    dpi::LogicalSize,
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder},
    platform::run_return::EventLoopExtRunReturn,
    window::WindowBuilder,
};
use wry::WebViewBuilder;

use crate::progress::ProgressUpdate;

pub fn open(
    action_name: &str,
    progress_rx: Receiver<ProgressUpdate>,
    on_close: Option<Box<dyn FnOnce() + Send>>,
) {
    if let Err(e) = run_window(action_name, progress_rx, on_close) {
        eprintln!("Progress window error: {}", e);
    }
}

fn run_window(
    action_name: &str,
    progress_rx: Receiver<ProgressUpdate>,
    on_close: Option<Box<dyn FnOnce() + Send>>,
) -> anyhow::Result<()> {
    let html_template = include_str!("../assets/progress_window.html");
    let html = html_template.replace("__ACTION_NAME__", action_name);

    let mut event_loop = {
        let mut builder = EventLoopBuilder::<ProgressUpdate>::with_user_event();
        #[cfg(target_os = "windows")]
        {
            use tao::platform::windows::EventLoopBuilderExtWindows;
            builder.with_any_thread(true);
        }
        builder.build()
    };

    let proxy = event_loop.create_proxy();
    thread::spawn(move || {
        for update in progress_rx {
            let terminal = matches!(update, ProgressUpdate::Done { .. } | ProgressUpdate::Error { .. });
            let _ = proxy.send_event(update);
            if terminal {
                break;
            }
        }
    });

    let window = WindowBuilder::new()
        .with_title(format!("En cours — {}", action_name))
        .with_inner_size(LogicalSize::new(500.0f64, 220.0f64))
        .build(&event_loop)
        .context("failed to create progress window")?;
    window.set_focus();

    let webview = WebViewBuilder::new(&window)
        .with_html(html)
        .build()
        .context("failed to create progress webview")?;

    let mut on_close = on_close;
    event_loop.run_return(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(update) => {
                let js = match &update {
                    ProgressUpdate::Step { current, total, message } => {
                        format!("step({},{},{:?})", current, total, message)
                    }
                    ProgressUpdate::Indeterminate { message } => {
                        format!("indeterminate({:?})", message)
                    }
                    ProgressUpdate::Done { summary } => {
                        format!("finish({:?})", summary)
                    }
                    ProgressUpdate::Error { message } => {
                        format!("error({:?})", message)
                    }
                };
                let _ = webview.evaluate_script(&js);
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                if let Some(f) = on_close.take() {
                    f();
                }
                *control_flow = ControlFlow::ExitWithCode(0);
            }
            _ => {}
        }
    });

    Ok(())
}
