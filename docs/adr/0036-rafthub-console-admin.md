# ADR 0036 — Rafthub : console d'admin web et launcher relié au hub

- **Statut** : accepté en autonomie le 19 novembre 2026, à relire par l'utilisatrice
- Complète l'ADR 0002 (hub), l'ADR 0003 (AGPL), l'ADR 0033 (API `/v1`), l'ADR 0035 (client du hub) et l'ADR 0020 (liens `raftup://`)
- Maquette : https://claude.ai/artifact/FeFFd35MJTsg3XLVAAhZKG (tableau de bord, joueurs, distribution du launcher, réglages)

## Contexte

Le hub a une API `/v1` mais rien pour la personne qui l'héberge : pas de vue sur les comptes, pas de modération, réglages SMTP et Discord seulement par variables d'environnement. Et un joueur qui télécharge le launcher doit taper l'adresse du hub à la main (ADR 0035 : `http://127.0.0.1:8787` par défaut), ce qui n'est pas « facile à utiliser ».

L'utilisatrice veut :
- un nom : **Rafthub** ;
- une **console d'admin web** servie par le hub (pas un site pour les joueurs) ;
- qu'un launcher téléchargé depuis un hub soit **déjà relié** à ce hub.

## Décision

### Nom
- Le produit s'appelle **Rafthub**. Le binaire devient `rafthub` (au lieu de `raftup-hub`). La crate reste `hub` (chemin `crates/hub`), en AGPL (ADR 0003).

### Pages HTML rendues par le serveur
- Même binaire, même routeur axum : la console vit sous `/admin`, l'API `/v1` ne change pas.
- Gabarits **askama** (MIT/Apache-2.0) dans `crates/hub/templates/`, échappement HTML automatique, vérifiés à la compilation. Un gabarit de base `admin/base.html` (menu de gauche, en-tête) ; une page = un gabarit + un module Rust dans `crates/hub/src/web/`.
- Pas de framework JavaScript. Un seul petit fichier JS autorisé (copier un lien dans le presse-papiers) ; tout le reste marche sans JS (formulaires `POST`).
- Une feuille de style `rafthub.css`, et les polices Chakra Petch et Inter (OFL) servies par le hub lui-même sous `/admin/static/` (`include_bytes!`) : aucune requête vers Google Fonts ni un autre domaine. Identité visuelle de la SPEC (acier, bleu électrique, angles de 2 px).

