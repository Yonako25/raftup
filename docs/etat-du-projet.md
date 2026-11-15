# État du projet

> [!IMPORTANT]
> - **J0 atteint** : client connecté à une vraie salle archipelago.gg
> - **J1 : étapes 1 à 6 faites** sur 9 — protocole de jeu complet, limites des connexions en place
> - **101 tests** passent, dont **6 scénarios** identiques au serveur officiel 0.6.8

Mis à jour le **15 novembre 2026**.

## À faire (J1)

- [ ] Étape 7 : tests différentiels contre le serveur Python (NF02)
- [ ] Étape 8 : chiffrement wss / TLS (NF06)
- [ ] Étape 9 : partie réelle de Slime Rancher sur le serveur Raftup
- [ ] Demander à l'auteur de Slimipelago l'autorisation d'installation automatique

## Crates

| Crate | Rôle | État |
| --- | --- | --- |
| `ap-protocol` | Types des paquets, dans les deux sens | **Complet** |
| `client-core` | Client Archipelago (ws/wss) | **Complet** pour J0 |
| `multidata` | Lecture sûre du `.archipelago`, modèle de partie | **Complet** pour J1 |
| `server` | Serveur WebSocket + binaire `raftup-server` | **Jeu complet**, limites en place |
| `py-bridge`, `launcher-core`, `launcher-ui` | Générateur, launcher | Pas commencé |

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

**server**

- Une tâche « salle » possède l'état ; une tâche par connexion (ADR 0006)
- Connexion : `RoomInfo`, `Connect` et ses refus, `Connected`, inventaire de départ, `GetDataPackage`
- Jeu : `LocationChecks`, objets, groupes, `Sync`, `ConnectUpdate`
- Fin : `StatusUpdate`, objectif, `collect` / `release` automatiques
- Échanges : `LocationScouts`, chat `Say`, `Bounce` (DeathLink)
- Stockage : `Get`, `Set` (18 opérations), `SetNotify`, clés `_read_…`
- Limites (ADR 0007) : message 1 Mio, chat 2 000 caractères, 50 paquets/s, 5 chats / 10 s, ping 20 s, muet 40 s, files bornées
- Binaire : `raftup-server <seed> --no-tls` (wss exigé par défaut, pas encore codé)

## Décisions (ADR)

- **0001** : catalogue public dans le dépôt
- **0002** : hub de comptes et d'amis
- **0003** : hub en AGPL
- **0004** : protocole, serveur et client écrits par Raftup
- **0005** : WebSocket avec tokio-tungstenite + rustls ; pas de compression
- **0006** : serveur organisé autour d'une tâche « salle »
- **0007** : limites des connexions et files bornées

## Reporté à J2

- Commandes `!` du chat (`!help`, `!hint`, `!release`…)
- Création d'indices (`create_as_hint`, `CreateHints`, `UpdateHint`)
- Coût des indices dans les points d'indice
- Sauvegarde et reprise (NF07), format `.raftup`

## Vérifier

```bash
cargo test --workspace
```

```bash
cargo run -p server --bin raftup-server -- crates/multidata/tests/data/seed-mit.archipelago --no-tls
```

```bash
cargo run -p client-core --example suivi -- 127.0.0.1:38281 Bob
```

<details>
<summary>Écarts connus avec le serveur officiel</summary>

- Entiers plus grands qu'un `i64` non gérés dans le stockage
- `mul` refuse un résultat de plus d'un million d'éléments
- Quelques paquets mal formés : réponse un peu différente (rare)
- Plantages du serveur officiel reproduits par une fermeture de connexion : changement de slot, connexion d'un groupe, lieu inconnu dans `LocationScouts`
