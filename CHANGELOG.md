# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.12.0] - 2026-06-25

### Added

- Fenêtre de revue : sélecteur de dossier en cascade hiérarchique (Jalon A) — arbre de navigation à plusieurs niveaux construit depuis `destinations.txt`, partagé entre la barre d'outils (affectation en masse) et chaque ligne du tableau ; implémentation centralisée dans `renderCascade` (règle des trois).
- Fenêtre de revue : bouton « + règle » par ligne (Jalon B) — affiche un mini-formulaire pré-rempli (`domain:<domaine>` ou `from:<adresse>`) permettant de créer une règle de routage depuis l'email en cours de révision. Désactivé si expéditeur sans adresse connue.
- `upsert_rule(destinations_file, target_path, rule)` dans `route.rs` — injecte ou fusionne une règle (`MatchRule::Domain` ou `MatchRule::From`) dans `destinations.txt` par réécriture ligne à ligne : préserve commentaires, groupes (`# === X ===`), lignes vides et lignes sans rapport ; active les lignes commentées correspondantes ; déduplique les tokens ; rejette les symlinks (anti-guard). Seuls `domain:` et `from:` sont acceptés (pas `subject:` ni `account:`). Le domaine est toujours normalisé en minuscules.
- `extract_addr_and_domain` dans `tray.rs` — parse le champ `from` du frontmatter (`"Nom <email>"` ou adresse nue) et retourne `(adresse, domaine)` sans Regex.
- `RuleCreatePayload` / `IpcKind` dans `tray.rs` — discrimine les messages IPC entrants : `{action: "create_rule", ...}` déclenche `AppCommand::PersistRoutingRule` ; les payloads sans champ `action` (apply existant) passent inchangés.
- `AppCommand::PersistRoutingRule` — commande routée via `EventLoopProxy` : résout `destinations.txt`, construit le `MatchRule`, appelle `upsert_rule`, re-lit les chemins connus, injecte `route_review_set_tree([...])` dans la WebView.
- `window.route_review_set_tree` (JS) — reconstruit `knownPaths`, `datalist` et `pathTree` depuis le tableau reçu ; ré-applique la règle en attente (`pendingRule`) sur les lignes correspondantes ; marque `source = "rule"` ; incrémente `ruleOverrideCount` si la ligne avait été modifiée manuellement ; reconstruit le tableau.
- Suivi de `source` par ligne (`"default"` | `"manual"` | `"rule"`) : la saisie libre/cascade positionne `"manual"` ; la ré-application de règle positionne `"rule"`.
- Badge visuel `"règle"` (violet) dans la colonne Statut pour les lignes réaffectées par règle.
- Résumé enrichi : `N réaffectés par règle, dont M choix manuels écrasés` ajouté à `updateSummary()`.
- **Inversion D10** : l'application peut désormais écrire dans `destinations.txt` (précédemment lecture seule). La persistance est effectuée en main-thread via `EventLoopProxy` ; aucun déplacement de fichier n'a lieu avant le clic Appliquer (staging D6 préservé).

### Tests

- 6 tests `upsert_rule` ajoutés (`tests/rust_tests.rs`) : préservation commentaires/groupes, ordre des chemins, fusion sur ligne existante, création si absent, activation d'une ligne commentée, déduplication du token identique.
- Delta : 275 → 281 tests.

## [0.11.0] - 2026-06-25

### Added

