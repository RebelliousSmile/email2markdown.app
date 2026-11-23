# Icône de notification et mise à *jour*

La fonctionnalité `tray` ajoute une interface graphique : une icône dans la zone de notification, des fenêtres web légères et un mécanisme de mise à jour automatique. Ce chapitre explique comment ces couches s'articulent, et les précautions prises pour qu'un fil de travail n'écrase jamais l'état de l'interface.

## Organisation du code

| Fichier | Taille approximative | Rôle |
| --- | --- | --- |
| `src/tray/mod.rs` | 820 lignes | commandes `AppCommand`, états de fenêtres, `run_tray`, `run_contextual` |
| `src/tray/event_loop.rs` | 690 lignes | boucle d'événements, registre des fenêtres |
| `src/tray/menu.rs` | 350 lignes | menu de l'icône et aiguillage des clics |
| `src/tray/windows.rs` | 1300 lignes | construction des fenêtres et de leurs vues web |
| `src/tray/ipc.rs` | 2900 lignes | messages venus des pages web |
| `src/tray_actions.rs` | 1400 lignes | actions exécutées en arrière-plan |
| `src/updater.rs` | 540 lignes | recherche et installation des mises à jour |
| `src/progress.rs` | 26 lignes | `ProgressUpdate` |

La règle de conception est simple : la boucle d'événements `tao` possède le fil principal et toutes les fenêtres. Les actions longues s'exécutent dans des fils séparés et lui parlent en lui envoyant des `AppCommand`.

## La boucle d'événements

`event_loop::run()` construit un `EventLoopBuilder::<AppCommand>::with_user_event()` et fonctionne en `ControlFlow::Wait`. L'`EventLoopProxy` est rangé dans un `static APP_PROXY: OnceLock<…>` ; `send_command(cmd)` s'en sert et renvoie une erreur si la boucle n'est pas initialisée.

Au démarrage :

- deux fils relais convertissent les événements de menu (`MenuEvent::receiver()`) et les résultats d'action (`ActionResult`) en `AppCommand::MenuEventReceived` et `AppCommand::ActionResultReceived` ;
- un fil d'arrière-plan appelle `updater::check_update` au lancement, puis toutes les `UPDATE_CHECK_INTERVAL` (12 h), jusqu'à trouver une version, qu'il signale par `UpdateAvailable` ;
- l'icône est créée à l'événement `StartCause::Init` par `menu::create_tray_icon`.

### Les commandes

`AppCommand` est une énumération volumineuse. Ses familles :

| Famille | Exemples |
| --- | --- |
| Ouverture de fenêtre | `OpenProgress`, `OpenConfig`, `OpenUpdate`, `OpenRouteReview`, `OpenDestGui` |
| Mise à jour de l'affichage | `ProgressUpdate`, `PushDestState`, `EvalScript`, `SetTooltip`, `RefreshMenu`, `CloseWindow` |
| Réactions de l'utilisateur | `DoneActionRequested`, `ErrorActionRequested`, `PersistRoutingRule`, `BrowseDestFolder` |
| Export contextuel | `ContextualSearchRequested/Finished`, `ContextualConvertRequested/Finished`, `ContextualRetryDeletionRequested/Finished` |
| Relais | `MenuEventReceived`, `ActionResultReceived` |

### Les fenêtres

Chaque fenêtre est décrite par un état propre (`ProgressState`, `ConfigState`, `UpdateState`, `RouteState`, `DestGuiState`, `ContextualState`), regroupés dans l'énumération `WState` et retrouvés par `WindowId`. Les champs sont déclarés dans l'ordre de destruction voulu : d'abord les rappels, puis la `WebView`, puis la `Window`, parce que la vue web doit libérer WebView2 avant que la fenêtre native disparaisse.

`WindowKind` (`Contextual`, `Progress`, `Config`, `Update`, `DestGui`, `Route`) et le registre `WINDOW_REGISTRY` implémentent les fenêtres à instance unique : `try_claim_window` refuse d'en ouvrir une seconde du même genre, et `close_window_kind` libère la place.

## Les fenêtres web

Chaque fenêtre charge un document HTML embarqué dans le binaire par `include_str!("../../assets/…")` : `contextual_export.html`, `progress_window.html`, `config_window.html`, `update_window.html`, `destinations_window.html` et `route_review.html`. Les constructeurs sont `build_contextual_window`, `build_progress_window`, `build_config_window`, `build_update_window`, `build_dest_gui_window` et `build_route_window`.

