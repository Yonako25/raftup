# Plan de conception : logique du launcher (`launcher-core`)

> [!NOTE]
> **Brouillon à valider.** Ce n'est pas encore un ADR : une fois les décisions de la fin validées, on écrit `docs/adr/0015-profils-installation-lancement.md`.
> Rédigé le 17 novembre 2026 à partir de deux études (architecte et détection des jeux).

## Objectif

Faire de `launcher-core` le « cerveau » du launcher, avant de brancher les écrans :

- **catalogue et profils** d'installation (F02, F07) ;
- **installation et désinstallation** réversibles, avec hash (F02, F11) ;
- **lockfile de session** (F03) ;
- **détection des jeux** (F01) ;
- **lancement et connexion** (F04, F05).

Écrans qui en dépendent ensuite : Bibliothèque, Catalogue, Téléchargements, Mises à jour, bouton « Lancer ».

## Principes

1. Le **profil TOML** embarqué dans le binaire est la seule source de vérité. Il est validé au chargement ; la CI refuse un profil faux.
2. Une installation écrit un **reçu** (un fichier TOML par jeu) avant de toucher au dossier du jeu. C'est ce qui rend la désinstallation sûre, même après un plantage.
3. Un téléchargement passe par un **cache rangé par sha256**, vérifié avant toute extraction. Une seule fonction extrait les zips, et elle contrôle chaque chemin (zip-slip).
4. Le lancement se fait en deux temps : `plan_launch` (fonction pure, facile à tester), puis `launch` (sans shell).
5. Le code d'installation est **synchrone** (plus simple en Rust débutant). L'interface l'appelle depuis un thread à part. Seul le diagnostic de connexion (F05) est asynchrone.

## Modules prévus dans `crates/launcher-core/src`

| Fichier | Rôle |
| --- | --- |
| `paths.rs` | Les dossiers de Raftup (config, données, cache), testables dans un dossier temporaire |
| `template.rs` | Variables `{game_dir}`, `{address}`… avec refus des variables inconnues |
| `profile.rs` | Le profil TOML : types, validation, `format_version` |
| `catalog.rs` | Tous les profils embarqués, liste blanche des domaines |
| `detect.rs` | Détection des jeux (Steam, Epic, dossier manuel) |
| `download.rs` | Téléchargement HTTPS, cache par sha256, taille maximale |
| `archive.rs` | Extraction zip sécurisée (zip-slip, bombe zip) |
| `record.rs` | Le reçu d'installation, écrit de façon atomique |
| `install.rs` / `uninstall.rs` | Installer, mettre à jour, désinstaller |
| `lockfile.rs` | Versions figées par l'hôte, sans aucune URL ni chemin |
| `config.rs` | Configuration utilisateur qui survit aux mises à jour |
| `launch.rs` | Plan de lancement et lancement du jeu |
| `diagnose.rs` | Explique les échecs de connexion en français (F05) |

## Format du profil (extrait)

```toml
format_version = 1

[game]
id = "slime-rancher"
name = "Slime Rancher"
ap_game = "Slime Rancher"
status = "available"

[game.detect]            # lu par la détection
steam_appid = 433340
expected_exe = "SlimeRancher.exe"

[[steps]]                # appliquées dans l'ordre, désinstallées à l'envers
id = "melonloader"
kind = "mod_loader"      # mod_loader | mod | apworld (jamais rom ni asset)
version = "0.7.3"
source = { url = "https://github.com/…" }
sha256 = "…"             # toujours obligatoire
install = { extract = "zip", into = "{game_dir}" }

[preserve]               # jamais écrasés par une mise à jour (F07)
paths = ["{game_dir}/UserData/Slimipelago.cfg"]

[launch]
exe = "{game_dir}/SlimeRancher.exe"
connect = { method = "manual", instructions = "…" }
```

Méthodes de connexion (F04) : `cli_args`, `config_file`, `env`, `uri`, `manual`.

## Détection des jeux (F01)

- **Steam** : lire `libraryfolders.vdf`, puis `appmanifest_<appid>.acf` dans **chaque** bibliothèque.
- **Epic** : lire les `.item` (JSON) dans `ProgramData\Epic\EpicGamesLauncher\Data\Manifests`, avec un parseur tolérant (pas de documentation officielle).
- **Manuel** : vérifier la présence de l'exécutable attendu ; un hash n'est qu'un avertissement.
- **Crates** : `keyvalues-parser` (MIT ou Apache-2.0) pour le format Steam, `winreg` pour le registre derrière `cfg(windows)`. Pas de `steamlocate`.
- **API testable** : `detect_games(&DetectionRoots, &Catalog)` reçoit ses dossiers en paramètre, donc les tests utilisent de faux dossiers.
- **À vérifier avant adoption** : les valeurs de `StateFlags`, les clés du registre, l'absence d'`unsafe` dans `keyvalues-parser`, les identifiants Epic des cinq jeux.

