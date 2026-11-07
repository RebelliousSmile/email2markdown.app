---
objective: "La profondeur maximale des destinations et le motif de date des dossiers se règlent dans settings.yaml et la fenêtre de configuration, sans changement pour une installation existante."
status: in-progress
---

<!-- Fill or omit these sections; never add, rename, or reorder one. -->

# Plan: Structure de l'arbre des notes configurable

## Overview

| Field      | Value                                                                                                                                         |
| ---------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| **Goal**   | Rendre configurables la profondeur des destinations sous la branche (défaut 2) et le motif de date (aucun, `YYYY`, `YYYY/MM`, `YYYY/MM/DD`). |
| **Source** | Brainstorm validé en conversation le 2026-11-07 (texte).                                                                                       |

## Phases

| #   | Phase                                                  | File                         |
| --- | ------------------------------------------------------ | ---------------------------- |
| 1   | Centraliser le motif de date (refactor sans effet)     | [`phase-1.md`](./phase-1.md) |
| 2   | Motif de date configurable                             | [`phase-2.md`](./phase-2.md) |
| 3   | Profondeur maximale des destinations                   | [`phase-3.md`](./phase-3.md) |
| 4   | Fenêtre de configuration, textes neutres, documentation | [`phase-4.md`](./phase-4.md) |

## Decisions

| Decision | Why |
| -------- | --- |
| Nouveau module `src/date_layout.rs` qui porte l'enum `DateLayout { None, Year, YearMonth, YearMonthDay }` et toute la logique de date des chemins (construction, reconnaissance, suppression du suffixe). | Le motif `YYYY/MM` est aujourd'hui codé en dur dans 6 fichiers (`route.rs`, `contextual_export.rs`, `email_export.rs`, `tray_actions.rs`, `tray/ipc.rs`, `dest_cmd.rs`) : la règle « Rule of Three » impose de centraliser avant de paramétrer. |
| Le paramètre `organize_by_date: bool` est remplacé partout par `DateLayout`. | Le booléen garde la même valeur pour `None` et pour les trois motifs : le garder à côté d'un enum créerait deux sources de vérité au moment de l'exécution. |
| `settings.yaml` : nouvelle clé `date_layout` (optionnelle). Si elle est absente, la valeur est dérivée de `organize_by_date` : `true` donne `year_month`, `false` donne `none`. À l'enregistrement, les deux clés sont écrites de façon cohérente. | Un fichier existant avec `organize_by_date: false` garde son comportement. Une ancienne version de l'application relancée sur un fichier neuf lit toujours une valeur juste. |
| La reconnaissance d'un suffixe date accepte toujours `YYYY`, `YYYY/MM` et `YYYY/MM/DD`, quel que soit le motif configuré. | C'est un sur-ensemble de « tout préfixe du motif ». Il garantit qu'un passage de `YYYY/MM/DD` à `YYYY/MM`, ou à « aucun », ne fait disparaître aucune note existante des statistiques ni du scan. |
| `destination_max_depth` : `Option<usize>` dans `Settings`, résolu à 2 par défaut. La valeur 0 est refusée dans `Settings::load` et `Settings::save` uniquement. | La profondeur 0 (branche seule) n'a pas de sens, comme décidé au brainstorm. Une erreur explicite vaut mieux qu'une correction silencieuse. Une validation unique couvre la CLI, la fenêtre et le chargement. |
| La recherche d'une note déjà exportée (`find_existing_proof`) parcourt `dest`, `dest/Y`, `dest/Y/M` et `dest/Y/M/D`, d'après la date de l'email. | Sinon, un changement de motif ferait réexporter en double par le clic droit un email déjà présent sous l'ancien motif. |
| Le repli `route::DEFAULT_BASE = "Perso/Messy/Emails"` reste codé en dur : il est hors périmètre. | Il ne sert qu'en l'absence d'une entrée `default` dans `destinations.yaml`, que toute organisation peut déclarer. Le rendre configurable est un autre chantier. |
