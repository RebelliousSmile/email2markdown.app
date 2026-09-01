pub mod cleaner; // Email body cleaner pipeline
pub mod config;
pub mod contextual_export;
pub mod dest_cmd; // `dest` CLI subcommand (list, add, suggest)
pub mod destinations; // YAML storage for routing destinations
pub mod email_export;
pub mod network; // [3][4] Progress indicator et retry logic
pub mod route;
pub mod shell_integration;
pub mod thunderbird; // [1] Import automatique depuis Thunderbird
#[cfg(feature = "tray")]
pub mod updater;
pub mod utils; // Auto-update: GitHub release check and binary replacement

// System tray modules (only available with the "tray" feature)
#[cfg(feature = "tray")]
pub mod progress;
#[cfg(feature = "tray")]
pub mod tray;
#[cfg(feature = "tray")]
pub mod tray_actions;