## Tâches

| Tâche | Contenu | Dépend de |
| --- | --- | --- |
| TA | Valider les décisions ci-dessous, écrire l'ADR 0015 | rien |
| T0 | Squelette : dépendances, modules vides, erreurs | TA |
| T1 | `template.rs`, `paths.rs` | T0 |
| T3 | `archive.rs` | T0 |
| T4 | `download.rs` | T0 |
| T5 | `record.rs` | T0 |
| T10 | `config.rs` | T0 |
| T12 | `diagnose.rs` | T0 |
| T13 | `detect.rs` (types, dossier manuel, puis Steam et Epic) | T0 |
| T2 | `profile.rs`, `catalog.rs` | T1 |
| T7 | `uninstall.rs` | T5 |
| T6 | `install.rs` | T1 à T5 |
| T9 | `lockfile.rs` | T2, T5 |
| T11 | `launch.rs` | T1, T2 |
| T8 | Aller-retour installer / désinstaller, mise à jour | T6, T7 |
| T14 | Profil réel de Slime Rancher | T2 et l'autorisation de l'auteur de Slimipelago |

**Ordre de travail** (chaque vague = tâches sans fichier en commun) :

1. TA, puis T0.
2. Vague 1 : T1, T3, T4, T5, T10, T12, T13.
3. Vague 2 : T2, T7.
4. Vague 3 : T6, T9, T11.
5. Vague 4 : T8, puis T14.

Chaque tâche arrive avec ses tests. Les archives zip de test sont fabriquées dans le code des tests, jamais commitées.

## Décisions à valider

| # | Question | Recommandation |
| --- | --- | --- |
| 1 | Client HTTP | `ureq` (bloquant, simple), moteur TLS `ring` |
| 2 | Dossiers utilisateur | crate `directories` |
| 3 | Embarquer les profils | liste `include_str!` écrite à la main, avec un test qui la compare au dossier `profiles/` |
| 4 | Sources de téléchargement | `url` et `thunderstore`, avec liste blanche (github.com, thunderstore.io…) |
| 5 | Où voyage le lockfile | fichier à côté de la seed au MVP, jamais d'URL dedans |
| 6 | Versions par étape | une seule version par étape et par profil |
| 7 | Fichiers créés par le jeu en tournant | champ `cleanup` dans le profil, suppression après confirmation |
| 8 | Fichier modifié depuis l'installation | rapport de conflits, suppression seulement avec `force` |
| 9 | Formats de `config_file` | JSON seulement ; `config_file` et `uri` codés au premier jeu qui en a besoin |
| 10 | Mot de passe dans la config du mod | l'accepter et le documenter ; préférer `env`, puis `config_file`, puis `cli_args` |
| 11 | Jeux Steam | lancer l'exécutable directement |
| 12 | Plusieurs copies d'un même jeu | une seule installation par jeu |
| 13 | Lien d'invitation (hors de ce plan) | à traiter avec le salon (F08) |

## Risques à ne pas oublier

- **Zip-slip et bombe zip** : un seul point d'extraction, refus des `..`, chemins absolus, flux NTFS, liens ; on compte les octets réellement écrits.
- **Hash** : vérifié avant d'ouvrir l'archive, téléchargement dans un `.part`.
- **Profil malveillant** : le hash ne protège pas contre ça ; la relecture des pull requests et la liste blanche sont indispensables.
- **Mot de passe** : masqué dans les logs, mais visible dans la liste des processus avec `cli_args`.
- **Droits** : jamais d'élévation automatique ; tester l'écriture dans le dossier du jeu avant d'installer.
- **Licences** : Slimipelago n'a pas de licence, donc autorisation écrite avant T14 ; aucun mod n'est ré-hébergé ; tout code porté va dans `THIRD_PARTY_NOTICES.md`.
- **Jamais de jeu, ROM ou asset distribué** : pas de type d'étape `rom` ni `asset`.

---

# Annexe : règles communes, types et tests par tâche

## Règles communes à toutes les tâches

