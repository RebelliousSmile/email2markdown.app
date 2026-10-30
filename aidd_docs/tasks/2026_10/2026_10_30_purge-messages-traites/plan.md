---
objective: "Avec delete_after_export, un message déjà routé ou écarté qui reste sur le serveur en est supprimé de façon ciblée et prouvée, sans jamais toucher un message seulement présent en staging, et chaque échec de suppression devient visible."
status: implemented
---

# Plan: Purge serveur des messages déjà traités

## Overview

| Field      | Value |
| ---------- | ----- |
| **Goal**   | Rétablir la suppression des messages déjà traités, retirée en `65c92f6` faute de preuve, en s'appuyant sur une preuve source (registre `handled_messages.txt` enrichi, frontmatter `source:` des notes routées) au lieu de l'empreinte floue `contains_message`. Rendre d'abord visible l'état réel du serveur et les échecs de suppression. |
| **Source** | Brainstorm de la conversation du 2026-10-30 (option B pour les nouvelles entrées, passage unique pour les 1002 entrées héritées de `gutsandgrit@gmail.com`), révisé par challenge : état des lieux préalable, journal des suppressions, complétion des entrées héritées par les notes routées. Reprend la tâche 5.2 jamais livrée de `aidd_docs/tasks/2026_09/2026_09_01-contextual-email-export/phase-4.md`. |

## Phases

| #   | Phase                                                   | File                         |
| --- | ------------------------------------------------------- | ---------------------------- |
| 1   | État des lieux serveur et journal des suppressions      | [`phase-1.md`](./phase-1.md) |
| 2   | Registre avec preuve et complétion depuis les notes     | [`phase-2.md`](./phase-2.md) |
| 3   | Purge prouvée pendant l'export global                   | [`phase-3.md`](./phase-3.md) |
| 4   | Nettoyage confirmé des entrées sans preuve (tray)       | [`phase-4.md`](./phase-4.md) |

## Decisions

| Decision | Why |
| -------- | --- |
| Seule une preuve source (registre ou frontmatter `source:` d'une note routée) autorise une suppression ; un message sauté parce que sa note existe en staging (`ExportedIndex::contains_message`) n'est jamais supprimé. | L'empreinte date + noms courts + hash de sujet produit des collisions (notifications, newsletters) : c'est la raison du retrait en `65c92f6`. Une note en staging n'est pas encore routée (déplacement différé D6). |
| L'état des lieux en lecture seule est livré avant tout code destructif. | Le staging de `gutsandgrit@gmail.com` est vide et certains messages de la capture du 2026-10-30 n'ont pas de note routée : rien ne prouve encore qu'ils figurent au registre. La phase 1 confirme ou infirme la cause avant d'agir. |
| Le registre passe à `compte\tmessage_id\t<preuve JSON>` ; à Message-ID égal, une ligne avec preuve l'emporte sur une ligne héritée. | Le Message-ID seul ne prouve pas que le message serveur est celui traité ; la preuve (UIDVALIDITY + UID, ou X-GM-MSGID sur Gmail) le prouve. Les lignes à deux colonnes restent lisibles pour le `skip_existing`. |
| Les entrées héritées sont d'abord complétées par la preuve du frontmatter des notes routées ; le Message-ID seul ne sert qu'aux messages écartés, sur action tray confirmée. | Les notes routées portent déjà la preuve complète (vérifié sur des notes du 2026-10-29 et 2026-10-30). Seuls les messages écartés n'ont plus de note : l'utilisateur autorise leur suppression une fois, en connaissant le nombre. |
| La suppression réutilise `delete_validated` (UID STORE + UID EXPUNGE ciblés, Trash + expunge sur Gmail), jamais d'EXPUNGE global ; tout résultat est écrit dans un journal fichier. | Même garantie que le routage et le rejet en revue. Dans le tray, `eprintln!` n'est vu par personne : sans journal, un échec reste muet comme aujourd'hui. |
