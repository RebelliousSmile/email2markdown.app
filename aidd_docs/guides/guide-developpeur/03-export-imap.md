# Le flux d'export *IMAP*

`email_export.rs` concentre la connexion au serveur, la boucle d'export d'un compte et la fabrication d'une note. Ce chapitre suit un export du déclenchement par la ligne de commande jusqu'à l'écriture du fichier ; le contenu de la note est détaillé au chapitre 4.

## De la commande à l'exportateur

La sous-commande `export` charge la configuration, filtre les comptes demandés (liste séparée par des virgules, sans tenir compte de la casse), puis exécute pour chacun la même séquence :

1. `ImapExporter::new(account, debug)` ;
2. `connect()` ;
3. `export_account(None, None, None)` ;
4. affichage de `purge_summary`.

L'option `--delete-after-export` ne fait qu'activer le réglage : `account.delete_after_export` vaut `flag || account.delete_after_export`. Si `settings.notes_dir` est défini, chaque note exportée est ensuite triée automatiquement par `route::apply_decision` ; sinon, elle reste dans la zone d'attente (`export_base_dir`).

La fenêtre de l'icône appelle `export_account_in(paths, on_progress, on_status, cancel_token)` avec des rappels de progression et un jeton d'annulation (chapitre 7).

## La connexion

`open_session` construit la session avec `imap::ClientBuilder`.

- Le TLS est strict. Un certificat auto-signé n'est accepté que si la variable `EMAIL_TO_MARKDOWN_IMAP_INSECURE_TEST=1` est définie et que le serveur est `localhost`, ce qui sert aux tests d'intégration.
- Un délai de lecture est posé à la main sur le flux.
- L'authentification tente la connexion standard puis `PLAIN` ; un refus devient `ProbeFailure::AuthRejected`.

`ImapExporter` conserve la session, le compte, un `NetworkConfig`, l'indicateur `is_gmail` (serveur `gmail.com` ou `googlemail.com`), les compteurs de lot (`batch_exported`, `batch_full`) et l'ensemble `purge_targeted`.

### Les tentatives

`NetworkConfig` fixe les valeurs par défaut suivantes, utilisées par `with_retry` (reprise avec attente exponentielle) :

| Paramètre | Valeur |
| --- | --- |
| tentatives maximales | 3 |
| attente initiale | 1 s |
| attente maximale | 30 s |
| délai de connexion | 30 s |
| délai de lecture | 60 s |

### La sonde de connexion

`probe_account` teste un compte sans rien exporter ; la fenêtre de paramètres et l'import Thunderbird s'en servent. Elle s'exécute dans un fil séparé et abandonne après `PROBE_TIMEOUT` (20 s). `describe_connection_error` traduit les échecs en messages français destinés à l'utilisateur.

## Lister les dossiers

Deux fonctions existent, avec deux usages.

| Fonction | Exclut | Usage |
| --- | --- | --- |
| `list_folders()` | dossiers spéciaux (`FolderRole::Special`) et `\Noselect` | export |
| `list_all_folders()` | rien | sélecteur de dossiers de la configuration |

`FolderName` porte à la fois le nom brut IMAP (UTF-7 modifié) et le nom affiché ; `FolderRole` distingue `Inbox`, `Sent`, `Special` et `Regular`. Le nom brut sert aux commandes IMAP et aux preuves de source, le nom affiché aux chemins locaux.

## La boucle d'un compte

`export_account_in` enchaîne :

1. chargement des destinations (`route::load_destinations_from`) et de la `date_layout` ;
2. lecture du registre `handled` par `load_proofs_from`, seulement si `skip_existing` est actif ;
3. partage des dossiers par `Account::exports_folder` ; un nom de dossier dangereux, rejeté par `imap_folder_relative_path`, est ignoré ;
4. export dossier par dossier ;
5. génération du fichier de contacts si `collect_contacts` est actif ;
6. arrêt dès que le lot est plein (`batch_full`) ou que l'annulation est demandée.

La boucle est enveloppée dans une fonction anonyme immédiatement appelée : ainsi `cleanup_empty_dirs`, qui supprime les répertoires vides laissés par l'export, s'exécute sur tous les chemins de sortie, erreurs comprises.

La progression compte `PROGRESS_STEPS_PER_FOLDER` (1000) pas par dossier, ou bien `(batch_exported, batch_size)` quand un lot est défini. Le fichier de contacts est un CSV UTF-8 avec BOM, écrit dans `<parent du répertoire d'export>/_local/contacts/`, avec l'en-tête `First Name, Last Name, Display Name, Email, Notes`.

## L'export d'un dossier

`export_folder` est le cœur de l'algorithme.

### Préparation

- `ExportedIndex::build` parcourt le répertoire d'export avec `WalkDir`, sans suivre les liens symboliques, et indexe les fichiers `.md`.
- `purge_enabled` vaut `delete_after_export && skip_existing` : on ne supprime jamais sans pouvoir reconnaître ce qui a déjà été traité.
- `select` mémorise le `uid_validity` du dossier.

### Le piège de la recherche

