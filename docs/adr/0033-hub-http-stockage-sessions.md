# ADR 0033 — `raftup-hub` : serveur HTTP, stockage et sessions

- **Statut** : accepté en autonomie le 19 novembre 2026, à relire par l'utilisatrice
- Complète l'ADR 0002 (comptes, amis, présence) et l'ADR 0003 (AGPL)

## Contexte

L'ADR 0002 fixe le modèle (compte séparé de ses identités, pseudo unique sans code ami, présence en mémoire, PostgreSQL via `sqlx`). Il ne dit pas quel serveur HTTP utiliser, comment tester sans PostgreSQL, ni la forme des sessions. Cette première version du hub doit tourner et se tester sur le PC, sans déploiement.

## Décision

### Crate et licence

- Nouvelle crate `crates/hub`, binaire `raftup-hub`, licence **AGPL-3.0-or-later** (`license` du `Cargo.toml`, texte dans `crates/hub/LICENSE-AGPL`), `publish = false`.
- Aucune crate MIT/Apache n'en dépend, et le workspace ne la propose pas dans `[workspace.dependencies]`. Un test (`crates/hub/tests/licence.rs`) lit les `Cargo.toml` du dépôt et échoue sinon.

### Serveur HTTP

- **axum 0.8** (MIT), sur `tokio`, déjà utilisé partout. Fonctions minimales (`http1`, `json`, `tokio`).
- API JSON **versionnée** sous `/v1` (exigence de l'ADR 0002). Erreurs au format `{"error": "<code stable>", "message": "<texte>"}`. Corps limité à 16 Kio.
- Le hub parle HTTP en clair : en production il se place derrière un proxy qui fait le TLS (Traefik ou équivalent). Pas de TLS dans le binaire.

### Stockage

- **sqlx 0.8** (MIT OR Apache-2.0), requêtes écrites à la main (`sqlx::query`), sans les macros vérifiées à la compilation : elles demandent une base ouverte au moment du build. Les tests d'intégration vérifient les requêtes.
- **SQLite** (compilé avec `libsqlite3-sys`, MIT ; SQLite est dans le domaine public) pour les tests et le développement : base en mémoire pour les tests, fichier en WAL pour le binaire.
- **PostgreSQL prévu, pas encore écrit.** Tout le SQL est dans `store.rs`, derrière les méthodes de `Store` ; le reste du hub n'en voit rien. Le SQL est portable (`RETURNING`, `ON CONFLICT ... DO NOTHING`, pas de fonction propre à SQLite). Le jour venu : ajouter la fonction `postgres` de sqlx, et faire de `Store` une énumération `Sqlite | Postgres` (ou passer par `sqlx::Any`). Changeront : les marques de paramètres (`?` → `$1`), `INTEGER PRIMARY KEY` → `BIGINT GENERATED ALWAYS AS IDENTITY`, et les tests de `Store` lancés aussi sur un PostgreSQL de test (`#[ignore]`, variable d'environnement).
- Schéma versionné : table `schema_version`, un hub refuse une base plus récente que lui.
- Tables (ADR 0002) : `users` (pseudo affiché + forme normalisée unique), `user_identities` (fournisseur `email`, adresse normalisée, hash, `verified`), `sessions`, `friend_requests`, `friendships` (paire rangée `user_a < user_b`). Les liens pointent vers l'identifiant du compte, jamais vers le pseudo.

### Mots de passe (NF09)

- **argon2 0.5** (RustCrypto, MIT OR Apache-2.0), argon2id, réglage OWASP (19 Mio, 2 passes, 1 voie), sel aléatoire du système, hash au format PHC. Calcul dans `spawn_blocking`.
- Le mot de passe en clair n'existe que dans un type `Password` dont `Debug` affiche `Password(***)`. Les journaux ne contiennent ni mot de passe ni adresse e-mail (seulement l'identifiant du compte). Un test (`tests/nf09.rs`) cherche le mot de passe dans la base, son WAL et le journal capturé.
- Connexion : même réponse et même coût pour une adresse inconnue et un mauvais mot de passe (vérification contre un hash bidon).
- 8 à 128 caractères. Réglage minimal du hachage seulement dans les tests (`HubConfig::for_tests`).

### Sessions

- Jeton **opaque** : 32 octets aléatoires du système, en hexadécimal, envoyé par `Authorization: Bearer`. La base ne garde que son **SHA-256** : une copie de la base ne donne accès à aucune session.
- Durée de vie 30 jours, **révocable** (`DELETE /v1/sessions/current` supprime la ligne). Ménage des sessions expirées toutes les heures dans le binaire.
- Pas de JWT : un jeton signé ne se révoque pas sans liste de révocation, et le hub consulte de toute façon la base.

### Pseudo, amis, présence

- Pseudo : règles de l'ADR 0002 (3 à 24 caractères, lettres, chiffres, `_ - .`, unicité sur NFKC + minuscules, liste de pseudos réservés). Crate `unicode-normalization` (MIT OR Apache-2.0).
- Demande d'ami par pseudo ; deux demandes croisées deviennent une amitié. Accepter, refuser, annuler, retirer un ami.
- Présence (`online`, `in_game` avec nom du jeu, `offline`) en mémoire, jamais en base, visible seulement dans la liste d'amis ; elle expire si le launcher ne la renouvelle pas (120 s).
- Limites en mémoire : 10 échecs de connexion par adresse en 15 minutes, 20 demandes d'ami par compte et par minute.

### Licences des dépendances (vérifiées le 19 novembre 2026, `cargo tree -p hub -e normal,build`)

Toutes compatibles avec l'AGPL-3.0 : MIT, Apache-2.0, MIT OR Apache-2.0, BSD-3-Clause (`subtle`, `matchit`), Zlib (`foldhash`), Unicode-3.0 (`icu_*`, `zerovec`…, via `idna` de `url`), Unlicense OR MIT (`memchr`), Apache-2.0 OR BSL-1.0 (`ryu`). Aucune dépendance sous GPL, LGPL ou licence propriétaire.

## Pas encore fait (à prévoir)

- **Discord** (OAuth 2.0 + PKCE) : pas pour l'instant ; la table `user_identities` est prête pour un fournisseur `discord`.
- Vérification de l'adresse par lien, « mot de passe oublié » : demandent un service d'envoi d'e-mails. Tant qu'ils manquent, l'inscription révèle qu'une adresse a déjà un compte (`email_taken`).
- Blocages, refus de toutes les demandes, changement de pseudo avec délai, limite par adresse IP.
- RGPD : export et suppression du compte.
- Côté launcher : client du hub, jeton rangé dans le trousseau (`keyring`), panneau d'amis.
- Plusieurs instances du hub (présence dans Redis), déploiement, sauvegardes.

## Conséquences

- Le hub se lance et se teste sur le PC sans rien installer : `cargo run -p hub --bin raftup-hub`, `cargo test -p hub`.
- Passer à PostgreSQL ne touche qu'à `store.rs` et à ses tests.
- Le binaire n'est pas exposable tel quel sur Internet : il lui faut un proxy TLS et les points ci-dessus (e-mails, RGPD) avant une instance publique.
