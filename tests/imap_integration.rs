use email_to_markdown::config::{Account, AppPaths};
use email_to_markdown::contextual_export::{
    build_deletion_batch, ConversionStatus, DeletionOutcome, LocalSourceProof,
};
use email_to_markdown::email_export::ImapExporter;
use email_to_markdown::handled::{LedgerProofs, PurgeMatch, SeenHandled};
use email_to_markdown::route::MatchRule;
use email_to_markdown::server_cleanup::clean_server_in;
use std::cell::Cell;

/// Fails instead of passing silently when an ignored test runs without the fixture.
fn require_fixture() {
    assert_eq!(
        std::env::var("EMAIL_TO_MARKDOWN_IMAP_TEST").as_deref(),
        Ok("1"),
        "EMAIL_TO_MARKDOWN_IMAP_TEST=1 is required: start tests/fixtures/imap/compose.yaml first"
    );
    std::env::set_var("EMAIL_TO_MARKDOWN_IMAP_INSECURE_TEST", "1");
}

/// Each test owns a Dovecot user seeded by the fixture's `seed` service.
fn account(username: &str) -> Account {
    Account {
        name: "Dovecot fixture".into(),
        server: "127.0.0.1".into(),
        port: 1993,
        username: username.into(),
        password: Some("password".into()),
        ignored_folders: vec![],
        export_directory: String::new(),
        quote_depth: 1,
        skip_existing: false,
        collect_contacts: false,
        skip_signature_images: false,
        delete_after_export: true,
        cleanup_empty_dirs: true,
        batch_size: 0,
    }
}

#[test]
#[ignore = "requires tests/fixtures/imap/compose.yaml and EMAIL_TO_MARKDOWN_IMAP_TEST=1"]
fn contextual_search_uses_uidvalidity_and_keeps_messages_unseen() {
    require_fixture();
    let fixture_account = account("contextual");
    let mut exporter = ImapExporter::new(fixture_account.clone(), true);
    exporter.connect().unwrap();
    let candidates = exporter
        .search_contextual(&[MatchRule::Correspondent("alice@example.com".into())])
        .unwrap();

    assert_eq!(candidates.len(), 2, "sender and recipient matches expected");
    assert!(candidates.iter().all(|candidate| {
        candidate
            .locations
            .iter()
            .all(|location| location.uid > 0 && location.uid_validity > 0)
    }));

    let deletion_preflight = exporter.contextual_deletion_preflight().unwrap();
    assert!(deletion_preflight.supported, "fixture must expose UIDPLUS");

    let target = tempfile::TempDir::new().unwrap();
    let converted = exporter
        .convert_contextual_selection(target.path(), &candidates[..1], false)
        .unwrap();
    assert_eq!(converted.len(), 1);
    assert!(matches!(
        converted[0].status,
        Some(ConversionStatus::Written { .. })
    ));
    assert_eq!(
        std::fs::read_dir(target.path())
            .unwrap()
            .filter(|entry| entry
                .as_ref()
                .unwrap()
                .path()
                .extension()
                .and_then(|e| e.to_str())
                == Some("md"))
            .count(),
        1,
        "only the selected UID is converted"
    );
    assert!(candidates.iter().all(|candidate| {
        !candidate
            .from
            .iter()
            .any(|address| address.ends_with("@notalice.example.com"))
    }));

    let deletion_batch = build_deletion_batch(&converted);
    assert_eq!(deletion_batch.len(), 1);
    let markdown_before_retry = std::fs::read(&deletion_batch[0].markdown).unwrap();
    let deleted = exporter.delete_proved_messages(&deletion_batch).unwrap();
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0].outcome, DeletionOutcome::Deleted);

    let retried = exporter.delete_proved_messages(&deletion_batch).unwrap();
    assert_eq!(retried.len(), 1);
    assert_eq!(retried[0].outcome, DeletionOutcome::AlreadyAbsent);
    assert_eq!(
        std::fs::read(&deletion_batch[0].markdown).unwrap(),
        markdown_before_retry,
        "a deletion retry must not rewrite the Markdown"
    );

    let client = imap::ClientBuilder::new("127.0.0.1", 1993)
        .mode(imap::ConnectionMode::Tls)
        .danger_skip_tls_verify(true)
        .connect()
        .unwrap();
    let mut session = client
        .login("contextual", "password")
        .map_err(|(e, _)| e)
        .unwrap();
    session.examine("INBOX").unwrap();
    let fetched = session.uid_fetch("1:*", "(UID FLAGS)").unwrap();
    assert_eq!(fetched.len(), 2, "only the selected UID must be deleted");
    assert!(fetched.iter().all(|item| {
        item.flags()
            .iter()
            .all(|flag| !matches!(flag, imap::types::Flag::Seen))
    }));
}

