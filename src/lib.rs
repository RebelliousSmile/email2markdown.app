pub mod email_export;
pub mod fix_yaml;
pub mod sort_emails;
pub mod config;
pub mod utils;
pub mod cleaner;      // Email body cleaner pipeline
pub mod thunderbird;  // [1] Import automatique depuis Thunderbird
pub mod network;      // [3][4] Progress indicator et retry logic

// System tray modules (only available with the "tray" feature)
#[cfg(feature = "tray")]
pub mod tray;
#[cfg(feature = "tray")]
pub mod tray_actions;
