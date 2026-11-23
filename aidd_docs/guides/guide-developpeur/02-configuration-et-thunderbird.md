# Configuration et import *Thunderbird*

Ce chapitre décrit où vivent les fichiers de configuration, comment `config.rs` fusionne les réglages, et comment le module `thunderbird` alimente `accounts.yaml` et `.env` à partir d'un profil existant.

## Le répertoire de configuration

`app_config_dir()` renvoie `dirs::config_dir()` (ou `.` à défaut) suivi de `email-to-markdown`. Tous les fichiers d'état de l'application y sont placés.

| Fichier | Contenu |
| --- | --- |
| `accounts.yaml` | liste des comptes IMAP (sans mot de passe) |
| `.env` | mots de passe, une variable par compte |
| `settings.yaml` | réglages globaux et par compte |
| `destinations.yaml` | règles de tri (voir le chapitre 5) |
| `handled_messages.txt` | registre des messages traités (chapitre 6) |
| `deletions.log` | journal des suppressions côté serveur (chapitre 6) |
| `tray.lock` | verrou d'instance unique de l'icône (chapitre 7) |

Les modèles `config/accounts.yaml.example` et `config/settings.yaml.example` documentent les clés disponibles.

### Les chemins injectés

Les fonctions qui touchent à ces fichiers ne lisent pas directement le répertoire de l'utilisateur : elles reçoivent un `AppPaths`.

`AppPaths { accounts, env, settings, ledger, deletion_log }`

`AppPaths::from_app_config()` construit la valeur de production. Les tests la construisent à partir d'un `TempDir`, si bien qu'aucun test ne touche à la vraie configuration. Les fonctions publiques d'usage courant existent donc par paires : `export_account` délègue à `export_account_in(paths, …)`, `clean_server` à `clean_server_in(paths, …)`.

## Les comptes et les mots de passe

`accounts.yaml` contient une liste `accounts:` de `RawAccount` : `name`, `server`, `port`, `username`, `included_folders` (optionnel) et `ignored_folders`. Le mot de passe n'y figure jamais ; le champ `Account.password` est `#[serde(skip)]`.

Le mot de passe se lit dans l'environnement, après chargement de `.env`. Le nom de variable dérive du nom du compte par `env_var_name` : majuscules, avec ` `, `@`, `.` et `-` remplacés par `_`.

| Variable testée | Usage |
| --- | --- |
| `{NOM}_APPLICATION_PASSWORD` | testée en premier |
| `{NOM}_PASSWORD` | repli |

Si aucune n'existe, la lecture échoue avec `ConfigError::NoPassword`.

`Account::exports_folder` décide si un dossier IMAP est exporté. Une liste `included_folders` non vide l'emporte sur `ignored_folders` ; le dossier `INBOX` se compare sans tenir compte de la casse.

## Les réglages et leur fusion

`settings.yaml` se désérialise en `Settings`.

| Champ | Rôle |
| --- | --- |
| `export_base_dir` | racine de l'export (zone d'attente) |
| `notes_dir` | racine des notes triées ; si elle est définie, `export` trie automatiquement |
| `destinations_file` | chemin du fichier de destinations |
| `ai_routing_enabled`, `ai_confidence_threshold` | routage assisté, non branché (0.7 par défaut) |
| `organize_by_date`, `date_layout` | arborescence de dates |
| `destination_max_depth` | profondeur maximale d'une destination (2 par défaut, 0 refusé) |
| `defaults` | `AccountBehavior` commun à tous les comptes |
| `accounts` | `AccountBehavior` par nom de compte |

`AccountBehavior` regroupe huit champs tous optionnels : `folder_name`, `quote_depth`, `skip_existing`, `collect_contacts`, `skip_signature_images`, `delete_after_export`, `cleanup_empty_dirs` et `batch_size`.

`merge_account` résout ces options en valeurs concrètes. La priorité est : réglage du compte, puis `defaults`, puis valeur codée en dur.

`quote_depth: per`\
`    .and_then(|a| a.quote_depth)`\
`    .or(def.quote_depth)`\
`    .unwrap_or(1),`

| Réglage | Valeur codée en dur |
| --- | --- |
| `quote_depth` | 1 |
| `skip_existing` | `true` |
| `collect_contacts` | `false` |
| `skip_signature_images` | `false` |
| `delete_after_export` | `false` |
| `cleanup_empty_dirs` | `true` |
| `batch_size` | 0 (pas de limite) |

Le répertoire d'export d'un compte est `export_base_dir/<folder_name ou name>`, avec des `/` comme séparateurs.

> **À retenir** : le runtime ne manipule jamais d'`Option` pour ces comportements. Ajouter un réglage suppose de le déclarer dans `AccountBehavior`, dans `Account`, puis dans `merge_account` (voir le chapitre 9).

### Chargement et validation

- `Config::load` et `Config::load_with_settings` lisent les deux fichiers ; un `accounts.yaml` absent donne une configuration vide.
- `validate()` vérifie le nom, le serveur, l'identifiant, le répertoire d'export et un port non nul.
- `get_account` recherche sans tenir compte de la casse.
- `Settings::load` renvoie les valeurs par défaut si le fichier manque, mais une erreur `ConfigError::SettingsFile` s'il est invalide : un fichier abîmé n'est jamais remplacé en silence.
- `Settings::save` valide avant d'écrire.

