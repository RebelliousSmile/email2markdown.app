# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & test

- **Build**: `rtk cargo build --release` — binary at `target/release/email-to-markdown.exe`
- **Build with tray** (system tray icon, optional feature): `rtk cargo build --release --features tray`
- **Tests**: `rtk cargo test` — 93 tests across 4 suites, ~0.1s
- **Single module**: `rtk cargo test <module_name>` (e.g. `rtk cargo test config_tests`)
- **Single test case**: `rtk cargo test test_env_var_name_gmail -- --exact`
- **Lint**: `rtk cargo clippy --all-targets`
- **Linux system deps before build**: `sudo apt-get install build-essential pkg-config libssl-dev`

Unit tests are inline in each source module (`#[cfg(test)] mod tests`), integration tests live in `tests/rust_tests.rs` grouped by `mod`. Conventions: `.claude/rules/05-testing/5-rust-tests.md`.

## Architecture

### CLI dispatcher

`main.rs` is a clap dispatcher routing to 5 subcommands: `import`, `export`, `fix`, `sort`, `tray`. Each subcommand uses named-field structs with `#[derive(Subcommand)]`. See `.claude/rules/03-frameworks-and-libraries/3-clap.md`.

### 3-file config split

Config lives in a platform-aware directory (`%APPDATA%\email-to-markdown\` on Windows, `~/.config/email-to-markdown/` on Linux, `~/Library/Application Support/email-to-markdown/` on macOS) resolved by `config::app_config_dir()`. Never build these paths with string concatenation — always `PathBuf::join`.

- `accounts.yaml` — pure IMAP connection info (server/port/username/ignored_folders)
- `settings.yaml` — behaviour (`export_base_dir`, defaults, per-account overrides)
- `.env` — passwords. Variables are `{SANITIZED_NAME}_PASSWORD` or `_APPLICATION_PASSWORD` (APPLICATION takes priority). The canonical sanitizer is `config::env_var_name()` — always reuse it, never reimplement the rule elsewhere (this was the source of a past bug).

`RawAccount` (read from YAML) is merged with `Settings` via `merge_account()` to produce resolved `Account` structs (concrete fields, not `Option<T>`). Details in `docs/memory-bank/configuration.md`.

### Modules and dependencies

```
main.rs — clap dispatcher
 ├─ config.rs         — paths, Settings, Account, merge, validation
 ├─ email_export.rs   — ImapExporter, YAML frontmatter, ContactsCollector
 ├─ thunderbird.rs    — profiles, prefs.js, NSS password extraction, generate_*
 ├─ fix_yaml.rs       — Python-tag frontmatter repair (migration legacy)
 ├─ sort_emails.rs    — Delete/Summarize/Keep categorisation
 ├─ tray.rs           — tao event loop + dynamic menu       [feature: tray]
 └─ tray_actions.rs   — menu actions, spawned on threads    [feature: tray]
```

Module-level detail in `docs/memory-bank/module_structure.md`.

### Feature-gated tray

All tray code (`tray.rs`, `tray_actions.rs`) sits behind `#[cfg(feature = "tray")]`. Tray actions return an `ActionResult` (`Success(title, message)`, `Imported(message)`, `Error(message)`) — **never** propagate `Result` to the event loop or it panics. The menu rebuilds automatically after an import to reflect newly-added accounts.

### Error handling

- `thiserror` only for typed domain errors exported from a module (`ConfigError`)
- `anyhow::Result<T>` everywhere else, with `.context("…")` on every `?` at module boundaries
- No `.unwrap()` in `src/` outside test code
- Tray actions convert `Err` to `ActionResult::Error(String)` — never propagate

See `.claude/rules/02-programming-languages/2-rust-errors.md`.

## Reference documentation

- `docs/memory-bank/` — detailed architecture (module_structure, configuration, cross_platform, error_handling, testing_strategy) — **human-facing, French**
- `.claude/rules/` — active code rules (Rust errors/types, clap, serde, tests, mermaid, project-specific rules under `custom/`)
- `README.md` — user-facing CLI reference (French)

## Language convention

- LLM working files (this file, plans in `aidd_docs/tasks/`, rules) — **English**
- Human documentation (README, memory-bank, CHANGELOG) — **French**

## Project rules

All files under `.claude/rules/` are active:

- Rules with a `paths:` frontmatter auto-load when Claude Code opens matching files
- Rules without `paths:` are always loaded (see `.claude/rules/04-tooling/ide-mapping.md`)

Add new rules under `.claude/rules/custom/` — no CLAUDE.md edit required.
