# Nettoyage du corps et format des *notes*

Ce chapitre suit le texte d'un message depuis l'arbre MIME jusqu'au fichier Markdown : choix de la partie lue, conversion du HTML, nettoyage par `cleaner::clean`, puis structure de la note écrite.

## Choisir la partie à lire

`collect_text_bodies` parcourt l'arbre MIME et rassemble les parties de texte. Un `text/plain` au format `flowed` passe d'abord par `cleaner::unflow`. Le choix entre les deux représentations suit trois règles :

1. le HTML l'emporte si sa conversion en Markdown contient une image `cid:` (image intégrée au message) ;
2. sinon, un `text/plain` lisible l'emporte ;
3. sinon, le HTML sert de repli.

`extract_body_reporting` applique ce choix. Quand la conversion d'un corps HTML échoue, elle consigne l'erreur dans une liste transmise par l'appelant et conserve le texte non converti plutôt que de le perdre.

## Convertir le HTML

`try_html_to_markdown` s'appuie sur `htmd` avec des gestionnaires personnalisés.

| Élément | Traitement |
| --- | --- |
| `head`, `style`, `script`, `noscript`, `title` | ignorés (`skip_tags`) |
| `td`, `th`, `tr` | `cell_handler` et `row_handler` : une ligne de tableau de mise en page reste sur une ligne |
| `span`, `font` | `inline_style_handler` : le gras issu de `font-weight` est conservé |
| `p`, `div` | `block_style_handler` |
| `a` | `anchor_handler` |
| `img` | pixel espion supprimé (`is_tracking_pixel`) ; image `cid:` d'une signature supprimée |

`html_layout.rs` reconnaît les signatures par la classe `gmail_signature` ou `moz-signature`, ou par l'identifiant `Signature`, `AppleMailSignature` ou `ms-outlook-mobile-signature`. Après la conversion, `collapse_blank_lines` réduit les lignes vides répétées.

## Le pipeline `cleaner::clean`

`clean(body)` renvoie un `CleanResult { body, social_links }`. Six étapes s'enchaînent, et leur ordre est le résultat d'un essai, non d'une convention.

`let (body, social_links) = extract_social_footer(&body);`\
`let body = reattach_urls(&body);`\
`let body = unwrap_lines(&body);`\
`let body = extract_links(&body);`\
`let body = decontaminate_ref_urls(&body);`

| Étape | Fonctions | Effet |
| --- | --- | --- |
| 1. Fidélité du décodage | `decode_residual_qp`, `decode_html_entities`, `strip_invisible_chars` | répare les restes de quoted-printable, les entités et les caractères invisibles |
| 2. Pied de page social | `extract_social_footer` | retire les liens de réseaux sociaux empilés et les range dans `social_links` |
| 3. Rattachement des URL | `reattach_urls` | recolle une URL coupée, seulement si la fin de la ligne précédente ressemble à une URL |
| 4. Dépliage | `unwrap_lines` | défait la coupure à 80 caractères |
| 5. Liens | `extract_links`, `decontaminate_ref_urls` | convertit les liens en références numérotées puis retire les paramètres de suivi |
| 6. Espaces | `collapse_whitespace`, `trim_trailing` | normalise les blancs |

`detect_mojibake` ne corrige rien : il affiche seulement un avertissement sur la sortie d'erreur quand le texte semble mal décodé.

> **Attention** : l'étape 2 précède volontairement l'étape 3. Le plan d'origine plaçait `reattach_urls` en premier, mais la parenthèse finale des liens sociaux empilés ressemblait alors à la fin d'une URL coupée. Le détecteur de pied de page, qui repose sur des lignes de la forme `^\[.+\]\(.+\)$`, doit voir la mise en page intacte. Un commentaire dans `clean` l'explique ; conservez-le si vous touchez à l'ordre.

Les constantes `UNWRAP_MIN_LEN` (60), `UNWRAP_TERMINAL_CHARS` et `SOCIAL_DOMAINS` règlent le dépliage et la détection sociale. Toutes les expressions régulières du module sont des `static LazyLock<Regex>`.

### Valider l'ordre

> **À retenir** : un pipeline d'au moins trois transformations dont l'ordre compte doit être validé par un test de bout en bout sur des données réalistes, livré dans la même phase que le câblage. Un test unitaire par étape ne détecte pas les interactions. Les tests de bout en bout associent une assertion inclusive et une assertion exclusive.

`assert!(body.contains("disponibles dans votre secteur"));`\
`assert!(!body.contains("dansvotre"));`

La première assertion seule laisserait passer une corruption collée au texte attendu ; la seconde la détecte.

## Les citations

Après l'extraction du corps, `limit_quote_depth` (module `utils`) retire les lignes dont le nombre de `>` dépasse `quote_depth`. Le remplacement d'une citation par un lien vers la note d'origine est un traitement distinct, fait au moment du tri (chapitre 5).

