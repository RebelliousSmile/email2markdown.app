---
objective: "Le critique de l'audit v0.23.0 est fermé par un test Dovecot d'export réel avec purge, aucune suppression ciblée ne part sans revérification serveur de l'identité du message, et les avertissements rattachés aux actions 1 à 6 du rapport sont traités, chacun couvert par un test ou une vérification décrite."
status: implemented
---

# Plan: Corrections de l'audit v0.23.0

## Overview

| Field      | Value |
| ---------- | ----- |
| **Goal**   | Traiter les actions 1 à 6 du rapport d'audit v0.23.0 en huit phases livrables séparément, en commençant par rendre la purge d'export testable. |
| **Source** | [`../2026_10_30_audit-v0.23.0/report.md`](../2026_10_30_audit-v0.23.0/report.md), sections « Top actions » et « Findings » ; détail par pilier dans les sept fichiers du même dossier. |

Les chemins des fichiers de phase sont relatifs à `app/`. Le dépôt est sur `main` : chaque phase se fait sur une branche dédiée (`fix/…`, `test/…`, `chore/…`), et aucun commit n'est créé sans demande explicite (`.claude/rules/04-tooling/git-main-protection.md`).

## Phases

| #   | Phase                                                          | File                         |
| --- | -------------------------------------------------------------- | ---------------------------- |
| 1   | Injecter les chemins de configuration dans l'export            | [`phase-1.md`](./phase-1.md) |
| 2   | Tester l'export réel avec purge sur Dovecot                    | [`phase-2.md`](./phase-2.md) |
| 3   | Revérifier l'identité du message avant toute suppression ciblée | [`phase-3.md`](./phase-3.md) |
| 4   | Regrouper et planifier la suppression Gmail                    | [`phase-4.md`](./phase-4.md) |
| 5   | Corriger les défauts isolés                                    | [`phase-5.md`](./phase-5.md) |
| 6   | Durcir la politique de dépendances en CI                       | [`phase-6.md`](./phase-6.md) |
| 7   | Libérer le thread de la boucle d'événements                    | [`phase-7.md`](./phase-7.md) |
| 8   | Remettre la mémoire du projet à jour                           | [`phase-8.md`](./phase-8.md) |

Ordre imposé :

- Les phases 1, 2, 3 et 4 se suivent dans cet ordre. La phase 2 a besoin des chemins injectés par la phase 1. Les phases 3 et 4 modifient `delete_validated`, que le test d'export avec purge de la phase 2 doit déjà couvrir.
- La phase 8 vient en dernier : elle décrit l'état laissé par les autres.

Les phases 5, 6 et 7 ne dépendent logiquement ni entre elles ni des phases 1 à 4, mais elles partagent des fichiers : `src/config.rs` (phases 1 et 5) et `src/tray/ipc.rs` (phases 5 et 7, fonctions distinctes : `set_env_password` pour l'une, `scan_destinations` et lecture des notes pour l'autre). La branche fusionnée en second se rebase sur la première.

## Resources

| Source | Verified |
| ------ | -------- |
| `~/.cargo/registry/src/*/rfd-0.15.4/Cargo.toml`, section `[features]` | `rfd` 0.15.4 a pour features par défaut `xdg-portal` et `async-std`, et propose `gtk3`. `xdg-portal` tire `ashpd`, d'où `zbus`/`zvariant`. Désactiver les features par défaut au profit de `gtk3` est donc possible sans changer de version. |

## Decisions

