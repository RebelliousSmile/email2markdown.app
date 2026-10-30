---
status: done
---

# Instruction: Revérifier l'identité du message avant toute suppression ciblée

Findings : security 🟡 `src/email_export.rs:2249-2300` (suppression pilotée par une preuve de note de staging) ; tests 🟡 `src/email_export.rs:1514-1523` (échec partiel d'`expunge_uids`).

À mener après la phase 2 : le test d'export avec purge doit couvrir `delete_validated` avant qu'on le modifie.

## Architecture projection

> Tree of the final files. ✅ create · ✏️ modify · ❌ delete

```txt
.
├── src/contextual_export.rs  ✏️ `identity_matches(proof, &FetchedMessage) -> bool` via `parse_header_candidate` ; divergence rendue par le variant existant `DeletionOutcome::Rejected`
├── src/email_export.rs       ✏️ `delete_validated` (branche UIDPLUS) : `fetch_headers_by_uid` puis filtrage par `identity_matches` avant `expunge_uids` ; `classify_expunge(present, store_expunge_result) -> Vec<DeletionOutcome>` pure
├── tests/imap_integration.rs ✏️ test : preuve forgée (bon UID, autre `Message-ID`) ne supprime rien
└── tests/rust_tests.rs       ✏️ tests de `identity_matches` et de `classify_expunge`
```

## User Journey

```mermaid
flowchart TD
  A[Suppression ciblée : purge, rejet en revue, conversion contextuelle, nettoyage] --> B[SELECT du dossier, contrôle UIDVALIDITY]
  B --> C[UID FETCH des en-têtes des UID visés]
  C --> D{Message-ID et empreinte égaux à la preuve ?}
  D -- non --> E[Rejected, message conservé, ligne rejected dans deletions.log]
  D -- oui --> F[UID STORE puis UID EXPUNGE du lot]
  F --> G[classify_expunge : Deleted, AlreadyAbsent ou RetryRequired]
```

## Test Scope

```mermaid
---
title: Test scope
---
journey
  section Setup
    utilisateur Dovecot dédié avec les 3 messages du seed => preuve légitime d'alice lue par search_contextual: 5: system
  section Happy path
    delete_discarded_messages avec la preuve légitime => Deleted, message absent du serveur: 5: system
    classify_expunge avec STORE et EXPUNGE réussis => Deleted pour les présents, AlreadyAbsent pour les autres: 5: system
  section Edge case - preuve forgée
    preuve d'alice dont l'UID pointe le faux positif => delete_discarded_messages => Rejected, les 3 messages toujours présents: 1: system
  section Edge case - EXPUNGE en échec après STORE
    STORE réussi puis EXPUNGE en erreur => classify_expunge => RetryRequired dont le texte signale le drapeau Deleted déjà posé: 1: system
  section Edge case - SEARCH en échec
    uid_search en erreur => classify_expunge => RetryRequired pour tous les UID, aucun STORE: 1: system
  section Teardown
    docker compose down -v => boîte jetée avec le volume: 5: system
```

## Tasks to do

### `1)` Comparaison d'identité pure

> Une seule définition de « c'est bien ce message ».

1. `identity_matches(proof, fetched)` : `parse_header_candidate` sur les en-têtes, comparaison de `message_id` et `header_fingerprint`.
2. Tests unitaires : égalité, `Message-ID` différent, même `Message-ID` et empreinte différente.
3. Vérifier qu'une preuve écrite par l'export (`export_folder`) et une preuve écrite par la conversion contextuelle donnent la même empreinte que `fetch_headers_by_uid` ; si l'export calcule l'empreinte sur d'autres en-têtes, aligner le calcul avant d'activer le contrôle.

### `2)` Contrôle dans `delete_validated`

> Aucune suppression générique sans revérification serveur.

1. Après le contrôle d'UIDVALIDITY, `fetch_headers_by_uid` sur les UID du groupe.
2. UID absent de la réponse : `AlreadyAbsent`. Identité différente : `Rejected("identité serveur différente de la preuve")`, exclu du lot. `tally_deletions` le compte déjà comme refus non réessayé et `deletion_log` l'écrit en `rejected:`.
3. `expunge_uids` sur les seuls UID confirmés.
4. Gmail : dans la boucle par message actuelle, comparer les en-têtes du message trouvé par `X-GM-MSGID` avec `identity_matches` avant le `UID MOVE`. Pas de restructuration : la phase 4 réutilise `identity_matches` dans son regroupement.

### `3)` Classification testable d'`expunge_uids`

> Couvrir l'échec partiel sans mock IMAP (`5-rust-tests.md`).

1. Extraire `classify_expunge(uids, present, result)` de `expunge_uids`.
2. Dans le cas STORE réussi puis EXPUNGE en échec, le message de `RetryRequired` indique que le drapeau `\Deleted` est posé.
3. Tests unitaires des trois cas.

### `4)` Test Dovecot de preuve forgée

> Reproduire le scénario de l'audit.

1. Construire une preuve avec l'identité d'alice et l'UID du faux positif.
2. Vérifier `Rejected` et la présence des trois messages par session brute.

## Test acceptance criteria

| Task | Acceptance criteria |
| ---- | ------------------- |
| 1    | Une preuve correspond à un message si et seulement si `Message-ID` et empreinte sont égaux ; preuves d'export et preuves contextuelles passent le contrôle. |
| 2    | Aucun `UID STORE` n'est envoyé pour un UID dont l'identité serveur diffère de la preuve ; les tests Dovecot existants et celui de la phase 2 restent verts. |
| 3    | Les trois issues d'`expunge_uids` (succès, échec de SEARCH, échec d'EXPUNGE après STORE) sont couvertes par des tests sans réseau. |
| 4    | Une preuve forgée ne supprime aucun message et laisse une ligne `rejected:` dans le journal. |
