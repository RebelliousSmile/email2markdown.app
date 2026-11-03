# Testing Guidelines

## Tools and Frameworks

- **Test runner:** `cargo test`
- **Integration tests:** `tests/rust_tests.rs`
- **IMAP tests:** `tests/imap_integration.rs`, Dovecot fixture `tests/fixtures/imap/compose.yaml`
- **Unit tests:** `#[cfg(test)] mod tests` inline in `src/` modules
- **Temp files:** `tempfile` crate (`TempDir`)
- **Email parsing in tests:** raw RFC 2822 bytes via `mailparse`

## Test Coverage

- **579 tests without `tray`, 668 with `--all-features`** (`cargo test --all-features -- --list`, 2026-11-03)
  - `--all-features`: 388 lib unit, 268 `rust_tests.rs`, 12 `imap_integration.rs` (all `#[ignore]`)
- Not covered automatically: `thunderbird.rs` (needs Thunderbird), tray event loop and windows (needs GUI), updater against GitHub, Gmail deletion
- `ImapExporter` covered by the Dovecot tests only (export, purge, contextual search, deletion outcomes, server cleanup)
- `tray_actions.rs` pure helpers covered inline under `--features tray`

## Testing Strategy

- Integration tests grouped in modules within `tests/rust_tests.rs`
- Config tests use `TempDir` — write YAML files then call `Config::load_with_settings()`
- Route tests use `TempDir` — real filesystem, real `load_yaml` / `upsert_rule` / `route_email` / `apply_decision` / `move_email`
- No shared test helpers file — data inlined or generated via `tempfile`
- Force a portable write failure: attachment filename > 255 bytes
  **Why:** fails on Windows and Unix alike, no permission setup needed
- IPC handlers keep their signature; their testable logic lives in functions that take the file paths as parameters
  **Why:** the handlers read `config::settings_path()` / `config::accounts_yaml_path()`, global paths — testing them directly would write into the user's real configuration
- Python tools: always run `python -m pytest tools/tests`, never bare `pytest` from the repository root
  **Why:** root collection walks `target/` (tens of GB) and `archive/`, takes minutes and ends in collection errors

## Test Modules

