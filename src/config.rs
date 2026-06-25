use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

// ── Platform-aware config paths ──────────────────────────────────────────────

/// Returns the app config directory, platform-appropriate:
/// - Windows : `%APPDATA%\email-to-markdown`
/// - macOS   : `~/Library/Application Support/email-to-markdown`
/// - Linux   : `~/.config/email-to-markdown`
///
/// Falls back to `./config` if the platform directory cannot be determined.
pub fn app_config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("email-to-markdown")
}

/// Path to `accounts.yaml`.
pub fn accounts_yaml_path() -> PathBuf {
    app_config_dir().join("accounts.yaml")
}

/// Path to `.env` (passwords).
pub fn env_file_path() -> PathBuf {
    app_config_dir().join(".env")
}

/// Canonical env var name prefix for an account.
///
/// Used by the loader to look up `{PREFIX}_PASSWORD` / `{PREFIX}_APPLICATION_PASSWORD`,
/// and by the template/extract-passwords writers to emit matching names.
/// Keep this as the single source of truth to avoid mismatches.
pub fn env_var_name(account_name: &str) -> String {
    account_name
        .to_uppercase()
        .replace([' ', '@', '.', '-'], "_")
}

/// Path to `settings.yaml` (app behaviour, export dirs).
pub fn settings_path() -> PathBuf {
    app_config_dir().join("settings.yaml")
}

// ── Settings (settings.yaml) ─────────────────────────────────────────────────

/// Per-account behaviour overrides stored in settings.yaml.
/// All fields are optional so unset values fall back to `Settings::defaults`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AccountBehavior {
    /// Override the subdirectory name used inside `export_base_dir`.
    /// Defaults to the account name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quote_depth: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_existing: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collect_contacts: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_signature_images: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delete_after_export: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cleanup_empty_dirs: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organize_by_type: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Settings {
    /// Root directory where all account sub-folders will be created.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub export_base_dir: Option<String>,

    /// Name of the local output folder used by sort-apply (default: `_local`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_folder_name: Option<String>,

    /// Root of the user's second-brain tree.
    /// Plain base path — joined with `(Perso|Pro)/<Category>/<Sub>/<Year>/<Month>`.
    /// NOT the tree of valid paths; that tree is defined by `destinations.txt`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes_dir: Option<String>,

    /// Path to `destinations.txt` (routing rules).
    /// Defaults to `<config_dir>/destinations.txt` when absent.
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destinations_file: Option<String>,

    /// Enable AI-assisted routing for emails that do not match any deterministic rule.
    /// Defaults to `false` — the app is fully functional without AI.
    #[serde(default)]
    pub ai_routing_enabled: bool,

    /// Minimum confidence score (0.0–1.0) required to accept an AI routing suggestion.
    /// Suggestions below this threshold fall back to the default path.
    #[serde(default = "default_ai_confidence_threshold")]
    pub ai_confidence_threshold: f32,

    /// Default behaviour applied to every account unless overridden.
    #[serde(default)]
    pub defaults: AccountBehavior,

    /// Per-account overrides keyed by account name.
    #[serde(default)]
    pub accounts: HashMap<String, AccountBehavior>,
}

impl Settings {
    /// Returns the configured local output folder name, defaulting to `"_local"`.
    pub fn local_folder(&self) -> &str {
        self.local_folder_name.as_deref().unwrap_or("_local")
    }

    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        if !path.exists() {
            return Ok(Settings::default());
        }
        let content = fs::read_to_string(path)?;
        Ok(serde_yaml::from_str(&content)?)
    }

    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_yaml::to_string(self)?)?;
        Ok(())
    }
}

// ── Raw accounts.yaml (connection info only) ─────────────────────────────────

/// A single account entry as stored in accounts.yaml.
/// Contains only connection details; behaviour comes from settings.yaml.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawAccount {
    pub name: String,
    pub server: String,
    pub port: u16,
    pub username: String,
    #[serde(default)]
    pub ignored_folders: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct RawAccountsFile {
    accounts: Vec<RawAccount>,
}

/// Load the raw account list from `path` without merging settings.
///
/// Returns an empty vec if the file does not exist.
pub fn load_raw_accounts(path: &Path) -> Result<Vec<RawAccount>, ConfigError> {
    if !path.exists() {
        return Ok(vec![]);
    }
    let content = fs::read_to_string(path)?;
    let raw_file: RawAccountsFile = serde_yaml::from_str(&content)?;
    Ok(raw_file.accounts)
}

