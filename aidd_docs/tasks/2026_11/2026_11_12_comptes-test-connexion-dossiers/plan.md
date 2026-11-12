---
objective: "Depuis l'app, un compte ajouté à la main ou importé de Thunderbird voit sa connexion vérifiée, ses dossiers choisis par cases à cocher, et tout échec (Gmail compris) est signalé par une alerte qui en donne la cause."
status: in-progress
---

# Plan: Test de connexion, dossiers à cocher et import Thunderbird sélectif

## Overview

| Field      | Value |
| ---------- | ----- |
| **Goal**   | Remplacer la saisie manuelle des dossiers ignorés par un test de connexion qui liste les dossiers du serveur à cocher, et rendre l'import Thunderbird sélectif, avec reprise des mots de passe et vérification de chaque connexion. |
| **Source** | Brainstorm du 2026-11-12 (conversation) : « tester la connexion et proposer la liste des répertoires à cocher », import Thunderbird « cocher parmi les comptes », « bien vérifier la connexion à Gmail dans les ajouts et les imports », « afficher une alerte » en cas d'échec. |

## Phases

| #   | Phase | File |
| --- | ----- | ---- |
| 1   | Sonde de connexion et liste complète des dossiers | [`phase-1.md`](./phase-1.md) |
| 2   | Liste d'inclusion des dossiers et sélection à l'export | [`phase-2.md`](./phase-2.md) |
| 3   | Formulaire de compte : bouton de test et dossiers à cocher | [`phase-3.md`](./phase-3.md) |
| 4   | Thunderbird : inventaire des comptes, OAuth, mot de passe principal | [`phase-4.md`](./phase-4.md) |
| 5   | Import Thunderbird sélectif dans la fenêtre Paramètres | [`phase-5.md`](./phase-5.md) |

## Resources

| Source | Verified |
| ------ | -------- |
| RFC 6154 (IMAP LIST Extension for Special-Use Mailboxes) | `\Sent`, `\Junk`, `\Trash`, `\Drafts`, `\All`, `\Flagged` identifient les dossiers spéciaux quelle que soit la langue ; c'est ce que `list_folders` filtre déjà. |

## Decisions

| Decision | Why |
| -------- | --- |
| **Gmail : mot de passe d'application seulement ; OAuth2 reporté à un projet séparé.** Les comptes Thunderbird en OAuth2 sont détectés et signalés par une alerte qui explique comment créer un mot de passe d'application. **À confirmer** : la question « il faut gérer les deux ? » a reçu cette recommandation sans approbation explicite. | Réutiliser le jeton OAuth de Thunderbird contrevient aux conditions de Google ; un client OAuth propre demande la portée restreinte `https://mail.google.com/`, donc une vérification Google avec audit de sécurité, faute de quoi l'app est plafonnée à 100 utilisateurs (faits à revérifier avant d'ouvrir ce chantier). |
| **Liste d'inclusion `included_folders`** à côté de `ignored_folders` : quand elle est présente, seuls les dossiers cochés sont exportés et un dossier créé plus tard sur le serveur reste ignoré tant qu'on ne l'a pas coché. **Hypothèse à confirmer.** | Le brainstorm propose de prendre par défaut « Inbox et Envoyés » : c'est une sélection positive. Les comptes existants gardent `ignored_folders` tant qu'on n'a pas enregistré une sélection, donc aucune migration forcée. |
| **Coché par défaut : la boîte de réception (`INBOX`) et le dossier `\Sent`**, ses sous-dossiers non cochés ; les dossiers spéciaux (corbeille, spam, brouillons, « Tous les messages », suivis, « Important ») affichés grisés « toujours ignoré ». **Hypothèses à confirmer.** | Reprend la proposition du brainstorm et rend visible le filtrage SPECIAL-USE qui se faisait jusqu'ici en silence. |
| **Un compte importé dont la connexion échoue est quand même importé**, avec `included_folders: [INBOX]`, et l'alerte le liste avec la cause. Même règle pour « Enregistrer quand même » sur un compte ajouté à la main. **Hypothèse à confirmer.** | Le compte et son mot de passe Thunderbird sont conservés pour une nouvelle tentative ; exporter seulement la boîte de réception évite d'aspirer tout le serveur avant qu'on ait pu cocher. |
| **L'import Thunderbird se fait dans la fenêtre Paramètres** (onglet Comptes), plus par la boîte de dialogue `rfd` à trois boutons. | Réutilise le canal IPC asynchrone créé en phase 3 et le formulaire de compte pour corriger un compte en échec, sans nouvelle fenêtre. |
| Le test de connexion et l'import tournent dans un `thread::spawn` et répondent par `AppCommand::EvalScript` ; jamais dans le gestionnaire IPC. | Une connexion IMAP peut durer jusqu'au délai de lecture ; bloquer le gestionnaire figerait la boucle `tao` et toutes les fenêtres. |
| La sonde est bornée par un délai global (`recv_timeout`) ; le fil d'une sonde expirée est abandonné. | `imap::ClientBuilder` 3.0.0-alpha.15 n'offre aucun délai de connexion ni de lecture avant le message d'accueil, et ses fonctions TLS sont privées ; reconstruire la pile TLS pour un seul cas coûterait plus que le fil abandonné. |
| Un mot de passe enregistré n'est réutilisé par le test que pour le serveur et l'identifiant auxquels il a été enregistré. | Une faute de frappe sur le serveur ne doit jamais envoyer le mot de passe à un autre hôte. |
