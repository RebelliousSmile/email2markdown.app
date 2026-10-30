//! Ledger of messages the user already sorted or discarded.
//!
//! An account that does not delete from the server keeps every message there.
//! Once a note leaves the export folder (sorted elsewhere, or discarded in the
//! review window) nothing local says it was handled, so the next export would
//! propose it again. The ledger remembers the `Message-ID` of each handled
//! message so `skip_existing` can recognise it.
//!
//! Line format: `account\tmessage_id[\tproof]`. The optional third column is
//! the `LocalSourceProof` as one-line JSON: it targets the exact server message,
//! so it can be deleted safely. Lines without it are legacy entries.

use crate::contextual_export::{
    proof_is_complete, read_source_proof, LocalSourceProof, MessageLocation,
};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

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
        .filter_map(|line| {
            let mut columns = line.split('\t');
            Some((columns.next()?, columns.next()?))
        })
        .filter(|(name, _)| name.eq_ignore_ascii_case(account))
        .map(|(_, id)| id.to_string())
        .collect()
}

/// Ledger entries of one account, split by evidence.
#[derive(Debug, Default)]
pub struct LedgerProofs {
    /// Proof by `Message-ID`, when at least one line carries a readable one.
    pub proofs: HashMap<String, LocalSourceProof>,
    /// `Message-ID`s recorded without any readable proof.
    pub legacy: HashSet<String>,
}

/// Proofs recorded for `account` in the ledger at `path`. An unreadable
/// proof counts as a legacy line; a proof on any line wins over legacy lines.
pub fn load_proofs_from(path: &Path, account: &str) -> LedgerProofs {
    let mut ledger = LedgerProofs::default();
    let Ok(content) = fs::read_to_string(path) else {
        return ledger;
    };
    for line in content.lines() {
        let mut columns = line.splitn(3, '\t');
        let (Some(name), Some(id)) = (columns.next(), columns.next()) else {
            continue;
        };
        if !name.eq_ignore_ascii_case(account) {
            continue;
        }
        let proof = columns
            .next()
            .and_then(|json| serde_json::from_str::<LocalSourceProof>(json).ok());
        match proof {
            Some(proof) => {
                ledger.proofs.insert(id.to_string(), proof);
            }
            None => {
                ledger.legacy.insert(id.to_string());
            }
        }
    }
    ledger.legacy.retain(|id| !ledger.proofs.contains_key(id));
    ledger
}

/// Proofs of `account` in the default ledger.
pub fn load_proofs(account: &str) -> LedgerProofs {
    load_proofs_from(&ledger_path(), account)
}

/// A handled message seen on the server during an export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenHandled {
    /// Normalized `Message-ID`, as recorded in the ledger.
    pub message_id: String,
    pub uid: u32,
    /// Gmail `X-GM-MSGID`, when read.
    pub provider_id: Option<String>,
}

/// What an export may do with a handled message still on the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PurgeMatch {
    /// The stored proof targets this message: delete it at its current place.
    Purge(LocalSourceProof),
    /// Recorded without proof: left to the explicit server cleanup.
    Legacy,
    /// The stored proof targets another message: kept.
    Stale,
}

impl LedgerProofs {
    /// Match a handled message seen at `current` against its stored proof.
    ///
    /// Generic IMAP: the stored folder, UIDVALIDITY and UID must be the
    /// current ones. Gmail: the stored `X-GM-MSGID` must be the one read, so
    /// a message moved to another label still matches. The returned proof
    /// keeps the stored identity and takes the current location.
    pub fn match_seen(
        &self,
        seen: &SeenHandled,
        current: &MessageLocation,
        gmail: bool,
    ) -> PurgeMatch {
        let Some(stored) = self.proofs.get(&seen.message_id) else {
            return PurgeMatch::Legacy;
        };
        let same_message = if gmail {
            seen.provider_id.is_some() && stored.identity.provider_id == seen.provider_id
        } else {
            stored.location == *current
        };
        if !same_message {
            return PurgeMatch::Stale;
        }
        PurgeMatch::Purge(LocalSourceProof {
            identity: stored.identity.clone(),
            location: current.clone(),
        })
    }
}

