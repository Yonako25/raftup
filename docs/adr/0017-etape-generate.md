# 0017 — Étape « generate » : générateur officiel dans un Python embarqué

- Statut : accepté
- Date : 2026-11-17

## Contexte

- Le MVP génère avec le générateur officiel d'Archipelago, version épinglée, dans le worker Python de `py-bridge` (ADR 0016).
- Il faut donc un Python, le code d'Archipelago et ses dépendances sur le PC du joueur, sans rien redistribuer.

## Décision

| # | Sujet | Décision |
| --- | --- | --- |
| 1 | Python | python-build-standalone `install_only` (`.tar.gz`), téléchargé au premier usage, hash épinglé |
| 2 | Code d'Archipelago | archive GitHub du commit épinglé (0.6.8, voir `THIRD_PARTY_NOTICES.md`), hash épinglé |
| 3 | Dépendances | wheels épinglées, téléchargées par Rust et décompressées directement dans `site-packages` ; ni pip ni réseau côté Python |
| 4 | Où vit le code | primitives et types JSON dans `py-bridge` ; orchestration dans `launcher-core::generation` ; pas de crate `generate` (réservée à la V2) |
| 5 | Process | un worker neuf par génération ; un seul `generate` par process |
| 6 | Progression | lignes de stderr transmises en direct, annulation par jeton ; pas de protocole 2 |
| 7 | Descendance | Job Object Windows `KILL_ON_JOB_CLOSE` via la crate `win32job` 2.1.0 (MIT OR Apache-2.0, confirmé) ; `tar` 0.4.46 est aussi MIT OR Apache-2.0 |
| 8 | Apworlds | copiées dans `<archipelago>/custom_worlds`, vidé avant chaque génération |
| 9 | Valeurs par défaut | une génération à la fois (verrou partagé avec l'installation du runtime, valable pour un seul process), délai de 15 min (`DEFAULT_TIMEOUT`), spoiler 3 (`DEFAULT_SPOILER`) ; `GenerateRequest::default()` les applique ; le dossier `generations/<horodatage>/` est supprimé après un échec ou une annulation, gardé après un succès |
| 10 | Consentement | la taille du téléchargement est montrée avant le premier usage |
| 11 | `compileall` | pas au départ ; à mesurer |
| 12 | Durcissement réseau | `RAFTUP_BLOCK_NETWORK=1` (variable lue par le worker, héritée du process parent) refuse toute connexion sortante et toute résolution de nom avant l'import d'Archipelago ; boucle locale permise (asyncio en a besoin sous Windows). **Implémentée, désactivée par défaut** ; le test réel passe avec et sans. Ce n'est pas un bac à sable (ctypes ou un sous-process la contournent) |

Fichier d'épinglage : `crates/launcher-core/generator-pins.toml` (URL HTTPS, hôtes autorisés, sha256 réels relevés sur les fichiers téléchargés, jamais inventés).

Contrat JSON de `generate` : paramètres `archipelago_dir`, `site_packages`, `player_files_dir`, `output_dir`, `seed`, `spoiler`, `race` ; résultat `zip`, `seed_name`, `ap_version`, `players`, `seconds`. Erreurs : -32010 génération échouée (avec traceback), -32011 import impossible, -32012 un seul `generate` par process, -32602 paramètres invalides.

## Conséquences

- Premier usage : environ 72 Mo à télécharger (somme des `size` épinglées), puis tout est local.
- Archipelago écrit `host.yaml`, `logs/` et `output/` dans son dossier : une génération à la fois.
- Une montée de version d'Archipelago passe par un seul fichier d'épinglage et le test réel `#[ignore]`.
- Licences des composants téléchargés (PSF-2.0, MPL-2.0, MIT, BSD…) : section dédiée de `THIRD_PARTY_NOTICES.md`. Rien n'est redistribué.
- Sans Job Object (hors Windows), les petits-enfants du worker peuvent survivre : documenté, à traiter en V2.
- Un apworld est du code Python arbitraire exécuté sans bac à sable : à dire clairement au joueur dans l'interface avant d'en ajouter un. `generate` ne vide et ne remplit `custom_worlds` que si le dossier d'Archipelago est sous les données de Raftup (`UnmanagedArchipelago` sinon).
- Les redirections de téléchargement ne sont pas contrôlées par hôte : l'intégrité repose sur le sha256 épinglé (GitHub utilise `codeload.github.com` et `release-assets.githubusercontent.com`).
- Le démarrage du pont (handshake, 30 s au plus) n'est pas annulable.
- La fenêtre entre le lancement du worker et son placement dans le Job Object est sans effet : le worker ne lance rien avant la première requête.
- Hypothèses sur l'Archipelago réel : vérifiées, voir la section « Vérification » en bas (reste : mondes qui demandent une ROM, non testés).
- Le chemin du zip annoncé par le worker n'est jamais utilisé : Rust reconstruit `output/AP_<seed_name>.zip` (nom de seed `[A-Za-z0-9_-]+`, fichier ordinaire, pas un lien).

## Vérification sur le vrai Archipelago 0.6.8 (test réel, 17 novembre 2026)

Test : `crates/launcher-core/tests/generation_real.rs` (`#[ignore]`, variable `RAFTUP_REAL_RUNTIME` = racine d'un runtime installé par `setup_generator` ; il ne télécharge jamais). Lancement : `cargo test -p launcher-core --test generation_real -- --ignored --nocapture --test-threads=1` (une génération à la fois).

Installation réelle (`setup_generator`, Windows 11) : environ 21 s, aucune erreur, aucun nom interdit ni chemin trop long. Disque : 291 Mo pour 7254 fichiers (Python 152 Mo, cache de téléchargement 69 Mo conservé, le reste = Archipelago et wheels). Entrées réelles : Python 3461 (max 6000), Archipelago 4050 avec dossiers (max 8000) : les pins n'ont pas changé.

Hypothèses confirmées :
- `Generate.main()` sans argument lit `sys.argv` (argparse) ; options `--player_files_path --outputpath --seed --spoiler --race` exactes ; elle rend `(args, seed)`.
- `Main.main(args, seed)` rend le `MultiWorld` ; `seed_name`, `player_ids`, `player_name`, `game` existent. Le nom de seed est dérivé de la seed numérique (20 chiffres pour `--seed 123456`) ; même seed, même `seed_name`.
- Le zip est `AP_<seed_name>.zip` dans `--outputpath` ; `Utils.__version__` vaut `0.6.8`.
- `ModuleUpdate.update()` avec `SKIP_REQUIREMENTS_UPDATE=1` : ni pip, ni `input()`, ni réseau.
- stdin remplacé par devnull : rien ne bloque ; aucune boîte de dialogue.
- Un YAML invalide (jeu inconnu) donne `GeneratorFailed { kind: "ValueError", traceback }` ; `generations/` est nettoyé. L'annulation à 0,7 s rend `Cancelled` en environ 0,8 s, sans python.exe restant.

Hypothèse corrigée : le jeu « Clique » n'existe pas en 0.6.8 ; le test utilise `APQuest` (sans ROM).

Durées : génération APQuest 6 s la première fois (compilation des `.pyc`, import de 75 mondes), environ 2 s ensuite ; la génération elle-même prend moins de 0,1 s. Une compilation préalable (décision 11) gagnerait environ 4 s au premier usage : pas nécessaire.

À résoudre : 7 mondes ne s'importent pas (erreur journalisée, sans effet sur les autres) car leurs dépendances ne sont pas épinglées : `requests` (ffmq, messenger, osrs, sm), `pkg_resources` / setuptools (pokemon_emerald), `pyevermizer` (soe), `zilliandomizer` (zillion). Les ajouter demande de nouveaux pins avec sha256.

Effets de bord observés : seul `host.yaml` (15 Ko) est créé dans le dossier d'Archipelago (le runtime) ; ni `logs/`, ni `output/`, ni `Players/` (l'init des logs n'a lieu que sous `__main__`). Des `__pycache__` apparaissent dans Python, Archipelago et `site-packages` (dans le runtime). `%LOCALAPPDATA%\Archipelago` est inchangé. Aucun dossier temporaire ni process restant. Aucune tentative réseau relevée : le test passe avec `RAFTUP_BLOCK_NETWORK=1` sans message de refus.

Avertissements au démarrage (stderr, 345 lignes de journal) : `_speedups not available` (UserWarning, sans effet) et les tracebacks des 7 mondes ci-dessus.