Une fenêtre se charge une fois depuis une chaîne HTML et ne navigue plus ensuite. `allow_navigation` applique cette règle.

`fn allow_navigation(url: &str, initial_load_seen: &AtomicBool) -> bool {`\
`    if url == "about:blank" {`\
`        return true;`\
`    }`\
`    !initial_load_seen.swap(true, Ordering::SeqCst)`\
`}`

> **À retenir** : la première navigation autre que `about:blank` est le chargement initial et passe ; toute autre est refusée, et seul le schéma est journalisé, car l'adresse refusée peut contenir des données de la page.

Pour injecter des données JSON dans une page, `escape_json_for_script` remplace `<`, `>` et `&` par `<`, `>` et `&`, ce qui empêche une donnée de fermer la balise `script`. Des tests vérifient aussi que les fichiers HTML embarqués existent et sont cohérents.

## Les messages venus des pages

`ipc.rs` reçoit les messages JSON des pages. Ses gestionnaires principaux sont `handle_dest_gui_ipc`, `handle_config_ipc`, `handle_dest_stats`, `handle_scan`, `browse_dest_folder`, et `background_config_job` pour les tâches longues de la configuration.

### La revue des destinations

Après un export, la fenêtre de revue (`OpenRouteReview`) présente la destination proposée pour chaque note. L'utilisateur peut la corriger, puis valider. Les lignes arrivent sous la forme `{decisions:[{file,dest_path}]}`, analysées par `parse_route_decisions` en `RouteDecisionRow`.

`apply_route_decisions(rows, notes_dir, allowed, layout)` procède en deux passes :

1. **Validation seule** : le fichier doit appartenir à l'ensemble `allowed`, la destination n'être pas vide, et `dated_dest_path` puis `join_safe_segments` doivent l'accepter. Rien n'est encore déplacé.
2. **Déplacement** : les notes sont déplacées une à une ; l'échec d'une ligne n'interrompt pas les suivantes.

Le résultat est un `RouteApplyReport { moved, missing, failed, deletions, held, routed }`. Chaque note déplacée dont la preuve est complète produit une `DeletionRequest` ; une note portant `keep_on_server` est comptée dans `held`. Les suppressions partent ensuite dans un fil d'arrière-plan (`spawn_server_deletions`).

`delete_staged_emails` supprime des notes en attente par `route::delete_email` et refuse tout fichier hors de `allowed`.

## Les actions

`tray_actions.rs` contient la logique, séparée de l'affichage. Le type `ActionResult` vaut `Success(titre, message)` ou `Error(String)`.

> **Attention** : une action de l'icône convertit toute erreur en `ActionResult::Error(String)` et ne la propage jamais jusqu'à la boucle d'événements. C'est une règle de la politique d'erreurs du projet : l'utilisateur voit le message, et la boucle d'événements n'a jamais à gérer un échec d'action.

### L'export depuis l'icône

`action_export(account, result_sender)` ouvre la fenêtre de progression (`OpenProgress`) avec un jeton d'annulation (`Arc<AtomicBool>`), un avertissement si `delete_after_export` est actif et un bouton « Continuer » pour passer au lot suivant. Le travail s'exécute dans un fil protégé par `catch_unwind` : une panique devient un `ProgressUpdate::Error` visible plutôt qu'une fenêtre figée.

`run_export` renvoie le résumé, les décisions de routage et un indicateur `has_next_batch`. Le résumé commence par « Export annulé », « Lot terminé » ou « Export terminé ». `classify_error` associe certaines erreurs au bouton « Ouvrir la configuration ». À la fin, si des décisions existent, `OpenRouteReview` s'ouvre ; les fichiers restent dans la zone d'attente jusqu'à la validation.

`ProgressUpdate` (module `progress`) a six variantes : `Step { message, current, total }`, `Indeterminate`, `Done { summary, action_label }`, `Error { message, action_label }`, `StatusLine` et `AutoClose`.

### Les autres actions

| Action | Rôle |
| --- | --- |
| `action_resume_sort` | reprend le tri des notes restées en attente |
| `action_choose_export_dir`, `action_choose_notes_dir` | choisit les répertoires (`set_export_dir_in`, `set_notes_dir_in`) |
| `action_fix_html` | corrige les corps HTML en Markdown dans le répertoire d'export d'un compte (`run_fix_html`) |
| `action_clean_server` | nettoyage du serveur (chapitre 6) |
| `action_open_config`, `action_open_documentation` | ouvre la configuration ou la documentation |

