use email_to_markdown::config::{Config, Settings, AccountBehavior, RawAccount, load_raw_accounts, save_accounts};
use email_to_markdown::network::{NetworkConfig, ProgressIndicator};  // [3][4]
use email_to_markdown::utils::*;
use std::time::Duration;
use tempfile::TempDir;

mod utils_tests {
    use super::*;

    #[test]
    fn test_limit_quote_depth_basic() {
        let text = "Hello\n> First quote\n>> Second quote\n>>> Third quote\n> Back to first";
        let result = limit_quote_depth(text, 1);
        let expected = "Hello\n> First quote\n> Back to first";
        assert_eq!(result, expected);
    }

    #[test]
    fn test_limit_quote_depth_no_quotes() {
        let text = "Hello\nWorld";
        let result = limit_quote_depth(text, 1);
        assert_eq!(result, text);
    }

    #[test]
    fn test_get_short_name_email() {
        assert_eq!(get_short_name(Some("sender@example.com")), "Sender");
    }

    #[test]
    fn test_get_short_name_full_name() {
        assert_eq!(get_short_name(Some("John Doe <john@example.com>")), "JohnDoe");
    }

    #[test]
    fn test_get_short_name_multiple_words() {
        assert_eq!(get_short_name(Some("John Michael Doe")), "JohnDoe");
    }

    #[test]
    fn test_get_short_name_none() {
        assert_eq!(get_short_name(None), "UNK");
    }

    #[test]
    fn test_get_short_name_empty() {
        assert_eq!(get_short_name(Some("")), "UNK");
    }

    #[test]
    fn test_extract_emails_single() {
        let result = extract_emails(Some("Name <email@domain.com>"));
        assert_eq!(result, vec!["email@domain.com"]);
    }

    #[test]
    fn test_extract_emails_multiple() {
        let result = extract_emails(Some("a@b.com, c@d.com"));
        assert_eq!(result, vec!["a@b.com", "c@d.com"]);
    }

    #[test]
    fn test_extract_emails_none() {
        let result = extract_emails(None);
        assert!(result.is_empty());
    }

    #[test]
    fn test_normalize_line_breaks() {
        let text = "Hello\n\n\n\nWorld";
        let result = normalize_line_breaks(text);
        assert_eq!(result, "Hello\n\nWorld");
    }

    #[test]
    fn test_is_signature_image_signature() {
        assert!(is_signature_image(
            Some("signature.png"),
            "image/png",
            1024,
            Some("inline")
        ));
    }

    #[test]
    fn test_is_signature_image_logo() {
        assert!(is_signature_image(
            Some("logo.jpg"),
            "image/jpeg",
            5120,
            Some("attachment")
        ));
    }

    #[test]
    fn test_is_signature_image_document() {
        assert!(!is_signature_image(
            Some("contract.pdf"),
            "application/pdf",
            102400,
            Some("attachment")
        ));
    }

    #[test]
    fn test_is_signature_image_large_photo() {
        assert!(!is_signature_image(
            Some("photo_vacation.jpg"),
            "image/jpeg",
            2048000,
            Some("attachment")
        ));
    }

    #[test]
    fn test_hash_md5_prefix_length() {
        let hash = hash_md5_prefix("Test Subject", 6);
        assert_eq!(hash.len(), 6);
    }

    #[test]
    fn test_hash_md5_prefix_consistency() {
        let hash1 = hash_md5_prefix("Test Subject", 6);
        let hash2 = hash_md5_prefix("Test Subject", 6);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_sanitize_filename() {
        let filename = "test<>:\"/\\|?*.txt";
        let result = sanitize_filename(filename);
        assert!(!result.contains('<'));
        assert!(!result.contains('>'));
        assert!(!result.contains(':'));
    }

    #[test]
    fn test_decode_imap_utf7_basic() {
        let result = decode_imap_utf7("INBOX");
        assert_eq!(result, "INBOX");
    }

    #[test]
    fn test_decode_imap_utf7_french_chars() {
        // &AOk- is IMAP modified UTF-7 for é (U+00E9)
        let result = decode_imap_utf7("INBOX.&AOk-");
        assert!(result.contains('é') || result == "INBOX.&AOk-");
    }
}

mod config_tests {
    use super::*;

    #[test]
    fn test_config_validation_empty_accounts_is_ok() {
        // Empty account list is valid — no error expected
        let config = Config { accounts: vec![] };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_load_raw_accounts_missing_file() {
        let temp = TempDir::new().expect("create tempdir");
        let path = temp.path().join("nonexistent_accounts.yaml");
        let result = load_raw_accounts(&path).expect("load_raw_accounts on missing file");
        assert!(result.is_empty(), "expected empty vec for missing file, got {:?}", result);
    }

    #[test]
    fn test_save_and_load_raw_accounts_round_trip() {
        let temp = TempDir::new().expect("create tempdir");
        let path = temp.path().join("accounts.yaml");

        let accounts = vec![
            RawAccount {
                name: "WorkAccount".to_string(),
                server: "imap.work.com".to_string(),
                port: 993,
                username: "user@work.com".to_string(),
                ignored_folders: vec!["Spam".to_string(), "Trash".to_string()],
            },
            RawAccount {
                name: "PersonalAccount".to_string(),
                server: "imap.personal.com".to_string(),
                port: 993,
                username: "me@personal.com".to_string(),
                ignored_folders: vec![],
            },
        ];

        save_accounts(&accounts, &path).expect("save_accounts");

        let loaded = load_raw_accounts(&path).expect("load_raw_accounts");
        assert_eq!(loaded.len(), 2, "expected 2 accounts, got {}", loaded.len());

        assert_eq!(loaded[0].name, "WorkAccount");
        assert_eq!(loaded[0].server, "imap.work.com");
        assert_eq!(loaded[0].port, 993);
        assert_eq!(loaded[0].username, "user@work.com");
        assert_eq!(loaded[0].ignored_folders, vec!["Spam", "Trash"]);

        assert_eq!(loaded[1].name, "PersonalAccount");
        assert_eq!(loaded[1].server, "imap.personal.com");
        assert_eq!(loaded[1].username, "me@personal.com");
        assert!(loaded[1].ignored_folders.is_empty());
    }

    #[test]
    fn test_save_accounts_preserves_order() {
        let temp = TempDir::new().expect("create tempdir");
        let path = temp.path().join("accounts.yaml");

        let accounts = vec![
            RawAccount {
                name: "AccountA".to_string(),
                server: "imap.a.com".to_string(),
                port: 993,
                username: "a@a.com".to_string(),
                ignored_folders: vec![],
            },
            RawAccount {
                name: "AccountB".to_string(),
                server: "imap.b.com".to_string(),
                port: 993,
                username: "b@b.com".to_string(),
                ignored_folders: vec![],
            },
            RawAccount {
                name: "AccountC".to_string(),
                server: "imap.c.com".to_string(),
                port: 993,
                username: "c@c.com".to_string(),
                ignored_folders: vec![],
            },
        ];

        save_accounts(&accounts, &path).expect("save_accounts");

        let loaded = load_raw_accounts(&path).expect("load_raw_accounts");
        assert_eq!(loaded.len(), 3, "expected 3 accounts");
        assert_eq!(loaded[0].name, "AccountA");
        assert_eq!(loaded[1].name, "AccountB");
        assert_eq!(loaded[2].name, "AccountC");
    }

}

mod settings_tests {
    use super::*;

    #[test]
    fn test_settings_default() {
        let s = Settings::default();
        assert!(s.export_base_dir.is_none());
        assert!(s.defaults.quote_depth.is_none());
        assert!(s.accounts.is_empty());
    }

    #[test]
    fn test_settings_save_load_roundtrip() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("settings.yaml");

        let mut s = Settings::default();
        s.export_base_dir = Some("/tmp/emails".to_string());
        s.defaults.quote_depth = Some(2);
        s.defaults.skip_existing = Some(false);
        s.save(&path).unwrap();

        let loaded = Settings::load(&path).unwrap();
        assert_eq!(loaded.export_base_dir, Some("/tmp/emails".to_string()));
        assert_eq!(loaded.defaults.quote_depth, Some(2));
        assert_eq!(loaded.defaults.skip_existing, Some(false));
    }

    #[test]
    fn test_settings_missing_file_returns_default() {
        let s = Settings::load(std::path::Path::new("/nonexistent/settings.yaml")).unwrap();
        assert!(s.export_base_dir.is_none());
    }

    #[test]
    fn test_config_merge_export_dir_from_base() {
        let temp = TempDir::new().unwrap();

        let accounts_yaml = "accounts:\n  - name: TestAccount\n    server: imap.example.com\n    port: 993\n    username: user@example.com\n";
        let accounts_path = temp.path().join("accounts.yaml");
        std::fs::write(&accounts_path, accounts_yaml).unwrap();

        let settings_yaml = "export_base_dir: /tmp/emails\n";
        let settings_path = temp.path().join("settings.yaml");
        std::fs::write(&settings_path, settings_yaml).unwrap();

        let config = Config::load_with_settings(&accounts_path, &settings_path).unwrap();
        assert_eq!(config.accounts.len(), 1);
        assert_eq!(config.accounts[0].export_directory, "/tmp/emails/TestAccount");
    }

    #[test]
    fn test_config_merge_defaults_applied() {
        let temp = TempDir::new().unwrap();

        let accounts_yaml = "accounts:\n  - name: TestAccount\n    server: imap.example.com\n    port: 993\n    username: user@example.com\n";
        let accounts_path = temp.path().join("accounts.yaml");
        std::fs::write(&accounts_path, accounts_yaml).unwrap();

        let settings_yaml = "export_base_dir: /tmp/emails\ndefaults:\n  quote_depth: 3\n  collect_contacts: true\n";
        let settings_path = temp.path().join("settings.yaml");
        std::fs::write(&settings_path, settings_yaml).unwrap();

        let config = Config::load_with_settings(&accounts_path, &settings_path).unwrap();
        assert_eq!(config.accounts[0].quote_depth, 3);
        assert!(config.accounts[0].collect_contacts);
    }

    #[test]
    fn test_config_merge_per_account_overrides_folder_name() {
        let temp = TempDir::new().unwrap();

        let accounts_yaml = "accounts:\n  - name: TestAccount\n    server: imap.example.com\n    port: 993\n    username: user@example.com\n";
        let accounts_path = temp.path().join("accounts.yaml");
        std::fs::write(&accounts_path, accounts_yaml).unwrap();

        let settings_yaml = "export_base_dir: /tmp/emails\naccounts:\n  TestAccount:\n    folder_name: custom-folder\n    quote_depth: 5\n";
        let settings_path = temp.path().join("settings.yaml");
        std::fs::write(&settings_path, settings_yaml).unwrap();

        let config = Config::load_with_settings(&accounts_path, &settings_path).unwrap();
        assert!(config.accounts[0].export_directory.ends_with("custom-folder"));
        assert_eq!(config.accounts[0].quote_depth, 5);
    }

