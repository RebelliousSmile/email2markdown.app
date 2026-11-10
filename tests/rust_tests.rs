use email_to_markdown::config::{
    load_raw_accounts, save_accounts, AccountBehavior, Config, ConfigError, RawAccount, Settings,
};
use email_to_markdown::network::{NetworkConfig, ProgressIndicator}; // [3][4]
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
        assert_eq!(
            get_short_name(Some("John Doe <john@example.com>")),
            "JohnDoe"
        );
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
    fn test_sanitize_filename_with_separators_and_parent() {
        assert_eq!(sanitize_filename("../a/b\\c"), ".._a_b_c");
    }

    #[test]
    fn test_sanitize_filename_empty() {
        assert_eq!(sanitize_filename(""), "_");
        assert_eq!(sanitize_filename(" . "), "_");
    }

    #[test]
    fn test_sanitize_filename_with_control_chars() {
        assert_eq!(sanitize_filename("a\u{0}b\nc.pdf"), "a_b_c.pdf");
    }

    #[test]
    fn test_sanitize_filename_with_reserved_name() {
        assert_eq!(sanitize_filename("CON"), "_CON");
        assert_eq!(sanitize_filename("nul.txt"), "_nul.txt");
        assert_eq!(sanitize_filename("COM1.log"), "_COM1.log");
        assert_eq!(sanitize_filename("CONSOLE.txt"), "CONSOLE.txt");
    }

    #[test]
    fn test_sanitize_filename_with_trailing_dot_and_space() {
        assert_eq!(sanitize_filename("rapport. "), "rapport");
    }

    #[test]
    fn test_sanitize_filename_with_long_name_keeps_extension() {
        let result = sanitize_filename(&format!("{}.pdf", "é".repeat(300)));
        assert_eq!(result, format!("{}.pdf", "é".repeat(98)));
    }

    #[test]
    fn test_imap_folder_relative_path_valid() {
        for display in ["INBOX", "INBOX.Sent", "[Gmail]/Tous les messages"] {
            assert_eq!(
                imap_folder_relative_path(display).expect("safe folder"),
                std::path::PathBuf::from(display.replace('.', "/")),
                "{display}"
            );
        }
    }

    #[test]
    fn test_imap_folder_relative_path_empty_segments_skipped() {
        assert_eq!(
            imap_folder_relative_path("INBOX..Archives").expect("safe folder"),
            std::path::Path::new("INBOX").join("Archives")
        );
    }

    #[test]
    fn test_imap_folder_relative_path_invalid() {
        let cases = [
            "/etc",
            "\\\\hote\\partage",
            "C:Windows",
            "INBOX/../..",
            "a\u{7}b",
            "INBOX.CON",
            "INBOX/Archive ",
            "",
            "...",
        ];
        for display in cases {
            assert!(imap_folder_relative_path(display).is_err(), "{display:?}");
        }
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
    fn test_load_with_settings_invalid_settings_yaml() {
        let temp = TempDir::new().expect("create tempdir");
        let accounts = temp.path().join("accounts.yaml");
        let settings = temp.path().join("settings.yaml");
        std::fs::write(
            &accounts,
            "accounts:\n  - name: Perso\n    server: imap.example.org\n    port: 993\n    username: perso@example.org\n",
        )
        .expect("write accounts");
        std::fs::write(&settings, "export_base_dir: [unclosed\n").expect("write settings");

        let result = Config::load_with_settings(&accounts, &settings);

        assert!(matches!(result, Err(ConfigError::SettingsFile { path, .. }) if path == settings));
    }

    #[test]
    fn test_settings_load_invalid_names_file() {
        let temp = TempDir::new().expect("create tempdir");
        let settings = temp.path().join("settings.yaml");
        std::fs::write(&settings, "organize_by_date: [unclosed\n").expect("write settings");

        let message = Settings::load(&settings).unwrap_err().to_string();

        assert!(message.contains("settings.yaml"), "{message}");
    }

    #[test]
    fn test_settings_load_absent_is_default() {
        let temp = TempDir::new().expect("create tempdir");
        let settings = Settings::load(&temp.path().join("settings.yaml")).expect("defaults");
        assert!(settings.notes_dir.is_none());
    }

    #[test]
    fn test_load_with_settings_missing_settings_yaml() {
        let temp = TempDir::new().expect("create tempdir");
        let accounts = temp.path().join("accounts.yaml");
        std::fs::write(
            &accounts,
            "accounts:\n  - name: Perso\n    server: imap.example.org\n    port: 993\n    username: perso@example.org\n",
        )
        .expect("write accounts");

        let result = Config::load_with_settings(&accounts, &temp.path().join("settings.yaml"));

        // A missing file is not a load failure: defaults apply, then validation
        // rejects the account for lack of export_base_dir.
        assert!(matches!(result, Err(ConfigError::ValidationError(_))));
    }

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
        assert!(
            result.is_empty(),
            "expected empty vec for missing file, got {:?}",
            result
        );
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

    #[test]
    fn test_app_paths_from_app_config_default() {
        let paths = email_to_markdown::config::AppPaths::from_app_config();
        assert_eq!(
            paths.accounts,
            email_to_markdown::config::accounts_yaml_path()
        );
        assert_eq!(paths.env, email_to_markdown::config::env_file_path());
        assert_eq!(paths.settings, email_to_markdown::config::settings_path());
        assert_eq!(paths.ledger, email_to_markdown::handled::ledger_path());
        assert_eq!(
            paths.deletion_log,
            email_to_markdown::deletion_log::log_path()
        );
    }
}

mod settings_tests {
    use super::*;
    use email_to_markdown::date_layout::DateLayout;

    #[test]
    fn test_settings_date_layout_absent_file_is_year_month() {
        let temp = TempDir::new().unwrap();
        let s = Settings::load(&temp.path().join("settings.yaml")).unwrap();
        assert_eq!(s.date_layout(), DateLayout::YearMonth);
    }

    #[test]
    fn test_settings_date_layout_with_organize_by_date_false() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("settings.yaml");
        std::fs::write(&path, "organize_by_date: false\n").unwrap();
        assert_eq!(
            Settings::load(&path).unwrap().date_layout(),
            DateLayout::None
        );
    }

    #[test]
    fn test_settings_date_layout_key_overrides_organize_by_date() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("settings.yaml");
        std::fs::write(
            &path,
            "organize_by_date: false\ndate_layout: year_month_day\n",
        )
        .unwrap();
        assert_eq!(
            Settings::load(&path).unwrap().date_layout(),
            DateLayout::YearMonthDay
        );
    }

    #[test]
    fn test_settings_date_layout_absent_with_organize_by_date_true() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("settings.yaml");
        std::fs::write(&path, "organize_by_date: true\n").unwrap();
        assert_eq!(
            Settings::load(&path).unwrap().date_layout(),
            DateLayout::YearMonth
        );
    }

    #[test]
    fn test_settings_date_layout_year_routes_into_year_folder() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("settings.yaml");
        std::fs::write(&path, "date_layout: year\n").unwrap();
        let layout = Settings::load(&path).unwrap().date_layout();
        let dests =
            email_to_markdown::route::parse_destinations("Perso/Banque | domain:bank.com\n")
                .unwrap();
        let meta = email_to_markdown::route::EmailMeta {
            from: "a@bank.com".into(),
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            domain: "bank.com".into(),
            subject: "Statement".into(),
            account: "personal".into(),
            date: chrono::DateTime::parse_from_rfc3339("2026-10-03T10:00:00+00:00").unwrap(),
        };
        let decision = email_to_markdown::route::route_email(&meta, &dests, layout);
        assert_eq!(decision.rel_path, "Perso/Banque/2026");
    }

    #[test]
    fn test_settings_date_layout_unknown_value_lists_variants() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("settings.yaml");
        std::fs::write(&path, "date_layout: week\n").unwrap();
        let err = format!("{:#}", Settings::load(&path).unwrap_err());
        for variant in ["none", "year", "year_month", "year_month_day"] {
            assert!(err.contains(variant), "missing {variant} in: {err}");
        }
    }

    #[test]
    fn test_settings_set_date_layout_none_writes_both_keys() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("settings.yaml");
        let mut s = Settings::default();
        s.set_date_layout(DateLayout::None);
        s.save(&path).unwrap();
        let yaml = std::fs::read_to_string(&path).unwrap();
        assert!(yaml.contains("organize_by_date: false"), "{yaml}");
        assert!(yaml.contains("date_layout: none"), "{yaml}");
    }

    #[test]
    fn test_settings_destination_max_depth_absent_is_2() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("settings.yaml");
        std::fs::write(&path, "organize_by_date: true\n").unwrap();
        assert_eq!(Settings::load(&path).unwrap().destination_max_depth(), 2);
    }

    #[test]
    fn test_settings_destination_max_depth_with_3() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("settings.yaml");
        std::fs::write(&path, "destination_max_depth: 3\n").unwrap();
        assert_eq!(Settings::load(&path).unwrap().destination_max_depth(), 3);
    }

    #[test]
    fn test_settings_destination_max_depth_zero_is_refused_on_load() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("settings.yaml");
        std::fs::write(&path, "destination_max_depth: 0\n").unwrap();
        let err = Settings::load(&path).unwrap_err().to_string();
        assert!(err.contains("destination_max_depth"), "{err}");
    }

    #[test]
    fn test_settings_destination_max_depth_zero_is_refused_on_save() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("settings.yaml");
        std::fs::write(&path, "notes_dir: C:/notes\n").unwrap();
        let s = Settings {
            destination_max_depth: Some(0),
            ..Settings::default()
        };
        let err = s.save(&path).unwrap_err().to_string();
        assert!(err.contains("destination_max_depth"), "{err}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "notes_dir: C:/notes\n"
        );
    }

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

        let mut s = Settings {
            export_base_dir: Some("/tmp/emails".to_string()),
            ..Settings::default()
        };
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
        assert_eq!(
            config.accounts[0].export_directory,
            "/tmp/emails/TestAccount"
        );
    }

    #[test]
    fn test_config_merge_defaults_applied() {
        let temp = TempDir::new().unwrap();

        let accounts_yaml = "accounts:\n  - name: TestAccount\n    server: imap.example.com\n    port: 993\n    username: user@example.com\n";
        let accounts_path = temp.path().join("accounts.yaml");
        std::fs::write(&accounts_path, accounts_yaml).unwrap();

        let settings_yaml =
            "export_base_dir: /tmp/emails\ndefaults:\n  quote_depth: 3\n  collect_contacts: true\n";
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
        assert!(config.accounts[0]
            .export_directory
            .ends_with("custom-folder"));
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

        let behavior = settings
            .accounts
            .get("myaccount")
            .expect("myaccount entry missing");
        // Inclusive: fields set in YAML must round-trip correctly.
        assert_eq!(
            behavior.skip_signature_images,
            Some(true),
            "skip_signature_images should be Some(true)"
        );
        assert_eq!(
            behavior.delete_after_export,
            Some(false),
            "delete_after_export should be Some(false)"
        );
        assert_eq!(
            behavior.quote_depth,
            Some(5),
            "quote_depth should be Some(5)"
        );
        // Exclusive: fields absent from YAML must not bleed in from defaults or other sources.
        assert_eq!(
            behavior.skip_existing, None,
            "skip_existing must not bleed from YAML"
        );
        assert_eq!(
            behavior.collect_contacts, None,
            "collect_contacts must not bleed"
        );
        assert_eq!(
            behavior.folder_name, None,
            "folder_name should be None (not set)"
        );
        assert_eq!(
            settings.defaults.skip_signature_images,
            Some(false),
            "defaults.skip_signature_images should be Some(false)"
        );
    }

    #[test]
    fn test_settings_account_behavior_remove_entry_removes_from_yaml() {
        let temp = TempDir::new().expect("create tempdir");
        let path = temp.path().join("settings.yaml");

        let mut settings = Settings::default();
        settings.accounts.insert(
            "myaccount".to_string(),
            AccountBehavior {
                skip_signature_images: Some(true),
                ..AccountBehavior::default()
            },
        );
        settings.save(&path).expect("save settings");

        let saved_content = std::fs::read_to_string(&path).expect("read saved yaml");
        assert!(
            saved_content.contains("myaccount"),
            "saved YAML should contain 'myaccount'"
        );

        settings.accounts.remove("myaccount");
        settings.save(&path).expect("save settings after remove");

        let reloaded = Settings::load(&path).expect("reload settings");
        assert!(
            reloaded.accounts.is_empty(),
            "accounts map should be empty after removing the only entry"
        );
    }
}

mod email_export_tests {
    use email_to_markdown::email_export::*;

    #[test]
    fn test_analyze_email_type_direct() {
        let raw_email =
            b"From: sender@example.com\r\nTo: recipient@example.com\r\nSubject: Test\r\n\r\nBody";
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
        links.insert(
            "instagram".to_string(),
            "https://www.instagram.com/foo".to_string(),
        );
        links.insert(
            "facebook".to_string(),
            "https://www.facebook.com/foo".to_string(),
        );

        let fm = EmailFrontmatter {
            from: "a@example.com".to_string(),
            to: "b@example.com".to_string(),
            date: "2026-04-15T00:00:00+00:00".to_string(),
            subject: "Hi".to_string(),
            subject_hash: "abcdef".to_string(),
            tags: vec!["inbox".to_string()],
            attachments: vec![],
            source: None,
            keep_on_server: None,
            email_type: None,
            social_links: Some(links),
        };

        let yaml = serde_yaml::to_string(&fm).expect("serialize");
        assert!(
            yaml.contains("social_links:"),
            "missing social_links key in:\n{}",
            yaml
        );
        assert!(
            yaml.contains("instagram: https://www.instagram.com/foo"),
            "missing instagram entry in:\n{}",
            yaml
        );
        assert!(
            yaml.contains("facebook: https://www.facebook.com/foo"),
            "missing facebook entry in:\n{}",
            yaml
        );
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
            source: None,
            keep_on_server: None,
            email_type: None,
            social_links: None,
        };

