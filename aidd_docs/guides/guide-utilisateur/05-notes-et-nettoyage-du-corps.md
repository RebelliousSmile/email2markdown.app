# Notes et nettoyage du *corps*

Un courriel brut contient beaucoup de bruit : retours à la ligne imposés, liens interminables, pieds de page de réseaux sociaux. Ce chapitre décrit la forme des notes produites et les traitements qui rendent leur texte lisible.

## Anatomie d'une note

Une note comporte, dans l'ordre : un en-tête technique au format YAML (frontmatter), un bloc `email` qui porte les en-têtes et le texte du message, puis des sections de liens.

### Le frontmatter

L'en-tête YAML ne reprend pas l'objet ni l'expéditeur : ceux-ci se trouvent dans le bloc `email`. Il contient les informations utiles aux outils.

| Clé | Contenu |
| --- | --- |
| `subject_hash` | Empreinte courte de six caractères calculée sur l'objet ; `no-subject` si l'objet est vide |
| `tags` | Toujours `email` ; le dossier IMAP d'origine n'est pas ajouté comme étiquette |
| `attachments` | Liste des fichiers joints ; absente s'il n'y en a pas |
| `source` | Preuve d'origine du message sur le serveur, utilisée par la suppression sécurisée |
| `keep_on_server` | Présent si le message doit rester sur le serveur, avec la raison |
| `email_type` | Nature du message |
| `social_links` | Liens vers des réseaux sociaux extraits du pied de page |

> **Bon à savoir** Ne modifiez pas les clés `source` et `keep_on_server` à la main. La suppression sur le serveur s'appuie sur elles pour vérifier qu'elle cible bien le bon message.

### Le bloc email

Le bloc commence par trois accents graves suivis du mot `email`. Il contient les en-têtes sous forme de paires clé-valeur : `subject`, `from`, `to`, et si nécessaire `cc` et `bcc`, la date au format normalisé, puis le texte du message dans `body`.

`subject = "Réunion de lundi"`\
`from = "Camille Martin <camille@exemple.org>"`\
`to = ["vous@exemple.org"]`\
`cc = ["equipe@exemple.org"]`\
`date = 2026-05-04T08:30:00+02:00`\
`body = """`\
`Bonjour à tous,`

`La réunion est déplacée à 10 h.`\
`"""`

Ce format est pensé pour être affiché proprement par un greffon de coffre de notes, et pour être relu par d'autres programmes.

### Après le bloc

Selon le contenu du message, l'application ajoute après le bloc :

| Section | Contenu |
| --- | --- |
| Résumé d'invitation | Si le message contient une invitation de calendrier, un résumé en est donné ; le fichier `.ics` reste en pièce jointe |
| `### Images :` | Les images du message, sous forme de liens vers les fichiers enregistrés |
| `### Liens :` | Les liens web trouvés dans le texte |
| `### Pieces jointes :` | La liste des pièces jointes qui ne sont pas déjà affichées comme images |

Les images et pièces jointes locales sont référencées par des liens de coffre de notes, de la forme `[[nom du fichier]]`, ou `![[nom du fichier]]` pour afficher une image.

## Les pièces jointes

Les pièces jointes sont enregistrées à plat, dans le même dossier que la note. Chaque fichier est nommé `<date>_<nom d'origine>`, par exemple `2026-03-14_facture.pdf`, ce qui évite les collisions entre messages.

Les images intégrées dans le corps du message (identifiées par des références `cid:`) sont enregistrées elles aussi, puis reliées à la note.

### Les dossiers spéciaux

Trois dossiers au nom précédé d'un tiret bas accueillent ce qui ne doit pas être classé comme une note ordinaire :

| Dossier | Contenu |
| --- | --- |
| `_skipped` | Images de signature écartées, rangées dans un sous-dossier au nom de la note (quarantaine) |
| `_failed` | Éléments dont la conversion a échoué |
| `_deleted` | Pièces jointes des notes écartées lors de la revue du classement |

Ces dossiers, comme un éventuel dossier `contacts`, sont ignorés lors du classement. Rien n'y est détruit : vous pouvez les consulter et en récupérer le contenu.

