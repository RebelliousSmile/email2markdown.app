//! Read-only census of the messages still on the server.
//!
//! Before anything is deleted, tell for every server message why it is still
//! there: handled (sorted or discarded) with or without a proof, waiting as a
//! staged note, or never exported. Nothing is written, locally or remotely.

use crate::contextual_export::LocalSourceProof;
use crate::email_export::{skip_reason_from_headers, ExportedIndex, SkipReason};
use mailparse::MailHeaderMap;
use std::collections::{BTreeMap, HashSet};

/// Why a message is still on the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ServerStatus {
    /// In the ledger, with a proof that targets it exactly.
    HandledWithProof,
    /// In the ledger by `Message-ID` only.
    HandledLegacy,
    /// A note of the export folder matches it: not routed yet.
    Staged,
    /// Never exported.
    Unknown,
}

impl ServerStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::HandledWithProof => "traité, avec preuve",
            Self::HandledLegacy => "traité, sans preuve",
            Self::Staged => "note en attente de tri",
            Self::Unknown => "jamais exporté",
        }
    }
}

/// Status of one message from its headers. `proved` holds the `Message-ID`s
/// whose ledger entry carries a proof.
pub fn classify(
    raw_headers: &[u8],
    exported: &ExportedIndex,
    handled: &HashSet<String>,
    proved: &HashSet<String>,
) -> ServerStatus {
    match skip_reason_from_headers(raw_headers, exported, handled) {
        Some(SkipReason::Handled(id)) if proved.contains(&id) => ServerStatus::HandledWithProof,
        Some(SkipReason::Handled(_)) => ServerStatus::HandledLegacy,
        Some(SkipReason::Exported) => ServerStatus::Staged,
        None => ServerStatus::Unknown,
    }
}

/// One server message and its status.
#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub folder: String,
    pub uid: u32,
    pub provider_id: Option<String>,
    pub message_id: Option<String>,
    pub date: String,
    pub from: String,
    pub subject: String,
    pub status: ServerStatus,
    /// Proof targeting this exact message, when it can be built (UIDVALIDITY,
    /// Message-ID or `X-GM-MSGID` on Gmail).
    pub proof: Option<LocalSourceProof>,
}

impl AuditEntry {
    pub fn from_headers(
        folder: &str,
        uid: u32,
        provider_id: Option<String>,
        raw_headers: &[u8],
        status: ServerStatus,
    ) -> Self {
        let mut entry = Self {
            folder: folder.to_string(),
            uid,
            provider_id,
            message_id: None,
            date: String::new(),
            from: String::new(),
            subject: String::new(),
            status,
            proof: None,
        };
        if let Ok(mail) = mailparse::parse_mail(raw_headers) {
            let get = |name| mail.headers.get_first_value(name).unwrap_or_default();
            entry.message_id = crate::contextual_export::normalize_message_id(&get("Message-ID"));
            entry.date = get("Date");
            entry.from = get("From");
            entry.subject = get("Subject");
        }
        entry
    }
}

/// Proofs of the handled messages the ledger holds without proof, one per
/// message: the targets of the confirmed server cleanup. Messages that cannot
/// be targeted exactly are left out.
pub fn legacy_proofs(entries: &[AuditEntry]) -> Vec<LocalSourceProof> {
    let mut seen = HashSet::new();
    entries
        .iter()
        .filter(|entry| entry.status == ServerStatus::HandledLegacy)
        .filter_map(|entry| entry.proof.clone())
        .filter(|proof| seen.insert(crate::contextual_export::proof_target_key(proof)))
        .collect()
}