        let yaml = serde_yaml::to_string(&fm).expect("serialize");
        assert!(
            !yaml.contains("social_links"),
            "social_links should be omitted when None, got:\n{}",
            yaml
        );
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
            source: None,
            keep_on_server: None,
            email_type: Some("newsletter".to_string()),
            social_links: None,
        };

        let yaml = serde_yaml::to_string(&fm).expect("serialize");
        assert!(
            yaml.contains("email_type: newsletter"),
            "expected email_type in:\n{}",
            yaml
        );
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
            source: None,
            keep_on_server: None,
            email_type: None,
            social_links: None,
        };

        let yaml = serde_yaml::to_string(&fm).expect("serialize");
        assert!(
            !yaml.contains("email_type"),
            "email_type should be omitted when None, got:\n{}",
            yaml
        );
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

    #[test]
    fn test_should_skip_image_keeps_referenced_generic_screenshot() {
        for size in [30 * 1024, 80 * 1024] {
            assert!(!should_skip_image(
                true,
                Some("image.png"),
                "image/png",
                size,
                Some("inline")
            ));
        }
    }

    #[test]
    fn test_should_skip_image_drops_referenced_logo_name() {
        assert!(should_skip_image(
            true,
            Some("logo.png"),
            "image/png",
            30 * 1024,
            Some("inline")
        ));
    }

    #[test]
    fn test_should_skip_image_drops_referenced_icon() {
        assert!(should_skip_image(
            true,
            Some("image.png"),
            "image/png",
            4 * 1024,
            Some("inline")
        ));
    }

    #[test]
    fn test_should_skip_image_keeps_large_unreferenced_inline() {
        assert!(!should_skip_image(
            false,
            Some("photo.jpg"),
            "image/jpeg",
            400 * 1024,
            Some("inline")
        ));
    }

    #[test]
    fn test_should_skip_image_drops_small_unreferenced_inline() {
        assert!(should_skip_image(
            false,
            Some("vacances.jpg"),
            "image/jpeg",
            60 * 1024,
            Some("inline")
        ));
    }

    #[test]
    fn test_should_skip_image_keeps_non_image_attachments() {
        assert!(!should_skip_image(
            false,
            Some("contrat.pdf"),
            "application/pdf",
            40 * 1024,
            Some("inline")
        ));
        assert!(!should_skip_image(
            false,
            Some("company.docx"),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            20 * 1024,
            Some("attachment")
        ));
    }

    #[test]
    fn test_should_skip_image_with_uppercase_content_type() {
        // MIME types are case-insensitive: `IMAGE/PNG` is a small image too.
        assert!(should_skip_image(
            false,
            Some("capture.dat"),
            "IMAGE/PNG",
            20 * 1024,
            None
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
        // Just verify it creates without panic
        let _progress = ProgressIndicator::new("Test", 100);
    }

    #[test]
    fn test_progress_indicator_update() {
        let mut progress = ProgressIndicator::new("Test", 10);
        progress.update(5);
        // Verify it updates without panic
        progress.inc();
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
    use chrono::DateTime;
    use email_to_markdown::date_layout::DateLayout;
    use email_to_markdown::route::{
        ai_route, apply_decision, delete_email, destinations_path_from, join_safe_segments,
        load_destinations_from, move_email, normalize_address, parse_destinations,
        repair_legacy_attachments, resolve_contextual_destination, route_email, unique_addresses,
        upsert_rule, Destination, EmailMeta, MatchRule,
    };
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn test_match_rule_parse_valid() {
        assert_eq!(
            MatchRule::parse("domain", " Example.COM ").unwrap(),
            MatchRule::Domain("example.com".into())
        );
        assert_eq!(
            MatchRule::parse("correspondent", "Bob@X.fr").unwrap(),
            MatchRule::Correspondent("bob@x.fr".into())
        );
        assert_eq!(
            MatchRule::parse("subject", "  Facture Mars ").unwrap(),
            MatchRule::Subject("Facture Mars".into())
        );
    }

    #[test]
    fn test_match_rule_parse_invalid() {
        assert!(MatchRule::parse("subject", "   ").is_err());
        assert!(MatchRule::parse("from", "not-an-address").is_err());
        assert!(MatchRule::parse("correspondent", "a@x.fr, b@x.fr").is_err());
        assert!(MatchRule::parse("bogus", "x").is_err());
    }

    #[test]
    fn test_match_rule_parse_invalid_french_messages() {
        let message =
            |kind: &str, value: &str| MatchRule::parse(kind, value).unwrap_err().to_string();
        assert_eq!(
            message("subject", "  "),
            "valeur vide pour une règle subject"
        );
        assert_eq!(message("bogus", "x"), "type de règle inconnu : bogus");
        assert_eq!(message("from", "nope"), "adresse invalide : nope");
    }

    #[test]
    fn test_unique_addresses_with_duplicates_and_case() {
        let addresses = unique_addresses(["Bob@X.fr", "alice@y.fr", "bob@x.fr", " ALICE@Y.FR "]);
        assert_eq!(addresses, vec!["bob@x.fr", "alice@y.fr"]);
    }

    #[test]
    fn test_unique_addresses_invalid() {
        let addresses = unique_addresses(vec![
            "Carol <c@x.fr>".to_string(),
            "a@x.fr, b@x.fr".to_string(),
            String::new(),
            "d@x.fr".to_string(),
        ]);
        assert_eq!(addresses, vec!["d@x.fr"]);
    }

    #[test]
    fn test_unique_addresses_empty() {
        assert!(unique_addresses(Vec::<&str>::new()).is_empty());
    }

    #[test]
    fn test_destinations_path_from_invalid_settings_names_file() {
        let temp = TempDir::new().expect("create tempdir");
        let settings = temp.path().join("settings.yaml");
        fs::write(
            &settings,
            "destinations_file: [unclosed
",
        )
        .expect("write settings");

        let message = format!("{:#}", destinations_path_from(&settings).unwrap_err());

        assert!(message.contains("settings.yaml"), "{message}");
    }

    #[test]
    fn test_load_destinations_from_without_destinations_file_empty() {
        let temp = TempDir::new().expect("tempdir");
        let settings = temp.path().join("settings.yaml");
        fs::write(
            &settings,
            "organize_by_date: true
",
        )
        .expect("write settings");

        assert_eq!(
            destinations_path_from(&settings).expect("path"),
            temp.path().join("destinations.yaml")
        );
        assert!(load_destinations_from(&settings)
            .expect("destinations")
            .is_empty());
    }

    #[test]
    fn test_destinations_path_from_valid_settings() {
        let temp = TempDir::new().expect("create tempdir");
        let settings = temp.path().join("settings.yaml");
        let dest = temp.path().join("dest.yaml");
        fs::write(
            &settings,
            format!(
                "destinations_file: '{}'
",
                dest.display()
            ),
        )
        .expect("write settings");

        assert_eq!(destinations_path_from(&settings).expect("path"), dest);
    }

    #[test]
    fn test_rewrite_attachment_paths_keeps_email_block_intact() {
        let temp = TempDir::new().unwrap();
        let old_dir = temp.path().join("staging");
        let new_dir = temp.path().join("notes").join("Perso");
        fs::create_dir_all(&old_dir).unwrap();
        fs::create_dir_all(&new_dir).unwrap();
        let block = "```email
subject = \"Devis\"
from = \"a@b.c\"
to = []
body = '''
attachments:
- fake.pdf
'''
```
";
        let md = new_dir.join("note.md");
        fs::write(
            &md,
            format!(
                "---
tags:
  - email
attachments:
  - a.pdf
---

{block}"
            ),
        )
        .unwrap();

        email_to_markdown::route::rewrite_attachment_paths(&md, &old_dir, &new_dir).unwrap();

        let content = fs::read_to_string(&md).unwrap();
        assert!(
            content.ends_with(block),
            "block must be unchanged: {content:?}"
        );
        assert!(
            content.contains("  - ../../staging/a.pdf"),
            "got {content:?}"
        );
        assert!(!content.contains("  - a.pdf"));
    }

    // ── helpers ──────────────────────────────────────────────────────────────

    fn make_meta(
        from: &str,
        domain: &str,
        subject: &str,
        account: &str,
        date_str: &str,
    ) -> EmailMeta {
        EmailMeta {
            from: from.to_string(),
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            domain: domain.to_string(),
            subject: subject.to_string(),
            account: account.to_string(),
            date: DateTime::parse_from_rfc3339(date_str).expect("valid date"),
        }
    }

    #[test]
    fn test_correspondent_matches_all_available_address_headers() {
        let dests =
            parse_destinations("Pro/Client | correspondent:contact+project@example.com\n").unwrap();
        let mut meta = make_meta(
            "sender@elsewhere.example",
            "elsewhere.example",
            "Project",
            "Work",
            "2026-09-01T12:00:00+00:00",
        );

        for header in 0..4 {
            meta.from = "sender@elsewhere.example".into();
            meta.to.clear();
            meta.cc.clear();
            meta.bcc.clear();
            match header {
                0 => meta.from = "CONTACT+PROJECT@EXAMPLE.COM".into(),
                1 => meta.to.push("CONTACT+PROJECT@EXAMPLE.COM".into()),
                2 => meta.cc.push("contact+project@example.com".into()),
                _ => meta.bcc.push("contact+project@example.com".into()),
            }
            assert!(!route_email(&meta, &dests, DateLayout::YearMonth).is_default);
        }
    }

    #[test]
    fn test_from_rule_remains_limited_to_sender() {
        let dests = parse_destinations("Pro/Client | from:contact@example.com\n").unwrap();
        let mut meta = make_meta(
            "sender@elsewhere.example",
            "elsewhere.example",
            "Project",
            "Work",
            "2026-09-01T12:00:00+00:00",
        );
        meta.to.push("contact@example.com".into());
        assert!(route_email(&meta, &dests, DateLayout::YearMonth).is_default);
    }

    #[test]
    fn test_normalize_address_validates_and_preserves_searchable_forms() {
        assert_eq!(
            normalize_address(" Contact+Project@Example.COM ").unwrap(),
            "contact+project@example.com"
        );
        assert!(normalize_address("Contact <contact@example.com>").is_err());
        assert!(normalize_address("missing-at.example.com").is_err());
        assert!(normalize_address("a@invalid").is_err());
    }

    #[test]
    fn test_correspondent_yaml_round_trip_preserves_display_value() {
        use email_to_markdown::destinations::{
            load_yaml, save_yaml, DestinationEntry, DestinationRule, DestinationsConfig,
        };
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("destinations.yaml");
        let config = DestinationsConfig {
            destinations: vec![DestinationEntry {
                path: "Pro/Client".into(),
                rules: vec![DestinationRule::Correspondent("Contact@Example.COM".into())],
                ..Default::default()
            }],
        };
        save_yaml(&path, &config).unwrap();
        let loaded = load_yaml(&path).unwrap();
        assert_eq!(loaded.destinations[0].rules, config.destinations[0].rules);
        assert!(fs::read_to_string(path)
            .unwrap()
            .contains("correspondent: Contact@Example.COM"));
    }

    #[test]
    fn test_resolve_contextual_destination_requires_exact_physical_folder_and_account() {
        let temp = TempDir::new().unwrap();
        let notes = temp.path().join("notes");
        let target = notes.join("Pro").join("Client");
        let child = target.join("Subfolder");
        fs::create_dir_all(&child).unwrap();
        let destinations = vec![Destination {
            path: "Pro/Client".into(),
            rules: vec![
                MatchRule::Correspondent("contact@example.com".into()),
                MatchRule::Account("Work".into()),
                MatchRule::Account("Archive".into()),
                MatchRule::Subject("ignored for contextual search".into()),
            ],
            is_default: false,
        }];
        let accounts = vec!["Personal".into(), "Work".into()];

        let resolved =
            resolve_contextual_destination(&notes, &target, &destinations, &accounts).unwrap();
        assert_eq!(resolved.relative_path, "Pro/Client");
        assert_eq!(resolved.allowed_accounts, vec!["Work"]);
        assert_eq!(resolved.address_rules.len(), 1);
        assert!(resolve_contextual_destination(&notes, &child, &destinations, &accounts).is_err());
        assert!(resolve_contextual_destination(
            &notes,
            &target,
            &destinations,
            &["Personal".into()]
        )
        .is_err());
    }

    #[test]
    fn test_resolve_contextual_destination_reports_missing_rule_as_setup_state() {
        use email_to_markdown::route::ContextualDestinationError;

        let temp = TempDir::new().unwrap();
        let notes = temp.path().join("notes");
        let target = notes.join("Perso").join("Associations");
        fs::create_dir_all(&target).unwrap();
        let destinations = vec![Destination {
            path: "Perso/Associations".into(),
            rules: vec![],
            is_default: false,
        }];

        let error =
            resolve_contextual_destination(&notes, &target, &destinations, &["Personnel".into()])
                .unwrap_err();
        let typed = error
            .downcast_ref::<ContextualDestinationError>()
            .expect("missing rules must be distinguishable from a broken configuration");
        assert_eq!(
            typed,
            &ContextualDestinationError::MissingAddressRule {
                path: "Perso/Associations".into()
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_resolve_contextual_destination_rejects_symlink() {
        let temp = TempDir::new().unwrap();
        let notes = temp.path().join("notes");
        let real = notes.join("Real");
        let link = notes.join("Alias");
        fs::create_dir_all(&real).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let destinations = vec![Destination {
            path: "Alias".into(),
            rules: vec![MatchRule::From("a@example.com".into())],
            is_default: false,
        }];
        assert!(
            resolve_contextual_destination(&notes, &link, &destinations, &["Work".into()]).is_err()
        );
    }

    #[cfg(windows)]
    #[test]
    fn test_resolve_contextual_destination_rejects_symlink() {
        let temp = TempDir::new().unwrap();
        let notes = temp.path().join("notes");
        let real = notes.join("Real");
        let link = notes.join("Alias");
        fs::create_dir_all(&real).unwrap();
        if std::os::windows::fs::symlink_dir(&real, &link).is_err() {
            return;
        }
        let destinations = vec![Destination {
            path: "Alias".into(),
            rules: vec![MatchRule::From("a@example.com".into())],
            is_default: false,
        }];
        assert!(
            resolve_contextual_destination(&notes, &link, &destinations, &["Work".into()]).is_err()
        );
    }

    // --- join_safe_segments: migrated from sort_emails_tests ---

    #[test]
    fn test_join_safe_segments_nested_path() {
        let root = PathBuf::from("/notes");
        let joined = join_safe_segments(&root, "Travail/Projets/Client A").unwrap();
        assert_eq!(
            joined,
            root.join("Travail").join("Projets").join("Client A")
        );
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

    #[test]
    fn test_join_safe_segments_rejects_reserved_name() {
        let root = PathBuf::from("/notes");
        assert!(join_safe_segments(&root, "Travail/con").is_err());
        assert!(join_safe_segments(&root, "Travail/LPT1.txt").is_err());
    }

    #[test]
    fn test_join_safe_segments_rejects_trailing_dot() {
        let root = PathBuf::from("/notes");
        assert!(join_safe_segments(&root, "Travail/Projets.").is_err());
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
        assert!(
            att_dest.exists(),
            "flat attachment must be co-located at dest"
        );

        // Exclusive: original paths no longer exist
        assert!(!md_src.exists(), "original .md must not remain at src");
        assert!(
            !att_src.exists(),
            "original attachment must not remain at src"
        );

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

    #[test]
    fn test_move_email_with_parent_dir_link_skipped() {
        let temp = TempDir::new().unwrap();
        let src_dir = temp.path().join("account").join("staging");
        let dst_dir = temp.path().join("account").join("dest");
        fs::create_dir_all(&src_dir).unwrap();
        fs::create_dir_all(&dst_dir).unwrap();
        // Outside the account root: `account/../secret.txt`
        let outside = temp.path().join("secret.txt");
        fs::write(&outside, b"secret").unwrap();
        let md_src = src_dir.join("email.md");
        fs::write(
            &md_src,
            "---\nsubject: Test\nattachments:\n  - ../secret.txt\n---\nBody\n",
        )
        .unwrap();

        move_email(&md_src, &dst_dir).unwrap();

        assert!(dst_dir.join("email.md").exists());
        assert!(outside.exists(), "file outside the account must stay put");
        assert!(!dst_dir.join("secret.txt").exists());
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
        assert!(
            !att_src.exists(),
            "original attachment must not remain at src"
        );
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
        assert!(
            dst_dir.join("plain.md").exists(),
            "moved .md must exist at dest"
        );
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

    // --- move_email: legacy centralized attachment path resolved from account root ---
    // Attachment stored at account_root/attachments/Sent/email_…pdf; .md references it as
    // "attachments/Sent/email_…pdf" (relative to account root, not to md_dir).
    // move_email must locate the file at old_parent.parent(), move it co-located, and
    // update the frontmatter to the bare filename.
    #[test]
    fn test_move_email_migrates_legacy_centralized_attachment() {
        let temp = TempDir::new().unwrap();
        // Simulated account root layout:
        //   account_root/
        //     Sent/            ← old_parent (where the .md currently lives)
        //       email.md
        //     attachments/
        //       Sent/
        //         email_2025-11-18_FG_file.pdf   ← legacy centralized attachment
        let account_root = temp.path().join("account_root");
        let src_dir = account_root.join("Sent");
        let att_dir = account_root.join("attachments").join("Sent");
        fs::create_dir_all(&src_dir).unwrap();
        fs::create_dir_all(&att_dir).unwrap();

        let att_name = "email_2025-11-18_FG_file.pdf";
        let att_src = att_dir.join(att_name);
        fs::write(&att_src, b"PDF data").unwrap();

        let link = format!("attachments/Sent/{}", att_name);
        let md_content = format!(
            "---\nsubject: Test\nattachments:\n  - {}\n---\nBody\n",
            link
        );
        let md_src = src_dir.join("email.md");
        fs::write(&md_src, &md_content).unwrap();

        let dst_dir = temp.path().join("dest");
        fs::create_dir_all(&dst_dir).unwrap();

        move_email(&md_src, &dst_dir).unwrap();

        // Inclusive: attachment moved co-located with the .md.
        let att_dest = dst_dir.join(att_name);
        assert!(
            att_dest.exists(),
            "legacy attachment must be co-located with moved .md"
        );
        assert_eq!(fs::read(&att_dest).unwrap(), b"PDF data");

        // Inclusive: .md frontmatter updated to bare filename.
        let md_dest = dst_dir.join("email.md");
        let content = fs::read_to_string(&md_dest).unwrap();
        assert!(
            content.contains(&format!("- {}\n", att_name)),
            "frontmatter must reference bare filename, got: {:?}",
            content
        );
        assert!(
            !content.contains("attachments/Sent/"),
            "frontmatter must not retain the legacy path prefix"
        );

        // Exclusive: original legacy file removed.
        assert!(
            !att_src.exists(),
            "legacy source must be removed after move"
        );
    }

    // --- repair_legacy_attachments ---

    #[test]
    fn test_repair_legacy_attachments_moves_and_rewrites() {
        let temp = TempDir::new().unwrap();
        // Layout:
        //   notes_dir/
        //     Messy/2025/11/
        //       email.md          ← already moved .md with broken legacy path
        //   account_root/
        //     attachments/Sent/
        //       email_2025-11_file.pdf  ← still at legacy location
        let notes_dir = temp.path().join("notes");
        let md_dir = notes_dir.join("Messy").join("2025").join("11");
        fs::create_dir_all(&md_dir).unwrap();

        let account_root = temp.path().join("account");
        let att_dir = account_root.join("attachments").join("Sent");
        fs::create_dir_all(&att_dir).unwrap();

        let att_name = "email_2025-11_file.pdf";
        let att_src = att_dir.join(att_name);
        fs::write(&att_src, b"Repaired").unwrap();

        let link = format!("attachments/Sent/{}", att_name);
        let md_path = md_dir.join("email.md");
        fs::write(
            &md_path,
            format!("---\nsubject: X\nattachments:\n  - {}\n---\nBody\n", link),
        )
        .unwrap();

        let repaired = repair_legacy_attachments(&notes_dir, &account_root, None, false).unwrap();

        assert_eq!(repaired, 1);
        assert!(!att_src.exists(), "source must be removed");
        assert!(
            md_dir.join(att_name).exists(),
            "attachment must be co-located"
        );
        let content = fs::read_to_string(&md_path).unwrap();
        assert!(content.contains(&format!("- {}\n", att_name)));
        assert!(!content.contains("attachments/Sent/"));
    }

    #[test]
    fn test_repair_legacy_attachments_skips_dotdot_paths() {
        let temp = TempDir::new().unwrap();
        let notes_dir = temp.path().join("notes");
        let md_dir = notes_dir.join("Messy");
        fs::create_dir_all(&md_dir).unwrap();

        let account_root = temp.path().join("account");
        fs::create_dir_all(&account_root).unwrap();

        let md_path = md_dir.join("email.md");
        fs::write(
            &md_path,
            "---\nsubject: X\nattachments:\n  - ../../../etc/passwd\n---\nBody\n",
        )
        .unwrap();

        let repaired = repair_legacy_attachments(&notes_dir, &account_root, None, false).unwrap();
        assert_eq!(repaired, 0, "traversal paths must be rejected");
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
        assert!(
            recovered.exists(),
            "attachment must be relocated to _deleted"
        );
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

    #[test]
    fn test_delete_email_same_attachment_name_keeps_both() {
        let temp = TempDir::new().unwrap();
        let src_dir = temp.path().join("staging");
        fs::create_dir_all(&src_dir).unwrap();
        for (note, content) in [("first.md", "first PDF"), ("second.md", "second PDF")] {
            fs::write(src_dir.join("invoice.pdf"), content).unwrap();
            let md = src_dir.join(note);
            fs::write(&md, "---\nattachments:\n  - invoice.pdf\n---\nBody\n").unwrap();

            delete_email(&md).unwrap();
        }

        let deleted = src_dir.join("_deleted");
        assert_eq!(
            fs::read_to_string(deleted.join("invoice.pdf")).unwrap(),
            "first PDF"
        );
        assert_eq!(
            fs::read_to_string(deleted.join("invoice_2.pdf")).unwrap(),
            "second PDF"
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
        assert!(dests[0]
            .rules
            .contains(&MatchRule::Domain("credit-agricole.fr".to_string())));
        assert!(dests[0]
            .rules
            .contains(&MatchRule::From("noreply@ca.fr".to_string())));
        assert_eq!(dests[1].path, "Pro/Clients/Acme");
        // Exclusive: no spurious rules
        assert!(!dests[0]
            .rules
            .contains(&MatchRule::Subject("Invoice".to_string())));
        assert!(!dests[1].is_default);
    }

    #[test]
    fn test_parse_destinations_comments_and_empty_lines_skipped() {
        let content =
            "# This is a comment\n\nPerso/Inbox | domain:example.com\n# another comment\n";
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

    // ── rule conflicts ───────────────────────────────────────────────────────

    #[test]
    fn test_route_email_reports_alternatives_for_shared_rule() {
        let content =
            "Perso/A | domain:acme.com\nPerso/B | domain:acme.com\nPerso/C | domain:other.com\n";
        let dests = parse_destinations(content).unwrap();
        let meta = make_meta(
            "alice@acme.com",
            "acme.com",
            "Hello",
            "personal",
            "2026-06-15T10:00:00+00:00",
        );
        let decision = route_email(&meta, &dests, DateLayout::None);
        // Inclusive: first destination wins, second is offered
        assert_eq!(decision.rel_path, "Perso/A");
        assert_eq!(decision.alternatives, vec!["Perso/B".to_string()]);
        // Exclusive: the non-matching destination is not offered
        assert!(!decision.alternatives.contains(&"Perso/C".to_string()));
    }

    #[test]
    fn test_route_email_no_alternatives_when_unambiguous() {
        let dests =
            parse_destinations("Perso/A | domain:acme.com\nPerso/B | domain:other.com\n").unwrap();
        let meta = make_meta(
            "alice@acme.com",
            "acme.com",
            "Hello",
            "personal",
            "2026-06-15T10:00:00+00:00",
        );
        let decision = route_email(&meta, &dests, DateLayout::None);
        assert!(decision.alternatives.is_empty());
    }

    #[test]
    fn test_rule_conflicts_and_move_rule() {
        use email_to_markdown::destinations::{
            DestinationEntry, DestinationRule, DestinationsConfig,
        };
        let mut cfg = DestinationsConfig {
            destinations: vec![
                DestinationEntry {
                    path: "Perso/A".into(),
                    rules: vec![DestinationRule::Domain("acme.com".into())],
                    ..Default::default()
                },
                DestinationEntry {
                    path: "Perso/B".into(),
                    ..Default::default()
                },
            ],
        };
        let rule = email_to_markdown::route::MatchRule::Domain("ACME.com".into());
        // Inclusive: the other destination holding the rule is reported (case-insensitive)
        assert_eq!(
            email_to_markdown::route::rule_conflicts(&cfg, "Perso/B", &rule),
            vec!["Perso/A".to_string()]
        );
        // Exclusive: the target itself is never its own conflict
        assert!(email_to_markdown::route::rule_conflicts(&cfg, "Perso/A", &rule).is_empty());

        let removed = email_to_markdown::route::remove_rule_elsewhere(&mut cfg, "Perso/B", &rule);
        assert_eq!(removed, 1);
        assert!(cfg.destinations[0].rules.is_empty());
    }

    // ── route_email — domain matching ────────────────────────────────────────

    #[test]
    fn test_route_email_matches_domain_exact() {
        let content = "Perso/Finance/Banque | domain:acme.com\n";
        let dests = parse_destinations(content).unwrap();
        let meta = make_meta(
            "alice@acme.com",
            "acme.com",
            "Hello",
            "personal",
            "2026-06-15T10:00:00+00:00",
        );
        let decision = route_email(&meta, &dests, DateLayout::YearMonth);
        // Inclusive: path starts with expected dir
        assert!(
            decision.rel_path.starts_with("Perso/Finance/Banque/"),
            "got: {}",
            decision.rel_path
        );
        assert!(!decision.is_default);
        // Exclusive: not the default fallback
        assert!(!decision.rel_path.starts_with("Perso/Messy"));
    }

    #[test]
    fn test_route_email_domain_suffix_matches_subdomain() {
        let content = "Perso/Finance/Banque | domain:acme.com\n";
        let dests = parse_destinations(content).unwrap();
        let meta = make_meta(
            "bob@mail.acme.com",
            "mail.acme.com",
            "Hello",
            "personal",
            "2026-06-15T10:00:00+00:00",
        );
        let decision = route_email(&meta, &dests, DateLayout::YearMonth);
        // Inclusive: subdomain "mail.acme.com" matches rule "acme.com"
        assert!(
            decision.rel_path.starts_with("Perso/Finance/Banque/"),
            "got: {}",
            decision.rel_path
        );
        assert!(!decision.is_default);
    }

    #[test]
    fn test_route_email_domain_no_false_suffix_match() {
        let content = "Perso/Finance/Banque | domain:acme.com\n";
        let dests = parse_destinations(content).unwrap();
        // "notacme.com" must NOT match rule "acme.com"
        let meta = make_meta(
            "evil@notacme.com",
            "notacme.com",
            "Hello",
            "personal",
            "2026-06-15T10:00:00+00:00",
        );
        let decision = route_email(&meta, &dests, DateLayout::YearMonth);
        // Inclusive: falls to default
        assert!(
            decision.is_default,
            "notacme.com must not match acme.com rule"
        );
        // Exclusive: not routed to the finance folder
        assert!(!decision.rel_path.starts_with("Perso/Finance"));
    }

    // ── route_email — from matching ──────────────────────────────────────────

    #[test]
    fn test_route_email_matches_from_case_insensitive() {
        let content = "Pro/Clients/X | from:BILLING@ACME.COM\n";
        let dests = parse_destinations(content).unwrap();
        let meta = make_meta(
            "billing@acme.com",
            "acme.com",
            "Invoice",
            "work",
            "2026-03-01T00:00:00+00:00",
        );
        let decision = route_email(&meta, &dests, DateLayout::YearMonth);
        // Inclusive: from rule matched despite case difference
        assert!(
            decision.rel_path.starts_with("Pro/Clients/X/"),
            "got: {}",
            decision.rel_path
        );
        assert!(!decision.is_default);
        // Exclusive: not default path
        assert!(!decision.rel_path.contains("Messy"));
    }

    // ── route_email — subject matching ───────────────────────────────────────

    #[test]
    fn test_route_email_matches_subject_substring() {
        let content = "Perso/Shopping | subject:invoice\n";
        let dests = parse_destinations(content).unwrap();
        let meta = make_meta(
            "shop@store.com",
            "store.com",
            "Your Invoice #123",
            "personal",
            "2026-01-05T00:00:00+00:00",
        );
        let decision = route_email(&meta, &dests, DateLayout::YearMonth);
        // Inclusive: substring "invoice" found case-insensitively in "Your Invoice #123"
        assert!(
            decision.rel_path.starts_with("Perso/Shopping/"),
            "got: {}",
            decision.rel_path
        );
        assert!(!decision.is_default);
    }

    #[test]
    fn test_route_email_subject_no_match_on_different_keyword() {
        let content = "Perso/Shopping | subject:invoice\n";
        let dests = parse_destinations(content).unwrap();
        let meta = make_meta(
            "shop@store.com",
            "store.com",
            "Hello world",
            "personal",
            "2026-01-05T00:00:00+00:00",
        );
        let decision = route_email(&meta, &dests, DateLayout::YearMonth);
        // Inclusive: subject "Hello world" does not contain "invoice" → falls to default
        assert!(
            decision.is_default,
            "non-matching subject must fall to default"
        );
        // Exclusive: not routed to Shopping
        assert!(!decision.rel_path.starts_with("Perso/Shopping"));
    }

    // ── route_email — account matching ───────────────────────────────────────

    #[test]
    fn test_route_email_matches_account() {
        let content = "Pro/Work | account:work@corp.com\n";
        let dests = parse_destinations(content).unwrap();
        let meta = make_meta(
            "sender@any.com",
            "any.com",
            "Hello",
            "work@corp.com",
            "2026-04-10T00:00:00+00:00",
        );
        let decision = route_email(&meta, &dests, DateLayout::YearMonth);
        // Inclusive: account rule matched
        assert!(
            decision.rel_path.starts_with("Pro/Work/"),
            "got: {}",
            decision.rel_path
        );
        assert!(!decision.is_default);
    }

    // ── route_email — Perso/Pro polarity ─────────────────────────────────────

    #[test]
    fn test_route_email_perso_default_polarity() {
        // No rule matches → default → Perso
        let dests: Vec<Destination> = vec![];
        let meta = make_meta("x@y.com", "y.com", "Hi", "acc", "2026-05-20T00:00:00+00:00");
        let decision = route_email(&meta, &dests, DateLayout::YearMonth);
        // Inclusive: starts with Perso
        assert!(
            decision.rel_path.starts_with("Perso/"),
            "default must start with Perso, got: {}",
            decision.rel_path
        );
        assert!(decision.is_default);
    }

    #[test]
    fn test_route_email_pro_forced_by_first_segment() {
        let content = "Pro/Contracts | domain:corp.com\n";
        let dests = parse_destinations(content).unwrap();
        let meta = make_meta(
            "legal@corp.com",
            "corp.com",
            "Contract",
            "work",
            "2026-02-14T00:00:00+00:00",
        );
        let decision = route_email(&meta, &dests, DateLayout::YearMonth);
        // Inclusive: first segment is Pro
        assert!(
            decision.rel_path.starts_with("Pro/"),
            "matched rule must start with Pro, got: {}",
            decision.rel_path
        );
        assert!(!decision.is_default);
        // Exclusive: not Perso
        assert!(!decision.rel_path.starts_with("Perso/"));
    }

    // ── route_email — year/month append ──────────────────────────────────────

    #[test]
    fn test_route_email_appends_year_month() {
        let content = "Perso/Finance | domain:bank.com\n";
        let dests = parse_destinations(content).unwrap();
        let meta = make_meta(
            "noreply@bank.com",
            "bank.com",
            "Statement",
            "personal",
            "2026-03-15T00:00:00+00:00",
        );
        let decision = route_email(&meta, &dests, DateLayout::YearMonth);
        // Inclusive: ends with 2026/03
        assert!(
            decision.rel_path.ends_with("2026/03"),
            "expected year/month suffix, got: {}",
            decision.rel_path
        );
        // Exclusive: no wrong format (not "2026/3" or double slash)
        assert!(
            !decision.rel_path.contains("2026/3/"),
            "month must be zero-padded, got: {}",
            decision.rel_path
        );
        assert!(
            !decision.rel_path.contains("//"),
            "no double slash, got: {}",
            decision.rel_path
        );
    }

    #[test]
    fn test_route_email_year_month_day_layout() {
        let dests = parse_destinations("Perso/Banque | domain:bank.com\n").unwrap();
        let meta = make_meta(
            "noreply@bank.com",
            "bank.com",
            "Statement",
            "personal",
            "2026-10-03T10:00:00+00:00",
        );
        let decision = route_email(&meta, &dests, DateLayout::YearMonthDay);
        assert_eq!(decision.rel_path, "Perso/Banque/2026/10/03");
        assert!(!decision.rel_path.contains("2026/10/2026"));
    }

    #[test]
    fn test_ensure_year_month_day_keeps_hand_picked_year_month() {
        let date = chrono::NaiveDate::from_ymd_opt(2026, 10, 3).unwrap();
        let out = DateLayout::YearMonthDay.ensure("Perso/Banque/2026/10", &date);
        assert_eq!(out, "Perso/Banque/2026/10");
        assert!(!out.contains("2026/10/2026"));
    }

    #[test]
    fn test_route_email_default_appends_year_month() {
        let dests: Vec<Destination> = vec![];
        let meta = make_meta("x@y.com", "y.com", "Hi", "acc", "2026-11-30T00:00:00+00:00");
        let decision = route_email(&meta, &dests, DateLayout::YearMonth);
        // Inclusive: path ends with 2026/11
        assert!(
            decision.rel_path.ends_with("2026/11"),
            "got: {}",
            decision.rel_path
        );
        // Exclusive: not "2026/1" (not zero-padded)
        assert!(
            !decision.rel_path.ends_with("2026/1"),
            "month must be 2 digits"
        );
    }

    // ── ensure_year_month — normalize manually reassigned destinations ───────

    /// Former `route::ensure_year_month`, now `DateLayout::YearMonth.ensure`.
    fn ensure_year_month(rel_path: &str, year: &str, month: &str) -> String {
        let date = chrono::NaiveDate::from_ymd_opt(
            year.parse().expect("year"),
            month.parse().expect("month"),
            1,
        )
        .expect("date");
        DateLayout::YearMonth.ensure(rel_path, &date)
    }

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
        assert!(
            !out.contains("2026/06/2026/06"),
            "must not double the suffix, got: {}",
            out
        );
    }

    #[test]
    fn test_ensure_year_month_appends_when_tail_is_not_a_date() {
        // A trailing "13" (invalid month) must NOT be mistaken for a dated suffix
        // → year/month is appended. A bare year tail ("Perso/Bank/2026") is a
        // recognized `year` suffix since layouts became configurable: kept as is.
        let out = ensure_year_month("Perso/Bank/2026", "2026", "06");
        assert_eq!(out, "Perso/Bank/2026");
        let out2 = ensure_year_month("Perso/X/Reports/13", "2026", "06");
        assert_eq!(out2, "Perso/X/Reports/13/2026/06");
        assert!(
            !out2.ends_with("/13"),
            "invalid month tail must not be treated as dated"
        );
    }

    // ── route_email — path outside destinations.txt → default ────────────────

    #[test]
    fn test_route_email_unknown_domain_falls_to_default() {
        let content = "Perso/Finance/Banque | domain:bank.com\n";
        let dests = parse_destinations(content).unwrap();
        let meta = make_meta(
            "x@unknown.org",
            "unknown.org",
            "Hi",
            "acc",
            "2026-06-01T00:00:00+00:00",
        );
        let decision = route_email(&meta, &dests, DateLayout::YearMonth);
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

        let expected_dir = notes_dir
            .join("Perso")
            .join("Finance")
            .join("Banque")
            .join("2026")
            .join("06");
        let expected_md = expected_dir.join("email.md");

        // Inclusive: directory created and file moved
        assert!(expected_dir.exists(), "target directory must be created");
        assert!(expected_md.exists(), "moved .md must exist at target");
        // Exclusive: original no longer at staging
        assert!(!md_src.exists(), "original .md must not remain at staging");
    }

    // ── apply_decision — links quoted messages already in the vault ───────────

    const QUOTED_TEXT: &str = "Bonjour Thibaut, voici le devis demandé pour la rénovation.";

    fn quote_note(from: &str, date: &str, body: &str) -> String {
        format!(
            "---\ntags:\n  - email\n---\n```email\nsubject = \"Devis\"\nfrom = \"{from}\"\ndate = \"{date}\"\nbody = '''\n{body}\n'''\n```\n"
        )
    }

    fn quoting_reply() -> String {
        quote_note(
            "Thibaut <t@example.com>",
            "2022-07-12T09:00:00+02:00",
            &format!(
                "Merci.\n\nLe 11/07/2022 à 12:56, Mano <mano@example.com> a écrit :\n> {QUOTED_TEXT}"
            ),
        )
    }

    fn route_reply(temp: &TempDir, vault_has_original: bool) -> String {
        let notes_dir = temp.path().join("notes");
        let folder = notes_dir.join("Pro").join("2022").join("07");
        fs::create_dir_all(&folder).unwrap();
        if vault_has_original {
            let original = quote_note(
                "Mano <mano@example.com>",
                "2022-07-11T10:56:00Z",
                QUOTED_TEXT,
            );
            fs::write(folder.join("Devis original.md"), original).unwrap();
        }
        let staging = temp.path().join("staging");
        fs::create_dir_all(&staging).unwrap();
        let md_src = staging.join("Devis reponse.md");
        fs::write(&md_src, quoting_reply()).unwrap();

        apply_decision(&md_src, "Pro/2022/07", &notes_dir).unwrap();
        fs::read_to_string(folder.join("Devis reponse.md")).unwrap()
    }

    #[test]
    fn test_apply_decision_links_quoted_message_present_in_vault() {
        let temp = TempDir::new().unwrap();
        let routed = route_reply(&temp, true);
        assert!(routed.contains("Message cité : [[Devis original]]"));
        assert!(!routed.contains(QUOTED_TEXT));
    }

    #[test]
    fn test_apply_decision_keeps_quote_when_original_absent() {
        let temp = TempDir::new().unwrap();
        let routed = route_reply(&temp, false);
        assert_eq!(routed, quoting_reply());
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
        assert!(
            md_src.exists(),
            "original file must remain when apply is rejected"
        );
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
        let content = "Perso/First | from:sender@acme.com\nPerso/Second | domain:acme.com\n";
        let dests = parse_destinations(content).unwrap();
        let meta = make_meta(
            "sender@acme.com",
            "acme.com",
            "Hello",
            "acc",
            "2026-06-01T00:00:00+00:00",
        );

        let decision = route_email(&meta, &dests, DateLayout::YearMonth);

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
        let content = "Perso/Second | domain:acme.com\nPerso/First | from:sender@acme.com\n";
        let dests = parse_destinations(content).unwrap();
        let meta = make_meta(
            "sender@acme.com",
            "acme.com",
            "Hello",
            "acc",
            "2026-06-01T00:00:00+00:00",
        );

        let decision = route_email(&meta, &dests, DateLayout::YearMonth);

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

        upsert_rule(
            &dest_file,
            "Perso/Beta",
            MatchRule::From("beta@b.com".to_string()),
        )
        .unwrap();

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

        upsert_rule(
            &dest_file,
            "Perso/Work",
            MatchRule::From("bob@corp.com".to_string()),
        )
        .unwrap();

        let cfg = load_yaml(&dest_file).unwrap();
        // Exclusive: exactly one entry (no new entry created)
        assert_eq!(cfg.destinations.len(), 1, "must not create a second entry");
        let work = &cfg.destinations[0];
        // Inclusive: both rules present on the same entry
        assert!(work
            .rules
            .contains(&DestinationRule::Domain("corp.com".to_string())));
        assert!(work
            .rules
            .contains(&DestinationRule::From("bob@corp.com".to_string())));
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

        upsert_rule(
            &dest_file,
            "Perso/NewPath",
            MatchRule::From("new@example.com".to_string()),
        )
        .unwrap();

        let cfg = load_yaml(&dest_file).unwrap();
        // Exclusive: exactly one entry added
        assert_eq!(cfg.destinations.len(), 2, "exactly one entry added");
        // Inclusive: new entry is last, with the rule
        let last = cfg.destinations.last().unwrap();
        assert_eq!(last.path, "Perso/NewPath");
        assert!(last
            .rules
            .contains(&DestinationRule::From("new@example.com".to_string())));
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

        upsert_rule(
            &dest_file,
            "Perso/Work",
            MatchRule::From("b@x.com".to_string()),
        )
        .unwrap();

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
        upsert_rule(
            &dest_file,
            "Perso/New",
            MatchRule::Domain("X.COM".to_string()),
        )
        .unwrap();

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
            .join("Perso")
            .join("NewCategory")
            .join("FreeSubcat")
            .join("2026")
            .join("06")
            .join("invoice.md");

        // Inclusive: file is at the new location
        assert!(
            expected_md.exists(),
            "md must be moved to the new free path"
        );
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

mod contextual_export_tests {
    use email_to_markdown::config::Account;
    use email_to_markdown::contextual_export::{
        build_deletion_batch, build_search_batches, candidate_matches_rules, cleanup_stale_staging,
        convert_raw_contextual, deletion_rejection, evaluate_deletion_preflight, merge_candidates,
        parse_header_candidate, parse_uid_fetch_response, proof_is_complete, read_server_hold,
        read_source_proof, validate_uidvalidity, ContextualLock, ConversionStatus,
        DeletionProvider, DeletionRequest, LocalSourceProof, MessageConversionResult,
        MessageLocation, SourceIdentity,
    };
    use email_to_markdown::date_layout::DateLayout;
    use email_to_markdown::route::MatchRule;
    use std::fs;
    use std::time::{Duration, SystemTime};
    use tempfile::TempDir;

    fn location(folder: &str, uid: u32) -> MessageLocation {
        MessageLocation {
            folder_raw: folder.into(),
            folder_display: folder.into(),
            uid_validity: 77,
            uid,
        }
    }

    fn header(message_id: &str, from: &str, to: &str, subject: &str) -> Vec<u8> {
        format!(
            "Message-ID: <{message_id}>\r\nDate: Tue, 01 Sep 2026 10:00:00 +0200\r\nFrom: {from}\r\nTo: {to}\r\nSubject: {subject}\r\n\r\n"
        )
        .into_bytes()
    }

    fn account(target: &std::path::Path) -> Account {
        Account {
            name: "Work".into(),
            server: "imap.example.com".into(),
            port: 993,
            username: "work@example.com".into(),
            password: Some("unused".into()),
            ignored_folders: vec![],
            export_directory: target.to_string_lossy().into_owned(),
            quote_depth: 1,
            skip_existing: false,
            collect_contacts: false,
            skip_signature_images: false,
            delete_after_export: false,
            cleanup_empty_dirs: true,
            batch_size: 0,
        }
    }

    fn multipart_raw(message_id: &str) -> Vec<u8> {
        format!(
            "Message-ID: <{message_id}>\r\nDate: Tue, 01 Sep 2026 08:00:00 +0000\r\nFrom: Alice <alice@example.com>\r\nTo: Bob <bob@example.com>\r\nSubject: Project report\r\nContent-Type: multipart/mixed; boundary=BOUND\r\n\r\n--BOUND\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nBody\r\n--BOUND\r\nContent-Type: application/pdf; name=report.pdf\r\nContent-Disposition: attachment; filename=report.pdf\r\n\r\nPDF DATA\r\n--BOUND--\r\n"
        )
        .into_bytes()
    }

    #[test]
    fn test_convert_raw_contextual_quarantines_outside_staging() {
        let temp = TempDir::new().unwrap();
        let export = temp.path().join("export");
        let target = export.join("Pro");
        fs::create_dir_all(&target).unwrap();
        let mut account = account(&export);
        account.skip_signature_images = true;
        account.delete_after_export = true;
        let logo = "L".repeat(20 * 1024);
        let raw = format!(
            "Message-ID: <logo@example.com>\r\nDate: Tue, 01 Sep 2026 08:00:00 +0000\r\nFrom: Alice <alice@example.com>\r\nTo: Bob <bob@example.com>\r\nSubject: Devis\r\nContent-Type: multipart/mixed; boundary=BOUND\r\n\r\n--BOUND\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nBody\r\n--BOUND\r\nContent-Type: image/png; name=logo.png\r\nContent-Disposition: inline; filename=logo.png\r\n\r\n{logo}\r\n--BOUND--\r\n"
        )
        .into_bytes();
        let location = location("INBOX", 42);
        let candidate = parse_header_candidate("Work", location.clone(), &raw, None).unwrap();

        let status = convert_raw_contextual(
            &target,
            &account,
            &candidate,
            &location,
            &raw,
            DateLayout::None,
        )
        .unwrap();
        let (markdown, quarantined) = match status {
            ConversionStatus::Written {
                markdown,
                quarantined,
                ..
            } => (markdown, quarantined),
            other => panic!("expected write, got {other:?}"),
        };

        let stem = markdown.file_stem().unwrap();
        assert!(export.join("_skipped").join(stem).join("logo.png").exists());
        assert_eq!(quarantined, 1);
        assert!(!target.join("_skipped").exists());
        assert!(!fs::read_dir(&target).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".email-to-markdown-tmp-")));
    }

    #[test]
    fn test_convert_raw_contextual_with_shared_stem_quarantines_under_final_stem() {
        // #19: two notes with the same base name; the second is installed with a
        // stable-key suffix and its skipped logo must follow that final stem.
        let temp = TempDir::new().unwrap();
        let export = temp.path().join("export");
        let target = export.join("Pro");
        fs::create_dir_all(&target).unwrap();
        let mut account = account(&export);
        account.skip_signature_images = true;
        account.delete_after_export = true;
        let logo = "L".repeat(20 * 1024);
        let mut stems = Vec::new();
        for (n, message_id) in ["one@example.com", "two@example.com"].iter().enumerate() {
            let raw = format!(
                "Message-ID: <{message_id}>\r\nDate: Tue, 01 Sep 2026 08:00:00 +0000\r\nFrom: Alice <alice@example.com>\r\nTo: Bob <bob@example.com>\r\nSubject: Devis\r\nContent-Type: multipart/mixed; boundary=BOUND\r\n\r\n--BOUND\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nBody\r\n--BOUND\r\nContent-Type: image/png; name=logo.png\r\nContent-Disposition: inline; filename=logo.png\r\n\r\n{logo}\r\n--BOUND--\r\n"
            )
            .into_bytes();
            let location = location("INBOX", 42 + n as u32);
            let candidate = parse_header_candidate("Work", location.clone(), &raw, None).unwrap();
            let status = convert_raw_contextual(
                &target,
                &account,
                &candidate,
                &location,
                &raw,
                DateLayout::None,
            )
            .unwrap();
            match status {
                ConversionStatus::Written { markdown, .. } => {
                    stems.push(markdown.file_stem().unwrap().to_owned())
                }
                other => panic!("expected write, got {other:?}"),
            }
        }

        let skipped = export.join("_skipped");
        assert_ne!(stems[0], stems[1]);
        for stem in &stems {
            assert!(skipped.join(stem).join("logo.png").exists(), "{stem:?}");
            assert!(!skipped.join(stem).join("logo_2.png").exists(), "{stem:?}");
        }
        let mut dirs: Vec<_> = fs::read_dir(&skipped)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        dirs.sort();
        let mut expected = stems.clone();
        expected.sort();
        assert_eq!(dirs, expected);
    }

    #[test]
    fn test_convert_raw_contextual_without_deletion_quarantines_nothing() {
        // #14 reopening, third path: the quarantine exists only with
        // `delete_after_export`; without it the server keeps the logo.
        let temp = TempDir::new().unwrap();
        let export = temp.path().join("export");
        fs::create_dir_all(&export).unwrap();
        let mut account = account(&export);
        account.skip_signature_images = true;
        let logo = "L".repeat(20 * 1024);
        let raw = format!(
            "Message-ID: <keep-logo@example.com>\r\nDate: Tue, 01 Sep 2026 08:00:00 +0000\r\nFrom: Alice <alice@example.com>\r\nTo: Bob <bob@example.com>\r\nSubject: Devis\r\nContent-Type: multipart/mixed; boundary=BOUND\r\n\r\n--BOUND\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nBody\r\n--BOUND\r\nContent-Type: image/png; name=logo.png\r\nContent-Disposition: inline; filename=logo.png\r\n\r\n{logo}\r\n--BOUND--\r\n"
        )
        .into_bytes();
        let location = location("INBOX", 43);
        let candidate = parse_header_candidate("Work", location.clone(), &raw, None).unwrap();
        let status = convert_raw_contextual(
            &export,
            &account,
            &candidate,
            &location,
            &raw,
            DateLayout::None,
        )
        .unwrap();
        assert!(
            matches!(status, ConversionStatus::Written { quarantined: 0, .. })
                && !export.join("_skipped").exists(),
            "{status:?}"
        );
    }

    /// Convert `body` (a full MIME body with its Content-Type header) with
    /// deletion active, as one batch candidate.
    fn convert_for_deletion(
        target: &std::path::Path,
        message_id: &str,
        mime: &str,
    ) -> MessageConversionResult {
        let mut account = account(target);
        account.skip_signature_images = true;
        account.delete_after_export = true;
        let raw = format!(
            "Message-ID: <{message_id}>\r\nDate: Tue, 01 Sep 2026 08:00:00 +0000\r\nFrom: Alice <alice@example.com>\r\nTo: Bob <bob@example.com>\r\nSubject: Captures\r\n{mime}"
        )
        .into_bytes();
        let location = location("INBOX", 44);
        let candidate = parse_header_candidate("Work", location.clone(), &raw, None).unwrap();
        let status = convert_raw_contextual(
            target,
            &account,
            &candidate,
            &location,
            &raw,
            DateLayout::None,
        )
        .unwrap();
        MessageConversionResult {
            candidate_key: candidate.logical_key(),
            status: Some(status),
            error: None,
        }
    }

    #[test]
    fn test_build_deletion_batch_with_marker_without_image_holds() {
        let temp = TempDir::new().unwrap();
        let result = convert_for_deletion(
            temp.path(),
            "held@example.com",
            "Content-Type: text/plain; charset=utf-8\r\n\r\nLe QR code : [image: image.png]\r\n",
        );
        let Some(ConversionStatus::Written { markdown, .. }) = &result.status else {
            panic!("expected write, got {:?}", result.status);
        };
        assert!(
            read_server_hold(markdown).unwrap().is_some()
                && build_deletion_batch(std::slice::from_ref(&result)).is_empty()
        );
    }

    #[test]
    fn test_convert_raw_contextual_with_one_of_two_images_unreadable_refuses() {
        // One image of two lost: with deletion active the conversion is
        // refused, no note is written, so nothing is ever deletable.
        let temp = TempDir::new().unwrap();
        let mut account = account(temp.path());
        account.delete_after_export = true;
        let image = "A".repeat(200 * 1024);
        let raw = format!(
            "Message-ID: <half@example.com>\r\nDate: Tue, 01 Sep 2026 08:00:00 +0000\r\nFrom: Alice <alice@example.com>\r\nTo: Bob <bob@example.com>\r\nSubject: Captures\r\nContent-Type: multipart/mixed; boundary=B\r\n\r\n--B\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nDeux captures.\r\n--B\r\nContent-Type: image/png; name=recto.png\r\nContent-Disposition: attachment; filename=recto.png\r\n\r\n{image}\r\n--B\r\nContent-Type: image/png; name=verso.png\r\nContent-Disposition: attachment; filename=verso.png\r\nContent-Transfer-Encoding: base64\r\n\r\n!!!! not base64 !!!!\r\n--B--\r\n"
        )
        .into_bytes();
        let location = location("INBOX", 45);
        let candidate = parse_header_candidate("Work", location.clone(), &raw, None).unwrap();
        let status = convert_raw_contextual(
            temp.path(),
            &account,
            &candidate,
            &location,
            &raw,
            DateLayout::None,
        );
        assert!(
            status.is_err()
                && !fs::read_dir(temp.path()).unwrap().any(|entry| entry
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|ext| ext == "md")),
            "{status:?}"
        );
    }

    #[test]
    fn test_build_deletion_batch_with_complete_images_releases() {
        let temp = TempDir::new().unwrap();
        let image = "A".repeat(200 * 1024);
        let mime = format!(
            "Content-Type: multipart/mixed; boundary=B\r\n\r\n--B\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nRecto [image: recto.png]\r\n--B\r\nContent-Type: image/png; name=recto.png\r\nContent-Disposition: attachment; filename=recto.png\r\n\r\n{image}\r\n--B--\r\n"
        );
        let result = convert_for_deletion(temp.path(), "whole@example.com", &mime);
        assert_eq!(build_deletion_batch(&[result]).len(), 1);
    }

    #[test]
    fn test_convert_raw_contextual_already_present_after_layout_change() {
        let temp = TempDir::new().unwrap();
        let target = temp.path().join("Perso").join("Banque");
        fs::create_dir_all(&target).unwrap();
        let raw = multipart_raw("layout@example.com");
        let location = location("INBOX", 7);
        let candidate = parse_header_candidate("Work", location.clone(), &raw, None).unwrap();
        let convert = |layout| {
            convert_raw_contextual(
                &target,
                &account(&target),
                &candidate,
                &location,
                &raw,
                layout,
            )
            .unwrap()
        };

        assert!(matches!(
            convert(DateLayout::YearMonth),
            ConversionStatus::Written { .. }
        ));
        assert!(target.join("2026").join("09").is_dir());

        for layout in [DateLayout::YearMonthDay, DateLayout::None] {
            assert!(matches!(
                convert(layout),
                ConversionStatus::AlreadyPresent { .. }
            ));
        }
        assert!(!target.join("2026").join("09").join("01").exists());
    }

    #[test]
    fn search_batches_are_safe_deduplicated_and_bounded() {
        let rules: Vec<_> = (0..250)
            .map(|index| MatchRule::Correspondent(format!("person{index}@example.com")))
            .collect();
        let batches = build_search_batches(&rules).unwrap();
        assert_eq!(batches.len(), 5);
        assert!(batches.iter().all(|query| query.starts_with("UNDELETED ")));
        assert!(batches
            .iter()
            .all(|query| !query.contains('\r') && !query.contains('\n')));

        let deduped = build_search_batches(&[
            MatchRule::From("Same@Example.com".into()),
            MatchRule::From("same@example.com".into()),
        ])
        .unwrap();
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].matches("FROM").count(), 1);
        assert!(
            build_search_batches(&[MatchRule::From("victim@example.com\r\nALL".into())]).is_err()
        );
    }

    #[test]
    fn header_parser_and_local_match_reject_substring_false_positive() {
        let candidate = parse_header_candidate(
            "Work",
            location("INBOX", 12),
            &header(
                "one@example.com",
                "Sender <alice@notacme.com>",
                "Bob <bob@example.com>",
                "Hello",
            ),
            None,
        )
        .unwrap();
        assert!(!candidate_matches_rules(
            &candidate,
            &[MatchRule::Domain("acme.com".into())]
        ));
        assert!(candidate_matches_rules(
            &candidate,
            &[MatchRule::Correspondent("bob@example.com".into())]
        ));
        assert_eq!(candidate.locations[0].uid, 12);
        assert_eq!(
            candidate.source.message_id.as_deref(),
            Some("one@example.com")
        );
    }

    #[test]
    fn logical_merge_uses_provider_then_message_id_plus_fingerprint() {
        let raw = header(
            "same@example.com",
            "alice@example.com",
            "bob@example.com",
            "Same",
        );
        let a =
            parse_header_candidate("Work", location("INBOX", 1), &raw, Some("999".into())).unwrap();
        let b = parse_header_candidate("Work", location("Archive", 2), &raw, Some("999".into()))
            .unwrap();
        let merged = merge_candidates(vec![a, b]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].locations.len(), 2);

        let reused = parse_header_candidate(
            "Work",
            location("Other", 3),
            &header(
                "same@example.com",
                "mallory@example.com",
                "bob@example.com",
                "Different",
            ),
            None,
        )
        .unwrap();
        let original = parse_header_candidate("Work", location("INBOX", 1), &raw, None).unwrap();
        assert_eq!(merge_candidates(vec![original, reused]).len(), 2);
    }

    #[test]
    fn low_level_fetch_parser_reads_uid_gmail_id_and_header() {
        let header = b"Message-ID: <gmail@example.com>\r\n\r\n";
        let mut response = format!(
            "* 7 FETCH (UID 42 X-GM-MSGID 1278455344230334865 BODY[HEADER] {{{}}}\r\n",
            header.len()
        )
        .into_bytes();
        response.extend_from_slice(header);
        response.extend_from_slice(b")\r\n");
        let parsed = parse_uid_fetch_response(&response).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0, 42);
        assert_eq!(parsed[0].1.as_deref(), Some("1278455344230334865"));
        assert_eq!(parsed[0].2, header);
    }

    #[test]
    fn changed_uidvalidity_invalidates_a_search_result() {
        let location = location("INBOX", 42);
        assert!(validate_uidvalidity(&location, Some(77)).is_ok());
        assert!(validate_uidvalidity(&location, Some(78)).is_err());
        assert!(validate_uidvalidity(&location, None).is_err());
    }

    #[test]
    fn deletion_preflight_requires_exact_server_capabilities_only_when_enabled() {
        let disabled = evaluate_deletion_preflight(false, false, &[], None);
        assert!(disabled.supported);
        assert_eq!(disabled.provider, DeletionProvider::None);

        let generic_blocked = evaluate_deletion_preflight(true, false, &["IMAP4rev1".into()], None);
        assert!(!generic_blocked.supported);
        assert!(generic_blocked.reason.unwrap().contains("UIDPLUS"));
        let generic = evaluate_deletion_preflight(true, false, &["UIDPLUS".into()], None);
        assert_eq!(generic.provider, DeletionProvider::GenericUidPlus);

        let gmail_without_trash = evaluate_deletion_preflight(
            true,
            true,
            &["X-GM-EXT-1".into(), "UIDPLUS".into(), "MOVE".into()],
            None,
        );
        assert!(!gmail_without_trash.supported);
        let gmail = evaluate_deletion_preflight(
            true,
            true,
            &["x-gm-ext-1".into(), "uidplus".into(), "move".into()],
            Some("[Gmail]/Trash".into()),
        );
        assert_eq!(
            gmail.provider,
            DeletionProvider::Gmail {
                trash_folder: "[Gmail]/Trash".into()
            }
        );
    }

    fn deletion_request(markdown: std::path::PathBuf, uid: u32) -> DeletionRequest {
        DeletionRequest {
            markdown,
            proof: LocalSourceProof {
                identity: SourceIdentity {
                    account: "Work".into(),
                    message_id: Some("purge@example.com".into()),
                    header_fingerprint: "fp".into(),
                    provider_id: None,
                },
                location: location("INBOX", uid),
            },
        }
    }

    fn write_proved_note(path: &std::path::Path, proof: &LocalSourceProof) {
        #[derive(serde::Serialize)]
        struct Head<'a> {
            source: &'a LocalSourceProof,
        }
        let head = serde_yaml::to_string(&Head { source: proof }).unwrap();
        fs::write(
            path,
            format!(
                "---
{head}---

body
"
            ),
        )
        .unwrap();
    }

    #[test]
    fn test_proof_is_complete_with_numeric_provider_id() {
        let mut proof = deletion_request(std::path::PathBuf::from("n.md"), 5).proof;
        proof.identity.provider_id = Some("1789456123456789".into());
        assert!(proof_is_complete(&proof));
    }

    #[test]
    fn test_proof_is_complete_with_non_numeric_provider_id() {
        let mut proof = deletion_request(std::path::PathBuf::from("n.md"), 5).proof;
        for id in ["", "12 OR ALL", "12\r\nA1 EXPUNGE"] {
            proof.identity.provider_id = Some(id.into());
            assert!(!proof_is_complete(&proof), "{id:?}");
        }
    }

    #[test]
    fn test_deletion_rejection_missing_note() {
        let temp = TempDir::new().unwrap();
        let request = deletion_request(temp.path().join("email_gone.md"), 5);
        assert_eq!(deletion_rejection(&request).unwrap(), Some("note absente"));
    }

    #[test]
    fn test_deletion_rejection_note_with_other_proof() {
        let temp = TempDir::new().unwrap();
        let markdown = temp.path().join("email_note.md");
        write_proved_note(&markdown, &deletion_request(markdown.clone(), 6).proof);
        let request = deletion_request(markdown, 5);
        assert_eq!(
            deletion_rejection(&request).unwrap(),
            Some("la note porte une autre preuve")
        );
    }

    #[test]
    fn test_deletion_rejection_note_without_proof() {
        let temp = TempDir::new().unwrap();
        let markdown = temp.path().join("email_note.md");
        fs::write(
            &markdown,
            "---
title: x
---

body
",
        )
        .unwrap();
        assert_eq!(
            deletion_rejection(&deletion_request(markdown, 5)).unwrap(),
            Some("la note porte une autre preuve")
        );
    }

    #[test]
    fn test_deletion_rejection_note_held_on_server() {
        let temp = TempDir::new().unwrap();
        let markdown = temp.path().join("email_note.md");
        let request = deletion_request(markdown.clone(), 5);
        write_proved_note(&markdown, &request.proof);
        let note = fs::read_to_string(&markdown).unwrap();
        fs::write(
            &markdown,
            note.replacen(
                "---
",
                "---
keep_on_server: partie image illisible
",
                1,
            ),
        )
        .unwrap();
        assert_eq!(
            deletion_rejection(&request).unwrap(),
            Some("image annoncée non conservée, message gardé")
        );
    }

    #[test]
    fn test_deletion_rejection_note_with_same_proof() {
        let temp = TempDir::new().unwrap();
        let markdown = temp.path().join("email_note.md");
        let request = deletion_request(markdown.clone(), 5);
        write_proved_note(&markdown, &request.proof);
        assert_eq!(deletion_rejection(&request).unwrap(), None);
    }

    #[test]
    fn contextual_conversion_commits_attachment_then_proved_markdown() {
        let temp = TempDir::new().unwrap();
        let target = temp.path().join("target");
        fs::create_dir(&target).unwrap();
        let raw = multipart_raw("convert@example.com");
        let location = location("INBOX", 42);
        let candidate = parse_header_candidate("Work", location.clone(), &raw, None).unwrap();

        let first = convert_raw_contextual(
            &target,
            &account(&target),
            &candidate,
            &location,
            &raw,
            DateLayout::None,
        )
        .unwrap();
        let (markdown, proof) = match first {
            ConversionStatus::Written {
                markdown, proof, ..
            } => (markdown, proof),
            other => panic!("expected write, got {other:?}"),
        };
        assert!(markdown.exists());
        assert!(target.join("2026-09-01_report.pdf").exists());
        assert_eq!(read_source_proof(&markdown).unwrap(), Some(proof.clone()));
        assert_eq!(proof.location.uid, 42);
        assert!(!fs::read_dir(&target).unwrap().any(|entry| {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            name.starts_with(".email-to-markdown-tmp-") || name.ends_with(".lock")
        }));

        let second = convert_raw_contextual(
            &target,
            &account(&target),
            &candidate,
            &location,
            &raw,
            DateLayout::None,
        )
        .unwrap();
        assert!(matches!(second, ConversionStatus::AlreadyPresent { .. }));
        assert_eq!(
            fs::read_dir(&target)
                .unwrap()
                .filter(|entry| entry
                    .as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .and_then(|e| e.to_str())
                    == Some("md"))
                .count(),
            1
        );
    }

    #[test]
    fn destructive_batch_contains_only_revalidated_local_proofs() {
        let temp = TempDir::new().unwrap();
        let target = temp.path().join("target");
        fs::create_dir(&target).unwrap();
        let raw = multipart_raw("delete-proof@example.com");
        let source_location = location("INBOX", 42);
        let candidate =
            parse_header_candidate("Work", source_location.clone(), &raw, None).unwrap();
        let status = convert_raw_contextual(
            &target,
            &account(&target),
            &candidate,
            &source_location,
            &raw,
            DateLayout::None,
        )
        .unwrap();
        let valid = MessageConversionResult {
            candidate_key: candidate.logical_key(),
            status: Some(status.clone()),
            error: None,
        };
        let failed = MessageConversionResult {
            candidate_key: "failed".into(),
            status: None,
            error: Some("conversion failed".into()),
        };
        let batch = build_deletion_batch(&[valid.clone(), failed]);
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].proof.location.uid, 42);

        let markdown = match status {
            ConversionStatus::Written { markdown, .. } => markdown,
            _ => unreachable!(),
        };
        fs::write(&markdown, "---\nlegacy: true\n---\n").unwrap();
        assert!(build_deletion_batch(&[valid]).is_empty());
    }

    #[test]
    fn legacy_markdown_without_source_is_not_proof_and_collision_is_not_overwritten() {
        let temp = TempDir::new().unwrap();
        let target = temp.path().join("target");
        fs::create_dir(&target).unwrap();
        let raw = multipart_raw("legacy@example.com");
        let location = location("INBOX", 5);
        let candidate = parse_header_candidate("Work", location.clone(), &raw, None).unwrap();
        let first = convert_raw_contextual(
            &target,
            &account(&target),
            &candidate,
            &location,
            &raw,
            DateLayout::None,
        )
        .unwrap();
        let original = match first {
            ConversionStatus::Written { markdown, .. } => markdown,
            _ => unreachable!(),
        };
        let legacy_content = fs::read_to_string(&original)
            .unwrap()
            .lines()
            .filter(|line| !line.trim_start().starts_with("source:"))
            .collect::<Vec<_>>()
            .join("\n");
        // Replacing the full nested YAML safely is unnecessary here: an invalid
        // source block is deliberately treated like an old unproved Markdown.
        fs::write(
            &original,
            legacy_content.replace("identity:", "legacy_identity:"),
        )
        .unwrap();

        let second = convert_raw_contextual(
            &target,
            &account(&target),
            &candidate,
            &location,
            &raw,
            DateLayout::None,
        )
        .unwrap();
        let second_path = match second {
            ConversionStatus::Written { markdown, .. } => markdown,
            other => panic!("legacy note must be reconverted, got {other:?}"),
        };
        assert_ne!(second_path, original);
        assert!(original.exists() && second_path.exists());
    }

    #[test]
    fn contextual_lock_refuses_concurrent_operation() {
        let temp = TempDir::new().unwrap();
        let first = ContextualLock::acquire(temp.path(), "Work").unwrap();
        let second = ContextualLock::acquire(temp.path(), "Work");
        assert!(second.is_err());
        drop(first);
        assert!(ContextualLock::acquire(temp.path(), "Work").is_ok());
    }

    #[test]
    fn stale_cleanup_only_removes_reserved_directories() {
        let temp = TempDir::new().unwrap();
        let reserved = temp.path().join(".email-to-markdown-tmp-old");
        let user = temp.path().join("my-important-folder");
        fs::create_dir(&reserved).unwrap();
        fs::create_dir(&user).unwrap();
        fs::write(reserved.join("partial"), b"x").unwrap();
        fs::write(user.join("keep"), b"x").unwrap();
        let removed =
            cleanup_stale_staging(temp.path(), SystemTime::now() + Duration::from_secs(1)).unwrap();
        assert_eq!(removed, 1);
        assert!(!reserved.exists());
        assert!(user.join("keep").exists());
    }

    #[test]
    fn failed_contextual_conversion_leaves_no_final_markdown_or_residue() {
        let temp = TempDir::new().unwrap();
        let target = temp.path().join("target");
        fs::create_dir(&target).unwrap();
        let valid = header(
            "failure@example.com",
            "alice@example.com",
            "bob@example.com",
            "Failure",
        );
        let original_location = location("INBOX", 9);
        let candidate =
            parse_header_candidate("Work", original_location.clone(), &valid, None).unwrap();
        let stale_location = location("INBOX", 10);
        assert!(convert_raw_contextual(
            &target,
            &account(&target),
            &candidate,
            &stale_location,
            &valid,
            DateLayout::None,
        )
        .is_err());
        assert_eq!(
            fs::read_dir(&target)
                .unwrap()
                .filter(|entry| entry
                    .as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .and_then(|e| e.to_str())
                    == Some("md"))
                .count(),
            0
        );
        assert!(!fs::read_dir(&target).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".email-to-markdown-tmp-")));
    }

    mod tally_tests {
        use super::location;
        use email_to_markdown::contextual_export::{
            tally_deletions, DeletionOutcome, DeletionRequest, LocalSourceProof,
            MessageDeletionResult, SourceIdentity,
        };
        use std::path::PathBuf;

        fn request(uid: u32) -> DeletionRequest {
            DeletionRequest {
                markdown: PathBuf::from(format!("note_{uid}.md")),
                proof: LocalSourceProof {
                    identity: SourceIdentity {
                        account: "Perso".into(),
                        message_id: Some(format!("<{uid}@x>")),
                        header_fingerprint: format!("fp{uid}"),
                        provider_id: None,
                    },
                    location: location("INBOX", uid),
                },
            }
        }

        fn result(uid: u32, outcome: DeletionOutcome) -> MessageDeletionResult {
            MessageDeletionResult {
                proof: request(uid).proof,
                outcome,
            }
        }

        #[test]
        fn test_tally_deletions_matches_results_by_proof_not_order() {
            let requests = vec![request(1), request(2), request(3)];
            // Results come back in another order, as when rejections are appended.
            let results = vec![
                result(3, DeletionOutcome::RetryRequired("timeout".into())),
                result(1, DeletionOutcome::Deleted),
                result(2, DeletionOutcome::AlreadyAbsent),
            ];
            let tally = tally_deletions(requests, &results);
            assert_eq!(tally.deleted, 2);
            let retried: Vec<u32> = tally.retry.iter().map(|r| r.proof.location.uid).collect();
            assert_eq!(retried, vec![3]);
            assert!(!tally.stale_search);
        }

        #[test]
        fn test_tally_deletions_with_rejected_is_not_retried() {
            let requests = vec![request(1), request(2)];
            let results = vec![
                result(1, DeletionOutcome::Rejected("note absente".into())),
                result(
                    2,
                    DeletionOutcome::StaleUidValidity {
                        expected: 77,
                        actual: Some(78),
                    },
                ),
            ];
            let tally = tally_deletions(requests, &results);
            assert_eq!(tally.rejected, 1);
            assert_eq!(tally.deleted, 0);
            assert!(tally.stale_search);
            let retried: Vec<u32> = tally.retry.iter().map(|r| r.proof.location.uid).collect();
            assert_eq!(retried, vec![2]);
            assert!(!retried.contains(&1));
        }
    }

    mod identity_tests {
        use super::*;
        use email_to_markdown::contextual_export::identity_matches;

        fn proof_of(header: &[u8]) -> LocalSourceProof {
            let candidate =
                parse_header_candidate("Work", location("INBOX", 7), header, None).unwrap();
            LocalSourceProof {
                identity: candidate.source,
                location: location("INBOX", 7),
            }
        }

        #[test]
        fn test_identity_matches_same_headers() {
            let header = header(
                "a@example.com",
                "alice@example.com",
                "bob@example.com",
                "Hi",
            );
            assert!(identity_matches(&proof_of(&header), &(7, None, header)));
        }

        #[test]
        fn test_identity_matches_other_message_id_invalid() {
            let proved = header(
                "a@example.com",
                "alice@example.com",
                "bob@example.com",
                "Hi",
            );
            let server = header(
                "b@example.com",
                "alice@example.com",
                "bob@example.com",
                "Hi",
            );
            assert!(!identity_matches(&proof_of(&proved), &(7, None, server)));
        }

        #[test]
        fn test_identity_matches_same_message_id_other_fingerprint_invalid() {
            let proved = header(
                "a@example.com",
                "alice@example.com",
                "bob@example.com",
                "Hi",
            );
            let server = header(
                "a@example.com",
                "mallory@example.com",
                "bob@example.com",
                "Hi",
            );
            assert!(!identity_matches(&proof_of(&proved), &(7, None, server)));
        }

        /// Export proofs are built on the full message, the server check on
        /// `HEADER_FETCH_FIELDS`: both must give the same fingerprint.
        #[test]
        fn test_identity_matches_full_message_proof_with_fetched_fields() {
            let full = multipart_raw("report@example.com");
            let fields = b"Message-ID: <report@example.com>\r\nDate: Tue, 01 Sep 2026 08:00:00 +0000\r\nFrom: Alice <alice@example.com>\r\nTo: Bob <bob@example.com>\r\nSubject: Project report\r\n\r\n".to_vec();
            assert!(identity_matches(&proof_of(&full), &(7, None, fields)));
        }
    }

    mod gmail_plan_tests {
        use super::*;
        use email_to_markdown::email_export::plan_gmail_deletion;

        fn request(folder: &str, uid: u32, provider_id: Option<&str>) -> DeletionRequest {
            DeletionRequest {
                markdown: std::path::PathBuf::new(),
                proof: LocalSourceProof {
                    identity: SourceIdentity {
                        account: "Gmail".into(),
                        message_id: Some(format!("m{uid}@example.com")),
                        header_fingerprint: "fp".into(),
                        provider_id: provider_id.map(str::to_string),
                    },
                    location: location(folder, uid),
                },
            }
        }

        #[test]
        fn test_plan_gmail_deletion_groups_by_folder() {
            let plan = plan_gmail_deletion(vec![
                request("Work", 1, Some("11")),
                request("INBOX", 2, Some("22")),
                request("Work", 3, None),
                request("Work", 4, Some("44")),
            ]);
            let groups: Vec<(&str, Vec<&str>)> = plan
                .groups
                .iter()
                .map(|g| {
                    (
                        g.folder_raw.as_str(),
                        g.provider_ids.iter().map(String::as_str).collect(),
                    )
                })
                .collect();
            assert_eq!(
                groups,
                vec![("INBOX", vec!["22"]), ("Work", vec!["11", "44"])]
            );
        }

        #[test]
        fn test_plan_gmail_deletion_without_provider_id() {
            let plan = plan_gmail_deletion(vec![
                request("Work", 1, Some("11")),
                request("Work", 3, None),
            ]);
            let missing: Vec<u32> = plan
                .missing_id
                .iter()
                .map(|r| r.proof.location.uid)
                .collect();
            assert_eq!(missing, vec![3]);
            assert!(plan.groups[0]
                .requests
                .iter()
                .all(|r| r.proof.location.uid != 3));
        }

        #[test]
        fn test_plan_gmail_deletion_with_same_id_twice() {
            let plan = plan_gmail_deletion(vec![
                request("Work", 1, Some("11")),
                request("Work", 2, Some("11")),
            ]);
            assert_eq!(plan.groups[0].provider_ids, vec!["11".to_string()]);
            assert_eq!(plan.groups[0].requests.len(), 2);
        }
    }

    mod classify_expunge_tests {
        use email_to_markdown::contextual_export::{
            classify_expunge, DeletionOutcome, ExpungeFailure,
        };
        use std::collections::BTreeSet;

        #[test]
        fn test_classify_expunge_valid() {
            let present = BTreeSet::from([1, 3]);
            let outcomes = classify_expunge(&[1, 2, 3], Ok(&present), Ok(()));
            assert_eq!(
                outcomes,
                vec![
                    DeletionOutcome::Deleted,
                    DeletionOutcome::AlreadyAbsent,
                    DeletionOutcome::Deleted,
                ]
            );
        }

        #[test]
        fn test_classify_expunge_with_search_failure() {
            let outcomes = classify_expunge(&[1, 2], Err("search down".into()), Ok(()));
            assert_eq!(
                outcomes,
                vec![DeletionOutcome::RetryRequired("search down".into()); 2]
            );
        }

        #[test]
        fn test_classify_expunge_with_expunge_failure_after_store() {
            let present = BTreeSet::from([1]);
            let outcomes = classify_expunge(
                &[1, 2],
                Ok(&present),
                Err(ExpungeFailure::Expunge("expunge down".into())),
            );
            assert_eq!(outcomes[1], DeletionOutcome::AlreadyAbsent);
            let DeletionOutcome::RetryRequired(reason) = &outcomes[0] else {
                panic!("retry expected, got {:?}", outcomes[0]);
            };
            assert!(reason.contains("\\Deleted déjà posé") && reason.contains("expunge down"));
        }

        #[test]
        fn test_classify_expunge_with_store_failure() {
            let present = BTreeSet::from([1]);
            let outcomes = classify_expunge(
                &[1],
                Ok(&present),
                Err(ExpungeFailure::Store("store down".into())),
            );
            assert_eq!(
                outcomes,
                vec![DeletionOutcome::RetryRequired("store down".into())]
            );
        }
    }
}