- Chaque agent est propriétaire de **ses fichiers seulement** : son module dans `crates/launcher-core/src/`, ses tests dans `crates/launcher-core/tests/<module>.rs` et ses données dans `crates/launcher-core/tests/data/<module>/`. Il ne touche ni à `lib.rs`, ni à `error.rs`, ni à un `Cargo.toml` (s'il manque une dépendance, il le signale dans son rapport).
- Le fichier de module posé par T0 contient un commentaire `//!` et une enum d'erreur : **garde le nom de l'enum d'erreur** (`error.rs` la référence avec `#[from]`) et étends-la librement.
- Le type `Sha256` (32 octets, hexadécimal, serde) existe déjà : `crate::hash::Sha256` (`of_bytes`, `of_reader`, `from_hex`, `Display`).
- Pas de `unwrap()`/`expect()` dans le code de bibliothèque (autorisés dans les tests), pas d'`unsafe`. Erreurs typées avec `thiserror`.
- Commentaires et documentation en français ; identifiants en anglais ; un court commentaire pédagogique pour chaque notion Rust non triviale (propriétaire est débutante en Rust).
- Ordre déterministe : `BTreeMap`, jamais d'itération sur une `HashMap` pour produire une sortie.
- Les builds partagent le même `target` : si la compilation échoue à cause d'un fichier qui n'est pas le tien, attends une minute et recommence. Écris ton fichier d'un seul bloc complet plutôt qu'en petites modifications successives.
- Utilise `CARGO_TARGET_DIR=target/dev-agents`.
- Avant de rendre la main : `cargo fmt --all`, `cargo clippy -p launcher-core --all-targets -- -D warnings`, `cargo test -p launcher-core --test <ton_module>` et `cargo test -p launcher-core --lib`. Ne commit rien. N'efface aucun fichier hors `target/`.
- Les archives zip de test sont fabriquées dans le code des tests (crate `zip`), jamais commitées.

## Types et signatures

**`paths.rs` (T1)**
- `Paths { config_dir, data_dir, cache_dir: PathBuf }`
- `Paths::from_os() -> Result<Paths, PathsError>` : avec `directories::BaseDirs`, `config_dir` = `%APPDATA%\Raftup`, `data_dir` = `%LOCALAPPDATA%\Raftup`, `cache_dir` = `data_dir/cache` ; `Paths::under(root: &Path) -> Paths` pour les tests.
- Accesseurs : `config_file()`, `installs_dir()`, `backups_dir(game_id)`, `downloads_dir()`, `worlds_dir(ap_version)`.

**`template.rs` (T1)**
- `Vars(BTreeMap<String, String>)` ; `expand(template, &Vars) -> Result<String, TemplateError>` ; `check(template, allowed: &[&str]) -> Result<(), TemplateError>`.
- `{{` donne une accolade ; variable inconnue refusée ; accolade non fermée refusée ; une valeur contenant un saut de ligne ou un NUL est refusée.
- `TemplateError { UnknownVariable { name }, Unclosed, ForbiddenChar { name } }`

**`archive.rs` (T3)**
- `ArchiveLimits { max_entries: usize (20_000), max_total_bytes: u64 (2 Gio) }` avec `Default`.
- `safe_relative_path(entry: &str) -> Result<PathBuf, ArchiveError>`
- `plan_zip(file: &Path, strip_prefix: &str, limits: &ArchiveLimits) -> Result<Vec<PathBuf>, ArchiveError>` : liste ce qui serait écrit, sans rien extraire.
- `extract_zip(file, strip_prefix, root: &Path, limits, on_file: &mut dyn FnMut(&Path, Sha256)) -> Result<(), ArchiveError>`
- `ArchiveError { UnsafePath(String), Link(String), TooManyEntries, TooLarge, Zip(String), Io { path, source } }`

**`download.rs` (T4)**
- `trait Fetcher: Send + Sync { fn open(&self, url: &str) -> Result<Box<dyn Read + Send>, DownloadError>; }`
- `HttpFetcher` (ureq, HTTPS seulement, redirections bornées, délais) ; `DirFetcher` (tests : une URL correspond à un fichier local).
- `Downloader { dir: PathBuf, fetcher: Box<dyn Fetcher> }` ; `get(&self, url: &str, sha256: &Sha256, max_bytes: u64, progress: &mut dyn FnMut(u64)) -> Result<PathBuf, DownloadError>`.
- `DownloadError { NotHttps(String), Http { url, status }, Network { url, reason }, TooLarge { url, max }, HashMismatch { url, expected, actual }, Io { path, source } }`

**`record.rs` (T5)**
- `InstallRecord { format_version, game_id, game_dir, state: RecordState, steps: Vec<StepRecord> }` ; `RecordState { InProgress, Complete }`.
- `StepRecord { id, version, sha256, files: Vec<FileRecord>, dirs: Vec<PathBuf> }` ; `FileRecord { path, sha256: Option<Sha256>, backup: Option<Backup> }` ; `Backup { path, original_sha256 }`.
- `load(paths, game_id) -> Result<Option<InstallRecord>, RecordError>`, `save(&self, paths)` (atomique : `.tmp` puis renommage), `remove(paths, game_id)`.
- `RecordError { Io, Toml, UnsupportedVersion }`

**`config.rs` (T10)**
- `UserConfig { format_version, player_name: Option<String>, games: BTreeMap<String, GameSettings { game_dir: Option<PathBuf> }>, last_port: Option<u16> }`.
- `load(path) -> Result<LoadedConfig { config, set_aside: Option<PathBuf> }, ConfigError>`, `save(&self, path)` (atomique).

**`diagnose.rs` (T12)**
- `ConnectionProblem { BadAddress, Unreachable, PortClosed, Tls, Timeout, InvalidSlot, InvalidGame, IncompatibleVersion, InvalidPassword, InvalidItemsHandling, Other(String) }`.
- `explain(e: &client_core::ClientError) -> ConnectionProblem` (fonction pure) ; `message(&self) -> String` (français) ; `async check_connection(address, slot, password, game) -> Result<(), ConnectionProblem>`.

**`detect.rs` (T13)**
- Découplé du profil : `DetectRule { game_id, steam_appid: Option<u32>, epic_app_name: Option<String>, expected_exe: Option<String> }`.
- `Store { Steam, Epic, Manual }` ; `DetectedGame { game_id, store, install_dir: PathBuf, build_id: Option<String> }`.
- `DetectionRoots { steam_dir: Option<PathBuf>, epic_manifests_dir: Option<PathBuf> }` ; `DetectionRoots::from_system()` (registre derrière `cfg(windows)`).
- `detect_games(roots, rules: &[DetectRule]) -> Vec<DetectedGame>` (triés par `game_id` puis store ; un manifeste cassé n'empêche pas les autres) ; `validate_manual_dir(rule, path) -> Result<DetectedGame, DetectError>`.

**`profile.rs`, `catalog.rs` (T2), `install.rs` (T6), `uninstall.rs` (T7), `lockfile.rs` (T9), `launch.rs` (T11)** : voir les tâches, détail donné à chaque agent le moment venu.

## Tests attendus par tâche (vague 1)

- **T1** : substitution simple ; `{{` ; variable inconnue refusée ; saut de ligne et NUL refusés ; `Paths::under(tmp)` donne la bonne arborescence.
- **T3** : refusés : `../x`, `a/../../x`, `/abs`, `C:\x`, `\\serveur\x`, `fichier:flux`, `CON`, lien symbolique ; trop d'entrées ; bombe zip (taille réelle décompressée) ; `strip_prefix` ; extraction normale avec les bons hash ; une jonction ou un lien déjà présent dans la racine n'est pas suivi.
- **T4** : bon hash → fichier rangé dans le cache ; mauvais hash → erreur, et ni `.part` ni fichier ne restent ; au-delà de `max_bytes` → erreur ; fichier déjà en cache → pas de nouveau téléchargement ; cache corrompu → retéléchargé ; `http://` refusé ; un test réseau réel marqué `#[ignore]`.
- **T5** : aller-retour TOML ; aucun `.tmp` ne reste ; `format_version` future refusé ; sortie stable.
- **T10** : fichier absent → valeurs par défaut ; aller-retour ; ancien fichier sans le nouveau champ lisible ; fichier illisible mis de côté et non écrasé ; aucun mot de passe sérialisé.
- **T12** : avec le serveur maison lancé dans le même processus : mauvais slot, mauvais jeu, mauvais mot de passe, port fermé, timeout (un écouteur qui n'accepte rien), TLS (serveur wss à certificat auto-signé) ; chaque cas donne la bonne variante et une phrase en français. Vérifier que le repli de `wss` vers `ws` de `client-core` ne cache pas l'erreur TLS.
- **T13** : faux dossiers Steam (`libraryfolders.vdf` à plusieurs bibliothèques, `appmanifest_<appid>.acf`) et Epic (`.item`), sans Steam installé ; bibliothèque absente ignorée sans erreur fatale ; manifeste cassé ignoré ; jeu en cours de mise à jour (`StateFlags` sans le bit 4) signalé ; chemins avec accents et espaces ; dossier manuel valide et invalide ; ordre déterministe.
