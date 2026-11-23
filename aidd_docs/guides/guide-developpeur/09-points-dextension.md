# Points d'*extension*

Ce chapitre indique, pour six évolutions courantes, les endroits exacts à modifier. Chaque liste a été relevée dans le code : elle montre aussi où la même information est répétée, ce qui est le meilleur indicateur du coût réel d'un changement.

## Ajouter une sous-commande

Une sous-commande se déclare à deux endroits de `src/main.rs`.

1. Ajoutez une variante à `enum Commands`, avec des commentaires `///` sur la variante et sur chaque champ : `clap` en fait l'aide en ligne.
2. Ajoutez le bras correspondant dans le `match cli.command` de `main`, et déléguez à une fonction de la bibliothèque. Gardez `main.rs` mince.

`/// Read-only census of the messages still on the server of one account.`\
`AuditServer {`\
`    /// Account name, as in accounts.yaml`\
`    account: String,`

`    /// Compare the identity of one message read from RFC822 and from its`\
`    /// header block (requires --folder)`\
`    #[arg(long, requires = "folder")]`\
`    uid: Option<u32>,`\
`},`

Quelques règles de la politique `clap` du projet :

- `#[arg(short, long)]` pour un indicateur à deux formes, `#[arg(long)]` pour un drapeau rare ;
- une valeur par défaut se déclare par `#[arg(default_value = "…")]` ;
- une opération destructrice est en simulation par défaut et exige `--apply`, comme `link-quotes` ; `--apply` l'emporte sur `--dry-run` si les deux sont passés ;
- une dépendance entre options s'exprime par `requires = "…"`, comme `--uid` qui exige `--folder`.

> **Attention** : une sous-commande qui n'a de sens qu'avec l'interface graphique se déclare derrière `#[cfg(feature = "tray")]`, comme `Tray`. `Contextual` est l'exception : elle existe toujours et renvoie un message explicite sans la fonctionnalité.

Si la commande échange avec le serveur, passez par `ImapExporter` et, pour l'accès aux fichiers, par un `AppPaths` : la fonction devient alors testable sans toucher à la configuration de l'utilisateur.

## Ajouter une étape de nettoyage du corps

Les étapes vivent dans `cleaner::clean`, fonction de six blocs numérotés. Pour en ajouter une :

1. écrivez une fonction `fn ma_etape(s: &str) -> String` dans `src/cleaner.rs` ;
2. si elle utilise une expression régulière, déclarez-la en `static MON_RE: LazyLock<Regex>` au niveau du module, près de la fonction ; ne compilez jamais une expression dans le corps d'une fonction appelée par ligne ou par message ;
3. insérez l'appel dans `clean`, au bon endroit ;
4. ajoutez un test de bout en bout sur un message réaliste, avec assertions inclusives et exclusives, dans `mod cleaner_tests` de `tests/rust_tests.rs`.

`let body = unwrap_lines(&body);`

`// 5. Inline link extraction → numbered refs, then tracker decontamination`\
`let body = extract_links(&body);`\
`let body = decontaminate_ref_urls(&body);`

> **À retenir** : l'ordre se décide par un test réaliste, pas par le plan. Les étapes 2 et 3 ont été inversées par rapport au plan initial parce que seule l'exécution sur des liens sociaux empilés a révélé l'interaction. Si votre étape modifie les sauts de ligne ou les parenthèses, vérifiez-la avant et après `extract_social_footer` et `reattach_urls`, puis documentez l'écart en commentaire à côté de l'appel.

## Ajouter un réglage de comportement

Prenons un réglage par compte, comme `quote_depth`. La règle de types impose `Option<T>` dans la configuration et une valeur concrète dans le runtime.