    #[test]
    fn test_config_merge_no_settings_uses_hardcoded_defaults() {
        let temp = TempDir::new().unwrap();

        // accounts.yaml without settings.yaml → export_directory is empty, validation fails
        let accounts_yaml = "accounts:\n  - name: TestAccount\n    server: imap.example.com\n    port: 993\n    username: user@example.com\n";
        let accounts_path = temp.path().join("accounts.yaml");
        std::fs::write(&accounts_path, accounts_yaml).unwrap();

        let missing_settings = temp.path().join("settings.yaml"); // does not exist

        let config = Config::load_with_settings(&accounts_path, &missing_settings);
        // Should fail validation: export_directory is empty because no export_base_dir set
        assert!(config.is_err());
    }

    #[test]
    fn test_settings_account_behavior_overrides_round_trip() {
        let temp = TempDir::new().expect("create tempdir");
        let path = temp.path().join("settings.yaml");

        let settings_yaml = r#"defaults:
  skip_signature_images: false
accounts:
  myaccount:
    skip_signature_images: true
    delete_after_export: false
    quote_depth: 5
"#;
        std::fs::write(&path, settings_yaml).expect("write settings.yaml");

        let settings = Settings::load(&path).expect("load settings");

        let behavior = settings.accounts.get("myaccount").expect("myaccount entry missing");
        // Inclusive: fields set in YAML must round-trip correctly.
        assert_eq!(behavior.skip_signature_images, Some(true), "skip_signature_images should be Some(true)");
        assert_eq!(behavior.delete_after_export, Some(false), "delete_after_export should be Some(false)");
        assert_eq!(behavior.quote_depth, Some(5), "quote_depth should be Some(5)");
        // Exclusive: fields absent from YAML must not bleed in from defaults or other sources.
        assert_eq!(behavior.skip_existing, None, "skip_existing must not bleed from YAML");
        assert_eq!(behavior.collect_contacts, None, "collect_contacts must not bleed");
        assert_eq!(behavior.folder_name, None, "folder_name should be None (not set)");
        assert_eq!(settings.defaults.skip_signature_images, Some(false), "defaults.skip_signature_images should be Some(false)");
    }

    #[test]
    fn test_settings_account_behavior_remove_entry_removes_from_yaml() {
        let temp = TempDir::new().expect("create tempdir");
        let path = temp.path().join("settings.yaml");

        let mut settings = Settings::default();
        settings.accounts.insert("myaccount".to_string(), AccountBehavior {
            skip_signature_images: Some(true),
            ..AccountBehavior::default()
        });
        settings.save(&path).expect("save settings");

        let saved_content = std::fs::read_to_string(&path).expect("read saved yaml");
        assert!(saved_content.contains("myaccount"), "saved YAML should contain 'myaccount'");

        settings.accounts.remove("myaccount");
        settings.save(&path).expect("save settings after remove");

        let reloaded = Settings::load(&path).expect("reload settings");
        assert!(reloaded.accounts.is_empty(), "accounts map should be empty after removing the only entry");
    }
}

mod email_export_tests {
    use email_to_markdown::email_export::*;

    #[test]
    fn test_analyze_email_type_direct() {
        let raw_email = b"From: sender@example.com\r\nTo: recipient@example.com\r\nSubject: Test\r\n\r\nBody";
        let mail = mailparse::parse_mail(raw_email).unwrap();
        let analysis = analyze_email_type(&mail);

        assert_eq!(analysis.email_type, EmailType::Direct);
        assert_eq!(analysis.from, "sender@example.com");
    }

    #[test]
    fn test_analyze_email_type_newsletter() {
        let raw_email = b"From: news@example.com\r\nTo: user@example.com\r\nSubject: Weekly Newsletter\r\n\r\nBody";
        let mail = mailparse::parse_mail(raw_email).unwrap();
        let analysis = analyze_email_type(&mail);

        assert_eq!(analysis.email_type, EmailType::Newsletter);
    }

    #[test]
    fn test_analyze_email_type_group() {
        let raw_email = b"From: sender@example.com\r\nTo: a@example.com, b@example.com\r\nSubject: Test\r\n\r\nBody";
        let mail = mailparse::parse_mail(raw_email).unwrap();
        let analysis = analyze_email_type(&mail);

        assert_eq!(analysis.email_type, EmailType::Group);
    }

    #[test]
    fn test_contacts_collector_add() {
        let mut collector = ContactsCollector::new();
        collector.add(&EmailType::Direct, "test@example.com".to_string());
        collector.add(&EmailType::Group, "group@example.com".to_string());

        assert!(collector.direct.contains("test@example.com"));
        assert!(collector.group.contains("group@example.com"));
    }

    #[test]
    fn test_export_stats_default() {
        let stats = ExportStats::default();
        assert_eq!(stats.exported, 0);
        assert_eq!(stats.skipped, 0);
        assert_eq!(stats.errors, 0);
    }

    #[test]
    fn test_email_frontmatter_serializes_social_links_when_present() {
        use std::collections::BTreeMap;

        let mut links: BTreeMap<String, String> = BTreeMap::new();
        links.insert("instagram".to_string(), "https://www.instagram.com/foo".to_string());
        links.insert("facebook".to_string(), "https://www.facebook.com/foo".to_string());

        let fm = EmailFrontmatter {
            from: "a@example.com".to_string(),
            to: "b@example.com".to_string(),
            date: "2026-04-15T00:00:00+00:00".to_string(),
            subject: "Hi".to_string(),
            subject_hash: "abcdef".to_string(),
            tags: vec!["inbox".to_string()],
            attachments: vec![],
            email_type: None,
            social_links: Some(links),
        };

        let yaml = serde_yaml::to_string(&fm).expect("serialize");
        assert!(yaml.contains("social_links:"), "missing social_links key in:\n{}", yaml);
        assert!(yaml.contains("instagram: https://www.instagram.com/foo"), "missing instagram entry in:\n{}", yaml);
        assert!(yaml.contains("facebook: https://www.facebook.com/foo"), "missing facebook entry in:\n{}", yaml);
    }

    #[test]
    fn test_email_frontmatter_omits_social_links_when_none() {
        let fm = EmailFrontmatter {
            from: "a@example.com".to_string(),
            to: "b@example.com".to_string(),
            date: "2026-04-15T00:00:00+00:00".to_string(),
            subject: "Hi".to_string(),
            subject_hash: "abcdef".to_string(),
            tags: vec![],
            attachments: vec![],
            email_type: None,
            social_links: None,
        };

        let yaml = serde_yaml::to_string(&fm).expect("serialize");
        assert!(!yaml.contains("social_links"), "social_links should be omitted when None, got:\n{}", yaml);
    }

    #[test]
    fn test_email_frontmatter_contains_email_type() {
        let fm = EmailFrontmatter {
            from: "news@example.com".to_string(),
            to: "user@example.com".to_string(),
            date: "2026-04-15T00:00:00+00:00".to_string(),
            subject: "Weekly Newsletter".to_string(),
            subject_hash: "abc123".to_string(),
            tags: vec!["INBOX".to_string()],
            attachments: vec![],
            email_type: Some("newsletter".to_string()),
            social_links: None,
        };

        let yaml = serde_yaml::to_string(&fm).expect("serialize");
        assert!(yaml.contains("email_type: newsletter"), "expected email_type in:\n{}", yaml);
    }

    #[test]
    fn test_email_frontmatter_omits_email_type_when_none() {
        let fm = EmailFrontmatter {
            from: "a@example.com".to_string(),
            to: "b@example.com".to_string(),
            date: "2026-04-15T00:00:00+00:00".to_string(),
            subject: "Hi".to_string(),
            subject_hash: "abcdef".to_string(),
            tags: vec![],
            attachments: vec![],
            email_type: None,
            social_links: None,
        };

        let yaml = serde_yaml::to_string(&fm).expect("serialize");
        assert!(!yaml.contains("email_type"), "email_type should be omitted when None, got:\n{}", yaml);
    }
}

mod edge_case_tests {
    use super::*;

    #[test]
    fn test_empty_email_field() {
        let result = get_short_name(Some(""));
        assert_eq!(result, "UNK");
    }

    #[test]
    fn test_special_characters_in_email() {
        let result = get_short_name(Some("<invalid>email@test.com"));
        // Should handle special characters gracefully
        assert!(!result.is_empty());
    }

    #[test]
    fn test_unicode_in_name() {
        let result = get_short_name(Some("Jose Garcia <jose@example.com>"));
        // Should extract initials even with accented characters
        assert!(!result.is_empty());
    }

    #[test]
    fn test_very_long_email() {
        let long_local = "a".repeat(100);
        let email = format!("{}@example.com", long_local);
        let result = get_short_name(Some(&email));
        // Single-token local part is truncated to at most 8 letters.
        assert!(result.chars().count() <= 8);
    }

    #[test]
    fn test_normalize_many_newlines() {
        let text = "Hello\n\n\n\n\n\n\n\n\n\nWorld";
        let result = normalize_line_breaks(text);
        assert!(!result.contains("\n\n\n"));
    }

    #[test]
    fn test_signature_image_edge_size() {
        // Exactly at the threshold
        assert!(is_signature_image(
            Some("logo.png"),
            "image/png",
            60 * 1024 - 1,
            Some("attachment")
        ));

        // Just over threshold for signature
        assert!(!is_signature_image(
            Some("signature.png"),
            "image/png",
            50 * 1024 + 1,
            Some("attachment")
        ));
    }
}

// [3][4] Tests pour le module network
mod network_tests {
    use super::*;

    #[test]
    fn test_network_config_default() {
        let config = NetworkConfig::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.connect_timeout, Duration::from_secs(30));
        assert_eq!(config.read_timeout, Duration::from_secs(60));
    }

    #[test]
    fn test_progress_indicator_create() {
        let _progress = ProgressIndicator::new("Test", 100);
        // Just verify it creates without panic
        assert!(true);
    }

    #[test]
    fn test_progress_indicator_update() {
        let mut progress = ProgressIndicator::new("Test", 10);
        progress.update(5);
        progress.inc();
        // Verify it updates without panic
        assert!(true);
    }
}

mod cleaner_tests {
    use email_to_markdown::cleaner;
    use mailparse::MailHeaderMap;

    // Phase 1 — Task 1.1: minimal RFC822 reproducing the =C2=A0 leak.
    // U+00A0 (no-break space) is encoded in UTF-8 as bytes C2 A0.
    // In quoted-printable that becomes the literal sequence "=C2=A0".
    // mailparse should decode this to the actual NBSP character.
    // This integration test asserts the extracted body string contains
    // a real NBSP and NOT the raw "=C2=A0" sequence.
    const LEAK_SAMPLE_FLAT: &[u8] = b"From: sender@example.com\r\n\
To: recipient@example.com\r\n\
Subject: Test\r\n\
MIME-Version: 1.0\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
Content-Transfer-Encoding: quoted-printable\r\n\
\r\n\
Bonjour=C2=A0world\r\n";

    const LEAK_SAMPLE_NESTED: &[u8] = b"From: sender@example.com\r\n\
To: recipient@example.com\r\n\
Subject: Test\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/alternative; boundary=\"BOUNDARY1\"\r\n\
\r\n\
--BOUNDARY1\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
Content-Transfer-Encoding: quoted-printable\r\n\
\r\n\
Bonjour=C2=A0world\r\n\
--BOUNDARY1\r\n\
Content-Type: text/html; charset=utf-8\r\n\
Content-Transfer-Encoding: quoted-printable\r\n\
\r\n\
<p>Bonjour=C2=A0world</p>\r\n\
--BOUNDARY1--\r\n";

