//! Persistent log of server deletion outcomes.
//!
//! Deletions run in background threads of the tray, where `eprintln!` reaches
//! nobody. Every outcome is appended to `deletions.log` next to
//! `accounts.yaml`, so a message left on the server can be explained.

use crate::contextual_export::{DeletionOutcome, MessageDeletionResult};
use anyhow::{Context, Result};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Log file next to `accounts.yaml`.
pub fn log_path() -> PathBuf {
    crate::config::app_config_dir().join("deletions.log")
}

fn outcome_label(outcome: &DeletionOutcome) -> String {
    match outcome {
        DeletionOutcome::Deleted => "deleted".into(),
        DeletionOutcome::AlreadyAbsent => "already-absent".into(),
        DeletionOutcome::RetryRequired(reason) => format!("retry-required: {reason}"),
        DeletionOutcome::StaleUidValidity { expected, actual } => format!(
            "stale-uidvalidity: expected {expected}, found {}",
            actual.map_or_else(|| "none".to_string(), |v| v.to_string())
        ),
        DeletionOutcome::Rejected(reason) => format!("rejected: {reason}"),
    }
}

/// One tab-separated line: time, account, folder, UID, Message-ID, outcome.
fn result_line(time: &str, result: &MessageDeletionResult) -> String {
    let proof = &result.proof;
    format!(
        "{time}\t{}\t{}\t{}\t{}\t{}\n",
        proof.identity.account,
        proof.location.folder_display,
        proof.location.uid,
        proof.identity.message_id.as_deref().unwrap_or("-"),
        outcome_label(&result.outcome).replace(['\t', '\r', '\n'], " ")
    )
}

fn append_to(path: &Path, lines: &str) -> Result<()> {
    if lines.is_empty() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("failed to create deletion log directory")?;
    }
    fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .context("failed to open deletion log")?
        .write_all(lines.as_bytes())
        .context("failed to write deletion log")
}

fn now() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

/// Append one line per deletion result to the log at `path`.
pub fn append_results_to(path: &Path, results: &[MessageDeletionResult]) -> Result<()> {
    let time = now();
    let lines: String = results.iter().map(|r| result_line(&time, r)).collect();
    append_to(path, &lines)
}

/// Append an error that prevented a whole batch for `account`.
pub fn append_error_to(path: &Path, account: &str, error: &str) -> Result<()> {
    let error = error.replace(['\t', '\r', '\n'], " ");
    append_to(
        path,
        &format!("{}\t{account}\t-\t-\t-\terror: {error}\n", now()),
    )
}

/// Best effort: a log failure never blocks a deletion, it is only reported.
pub fn record_results(results: &[MessageDeletionResult]) {
    if let Err(error) = append_results_to(&log_path(), results) {
        eprintln!("warning: deletion log not updated: {error:#}");
    }
}

/// Best effort: a log failure never blocks a deletion, it is only reported.
pub fn record_error(account: &str, error: &str) {
    if let Err(log_error) = append_error_to(&log_path(), account, error) {
        eprintln!("warning: deletion log not updated: {log_error:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contextual_export::{LocalSourceProof, MessageLocation, SourceIdentity};
    use tempfile::TempDir;

    fn result(uid: u32, outcome: DeletionOutcome) -> MessageDeletionResult {
        MessageDeletionResult {
            proof: LocalSourceProof {
                identity: SourceIdentity {
                    account: "Perso".into(),
                    message_id: Some(format!("<{uid}@x>")),
                    header_fingerprint: "fp".into(),
                    provider_id: None,
                },
                location: MessageLocation {
                    folder_raw: "INBOX".into(),
                    folder_display: "INBOX".into(),
                    uid_validity: 7,
                    uid,
                },
            },
            outcome,
        }
    }

    #[test]
    fn test_append_results_writes_one_line_per_result() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("deletions.log");
        append_results_to(
            &path,
            &[
                result(1, DeletionOutcome::Deleted),
                result(2, DeletionOutcome::RetryRequired("NO\tbusy\nlater".into())),
            ],
        )
        .unwrap();
        let content = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].ends_with("\tPerso\tINBOX\t1\t<1@x>\tdeleted"));
        assert!(lines[1].ends_with("\tPerso\tINBOX\t2\t<2@x>\tretry-required: NO busy later"));
        assert!(!content.contains("NO\tbusy"));
    }

    #[test]
    fn test_append_error_appends_after_results() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("deletions.log");
        append_results_to(&path, &[result(1, DeletionOutcome::Deleted)]).unwrap();
        append_error_to(&path, "Perso", "connection refused").unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content.lines().count(), 2);
        assert!(content
            .lines()
            .nth(1)
            .unwrap()
            .ends_with("\tPerso\t-\t-\t-\terror: connection refused"));
    }

    #[test]
    fn test_append_to_unwritable_path_is_an_error_not_a_panic() {
        let temp = TempDir::new().unwrap();
        // A directory where the log file should be: opening it fails.
        let path = temp.path().join("deletions.log");
        fs::create_dir(&path).unwrap();
        assert!(append_results_to(&path, &[result(1, DeletionOutcome::Deleted)]).is_err());
    }
}