#[test]
#[ignore = "requires tests/fixtures/imap/compose.yaml and EMAIL_TO_MARKDOWN_IMAP_TEST=1"]
fn handled_message_with_ledger_proof_is_purged_at_its_current_place() {
    require_fixture();
    let mut exporter = ImapExporter::new(account("purge"), true);
    exporter.connect().unwrap();
    let candidates = exporter
        .search_contextual(&[MatchRule::Correspondent("alice@example.com".into())])
        .unwrap();
    let candidate = &candidates[0];
    let current = candidate.locations[0].clone();
    let message_id = candidate.source.message_id.clone().unwrap();

    // The ledger proof recorded at sorting time, for the same place.
    let mut ledger = LedgerProofs::default();
    ledger.proofs.insert(
        message_id.clone(),
        email_to_markdown::contextual_export::LocalSourceProof {
            identity: candidate.source.clone(),
            location: current.clone(),
        },
    );
    let seen = SeenHandled {
        message_id,
        uid: current.uid,
        provider_id: None,
    };
    let PurgeMatch::Purge(target) = ledger.match_seen(&seen, &current, false) else {
        panic!("the stored proof targets this message");
    };
    let mut elsewhere = current.clone();
    elsewhere.uid += 1000;
    assert_eq!(
        ledger.match_seen(&seen, &elsewhere, false),
        PurgeMatch::Stale
    );

    let results = exporter.delete_discarded_messages(&[target]).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].outcome, DeletionOutcome::Deleted);
    let after = exporter
        .search_contextual(&[MatchRule::Correspondent("alice@example.com".into())])
        .unwrap();
    assert_eq!(
        after.len(),
        candidates.len() - 1,
        "only the proved message is gone"
    );
}

/// Raw session on a fixture user, outside the code under test.
fn raw_session(username: &str) -> imap::Session<Box<dyn imap::ImapConnection>> {
    let client = imap::ClientBuilder::new("127.0.0.1", 1993)
        .mode(imap::ConnectionMode::Tls)
        .danger_skip_tls_verify(true)
        .connect()
        .unwrap();
    client
        .login(username, "password")
        .map_err(|(e, _)| e)
        .unwrap()
}

/// Message-IDs still in the INBOX of a fixture user, sorted.
fn inbox_message_ids(username: &str) -> Vec<String> {
    let mut session = raw_session(username);
    session.examine("INBOX").unwrap();
    let fetched = session
        .uid_fetch("1:*", "BODY.PEEK[HEADER.FIELDS (MESSAGE-ID)]")
        .unwrap();
    let mut ids: Vec<String> = fetched
        .iter()
        .filter_map(|item| item.header())
        .map(|header| {
            let header = String::from_utf8_lossy(header);
            let value = header.trim().split_once(':').map_or("", |(_, v)| v);
            value.trim().to_string()
        })
        .collect();
    session.logout().ok();
    ids.sort();
    ids
}

const SEEDED_IDS: [&str; 3] = [
    "<fixture-false-positive@example.com>",
    "<fixture-from-alice@example.com>",
    "<fixture-to-alice@example.com>",
];