1. `AccountBehavior` (`config.rs`) : ajoutez le champ avec `#[serde(skip_serializing_if = "Option::is_none")]`.
2. `Account` : ajoutez le champ concret.
3. `merge_account` : résolvez-le avec la priorité habituelle, puis une valeur par défaut codée en dur.
4. **Le code qui l'utilise** : lisez le champ de `Account`, jamais de `Settings`.
5. **Les modèles** : documentez-le dans `config/settings.yaml.example`.
6. **La fenêtre de paramètres** : si le réglage doit être éditable, ajoutez-le dans les structures de `tray/ipc.rs` (là où `quote_depth` figure dans les données reçues, dans la fabrication des valeurs par défaut et dans l'écriture du comportement du compte) et dans `assets/config_window.html`.

`quote_depth: per`\
`    .and_then(|a| a.quote_depth)`\
`    .or(def.quote_depth)`\
`    .unwrap_or(1),`

> **Bon à savoir** : un champ qui doit être aussi un réglage global se place dans `Settings` et non dans `AccountBehavior`. Il n'a alors pas de niveau « par compte » ; un défaut non trivial se déclare par une fonction `default_<champ>` référencée par `#[serde(default = "…")]`.

Testez avec un `TempDir` : écrivez un `settings.yaml`, chargez par `Config::load_with_settings`, puis vérifiez la valeur résolue, par exemple dans `mod settings_tests`.

## Ajouter un type de règle de destination

C'est l'extension la plus coûteuse, parce que les cinq types de règles existants sont dupliqués. Une recherche de `MatchRule::Subject` et de `DestinationRule::Subject` dans `src/` donne les endroits à toucher.

| Fichier | Ce qu'il faut ajouter |
| --- | --- |
| `src/route.rs` | variante de `MatchRule`, branche de `MatchRule::parse`, branche de `rule_matches`, branche de `same_rule` |
| `src/destinations.rs` | variante de `DestinationRule` et les deux conversions `From` |
| `src/dest_cmd.rs` | option de `dest add`, analyseur interactif, affichage de `format_entry` |
| `src/contextual_export.rs` | évaluation d'une règle contre un message candidat de l'export contextuel (les règles de sujet et de compte y valent `false`) |
| `src/tray/ipc.rs` | conversion d'une règle reçue de la fenêtre de destinations |
| `src/tray/windows.rs`, `src/tray_actions.rs` | libellés et clés d'affichage de la règle |
| `src/tray/event_loop.rs`, `src/tray/mod.rs` | listes de types de règles acceptés pour la persistance depuis la fenêtre de revue |
| `assets/route_review.html`, `assets/destinations_window.html`, `assets/contextual_export.html` | sélecteurs et affichage des types de règles |

> **Attention** : la règle de refactorisation du projet demande de s'arrêter quand un changement touche la même logique dans trois fichiers ou plus, et de factoriser d'abord. Ici, l'information est répétée dans plus de huit fichiers. Avant d'ajouter un sixième type de règle, envisagez de regrouper la conversion entre nom, valeur et règle dans une seule fonction partagée, puis de remplacer les répétitions.

Quel que soit le choix, écrivez l'évaluation de la nouvelle règle dans `rule_matches` avec le même soin que les autres : les comparaisons d'adresses ou de domaines sont insensibles à la casse (`eq_ignore_ascii_case`), les comparaisons à base de sous-chaîne n'utilisent pas d'expression régulière dynamique, et la règle d'un compte reste sensible à la casse.

## Ajouter une destination ou une variante d'arborescence

### Une destination

Une destination n'est pas du code : c'est une entrée de `destinations.yaml`, que l'on crée par `dest add`, par la fenêtre de gestion ou à la main. Le seul réglage global est la profondeur maximale (`destination_max_depth`) ; une destination plus profonde déclenche l'avertissement de `exceeds_max_depth`.

Si vous créez du code qui compose un chemin de destination, passez-le par `join_safe_segments`, sans exception.

### Une variante de `DateLayout`

Ajoutez la variante à `DateLayout` (sérialisation `snake_case`), puis traitez-la dans `segments`, qui est la source de vérité des dossiers de date. `apply`, `ensure` et `lookup_dirs` en dérivent. N'oubliez pas :

- `MAX_DATE_SEGMENTS`, la constante du nombre maximal de segments (3 aujourd'hui) ;
- `date_suffix_len` et `strip_date_suffix`, qui reconnaissent les segments déjà présents dans un chemin ;
- le lien avec l'ancien booléen `organize_by_date` dans `Settings::date_layout()` et `set_date_layout` ;
- les tests de `date_layout.rs`.

## Ajouter une entrée au menu de l'icône

Les modifications se concentrent dans `src/tray/menu.rs`.

1. Déclarez une constante dans `menu_ids` : un identifiant fixe pour une action simple, un préfixe se terminant par `_` pour une action par compte.
2. Ajoutez l'élément dans `create_menu` avec `MenuItem::with_id(menu_ids::MON_ID, "Libellé", true, no_accel)`.
3. Ajoutez la branche dans `handle_menu_event` : une branche d'identifiant exact, ou `id if id.starts_with(menu_ids::MON_PREFIX)` suivie d'un `strip_prefix` pour récupérer le nom du compte.
4. Écrivez l'action dans `src/tray_actions.rs`, avec la signature `fn action_xxx(…, result_sender: Sender<ActionResult>)`.

`id if id.starts_with(menu_ids::EXPORT_PREFIX) => {`\
`    if let Some(account_name) = id.strip_prefix(menu_ids::EXPORT_PREFIX) {`\
`        tray_actions::action_export(account_name.to_string(), result_sender);`\
`    }`\
`}`

Pour l'action elle-même :

- lancez le travail dans `thread::spawn` ; ne bloquez jamais la boucle d'événements ;
- convertissez toute erreur en `ActionResult::Error(String)` ;
- si une opération ne doit pas tourner deux fois en parallèle, protégez-la par un `AtomicBool`, comme `CLEANING_SERVER` ;
- pour afficher une progression, envoyez `AppCommand::OpenProgress` et des `ProgressUpdate` ;
- enveloppez un travail long dans `catch_unwind` si une panique ne doit pas laisser une fenêtre figée.

> **Renvoi** : pour une nouvelle fenêtre, ajoutez un `WindowKind`, un état dans `WState`, un constructeur `build_…_window` dans `tray/windows.rs`, un document HTML dans `assets/` embarqué par `include_str!`, et les variantes d'`AppCommand` nécessaires (chapitre 7).

Une répétition mérite d'être signalée : `create_menu` construit à la main quatre sous-menus « un élément par compte » (export, reprise du tri, nettoyage, correction HTML) avec le même motif. Un cinquième est le bon moment pour en extraire une fonction.

## Vérifier avant de livrer

Quelle que soit l'extension, passez ces contrôles dans l'ordre :

1. `cargo clippy -- -D warnings` ;
2. `cargo test` et `cargo test --all-features` ;
3. pour une extension qui touche IMAP, les tests d'intégration avec le conteneur Dovecot ;
4. pour une extension qui touche la suppression côté serveur, relisez le chapitre 6 : une suppression ne doit jamais s'appuyer sur un `EXPUNGE` global ni sur une preuve incomplète.