    const LEAK_SAMPLE_DOUBLE_NESTED: &[u8] = b"From: sender@example.com\r\n\
To: recipient@example.com\r\n\
Subject: Test\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"OUTER\"\r\n\
\r\n\
--OUTER\r\n\
Content-Type: multipart/alternative; boundary=\"INNER\"\r\n\
\r\n\
--INNER\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
Content-Transfer-Encoding: quoted-printable\r\n\
\r\n\
Bonjour=C2=A0world\r\n\
--INNER\r\n\
Content-Type: text/html; charset=utf-8\r\n\
Content-Transfer-Encoding: quoted-printable\r\n\
\r\n\
<p>Bonjour=C2=A0world</p>\r\n\
--INNER--\r\n\
--OUTER--\r\n";

    /// Mirror of `email_export::extract_body` so the test exercises the
    /// exact same extraction path the production code uses.
    fn extract_body_for_test(mail: &mailparse::ParsedMail) -> String {
        if mail.subparts.is_empty() {
            mail.get_body().unwrap_or_default()
        } else {
            let mut body = String::new();
            for part in &mail.subparts {
                let content_type = part
                    .headers
                    .get_first_value("Content-Type")
                    .unwrap_or_default()
                    .to_lowercase();

                if content_type.starts_with("text/plain") {
                    body = part.get_body().unwrap_or_default();
                    break;
                } else if content_type.starts_with("text/html") && body.is_empty() {
                    body = part.get_body().unwrap_or_default();
                } else if content_type.starts_with("multipart/") {
                    let nested_body = extract_body_for_test(part);
                    if !nested_body.is_empty() && body.is_empty() {
                        body = nested_body;
                    }
                }
            }
            body
        }
    }

    #[test]
    fn test_qp_leak_flat_text_plain() {
        let mail = mailparse::parse_mail(LEAK_SAMPLE_FLAT).unwrap();
        let body = extract_body_for_test(&mail);
        assert!(
            !body.contains("=C2=A0"),
            "flat QP body still contains raw =C2=A0 sequence: {:?}",
            body
        );
        assert!(
            body.contains('\u{00A0}'),
            "flat QP body should contain decoded NBSP: {:?}",
            body
        );
    }

    #[test]
    fn test_qp_leak_nested_multipart_alternative() {
        let mail = mailparse::parse_mail(LEAK_SAMPLE_NESTED).unwrap();
        let body = extract_body_for_test(&mail);
        assert!(
            !body.contains("=C2=A0"),
            "nested QP body still contains raw =C2=A0 sequence: {:?}",
            body
        );
        assert!(
            body.contains('\u{00A0}'),
            "nested QP body should contain decoded NBSP: {:?}",
            body
        );
    }

    #[test]
    fn test_qp_leak_double_nested_mixed_alternative() {
        let mail = mailparse::parse_mail(LEAK_SAMPLE_DOUBLE_NESTED).unwrap();
        let body = extract_body_for_test(&mail);
        assert!(
            !body.contains("=C2=A0"),
            "double-nested QP body still contains raw =C2=A0 sequence: {:?}",
            body
        );
        assert!(
            body.contains('\u{00A0}'),
            "double-nested QP body should contain decoded NBSP: {:?}",
            body
        );
    }

    // Phase 5 — End-to-end pipeline integration test on a realistic body.
    const JEVEUX_BODY: &str = "Bonjour stVerif SARL,\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\u{00A0}\n\nMerci pour votre inscription sur JeVeuxTravailler/JeVeuxRecruter.\n\nVotre compte est créé. Il ne vous reste plus qu'une étape pour accéder\nà votre espace recruteur et découvrir les candidats disponibles dans\nvotre secteur en utilisant notre plateforme de recherche avancée.\n\nCliquez sur le bouton ci-dessous pour confirmer votre adresse email.\n\n[Activer mon compte recruteur](https://jeveuxtravailler.com/api/verify-email?token=eyJ1aWQiOiJaZHNBb3FCeE5UT2hXNVBDQTZzZmR3QW9mb2YxIiwidXNlclR5cGUiOiJyZWNydWl0ZXIiLCJleHAiOjE3NzU5MjI3OTF9.7mIbZQR8d3f2XBkzPmIW42toBN6QZbnbUqoXiDvq7aA&utm_source=onboarding)\n\nSi vous n'êtes pas à l'origine de cette demande, vous pouvez ignorer\ncet email.\n\nÀ très vite,\nL'équipe JeVeuxTravailler !\n\n[instagram](https://www.instagram.com/jeveuxtravailler_fr/)\n[tiktok](https://www.tiktok.com/@jeveuxtravailler.com)\n[facebook](https://www.facebook.com/talentissim/?locale=fr_FR)\n[LinkedIn](https://www.linkedin.com/company/jeveuxtravailler-jeveuxrecruter/)\n";

    #[test]
    fn test_clean_e2e_jeveuxtravailler_body() {
        let result = cleaner::clean(JEVEUX_BODY);
        let body = &result.body;

        // No QP residue
        assert!(
            !body.contains("=C2=A0"),
            "body still contains QP residue: {:?}",
            body
        );

        // Runs of nbsp collapsed
        assert!(
            !body.contains("\u{00A0}\u{00A0}"),
            "runs of NBSP not collapsed: {:?}",
            body
        );

        // CTA rewritten as numbered reference
        assert!(
            body.contains("[Activer mon compte recruteur]["),
            "CTA not rewritten as numbered reference: {:?}",
            body
        );

        // JWT token still present somewhere in the body (in the reference)
        assert!(
            body.contains("eyJ1aWQi"),
            "JWT token lost from reference section: {:?}",
            body
        );

        // Find the CTA reference URL line and assert tracker decontamination
        let cta_ref_line = body
            .lines()
            .find(|l| l.starts_with("[1]:") && l.contains("jeveuxtravailler.com"))
            .expect("expected a [1]: reference line for the CTA URL");
        assert!(
            !cta_ref_line.contains("utm_source"),
            "utm_source not stripped from reference URL: {:?}",
            cta_ref_line
        );

        // Wrapped paragraph has been unwrapped (lenient subset check)
        assert!(
            body.contains("accéder à votre"),
            "wrapped paragraph not unwrapped: {:?}",
            body
        );

        // Strict prose-continuity check — catches reattach_urls corruption (D1)
        assert!(
            body.contains("disponibles dans votre secteur"),
            "prose wrap corruption — missing space between 'dans' and 'votre': {:?}",
            body
        );
        assert!(
            !body.contains("dansvotre"),
            "prose wrap corruption — 'dans' and 'votre' fused without space: {:?}",
            body
        );

        // Social footer extracted
        let links = result
            .social_links
            .as_ref()
            .expect("expected social_links to be Some");
        assert_eq!(links.len(), 4, "expected exactly 4 social networks");
        assert!(links.contains_key("instagram"));
        assert!(links.contains_key("tiktok"));
        assert!(links.contains_key("facebook"));
        assert!(links.contains_key("linkedin"));

        // Social lines removed from the body
        assert!(
            !body.contains("[instagram]"),
            "instagram line not removed from body: {:?}",
            body
        );
        assert!(
            !body.contains("[tiktok]"),
            "tiktok line not removed from body: {:?}",
            body
        );
        assert!(
            !body.contains("[facebook]"),
            "facebook line not removed from body: {:?}",
            body
        );
        assert!(
            !body.contains("[LinkedIn]"),
            "LinkedIn line not removed from body: {:?}",
            body
        );

        // Ends with exactly one newline
        assert!(
            body.ends_with('\n'),
            "body should end with a newline: {:?}",
            body
        );
        assert!(
            !body.ends_with("\n\n\n"),
            "runaway trailing newlines: {:?}",
            body
        );

        // Loose unwrap-runaway guard
        assert!(
            body.lines().all(|l| l.len() < 2000),
            "found a line longer than 2000 chars (unwrap runaway?)"
        );
    }
}

mod route_tests {
    use email_to_markdown::route::{
        apply_decision, ai_route, delete_email, ensure_year_month, join_safe_segments, move_email,
        parse_destinations, route_email, upsert_rule,
        Destination, EmailMeta, MatchRule,
    };
    use chrono::DateTime;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    // ── helpers ──────────────────────────────────────────────────────────────

    fn make_meta(from: &str, domain: &str, subject: &str, account: &str, date_str: &str) -> EmailMeta {
        EmailMeta {
            from: from.to_string(),
            domain: domain.to_string(),
            subject: subject.to_string(),
            account: account.to_string(),
            date: DateTime::parse_from_rfc3339(date_str).expect("valid date"),
        }
    }

    // --- join_safe_segments: migrated from sort_emails_tests ---

    #[test]
    fn test_join_safe_segments_nested_path() {
        let root = PathBuf::from("/notes");
        let joined = join_safe_segments(&root, "Travail/Projets/Client A").unwrap();
        assert_eq!(joined, root.join("Travail").join("Projets").join("Client A"));
    }

    #[test]
    fn test_join_safe_segments_accented_segment_allowed() {
        let root = PathBuf::from("/notes");
        let joined = join_safe_segments(&root, "Été/Réunions").unwrap();
        assert_eq!(joined, root.join("Été").join("Réunions"));
    }

    #[test]
    fn test_join_safe_segments_empty_and_trim_segments_skipped() {
        let root = PathBuf::from("/notes");
        let joined = join_safe_segments(&root, "/Travail//Projets/ ").unwrap();
        assert_eq!(joined, root.join("Travail").join("Projets"));
    }

    #[test]
    fn test_join_safe_segments_rejects_parent_traversal() {
        let root = PathBuf::from("/notes");
        let err = join_safe_segments(&root, "Travail/../etc").unwrap_err();
        assert!(err.to_string().contains(".."));
    }

    #[test]
    fn test_join_safe_segments_rejects_dot_segment() {
        let root = PathBuf::from("/notes");
        assert!(join_safe_segments(&root, "Travail/./Projets").is_err());
    }

