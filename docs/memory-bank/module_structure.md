# Module Structure - Memory Bank

## Overview

Structure des modules Rust dans `src/`.

---

## Modules

### `main.rs`
Point d'entrée CLI (clap). Dispatche vers les sous-commandes : `import`, `export`, `fix`, `sort`, `tray`.

### `lib.rs`
Exporte les modules publics du crate.

### `config.rs`
Gestion de la configuration. Structures clés :
- `RawAccount` : données de connexion lues depuis `accounts.yaml`
- `AccountBehavior` : surcharges de comportement (tous champs `Option<T>`)
- `Settings` : `settings.yaml` — `export_base_dir` + defaults + overrides par compte
- `Account` : struct résolu après fusion `RawAccount` + `Settings`
- `Config` : conteneur de `Vec<Account>`, chargé via `load()` / `load_with_settings()`
- `SortConfig` : règles de tri lues depuis `sort_config.json`
- `ConfigError` : erreurs de configuration

Helpers de chemins : `app_config_dir()`, `accounts_yaml_path()`, `env_file_path()`, `settings_path()`, `sort_config_path()`

### `email_export.rs`
Client IMAP et export vers Markdown. Structures clés :
- `ImapExporter` : connexion IMAP et itération des dossiers
- `EmailFrontmatter` : métadonnées YAML de l'email
- `EmailAnalysis` / `EmailType` : classification (Direct, Group, Newsletter, MailingList)
- `ContactsCollector` : collecte et export CSV des contacts
- `ExportStats` : compteurs exported/skipped/errors
- `FolderName` : paire `{ raw, display }` — `raw` (modified UTF-7 du serveur) pour les appels IMAP `SELECT`, `display` (UTF-8 décodé) pour les chemins locaux et le matching `ignored_folders`

Fonctions publiques : `export_to_markdown()`, `analyze_email_type()`

**Gotchas load-bearing pour qui modifie cette zone :**

- `skip_existing` fait un **early-return à `Ok(None)`** dans `export_to_markdown()` **avant** tout side-effect — avant la création eager de `export_directory` et `attachments/<folder>/`, avant `extract_attachments`, avant `analyze_email_type`. Tout effet de bord per-message (création de dossier, collecte de contacts, écriture sur disque) ne se produit **que si le message est réellement processé**. Un re-run sur un compte 100% à jour n'écrit rien et ne crée aucun dossier. Conséquence pour les tests end-to-end : il faut injecter des données observables avant le run, sinon la feature teste un no-op.
- `export_account()` wrappe son corps dans une **IIFE** pour garantir que le cleanup final (`cleanup_empty_dirs`) tourne sur **tous les chemins de sortie**, y compris quand un `?` propage une erreur depuis l'intérieur du loop. Ne pas "simplifier" ce wrap en le retirant — la contrainte est "runs even on partial failure".
- Les erreurs classifiées comme **malformed emails** (détection via `e.downcast_ref::<mailparse::MailParseError>()`) sont comptées dans `stats.skipped`, pas `stats.errors`. Les vraies erreurs réseau/fetch restent dans `stats.errors`. Ne jamais mélanger les deux compteurs.

### `thunderbird.rs`
Import depuis Thunderbird (profils, comptes, mots de passe). Fonctions clés :
- `list_profiles()` : liste les profils Thunderbird
- `extract_accounts()` : extrait les comptes IMAP depuis `prefs.js`
- `extract_passwords()` : déchiffre les mots de passe depuis le NSS key store
- `generate_accounts_yaml()` : génère le contenu `accounts.yaml` (connexion uniquement)
- `write_passwords_to_env()` : écrit le fichier `.env`

