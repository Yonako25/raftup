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
- **PostgreSQL** (ajout du 19 novembre 2026, à relire) : même code pour les deux bases, par le pilote générique **`sqlx::Any`** (fonctions `any`, `sqlite`, `postgres` de sqlx), qui choisit la base d'après l'URL (`sqlite:` ou `postgres://`). Écarté : une énumération `Sqlite | Postgres` (chaque requête écrite deux fois) et du code générique sur `sqlx::Database` (traits trop lourds pour une débutante). Règles du SQL commun : paramètres `$1`, `$2`… (SQLite les accepte et sqlx les lie par leur numéro), entiers en `BIGINT` (`i64`), seule la colonne d'identifiant change (`INTEGER PRIMARY KEY` en SQLite, `BIGINT GENERATED ALWAYS AS IDENTITY` en PostgreSQL). Pas de TLS vers PostgreSQL (aucune fonction `tls-*`) : la base doit être sur la même machine ou un réseau privé. Une URL `sqlite://C:\…` est récrite en `sqlite:C:\…` (le pilote `Any` lit l'URL avec la crate `url`, qui prendrait « C » pour un hôte).
- Tests : SQLite en mémoire toujours ; avec `RAFTUP_HUB_TEST_POSTGRES=postgres://…`, les tests de `tests/api.rs` tournent sur PostgreSQL, chacun dans un schéma neuf (`search_path`). Les schémas de test ne sont pas supprimés : prendre une base jetable.
- Migrations : `MIGRATIONS` dans `store.rs` (version atteinte, instructions), appliquées en transaction à l'ouverture d'une base plus ancienne ; les tables nouvelles viennent de `CREATE TABLE IF NOT EXISTS`.
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

### Blocages (ajout du 19 novembre 2026, à relire)

- Table `blocks (blocker, blocked)`, un sens par ligne (schéma version 2). Routes `GET /v1/blocks`, `POST /v1/blocks` (`username`), `DELETE /v1/blocks/{username}`.
- Bloquer, en une transaction : retire l'amitié et les demandes dans les deux sens, puis inscrit le blocage (deux fois = sans effet). La présence n'étant visible qu'entre amis, elle disparaît dans les deux sens.
- Tant que le blocage dure, plus de demande : celui qui a bloqué reçoit `you_blocked` ; le joueur bloqué reçoit `user_not_found`, la même réponse qu'un pseudo inconnu, pour ne pas apprendre qu'il est bloqué. Le joueur bloqué n'est pas prévenu.
- Lever le blocage ne rend pas l'amitié : il faut une nouvelle demande.

### RGPD (ajout du 19 novembre 2026, à relire)

- **Export** : `GET /v1/me/export` renvoie un seul document JSON (`format_version` 1) : compte, identités (fournisseur, adresse, vérifiée ou non), sessions (dates seulement), amis, demandes reçues et envoyées, blocages, chacun avec sa date, plus la présence du moment. Jamais de hash de mot de passe ni d'empreinte de session.
- **Suppression** : `DELETE /v1/me`. Un compte avec mot de passe doit le redonner (`{"password": …}`) : une session volée ne suffit pas. Les échecs comptent dans la limite de la connexion (10 en 15 minutes). La suppression efface la ligne de `users` ; toutes les autres tables suivent par `ON DELETE CASCADE`. Elle est immédiate et définitive (pas de délai de grâce) ; le pseudo et l'adresse redeviennent libres tout de suite (pas de pseudo gardé en réserve, à revoir avec le changement de pseudo de l'ADR 0002).
- Le journal du hub ne garde que l'identifiant du compte (« compte supprimé »), sans pseudo ni adresse.

### E-mails : vérification d'adresse et mot de passe oublié (ajout du 19 novembre 2026, à relire)

