pub mod email_export;
pub mod fix_yaml;
pub mod route;
pub mod sort_emails;
pub mod config;
pub mod utils;
pub mod cleaner;      // Email body cleaner pipeline
pub mod thunderbird;  // [1] Import automatique depuis Thunderbird
pub mod network;      // [3][4] Progress indicator et retry logic
pub mod summarize;    // Python subprocess wrappers (summarize, classify, group, apply_template)
pub mod notes_review; // Walk .md notes + frontmatter for the organize window
#[cfg(feature = "tray")]
pub mod updater;      // Auto-update: GitHub release check and binary replacement

// System tray modules (only available with the "tray" feature)
#[cfg(feature = "tray")]
pub mod tray;
#[cfg(feature = "tray")]
pub mod tray_actions;
#[cfg(feature = "tray")]
pub mod progress;
