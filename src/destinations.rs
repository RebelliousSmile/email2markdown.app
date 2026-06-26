//! YAML storage for routing destinations.
//!
//! Replaces the legacy line-based `destinations.txt` format. The file is
//! tool-owned: entries carry an optional `note` field instead of free-form
//! comments. The legacy parser (`route::parse_destinations`) is kept solely to
//! feed `migrate_from_txt`.
//!
//! Serialized shape (external tagging on `DestinationRule` → one-key maps):
//!
//! ```yaml
//! destinations:
//!   - path: Perso/Banque
//!     note: relevés et factures bancaires
//!     rules:
//!       - domain: ubs.ch
//!       - subject: facture
//!   - path: Perso/Messy/Emails
//!     default: true
//! ```

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// A match condition on an email. Serde external tagging (the default) plus
/// `rename_all = "lowercase"` emits each variant as a one-key map, e.g.
/// `{domain: "ubs.ch"}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DestinationRule {
    Domain(String),
    From(String),
    Subject(String),
    Account(String),
}

/// A single routing destination: a relative path under `notes_dir`, optional
/// human note, optional match rules, and an optional `default` flag.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DestinationEntry {
    /// Relative path under `notes_dir` (e.g. `Perso/Finance/Banque`).
    pub path: String,
    /// Free human description (replaces legacy free-form comments).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Rules that trigger routing here. Empty = bare classification option.
    ///
    /// `singleton_map_recursive` renders each externally-tagged variant as a
    /// one-key map (`- domain: ubs.ch`) instead of serde_yaml's default YAML-tag
    /// syntax (`- !domain ubs.ch`).
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        with = "serde_yaml::with::singleton_map_recursive"
    )]
    pub rules: Vec<DestinationRule>,
    /// `true` if this is the fallback destination.
    #[serde(default, skip_serializing_if = "is_false")]
    pub default: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Top-level `destinations.yaml` document.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DestinationsConfig {
    #[serde(default)]
    pub destinations: Vec<DestinationEntry>,
}

// ── Conversions to/from the router's `MatchRule` ──────────────────────────────

impl From<crate::route::MatchRule> for DestinationRule {
    fn from(m: crate::route::MatchRule) -> Self {
        use crate::route::MatchRule;
        match m {
            // Lowercase the domain on write — parity with the legacy `upsert_rule`,
            // and `route_email` compares domains case-insensitively anyway.
            MatchRule::Domain(d) => DestinationRule::Domain(d.to_lowercase()),
            MatchRule::From(a) => DestinationRule::From(a),
            MatchRule::Subject(k) => DestinationRule::Subject(k),
            MatchRule::Account(n) => DestinationRule::Account(n),
        }
    }
}

impl From<&DestinationRule> for crate::route::MatchRule {
    fn from(d: &DestinationRule) -> Self {
        use crate::route::MatchRule;
        match d {
            DestinationRule::Domain(d) => MatchRule::Domain(d.clone()),
            DestinationRule::From(a) => MatchRule::From(a.clone()),
            DestinationRule::Subject(k) => MatchRule::Subject(k.clone()),
            DestinationRule::Account(n) => MatchRule::Account(n.clone()),
        }
    }
}

// ── Load / save ───────────────────────────────────────────────────────────────

/// Load `destinations.yaml`. A missing file yields an empty config.
///
/// # Errors
/// Returns `Err` if `path` is a symlink (anti-symlink guard), or on I/O / parse failure.
pub fn load_yaml(path: &Path) -> Result<DestinationsConfig> {
    if !path.exists() {
        return Ok(DestinationsConfig::default());
    }
    reject_symlink(path)?;
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    if content.trim().is_empty() {
        return Ok(DestinationsConfig::default());
    }
    serde_yaml::from_str(&content)
        .with_context(|| format!("failed to parse destinations YAML at {}", path.display()))
}

/// Serialize and write `config` to `path`, creating parent directories.
///
/// # Errors
/// Returns `Err` if `path` is a symlink (anti-symlink guard), or on I/O failure.
pub fn save_yaml(path: &Path, config: &DestinationsConfig) -> Result<()> {
    reject_symlink(path)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let yaml = serde_yaml::to_string(config).context("failed to serialize destinations")?;
    fs::write(path, yaml).with_context(|| format!("failed to write {}", path.display()))
}

/// Refuse to touch a symlink (rule 02-rust-filesystem-safety). No-op if absent.
fn reject_symlink(path: &Path) -> Result<()> {
    if path.exists() {
        let meta = path
            .symlink_metadata()
            .with_context(|| format!("failed to stat {}", path.display()))?;
        if meta.file_type().is_symlink() {
            anyhow::bail!("refusing to access symlink: {}", path.display());
        }
    }
    Ok(())
}

// ── Upsert ────────────────────────────────────────────────────────────────────

