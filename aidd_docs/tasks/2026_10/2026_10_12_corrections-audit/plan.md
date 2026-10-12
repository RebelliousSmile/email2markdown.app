---
objective: "Les sept défauts critiques de l'audit du 2026-10-12 et les avertissements rattachés à ses huit premières actions sont corrigés, chacun couvert par un test ou une vérification manuelle décrite."
status: implemented
---

# Plan: Corrections de l'audit du 2026-10-12

## Overview

| Field      | Value                                                                                                                                                                  |
| ---------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Goal**   | Traiter les actions 1 à 8 du rapport d'audit, critiques d'abord, en huit phases livrables séparément.                                                                    |
| **Source** | [`../2026_10_12_audit/report.md`](../2026_10_12_audit/report.md), sections « Top actions » et « Findings » ; détail par pilier dans les sept fichiers du même dossier. |

Les chemins des fichiers de phase sont relatifs à `app/`, sauf la phase 3 qui travaille dans `../site/` (dépôt git distinct). Les deux dépôts sont sur `main` : chaque phase se fait sur une branche dédiée, et aucun commit n'est créé sans demande explicite.

## Phases

| #   | Phase                                                  | File                         |
| --- | ------------------------------------------------------ | ---------------------------- |
| 1   | Ne plus écraser la configuration après un échec de chargement | [`phase-1.md`](./phase-1.md) |
| 2   | Échapper les données injectées dans les fenêtres       | [`phase-2.md`](./phase-2.md) |
| 3   | Mettre à jour Nuxt et couper les devtools en production | [`phase-3.md`](./phase-3.md) |
| 4   | Rendre « Appliquer » véridique dans la revue de routage | [`phase-4.md`](./phase-4.md) |
| 5   | Sécuriser la mise à jour automatique                   | [`phase-5.md`](./phase-5.md) |
| 6   | Builds reproductibles et CI plus stricte               | [`phase-6.md`](./phase-6.md) |
| 7   | Réduire les deux coûts d'export dominants              | [`phase-7.md`](./phase-7.md) |
| 8   | Fiabiliser les retours des fenêtres                    | [`phase-8.md`](./phase-8.md) |

Les phases sont indépendantes, à trois exceptions près. La phase 8 vient après la phase 1 : elle retouche `handle_config_ipc` et termine la remontée des échecs d'ouverture de fenêtre commencée en phase 1. La phase 6 vient après la phase 5 si l'on veut que `Cargo.lock` suivi contienne déjà `sha2`. Les phases 5 et 6 modifient toutes deux `release.yml` : les mener l'une après l'autre, pas en parallèle.

## Decisions

| Decision | Why |
| -------- | --- |
| Périmètre limité aux actions 1 à 8 du rapport. Les actions 9 (socle visuel et accessibilité) et 10 (doublons structurels) sont exclues, ainsi que les findings mineurs non rattachés à une action. | Les actions 9 et 10 sont des chantiers de conception et de refonte (effort L), sans critère d'arrêt net : ils demandent chacun leur propre plan. Les mêler ici rendrait les phases non vérifiables seules. |
| Les handlers IPC gardent leur signature ; la logique testable est extraite dans des fonctions paramétrées par chemin. | Les handlers lisent `config::settings_path()` et `config::accounts_yaml_path()`, des chemins globaux : impossible de les tester avec `TempDir` sans écrire dans la vraie configuration de l'utilisateur. |
| La fenêtre de routage n'agit que sur les fichiers que l'hôte lui a envoyés. | L'hôte connaît déjà la liste (`decisions` dans `build_route_window`). Comparer à cette liste borne « Appliquer » et « Supprimer » sans introduire de notion de dossier de transit. |
| « Appliquer » valide tout avant le premier déplacement, puis rend compte fichier par fichier. Pas de retour arrière. | Un déplacement de fichiers et de pièces jointes n'est pas transactionnel ; un retour arrière peut lui-même échouer à mi-chemin. La validation préalable élimine les échecs prévisibles, le compte rendu couvre les autres. |
| La mise à jour est vérifiée par une somme SHA-256 publiée dans la release, pas par une signature. | La somme détecte un téléchargement tronqué ou altéré en transit. Elle ne protège pas d'un compte GitHub compromis, puisque la somme vient de la même release : une signature à clé hors ligne reste à décider séparément (gestion de clé, rotation). |
| L'URL de téléchargement reste côté hôte ; la fenêtre n'envoie plus qu'une confirmation. | Aujourd'hui `windows.rs:215-219` télécharge et exécute ce que la webview lui indique. Une injection dans la page suffirait à faire installer un binaire arbitraire. |