/// One ledger line: account, `Message-ID`, optional proof.
pub type LedgerEntry = (String, String, Option<LocalSourceProof>);

/// Append entries to the ledger at `path`. An incomplete proof is dropped:
/// the line is then a legacy one.
pub fn record_to(path: &Path, entries: &[LedgerEntry]) -> Result<()> {
    let mut lines = String::new();
    for (account, id, proof) in entries {
        if !clean(account) || !clean(id) {
            continue;
        }
        lines.push_str(account);
        lines.push('\t');
        lines.push_str(id);
        if let Some(proof) = proof.as_ref().filter(|proof| proof_is_complete(proof)) {
            lines.push('\t');
            lines.push_str(&serde_json::to_string(proof).context("failed to serialize proof")?);
        }
        lines.push('\n');
    }
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
pub fn record_proofs<'a>(proofs: impl IntoIterator<Item = &'a LocalSourceProof>) {
    record_proofs_to(&ledger_path(), proofs);
}

/// `record_proofs` on the ledger at `path`.
pub fn record_proofs_to<'a>(path: &Path, proofs: impl IntoIterator<Item = &'a LocalSourceProof>) {
    let entries: Vec<LedgerEntry> = proofs
        .into_iter()
        .filter_map(|proof| {
            proof
                .identity
                .message_id
                .clone()
                .map(|id| (proof.identity.account.clone(), id, Some(proof.clone())))
        })
        .collect();
    if let Err(error) = record_to(path, &entries) {
        eprintln!("warning: handled messages ledger not updated: {error:#}");
    }
}

/// Give a proof to the legacy entries of `account` whose routed note is
/// still under `notes_dir`. Appends lines only, never rewrites. Returns the
/// number of entries completed.
///
/// Walks `notes_dir` once, without following symlinks, skipping `.*` and `_*`
/// directories, reading only `email_*.md` files. `notes_dir` may span a whole
/// documents tree: call it from an explicit action, never on every export.
pub fn complete_from_notes_to(ledger: &Path, notes_dir: &Path, account: &str) -> Result<usize> {
    if notes_dir.as_os_str().is_empty() || !notes_dir.is_dir() {
        return Ok(0);
    }
    let legacy = load_proofs_from(ledger, account).legacy;
    if legacy.is_empty() {
        return Ok(0);
    }
    let mut found: HashMap<String, LocalSourceProof> = HashMap::new();
    let walker = WalkDir::new(notes_dir)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() == 0
                || !entry.file_type().is_dir()
                || !entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with('.') || name.starts_with('_'))
        });
    for entry in walker.filter_map(|entry| entry.ok()) {
        let name = entry.file_name().to_string_lossy();
        // Without `follow_links`, a symlink is never reported as a file.
        if !entry.file_type().is_file() || !name.starts_with("email_") || !name.ends_with(".md") {
            continue;
        }
        let Ok(Some(proof)) = read_source_proof(entry.path()) else {
            continue;
        };
        if !proof.identity.account.eq_ignore_ascii_case(account) || !proof_is_complete(&proof) {
            continue;
        }
        if let Some(id) = proof.identity.message_id.clone() {
            if legacy.contains(&id) {
                found.entry(id).or_insert(proof);
            }
        }
    }
    let entries: Vec<LedgerEntry> = found
        .into_iter()
        .map(|(id, proof)| (proof.identity.account.clone(), id, Some(proof)))
        .collect();
    record_to(ledger, &entries)?;
    Ok(entries.len())
}

