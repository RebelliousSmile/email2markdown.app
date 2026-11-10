# Corpus Gmail seed

Ce dossier produit le corpus de la campagne Gmail (issue #19, avec #14, #15 et #18). Il compte quatre conversations et douze messages. Ils ont la forme de ce que Gmail livre en IMAP : captures collées, signature avec logo, listes, liens, réponses, transferts et texte seul.

- `build_seed.py` : générateur déterministe, bibliothèque standard Python uniquement.
- `../seed/gmail/*.eml` : sa sortie, versionnée. Le sous-dossier n'est pas copié par la boucle `/seed/*.eml` de `compose.yaml`, donc Dovecot ne le voit pas.
- `tests/common/gmail_seed.rs` : ce que chaque message doit produire une fois exporté (`CATALOGUE`), la vérification `assert_seed_note` et la substitution utilisée pour un envoi réel.
- `tests/rust_tests.rs`, module `gmail_seed_tests` : un test par message, plus la vérification du générateur et de la substitution.

## Régénérer

```bash
python -I tests/fixtures/imap/gmail/build_seed.py
```

Sans argument, le script écrit dans `tests/fixtures/imap/seed/gmail/`. Avec un argument, il écrit dans ce dossier. La sortie ne change jamais d'une exécution à l'autre : images en bruit à graine fixe, dates fixes, frontières MIME fixes.

Le test `test_gmail_seed_corpus_matches_generator` régénère le corpus dans un dossier temporaire et compare les octets. Il cherche `python3`, puis `python`, et échoue explicitement si aucun des deux n'est disponible. Après une modification de `build_seed.py`, il faut donc régénérer puis versionner les `.eml`.

Les `.eml` sont en CRLF, comme sur le réseau. `.gitattributes` les déclare `-text` pour que `core.autocrlf` ne les convertisse pas.

## Origine du gabarit MIME

Le plan prévoyait de caler les gabarits sur des originaux téléchargés depuis Gmail (« Afficher l'original »). Aucun original n'était disponible lors de la phase 1. Le corpus suit donc la structure décrite dans #14 : `multipart/related` > `multipart/alternative` + `image/png`, des `Content-ID: <ii_…>` et `X-Attachment-Id`, et un balisage HTML `div dir="ltr"`, `gmail_signature`, `gmail_quote`. Ce calage reste à confirmer quand l'envoi réel de la phase 5 sera fait.

Aucune donnée réelle n'est versionnée. Les adresses sont `alice@example.com`, `bob@example.com` et `claire@example.org`, et les sujets portent l'étiquette `[email-to-markdown seed fixture]`. Les `Message-ID` ont la forme `<seed-cN-mN.fixture@email-to-markdown.test>`.

Pour un envoi réel, `substitute(raw, run, from, to)` remplace `From`, `To` et l'étiquette `fixture` du sujet et des `Message-ID`, `In-Reply-To` et `References`. Seul le bloc d'en-têtes de premier niveau change. Le corps garde tous ses octets.

## Cas couverts

| Message | Structure | Ce qu'il éprouve | Issue | État |
| ------- | --------- | ---------------- | ----- | ---- |
| C1-M1 | mixed > related > alternative + 2 captures + logo et 2 icônes de signature | listes ordonnée et imbriquée, liens avec libellé, URL nue, URL à parenthèses, `mailto:`, signature mise en quarantaine | #19 | ignoré |
| C1-M2 | réponse, captures de M1 réintégrées sous de nouveaux `cid:` | capture citée écrite et liée, citation intacte | #19 | vert |
| C1-M3 | réponse, un même `cid:` référencé deux fois | un seul fichier, citation sur deux niveaux | #19 | vert |
| C2-M4 | related à la racine, image sans `Content-Disposition` | QR code seul | #14 | vert |
| C2-M5 | mixed > related + PDF + JPEG + PNG au nom RFC 2231 | pièces jointes, nom accentué | #19 | ignoré |
| C2-M6 | related > alternative + GIF animé | corps réduit à une image | #19 | vert |
| C3-M7 | `text/plain; format=flowed; delsp=yes` + PNG joint | items de liste sur deux lignes, guillemet en début de ligne, adresse sur trois lignes | #18 | ignoré |
| C3-M8 | `text/plain` coupé à 72 colonnes + 2 PNG joints | deux marqueurs `[image: image.png]` | #15 | ignoré |
| C3-M9 | `text/plain` seul | marqueur sans partie image (message tronqué) | garde-fou | vert |
| C4-M10 | transfert en ligne, related + 2 captures | en-tête de transfert, tableau HTML | #19 | ignoré |
| C4-M11 | mixed > alternative + `message/rfc822` qui contient une capture | transfert en pièce jointe | #19 | vert |
| C4-M12 | réponse en `multipart/alternative` sans `cid:` | sujet en encoded-word avec emoji, URL de plus de 76 caractères, `*`, `_`, `[x]` | #15 | ignoré |

Un test ignoré reproduit un défaut constaté. Son attribut `#[ignore = "#NN …"]` nomme l'issue, et un commentaire au-dessus décrit l'échec observé. Les attentes du catalogue décrivent le résultat voulu : un correctif retire l'`#[ignore]`, sans jamais assouplir l'attente.

### Écarts par rapport au tableau du plan

- **URL de C4-M12** : elle tient sur une seule ligne de plus de 76 caractères. Gmail ne coupe pas une URL dans le `text/plain`, c'est le reste du texte qui est coupé autour.
- **Signature de C4-M12** : ses images sont distantes (`https://example.com/sig/…`) dans le HTML, et apparaissent comme `[image: logo.png]` dans le `text/plain`. Comme le HTML ne contient aucun `cid:`, l'export garde le `text/plain`, et les marqueurs y restent.
- **C1-M3** ne réintègre pas les captures citées de M2. Leurs `cid:` restent sans partie, et l'export les retire sans bruit.
- **C2-M4** ne reproduit pas la perte décrite dans #14 : la capture y est référencée par le HTML, donc écrite et liée. La cause de la réouverture de #14 reste à trouver avec une autre structure.

## Défauts constatés en phase 1

- **URL à parenthèses** (C1-M1) : `https://fr.wikipedia.org/wiki/Recette_(informatique)` devient `Recette_/(informatique/`, suivie d'une `)` parasite.
- **Liste imbriquée** (C1-M1) : le sous-item n'est indenté que d'un espace (` * menu`), et l'imbrication disparaît dans une citation.
- **Nom RFC 2231** (C2-M5) : la pièce jointe est écrite sous `UTF-8''capture%20d%27%C3%A9cran%20%C3%A9t%C3%A9.png`.
- **Format flowed** (C3-M7) : les items de liste gardent leurs coupures, et une ligne commence par `«`.
- **Marqueurs `[image: …]`** (C3-M8, C4-M12) : ils ne sont pas reliés aux images, et un marqueur peut être coupé sur deux lignes.
- **En-tête de transfert** (C4-M10) : `De :`, `Date:`, `Subject:` et `To:` fusionnent sur une seule ligne.
- **Quarantaine dans un export partagé** : la quarantaine de C1-M3 va dans le dossier `_skipped/` du nom de base de C1-M1 (`logo_2.png`), et non dans celui de sa note suffixée. Rien n'est perdu, mais le rattachement est trompeur.