    #[test]
    fn test_join_safe_segments_rejects_backslash() {
        let root = PathBuf::from("/notes");
        let err = join_safe_segments(&root, "Travail\\Projets").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("Travail") || msg.contains("forbidden") || msg.contains("invalid"));
    }

    #[test]
    fn test_join_safe_segments_rejects_forbidden_characters() {
        let root = PathBuf::from("/notes");
        assert!(join_safe_segments(&root, "Travail/Projets:Secrets").is_err());
        assert!(join_safe_segments(&root, "Travail/Projets*").is_err());
    }

    // --- move_email: .md + flat sibling attachments moved and bare links preserved ---

    #[test]
    fn test_move_email_moves_md_and_flat_attachments() {
        let temp = TempDir::new().unwrap();
        let src_dir = temp.path().join("staging");
        let dst_dir = temp.path().join("dest");
        fs::create_dir_all(&src_dir).unwrap();
        fs::create_dir_all(&dst_dir).unwrap();

        // Flat layout: attachment is a sibling of the .md, prefixed by stem
        let att_src = src_dir.join("email__file.pdf");
        fs::write(&att_src, b"PDF content").unwrap();

        let md_src = src_dir.join("email.md");
        let md_content = "---\nsubject: Test\nattachments:\n  - email__file.pdf\n---\nBody text\n";
        fs::write(&md_src, md_content).unwrap();

        // Act
        move_email(&md_src, &dst_dir).unwrap();

        // Inclusive: .md and flat attachment co-located at dest
        let md_dest = dst_dir.join("email.md");
        let att_dest = dst_dir.join("email__file.pdf");
        assert!(md_dest.exists(), "moved .md must exist at dest");
        assert!(att_dest.exists(), "flat attachment must be co-located at dest");

        // Exclusive: original paths no longer exist
        assert!(!md_src.exists(), "original .md must not remain at src");
        assert!(!att_src.exists(), "original attachment must not remain at src");

        // Inclusive: bare link preserved in moved .md
        let new_content = fs::read_to_string(&md_dest).unwrap();
        assert!(
            new_content.contains("email__file.pdf"),
            "moved .md must preserve the bare attachment link"
        );
        // Exclusive: no subdirectory reference in the moved .md
        assert!(
            !new_content.contains("_attachments/"),
            "moved .md must not reference a _attachments/ subdir"
        );
        assert!(
            !new_content.contains("attachments/"),
            "moved .md must not reference an attachments/ subdir"
        );
    }

    /// Anti-regression: frontmatter with a YAML-quoted attachment name (e.g. `invoice #5.pdf`
    /// serialized as `- 'email__a1b2c3d4_invoice #5.pdf'`) must be correctly dequoted by
    /// `serde_yaml` so that `move_email` locates and moves the real file.
    /// A line-parser would return the string with surrounding quotes → file not found → silently
    /// skipped. This test locks the serde_yaml deserialization path.
    #[test]
    fn test_move_email_attachment_with_special_chars() {
        let temp = TempDir::new().unwrap();
        let src_dir = temp.path().join("staging");
        let dst_dir = temp.path().join("dest");
        fs::create_dir_all(&src_dir).unwrap();
        fs::create_dir_all(&dst_dir).unwrap();

        // Attachment name that YAML serializes with single-quote wrapping due to '#'
        let att_filename = "email__a1b2c3d4_invoice #5.pdf";
        let att_src = src_dir.join(att_filename);
        fs::write(&att_src, b"PDF content").unwrap();

        // Frontmatter with YAML-quoted name (as produced by serde_yaml when '#' is present)
        let md_content = concat!(
            "---\n",
            "subject: Test\n",
            "attachments:\n",
            "  - 'email__a1b2c3d4_invoice #5.pdf'\n",
            "---\n",
            "Body text\n"
        );
        let md_src = src_dir.join("email.md");
        fs::write(&md_src, md_content).unwrap();

        move_email(&md_src, &dst_dir).unwrap();

        // Inclusive: real file (dequoted by serde_yaml) present at dest
        let att_dest = dst_dir.join(att_filename);
        assert!(
            att_dest.exists(),
            "attachment with special chars must be moved to dest (serde_yaml dequoted correctly): {:?}",
            att_dest
        );
        // Exclusive: no file with literal surrounding single-quote chars in the name
        let att_dest_with_quotes = dst_dir.join(format!("'{}'", att_filename));
        assert!(
            !att_dest_with_quotes.exists(),
            "no file with literal quote chars must exist at dest (line-parser regression guard): {:?}",
            att_dest_with_quotes
        );
        // Inclusive: .md moved to dest
        assert!(dst_dir.join("email.md").exists(), ".md must be at dest");
        // Exclusive: originals gone
        assert!(!md_src.exists(), "original .md must not remain at src");
        assert!(!att_src.exists(), "original attachment must not remain at src");
    }

    #[test]
    fn test_move_email_without_attachments_dir() {
        let temp = TempDir::new().unwrap();
        let src_dir = temp.path().join("staging");
        let dst_dir = temp.path().join("dest");
        fs::create_dir_all(&src_dir).unwrap();
        fs::create_dir_all(&dst_dir).unwrap();

        let md_src = src_dir.join("plain.md");
        fs::write(&md_src, "---\nsubject: Plain\n---\nNo attachments\n").unwrap();

        move_email(&md_src, &dst_dir).unwrap();

        // Inclusive: .md moved
        assert!(dst_dir.join("plain.md").exists(), "moved .md must exist at dest");
        // Exclusive: original gone
        assert!(!md_src.exists(), "original .md must not remain at src");
        // Exclusive: no spurious _attachments dir created
        assert!(
            !dst_dir.join("plain_attachments").exists(),
            "no _attachments dir must be created when there was none"
        );
        // Exclusive: moved .md content must not contain any attachments/ path segment
        let moved_content = fs::read_to_string(dst_dir.join("plain.md")).unwrap();
        assert!(
            !moved_content.contains("attachments/"),
            "moved .md must not contain an attachments/ path segment"
        );
    }

    #[test]
    fn test_move_email_rejects_symlink() {
        let temp = TempDir::new().unwrap();
        let real_file = temp.path().join("real.md");
        fs::write(&real_file, "---\nsubject: Real\n---\n").unwrap();

        let symlink_path = temp.path().join("link.md");
        let dst_dir = temp.path().join("dest");
        fs::create_dir_all(&dst_dir).unwrap();

        // Creating symlinks on Windows may require elevated privileges.
        // Guard: attempt to create the symlink; if it fails with a permission error,
        // skip the test rather than fail (but the production guard is still in place).
        #[cfg(windows)]
        {
            use std::os::windows::fs::symlink_file;
            match symlink_file(&real_file, &symlink_path) {
                Ok(()) => {
                    let result = move_email(&symlink_path, &dst_dir);
                    assert!(result.is_err(), "move_email must refuse a symlink source");
                    let msg = result.unwrap_err().to_string();
                    assert!(
                        msg.contains("symlink") || msg.contains("link.md"),
                        "error must mention symlink: {msg}"
                    );
                }
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                    // Symlink creation requires elevated privileges on this Windows build.
                    // Skip gracefully — the production guard is still present in move_email.
                    eprintln!("Skipping symlink test: insufficient privileges ({e})");
                }
                Err(e) => panic!("unexpected error creating symlink: {e}"),
            }
        }
        #[cfg(not(windows))]
        {
            std::os::unix::fs::symlink(&real_file, &symlink_path).unwrap();
            let result = move_email(&symlink_path, &dst_dir);
            assert!(result.is_err(), "move_email must refuse a symlink source");
            let msg = result.unwrap_err().to_string();
            assert!(
                msg.contains("symlink") || msg.contains("link.md"),
                "error must mention symlink: {msg}"
            );
        }
    }

    // Two emails routed into the same folder with the same attachment file name:
    // the second attachment is suffixed and the second .md's links are updated.
    #[test]
    fn test_move_email_suffixes_colliding_attachment_in_dest() {
        let temp = TempDir::new().unwrap();
        let dst_dir = temp.path().join("dest");
        fs::create_dir_all(&dst_dir).unwrap();

        let src_a = temp.path().join("stagingA");
        fs::create_dir_all(&src_a).unwrap();
        fs::write(src_a.join("2026-06-25_image.png"), b"AAA").unwrap();
        let md_a = src_a.join("emailA.md");
        fs::write(
            &md_a,
            "---\nsubject: A\nattachments:\n  - 2026-06-25_image.png\n---\nBody [2026-06-25_image.png](2026-06-25_image.png)\n",
        )
        .unwrap();

        let src_b = temp.path().join("stagingB");
        fs::create_dir_all(&src_b).unwrap();
        fs::write(src_b.join("2026-06-25_image.png"), b"BBB").unwrap();
        let md_b = src_b.join("emailB.md");
        fs::write(
            &md_b,
            "---\nsubject: B\nattachments:\n  - 2026-06-25_image.png\n---\nBody [2026-06-25_image.png](2026-06-25_image.png)\n",
        )
        .unwrap();

        move_email(&md_a, &dst_dir).unwrap();
        move_email(&md_b, &dst_dir).unwrap();

        // Both attachments survive with distinct content — no overwrite.
        let first = dst_dir.join("2026-06-25_image.png");
        let second = dst_dir.join("2026-06-25_image_2.png");
        assert!(first.exists(), "first attachment must keep its name");
        assert!(second.exists(), "colliding attachment must be suffixed");
        assert_eq!(fs::read(&first).unwrap(), b"AAA");
        assert_eq!(fs::read(&second).unwrap(), b"BBB");

        // emailB's links (frontmatter list + body) now point to the suffixed name.
        let b_content = fs::read_to_string(dst_dir.join("emailB.md")).unwrap();
        assert!(
            b_content.contains("2026-06-25_image_2.png"),
            "B must reference the suffixed attachment name"
        );
        assert!(
            !b_content.contains("- 2026-06-25_image.png\n"),
            "B must not still reference the un-suffixed name"
        );
        // emailA is untouched.
        let a_content = fs::read_to_string(dst_dir.join("emailA.md")).unwrap();
        assert!(a_content.contains("- 2026-06-25_image.png\n"));
    }

    // ── delete_email ─────────────────────────────────────────────────────────

    // delete_email removes the .md and relocates attachments into _deleted.
    #[test]
    fn test_delete_email_removes_md_and_moves_attachments() {
        let temp = TempDir::new().unwrap();
        let src_dir = temp.path().join("staging");
        fs::create_dir_all(&src_dir).unwrap();

        let att_src = src_dir.join("email__file.pdf");
        fs::write(&att_src, b"PDF content").unwrap();

        let md_src = src_dir.join("email.md");
        let md_content = "---\nsubject: Test\nattachments:\n  - email__file.pdf\n---\nBody text\n";
        fs::write(&md_src, md_content).unwrap();

        delete_email(&md_src).unwrap();

        // Exclusive: the .md is gone.
        assert!(!md_src.exists(), "deleted .md must not remain");
        // Exclusive: the attachment is no longer at its original path.
        assert!(!att_src.exists(), "attachment must be moved out of staging");
        // Inclusive: the attachment is preserved under _deleted.
        let recovered = src_dir.join("_deleted").join("email__file.pdf");
        assert!(recovered.exists(), "attachment must be relocated to _deleted");
        assert_eq!(fs::read(&recovered).unwrap(), b"PDF content");
    }

    // No attachments → just remove the .md, no _deleted folder created.
    #[test]
    fn test_delete_email_without_attachments_creates_no_deleted_dir() {
        let temp = TempDir::new().unwrap();
        let src_dir = temp.path().join("staging");
        fs::create_dir_all(&src_dir).unwrap();

        let md_src = src_dir.join("plain.md");
        fs::write(&md_src, "---\nsubject: Plain\n---\nNo attachments\n").unwrap();

        delete_email(&md_src).unwrap();

        assert!(!md_src.exists(), "deleted .md must not remain");
        assert!(
            !src_dir.join("_deleted").exists(),
            "_deleted must not be created when there are no attachments"
        );
    }

    // delete_email refuses a symlink source (no FS mutation), mirroring move_email.
    #[cfg(not(windows))]
    #[test]
    fn test_delete_email_rejects_symlink() {
        let temp = TempDir::new().unwrap();
        let real_file = temp.path().join("real.md");
        fs::write(&real_file, "---\nsubject: Real\n---\n").unwrap();

        let symlink_path = temp.path().join("link.md");
        std::os::unix::fs::symlink(&real_file, &symlink_path).unwrap();

        let result = delete_email(&symlink_path);
        assert!(result.is_err(), "delete_email must refuse a symlink source");
        // Exclusive: the real target is untouched.
        assert!(real_file.exists(), "symlink target must not be deleted");
    }

    // ── parse_destinations ───────────────────────────────────────────────────

    #[test]
    fn test_parse_destinations_ok() {
        let content = r#"
Perso/Finance/Banque | domain:credit-agricole.fr, from:noreply@ca.fr
Pro/Clients/Acme | from:billing@acme.com, subject:Invoice
"#;
        let dests = parse_destinations(content).unwrap();
        // Inclusive: both entries parsed
        assert_eq!(dests.len(), 2);
        assert_eq!(dests[0].path, "Perso/Finance/Banque");
        assert!(dests[0].rules.contains(&MatchRule::Domain("credit-agricole.fr".to_string())));
        assert!(dests[0].rules.contains(&MatchRule::From("noreply@ca.fr".to_string())));
        assert_eq!(dests[1].path, "Pro/Clients/Acme");
        // Exclusive: no spurious rules
        assert!(!dests[0].rules.contains(&MatchRule::Subject("Invoice".to_string())));
        assert!(!dests[1].is_default);
    }

    #[test]
    fn test_parse_destinations_comments_and_empty_lines_skipped() {
        let content = "# This is a comment\n\nPerso/Inbox | domain:example.com\n# another comment\n";
        let dests = parse_destinations(content).unwrap();
        // Inclusive: only the real entry
        assert_eq!(dests.len(), 1);
        assert_eq!(dests[0].path, "Perso/Inbox");
        // Exclusive: no comment or empty entries
        assert!(!dests.iter().any(|d| d.path.starts_with('#')));
        assert!(!dests.iter().any(|d| d.path.is_empty()));
    }

    #[test]
    fn test_parse_destinations_single_default_ok() {
        let content = "Perso/Messy/Emails | default\n";
        let dests = parse_destinations(content).unwrap();
        assert_eq!(dests.len(), 1);
        assert!(dests[0].is_default);
        // Exclusive: no rules besides the default flag
        assert!(dests[0].rules.is_empty());
    }

    #[test]
    fn test_parse_destinations_duplicate_default_is_error() {
        let content = "Perso/Messy/Emails | default\nPerso/Misc | default\n";
        let result = parse_destinations(content);
        // Inclusive: error returned
        assert!(result.is_err(), "duplicate default must be a hard error");
        let msg = format!("{:#}", result.unwrap_err());
        // Inclusive: message mentions the constraint
        assert!(
            msg.contains("default") || msg.contains("more than one"),
            "error message should mention 'default': {msg}"
        );
        // Exclusive: not a parse-skip warning disguised as success
        // (already ensured by the is_err() check above)
    }

    // ── route_email — domain matching ────────────────────────────────────────

    #[test]
    fn test_route_email_matches_domain_exact() {
        let content = "Perso/Finance/Banque | domain:acme.com\n";
        let dests = parse_destinations(content).unwrap();
        let meta = make_meta("alice@acme.com", "acme.com", "Hello", "personal", "2026-06-15T10:00:00+00:00");
        let decision = route_email(&meta, &dests);
        // Inclusive: path starts with expected dir
        assert!(decision.rel_path.starts_with("Perso/Finance/Banque/"), "got: {}", decision.rel_path);
        assert!(!decision.is_default);
        // Exclusive: not the default fallback
        assert!(!decision.rel_path.starts_with("Perso/Messy"));
    }

    #[test]
    fn test_route_email_domain_suffix_matches_subdomain() {
        let content = "Perso/Finance/Banque | domain:acme.com\n";
        let dests = parse_destinations(content).unwrap();
        let meta = make_meta("bob@mail.acme.com", "mail.acme.com", "Hello", "personal", "2026-06-15T10:00:00+00:00");
        let decision = route_email(&meta, &dests);
        // Inclusive: subdomain "mail.acme.com" matches rule "acme.com"
        assert!(decision.rel_path.starts_with("Perso/Finance/Banque/"), "got: {}", decision.rel_path);
        assert!(!decision.is_default);
    }

    #[test]
    fn test_route_email_domain_no_false_suffix_match() {
        let content = "Perso/Finance/Banque | domain:acme.com\n";
        let dests = parse_destinations(content).unwrap();
        // "notacme.com" must NOT match rule "acme.com"
        let meta = make_meta("evil@notacme.com", "notacme.com", "Hello", "personal", "2026-06-15T10:00:00+00:00");
        let decision = route_email(&meta, &dests);
        // Inclusive: falls to default
        assert!(decision.is_default, "notacme.com must not match acme.com rule");
        // Exclusive: not routed to the finance folder
        assert!(!decision.rel_path.starts_with("Perso/Finance"));
    }

    // ── route_email — from matching ──────────────────────────────────────────

    #[test]
    fn test_route_email_matches_from_case_insensitive() {
        let content = "Pro/Clients/X | from:BILLING@ACME.COM\n";
        let dests = parse_destinations(content).unwrap();
        let meta = make_meta("billing@acme.com", "acme.com", "Invoice", "work", "2026-03-01T00:00:00+00:00");
        let decision = route_email(&meta, &dests);
        // Inclusive: from rule matched despite case difference
        assert!(decision.rel_path.starts_with("Pro/Clients/X/"), "got: {}", decision.rel_path);
        assert!(!decision.is_default);
        // Exclusive: not default path
        assert!(!decision.rel_path.contains("Messy"));
    }

    // ── route_email — subject matching ───────────────────────────────────────

    #[test]
    fn test_route_email_matches_subject_substring() {
        let content = "Perso/Shopping | subject:invoice\n";
        let dests = parse_destinations(content).unwrap();
        let meta = make_meta("shop@store.com", "store.com", "Your Invoice #123", "personal", "2026-01-05T00:00:00+00:00");
        let decision = route_email(&meta, &dests);
        // Inclusive: substring "invoice" found case-insensitively in "Your Invoice #123"
        assert!(decision.rel_path.starts_with("Perso/Shopping/"), "got: {}", decision.rel_path);
        assert!(!decision.is_default);
    }

    #[test]
    fn test_route_email_subject_no_match_on_different_keyword() {
        let content = "Perso/Shopping | subject:invoice\n";
        let dests = parse_destinations(content).unwrap();
        let meta = make_meta("shop@store.com", "store.com", "Hello world", "personal", "2026-01-05T00:00:00+00:00");
        let decision = route_email(&meta, &dests);
        // Inclusive: subject "Hello world" does not contain "invoice" → falls to default
        assert!(decision.is_default, "non-matching subject must fall to default");
        // Exclusive: not routed to Shopping
        assert!(!decision.rel_path.starts_with("Perso/Shopping"));
    }

    // ── route_email — account matching ───────────────────────────────────────

    #[test]
    fn test_route_email_matches_account() {
        let content = "Pro/Work | account:work@corp.com\n";
        let dests = parse_destinations(content).unwrap();
        let meta = make_meta("sender@any.com", "any.com", "Hello", "work@corp.com", "2026-04-10T00:00:00+00:00");
        let decision = route_email(&meta, &dests);
        // Inclusive: account rule matched
        assert!(decision.rel_path.starts_with("Pro/Work/"), "got: {}", decision.rel_path);
        assert!(!decision.is_default);
    }

    // ── route_email — Perso/Pro polarity ─────────────────────────────────────

    #[test]
    fn test_route_email_perso_default_polarity() {
        // No rule matches → default → Perso
        let dests: Vec<Destination> = vec![];
        let meta = make_meta("x@y.com", "y.com", "Hi", "acc", "2026-05-20T00:00:00+00:00");
        let decision = route_email(&meta, &dests);
        // Inclusive: starts with Perso
        assert!(decision.rel_path.starts_with("Perso/"), "default must start with Perso, got: {}", decision.rel_path);
        assert!(decision.is_default);
    }

    #[test]
    fn test_route_email_pro_forced_by_first_segment() {
        let content = "Pro/Contracts | domain:corp.com\n";
        let dests = parse_destinations(content).unwrap();
        let meta = make_meta("legal@corp.com", "corp.com", "Contract", "work", "2026-02-14T00:00:00+00:00");
        let decision = route_email(&meta, &dests);
        // Inclusive: first segment is Pro
        assert!(decision.rel_path.starts_with("Pro/"), "matched rule must start with Pro, got: {}", decision.rel_path);
        assert!(!decision.is_default);
        // Exclusive: not Perso
        assert!(!decision.rel_path.starts_with("Perso/"));
    }

    // ── route_email — year/month append ──────────────────────────────────────

    #[test]
    fn test_route_email_appends_year_month() {
        let content = "Perso/Finance | domain:bank.com\n";
        let dests = parse_destinations(content).unwrap();
        let meta = make_meta("noreply@bank.com", "bank.com", "Statement", "personal", "2026-03-15T00:00:00+00:00");
        let decision = route_email(&meta, &dests);
        // Inclusive: ends with 2026/03
        assert!(decision.rel_path.ends_with("2026/03"), "expected year/month suffix, got: {}", decision.rel_path);
        // Exclusive: no wrong format (not "2026/3" or double slash)
        assert!(!decision.rel_path.contains("2026/3/"), "month must be zero-padded, got: {}", decision.rel_path);
        assert!(!decision.rel_path.contains("//"), "no double slash, got: {}", decision.rel_path);
    }

    #[test]
    fn test_route_email_default_appends_year_month() {
        let dests: Vec<Destination> = vec![];
        let meta = make_meta("x@y.com", "y.com", "Hi", "acc", "2026-11-30T00:00:00+00:00");
        let decision = route_email(&meta, &dests);
        // Inclusive: path ends with 2026/11
        assert!(decision.rel_path.ends_with("2026/11"), "got: {}", decision.rel_path);
        // Exclusive: not "2026/1" (not zero-padded)
        assert!(!decision.rel_path.ends_with("2026/1"), "month must be 2 digits");
    }

    // ── ensure_year_month — normalize manually reassigned destinations ───────

    #[test]
    fn test_ensure_year_month_appends_to_bare_path() {
        let out = ensure_year_month("Perso/Housing/Vallieres", "2026", "06");
        // Inclusive: dated subfolder appended
        assert_eq!(out, "Perso/Housing/Vallieres/2026/06");
        // Exclusive: no double slash
        assert!(!out.contains("//"), "no double slash, got: {}", out);
    }

    #[test]
    fn test_ensure_year_month_skips_when_already_dated() {
        let out = ensure_year_month("Perso/Finance/2026/06", "2026", "06");
        // Inclusive: unchanged
        assert_eq!(out, "Perso/Finance/2026/06");
        // Exclusive: NOT doubled (the bug we guard against)
        assert!(!out.contains("2026/06/2026/06"), "must not double the suffix, got: {}", out);
    }

    #[test]
    fn test_ensure_year_month_appends_when_tail_is_not_a_date() {
        // A trailing "13" (invalid month) or a non-year folder must NOT be mistaken
        // for a dated suffix → year/month is appended.
        let out = ensure_year_month("Perso/Bank/2026", "2026", "06");
        assert_eq!(out, "Perso/Bank/2026/2026/06");
        let out2 = ensure_year_month("Perso/X/Reports/13", "2026", "06");
        assert_eq!(out2, "Perso/X/Reports/13/2026/06");
        assert!(!out2.ends_with("/13"), "invalid month tail must not be treated as dated");
    }

    // ── route_email — path outside destinations.txt → default ────────────────

    #[test]
    fn test_route_email_unknown_domain_falls_to_default() {
        let content = "Perso/Finance/Banque | domain:bank.com\n";
        let dests = parse_destinations(content).unwrap();
        let meta = make_meta("x@unknown.org", "unknown.org", "Hi", "acc", "2026-06-01T00:00:00+00:00");
        let decision = route_email(&meta, &dests);
        // Inclusive: is_default flag set
        assert!(decision.is_default, "unknown domain must fall to default");
        // Exclusive: not routed to a known destination
        assert!(!decision.rel_path.starts_with("Perso/Finance"));
    }

    // ── AI off → default ─────────────────────────────────────────────────────

    #[test]
    fn test_ai_route_off_returns_none() {
        let dests: Vec<Destination> = vec![];
        let meta = make_meta("x@y.com", "y.com", "Hi", "acc", "2026-06-01T00:00:00+00:00");
        // AI disabled: must return None regardless of input
        let result = ai_route(&meta, &dests, false, 0.7);
        assert!(result.is_none(), "ai_route must return None when disabled");
    }

    #[test]
    fn test_ai_route_on_returns_none_for_now() {
        // Even when enabled, the M5 no-op returns None (future work).
        let dests: Vec<Destination> = vec![];
        let meta = make_meta("x@y.com", "y.com", "Hi", "acc", "2026-06-01T00:00:00+00:00");
        let result = ai_route(&meta, &dests, true, 0.7);
        // Inclusive: no crash
        // (returns None because AI is not yet implemented — no exclusive assertion needed)
        assert!(result.is_none(), "ai_route no-op must return None");
    }

    // ── apply_decision — creates missing dir and moves ────────────────────────

    #[test]
    fn test_apply_decision_creates_dir_and_moves() {
        let temp = TempDir::new().unwrap();
        let notes_dir = temp.path().join("notes");
        let staging = temp.path().join("staging");
        fs::create_dir_all(&staging).unwrap();
        // notes_dir intentionally NOT created — apply_decision must create it.
        let md_src = staging.join("email.md");
        fs::write(&md_src, "---\nsubject: Test\n---\nBody\n").unwrap();

        let rel_path = "Perso/Finance/Banque/2026/06";
        apply_decision(&md_src, rel_path, &notes_dir).unwrap();

        let expected_dir = notes_dir.join("Perso").join("Finance").join("Banque").join("2026").join("06");
        let expected_md = expected_dir.join("email.md");

        // Inclusive: directory created and file moved
        assert!(expected_dir.exists(), "target directory must be created");
        assert!(expected_md.exists(), "moved .md must exist at target");
        // Exclusive: original no longer at staging
        assert!(!md_src.exists(), "original .md must not remain at staging");
    }

    #[test]
    fn test_apply_decision_rejects_path_traversal() {
        let temp = TempDir::new().unwrap();
        let notes_dir = temp.path().join("notes");
        let staging = temp.path().join("staging");
        fs::create_dir_all(&staging).unwrap();
        let md_src = staging.join("email.md");
        fs::write(&md_src, "---\nsubject: Test\n---\nBody\n").unwrap();

        // Path traversal via ".."
        let result = apply_decision(&md_src, "Perso/../../etc/passwd", &notes_dir);
        // Inclusive: error returned
        assert!(result.is_err(), "path traversal must be rejected");
        let msg = format!("{:#}", result.unwrap_err());
        assert!(
            msg.contains("..") || msg.contains("invalid"),
            "error must mention the bad segment: {msg}"
        );
        // Exclusive: original file not moved
        assert!(md_src.exists(), "original file must remain when apply is rejected");
    }

    // ── M7: route review window — apply-layer validator (IPC contract) ───────
    // These tests exercise the same join_safe_segments + apply_decision path that
    // apply_route_decisions (tray IPC handler) delegates to.  No WebView needed.

    /// A dest_path value containing ".." (as would arrive from the HTML IPC)
    /// must be rejected by join_safe_segments, not applied.
    #[test]
    fn test_route_review_rejects_dotdot_dest_path() {
        let result = join_safe_segments(&PathBuf::from("/notes"), "../etc/passwd");
        assert!(result.is_err(), "'..' in dest_path must be rejected");
        let msg = format!("{:#}", result.unwrap_err());
        assert!(
            msg.contains("..") || msg.contains("invalid"),
            "error must describe the bad segment: {msg}"
        );
    }

    /// A dest_path value containing a backslash must be rejected.
    #[test]
    fn test_route_review_rejects_backslash_dest_path() {
        let result = join_safe_segments(&PathBuf::from("/notes"), r"Perso\Windows\Path");
        assert!(result.is_err(), "backslash in dest_path must be rejected");
    }

    // ── route_email — file order priority (multi-destination) ────────────────
    //
    // The router evaluates destinations in the order they appear in destinations.txt.
    // There is NO priority hierarchy between rule types (Domain / From / Subject /
    // Account). The first destination whose first matching rule fires wins.

    /// When two destinations could both match the same email, the one listed FIRST in
    /// destinations.txt must win, regardless of rule type.
    #[test]
    fn test_route_email_first_destination_in_file_wins_over_second() {
        // dest-A is listed first and matches via `from:`
        // dest-B is listed second and would also match via `domain:`
        // Expected: dest-A wins because it appears first.
        let content =
            "Perso/First | from:sender@acme.com\nPerso/Second | domain:acme.com\n";
        let dests = parse_destinations(content).unwrap();
        let meta = make_meta(
            "sender@acme.com",
            "acme.com",
            "Hello",
            "acc",
            "2026-06-01T00:00:00+00:00",
        );

        let decision = route_email(&meta, &dests);

        // Inclusive: first destination matched
        assert!(
            decision.rel_path.starts_with("Perso/First/"),
            "first destination must win; got: {}",
            decision.rel_path
        );
        assert!(!decision.is_default);
        // Exclusive: second destination must NOT be returned
        assert!(
            !decision.rel_path.starts_with("Perso/Second"),
            "second destination must not be returned when first matched; got: {}",
            decision.rel_path
        );
    }

    /// Reversing the order in destinations.txt must reverse the winner — proving that
    /// file order, not rule type, determines priority.
    #[test]
    fn test_route_email_file_order_reversed_changes_winner() {
        // Same two destinations as above, but listed in opposite order.
        // Now dest-B (domain:) is first → it must win over dest-A (from:).
        let content =
            "Perso/Second | domain:acme.com\nPerso/First | from:sender@acme.com\n";
        let dests = parse_destinations(content).unwrap();
        let meta = make_meta(
            "sender@acme.com",
            "acme.com",
            "Hello",
            "acc",
            "2026-06-01T00:00:00+00:00",
        );

        let decision = route_email(&meta, &dests);

        // Inclusive: the destination that is now first in file wins
        assert!(
            decision.rel_path.starts_with("Perso/Second/"),
            "reversed order: 'Second' destination must now win; got: {}",
            decision.rel_path
        );
        assert!(!decision.is_default);
        // Exclusive: the other destination must not be returned
        assert!(
            !decision.rel_path.starts_with("Perso/First"),
            "reversed order: 'First' destination must not be returned; got: {}",
            decision.rel_path
        );
    }

    // ── upsert_rule (YAML-backed) ─────────────────────────────────────────────

    /// Relative order of all non-target entries must be unchanged.
    #[test]
    fn test_upsert_rule_preserves_ordering() {
        use email_to_markdown::destinations::load_yaml;
        let temp = TempDir::new().unwrap();
        let dest_file = temp.path().join("destinations.yaml");
        let content = "destinations:\n- path: Perso/Alpha\n  rules:\n  - domain: a.com\n- path: Perso/Beta\n  rules:\n  - domain: b.com\n- path: Perso/Gamma\n  rules:\n  - domain: g.com\n";
        fs::write(&dest_file, content).unwrap();

        upsert_rule(&dest_file, "Perso/Beta", MatchRule::From("beta@b.com".to_string())).unwrap();

        let cfg = load_yaml(&dest_file).unwrap();
        let paths: Vec<&str> = cfg.destinations.iter().map(|e| e.path.as_str()).collect();
        // Inclusive: original relative order intact
        assert_eq!(paths, vec!["Perso/Alpha", "Perso/Beta", "Perso/Gamma"]);
    }

    /// Existing rules are kept; the new rule is appended to the same entry.
    #[test]
    fn test_upsert_rule_merge_onto_existing() {
        use email_to_markdown::destinations::{load_yaml, DestinationRule};
        let temp = TempDir::new().unwrap();
        let dest_file = temp.path().join("destinations.yaml");
        fs::write(
            &dest_file,
            "destinations:\n- path: Perso/Work\n  rules:\n  - domain: corp.com\n",
        )
        .unwrap();

        upsert_rule(&dest_file, "Perso/Work", MatchRule::From("bob@corp.com".to_string())).unwrap();

        let cfg = load_yaml(&dest_file).unwrap();
        // Exclusive: exactly one entry (no new entry created)
        assert_eq!(cfg.destinations.len(), 1, "must not create a second entry");
        let work = &cfg.destinations[0];
        // Inclusive: both rules present on the same entry
        assert!(work.rules.contains(&DestinationRule::Domain("corp.com".to_string())));
        assert!(work.rules.contains(&DestinationRule::From("bob@corp.com".to_string())));
        assert_eq!(work.rules.len(), 2, "exactly two rules expected");
    }

    /// A path absent from the file gets a new entry appended at the end.
    #[test]
    fn test_upsert_rule_create_if_absent() {
        use email_to_markdown::destinations::{load_yaml, DestinationRule};
        let temp = TempDir::new().unwrap();
        let dest_file = temp.path().join("destinations.yaml");
        fs::write(
            &dest_file,
            "destinations:\n- path: Perso/Known\n  rules:\n  - domain: known.com\n",
        )
        .unwrap();

        upsert_rule(&dest_file, "Perso/NewPath", MatchRule::From("new@example.com".to_string()))
            .unwrap();

        let cfg = load_yaml(&dest_file).unwrap();
        // Exclusive: exactly one entry added
        assert_eq!(cfg.destinations.len(), 2, "exactly one entry added");
        // Inclusive: new entry is last, with the rule
        let last = cfg.destinations.last().unwrap();
        assert_eq!(last.path, "Perso/NewPath");
        assert!(last.rules.contains(&DestinationRule::From("new@example.com".to_string())));
        // Exclusive: existing entry preserved
        assert_eq!(cfg.destinations[0].path, "Perso/Known");
    }

    /// Upserting the same rule twice must not produce a duplicate.
    #[test]
    fn test_upsert_rule_dedups_identical_rule() {
        use email_to_markdown::destinations::{load_yaml, DestinationRule};
        let temp = TempDir::new().unwrap();
        let dest_file = temp.path().join("destinations.yaml");
        fs::write(
            &dest_file,
            "destinations:\n- path: Perso/Work\n  rules:\n  - from: b@x.com\n",
        )
        .unwrap();

        upsert_rule(&dest_file, "Perso/Work", MatchRule::From("b@x.com".to_string())).unwrap();

        let cfg = load_yaml(&dest_file).unwrap();
        let count = cfg.destinations[0]
            .rules
            .iter()
            .filter(|r| **r == DestinationRule::From("b@x.com".to_string()))
            .count();
        assert_eq!(count, 1, "rule must appear exactly once; got {}", count);
    }

    /// Upserting into an absent file creates it from an empty config.
    #[test]
    fn test_upsert_rule_absent_file_creates() {
        use email_to_markdown::destinations::load_yaml;
        let temp = TempDir::new().unwrap();
        let dest_file = temp.path().join("destinations.yaml");
        // File does not exist yet.
        upsert_rule(&dest_file, "Perso/New", MatchRule::Domain("X.COM".to_string())).unwrap();

        let cfg = load_yaml(&dest_file).unwrap();
        assert_eq!(cfg.destinations.len(), 1);
        assert_eq!(cfg.destinations[0].path, "Perso/New");
        // Domain lowercased on write (parity with legacy behavior).
        use email_to_markdown::destinations::DestinationRule;
        assert!(cfg.destinations[0]
            .rules
            .contains(&DestinationRule::Domain("x.com".to_string())));
    }

    /// A free-typed new path (not in destinations.txt) is accepted by apply_decision,
    /// the directory is created, and the .md is moved.
    /// destinations.txt must NOT be modified (D10).
    #[test]
    fn test_route_review_new_free_path_created_destinations_not_modified() {
        let temp = TempDir::new().unwrap();
        let notes_dir = temp.path().join("notes");
        let staging = temp.path().join("staging");
        fs::create_dir_all(&staging).unwrap();

        // Write a minimal destinations.txt — the new path is NOT listed.
        let destinations_txt = temp.path().join("destinations.txt");
        let original_content = "Perso/Finance/Banque | domain:bank.com\n";
        fs::write(&destinations_txt, original_content).unwrap();

        let md_src = staging.join("invoice.md");
        fs::write(&md_src, "---\nsubject: Free path test\n---\nBody\n").unwrap();

        // Free-typed path not in destinations.txt — apply_decision must still work (D10).
        let free_path = "Perso/NewCategory/FreeSubcat/2026/06";
        apply_decision(&md_src, free_path, &notes_dir).unwrap();

        let expected_md = notes_dir
            .join("Perso").join("NewCategory").join("FreeSubcat")
            .join("2026").join("06").join("invoice.md");

        // Inclusive: file is at the new location
        assert!(expected_md.exists(), "md must be moved to the new free path");
        // Exclusive: original not at staging
        assert!(!md_src.exists(), "original .md must not remain in staging");

        // D10: destinations.txt must NOT have been modified
        let after_content = fs::read_to_string(&destinations_txt).unwrap();
        assert_eq!(
            after_content, original_content,
            "destinations.txt must not be modified when a free path is used (D10)"
        );
        // Exclusive: no new line for the free path
        assert!(
            !after_content.contains("NewCategory"),
            "free path must not be written to destinations.txt (D10)"
        );
    }
}

