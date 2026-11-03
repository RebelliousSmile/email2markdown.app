---
objective: "Dans l'onglet Comptes des paramètres, on peut réordonner les comptes par flèches, dupliquer un compte (serveur, port, dossiers ignorés) et renommer un dossier ignoré sur place."
status: implemented
---

# Plan: Comptes — dupliquer, réordonner, corriger un dossier ignoré

## Overview

| Field      | Value |
| ---------- | ----- |
| **Goal**   | Trois commodités dans l'onglet Comptes de `assets/config_window.html`, sans connexion IMAP depuis les paramètres |
| **Source** | Brainstorm du 2026-11-03 (conversation) : dupliquer = serveur + port + dossiers ignorés, nom/identifiant/mot de passe vides, réglages de comportement non copiés ; ordre par flèches haut/bas, enregistré dans `accounts.yaml` ; dossier ignoré renommable en cliquant sur l'étiquette |

## Phases

| #   | Phase | File |
| --- | ----- | ---- |
| 1   | Réordonner les comptes (flèches + action IPC `move_account`) | [`phase-1.md`](./phase-1.md) |
| 2   | Dupliquer un compte et renommer un dossier ignoré (page seule) | [`phase-2.md`](./phase-2.md) |

## Decisions

| Decision | Why |
| -------- | --- |
| Le déplacement s'envoie comme `move_account { account_name, direction }`, pas comme la liste complète de l'ordre | Opère sur le fichier tel qu'il est sur disque : une édition manuelle de `accounts.yaml` entre-temps ne fait pas échouer ni n'écrase l'ordre ; pas de validation de permutation |
| Chaque clic sur une flèche enregistre immédiatement et reçoit sa propre réponse `move_account` (liste des comptes), pas `reload` | `reload` réécrit l'onglet Général depuis le disque et effacerait des saisies non enregistrées ; `move_account` est ajouté à `reply_changed_accounts` pour reconstruire le menu |