- Nouveau module `route.rs` : routage déterministe des emails vers un arbre « second cerveau » défini dans `destinations.txt`. Règles de correspondance par `domain:`, `from:`, `subject:`, `account:` (insensible à la casse) ; append automatique `<Année>/<Mois>` ; défaut `Perso/Messy/Emails/<Année>/<Mois>` si aucune règle ne correspond.
- `destinations.txt` : format plat, une ligne = un chemin de destination, attributs de règle séparés par `|`. Au plus une ligne `default` — doublon = erreur fatale au parsing. Chemins listés mais absents du disque créés automatiquement (`mkdir -p`) à l'apply.
- `join_safe_segments` (extrait de `sort_emails.rs`) : protection anti-path-traversal — rejette `..`, `.`, `\` et les caractères hors `[A-Za-z0-9À-ſ _.\-]`.
- `move_email` : déplace le fichier `.md` **et** son répertoire `<stem>_attachments/` ensemble vers la destination, puis réécrit les chemins d'attachements dans le frontmatter YAML.
- `apply_decision` : apply direct hors GUI — utilisé par la commande CLI en mode automatique.
- Fenêtre de revue du routage (tray, `assets/route_review.html`) : colonnes Sujet / Expéditeur / Date / Chemin proposé / Défaut ? ; table triable ; réaffectation par autocomplétion depuis `destinations.txt` ou saisie libre d'un chemin nouveau ; validation `join_safe_segments` côté Rust avant tout déplacement. Aucun fichier n'est déplacé avant le clic Appliquer.
- Paramètre `notes_dir` (existant) documenté comme **racine du second cerveau** — les chemins de `destinations.txt` y sont joints.
- Paramètres `destinations_file` (chemin vers `destinations.txt`), `ai_routing_enabled` (défaut `false`), `ai_confidence_threshold` (défaut `0.7`) dans `settings.yaml`.
- Routage câblé dans la commande `export` : `destinations.txt` parsé **une seule fois** par `export_account`, `RouteDecision` calculée par email, apply automatique en CLI ou revue GUI.

### Fixed

- Documentation de `route_email` corrigée : la priorité de routage n'est **pas** hiérarchisée par type de règle (`Domain` > `From` > `Subject` > `Account`). La règle réelle est : les destinations sont évaluées dans l'ordre du fichier `destinations.txt` (première destination = priorité haute) ; au sein d'une destination, les règles sont évaluées dans l'ordre de déclaration sur la ligne. Le premier match gagne, quel que soit le type de règle.

### Removed

- Commandes `fix` (correction YAML Python), `sort` et `sort-apply` (catégorisation delete/summarize/keep) — supprimées ; l'application ne conserve que `import`, `export`, `tray`.
- Modules `fix_yaml.rs`, `sort_emails.rs`, `summarize.rs`, `notes_review.rs` entièrement retirés.
- Menus tray « Trier emails », « Résumer (Python) » et « Organiser les notes » supprimés.
- Champs `SortConfig`, `is_whitelisted`, `sort_config_path`, `python_venv_path`, `tools_dir`, `resolve_tools_*` retirés de `config.rs`.
- Asset `sort_review.html` remplacé par `route_review.html`.
- Pipeline Python `tools/` archivé dans un dépôt séparé — hors périmètre de cette application.

### Tests

- 30 tests `route_tests` : parsing `destinations.txt`, correspondance par domain/from/subject/account, défaut Perso, Pro forcé, append year/month, chemin hors fichier → défaut, IA off → défaut, `join_safe_segments` (rejet `..`/`.`/`\`/caractères interdits), `move_email` (déplacement `.md` + `_attachments/`, réécriture chemins, rejet symlink), `apply_decision` (création dossier manquant + déplacement).
- Delta tests M1→M8 : 326 → 275 (−53 : suppression des tests des modules retirés, +30 `route_tests`, +2 tests ordre-fichier multi-destinations).

## [0.10.0] - 2026-05-12

### Added

- Python tooling integration: the `email-to-markdown-tools` Python pipeline is now bundled under `tools/` and invoked via `std::process::Command`. New settings `python_venv_path`, `notes_dir`, `tools_dir` configure the venv interpreter and resolution paths. Helpers `resolve_tools_scripts_dir`, `resolve_tools_templates_dir`, `resolve_tools_data_dir`, `find_python` cascade `tools_dir → {exe_dir}/tools → {cwd}/tools` and fail with `ConfigError::ToolsDirNotFound` / `PythonVenvNotConfigured` / `PythonNotFound`.
- CLI: new `summarize` subcommand runs `tools/scripts/summarize.py` against one account's export directory (or all configured accounts when omitted). Streams stdout/stderr through the parent process and reloads `.env` explicitly so the Python child sees fresh `ANTHROPIC_API_KEY`.
- Tray: per-account "Résumer" entry and a new top-level "Organiser les notes" menu (folder or files picker via `rfd::FileDialog`). New "Choisir répertoire de notes…" config entry, mirroring "Choisir répertoire d'export".
- Sort review window: emails in the `summarize` category now get a "Destination notes" column pre-filled by `classify.py --batch` (with `ml` / `ollama` / `imap` / `fallback` / `error` badge and confidence). Supports multi-select bulk-assign, filter-then-assign on visible rows, "↓ vers les lignes du même expéditeur", and `<datalist>` autocomplete from `known_classes.json`. User confirmations feed back into the corpus via `classify.py --record-decision`.
- Organize notes window (post-export workflow): walks `.md` notes, parses frontmatter, displays a sortable / filterable table with multi-select. Available bulk actions: Classify, Summarize, Group, Apply template, Archive, Delete. New `apply_template.py` script renders Jinja2 templates (`StrictUndefined`, `trim_blocks`, `lstrip_blocks`) and supports `--input-files-stdin` for batches too large for argv.
- Templates: `tools/templates/sent_digest.md` (aperçu + détail des envois sur période) and `tools/templates/meeting_recap.md` (participants, fil chronologique, décisions à remplir).
- Apply-sort: when a `summarize` row has a confirmed `notes_destination`, the email is moved into `{notes_dir}/{destination}` instead of the generic `to-summarize/` bucket.
- Path-traversal guard: shared `sort_emails::join_safe_segments(root, dest)` rejects `..`, `.`, backslash, and characters outside `[A-Za-z0-9À-ſ _.\-]`. Used by both the sort apply and the Organize Classify action.

### Changed

- `EmailSummary.classify_method` is now a typed `Option<ClassifyMethod>` (serde tagged, `snake_case`) with a forward-compatible `#[serde(other)] Unknown` variant — replaces the prior `Option<String>` field.
- `folder_classifier.py` cold-start: prompts include locked IMAP hint levels (short-circuit when 3 levels are known), a reuse-first instruction over `known_classes`, and a single retry constrained to existing labels before falling back to rule-based paths. Eliminates duplicate label variants (`Travail/Projets/ClientX` vs `Pro/Projets/Client-X`) on repeated classification sessions.

