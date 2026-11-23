# Routage et *destinations*

Une note exportée reste d'abord dans une zone d'attente. Le routage décide ensuite, d'après des règles, dans quel dossier de `notes_dir` elle sera classée. Ce chapitre décrit le fichier de destinations, l'algorithme de choix, le déplacement sécurisé des notes et la sous-commande `dest`.

## Le fichier de destinations

Le fichier se trouve par `destinations_path_from(settings_file)` : c'est `destinations_file` de `settings.yaml` s'il est défini, sinon `destinations.yaml` à côté de `settings.yaml`. Un chemin en `.txt` est normalisé en `.yaml`, et `destinations::migrate_from_txt` convertit une seule fois l'ancien format texte.

`destinations:`\
`  - path: Perso/Banque`\
`    note: relevés et factures bancaires`\
`    rules:`\
`      - domain: ubs.ch`\
`      - subject: facture`\
`  - path: Perso/Messy/Emails`\
`    default: true`

Le type `DestinationsConfig` contient des `DestinationEntry { path, note, rules, default }` ; chaque règle est une `DestinationRule`. Les fonctions de `destinations.rs` modifient ce fichier : `load_yaml` (qui refuse un lien symbolique), `save_yaml`, `upsert_entry`, `remove_entry`, `set_default`, `set_note`, `add_rule`, `remove_rule` et `reorder_destinations`.

> **Attention** : plusieurs commentaires de documentation de `route.rs` et de `tray/ipc.rs` parlent encore de `destinations.txt`. Le code lit `destinations.yaml` ; le format texte ne subsiste que pour la migration, et `route::parse_destinations` n'alimente qu'elle.

`load_destinations` ne plante jamais : un fichier absent ou mal formé donne une liste vide et un avertissement.

### Deux types de règles

Il existe deux représentations d'une règle, volontairement séparées.

| Type | Module | Rôle |
| --- | --- | --- |
| `DestinationRule` | `destinations` | forme sérialisée dans le YAML |
| `MatchRule` | `route` | forme utilisée à l'évaluation |

Les deux ont les mêmes cinq variantes : `Domain`, `From`, `Correspondent`, `Subject` et `Account`. Les implémentations `From<MatchRule> for DestinationRule` et `From<&DestinationRule> for MatchRule` les convertissent ; `MatchRule::parse(kind, valeur)` construit une règle à partir de son nom.

## Évaluer une règle

`rule_matches(meta, rule)` compare les métadonnées d'un message, `EmailMeta { from, to, cc, bcc, domain, subject, account, date }`, à une règle.

`match rule {`\
`    MatchRule::Domain(d) => {`\
`        let meta_domain = meta.domain.to_lowercase();`\
`        let rule_domain = d.to_lowercase();`\
`        meta_domain == rule_domain || meta_domain.ends_with(&format!(".{}", rule_domain))`\
`    }`\
`    MatchRule::From(a) => meta.from.eq_ignore_ascii_case(a),`\
`    MatchRule::Correspondent(a) => email_has_correspondent(meta, a),`\
`    MatchRule::Subject(k) => meta.subject.to_lowercase().contains(&k.to_lowercase()),`\
`    MatchRule::Account(a) => meta.account == *a,`\
`}`

| Règle | Comparaison |
| --- | --- |
| `Domain` | domaine exact, ou sous-domaine (`.acme.com` mais pas `notacme.com`) |
| `From` | adresse de l'expéditeur, sans tenir compte de la casse |
| `Correspondent` | adresse présente dans `From`, `To`, `Cc` ou `Bcc` |
| `Subject` | sous-chaîne du sujet, sans tenir compte de la casse |
| `Account` | nom du compte, **avec** la casse |

Le sujet utilise `contains` et non une expression régulière : la valeur est dynamique, et compiler une expression à chaque appel est précisément ce que la règle `LazyLock` du projet interdit.

## Choisir une destination

`route_email(meta, dests, layout)` renvoie une `RouteDecision { rel_path, matched_rule, is_default, alternatives }`.

1. La première destination dont une règle correspond, **dans l'ordre du fichier**, l'emporte ; les autres destinations qui correspondent vont dans `alternatives`.
2. Sinon, la destination marquée `default`.
3. Sinon, la constante `DEFAULT_BASE`, `Perso/Messy/Emails`.
4. Les dossiers de date sont ajoutés par `layout.apply`.

