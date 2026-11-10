---
objective: "Un corpus d'une douzaine de messages (cinq conversations, treize messages) au format MIME de Gmail, qui couvre chaque cas (liens, images collées et jointes, listes, sauts de ligne, transferts), s'exporte sans perte ni bruit, en local et sur Dovecot, avec `skip_signature_images: true`, et chaque cas est vérifié automatiquement."
status: implemented
---

# Plan : campagne de tests Gmail par seed (issues #19, #14, #15, #18)

## Overview

| Field      | Value |
| ---------- | ----- |
| **Goal**   | Remplacer la campagne manuelle de #19 par un seed reproductible de conversations Gmail, exporté et contrôlé de bout en bout, puis corriger ce qu'il révèle |
| **Source** | Issue GitHub RebelliousSmile/email2markdown.app#19, regroupée avec #14 (réouverte), #15 et #18 ; demande utilisateur du 10/11/2026 : « un seed avec deux ou trois conversations à destination de gmail, incluant des liens, des images, des listes, des sauts de ligne », puis un export contrôlé ; précision : envoyer vers les deux comptes Gmail personnels de l'utilisateur (comptes A et B, adresses fournies par variables d'environnement, jamais versionnées) |

### Regroupement

Les quatre issues portent sur le même flux : un mail écrit dans Gmail, dont la partie `text/plain` remplace les images par `[image: …]` et coupe les lignes à ~76 colonnes, et dont les images collées peuvent être écartées sans trace.

| Issue | Sujet | Couverte par |
| ----- | ----- | ------------ |
| #19 | Campagne de bout en bout, fixtures Gmail, compte Dovecot `skip_signature_images: true`, garde-fou de suppression, défauts de rendu révélés par le corpus | phases 1, 2, 3, 5 ; envoi réel entre les comptes Gmail : tâche `2026_11_11_campagne-gmail-reelle` |
| #14 | Capture Gmail perdue après la v0.20.0, sans quarantaine | phases 1, 2 |
| #15 | `[image: …]` reste dans une note issue de `text/plain` | phases 1, 2 |
| #18 | Retours à la ligne du client mail conservés dans le corps `text/plain` | phases 1, 4 |

### Frontière avec la skill obs-mail

email2markdown transforme un message IMAP en note ; obs-mail retravaille des notes déjà écrites. Seul email2markdown voit le MIME : tout ce qui dépend d'une partie, d'un `Content-ID`, d'un paramètre `format=flowed` ou d'une suppression serveur lui revient. Tout ce qui se décide en lisant la note, sans le message d'origine, revient à obs-mail.