/// Config directory of a cleanup test: one account on the fixture user, a
/// ledger holding the two alice messages as legacy entries, without proof.
fn cleanup_dir(username: &str, account_name: &str) -> (tempfile::TempDir, AppPaths) {
    let temp = tempfile::TempDir::new().unwrap();
    let paths = AppPaths {
        accounts: temp.path().join("accounts.yaml"),
        env: temp.path().join(".env"),
        settings: temp.path().join("settings.yaml"),
        ledger: temp.path().join("handled_messages.txt"),
        deletion_log: temp.path().join("deletions.log"),
    };
    std::fs::write(
        &paths.accounts,
        format!(
            "accounts:\n  - name: {account_name}\n    server: 127.0.0.1\n    port: 1993\n    username: {username}\n"
        ),
    )
    .unwrap();
    let notes = temp.path().join("notes");
    std::fs::create_dir_all(&notes).unwrap();
    std::fs::write(
        &paths.settings,
        format!(
            "export_base_dir: '{}'\nnotes_dir: '{}'\ndefaults:\n  delete_after_export: true\n",
            temp.path().join("export").display(),
            notes.display()
        ),
    )
    .unwrap();
    std::fs::write(
        &paths.ledger,
        format!(
            "{account_name}\tfixture-from-alice@example.com\n{account_name}\tfixture-to-alice@example.com\n"
        ),
    )
    .unwrap();
    // The process environment wins over `.env`.
    std::env::set_var(
        format!(
            "{}_PASSWORD",
            email_to_markdown::config::env_var_name(account_name)
        ),
        "password",
    );
    (temp, paths)
}

#[test]
#[ignore = "requires tests/fixtures/imap/compose.yaml and EMAIL_TO_MARKDOWN_IMAP_TEST=1"]
fn clean_server_accepted_deletes_legacy_messages_and_records_proofs() {
    require_fixture();
    let (_temp, paths) = cleanup_dir("clean-accepted", "CleanAccepted");
    let asked = Cell::new(None);
    let summary = clean_server_in(&paths, "CleanAccepted", |_, count, _| {
        asked.set(Some(count));
        true
    })
    .unwrap();

    assert_eq!(asked.get(), Some(2));
    assert_eq!(summary, "2 supprimé(s), 0 déjà absent(s), 0 échec(s)");
    assert_eq!(
        inbox_message_ids("clean-accepted"),
        vec![SEEDED_IDS[0].to_string()],
        "only the unhandled message stays on the server"
    );
    let proofs = email_to_markdown::handled::load_proofs_from(&paths.ledger, "CleanAccepted");
    assert_eq!(proofs.proofs.len(), 2);
    assert_eq!(
        std::fs::read_to_string(&paths.deletion_log)
            .unwrap()
            .lines()
            .filter(|line| line.ends_with("\tdeleted"))
            .count(),
        2
    );
}

#[test]
#[ignore = "requires tests/fixtures/imap/compose.yaml and EMAIL_TO_MARKDOWN_IMAP_TEST=1"]
fn clean_server_refused_deletes_nothing() {
    require_fixture();
    let (_temp, paths) = cleanup_dir("clean-refused", "CleanRefused");
    let ledger_before = std::fs::read(&paths.ledger).unwrap();
    let summary = clean_server_in(&paths, "CleanRefused", |_, _, _| false).unwrap();

    assert_eq!(summary, "Nettoyage annulé, rien n'a été supprimé");
    assert_eq!(inbox_message_ids("clean-refused"), SEEDED_IDS.to_vec());
    assert_eq!(std::fs::read(&paths.ledger).unwrap(), ledger_before);
    assert!(!paths.deletion_log.exists());
}

/// Proof of the first alice message of a fixture user, as a census builds it.
fn alice_proof(exporter: &mut ImapExporter) -> LocalSourceProof {
    let candidates = exporter
        .search_contextual(&[MatchRule::Correspondent("alice@example.com".into())])
        .unwrap();
    LocalSourceProof {
        identity: candidates[0].source.clone(),
        location: candidates[0].locations[0].clone(),
    }
}

