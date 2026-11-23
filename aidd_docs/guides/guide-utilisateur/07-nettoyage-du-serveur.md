# Nettoyage du *serveur*

Une fois vos messages archivés en notes, il est possible de les supprimer de la boîte mail pour libérer de la place. Cette opération est irréversible : ce chapitre explique comment l'activer, quels garde-fous l'application applique et comment suivre ce qui a été supprimé.

## Une fonction désactivée par défaut

Par défaut, l'application ne fait que lire vos messages. La suppression sur le serveur n'a lieu que si vous l'activez explicitement, compte par compte ou pour tous les comptes.

| Où | Comment |
| --- | --- |
| Pour tous les comptes | Case **Supprimer après exportation** de l'onglet **Paramètres généraux** de la fenêtre **Paramètres…** |
| Pour un compte | Choix **Oui** de **Supprimer après export** dans le bloc **Comportement** du compte |

En utilisation avancée, les mêmes réglages existent dans le fichier `settings.yaml` (clé `delete_after_export`), et l'option `--delete-after-export` de la commande `export` les force pour un seul export. Ils sont décrits dans les chapitres « Fichiers de configuration » et « Commandes en ligne ».

> **Attention** Les messages exportés sont effacés du serveur, sans retour possible. Sur Gmail, ils passent par la corbeille puis sont purgés aussitôt. Sur les autres serveurs, ils sont supprimés définitivement. Vérifiez vos premières notes avant d'activer cette option.

Pour Gmail, un mot de passe d'application est indispensable à la suppression.

## Les garde-fous

L'application supprime un message seulement quand elle est certaine que sa note existe et que la suppression vise le bon message. Elle applique pour cela plusieurs protections.

### La preuve d'origine

À l'export, chaque note reçoit dans son frontmatter une **preuve** (clé `source`) qui désigne le message exact sur le serveur : le dossier, le numéro de version du dossier (UIDVALIDITY), le numéro du message (UID), l'identifiant de message, une empreinte de ses en-têtes et, pour Gmail, l'identifiant interne du fournisseur.

Avant de supprimer, l'application relit l'en-tête du message sur le serveur et vérifie qu'il correspond bien à la preuve. Si le serveur a renuméroté ses messages entre-temps, la suppression est refusée au lieu de viser un autre message.

### Le registre des messages traités

Le fichier `handled_messages.txt`, dans le dossier de configuration, est le registre de tout ce qui a été traité, compte par compte. Il sert à éviter de réexporter un message et à retrouver ceux qui restent à supprimer.

Un message peut y être marqué comme **conservé** (colonne `held`) : il n'est alors jamais supprimé.

### Les messages que l'on garde sur le serveur

Plusieurs situations font conserver un message sur le serveur, même avec la suppression activée :

- une pièce jointe illisible : le message n'est pas exporté du tout, car l'application refuse de supprimer ce qu'elle n'a pas pu conserver intégralement ;
- une image annoncée dans le message mais non conservée : le message est gardé, avec l'avertissement « Message conservé sur le serveur, image annoncée non conservée » ;
- une note qui n'existe plus, ou dont la preuve ne correspond plus au message visé : la suppression est refusée.

Quand un message est volontairement conservé, sa note porte la clé `keep_on_server` avec la raison.

> **À retenir** En cas de doute, l'application conserve le message. Elle ne supprime jamais « au jugé » : une suppression exige une note existante, une preuve valable et une correspondance confirmée avec le serveur.

## Le journal des suppressions

Chaque tentative de suppression est consignée dans le fichier `deletions.log` du dossier de configuration. Chaque ligne contient, séparés par des tabulations : l'heure, le compte, le dossier, le numéro du message, l'identifiant du message et le résultat.

| Résultat | Signification |
| --- | --- |
| `deleted` | Le message a été supprimé |
| `already-absent` | Le message n'était plus sur le serveur |
| `retry-required: …` | La suppression n'a pas abouti et sera retentée (par exemple, il manque une information propre à Gmail) |
| `stale-uidvalidity: expected X, found Y` | Le serveur a renuméroté le dossier : le message n'a pas été touché |
| `rejected: …` | La suppression a été refusée par les garde-fous, avec la raison |
| `error: …` | Une erreur est survenue pendant la suppression d'un lot |

