# Maquette doc-pdf

Cette maquette décrit la mise en page d'un document : page, grille, nuancier, styles, gabarits, pied et mentions. Le moteur de `doc-pdf` ne contient aucune valeur de marque : il lit ce dossier.

## L'utiliser

Un projet désigne la maquette par le champ `maquette` de son `documentation.config.json`, chemin relatif au fichier de configuration :

```json
{ "maquette": "../maquette-doc-pdf" }
```

## Ce qui se modifie

Seulement `maquette.json` et `logo.svg` (et `picto.json` s'il existe). Jamais le moteur, `run-indesign.ps1` ni les scripts ExtendScript. Les noms de styles, de gabarits, de cadres et de variables restent ceux du squelette : ils sont le vocabulaire du moteur.

## La créer ou la recaler : trois temps

La skill `doc-pdf:maquette` conduit la méthode.

1. **Modèle** : `maquette init <dossier>`, puis remplir `maquette.json` et poser `logo.svg`, générer les ICML et les modèles IDML du spécimen.
2. **Calage** : composer le spécimen dans InDesign, lire les pages, ajuster `maquette.json` un réglage à la fois, comparer avec l'essai précédent.
3. **Production** : brancher un projet (`init --maquette <dossier>`), puis `pdf compose` et `pdf update`.

Le dossier `specimen/` est un texte neutre qui exerce chaque style : il sert à caler sans toucher aux textes d'un projet. Ses sorties (`specimen/output/`) ne se versionnent pas.