#[test]
#[ignore = "requires tests/fixtures/imap/compose.yaml and EMAIL_TO_MARKDOWN_IMAP_TEST=1"]
fn delete_discarded_stale_uidvalidity_keeps_message() {
    require_fixture();
    let mut exporter = ImapExporter::new(account("clean-stale"), false);
    exporter.connect().unwrap();
    let mut proof = alice_proof(&mut exporter);
    let actual = proof.location.uid_validity;
    proof.location.uid_validity = actual.wrapping_add(1);

    let results = exporter.delete_discarded_messages(&[proof]).unwrap();
    assert_eq!(
        results[0].outcome,
        DeletionOutcome::StaleUidValidity {
            expected: actual.wrapping_add(1),
            actual: Some(actual),
        }
    );
    assert_eq!(inbox_message_ids("clean-stale"), SEEDED_IDS.to_vec());
}

#[test]
#[ignore = "requires tests/fixtures/imap/compose.yaml and EMAIL_TO_MARKDOWN_IMAP_TEST=1"]
fn delete_discarded_message_removed_before_is_already_absent() {
    require_fixture();
    let mut exporter = ImapExporter::new(account("clean-absent"), false);
    exporter.connect().unwrap();
    let proof = alice_proof(&mut exporter);

    let mut session = raw_session("clean-absent");
    session.select("INBOX").unwrap();
    let uid = proof.location.uid.to_string();
    session
        .uid_store(&uid, "+FLAGS.SILENT (\\Deleted)")
        .unwrap();
    session.uid_expunge(&uid).unwrap();
    session.logout().ok();

    let results = exporter.delete_discarded_messages(&[proof]).unwrap();
    assert_eq!(results[0].outcome, DeletionOutcome::AlreadyAbsent);
    assert_eq!(inbox_message_ids("clean-absent").len(), 2);
}

#[test]
#[ignore = "requires tests/fixtures/imap/compose.yaml and EMAIL_TO_MARKDOWN_IMAP_TEST=1"]
fn delete_discarded_batch_mixes_deleted_and_already_absent() {
    require_fixture();
    let mut exporter = ImapExporter::new(account("batch"), false);
    exporter.connect().unwrap();
    let candidates = exporter
        .search_contextual(&[MatchRule::Domain("example.com".into())])
        .unwrap();
    let proofs: Vec<LocalSourceProof> = candidates
        .iter()
        .map(|candidate| LocalSourceProof {
            identity: candidate.source.clone(),
            location: candidate.locations[0].clone(),
        })
        .collect();
    assert_eq!(proofs.len(), 3);

    let mut session = raw_session("batch");
    session.select("INBOX").unwrap();
    let uid = proofs[1].location.uid.to_string();
    session
        .uid_store(&uid, "+FLAGS.SILENT (\\Deleted)")
        .unwrap();
    session.uid_expunge(&uid).unwrap();
    session.logout().ok();

    let results = exporter.delete_discarded_messages(&proofs).unwrap();
    let outcomes: Vec<DeletionOutcome> = results.into_iter().map(|r| r.outcome).collect();
    assert_eq!(outcomes.len(), 3);
    assert_eq!(
        outcomes
            .iter()
            .filter(|o| **o == DeletionOutcome::Deleted)
            .count(),
        2
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|o| **o == DeletionOutcome::AlreadyAbsent)
            .count(),
        1
    );
    assert!(inbox_message_ids("batch").is_empty());
}

#[test]
#[ignore = "requires tests/fixtures/imap/compose.yaml and EMAIL_TO_MARKDOWN_IMAP_TEST=1"]
fn delete_discarded_missing_folder_requires_retry() {
    require_fixture();
    let mut exporter = ImapExporter::new(account("clean-missing"), false);
    exporter.connect().unwrap();
    let mut proof = alice_proof(&mut exporter);
    proof.location.folder_raw = "Absent".into();
    proof.location.folder_display = "Absent".into();

    let results = exporter.delete_discarded_messages(&[proof]).unwrap();
    assert!(matches!(
        results[0].outcome,
        DeletionOutcome::RetryRequired(_)
    ));
    assert_eq!(inbox_message_ids("clean-missing"), SEEDED_IDS.to_vec());
}