/// Totals per status and per folder, then one line per message if `details`.
pub fn render(account: &str, entries: &[AuditEntry], details: bool) -> String {
    let mut totals: BTreeMap<ServerStatus, usize> = BTreeMap::new();
    let mut per_folder: BTreeMap<&str, BTreeMap<ServerStatus, usize>> = BTreeMap::new();
    for entry in entries {
        *totals.entry(entry.status).or_default() += 1;
        *per_folder
            .entry(&entry.folder)
            .or_default()
            .entry(entry.status)
            .or_default() += 1;
    }

    let mut out = format!("{account} : {} message(s) sur le serveur\n", entries.len());
    for (status, count) in &totals {
        out.push_str(&format!("  {:<24} {count}\n", status.label()));
    }
    for (folder, counts) in &per_folder {
        out.push_str(&format!("\n{folder}\n"));
        for (status, count) in counts {
            out.push_str(&format!("  {:<24} {count}\n", status.label()));
        }
    }
    if details {
        let mut sorted: Vec<&AuditEntry> = entries.iter().collect();
        sorted.sort_by(|a, b| (a.status, &a.folder, a.uid).cmp(&(b.status, &b.folder, b.uid)));
        out.push('\n');
        for entry in sorted {
            out.push_str(&format!(
                "[{}] {} uid {} | {} | {} | {} | {}\n",
                entry.status.label(),
                entry.folder,
                entry.uid,
                entry.date,
                entry.from,
                entry.subject,
                entry.message_id.as_deref().unwrap_or("-")
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn headers(id: &str, subject: &str) -> Vec<u8> {
        format!(
            "Message-ID: <{id}>\r\nDate: Thu, 29 Oct 2026 10:00:00 +0100\r\nFrom: Alice Martin <alice@x.org>\r\nTo: Bob <bob@y.org>\r\nSubject: {subject}\r\n\r\n"
        )
        .into_bytes()
    }

    #[test]
    fn test_classify_distinguishes_the_four_statuses() {
        let temp = TempDir::new().unwrap();
        // A staged note for <staged@x>: name from date and sender, subject hash inside.
        let subject_hash = crate::utils::hash_md5_prefix("Staged", 6);
        fs::write(
            temp.path().join("email_2026-10-29_AlicMart_staged.md"),
            format!("---\nsubject_hash: {subject_hash}\n---\n"),
        )
        .unwrap();
        let exported = ExportedIndex::build(temp.path());
        let handled: HashSet<String> = ["proved@x".to_string(), "legacy@x".to_string()].into();
        let proved: HashSet<String> = ["proved@x".to_string()].into();

        let status = |id, subject| classify(&headers(id, subject), &exported, &handled, &proved);
        assert_eq!(status("proved@x", "A"), ServerStatus::HandledWithProof);
        assert_eq!(status("legacy@x", "B"), ServerStatus::HandledLegacy);
        assert_eq!(status("staged@x", "Staged"), ServerStatus::Staged);
        assert_eq!(status("new@x", "C"), ServerStatus::Unknown);
    }

    #[test]
    fn test_classify_prefers_the_ledger_over_a_staged_note() {
        let temp = TempDir::new().unwrap();
        let subject_hash = crate::utils::hash_md5_prefix("Both", 6);
        fs::write(
            temp.path().join("email_2026-10-29_AlicMart_both.md"),
            format!("---\nsubject_hash: {subject_hash}\n---\n"),
        )
        .unwrap();
        let exported = ExportedIndex::build(temp.path());
        let handled: HashSet<String> = ["both@x".to_string()].into();
        assert_eq!(
            classify(
                &headers("both@x", "Both"),
                &exported,
                &handled,
                &HashSet::new()
            ),
            ServerStatus::HandledLegacy
        );
    }

    fn legacy_entry(uid: u32, provider_id: Option<&str>, with_proof: bool) -> AuditEntry {
        use crate::contextual_export::{MessageLocation, SourceIdentity};
        let mut entry = AuditEntry::from_headers(
            "INBOX",
            uid,
            provider_id.map(str::to_string),
            &headers(&format!("{uid}@x"), "S"),
            ServerStatus::HandledLegacy,
        );
        entry.proof = with_proof.then(|| LocalSourceProof {
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
        });
        entry
    }

    #[test]
    fn test_legacy_proofs_keeps_targetable_legacy_messages_once() {
        let mut proved = legacy_entry(1, None, true);
        proved.status = ServerStatus::HandledWithProof;
        let entries = vec![
            legacy_entry(2, None, true),
            legacy_entry(3, None, false),
            proved,
            legacy_entry(4, Some("g"), true),
            legacy_entry(5, Some("g"), true),
        ];
        let uids: Vec<u32> = legacy_proofs(&entries)
            .iter()
            .map(|proof| proof.location.uid)
            .collect();
        assert_eq!(uids, vec![2, 4]);
    }

    #[test]
    fn test_render_totals_and_details() {
        let entries = vec![
            AuditEntry::from_headers(
                "INBOX",
                3,
                None,
                &headers("a@x", "Un"),
                ServerStatus::Unknown,
            ),
            AuditEntry::from_headers(
                "INBOX",
                4,
                None,
                &headers("b@x", "Deux"),
                ServerStatus::HandledLegacy,
            ),
        ];
        let summary = render("Perso", &entries, false);
        assert!(summary.contains("Perso : 2 message(s) sur le serveur"));
        assert!(summary.contains("traité, sans preuve"));
        assert!(!summary.contains("uid 3"));

        let details = render("Perso", &entries, true);
        assert!(details.contains("[jamais exporté] INBOX uid 3 |"));
        assert!(details.contains("| Un | a@x\n"));
        assert!(!details.contains("<a@x>"));
    }
}
