use email_to_markdown::config::Account;
use email_to_markdown::contextual_export::{
    build_deletion_batch, ConversionStatus, DeletionOutcome,
};
use email_to_markdown::email_export::ImapExporter;
use email_to_markdown::route::MatchRule;

fn enabled() -> bool {
    std::env::var("EMAIL_TO_MARKDOWN_IMAP_TEST").as_deref() == Ok("1")
}

fn account() -> Account {
    Account {
        name: "Dovecot fixture".into(),
        server: "127.0.0.1".into(),
        port: 1993,
        username: "test".into(),
        password: Some("password".into()),
        ignored_folders: vec![],
        export_directory: String::new(),
        quote_depth: 1,
        skip_existing: false,
        collect_contacts: false,
        skip_signature_images: false,
        delete_after_export: true,
        cleanup_empty_dirs: true,
    }
}

#[test]
#[ignore = "requires tests/fixtures/imap/compose.yaml and EMAIL_TO_MARKDOWN_IMAP_TEST=1"]
fn contextual_search_uses_uidvalidity_and_keeps_messages_unseen() {
    if !enabled() {
        return;
    }
    std::env::set_var("EMAIL_TO_MARKDOWN_IMAP_INSECURE_TEST", "1");
    let fixture_account = account();
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
        .convert_contextual_selection(target.path(), &candidates[..1])
        .unwrap();
    assert_eq!(converted.len(), 1);
    assert!(matches!(
        converted[0].status,
        Some(ConversionStatus::Written { .. })
    ));
    assert_eq!(
        std::fs::read_dir(target.path())
            .unwrap()
            .filter(|entry| entry.as_ref().unwrap().path().extension().and_then(|e| e.to_str()) == Some("md"))
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
    let mut session = client.login("test", "password").map_err(|(e, _)| e).unwrap();
    session.examine("INBOX").unwrap();
    let fetched = session.uid_fetch("1:*", "(UID FLAGS)").unwrap();
    assert_eq!(fetched.len(), 2, "only the selected UID must be deleted");
    assert!(fetched.iter().all(|item| {
        item.flags()
            .iter()
            .all(|flag| !matches!(flag, imap::types::Flag::Seen))
    }));
}
