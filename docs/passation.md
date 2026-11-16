# Passation — reprise du travail (16 novembre 2026, fin de journée)

Pour le prochain Claude : lis d'abord `CLAUDE.md`, `docs/SPEC.md`, `docs/etat-du-projet.md` et les ADR `docs/adr/` (surtout 0010 à 0014).
L'utilisatrice est apprentie, débutante en Rust : réponses courtes, en français, une étape à la fois. Elle valide chaque étape, puis demande le commit (« Validez les modifications… ») ; ne committe pas sans qu'elle le demande. Elle a aussi demandé de travailler avec les sous-agents du plugin `equipe` pour économiser des tokens.

## État du dépôt

- Branche `main`, poussée sur `https://github.com/Yonako25/raftup` ; dernier commit `740ae67`.
- **CI GitHub (Windows) verte** sur `740ae67`.
- `cargo test --workspace` : **202 tests** au vert ; clippy (`-D warnings`) et `fmt --check` propres.

## Ce qui a été fait aujourd'hui

- **Relecture** de `1444f29..da59b3b`. Corrigé : fuite du mot de passe `!admin` dans le chat (casse, espaces, guillemets) ; lecture `.raftup` coupée au milieu d'un caractère UTF-8 ; `SaveFile::create` atomique ; synchronisation du dossier sous Unix ; `format_version` 0 refusé. Les autres constats sont listés dans `docs/etat-du-projet.md` (écarts connus).
- **T7b** : `!release`, `!collect`, `!alias` (nom « Surnom (Nom) » partout où Python l'utilise), `!getitem`, `!countdown` (minuteur dans la boucle de la salle avec `select!`), `!admin login` / `logout` (un seul administrateur ; commandes serveur reportées à J3).
- **T8** : `!hint` et `!hint_location` (`commands/hint.rs`), tirage déterministe FNV-1a + SplitMix64 (ADR 0012).
- **T9** : sauvegarde branchée.
  - `raftup::diff` entre deux instantanés ; `GameState::fill_snapshot` / `from_snapshot`.
  - La salle compare l'état avant / après chaque événement, écrit les deltas + fsync, **puis** envoie (file `pending` par client, `RoomTask::commit`).
  - Salle sur un thread système dédié avec son runtime ; arrêt par `Event::Stop`.
  - `raftup-server --savefile X | --disable_save` ; par défaut `seed.raftup` à côté de la seed ; reprise si le fichier existe ; compactage au démarrage, à 10 000 deltas et à l'arrêt ; Ctrl+C appelle `stop()`.
  - Tests : `tests/sauvegarde.rs` (écrit avant la réponse, reprise) et `tests/arret_brutal.rs` (binaire tué puis relancé).
- `docs/etat-du-projet.md` mis à jour.

## Ce qui reste à faire

1. ~~T4~~ : fait. Liste fixe dans `server/src/hints.rs` (`is_non_hintable`) : seuls deux mondes de base de 0.6.8 en ont une.
2. **Captures et différentiel** des scénarios `COMMANDES` et `INDICES` (`crates/server/tests/data/scripts/scenarios.py`). Il faut un Python avec les dépendances d'Archipelago : **demander à l'utilisatrice avant tout téléchargement**. Source officielle 0.6.8 dans `target/reference/` (ignorée par git). Le pilote lance maintenant Raftup avec `--disable_save`.
3. **Trancher les écarts relevés par la relecture** : textes « Unknown item (ID:x) » de `!missing` / `!checked` / `!remaining`, ordre des lieux, cas limites de `UpdateHint` / `CreateHints` (voir `etat-du-projet.md`).
4. **Relire** `room_task/connect.rs`, `messages.rs`, `monitor.rs`, `launcher-core` et `launcher-ui` (pas couverts par la relecture).
5. **Contrôle visuel du launcher** avec l'utilisatrice, et vrai gras (Inter SemiBold en famille séparée). Le launcher ne sauvegarde pas encore la partie hébergée (`Server::with_save` à brancher dans `launcher-core::host`).
6. **J1, étape 9** : une vraie partie de Slime Rancher sur `raftup-server`. C'est l'utilisatrice qui la joue ; elle veut tout tester à la fin.
7. **Suite de J3** : `py-bridge`, catalogue et profils, installation des mods, détection des jeux, rejoindre par lien, salon, webhook Discord, commandes serveur après `!admin login`.

## Points d'attention

- **Seed de test** `seed-mit.archipelago` : Alice (slot 1) a déjà **11 indices de départ** sur « Map Width » (80000). Pour tester `!hint`, prendre « Map Height » (80001).
- **Limite du chat** : 5 messages / 10 s par connexion. Un test qui envoie plus de commandes doit ouvrir une nouvelle connexion ou une nouvelle salle.
- **Après `LocationChecks`**, le serveur envoie plusieurs trames : pour savoir qu'un envoi est acquitté, attendre le `RoomUpdate` qui annonce le lieu, pas « la prochaine trame ».
- **Outils** : un script Python passé en ligne (`python - <<EOF`) peut perdre ses `\n` échappés ; pour un remplacement long, écrire le script dans le dossier scratchpad puis le lancer.
- Les sous-agents en copie isolée (`isolation: worktree`) partent de la version **distante** de `main` : pousser avant de les lancer.
- Des copies de travail d'agents restent dans `.claude/worktrees/` (6, ignorées par git) avec leurs branches `worktree-agent-*`. Les supprimer seulement avec l'accord de l'utilisatrice.
- Un dossier vide `crates/launcher-ui/ui` reste là, verrouillé par Windows ; git l'ignore.