#[test]
#[ignore = "requires an isolated Gmail fixture account and EMAIL_TO_MARKDOWN_GMAIL_DESTRUCTIVE_TEST=1"]
fn gmail_targeted_deletion_fixture() {
    if std::env::var("EMAIL_TO_MARKDOWN_GMAIL_DESTRUCTIVE_TEST").as_deref() != Ok("1") {
        return;
    }
    let username = std::env::var("EMAIL_TO_MARKDOWN_GMAIL_USER").unwrap();
    let password = std::env::var("EMAIL_TO_MARKDOWN_GMAIL_PASSWORD").unwrap();
    let correspondent = std::env::var("EMAIL_TO_MARKDOWN_GMAIL_FIXTURE_ADDRESS").unwrap();
    let fixture = Account {
        name: "Dedicated Gmail destructive fixture".into(),
        server: "imap.gmail.com".into(),
        port: 993,
        username,
        password: Some(password),
        ignored_folders: vec![
            "[Gmail]/Spam".into(),
            "[Gmail]/Trash".into(),
            "[Gmail]/Drafts".into(),
        ],
        export_directory: String::new(),
        quote_depth: 1,
        skip_existing: false,
        collect_contacts: false,
        skip_signature_images: false,
        delete_after_export: true,
        cleanup_empty_dirs: true,
        batch_size: 0,
    };
    let mut exporter = ImapExporter::new(fixture, false);
    exporter.connect().unwrap();
    let candidates = exporter
        .search_contextual(&[MatchRule::Correspondent(correspondent)])
        .unwrap();
    let selected: Vec<_> = candidates
        .into_iter()
        .filter(|candidate| candidate.subject == "[email-to-markdown destructive fixture]")
        .collect();
    assert_eq!(
        selected.len(),
        1,
        "the dedicated account must contain exactly one marked fixture"
    );
    assert!(selected[0].source.provider_id.is_some());
    let target = tempfile::TempDir::new().unwrap();
    let conversions = exporter
        .convert_contextual_selection(target.path(), &selected, false)
        .unwrap();
    let requests = build_deletion_batch(&conversions);
    assert_eq!(requests.len(), 1);
    let outcomes = exporter.delete_proved_messages(&requests).unwrap();
    assert_eq!(outcomes.len(), 1);
    assert_eq!(outcomes[0].outcome, DeletionOutcome::Deleted);
}

/// Config directory of an export test: settings without destinations, ledger
/// and deletion log in a `TempDir`, and an account exporting into it.
fn export_dir(username: &str) -> (tempfile::TempDir, AppPaths, Account) {
    let temp = tempfile::TempDir::new().unwrap();
    let paths = AppPaths {
        accounts: temp.path().join("accounts.yaml"),
        env: temp.path().join(".env"),
        settings: temp.path().join("settings.yaml"),
        ledger: temp.path().join("handled_messages.txt"),
        deletion_log: temp.path().join("deletions.log"),
    };
    std::fs::write(&paths.settings, "organize_by_date: false\n").unwrap();
    let mut fixture_account = account(username);
    fixture_account.export_directory = temp.path().join("export").display().to_string();
    fixture_account.delete_after_export = false;
    (temp, paths, fixture_account)
}