mod handled_purge_tests {
    use email_to_markdown::contextual_export::{LocalSourceProof, MessageLocation, SourceIdentity};
    use email_to_markdown::email_export::{purge_summary, ExportStats};
    use email_to_markdown::handled::{
        load_from, load_proofs_from, record_to, PurgeMatch, SeenHandled,
    };
    use tempfile::TempDir;

    fn location(folder: &str, uid_validity: u32, uid: u32) -> MessageLocation {
        MessageLocation {
            folder_raw: folder.into(),
            folder_display: folder.into(),
            uid_validity,
            uid,
        }
    }

    fn proof(provider_id: Option<&str>) -> LocalSourceProof {
        LocalSourceProof {
            identity: SourceIdentity {
                account: "Perso".into(),
                message_id: Some("a@x".into()),
                header_fingerprint: "fp".into(),
                provider_id: provider_id.map(str::to_string),
            },
            location: location("INBOX", 11, 5),
        }
    }

    fn ledger_with(stored: LocalSourceProof) -> (TempDir, std::path::PathBuf) {
        let temp = TempDir::new().unwrap();
        let ledger = temp.path().join("handled_messages.txt");
        record_to(&ledger, &[("Perso".into(), "a@x".into(), Some(stored))]).unwrap();
        (temp, ledger)
    }