> **Attention** : `search("ALL")` renvoie des **numéros de séquence**, non des UID. Toute commande suivante qui attend des UID doit d'abord les récupérer. Mélanger les deux désigne un autre message.

### Préfiltre et récupération

Le préfiltre interroge les en-têtes seuls, par fenêtres de `HEADER_UID_BATCH` (500) messages. Sur Gmail, il demande aussi `X-GM-MSGID`. Il écarte les messages déjà traités sans télécharger leur corps ; si le préfiltre échoue pour une fenêtre, celle-ci est conservée entière plutôt que perdue.

Les corps arrivent par lots de `BODY_FETCH_BATCH` (10), avec la requête `UID RFC822 BODY.PEEK[HEADER]` sous `with_retry`. En cas d'échec du lot, un repli récupère les messages un par un. Une erreur `imap::Error::Io` arrête le dossier.

### Décision pour chaque message

1. `build_global_source_proof` construit la preuve de source ; elle peut être absente, le message est alors compté dans `proofless`.
2. Si le Message-ID figure dans le registre, le message est `SeenHandled` : il n'est pas réexporté, mais il peut être purgé.
3. Sinon, `export_to_markdown` écrit la note.
4. Une erreur `MailParseError` compte le message comme ignoré et malformé ; en mode débogage, l'`.eml` brut est copié dans `_failed/`.

### La purge des messages traités

Quand la suppression est active, un message déjà traité n'est supprimé que si `match_seen(seen, &current, is_gmail)` renvoie `PurgeMatch::Purge`. La chaîne est `purge_handled`, puis `delete_discarded_messages`, puis `deletion_log::record_*`. Une cible n'est purgée qu'une fois par exécution (`purge_targeted`), et un échec de suppression n'est jamais fatal pour l'export.

> **Renvoi** : le registre, `match_seen` et les garde-fous de suppression sont décrits au chapitre 6.

## Les statistiques

`ExportStats` compte ce qui s'est passé :

| Champ | Signification |
| --- | --- |
| `exported` | notes écrites |
| `skipped` | messages ignorés (déjà exportés ou illisibles) |
| `handled` | messages déjà traités d'après le registre |
| `malformed` | messages dont l'analyse a échoué |
| `errors` | erreurs d'export |
| `purged` | messages supprimés du serveur |
| `legacy_handled` | traités sans preuve de source |
| `stale_proof` | preuve périmée (UIDVALIDITY changé) |
| `held_on_server` | messages conservés car une image n'a pas pu être gardée |

`counts_summary` et `purge_summary` en tirent les phrases françaises affichées à l'utilisateur.

## Reconnaître un message déjà exporté

`skip_reason_from_headers` renvoie `Option<SkipReason>` avec deux variantes, `Handled(id)` et `Exported`. Elle consulte d'abord le registre, puis un index approximatif des notes déjà présentes : un fichier correspondant au motif `email_<date>_<expéditeur>[_.]*`, dont le `subject_hash` et le Message-ID enregistré sont égaux à ceux du message. Cette double vérification évite de confondre deux messages du même jour et du même expéditeur.

## Fabriquer une note : `export_to_markdown`

La fonction suit un ordre précis, qui compte parce que chaque étape suppose les précédentes.

1. Analyse des en-têtes ; la date est mise au format `%Y-%m-%d`, ou `unknown-date`.
2. `subject_hash` : six caractères du MD5 du sujet, ou `no-subject`.
3. Abandon si la note existe déjà.
4. `analyze_email_type` (`EmailType` : direct, group, newsletter, mailing_list, unknown) et collecte des contacts.
5. Création du répertoire.
6. Nom du fichier : `email_<date>_<expéditeur>_<sujet>.md`. Le nom court de l'expéditeur compte jusqu'à huit lettres (`JohnDoe`, ou `UNK`) ; en cas de collision, le suffixe est `_2`, `_3`…
7. Extraction du corps (chapitre 4).
8. `limit_quote_depth` si `quote_depth > 0`.
9. `extract_attachments` selon l'`AttachmentPolicy` ; chaque pièce jointe est écrite à côté de la note sous `<date>_<nom>`.
10. `rewrite_cid_images`, puis `link_image_markers`.
11. `server_hold_reason` : décide si le message doit rester sur le serveur.
12. `normalize_line_breaks`, puis `cleaner::clean`.
13. Construction de `EmailFrontmatter`, du bloc `email` et des sections de médias.
14. `write_note`.
15. `route_email` produit une `RouteDecision` ; la note reste dans la zone d'attente.

> **Attention** : si le message doit être supprimé du serveur et qu'une pièce jointe est illisible, l'export échoue pour ce message au lieu de continuer. Une suppression ne doit jamais perdre une pièce jointe.

### La quarantaine des images

Quand `skip_signature_images` retire une image de signature, elle n'est pas supprimée d'office. Elle est déposée sous `<répertoire d'export>/_skipped/<nom de la note>/` si la copie du serveur peut être supprimée ; l'image reste ainsi récupérable. Sinon elle est simplement omise.