### Tests

- Unit tests added for `join_safe_segments` (7 cases: nested paths, accented segments, empty/trim handling, `..`/`.` rejection, backslash rejection, forbidden chars), `find_python` (3 cases: not configured, binary missing, happy path), `resolve_tools_scripts_dir` happy path, and the tray helpers `ensure_unique_path`, `work_root`, `sanitize_name` (8 cases covering notes_dir present/missing, common-parent fallback, multi-parent bail, empty input bail, collision increments).

## [0.9.0] - 2026-05-12

### Added

- Contacts: CSV export now centralised in `{export_base_dir}/_local/contacts/{account}.csv` (one file per account) instead of per-account subdirectories. Columns match Thunderbird address book import format (`First Name`, `Last Name`, `Display Name`, `Email`, `Notes`). File written with UTF-8 BOM for correct encoding detection on Windows.
- Contacts: contacts are now also collected during the header pre-check phase, so emails that are skipped because they were already exported still contribute to the contacts CSV.

### Fixed

- Config GUI: per-account behaviour settings now persist correctly within the same config window session. `window.__SETTINGS_DATA__` was only injected at window open time; navigating back to the account list and re-opening the same account showed stale values. The in-memory snapshot is now updated immediately after each save.

## [0.8.0] - 2026-05-12

### Added

- Config GUI: "Comportement" settings split into two labeled subsections — "Export" (organize by type, delete after export, cleanup empty dirs, skip existing, collect contacts, skip signature images, quote depth) and "Tri" (organize by type for sort) — in both global defaults and per-account fieldsets.

### Fixed

- IMAP auth: automatically fall back to `AUTHENTICATE PLAIN` (SASL) when the server rejects the plain `LOGIN` command. Accommodates servers such as alwaysdata that disable LOGIN and require SASL mechanisms.
- Config: migrated from `dotenv` to `dotenvy` (maintained fork). `dotenv` 0.15 silently truncates passwords containing `#` (treated as comment) and drops `$` inside double-quoted values (variable substitution). Passwords written by `--extract-passwords` are now quoted with single quotes to prevent all substitution.
- Thunderbird import: `[Gmail]/Important` added to the default ignored folders list for Gmail accounts.