## Le menu

`menu.rs` définit les identifiants dans un module privé `menu_ids`, et `handle_menu_event(id, result_sender)` aiguille les clics par identifiant exact ou par préfixe (`export_`, `fixhtml_`, `resume_sort_`, `clean_server_`) suivi du nom du compte.

Le menu comprend un sous-menu « Export compte », « Reprendre le tri », « Nettoyer le serveur » (seulement si un compte a `delete_after_export`), un sous-menu « Outils » et « Quitter ». Quand un compte est illisible, un élément « Comptes illisibles — voir Paramètres » remplace la liste. `AVAILABLE_UPDATE` mémorise la version trouvée pour l'afficher dans le menu et l'info-bulle.

## L'instance unique et le lancement

`run_tray(after_update)` prend un verrou exclusif (`fs2::try_lock_exclusive`) sur `tray.lock` du répertoire de configuration, par `acquire_single_instance_lock`.

- Sans le verrou, une boîte `rfd` indique que le logiciel est déjà présent dans la zone de notification, puis la fonction retourne.
- Avec `--after-update`, `acquire_with_retry` réessaie `LOCK_ATTEMPTS_AFTER_UPDATE` (50) fois, toutes les `LOCK_RETRY_DELAY` (200 ms), le temps que l'ancien processus se termine.
- Une erreur de verrouillage n'empêche jamais le démarrage ; elle produit un avertissement.

`run_contextual(dossier)` ouvre sa propre boucle d'événements pour l'export déclenché depuis l'explorateur. Il prépare le lancement par `prepare_contextual_launch`. Si l'erreur est `ContextualDestinationError::MissingAddressRule` (détectée par `downcast_ref`), il ouvre la gestion des destinations en mode configuration ; toute autre erreur affiche une boîte `rfd`. Les travaux de recherche et de conversion tournent dans des fils et renvoient des événements `…Finished` ; la fenêtre se ferme quand `summary.complete()` est vrai.

## La mise à jour automatique

`updater.rs` interroge l'API GitHub des versions publiées du dépôt par `ureq`.

### Choisir une version

`check_update(current)` appelle `select_release`, qui retire le préfixe `v` de l'étiquette, analyse les deux versions avec `semver::Version::parse`, et renvoie `None` si `remote <= current`. Une version dont le fichier `SHA256SUMS` ou la ressource de la plateforme manque est une erreur. Ensuite `fetch_expected_checksum` récupère la somme attendue : on n'offre jamais une mise à jour qu'on ne pourrait pas vérifier.

`release_asset_matches` repère la ressource par son suffixe : `windows-x86_64.exe`, `macos-native` ou `linux-x86_64`. `checksum_for` lit le format de `sha256sum`, avec ou sans l'étoile du mode binaire.

### Installer

`download_and_apply(release, on_progress)` appelle `install_download` :

1. le flux est copié dans un `NamedTempFile::new_in(temp_dir)`, borné par `take(max + 1)` avec `MAX_ASSET_BYTES` (100 Mio) ;
2. la taille est contrôlée ;
3. la somme SHA-256 est vérifiée **avant** `tmp.keep()` ;
4. `replace` appelle `self_replace::self_replace` sur le chemin persistant.

`let (_, tmp_path) = tmp.keep()?;          // persist before self_replace`\
`self_replace::self_replace(&tmp_path)?;`

> **Attention** : sous Windows, la destruction d'un `NamedTempFile` supprime le fichier, et `self_replace` ouvre le remplaçant par son chemin. Appelez toujours `keep()` avant `self_replace`, sinon la copie échoue sans bruit. Cette règle figure aussi dans `.claude/rules`.

Sous Windows, la version affichée par le programme de désinstallation (clé `DisplayVersion`, créée par Inno Setup sous `HKCU`) est mise à jour ; un échec n'est que signalé sur la sortie d'erreur.

`spawn_tray_after_update(exe)` relance `tray --after-update` avec des flux nuls et, sous Windows, `CREATE_NO_WINDOW`. Le chemin `exe` doit être capturé avant `self_replace`.

Les délais réseau sont `METADATA_TIMEOUT` (15 s), `DOWNLOAD_CONNECT_TIMEOUT` (15 s) et `DOWNLOAD_READ_TIMEOUT` (30 s). Des tests couvrent la sélection (`test_select_release_*`), la somme (`test_checksum_for_*`, `test_verify_checksum_*`) et l'installation : une somme fausse ou un flux trop long ne remplace jamais le binaire.
