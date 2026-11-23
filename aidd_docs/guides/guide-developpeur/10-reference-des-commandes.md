# Référence des *commandes*

Ce chapitre est la référence de la ligne de commande : pour chaque sous-commande, sa syntaxe, ses options et quelques exemples. Les options sont relevées dans `src/main.rs` et `src/dest_cmd.rs`, où `clap` les déclare ; en cas de doute, `--help` fait foi.

## Principes généraux

Le binaire s'appelle `email-to-markdown`. Il charge d'abord le fichier `.env` du répertoire de configuration, puis exécute la sous-commande demandée.

`email-to-markdown --version`\
`email-to-markdown --help`\
`email-to-markdown export --help`

`--help` s'applique à toutes les sous-commandes. Plusieurs d'entre elles acceptent `-c, --config <CHEMIN>` pour désigner un autre fichier de configuration ; sans cette option, elles lisent le répertoire de configuration du système (chapitres 2 et 11).

| Sous-commande | Rôle | Section |
| --- | --- | --- |
| `import` | importe comptes et mots de passe depuis Thunderbird | `import` |
| `export` | exporte les comptes IMAP et range les notes | `export` |
| `dest` | gère les destinations de rangement | `dest` |
| `audit-server` | recense les messages restés sur le serveur | Les commandes de maintenance |
| `link-quotes` | remplace les citations par des liens | Les commandes de maintenance |
| `repair-attachments` | répare d'anciens chemins de pièces jointes | Les commandes de maintenance |
| `contextual` | exporte depuis un dossier de destination | `contextual` et `tray` |
| `tray` | lance l'icône de notification | `contextual` et `tray` |

> **Renvoi** : le chapitre 1 relie chaque sous-commande à la fonction qu'elle appelle ; le chapitre 9 explique comment en ajouter une.

## `import`

`import` génère `accounts.yaml`, qui ne contient que les paramètres de connexion, dans le répertoire de configuration du système.

`email-to-markdown import [OPTIONS]`

| Option | Description |
| --- | --- |
| `--list-profiles` | liste les profils Thunderbird disponibles, puis quitte |
| `-p, --profile <CHEMIN>` | utilise ce profil (chemin absolu) ; sans cette option, détection automatique |
| `-o, --output <CHEMIN>` | fichier de sortie ; par défaut, le répertoire de configuration du système |
| `--generate-env` | génère aussi un fichier `.env.template` listant les variables à remplir |
| `--extract-passwords` | déchiffre les mots de passe de Thunderbird et les écrit dans `.env` ; Thunderbird doit être fermé |
| `--master-password <MDP>` | Master Password de Thunderbird, seulement si vous en avez configuré un |

Quelques exemples :

`email-to-markdown import --list-profiles`\
`email-to-markdown import`\
`email-to-markdown import --extract-passwords`\
`email-to-markdown import --extract-passwords --master-password "secret"`\
`email-to-markdown import --profile ~/.thunderbird/abc123.default --generate-env`\
`email-to-markdown import --output /chemin/vers/accounts.yaml`

> **Renvoi** : l'extraction des mots de passe et le rapprochement avec `.env` sont décrits au chapitre 2.

## `export`

`export` connecte les comptes IMAP configurés, convertit chaque message en note Markdown dans le répertoire d'export, puis déplace la note vers sa destination selon `destinations.yaml`.

Le flux complet tient en quatre temps :

1. connexion IMAP et téléchargement des messages ;
2. conversion en note Markdown, dans le répertoire d'export qui sert de zone tampon ;
3. calcul de la destination d'après les règles de `destinations.yaml` ;
4. déplacement vers `notes_dir/<chemin>/<dossiers datés>`, selon le motif `date_layout` (par défaut `<Année>/<Mois>`).

Si aucune règle ne correspond, la note atterrit dans le dossier fourre-tout, `notes_dir/Perso/Messy/Emails`, suivi des dossiers datés. Si `destinations.yaml` est absent ou non configuré, un avertissement s'affiche et tous les messages tombent dans ce fourre-tout : l'export continue sans erreur fatale.