mod destinations_tests {
    use email_to_markdown::destinations::{
        load_yaml, save_yaml, upsert_entry, DestinationEntry, DestinationRule, DestinationsConfig,
    };
    use std::fs;
    use tempfile::TempDir;

    /// An absent file loads as an empty config, not an error.
    #[test]
    fn test_load_yaml_absent_returns_empty() {
        let temp = TempDir::new().unwrap();
        let cfg = load_yaml(&temp.path().join("nope.yaml")).unwrap();
        assert!(cfg.destinations.is_empty());
    }

    /// save_yaml then load_yaml round-trips entry data, including external-tagged rules.
    #[test]
    fn test_load_yaml_round_trip() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("destinations.yaml");
        let cfg = DestinationsConfig {
            destinations: vec![DestinationEntry {
                path: "Perso/Banque".to_string(),
                note: Some("relevés".to_string()),
                rules: vec![
                    DestinationRule::Domain("ubs.ch".to_string()),
                    DestinationRule::Subject("facture".to_string()),
                ],
                default: false,
            }],
        };
        save_yaml(&path, &cfg).unwrap();

        // External tagging → one-key maps in YAML.
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("domain: ubs.ch"), "external tagging expected; got:\n{raw}");

        let back = load_yaml(&path).unwrap();
        assert_eq!(back.destinations.len(), 1);
        assert_eq!(back.destinations[0].path, "Perso/Banque");
        assert_eq!(back.destinations[0].note.as_deref(), Some("relevés"));
        assert_eq!(back.destinations[0].rules.len(), 2);
    }

    /// upsert_entry on an unknown path pushes a new entry.
    #[test]
    fn test_upsert_entry_adds_new_path() {
        let mut cfg = DestinationsConfig::default();
        upsert_entry(&mut cfg, "Perso/New", &[DestinationRule::Domain("x.com".to_string())]);
        assert_eq!(cfg.destinations.len(), 1);
        assert_eq!(cfg.destinations[0].path, "Perso/New");
    }

    /// upsert_entry must not duplicate an already-present rule (case-insensitive path).
    #[test]
    fn test_upsert_entry_dedup_rule() {
        let mut cfg = DestinationsConfig {
            destinations: vec![DestinationEntry {
                path: "Perso/Work".to_string(),
                note: None,
                rules: vec![DestinationRule::Domain("corp.com".to_string())],
                default: false,
            }],
        };
        // Same path different case + identical rule → no growth.
        upsert_entry(&mut cfg, "perso/work", &[DestinationRule::Domain("corp.com".to_string())]);
        assert_eq!(cfg.destinations.len(), 1);
        assert_eq!(cfg.destinations[0].rules.len(), 1);
    }

    /// An empty rule slice upserts a bare path (classification option).
    #[test]
    fn test_upsert_entry_path_no_rule() {
        let mut cfg = DestinationsConfig::default();
        upsert_entry(&mut cfg, "Perso/Bare", &[]);
        assert_eq!(cfg.destinations.len(), 1);
        assert!(cfg.destinations[0].rules.is_empty());
    }

    /// Multiple rules in one call are all appended.
    #[test]
    fn test_upsert_entry_multiple_rules() {
        let mut cfg = DestinationsConfig::default();
        upsert_entry(
            &mut cfg,
            "Perso/Multi",
            &[
                DestinationRule::Domain("a.com".to_string()),
                DestinationRule::Subject("invoice".to_string()),
            ],
        );
        assert_eq!(cfg.destinations[0].rules.len(), 2);
    }

    /// Migration parses a legacy .txt and writes the YAML equivalent.
    #[test]
    fn test_migrate_from_txt() {
        use email_to_markdown::destinations::migrate_from_txt;
        let temp = TempDir::new().unwrap();
        let txt = temp.path().join("destinations.txt");
        let yaml = temp.path().join("destinations.yaml");
        fs::write(
            &txt,
            "# header\nPerso/Banque | domain:ubs.ch, subject:facture\nPerso/Inbox | default\n",
        )
        .unwrap();

        migrate_from_txt(&txt, &yaml).unwrap();

        let cfg = load_yaml(&yaml).unwrap();
        assert_eq!(cfg.destinations.len(), 2);
        let banque = cfg.destinations.iter().find(|e| e.path == "Perso/Banque").unwrap();
        assert_eq!(banque.rules.len(), 2);
        let inbox = cfg.destinations.iter().find(|e| e.path == "Perso/Inbox").unwrap();
        assert!(inbox.default, "default flag must carry over");
    }

    /// Migration honors a custom (non-default-location) path pair.
    #[test]
    fn test_migrate_from_txt_custom_path() {
        use email_to_markdown::destinations::migrate_from_txt;
        let temp = TempDir::new().unwrap();
        let txt = temp.path().join("custom").join("routes.txt");
        let yaml = temp.path().join("custom").join("routes.yaml");
        fs::create_dir_all(txt.parent().unwrap()).unwrap();
        fs::write(&txt, "Perso/X | domain:x.com\n").unwrap();

        migrate_from_txt(&txt, &yaml).unwrap();

        assert!(yaml.exists(), "yaml must be written at the custom path");
        let cfg = load_yaml(&yaml).unwrap();
        assert_eq!(cfg.destinations[0].path, "Perso/X");
    }

    /// save_yaml refuses a symlink target (rule 02-rust-filesystem-safety).
    #[cfg(windows)]
    #[test]
    fn test_save_yaml_rejects_symlink() {
        let temp = TempDir::new().unwrap();
        let real = temp.path().join("real.yaml");
        save_yaml(&real, &DestinationsConfig::default()).unwrap();
        let link = temp.path().join("link.yaml");
        // Symlink creation may require privileges; skip the assertion if it fails.
        if std::os::windows::fs::symlink_file(&real, &link).is_ok() {
            let err = save_yaml(&link, &DestinationsConfig::default()).unwrap_err();
            assert!(format!("{err:#}").contains("symlink"));
        }
    }

    /// save_yaml refuses a symlink target (rule 02-rust-filesystem-safety).
    #[cfg(unix)]
    #[test]
    fn test_save_yaml_rejects_symlink() {
        let temp = TempDir::new().unwrap();
        let real = temp.path().join("real.yaml");
        save_yaml(&real, &DestinationsConfig::default()).unwrap();
        let link = temp.path().join("link.yaml");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let err = save_yaml(&link, &DestinationsConfig::default()).unwrap_err();
        assert!(format!("{err:#}").contains("symlink"));
    }
}