## Anatomie d'une note

Une note est un fichier `.md` composé d'un en-tête YAML entre deux lignes `---`, puis du corps.

### L'en-tête YAML

`EmailFrontmatter` regroupe les champs écrits :

| Champ | Contenu |
| --- | --- |
| `subject_hash` | six caractères du MD5 du sujet |
| `tags` | `["email"]` |
| `attachments` | noms des pièces jointes écrites à côté de la note |
| `source` | `LocalSourceProof` : identité et emplacement du message sur le serveur |
| `keep_on_server` | motif de conservation sur le serveur, s'il y en a un |
| `email_type` | `direct`, `group`, `newsletter`, `mailing_list` ou `unknown` |
| `social_links` | liens sociaux extraits du pied de page |

Les champs `from`, `to`, `date` et `subject` existent encore dans la structure, mais en lecture seule, pour les anciennes notes : leur écriture est omise quand ils sont vides, et l'en-tête du message vit désormais dans le bloc `email`.

> **Bon à savoir** : `source` est la preuve qui autorise plus tard la suppression du message sur le serveur. Une note sans preuve complète ne déclenche jamais de suppression (chapitre 6).

### Le bloc `email`

Le corps de la note commence par un bloc de code dont la langue est `email` et dont le contenu est du TOML. Il est produit par `email_block::render(header, body)`.

- La clôture est composée de backticks, plus longue que toute suite de backticks du corps (trois au minimum), suivie de `email`, ce qui évite qu'un message qui contient du code casse le bloc.
- Les clés sont écrites dans l'ordre : `subject`, `from`, `to` (liste), `cc` et `bcc` (seulement s'ils ne sont pas vides), `date`, `body`.
- `date` est une date TOML nue quand elle est au format RFC 3339, sinon une chaîne.
- `body` est une chaîne littérale multiligne `'''…'''` ; si le corps contient `'''`, une variante `"""` échappe `\`, `"` et les caractères de contrôle sous la forme `\uXXXX`.

À la lecture, `block_source` retrouve le premier bloc `email` ; `read_header`, `read_body` et `note_header_field(s)` y reviennent quand l'en-tête YAML est absent, ce qui garde les anciennes notes lisibles.

> **Renvoi** : le dépôt contient un miroir Python de ce format, `tools/src/email_block.py` (chapitre 8).

### Les sections de médias

`body_media(body)` extrait du corps les images et les liens (`BodyMedia`), après avoir converti les liens de style référence en liens en ligne par `inline_references`. Une image `data:` est exclue ; les liens doivent être distants (`http` ou `https`) et sont dédoublonnés par cible.

`render_media` produit deux sections `### Images :` et `### Liens :`. Une image distante s’écrit `!` suivi de `[texte](source)`, une image locale `![[fichier]]` ; un lien s'écrit `- [libellé](cible)`. `local_link(nom, embed)` fabrique `[[nom]]` ou `![[nom]]`.

### Pièces jointes et invitations

Après les médias, la note liste ses pièces jointes sous le titre `### Pieces jointes :` (orthographe exacte du code, sans accent). Une image déjà affichée n'y est pas répétée.

`calendar::find_invitation` cherche la première partie `text/calendar`, `application/ics` ou pièce jointe `.ics`. `calendar::render` produit une section `### <titre> :` listant le titre, le moment, le lieu, l'organisateur. La description est omise si le corps la contient déjà ; récurrence, participants et alarmes restent dans le fichier `.ics`.

L'ordre du corps final est donc : bloc `email`, invitation, section de médias, section de pièces jointes.

### Les citations remplacées

Après le tri, `quote_link` peut remplacer une citation (une attribution du type « Le … a écrit : » suivie de lignes en `>`) par la ligne `Message cité : [[note]]` et ajouter une section `### Messages cités :`. Les règles sont volontairement prudentes : le remplacement n'a lieu que si une seule note correspond à la citation, sans interprétation. Les constantes `TEXT_KEY_LEN` (40) et `TEXT_KEY_MIN` (12) fixent la longueur de la clé de comparaison du texte.

## Réparer ou migrer des notes

`quote_link::link_vault(root, backup_dir)` traite un coffre existant. Sans dossier de sauvegarde, il ne modifie rien ; avec `--apply`, la sauvegarde doit se trouver hors du coffre. Le parcours ne suit pas les liens symboliques et saute les dossiers dont le nom commence par un point. Il renvoie un `VaultReport` (`scanned`, `rewritten`, `linked`, `left`), où `left` indique par catégorie (`NoDate`, `NoNote`, `Multiple`, `Interleaved`, `NoBody`) pourquoi une citation reste en texte.
