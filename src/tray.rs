//! System tray module for Email to Markdown.
//!
//! This module provides a system tray icon with a context menu
//! for easy access to common operations without using the CLI.

use std::sync::mpsc;
use std::thread;

use anyhow::{Context, Result};
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu, accelerator::Accelerator},
    TrayIcon, TrayIconBuilder,
};

use crate::tray_actions::{
    self, ActionResult,
};

/// Menu item identifiers.
mod menu_ids {
    pub const IMPORT_THUNDERBIRD: &str = "import_thunderbird";
    pub const CHOOSE_EXPORT_DIR: &str = "choose_export_dir";
    pub const OPEN_CONFIG: &str = "open_config";
    pub const OPEN_DOCUMENTATION: &str = "open_documentation";
    pub const QUIT: &str = "quit";
    pub const EXPORT_PREFIX: &str = "export_";
    pub const SORT_PREFIX: &str = "sort_";
    pub const FIXYAML_PREFIX: &str = "fixyaml_";
    pub const FIXHTML_PREFIX: &str = "fixhtml_";
}

/// Run the system tray application.
pub fn run_tray() -> Result<()> {
    // Create event loop
    let event_loop = EventLoopBuilder::new().build();

    // Channel for receiving action results
    let (result_sender, result_receiver) = mpsc::channel::<ActionResult>();

    // Menu event receiver
    let menu_channel = MenuEvent::receiver();

    // Tray icon must be created after event loop on some platforms
    let mut tray_icon: Option<TrayIcon> = None;

    // Run the event loop
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Poll;

        match event {
            Event::NewEvents(StartCause::Init) => {
                // Create tray icon on init
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
            _ => {}
        }

        // Handle menu events
        if let Ok(event) = menu_channel.try_recv() {
            handle_menu_event(&event.id.0, result_sender.clone());
        }

        // Handle action results (notifications)
        if let Ok(result) = result_receiver.try_recv() {
            match &result {
                crate::tray_actions::ActionResult::Imported(_) => {
                    // Rebuild menu so Export/Sort submenus reflect new accounts
                    if let Some(ref icon) = tray_icon {
                        match create_menu() {
                            Ok(new_menu) => icon.set_menu(Some(Box::new(new_menu))),
                            Err(e) => eprintln!("Failed to rebuild menu: {}", e),
                        }
                    }
                    show_notification(&result);
                }
                crate::tray_actions::ActionResult::SortCompleted { report_path, account } => {
                    let total = count_report_emails(report_path);
                    if total == 0 {
                        let nothing = crate::tray_actions::ActionResult::Success(
                            "Tri terminé".to_string(),
                            "Rien à réviser".to_string(),
                        );
                        show_notification(&nothing);
                    } else {
                        let sender = result_sender.clone();
                        let path = report_path.clone();
                        let account = account.clone();
                        thread::spawn(move || {
                            crate::tray_sort_window::open(path, account, sender);
                        });
                    }
                }
                _ => {
                    show_notification(&result);
                }
            }
        }

        // Keep the tray icon alive
        let _ = &tray_icon;
    });
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

    // Get account names for submenus
    let accounts = tray_actions::get_account_names().unwrap_or_default();
    let has_accounts = !accounts.is_empty();

    let no_accel: Option<Accelerator> = None;

    // Export submenu — disabled until at least one account is configured
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

    // Sort submenu — disabled until at least one account is configured
    let sort_submenu = Submenu::new("Trier emails", has_accounts);
    for account in &accounts {
        let id = format!("{}{}", menu_ids::SORT_PREFIX, account);
        let _ = sort_submenu.append(&MenuItem::with_id(
            id,
            account,
            true,
            no_accel.clone(),
        ));
    }
    menu.append(&sort_submenu)?;

    // Outils submenu
    let outils_submenu = Submenu::new("Outils", true);

    // Fix YAML submenu — one entry per account
    let fixyaml_submenu = Submenu::new("Fix YAML", has_accounts);
    for account in &accounts {
        let id = format!("{}{}", menu_ids::FIXYAML_PREFIX, account);
        let _ = fixyaml_submenu.append(&MenuItem::with_id(
            id,
            account,
            true,
            no_accel.clone(),
        ));
    }
    let _ = outils_submenu.append(&fixyaml_submenu);

    // Fix HTML→Markdown submenu — one entry per account
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
        menu_ids::OPEN_CONFIG,
        "Paramètres…",
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

    // Separator
    menu.append(&PredefinedMenuItem::separator())?;

    // Quit
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
        menu_ids::OPEN_CONFIG => {
            if let Err(e) = tray_actions::action_open_config() {
                let _ = result_sender.send(ActionResult::Error(format!(
                    "Failed to open config: {}",
                    e
                )));
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
        id if id.starts_with(menu_ids::SORT_PREFIX) => {
            if let Some(account_name) = id.strip_prefix(menu_ids::SORT_PREFIX) {
                tray_actions::action_sort(account_name.to_string(), result_sender);
            }
        }
        id if id.starts_with(menu_ids::FIXYAML_PREFIX) => {
            if let Some(account_name) = id.strip_prefix(menu_ids::FIXYAML_PREFIX) {
                tray_actions::action_fix_yaml(account_name.to_string(), result_sender);
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
    // Try to load from file first
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

    // Fall back to embedded icon
    create_default_icon()
}

/// Load icon from a file.
fn load_icon_from_file(path: &str) -> Result<tray_icon::Icon> {
    let img = image::open(path).context("Failed to load icon image")?;
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();

    tray_icon::Icon::from_rgba(rgba.into_raw(), width, height)
        .context("Failed to create icon from image")
}

/// Create a default envelope icon (16x16).
fn create_default_icon() -> Result<tray_icon::Icon> {
    let size = 16u32;
    let mut rgba = vec![0u8; (size * size * 4) as usize];

    // Blue background
    for chunk in rgba.chunks_exact_mut(4) {
        chunk.copy_from_slice(&[30u8, 136, 229, 255]);
    }

    let set = |buf: &mut Vec<u8>, x: u32, y: u32| {
        if x < size && y < size {
            let i = ((y * size + x) * 4) as usize;
            buf[i..i + 4].copy_from_slice(&[255u8, 255, 255, 255]);
        }
    };

    // Envelope rectangle border: (1,2) to (14,13)
    for x in 1u32..15 {
        set(&mut rgba, x, 2);
        set(&mut rgba, x, 13);
    }
    for y in 3u32..13 {
        set(&mut rgba, 1, y);
        set(&mut rgba, 14, y);
    }

    // Flap: V shape from top corners down to centre
    for i in 0u32..6 {
        set(&mut rgba, 2 + i, 3 + i);   // left diagonal
        set(&mut rgba, 13 - i, 3 + i);  // right diagonal
    }

    tray_icon::Icon::from_rgba(rgba, size, size).context("Failed to create default icon")
}

/// Count the total emails across all categories in a sort report.
fn count_report_emails(report_path: &std::path::Path) -> usize {
    let Ok(json) = std::fs::read_to_string(report_path) else {
        return 0;
    };
    let Ok(report) = serde_json::from_str::<crate::sort_emails::SortReport>(&json) else {
        return 0;
    };
    report.categories.values().map(|v| v.len()).sum()
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
        ActionResult::SortCompleted { .. } => {
            // SortCompleted is handled by the event loop before reaching show_notification
            unreachable!("SortCompleted should never be forwarded to show_notification")
        }
    };

    thread::spawn(move || {
        rfd::MessageDialog::new()
            .set_title(&title)
            .set_description(&description)
            .set_level(level)
            .show();
    });
}