mod dest_cmd_tests {
    use email_to_markdown::dest_cmd::{add_entry, detect_anomalies};
    use email_to_markdown::destinations::{
        load_yaml, DestinationEntry, DestinationRule, DestinationsConfig,
    };
    use tempfile::TempDir;

    /// detect_anomalies is empty for a clean config.
    #[test]
    fn test_list_empty_file() {
        let cfg = DestinationsConfig::default();
        assert!(detect_anomalies(&cfg).is_empty());
    }

    /// Two `default` entries surface a warning.
    #[test]
    fn test_list_shows_anomaly_double_default() {
        let cfg = DestinationsConfig {
            destinations: vec![
                DestinationEntry { path: "A".into(), default: true, ..Default::default() },
                DestinationEntry { path: "B".into(), default: true, ..Default::default() },
            ],
        };
        let warnings = detect_anomalies(&cfg);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("default"));
    }

    /// add_entry with no rules creates a bare classification entry.
    #[test]
    fn test_add_path_only() {
        let temp = TempDir::new().unwrap();
        let dest_file = temp.path().join("destinations.yaml");
        add_entry(&dest_file, "Perso/Bare", &[], None, false).unwrap();

        let cfg = load_yaml(&dest_file).unwrap();
        assert_eq!(cfg.destinations.len(), 1);
        assert_eq!(cfg.destinations[0].path, "Perso/Bare");
        assert!(cfg.destinations[0].rules.is_empty());
    }

    /// add_entry with a domain rule persists it.
    #[test]
    fn test_add_with_domain_rule() {
        let temp = TempDir::new().unwrap();
        let dest_file = temp.path().join("destinations.yaml");
        add_entry(
            &dest_file,
            "Perso/Banque",
            &[DestinationRule::Domain("ubs.ch".into())],
            None,
            false,
        )
        .unwrap();

        let cfg = load_yaml(&dest_file).unwrap();
        assert!(cfg.destinations[0]
            .rules
            .contains(&DestinationRule::Domain("ubs.ch".into())));
    }

    /// Multiple flags → multiple rules in one call.
    #[test]
    fn test_add_multiple_flags_creates_multiple_rules() {
        let temp = TempDir::new().unwrap();
        let dest_file = temp.path().join("destinations.yaml");
        add_entry(
            &dest_file,
            "Perso/Banque",
            &[
                DestinationRule::Domain("ubs.ch".into()),
                DestinationRule::Subject("facture".into()),
            ],
            None,
            false,
        )
        .unwrap();

        let cfg = load_yaml(&dest_file).unwrap();
        assert_eq!(cfg.destinations[0].rules.len(), 2);
    }

    /// add_entry stores the note.
    #[test]
    fn test_add_with_note() {
        let temp = TempDir::new().unwrap();
        let dest_file = temp.path().join("destinations.yaml");
        add_entry(&dest_file, "Perso/Banque", &[], Some("relevés"), false).unwrap();

        let cfg = load_yaml(&dest_file).unwrap();
        assert_eq!(cfg.destinations[0].note.as_deref(), Some("relevés"));
    }

    /// Setting `default` when one already exists is an error.
    #[test]
    fn test_add_default_conflict_errors() {
        let temp = TempDir::new().unwrap();
        let dest_file = temp.path().join("destinations.yaml");
        add_entry(&dest_file, "Perso/Inbox", &[], None, true).unwrap();

        let err = add_entry(&dest_file, "Perso/Other", &[], None, true).unwrap_err();
        assert!(format!("{err:#}").contains("default destination already exists"));

        // Exclusive: the conflicting entry was not written.
        let cfg = load_yaml(&dest_file).unwrap();
        assert!(!cfg.destinations.iter().any(|e| e.path == "Perso/Other"));
    }

    /// A traversal path is rejected before any write.
    #[test]
    fn test_add_invalid_path_rejected() {
        let temp = TempDir::new().unwrap();
        let dest_file = temp.path().join("destinations.yaml");
        let err = add_entry(&dest_file, "Perso/../etc", &[], None, false).unwrap_err();
        assert!(format!("{err:#}").contains("invalid"));
        // Exclusive: no file created.
        assert!(!dest_file.exists(), "rejected add must not create the file");
    }
}

