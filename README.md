# Email to Markdown Exporter

Outil Rust pour exporter vos emails IMAP vers des fichiers Markdown avec métadonnées YAML.

## Installation

```bash
# Installer Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Compiler le projet
cargo build --release
```

## Configuration

1. Copier le fichier d'exemple :

```bash
cp config/accounts.yaml.example config/accounts.yaml
```

2. Configurer les comptes dans `config/accounts.yaml`

3. Créer le fichier `.env` avec les mots de passe :
```bash
cp .env.example .env
```

## Utilisation

```bash
# Exporter tous les emails
./target/release/email-to-markdown export

# Exporter un compte spécifique
./target/release/email-to-markdown export --account Gmail

# Importer la configuration depuis Thunderbird
./target/release/email-to-markdown import

# Corriger les fichiers YAML
./target/release/email-to-markdown fix ./exports/gmail --apply
```

## Fonctionnalités

- Export IMAP vers Markdown avec frontmatter YAML
- Gestion des pièces jointes et structure de dossiers
- Import automatique depuis Thunderbird
- Correction des fichiers YAML malformés
- Tri et catégorisation des emails

## Prérequis

- Rust 1.60+
- Comptes email avec accès IMAP activé
- Pour Gmail : mot de passe spécifique à l'application (si 2FA activé)

## Configuration

### Options principales

- `delete_after_export`: Supprime les emails après export (désactivé par défaut)
- `ignored_folders`: Liste des dossiers à ignorer
- `quote_depth`: Profondeur des citations à conserver (par défaut: 1)
- `skip_existing`: Ignore les emails déjà exportés (activé par défaut)
- `collect_contacts`: Génère un fichier CSV des contacts
- `skip_signature_images`: Ignore les images de signature

### Exemple de configuration

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
    quote_depth: 1
    skip_existing: true
    collect_contacts: true
    skip_signature_images: true
```

## Dépannage

### Erreurs courantes

**Mot de passe Gmail requis** : Créez un [mot de passe spécifique](https://support.google.com/accounts/answer/185833) pour les comptes avec 2FA.

**Problèmes de connexion** : Vérifiez `.env` et l'accès IMAP sur le serveur.

**Dossiers invalides** : Le script ignore automatiquement les dossiers problématiques.

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