| Sujet | email2markdown (ce plan) | obs-mail (hors plan) |
| ----- | ------------------------ | -------------------- |
| Images collées, jointes, signature | extraire, nommer, mettre en quarantaine, lister dans `### Images :` / `### Pieces jointes :` | aucun rôle : `convert --links-only` répare seulement les liens de notes anciennes |
| `[image: …]` (#15) | relier aux parties MIME | aucun rôle, la note ne contient plus les images d'origine |
| Lignes coupées (#18) | `format=flowed` et `unwrap_lines` à l'export | aucun rôle |
| Citations et transferts | conservés **tels quels et sans perte** dans le corps du bloc | `split` : une note par message cité, section `### Conversation :` |
| Signatures texte répétées dans les citations | conservées | `quote_clean` : suppression sans perte |
| Notation de lien du `text/plain` (`texte <url>`) | URL conservée, notation laissée telle quelle | `convert` réécrit la notation, y compris sur une note déjà convertie |
| Tri, classement, envoi | aucun rôle | `triage`, `perso-filing`, `compose` |
| Suppression sur le serveur | garde-fou (#19) | aucun rôle, obs-mail ne supprime rien |

Le seed vérifie le passage de relais : les notes exportées doivent être lisibles par obs-mail (bloc TOML relu par `tomllib`, en-têtes de réponse et de transfert de Gmail intacts). Un `split` en mode simulation sur l'export de l'envoi réel (tâche `2026_11_11_campagne-gmail-reelle`) ne doit signaler aucune « découpage avec perte ». Ce contrôle reste manuel, car les scripts d'obs-mail ne font pas partie du dépôt.

Restent hors périmètre : #16 (empreinte de logo, demande un index persistant), #13 (HTML marketing, tables de mise en page) et #10 (`#` du HTML et tags Obsidian). Le seed ne les exerce pas.

### Constats de l'exploration

- **Perte silencieuse possible** : dans `extract_attachments` (`src/email_export.rs:1146`), une partie sans `Content-Disposition` et dont le `Content-ID` n'est pas référencé par le corps retenu est sautée par `continue`. Elle n'est ni écrite ni mise en quarantaine. Cela correspond à la piste 2 de la réouverture de #14.
- **Le choix du corps HTML** ne se fait que si sa conversion référence encore un `cid:` (`extract_body_reporting`). Si la conversion perd l'image, ou si la structure diffère (`multipart/related` à la racine), on retombe sur le `text/plain` et ses `[image: …]`.
- **#18 ne part pas de zéro** : `cleaner::unwrap_lines` recolle déjà la prose coupée (seuil `UNWRAP_MIN_LEN = 60`). Elle ne recolle pas la suite d'un item de liste (une ligne `1. …` est protégée, donc la ligne suivante reste isolée), ni une ligne qui commence par `»`. `format=flowed` n'est pas lu.
- **Seed Dovecot** : `imap` 3.0.0-alpha.15 fournit `Session::append(mailbox, content)` avec `internal_date` et `finish`. Pour Dovecot, la copie Maildir de `compose.yaml` suffit.
- **Seed Gmail** : les conversations sont réellement envoyées entre les comptes A et B par `smtp.gmail.com`. Le MIME est transmis octet pour octet (`lettre`, `send_raw`), seuls `From`, `To`, `Message-ID` et l'étiquette de lancement sont substitués. Chaque compte reçoit des messages dans INBOX et garde ses envois dans `[Gmail]/Sent Mail` (ou `[Gmail]/Messages envoyés` selon la langue).
- **Boîtes personnelles** : `export_account_in` exporte tous les dossiers sauf `ignored_folders`, sans liste blanche. Le test isole donc chaque essai dans son propre libellé (`email-to-markdown-seed/<run_id>` : `UID MOVE` depuis INBOX et Spam, `UID COPY` depuis les envoyés, qui gardent ainsi leur libellé Sent) et ignore dynamiquement tous les autres dossiers. Chaque essai exporte ainsi 12 messages par compte, jamais l'historique des essais précédents. Rien n'est supprimé : `delete_after_export: false` est imposé et vérifié.
- **Identifiants** : Gmail refuse le mot de passe du compte en IMAP et SMTP, il faut un mot de passe d'application par compte. Le job CI `gmail-destructive` échoue déjà pour cette raison (`aidd_docs/memory/testing.md`).

### Constats de la phase 1 (replan du 10/11/2026)

- **C2-M4 ne reproduit pas #14** : la capture y est référencée par le HTML, donc écrite et liée. La reproduction passe par un nouveau message, C5-M13 : `text/plain` seul et image sans `Content-Disposition`, ce qui atteint le `continue` de `src/email_export.rs:1146` (phase 2).
- **Garde-fou** : la règle « marqueur restant et moins de parties image que de marqueurs » bloquerait à tort C4-M12, dont les trois marqueurs viennent d'images distantes de la signature HTML. Un marqueur ne bloque que s'il n'a aucune contrepartie, ni partie image ni `<img>` du HTML (phase 2).
- **Marqueur coupé** : Gmail peut couper `[image:` en fin de ligne (C4-M12). Le motif doit l'accepter (phase 2).
- **CI** : le job `dovecot` lance `--ignored --skip gmail_`. Les tests Dovecot du corpus ne doivent donc pas contenir `gmail_` dans leur nom (phase 5). Le test Gmail réel, `gmail_seed_send_and_export`, en est exclu volontairement.
- **Défauts sans issue dédiée**, rattachés à #19 (phase 3) : URL à parenthèses corrompue, liste imbriquée indentée d'un espace, nom RFC 2231 non décodé, en-tête de transfert fusionné, stem de quarantaine faux dans un export partagé.
- **Déjà livré** : `.gitattributes` (`.eml` en `-text`), `tests/common/gmail_seed.rs`, `substitute`, exception consignée dans `testing.md` et dans la règle de tests (commit `f7968a6`).

L'ordre des phases suit la règle de test du projet : le corpus et le harnais d'abord, avec les cas qui échouent aujourd'hui marqués `#[ignore]` et le numéro de l'issue. Ensuite chaque correction réactive ses cas, puis les mêmes assertions passent sur Dovecot et sur Gmail.

## Phases

| #   | Phase                                                      | File                         |
| --- | ---------------------------------------------------------- | ---------------------------- |
| 1   | Corpus Gmail et harnais de vérification en local (fait, `f7968a6`) | [`phase-1.md`](./phase-1.md) |
| 2   | Images : reproduction de #14, aucune perte silencieuse, `[image: …]` relié, garde-fou | [`phase-2.md`](./phase-2.md) |
| 3   | Rendu : liens à parenthèses, listes imbriquées, transferts, noms RFC 2231, quarantaine | [`phase-3.md`](./phase-3.md) |
| 4   | Texte : recoller les lignes coupées du `text/plain`        | [`phase-4.md`](./phase-4.md) |
| 5   | Export réel sur Dovecot avec `skip_signature_images: true` | [`phase-5.md`](./phase-5.md) |

## Livraison sans la phase 6 (2026-11-11)

Les phases 1 à 5 sont faites et livrées. L'ancienne phase 6, l'envoi du seed entre les deux comptes Gmail, est sortie de ce plan à la demande de l'utilisateur : elle exige les mots de passe d'application des comptes A et B, que seul l'utilisateur peut créer. Elle devient la tâche [`2026_11_11_campagne-gmail-reelle`](../2026_11_11_campagne-gmail-reelle/plan.md), bloquée sur ces mots de passe. L'issue #19 reste ouverte pour cette partie.

## Resources

| Source | Verified |
| ------ | -------- |
| https://docs.rs/lettre (`Transport::send_raw(&Envelope, &[u8])`) | Envoi SMTP d'octets MIME déjà composés, sans recomposition ; dépendance de dev uniquement. À confirmer à l'implémentation sur la version courante |
| https://www.rfc-editor.org/rfc/rfc3676 | `format=flowed` : une ligne terminée par une espace se recolle à la suivante ; `delsp=yes` retire cette espace ; le séparateur `-- ` et les lignes `>` ont leurs propres règles |
| `~/.cargo/registry/src/*/imap-3.0.0-alpha.15/src/client.rs:1210` | `Session::append` → `AppendCmd` (`flag`, `internal_date`, `finish`) : le seed IMAP n'ajoute aucune dépendance |

## Decisions

| Decision | Why |
| -------- | --- |
| Le seed est un jeu de `.eml` versionnés qui reproduisent la structure MIME de Gmail, générés par un script déterministe. Ils sont copiés tels quels sur Dovecot et envoyés en SMTP brut entre les comptes A et B | La structure dépend du logiciel de l'expéditeur : un script ne peut qu'imiter Gmail, mais `send_raw` transmet ce MIME sans le recomposer. L'envoi réel ajoute la livraison Gmail (en-têtes, Spam, Sent Mail, fils) que `APPEND` ne testerait pas |
| Le gabarit MIME suit la structure décrite dans #14, faute d'original disponible en phase 1 ; il sera confirmé par l'envoi réel (tâche `2026_11_11_campagne-gmail-reelle`) avec un original Gmail (compte A vers compte B, capture collée, « Afficher l'original »), et chaque écart devient une fixture | Des fixtures écrites d'après la documentation peuvent manquer exactement la structure qui fait perdre l'image. L'original n'est accessible qu'une fois les mots de passe d'application en place |
| Aucune adresse ni aucun mot de passe dans le dépôt : comptes A et B par `EMAIL_TO_MARKDOWN_GMAIL_{A,B}_{USER,PASSWORD}`, test lancé en local seulement | Ce sont des boîtes personnelles. Mettre leurs mots de passe d'application dans les secrets GitHub reste une décision de l'utilisateur, hors de ce plan |
| Les citations, transferts et signatures texte sont conservés sans perte dans le corps ; découpage de fils et nettoyage relèvent d'obs-mail | Une note exportée est souvent la seule copie d'un message cité. Supprimer ou découper à l'export doublerait `split` et `quote_clean`, qui ont déjà leurs garde-fous de perte |
| Un module d'assertions partagé `tests/common/gmail_seed.rs`, appelé par les trois harnais (local, Dovecot, Gmail) | Trois appelants vérifient la même chose (règle de trois). Le projet n'avait pas jusqu'ici de helper de test partagé (`testing.md`). Cette exception est consignée dans la mémoire |
| Les cas qui échouent aujourd'hui sont écrits dès la phase 1 et marqués `#[ignore = "#NN …"]`, puis réactivés par la phase qui les corrige | Chaque phase reste livrable avec une CI verte. Les tests ignorés constituent la reproduction attendue par #19 |
| Un marqueur `[image: …]` est apparié aux `<img>` du HTML (locales ou distantes), ou à défaut aux parties image ; une image distante n'est jamais intégrée, seulement liée ou retirée si c'est une signature | Couvre C3-M8 et C4-M12 par une seule règle, et n'ouvre pas Obsidian au chargement de ressources distantes (pixels de suivi) |
| Le garde-fou ne bloque un marqueur que s'il n'a aucune contrepartie | Une signature distante ne doit pas empêcher la suppression ; un message tronqué (C3-M9) doit l'empêcher |
| Les défauts de rendu sans issue dédiée forment une phase à part, avant celle du texte | Leur correction peut toucher `unwrap_lines` (en-tête de transfert) ; la phase texte, qui étend cette fonction, part ainsi d'un état vert |