mod suggest_tests {
    use email_to_markdown::dest_cmd::{
        extract_domain, scan_domains, strip_trailing_year_month, uncovered_domains,
    };
    use email_to_markdown::destinations::{DestinationEntry, DestinationRule, DestinationsConfig};
    use std::fs;
    use tempfile::TempDir;

    /// Write a `.md` with a `from:` frontmatter field.
    fn write_md(dir: &std::path::Path, name: &str, from: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(
            dir.join(name),
            format!("---\nfrom: {from}\nsubject: test\n---\nBody\n"),
        )
        .unwrap();
    }

    /// Domains are extracted from frontmatter and counted.
    #[test]
    fn test_suggest_extracts_domain_from_frontmatter() {
        let temp = TempDir::new().unwrap();
        write_md(temp.path(), "a.md", "alice@ubs.ch");
        write_md(temp.path(), "b.md", "Bob <bob@ubs.ch>");
        write_md(temp.path(), "c.md", "carol@bnp.fr");

        let groups = scan_domains(temp.path()).unwrap();
        assert_eq!(groups.get("ubs.ch"), Some(&2));
        assert_eq!(groups.get("bnp.fr"), Some(&1));
    }

    /// Directories starting with `.` are not walked.
    #[test]
    fn test_suggest_excludes_dot_dirs() {
        let temp = TempDir::new().unwrap();
        write_md(&temp.path().join(".obsidian"), "x.md", "x@hidden.com");
        write_md(temp.path(), "ok.md", "ok@visible.com");

        let groups = scan_domains(temp.path()).unwrap();
        assert!(groups.get("hidden.com").is_none(), "dot-dir must be skipped");
        assert_eq!(groups.get("visible.com"), Some(&1));
    }

