# Dépannage

Ce chapitre rassemble les messages d'erreur et les situations que l'application peut rencontrer, avec leur cause probable et la marche à suivre. Les messages sont cités tels que l'application les affiche, souvent en anglais en ligne de commande. Les entrées qui concernent les fichiers de configuration ou la ligne de commande relèvent de l'utilisation avancée.

## Avant de chercher plus loin

Trois vérifications règlent la plupart des problèmes :

1. **Le compte est-il bien déclaré ?** Ouvrez **Outils**, puis **Paramètres…**, onglet **Comptes**, et lancez **Tester la connexion**.
2. **Le mot de passe est-il présent ?** Il doit figurer dans le fichier `.env` du dossier de configuration, sous le nom de variable attendu.
3. **La version est-elle à jour ?** La commande `email-to-markdown --version` affiche le numéro installé.

Pour obtenir davantage de détails sur les échanges avec le serveur, lancez l'export en ligne de commande avec l'option `--debug`.

`email-to-markdown export -a Gmail --debug`

## Erreurs de configuration

Ces messages apparaissent au chargement de la configuration.

| Message | Cause | Remède |
| --- | --- | --- |
| `Failed to read config file` | Le fichier `accounts.yaml` est introuvable ou illisible | Vérifier son emplacement dans le dossier de configuration, ou l'option `-c` |
| `Failed to parse YAML` | Le fichier contient une erreur de syntaxe | Corriger l'indentation, les deux-points, les guillemets |
| `Account name cannot be empty` | Un compte n'a pas de nom | Renseigner `name` |
| `Server not configured for account 'X'` | Le champ `server` est vide | Renseigner l'adresse du serveur IMAP |
| `Username not configured for account 'X'` | Le champ `username` est vide | Renseigner l'identifiant |
| `Invalid port (0) for account 'X'` | Le port vaut 0 | Utiliser le port du serveur, souvent 993 |
| `Export directory not configured for account 'X'. Set 'export_base_dir' in settings.yaml or via the tray.` | Aucun dossier d'export n'est défini | Utiliser **Choisir répertoire d'export…** dans le menu **Outils** |
| `Account not found` | Le nom passé à une commande ne correspond à aucun compte | Vérifier l'orthographe ; la casse est sans importance |
| `No password found for account` | Aucun mot de passe dans `.env` | Voir la section suivante |
| « Lecture de … impossible » | Le fichier `settings.yaml` est invalide | Corriger sa syntaxe ; l'application ne le modifie pas |
| « destination_max_depth doit valoir au moins 1 » | La profondeur est à 0 | Indiquer 1 ou plus |

Dans l'icône, un fichier de comptes illisible produit la ligne grisée « Comptes illisibles — voir Paramètres ».

## Mot de passe et connexion

### Aucun mot de passe trouvé

Le message en ligne de commande est « Error for X: No password found. Check your .env file. ». Le nom de la variable est dérivé du nom du compte : majuscules, avec les espaces, `@`, points et tirets remplacés par des tirets bas.

| Nom du compte | Variable cherchée en premier | Variable cherchée ensuite |
| --- | --- | --- |
| `Gmail` | `GMAIL_APPLICATION_PASSWORD` | `GMAIL_PASSWORD` |
| `Travail - Paris` | `TRAVAIL___PARIS_APPLICATION_PASSWORD` | `TRAVAIL___PARIS_PASSWORD` |

Dans le second exemple, l'espace, le tiret et l'espace suivante donnent trois tirets bas consécutifs. Le plus simple est de saisir le mot de passe dans la fenêtre **Paramètres…**, qui écrit la bonne variable à votre place.

### La connexion échoue

Le message est « Connection failed for X: … » ou, dans la fenêtre, « Échec de la connexion ». Les causes habituelles :

| Cause | Vérification |
| --- | --- |
| Mot de passe incorrect | Ressaisir le mot de passe dans la fenêtre de configuration |
| Gmail refuse le mot de passe habituel | Créer un mot de passe d'application dans les paramètres de sécurité du compte Google et l'utiliser |
| Mauvais serveur ou mauvais port | Comparer avec les informations de votre fournisseur ; le port est en général 993 |
| Réseau ou pare-feu | Tester l'accès à Internet, désactiver temporairement un proxy ou un pare-feu bloquant |
| IMAP désactivé | Activer l'accès IMAP dans les paramètres de votre compte mail |

### Erreur d'export

« Export failed for X: … » signale un problème pendant la lecture ou l'écriture. Relancez avec `--debug` pour connaître l'étape en cause. Le bilan final, de la forme « Export completed for X: N exported, N skipped, N errors », indique le nombre de messages en erreur.

## Import depuis Thunderbird

| Situation | Remède |
| --- | --- |
| « No IMAP accounts found in Thunderbird profile » | Le profil ne contient pas de compte IMAP ; essayer un autre profil avec `--list-profiles` puis `-p` |
| « Could not extract passwords » | Fermer Thunderbird, puis relancer l'import ; si un mot de passe principal est défini, le fournir avec `--master-password` ou dans la fenêtre d'import |
| Compte importé mais connexion en échec | Saisir ou corriger le mot de passe dans la fenêtre **Paramètres…** |

