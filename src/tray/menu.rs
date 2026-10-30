//! Tray icon and context menu: construction and click dispatch.

use std::sync::mpsc;

use anyhow::{Context, Result};
use tray_icon::{
    menu::{accelerator::Accelerator, Menu, MenuItem, PredefinedMenuItem, Submenu},
    TrayIcon, TrayIconBuilder,
};

use crate::tray_actions::{self, ActionResult};

use super::{send_command, AppCommand};

/// Menu item identifiers.
mod menu_ids {
    pub const IMPORT_THUNDERBIRD: &str = "import_thunderbird";
    pub const CHOOSE_EXPORT_DIR: &str = "choose_export_dir";
    pub const CHOOSE_NOTES_DIR: &str = "choose_notes_dir";
    pub const MANAGE_DESTINATIONS: &str = "manage_destinations";
    pub const OPEN_CONFIG: &str = "open_config";
    pub const OPEN_DOCUMENTATION: &str = "open_documentation";
    pub const UPDATE: &str = "update";
    pub const QUIT: &str = "quit";
    pub const EXPORT_PREFIX: &str = "export_";
    pub const FIXHTML_PREFIX: &str = "fixhtml_";
    pub const RESUME_SORT_PREFIX: &str = "resume_sort_";
    pub const CLEAN_SERVER_PREFIX: &str = "clean_server_";
}