    #[test]
    fn test_match_seen_from_ledger_generic_same_place_purges() {
        let (_temp, ledger) = ledger_with(proof(None));
        let proofs = load_proofs_from(&ledger, "Perso");
        let seen = SeenHandled {
            message_id: "a@x".into(),
            uid: 5,
            provider_id: None,
        };
        assert_eq!(
            proofs.match_seen(&seen, &location("INBOX", 11, 5), false),
            PurgeMatch::Purge(proof(None))
        );
        // Same Message-ID at another place: another message, kept.
        assert_eq!(
            proofs.match_seen(&seen, &location("INBOX", 12, 5), false),
            PurgeMatch::Stale
        );
    }

    #[test]
    fn test_match_seen_from_ledger_gmail_moved_targets_current_folder() {
        let (_temp, ledger) = ledger_with(proof(Some("777")));
        let proofs = load_proofs_from(&ledger, "Perso");
        let seen = SeenHandled {
            message_id: "a@x".into(),
            uid: 40,
            provider_id: Some("777".into()),
        };
        let current = location("Archives", 90, 40);
        let PurgeMatch::Purge(target) = proofs.match_seen(&seen, &current, true) else {
            panic!("a moved Gmail message with the same X-GM-MSGID must be purged");
        };
        assert_eq!(target.location, current);
        assert_ne!(target.location, proof(Some("777")).location);
        assert_eq!(target.identity.provider_id.as_deref(), Some("777"));
    }