/// Persist a slice of `RawAccount` values to `path` as `accounts.yaml`.
///
/// Creates parent directories if needed — same pattern as `Settings::save`.
pub fn save_accounts(accounts: &[RawAccount], path: &Path) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let raw_file = RawAccountsFile {
        accounts: accounts.to_vec(),
    };
    fs::write(path, serde_yaml::to_string(&raw_file)?)?;
    Ok(())
}

/// Merge a raw account with the app settings to produce a fully-resolved Account.
fn merge_account(raw: &RawAccount, settings: &Settings) -> Account {
    let per = settings.accounts.get(&raw.name);
    let def = &settings.defaults;

    let folder = per
        .and_then(|a| a.folder_name.as_deref())
        .unwrap_or(&raw.name);

    let export_directory = settings
        .export_base_dir
        .as_ref()
        .map(|base| PathBuf::from(base).join(folder).to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();

    Account {
        name: raw.name.clone(),
        server: raw.server.clone(),
        port: raw.port,
        username: raw.username.clone(),
        password: None,
        ignored_folders: raw.ignored_folders.clone(),
        export_directory,
        quote_depth: per.and_then(|a| a.quote_depth).or(def.quote_depth).unwrap_or(1),
        skip_existing: per.and_then(|a| a.skip_existing).or(def.skip_existing).unwrap_or(true),
        collect_contacts: per.and_then(|a| a.collect_contacts).or(def.collect_contacts).unwrap_or(false),
        skip_signature_images: per.and_then(|a| a.skip_signature_images).or(def.skip_signature_images).unwrap_or(false),
        delete_after_export: per.and_then(|a| a.delete_after_export).or(def.delete_after_export).unwrap_or(false),
        cleanup_empty_dirs: per.and_then(|a| a.cleanup_empty_dirs).or(def.cleanup_empty_dirs).unwrap_or(true),
        organize_by_type: per.and_then(|a| a.organize_by_type).or(def.organize_by_type).unwrap_or(true),
    }
}

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    FileReadError(#[from] std::io::Error),
    #[error("Failed to parse YAML: {0}")]
    YamlParseError(#[from] serde_yaml::Error),
    #[error("Account not found: {0}")]
    AccountNotFound(String),
    #[error("No password found for account: {0}")]
    NoPassword(String),
    #[error("Configuration validation error: {0}")]  // [6]
    ValidationError(String),
}

/// Fully-resolved account used by the exporter.
/// Populated by merging accounts.yaml + settings.yaml — never serialised back to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub name: String,
    pub server: String,
    pub port: u16,
    pub username: String,
    #[serde(skip)]
    pub password: Option<String>,
    /// Computed: `export_base_dir / folder_name`
    pub export_directory: String,
    #[serde(default)]
    pub ignored_folders: Vec<String>,
    pub quote_depth: usize,
    pub skip_existing: bool,
    pub collect_contacts: bool,
    pub skip_signature_images: bool,
    pub delete_after_export: bool,
    pub cleanup_empty_dirs: bool,
    pub organize_by_type: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub accounts: Vec<Account>,
}

impl Config {
    /// Load accounts from `accounts_path` and settings from the platform default path.
    pub fn load(accounts_path: &Path) -> Result<Self, ConfigError> {
        Self::load_with_settings(accounts_path, &settings_path())
    }

    /// Load and merge accounts.yaml + an explicit settings path (useful for tests).
    pub fn load_with_settings(
        accounts_path: &Path,
        settings_file: &Path,
    ) -> Result<Self, ConfigError> {
        if !accounts_path.exists() {
            return Ok(Config { accounts: vec![] });
        }

        let content = fs::read_to_string(accounts_path)?;
        let raw_file: RawAccountsFile = serde_yaml::from_str(&content)?;

        let settings = Settings::load(settings_file).unwrap_or_default();

        let mut accounts: Vec<Account> = raw_file
            .accounts
            .iter()
            .map(|raw| merge_account(raw, &settings))
            .collect();

        // Inject passwords from environment
        for account in &mut accounts {
            let sanitized = env_var_name(&account.name);
            account.password = env::var(format!("{}_APPLICATION_PASSWORD", sanitized))
                .ok()
                .or_else(|| env::var(format!("{}_PASSWORD", sanitized)).ok());
        }

        let config = Config { accounts };
        config.validate()?;
        Ok(config)
    }

    /// [6] Validate the configuration
    pub fn validate(&self) -> Result<(), ConfigError> {
        for account in &self.accounts {
            // Check required fields
            if account.name.is_empty() {
                return Err(ConfigError::ValidationError(
                    "Account name cannot be empty".into(),
                ));
            }
            if account.server.is_empty() {
                return Err(ConfigError::ValidationError(format!(
                    "Server not configured for account '{}'",
                    account.name
                )));
            }
            if account.username.is_empty() {
                return Err(ConfigError::ValidationError(format!(
                    "Username not configured for account '{}'",
                    account.name
                )));
            }
            if account.export_directory.is_empty() {
                return Err(ConfigError::ValidationError(format!(
                    "Export directory not configured for account '{}'. \
                     Set 'export_base_dir' in settings.yaml or via the tray.",
                    account.name
                )));
            }

            // Validate port
            if account.port == 0 {
                return Err(ConfigError::ValidationError(format!(
                    "Invalid port (0) for account '{}'",
                    account.name
                )));
            }
        }

        Ok(())
    }

    /// Get account by name (case-insensitive).
    pub fn get_account(&self, name: &str) -> Option<&Account> {
        self.accounts
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(name))
    }

    /// List all account names.
    pub fn list_accounts(&self) -> Vec<&str> {
        self.accounts.iter().map(|a| a.name.as_str()).collect()
    }
}

