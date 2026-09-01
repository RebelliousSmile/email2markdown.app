# Tests IMAP de l’export contextuel

## Dovecot jetable

Le serveur de test utilise uniquement le compte `test` et le mot de passe public
`password`. Il ne doit jamais être relié à une boîte réelle.

```sh
docker compose -f tests/fixtures/imap/compose.yaml up -d --wait
EMAIL_TO_MARKDOWN_IMAP_TEST=1 cargo test --test imap_integration -- --ignored
docker compose -f tests/fixtures/imap/compose.yaml down -v
```

Le test vérifie la recherche expéditeur/destinataire, UID/UIDVALIDITY, le rejet
d’un faux positif par sous-chaîne et la conservation du drapeau `Seen`.

## Gmail dédié

Le test Gmail destructif doit utiliser un compte créé uniquement pour les tests,
stocké dans les secrets CI, jamais une adresse personnelle. Injecter des messages
générés avec un préfixe `ETM-CONTEXT-<identifiant unique>`, vérifier la fusion des
labels par `X-GM-MSGID`, puis supprimer uniquement les messages portant ce préfixe.
Cette recette reste manuelle tant que la phase de suppression ciblée n’est pas
activée.
