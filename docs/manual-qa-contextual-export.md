# Recette manuelle — export contextuel

Cette recette est un verrou de livraison. Elle doit être remplie sur les quatre
environnements nommés avant une release. Joindre une capture de la fenêtre de
sélection et une capture du menu du gestionnaire de fichiers aux artefacts de la
release.

## Préparation commune

1. Construire le binaire distribué avec `cargo build --release --features tray`.
2. Configurer `notes_dir`, au moins un compte IMAP et une destination physique
   exacte possédant une règle `correspondent`, `from` ou `domain`.
3. Exécuter `email-to-markdown shell install`, puis deux fois `shell status`.
4. Vérifier que les deux états sont identiques et indiquent `installé`.
5. Copier le binaire ailleurs et lancer `shell status` depuis la copie : l’ancien
   chemin doit être signalé comme périmé. `shell install` doit le réparer.

## Parcours fonctionnel

Pour chaque plateforme :

1. Déclencher l’action sur un dossier configuré contenant espace et caractère
   non ASCII dans son chemin.
2. Vérifier que le programme n’était pas lancé auparavant et qu’une seule fenêtre
   apparaît.
3. Choisir la boîte, rechercher, filtrer, sélectionner plusieurs lignes et
   confirmer le nombre exact.
4. Vérifier que la fenêtre se ferme après réussite et que le processus disparaît.
5. Déclencher l’action sur un dossier non configuré : aucun accès IMAP ne doit
   avoir lieu et un diagnostic doit être affiché.
6. Activer `delete_after_export` sur le compte fixture, convertir un message et
   vérifier que les témoins non sélectionnés restent sur le serveur.
7. Exécuter `shell uninstall` deux fois et vérifier qu’un artefact tiers placé à
   côté n’est pas supprimé.

## Matrice de preuve de release

| Environnement | Version testée | Menu sélection | Menu fond | Unicode | Conversion | Désinstallation | Capture/résultat |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Windows 11 / Explorer | à renseigner | à faire | à faire | à faire | à faire | à faire | à joindre |
| macOS courant / Finder | à renseigner | à faire | N/A | à faire | à faire | à faire | à joindre |
| Ubuntu 24.04 / Nautilus | à renseigner | à faire | à faire si URI fournie | à faire | à faire | à faire | à joindre |
| KDE Neon stable / Dolphin | à renseigner | à faire | selon version | à faire | à faire | à faire | à joindre |

## Gmail destructif protégé

Le workflow CI `gmail-destructive` ne s’exécute que manuellement, dans
l’environnement GitHub protégé du même nom. Le compte doit être dédié et contenir
exactement un message dont l’objet est `[email-to-markdown destructive fixture]`.
Les secrets requis sont `EMAIL_TO_MARKDOWN_GMAIL_USER`,
`EMAIL_TO_MARKDOWN_GMAIL_PASSWORD` et
`EMAIL_TO_MARKDOWN_GMAIL_FIXTURE_ADDRESS`.

## Dépendances d’interface

- Windows : WebView2 Runtime (présent par défaut sur Windows 11).
- macOS : WebKit système utilisé par Wry.
- Linux : WebKitGTK 4.1, GTK 3, AppIndicator, OpenSSL et `pkg-config`. Sur
  Ubuntu 24.04 : `libwebkit2gtk-4.1-dev libgtk-3-dev
  libayatana-appindicator3-dev librsvg2-dev libssl-dev pkg-config`.

L’action peut apparaître sur un dossier non configuré. C’est volontaire : le
binaire valide l’identité physique du dossier avant toute connexion IMAP.