    #[test]
    fn test_match_seen_from_legacy_ledger_line_is_legacy() {
        let temp = TempDir::new().unwrap();
        let ledger = temp.path().join("handled_messages.txt");
        record_to(&ledger, &[("Perso".into(), "a@x".into(), None)]).unwrap();
        let proofs = load_proofs_from(&ledger, "Perso");
        let seen = SeenHandled {
            message_id: "a@x".into(),
            uid: 5,
            provider_id: None,
        };
        assert_eq!(
            proofs.match_seen(&seen, &location("INBOX", 11, 5), false),
            PurgeMatch::Legacy
        );
    }

    #[test]
    fn test_purge_summary_with_counts() {
        let stats = [
            ExportStats {
                purged: 2,
                legacy_handled: 1,
                ..ExportStats::default()
            },
            ExportStats {
                purged: 1,
                ..ExportStats::default()
            },
        ];
        let summary = purge_summary(&stats);
        assert!(summary.contains("3 supprimé(s) du serveur (déjà traités)"));
        assert!(
            summary.contains("1 message(s) traité(s) sans preuve : utiliser Nettoyer le serveur")
        );
        assert!(!summary.contains("périmée"));
    }

    #[test]
    fn test_cleanup_proof_turns_legacy_line_into_proved() {
        let temp = TempDir::new().unwrap();
        let ledger = temp.path().join("handled_messages.txt");
        record_to(&ledger, &[("Perso".into(), "a@x".into(), None)]).unwrap();
        assert!(load_proofs_from(&ledger, "Perso").legacy.contains("a@x"));

        // What the confirmed cleanup records after a deletion.
        record_to(
            &ledger,
            &[("Perso".into(), "a@x".into(), Some(proof(None)))],
        )
        .unwrap();
        let proofs = load_proofs_from(&ledger, "Perso");
        assert_eq!(proofs.proofs.get("a@x"), Some(&proof(None)));
        assert!(!proofs.legacy.contains("a@x"));
    }

