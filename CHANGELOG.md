# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2026-04-16

### Added

- Email body cleaner pipeline: decodes residual quoted-printable, HTML entities, strips invisible characters, detects mojibake, extracts social footers, reattaches wrapped URLs, unwraps 80-char line wraps, rewrites inline links as reference-style, strips tracker URL wrappers (`utm_*`, mailchimp, sendgrid), collapses whitespace. Runs after `normalize_line_breaks` in `export_to_markdown`. New `html-escape` and `url` dependencies.
- Silent cleanup of empty directories after each account export. New `cleanup_empty_dirs` toggle in `settings.yaml` (default `true`, overridable per account) recursively prunes empty directories from the account export tree at the end of every export, removing the `attachments/<folder>/` noise left behind for folders that contain no attachments. OS junk files (`Thumbs.db`, `.DS_Store`, `desktop.ini`) are deleted alongside their empty parent. Symlinks are never followed or deleted. Runs on every exit path of `export_account`, including when folder-level errors propagate.

### Fixed

- IMAP: skip mailboxes flagged `\Noselect` (e.g. Gmail's `[Gmail]` parent container) during folder listing. Previously the export aborted with `[NONEXISTENT] Unknown Mailbox: [Gmail]`.
- IMAP: use the raw modified-UTF-7 folder name received from the server when issuing `SELECT`, instead of the decoded display form. Previously folders with non-ASCII characters (e.g. `[Gmail]/Messages envoyés`) failed with `Bad Response: Could not parse command`. Introduces an internal `FolderName { raw, display }` struct.
- Config: align `.env` variable name sanitization across the loader, the `--generate-env` template writer and the `--extract-passwords` writer. Previously the template produced names like `FX.REBELLIOUS.SMILE@GMAIL.COM_PASSWORD` that the loader could never match (it queried `FX_REBELLIOUS_SMILE_GMAIL_COM_PASSWORD`), causing `No password found` errors on any account name containing `.` or `@`. The `--extract-passwords` flow also omitted the `_PASSWORD` suffix entirely. Extracted to a single canonical `config::env_var_name()` helper.
- Cleaner: decode RFC 2045 quoted-printable soft line breaks to prevent `=XX` leakage when an MTA wraps a QP-encoded sequence at ~76 bytes.

### Changed

- Malformed emails (RFC-invalid MIME — typically spam with broken multipart boundaries) are now classified as `skipped` rather than `errored` in the per-folder stats. Detection uses `e.downcast_ref::<mailparse::MailParseError>()` in the `export_folder` error arm. In `--debug` mode the offending message's raw RFC822 bytes are dumped to `<account>/_failed/<folder>_uid_N.eml` for post-mortem inspection.

## [0.1.0] - 2026-03-02

### Added

- Complete rewrite from Python to Rust
- System tray (optional `tray` feature): envelope icon, dynamic menu rebuild after import, disabled submenus without config
- System tray: export directory picker via folder browser dialog
- System tray: Thunderbird import with YesNoCancel dialog (import accounts / import with passwords / cancel)
- System tray: action-specific notification titles (Export terminé, Tri terminé, etc.)
- Thunderbird password extraction from NSS, written directly to `.env`
- Config split into three files: `accounts.yaml` (connection), `settings.yaml` (behaviour), `.env` (passwords)
- Platform-appropriate config directory: `%APPDATA%` (Windows), `~/.config` (Linux), `~/Library/Application Support` (macOS)

### Fixed

- System tray: silent notifications caused by `ControlFlow::Wait` (changed to `Poll`)
- System tray: Thunderbird profile detection now matches CLI logic (looks for `prefs.js`)
- `get_short_name()` incorrectly parsed `"Name <email>"` format (returned `JDJ` instead of `JD`)
- Two pre-existing test bugs: `get_short_name` assertion and `decode_imap_utf7` expected value

### Changed

- `accounts.yaml` no longer stores behaviour fields (moved to `settings.yaml`)
- Tray "Ouvrir config" menu item renamed to "Paramètres…"
- `ActionResult::Success` now carries `(title, message)` for per-action notification titles