`email-to-markdown export [OPTIONS]`

| Option | Description |
| --- | --- |
| `--list-accounts` | liste les comptes configurés dans `accounts.yaml`, puis quitte |
| `-a, --account <NOM>` | exporte seulement le ou les comptes indiqués, séparés par des virgules |
| `-c, --config <CHEMIN>` | fichier de configuration ; par défaut, le répertoire de configuration du système |
| `-d, --debug` | mode verbeux, avec la sortie IMAP brute |
| `--delete-after-export` | supprime les messages du serveur après l'export |

> **Attention** : `--delete-after-export` supprime des messages sur le serveur. Une suppression exige une preuve de source complète dans la note ; le chapitre 6 décrit ce garde-fou.

Quelques exemples :

`email-to-markdown export`\
`email-to-markdown export --list-accounts`\
`email-to-markdown export --account Gmail`\
`email-to-markdown export --account Gmail,Outlook`\
`email-to-markdown export --account Gmail --debug`\
`email-to-markdown export --config /chemin/vers/accounts.yaml`\
`email-to-markdown export --account Gmail --delete-after-export`

> **Renvoi** : le chapitre 3 suit l'export de la ligne de commande jusqu'à l'écriture du fichier ; le chapitre 4 décrit le format de la note.

## `dest`

`dest` gère `destinations.yaml`. Sans sous-commande, elle ouvre l'éditeur interactif guidé.

| Commande | Effet |
| --- | --- |
| `dest` | lance l'éditeur interactif |
| `dest list` | affiche les destinations et leurs règles, et repère les anomalies |
| `dest add <CHEMIN> [OPTIONS]` | ajoute une destination, avec ou sans règle |
| `dest suggest` | propose, de façon interactive, des règles à partir des messages déjà tombés dans le fourre-tout |

`<CHEMIN>` est un chemin relatif sous `notes_dir`, par exemple `Perso/Banque`. Un chemin nu, sans règle, est une option de classement. Les options de `dest add` :

| Option | Règle ajoutée |
| --- | --- |
| `--domain <DOMAINE>` | les messages de ce domaine d'expéditeur |
| `--from <ADRESSE>` | les messages de cette adresse exacte |
| `--correspondent <ADRESSE>` | les messages où cette adresse figure dans From, To, Cc ou Bcc |
| `--subject <MOT>` | les messages dont le sujet contient ce mot |
| `--account <NOM>` | les messages reçus sur ce compte IMAP |
| `--note <TEXTE>` | une note qui décrit la destination |
| `--default` | marque la destination comme dossier par défaut |

Quelques exemples :

`email-to-markdown dest`\
`email-to-markdown dest list`\
`email-to-markdown dest add "Perso/Famille"`\
`email-to-markdown dest add "Perso/Banque" --domain mabanque.fr --subject relevé --note "factures"`\
`email-to-markdown dest add "Pro/Clients/Acme" --correspondent contact@acme.example`\
`email-to-markdown dest suggest`

Lancé sans sous-commande, `dest` ouvre une liste numérotée que filtre une sous-chaîne du chemin, puis des actions au clavier la pilotent : `a` ajoute, `e N` édite la note, `s N` supprime, `r N` gère les règles, `d N` définit le défaut, `f` filtre à nouveau et `q` quitte. Chaque action est sauvegardée immédiatement. `dest suggest` regroupe les messages du dossier par défaut par domaine d'expéditeur ; aucun message déjà exporté n'est déplacé, les règles ne valent que pour les exports suivants.

> **Renvoi** : le chapitre 5 décrit l'évaluation des règles et le code de `dest_cmd.rs`.

## Les commandes de maintenance

Ces trois commandes agissent sur des notes ou des messages déjà exportés. Les deux premières ne modifient rien sans demande explicite.