`write_env_file` refuse les retours à la ligne et le caractère NUL dans une valeur, et crée le fichier en mode 0600 sous Unix.

## L'arborescence de dates

`DateLayout` (`date_layout.rs`) vaut `None`, `Year`, `YearMonth` ou `YearMonthDay` (sérialisé en `snake_case`). Ses méthodes principales :

| Méthode | Effet |
| --- | --- |
| `segments(date)` | segments de dossier pour une date |
| `apply(base, date)` | ajoute les segments à un chemin |
| `ensure(rel, date)` | n'ajoute rien si le suffixe existe déjà |
| `lookup_dirs(base, date)` | tous les dossiers où une note de cette date peut se trouver, quel que soit le layout qui l'a classée |

`Settings::date_layout()` retombe sur l'ancien booléen `organize_by_date` quand `date_layout` est absent, et `set_date_layout` garde les deux cohérents. La reconnaissance d'un segment de date (années 19xx et 20xx, mois 01 à 12, jours 01 à 31) n'utilise pas d'expression régulière.

## L'import depuis Thunderbird

La sous-commande `import` et la fenêtre de paramètres partagent le module `thunderbird`.

### Profils et comptes

- `get_thunderbird_profiles_dir` renvoie `%APPDATA%\Thunderbird\Profiles` sous Windows, `~/Library/Thunderbird/Profiles` sous macOS et `~/.thunderbird` sous Linux.
- `default_profile` et `list_profiles` choisissent le profil.
- `extract_accounts` lit les préférences (`parse_prefs_servers`) et produit des `Account` avec un type d'authentification `AuthKind` : `Password`, `OAuth2` (code 10) ou `Other(n)`.
- `write_accounts_merged` ajoute les nouveaux comptes sans jamais écraser l'existant et crée une copie `accounts.yaml.bak` ; il renvoie un `ImportSummary`.

### L'extraction des mots de passe

Thunderbird chiffre ses mots de passe avec NSS. `extract_passwords(profile, primary_password)` charge dynamiquement la bibliothèque NSS installée avec Thunderbird.

| Plateforme | Bibliothèque recherchée |
| --- | --- |
| Windows | `nss3.dll` dans `C:\Program Files[ (x86)]\Mozilla Thunderbird\` |
| macOS | `/Applications/Thunderbird.app/Contents/MacOS/libnss3.dylib` |
| Linux | `libnss3.so` |

Le déroulement est le suivant :

1. lecture de `logins.json` (entrées `imap://hôte`) ;
2. sous Windows, ajout du dossier de Thunderbird au `PATH`, une seule fois ;
3. prise du verrou `NSS_LOCK`, qui sérialise les accès à NSS au sein du processus ;
4. `Library::new` (bloc `unsafe`) puis `NSS_Init` sur le profil ; un statut non nul signale généralement que Thunderbird est encore ouvert ;
5. `NssSession` garantit `NSS_Shutdown` à sa destruction ;
6. `unlock_key_slot` consulte `PK11_NeedLogin` et `PK11_CheckUserPassword` ; sans mot de passe principal fourni, l'erreur typée `PrimaryPasswordRequired` remonte ;
7. `decrypt_nss_string` appelle `PK11SDR_Decrypt`.

Une entrée qui ne se déchiffre pas est ignorée avec un avertissement, sans interrompre les autres.

> **Attention** : ce code n'a aucun test automatisé. NSS et Thunderbird relèvent des essais manuels ; la règle de tests du projet interdit de les simuler.

### Rapprochement et écriture dans `.env`

`find_password` exige que le serveur et l'identifiant correspondent. Le serveur seul suffit uniquement s'il n'existe qu'un seul mot de passe enregistré pour lui. `write_passwords_to_env` écrit `{ENV}_PASSWORD='…'` en échappant `'` par `'\''`, remplace une ligne existante, ignore un mot de passe contenant un saut de ligne et passe par `config::write_env_file`.

### L'import de la fenêtre de paramètres

`scan_accounts(profile, accounts_path)` liste les comptes à proposer sans charger NSS. Elle renvoie des `ThunderbirdAccount` indiquant le type d'authentification, la présence d'un mot de passe enregistré et si le compte est déjà configuré.

`import_selected` reçoit les identifiants choisis et délègue à `import_selected_with`, qui prend en paramètre la source des mots de passe et la sonde de connexion pour rester testable. Elle :

- ignore les comptes déjà configurés ;
- n'appelle `extract_passwords` que si un candidat a un mot de passe enregistré ;
- n'accepte pour un compte OAuth2 qu'une correspondance exacte ;
- lance une sonde `probe_account` par compte, chacune dans son fil (`thread::scope`) ;
- choisit les dossiers inclus par `default_included_folders` (boîte de réception et envoyés, sinon `INBOX`) ;
- écrit les comptes, puis les mots de passe des seuls comptes réellement ajoutés ;
- renvoie un `ImportReport` par compte (`name`, `ok`, `message`).

> **Renvoi** : la sonde de connexion `probe_account` est décrite au chapitre 3.