Ce journal est votre trace d'audit : ouvrez-le avec un éditeur de texte pour savoir précisément ce qui a été supprimé, quand et pourquoi une suppression a été refusée.

## La commande « Nettoyer le serveur »

Il peut rester sur le serveur des messages déjà triés ou écartés (par exemple, parce que vous avez activé la suppression après avoir exporté). L'entrée **Nettoyer le serveur** de l'icône les supprime en une fois. Elle n'apparaît que si au moins un compte a la suppression activée, et propose un élément par compte concerné.

### Déroulement

1. L'application fait le **recensement** des messages concernés sur le serveur.
2. Si des messages traités n'ont pas de preuve dans le registre, elle tente de la reconstituer à partir des notes déjà classées dans votre dossier de notes.
3. Elle vous demande une **confirmation**, qui par défaut est sur « Non ».
4. Après votre accord, elle supprime les messages et affiche un bilan.

### La confirmation

La fenêtre indique combien de messages du compte sont déjà triés ou écartés mais restent sur le serveur. Elle précise que le registre ne connaît ces messages que par leur identifiant de message, donc que l'appariement repose sur cet identifiant seul. Elle rappelle ensuite que la suppression est irréversible : pour Gmail, les messages passent par la corbeille d'où ils sont aussitôt purgés ; pour les autres serveurs, le serveur ne les conserve pas.

Enfin, elle demande « Les supprimer du serveur ? ». Sous Windows, il s'agit d'une boîte de dialogue Oui / Non au premier plan, dont le choix par défaut est Non.

### Le bilan

Le résultat est de la forme « N supprimé(s), N déjà absent(s), N échec(s) », complété selon le cas :

| Mention | Signification |
| --- | --- |
| N preuve(s) retrouvée(s) dans les notes, purgées au prochain export | Des preuves ont été reconstituées ; les messages seront supprimés lors du prochain export |
| N message(s) traité(s) avec preuve, supprimés par l'export (lancer Exporter) | Ces messages seront supprimés par un export ordinaire |
| N message(s) impossibles à cibler exactement, laissés sur le serveur | Sans preuve fiable, l'application s'abstient |

Si rien n'est à supprimer, le message est « Rien à nettoyer pour » suivi du compte. Si vous refusez, il est « Nettoyage annulé, rien n'a été supprimé ».

### Les refus et erreurs

| Message | Cause |
| --- | --- |
| Compte « X » introuvable | Le compte n'existe plus dans la configuration |
| X ne supprime pas les messages du serveur | La suppression n'est pas activée pour ce compte |
| Aucun mot de passe pour X, vérifier le fichier `.env` | Le mot de passe est absent |
| Connexion au serveur impossible | Problème de réseau, d'identifiants ou de serveur |
| Lecture de la configuration impossible | Le fichier de comptes ou de réglages est illisible |
| Nettoyer le serveur : un nettoyage est déjà en cours | Une autre opération de nettoyage tourne déjà |

Pendant l'opération, l'infobulle de l'icône indique « Nettoyage du serveur en cours » suivi du compte, puis l'étape en cours.

## Comprendre ce qui reste

Avant de nettoyer, vous pouvez examiner l'état d'un compte sans rien modifier : la commande `audit-server`, décrite dans le chapitre « Commandes en ligne », recense les messages encore présents sur le serveur et indique pourquoi : traités avec preuve, traités sans preuve, en attente de tri dans le dossier de transit, ou jamais exportés.

## Bonnes pratiques

1. Exportez d'abord sans suppression, et vérifiez plusieurs notes, pièces jointes comprises.
2. Sauvegardez votre dossier de notes avant la première suppression.
3. Activez la suppression pour un seul compte au début.
4. Consultez `deletions.log` après les premiers exports.
5. Pour Gmail, vérifiez que le mot de passe d'application est bien renseigné.

> **Renvoi** Pour les problèmes de connexion, de mot de passe ou de configuration, consultez le chapitre « Dépannage ».
