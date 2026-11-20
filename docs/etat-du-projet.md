# État du projet

> [!IMPORTANT]
> - **J1 presque fini** : protocole complet, limites, tests différentiels, wss ; Slime Rancher connecté en vrai avec le mod non modifié (étape 9), reste à finir une partie jusqu'à l'objectif (critère de la SPEC)
> - **J2 fini** : indices, commandes `!`, sauvegarde `.raftup` avec reprise, captures différentielles comprises
> - **J3 bien avancé côté logique** : `launcher-core` (profils, installation, détection, lancement), `py-bridge` et l'étape `generate` sont faits et testés avec le **vrai Archipelago 0.6.8** (APQuest) ; `generate_and_host` génère puis héberge en un appel
> - **J3 côté interface** : hébergement, habillage et écran Bibliothèque faits ; Bibliothèque regardée à l'œil (vide et avec un profil de démo), le reste jamais contrôlé
> - Aucun test ne casse sur le workspace (`cargo test --workspace`) ; les tests réels sont `#[ignore]`
> - Différentiel : **0 écart** sur 24 scénarios fixes (dont `commandes` et `indices`) et 100 au hasard avec commandes `!`, indices et `!hint` seul (graine 7), indices non triés, mesuré le 17 novembre 2026

Mis à jour le **21 novembre 2026** (V3 phase B : conversion apworld ↔ monde natif, ADR 0042 ; avant : présence « en partie » et liaison Discord dans le launcher).

Méthode de travail avec les agents : voir `docs/regles-agents.md`.

## À faire

