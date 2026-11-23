# Fichiers de *configuration*

Ce chapitre relève de l'utilisation avancée. Pour un usage courant, la fenêtre **Paramètres…** de l'icône de la zone de notification écrit ces fichiers à votre place. Les éditer à la main sert à automatiser une installation, à sauvegarder ou à partager une configuration, à travailler sur plusieurs machines ou à dépanner.

## Les fichiers et leur rôle

Tous les réglages se trouvent dans le dossier de configuration de l'application, `%APPDATA%\email-to-markdown\` sous Windows. Chaque fichier a un rôle précis.

| Fichier | Rôle |
| --- | --- |
| `accounts.yaml` | Informations de connexion : nom du compte, serveur, port, identifiant, dossiers à lire |
| `.env` | Mots de passe, un par compte |
| `settings.yaml` | Comportement : dossier d'export, profondeur des citations, suppression, découpage en lots… |
| `destinations.yaml` | Règles de classement des notes |
| `handled_messages.txt` | Registre des messages déjà traités (géré par l'application) |
| `deletions.log` | Journal des suppressions sur le serveur (géré par l'application) |

Les deux derniers fichiers, `handled_messages.txt` et `deletions.log`, appartiennent à l'application : ne les modifiez pas.

> **Bon à savoir** Le fichier `accounts.yaml` ne contient jamais de mot de passe. Les mots de passe sont séparés dans le fichier `.env`, ce qui permet de partager ou de sauvegarder la liste des comptes sans exposer d'identifiants.

> **Attention** Si la fenêtre **Paramètres…** est ouverte pendant que vous modifiez un fichier à la main, cliquez sur son bouton **Recharger** avant d'enregistrer, pour qu'elle reprenne votre modification.

## accounts.yaml : les comptes

Chaque compte est une entrée de la liste `accounts`. Voici un exemple avec deux comptes :

`accounts:`\
`  - name: Gmail`\
`    server: imap.gmail.com`\
`    port: 993`\
`    username: prenom.nom@gmail.com`\
`    ignored_folders:`\
`      - "[Gmail]/Spam"`\
`      - "[Gmail]/Trash"`\
`      - "[Gmail]/All Mail"`\
`      - "[Gmail]/Drafts"`

`  - name: Outlook`\
`    server: outlook.office365.com`\
`    port: 993`\
`    username: prenom.nom@outlook.com`\
`    ignored_folders:`\
`      - Junk`\
`      - Deleted Items`

| Clé | Signification |
| --- | --- |
| `name` | Nom du compte, au choix, jamais vide. La recherche d'un compte ignore les majuscules |
| `server` | Adresse du serveur IMAP |
| `port` | Port de connexion, généralement 993 ; la valeur 0 est refusée |
| `username` | Identifiant de connexion, souvent l'adresse mail complète |
| `included_folders` | Facultatif. Liste des dossiers à lire ; si elle est présente, elle remplace `ignored_folders` |
| `ignored_folders` | Liste des dossiers à ne pas lire |

Le dossier `INBOX` est reconnu sans tenir compte des majuscules ou minuscules.

## .env : les mots de passe

Le mot de passe de chaque compte est lu dans le fichier `.env`, sous un nom de variable dérivé du nom du compte : le nom est mis en majuscules, et les espaces, `@`, points et tirets sont remplacés par des tirets bas. L'application cherche d'abord `<NOM>_APPLICATION_PASSWORD`, puis `<NOM>_PASSWORD`.

| Nom du compte | Variables reconnues |
| --- | --- |
| `Gmail` | `GMAIL_APPLICATION_PASSWORD`, sinon `GMAIL_PASSWORD` |
| `My Work Account` | `MY_WORK_ACCOUNT_APPLICATION_PASSWORD`, sinon `MY_WORK_ACCOUNT_PASSWORD` |
| `perso-2` | `PERSO_2_APPLICATION_PASSWORD`, sinon `PERSO_2_PASSWORD` |

Exemple de fichier `.env` :

`GMAIL_APPLICATION_PASSWORD=abcdefghijklmnop`\
`OUTLOOK_PASSWORD=motdepasse`

> **Attention** Gmail refuse le mot de passe habituel du compte pour les applications externes. Il faut créer un « mot de passe d'application » dans les paramètres de sécurité de votre compte Google, et le placer dans la variable `GMAIL_APPLICATION_PASSWORD`. Ce mot de passe d'application est aussi requis pour la suppression sur le serveur.

Les valeurs qui contiennent un saut de ligne ou un caractère nul sont refusées lors de l'enregistrement. Sous Linux et macOS, le fichier est créé lisible par son seul propriétaire.

## settings.yaml : les réglages

Le fichier `settings.yaml` décrit le comportement commun à tous les comptes, avec la possibilité de le surcharger compte par compte. S'il n'existe pas, les valeurs par défaut s'appliquent. S'il existe mais qu'il est invalide, l'application le signale par un message de la forme « Lecture de … impossible : … » et ne le modifie pas.

`export_base_dir: C:/Users/Prenom/Documents/Emails`\
`notes_dir: C:/Users/Prenom/Documents/Notes`\
`date_layout: year_month`\
`destination_max_depth: 2`

`defaults:`\
`  quote_depth: 1`\
`  skip_existing: true`\
`  collect_contacts: false`\
`  skip_signature_images: true`\
`  delete_after_export: false`\
`  cleanup_empty_dirs: true`\
`  batch_size: 0`

`accounts:`\
`  Gmail:`\
`    folder_name: gmail`\
`    quote_depth: 2`

### Les clés générales

| Clé | Rôle | Valeur par défaut |
| --- | --- | --- |
| `export_base_dir` | Dossier de transit : chaque compte y reçoit un sous-dossier | Aucune : à renseigner |
| `notes_dir` | Dossier racine où les notes sont classées | Aucune |
| `destinations_file` | Chemin du fichier de règles | `destinations.yaml` dans le dossier de configuration |
| `date_layout` | Sous-dossiers datés : `none`, `year`, `year_month` ou `year_month_day` | `year_month` |
| `destination_max_depth` | Nombre de niveaux autorisés sous la branche d'une destination | 2 |

La valeur 0 pour `destination_max_depth` est refusée : « destination_max_depth doit valoir au moins 1 ».

### Les comportements par compte

Ces clés se placent soit dans `defaults` (pour tous les comptes), soit dans l'entrée d'un compte sous `accounts` (pour ce compte seulement). Une valeur propre au compte l'emporte sur `defaults`, qui l'emporte à son tour sur la valeur de repli.

| Clé | Effet | Repli |
| --- | --- | --- |
| `folder_name` | Nom du sous-dossier de transit (par compte uniquement) | Nom du compte |
| `quote_depth` | Nombre de niveaux de citations conservés dans le corps | 1 |
| `skip_existing` | Ne pas ré-exporter un message déjà exporté | oui |
| `collect_contacts` | Constituer un fichier CSV des contacts rencontrés | non |
| `skip_signature_images` | Ignorer les images de signature et logos | non |
| `delete_after_export` | Supprimer du serveur les messages archivés | non |
| `cleanup_empty_dirs` | Supprimer les dossiers vides après chaque export | oui |
| `batch_size` | Nombre de messages par export ; 0 signifie tout d'un coup | 0 |

> **Bon à savoir** Le fichier d'exemple fourni avec l'application active `skip_signature_images`. Si vous créez votre configuration vous-même et ne renseignez pas cette clé, les images de signature ne sont pas ignorées. Précisez toujours la valeur voulue.

> **Attention** `delete_after_export: true` supprime des messages du serveur, sans retour possible. Lisez le chapitre « Nettoyage du serveur » avant de l'activer.

## destinations.yaml : les règles de classement

Les destinations sont décrites dans le fichier `destinations.yaml` du dossier de configuration. Le principe du tri est présenté dans le chapitre « Destinations et tri » ; voici le format du fichier.

`destinations:`\
`  - path: Perso/Banque`\
`    note: relevés et factures bancaires`\
`    rules:`\
`      - domain: ubs.ch`\
`      - subject: facture`

`  - path: Pro/Clients/Dupont`\
`    rules:`\
`      - correspondent: marie.dupont@exemple.org`

`  - path: Perso/Messy/Emails`\
`    default: true`

| Clé | Rôle |
| --- | --- |
| `path` | Chemin de la destination, relatif au dossier de notes |
| `note` | Description libre, pour vous y retrouver |
| `rules` | Liste de règles ; une seule suffit à désigner la destination |
| `default` | Si `true`, la destination reçoit les notes qu'aucune règle ne désigne |

Une seule destination peut être marquée `default: true`. Les noms de règles du fichier (`domain`, `from`, `correspondent`, `subject`, `account`) sont ceux décrits dans le chapitre « Destinations et tri », qui détaille aussi leur priorité, les règles de forme des chemins et la profondeur maximale.

> **À retenir** L'ordre des destinations dans le fichier a de l'importance : la première destination dont une règle correspond l'emporte. Mettez les règles les plus précises en premier, et les plus générales en dernier.

Si le fichier `destinations.yaml` est absent, l'application le signale par un avertissement et dirige toutes les notes vers la destination par défaut.

## Sauvegarder et déplacer sa configuration

Pour reprendre votre configuration sur un autre ordinateur, copiez le dossier de configuration. Les chemins de `settings.yaml` (`export_base_dir`, `notes_dir`) sont à adapter à la nouvelle machine.

> **Attention** Le fichier `.env` contient vos mots de passe en clair : ne le copiez que sur un support de confiance, et ne le placez jamais dans un dépôt partagé ni dans une synchronisation publique.