> **Bon à savoir** L'extraction des mots de passe dépend des composants de sécurité de Thunderbird et peut échouer selon votre installation. Elle n'est jamais indispensable : vous pouvez toujours saisir les mots de passe à la main.

## Export et tri

| Symptôme | Cause probable | Remède |
| --- | --- | --- |
| Les notes restent dans le dossier de transit | Le dossier de notes (`notes_dir`) n'est pas défini, ou la revue n'a pas été validée | Définir le dossier avec **Choisir répertoire de notes…**, puis **Reprendre le tri** |
| « notes_dir not configured in settings.yaml » | Idem, en ligne de commande | Définir `notes_dir` dans `settings.yaml` |
| « Aucun email à trier en attente pour » le compte | Aucune note en attente dans le dossier de transit | Rien à faire |
| « Lot terminé » | L'export par lots s'est arrêté après le nombre de messages fixé | Trier le lot, puis relancer l'export |
| « Export annulé » | L'export a été interrompu | Relancer l'export ; les messages déjà exportés sont ignorés |
| Toutes les notes arrivent dans `Perso/Messy/Emails` | Aucune règle ne correspond, ou le fichier `destinations.yaml` est absent | Ajouter des règles avec **Gérer les destinations…** |
| Un message est classé au mauvais endroit | Une destination placée plus haut dans la liste correspond aussi | Réordonner les destinations : la première qui correspond l'emporte |
| Un message n'a pas été réexporté | L'option **Ne pas ré-exporter les emails existants** reconnaît une note déjà faite | Comportement normal ; décocher l'option pour forcer |

### Chemins de destination refusés

Les messages « notes destination segment contains forbidden characters » et « invalid routing path » indiquent qu'un chemin contient un caractère interdit : `..`, une barre oblique inverse, un point final ou un nom réservé de Windows comme `CON` ou `NUL`. Remplacez-les par des lettres, chiffres, espaces, tirets ou tirets bas.

### Notes au mauvais format ou pièces jointes manquantes

| Symptôme | Remède |
| --- | --- |
| Un ancien message est resté en HTML brut | Menu **Outils**, **Fix HTML→Markdown**, pour le compte concerné |
| Les liens de pièces jointes d'anciennes notes sont cassés | `email-to-markdown repair-attachments -a Compte --dry-run`, puis sans `--dry-run` |
| Des images de signature ont disparu | Elles ont été écartées dans le dossier `_skipped` ; désactiver **Ignorer les images de signature** pour les conserver |
| Une note citée n'est pas reliée | Lancer `email-to-markdown link-quotes <coffre>` |

## Suppression sur le serveur

| Message ou symptôme | Signification | Remède |
| --- | --- | --- |
| « Message conservé sur le serveur, image annoncée non conservée » | Le message est volontairement gardé | Aucun ; l'application protège votre contenu |
| Un message avec pièce jointe illisible n'est pas exporté | Garde-fou de la suppression | Récupérer la pièce jointe autrement, ou désactiver la suppression pour ce compte |
| `stale-uidvalidity` dans `deletions.log` | Le serveur a renuméroté le dossier ; le message n'a pas été touché | Aucune action immédiate ; examiner le compte avec `audit-server` |
| `retry-required` dans `deletions.log` | La suppression sera retentée | Relancer un export |
| « Rien à nettoyer pour » le compte | Aucun message à supprimer | Rien à faire |
| « Nettoyer le serveur : un nettoyage est déjà en cours » | Une opération est déjà lancée | Attendre la fin |
| Le menu **Nettoyer le serveur** est absent | Aucun compte n'a la suppression activée | Activer **Supprimer après exportation** pour un compte |

Pour comprendre l'état d'un compte, utilisez `email-to-markdown audit-server <Compte> --details`.

## Mise à jour

Les erreurs de mise à jour (« GitHub API error », « No SHA256SUMS asset found in release », « No compatible asset found in release », « Download error », « failed to replace the running binary ») sont détaillées dans le chapitre « Installation et mise à jour ». Dans tous les cas, votre version actuelle reste utilisable.

## L'icône n'apparaît pas

| Cause | Remède |
| --- | --- |
| Icône masquée par Windows | Déployer la flèche « Afficher les icônes cachées » de la barre des tâches |
| L'application n'est pas lancée | Ouvrir « Email to Markdown » depuis le menu Démarrer |
| Commande inconnue en ligne de commande (`tray`, `contextual`) | Ces commandes n'existent que dans la version fournie par l'installateur, pas dans une version compilée sans l'option `tray` |

> **Attention** Ne supprimez jamais à la main les fichiers `handled_messages.txt` ou `deletions.log` pour « réparer » un problème : ils conditionnent la sécurité de la suppression sur le serveur.

## Réinitialiser sa configuration

Si la configuration est irrémédiablement confuse, vous pouvez repartir de zéro en conservant vos notes :

1. Quittez l'application depuis l'icône.
2. Sauvegardez le dossier `%APPDATA%\email-to-markdown\` en le copiant ailleurs.
3. Supprimez les fichiers `accounts.yaml` et `settings.yaml`, et si besoin les mots de passe du fichier `.env`.
4. Relancez l'application et redéclarez vos comptes, ou importez-les depuis Thunderbird.

Vos notes déjà exportées ne sont pas touchées. Conservez `handled_messages.txt` : il évite de réexporter des messages déjà traités.
