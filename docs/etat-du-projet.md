# État du projet

> [!IMPORTANT]
> - **J1 : étapes 1 à 8 faites** sur 9 : protocole complet, limites, tests différentiels, wss
> - **J2 fini** : indices, commandes `!`, sauvegarde `.raftup` avec reprise, captures différentielles comprises
> - **J3 bien avancé côté logique** : `launcher-core` (profils, installation, détection, lancement), `py-bridge` et l'étape `generate` sont faits et testés avec le **vrai Archipelago 0.6.8** (APQuest) ; `generate_and_host` génère puis héberge en un appel
> - **J3 côté interface** : hébergement, habillage et écran Bibliothèque faits ; Bibliothèque regardée à l'œil (vide et avec un profil de démo), le reste jamais contrôlé
> - Aucun test ne casse sur le workspace (`cargo test --workspace`) ; les tests réels sont `#[ignore]`
> - Différentiel : **0 écart** sur 24 scénarios fixes (dont `commandes` et `indices`) et 100 au hasard avec commandes `!`, indices et `!hint` seul (graine 7), indices non triés, mesuré le 17 novembre 2026

Mis à jour le **17 novembre 2026**.

Méthode de travail avec les agents : voir `docs/regles-agents.md`.

## À faire

- [ ] **Interface de génération** : écran « Créer une partie », étape 1 (préparation) faite ; reste le runtime (consentement au téléchargement d'environ 72 Mo), la génération (progression, annulation), l'hébergement et les erreurs ; contrat dans `docs/contrat-ui-generation.md`, décisions prises (ADR 0019)
- [x] **Règle de conservation de `generations/`** : purge manuelle dans l'interface (ADR 0019)
- [ ] **Slime Rancher dans le catalogue** : brouillon dans `docs/profils-en-attente/` ; il manque l'autorisation écrite de l'auteur de Slimipelago (brouillon de demande dans le même dossier)
- [ ] **Mondes d'Archipelago qui ne s'importent pas** faute de dépendances épinglées : ffmq, messenger, osrs, sm (`requests`), pokemon_emerald (`pkg_resources`), soe (`pyevermizer`), zillion (`zilliandomizer`) ; laissés pour l'instant
- [ ] **Rejoindre par lien d'invitation**, salon de préparation, webhook Discord, presets, mise à jour automatique (F06, F08, F10, F12)
- [ ] **Contrôle visuel du launcher**, et un vrai gras (Inter SemiBold en famille séparée)
- [ ] **J1, étape 9** : partie réelle de Slime Rancher sur le serveur Raftup
- [ ] Relire le code de `launcher-ui` (le serveur, `launcher-core` et `py-bridge` ont été relus)
- [ ] Test sur une VM Windows propre (critère de fin de J3)

## Crates

| Crate | Rôle | État |
| --- | --- | --- |
| `ap-protocol` | Types des paquets, dans les deux sens | **Complet** |
| `client-core` | Client Archipelago (ws/wss) | **Complet** pour J0 |
| `multidata` | Lecture sûre du `.archipelago`, modèle de partie, format `.raftup` | **Complet** pour J2 |
| `server` | Serveur WebSocket + binaire `raftup-server` | **Complet** pour J2 |
| `launcher-core` | Héberger, profils et installation, détection, lancement, runtime Python, `generate`, `generate_and_host` | **Logique complète** pour J3 (reste le catalogue réel) |
| `launcher-ui` | Application egui, binaire `raftup` | Commencé (J3) |
| `py-bridge` | Pont JSON-RPC vers le worker Python (`ping`, `version`, `echo`, `generate`), annulation, Job Object Windows | **Complet** pour le MVP |

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
- Format `.raftup` (ADR 0014) : en-tête, instantané, deltas ; `diff` entre deux instantanés ; lecture tolérante à une dernière ligne coupée (même au milieu d'un caractère) ; création et compactage atomiques

**server**

- Une tâche « salle » possède l'état, **sur son propre thread** ; une tâche par connexion (ADR 0006, 0014)
- Connexion : `RoomInfo`, `Connect` et ses refus, `Connected`, inventaire de départ, `GetDataPackage`
- Jeu : `LocationChecks`, objets, groupes, `Sync`, `ConnectUpdate`
- Fin : `StatusUpdate`, objectif, `collect` / `release` automatiques
- Échanges : `LocationScouts`, chat `Say`, `Bounce` (DeathLink)
- Stockage : `Get`, `Set` (18 opérations), `SetNotify`, clés `_read_…`
- **Indices** (ADR 0012, 0018) : `create_as_hint`, `CreateHints`, `UpdateHint`, points et coût des indices ; rangés et parcourus comme le `set` Python de l'officiel
- **Commandes `!`** (ADR 0013) : `help`, `license`, `options`, `players`, `status`, `remaining`, `missing`, `checked`, `release`, `collect`, `alias`, `getitem`, `countdown`, `hint`, `hint_location`, `admin login` / `logout`
- **Sauvegarde** (ADR 0014, NF07) : écrite et synchronisée sur le disque **avant** chaque réponse ; reprise au démarrage ; compactage au démarrage, à 10 000 deltas et à l'arrêt
- Limites (ADR 0007) : message 1 Mio, chat 2 000 caractères, 50 paquets/s, 5 chats / 10 s, ping 20 s, muet 40 s, files bornées
- Chiffrement (ADR 0009) : `wss` avec `--cert` et `--cert_key` (fichiers PEM) ; clair seulement avec `--no-tls`
- Binaire : `raftup-server <seed> --cert fullchain.pem --cert_key privkey.pem [--savefile X.raftup | --disable_save]`
- Tests différentiels (ADR 0008) : `tests/differentiel/`, lancés à la main, hors CI

**launcher-core** (ADR 0010)

- `host` : héberger une partie, la suivre (`status`, fil d'événements), l'arrêter
- `seed` : résumé d'une seed ; `logs` : journal en mémoire, filtré

**launcher-ui** (ADR 0011)

- egui / eframe (wgpu), binaire `raftup`
- Écrans « Héberger une partie », « Créer une partie » (préparation seule), « Partie en cours » et « Bientôt », d'après la maquette
- Polices Chakra Petch et Inter intégrées (OFL)
- Écran « Bibliothèque » : jeux du catalogue, détection, Installer / Lancer / Désinstaller dans un thread à part ; en debug, `RAFTUP_DEV_PAGE`, `RAFTUP_DEV_PROFILS`, `RAFTUP_DEV_RACINE` pour les captures
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

## Reporté à J3

- Commandes serveur après `!admin login` (`/players`, `/send`…)
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
- `!release` : pas d'autorisation par joueur depuis la console (`/allow_release`)
- `!admin` : après connexion, les commandes serveur répondent « Server commands are not available yet. » (J3)

</details>
