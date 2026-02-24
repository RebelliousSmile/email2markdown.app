use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;
use thiserror::Error;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub name: String,
    pub server: String,
    pub port: u16,
    pub username: String,
    #[serde(skip_deserializing, default)]
    pub password: Option<String>,
    pub export_directory: String,
    #[serde(default)]
    pub ignored_folders: Vec<String>,
    #[serde(default = "default_quote_depth")]
    pub quote_depth: usize,
    #[serde(default = "default_true")]
    pub skip_existing: bool,
    #[serde(default)]
    pub collect_contacts: bool,
    #[serde(default)]
    pub skip_signature_images: bool,
    #[serde(default)]
    pub delete_after_export: bool,
}

fn default_quote_depth() -> usize {
    1
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub accounts: Vec<Account>,
}

impl Config {
    /// Load configuration from YAML file and inject passwords from environment.
    pub fn load(config_path: &Path) -> Result<Self, ConfigError> {
        if !config_path.exists() {
            return Ok(Config { accounts: vec![] });
        }
        let content = fs::read_to_string(config_path)?;
        let mut config: Config = serde_yaml::from_str(&content)?;

        // Inject passwords from environment
        for account in &mut config.accounts {
            let sanitized_name = account.name.to_uppercase().replace(['@', '.', '-'], "_");
            let app_password_var = format!("{}_APPLICATION_PASSWORD", sanitized_name);
            let password_var = format!("{}_PASSWORD", sanitized_name);

            account.password = env::var(&app_password_var)
                .ok()
                .or_else(|| env::var(&password_var).ok());
        }

        // [6] Validate configuration
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
                    "Export directory not configured for account '{}'",
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

/// Configuration for the email sorting tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SortConfig {
    #[serde(default = "default_delete_keywords")]
    pub delete_keywords: Vec<String>,
    #[serde(default)]
    pub delete_senders: Vec<String>,
    #[serde(default)]
    pub delete_subjects: Vec<String>,

    #[serde(default = "default_summarize_max_length")]
    pub summarize_max_length: usize,
    #[serde(default)]
    pub summarize_keywords: Vec<String>,

    #[serde(default = "default_keep_keywords")]
    pub keep_keywords: Vec<String>,
    #[serde(default)]
    pub keep_senders: Vec<String>,
    #[serde(default)]
    pub keep_subjects: Vec<String>,

    #[serde(default)]
    pub whitelist: Vec<String>,

    #[serde(default = "default_recent_threshold")]
    pub recent_threshold_days: i64,
    #[serde(default = "default_old_threshold")]
    pub old_threshold_days: i64,

    #[serde(default = "default_small_threshold")]
    pub small_email_threshold: usize,
    #[serde(default = "default_large_threshold")]
    pub large_email_threshold: usize,

    #[serde(default = "default_true")]
    pub keep_with_attachments: bool,

    #[serde(default = "default_type_weights")]
    pub type_weights: HashMap<String, i32>,
}

fn default_delete_keywords() -> Vec<String> {
    vec![
        "newsletter".into(),
        "bulletin".into(),
        "digest".into(),
        "promotion".into(),
        "offer".into(),
        "coupon".into(),
        "sale".into(),
        "unsubscribe".into(),
        "marketing".into(),
        "advertisement".into(),
    ]
}

fn default_keep_keywords() -> Vec<String> {
    vec![
        "contract".into(),
        "invoice".into(),
        "legal".into(),
        "urgent".into(),
        "important".into(),
        "confidential".into(),
    ]
}

fn default_summarize_max_length() -> usize {
    5000
}

fn default_recent_threshold() -> i64 {
    30
}

fn default_old_threshold() -> i64 {
    365
}

fn default_small_threshold() -> usize {
    500
}

fn default_large_threshold() -> usize {
    10000
}

fn default_type_weights() -> HashMap<String, i32> {
    let mut weights = HashMap::new();
    weights.insert("newsletter".into(), -2);
    weights.insert("mailing_list".into(), -1);
    weights.insert("group".into(), 0);
    weights.insert("direct".into(), 1);
    weights.insert("unknown".into(), 0);
    weights
}

impl Default for SortConfig {
    fn default() -> Self {
        SortConfig {
            delete_keywords: default_delete_keywords(),
            delete_senders: Vec::new(),
            delete_subjects: Vec::new(),
            summarize_max_length: default_summarize_max_length(),
            summarize_keywords: Vec::new(),
            keep_keywords: default_keep_keywords(),
            keep_senders: Vec::new(),
            keep_subjects: Vec::new(),
            whitelist: Vec::new(),
            recent_threshold_days: default_recent_threshold(),
            old_threshold_days: default_old_threshold(),
            small_email_threshold: default_small_threshold(),
            large_email_threshold: default_large_threshold(),
            keep_with_attachments: true,
            type_weights: default_type_weights(),
        }
    }
}

impl SortConfig {
    /// Load configuration from JSON file.
    pub fn load(config_path: &Path) -> Result<Self, ConfigError> {
        if config_path.exists() {
            let content = fs::read_to_string(config_path)?;
            let config: SortConfig = serde_json::from_str(&content)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            Ok(config)
        } else {
            Ok(Self::default())
        }
    }

    /// Save configuration to JSON file.
    pub fn save(&self, config_path: &Path) -> Result<(), std::io::Error> {
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        fs::write(config_path, content)
    }

    /// Check if a sender is whitelisted.
    pub fn is_whitelisted(&self, sender_email: &str) -> bool {
        if sender_email.is_empty() {
            return false;
        }

        let sender_lower = sender_email.to_lowercase();

        for entry in &self.whitelist {
            let entry_lower = entry.to_lowercase();
            // Exact email match
            if sender_lower == entry_lower {
                return true;
            }
            // Domain match (@company.com)
            if entry_lower.starts_with('@') && sender_lower.ends_with(&entry_lower) {
                return true;
            }
            // Prefix match (john@)
            if entry_lower.ends_with('@') && sender_lower.starts_with(&entry_lower) {
                return true;
            }
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sort_config_default() {
        let config = SortConfig::default();
        assert!(config.delete_keywords.contains(&"newsletter".to_string()));
        assert!(config.keep_keywords.contains(&"contract".to_string()));
        assert_eq!(config.recent_threshold_days, 30);
    }

    #[test]
    fn test_is_whitelisted() {
        let mut config = SortConfig::default();
        config.whitelist = vec![
            "important@client.com".into(),
            "@company.com".into(),
            "boss@".into(),
        ];

        assert!(config.is_whitelisted("important@client.com"));
        assert!(config.is_whitelisted("anyone@company.com"));
        assert!(config.is_whitelisted("boss@anywhere.com"));
        assert!(!config.is_whitelisted("random@other.com"));
    }
}
