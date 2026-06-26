# Email to Markdown Exporter

Outil Rust pour exporter vos emails IMAP vers des fichiers Markdown avec métadonnées YAML, puis les ranger automatiquement dans votre arborescence « second cerveau ».

## Installation

```bash
# Installer Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Compiler en mode release (optimisé)
cargo build --release

# Compiler avec l'icône dans la barre système (optionnel)
cargo build --release --features tray
```

**Linux** — dépendances système requises avant de compiler :
```bash
sudo apt-get install build-essential pkg-config libssl-dev
```

## Configuration rapide

```bash
# 1. Importer automatiquement les comptes depuis Thunderbird (recommandé)
./target/release/email-to-markdown import --extract-passwords

# 2. Choisir le répertoire d'export et le répertoire de notes dans settings.yaml
# Voir section Configuration ci-dessous

# 3. Définir votre arborescence de rangement dans destinations.yaml
#    (à la main, ou via `dest add` / `dest suggest` — voir section Destinations)
```

> Les fichiers de configuration sont stockés dans le répertoire système :
> - **Windows** : `%APPDATA%\email-to-markdown\`
> - **macOS** : `~/Library/Application Support/email-to-markdown/`
> - **Linux** : `~/.config/email-to-markdown/`

---

## Référence des commandes

### `import` — Importer depuis Thunderbird

Génère `accounts.yaml` (connexion uniquement) dans le répertoire de config système.

```
email-to-markdown import [OPTIONS]
```

| Option | Description |
|--------|-------------|
| `--list-profiles` | Liste les profils Thunderbird disponibles et quitte |
| `--profile <CHEMIN>` | Utilise un profil spécifique (chemin absolu) ; sinon détection automatique |
| `--output <CHEMIN>` | Fichier de sortie (défaut : répertoire de config système) |
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

### `export` — Exporter et ranger les emails

Connecte les comptes IMAP configurés, exporte les emails en fichiers Markdown, puis les déplace automatiquement vers le bon chemin dans votre second cerveau selon `destinations.yaml`.

**Flux complet :**

1. Connexion IMAP → téléchargement des emails
2. Conversion en Markdown avec frontmatter YAML (répertoire d'export comme zone tampon)
3. Calcul du chemin de destination selon les règles de `destinations.yaml`
4. Déplacement automatique vers `notes_dir/<chemin>/<Année>/<Mois>`

Si aucune règle ne correspond, l'email atterrit dans le dossier fourre-tout : `notes_dir/Perso/Messy/Emails/<Année>/<Mois>`.

Si `destinations.yaml` est absent ou non configuré, un avertissement est affiché et tous les emails tombent dans le fourre-tout — l'export continue sans erreur fatale.

```
email-to-markdown export [OPTIONS]
```

| Option | Description |
|--------|-------------|
| `--list-accounts` | Liste les comptes configurés dans `accounts.yaml` et quitte |
| `--account <NOM>` | Exporte uniquement le(s) compte(s) indiqué(s) (séparés par virgule) |
| `--config <CHEMIN>` | Fichier de configuration (défaut : répertoire de config système) |
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

### `tray` — Interface dans la barre système *(optionnel)*

Lance l'application en tant qu'icône enveloppe dans la barre système (Windows/macOS/Linux).

> Nécessite la compilation avec la feature `tray` :
> ```bash
> cargo build --release --features tray
> ```

```bash
email-to-markdown tray
```

Au premier lancement sans comptes configurés, le sous-menu **Export** est désactivé. Le menu contextuel propose :

| Entrée | Action |
|--------|--------|
| Export compte › *Nom* | Exporte les emails du compte via IMAP, puis ouvre la fenêtre de revue du routage |
| Import Thunderbird | Importe comptes + mots de passe (dialog Oui/Non/Annuler) |
| Choisir répertoire d'export… | Sélecteur de dossier → met à jour `settings.yaml` |
| Paramètres… | Ouvre la fenêtre de configuration |
| Mise à jour… | Vérifie et applique une mise à jour du binaire |
| Quitter | Ferme l'application |

**Fenêtre de revue du routage** — après chaque export, une fenêtre affiche les emails avec le chemin proposé. Vous pouvez :

- Conserver la proposition (chemin calculé depuis `destinations.yaml`)
- Réaffecter vers un chemin connu via autocomplétion
- Saisir librement un chemin nouveau (créé physiquement à l'apply, jamais réécrit dans `destinations.yaml`)
- Cliquer **Appliquer** pour déplacer tous les fichiers vers leur destination finale

Aucun fichier n'est déplacé avant que vous cliquiez Appliquer.

---

## Configuration

La configuration est répartie en plusieurs fichiers dans le répertoire système
(`%APPDATA%\email-to-markdown\` sur Windows, `~/.config/email-to-markdown/` sur Linux/macOS) :

### `accounts.yaml` — Connexion IMAP

Généré automatiquement par `import`. Contient uniquement les infos de connexion.

```yaml
accounts:
  - name: "Gmail"
    server: "imap.gmail.com"
    port: 993
    username: "votre.email@gmail.com"
    ignored_folders:
      - "[Gmail]/Spam"
      - "[Gmail]/Trash"
      - "[Gmail]/All Mail"
      - "[Gmail]/Drafts"

  - name: "Outlook"
    server: "outlook.office365.com"
    port: 993
    username: "votre.email@outlook.com"
    ignored_folders:
      - Junk
      - "Deleted Items"
