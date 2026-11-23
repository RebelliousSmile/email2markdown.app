# Structure des exports et *dépannage*

Ce chapitre indique où atterrissent les fichiers d'un export, puis les pannes les plus courantes de l'application, avec la vérification à faire pour chacune.

## Où atterrissent les fichiers

Un export passe par deux zones. Le répertoire d'export (`export_base_dir`) sert de zone d'attente : chaque compte y crée un sous-dossier, puis un sous-dossier par dossier IMAP. Une fois le tri appliqué, la note est déplacée sous `notes_dir`, selon les règles de `destinations.yaml`, et la zone d'attente peut être vidée.

| Zone | Exemple de chemin |
| --- | --- |
| Zone d'attente | `export_base_dir/Gmail/INBOX/email_2024-01-15_AB_to_CD.md` |
| Zone d'attente, autre dossier | `export_base_dir/Gmail/Sent/email_2024-01-15_AB_to_CD.md` |
| Destination avec règle | `notes_dir/Pro/Clients/Acme/2024/01/email_2024-01-15_AB_to_CD.md` |
| Destination par défaut | `notes_dir/Perso/Messy/Emails/2024/01/email_2024-01-15_AB_to_CD.md` |

Les dossiers `2024/01` dépendent de `date_layout` ; le chapitre 11 donne les quatre motifs possibles.

> **Renvoi** : le chapitre 3 suit l'export jusqu'à l'écriture de la note ; le chapitre 5 décrit le déplacement vers la destination.

## Dépannage

Les problèmes liés à l'environnement de développement ou au système (compilation sous Windows, icône de barre système, certificats TLS) sont dans le fichier `TROUBLESHOOTING.md`, à la racine du dépôt. Le tableau suivant concerne l'application elle-même.

| Symptôme | Vérification |
| --- | --- |
| Mot de passe introuvable | la variable `NOM_PASSWORD` de `.env` doit correspondre exactement au `name` du compte dans `accounts.yaml` (chapitre 11) |
| Échec de `--extract-passwords` | fermer Thunderbird avant de lancer la commande ; si un Master Password est configuré, ajouter `--master-password` |
| Connexion IMAP refusée | activer l'accès IMAP dans les paramètres du compte de messagerie |
| Dossiers manquants | ajuster `ignored_folders` ou `included_folders` dans `accounts.yaml` ; `--debug` affiche les dossiers disponibles |
| Messages dans le fourre-tout | vérifier `destinations.yaml` (et `destinations_file` dans `settings.yaml`) et que les règles correspondent aux expéditeurs ; `dest list` aide à les inspecter |

> **Bon à savoir** : le guide de l'utilisateur consacre un chapitre entier au dépannage, avec les messages d'erreur exacts. Le tableau ci-dessus en est la version courte, à l'usage de qui lit le code.

> **Renvoi** : le chapitre 10 décrit `--debug`, `--master-password` et `dest list`.
