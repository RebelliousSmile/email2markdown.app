---
objective: "Les treize messages du seed Gmail, envoyés réellement entre les deux comptes Gmail de l'utilisateur, s'exportent depuis chaque compte sans perte ni bruit avec `skip_signature_images: true`, et chaque note satisfait `assert_seed_note`."
status: blocked
---

# Plan : envoi réel du seed Gmail entre les deux comptes (issue #19)

## Overview

| Field      | Value |
| ---------- | ----- |
| **Goal**   | Faire transiter le seed par la livraison Gmail réelle (en-têtes, Spam, Sent Mail, fils), puis l'exporter et le vérifier comme sur Dovecot |
| **Source** | Ancienne phase 6 du plan [`2026_11_10_campagne-gmail-seed`](../2026_11_10_campagne-gmail-seed/plan.md), sortie de ce plan le 11/11/2026 pour livrer les phases 1 à 5 ; issue GitHub RebelliousSmile/email2markdown.app#19 |

Le contexte (corpus, harnais, `tests/common/gmail_seed.rs`, frontière avec obs-mail, décisions) est dans le plan d'origine. Cette tâche ne reprend que l'envoi réel.

## Phases

| #   | Phase                                                | File                         |
| --- | ---------------------------------------------------- | ---------------------------- |
| 1   | Envoi du seed entre les deux comptes Gmail et export | [`phase-1.md`](./phase-1.md) |

## Blocage (2026-11-10)

La phase 1 exige les mots de passe d'application des comptes A et B, que seul l'utilisateur peut créer.

Pour débloquer :

1. Sur chacun des deux comptes, activer la validation en deux étapes puis créer un mot de passe d'application.
2. Définir localement, hors du dépôt : `EMAIL_TO_MARKDOWN_GMAIL_A_USER`, `EMAIL_TO_MARKDOWN_GMAIL_A_PASSWORD`, `EMAIL_TO_MARKDOWN_GMAIL_B_USER`, `EMAIL_TO_MARKDOWN_GMAIL_B_PASSWORD`.
3. Lancer l'implémentation de la phase 1.

## Decisions

| Decision | Why |
| -------- | --- |
| Tâche séparée plutôt que phase 6 du plan d'origine | Les phases 1 à 5 sont livrables sans les mots de passe d'application ; les garder bloquées par la phase 6 retenait des correctifs déjà testés sur Dovecot |
| Le gabarit MIME du seed est confirmé ici par un original Gmail réel (compte A vers compte B, capture collée, « Afficher l'original ») ; chaque écart devient une fixture | Décision reprise du plan d'origine : l'original n'est accessible qu'une fois les mots de passe d'application en place |
