//! "Nettoyer le serveur": delete the handled messages the ledger records
//! without proof, after an explicit confirmation.
//!
//! The flow is split so that everything but the two IMAP steps (census,
//! deletion) is testable without a server: the account guard, the cleanup
//! plan and the recording of the results. Every path is injected through
//! `AppPaths`, so tests never write under the user's config directory.

use crate::config::{Account, AppPaths, Config, Settings};
use crate::contextual_export::{DeletionOutcome, LocalSourceProof, MessageDeletionResult};
use crate::email_export::ImapExporter;
use crate::server_audit::{AuditEntry, ServerStatus};
use anyhow::{Context, Result};
use std::path::Path;

/// The account to clean, refused before any connection when it does not
/// delete from the server or has no password.
pub fn cleanup_account(config: &Config, account_name: &str, env: &Path) -> Result<Account> {
    let account = config
        .get_account(account_name)
        .context(format!("Compte « {account_name} » introuvable"))?;
    if !account.delete_after_export {
        anyhow::bail!("{account_name} ne supprime pas les messages du serveur");
    }
    if account.password.is_none() {
        anyhow::bail!(
            "Aucun mot de passe pour {}, vérifier {}",
            account_name,
            env.display()
        );
    }
    Ok(account.clone())
}

/// What the census allows to delete.
#[derive(Debug)]
pub struct CleanupPlan {
    /// One proof per legacy handled message that can be targeted exactly.
    pub targets: Vec<LocalSourceProof>,
    /// Legacy handled messages without a proof: left on the server.
    pub untargetable: usize,
}

/// Cleanup plan from the census entries. A Gmail message seen under several
/// labels is one target, not several.
pub fn plan(entries: &[AuditEntry]) -> CleanupPlan {
    CleanupPlan {
        targets: crate::server_audit::legacy_proofs(entries),
        untargetable: entries
            .iter()
            .filter(|e| e.status == ServerStatus::HandledLegacy)
            .filter(|e| e.proof.is_none())
            .count(),
    }
}

/// Suffix appended to every summary: proofs completed from the notes and
/// messages left on the server.
pub fn notes_suffix(completed: usize, untargetable: usize) -> String {
    let mut notes = Vec::new();
    if completed > 0 {
        notes.push(format!(
            "{completed} preuve(s) retrouvée(s) dans les notes, purgées au prochain export"
        ));
    }
    if untargetable > 0 {
        notes.push(format!(
            "{untargetable} message(s) impossibles à cibler exactement, laissés sur le serveur"
        ));
    }
    notes.iter().map(|note| format!(" — {note}")).collect()
}

/// Log every result, record in the ledger the messages now gone from the
/// server (`Deleted`, `AlreadyAbsent`), and summarise.
pub fn record_results(paths: &AppPaths, results: &[MessageDeletionResult], notes: &str) -> String {
    crate::deletion_log::record_results_to(&paths.deletion_log, results);
    let gone: Vec<_> = results
        .iter()
        .filter(|r| {
            matches!(
                r.outcome,
                DeletionOutcome::Deleted | DeletionOutcome::AlreadyAbsent
            )
        })
        .map(|r| &r.proof)
        .collect();
    crate::handled::record_proofs_to(&paths.ledger, gone.iter().copied());
    let deleted = results
        .iter()
        .filter(|r| r.outcome == DeletionOutcome::Deleted)
        .count();
    let failed = results.len() - gone.len();
    format!(
        "{deleted} supprimé(s), {} déjà absent(s), {failed} échec(s){notes}",
        gone.len() - deleted
    )
}

