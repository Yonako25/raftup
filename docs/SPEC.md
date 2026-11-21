# Raftup — cahier des charges technique

Version 1 · 11 novembre 2026 · document de référence pour le développement (humain et Claude Code).

## 1. Vision

Raftup est un **jumeau d'Archipelago écrit en Rust** : générateur, serveur, client et launcher. Il reprend le concept d'Archipelago (un randomizer multiworld où les objets d'un joueur sont cachés dans les jeux des autres), reste **compatible avec le protocole et les mods de jeux existants**, corrige les défauts connus d'Archipelago et ajoute un launcher qui rend une soirée multiworld jouable en un clic.

Public : des groupes d'amis, en synchrone ou en asynchrone. Pas les rooms géantes de plusieurs centaines de joueurs.

Positionnement : « implémentation alternative compatible », pas concurrent. Une partie qui marche sur Raftup doit marcher sur archipelago.gg, et inversement.

## 2. Problèmes d'Archipelago que Raftup corrige

| Problème | Réponse Raftup |
| --- | --- |
| Procédure d'installation différente pour chaque jeu | Profils d'installation déclaratifs, installation en un clic |
| Versions world / client / serveur incohérentes | Profil de session qui fige les versions, résolution automatique |
| Apworlds = code arbitraire, sans signature ni registre | Registre de paquets signés, niveaux de confiance (V2/V3) |
| Connexion manuelle, erreurs muettes | Rejoindre par lien, diagnostic de connexion lisible |
| Génération lente, FillError inexpliqués | Générateur Rust avec faisabilité vérifiée et erreurs expliquées (V2) |
| Serveur fragile (message géant, pickle, ws en clair) | Quotas, wss par défaut, lecture sécurisée des seeds |
| Joueur bloqué (BK) ou qui abandonne | Statut de blocage, notifications, release par vote |
| Suivi des parties async par outils tiers | Fil d'activité, résumé de retour, webhook Discord |

## 3. Architecture

```
Launcher (egui)  ──► launcher-core ──► installe mods / profils, lance le jeu avec les identifiants
      │                    │
      │                    ├──► py-bridge ──► générateur Archipelago officiel (process séparé, MVP)
      │                    │                    └─► fichier .archipelago
      │                    └──► server (maison)   ◄── lit .archipelago / .raftup
      │                                   ▲
      └── client-core (suivi de partie)    │  protocole Archipelago (WebSocket, wss)
                                           │
                               Jeu du joueur + mod Archipelago inchangé
```

### Crates

| Crate | Rôle | Jalon |
| --- | --- | --- |
| `ap-protocol` | Types serde de tous les paquets du protocole, dans les deux sens ; seule source de vérité (ADR 0004) | J0 |
| `client-core` | Client Archipelago maison (tokio + `ap-protocol`) : connexion, objets reçus, progression | J0 |
| `multidata` | Modèle de partie ; lecture sécurisée du `.archipelago` (pickle en liste blanche, quotas) ; format natif `.raftup` (JSON lignes, versionné, ADR 0014) | J1 |
| `server` | Serveur WebSocket maison (tokio + `ap-protocol`) | J1 |
| `py-bridge` | Lance et pilote le générateur officiel dans un process Python séparé | J3 |
| `launcher-core` | Catalogue, profils, installation, détection des jeux, lancement | J3 |
| `launcher-ui` | Interface egui (ADR 0011) | J3 |
| `logic`, `fill`, `generate` | Générateur Rust | V2 |
| `worlds-native` | Format de world natif déclaratif | V3 |
| `hub` | Service `raftup-hub` : comptes, amis, présence (AGPL) | V2 |

### Décisions déjà prises

- **Protocole, serveur et client maison** : `ap-protocol`, `server` et `client-core` sont écrits par Raftup, sans dépendre de pahoa ni de `archipelago_rs` ; ces projets et le serveur Python officiel servent de références et d'oracles de test (ADR 0004).
- **Génération au MVP** : 100 % par le générateur Archipelago officiel, version épinglée. Aucune règle Python ne traverse la frontière au MVP.
- **Process Python** : python-build-standalone téléchargé au premier usage, un environnement par version d'Archipelago ; dialogue JSON-RPC sur stdin/stdout (un message JSON par ligne), sans port réseau ouvert.
- **Pas de PyO3** (Python embarqué dans le process du launcher) : packaging difficile, pas d'isolation.
- **Une version d'Archipelago épinglée par version de Raftup**, tests de non-régression sur des seeds de référence.
- **Catalogue public** : le catalogue entier (profils d'installation) vit dans le dépôt, sous `profiles/`, sans base de données privée, embarqué dans chaque version du launcher ; contributions par pull request (ADR 0001).
- **Comptes et amis** : service séparé `raftup-hub` (PostgreSQL), vrai compte avec connexion par e-mail + mot de passe ou Discord (d'autres fournisseurs plus tard), pseudo unique (sans code ami), après le MVP (ADR 0002). Code public, base de l'instance officielle privée, sur le modèle de Modrinth.
- **Licences** : MIT / Apache-2.0 partout, sauf `raftup-hub` en AGPL-3.0-or-later (ADR 0003).
- **Interface du launcher** : egui / eframe, 100 % Rust, sans webview ; remplace Tauri (ADR 0011).

## 4. Périmètre par jalon

### MVP

| Jalon | Contenu | Critère de « terminé » |
| --- | --- | --- |
| J0 · Bases Rust | `ap-protocol` + `client-core` : une CLI qui se connecte à une vraie room, affiche les joueurs, les objets reçus et la progression | Connexion réussie à une room archipelago.gg ; tests sur des paquets réels capturés |
| J1 · « Je joue seul » | `server` maison, qui charge un `.archipelago` généré par AP | Une partie solo d'un premier jeu jouée jusqu'au bout avec le mod officiel non modifié |
| J2 · « On joue à 4 » | 3 à 4 jeux PC sans ROM ; hints, DeathLink, release/collect, sauvegarde et reprise | Une partie à 4 joueurs sur 4 jeux différents terminée ; reprise après redémarrage du serveur sans perte |
| J3 · Launcher | Détection des jeux, installation par profil, génération via le process Python, hébergement en un clic, rejoindre par lien, salon de préparation, webhook Discord | Sur une VM Windows propre : lien d'invitation → jeu lancé et connecté en moins de 5 min, sans saisie manuelle quand le mod le permet |

Catalogue MVP (à confirmer) : Slime Rancher, Risk of Rain 2, Stardew Valley, Hollow Knight, A Short Hike. **Premier jeu : Slime Rancher (le premier, pas Slime Rancher 2).**

Slime Rancher passe par une world communautaire, pas une world intégrée à Archipelago :
- mod client : Slimipelago (SWCreeperKing), sur MelonLoader 0.7.3 ; apworld `slime_rancher` distribuée via l'index communautaire des apworlds ;
- connexion par un menu dans les options du jeu : méthode `manual` au départ (F04) ;
- l'apworld doit être ajoutée au générateur officiel épinglé, et sa version doit correspondre à celle du mod.

### V2

- Générateur Rust (`logic`, `fill`, `generate`) pour les worlds natives et celles qui exportent leurs règles.
- Export des règles depuis les apworlds : module de règles déclaratives d'Archipelago d'abord, puis traçage des lambdas, sinon règle « opaque » évaluée par Python ; un monde avec trop de règles opaques reste généré en Python.
- Faisabilité vérifiée avant génération, erreurs expliquées (clause qui échoue).
- Durée estimée par joueur, équilibrage par temps de jeu.
- Suivi async : résumé de retour, notifications, release par vote, mode absence.
- Registre de paquets avec CLI de publication.

### V3

- Format de world natif déclaratif (TOML + mini-langage de règles), spécifié dans un dépôt séparé et versionné.
- Outil de conversion assistée apworld → natif, avec export inverse en apworld.
- Niveaux de confiance des paquets attribués par CI (tests, fuzzing).

### V4

- Mode économie : échanges libres de ressources hors seed entre joueurs, rareté de 1 à 100, serveur notaire. Détail du design dans le cahier des charges fonctionnel (onglet Brainstorm).

### Hors périmètre

- Modifier les mods de jeux Archipelago existants.
- Distribuer des jeux, ROMs, assets ou clés.
- Jeux de consoles Nintendo actuelles dans le catalogue public.
- Rooms de plusieurs centaines de joueurs.

## 5. Exigences fonctionnelles du MVP

| ID | Le launcher doit… |
| --- | --- |
| F01 | Détecter les jeux installés (Steam, Epic, dossier choisi à la main) |
| F02 | Installer, pour chaque jeu, mod loader + mod client + world à partir d'un profil déclaratif |
| F03 | Figer les versions dans un profil de session (lockfile) partagé par l'hôte |
| F04 | Rejoindre par lien et transmettre adresse, slot et mot de passe au jeu selon la méthode déclarée par le profil (`cli_args`, `config_file`, `env`, `uri`) ; sinon repli `manual` : copie dans le presse-papiers + instructions |
| F05 | Expliquer chaque échec de connexion (refus, TLS, timeout, mauvaise version, mauvais slot) |
| F06 | Proposer des presets d'options (Rapide, Standard, Long) avant tout éditeur avancé |
| F07 | Conserver la configuration utilisateur entre les mises à jour |
| F08 | Proposer un salon de préparation : invitation, choix du jeu, statut « prêt ? » testé par une connexion réelle |
| F09 | Héberger la partie en un clic (serveur intégré) avec une adresse stable |
| F10 | Afficher joueurs, progression et dernière activité ; poster les événements clés sur un webhook Discord |
| F11 | Désinstaller proprement un mod et remettre le jeu dans son état d'origine |
| F12 | Se mettre à jour automatiquement (signature minisign) |

### Exemple de profil d'installation (illustratif, valeurs à vérifier)

```toml
format_version = 1

[game]
id = "risk-of-rain-2"
ap_game = "Risk of Rain 2"
detect = [{ steam_appid = 632360 }]

[[steps]]
kind = "mod_loader"
name = "BepInEx"
version = "x.y.z"
sha256 = "..."

[[steps]]
kind = "mod"
source = "thunderstore:<auteur>-<paquet>"
version = "x.y.z"

[launch]
exe = "{game_dir}/Risk of Rain 2.exe"
connect = { method = "manual" }   # à remplacer par la vraie méthode après vérification du mod
```

## 6. Exigences non fonctionnelles

| ID | Exigence | Critère |
| --- | --- | --- |
| NF01 | Protocole Archipelago identique | Matrice de clients (Python, C#, BizHawk/Lua, C++, JS) : connexion, check, objet reçu, `!hint`, DeathLink, reconnexion |
| NF02 | Tests de conformité du serveur | Vecteurs de référence produits par le serveur Python + test différentiel |
| NF03 | Refuser un message > 1 Mio ou un chat > 2 000 caractères sans déconnecter les autres | Test automatisé |
| NF04 | Limiter chaque connexion à 50 paquets/s et 5 messages de chat / 10 s | Test automatisé |
| NF05 | Ping toutes les 20 s, fermeture d'une connexion muette après 40 s | Test automatisé |
| NF06 | wss par défaut ; ws en clair uniquement sur option explicite | Configuration par défaut |
| NF07 | Écrire chaque check et objet sur disque avant l'acquittement | Zéro perte après arrêt brutal |
| NF08 | Refuser une seed > 64 Mio décompressés ; aucun pickle exécuté | Seed piégée de test rejetée |
| NF09 | Mots de passe hachés (argon2), jamais dans les sauvegardes ni les logs | Revue + test |
| NF10 | Binaire autonome, sans interpréteur à installer, sans droits admin | Installation sur un compte utilisateur standard |
| NF11 | Windows d'abord ; macOS et Linux ensuite | CI sur les 3 OS avant la V2 |
| NF12 | Génération reproductible à seed, versions et options identiques (V2) | Même empreinte sur deux OS |

## 7. Identité visuelle

Le nom est **Raftup** (un raft-up : plusieurs bateaux amarrés ensemble). Logo : planches croisées reliées par un maillon central (fichier fourni à part).

| Rôle | Nom | Valeur |
| --- | --- | --- |
| Fond le plus sombre | Coque | `#111316` |
| Fond principal | Acier | `#16181B` |
| Cartes, panneaux | Pont | `#202328` |
| Bordures | Rivet | `#30343A` |
| Texte secondaire | Brume | `#A7ADB5` |
| Texte principal | Écume | `#ECEEF1` |
| Accent (actions, sélection) | Signal | `#4C9AFF` |
| Fond d'accent | Abysse | `#16294A` |
| Alerte | Balise | `#F2B45C` |
| En cours | Lueur | `#A78BFA` |
| Erreur | Récif | `#F26D6D` |

Règles :

- Angles de **2 px maximum**, partout (boutons, cartes, badges, avatars).
- Bordures de 1 px ; un seul liseré coloré pour signaler un état.
- Fond d'ambiance dégradé (halos flous) et panneaux semi-transparents, dans l'esprit de GOG Galaxy ou des thèmes Discord. Thèmes : Acier (défaut), Océan, Braise, Uni.
- Police **Chakra Petch** pour le logo et les titres, **Inter** pour l'interface.
- Structure de l'accueil : barre latérale texte (Jeux / Parties / Catalogue, compteurs), barre du haut (précédent / suivant, « Créer un salon », « Rejoindre par lien »), centre (joués récemment, parties en cours, paquets mis à jour), panneau d'amis à droite.
- Jaquettes : illustrations servies par le hub depuis une API qui permet le cache (IGDB, ADR 0047), jamais dans le dépôt ni dans le binaire ; aplats colorés avec le titre du jeu sinon.

Maquettes de référence : canevas « Raftup — maquette du launcher » (Accueil, Salon, Catalogue, Partie).

## 8. Conduite envers la communauté Archipelago

1. Nom et logo distincts, mention « projet non officiel, compatible avec Archipelago ».
2. Crédits : notices MIT de tout code porté (Archipelago, autres projets) ; auteurs des worlds affichés dans le launcher.
3. Compatibilité stricte et tests publics ; aucun paquet réseau propriétaire sans repli.
4. Remonter bugs et clarifications du protocole en amont ; prévenir les mainteneurs sur Discord avant toute annonce publique.
5. Aucune world exclusive à Raftup.

## 9. Hypothèses à vérifier avant d'écrire du code

- [ ] Comment le mod Archipelago du premier jeu reçoit hôte, slot et mot de passe (argument, fichier de config, saisie en jeu). Slimipelago : menu en jeu ; vérifier s'il existe aussi un fichier de config.
- [ ] Slimipelago n'affiche pas de licence : demander à l'auteur l'autorisation d'installation automatique (référencement par URL + hash) avant de l'ajouter au catalogue.
- [ ] L'apworld `slime_rancher` fonctionne avec la version d'Archipelago épinglée, et la durée d'une partie complète reste raisonnable pour le critère de J1.
- [ ] Le lecteur `.archipelago` de `multidata` charge une seed générée par la version d'Archipelago épinglée, et le serveur maison tient une partie complète avec ce jeu.
- [x] `client-core` se connecte à une vraie room et reçoit les objets (15 novembre 2026 : salle archipelago.gg en wss, exemple `suivi`).
- [ ] Le générateur officiel tourne seul via python-build-standalone, sans Python installé, sans droits admin.
- [ ] Les licences des mods du catalogue MVP autorisent l'installation automatique (référencement par URL + hash, pas de ré-hébergement).

## 10. Risques

| Risque | Parade |
| --- | --- |
| Archipelago change son protocole ou l'API des worlds | Version épinglée, tests sur seeds de référence, suivi des releases |
| Écart de comportement avec le serveur officiel | Tests différentiels contre le serveur Python, vecteurs de paquets réels, matrice de clients (NF01, NF02) |
| Maintenance des profils par jeu | Versions épinglées, test automatique par profil, jeu cassé marqué indisponible |
| Courbe d'apprentissage de Rust | Jalons courts, code simple, commentaires pédagogiques |
| Antivirus / SmartScreen | Signature de code (SignPath pour l'open source, à vérifier) |
| Code tiers des worlds | Process séparé sans réseau au MVP, sandbox par OS en V2 |
