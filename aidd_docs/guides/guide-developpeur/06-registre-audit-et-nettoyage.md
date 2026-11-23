# Registre, audit et *nettoyage*

Supprimer un message du serveur est la seule opération irréversible de l'application. Ce chapitre décrit les garde-fous : la preuve de source gravée dans chaque note, le registre des messages traités, le contrôle avant suppression, le journal des suppressions, puis l'audit et le nettoyage du serveur.

## La preuve de source

Chaque note exportée porte dans son en-tête YAML une `LocalSourceProof`, définie dans `contextual_export.rs`. Elle réunit deux parties.

| Type | Champs | Rôle |
| --- | --- | --- |
| `SourceIdentity` | `account`, `message_id` (optionnel), `header_fingerprint` (MD5 des en-têtes), `provider_id` (optionnel) | dire de quel message il s'agit |
| `MessageLocation` | `folder_raw`, `folder_display`, `uid_validity`, `uid` | dire où il se trouve |

`provider_id` est l'identifiant propre au fournisseur (`X-GM-MSGID` pour Gmail). `proof_is_complete` décide si une preuve suffit à autoriser une suppression ; `identity_matches` compare une identité stockée à une identité relue sur le serveur ; `proof_target_key` donne la clé qui désigne une cible unique, de façon que plusieurs libellés Gmail d'un même message ne comptent que pour une cible.

## Le registre `handled`

`handled_messages.txt` est un fichier texte en ajout seul. Une ligne a le format `compte<TAB>message_id[<TAB>preuve]`. La preuve est la `LocalSourceProof` sérialisée en JSON sur une ligne ; la valeur littérale `held` (`HELD_MARK`) dans la troisième colonne marque un message conservé volontairement sur le serveur.

### Lecture

`load_from(path, account)` charge les Message-ID d'un compte. `load_proofs_from` renvoie un `LedgerProofs { proofs, legacy, held }`. En cas de conflit sur un même message, la précédence est : conservé (`held`), puis avec preuve, puis ancien (`legacy`). Une preuve illisible est traitée comme un enregistrement ancien.

`match_seen(seen, current, gmail)` compare un message vu sur le serveur (`SeenHandled { message_id, uid, provider_id }`) à ce que le registre sait de lui, et renvoie un `PurgeMatch` :

| Résultat | Signification |
| --- | --- |
| `Purge(preuve)` | l'emplacement ou l'identifiant fournisseur concorde : suppression autorisée |
| `Legacy` | traité mais sans preuve exploitable : aucune suppression |
| `Held` | conservé volontairement : aucune suppression |
| `Stale` | la preuve ne correspond plus (UIDVALIDITY ou identifiant changé) : aucune suppression |

Hors Gmail, la concordance exige que l'emplacement stocké soit celui du message courant. Sur Gmail, elle exige le même `provider_id`.

### Écriture

Les écrivains ajoutent des lignes en fin de fichier et ne modifient jamais les lignes existantes.

- `record_to` écrit une ligne, et dégrade une preuve incomplète en ligne ancienne ;
- `record_proofs(_to)` écrit des preuves, au mieux ;
- `record_held(_to)` écrit une ligne `held`.

`complete_from_notes_to(ledger, notes_dir, account)` donne une preuve aux entrées anciennes en parcourant les notes déjà triées (`email_*.md`) : elle ne suit pas les liens symboliques, saute les dossiers dont le nom commence par `.` ou `_`, et une note `held` ne fournit aucune preuve.

> **À retenir** : le registre n'est jamais réécrit. Un état « plus fort » n'efface pas l'ancien ; il est lu en priorité grâce à la règle de précédence.

## Supprimer un message

La suppression passe par `contextual_export::delete_validated`, qui renvoie un `DeletionOutcome` par message : `Deleted`, `AlreadyAbsent`, `RetryRequired`, `StaleUidValidity` ou `Rejected`.

1. **Préparation** : `evaluate_deletion_preflight` examine la note et sa preuve et renvoie un `DeletionPreflight` ; une `DeletionRequest { markdown, proof }` décrit la demande.
2. **Contrôle** : le dossier est sélectionné, l'`UIDVALIDITY` doit être celui de la preuve, puis l'identité du message est relue et comparée.
3. **Suppression générique** : `UID STORE … \Deleted` puis `UID EXPUNGE` sur ce seul UID.
4. **Suppression Gmail** : déplacement vers la Corbeille (`MOVE`), puis suppression dans la Corbeille.

> **Attention** : n'utilisez jamais un `EXPUNGE` global. Il supprimerait aussi tout message marqué par un autre client. Seul `UID EXPUNGE` sur l'UID validé est admis.

Un changement d'`UIDVALIDITY` signifie que les UID ont été renumérotés : le résultat est `StaleUidValidity` et rien n'est supprimé.

## Le journal `deletions.log`

