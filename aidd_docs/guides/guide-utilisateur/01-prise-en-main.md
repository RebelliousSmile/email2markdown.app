# Prise en *main*

Email to Markdown copie le contenu de vos boîtes mail dans des notes Markdown propres, classées dans les dossiers de votre choix. Ce chapitre présente ce que l'application produit et la manière dont les chapitres suivants s'enchaînent.

## À quoi sert l'application

L'application se connecte à vos comptes mail par le protocole IMAP, lit les messages et écrit chacun d'eux dans un fichier Markdown. Ces fichiers sont lisibles dans n'importe quel éditeur de texte et s'intègrent naturellement à un coffre de notes de type Obsidian.

Elle accomplit quatre tâches principales :

- **Exporter** les messages d'un ou plusieurs comptes vers des notes Markdown ;
- **Nettoyer** le corps des messages (retours à la ligne parasites, liens illisibles, pieds de page de réseaux sociaux) ;
- **Classer** chaque note dans le dossier de destination qui lui correspond, selon des règles que vous définissez ;
- **Supprimer**, si vous l'activez, les messages du serveur une fois qu'ils sont archivés en toute sécurité.

Au quotidien, vous l'utilisez par une icône dans la zone de notification de Windows, qui regroupe les actions courantes dans un menu et dans des fenêtres. Une ligne de commande et des fichiers de configuration modifiables à la main existent aussi, pour les usages avancés ou automatisés ; vous pouvez ne jamais y toucher.

> **Bon à savoir** La suppression sur le serveur est désactivée par défaut. Tant que vous ne l'activez pas, l'application ne fait que lire vos messages et ne modifie jamais votre boîte mail.

## Ce que contient une note

Chaque message devient un fichier `.md`. Il commence par un en-tête technique au format YAML, appelé frontmatter :

`subject_hash: a3f9c2`\
`tags:`\
`  - email`\
`attachments:`\
`  - 2026-03-14_facture.pdf`

Vient ensuite un bloc de code de type `email`, qui contient les en-têtes du message et son texte, dans un format proche de TOML :

`subject = "Facture de mars"`\
`from = "Banque Exemple <service@banque-exemple.ch>"`\
`to = ["vous@exemple.org"]`\
`date = 2026-03-14T09:12:00+01:00`\
`body = """`\
`Bonjour,`

`Veuillez trouver ci-joint votre facture du mois.`\
`"""`

Dans la note, ce bloc est encadré de trois accents graves suivis du mot `email`. Après le bloc, l'application ajoute des sections de liens :

`### Pieces jointes :`\
`- [[2026-03-14_facture.pdf]]`

Une note se compose donc de trois parties :

| Partie | Contenu |
| --- | --- |
| Frontmatter YAML | Informations techniques : empreinte du sujet, étiquette `email`, liste des pièces jointes |
| Bloc `email` | Les en-têtes du message (sujet, expéditeur, destinataires, date) et son texte nettoyé |
| Sections finales | Images, liens et pièces jointes, listés après le bloc pour s'afficher correctement |

Le chapitre sur les notes détaille chacune de ces parties.

### Le nom des fichiers

Le nom d'une note suit le modèle `email_<date>_<expéditeur>_<sujet>.md`, par exemple `email_2026-03-14_banque-exemple_facture-de-mars.md`. Si deux messages produisent le même nom, un suffixe `_2`, `_3`, etc. est ajouté. Si la date est illisible, elle est remplacée par `unknown-date`.

## Vos messages restent chez vous

L'application est un programme qui s'exécute sur votre ordinateur. Elle n'a besoin ni de compte, ni de service en ligne, et elle ne contient aucune télémétrie.

Elle ne contacte que deux destinations :

- vos propres serveurs mail, par IMAP, pour lire les messages ;
- l'interface publique de GitHub, pour savoir si une nouvelle version existe (voir le chapitre Installation et mise à jour).

Les notes sont écrites uniquement dans les dossiers que vous avez choisis. Le code source est public : chaque comportement décrit dans ce guide peut être vérifié dans le dépôt.

## Où atterrissent les notes

L'export se déroule en deux temps :

1. Les notes sont d'abord écrites dans un dossier de **transit** (appelé aussi staging), un sous-dossier par compte, situé dans le répertoire d'export.
2. Elles sont ensuite **classées** vers votre dossier de notes, dans le sous-dossier prévu par vos règles de destination, éventuellement rangé par année, mois ou jour.

Voici un exemple d'arborescence après classement :

`Notes/`\
`  Perso/`\
`    Banque/`\
`      2026/`\
`        03/`\
`          email_2026-03-14_banque-exemple_facture-de-mars.md`\
`          2026-03-14_facture.pdf`\
`    Messy/`\
`      Emails/`\
`        2026/`\
`          03/`\
`            email_2026-03-02_inconnu_bonjour.md`

Les pièces jointes sont enregistrées à plat, à côté de la note, et leur nom commence par la date du message.

## Parcours conseillé

Le guide se lit en deux parties. La première suffit pour utiliser l'application au quotidien : installation, puis tout se fait depuis l'icône de la zone de notification. La seconde, facultative, s'adresse à l'utilisation avancée.

| Étape | Chapitre | Objectif |
| --- | --- | --- |
| 1 | Installation et mise à jour | Installer l'application et la garder à jour |
| 2 | Configurer ses comptes | Déclarer vos comptes, mots de passe et dossiers depuis la fenêtre de paramètres |
| 3 | La zone de notification | Lancer un export et classer les notes |
| 4 | Notes et nettoyage du corps | Comprendre le résultat produit |
| 5 | Destinations et tri | Définir vos règles de classement depuis la fenêtre dédiée |
| 6 | Nettoyage du serveur | Libérer la boîte mail en toute sécurité |

| Utilisation avancée | Chapitre | Objectif |
| --- | --- | --- |
| A | Fichiers de configuration | Personnaliser les comptes, les réglages et les règles en éditant les fichiers |
| B | Commandes en ligne | Automatiser ou scripter les opérations |

Le chapitre Dépannage, en fin de guide, sert aux deux parties : il rassemble les messages d'erreur courants.

> **À retenir** Pour un premier essai, déclarez un seul compte, laissez la suppression désactivée, lancez un export depuis l'icône de la zone de notification et vérifiez les notes obtenues avant d'aller plus loin.

## Vocabulaire utile

| Terme | Signification |
| --- | --- |
| Compte | Une boîte mail IMAP déclarée dans l'application |
| Transit (staging) | Dossier où les notes sont écrites avant d'être classées |
| Destination | Un dossier de votre coffre de notes, défini par un chemin comme `Perso/Banque` |
| Règle | Condition (domaine, expéditeur, sujet…) qui désigne une destination |
| Pièce jointe | Fichier attaché au message, enregistré à côté de la note |
| Registre | Fichier qui mémorise les messages déjà traités |

Ces notions reviennent dans tous les chapitres suivants.
