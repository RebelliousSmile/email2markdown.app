pub mod app_icon;
pub mod calendar; // Calendar invitation (VEVENT) summary for notes
pub mod cleaner; // Email body cleaner pipeline
pub mod config;
pub mod contextual_export;
pub mod date_layout; // Date suffix of note folders (YYYY, YYYY/MM, …)
pub mod deletion_log; // Persistent log of server deletion outcomes
pub mod dest_cmd; // `dest` CLI subcommand (list, add, suggest)
pub mod destinations; // YAML storage for routing destinations
pub mod email_block; // ```email block of a note: header + body as TOML
pub mod email_export;
pub mod handled; // Ledger of sorted/discarded messages, so they are not re-exported
pub mod html_layout; // Email HTML handlers for the htmd converter
pub mod network; // [3][4] Progress indicator et retry logic
pub mod quote_link; // Replace a quoted message by a wikilink to its note
pub mod route;
pub mod server_audit; // Read-only census of messages still on the server
pub mod server_cleanup; // "Nettoyer le serveur": guard, plan, deletion, recording
pub mod thunderbird; // [1] Import automatique depuis Thunderbird
#[cfg(feature = "tray")]
pub mod updater; // Auto-update: GitHub release check and binary replacement
pub mod utils;

// System tray modules (only available with the "tray" feature)
#[cfg(feature = "tray")]
pub mod progress;
#[cfg(feature = "tray")]
pub mod tray;
#[cfg(feature = "tray")]
pub mod tray_actions;