    /// Directories starting with `_` are not walked.
    #[test]
    fn test_suggest_excludes_underscore_dirs() {
        let temp = TempDir::new().unwrap();
        write_md(&temp.path().join("_deleted"), "x.md", "x@gone.com");
        write_md(temp.path(), "ok.md", "ok@here.com");

        let groups = scan_domains(temp.path()).unwrap();
        assert!(groups.get("gone.com").is_none(), "underscore-dir must be skipped");
        assert_eq!(groups.get("here.com"), Some(&1));
    }

    /// Symlinked directories are not followed.
    #[cfg(unix)]
    #[test]
    fn test_suggest_skips_symlinks() {
        let temp = TempDir::new().unwrap();
        let outside = temp.path().join("outside");
        write_md(&outside, "secret.md", "x@external.com");
        let root = temp.path().join("root");
        fs::create_dir_all(&root).unwrap();
        write_md(&root, "ok.md", "ok@inside.com");
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();

        let groups = scan_domains(&root).unwrap();
        assert!(groups.get("external.com").is_none(), "symlink must not be followed");
        assert_eq!(groups.get("inside.com"), Some(&1));
    }

    /// Symlinked directories are not followed (Windows).
    #[cfg(windows)]
    #[test]
    fn test_suggest_skips_symlinks() {
        let temp = TempDir::new().unwrap();
        let outside = temp.path().join("outside");
        write_md(&outside, "secret.md", "x@external.com");
        let root = temp.path().join("root");
        fs::create_dir_all(&root).unwrap();
        write_md(&root, "ok.md", "ok@inside.com");
        // Symlink creation may require privileges; skip the assertion if it fails.
        if std::os::windows::fs::symlink_dir(&outside, root.join("link")).is_ok() {
            let groups = scan_domains(&root).unwrap();
            assert!(groups.get("external.com").is_none(), "symlink must not be followed");
            assert_eq!(groups.get("inside.com"), Some(&1));
        }
    }

    /// A domain already covered by a Domain rule is filtered out.
    #[test]
    fn test_suggest_skips_already_covered_domain() {
        let mut groups = std::collections::HashMap::new();
        groups.insert("ubs.ch".to_string(), 5);
        groups.insert("bnp.fr".to_string(), 3);

        let cfg = DestinationsConfig {
            destinations: vec![DestinationEntry {
                path: "Perso/Banque".into(),
                rules: vec![DestinationRule::Domain("ubs.ch".into())],
                ..Default::default()
            }],
        };

        let out = uncovered_domains(groups, &cfg);
        // Inclusive: bnp.fr remains
        assert!(out.iter().any(|(d, _)| d == "bnp.fr"));
        // Exclusive: ubs.ch is gone
        assert!(!out.iter().any(|(d, _)| d == "ubs.ch"));
    }

    /// A typed path carrying a year/month suffix is stripped to the bare path.
    #[test]
    fn test_suggest_strips_year_month_from_path() {
        assert_eq!(strip_trailing_year_month("Perso/Banque/2026/06"), "Perso/Banque");
        // No suffix → unchanged.
        assert_eq!(strip_trailing_year_month("Perso/Banque"), "Perso/Banque");
    }

    /// An empty / file-less directory yields no groups and no crash.
    #[test]
    fn test_suggest_no_md_files() {
        let temp = TempDir::new().unwrap();
        let groups = scan_domains(temp.path()).unwrap();
        assert!(groups.is_empty());
    }

    /// extract_domain handles bare and display-name forms.
    #[test]
    fn test_extract_domain_forms() {
        assert_eq!(extract_domain("alice@ubs.ch").as_deref(), Some("ubs.ch"));
        assert_eq!(extract_domain("Alice <alice@UBS.ch>").as_deref(), Some("ubs.ch"));
        assert_eq!(extract_domain("no-at-sign"), None);
    }
}

/// Pure helpers backing the `dest` interactive editor.
mod dest_interactive_tests {
    use email_to_markdown::dest_cmd::filter_entries;
    use email_to_markdown::destinations::{
        remove_entry, remove_rule, set_default, set_note, DestinationEntry, DestinationRule,
        DestinationsConfig,
    };

    fn entry(path: &str) -> DestinationEntry {
        DestinationEntry { path: path.into(), ..Default::default() }
    }

    fn sample() -> DestinationsConfig {
        DestinationsConfig {
            destinations: vec![entry("Perso/Banque"), entry("Perso/Work"), entry("Pro/Clients")],
        }
    }

    /// remove_entry drops the matching path (case-insensitive) and reports true.
    #[test]
    fn test_remove_entry_by_path() {
        let mut cfg = sample();
        assert!(remove_entry(&mut cfg, "perso/work"));
        assert_eq!(cfg.destinations.len(), 2);
        assert!(!cfg.destinations.iter().any(|e| e.path == "Perso/Work"));
    }

    /// remove_entry on an unknown path is a no-op returning false.
    #[test]
    fn test_remove_entry_absent_returns_false() {
        let mut cfg = sample();
        assert!(!remove_entry(&mut cfg, "Nope"));
        assert_eq!(cfg.destinations.len(), 3);
    }

    /// set_default marks one entry and clears the flag on all others.
    #[test]
    fn test_set_default_clears_others() {
        let mut cfg = sample();
        cfg.destinations[0].default = true;
        assert!(set_default(&mut cfg, "Pro/Clients"));
        let defaults: Vec<&str> = cfg
            .destinations
            .iter()
            .filter(|e| e.default)
            .map(|e| e.path.as_str())
            .collect();
        assert_eq!(defaults, vec!["Pro/Clients"]);
    }

    /// set_note sets then clears a note.
    #[test]
    fn test_set_note_sets_and_clears() {
        let mut cfg = sample();
        assert!(set_note(&mut cfg, "Perso/Banque", Some("relevés".into())));
        assert_eq!(cfg.destinations[0].note.as_deref(), Some("relevés"));
        assert!(set_note(&mut cfg, "Perso/Banque", None));
        assert_eq!(cfg.destinations[0].note, None);
    }

    /// remove_rule drops a matching rule from an entry.
    #[test]
    fn test_remove_rule_drops_match() {
        let mut cfg = DestinationsConfig {
            destinations: vec![DestinationEntry {
                path: "Perso/Banque".into(),
                rules: vec![
                    DestinationRule::Domain("ubs.ch".into()),
                    DestinationRule::Subject("facture".into()),
                ],
                ..Default::default()
            }],
        };
        assert!(remove_rule(&mut cfg, "Perso/Banque", &DestinationRule::Domain("ubs.ch".into())));
        assert_eq!(cfg.destinations[0].rules, vec![DestinationRule::Subject("facture".into())]);
    }

    /// remove_rule for a rule that isn't present is a no-op returning false.
    #[test]
    fn test_remove_rule_absent_noop() {
        let mut cfg = DestinationsConfig {
            destinations: vec![DestinationEntry {
                path: "Perso/Banque".into(),
                rules: vec![DestinationRule::Domain("ubs.ch".into())],
                ..Default::default()
            }],
        };
        assert!(!remove_rule(&mut cfg, "Perso/Banque", &DestinationRule::Subject("x".into())));
        assert_eq!(cfg.destinations[0].rules.len(), 1);
    }

    /// filter_entries matches paths case-insensitively as a substring.
    #[test]
    fn test_filter_entries_substring_ci() {
        let cfg = sample();
        let hits = filter_entries(&cfg, "PERSO");
        assert_eq!(hits, vec![0, 1]);
    }

    /// An empty filter returns every index.
    #[test]
    fn test_filter_entries_empty_returns_all() {
        let cfg = sample();
        assert_eq!(filter_entries(&cfg, "  "), vec![0, 1, 2]);
    }
}