fn default_ai_confidence_threshold() -> f32 {
    0.7
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_env_var_name_gmail() {
        assert_eq!(
            env_var_name("fx.rebellious.smile@gmail.com"),
            "FX_REBELLIOUS_SMILE_GMAIL_COM"
        );
    }

    #[test]
    fn test_env_var_name_dash_domain() {
        assert_eq!(
            env_var_name("compta@cabinet-partage.fr"),
            "COMPTA_CABINET_PARTAGE_FR"
        );
    }

    #[test]
    fn test_env_var_name_with_space() {
        assert_eq!(env_var_name("My Work Account"), "MY_WORK_ACCOUNT");
    }

    #[test]
    fn test_env_var_name_mixed_punctuation() {
        assert_eq!(
            env_var_name("first.last-tag@host.example.com"),
            "FIRST_LAST_TAG_HOST_EXAMPLE_COM"
        );
    }

    #[test]
    fn test_cleanup_empty_dirs_default_true() {
        let temp = tempfile::TempDir::new().unwrap();
        let accounts_path = temp.path().join("accounts.yaml");
        let settings_path = temp.path().join("settings.yaml");
        std::fs::write(
            &accounts_path,
            "accounts:\n  - name: Test\n    server: imap.example.com\n    port: 993\n    username: test@example.com\n",
        )
        .unwrap();
        std::fs::write(
            &settings_path,
            "export_base_dir: /tmp/exports\n",
        )
        .unwrap();

        let config = Config::load_with_settings(&accounts_path, &settings_path).unwrap();
        assert!(config.accounts[0].cleanup_empty_dirs);
    }

    #[test]
    fn test_cleanup_empty_dirs_defaults_false() {
        let temp = tempfile::TempDir::new().unwrap();
        let accounts_path = temp.path().join("accounts.yaml");
        let settings_path = temp.path().join("settings.yaml");
        std::fs::write(
            &accounts_path,
            "accounts:\n  - name: Test\n    server: imap.example.com\n    port: 993\n    username: test@example.com\n",
        )
        .unwrap();
        std::fs::write(
            &settings_path,
            "export_base_dir: /tmp/exports\ndefaults:\n  cleanup_empty_dirs: false\n",
        )
        .unwrap();

        let config = Config::load_with_settings(&accounts_path, &settings_path).unwrap();
        assert!(!config.accounts[0].cleanup_empty_dirs);
    }

    #[test]
    fn test_cleanup_empty_dirs_per_account_override() {
        let temp = tempfile::TempDir::new().unwrap();
        let accounts_path = temp.path().join("accounts.yaml");
        let settings_path = temp.path().join("settings.yaml");
        std::fs::write(
            &accounts_path,
            "accounts:\n  - name: Test\n    server: imap.example.com\n    port: 993\n    username: test@example.com\n",
        )
        .unwrap();
        std::fs::write(
            &settings_path,
            "export_base_dir: /tmp/exports\ndefaults:\n  cleanup_empty_dirs: false\naccounts:\n  Test:\n    cleanup_empty_dirs: true\n",
        )
        .unwrap();

        let config = Config::load_with_settings(&accounts_path, &settings_path).unwrap();
        assert!(config.accounts[0].cleanup_empty_dirs);
    }

}
