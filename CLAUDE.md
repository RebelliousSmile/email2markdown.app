# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Test

```bash
# CLI build (no GUI)
cargo build
cargo build --release

# Tray build (system tray + WebView UI)
cargo build --features tray
cargo run --features tray -- tray

# Tests
cargo test
cargo test <test_name>     # single test by name fragment
cargo clippy --features tray
```

The `tray` feature gates all GUI code (`tao`, `wry`, `tray-icon`, `rfd`). CLI builds are always available without it.

## Architecture

**Dual-mode app**: a CLI binary (`src/main.rs`, clap subcommands) and an optional tray application behind the `tray` feature flag.

### CLI subcommands

| Command | Module | Purpose |
|---------|--------|---------|
| `export` | `email_export.rs` | IMAP → Markdown pipeline |
| `sort` | `sort_emails.rs` | Score emails, write `sort_report.json` |
| `sort-apply` | `sort_emails.rs` | Apply decisions from `sort_report.json` |
| `fix` | `fix_yaml.rs` | Repair malformed YAML frontmatter |
| `import` | `thunderbird.rs` | Import accounts/passwords from Thunderbird |
| `tray` | `tray.rs` | Launch tray application |

### IMAP → Markdown pipeline (`email_export.rs`)

1. IMAP connect (native TLS) with retry/backoff (`network.rs`)
2. Enumerate folders (skip `ignored_folders`, decode IMAP UTF-7)
3. Parse email via `mailparse` (headers, MIME parts, attachments)
4. Clean body via `cleaner.rs` pipeline: QP decode → HTML entities → URL repair → unwrap lines → extract links → decontaminate trackers
5. HTML bodies converted with `htmd::convert()` inside `extract_body()`, never in `cleaner.rs`
6. Classify email type (Direct/Group/Newsletter/MailingList)
7. Write Markdown file with YAML frontmatter (`subject_hash` for dedup)

### Tray architecture (`tray.rs`, `tray_actions.rs`, `tray_sort_window.rs`)

- Main thread: `tao` event loop + MPSC receiver for results
- Each menu action: spawns an OS thread, sends `ActionResult` back via MPSC
- `ActionResult`: `Success`, `Error`, `Imported`, `SortCompleted`
- Sort review window: separate OS thread, its own `tao::EventLoop` (required on Windows via `with_any_thread(true)`), `wry` WebView embeds `assets/sort_review.html`
- WebView → Rust IPC: `window.ipc.postMessage(json)` (one-shot, triggers `should_exit` AtomicBool)

### Sort review window (`assets/sort_review.html`)

Vanilla JS (ES5), self-contained (no external deps). The `rows[]` model is the source of truth — `buildTable()` regenerates `<tbody>` only, so `<thead>` listeners survive re-renders. Report JSON is injected at load time via `__REPORT_JSON__` placeholder.

## Config system

Three files in the platform config dir (`%APPDATA%\email-to-markdown\` on Windows):

- **`accounts.yaml`** — IMAP connection info (server, port, username, ignored folders)
- **`settings.yaml`** — behaviour overrides per account and global defaults (export dir, quote depth, skip_existing, organize_by_type, sort weights)
- **`.env`** — passwords (`ACCOUNTNAME_APPLICATION_PASSWORD` or `ACCOUNTNAME_PASSWORD`)

Password env var key = account name uppercased, `@` and `.` replaced by `_AT_` and `_`.

Sort rules live in **`sort_config.json`** (same dir): keyword regexes, sender/subject whitelists, scoring weights, toggles.

## Test structure

Single integration file: `tests/rust_tests.rs`, grouped by `mod` (e.g., `mod utils_tests`, `mod config_tests`). Unit tests inline as `#[cfg(test)] mod tests`. Always use `tempfile::TempDir` for filesystem tests. Naming: `test_<function>_<condition>`.

## Key constraints

- Regex on hot paths (per-line/per-email): use `static LazyLock<Regex>`, never `Regex::new` inside a function body.
- No `.unwrap()` in `src/`; use `?` with `.context("…")` at module boundaries.
- Filesystem helpers must never follow symlinks; treat them as opaque content.
- Gmail IMAP: `SELECT [Gmail]/All Mail` + EXPUNGE; never EXPUNGE on the current folder.
- `MailParseError` → `stats.skipped`, not `stats.errors`.