- [ ] **V2, générateur Rust** (ADR 0027, accepté le 18 novembre 2026) : étapes 1 `logic`, 2 écriture du `.archipelago`, 3 `fill`, 4 `generate` + APQuest + `RustBackend` (case « Moteur Rust », décochée par défaut) et 5 équilibrage de progression faites ; reste : vraie partie APQuest sur seed Rust (avant de rendre le moteur Rust par défaut), étape 6 export des mondes Python (ADR 0028) : sous-étapes 1 (`export_worlds`, `generate::imported`, APQuest exporté identique au portage Rust en vrai), 2 (sondage des lambdas, contrôle croisé, refus du `collect` personnalisé) , 3 (lieux prioritaires et exclus dans `logic` et `fill`) et 4 (`finish_worlds` : Archipelago écrit la seed placée par Raftup ; `ExportBackend` avec repli Python ; case « Moteur Rust » → export quand le moteur Rust seul ne suffit pas ; `PYTHONHASHSEED=0`) faites ; étape 5 (Slime Rancher : 147 règles traduites, 0 opaque, seed identique à celle d'Archipelago seul sauf le placement, même `slot_data`) faite ; reste une vraie partie de Slime Rancher sur une seed placée par Raftup (case « Moteur Rust » cochée)

- [x] **Interface de génération** : écran « Créer une partie », étapes 1 à 4 codées (préparation, installation du générateur, génération, hébergement avec « Réessayer »), vérifiées par un test réel (`reel_generer_puis_heberger`) et regardées sur captures (macro `RAFTUP_DEV_CAPTURE`) ; contrat dans `docs/contrat-ui-generation.md`, décisions prises (ADR 0019)
- [x] **Règle de conservation de `generations/`** : purge manuelle dans l'interface (ADR 0019)
- [x] **Slime Rancher dans le catalogue** (ADR 0021 : mod sans licence, téléchargé chez l'auteur) ; reste J1 étape 9, la partie réelle
- [x] **Mondes d'Archipelago qui ne s'importaient pas** : dépendances épinglées ajoutées (requests + 3 dépendances, setuptools 80.9.0, pyevermizer, zilliandomizer en source GitHub, section `[[source_packages]]`) ; les 7 mondes s'importent et se génèrent, sauf soe (ROM demandée à la génération : message clair en 6 s, sans ROM fournie) ; installation réelle à refaire chez un joueur (nouveaux pins = réinstallation)
- [x] **Rejoindre par lien d'invitation** (F04, ADR 0020) : liens `raftup://` par joueur, écran « Rejoindre », instance unique, schéma enregistré au premier lancement du binaire livré ; essayé en vrai (2e instance qui passe le lien)
- [x] Clic sur un lien `raftup://` (18 novembre 2026) : marche depuis le navigateur (forme `join/?` acceptée) ; Discord ne le rend pas cliquable, il faut le coller dans « Invitations »
- [x] **F08 salon de préparation** (côté hôte, sans hub) : onglet « Salon », test « prêt ? » par vraie connexion (ADR 0024) ; jamais regardé à l'écran
- [x] **F10 webhook Discord** : branché dans « Héberger » ; champ aussi dans « Créer », URL mémorisée en clair (ADR 0025) ; jamais essayé avec un vrai webhook
- [x] **F06 presets** : délai + `meta.yaml` (Rapide, Long), choix mémorisé (ADR 0023) ; test réel du meta non lancé
- [x] **F12 mise à jour** (ADR 0022) : outil `update_tool` (keygen, sign, manifest), vraie clé publique intégrée, bannière « Installer » + remplacement de l'exe ; jamais essayé sur un vrai release ; clé privée dans `C:\Users\ionak\.raftup-keys` (à sauvegarder hors ligne) ; exe nu (décision prise)
- [ ] **Contrôle visuel du launcher** (vrai gras Inter SemiBold fait le 18 novembre) ; vus sur captures le 18 novembre : onglet « Salon », préréglages et champ webhook de « Créer », panneau d'installation du générateur
- [x] **J1, étape 9** : partie réelle de Slime Rancher sur le serveur Raftup (18 novembre 2026 : Slimipelago 0.3.2.1 connecté sans modification, objets reçus ; reprise réelle de la sauvegarde après redémarrage, sans objet en double)
- [x] « Créer une partie » reprend seul les apworlds installés par la Bibliothèque (vérifié avec Slime Rancher par la macro)
- [x] « Lancer le jeu » sans adresse ni slot : le jeu démarre sans connexion (`plan_launch_offline`)
- [x] Relire le code de `launcher-ui` (fait le 18 novembre 2026 ; reste à vérifier le redimensionnement de la fenêtre sans bordure)
- [ ] **Hub Rafthub** (binaire `rafthub`, ex-`raftup-hub`) (ADR 0002, 0003, 0033) : crate `hub` en AGPL, comptes, amis, présence, blocages, RGPD (export, suppression), e-mails (vérification, mot de passe oublié, SMTP derrière un trait), connexion Discord (OAuth 2.0 + PKCE) ; SQLite et PostgreSQL, mêmes tests sur les deux ; HTTPS intégré en option (fichiers PEM ou Let's Encrypt, ADR 0039), le proxy restant possible ; console d'admin servie à part avec `--admin-listen` (`/admin` en 404 côté public, ADR 0040) ; changement de pseudo fait le 20 novembre (`PATCH /v1/me`, une fois par 24 h ; renommage par l'admin dans la console ; champ dans l'écran « Amis ») ; reste : Let's Encrypt jamais essayé, vrai SMTP et vraie application Discord jamais essayés, déploiement sur le serveur **fait le 20 novembre : `https://hub.raftup-games.com` sur triomyno, console par le VPN** (`deploy/rafthub/` : Docker + Traefik + sauvegarde nuitière, essayé en local le 20 novembre ; domaine à choisir)
- [ ] **Console d'admin Rafthub** (ADR 0036) : console `/admin` en gabarits askama, connexion admin (cookie `rafthub_admin`, CSRF, `rafthub admin create|remove`), tableau de bord, et les quatre pages faites le 20 novembre : **Joueurs** (liste, filtres, suspendre, réactiver, invitations), **Distribution du launcher** (zip exe signé vérifié + `rafthub.toml`, page publique `/telecharger`), **Réglages** (source de chaque valeur, retour au défaut, secrets jamais réaffichés, e-mail de test), **Journaux** (fil `admin_events` filtré et paginé), toutes parcourues dans le navigateur le 20 novembre ; **installation** : assistant au premier lancement, base intégrée (SQLite dans `rafthub-data/`) ou externe (PostgreSQL, MySQL ou MariaDB, toute la suite de tests passée sur PostgreSQL 18, MariaDB 11.4 et MySQL 8.4) essayée puis gardée dans `rafthub.toml`, puis premier admin, avec un code écrit au journal et tapé une seule fois (cookie de l'étape 1, ADR 0037) ; champ « Code d'invitation » dans l'écran « Amis » du launcher fait le 20 novembre (inscription par e-mail) ; connexion Discord dans le launcher faite le 20 novembre (bouton seulement si `GET /v1/info` la propose, ADR 0035) ; reste : vrai téléchargement depuis la release GitHub
- [x] **Rafthub, couche données** (ADR 0036) : schéma en version 6, suspension des comptes, réglages en base (base > environnement > défaut), modes d'inscription et codes d'invitation, journal `admin_events` sans e-mail ni secret ; testé sur SQLite et PostgreSQL 18
- [x] **Client du hub et écran « Amis »** (ADR 0035, accepté en autonomie le 19 novembre 2026, à relire) : `launcher_core::hub_client` (API `/v1` par `ureq`, sans dépendre du hub), jeton dans le trousseau (`keyring`), adresse par défaut `http://127.0.0.1:8787` modifiable ; écran « Amis » (connexion, inscription, amis et présence, demandes, ajout par pseudo), appels dans un thread à part, présence renvoyée toutes les 60 s ; vu sur captures (macro, scénario `amis`) et essayé contre un vrai `raftup-hub` ; présence « en partie · <jeu> » (jeu lancé d'ici ou partie hébergée, nom du jeu seul, `launcher_core::activity`) et liaison / déliaison de Discord depuis le panneau « Profil » faites le 20 novembre (jamais regardées à l'écran ni essayées avec une vraie application Discord)
- [x] **Launcher relié à un Rafthub** (ADR 0036, côté launcher) : `rafthub.toml` posé à côté de l'exe adopté une fois au démarrage (`launcher_core::rafthub`), lien `raftup://hub/<hôte>` avec boîte de confirmation « Passer sur le hub … ? » (présence « hors ligne » envoyée à l'ancien hub, jeton gardé), hôte du Rafthub affiché dans « Amis » ; vu sur captures (macro, scénario `changer-de-hub`) ; côté hub, page « Distribution du launcher », page publique `/telecharger` et zip signé vérifié faits (session `rafthub-distribution`, testés sans réseau) ; reste un vrai téléchargement depuis la release GitHub
- [ ] **V2, registre de paquets signés** (ADR 0032, à relire) : crate `registry` + binaire `raftup-registry` (`keygen`, `publish`, `verify`, local, n'envoie rien), lecture vérifiée par `launcher-core::registry` ; vraie clé `A3D76747B9BA50E2` (secrète dans `C:\Users\ionak\.raftup-keys`, à sauvegarder hors ligne) ; source `registre/registre.toml` (apworld Slime Rancher) et procédure dans `registre/README.md` ; branché dans la Bibliothèque (jeux en plus marqués « Registre », « Mondes du registre » avec « Installer », échec discret) ; **reste à l'utilisatrice : créer la release GitHub `registre` et y envoyer `index.json` + `index.json.minisig`** (d'ici là : « Registre indisponible »)
- [x] **NF11 / NF12, CI multi-OS** (19 novembre 2026) : `ci.yml` en matrice Windows, macOS, Linux ; test d'empreinte sha256 d'une seed APQuest du moteur Rust (`generate/tests/empreinte.rs`) ; rfd en `xdg-portal` sous Linux ; runtime Python Windows seulement (repli : moteur Rust) ; CI verte sous macOS et Linux le 20 novembre 2026 (launcher jamais essayé à la main hors Windows)
- [x] **Bac à sable du générateur** (V2, ADR 0034, 19 novembre 2026) : worker en intégrité basse sous Windows, activé par défaut, refus s'il n'est pas appliqué ; tests réels de génération et d'export passent avec ; hors Windows, repli sans bac à sable
- [x] **Moteur « export » : règles d'objet, objets à placer tôt, accessibilité** (ADR 0029, à relire) : `item_rule` / `always_allow` (donc `local_items` / `non_local_items`), `early_items` / `local_early_items` et `accessibility: minimal` / `items` ne forcent plus le repli Python ; vérifiés en vrai sur le monde `raftup_sonde` ; pas encore essayé sur un monde tiers qui s'en sert
- [ ] **Durée estimée et équilibrage par temps** (ADR 0030, à relire) : estimation par joueur affichée après génération (`launcher_core::estimate`, panneau de « Créer une partie »), `fill::time_balance` branché derrière la case « Équilibrer par temps de jeu » (moteurs Rust et export, décochée par défaut, seed inchangée sans elle) ; estimation aussi dans « Partie en cours » (20 novembre, vue sur capture) ; reste à mesurer les temps par lieu en vraie partie
- [x] **V2, suivi async** (ADR 0031, accepté en autonomie le 19 novembre 2026, à relire) : `!away`, `!back`, `!summary`, `!vote_release`, résumé de retour à la connexion, clés `_read_raftup_…`, `.raftup` en `format_version` 2 ; actif dans le launcher, `--async_follow` pour `raftup-server` ; webhook et « Partie en cours » ; jamais regardé à l'écran ni essayé avec un vrai mod
- [ ] **V3, format de monde natif** (ADR 0041, accepté en autonomie le 20 novembre 2026, à relire) : spécification autonome dans `spec/monde-natif/` (README + exemples `apquest` et `minimal`), prête à devenir son propre dépôt ; crate `worlds-native` (TOML borné, mini-langage de règles, erreurs avec ligne et colonne, compilation vers `logic`) ; `GameOptions::Native` et lecture des YAML d'un monde natif ; APQuest natif identique au portage (`generate/tests/natif.rs` : structures, seed, spoiler, datapackage, YAML, même empreinte) ; le launcher lit `mondes-natifs/` (dossier des données) pour le moteur Rust ; **phase B faite le 21 novembre (ADR 0042, à relire)** : outil `raftup-monde` (`importer` : export JSON → `world.toml` d'un seul jeu d'options, options en commentaire, liste « À vérifier » ; `apworld` : `world.toml` → `.apworld`), ordre de création et options ajoutés à l'export, `format_version` 2 (objet d'événement partagé) ; APQuest converti = même seed que le portage ; APQuest natif en apworld généré par le vrai Archipelago (même datapackage, aller-retour avec contrôle croisé) ; Slime Rancher converti sur le PC seulement (pas de licence), généré par le moteur Rust seul ; reste : affichage des mondes natifs dans la Bibliothèque, installation par le registre, export JSON accessible depuis le launcher
- [ ] **Relire les ADR 0029 à 0035** (acceptés en autonomie le 19 novembre 2026)
- [x] **Texte officiel de l'AGPL** pour `crates/hub/LICENSE-AGPL` (texte de gnu.org récupéré le 2026-11-20, empreinte vérifiée par `tests/licence.rs`)
- [x] **CI macOS et Linux** : verte sur les trois systèmes le 20 novembre 2026 (après deux corrections de tests propres à Windows)
- [ ] **Releases avec binaires** (20 novembre 2026) : `release.yml` sur tag `v*` construit Rafthub public (Linux, Windows), Rafthub officiel (Linux) et le launcher Windows, avec `.sha256`, en brouillon de release ; signature du launcher à la main (`docs/publier-une-release.md`) ; job CI `hub-officiel` ; jamais lancé (aucun tag poussé)

## Crates

| Crate | Rôle | État |
| --- | --- | --- |
| `ap-protocol` | Types des paquets, dans les deux sens | **Complet** |
| `client-core` | Client Archipelago (ws/wss) | **Complet** pour J0 |
| `multidata` | Lecture sûre du `.archipelago`, modèle de partie, format `.raftup` | **Complet** pour J2 |
| `server` | Serveur WebSocket + binaire `raftup-server` | **Complet** pour J2 |
| `launcher-core` | Héberger, profils et installation, détection, lancement, runtime Python, `generate`, `generate_and_host` | **Logique complète** pour J3 (reste le catalogue réel) |
| `launcher-ui` | Application egui, binaire `raftup` | Commencé (J3) |
| `py-bridge` | Pont JSON-RPC vers le worker Python (`ping`, `version`, `echo`, `generate`), annulation, Job Object Windows, bac à sable en intégrité basse (ADR 0034) | **Complet** pour le MVP |
| `logic` | Modèle d'un multiworld, règles, balayage par sphères (V2) | **Fait** pour l'étape 1 de l'ADR 0027 |
| `fill` | Placement des objets (supposé), équilibrage de progression, faisabilité avant et après, erreurs expliquées (V2) | **Fait** pour les étapes 3 et 5 de l'ADR 0027 |
| `generate` | Générateur Rust : YAML → mondes Rust (APQuest) → placement → seed `.archipelago` (V2) | **Fait** pour l'étape 4 de l'ADR 0027 |
| `hub` | Rafthub, binaire `rafthub` (AGPL) : comptes e-mail + mot de passe ou Discord, amis, blocages, présence, RGPD ; API HTTP `/v1` (V2) | **Côté serveur fait** : SQLite et PostgreSQL, e-mails et Discord derrière des traits (essayés avec de faux services seulement) ; client dans le launcher (ADR 0035) |
| `worlds-native` | Format de monde natif déclaratif (versions 1 et 2) : lecture sûre du TOML, validation, mini-langage de règles, compilation vers `logic`, export en apworld (V3) | **Fait** pour les phases A et B (ADR 0041, 0042) |
| `registry` | Registre de paquets signés : index versionné, signature minisign, publication locale (V2) | **Fait** (ADR 0032), branché dans la Bibliothèque ; release `registre` à créer |

## Ce que fait chaque crate

**ap-protocol**

- Tous les paquets serveur → client et client → serveur
- Champs inconnus gardés (`extra`), `"class"` écrit et toléré
- Vérifié sur **37 captures** du serveur et du client officiels

**client-core**

- Connexion `wss`, puis `ws` en secours ; port **38281** par défaut
- `Client::connect`, `send`, `recv`, `close`
- Exemple `suivi` : affiche joueurs, objets reçus, messages

**multidata**

- Lecteur pickle maison, **sans exécuter de code**, liste blanche officielle
- Limites : **64 Mio** décompressés (NF08), 4 millions de nœuds
- Modèle `MultiData` identique à Python sur 2 seeds (MIT, Slime Rancher)
- Écriture d'une seed `.archipelago` (`write_archipelago`, ADR 0027), relue à l'identique par le vrai Python 0.6.8
- Format `.raftup` (ADR 0014) : en-tête, instantané, deltas ; `diff` entre deux instantanés ; lecture tolérante à une dernière ligne coupée (même au milieu d'un caractère) ; création et compactage atomiques

**server**

- Une tâche « salle » possède l'état, **sur son propre thread** ; une tâche par connexion (ADR 0006, 0014)
- Connexion : `RoomInfo`, `Connect` et ses refus, `Connected`, inventaire de départ, `GetDataPackage`
- Jeu : `LocationChecks`, objets, groupes, `Sync`, `ConnectUpdate`
- Fin : `StatusUpdate`, objectif, `collect` / `release` automatiques
- Échanges : `LocationScouts`, chat `Say`, `Bounce` (DeathLink)
- Stockage : `Get`, `Set` (18 opérations), `SetNotify`, clés `_read_…`
- **Indices** (ADR 0012, 0018) : `create_as_hint`, `CreateHints`, `UpdateHint`, points et coût des indices ; rangés et parcourus comme le `set` Python de l'officiel
- **Commandes serveur** après `!admin login` : `/players`, `/status`, `/send`, `/send_location`, `/release`, `/collect`, `/allow_release`, `/hint`, `/save`…
- **Commandes `!`** (ADR 0013) : `help`, `license`, `options`, `players`, `status`, `remaining`, `missing`, `checked`, `release`, `collect`, `alias`, `getitem`, `countdown`, `hint`, `hint_location`, `admin login` / `logout`
- **Sauvegarde** (ADR 0014, NF07) : écrite et synchronisée sur le disque **avant** chaque réponse ; reprise au démarrage ; compactage au démarrage, à 10 000 deltas et à l'arrêt
- Limites (ADR 0007) : message 1 Mio, chat 2 000 caractères, 50 paquets/s, 5 chats / 10 s, ping 20 s, muet 40 s, files bornées
- Chiffrement (ADR 0009) : `wss` avec `--cert` et `--cert_key` (fichiers PEM) ; clair seulement avec `--no-tls`
- Binaire : `raftup-server <seed> --cert fullchain.pem --cert_key privkey.pem [--savefile X.raftup | --disable_save] [--async_follow]`
- **Suivi async** (ADR 0031, avec l'option) : `!away`, `!back`, `!summary`, `!vote_release`, résumé de retour, réponse automatique quand on cite un absent, clés `_read_raftup_away_<équipe>` et `_read_raftup_release_votes_<équipe>`
- Tests différentiels (ADR 0008) : `tests/differentiel/`, lancés à la main, hors CI

**launcher-core** (ADR 0010)

- `host` : héberger une partie, la suivre (`status`, fil d'événements), l'arrêter
- `seed` : résumé d'une seed ; `logs` : journal en mémoire, filtré
- `hub_client` (ADR 0035) : client bloquant de l'API `/v1` du hub (inscription, connexion, déconnexion, amis, demandes, présence, liaison et déliaison de Discord), erreurs typées avec le code stable du hub, jeton par adresse de hub dans le trousseau ; testé contre un faux serveur HTTP, et contre un vrai hub (`#[ignore]`, `RAFTUP_HUB_TEST_URL`)
- `rafthub` (ADR 0036) : lecture et adoption unique de `rafthub.toml`, analyse des liens `raftup://hub/…` (rend l'adresse `https://<hôte>`)

**launcher-ui** (ADR 0011)

- egui / eframe (wgpu), binaire `raftup`
- Écrans « Héberger une partie », « Créer une partie » (préparation, installation du générateur, génération, hébergement), « Partie en cours » et « Bientôt », d'après la maquette
- Polices Chakra Petch et Inter intégrées (OFL)
- Fenêtre sans bordure redimensionnable par ses bords et ses coins (`resize.rs`)
- Écran « Bibliothèque » : jeux du catalogue, détection, Installer / Lancer / Désinstaller dans un thread à part ; en debug, `RAFTUP_DEV_PAGE`, `RAFTUP_DEV_PROFILS`, `RAFTUP_DEV_RACINE` pour les captures
- En debug, macro `RAFTUP_DEV_CAPTURE=<dossier>` (+ `RAFTUP_DEV_YAML`, `RAFTUP_DEV_PORT`) : joue « Créer une partie » et enregistre des captures PNG, sans être devant le PC
- Registre : `RAFTUP_NO_REGISTRY` coupe sa lecture ; en debug, `RAFTUP_DEV_REGISTRE=<dossier>` lit un registre signé local (`index.json`, `.minisig` et paquets nommés comme le dernier segment de leur adresse)
- Écran « Amis » (ADR 0035) : connexion ou nouveau compte, amis triés par présence, demandes reçues (Accepter / Refuser) et envoyées (Annuler), ajout par pseudo, retrait confirmé, adresse du hub ; cartes « Amis » et profil de la barre latérale branchées ; en debug `RAFTUP_DEV_HUB` (jeton en mémoire) et `RAFTUP_DEV_HUB_LOGIN=<e-mail>:<mot de passe>` ; `RAFTUP_DEV_HUB_INSCRIPTION=1` ouvre l'onglet « Nouveau compte »
- Case « Moteur Rust (expérimental) » dans « Créer une partie » : APQuest généré sans Python (ADR 0027), sinon Archipelago
- Fin de génération : moteur réellement utilisé (Rust seul, export placé par Raftup, Archipelago seul) et raison d'un repli affichés ; le moteur Rust seul écrit `AP_<seed>_Spoiler.txt` dans le zip (19 novembre 2026)
- `cargo run -p launcher-ui` démarre sans erreur ; **rendu, boîtes de fichiers et presse-papiers jamais contrôlés à l'œil**
- Sauvegarde la partie hébergée dans un `.raftup` à côté de la seed, et la reprend au prochain hébergement (`HostOptions::disable_save` pour s'en passer, pas encore dans l'interface)
- Fermer la fenêtre arrête la partie proprement (sauvegarde compactée), 5 s au plus

## Décisions (ADR)

- **0001** : catalogue public dans le dépôt
- **0002** : hub de comptes et d'amis
- **0003** : hub en AGPL
- **0004** : protocole, serveur et client écrits par Raftup
- **0005** : WebSocket avec tokio-tungstenite + rustls ; pas de compression
- **0006** : serveur organisé autour d'une tâche « salle »
- **0007** : limites des connexions et files bornées
- **0008** : tests différentiels contre le serveur officiel
- **0009** : wss par certificat PEM
- **0010** : architecture du launcher (la partie interface est remplacée par 0011)
- **0011** : interface du launcher en egui, à la place de Tauri
- **0012** : indices (hints) comme le serveur officiel, tirage déterministe
- **0013** : commandes `!` du chat
- **0014** : sauvegarde `.raftup` en JSON lignes
- **0015** : profils d'installation, installation, lockfile et lancement
- **0016** : `py-bridge`, pont JSON-RPC vers le worker Python
- **0017** : étape `generate` (Python embarqué, générateur officiel épinglé)
- **0018** : ordre des indices comme un `set` Python (module `pyset`)
- **0019** : écran « Créer une partie » : purge manuelle, YAML par sélecteur, réseau du générateur bloqué par défaut
- **0020** : rejoindre par lien `raftup://`, sans mot de passe, instance unique
- **0028** : export des mondes Python vers le moteur Rust : `export_worlds` / `finish_worlds`, sondage des lambdas, contrôle croisé, repli Python
- **0027** : générateur Rust (V2) : crates `logic` / `fill` / `generate`, sortie `.archipelago`, seed reproductible dans Raftup (ChaCha), APQuest en premier monde, Python en secours
- **0033** : `raftup-hub` : axum, sqlx `Any` (SQLite et PostgreSQL), jetons de session opaques hachés et révocables, blocages, RGPD, e-mails (`lettre`), Discord OAuth 2.0 + PKCE (accepté en autonomie, ajouts à relire)
- **0032** : registre de paquets signés : index JSON versionné (URL + sha256), une signature minisign pour l'index, clé distincte des mises à jour, catalogue embarqué prioritaire (accepté en autonomie, à relire)
- **0037** : Rafthub : assistant de premier lancement (base intégrée SQLite ou externe PostgreSQL / MySQL / MariaDB, gardée dans `rafthub-data/rafthub.toml`, puis premier admin), protégé par un code d'installation écrit au journal (accepté en autonomie, à relire)
- **0038** : Rafthub officiel (option `officiel`, connexion Discord possible) et auto-hébergé (pseudo ou e-mail et mot de passe seulement) ; connexion par pseudo ou e-mail partout (accepté le 20 novembre 2026, mise en œuvre à relire)
- **0039** : HTTPS intégré à Rafthub en option (fichiers PEM ou Let's Encrypt), le proxy restant possible (accepté en autonomie le 20 novembre 2026, à relire)
- **0040** : Rafthub officiel : console d'admin servie à part (`--admin-listen`, IP du VPN), `/admin` en 404 côté public ; Cloudflare devant le hub ; adresse officielle du launcher donnée à la compilation (`RAFTUP_OFFICIAL_HUB_URL`) (accepté le 20 novembre 2026)
- **0034** : bac à sable du générateur : sous Windows, worker en intégrité basse (copie de `python.exe` étiquetée par `icacls`), écriture seulement dans le dossier de la génération ; rien hors Windows (à relire par l'utilisatrice)
- **0029** : moteur « export » : règles d'objet en listes calculées par Archipelago (contrôle croisé), objets à placer tôt en sphère 0, `accessibility` (accepté en autonomie, à relire)
- **0030** : durée estimée par joueur (temps par lieu dans le profil, sphères de la seed) et équilibrage par temps de jeu (`fill::time_balance`, sur option, décochée par défaut) ; accepté en autonomie, à relire
- **0036** : Rafthub : console d'admin web servie par le hub ; launcher relié au hub par `rafthub.toml` ou un lien `raftup://hub/<hôte>` confirmé (accepté en autonomie, à relire)
- **0035** : client du hub dans le launcher : sans dépendre de la crate AGPL, `ureq` bloquant hors du thread d'interface, jeton dans le trousseau (`keyring`), adresse par défaut locale, HTTPS sauf vers ce PC (accepté en autonomie, à relire)
- **0041** : format de monde natif déclaratif (TOML versionné + mini-langage de règles), compilé vers `logic` sans nouveau moteur ; spécification dans `spec/monde-natif/` ; APQuest natif identique au portage (accepté en autonomie, à relire)
- **0031** : suivi async (option de la salle, désactivée par défaut) : absence, résumé de retour, release par vote à la majorité, fil d'activité, `.raftup` version 2

## Reporté à la fin de la V3 (décision du 18 novembre 2026)

- Essais de validation des jalons du MVP : partie solo menée à l'objectif (J1), partie à 4 joueurs sur 4 jeux (J2), test sur une VM Windows propre (J3). Le développement continue sans les attendre.

## Reporté à J3

- Certificat automatique (Let's Encrypt) et rechargement sans redémarrage (ADR 0009)
- `py-bridge`, catalogue et profils, installation des mods, détection des jeux, rejoindre par lien, salon, webhook Discord

## Vérifier

```bash
cargo test --workspace
```

```bash
cargo run -p server --bin raftup-server -- crates/multidata/tests/data/seed-mit.archipelago --no-tls --disable_save
```

```bash
cargo run -p client-core --example suivi -- 127.0.0.1:38281 Bob
```

```bash
cargo run -p launcher-ui
```

Tests différentiels : voir `crates/server/tests/differentiel/README.md`.

## Mesures

- `DataPackage` complet de la seed MIT : **26 298 octets** sans compression, **5 732** compressé (zlib) : Raftup, sans compression (ADR 0005), envoie environ 4,6 fois plus. Acceptable pour l'instant : envoyé une fois par client et par jeu.

<details>
<summary>Écarts connus avec le serveur officiel</summary>

**Protocole**

- Entiers plus grands qu'un `i64` non gérés dans le stockage
- `mul` refuse un résultat de plus d'un million d'éléments
- `Bounce` dont `data` n'est pas un objet : refusé chez Raftup, accepté par le serveur officiel
- Paquet sans `cmd` : les caractères Unicode « non imprimables » ne sont pas échappés comme en Python dans la réponse
- Plantages du serveur officiel reproduits par une fermeture de connexion : changement de slot, connexion d'un groupe, lieu inconnu dans `LocationScouts`
- `ConnectUpdate` avec `"tags": null` : traité comme un champ absent ; le serveur officiel fait `set(None)`, plante et ferme la connexion
- Pas d'avertissement « your client does not support compressed websocket connections! » à la connexion : Raftup ne compresse pas (ADR 0005), le message n'aurait pas de sens

**Commandes et indices** (ADR 0012, 0013)

- `!options` masque le mot de passe de la salle (le serveur officiel l'affiche)
- `!license` affiche la licence de Raftup
- Erreurs d'arguments : texte court à la place d'une trace Python
- `!hint` : tirage déterministe entre ex æquo, au lieu du Mersenne Twister
- `!hint` : noms candidats dans l'ordre du datapackage (un ensemble, donc au hasard, en Python)
- `!hint` : non indiçables connus seulement pour les mondes de base de 0.6.8 (« Triforce » d'A Link to the Past, « Battle Arena: End reward » de Circle of the Moon) ; ceux d'un `.apworld` installé à part ne sont pas connus
- `!missing`, `!checked`, indices d'un objet : lieux par numéro croissant, comme `_speedups` (le serveur officiel compilé) ; seul son repli en pur Python, qui affiche un avertissement, suit l'ordre du multidata
- Ordre des indices (ADR 0018) : exact si `entrance` est vide, ou si l'officiel tourne avec `PYTHONHASHSEED=0` ; la sauvegarde automatique de l'officiel (toutes les 60 s) peut, rarement, changer son ordre
- `!countdown` au-delà d'une heure : message court au lieu d'une trace
- `!admin /…` : `/help`, `/license`, `/options`, `/exit`, `/alias`, `/option`, `/datastore` sont refusées (« Server command /x is not available in Raftup yet. ») ; les autres commandes serveur sont reprises
- `!admin /…` : erreurs d'arguments en texte court (`Invalid arguments for /send.`), et `/send_multiple` au-delà de 100 ou `/countdown` au-delà d'une heure répondent par un message au lieu d'une trace
- `!admin /allow_release` n'est pas sauvegardé (comme l'officiel) ; `/save` force une écriture du fichier `.raftup` et répond « Game saved »

</details>
