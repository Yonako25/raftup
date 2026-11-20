# ADR 0035 — Client du hub dans le launcher : jeton, adresse, appels réseau

- **Statut** : accepté en autonomie le 19 novembre 2026, à relire par l'utilisatrice
- Complète l'ADR 0002 (comptes, amis, jeton dans le trousseau), l'ADR 0003 (AGPL du hub) et l'ADR 0033 (API `/v1`)

## Contexte

Le hub `raftup-hub` existe (ADR 0033) mais le launcher ne savait pas lui parler. Il faut un client et un écran « Amis ». Trois points ne sont pas tranchés par les ADR précédents : comment le launcher parle au hub sans dépendre de sa crate AGPL, quelle adresse utiliser tant qu'aucun hub public n'est déployé, et ce qui se passe quand le trousseau du système manque.

## Décision

### Client sans dépendance au hub

- Module `launcher_core::hub_client` (MIT OR Apache-2.0). Il ne dépend **pas** de `crates/hub`, même en test : les types JSON de l'API `/v1` sont **redéfinis** côté client (`hub_client::types`). Le test `crates/hub/tests/licence.rs` continue de le vérifier.
- Client HTTP **bloquant** avec `ureq` (déjà utilisé par le téléchargeur et le webhook) : pas de nouvelle pile HTTP. L'interface l'appelle toujours dans un thread à part ; le client part dans le thread avec la demande et revient avec la réponse (un appel à la fois, pas de verrou).
- Aucune redirection suivie (le jeton ne part jamais ailleurs), délai de 10 s par appel, réponse lue sur 1 Mio au plus.
- Erreurs typées (`HubClientError`) : adresse refusée, hub injoignable, erreur de l'API avec son **code stable** (`ErrorCode`, et `Other` pour un code inconnu d'un hub plus récent), réponse inattendue, trousseau en échec. `GET /v1` vérifie le nom du service et la version de l'API (1).
- Mot de passe et jeton dans des types (`Password`, `SessionToken`) dont `Debug` affiche `***`. Le mot de passe n'est jamais écrit sur le disque ; le champ de l'écran est vidé à l'envoi.
- Un pseudo placé dans un chemin (`/v1/friends/{pseudo}`) est encodé : tout caractère autre qu'une lettre ou un chiffre ASCII devient `%XX`.

### Adresse du hub

- **Par défaut : `http://127.0.0.1:8787`**, le hub lancé sur ce PC (`cargo run -p hub --bin raftup-hub`), tant qu'aucun hub public n'existe. À remplacer par l'adresse officielle (`https://…`) le jour du déploiement : une constante, `DEFAULT_HUB_URL`.
- Adresse modifiable dans l'écran « Amis », mémorisée dans `config.toml` (champ facultatif `hub_url`, ajouté **sans** changer `format_version` : un launcher plus ancien l'ignore). L'adresse par défaut n'est pas écrite, pour suivre un changement de défaut.
- **HTTPS obligatoire**, sauf vers ce PC (`localhost`, `127.0.0.1`, `[::1]`) : ailleurs, mot de passe et jeton passeraient en clair. Pas de `?`, `#`, ni d'identifiants dans l'adresse ; un chemin est permis (hub derrière un proxy).

### Jeton de session

- Rangé dans le **trousseau du système** (ADR 0002) avec la crate `keyring` 3.6 (MIT OR Apache-2.0) : Gestionnaire d'identification sous Windows, Trousseau sous macOS, keyutils du noyau sous Linux (Rust pur, sans D-Bus ni bibliothèque système à installer : la CI Linux reste simple). Sous Linux, keyutils ne survit pas à un redémarrage : il faudra se reconnecter (acceptable pour l'instant ; Secret Service plus tard si besoin).
- Une entrée par adresse de hub (service `raftup-hub`, compte = l'adresse) : changer de hub ne lui envoie pas le jeton d'un autre.
- Trousseau indisponible : la connexion marche quand même, un message prévient que la session ne sera pas gardée à la fermeture. Jamais de repli dans un fichier en clair.
- Au démarrage, le launcher reprend la session rangée (`GET /v1/me`) ; un jeton refusé (expiré, révoqué) est oublié. La déconnexion oublie le jeton sur ce PC **même si** le hub ne répond pas.

### Présence

- Tant que le joueur est connecté, le launcher envoie « en ligne » et relit amis et demandes **toutes les 60 s**, quel que soit l'écran affiché (le hub oublie une présence au bout de 120 s). Pas encore de présence « en partie » : le launcher ne suit pas encore le jeu lancé.

## Conséquences

- Le launcher se teste sans le hub : un faux serveur HTTP dans `launcher-core/tests/hub_client.rs` ; un test `#[ignore]` joue un scénario complet contre un vrai `raftup-hub` lancé à part (`RAFTUP_HUB_TEST_URL`).
- Une dépendance de plus : `keyring`, qui tire `zeroize`, `log`, `byteorder` et, selon le système, `windows-sys`, `security-framework` / `core-foundation` (macOS) ou `linux-keyutils` (toutes MIT et/ou Apache-2.0, `byteorder` aussi Unlicense ; vérifié par `cargo tree -p keyring` le 19 novembre 2026).
- Tant que l'adresse par défaut est locale, l'écran « Amis » dit « hub injoignable » chez un joueur qui n'a pas lancé de hub : normal avant le déploiement.
- Inscription sur un hub « sur invitation » (ADR 0036) : champ « Code d'invitation » facultatif dans « Nouveau compte » (vide = rien envoyé) ; `registration_closed`, `invite_required` et `invalid_invite` sont des codes typés, avec un message en français chacun (20 novembre 2026).
- Reste à faire : présence « en partie » (nom du jeu lancé), connexion Discord, notifications de demandes, adresse officielle du hub.