### Connexion à la console
- Colonne `role` sur `users` (`player` par défaut, `admin`). Le premier admin se crée en ligne de commande : `rafthub admin create <e-mail>` (le compte doit exister) ; `rafthub admin remove <e-mail>`.
- Page `/admin/login` (e-mail + mot de passe, mêmes hachages argon2 que l'API). Session d'admin dans un **cookie** `rafthub_admin` : jeton opaque haché en base (table `admin_sessions`, comme les sessions de l'ADR 0033), `HttpOnly`, `Secure` (sauf vers `127.0.0.1`), `SameSite=Strict`, expiration 12 h. Les jetons Bearer de l'API ne donnent **pas** accès à `/admin`.
- Chaque formulaire porte un jeton anti-CSRF lié à la session.
- Pas de compte Discord pour entrer dans la console : e-mail + mot de passe seulement.

### Données nouvelles
- `users.suspended_at` : un compte suspendu ne peut plus se connecter, ses sessions sont révoquées, il disparaît des listes d'amis et de la présence des autres. Réactiver efface la colonne.
- Table `settings` (clé, valeur texte) : nom du hub, adresse publique, mode d'inscription (`open`, `invite`, `closed`), réglages SMTP et Discord, page publique de téléchargement (oui/non). **Ordre de priorité** : valeur en base, sinon variable d'environnement actuelle (ADR 0033), sinon défaut. Les secrets (mot de passe SMTP, secret Discord) sont **écrits** depuis la console mais **jamais réaffichés** (« configuré » / « non configuré »).
- Mode `invite` : table `invites` (code haché, créé par, expiration, utilisé par) ; un code d'invitation est demandé à l'inscription.
- Table `admin_events` (date, acteur, type, cible, détail court) : le « fil d'activité » et la page « Journaux ». Écrit pour : création de compte, suspension, réactivation, changement de réglage, export ou suppression RGPD. Jamais de mot de passe, de jeton ou d'e-mail complet dans le détail.

### Précisions de la couche données (session `rafthub-donnees`, 19 novembre 2026)

- **Schéma en version 5** : migration `ALTER TABLE users ADD COLUMN suspended_at BIGINT` (sautée si la colonne existe déjà) ; tables `settings`, `invites`, `admin_events` créées par le schéma. La colonne `role` et `admin_sessions` viennent dans une version suivante.
- **Suspension** : sessions supprimées dans la même transaction ; la connexion (mot de passe ou Discord) répond `403 account_suspended` **après** la vérification du mot de passe (un inconnu n'apprend rien) ; le compte suspendu est caché des amis, des demandes, des blocages et de toute recherche par pseudo des autres joueurs (`user_not_found`). Réactiver rend l'amitié et les demandes, qui n'avaient pas été effacées.
- **Réglages** : clés `hub_name` (défaut « Rafthub »), `public_url`, `registration_mode`, `smtp_url`, `mail_from`, `discord_client_id`, `discord_client_secret`, `public_download` (défaut `true`). SMTP reste une seule adresse `smtps://utilisateur:motdepasse@hote:465`, comme dans l'ADR 0033 : elle est donc **secrète** ; la console n'en montre que l'hôte. Variables d'environnement : celles de l'ADR 0033, plus `RAFTUP_HUB_PUBLIC_URL`, `RAFTUP_HUB_NAME`, `RAFTUP_HUB_REGISTRATION`, `RAFTUP_HUB_PUBLIC_DOWNLOAD` ; `--public-url` passe avant la variable. Valeur invalide en base : ignorée (avertissement), on passe à la source suivante. Le journal ne garde que le **nom** du réglage changé, jamais la valeur.
- **Relecture à l'usage** : le hub de production (`AppState::from_settings`) reconstruit le service SMTP et le client Discord à chaque envoi ou connexion ; les tests gardent des services fixés (`with_services`).
- **Invitations** : code de 16 caractères en base32 de Crockford (`K7QX-2M9P-…`, 80 bits), tapé sans souci de casse ni de tirets ; seule son empreinte SHA-256 est gardée ; consommé dans la transaction qui crée le compte (deux inscriptions avec le même code : une seule réussit). Erreurs : `registration_closed`, `invite_required`, `invalid_invite` (403). Côté Discord : `needs_username` porte `invite_required: true`, et `/v1/oauth/discord/register` prend `invite_code`.
- **Journal** : types `account_created`, `account_suspended`, `account_reactivated`, `setting_changed`, `invite_created`, `invite_revoked`, `gdpr_export`, `gdpr_delete`. Les pseudos sont lus au moment de l'affichage (pas de copie) ; la suppression RGPD efface l'identifiant du compte de toutes les lignes, et la ligne `gdpr_delete` n'a ni acteur ni cible. Tout mot contenant `@` est masqué (`a…@exemple.org`), détail coupé à 200 caractères.

### Launcher relié au hub
- Le launcher garde **un seul exe signé** (ADR 0022) : on ne le modifie jamais.
- Le hub sert `GET /telecharger/raftup-windows.zip` : un zip qui contient l'exe officiel **et** un fichier `rafthub.toml` (`format_version = 1`, `hub_url = "<adresse publique>"`). Le hub récupère l'exe depuis la release GitHub (liste blanche), **vérifie sa signature** avec la clé des mises à jour avant de le servir, et le garde en cache sur le disque.
- Au démarrage, si aucun hub n'est réglé, le launcher lit `rafthub.toml` posé à côté de l'exe et adopte cette adresse (HTTPS obligatoire, sauf `127.0.0.1`). Il l'enregistre dans sa config ; le fichier n'est plus relu ensuite.
- Lien `raftup://hub/<hôte>` : ouvre le launcher, demande confirmation (« Passer sur le hub <hôte> ? »), puis règle l'adresse `https://<hôte>`. Même instance unique que l'ADR 0020.
- Précisions de la mise en œuvre côté launcher (module `launcher_core::rafthub`) :
  - `rafthub.toml` : lu sur 4 Kio au plus ; champs inconnus ignorés (un hub plus récent pourra en ajouter) ; `format_version` autre que 1, adresse refusée par les règles de l'ADR 0035 (`HubUrl::parse` : HTTPS sauf vers ce PC, sans identifiants, ni `?`, ni `#`) ou fichier illisible : défaut de l'ADR 0035 et une ligne de journal. Une fois l'adresse adoptée, `config.toml` garde `rafthub_file_adopted = true` (champ facultatif, sans changer `format_version`) : le fichier n'est plus relu, même si le joueur revient ensuite à l'adresse par défaut.
  - Formes du lien acceptées : `raftup://hub/<hôte>` (un `/` final toléré), `raftup://hub?host=<hôte>` et `raftup://hub/?host=<hôte>` (comme `join/?`, ADR 0020 ; paramètres inconnus ignorés, `host` en double refusé). Hôte ASCII `[A-Za-z0-9.-]` avec `:port` facultatif, remis en minuscules ; ni chemin, ni `user@`, ni IPv6, ni nom accentué ; 2 048 octets au plus. Le lien passe par la même boîte aux lettres d'instance unique que `raftup://join`.
  - Lien refusé : bandeau rouge dans l'écran « Amis », rien ne change. Lien vers le hub déjà en service : simple message, pas de question.
  - Confirmation : boîte « Passer sur le hub <hôte> ? » (« Passer sur ce hub » / « Rester », Échap = « Rester »), par-dessus n'importe quel écran. Changer de hub envoie la présence « hors ligne » à l'ancien (s'il y avait une session), **garde** son jeton dans le trousseau (une entrée par adresse, ADR 0035 : on peut y revenir sans se reconnecter), puis reprend la session éventuellement rangée pour le nouveau. Le même chemin sert quand l'adresse est changée à la main.
  - L'interface écrit « Rafthub » là où elle parlait du hub, et affiche l'hôte en service dans l'écran « Amis ».
- Page publique `/telecharger` (désactivable dans les réglages) : nom du hub, bouton de téléchargement, lien `raftup://hub/…`. C'est la seule page publique ; tout le reste demande une session d'admin.
- Tant qu'aucun hub officiel n'existe, le défaut du launcher reste celui de l'ADR 0035.

## Conséquences

- Une dépendance de plus dans le hub (askama) ; rien de neuf dans le launcher (lecture TOML déjà là).
- Le hub télécharge un fichier depuis GitHub : seul appel sortant nouveau, borné à la liste blanche et vérifié par signature.
- Les secrets en base sont lisibles par qui a accès à la base : c'est déjà le cas des données des joueurs. À revoir si le hub est hébergé chez un tiers.
- Le renommage du binaire touche la doc et le futur déploiement, pas l'API.
- Pas encore décidé : déploiement (domaine, serveur), journaux techniques (`tracing`) dans la console.
