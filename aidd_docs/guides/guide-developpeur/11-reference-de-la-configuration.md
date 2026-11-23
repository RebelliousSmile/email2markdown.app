# Référence de la *configuration*

Ce chapitre est la référence des fichiers de configuration : pour chaque option, sa valeur par défaut et ce qu'elle change. Les clés sont celles des modèles `config/accounts.yaml.example` et `config/settings.yaml.example` et des structures de `src/config.rs` ; en cas de doute, le code fait foi.

## Principes généraux

La configuration tient en quatre fichiers, rangés dans le répertoire de configuration du système.

| Système | Répertoire |
| --- | --- |
| Windows | `%APPDATA%\email-to-markdown\` |
| macOS | `~/Library/Application Support/email-to-markdown/` |
| Linux | `~/.config/email-to-markdown/` |

| Fichier | Contenu | Produit par |
| --- | --- | --- |
| `accounts.yaml` | connexion IMAP de chaque compte | `import` ou à la main |
| `.env` | un mot de passe par compte | `import --extract-passwords` ou à la main |
| `settings.yaml` | répertoires, arborescence de dates, comportement des comptes | la fenêtre **Paramètres…** ou à la main |
| `destinations.yaml` | chemins de rangement et règles de tri | `dest` ou à la main |

Pour les réglages de comportement, la priorité est toujours la même : le réglage du compte l'emporte sur `defaults`, qui l'emporte sur la valeur par défaut du programme.

> **Renvoi** : le chapitre 2 explique comment ces fichiers sont lus et fusionnés ; le chapitre 10 donne les commandes qui les produisent.

## `accounts.yaml`

Le fichier contient une liste `accounts:`. Il ne contient jamais de mot de passe.

`accounts:`\
`  - name: "Gmail"`\
`    server: "imap.gmail.com"`\
`    port: 993`\
`    username: "votre.email@gmail.com"`\
`    ignored_folders:`\
`      - "[Gmail]/Spam"`\
`      - "[Gmail]/Trash"`

| Option | Effet |
| --- | --- |
| `name` | nom du compte ; il désigne le compte dans `--account`, dans `settings.yaml` et dans le nom de la variable du mot de passe |
| `server`, `port` | serveur IMAP et son port (993 pour une connexion chiffrée) |
| `username` | identifiant de connexion |
| `included_folders` | si la liste n'est pas vide, seuls ces dossiers sont exportés ; elle l'emporte sur `ignored_folders` |
| `ignored_folders` | dossiers exclus de l'export, par leur nom affiché |

Le dossier `INBOX` se compare sans tenir compte de la casse.

## `.env`

`GMAIL_PASSWORD=votre_mot_de_passe`\
`GMAIL_APPLICATION_PASSWORD=xxxx-xxxx-xxxx-xxxx`

Le nom de la variable est le nom du compte en majuscules, suivi de `_PASSWORD`. Les espaces et les caractères `@`, `.` et `-` du nom deviennent `_`. La variable `_APPLICATION_PASSWORD`, testée en premier, est prioritaire ; elle sert aux mots de passe d'application, par exemple pour un compte Gmail protégé par une double authentification.

> **Attention** : sans l'une de ces deux variables, la lecture du compte échoue. Le nom de la variable doit correspondre exactement au `name` du compte.

## `settings.yaml`

Le fichier se modifie à la main ou par la fenêtre **Paramètres…** de l'icône de notification.

### Les réglages globaux

| Option | Défaut | Effet |
| --- | --- | --- |
| `export_base_dir` | aucun | zone d'attente de l'export ; chaque compte y crée son sous-dossier |
| `notes_dir` | aucun | racine de vos notes ; si elle est définie, `export` trie automatiquement vers les destinations |
| `destinations_file` | `destinations.yaml` du répertoire de configuration | emplacement du fichier de destinations |
| `date_layout` | `year_month` | dossiers datés ajoutés sous chaque destination |
| `organize_by_date` | — | ancienne clé, lue seulement si `date_layout` est absent |
| `destination_max_depth` | 2 | nombre maximal de niveaux d'une destination sous sa branche |
| `ai_routing_enabled` | `false` | active le routage assisté ; à `false`, le tri reste déterministe |
| `ai_confidence_threshold` | 0.7 | confiance minimale pour retenir une proposition du routage assisté |

`date_layout` choisit les dossiers créés pour un message du 3 octobre 2026.

| Valeur | Dossier final |
| --- | --- |
| `none` | `Perso/Banque` |
| `year` | `Perso/Banque/2026` |
| `year_month` | `Perso/Banque/2026/10` |
| `year_month_day` | `Perso/Banque/2026/10/03` |

Changer de motif ne déplace aucune note : les notes rangées sous un ancien motif restent reconnues. Quand `date_layout` est présent, il l'emporte sur `organize_by_date` (`true` équivaut à `year_month`, `false` à `none`) ; la fenêtre **Paramètres…** écrit les deux clés.

`destination_max_depth` ne compte ni la branche (`Perso`, `Pro`) ni les dossiers datés. Avec la valeur 2, `Perso/Banque` et `Perso/Banque/BNP` sont proposés, mais pas `Perso/Banque/BNP/Livret`. La valeur 0 est refusée. Une destination plus profonde reste utilisable : `dest add` affiche un avertissement et la fenêtre des destinations la marque « trop profonde ».

### Le comportement des comptes

Ces options se placent dans `defaults`, pour tous les comptes, ou sous `accounts.<nom>`, pour un seul.

`defaults:`\
`  quote_depth: 1`\
`  delete_after_export: false`\
`accounts:`\
`  Outlook:`\
`    collect_contacts: true`

| Option | Défaut | Effet |
| --- | --- | --- |
| `folder_name` | nom du compte | nom du sous-dossier du compte dans `export_base_dir` (par compte seulement) |
| `quote_depth` | 1 | profondeur maximale des citations conservées dans le corps |
| `skip_existing` | `true` | ne ré-exporte pas les messages déjà présents |
| `collect_contacts` | `false` | produit un fichier CSV des contacts |
| `skip_signature_images` | `false` | n'enregistre pas les images de signature ni les logos |
| `delete_after_export` | `false` | supprime du serveur les messages exportés |
| `cleanup_empty_dirs` | `true` | supprime sans bruit les dossiers vides après l'export d'un compte |
| `batch_size` | 0 | arrête l'export après ce nombre de nouveaux messages, pour les trier avant le lot suivant ; 0 n'impose aucune limite |

> **Bon à savoir** : le modèle `settings.yaml.example` active `skip_signature_images`, alors que la valeur codée en dur est `false`. Recopier le modèle change donc le comportement par défaut.

### `skip_signature_images`

Une image collée dans le corps du message est toujours conservée. Sont écartées : les images du bloc de signature du client de messagerie, celles dont le nom évoque un logo ou une signature, et les icônes de moins de 10 Ko. Avec `skip_signature_images: false`, une image de signature reste enregistrée et listée dans `attachments`, mais n'apparaît plus dans le corps de la note.

Les marqueurs `[image: nom]` du texte brut de Gmail sont remplacés par les images qu'ils désignent, dans l'ordre. Une image distante autre qu'une signature devient un simple lien, jamais une image intégrée. Si le nombre de marqueurs diffère du nombre d'images, le texte reste inchangé.

### `delete_after_export`

L'option supprime du serveur chaque message exporté, avec trois garde-fous.

- Une image écartée n'est pas perdue : elle est mise en quarantaine dans `<export_directory>/_skipped/<nom de la note>/`, et l'export en signale le nombre. Le dossier se vide à la main, une fois vérifié.
- Un message n'est jamais supprimé si une image qu'il annonce n'a été ni enregistrée ni mise en quarantaine. La note porte alors la clé `keep_on_server`, accompagnée de la raison.
- Un tel message est noté « retenu » dans `handled_messages.txt`, n'est plus proposé à l'export, et « Nettoyer le serveur » ne le vise jamais. Seul un rejet dans la fenêtre de revue en demande explicitement la suppression.

> **Renvoi** : le chapitre 6 décrit le registre des messages traités et les contrôles avant suppression.

## `destinations.yaml`

Le fichier décrit les chemins de rangement et les règles qui y mènent. Un `destinations.txt` de l'ancien format est converti en `destinations.yaml` au premier lancement ; le `.txt` est conservé.

`destinations:`\
`  - path: Perso/Famille`\
`  - path: Pro/Clients/Acme`\
`    note: client principal`\
`    rules:`\
`      - domain: acme.com`\
`  - path: Perso/Messy/Emails`\
`    default: true`

| Option | Effet |
| --- | --- |
| `path` | chemin relatif sous `notes_dir` ; seul, il devient une option de classement proposée dans la fenêtre de revue |
| `note` | texte libre qui décrit la destination |
| `rules` | liste de règles ; un message qui en vérifie une est rangé ici |
| `default: true` | fait de la destination le dossier fourre-tout ; au plus une entrée |

Les règles disponibles :

| Règle | Un message correspond si… |
| --- | --- |
| `domain: <d>` | l'expéditeur vient du domaine `d`, sous-domaines compris |
| `from: <adresse>` | l'expéditeur a exactement cette adresse |
| `correspondent: <adresse>` | cette adresse figure dans From, To, Cc ou Bcc |
| `subject: <mot>` | le sujet contient ce mot |
| `account: <nom>` | le message a été reçu sur ce compte |

Les comparaisons ignorent la casse, sauf le nom de compte. La première règle qui correspond l'emporte. Le premier segment du chemin (`Perso` ou `Pro`) détermine la polarité : `Perso` par défaut, tant qu'aucune règle ne force `Pro`. Sans correspondance, le message tombe dans la destination par défaut.

> **Renvoi** : `dest` est décrite au chapitre 10, les règles au chapitre 5.