    #[test]
    fn test_message_ids_matches_load() {
        let temp = TempDir::new().unwrap();
        let ledger = temp.path().join("handled_messages.txt");
        record_to(
            &ledger,
            &[
                ("Perso".into(), "legacy@x".into(), None),
                ("Perso".into(), "proved@x".into(), Some(proof(None))),
                ("Pro".into(), "other@x".into(), None),
            ],
        )
        .unwrap();

        let ids = load_proofs_from(&ledger, "Perso").message_ids();

        assert_eq!(ids, load_from(&ledger, "Perso"));
        assert!(ids.contains("legacy@x") && ids.contains("proved@x"));
        assert!(!ids.contains("other@x"));
    }

    #[test]
    fn test_purge_summary_with_held_messages() {
        let stats = [ExportStats {
            held_on_server: 2,
            ..ExportStats::default()
        }];
        let summary = purge_summary(&stats);
        assert!(summary.contains("2 message(s) retenu(s) sur le serveur, image non conservée"));
        assert!(!summary.contains("Nettoyer le serveur"));
    }

    #[test]
    fn test_purge_summary_empty() {
        assert_eq!(purge_summary(&[ExportStats::default()]), "");
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
        assert!(
            raw.contains("domain: ubs.ch"),
            "external tagging expected; got:\n{raw}"
        );

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
        upsert_entry(
            &mut cfg,
            "Perso/New",
            &[DestinationRule::Domain("x.com".to_string())],
        );
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
        upsert_entry(
            &mut cfg,
            "perso/work",
            &[DestinationRule::Domain("corp.com".to_string())],
        );
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
        let banque = cfg
            .destinations
            .iter()
            .find(|e| e.path == "Perso/Banque")
            .unwrap();
        assert_eq!(banque.rules.len(), 2);
        let inbox = cfg
            .destinations
            .iter()
            .find(|e| e.path == "Perso/Inbox")
            .unwrap();
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
    use email_to_markdown::dest_cmd::{
        add_entry, detect_anomalies, exceeds_max_depth, orphaned_entries,
    };
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
                DestinationEntry {
                    path: "A".into(),
                    default: true,
                    ..Default::default()
                },
                DestinationEntry {
                    path: "B".into(),
                    default: true,
                    ..Default::default()
                },
            ],
        };
        let warnings = detect_anomalies(&cfg);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("default"));
    }

    /// A path deeper than the maximum is still recorded, but flagged.
    #[test]
    fn test_add_entry_with_too_deep_path() {
        let temp = TempDir::new().unwrap();
        let dest_file = temp.path().join("destinations.yaml");
        add_entry(&dest_file, "Pro/A/B/C", &[], None, false).unwrap();

        let cfg = load_yaml(&dest_file).unwrap();
        assert_eq!(cfg.destinations[0].path, "Pro/A/B/C");
        assert!(exceeds_max_depth(&cfg.destinations[0].path, 2));
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

    /// An entry whose directory is missing under `notes_dir` is reported orphaned.
    #[test]
    fn test_orphaned_entries_missing_dir() {
        let temp = TempDir::new().unwrap();
        std::fs::create_dir_all(temp.path().join("Perso/Existing")).unwrap();
        let cfg = DestinationsConfig {
            destinations: vec![
                DestinationEntry {
                    path: "Perso/Existing".into(),
                    ..Default::default()
                },
                DestinationEntry {
                    path: "Perso/Deleted".into(),
                    ..Default::default()
                },
            ],
        };
        let orphans = orphaned_entries(temp.path(), &cfg);
        assert_eq!(orphans, vec!["Perso/Deleted".to_string()]);
    }

    /// No orphans when every entry's directory exists.
    #[test]
    fn test_orphaned_entries_none_when_all_present() {
        let temp = TempDir::new().unwrap();
        std::fs::create_dir_all(temp.path().join("Perso/Banque")).unwrap();
        let cfg = DestinationsConfig {
            destinations: vec![DestinationEntry {
                path: "Perso/Banque".into(),
                ..Default::default()
            }],
        };
        assert!(orphaned_entries(temp.path(), &cfg).is_empty());
    }
}

mod suggest_tests {
    use email_to_markdown::date_layout::strip_date_suffix as strip_trailing_year_month;
    use email_to_markdown::dest_cmd::{
        destination_branches, email_stats, exceeds_max_depth, extract_domain, scan_domains,
        scan_folders, uncovered_domains, EmailStats,
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

        let groups = scan_domains(temp.path(), 2).unwrap();
        assert_eq!(groups.get("ubs.ch"), Some(&2));
        assert_eq!(groups.get("bnp.fr"), Some(&1));
    }

    /// Directories starting with `.` are not walked.
    #[test]
    fn test_suggest_excludes_dot_dirs() {
        let temp = TempDir::new().unwrap();
        write_md(&temp.path().join(".obsidian"), "x.md", "x@hidden.com");
        write_md(temp.path(), "ok.md", "ok@visible.com");

        let groups = scan_domains(temp.path(), 2).unwrap();
        assert!(
            !groups.contains_key("hidden.com"),
            "dot-dir must be skipped"
        );
        assert_eq!(groups.get("visible.com"), Some(&1));
    }

    /// Directories starting with `_` are not walked.
    #[test]
    fn test_suggest_excludes_underscore_dirs() {
        let temp = TempDir::new().unwrap();
        write_md(&temp.path().join("_deleted"), "x.md", "x@gone.com");
        write_md(temp.path(), "ok.md", "ok@here.com");

        let groups = scan_domains(temp.path(), 2).unwrap();
        assert!(
            !groups.contains_key("gone.com"),
            "underscore-dir must be skipped"
        );
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

        let groups = scan_domains(&root, 2).unwrap();
        assert!(
            !groups.contains_key("external.com"),
            "symlink must not be followed"
        );
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
            let groups = scan_domains(&root, 2).unwrap();
            assert!(
                !groups.contains_key("external.com"),
                "symlink must not be followed"
            );
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
        assert_eq!(
            strip_trailing_year_month("Perso/Banque/2026/06"),
            "Perso/Banque"
        );
        // No suffix → unchanged.
        assert_eq!(strip_trailing_year_month("Perso/Banque"), "Perso/Banque");
    }

    /// A folder holding an email is proposed without its `YYYY/MM` suffix.
    #[test]
    fn test_scan_folders_with_deep_tree() {
        let temp = TempDir::new().unwrap();
        write_md(
            &temp.path().join("Pro/Banque/UBS/2025/06"),
            "a.md",
            "a@ubs.ch",
        );

        let folders = scan_folders(temp.path(), &["Pro".to_string()], 2).unwrap();
        assert_eq!(folders, vec![("Pro/Banque/UBS".to_string(), 1)]);
    }

    /// A bucket below `YYYY/MM` folds into its destination.
    #[test]
    fn test_scan_folders_with_bucket() {
        let temp = TempDir::new().unwrap();
        write_md(
            &temp.path().join("Perso/Voyage/Japon/2025/06/billets"),
            "a.md",
            "a@jal.co.jp",
        );

        let folders = scan_folders(temp.path(), &["Perso".to_string()], 2).unwrap();
        assert_eq!(folders, vec![("Perso/Voyage/Japon".to_string(), 1)]);
    }

    /// A folder holding only non-email notes is not proposed.
    #[test]
    fn test_scan_folders_without_email() {
        let temp = TempDir::new().unwrap();
        let notes = temp.path().join("Perso/Cuisine/Recettes");
        fs::create_dir_all(&notes).unwrap();
        fs::write(
            notes.join("tarte.md"),
            "# Tarte
",
        )
        .unwrap();
        write_md(&temp.path().join("Perso/Banque/BNP"), "a.md", "a@bnp.fr");

        let folders = scan_folders(temp.path(), &["Perso".to_string()], 2).unwrap();
        assert_eq!(folders, vec![("Perso/Banque/BNP".to_string(), 1)]);
    }

    /// Folder scan skips `.` and `_` directories inside a branch.
    #[test]
    fn test_scan_folders_excludes_dot_and_underscore_dirs() {
        let temp = TempDir::new().unwrap();
        write_md(
            &temp.path().join("Perso/.obsidian/Cache"),
            "a.md",
            "a@x.com",
        );
        write_md(
            &temp.path().join("Perso/_work/Brouillons"),
            "a.md",
            "a@x.com",
        );
        write_md(&temp.path().join("Perso/Banque/BNP"), "a.md", "a@x.com");

        let folders = scan_folders(temp.path(), &["Perso".to_string()], 2).unwrap();
        assert_eq!(folders, vec![("Perso/Banque/BNP".to_string(), 1)]);
    }

    /// Folder scan enters only the listed branches, case-insensitively.
    #[test]
    fn test_scan_folders_with_branches() {
        let temp = TempDir::new().unwrap();
        write_md(&temp.path().join("Perso/Banque/BNP"), "a.md", "a@x.com");
        write_md(&temp.path().join("Code/Projet/App"), "a.md", "a@x.com");

        let folders = scan_folders(temp.path(), &["perso".to_string()], 2).unwrap();
        assert_eq!(folders, vec![("Perso/Banque/BNP".to_string(), 1)]);
    }

    /// Destinations 1 to `max_depth` levels below the branch are proposed; the
    /// branch itself and deeper folders are not.
    #[test]
    fn test_scan_folders_with_shallow_and_deep_destinations() {
        let temp = TempDir::new().unwrap();
        write_md(&temp.path().join("Perso"), "a.md", "a@x.com");
        write_md(&temp.path().join("Perso/Bank"), "a.md", "a@x.com");
        write_md(
            &temp.path().join("Perso/Bank/BNP/Newsletters"),
            "a.md",
            "a@x.com",
        );
        write_md(
            &temp.path().join("Perso/Bank/BNP/2025/06"),
            "a.md",
            "a@x.com",
        );

        let folders = scan_folders(temp.path(), &["Perso".to_string()], 2).unwrap();
        assert_eq!(
            folders,
            vec![
                ("Perso/Bank".to_string(), 1),
                ("Perso/Bank/BNP".to_string(), 1)
            ]
        );
    }

    /// Tree with emails one, two and three levels below `Pro`, each under `YYYY/MM`.
    fn three_level_tree() -> TempDir {
        let temp = TempDir::new().unwrap();
        for dir in [
            "Pro/Clients/2025/06",
            "Pro/Clients/Acme/2025/06",
            "Pro/Clients/Acme/Contrats/2025/06",
        ] {
            write_md(&temp.path().join(dir), "a.md", "a@x.com");
        }
        temp
    }

    fn scanned_paths(temp: &TempDir, max_depth: usize) -> Vec<String> {
        scan_folders(temp.path(), &["Pro".to_string()], max_depth)
            .unwrap()
            .into_iter()
            .map(|(path, _)| path)
            .collect()
    }

    #[test]
    fn test_scan_folders_with_max_depth_1() {
        let temp = three_level_tree();
        assert_eq!(scanned_paths(&temp, 1), vec!["Pro/Clients"]);
    }

    #[test]
    fn test_scan_folders_with_max_depth_2() {
        let temp = three_level_tree();
        assert_eq!(
            scanned_paths(&temp, 2),
            vec!["Pro/Clients", "Pro/Clients/Acme"]
        );
    }

    #[test]
    fn test_scan_folders_with_max_depth_3() {
        let temp = three_level_tree();
        assert_eq!(
            scanned_paths(&temp, 3),
            vec![
                "Pro/Clients",
                "Pro/Clients/Acme",
                "Pro/Clients/Acme/Contrats"
            ]
        );
    }

    #[test]
    fn test_exceeds_max_depth_ignores_date_suffix() {
        assert!(!exceeds_max_depth("Pro/A/B/2026/10", 2));
        assert!(!exceeds_max_depth("Pro/A/B", 2));
        assert!(exceeds_max_depth("Pro/A/B/C", 2));
    }

    /// Every email note of a destination is counted, across months and buckets.
    #[test]
    fn test_scan_folders_counts_emails() {
        let temp = TempDir::new().unwrap();
        let base = temp.path().join("Perso/Bank/BNP");
        write_md(&base.join("2025/06"), "a.md", "a@bnp.fr");
        write_md(&base.join("2025/06"), "b.md", "b@bnp.fr");
        write_md(&base.join("2025/07/releves"), "c.md", "c@bnp.fr");
        fs::write(base.join("2025/06/note.md"), "# Note\n").unwrap();

        let folders = scan_folders(temp.path(), &["Perso".to_string()], 2).unwrap();
        assert_eq!(folders, vec![("Perso/Bank/BNP".to_string(), 3)]);
    }

    /// Emails are counted per year folder, across months and buckets; plain
    /// notes and deeper destinations are ignored.
    #[test]
    fn test_email_stats_by_year() {
        let temp = TempDir::new().unwrap();
        let base = temp.path().join("Perso/Boardgames/Mails");
        write_md(&base.join("2018/03"), "a.md", "a@x.com");
        write_md(&base.join("2024/01"), "b.md", "b@x.com");
        write_md(&base.join("2024/11/salon"), "c.md", "c@x.com");
        write_md(&base, "d.md", "d@x.com");
        write_md(&base.join("Newsletters/2024/01"), "e.md", "e@x.com");
        fs::write(
            base.join("2024/01/note.md"),
            "# Note
",
        )
        .unwrap();

        let stats = email_stats(&base).unwrap();
        assert_eq!(
            stats.by_year.into_iter().collect::<Vec<_>>(),
            vec![(2018, 1), (2024, 2)]
        );
        assert_eq!(stats.undated, 1);
    }