| Decision | Why |
| -------- | --- |
| Périmètre : actions 1 à 6 du rapport. L'action 7 (cycles et découpage d'`email_export`, `route` ↔ `destinations`, base UI commune, helpers DRY), la migration de `serde_yaml`, les montées de version majeures et la signature de `SHA256SUMS` sont exclues, de même que les findings non rattachés à une action. | Ce sont des chantiers de conception sans critère d'arrêt net, chacun avec son propre plan (`aidd-dev:07-refactor`, `design:define`). Le plan du matin les avait déjà exclus pour la même raison. Seule exception : la duplication introduite par le batching IMAP (code-quality 🟡 `src/email_export.rs:2474`), retirée en phase 2 une fois couverte par le nouveau test. |
| `CleanupPaths` devient `config::AppPaths` et sert aussi à l'export. `export_account` prend des chemins injectés ; un wrapper garde l'appel actuel pour `main.rs` et `tray_actions.rs`. | L'export et le nettoyage lisent les mêmes fichiers (registre, `settings.yaml`, `deletions.log`). Un second type de chemins dupliquerait celui-ci, et `run_contextual_batch` (tests 🟡 `src/tray_actions.rs:284`) en serait le troisième consommateur (`.claude/rules/07-quality/dry-refactor.md`). |
| La revérification serveur se fait dans `delete_validated`, pour tous les appelants : purge d'export, rejet en revue, suppression après conversion contextuelle, nettoyage serveur. Elle compare `Message-ID` et `header_fingerprint`, recalculés par `parse_header_candidate` sur un `UID FETCH` des mêmes en-têtes. Une divergence rend le variant existant `DeletionOutcome::Rejected`. | Revérifier seulement sur le chemin « rejet en revue » laisserait les autres appelants dépendre d'une preuve lue sur disque. Un seul point de contrôle couvre aussi les appelants futurs. Le coût est un `UID FETCH` d'en-têtes par lot de 500, déjà fait par la recherche contextuelle. |
| Les tests Dovecot passent par `--test-threads=1` en CI au lieu de retirer chaque `set_var`. | `require_fixture` pose `EMAIL_TO_MARKDOWN_IMAP_INSECURE_TEST`, lu par le code de connexion : le retirer demanderait un paramètre TLS dans `Account` pour les seuls tests. Sérialiser le binaire d'intégration supprime la course à coût nul (moins de dix tests). Le `set_var` du mot de passe (`cleanup_dir`) reste : le déplacer dans le `.env` du `TempDir` ne changerait rien, car `dotenvy::from_path` pose lui aussi la variable dans le processus, et tous les comptes de test partagent le nom « Dovecot fixture », donc la même variable. |

## Vérifications en attente

Le code des huit phases est livré en v0.23.1 (tag `v0.23.1`, CI verte sur Windows, macOS et Ubuntu). Le statut `implemented` porte sur le code : quatre critères d'acceptation reposent sur des vérifications manuelles qui n'ont pas encore été faites.

| Phase | Critère | État au 2026-10-31 |
| ----- | ------- | ------------------ |
| 4, tâche 3 | La suppression Gmail regroupée par dossier fonctionne sur un vrai compte. | Jamais passée. Le job `gmail-destructive` échoue à la connexion : le secret `EMAIL_TO_MARKDOWN_GMAIL_PASSWORD` contient le mot de passe du compte, alors que Gmail exige un mot de passe d'application. Aucune opération n'a eu lieu sur la boîte. |
| 5, tâche 4 | Avec un `accounts.yaml` invalide, le menu affiche « Comptes illisibles — voir Paramètres ». | Non vérifiée. |
| 6 | Les dialogues `rfd` s'ouvrent sous Linux sans portail XDG. | Compilation et tests validés par la CI Ubuntu ; l'affichage des dialogues reste à vérifier. |
| 7, tâche 2 | Le menu du tray et les fenêtres répondent pendant un scan de destinations. | Non vérifiée. |

Le risque principal est la première ligne : le chemin réseau de la suppression Gmail (`uid_mv`, recherche des `X-GM-MSGID`, reprise message par message) n'a tourné que contre la planification pure, jamais contre Gmail. Pour relancer le job une fois le secret corrigé :

```bash
gh workflow run ci.yml --ref main -f run_gmail_destructive=true
```
