//! Ledger of messages the user already sorted or discarded.
//!
//! An account that does not delete from the server keeps every message there.
//! Once a note leaves the export folder (sorted elsewhere, or discarded in the
//! review window) nothing local says it was handled, so the next export would
//! propose it again. The ledger remembers the `Message-ID` of each handled
//! message so `skip_existing` can recognise it.

use anyhow::{Context, Result};
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Ledger file next to `accounts.yaml`.
pub fn ledger_path() -> PathBuf {
    crate::config::app_config_dir().join("handled_messages.txt")
}

fn clean(value: &str) -> bool {
    !value.is_empty() && !value.contains(['\t', '\r', '\n'])
}

/// Message-IDs recorded for `account` in the ledger at `path`.
/// A missing or unreadable ledger yields an empty set.
pub fn load_from(path: &Path, account: &str) -> HashSet<String> {
    let Ok(content) = fs::read_to_string(path) else {
        return HashSet::new();
    };
    content
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .filter(|(name, _)| name.eq_ignore_ascii_case(account))
        .map(|(_, id)| id.to_string())
        .collect()
}

/// Append `(account, message_id)` pairs to the ledger at `path`.
pub fn record_to(path: &Path, entries: &[(String, String)]) -> Result<()> {
    let lines: String = entries
        .iter()
        .filter(|(account, id)| clean(account) && clean(id))
        .map(|(account, id)| format!("{account}\t{id}\n"))
        .collect();
    if lines.is_empty() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("failed to create ledger directory")?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .context("failed to open handled messages ledger")?;
    file.write_all(lines.as_bytes())
        .context("failed to write handled messages ledger")
}

/// Message-IDs handled so far for `account`, from the default ledger.
pub fn load(account: &str) -> HashSet<String> {
    load_from(&ledger_path(), account)
}

/// Remember the messages behind these proofs as handled. Best effort: a
/// ledger failure must never block sorting, so it is only logged.
pub fn record_proofs<'a>(
    proofs: impl IntoIterator<Item = &'a crate::contextual_export::LocalSourceProof>,
) {
    let entries: Vec<(String, String)> = proofs
        .into_iter()
        .filter_map(|proof| {
            proof
                .identity
                .message_id
                .clone()
                .map(|id| (proof.identity.account.clone(), id))
        })
        .collect();
    if let Err(error) = record_to(&ledger_path(), &entries) {
        eprintln!("warning: handled messages ledger not updated: {error:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_load_from_missing_ledger_is_empty() {
        let temp = TempDir::new().unwrap();
        assert!(load_from(&temp.path().join("absent.txt"), "a").is_empty());
    }

    #[test]
    fn test_record_to_then_load_from_filters_by_account() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("handled.txt");
        record_to(
            &path,
            &[
                ("Perso".into(), "<a@x>".into()),
                ("Pro".into(), "<b@x>".into()),
            ],
        )
        .unwrap();
        record_to(&path, &[("perso".into(), "<c@x>".into())]).unwrap();

        let perso = load_from(&path, "Perso");
        assert_eq!(perso.len(), 2);
        assert!(perso.contains("<a@x>") && perso.contains("<c@x>"));
        assert!(!perso.contains("<b@x>"));
    }

    #[test]
    fn test_record_to_ignores_entries_with_separators() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("handled.txt");
        record_to(
            &path,
            &[
                ("Perso".into(), "a\tb".into()),
                ("Perso".into(), String::new()),
            ],
        )
        .unwrap();
        assert!(!path.exists());
    }
}