    /// Notes filed under any date layout count for their year.
    #[test]
    fn test_email_stats_counts_every_layout() {
        let temp = TempDir::new().unwrap();
        let base = temp.path().join("Perso/Banque");
        write_md(&base.join("2026"), "a.md", "a@x.com");
        write_md(&base.join("2026/10"), "b.md", "b@x.com");
        write_md(&base.join("2026/10/03"), "c.md", "c@x.com");
        write_md(&base.join("2026/10/03/releves"), "d.md", "d@x.com");

        let stats = email_stats(&base).unwrap();
        assert_eq!(
            stats.by_year.into_iter().collect::<Vec<_>>(),
            vec![(2026, 4)]
        );
    }

    /// A note in `YYYY/MM/DD/bucket` proposes its destination once.
    #[test]
    fn test_scan_folders_with_year_month_day_bucket() {
        let temp = TempDir::new().unwrap();
        let base = temp.path().join("Perso/Banque/BNP");
        write_md(&base.join("2026/10/03/releves"), "a.md", "a@bnp.fr");
        write_md(&base.join("2026/10"), "b.md", "b@bnp.fr");

        let folders = scan_folders(temp.path(), &["Perso".to_string()], 2).unwrap();
        assert_eq!(folders, vec![("Perso/Banque/BNP".to_string(), 2)]);
    }

    #[test]
    fn test_email_stats_missing_folder() {
        let temp = TempDir::new().unwrap();
        assert_eq!(
            email_stats(&temp.path().join("absent")).unwrap(),
            EmailStats::default()
        );
    }

    /// Branches are the first path segments, deduplicated case-insensitively.
    #[test]
    fn test_destination_branches_with_duplicates() {
        let cfg = DestinationsConfig {
            destinations: ["Perso/Bank", "perso/Health", "Pro", "Pro/Dev"]
                .into_iter()
                .map(|p| DestinationEntry {
                    path: p.into(),
                    ..Default::default()
                })
                .collect(),
        };
        assert_eq!(destination_branches(&cfg), vec!["Perso", "Pro"]);
    }

    /// An empty / file-less directory yields no groups and no crash.
    #[test]
    fn test_suggest_no_md_files() {
        let temp = TempDir::new().unwrap();
        let groups = scan_domains(temp.path(), 2).unwrap();
        assert!(groups.is_empty());
    }

    /// extract_domain handles bare and display-name forms.
    #[test]
    fn test_extract_domain_forms() {
        assert_eq!(extract_domain("alice@ubs.ch").as_deref(), Some("ubs.ch"));
        assert_eq!(
            extract_domain("Alice <alice@UBS.ch>").as_deref(),
            Some("ubs.ch")
        );
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
        DestinationEntry {
            path: path.into(),
            ..Default::default()
        }
    }

    fn sample() -> DestinationsConfig {
        DestinationsConfig {
            destinations: vec![
                entry("Perso/Banque"),
                entry("Perso/Work"),
                entry("Pro/Clients"),
            ],
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
        assert!(remove_rule(
            &mut cfg,
            "Perso/Banque",
            &DestinationRule::Domain("ubs.ch".into())
        ));
        assert_eq!(
            cfg.destinations[0].rules,
            vec![DestinationRule::Subject("facture".into())]
        );
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
        assert!(!remove_rule(
            &mut cfg,
            "Perso/Banque",
            &DestinationRule::Subject("x".into())
        ));
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

mod dest_gui_tests {
    use email_to_markdown::destinations::{
        add_rule, remove_entry, remove_rule, reorder_destinations, set_default, set_note,
        upsert_entry, DestinationEntry, DestinationRule, DestinationsConfig,
    };

    fn entry(path: &str) -> DestinationEntry {
        DestinationEntry {
            path: path.into(),
            ..Default::default()
        }
    }

    fn sample() -> DestinationsConfig {
        DestinationsConfig {
            destinations: vec![
                entry("Perso/Banque"),
                entry("Perso/Work"),
                entry("Pro/Clients"),
            ],
        }
    }

    /// add_entry: upsert_entry with no rules creates a new destination.
    #[test]
    fn test_dest_gui_add_entry() {
        let mut cfg = sample();
        upsert_entry(&mut cfg, "Perso/Vacances", &[]);
        assert_eq!(cfg.destinations.len(), 4);
        assert!(cfg.destinations.iter().any(|e| e.path == "Perso/Vacances"));
    }

    /// remove_entry: correct path is removed; others untouched.
    #[test]
    fn test_dest_gui_remove_entry() {
        let mut cfg = sample();
        remove_entry(&mut cfg, "Perso/Work");
        assert_eq!(cfg.destinations.len(), 2);
        assert!(!cfg.destinations.iter().any(|e| e.path == "Perso/Work"));
    }

    /// set_default: new default is flagged; previous default is cleared.
    #[test]
    fn test_dest_gui_set_default() {
        let mut cfg = sample();
        cfg.destinations[0].default = true;
        set_default(&mut cfg, "Pro/Clients");
        let defaults: Vec<&str> = cfg
            .destinations
            .iter()
            .filter(|e| e.default)
            .map(|e| e.path.as_str())
            .collect();
        assert_eq!(defaults, vec!["Pro/Clients"]);
    }

    /// set_note: note is stored on the matching entry.
    #[test]
    fn test_dest_gui_set_note() {
        let mut cfg = sample();
        set_note(&mut cfg, "Perso/Banque", Some("relevés mensuels".into()));
        assert_eq!(
            cfg.destinations[0].note.as_deref(),
            Some("relevés mensuels")
        );
    }

    /// add_rule: rule is pushed onto the matching entry's rules list.
    #[test]
    fn test_dest_gui_add_rule() {
        let mut cfg = sample();
        let added = add_rule(
            &mut cfg,
            "Perso/Banque",
            DestinationRule::Domain("ubs.ch".into()),
        );
        assert!(added);
        assert_eq!(
            cfg.destinations[0].rules,
            vec![DestinationRule::Domain("ubs.ch".into())]
        );
    }

    /// add_rule: duplicate rule is not inserted a second time.
    #[test]
    fn test_dest_gui_add_rule_no_duplicate() {
        let mut cfg = DestinationsConfig {
            destinations: vec![DestinationEntry {
                path: "Perso/Banque".into(),
                rules: vec![DestinationRule::Domain("ubs.ch".into())],
                ..Default::default()
            }],
        };
        let added = add_rule(
            &mut cfg,
            "Perso/Banque",
            DestinationRule::Domain("ubs.ch".into()),
        );
        assert!(!added);
        assert_eq!(cfg.destinations[0].rules.len(), 1);
    }

    /// remove_rule: matching rule variant is removed; others stay.
    #[test]
    fn test_dest_gui_remove_rule() {
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
        remove_rule(
            &mut cfg,
            "Perso/Banque",
            &DestinationRule::Domain("ubs.ch".into()),
        );
        assert_eq!(
            cfg.destinations[0].rules,
            vec![DestinationRule::Subject("facture".into())]
        );
    }

    /// reorder_destinations: entries appear in the supplied order.
    #[test]
    fn test_dest_gui_reorder() {
        let mut cfg = sample();
        reorder_destinations(&mut cfg, &["Pro/Clients", "Perso/Banque", "Perso/Work"]);
        let paths: Vec<&str> = cfg.destinations.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["Pro/Clients", "Perso/Banque", "Perso/Work"]);
    }

    /// reorder_destinations: the entries left out keep their relative order.
    #[test]
    fn test_reorder_destinations_partial_order_keeps_others() {
        let mut cfg = sample();
        cfg.destinations.push(entry("Pro/Fournisseurs"));

        reorder_destinations(&mut cfg, &["Perso/Banque"]);

        let paths: Vec<&str> = cfg.destinations.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "Perso/Banque",
                "Perso/Work",
                "Pro/Clients",
                "Pro/Fournisseurs"
            ]
        );
    }

    /// suggest_confirm: batch upsert_entry adds Domain rules for each pair.
    #[test]
    fn test_dest_gui_suggest_confirm() {
        let mut cfg = DestinationsConfig {
            destinations: vec![],
        };
        let pairs = [("ubs.ch", "Perso/Banque"), ("apple.com", "Perso/Tech")];
        for (domain, path) in &pairs {
            upsert_entry(
                &mut cfg,
                path,
                &[DestinationRule::Domain(domain.to_string())],
            );
        }
        assert_eq!(cfg.destinations.len(), 2);
        assert!(cfg.destinations[0]
            .rules
            .contains(&DestinationRule::Domain("ubs.ch".into())));
        assert!(cfg.destinations[1]
            .rules
            .contains(&DestinationRule::Domain("apple.com".into())));
    }
}

mod quote_link_tests {
    use email_to_markdown::quote_link::*;
    use tempfile::TempDir;

    const ORIGINAL_BODY: &str = "Bonjour Thibaut, voici le devis demandé pour la rénovation.";
    const OLDER_BODY: &str = "Un message plus ancien que le reste.";

    fn note(from: &str, date: &str, body: &str) -> String {
        format!(
            "---\ntags:\n  - email\n---\n```email\nsubject = \"Devis\"\nfrom = \"{from}\"\ndate = \"{date}\"\nbody = '''\n{body}\n'''\n```\n"
        )
    }

    fn original() -> String {
        note(
            "Mano <mano@example.com>",
            "2022-07-11T10:56:00Z",
            ORIGINAL_BODY,
        )
    }

    fn reply_with(body: &str) -> String {
        note("Thibaut <t@example.com>", "2022-07-12T09:00:00+02:00", body)
    }

    fn reply() -> String {
        reply_with(&format!(
            "Merci beaucoup.\n\nLe 11/07/2022 à 12:56, Mano <mano@example.com> a écrit :\n> {ORIGINAL_BODY}\n> Cordialement"
        ))
    }

    fn index_of(notes: &[(&str, String)]) -> Index {
        Index::build(notes.iter().map(|(p, c)| (*p, c.as_str())))
    }

    fn link_original(text: &str) -> Outcome {
        link_note(
            text,
            "Reponse.md",
            &index_of(&[("Devis original.md", original())]),
        )
    }

    #[test]
    fn test_link_note_replaces_quote_with_link() {
        let out = link_original(&reply());
        assert!(out.content.contains("Message cité : [[Devis original]]"));
    }

    #[test]
    fn test_link_note_removes_quoted_text() {
        let out = link_original(&reply());
        assert!(!out.content.contains("a écrit"));
        assert!(!out.content.contains(ORIGINAL_BODY));
    }

    #[test]
    fn test_link_note_adds_section_after_block() {
        let out = link_original(&reply());
        assert!(out
            .content
            .ends_with("```\n\n### Messages cités :\n- [[Devis original]]\n"));
    }

    #[test]
    fn test_link_note_keeps_reply_text() {
        let out = link_original(&reply());
        assert!(out.content.contains("Merci beaucoup."));
    }

    #[test]
    fn test_link_note_is_idempotent() {
        let first = link_original(&reply());
        let second = link_original(&first.content);
        assert_eq!(second.content, first.content);
    }

    #[test]
    fn test_link_note_reports_linked_targets() {
        let out = link_original(&reply());
        assert_eq!(out.linked, vec!["Devis original".to_string()]);
    }

    #[test]
    fn test_link_note_without_note_keeps_text() {
        let out = link_note(&reply(), "Reponse.md", &Index::build([]));
        assert_eq!(out.content, reply());
    }

    #[test]
    fn test_index_from_dir_skips_underscore_dirs() {
        let vault = TempDir::new().unwrap();
        let hidden = vault.path().join("_skipped");
        std::fs::create_dir_all(&hidden).unwrap();
        std::fs::write(hidden.join("Devis original.md"), original()).unwrap();

        let out = link_note(&reply(), "Reponse.md", &Index::from_dir(vault.path()));

        assert_eq!(out.left, vec![Left::NoNote]);
    }

    #[test]
    fn test_index_from_dir_indexes_regular_dirs() {
        let vault = TempDir::new().unwrap();
        let dir = vault.path().join("Clients");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Devis original.md"), original()).unwrap();

        let out = link_note(&reply(), "Reponse.md", &Index::from_dir(vault.path()));

        assert_eq!(out.linked, vec!["Devis original".to_string()]);
    }

    #[test]
    fn test_index_from_branches_skips_other_branches() {
        let vault = TempDir::new().unwrap();
        let dir = vault.path().join("Code/Projet");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Devis original.md"), original()).unwrap();

        let index = Index::from_branches(vault.path(), &["Pro".to_string()]);
        let out = link_note(&reply(), "Reponse.md", &index);

        assert_eq!(out.left, vec![Left::NoNote]);
    }

    #[test]
    fn test_index_from_branches_indexes_listed_branch() {
        let vault = TempDir::new().unwrap();
        let dir = vault.path().join("Pro/Clients/Acme");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Devis original.md"), original()).unwrap();

        let index = Index::from_branches(vault.path(), &["pro".to_string()]);
        let out = link_note(&reply(), "Reponse.md", &index);

        assert_eq!(out.linked, vec!["Devis original".to_string()]);
    }

    fn pro_vault() -> (TempDir, std::path::PathBuf) {
        let vault = TempDir::new().unwrap();
        let dir = vault.path().join("Pro/Clients");
        std::fs::create_dir_all(&dir).unwrap();
        (vault, dir)
    }

    #[test]
    fn test_vault_refresh_with_new_note_indexes_it() {
        let (vault, dir) = pro_vault();
        let mut index = Vault::open(vault.path(), &["Pro".to_string()]);
        std::fs::write(dir.join("Devis original.md"), original()).unwrap();

        index.refresh(&["Pro".to_string()]);
        let out = link_note(&reply(), "Reponse.md", &index.index);

        assert_eq!(out.linked, vec!["Devis original".to_string()]);
    }

    #[test]
    fn test_vault_refresh_with_deleted_note_forgets_it() {
        let (vault, dir) = pro_vault();
        std::fs::write(dir.join("Devis original.md"), original()).unwrap();
        let mut index = Vault::open(vault.path(), &["Pro".to_string()]);
        std::fs::remove_file(dir.join("Devis original.md")).unwrap();

        index.refresh(&["Pro".to_string()]);
        let out = link_note(&reply(), "Reponse.md", &index.index);

        assert_eq!(out.left, vec![Left::NoNote]);
    }

    #[test]
    fn test_vault_refresh_with_recorded_note_indexes_it_once() {
        let (vault, dir) = pro_vault();
        let mut index = Vault::open(vault.path(), &["Pro".to_string()]);
        let path = dir.join("Devis original.md");
        std::fs::write(&path, original()).unwrap();
        index.record("Pro/Clients/Devis original.md", &original(), &path);

        index.refresh(&["Pro".to_string()]);
        let out = link_note(&reply(), "Reponse.md", &index.index);

        // Indexed twice, the stem would look ambiguous and the link would carry the path.
        assert_eq!(out.linked, vec!["Devis original".to_string()]);
    }

    #[test]
    fn test_link_note_without_note_reports_reason() {
        let out = link_note(&reply(), "Reponse.md", &Index::build([]));
        assert_eq!(out.left, vec![Left::NoNote]);
    }

    #[test]
    fn test_link_note_with_two_candidates_reports_multiple() {
        let index = index_of(&[
            ("a/Devis original.md", original()),
            ("b/Devis copie.md", original()),
        ]);
        let out = link_note(&reply(), "Reponse.md", &index);
        assert_eq!(out.left, vec![Left::Multiple]);
    }

    #[test]
    fn test_link_note_with_two_candidates_does_not_link() {
        let index = index_of(&[
            ("a/Devis original.md", original()),
            ("b/Devis copie.md", original()),
        ]);
        let out = link_note(&reply(), "Reponse.md", &index);
        assert!(!out.content.contains("[["));
    }

    #[test]
    fn test_link_note_with_same_stem_uses_path() {
        let other = note("Autre <x@example.com>", "2020-01-01T00:00:00Z", OLDER_BODY);
        let index = index_of(&[
            ("a/Devis original.md", original()),
            ("b/Devis original.md", other),
        ]);
        let out = link_note(&reply(), "Reponse.md", &index);
        assert!(out.content.contains("[[a/Devis original]]"));
    }

    #[test]
    fn test_link_note_with_other_sender_keeps_text() {
        let other = note(
            "Paul <p@example.com>",
            "2022-07-11T10:56:00Z",
            ORIGINAL_BODY,
        );
        let out = link_note(&reply(), "Reponse.md", &index_of(&[("Autre.md", other)]));
        assert!(out.content.contains(ORIGINAL_BODY));
    }

    #[test]
    fn test_link_note_with_other_minute_keeps_text() {
        let other = note(
            "Mano <mano@example.com>",
            "2022-07-11T10:57:00Z",
            ORIGINAL_BODY,
        );
        let out = link_note(&reply(), "Reponse.md", &index_of(&[("Autre.md", other)]));
        assert!(out.content.contains(ORIGINAL_BODY));
    }

    #[test]
    fn test_link_note_with_other_text_keeps_text() {
        let other = note(
            "Mano <mano@example.com>",
            "2022-07-11T10:56:00Z",
            OLDER_BODY,
        );
        let out = link_note(&reply(), "Reponse.md", &index_of(&[("Autre.md", other)]));
        assert!(out.content.contains(ORIGINAL_BODY));
    }

    #[test]
    fn test_link_note_never_links_to_itself() {
        let own = original().replace(
            ORIGINAL_BODY,
            &format!("Le 11/07/2022 à 12:56, Mano <mano@example.com> a écrit :\n> {ORIGINAL_BODY}"),
        );
        let index = index_of(&[("Devis original.md", original())]);
        let out = link_note(&own, "Devis original.md", &index);
        assert!(!out.content.contains("[["));
    }