### `fix_yaml.rs`
Correction du frontmatter YAML Python-spécifique (hérité de l'ancienne version Python) :
- `fix_complex_yaml_tags()` : supprime les tags `!!python/object:` etc.
- `extract_frontmatter()` : sépare frontmatter et corps
- `scan_and_fix_directory()` : correction batch

### `sort_emails.rs`
Catégorisation des emails exportés :
- `EmailSorter` : analyse et classe les emails
- `Category` : Delete / Summarize / Keep
- `EmailSortType` : Direct / Newsletter / Group / Mailing
- `generate_report()`, `save_report()`

### `tray.rs`
Interface icône dans la barre système (feature `tray`). Utilise `tao` + `tray-icon`.
- Crée et gère l'event loop `tao`
- Construit le menu contextuel dynamiquement (sous-menus Export/Sort peuplés depuis `accounts.yaml`)
- Reconstruit le menu après un import (`ActionResult::Imported`)
- Affiche les notifications via `rfd::MessageDialog` (thread séparé)

### `tray_actions.rs`
Actions déclenchées par le menu tray (feature `tray`). Chaque action s'exécute dans un thread séparé :
- `action_export()` → `run_export()` : export IMAP pour un compte
- `action_sort()` → `run_sort()` : tri des emails d'un compte
- `action_import_thunderbird()` → `run_import_thunderbird()` : import depuis Thunderbird avec dialog Yes/No/Cancel
- `action_choose_export_dir()` → `set_export_dir()` : browser de dossier + mise à jour `settings.yaml`
- `action_open_config()` : ouvre `settings.yaml` dans l'éditeur par défaut
- `action_open_documentation()` : ouvre `README.md`
- `get_account_names()` : liste les comptes pour peupler le menu

`ActionResult` : `Success(title, message)` | `Imported(message)` | `Error(message)`

### `network.rs`
Utilitaires réseau : indicateur de progression, logique de retry.

### `utils.rs`
Fonctions utilitaires partagées :
- `limit_quote_depth()` : réduit la profondeur des citations
- `get_short_name()` : extrait les initiales d'une adresse email
- `is_signature_image()` : détecte les images de signature
- `decode_imap_utf7()` : décode les noms de dossiers IMAP (modified UTF-7)
- `sanitize_filename()` : génère un nom de fichier sûr
- `extract_emails()` : extrait les adresses email d'une chaîne
- `normalize_line_breaks()`, `hash_md5_prefix()`

### `cleaner.rs`
Pipeline de post-traitement du corps d'email, appelé après `normalize_line_breaks` dans `export_to_markdown`. Fonctions pures `&str → String` enchaînées par `clean()`, qui retourne un `CleanResult { body, social_links }` pour rester découplé de `EmailFrontmatter`.

Étapes du pipeline (ordre strict) :
1. `decode_residual_qp()` — fallback défensif sur les séquences `=XX` résiduelles (mailparse ne décode pas toujours)
2. `decode_html_entities()` — via crate `html-escape`
3. `strip_invisible_chars()` — retire U+200B, U+200C, U+200D, U+FEFF, U+00AD
4. `detect_mojibake()` — warn seulement (pas de réparation)
5. `extract_social_footer()` — doit tourner AVANT reattach/unwrap (la disposition verticale est le signal)
6. `reattach_urls()` — recolle les URL wrappées, exige un marqueur `https?://` ou `www.` en queue pour ne pas corrompre la prose
7. `unwrap_lines()` — déplie le wrap 80-char, préserve listes/code/citations/tableaux/signatures/lignes-liens isolées
8. `extract_links()` — réécrit `[text](url)` en `[text][N]` + références numérotées en bas
9. `decontaminate_ref_urls()` → `decontaminate_trackers()` — strip `utm_*`, unwrap mailchimp/sendgrid/click wrappers
10. `collapse_whitespace()` + `trim_trailing()` — hygiène finale

Nouvelles dépendances : `html-escape`, `url`.

---

## Dépendances entre modules

```
main.rs
  ├── config.rs
  ├── email_export.rs  ──► config.rs, utils.rs, network.rs, cleaner.rs
  ├── cleaner.rs       (pur, pas de dépendance interne)
  ├── thunderbird.rs   ──► utils.rs
  ├── fix_yaml.rs
  ├── sort_emails.rs   ──► config.rs
  ├── tray.rs          ──► tray_actions.rs          [feature: tray]
  └── tray_actions.rs  ──► config.rs, email_export.rs, sort_emails.rs, thunderbird.rs
```

---

## Feature flags

- `tray` : compile `tray.rs` et `tray_actions.rs`, ajoute les dépendances `tray-icon`, `tao`, `rfd`, `image`

```bash
cargo build --release --features tray
cargo run --features tray -- tray
```
