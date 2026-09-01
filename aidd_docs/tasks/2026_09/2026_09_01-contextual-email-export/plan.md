---
objective: "Depuis un dossier configuré, l’utilisateur peut choisir un compte IMAP, sélectionner les messages liés à ses correspondants, les convertir directement en Markdown et supprimer du serveur uniquement ceux dont l’écriture a réussi, sur Windows, macOS et Linux."
status: in-progress
---

# Plan: Export contextuel d’emails vers Markdown

## Overview

| Field      | Value |
| ---------- | ----- |
| **Goal**   | Remplacer l’export global par une recherche et une conversion déclenchées depuis un dossier précis. |
| **Source** | Brainstorm de la conversation du 1er septembre 2026. |

## Phases

| #   | Phase | File |
| --- | ----- | ---- |
| 1 | Dossiers autorisés et règles de correspondants | [`phase-1.md`](./phase-1.md) |
| 2 | Recherche IMAP ciblée et résultats stables | [`phase-2.md`](./phase-2.md) |
| 3 | Conversion sélectionnée et preuve locale | [`phase-3.md`](./phase-3.md) |
| 4 | Suppression serveur ciblée et reprise | [`phase-4.md`](./phase-4.md) |
| 5 | Fenêtre de recherche et de sélection multi-OS | [`phase-5.md`](./phase-5.md) |
| 6 | Déclencheurs des gestionnaires de fichiers | [`phase-6.md`](./phase-6.md) |

## Resources

| Source | Verified |
| ------ | -------- |
| https://docs.rs/imap/latest/imap/struct.Session.html | Le client Rust expose `uid_search`, `uid_fetch`, `uid_store` et `uid_expunge`; les critères IMAP combinent implicitement les termes par AND et utilisent `OR` explicitement. |
| https://www.rfc-editor.org/rfc/rfc9051.html | Les UID restent l’identifiant sûr entre recherche, sélection et action; `UID EXPUNGE` limite l’expunge aux messages explicitement visés. |
| https://developers.google.com/workspace/gmail/imap/imap-extensions | Gmail expose `X-GM-MSGID`, identifiant stable d’un même message à travers ses différents labels IMAP. |
| https://github.com/jonhoo/rust-imap | La bibliothèque recommande un vrai serveur IMAP jetable pour ses tests d’intégration; ses commandes bas niveau permettent de couvrir une extension non exposée par les types haut niveau. |
| https://learn.microsoft.com/en-us/windows/win32/shell/context | Un verbe Shell peut lancer le binaire en lui transmettant le chemin du dossier sélectionné. |
| https://support.apple.com/en-gb/guide/mac-help/mchl97ff9142/mac | Finder expose les processus Automator comme actions rapides et services. |
| https://help.gnome.org/gnome-help/nautilus-behavior.html | Nautilus expose les scripts utilisateur dans son menu contextuel et transmet les chemins sélectionnés. |
| https://develop.kde.org/docs/apps/dolphin/service-menus/ | Dolphin accepte un service menu ciblant `inode/directory` et transmet l’URL du dossier à une commande. |

## Decisions

| Decision | Why |
| -------- | --- |
| Interroger directement le serveur IMAP configuré plutôt que piloter Thunderbird à l’exécution. | Le projet possède déjà les comptes, l’authentification et le convertisseur IMAP; Thunderbird reste la source d’import de configuration. |
| Utiliser `destinations.yaml` comme liste d’autorisation des dossiers et y ajouter une règle `correspondent`. | Le chemin de destination et ses règles restent dans une source unique, sans seconde configuration parallèle. |
| Identifier les résultats par dossier, `UIDVALIDITY` et UID. | Les numéros de séquence IMAP peuvent changer entre l’affichage de la liste et la conversion. |
| Ajouter un identifiant source au frontmatter et utiliser `X-GM-MSGID` sur Gmail. | La présence locale, la déduplication entre labels et la reprise d’une suppression doivent reposer sur une preuve stable. |
| Lire `X-GM-MSGID` par l’API de commande bas niveau et `imap-proto` déjà présent. | La version verrouillée de `imap` ne doit pas être supposée exposer cet attribut Gmail dans son type `Fetch`. |
| Écrire chaque message complètement avant toute suppression serveur. | Un échec de conversion, de pièce jointe ou de déplacement ne doit jamais entraîner la perte de l’email source. |
| Restreindre le sélecteur de compte par les règles `account` de la destination lorsqu’elles existent. | Le choix explicite de la boîte ne doit pas contourner une contrainte déjà enregistrée. |
| Produire un Markdown autonome par email, avec ses pièces jointes à côté. | Le nouvel usage change la sélection et la destination, pas le format élémentaire déjà produit par l’application. |
| Exclure les dossiers locaux propres à Thunderbird. | Le flux cible les boîtes IMAP et leur suppression serveur; un contenu disponible seulement hors serveur n’est pas interrogeable par ce chemin. |
| Exiger une cible de système de fichiers local supportant renommage atomique et verrou de fichier. | Les URI virtuelles et stockages distants ne garantissent pas la transaction locale utilisée avant suppression. |
| Supporter le dossier sélectionné sur tous les OS et le fond du dossier courant lorsque le gestionnaire le permet. | Les gestionnaires de fichiers n’exposent pas tous un menu d’arrière-plan équivalent. |
| Garder un cœur portable et isoler les déclencheurs par OS. | Explorer, Finder, Nautilus et Dolphin n’exposent pas le même mécanisme d’intégration. |
| Valider le dossier après le clic plutôt que développer une extension dynamique par gestionnaire de fichiers. | Les intégrations simples peuvent être visibles sur d’autres dossiers; l’application refuse néanmoins tout chemin non déclaré ou hors de `notes_dir`. |
| Bloquer la livraison destructive sur un banc IMAP jetable et la livraison multi-OS sur une matrice CI et manuelle nommée. | Les chemins les plus risqués ne peuvent pas être validés uniquement par des tests unitaires locaux. |