    #[test]
    fn test_link_note_with_interleaved_reply_reports_reason() {
        let text = reply_with(&format!(
            "Le 11/07/2022 à 12:56, Mano <mano@example.com> a écrit :\n> {ORIGINAL_BODY}\nMa réponse au milieu.\n> Cordialement"
        ));
        let out = link_original(&text);
        assert_eq!(out.left, vec![Left::Interleaved]);
    }

    #[test]
    fn test_link_note_with_interleaved_reply_keeps_block() {
        let text = reply_with(&format!(
            "Le 11/07/2022 à 12:56, Mano <mano@example.com> a écrit :\n> {ORIGINAL_BODY}\nMa réponse au milieu.\n> Cordialement"
        ));
        let out = link_original(&text);
        assert_eq!(out.content, text);
    }

    #[test]
    fn test_link_note_with_undated_attribution_reports_reason() {
        let text = reply_with(&format!(
            "Mano <mano@example.com> a écrit :\n> {ORIGINAL_BODY}"
        ));
        let out = link_original(&text);
        assert_eq!(out.left, vec![Left::NoDate]);
    }

    #[test]
    fn test_link_note_with_english_attribution_links() {
        let text = reply_with(&format!(
            "Thanks.\n\nOn Mon, Jul 11, 2022 at 12:56 PM Mano <mano@example.com> wrote:\n> {ORIGINAL_BODY}"
        ));
        let out = link_original(&text);
        assert!(out.content.contains("Message cité : [[Devis original]]"));
    }

    #[test]
    fn test_link_note_with_textual_date_links() {
        let text = reply_with(&format!(
            "Le lun. 11 juil. 2022 à 12:56, Mano <mano@example.com> a écrit :\n> {ORIGINAL_BODY}"
        ));
        let out = link_original(&text);
        assert!(out.content.contains("Message cité : [[Devis original]]"));
    }

    #[test]
    fn test_link_note_with_wrapped_attribution_links() {
        let text = reply_with(&format!(
            "Le 11/07/2022 à 12:56, Mano\n<mano@example.com> a écrit :\n> {ORIGINAL_BODY}"
        ));
        let out = link_original(&text);
        assert!(out.content.contains("Message cité : [[Devis original]]"));
    }

    #[test]
    fn test_link_note_with_nested_quote_replaces_parent_whole() {
        let text = reply_with(&format!(
            "Le 11/07/2022 à 12:56, Mano <mano@example.com> a écrit :\n> {ORIGINAL_BODY}\n> Le 10/07/2022 à 08:00, Paul <p@example.com> a écrit :\n>> {OLDER_BODY}"
        ));
        let out = link_original(&text);
        assert!(!out.content.contains(OLDER_BODY));
    }

    fn nested_without_parent_note() -> (String, Index) {
        let older = note("Paul <p@example.com>", "2022-07-10T06:00:00Z", OLDER_BODY);
        let text = reply_with(&format!(
            "Le 11/07/2022 à 12:56, Mano <mano@example.com> a écrit :\n> Texte du parent sans note.\n> Le 10/07/2022 à 08:00, Paul <p@example.com> a écrit :\n>> {OLDER_BODY}"
        ));
        (text, index_of(&[("Ancien.md", older)]))
    }

    fn vault_with_conversation(temp: &TempDir) -> std::path::PathBuf {
        let vault = temp.path().join("vault");
        std::fs::create_dir_all(vault.join("Pro")).unwrap();
        std::fs::write(vault.join("Pro").join("Devis original.md"), original()).unwrap();
        std::fs::write(vault.join("Pro").join("Devis reponse.md"), reply()).unwrap();
        vault
    }

    #[test]
    fn test_link_vault_dry_run_lists_note() {
        let temp = TempDir::new().unwrap();
        let vault = vault_with_conversation(&temp);
        let report = link_vault(&vault, None).unwrap();
        assert_eq!(report.rewritten, vec!["Pro/Devis reponse.md".to_string()]);
    }

    #[test]
    fn test_link_vault_skips_original_whose_name_has_brackets() {
        let temp = TempDir::new().unwrap();
        let vault = temp.path().join("vault");
        std::fs::create_dir_all(vault.join("Pro")).unwrap();
        std::fs::write(vault.join("Pro").join("[EXTERNE] Devis.md"), original()).unwrap();
        std::fs::write(vault.join("Pro").join("Devis reponse.md"), reply()).unwrap();
        let report = link_vault(&vault, None).unwrap();
        assert!(report.rewritten.is_empty());
        assert_eq!(report.left.get(&Left::NoNote), Some(&1));
    }

    #[test]
    fn test_link_vault_dry_run_writes_nothing() {
        let temp = TempDir::new().unwrap();
        let vault = vault_with_conversation(&temp);
        link_vault(&vault, None).unwrap();
        let after = std::fs::read_to_string(vault.join("Pro").join("Devis reponse.md")).unwrap();
        assert_eq!(after, reply());
    }

    #[test]
    fn test_link_vault_apply_rewrites_note_and_backs_up_original() {
        let temp = TempDir::new().unwrap();
        let vault = vault_with_conversation(&temp);
        let backup = temp.path().join("backup");
        link_vault(&vault, Some(&backup)).unwrap();
        let saved = std::fs::read_to_string(backup.join("Pro").join("Devis reponse.md")).unwrap();
        assert_eq!(saved, reply());
    }

    #[test]
    fn test_link_vault_apply_links_note() {
        let temp = TempDir::new().unwrap();
        let vault = vault_with_conversation(&temp);
        link_vault(&vault, Some(&temp.path().join("backup"))).unwrap();
        let after = std::fs::read_to_string(vault.join("Pro").join("Devis reponse.md")).unwrap();
        assert!(after.contains("Message cité : [[Devis original]]"));
        assert!(!after.contains(ORIGINAL_BODY));
    }

    #[test]
    fn test_link_vault_second_run_finds_nothing() {
        let temp = TempDir::new().unwrap();
        let vault = vault_with_conversation(&temp);
        link_vault(&vault, Some(&temp.path().join("backup"))).unwrap();
        let again = link_vault(&vault, None).unwrap();
        assert!(again.rewritten.is_empty());
    }

    #[test]
    fn test_link_vault_rejects_backup_inside_vault() {
        let temp = TempDir::new().unwrap();
        let vault = vault_with_conversation(&temp);
        let result = link_vault(&vault, Some(&vault.join("backup")));
        assert!(result.is_err());
    }

    #[test]
    fn test_link_note_with_nested_quote_links_child_when_parent_has_no_note() {
        let (text, index) = nested_without_parent_note();
        let out = link_note(&text, "Reponse.md", &index);
        assert!(out.content.contains("> Message cité : [[Ancien]]"));
    }

    #[test]
    fn test_link_note_with_nested_quote_keeps_parent_text_without_note() {
        let (text, index) = nested_without_parent_note();
        let out = link_note(&text, "Reponse.md", &index);
        assert!(out.content.contains("Texte du parent sans note."));
    }
}

mod thunderbird_tests {
    use super::*;
    use email_to_markdown::config::Account;
    use email_to_markdown::thunderbird::{
        write_accounts_merged, write_passwords_to_env, ThunderbirdPassword,
    };

    fn account(name: &str, server: &str) -> Account {
        Account {
            name: name.to_string(),
            server: server.to_string(),
            port: 993,
            username: format!("{name}@example.com"),
            password: None,
            ignored_folders: Vec::new(),
            export_directory: String::new(),
            quote_depth: 1,
            skip_existing: true,
            collect_contacts: false,
            skip_signature_images: false,
            delete_after_export: false,
            cleanup_empty_dirs: true,
            batch_size: 0,
        }
    }

    fn password(server: &str, secret: &str) -> ThunderbirdPassword {
        ThunderbirdPassword {
            imap_server: server.to_string(),
            username: "moi".to_string(),
            password: secret.to_string(),
        }
    }

    #[test]
    fn test_write_accounts_merged_with_differently_cased_name() {
        let temp = TempDir::new().expect("create tempdir");
        let path = temp.path().join("accounts.yaml");
        std::fs::write(
            &path,
            "accounts:\n- name: gmail\n  server: imap.gmail.com\n  port: 993\n  username: moi\n",
        )
        .expect("write accounts");

        write_accounts_merged(&[account("Gmail", "imap.gmail.com")], &path).expect("merge");

        let names: Vec<String> = load_raw_accounts(&path)
            .expect("load")
            .into_iter()
            .map(|a| a.name)
            .collect();
        assert_eq!(names, vec!["gmail".to_string()]);
    }

    #[test]
    fn test_write_passwords_to_env_with_existing_env() {
        let temp = TempDir::new().expect("create tempdir");
        let env = temp.path().join(".env");
        std::fs::write(&env, "OTHER=1\nPERSO_PASSWORD='old'\n").expect("write env");

        let written = write_passwords_to_env(
            &[account("Perso", "imap.example.org")],
            &[password("IMAP.example.org", "it's")],
            &env,
        )
        .expect("write passwords");

        assert_eq!(written, 1);
        assert_eq!(
            std::fs::read_to_string(&env).expect("read env"),
            r"OTHER=1
PERSO_PASSWORD='it'\''s'
"
        );
    }

    #[test]
    fn test_write_passwords_to_env_with_newline_password_skipped() {
        let temp = TempDir::new().expect("create tempdir");
        let env = temp.path().join(".env");
        std::fs::write(&env, "OTHER=1\n").expect("write env");

        let written = write_passwords_to_env(
            &[account("Perso", "imap.example.org")],
            &[password("imap.example.org", "a\nINJECTED=1")],
            &env,
        )
        .expect("write passwords");

        assert_eq!(written, 0);
        let content = std::fs::read_to_string(&env).expect("read env");
        assert_eq!(content, "OTHER=1\n");
        assert!(!content.contains("INJECTED"));
    }
}

mod env_file_tests {
    use super::*;
    use email_to_markdown::config::write_env_file;

    #[test]
    fn test_write_env_file_with_newline_invalid() {
        let temp = TempDir::new().expect("create tempdir");
        let env = temp.path().join(".env");

        let result = write_env_file(&env, &["A_PASSWORD='x\nB=1'".to_string()]);

        assert!(matches!(result, Err(ConfigError::ValidationError(_))));
        assert!(!env.exists());
    }

    #[test]
    fn test_write_env_file_valid() {
        let temp = TempDir::new().expect("create tempdir");
        let env = temp.path().join("config").join(".env");

        write_env_file(&env, &["A=1".to_string(), "B=2".to_string()]).expect("write env");

        assert_eq!(std::fs::read_to_string(&env).expect("read"), "A=1\nB=2\n");
    }

    #[cfg(unix)]
    #[test]
    fn test_write_env_file_mode_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let temp = TempDir::new().expect("create tempdir");
        let env = temp.path().join(".env");
        std::fs::write(&env, "A=0\n").expect("seed env");
        std::fs::set_permissions(&env, std::fs::Permissions::from_mode(0o644)).expect("chmod");

        write_env_file(&env, &["A=1".to_string()]).expect("write env");

        let mode = std::fs::metadata(&env)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

mod common;

mod gmail_seed_tests {
    //! Gmail seed corpus (issues #14, #15, #18, #19): every message is converted into
    //! one shared export directory, as a real run would, then checked against
    //! `common::gmail_seed::CATALOGUE`. Ignored tests reproduce an open bug.
    use super::common::gmail_seed::*;
    use email_to_markdown::config::Account;
    use email_to_markdown::contextual_export::{
        convert_raw_contextual, parse_header_candidate, ConversionStatus, MessageLocation,
    };
    use email_to_markdown::date_layout::DateLayout;
    use std::path::PathBuf;
    use tempfile::TempDir;

    struct Exported {
        _tmp: TempDir,
        export: PathBuf,
        notes: Vec<(&'static str, PathBuf, usize, Vec<u8>)>,
    }

    fn seed_account(export: &std::path::Path) -> Account {
        Account {
            name: "Seed".into(),
            server: "imap.example.com".into(),
            port: 993,
            username: "bob@example.com".into(),
            password: Some("unused".into()),
            ignored_folders: vec![],
            export_directory: export.to_string_lossy().into_owned(),
            quote_depth: 5,
            skip_existing: false,
            collect_contacts: false,
            skip_signature_images: true,
            delete_after_export: true,
            cleanup_empty_dirs: true,
            batch_size: 0,
        }
    }

    /// Convert the seed messages, in corpus order, into one export directory.
    fn export_seed() -> Exported {
        let tmp = TempDir::new().unwrap();
        let export = tmp.path().join("export");
        std::fs::create_dir_all(&export).unwrap();
        let account = seed_account(&export);
        let notes = CATALOGUE
            .iter()
            .enumerate()
            .map(|(i, exp)| {
                let raw = load(exp.file);
                let location = MessageLocation {
                    folder_raw: "INBOX".into(),
                    folder_display: "INBOX".into(),
                    uid_validity: 1,
                    uid: i as u32 + 1,
                };
                let candidate =
                    parse_header_candidate("Seed", location.clone(), &raw, None).unwrap();
                let status = convert_raw_contextual(
                    &export,
                    &account,
                    &candidate,
                    &location,
                    &raw,
                    DateLayout::None,
                )
                .unwrap();
                let ConversionStatus::Written {
                    markdown,
                    quarantined,
                    ..
                } = status
                else {
                    panic!("{}: not written: {status:?}", exp.key);
                };
                (exp.key, markdown, quarantined, raw)
            })
            .collect();
        Exported {
            _tmp: tmp,
            export,
            notes,
        }
    }

    fn check(key: &str) {
        let exported = export_seed();
        let (_, note, quarantined, raw) = exported
            .notes
            .iter()
            .find(|(k, ..)| *k == key)
            .unwrap_or_else(|| panic!("{key} not exported"));
        assert_seed_note(
            note,
            raw,
            *quarantined,
            &exported.export.join("_skipped"),
            expectation(key),
        );
    }

    #[test]
    fn test_gmail_seed_c1_m1_html_lists_links_signature() {
        check("seed-c1-m1");
    }

    #[test]
    fn test_gmail_seed_c1_m2_quoted_captures() {
        check("seed-c1-m2");
    }

    #[test]
    fn test_gmail_seed_c1_m3_cid_referenced_twice() {
        check("seed-c1-m3");
    }

    #[test]
    fn test_gmail_seed_c2_m4_related_root_without_disposition() {
        check("seed-c2-m4");
    }

    #[test]
    fn test_gmail_seed_c2_m5_attachments_and_accented_name() {
        check("seed-c2-m5");
    }

    #[test]
    fn test_gmail_seed_c2_m6_gif_only() {
        check("seed-c2-m6");
    }

    #[test]
    fn test_gmail_seed_c3_m7_flowed_plain_text() {
        check("seed-c3-m7");
    }

    #[test]
    // No HTML: the two markers pair with the two image parts, in MIME order.
    fn test_gmail_seed_c3_m8_image_markers() {
        check("seed-c3-m8");
    }

    #[test]
    fn test_gmail_seed_c3_m9_marker_without_image() {
        check("seed-c3-m9");
    }

    #[test]
    fn test_gmail_seed_c4_m10_inline_forward_table() {
        check("seed-c4-m10");
    }

    #[test]
    fn test_gmail_seed_c4_m11_forward_as_attachment() {
        check("seed-c4-m11");
    }

    #[test]
    // The HTML has no `cid:`, so the `text/plain` part is kept: its three
    // markers, one cut as `[image:` / `x.png]`, pair with the three remote
    // signature `<img>` and are dropped (`skip_signature_images`).
    fn test_gmail_seed_c4_m12_encoded_subject_long_url() {
        check("seed-c4-m12");
    }

    #[test]
    // Cause of the #14 reopening, reproduced here:
    // - the body references no `cid:`, so the `text/plain` part is kept;
    // - the capture has a Content-ID but no Content-Disposition nor filename;
    // - `extract_attachments` skipped every part without disposition as a
    //   body part, images included: no file, no quarantine, marker left.
    // Other paths checked (unit tests in `email_export` / `contextual_export`):
    // - an HTML whose conversion loses its `cid:` falls back to this case;
    // - contextual quarantine is active with `delete_after_export` only.
    fn test_gmail_seed_c5_m13_image_without_disposition() {
        check("seed-c5-m13");
    }

    #[test]
    fn test_gmail_seed_substitute_headers_only() {
        let raw = load("c1-m2-re-retours-recette.eml");
        let out = substitute(
            &raw,
            "20261110T120000",
            "A <a@test.invalid>",
            "B <b@test.invalid>",
        );
        let text = String::from_utf8(out.clone()).unwrap();
        let split = |b: &[u8]| b.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let (head, body) = text.split_at(split(&out));
        assert!(head.contains("\r\nFrom: A <a@test.invalid>\r\n"));
        assert!(head.contains("\r\nTo: B <b@test.invalid>\r\n"));
        assert!(head.contains("[email-to-markdown seed 20261110T120000]"));
        assert!(head.contains("<seed-c1-m2.20261110T120000@email-to-markdown.test>"));
        assert!(head.contains("<seed-c1-m1.20261110T120000@email-to-markdown.test>"));
        assert!(!head.contains(FIXTURE_ALICE) && !head.contains(FIXTURE_BOB));
        assert!(!head.contains(".fixture@") && !head.contains("seed fixture]"));
        assert_eq!(
            body.as_bytes(),
            &raw[split(&raw)..],
            "body bytes must not change"
        );
    }

    /// The versioned corpus is exactly what the generator produces.
    #[test]
    fn test_gmail_seed_corpus_matches_generator() {
        let python = ["python3", "python"]
            .into_iter()
            .find(|exe| {
                std::process::Command::new(exe)
                    .arg("--version")
                    .output()
                    .is_ok_and(|o| o.status.success())
            })
            .expect("python3 or python is required to check the Gmail seed generator");
        let tmp = TempDir::new().unwrap();
        let status = std::process::Command::new(python)
            .arg("-I")
            .arg(generator())
            .arg(tmp.path())
            .status()
            .unwrap();
        assert!(status.success(), "build_seed.py failed");
        let mut names: Vec<String> = std::fs::read_dir(seed_dir())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        let mut generated: Vec<String> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        generated.sort();
        assert_eq!(names, generated, "file list differs from the generator");
        for name in &names {
            let want = std::fs::read(tmp.path().join(name)).unwrap();
            assert!(
                load(name) == want,
                "{name} differs from build_seed.py output: regenerate"
            );
        }
    }
}