`deletion_log.rs` écrit une ligne par suppression dans `deletions.log`, séparée par des tabulations : date (RFC 3339, à la seconde), compte, dossier affiché, UID, Message-ID (ou `-`), résultat.

| Résultat consigné | Sens |
| --- | --- |
| `deleted` | supprimé |
| `already-absent` | déjà absent du serveur |
| `retry-required: …` | à retenter, avec la cause |
| `stale-uidvalidity: expected X, found Y` | dossier renuméroté (`Y` peut valoir `none`) |
| `rejected: …` | refusé, avec la cause |
| `error: …` | erreur de lot |

Les fonctions `record_results(_to)` et `record_error(_to)` travaillent au mieux : un échec d'écriture du journal ne bloque jamais une suppression. Le journal existe parce que les suppressions s'exécutent dans des fils de l'icône, où `eprintln!` n'atteint personne.

## L'audit du serveur

`audit-server` est un recensement en lecture seule : rien n'est écrit, ni en local ni sur le serveur. Pour chaque message, `classify` produit un `ServerStatus`.

| Statut | Libellé affiché |
| --- | --- |
| `HandledWithProof` | traité, avec preuve |
| `HandledLegacy` | traité, sans preuve |
| `Held` | retenu, image non conservée |
| `Staged` | note en attente de tri |
| `Unknown` | jamais exporté |

`classify` s'appuie sur `skip_reason_from_headers`. `confirm_proof` rétrograde `HandledWithProof` en `HandledLegacy` si `match_seen` ne répond pas `Purge`, et `mark_held` applique le statut `Held`. Chaque `AuditEntry` (`folder`, `uid`, `provider_id`, `message_id`, `date`, `from`, `subject`, `status`, `proof`) est construite par `AuditEntry::from_headers`. `render(account, entries, details)` affiche les totaux par statut et par dossier, puis une ligne par message avec `--details`.

Avec `--uid`, la commande compare l'identité d'un message calculée depuis le message complet et depuis son seul bloc d'en-têtes (`compare_identity`), ce qui sert à diagnostiquer une preuve qui ne concorde pas.

## Le nettoyage du serveur

Le menu « Nettoyer le serveur » supprime les messages que le registre connaît sans preuve exploitable, après confirmation. Il n'apparaît que pour les comptes dont `delete_after_export` est actif.

`clean_server_in(paths, account_name, confirm, on_step)` suit ces étapes :

1. charge `.env` et la configuration ;
2. recense le serveur (fonction privée `census`) ;
3. si des entrées anciennes existent et que `notes_dir` est défini, appelle `complete_from_notes_to` pour leur trouver une preuve, puis refait le recensement ;
4. construit un `CleanupPlan { targets, untargetable, proved }` par `plan(entries)` ;
5. demande confirmation par le rappel `confirm(account, count, is_gmail)` ;
6. se reconnecte et appelle `delete_discarded_messages(&plan.targets)` ;
7. enregistre les résultats, et en cas d'erreur appelle `deletion_log::record_error_to`.

`record_results(paths, results, notes)` consigne chaque résultat, ne note dans le registre que `Deleted` et `AlreadyAbsent`, puis renvoie un résumé de la forme « N supprimé(s), N déjà absent(s), N échec(s) ». `cleanup_account` refuse un compte inconnu, un compte sans `delete_after_export` ou sans mot de passe, avec des messages en français.

Tout ce qui n'est pas un échange IMAP est testable sans serveur, parce que les chemins arrivent par `AppPaths`.

> **Attention** : un seul nettoyage tourne à la fois, car deux exécutions concurrentes enregistreraient deux fois les mêmes preuves. Le drapeau `CLEANING_SERVER` (un `AtomicBool`) de `tray_actions.rs` porte cette garde.

`if CLEANING_SERVER.swap(true, Ordering::SeqCst) {`\
`    let _ = result_sender.send(ActionResult::Error(`\
`        "Nettoyer le serveur : un nettoyage est déjà en cours".to_string(),`\
`    ));`\
`    return;`\
`}`

### La confirmation

`confirm_deletion` présente une question Oui/Non avec « Non » par défaut. Sous Windows, c'est un `MessageBoxW` placé au premier plan (`MB_TOPMOST`, `MB_SETFOREGROUND`), car une boîte ordinaire s'ouvrirait derrière les autres fenêtres. Ailleurs, `rfd` ne permet pas de choisir le bouton par défaut ; le bouton qui supprime porte donc l'intitulé « Supprimer définitivement ». Le texte de `clean_confirmation_text` précise que l'appariement ne repose que sur le Message-ID et que, sur Gmail, la suppression passe par la Corbeille puis est définitive.

## D'où part une demande de suppression

> **Bon à savoir** : après un tri depuis la fenêtre de revue (chapitre 7), les suppressions sont demandées par `apply_route_decisions`. Seules les notes dont la preuve est complète produisent une `DeletionRequest` ; une note qui porte `keep_on_server` est comptée comme conservée.
