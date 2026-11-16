# État du projet

> [!IMPORTANT]
> - **J1 : étapes 1 à 8 faites** sur 9 : protocole complet, limites, tests différentiels, wss
> - **J2 presque fini** : indices, commandes `!`, sauvegarde `.raftup` avec reprise. Reste T4 et les captures différentielles
> - **J3 commencé** : le launcher egui héberge une partie (rendu jamais contrôlé à l'œil)
> - **202 tests** passent, dont un arrêt brutal du vrai binaire sans perte
> - Différentiel : **0 écart** sur 22 scénarios fixes et 150 au hasard, mesuré **avant J2**. Les scénarios `COMMANDES` et `INDICES` ne sont pas encore capturés

Mis à jour le **16 novembre 2026**.

## À faire

- [ ] **T4** : liste des objets non indiçables (`hint_blacklist`), à générer depuis le Python d'Archipelago 0.6.8
- [ ] **Captures** des scénarios `COMMANDES` et `INDICES` sur le serveur officiel (il faut un Python avec les dépendances d'Archipelago)
- [ ] **Contrôle visuel du launcher**, et un vrai gras (Inter SemiBold en famille séparée)
- [ ] **J1, étape 9** : partie réelle de Slime Rancher sur le serveur Raftup
- [ ] Demander à l'auteur de Slimipelago l'autorisation d'installation automatique
- [ ] Relire `connect.rs`, `messages.rs`, `monitor.rs` et le launcher (pas couverts par la relecture de J2)

## Crates

| Crate | Rôle | État |
| --- | --- | --- |
| `ap-protocol` | Types des paquets, dans les deux sens | **Complet** |
| `client-core` | Client Archipelago (ws/wss) | **Complet** pour J0 |
| `multidata` | Lecture sûre du `.archipelago`, modèle de partie, format `.raftup` | **Complet** pour J2 |
| `server` | Serveur WebSocket + binaire `raftup-server` | **Complet** pour J2, sauf T4 |
| `launcher-core` | Héberger une partie, résumé de seed, journal | Commencé (J3) |
| `launcher-ui` | Application egui, binaire `raftup` | Commencé (J3) |
| `py-bridge` | Générateur officiel en Python | Pas commencé |

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
- **Indices** (ADR 0012) : `create_as_hint`, `CreateHints`, `UpdateHint`, points et coût des indices
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
- Écrans « Héberger une partie », « Partie en cours » et « Bientôt », d'après la maquette
- Polices Chakra Petch et Inter intégrées (OFL)
- `cargo run -p launcher-ui` démarre sans erreur ; **rendu, boîtes de fichiers et presse-papiers jamais contrôlés à l'œil**
- Ne sauvegarde pas encore la partie qu'il héberge

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

## Reporté à J3

- Commandes serveur après `!admin login` (`/players`, `/send`…)
- Sauvegarde de la partie hébergée par le launcher
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
- Cas limites de `UpdateHint` / `CreateHints` (champ `status` absent ou `null`, statut en texte ou en flottant, `player` à `null`) : comportement différent, relevé par la relecture, pas encore tranché

**Commandes et indices** (ADR 0012, 0013)

- `!options` masque le mot de passe de la salle (le serveur officiel l'affiche)
- `!license` affiche la licence de Raftup
- Erreurs d'arguments : texte court à la place d'une trace Python
- `!hint` : tirage déterministe entre ex æquo, au lieu du Mersenne Twister
- `!hint` : noms candidats dans l'ordre du datapackage (un ensemble, donc au hasard, en Python)
- `!hint` : liste des objets non indiçables vide en attendant T4
- `!missing`, `!checked` : ordre des lieux croissant (ordre du multidata en Python), à confirmer par capture
- `!missing`, `!checked`, `!remaining` : « Invalid arguments » sur un id sans nom, au lieu de « Unknown item (ID:x) »
- `!countdown` au-delà d'une heure : message court au lieu d'une trace
- `!release` : pas d'autorisation par joueur depuis la console (`/allow_release`)
- `!admin` : après connexion, les commandes serveur répondent « Server commands are not available yet. » (J3)

</details>