/// `complete_from_notes_to` on the default ledger.
pub fn complete_from_notes(notes_dir: &Path, account: &str) -> Result<usize> {
    complete_from_notes_to(&ledger_path(), notes_dir, account)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn seen(id: &str, uid: u32, provider_id: Option<&str>) -> SeenHandled {
        SeenHandled {
            message_id: id.to_string(),
            uid,
            provider_id: provider_id.map(str::to_string),
        }
    }

    fn location(folder: &str, uid_validity: u32, uid: u32) -> MessageLocation {
        MessageLocation {
            folder_raw: folder.to_string(),
            folder_display: folder.to_string(),
            uid_validity,
            uid,
        }
    }

    fn ledger_with(stored: LocalSourceProof) -> LedgerProofs {
        let mut ledger = LedgerProofs::default();
        let id = stored.identity.message_id.clone().unwrap_or_default();
        ledger.proofs.insert(id, stored);
        ledger
    }

    #[test]
    fn test_match_seen_generic_same_location_purges() {
        let stored = proof("Perso", "a@x", 7);
        let current = stored.location.clone();
        let ledger = ledger_with(stored.clone());
        assert_eq!(
            ledger.match_seen(&seen("a@x", 7, None), &current, false),
            PurgeMatch::Purge(stored)
        );
    }

    #[test]
    fn test_match_seen_generic_other_uid_is_stale() {
        let stored = proof("Perso", "a@x", 7);
        let mut current = stored.location.clone();
        current.uid = 8;
        let ledger = ledger_with(stored);
        assert_eq!(
            ledger.match_seen(&seen("a@x", 8, None), &current, false),
            PurgeMatch::Stale
        );
    }

    #[test]
    fn test_match_seen_without_proof_is_legacy() {
        let ledger = LedgerProofs::default();
        assert_eq!(
            ledger.match_seen(&seen("a@x", 7, None), &location("INBOX", 1, 7), false),
            PurgeMatch::Legacy
        );
    }

    #[test]
    fn test_match_seen_gmail_moved_message_targets_current_folder() {
        let mut stored = proof("Perso", "a@x", 7);
        stored.identity.provider_id = Some("123".into());
        let ledger = ledger_with(stored.clone());
        let current = location("[Gmail]/Archive", 99, 42);
        let PurgeMatch::Purge(target) =
            ledger.match_seen(&seen("a@x", 42, Some("123")), &current, true)
        else {
            panic!("expected a purge");
        };
        assert_eq!(target.location, current);
        assert_eq!(target.identity, stored.identity);
    }

    #[test]
    fn test_match_seen_gmail_other_provider_id_is_stale() {
        let mut stored = proof("Perso", "a@x", 7);
        stored.identity.provider_id = Some("123".into());
        let ledger = ledger_with(stored.clone());
        assert_eq!(
            ledger.match_seen(&seen("a@x", 7, Some("456")), &stored.location, true),
            PurgeMatch::Stale
        );
        assert_eq!(
            ledger.match_seen(&seen("a@x", 7, None), &stored.location, true),
            PurgeMatch::Stale
        );
    }

    fn proof(account: &str, id: &str, uid: u32) -> LocalSourceProof {
        use crate::contextual_export::{MessageLocation, SourceIdentity};
        LocalSourceProof {
            identity: SourceIdentity {
                account: account.into(),
                message_id: Some(id.into()),
                header_fingerprint: format!("fp-{id}"),
                provider_id: Some(format!("{uid}000")),
            },
            location: MessageLocation {
                folder_raw: "INBOX".into(),
                folder_display: "INBOX".into(),
                uid_validity: 9,
                uid,
            },
        }
    }

    fn note(path: &Path, proof: &LocalSourceProof) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let source = serde_yaml::to_string(&serde_json::json!({ "source": proof })).unwrap();
        fs::write(path, format!("---\n{source}---\n\nCorps\n")).unwrap();
    }

    #[test]
    fn test_record_to_with_proof_reads_back_identical() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("handled.txt");
        let expected = proof("Perso", "a@x", 3);
        record_to(
            &path,
            &[("Perso".into(), "a@x".into(), Some(expected.clone()))],
        )
        .unwrap();

        let ledger = load_proofs_from(&path, "Perso");
        assert_eq!(ledger.proofs.get("a@x"), Some(&expected));
        assert!(ledger.legacy.is_empty());
        // `load_from` still sees the Message-ID alone.
        assert_eq!(
            load_from(&path, "Perso"),
            HashSet::from(["a@x".to_string()])
        );
    }

    #[test]
    fn test_record_to_with_incomplete_proof_writes_a_legacy_line() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("handled.txt");
        let mut incomplete = proof("Perso", "a@x", 3);
        incomplete.location.uid = 0;
        record_to(&path, &[("Perso".into(), "a@x".into(), Some(incomplete))]).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "Perso\ta@x\n");
    }

    #[test]
    fn test_load_proofs_from_invalid_json_is_legacy() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("handled.txt");
        fs::write(&path, "Perso\ta@x\t{not json\nPerso\tb@x\n").unwrap();
        let ledger = load_proofs_from(&path, "Perso");
        assert!(ledger.proofs.is_empty());
        assert_eq!(ledger.legacy.len(), 2);
    }

    #[test]
    fn test_load_proofs_from_proof_wins_over_legacy_line() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("handled.txt");
        record_to(&path, &[("Perso".into(), "a@x".into(), None)]).unwrap();
        record_to(
            &path,
            &[("Perso".into(), "a@x".into(), Some(proof("Perso", "a@x", 3)))],
        )
        .unwrap();
        let ledger = load_proofs_from(&path, "Perso");
        assert!(ledger.proofs.contains_key("a@x"));
        assert!(!ledger.legacy.contains("a@x"));
    }

    #[test]
    fn test_complete_from_notes_to_with_routed_notes() {
        let temp = TempDir::new().unwrap();
        let ledger = temp.path().join("handled.txt");
        let notes = temp.path().join("notes");
        let ids = ["a@x", "b@x", "c@x", "d@x", "e@x", "f@x"];
        let entries: Vec<LedgerEntry> = ids
            .iter()
            .map(|id| ("Perso".to_string(), id.to_string(), None))
            .collect();
        record_to(&ledger, &entries).unwrap();

        // Routed notes: found.
        note(
            &notes.join("Perso/Banque/2026/10/email_a.md"),
            &proof("Perso", "a@x", 1),
        );
        note(&notes.join("Pro/email_b.md"), &proof("perso", "b@x", 2));
        // Ignored: working and hidden directories, name outside the pattern,
        // other account. `d@x` has no note at all.
        note(
            &notes.join("_travail/email_c.md"),
            &proof("Perso", "c@x", 3),
        );
        note(
            &notes.join(".obsidian/email_e.md"),
            &proof("Perso", "e@x", 5),
        );
        note(&notes.join("Perso/note_f.md"), &proof("Perso", "f@x", 6));
        note(&notes.join("Perso/email_g.md"), &proof("Pro", "a@x", 7));

        assert_eq!(complete_from_notes_to(&ledger, &notes, "Perso").unwrap(), 2);
        let ledger = load_proofs_from(&ledger, "Perso");
        let mut proved: Vec<&str> = ledger.proofs.keys().map(String::as_str).collect();
        proved.sort_unstable();
        assert_eq!(proved, ["a@x", "b@x"]);
        assert_eq!(ledger.proofs["a@x"].location.uid, 1);
        let mut legacy: Vec<&str> = ledger.legacy.iter().map(String::as_str).collect();
        legacy.sort_unstable();
        assert_eq!(legacy, ["c@x", "d@x", "e@x", "f@x"]);
    }

    #[test]
    fn test_complete_from_notes_to_twice_appends_nothing() {
        let temp = TempDir::new().unwrap();
        let ledger = temp.path().join("handled.txt");
        let notes = temp.path().join("notes");
        record_to(&ledger, &[("Perso".into(), "a@x".into(), None)]).unwrap();
        note(&notes.join("email_a.md"), &proof("Perso", "a@x", 1));

        assert_eq!(complete_from_notes_to(&ledger, &notes, "Perso").unwrap(), 1);
        assert_eq!(complete_from_notes_to(&ledger, &notes, "Perso").unwrap(), 0);
        assert_eq!(fs::read_to_string(&ledger).unwrap().lines().count(), 2);
    }

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
                ("Perso".into(), "<a@x>".into(), None),
                ("Pro".into(), "<b@x>".into(), None),
            ],
        )
        .unwrap();
        record_to(&path, &[("perso".into(), "<c@x>".into(), None)]).unwrap();

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
                ("Perso".into(), "a\tb".into(), None),
                ("Perso".into(), String::new(), None),
            ],
        )
        .unwrap();
        assert!(!path.exists());
    }
}