## [0.7.0] - 2026-05-12

### Added

- Tray: export progress window now shows a status line below the progress bar. It displays "Récupération des en-têtes…" while headers are being fetched, then updates to the per-folder result (e.g. `INBOX/jennifer — 0 exportés, 166 ignorés, 0 erreurs`) once the folder completes.

## [0.6.1] - 2026-05-12

### Fixed

- Export: progress line now fully cleared before printing the per-folder summary, preventing leftover characters from the progress bar from bleeding into the stats line. Duplicate stats print removed.

## [0.6.0] - 2026-05-12

### Added

- Export: virtual IMAP folders (Junk, Trash, Drafts, All Mail, Starred/Flagged, Important) are now automatically excluded via RFC 6154 SPECIAL-USE attributes, eliminating redundant downloads and locale-specific folder name maintenance.
- Export: when `skip_existing = true`, all message headers are fetched in a single IMAP batch call before downloading any body. Full RFC 822 fetch is issued only for messages not yet exported, drastically reducing network traffic on re-exports.

## [0.5.0] - 2026-05-12

### Added

- Tray: "Annuler" button in the export progress window stops the export cleanly after the current message finishes, without killing the process. Uses a shared `Arc<AtomicBool>` cancel token checked before each IMAP fetch and at each folder boundary. Other operations (Sort, Fix YAML/HTML, Import Thunderbird) are unaffected.

## [0.4.0] - 2026-05-12

### Added

- Settings GUI: per-account behavior overrides (organize by type, delete after export, cleanup empty dirs, skip existing, collect contacts, skip signature images, quote depth) in the Accounts tab.
- Sort: output of `sort-apply` now lands in a configurable `_local/` subfolder under the export base directory, keeping account folders and processing folders cleanly separated.

### Fixed

- Tray: all GUI windows (progress, sort review, settings) now run on the main-thread `tao` EventLoop instead of dedicated OS threads, eliminating tray freezes on Windows caused by per-thread `EventLoop` teardown.
- Tray: sort review window opens immediately when the scan completes — progress window closes automatically instead of requiring a manual "Fermer" click.
- Tray: "Fermer" and "Annuler" buttons in progress and sort review windows now work correctly (replaced non-functional `window.close()` with WebView2 IPC).
- Tray: stale proxy cleanup that incorrectly cleared newer window proxies on re-open.
- Export: Gmail "All Mail" folder is now discovered via RFC 6154 `\All` special-use attribute with locale-aware name fallback, fixing `[NONEXISTENT]` errors on non-English Gmail accounts.
- Cleaner: URL warnings suppressed for fragment-only anchors (`#`) and Markdown links with a title attribute (`[text](url "title")`); warning is now reserved for genuinely malformed absolute URLs.

### Changed

- Tray: `tray_progress_window`, `tray_sort_window`, and `tray_config_window` modules removed; window logic consolidated into `tray.rs`.

## [0.3.0] - 2026-04-16

### Added

- `sort-apply` command: interactive review and apply of sort reports; moves emails to trash, `to-summarize/`, or keeps them in place with optional type-based organization.
- Sort scoring: toggle-based scoring with folder, recurring, and per-account overrides.
- Email filenames now include a 12-character CamelCase subject extract.
- `organize_by_type` in `sort-apply`: moves "keep" emails into type subfolders (`direct/`, `newsletter/`, `mailing_list/`, `unknown/`) within the base export directory. Enabled by default; overridable per account in `settings.yaml`.
- `fix --html-bodies`: retroactively converts raw HTML body content in existing `.md` files to Markdown. Supports `--account` for automatic directory resolution and `--dry-run` preview.

### Fixed

- Export: HTML-only emails (no `text/plain` part) are now converted to Markdown at extract time instead of writing raw HTML into `.md` files (`htmd` crate).
- Export: Gmail permanent deletion now uses `[Gmail]/All Mail` EXPUNGE. Previously EXPUNGE on INBOX only removed the label (archived the message) instead of deleting it permanently.
- `sort-apply`: missing files are silently skipped instead of aborting the apply run.
- Thunderbird import: `organize_by_type` field is now included in generated account config.

### Changed

- `organize_by_type` defaults to `true`.

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
