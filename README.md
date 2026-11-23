# Email to Markdown Exporter

> *Exporte vos boîtes IMAP en Markdown à métadonnées YAML et range les notes dans votre arborescence, plutôt que de les laisser dans un client mail.*

Application Rust en ligne de commande, avec une icône de barre système en option (feature `tray`). Trois guides PDF l'accompagnent : [utilisateur](aidd_docs/guides/output/pdf/guide-utilisateur-v0.29.0.pdf), [contributeur](aidd_docs/guides/output/pdf/guide-contributeur-v0.29.0.pdf) et [développeur](aidd_docs/guides/output/pdf/guide-developpeur-v0.29.0.pdf).

- [Aperçu](#aperçu)
- [Télécharger](#télécharger)
- [État du projet](#état-du-projet)
- [Prérequis](#prérequis)
- [Démarrage rapide](#démarrage-rapide)
- [Documentation](#documentation)
- [Contribuer](#contribuer) · [Licence](#licence)

## Aperçu

Après chaque export, la fenêtre de revue propose un chemin de rangement par email. Vous le gardez, le corrigez ou l'ignorez, et rien ne bouge avant d'avoir cliqué **Appliquer**.

<img src="aidd_docs/guides/guide-utilisateur/images/revue-routage.png" alt="Fenêtre de revue du routage : quatre emails avec leur chemin proposé, par exemple Perso/Banque, et le statut auto ou défaut" width="720">

Depuis un dossier de vos notes, l'export contextuel retrouve les messages d'un correspondant et ne convertit que ceux que vous cochez.

<img src="aidd_docs/guides/guide-utilisateur/images/export-contextuel.png" alt="Export contextuel : liste des messages d'une boîte aux lettres, filtrée par les règles du dossier, avec les cases à cocher et le bouton Convertir" width="720">

Les captures montrent des données fictives. Le [guide de l'utilisateur](aidd_docs/guides/output/pdf/guide-utilisateur-v0.29.0.pdf) détaille les autres écrans.

## Télécharger

Choisissez votre système : aucun prérequis, la barre système est incluse.

| Système | Téléchargement | Fichier |
|---|---|---|
| [![Windows](https://img.shields.io/badge/Windows-0078D4?style=for-the-badge&logo=windows11&logoColor=white)](https://github.com/RebelliousSmile/email2markdown.app/releases/download/v0.29.0/email-to-markdown-v0.29.0-windows-x86_64.exe) | [Télécharger pour Windows](https://github.com/RebelliousSmile/email2markdown.app/releases/download/v0.29.0/email-to-markdown-v0.29.0-windows-x86_64.exe) | `email-to-markdown-v0.29.0-windows-x86_64.exe` |
| [![macOS](https://img.shields.io/badge/macOS-000000?style=for-the-badge&logo=apple&logoColor=white)](https://github.com/RebelliousSmile/email2markdown.app/releases/download/v0.29.0/email-to-markdown-v0.29.0-macos-native) | [Télécharger pour macOS](https://github.com/RebelliousSmile/email2markdown.app/releases/download/v0.29.0/email-to-markdown-v0.29.0-macos-native) | `email-to-markdown-v0.29.0-macos-native` |
| [![Linux](https://img.shields.io/badge/Linux-FCC624?style=for-the-badge&logo=linux&logoColor=black)](https://github.com/RebelliousSmile/email2markdown.app/releases/download/v0.29.0/email-to-markdown-v0.29.0-linux-x86_64) | [Télécharger pour Linux](https://github.com/RebelliousSmile/email2markdown.app/releases/download/v0.29.0/email-to-markdown-v0.29.0-linux-x86_64) | `email-to-markdown-v0.29.0-linux-x86_64` (GTK et WebKit requis pour la barre système) |

Un fichier [`SHA256SUMS`](https://github.com/RebelliousSmile/email2markdown.app/releases/download/v0.29.0/SHA256SUMS) accompagne chaque release, et les [autres versions](https://github.com/RebelliousSmile/email2markdown.app/releases) sont sur la page des releases.

## État du projet

**Statut : 🧰 Beta.** Version 0.29.0, sous SemVer, pas encore de 1.0. Build et tests en CI sur Windows, macOS et Linux (Ubuntu 24.04).

- *Ça marche aujourd'hui :* export IMAP vers Markdown ; import des comptes Thunderbird ; routage déterministe par `destinations.yaml` ; fenêtre de revue du routage dans la barre système ; recherche contextuelle depuis un dossier ; audit en lecture seule des messages restés sur le serveur
- *Pas encore :* connexion OAuth2 (un compte Gmail demande un mot de passe d'application)

## Prérequis

- Accès IMAP activé sur le serveur email
- Gmail avec 2FA : [mot de passe spécifique à l'application](https://support.google.com/accounts/answer/185833)
- Aucun autre prérequis avec le binaire précompilé

## Démarrage rapide

Téléchargez le binaire de votre système ci-dessus, puis suivez le [guide de l'utilisateur](aidd_docs/guides/output/pdf/guide-utilisateur-v0.29.0.pdf) : installation, ajout des comptes et premier export.

Compiler depuis les sources, importer les comptes de Thunderbird en ligne de commande et configurer les fichiers se trouvent dans le [guide du développeur](aidd_docs/guides/output/pdf/guide-developpeur-v0.29.0.pdf) (chapitres 10 et 11).

---

## Documentation

Trois guides PDF (version 0.29.0), à lire ou à télécharger :

| Guide | Pour qui | PDF |
|-------|----------|-----|
| Guide de l’utilisateur | Installer et utiliser l’application au quotidien, dépannage compris | [guide-utilisateur-v0.29.0.pdf](aidd_docs/guides/output/pdf/guide-utilisateur-v0.29.0.pdf) |
| Guide du contributeur | Participer au projet | [guide-contributeur-v0.29.0.pdf](aidd_docs/guides/output/pdf/guide-contributeur-v0.29.0.pdf) |
| Guide du développeur | Comprendre l’architecture et le code ; compilation, référence des commandes, de la configuration et du dépannage (chapitres 10 à 12) | [guide-developpeur-v0.29.0.pdf](aidd_docs/guides/output/pdf/guide-developpeur-v0.29.0.pdf) |

Les problèmes de compilation ou de système (linker Windows, icône de barre système, certificats TLS) sont dans [TROUBLESHOOTING.md](TROUBLESHOOTING.md).

---

## Contribuer

Les contributions sont les bienvenues ! Consultez [CONTRIBUTING.md](CONTRIBUTING.md) pour les directives.

## Licence

MIT — voir [LICENSE](LICENSE).