Si l'option **Collecter les contacts** est active, un fichier CSV des contacts rencontrés est écrit dans le dossier `_local\contacts`, situé dans le répertoire d'export, à côté des dossiers des comptes.

> **Attention** Si la suppression sur le serveur est active, un message dont les pièces jointes sont illisibles n'est pas exporté : l'application refuse de supprimer un message dont elle n'a pas pu conserver tout le contenu. Il reste sur le serveur.

Un message dont une image annoncée n'a pas pu être conservée est lui aussi gardé sur le serveur, avec l'avertissement « Message conservé sur le serveur, image annoncée non conservée ».

## Le nettoyage du corps

Le texte d'un courriel subit une série de traitements dans un ordre précis. Chacun corrige un défaut courant.

| Étape | Traitement |
| --- | --- |
| 1 | Fidélité du décodage |
| 2 | Recollage des adresses web coupées |
| 3 | Extraction du pied de page des réseaux sociaux |
| 4 | Suppression des retours à la ligne imposés |
| 5 | Liens en références numérotées |
| 6 | Hygiène des espaces |

### 1. Fidélité du décodage

Cette étape élimine les résidus de codage : séquences `=E9` d'un codage ancien, entités HTML (`&amp;`, `&nbsp;`), caractères invisibles (espace de largeur nulle, trait d'union conditionnel, marque d'ordre des octets). Si des caractères mal décodés (`Ã©` pour `é`) subsistent, un avertissement est émis.

### 2. Recollage des adresses

Les logiciels de messagerie coupent parfois une longue adresse web sur deux lignes. L'application recolle ces fragments pour que le lien reste cliquable.

### 3. Pied de page des réseaux sociaux

Quand un bloc final contient au moins deux liens vers des réseaux sociaux (Facebook, LinkedIn, etc.), il est retiré du corps et ses liens sont rangés dans la clé `social_links` du frontmatter.

### 4. Retours à la ligne imposés

Beaucoup de messages sont coupés artificiellement vers 80 caractères. L'application rassemble ces lignes en paragraphes naturels, tout en respectant les listes et les citations.

### 5. Liens numérotés

Un lien long au milieu d'une phrase la rend illisible. L'application le remplace par une référence : le texte reste fluide, et les adresses sont regroupées en bas de la note.

`Consultez [le règlement][1] avant de répondre.`

`[1]: exemple.org/reglement`

Cette étape retire aussi les paramètres de suivi (ceux qui servent à pister l'ouverture d'un message) des adresses.

### 6. Hygiène des espaces

Dernier passage : les suites de lignes vides ou d'espaces sont réduites, les espaces en fin de ligne sont supprimés, et l'imbrication des listes est conservée.

## Les messages au format HTML

Certains messages n'ont pas de version texte : seul le HTML est fourni. L'application convertit alors le HTML en Markdown avant d'appliquer le nettoyage décrit ci-dessus.

Si d'anciennes notes sont restées au format HTML brut, l'outil **Fix HTML→Markdown** du menu **Outils** les reconvertit pour un compte donné.

## Les citations

Les réponses reprennent souvent l'intégralité des échanges précédents, imbriqués les uns dans les autres (lignes précédées d'un, deux, trois signes `>`). Le réglage `quote_depth` fixe le nombre de niveaux de citation conservés ; les lignes plus profondément citées sont supprimées de la note :

| Valeur | Effet |
| --- | --- |
| 0 | Pas de limitation : toutes les citations sont conservées |
| 1 (défaut) | Le texte du message et son premier niveau de citation sont conservés ; les niveaux plus profonds sont supprimés |
| 2 | Deux niveaux de citation sont conservés, et ainsi de suite |

Le réglage se trouve dans **Paramètres…** sous le nom « Profondeur des citations », au niveau général ou compte par compte.

Lors du classement, une note qui cite un message déjà présent dans votre coffre reçoit un lien vers la note citée. Un échec de cette liaison n'annule jamais le classement.

> **Renvoi** Le chapitre « Destinations et tri » explique comment les notes sont ensuite rangées, et la commande `link-quotes` du chapitre « Commandes en ligne » permet de relier rétroactivement les notes déjà classées.
