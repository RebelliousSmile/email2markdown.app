# Email to Markdown Exporter

Outil Rust pour exporter vos emails IMAP vers des fichiers Markdown avec métadonnées YAML.

## Installation

```bash
# Installer Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Compiler le projet (debug)
cargo build

# Compiler en mode release (optimisé)
cargo build --release
```

## Configuration rapide

```bash
# 1. Importer automatiquement la config depuis Thunderbird (recommandé)
./target/release/email-to-markdown import --extract-passwords

# 2. Ou configurer manuellement
cp config/accounts.yaml.example config/accounts.yaml
# Éditer config/accounts.yaml, puis créer .env avec les mots de passe
```

---

## Référence des commandes

### `import` — Importer depuis Thunderbird

Génère `config/accounts.yaml` à partir des comptes IMAP configurés dans Thunderbird.

```
email-to-markdown import [OPTIONS]
```

| Option | Description |
|--------|-------------|
| `--list-profiles` | Liste les profils Thunderbird disponibles et quitte |
| `--profile <CHEMIN>` | Utilise un profil spécifique (chemin absolu) ; sinon détection automatique |
| `--output <CHEMIN>` | Fichier de sortie (défaut : `config/accounts.yaml`) |
| `--generate-env` | Génère aussi un fichier `.env.template` avec les variables à remplir |
| `--extract-passwords` | Déchiffre les mots de passe depuis Thunderbird et les écrit dans `.env` (Thunderbird doit être fermé) |
| `--master-password <MDP>` | Master Password Thunderbird, si vous en avez configuré un |

**Exemples :**

```bash
# Lister les profils disponibles
email-to-markdown import --list-profiles

# Import automatique (profil par défaut)
email-to-markdown import

# Import + extraction automatique des mots de passe
email-to-markdown import --extract-passwords

# Avec Master Password Thunderbird
email-to-markdown import --extract-passwords --master-password "secret"

# Profil spécifique + template .env
email-to-markdown import --profile ~/.thunderbird/abc123.default --generate-env

# Fichier de sortie personnalisé
email-to-markdown import --output /chemin/vers/accounts.yaml
```

---

### `export` — Exporter les emails

Connecte les comptes IMAP configurés et exporte les emails en fichiers Markdown.

```
email-to-markdown export [OPTIONS]
```

| Option | Description |
|--------|-------------|
| `--list-accounts` | Liste les comptes configurés dans `accounts.yaml` et quitte |
| `--account <NOM>` | Exporte uniquement le(s) compte(s) indiqué(s) (séparés par virgule) |
| `--config <CHEMIN>` | Fichier de configuration (défaut : `config/accounts.yaml`) |
| `--debug` | Active le mode verbeux (sortie IMAP brute) |
| `--delete-after-export` | Supprime les emails du serveur après export (dangereux !) |

**Exemples :**

```bash
# Exporter tous les comptes configurés
email-to-markdown export

# Lister les comptes disponibles
email-to-markdown export --list-accounts

# Exporter un seul compte
email-to-markdown export --account Gmail

# Exporter plusieurs comptes
email-to-markdown export --account Gmail,Outlook

# Mode debug (verbose IMAP)
email-to-markdown export --account Gmail --debug

# Config personnalisée
email-to-markdown export --config /chemin/vers/accounts.yaml

# Supprimer les emails après export
email-to-markdown export --account Gmail --delete-after-export
```

---

### `fix` — Corriger le YAML malformé