- **Envoi derrière un trait** `Mailer` (`mail.rs`), synchrone, appelé sur un thread à part (`spawn_blocking`) **après** la réponse HTTP : la réponse ne dépend ni du serveur SMTP ni de l'existence de l'adresse. Trois implémentations : `SmtpMailer` (vrai envoi), `MemoryMailer` (tests), `DisabledMailer` (pas de SMTP réglé : message abandonné, avertissement au journal). Un échec d'envoi ne laisse qu'un avertissement.
- **SMTP** : crate **`lettre` 0.11** (MIT), transport synchrone, TLS par rustls + ring (comme le reste du dépôt), certificats racines `webpki-roots`. Réglage par l'environnement (`RAFTUP_HUB_SMTP_URL`, `RAFTUP_HUB_MAIL_FROM`), jamais en argument (visible dans la liste des process). Les erreurs de `lettre` sont remplacées par des erreurs sans détail (elles pourraient recopier l'URL et son mot de passe). Fournisseur SMTP à choisir au déploiement (hébergé en Europe, ADR 0002).
- **Jetons à usage unique** : table `email_tokens` (schéma version 3) : empreinte SHA-256 du jeton (32 octets aléatoires, comme les sessions), compte, usage (`verify_email` ou `reset_password`), date d'expiration. Un jeton ne sert qu'à son usage. Consommation en une requête `DELETE … RETURNING` : deux requêtes simultanées ne réussissent pas toutes les deux. Un nouveau message annule le jeton précédent du même usage. Jetons expirés retirés par le ménage horaire.
- **Vérification d'adresse** : message envoyé à l'inscription, lien `<public_url>/v1/email/verify?token=…` valable 24 h (réponse en texte pour le navigateur), aussi `POST /v1/email/verify` (`token`) ; `POST /v1/email/verify/resend` pour un nouveau message. `GET /v1/me` dit `email_verified`. Une adresse non vérifiée n'empêche pas encore la connexion.
- **Mot de passe oublié** : `POST /v1/password/forgot` (`email`) répond toujours `202`, adresse connue ou non, limite atteinte ou non. Le message contient un code valable 1 h et le lien profond `raftup://password-reset?token=…` (le hub n'a pas de page web). `POST /v1/password/reset` (`token`, `password`) : mot de passe vérifié avant de consommer le code ; nouveau hash, adresse marquée vérifiée, **toutes les sessions fermées**.
- Limite : 3 messages par heure et par adresse (mot de passe oublié) ou par compte (renvoi de la vérification).
- `--public-url` donne l'adresse publique du hub pour les liens (par défaut `http://127.0.0.1:8787`).
### Connexion Discord (ajout du 19 novembre 2026, à relire)

- **OAuth 2.0 avec PKCE (`S256`)**, portée `identify` seulement (ni e-mail ni serveurs : données minimales). Le **hub** est le client OAuth : il garde le secret de l'application et le vérificateur PKCE ; le launcher ne voit jamais le code d'autorisation. L'adresse de retour déclarée chez Discord est celle du hub (`<public_url>/v1/oauth/discord/callback`), fixe : Discord n'accepte pas un port local variable.
- Parcours : `POST /v1/oauth/discord/start` → `authorize_url`, `state`, `secret` ; le navigateur du système va chez Discord, revient au hub, qui échange le code (jeton d'accès jamais gardé) puis renvoie le navigateur vers le launcher (`http://127.0.0.1:<return_port>/discord` ou `raftup://oauth/discord`) avec un `ticket` ; `POST /v1/oauth/discord/finish` (`state`, `secret`, `ticket`) → `logged_in` (session), `linked`, ou `needs_username` (pseudo proposé : nom Discord s'il est valide et libre) puis `POST /v1/oauth/discord/register` (`username`).
- Pourquoi `secret` **et** `ticket` : `secret` prouve que c'est le launcher qui a commencé ; `ticket` n'arrive que dans le navigateur qui a dit oui à Discord, sur la machine du joueur. Sans le ticket, quelqu'un qui enverrait son lien d'autorisation à une victime récupérerait la session de celle-ci (hameçonnage du parcours « appareil »).
- Connexions en attente **en mémoire** (comme la présence), 10 minutes au plus, 10 000 au plus ; le retour de Discord n'est accepté qu'une fois. Un hub redémarré oublie les connexions en cours (le joueur recommence).
- Liaison : `start` avec une session lie Discord au compte connecté. Un compte Discord ne va qu'à un compte (`discord_taken`), un compte n'a qu'un Discord (`already_linked`, index unique `(user_id, provider)`, schéma version 4). Pas de liaison automatique par adresse e-mail (on ne lit pas l'e-mail Discord). `DELETE /v1/me/identities/discord` délie, sauf si c'est le dernier moyen de connexion (`last_identity`). Un compte Discord seul se supprime sans mot de passe.
- Client HTTP derrière le trait `DiscordApi` (`discord.rs`) : `HttpDiscord` (crate `ureq`, déjà utilisée par le launcher, HTTPS seulement, 15 s au plus) et `FakeDiscord` pour les tests. Appel sur un thread à part (`spawn_blocking`). Réglage par l'environnement (`RAFTUP_HUB_DISCORD_CLIENT_ID`, `RAFTUP_HUB_DISCORD_CLIENT_SECRET`) ; sans lui, les routes répondent `discord_disabled` (503). Nouvelles dépendances : `ureq`, `url`, `base64` (MIT OR Apache-2.0).
- Licences ajoutées (même date) : ISC (`rustls-webpki`, `untrusted`, `ring` avec Apache-2.0), 0BSD (`quoted_printable`), CDLA-Permissive-2.0 (`webpki-roots`, données des certificats) ; compatibles avec l'AGPL.

