---
status: pending
---

# Instruction: Dupliquer un compte et renommer un dossier ignoré

## Architecture projection

> Tree of the final files. ✅ create · ✏️ modify · ❌ delete

```txt
.
├── assets/
    └── config_window.html        ✏️ bouton Dupliquer, showNewAccount(source), étiquette éditable sur place, styles
```

## User Journey

```mermaid
flowchart TD
  A[Liste des comptes] --> B[Clic Dupliquer sur Perso]
  B --> C[Formulaire Ajouter un compte prérempli : serveur, port, dossiers ignorés]
  C --> D[Saisie nom, identifiant, mot de passe]
  D --> E[Enregistrer : save_account create=true]
  E --> F[Nouveau compte en fin de liste]
  A --> G[Modifier un compte]
  G --> H[Clic sur l'étiquette Spam]
  H --> I[Champ texte en place, prérempli]
  I -- Entrée ou perte du focus --> J[Étiquette renommée]
  I -- Échap ou valeur vide --> K[Étiquette inchangée]
  J --> L[Enregistrer : ignored_folders mis à jour]
```

## Test Scope

```mermaid
---
title: Test scope
---
journey
  section Setup
    accounts.yaml avec Perso (imap.example, 993, dossiers Spam et Trash) et un réglage de comportement => fenêtre des paramètres ouverte: 5: system
  section Happy path
    clic Dupliquer sur Perso => formulaire Ajouter un compte avec serveur imap.example, port 993, étiquettes Spam et Trash, nom, identifiant et mot de passe vides, comportement sur Défaut: 5: browser
    saisie nom Perso2 et identifiant puis Enregistrer => Perso2 apparaît en fin de liste: 5: browser
    Modifier Perso, clic sur Spam, saisie Junk, Entrée => étiquettes Junk et Trash: 5: browser
    Enregistrer => accounts.yaml contient Junk et Trash pour Perso: 5: browser
  section Edge case - annulation
    étiquette en édition => Échap => étiquette d'origine conservée: 1: browser
  section Edge case - valeur vide
    étiquette en édition => effacer le texte puis Entrée => étiquette d'origine conservée: 1: browser
  section Edge case - doublon
    étiquettes Spam et Trash => renommer Spam en Trash => une seule étiquette Trash: 1: browser
  section Edge case - nom déjà pris
    duplication de Perso => nom Perso puis Enregistrer => erreur nom existant, rien n'est écrit: 1: browser
  section Teardown
    supprimer Perso2 et rétablir Spam => accounts.yaml d'origine: 5: browser
```

## Wireframe

```txt
┌──────────────────────────────────────────────────────────┐
│ (1) ┌──────────────────────────────────────────────────┐ │
│     │ [↑][↓]  Perso            [Dupliquer] [Modifier]  │ │
│     │         Supprimer la configuration               │ │
│     └──────────────────────────────────────────────────┘ │
├──────────────────────────────────────────────────────────┤
│ (2) Modifier le compte                                   │
│     Dossiers ignorés                                     │
│ (3) ( Spam × ) ( [Trash______] ) ( Archives × )          │
│     [Ajouter un dossier...] [+]                          │
└──────────────────────────────────────────────────────────┘
```

1. Ligne de compte : bouton Dupliquer à côté de Modifier.
2. Formulaire de détail existant (`#account-detail-view`), en mode création lors d'une duplication.
3. Zone d'étiquettes (`#acct-chips`) : une étiquette cliquée devient un champ texte à sa place.

## Tasks to do

### `1)` Dupliquer un compte

> Ouvrir le formulaire de création prérempli depuis un compte existant.

1. Dans `renderAccountList`, ajouter `.acct-dup-btn` (`data-idx`) « Dupliquer » à côté de « Modifier ».
2. Faire accepter à `showNewAccount` un compte source optionnel. Avec une source, préremplir `acct-server`, `acct-port` et `currentFolders` (copie par `slice()`).
3. Garder vides `acct-newname`, `acct-username` et `acct-password`, et le comportement sur Défaut, avec ou sans source.
4. Aucune modification Rust : `save_account` avec `create: true` valide déjà le nom.

### `2)` Renommer une étiquette sur place

> Corriger un dossier ignoré sans le supprimer puis le ressaisir.

1. Dans `renderChips`, rendre le libellé cliquable (`.chip-label`, `data-folder-idx`), distinct de `.chip-remove`.
2. Au clic, remplacer le libellé par un `<input>` prérempli, sélectionné et focalisé.
3. Une fonction de fin d'édition unique, protégée par un drapeau `done` propre au champ : seul le premier appel agit. Un `blur` déclenché par le retrait du champ du DOM après Entrée ou Échap est ainsi ignoré.
4. Entrée ou perte du focus : valeur nettoyée (trim). Si elle est vide ou inchangée, rien ne change. Si elle est égale à une autre étiquette, supprimer celle en édition. Sinon remplacer `currentFolders[idx]`. Puis appeler `renderChips()`.
5. Échap : fin d'édition sans modification. Entrée : `preventDefault()`, pour ne déclencher ni `addFolder` ni aucune autre action.
6. Ajouter un `title` « Cliquer pour modifier » et un curseur texte sur `.chip-label`.

## Test acceptance criteria

| Task | Acceptance criteria |
| ---- | ------------------- |
| 1 | « Dupliquer » ouvre « Ajouter un compte » avec serveur, port et dossiers ignorés du compte source. Le nom, l'identifiant, le mot de passe et le comportement sont vides. Modifier les étiquettes du nouveau compte ne touche pas celles du compte source en mémoire. |
| 2 | Cliquer sur une étiquette permet de la renommer. Entrée ou perte du focus applique le nom. Échap ou une valeur vide le laisse intact. Un renommage en doublon ne laisse qu'une étiquette. Après Enregistrer, `accounts.yaml` contient le nouveau nom. |