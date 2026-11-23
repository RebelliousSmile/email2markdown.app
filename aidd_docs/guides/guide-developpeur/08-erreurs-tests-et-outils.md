# Erreurs et *tests*

Ce chapitre rassemble ce qui entoure le code métier : la politique d'erreurs et l'organisation des tests.

## La politique d'erreurs

Le projet distingue les erreurs typées, qui forment un contrat, des erreurs opérationnelles.

| Mécanisme | Usage |
| --- | --- |
| `thiserror` | erreurs de domaine exposées par un module |
| `anyhow::Result<T>` | tous les autres modules |

Les erreurs typées existantes sont :

- `ConfigError` (`config.rs`) : `FileReadError`, `YamlParseError`, `SettingsFile { path, reason }`, `AccountNotFound`, `NoPassword`, `ValidationError` ;
- `ContextualDestinationError` (`route.rs`), variante `MissingAddressRule { path }`, que le lancement contextuel reconnaît par `downcast_ref` ;
- `PrimaryPasswordRequired` (`thunderbird.rs`), qui signale qu'un mot de passe principal est nécessaire.

> **Bon à savoir** : la règle écrite parle de `thiserror` « seulement pour `ConfigError` ». Le code en emploie en réalité aussi pour les deux autres types, parce que l'appelant doit les distinguer des autres erreurs. Suivez cette logique : un type d'erreur n'est justifié que si quelqu'un le teste avec `downcast_ref` ou `match`.

### Propager

- Utilisez `?` et ajoutez `.context("…")` aux frontières de module. Le message est en minuscules, à l'impératif, sans ponctuation finale.
- Pas de `.unwrap()` dans `src/`. Préférez `?`, `.unwrap_or_default()` ou `.unwrap_or_else()`.
- `.ok()` n'est admis que lorsque l'erreur peut être ignorée sans conséquence, comme pour `dotenvy::from_path(…).ok()`.
- Les actions de l'icône convertissent l'erreur en `ActionResult::Error(String)`.

Plusieurs opérations sont explicitement « au mieux » : écrire une preuve dans le registre, consigner une suppression, lier des citations après un tri. Elles échouent sans interrompre l'opération principale, et le choix est documenté dans le code de chacune.

### Messages destinés à l'utilisateur

Les messages affichés sont en français et passent par des fonctions dédiées, comme `describe_connection_error`, `counts_summary` ou `purge_summary`. Les messages de contexte `anyhow`, eux, sont en anglais : ils s'adressent au développeur.

## Les tests

### Où se trouvent les tests

| Emplacement | Contenu |
| --- | --- |
| `tests/rust_tests.rs` | tests d'intégration regroupés par module (environ 6600 lignes) |
| `tests/imap_integration.rs` | tests contre un vrai serveur IMAP, ignorés par défaut |
| `tests/common/` | catalogue de messages Gmail partagé (`gmail_seed.rs`) |
| `tests/fixtures/` | fichiers d'exemple |
| `#[cfg(test)]` dans `src/` | tests unitaires de `updater`, `server_audit`, `server_cleanup`, `menu`, `tray_actions`, `main`… |

Dans `rust_tests.rs`, un module `mod` par module du crate : `utils_tests`, `config_tests`, `settings_tests`, `email_export_tests`, `cleaner_tests`, `route_tests`, `contextual_export_tests`, `handled_purge_tests`, `destinations_tests`, `dest_cmd_tests`, `quote_link_tests`, `thunderbird_tests`, `env_file_tests`, `gmail_seed_tests`, entre autres. Chacun commence par `use super::*;`.

### Conventions

- **Nom** : `test_<fonction_ou_structure>_<condition>`, avec une condition comme `default`, `empty`, `valid`, `invalid` ou `with_<variante>`.
- **Une assertion logique par test**, avec des assertions précises (`assert_eq!`, `assert!(x.contains(…))`).
- **Système de fichiers** : toujours un `TempDir` de `tempfile`, jamais un chemin codé en dur ; on écrit les fichiers avec `std::fs::write(temp.path().join(…), …)`.

`let temp = TempDir::new().unwrap();`\
`std::fs::write(temp.path().join("accounts.yaml"), YAML)?;`\
`let config = Config::load_with_settings(`\
`    &temp.path().join("accounts.yaml"),`\
`    &temp.path().join("settings.yaml"),`\
`)?;`

- **Bout en bout** : une assertion inclusive est toujours associée à une assertion exclusive (voir le chapitre 4).
- **Pas de fichier d'aide partagé**, sauf `tests/common/` pour le catalogue Gmail.

### L'intégration IMAP

`tests/imap_integration.rs` s'exécute contre un conteneur Dovecot réel. Les tests sont marqués `#[ignore]` pour ne pas tourner sans le conteneur ni la variable `EMAIL_TO_MARKDOWN_IMAP_TEST=1`, et il n'y a pas de simulation du protocole : la règle du projet est que le comportement IMAP se vérifie sur un vrai serveur. La variable `EMAIL_TO_MARKDOWN_IMAP_INSECURE_TEST=1`, lue par `open_session`, autorise le certificat auto-signé de `localhost`.

> **Attention** : Thunderbird (NSS) et l'interface graphique n'ont aucun test automatisé et se vérifient à la main. N'ajoutez pas de simulation de NSS ou de la vue web : la règle l'exclut.

### Commandes

Utilisez les commandes de `cargo`, y compris avec la fonctionnalité `tray` pour tester la couche graphique :

`cargo clippy -- -D warnings`\
`cargo test`\
`cargo test --all-features`\
`EMAIL_TO_MARKDOWN_IMAP_TEST=1 cargo test --test imap_integration -- --ignored --skip gmail_ --test-threads=1`

La dernière commande lance les tests IMAP réels et suppose que le conteneur décrit par `tests/fixtures/imap/compose.yaml` est démarré. Les tests qui touchent à un vrai compte Gmail exigent en plus `EMAIL_TO_MARKDOWN_GMAIL_DESTRUCTIVE_TEST=1` et un compte de test isolé.
