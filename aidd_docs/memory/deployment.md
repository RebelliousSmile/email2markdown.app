# Deployment

Application locale CLI/WebView — aucun serveur de production. Une matrice CI
compile et teste Windows, macOS et Ubuntu ; Dovecot est utilisé uniquement comme
fixture jetable dans les tests destructifs.

## Build

```bash
# Standard release binary
cargo build --release

# With system tray support
cargo build --release --features tray
```

La feature `tray` est obligatoire pour `contextual` et `shell install`. Le même
binaire ouvre une fenêtre autonome depuis Explorer, Finder, Nautilus ou Dolphin ;
il n’a pas besoin de résider dans la barre système.

Release profile: `opt-level = 3`, `lto = true`, `codegen-units = 1`, `strip = true`

Output: `target/release/email-to-markdown` (`.exe` on Windows)

## Intégration gestionnaire de fichiers

```bash
email-to-markdown shell install
email-to-markdown shell status
email-to-markdown shell uninstall
```

L’installation est strictement utilisateur : HKCU sous Windows,
`~/Library/Services` sous macOS et `~/.local/share` sous Linux. Les artefacts
incorporent le chemin absolu du binaire ; `status` détecte un chemin périmé et
`install` le répare. L’auto-updater répare un artefact déjà présent après
remplacement du binaire.

Les dépendances GUI sont WebView2 sur Windows, WebKit système sur macOS et
WebKitGTK 4.1/GTK 3/AppIndicator sur Linux. La recette de livraison et les preuves
manuelles attendues sont dans `docs/manual-qa-contextual-export.md`.

## Environment Variables

### Environment Files

`.env` file in platform config directory (loaded by `dotenv` at startup):

| Platform | Path |
|----------|------|
| Windows | `%APPDATA%\email-to-markdown\.env` |
| macOS | `~/Library/Application Support/email-to-markdown/.env` |
| Linux | `~/.config/email-to-markdown/.env` |

### Required Variables

| Variable | Description |
|----------|-------------|
| `{ACCOUNT}_PASSWORD` | IMAP password for account |
| `{ACCOUNT}_APPLICATION_PASSWORD` | App-specific password (takes priority over `_PASSWORD`) |

`{ACCOUNT}` = account name uppercased, `@` `.` `-` replaced by `_`

## Project Structure

```plaintext
email-to-markdown/
├── src/                  # Rust source
├── tests/                # Integration tests
├── config/               # Example config files (.example)
├── archive/              # Legacy Python code (reference only)
├── Cargo.toml
└── target/release/
    └── email-to-markdown(.exe)   # Distributed binary
```