/// Add or update a destination entry by `path` (case-insensitive match).
///
/// - Existing entry → append each rule not already present (dedup).
/// - No entry → push a new one carrying `rules` (empty slice = bare path option).
pub fn upsert_entry(config: &mut DestinationsConfig, path: &str, rules: &[DestinationRule]) {
    if let Some(entry) = config
        .destinations
        .iter_mut()
        .find(|e| e.path.eq_ignore_ascii_case(path))
    {
        for rule in rules {
            if !entry.rules.contains(rule) {
                entry.rules.push(rule.clone());
            }
        }
    } else {
        config.destinations.push(DestinationEntry {
            path: path.to_string(),
            note: None,
            rules: rules.to_vec(),
            default: false,
        });
    }
}

// ── Mutators (pure, for the interactive editor) ───────────────────────────────

/// Remove the entry whose `path` matches (case-insensitive). Returns `true` if one was removed.
pub fn remove_entry(config: &mut DestinationsConfig, path: &str) -> bool {
    let before = config.destinations.len();
    config
        .destinations
        .retain(|e| !e.path.eq_ignore_ascii_case(path));
    config.destinations.len() != before
}

/// Mark the matching entry as the default and clear `default` on every other entry.
///
/// Returns `true` if the path existed (and is now the sole default).
pub fn set_default(config: &mut DestinationsConfig, path: &str) -> bool {
    if !config
        .destinations
        .iter()
        .any(|e| e.path.eq_ignore_ascii_case(path))
    {
        return false;
    }
    for entry in &mut config.destinations {
        entry.default = entry.path.eq_ignore_ascii_case(path);
    }
    true
}

/// Set (or clear, with `None`) an entry's note. Returns `true` if the path existed.
pub fn set_note(config: &mut DestinationsConfig, path: &str, note: Option<String>) -> bool {
    if let Some(entry) = config
        .destinations
        .iter_mut()
        .find(|e| e.path.eq_ignore_ascii_case(path))
    {
        entry.note = note;
        true
    } else {
        false
    }
}

/// Drop a matching rule from an entry. Returns `true` if a rule was removed.
pub fn remove_rule(config: &mut DestinationsConfig, path: &str, rule: &DestinationRule) -> bool {
    if let Some(entry) = config
        .destinations
        .iter_mut()
        .find(|e| e.path.eq_ignore_ascii_case(path))
    {
        let before = entry.rules.len();
        entry.rules.retain(|r| r != rule);
        entry.rules.len() != before
    } else {
        false
    }
}

/// Add `rule` to the entry at `path` if it exists and the rule is not already present.
///
/// Returns `true` if the rule was inserted.
pub fn add_rule(config: &mut DestinationsConfig, path: &str, rule: DestinationRule) -> bool {
    if let Some(entry) = config
        .destinations
        .iter_mut()
        .find(|e| e.path.eq_ignore_ascii_case(path))
    {
        if !entry.rules.contains(&rule) {
            entry.rules.push(rule);
            return true;
        }
    }
    false
}

/// Reorder destinations to match `order` (slice of paths).
///
/// Entries whose path appears in `order` are moved to that position; any
/// remaining entries (not mentioned in `order`) are appended at the end in
/// their original relative order.
pub fn reorder_destinations(config: &mut DestinationsConfig, order: &[&str]) {
    let mut reordered = Vec::with_capacity(config.destinations.len());
    for path in order {
        if let Some(pos) = config
            .destinations
            .iter()
            .position(|e| e.path.eq_ignore_ascii_case(path))
        {
            reordered.push(config.destinations.swap_remove(pos));
        }
    }
    reordered.extend(config.destinations.drain(..));
    config.destinations = reordered;
}

// ── Migration ─────────────────────────────────────────────────────────────────

/// One-shot migration: parse a legacy `destinations.txt` and write the YAML form.
///
/// Uses `route::parse_destinations` (the only remaining `.txt` reader) to keep a
/// single source of truth for the legacy grammar.
pub fn migrate_from_txt(txt_path: &Path, yaml_path: &Path) -> Result<()> {
    let content = fs::read_to_string(txt_path)
        .with_context(|| format!("failed to read {}", txt_path.display()))?;
    let dests = crate::route::parse_destinations(&content)
        .with_context(|| format!("failed to parse legacy {}", txt_path.display()))?;

    let config = DestinationsConfig {
        destinations: dests
            .into_iter()
            .map(|d| DestinationEntry {
                path: d.path,
                note: None,
                rules: d.rules.into_iter().map(DestinationRule::from).collect(),
                default: d.is_default,
            })
            .collect(),
    };

    save_yaml(yaml_path, &config)?;
    eprintln!(
        "notice: migrated {} → {} (legacy file left in place)",
        txt_path.display(),
        yaml_path.display()
    );
    Ok(())
}
