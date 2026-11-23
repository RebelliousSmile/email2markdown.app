# Vue d'ensemble du *projet*

`email-to-markdown` est une application Rust qui exporte des comptes IMAP vers des notes Markdown, les trie dans une arborescence de destinations et peut supprimer du serveur les messages déjà traités. Ce chapitre situe le code : le crate, ses modules, ses points d'entrée et les conventions qui valent partout.

## Le crate et ses fonctionnalités

Le dépôt produit une bibliothèque (`src/lib.rs`) et un binaire du même nom (`src/main.rs`). Le binaire ne contient que l'analyse des arguments et l'aiguillage vers la bibliothèque ; toute la logique testable vit dans les modules de `lib.rs`.

| Élément | Valeur |
| --- | --- |
| Version | 0.29.0 |
| Édition | 2021 |
| Version minimale de Rust | 1.93.0 (`rust-version`) |
| Fonctionnalité par défaut | aucune (`default = []`) |
| Fonctionnalité optionnelle | `tray` |

La fonctionnalité `tray` regroupe tout le code d'interface graphique et de mise à jour. Sans elle, le binaire est une application en ligne de commande pure.

`tray = ["tray-icon", "tao", "wry", "rfd", "dep:ureq", "dep:self-replace", "dep:sha2", "dep:windows-sys"]`

> **À retenir** : le code graphique se trouve derrière `#[cfg(feature = "tray")]`. Le binaire en ligne de commande doit toujours compiler sans cette fonctionnalité ; ne faites donc jamais dépendre un module hors `tray` de `tao`, `wry`, `tray-icon` ou `rfd`.

Quelques dépendances méritent d'être connues avant de lire le code :

- `imap` est épinglé à `=3.0.0-alpha.15`, une préversion dont l'API peut changer à chaque alpha ;
- `mailparse` analyse les messages MIME, `htmd` convertit le HTML en Markdown ;
- `serde_yaml` lit et écrit les fichiers de configuration, `toml` lit le bloc `email` des notes ;
- `fs2` fournit les verrous de fichier, `walkdir` les parcours d'arborescence ;
- `libloading` charge la bibliothèque NSS de Thunderbird à l'exécution ;
- `semver` compare les versions lors de la mise à jour (la règle du projet interdit la comparaison de chaînes) ;
- `thiserror` et `anyhow` structurent la gestion des erreurs.

Le tableau `[dev-dependencies]` est vide : les tests s'appuient sur `tempfile`, déjà présent dans les dépendances normales.

## Le graphe des modules

`lib.rs` déclare vingt modules toujours compilés et quatre modules réservés à la fonctionnalité `tray`.