#[test]
#[ignore = "requires tests/fixtures/imap/compose.yaml and EMAIL_TO_MARKDOWN_IMAP_TEST=1"]
fn export_account_in_pairs_each_uid_with_its_body_across_batches() {
    require_fixture();
    let (_temp, paths, fixture_account) = export_dir("export-batch");
    let mut exporter = ImapExporter::new(fixture_account, false);
    exporter.connect().unwrap();
    let (stats, decisions) = exporter
        .export_account_in(&paths, None, None, None)
        .unwrap();
    exporter.disconnect().ok();

    assert_eq!(stats.values().map(|s| s.exported).sum::<usize>(), 15);
    assert_eq!(decisions.len(), 15);
    let markers: Vec<String> = (1..=12).map(|n| format!("BATCHMARK-{n:02}-END")).collect();
    let mut seen = 0;
    for (note, _) in &decisions {
        let content = std::fs::read_to_string(note).unwrap();
        let own: Vec<usize> = (0..markers.len())
            .filter(|&i| content.contains(&markers[i]))
            .collect();
        // Seeded messages carry no marker; batch messages exactly their own.
        assert!(own.len() <= 1, "{} carries several bodies", note.display());
        if let Some(&i) = own.first() {
            assert!(
                content.contains(&format!("Batch message {:02}", i + 1)),
                "{} pairs a body with another message",
                note.display()
            );
            seen += 1;
        }
    }
    assert_eq!(seen, 12, "every batch message is exported once");
}

#[test]
#[ignore = "requires tests/fixtures/imap/compose.yaml and EMAIL_TO_MARKDOWN_IMAP_TEST=1"]
fn export_account_in_purges_only_proved_handled_messages() {
    require_fixture();
    let (_temp, paths, mut fixture_account) = export_dir("export-purge");
    fixture_account.skip_existing = true;

    // First export: the notes carry the proofs the sorting would record.
    let mut exporter = ImapExporter::new(fixture_account.clone(), false);
    exporter.connect().unwrap();
    let (_, decisions) = exporter
        .export_account_in(&paths, None, None, None)
        .unwrap();
    exporter.disconnect().ok();
    let proofs: Vec<LocalSourceProof> = decisions
        .iter()
        .filter_map(|(note, _)| {
            email_to_markdown::contextual_export::read_source_proof(note).unwrap()
        })
        .collect();
    assert_eq!(proofs.len(), 3);
    let by_id = |id: &str| {
        proofs
            .iter()
            .find(|p| {
                p.identity.message_id.as_deref() == Some(id.trim_matches(|c| c == '<' || c == '>'))
            })
            .unwrap()
            .clone()
    };
    let mut stale = by_id(SEEDED_IDS[0]);
    stale.location.uid += 1000;
    email_to_markdown::handled::record_proofs_to(
        &paths.ledger,
        [by_id(SEEDED_IDS[1]), by_id(SEEDED_IDS[2]), stale].iter(),
    );

    // Second export: the two proved messages are purged, the stale one kept.
    fixture_account.delete_after_export = true;
    let mut exporter = ImapExporter::new(fixture_account.clone(), false);
    exporter.connect().unwrap();
    let (stats, _) = exporter
        .export_account_in(&paths, None, None, None)
        .unwrap();
    exporter.disconnect().ok();
    assert_eq!(stats.values().map(|s| s.purged).sum::<usize>(), 2);
    assert_eq!(stats.values().map(|s| s.stale_proof).sum::<usize>(), 1);
    assert_eq!(
        inbox_message_ids("export-purge"),
        vec![SEEDED_IDS[0].to_string()]
    );
    let deleted_lines = |log: &str| log.lines().filter(|l| l.ends_with("\tdeleted")).count();
    let log = std::fs::read_to_string(&paths.deletion_log).unwrap();
    assert_eq!(deleted_lines(&log), 2);

    // Third export: nothing left to purge, nothing logged.
    let mut exporter = ImapExporter::new(fixture_account, false);
    exporter.connect().unwrap();
    let (stats, _) = exporter
        .export_account_in(&paths, None, None, None)
        .unwrap();
    exporter.disconnect().ok();
    assert_eq!(stats.values().map(|s| s.purged).sum::<usize>(), 0);
    assert_eq!(std::fs::read_to_string(&paths.deletion_log).unwrap(), log);
    assert_eq!(
        inbox_message_ids("export-purge"),
        vec![SEEDED_IDS[0].to_string()]
    );
}