/// Create the system tray icon with menu.
pub(super) fn create_tray_icon() -> Result<TrayIcon> {
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
pub(super) fn create_menu() -> Result<Menu> {
    let menu = Menu::new();

    let accounts = tray_actions::get_account_names().unwrap_or_default();
    let has_accounts = !accounts.is_empty();

    let no_accel: Option<Accelerator> = None;

    let export_submenu = Submenu::new("Export compte", has_accounts);
    for account in &accounts {
        let id = format!("{}{}", menu_ids::EXPORT_PREFIX, account);
        if let Err(e) = export_submenu.append(&MenuItem::with_id(id, account, true, no_accel)) {
            eprintln!("Menu export compte: {:#}", e);
        }
    }
    menu.append(&export_submenu)?;

    // "Reprendre le tri" — re-open the route review for emails left in staging
    // (e.g. when a previous review was cancelled). One entry per account.
    let resume_submenu = Submenu::new("Reprendre le tri", has_accounts);
    for account in &accounts {
        let id = format!("{}{}", menu_ids::RESUME_SORT_PREFIX, account);
        if let Err(e) = resume_submenu.append(&MenuItem::with_id(id, account, true, no_accel)) {
            eprintln!("Menu reprendre le tri: {:#}", e);
        }
    }
    menu.append(&resume_submenu)?;

    // "Nettoyer le serveur" — delete handled messages recorded without proof,
    // after confirmation. Only for accounts that delete from the server.
    let cleanup_accounts = tray_actions::get_cleanup_account_names().unwrap_or_default();
    if !cleanup_accounts.is_empty() {
        let clean_submenu = Submenu::new("Nettoyer le serveur", true);
        for account in &cleanup_accounts {
            let id = format!("{}{}", menu_ids::CLEAN_SERVER_PREFIX, account);
            if let Err(e) = clean_submenu.append(&MenuItem::with_id(id, account, true, no_accel)) {
                eprintln!("Menu nettoyer le serveur: {:#}", e);
            }
        }
        menu.append(&clean_submenu)?;
    }

    let outils_submenu = Submenu::new("Outils", true);

    let fixhtml_submenu = Submenu::new("Fix HTML→Markdown", has_accounts);
    for account in &accounts {
        let id = format!("{}{}", menu_ids::FIXHTML_PREFIX, account);
        if let Err(e) = fixhtml_submenu.append(&MenuItem::with_id(id, account, true, no_accel)) {
            eprintln!("Menu Fix HTML→Markdown: {:#}", e);
        }
    }
    if let Err(e) = outils_submenu.append(&fixhtml_submenu) {
        eprintln!("Menu Outils (Fix HTML→Markdown): {:#}", e);
    }

    if let Err(e) = outils_submenu.append(&PredefinedMenuItem::separator()) {
        eprintln!("Menu Outils (séparateur): {:#}", e);
    }

    if let Err(e) = outils_submenu.append(&MenuItem::with_id(
        menu_ids::IMPORT_THUNDERBIRD,
        "Import Thunderbird",
        true,
        no_accel,
    )) {
        eprintln!("Menu Outils (Import Thunderbird): {:#}", e);
    }

    if let Err(e) = outils_submenu.append(&MenuItem::with_id(
        menu_ids::CHOOSE_EXPORT_DIR,
        "Choisir répertoire d'export…",
        true,
        no_accel,
    )) {
        eprintln!("Menu Outils (Choisir répertoire d'export): {:#}", e);
    }

    if let Err(e) = outils_submenu.append(&MenuItem::with_id(
        menu_ids::CHOOSE_NOTES_DIR,
        "Choisir répertoire de notes…",
        true,
        no_accel,
    )) {
        eprintln!("Menu Outils (Choisir répertoire de notes): {:#}", e);
    }

    if let Err(e) = outils_submenu.append(&MenuItem::with_id(
        menu_ids::MANAGE_DESTINATIONS,
        "Gérer les destinations…",
        true,
        no_accel,
    )) {
        eprintln!("Menu Outils (Gérer les destinations): {:#}", e);
    }

    if let Err(e) = outils_submenu.append(&MenuItem::with_id(
        menu_ids::OPEN_CONFIG,
        "Paramètres…",
        true,
        no_accel,
    )) {
        eprintln!("Menu Outils (Paramètres): {:#}", e);
    }

    if let Err(e) = outils_submenu.append(&PredefinedMenuItem::separator()) {
        eprintln!("Menu Outils (séparateur): {:#}", e);
    }

    if let Err(e) = outils_submenu.append(&MenuItem::with_id(
        menu_ids::UPDATE,
        "Mise à jour…",
        true,
        no_accel,
    )) {
        eprintln!("Menu Outils (Mise à jour): {:#}", e);
    }

    if let Err(e) = outils_submenu.append(&MenuItem::with_id(
        menu_ids::OPEN_DOCUMENTATION,
        "Documentation",
        true,
        no_accel,
    )) {
        eprintln!("Menu Outils (Documentation): {:#}", e);
    }

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
pub(super) fn handle_menu_event(id: &str, result_sender: mpsc::Sender<ActionResult>) {
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
        menu_ids::MANAGE_DESTINATIONS => {
            let opened = crate::route::destinations_path()
                .and_then(|dest_file| send_command(AppCommand::OpenDestGui { dest_file }));
            if let Err(e) = opened {
                let _ = result_sender.send(ActionResult::Error(format!(
                    "Ouverture des destinations impossible : {e:#}"
                )));
            }
        }
        menu_ids::OPEN_CONFIG => {
            if let Err(e) = send_command(AppCommand::OpenConfig {
                sender: result_sender.clone(),
            }) {
                eprintln!("Ouverture des paramètres impossible : {:#}", e);
            }
        }
        menu_ids::UPDATE => {
            if let Err(e) = send_command(AppCommand::OpenUpdate) {
                eprintln!("Ouverture de la mise à jour impossible : {:#}", e);
            }
        }
        menu_ids::OPEN_DOCUMENTATION => {
            if let Err(e) = tray_actions::action_open_documentation() {
                let _ = result_sender.send(ActionResult::Error(format!(
                    "Ouverture de la documentation impossible : {:#}",
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
        id if id.starts_with(menu_ids::RESUME_SORT_PREFIX) => {
            if let Some(account_name) = id.strip_prefix(menu_ids::RESUME_SORT_PREFIX) {
                tray_actions::action_resume_sort(account_name.to_string(), result_sender);
            }
        }
        id if id.starts_with(menu_ids::CLEAN_SERVER_PREFIX) => {
            if let Some(account_name) = id.strip_prefix(menu_ids::CLEAN_SERVER_PREFIX) {
                tray_actions::action_clean_server(account_name.to_string(), result_sender);
            }
        }
        _ => {}
    }
}

/// Load the tray icon.
fn load_icon() -> Result<tray_icon::Icon> {
    let size = crate::app_icon::WINDOWS_ICON_SIZE;
    tray_icon::Icon::from_rgba(crate::app_icon::rgba(size), size, size)
        .context("Failed to create application icon")
}
