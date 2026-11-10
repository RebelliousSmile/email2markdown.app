# Corpus Gmail seed

Ce dossier produit le corpus de la campagne Gmail (issue #19, avec #14, #15 et #18). Il compte cinq conversations et treize messages. Ils ont la forme de ce que Gmail livre en IMAP : captures collées, signature avec logo, listes, liens, réponses, transferts et texte seul.

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

Le plan prévoyait de caler les gabarits sur des originaux téléchargés depuis Gmail (« Afficher l'original »). Aucun original n'était disponible lors de la phase 1. Le corpus suit donc la structure décrite dans #14 : `multipart/related` > `multipart/alternative` + `image/png`, des `Content-ID: <ii_…>` et `X-Attachment-Id`, et un balisage HTML `div dir="ltr"`, `gmail_signature`, `gmail_quote`. Ce calage reste à confirmer quand l'envoi réel de la phase 6 sera fait.

Aucune donnée réelle n'est versionnée. Les adresses sont `alice@example.com`, `bob@example.com` et `claire@example.org`, et les sujets portent l'étiquette `[email-to-markdown seed fixture]`. Les `Message-ID` ont la forme `<seed-cN-mN.fixture@email-to-markdown.test>`.

Pour un envoi réel, `substitute(raw, run, from, to)` remplace `From`, `To` et l'étiquette `fixture` du sujet et des `Message-ID`, `In-Reply-To` et `References`. Seul le bloc d'en-têtes de premier niveau change. Le corps garde tous ses octets.

## Cas couverts

| Message | Structure | Ce qu'il éprouve | Issue | État |
| ------- | --------- | ---------------- | ----- | ---- |
| C1-M1 | mixed > related > alternative + 2 captures + logo et 2 icônes de signature | listes ordonnée et imbriquée, liens avec libellé, URL nue, URL à parenthèses, `mailto:`, signature mise en quarantaine | #19 | vert |
| C1-M2 | réponse, captures de M1 réintégrées sous de nouveaux `cid:` | capture citée écrite et liée, citation intacte | #19 | vert |
| C1-M3 | réponse, un même `cid:` référencé deux fois | un seul fichier, citation sur deux niveaux | #19 | vert |
| C2-M4 | related à la racine, image sans `Content-Disposition` | QR code seul | #14 | vert |
| C2-M5 | mixed > related + PDF + JPEG + PNG au nom RFC 2231 | pièces jointes, nom accentué | #19 | vert |
| C2-M6 | related > alternative + GIF animé | corps réduit à une image | #19 | vert |
| C3-M7 | `text/plain; format=flowed; delsp=yes` + PNG joint | items de liste sur deux lignes, guillemet en début de ligne, adresse sur trois lignes | #18 | vert |
| C3-M8 | `text/plain` coupé à 72 colonnes + 2 PNG joints | deux marqueurs `[image: image.png]` | #15 | vert |
| C3-M9 | `text/plain` seul | marqueur sans partie image (message tronqué), message retenu sur le serveur | garde-fou | vert |
| C4-M10 | transfert en ligne, related + 2 captures | en-tête de transfert, tableau HTML | #19 | vert |
| C4-M11 | mixed > alternative + `message/rfc822` qui contient une capture | transfert en pièce jointe | #19 | vert |
| C4-M12 | réponse en `multipart/alternative` sans `cid:` | sujet en encoded-word avec emoji, URL de plus de 76 caractères, `*`, `_`, `[x]` | #15 | vert |
| C5-M13 | related > `text/plain` + PNG sans `Content-Disposition` ni nom | capture d'un corps texte seul, sans `cid:` | #14 | vert |

Un test ignoré reproduit un défaut constaté. Son attribut `#[ignore = "#NN …"]` nomme l'issue, et un commentaire au-dessus décrit l'échec observé. Les attentes du catalogue décrivent le résultat voulu : un correctif retire l'`#[ignore]`, sans jamais assouplir l'attente.

### Écarts par rapport au tableau du plan

- **URL de C4-M12** : elle tient sur une seule ligne de plus de 76 caractères. Gmail ne coupe pas une URL dans le `text/plain`, c'est le reste du texte qui est coupé autour.
- **Signature de C4-M12** : ses images sont distantes (`https://example.com/sig/…`) dans le HTML, et apparaissent comme `[image: logo.png]` dans le `text/plain`. Comme le HTML ne contient aucun `cid:`, l'export garde le `text/plain`, et les marqueurs y restent.
- **C1-M3** ne réintègre pas les captures citées de M2. Leurs `cid:` restent sans partie, et l'export les retire sans bruit.
- **C2-M4** ne reproduit pas la perte décrite dans #14 : la capture y est référencée par le HTML, donc écrite et liée. C5-M13, ajouté en phase 2, la reproduit.

## Cause de #14 (phase 2)

Dans C5-M13, aucun `cid:` ne référence la capture, donc l'export retient le `text/plain`. La capture porte un `Content-ID`, mais ni `Content-Disposition` ni nom. Or `extract_attachments` sautait toute partie sans disposition, images comprises. Il ne restait donc que le marqueur `[image: image.png]` : pas d'`attachments`, pas de `### Images :`, rien dans `_skipped/`. Avec `delete_after_export`, le message disparaissait du serveur.

Depuis la phase 2 :

- toute partie image est écrite à côté de la note, ou mise en quarantaine, même sans disposition ;
- chaque marqueur `[image: …]` est relié à son image, dans l'ordre, ou retiré quand il désigne une image distante de signature ;
- une image annoncée mais non conservée inscrit `keep_on_server` dans la note, et le message reste sur le serveur (C3-M9).

## Défauts constatés en phase 1

- **URL à parenthèses** (C1-M1) : `https://fr.wikipedia.org/wiki/Recette_(informatique)` devenait `Recette_/(informatique/`, suivie d'une `)` parasite. Corrigé en phase 3 : htmd échappe les parenthèses de la destination (`Recette_\(informatique\)`), et `extract_links` coupait le lien à la première `)`. La destination accepte désormais les échappements et une paire de parenthèses, puis elle est déséchappée.
- **Liste imbriquée** (C1-M1) : le sous-item n'était indenté que d'un espace (` * menu`), et l'imbrication disparaissait dans une citation. Corrigé en phase 3 : `collapse_whitespace` réduisait l'indentation de htmd à un espace. Il recalcule maintenant l'indentation de chaque item d'après son parent (largeur du marqueur plus un), y compris après `> `.
- **Nom RFC 2231** (C2-M5) : la pièce jointe était écrite sous `UTF-8''capture%20d%27%C3%A9cran%20%C3%A9t%C3%A9.png`. Corrigé en phase 3 : le nom vient des paramètres déjà décodés par mailparse.
- **Format flowed** (C3-M7) : les items de liste gardaient leurs coupures, et une ligne commençait par `«`. Corrigé en phase 4 (#18) : le `text/plain` déclaré `format=flowed` est recollé selon la RFC 3676 dès sa lecture (`cleaner::unflow`). Sans déclaration, `unwrap_lines` recolle aussi la suite d'un item de liste et une ligne qui commence par `»`.
- **Marqueurs `[image: …]`** (C3-M8, C4-M12) : ils n'étaient pas reliés aux images, et un marqueur pouvait être coupé sur deux lignes. Corrigé en phase 2.
- **En-tête de transfert** (C4-M10) : `De :`, `Date:`, `Subject:` et `To:` fusionnaient sur une seule ligne. Corrigé en phase 3 : htmd sépare bien les champs, c'est `unwrap_lines` qui les recollait. Une ligne de champ d'en-tête n'est plus jamais recollée.
- **Quarantaine dans un export partagé** : la quarantaine de C1-M3 allait dans le dossier `_skipped/` du nom de base de C1-M1 (`logo_2.png`), et non dans celui de sa note suffixée. Corrigé en phase 3 : l'export contextuel met les images en attente, puis les range sous le nom final de la note installée. `assert_seed_note` vérifie ce rattachement.