### Licences des dépendances (vérifiées le 19 novembre 2026, `cargo tree -p hub -e normal,build`)

Toutes compatibles avec l'AGPL-3.0 : MIT, Apache-2.0, MIT OR Apache-2.0, BSD-3-Clause (`subtle`, `matchit`), Zlib (`foldhash`), Unicode-3.0 (`icu_*`, `zerovec`…, via `idna` de `url`), Unlicense OR MIT (`memchr`), Apache-2.0 OR BSL-1.0 (`ryu`). Aucune dépendance sous GPL, LGPL ou licence propriétaire. Revérifié après l'ajout de PostgreSQL (même date) : en plus, BSD-2-Clause OR Apache-2.0 OR MIT, Zlib OR Apache-2.0 OR MIT, MIT AND BSD-3-Clause ; toujours compatibles.

## Pas encore fait (à prévoir)

- Discord : révocation du jeton d'accès après lecture (inutile tant qu'il n'est pas gardé), vrai essai avec une application Discord.
- L'inscription révèle toujours qu'une adresse a déjà un compte (`email_taken`) : à remplacer par un message « vous avez déjà un compte » envoyé à l'adresse, quand le launcher saura l'afficher. Connexion refusée tant que l'adresse n'est pas vérifiée : pas encore.
- Refus de toutes les demandes, changement de pseudo avec délai, limite par adresse IP.
- RGPD : hébergement en Europe, politique de confidentialité, sauvegardes purgées des comptes supprimés.
- Côté launcher : client du hub, jeton rangé dans le trousseau (`keyring`), panneau d'amis.
- Plusieurs instances du hub (présence dans Redis), déploiement, sauvegardes.

## Conséquences

- Le hub se lance et se teste sur le PC sans rien installer : `cargo run -p hub --bin raftup-hub`, `cargo test -p hub`.
- PostgreSQL et SQLite partagent tout le SQL de `store.rs` ; un test lancé sur les deux attrape les écarts.
- Le binaire n'est pas exposable tel quel sur Internet : il lui faut un proxy TLS et les points ci-dessus (e-mails, RGPD) avant une instance publique.