```

### `settings.yaml` — Comportement de l'application

Éditable via **Paramètres…** dans le tray ou directement.

```yaml
# Répertoire d'export (zone tampon) — chaque compte crée un sous-dossier automatiquement
export_base_dir: C:/Users/VotreNom/Documents/Emails

# Racine du second cerveau — les chemins de destinations.yaml sont joints ici
notes_dir: C:/Users/VotreNom/Documents/Notes

# Chemin vers destinations.yaml (défaut : <config_dir>/destinations.yaml)
# destinations_file: C:/Users/VotreNom/.config/email-to-markdown/destinations.yaml

# Routage par IA — désactivé par défaut
# ai_routing_enabled: false
# ai_confidence_threshold: 0.7

# Comportement par défaut pour tous les comptes
defaults:
  quote_depth: 1            # Profondeur max des citations à conserver
  skip_existing: true       # Ne pas ré-exporter les emails déjà présents
  collect_contacts: false   # Générer un CSV des contacts
  skip_signature_images: true  # Ignorer les images de signature/logo
  delete_after_export: false   # Supprimer du serveur après export

# Surcharges par compte (optionnel)
# accounts:
#   Gmail:
#     folder_name: gmail      # Nom du sous-dossier (défaut : nom du compte)
#     delete_after_export: false
#   Outlook:
#     collect_contacts: true
```

### `destinations.yaml` — Arborescence de rangement

Fichier YAML décrivant les chemins valides de votre second cerveau et les règles de correspondance. Édité à la main ou via la sous-commande `dest`.

```yaml
destinations:
  # Chemin seul — simple option de classement (apparaît dans l'autocomplétion de la revue)
  - path: Perso/Famille

  # Chemin avec règles — correspondance déterministe (sans IA)
  - path: Pro/Clients/Acme
    note: client principal
    rules:
      - domain: acme.com
      - domain: acme.fr
  - path: Perso/Banque
    rules:
      - from: contact@mabanque.fr
      - subject: relevé

  # Fourre-tout explicite (au plus une entrée default)
  - path: Perso/Messy/Emails
    default: true
```

**Types de règles :**

| Règle | Description |
|----------|-------------|
| `domain: <d>` | L'expéditeur vient du domaine `d` (insensible à la casse, sous-domaines inclus) |
| `from: <adresse>` | Adresse exacte de l'expéditeur (insensible à la casse) |
| `subject: <mot>` | Le sujet contient `mot` (insensible à la casse) |
| `account: <nom>` | Email reçu sur le compte `nom` |
| `default: true` | Chemin fourre-tout si aucune autre règle ne correspond |

La première règle qui correspond l'emporte. Le premier segment du chemin (`Perso` ou `Pro`) détermine la polarité : `Perso` par défaut si aucune règle ne force `Pro`.

> **Migration** : un `destinations.txt` hérité de l'ancien format est converti automatiquement en `destinations.yaml` au premier lancement (le `.txt` est conservé tel quel).

#### Gérer les destinations en ligne de commande — `dest`

```bash
# Lister les destinations et repérer les anomalies
email-to-markdown dest list

# Ajouter une destination (chemin nu = option de classement)
email-to-markdown dest add "Perso/Famille"

# Ajouter avec une ou plusieurs règles + une note
email-to-markdown dest add "Perso/Banque" --domain mabanque.fr --subject relevé --note "factures"

# Proposer des règles à partir des emails déjà tombés dans le fourre-tout
email-to-markdown dest suggest
```

`dest suggest` parcourt le dossier par défaut sous `notes_dir`, regroupe les emails par domaine d'expéditeur et propose interactivement une destination pour chacun (les dossiers commençant par `.` ou `_` et les liens symboliques sont ignorés). Aucun email déjà exporté n'est déplacé — les règles ne s'appliquent qu'aux exports suivants.

### `.env` — Mots de passe

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

Le répertoire d'export (`export_base_dir`) sert de zone tampon. Après l'export, les fichiers Markdown sont déplacés vers le second cerveau (`notes_dir`) selon les règles de `destinations.yaml`.

```
export_base_dir/            ← zone tampon (peut être vidée après apply)
├── Gmail/
│   ├── INBOX/
│   │   └── email_2024-01-15_AB_to_CD.md
│   └── Sent/
│       └── email_2024-01-15_AB_to_CD.md

notes_dir/                  ← second cerveau, destination finale
├── Pro/
│   └── Clients/
│       └── Acme/
│           └── 2024/
│               └── 01/
│                   └── email_2024-01-15_AB_to_CD.md
└── Perso/
    └── Messy/
        └── Emails/
            └── 2024/
                └── 01/
                    └── email_2024-01-15_AB_to_CD.md
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

**Emails dans le fourre-tout** : Vérifiez que `destinations.yaml` est correctement configuré (`destinations_file` dans `settings.yaml`) et que les règles correspondent bien à vos expéditeurs (`dest list` aide à les inspecter).

---

## Note sur l'outillage Python

Les scripts Python d'analyse (`tools/`) qui existaient dans les versions précédentes ont été archivés dans un dépôt séparé et ne font plus partie du périmètre de cette application. Le routage est désormais entièrement déterministe via `destinations.yaml`, avec une option IA désactivée par défaut.

---

## Contribuer

Les contributions sont les bienvenues ! Consultez [CONTRIBUTING.md](CONTRIBUTING.md) pour les directives.

## Licence

MIT
