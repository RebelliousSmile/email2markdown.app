# Signaler un *bug*

Un bon signalement permet de corriger une anomalie en quelques minutes au lieu de plusieurs échanges. Ce chapitre décrit où signaler, quoi vérifier avant d'écrire, et quoi mettre dans le message.

## Où signaler

Les signalements se font dans les **issues** du dépôt GitHub du projet, à l'adresse `github.com/RebelliousSmile/email2markdown.app/issues`. Il faut un compte GitHub gratuit.

Le dépôt ne propose pas de modèle de ticket : vous rédigez librement, en suivant le plan de ce chapitre. Les discussions GitHub ne sont pas activées ; une simple question se pose elle aussi dans les issues.

## Avant d'écrire

1. **Cherchez un ticket existant.** Tapez un mot du message d'erreur dans la barre de recherche des issues, ouvertes et fermées. Si le problème est déjà décrit, ajoutez un commentaire plutôt qu'un doublon.
2. **Vérifiez la version.** La commande `email-to-markdown --version` affiche le numéro installé. Comparez-le à la dernière version publiée : l'anomalie est peut-être déjà corrigée.
3. **Testez la connexion du compte.** Dans la fenêtre des paramètres, onglet **Comptes**, le bouton **Tester la connexion** écarte une erreur de configuration.

## Ce que doit contenir le ticket

| Élément | Exemple |
| ------- | ------- |
| Titre court et précis | Gmail : les images collées disparaissent de la note |
| Version | `0.29.0` |
| Système | Windows 11, Ubuntu 24.04… |
| Forme utilisée | ligne de commande ou application de barre système |
| Ce que vous faisiez | commande lancée, ou action dans la fenêtre |
| Ce qui s'est passé | message d'erreur copié tel quel |
| Ce que vous attendiez | la note devait contenir l'image |
| Fournisseur de messagerie | Gmail, Outlook, serveur autohébergé |

Pour obtenir plus de détails sur les échanges avec le serveur, relancez la commande avec l'option `--debug`, puis copiez les lignes pertinentes.

`email-to-markdown export -a Perso --debug`

## Protéger vos données

> **Attention** Un ticket GitHub est public. Relisez-le avant de l'envoyer.

- Ne transmettez **jamais** un mot de passe, ni le contenu du fichier `.env`.
- Remplacez les adresses de courriel, les noms de serveur privés et les chemins personnels par des exemples (`camille@exemple.org`).
- Ne collez pas le contenu d'un message réel. Décrivez sa structure (message HTML avec une pièce jointe, par exemple) ou recopiez un message de test.
- Floutez les captures d'écran avant de les joindre.

## Choisir l'étiquette

Les mainteneurs posent les étiquettes, mais vous pouvez proposer la bonne dans le texte.

| Étiquette | Quand l'utiliser |
| --------- | ---------------- |
| `bug` | l'application se comporte autrement que ce que le guide décrit |
| `enhancement` | vous demandez une fonction nouvelle |
| `documentation` | une erreur ou un manque dans un guide ou dans le README |
| `question` | vous cherchez à comprendre, sans anomalie avérée |

## Après l'envoi

Une réponse peut demander une précision : répondez dans le même ticket pour garder l'historique. Quand une correction est publiée, le ticket est fermé avec la version concernée. Si le problème persiste après la mise à jour, dites-le dans le ticket au lieu d'en ouvrir un nouveau.
