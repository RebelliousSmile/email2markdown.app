---
objective: "Les trois critiques de l'audit du 2026-10-30 sont corrigés et les avertissements rattachés aux actions 1 à 7 et 10 sont traités, chacun couvert par un test ou une vérification décrite."
status: implemented
---

# Plan: Corrections de l'audit du 2026-10-30

## Overview

| Field      | Value |
| ---------- | ----- |
| **Goal**   | Traiter les actions 1 à 7 et 10 du rapport d'audit en douze phases livrables séparément. Le chemin qui supprime sur le serveur passe en premier. |
| **Source** | [`../2026_10_30_audit/report.md`](../2026_10_30_audit/report.md), sections « Top actions » et « Findings » ; détail par pilier dans les sept fichiers du même dossier. |

Les chemins des fichiers de phase sont relatifs à `app/`. Le dépôt est sur `main` : chaque phase se fait sur une branche dédiée (`fix/…`, `test/…`, `chore/…`), et aucun commit n'est créé sans demande explicite (`.claude/rules/04-tooling/git-main-protection.md`).

## Phases

| #   | Phase                                                         | File                           |
| --- | ------------------------------------------------------------- | ------------------------------ |
| 1   | Faire tourner en CI tous les tests IMAP Dovecot               | [`phase-1.md`](./phase-1.md)   |
| 2   | Tester le nettoyage serveur et la purge après export          | [`phase-2.md`](./phase-2.md)   |
| 3   | Valider les entrées venues du serveur et des notes            | [`phase-3.md`](./phase-3.md)   |
| 4   | Rendre « Nettoyer le serveur » explicite et suivi             | [`phase-4.md`](./phase-4.md)   |
| 5   | Retirer la sous-commande `dest gui`                           | [`phase-5.md`](./phase-5.md)   |
| 6   | Hygiène des dépendances et politique `cargo audit`            | [`phase-6.md`](./phase-6.md)   |
| 7   | Borner et tester la mise à jour automatique                   | [`phase-7.md`](./phase-7.md)   |
| 8   | Rendre visibles les erreurs masquées                          | [`phase-8.md`](./phase-8.md)   |
| 9   | Corriger trois défauts de comportement isolés                 | [`phase-9.md`](./phase-9.md)   |
| 10  | Sortir le routage du thread de la boucle d'événements         | [`phase-10.md`](./phase-10.md) |
| 11  | Regrouper les commandes IMAP d'export et de suppression       | [`phase-11.md`](./phase-11.md) |
| 12  | Remettre la mémoire du projet à jour                          | [`phase-12.md`](./phase-12.md) |

Ordre imposé :

- La phase 1 précède les phases 2, 3 et 11. Ces trois phases touchent le chemin de suppression : le test de purge doit tourner en CI avant qu'on y change quoi que ce soit.
- La phase 2 précède la phase 11. Le regroupement des `STORE`/`EXPUNGE` change `delete_validated`, que la phase 2 couvre de tests.
- Les phases 2, 8 et 4 se suivent dans cet ordre. Toutes trois touchent `clean_server` : la phase 2 la découpe, la phase 8 retire l'`unwrap_or_default` sur les réglages dans le cœur découpé, la phase 4 change les dialogues.
- La phase 12 vient en dernier : elle décrit l'état laissé par les autres.

Les phases 5, 6, 7, 9 et 10 sont indépendantes entre elles. La phase 10 et la phase 8 touchent toutes deux le handler de routage de `tray/windows.rs` (`:740-760`) : si les deux sont menées en parallèle, la seconde se rebase sur la première.

## Decisions

| Decision | Why |
| -------- | --- |
| Périmètre : actions 1 à 7 et 10 du rapport. Les actions 8 (base UI commune) et 9 (architecture de fond, effort L) sont exclues, de même que les findings mineurs non rattachés à une action. | Ce sont des chantiers de conception sans critère d'arrêt net. L'action 8 passe par `design:define`, l'action 9 par `aidd-dev:07-refactor` ; chacune demande son propre plan. Les seules retouches UI retenues sont celles du flux « Nettoyer le serveur » (phase 4), parce qu'elles relèvent de l'action 1. |
| `dest gui` est retiré, pas réparé. | Le tray tient un verrou d'instance unique (`tray/mod.rs:241`) et le menu ouvre déjà la fenêtre des destinations (`tray/menu.rs:199`). Réparer la commande demanderait soit un IPC entre processus, soit une troisième boucle d'événements, alors que l'audit relève déjà la duplication de `run_contextual`. La commande n'est documentée nulle part (README, CHANGELOG). |
| `clean_server` est découpée : garde du compte, plan de nettoyage et enregistrement des résultats deviennent des fonctions testées sans réseau ; le cœur `clean_server_in` prend des chemins injectés et une confirmation en paramètre. Le point d'entrée garde sa signature. | Les chemins actuels sont globaux (`handled::ledger_path`, `config::app_config_dir`) et le flux ouvre deux connexions IMAP (recensement, suppression) : injecter les chemins seuls ne rend rien testable hors réseau. L'enchaînement complet est couvert par Dovecot. |
| Chaque test Dovecot a son propre utilisateur IMAP, dont la Maildir est ensemencée par le service `seed` du fixture, au lieu de partager la boîte `test`. | Deux tests partagent aujourd'hui 3 messages, et l'un supprime ce que l'autre lit. Un dossier par test ne suffirait pas : la recherche contextuelle parcourt tous les dossiers du compte. La `passdb` statique du fixture accepte n'importe quel nom, donc un utilisateur par test ne coûte qu'un répertoire. |
| La signature de `SHA256SUMS` (minisign) reste hors de ce plan ; la phase 7 ne traite que le plafond de taille et les tests. | Une signature à clé hors ligne demande une décision sur la gestion et la rotation de la clé, déjà reportée au plan du 2026-10-12. Le plafond et les tests ne dépendent pas de cette décision. |
| `cargo audit` reste bloquant sur les seules vulnérabilités ; les avis `unmaintained` et `unsound` passent par un `.cargo/audit.toml` où chaque exception est justifiée, sans `-D warnings`. | `serde_yaml`/`unsafe-libyaml`, `instant`, `glib` et les crates `zbus` n'ont pas de correctif compatible : `-D warnings` ferait échouer la CI sans action possible. Un fichier d'exceptions daté oblige à revoir chaque nouvel avis. |
| Les montées de version majeures (wry, tao, tray-icon, rfd, mailparse, ureq) et le remplacement de `serde_yaml` sont exclus. | Chacune casse l'API et demande sa propre phase de migration avec tests manuels sur trois plateformes. |