### `audit-server`

`audit-server` fait un recensement en lecture seule. Pour chaque message encore présent sur le serveur d'un compte, elle indique pourquoi : traité avec ou sans preuve, rangé en note, ou jamais exporté. Rien n'est modifié.

`email-to-markdown audit-server <COMPTE> [OPTIONS]`

| Option | Description |
| --- | --- |
| `--folder <DOSSIER>` | n'audite que ce dossier, par son nom affiché ou son nom IMAP brut |
| `--details` | liste chaque message, pas seulement les totaux |
| `--uid <UID>` | compare l'identité d'un message lue en RFC822 et dans son bloc d'en-têtes ; exige `--folder` |
| `-c, --config <CHEMIN>` | fichier de configuration |

`email-to-markdown audit-server Gmail`\
`email-to-markdown audit-server Gmail --folder INBOX --details`

### `link-quotes`

`link-quotes` remplace, dans un coffre existant, les messages cités par des wikilinks vers leurs notes. Sans option, elle ne fait qu'un essai : elle liste ce qui serait lié et pourquoi le reste reste du texte.

`email-to-markdown link-quotes <COFFRE> [--apply --backup <DOSSIER>]`

| Option | Description |
| --- | --- |
| `--apply` | réécrit les notes ; sans elle, rien n'est écrit |
| `--backup <DOSSIER>` | dossier extérieur au coffre qui reçoit une copie de chaque note avant sa réécriture |

`--apply` exige `--backup`.

`email-to-markdown link-quotes /chemin/vers/le/coffre`\
`email-to-markdown link-quotes /chemin/vers/le/coffre --apply --backup /chemin/hors/coffre`

### `repair-attachments`

`repair-attachments` concerne les notes exportées avec l'ancien schéma centralisé (`<compte>/attachments/<dossier>/`), puis déplacées. Elle range les pièces jointes à côté de leur note et ramène le frontmatter à des noms de fichier seuls.

`email-to-markdown repair-attachments --account <NOM> [--dry-run]`

| Option | Description |
| --- | --- |
| `-a, --account <NOM>` | compte dont le dossier de pièces jointes sert de source ; obligatoire |
| `--dry-run` | affiche les changements sans les appliquer |
| `-c, --config <CHEMIN>` | fichier de configuration |

`email-to-markdown repair-attachments --account Gmail --dry-run`

> **Attention** : `repair-attachments` applique ses changements par défaut et n'a une simulation qu'avec `--dry-run`. `link-quotes` fait l'inverse, et c'est le modèle à suivre pour une nouvelle commande destructrice (chapitre 9).

## `contextual` et `tray`

### `contextual`

`contextual` ouvre la fenêtre de recherche directement dans une destination exacte de `destinations.yaml`.

`email-to-markdown contextual "/chemin/Notes/Pro/Client"`

Le programme n'a pas besoin d'être lancé au préalable. Il demande la boîte aux lettres, recherche les messages qui correspondent aux règles d'adresse du dossier, laisse la liste entièrement décochée, convertit la sélection dans ce dossier, puis se ferme. Si `delete_after_export` est actif, seuls les messages qui portent une preuve Markdown locale complète sont supprimés du serveur.

Le dossier doit être un répertoire local existant, non symbolique, situé sous `notes_dir` et identique à une destination configurée. Tout autre chemin est refusé avant la moindre connexion IMAP.

### `tray`

`tray` lance l'application en icône de notification, sous Windows, macOS et Linux.

`email-to-markdown tray`

> **Attention** : `tray` et `contextual` exigent un binaire compilé avec la fonctionnalité `tray`, par exemple avec `cargo build --release --features tray`. Sans elle, `tray` n'existe pas et `contextual` s'arrête avec un message explicite.

Au premier lancement sans compte configuré, le sous-menu d'export est désactivé. Le menu de l'icône, la fenêtre de revue du routage et la mise à jour sont décrits au chapitre 7.