/// Complete the ledger from the routed notes, list the legacy handled
/// messages still on the server, and delete them once `confirm` agrees;
/// `confirm` receives the account name, the message count and whether the
/// server is Gmail.
/// Successful deletions are recorded in the ledger with their proof.
pub fn clean_server_in(
    paths: &AppPaths,
    account_name: &str,
    confirm: impl Fn(&str, usize, bool) -> bool,
) -> Result<String> {
    dotenvy::from_path(&paths.env).ok();
    let config = Config::load_with_settings(&paths.accounts, &paths.settings)
        .context("Lecture de la configuration impossible")?;
    let account = cleanup_account(&config, account_name, &paths.env)?;

    // Routed notes may still carry the proof the ledger lacks.
    let completed = match Settings::load(&paths.settings)
        .context("Lecture des réglages impossible")?
        .notes_dir
    {
        Some(notes_dir) => crate::handled::complete_from_notes_to(
            &paths.ledger,
            Path::new(&notes_dir),
            &account.name,
        )
        .context("Complément du registre depuis les notes impossible")?,
        None => 0,
    };

    let handled = crate::handled::load_from(&paths.ledger, &account.name);
    let ledger = crate::handled::load_proofs_from(&paths.ledger, &account.name);
    let mut exporter = ImapExporter::new(account.clone(), false);
    exporter
        .connect()
        .context("Connexion au serveur impossible")?;
    let entries = exporter.audit_server(None, &handled, &ledger);
    exporter.disconnect().ok();
    let plan = plan(&entries?);
    let notes = notes_suffix(completed, plan.untargetable);

    if plan.targets.is_empty() {
        return Ok(format!("Rien à nettoyer pour {account_name}{notes}"));
    }
    let is_gmail = crate::email_export::is_gmail_server(&account.server);
    if !confirm(account_name, plan.targets.len(), is_gmail) {
        return Ok(format!("Nettoyage annulé, rien n'a été supprimé{notes}"));
    }

    // The proofs carry UIDVALIDITY: reconnecting after the dialog is safe.
    let mut exporter = ImapExporter::new(account.clone(), false);
    exporter
        .connect()
        .context("Connexion au serveur impossible")?;
    let results = exporter.delete_discarded_messages(&plan.targets);
    exporter.disconnect().ok();
    match results {
        Ok(results) => Ok(record_results(paths, &results, &notes)),
        Err(e) => {
            crate::deletion_log::record_error_to(
                &paths.deletion_log,
                &account.name,
                &format!("{e:#}"),
            );
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contextual_export::{MessageLocation, SourceIdentity};
    use tempfile::TempDir;

    fn account(delete_after_export: bool, password: Option<&str>) -> Account {
        Account {
            name: "Perso".into(),
            server: "imap.example.com".into(),
            port: 993,
            username: "me".into(),
            password: password.map(str::to_string),
            export_directory: String::new(),
            ignored_folders: Vec::new(),
            quote_depth: 1,
            skip_existing: true,
            collect_contacts: false,
            skip_signature_images: true,
            delete_after_export,
            cleanup_empty_dirs: false,
            batch_size: 0,
        }
    }

    fn proof(uid: u32, provider_id: Option<&str>) -> LocalSourceProof {
        LocalSourceProof {
            identity: SourceIdentity {
                account: "Perso".into(),
                message_id: Some(format!("{uid}@x")),
                header_fingerprint: "fp".into(),
                provider_id: provider_id.map(str::to_string),
            },
            location: MessageLocation {
                folder_raw: "INBOX".into(),
                folder_display: "INBOX".into(),
                uid_validity: 1,
                uid,
            },
        }
    }

    fn entry(
        folder: &str,
        uid: u32,
        status: ServerStatus,
        proof: Option<LocalSourceProof>,
    ) -> AuditEntry {
        AuditEntry {
            folder: folder.into(),
            uid,
            provider_id: proof.as_ref().and_then(|p| p.identity.provider_id.clone()),
            message_id: proof.as_ref().and_then(|p| p.identity.message_id.clone()),
            date: String::new(),
            from: String::new(),
            subject: String::new(),
            status,
            proof,
        }
    }

    fn paths(temp: &TempDir) -> AppPaths {
        AppPaths {
            accounts: temp.path().join("accounts.yaml"),
            env: temp.path().join(".env"),
            settings: temp.path().join("settings.yaml"),
            ledger: temp.path().join("handled_messages.txt"),
            deletion_log: temp.path().join("deletions.log"),
        }
    }

    #[test]
    fn test_cleanup_account_refuses_account_that_keeps_messages() {
        let config = Config {
            accounts: vec![account(false, Some("secret"))],
        };
        let error = cleanup_account(&config, "Perso", Path::new(".env")).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Perso ne supprime pas les messages du serveur"
        );
    }

    #[test]
    fn test_cleanup_account_refuses_missing_password() {
        let config = Config {
            accounts: vec![account(true, None)],
        };
        let error = cleanup_account(&config, "Perso", Path::new("x.env")).unwrap_err();
        assert!(error
            .to_string()
            .starts_with("Aucun mot de passe pour Perso"));
    }

    #[test]
    fn test_cleanup_account_refuses_unknown_account() {
        let config = Config { accounts: vec![] };
        assert!(cleanup_account(&config, "Perso", Path::new(".env")).is_err());
    }

    #[test]
    fn test_cleanup_account_accepts_deleting_account_with_password() {
        let config = Config {
            accounts: vec![account(true, Some("secret"))],
        };
        assert_eq!(
            cleanup_account(&config, "Perso", Path::new(".env"))
                .unwrap()
                .name,
            "Perso"
        );
    }

    #[test]
    fn test_plan_counts_untargetable_and_skips_other_statuses() {
        let entries = vec![
            entry(
                "INBOX",
                1,
                ServerStatus::HandledLegacy,
                Some(proof(1, None)),
            ),
            entry("INBOX", 2, ServerStatus::HandledLegacy, None),
            entry("INBOX", 3, ServerStatus::HandledLegacy, None),
            entry(
                "INBOX",
                4,
                ServerStatus::HandledWithProof,
                Some(proof(4, None)),
            ),
            entry("INBOX", 5, ServerStatus::Unknown, None),
            entry("INBOX", 6, ServerStatus::Staged, Some(proof(6, None))),
        ];
        let plan = plan(&entries);
        assert_eq!(plan.untargetable, 2);
        assert_eq!(plan.targets, vec![proof(1, None)]);
    }

    #[test]
    fn test_plan_gmail_message_under_several_labels_is_one_target() {
        let mut labelled = proof(9, Some("123"));
        labelled.location.folder_raw = "[Gmail]/All Mail".into();
        labelled.location.folder_display = "[Gmail]/All Mail".into();
        let entries = vec![
            entry(
                "INBOX",
                1,
                ServerStatus::HandledLegacy,
                Some(proof(1, Some("123"))),
            ),
            entry(
                "[Gmail]/All Mail",
                9,
                ServerStatus::HandledLegacy,
                Some(labelled),
            ),
        ];
        let plan = plan(&entries);
        assert_eq!(plan.targets.len(), 1);
        assert_eq!(plan.untargetable, 0);
    }

    #[test]
    fn test_notes_suffix_lists_each_non_zero_count() {
        assert_eq!(notes_suffix(0, 0), "");
        assert_eq!(
            notes_suffix(2, 1),
            " — 2 preuve(s) retrouvée(s) dans les notes, purgées au prochain export \
             — 1 message(s) impossibles à cibler exactement, laissés sur le serveur"
        );
    }

    #[test]
    fn test_record_results_ledger_keeps_only_gone_messages() {
        let temp = TempDir::new().unwrap();
        let paths = paths(&temp);
        let results = vec![
            MessageDeletionResult {
                proof: proof(1, None),
                outcome: DeletionOutcome::Deleted,
            },
            MessageDeletionResult {
                proof: proof(2, None),
                outcome: DeletionOutcome::AlreadyAbsent,
            },
            MessageDeletionResult {
                proof: proof(3, None),
                outcome: DeletionOutcome::RetryRequired("busy".into()),
            },
        ];
        let summary = record_results(&paths, &results, " — note");
        assert_eq!(
            summary,
            "1 supprimé(s), 1 déjà absent(s), 1 échec(s) — note"
        );

        let proofs = crate::handled::load_proofs_from(&paths.ledger, "Perso").proofs;
        let mut ids: Vec<_> = proofs.keys().cloned().collect();
        ids.sort();
        assert_eq!(ids, vec!["1@x".to_string(), "2@x".to_string()]);
        assert!(!crate::handled::load_from(&paths.ledger, "Perso").contains("3@x"));

        let log = std::fs::read_to_string(&paths.deletion_log).unwrap();
        assert_eq!(log.lines().count(), 3);
        assert!(log.lines().any(|l| l.ends_with("\tretry-required: busy")));
    }

    #[test]
    fn test_record_results_nothing_gone_leaves_ledger_absent() {
        let temp = TempDir::new().unwrap();
        let paths = paths(&temp);
        let results = vec![MessageDeletionResult {
            proof: proof(1, None),
            outcome: DeletionOutcome::StaleUidValidity {
                expected: 1,
                actual: Some(2),
            },
        }];
        assert_eq!(
            record_results(&paths, &results, ""),
            "0 supprimé(s), 0 déjà absent(s), 1 échec(s)"
        );
        assert!(!paths.ledger.exists());
        assert_eq!(
            std::fs::read_to_string(&paths.deletion_log)
                .unwrap()
                .lines()
                .count(),
            1
        );
    }
}