> **À retenir** : l'ordre des destinations dans le fichier est significatif, puisque la première correspondance gagne. `reorder_destinations` et la fenêtre de gestion existent pour le modifier.

Les décisions d'un lot sont collectées dans un `RouteDecisions = Vec<(PathBuf, RouteDecision)>`. Une fonction `ai_route` existe pour un routage assisté, mais c'est aujourd'hui un squelette qui renvoie `None` : le réglage `ai_routing_enabled` ne produit encore aucun effet.

### Cohérence des règles

`same_rule`, `rule_conflicts`, `remove_rule_elsewhere` et `upsert_rule_exclusive` empêchent qu'une même condition figure dans deux destinations. Ajouter une règle exclusive la retire des autres destinations.

## Déplacer une note

`apply_decision(staging_path, rel_path, notes_dir)` enchaîne `move_decision` puis `link_routed_notes`.

### Le chemin cible

`join_safe_segments(root, dest)` construit le chemin final et rejette tout ce qui pourrait en sortir. Chaque segment doit satisfaire `SAFE_PATH_SEGMENT_RE`, `^[A-Za-z0-9À-ſ _.\-]+$`, et la fonction refuse aussi `\`, `.` et `..`, un point final et les noms de périphériques réservés de Windows.

> **Attention** : un chemin de destination vient d'un fichier modifiable et d'une fenêtre : traitez-le comme une entrée non fiable. Tout nouveau code qui compose un chemin à partir d'une destination passe par `join_safe_segments`.

### `move_email`

`move_email` déplace la note et ses pièces jointes :

- elle refuse un lien symbolique ;
- elle lit la liste des pièces jointes dans l'en-tête de la note et les déplace avec elle, en ajoutant `_2` en cas de collision ; un lien qui sortirait du dossier est ignoré ;
- elle déplace le fichier `.md`, puis `rewrite_attachment_paths` met à jour les chemins ;
- le déplacement utilise `fs::rename`, avec une copie suivie d'une suppression quand les deux emplacements sont sur des volumes différents.

`delete_email(md)` supprime la note et range ses pièces jointes dans `<dossier parent>/_deleted/`, au lieu de les effacer.

### Liens entre notes

`link_routed_notes` appelle `quote_link::link_note` pour remplacer les citations des notes tout juste classées. Son échec n'annule jamais le tri. L'index des notes est conservé dans un `QUOTE_INDEX` global (un `Mutex`) avec une durée de vie `QUOTE_INDEX_TTL` de 300 s ; `warm_quote_index` le prépare dans un fil d'arrière-plan.

### Notes en attente et anciens formats

- `scan_staged_decisions` (côté `tray_actions`) retrouve les notes en attente et en refait les décisions ; elle exclut `_deleted`, `_failed`, `_skipped` et `contacts`.
- `repair_legacy_attachments` répare les notes dont les pièces jointes étaient rangées dans l'ancien dossier centralisé.
- `contextual_relative_path` et `resolve_contextual_destination` servent l'export contextuel (chapitre 7) ; l'erreur typée `ContextualDestinationError::MissingAddressRule` signale qu'aucune règle d'adresse n'existe pour le dossier choisi.

## La sous-commande `dest`

`dest_cmd.rs` expose `DestArgs { command: Option<DestCommand> }`.

| Commande | Effet |
| --- | --- |
| `dest list` | affiche les destinations et leurs règles |
| `dest add <chemin> [--domain …] [--from …] [--correspondent …] [--subject …] [--account …] [--note …] [--default]` | ajoute ou complète une destination |
| `dest suggest` | parcourt le dossier par défaut et propose, de façon interactive, des règles de domaine |
| `dest` (sans sous-commande) | lance le mode interactif |

`run(args)` envoie à `interactive(&dest_file)` quand aucune sous-commande n'est donnée. `detect_anomalies` signale les incohérences du fichier, et `exceeds_max_depth` compare la profondeur d'une destination à `destination_max_depth`. `format_entry` fabrique l'affichage d'une entrée.

> **Bon à savoir** : `dest_cmd.rs` construit lui-même des `DestinationRule` à deux endroits, dans la sous-commande `add` et dans l'analyseur interactif. Ajouter un type de règle oblige à toucher ces deux endroits, entre autres (chapitre 9).
