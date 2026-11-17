# Raftup

Jumeau d'Archipelago (randomizer multiworld multi-jeux) écrit en Rust, avec un launcher de bureau façon CurseForge / GOG Galaxy.
Le cahier des charges complet est dans `docs/SPEC.md` : lis-le avant toute tâche qui touche à l'architecture, au protocole ou au périmètre.

## Règles non négociables

- **Compatibilité stricte avec le protocole réseau d'Archipelago.** Un mod de jeu Archipelago existant doit se connecter au serveur Raftup sans aucune modification. Référence : `docs/network protocol.md` du dépôt ArchipelagoMW/Archipelago.
- **Jamais de jeu, de ROM, d'asset ou de clé distribués.** Raftup installe des mods et applique des patchs sur la copie du joueur, après vérification du hash.
- **Pas de jeux de consoles Nintendo actuelles** dans le catalogue.
- **Licences** : Raftup est en double licence MIT / Apache-2.0, sauf le hub en AGPL-3.0-or-later (ADR 0003) ; une crate MIT/Apache ne dépend jamais du hub. Tout code porté depuis un autre projet (Archipelago, pahoa…) garde sa notice de copyright dans `THIRD_PARTY_NOTICES.md`.
- Ne jamais écrire « Archipelago » ou « AP » dans le nom ou le logo du produit ; écrire « compatible avec Archipelago ».

## Stack

- Rust (workspace Cargo), édition 2021, toolchain stable.
- Async : `tokio`. Sérialisation : `serde` / `serde_json`. Erreurs : `thiserror` dans les bibliothèques, `anyhow` dans les binaires. Logs : `tracing`.
- Launcher : egui / eframe (rendu wgpu), interface 100 % Rust, sans webview (ADR 0011).
- Protocole, serveur et client : écrits par Raftup (ADR 0004), sur `tokio` et `ap-protocol`. Pas de dépendance à pahoa ni à `archipelago_rs` : ce sont des références, avec le serveur Python officiel comme oracle des tests différentiels.
- Génération au MVP : générateur Archipelago officiel, version épinglée, lancé dans un process Python séparé (python-build-standalone), dialogue JSON-RPC sur stdin/stdout (un message JSON par ligne), sans port réseau ouvert.

## Structure du dépôt

```
crates/
  ap-protocol/     types des paquets du protocole Archipelago, dans les deux sens (seule source de vérité)
  multidata/       modèle de partie, lecture sécurisée du .archipelago, format natif .raftup
  server/          serveur WebSocket maison (tokio + ap-protocol)
  client-core/     client Archipelago maison, utilisé par le launcher et les tests
  py-bridge/       pilote le process Python du générateur officiel
  launcher-core/   catalogue, profils d'installation, installation des mods, lancement des jeux
  launcher-ui/     application egui (ne dépend que de launcher-core)
  logic/  fill/  worlds-native/  generate/   → V2 et V3, ne pas créer avant
  hub/             service raftup-hub : comptes, amis, présence (AGPL, après le MVP, ne pas créer avant)
profiles/          profils d'installation déclaratifs, un fichier TOML par jeu
docs/              SPEC.md, décisions d'architecture (docs/adr/)
```

## Commandes

```
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo run -p launcher-ui   # lance le launcher
```

## Conventions de code

- Identifiants et code en anglais ; documentation, commentaires de module et messages de commit en français.
- Pas de `unwrap()` ni `expect()` dans le code de bibliothèque ; erreurs typées.
- Pas de `unsafe`.
- Graphes et structures liées : arènes avec index `u32` plutôt que des références avec durées de vie complexes.
- Ordre déterministe partout où un résultat doit être reproductible : `BTreeMap` / `IndexMap`, jamais d'itération sur une `HashMap` pour produire une sortie.
- Chaque crate a des tests ; un bug corrigé arrive avec un test qui le reproduit.
- Le format natif `.raftup` et le format des profils sont versionnés (champ `format_version`).

## Comment travailler avec moi

- Je suis apprentie développeuse, à l'aise en Java, **débutante en Rust**. Garde le code simple et lisible plutôt que malin.
- Quand tu utilises une notion Rust non triviale (ownership, durées de vie, traits, async), ajoute un court commentaire qui explique pourquoi.
- Avance par petites étapes testables ; à la fin de chaque tâche, dis ce qui marche, ce qui reste et comment le vérifier.
- Avant une décision d'architecture qui n'est pas dans `docs/SPEC.md`, propose-la et attends ma validation ; une fois validée, consigne-la dans `docs/adr/`.
- Ne lance pas de commande qui supprime des fichiers en dehors de `target/` sans me demander.

## Travail avec des agents

Les règles communes, les vérifications et le découpage d'une étape sont dans `docs/regles-agents.md` : un prompt d'agent y renvoie au lieu de les recopier. L'état à jour du projet est dans `docs/etat-du-projet.md`.

## Suivi entre les sessions

- **Au démarrage** : lire `docs/etat-du-projet.md` et les **5 dernières entrées** de `docs/journal.md` (pas les anciennes, pas les transcripts d'autres sessions). Respecter la ligne « Ne pas toucher » des entrées et la section « En cours ».
- **À la fin d'une tâche ou d'une étape** : lancer la compétence `/cloture` (`.claude/skills/cloture/`) : entrée de 5 lignes dans le journal, état du projet mis à jour si besoin, proposition de commit.
- Une tâche = une session courte : après une étape commitée, repartir d'une session neuve plutôt que de prolonger celle-ci.
- Un hook de fin de tour (`.claude/settings.json`) rappelle `/cloture` quand le dernier commit ne touche pas le journal.

## Design de l'interface

Voir `docs/SPEC.md` § « Identité visuelle » : palette acier + bleu électrique, angles de 2 px maximum, fonds d'ambiance dégradés avec panneaux semi-transparents, police Chakra Petch pour les titres et Inter pour l'interface.