| Module | What it tests |
|--------|--------------|
| `utils_tests` | `limit_quote_depth`, `get_short_name`, `extract_emails`, `normalize_line_breaks`, `hash_md5_prefix`, `sanitize_filename`, `decode_imap_utf7`, `is_signature_image`, `should_skip_image` (referenced/unreferenced, non-image) |
| `config_tests` | `Config::validate`, save/load YAML roundtrip, `load_raw_accounts`/`save_accounts` round-trip |
| `settings_tests` | `Settings::default`, save/load YAML roundtrip, `Config::load_with_settings` merge scenarios, `AccountBehavior` per-account overrides round-trip, clear overrides removes YAML entry |
| `email_export_tests` | `analyze_email_type` (Direct/Newsletter/Group), `ContactsCollector::add`, `ExportStats::default` |
| `route_tests` | `parse_destinations` (ok, comments, duplicate default → error), `route_email` (domain/from/subject/account match, Perso default, Pro forced, year/month append, path outside destinations → default, AI off → default, **first-destination-in-file wins over second — two tests with reversed order confirm priority = file order**), `join_safe_segments` (rejects `..`/`.`/`\`/forbidden chars; accepts accented, space, dash, underscore), `move_email` (moves `.md` + `_attachments/` dir, rewrites paths, rejects symlink), `apply_decision` (creates missing dir, moves file), `upsert_rule` (YAML: merge, create-if-absent, dedup, absent-file create) |
| `destinations_tests` | `load_yaml` (absent → empty, round-trip, external-tag format), `upsert_entry` (new/dedup/path-only/multi-rule), `migrate_from_txt` (default + custom path), `save_yaml` rejects symlink |
| `dest_cmd_tests` | `add_entry` (path-only, domain rule, multi-flag, note, default conflict, invalid path), `detect_anomalies` (double default, empty) |
| `suggest_tests` | `scan_domains` (domain extraction, excludes `.`/`_` dirs, skips symlinks, no-md), `uncovered_domains` (filters covered), `strip_trailing_year_month`, `extract_domain` forms |
| `edge_case_tests` | Empty emails, Unicode, special chars, `normalize_line_breaks`, `is_signature_image` boundary values |
| `contextual_export_tests` | `convert_raw_contextual` incl. quarantine under real `export_directory/_skipped` |
| `cleaner_tests` | Body cleaner pipeline end to end |
| `handled_purge_tests` | Ledger `record_to`/`load_proofs_from`, `match_seen`, `LedgerProofs::message_ids` vs `load_from` |
| `dest_interactive_tests` | Interactive `dest` flows |
| `dest_gui_tests` | Destinations GUI state and IPC helpers (no window) |
| `quote_link_tests` | `Index::from_dir` (skips `.`/`_` dirs), `link_note`, `link_vault` |
| `server_cleanup::tests` (inline) | `cleanup_account` refusals, `plan` (Gmail labels), `notes_suffix`, `record_results` |
| `tray_actions::tests` (inline) | `clean_confirmation_text`, contextual summary, `set_export_dir_in`/`set_notes_dir_in` |
| `email_export::tests` (inline) | `html_to_markdown` signature `cid:` images (Gmail, Outlook, Thunderbird, look-alike class kept), `extract_body` HTML choice, `extract_attachments`, quarantine (counter, collisions, none without deletion, discard on failed export, `discard_quarantined`, `quarantine_warning`), issue #14 E2E with a 30 KB screenshot |
| `tray_actions::tests` (inline, `--features tray`) | `is_excluded_staging_dir` (incl. `_skipped`), `contextual_summary_message`, settings writers |
| `tray::ipc::tests` (inline, `--features tray`) | `move_account` (up, down case-insensitive, edge no-write, unknown name, bad direction), `move_outcome` (new order, missing direction), `delete_account` |
| `tray::windows::tests` (inline, `--features tray`) | `allow_navigation`, `escape_json_for_script`, `reply_changed_accounts` (`move_account` ok → true, failed → false) |
| `network_tests` | `NetworkConfig::default`, `ProgressIndicator::new`/`update`/`inc` |

## Test Execution

```bash
# All tests
cargo test

# Specific module
cargo test route_tests

# Specific test
cargo test test_route_email_appends_year_month

# With stdout
cargo test -- --nocapture

# Dovecot tests (Docker)
docker compose -f tests/fixtures/imap/compose.yaml down -v
docker compose -f tests/fixtures/imap/compose.yaml up -d --wait
EMAIL_TO_MARKDOWN_IMAP_TEST=1 cargo test --test imap_integration -- --ignored --skip gmail_ --test-threads=1
```

- `--test-threads=1`: run Dovecot tests serially, as validated by the audit plan

- CI job `dovecot` (`.github/workflows/ci.yml`): same three steps, `down -v` always
- `compose.yaml` seeds one user per test (`contextual`, `purge`, `clean-*`, `batch`); a new destructive test gets its own user
- CI job `gmail-destructive`: `workflow_dispatch` only, protected environment, dedicated Gmail account
  - Trigger: `gh workflow run ci.yml --ref main -f run_gmail_destructive=true`
  - Env secrets `EMAIL_TO_MARKDOWN_GMAIL_USER`, `EMAIL_TO_MARKDOWN_GMAIL_PASSWORD` (Gmail app password, not account password)
  - Never passed once yet (2026-10-31): password secret still the account password
- Restart the fixture (`down -v`) before a local rerun: tests delete seeded messages

## Mocking and Stubbing

- No mocking framework — real `tempfile` directories for config and route tests
- Real RFC 2822 byte strings for email parsing tests
- IMAP: real Dovecot container, never a mock; Thunderbird and GUI tested manually
- Never call `export_account`, `run_contextual_batch` or `clean_server` from a test: they use the user's real files
  - Drive the `*_in` variant with a `config::AppPaths` on a `TempDir`: `export_account_in`, `run_contextual_batch_in`, `server_cleanup::clean_server_in`
  - Helpers in `tests/imap_integration.rs`: `export_dir(username)`, `cleanup_dir(username, account)`
  - Unit-level: `handled::*_to`/`*_from` path variants and in-memory ledgers
- Dovecot coverage to extend, not duplicate:
  - Export body batching: `export_account_in_pairs_each_uid_with_its_body_across_batches`
  - Export purge: `export_account_in_purges_only_proved_handled_messages`
  - Identity re-check: `delete_discarded_forged_proof_is_rejected_and_keeps_messages`
  - Contextual unseen + UIDVALIDITY: `contextual_search_uses_uidvalidity_and_keeps_messages_unseen`

## Vault-wide rewrites

- Rehearse on a copy of the `.md` files only; vaults hold photos, videos and dumps that fill the disk
- Dry-run first, then `--apply --backup <dir outside vault>`, then a second dry-run that must report 0
- After applying, scan the vault for `[[…]]` targets that resolve to no file; real names (brackets, `%20`) expose cases unit tests miss
- Read a few diffs by hand, including a multi-reply conversation