Corrige les fichiers Markdown générés avec des tags YAML Python-spécifiques (hérités de l'ancienne version Python).

```
email-to-markdown fix <DOSSIER> [OPTIONS]
```

| Argument / Option | Description |
|--------|-------------|
| `<DOSSIER>` | Dossier contenant les fichiers email à analyser (obligatoire) |
| `--dry-run` | Simule les corrections sans modifier les fichiers |
| `--apply` | Applique réellement les corrections (sans `--apply`, mode dry-run par défaut) |

**Exemples :**

```bash
# Analyser sans modifier (dry-run)
email-to-markdown fix ./exports/gmail

# Voir ce qui serait corrigé
email-to-markdown fix ./exports/gmail --dry-run

# Appliquer les corrections
email-to-markdown fix ./exports/gmail --apply
```

---

### `sort` — Trier et catégoriser les emails

Analyse les emails exportés et les classe en catégories : `delete`, `summarize`, `keep`.

```
email-to-markdown sort [DOSSIER] [OPTIONS]
```

| Argument / Option | Description |
|--------|-------------|
| `[DOSSIER]` | Dossier contenant les fichiers email Markdown |
| `--account <NOM>` | Trie les emails d'un compte (lit le dossier depuis `accounts.yaml`) |
| `--config <CHEMIN>` | Fichier de règles de tri (défaut : `config/sort_config.json`) |
| `--report <NOM>` | Nom du fichier rapport de sortie (défaut : `sort_report.json`) |
| `--verbose` | Affiche les détails des emails classés |
| `--dry-run` | Analyse sans créer de rapport |
| `--list-accounts` | Liste les comptes disponibles dans `accounts.yaml` |
| `--create-config` | Crée un fichier `sort_config.json` avec les valeurs par défaut |

**Exemples :**

```bash
# Trier les emails d'un dossier
email-to-markdown sort ./exports/gmail

# Trier par nom de compte (lit le dossier depuis accounts.yaml)
email-to-markdown sort --account Gmail

# Simulation sans créer de rapport
email-to-markdown sort --account Gmail --dry-run

# Avec sortie détaillée
email-to-markdown sort --account Gmail --verbose

# Rapport personnalisé
email-to-markdown sort ./exports/gmail --report mon_rapport.json

# Créer la config de tri par défaut
email-to-markdown sort --create-config

# Config de tri personnalisée
email-to-markdown sort --account Gmail --config config/mes_regles.json

# Lister les comptes disponibles
email-to-markdown sort --list-accounts
```

---

### `tray` — Interface dans la barre système *(optionnel)*

Lance l'application en tant qu'icône dans la barre système (Windows/macOS/Linux).

> Nécessite la compilation avec la feature `tray` :
> ```bash
> cargo build --release --features tray
> ```

```bash
email-to-markdown tray
```

---

## Configuration

### `config/accounts.yaml`

```yaml
accounts:
  - name: "Gmail"
    server: "imap.gmail.com"
    port: 993
    username: "votre.email@gmail.com"
    export_directory: "./exports/gmail"
    ignored_folders:
      - "[Gmail]/Spam"
      - "[Gmail]/Trash"
      - "[Gmail]/All Mail"
      - "[Gmail]/Drafts"
    quote_depth: 1          # Profondeur max des citations à conserver
    skip_existing: true     # Ne pas ré-exporter les emails déjà présents
    collect_contacts: false # Générer un CSV des contacts
    skip_signature_images: true  # Ignorer les images de signature/logo
    delete_after_export: false   # Supprimer du serveur après export
```

### `.env`

```bash
# Mot de passe standard
GMAIL_PASSWORD=votre_mot_de_passe

# Mot de passe applicatif Gmail (si 2FA activé)
GMAIL_APPLICATION_PASSWORD=xxxx-xxxx-xxxx-xxxx

OUTLOOK_PASSWORD=votre_mot_de_passe
```

Le nom de la variable est `{NOM_DU_COMPTE_EN_MAJUSCULES}_PASSWORD`.
Le suffixe `_APPLICATION_PASSWORD` est prioritaire sur `_PASSWORD`.

---

## Structure des exports

```
export_directory/
├── INBOX/
│   └── email_2024-01-15_AB_to_CD.md
├── Sent/
│   └── email_2024-01-15_AB_to_CD.md
└── attachments/
    └── INBOX/
        └── email_2024-01-15_AB_to_CD_a1b2c3_fichier.pdf
```

---

## Prérequis

- Rust 1.70+
- Accès IMAP activé sur le serveur email
- Pour Gmail avec 2FA : [mot de passe spécifique à l'application](https://support.google.com/accounts/answer/185833)
- Pour `--extract-passwords` : Thunderbird installé et **fermé**

---

## Dépannage

**Mot de passe non trouvé** : Vérifiez que la variable `NOM_PASSWORD` dans `.env` correspond exactement au `name` du compte dans `accounts.yaml`.

**Échec `--extract-passwords`** : Fermez Thunderbird avant de lancer la commande. Si vous avez un Master Password configuré, utilisez `--master-password`.

**Connexion IMAP refusée** : Vérifiez que l'accès IMAP est activé dans les paramètres du compte email.

**Dossiers manquants** : Ajustez `ignored_folders` dans `accounts.yaml` ; utilisez `--debug` pour voir les dossiers disponibles.

---

## Documentation

- [Terminologie Rust](docs/memory-bank/rust_terminology.md)
- [Structure des modules](docs/memory-bank/module_structure.md)
- [Gestion des erreurs](docs/memory-bank/error_handling.md)
- [Configuration](docs/memory-bank/configuration.md)
- [Stratégie de test](docs/memory-bank/testing_strategy.md)

## Contribuer

Les contributions sont les bienvenues ! Consultez [CONTRIBUTING.md](CONTRIBUTING.md) pour les directives.

## Licence

MIT