| Module | Rôle |
| --- | --- |
| `config` | comptes, réglages, chemins (`AppPaths`), fichier `.env` |
| `date_layout` | arborescence de dates sous une destination |
| `email_export` | connexion IMAP, boucle d'export, écriture des notes |
| `network` | tentatives, délais, indicateurs de progression |
| `cleaner` | nettoyage du corps du message |
| `html_layout` | repérage des signatures et des tableaux de mise en page HTML |
| `email_block` | bloc ` ```email ` des notes (en-tête et corps) |
| `calendar` | invitations `.ics` |
| `quote_link` | remplacement des citations par des liens vers les notes |
| `route` | règles de tri, déplacement des notes, pièces jointes |
| `destinations` | fichier `destinations.yaml` |
| `dest_cmd` | sous-commande `dest` |
| `handled` | registre des messages traités |
| `deletion_log` | journal des suppressions côté serveur |
| `contextual_export` | preuve de source, contrôles avant suppression |
| `server_audit` | recensement en lecture seule du serveur |
| `server_cleanup` | nettoyage du serveur |
| `thunderbird` | import des comptes et des mots de passe |
| `utils` | noms de fichiers, citations, répertoires vides |
| `app_icon` | icône de l'application |
| `updater`, `progress`, `tray`, `tray_actions` | réservés à `tray` |

Le flux de données principal traverse les modules dans cet ordre : `config` fournit le compte, `email_export` lit le serveur, `cleaner` et `email_block` fabriquent la note, `route` choisit sa destination, `handled` et `deletion_log` consignent ce qui a été traité.

> **Attention** : les modules `config`, `handled`, `deletion_log`, `contextual_export`, `email_export`, `html_layout`, `server_audit`, `route` et `destinations` se référencent mutuellement. Ce n'est pas un graphe en couches ; avant de déplacer un type d'un module à l'autre, vérifiez les imports avec `cargo check`.

## Les points d'entrée

### Le binaire

`main` charge d'abord le fichier `.env` du répertoire de configuration, puis laisse `clap` analyser la ligne de commande et aiguille selon la variante de `Commands`.

`fn main() -> Result<()> {`\
`    // Load .env from the platform config directory`\
`    dotenvy::from_path(config::env_file_path()).ok();`

`    let cli = Cli::parse();`

`    match cli.command {`

Le `.ok()` est volontaire : un `.env` absent n'est pas une erreur.

### Les sous-commandes

| Sous-commande | Rôle | Fonction appelée |
| --- | --- | --- |
| `import` | importe comptes et mots de passe depuis Thunderbird | `thunderbird::*` |
| `export` | exporte un ou plusieurs comptes | `ImapExporter::export_account` |
| `dest` | gère les destinations (`list`, `add`, `suggest`) | `dest_cmd::run` |
| `repair-attachments` | répare d'anciens chemins de pièces jointes | `route::repair_legacy_attachments` |
| `link-quotes` | remplace les citations par des liens | `quote_link::link_vault` |
| `audit-server` | recense les messages restés sur le serveur | `ImapExporter::audit_server` |
| `contextual` | exporte un dossier choisi dans l'explorateur | `tray::run_contextual` |
| `tray` | lance l'icône de notification (fonctionnalité `tray`) | `tray::run_tray` |

Deux conventions de la règle `clap` du projet se voient ici. Les commentaires `///` servent d'aide en ligne. Les opérations destructrices ont un mode simulation par défaut : `link-quotes` n'écrit rien sans `--apply`, et `--apply` exige `--backup` (`requires = "backup"`).

> **Bon à savoir** : `repair-attachments` fait exception, car sa simulation est à activer avec `--dry-run` au lieu d'être le comportement par défaut. Si vous ajoutez une commande destructrice, suivez plutôt `link-quotes`.

Sans la fonctionnalité `tray`, la sous-commande `contextual` existe mais s'arrête avec un message indiquant qu'une version compilée avec l'interface graphique est nécessaire.

### Le détachement de la console

Sous Windows, les sous-commandes `tray` et `contextual` appellent `detach_console()` avant de démarrer. La fonction redirige l'entrée, la sortie et la sortie d'erreur vers `NUL` avec `SetStdHandle`, puis appelle `FreeConsole`. L'entrée standard est comprise : sans cela, le lancement du nouvel exécutable par `self_replace` échoue avec l'erreur système 50.

### La couche graphique

`tray::run_tray` et `tray::run_contextual` ouvrent chacune une boucle d'événements `tao`. Le chapitre 7 les décrit en détail.

## Conventions transverses

Plusieurs règles s'appliquent à tout le code.

- **Chemins** : toujours `Path` et `PathBuf`, assemblés avec `.join()`. Les séparateurs ne sont normalisés qu'à l'écriture dans un fichier YAML.
- **Comparaisons insensibles à la casse** : `eq_ignore_ascii_case`, jamais `.to_lowercase() ==`.
- **Champs optionnels de configuration** : `Option<T>` avec `skip_serializing_if`, résolus à la fusion ; les structures d'exécution contiennent des valeurs concrètes.
- **Secrets** : `#[serde(skip)]`, par exemple sur `Account.password`.
- **Liens symboliques** : jamais suivis par les fonctions qui parcourent une arborescence ; un lien symbolique compte comme du contenu.
- **Expressions régulières** : toute expression utilisée dans un chemin chaud est un `static LazyLock<Regex>` placé près de son consommateur.
- **Injection des chemins** : les fonctions de fond reçoivent un `AppPaths` plutôt que de lire le répertoire de configuration de l'utilisateur, ce qui rend les tests hermétiques.

`use std::sync::LazyLock;`

`static LONE_LINK_RE: LazyLock<Regex> =`\
`    LazyLock::new(|| Regex::new(r"^\[.+\]\(.+\)$").expect("static regex"));`

> **Renvoi** : le chapitre 2 détaille `AppPaths` et la fusion des réglages ; le chapitre 8 traite des erreurs et des tests.

## Parcours conseillé

Pour comprendre une note exportée de bout en bout, lisez dans l'ordre le chapitre 3 (export IMAP), le chapitre 4 (corps et format des notes), puis le chapitre 5 (routage). Les chapitres 6 et 7 couvrent ce qui entoure l'export : suppressions côté serveur et interface graphique. Le chapitre 10 est la référence de la ligne de commande : syntaxe, options et exemples de chaque sous-commande. Le chapitre 11 est la référence de la configuration : les options de chaque fichier et les changements qu'elles opèrent. Le chapitre 12 indique où atterrissent les fichiers d'un export et ce qu'il faut vérifier en cas de panne.
