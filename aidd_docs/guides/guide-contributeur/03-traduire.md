# Se charger des *traductions*

Aujourd'hui, `email-to-markdown` n'existe qu'en français : les fenêtres, les menus, les messages et les guides sont rédigés dans cette langue. Ce chapitre décrit ce qu'il est possible de traduire et comment s'y prendre sans perdre de travail.

## Ce qui est à traduire

| Élément | Où il se trouve | Remarque |
| ------- | --------------- | -------- |
| README | racine du dépôt | premier contact des visiteurs, à traduire en priorité |
| Guides | guide de l'utilisateur, guide du contributeur | documents PDF produits à partir de fichiers Markdown |
| Fenêtres de l'application | fichiers `assets/*.html` | textes écrits directement dans les gabarits |
| Menus et messages | code source Rust | textes écrits directement dans le code |
| Messages d'erreur de la ligne de commande | code source Rust | souvent déjà en anglais |

> **À retenir** L'application n'a pas encore de système de traduction : les textes de l'interface sont dans le code. Traduire l'interface demande d'abord un travail de préparation côté code, que les mainteneurs doivent cadrer avec vous.

## Commencer par un ticket

Avant de traduire, ouvrez un ticket dans les issues du dépôt (voir le chapitre précédent) :

1. Indiquez la **langue** visée et l'**élément** que vous proposez de traduire.
2. Dites si vous traduisez seul ou à plusieurs.
3. Attendez la réponse d'un mainteneur : elle précise le format de livraison et évite qu'une autre personne fasse la même chose.

Le ticket sert aussi de lieu de suivi : l'avancement, les questions de vocabulaire et les relectures s'y retrouvent.

## Bonnes pratiques

- **Traduisez le sens, pas les mots.** Une phrase naturelle vaut mieux qu'un calque.
- **Gardez le vocabulaire cohérent.** Dressez la liste des termes clés (note, destination, compte, export…) et tenez-vous-y du début à la fin.
- **Ne traduisez pas ce qui est technique.** Les commandes, les noms d'options (`--debug`), les noms de fichiers (`accounts.yaml`), les champs YAML et les chemins restent tels quels.
- **Conservez les noms de l'interface.** Quand le guide cite un bouton, utilisez le libellé que l'application affiche dans la langue concernée.
- **Respectez le Markdown.** Titres, listes, tableaux et blocs de code gardent leur structure : seuls les textes changent.

## Relecture et maintenance

Une traduction vit avec le logiciel. À chaque nouvelle version, des textes changent ou apparaissent :

- relisez le journal des changements pour repérer les ajouts ;
- signalez dans le ticket les passages devenus faux ;
- indiquez la version du logiciel à laquelle correspond votre traduction.

Une traduction qui n'est plus tenue à jour peut induire en erreur. Si vous ne pouvez plus la maintenir, dites-le dans le ticket pour qu'une autre personne reprenne.
