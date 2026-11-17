# 0015 — Profils, installation, lockfile et lancement (`launcher-core`)

- Statut : accepté
- Date : 2026-11-17

## Contexte

- `launcher-core` sait seulement héberger une partie (ADR 0010).
- J3 demande aussi : détecter les jeux (F01), installer par profil (F02), figer les versions de la session (F03), lancer le jeu avec les identifiants (F04), expliquer les échecs de connexion (F05), garder la configuration (F07) et désinstaller proprement (F11).
- L'ADR 0001 fixe déjà le catalogue : public, dans `profiles/`, embarqué dans le launcher, avec un sha256 épinglé.
- Le détail (modules, types, tâches) est dans `docs/plan-launcher-core.md`.

## Décision

### Principes

1. Le **profil TOML** embarqué est la seule source de vérité. Il est validé au chargement ; un test de la CI refuse un profil faux.
2. Une installation écrit un **reçu** (un fichier TOML par jeu) **avant** de toucher au dossier du jeu. C'est ce qui rend la désinstallation sûre, même après un plantage.
3. Un téléchargement passe par un **cache rangé par sha256**, vérifié avant toute extraction. Une seule fonction extrait les zips et contrôle chaque chemin.
4. Le lancement se fait en deux temps : `plan_launch` (fonction pure, testable), puis `launch` (sans shell).
5. Le code d'installation et de lancement est **synchrone** ; l'interface l'appelle depuis un thread à part. Seul le diagnostic de connexion (F05) est asynchrone, car il s'appuie sur `client-core`.

### Profil (`profiles/<game.id>.toml`)

- Champ `format_version` obligatoire ; le launcher refuse une version plus récente que la sienne.
- Sections : `[game]` (id, nom, jeu Archipelago, statut, règles de détection), `[credits]`, `[[steps]]`, `[preserve]`, `[launch]`.
- Une étape : `kind` = `mod_loader`, `mod` ou `apworld` (**jamais** `rom` ni `asset`), version, source, `sha256` **toujours obligatoire**, action d'installation.
- Les étapes sont appliquées dans l'ordre et désinstallées dans l'ordre inverse.
- Variables : `{game_dir}` et `{worlds_dir}` dans les étapes ; au lancement s'ajoutent `{host}`, `{port}`, `{address}`, `{slot}`, `{password}`. Une variable inconnue est refusée au chargement.
- Méthodes de connexion (F04) : `cli_args`, `config_file`, `env`, `uri`, `manual`.
- Le nom du fichier doit être égal à `game.id`.

### Décisions de détail

| # | Sujet | Décision |
| --- | --- | --- |
| 1 | Client HTTP | `ureq` (bloquant), moteur TLS `ring`, comme le reste du projet |
| 2 | Dossiers de l'utilisateur | crate `directories` ; tout passe par une structure `Paths`, remplaçable par un dossier temporaire dans les tests |
| 3 | Embarquer les profils | liste `include_str!` écrite à la main, avec un test qui la compare au contenu de `profiles/` |
| 4 | Sources de téléchargement | `url` (HTTPS seulement) et `thunderstore` ; liste blanche de domaines (`github.com`, `objects.githubusercontent.com`, `thunderstore.io`, `gcdn.thunderstore.io`), vérifiée par un test |
| 5 | Lockfile | fichier `<seed>.raftup-lock.toml` à côté de la seed au MVP ; **jamais d'URL ni de chemin** dedans |
| 6 | Versions | une seule version par étape et par profil ; mettre à jour un mod demande une nouvelle version du launcher |
| 7 | Fichiers créés par le jeu en tournant | champ `cleanup` dans le profil ; suppression seulement après confirmation dans l'interface |
| 8 | Fichier modifié depuis l'installation | la désinstallation le signale ; elle ne le supprime qu'avec `force` |
| 9 | `config_file` | JSON seulement ; `config_file` et `uri` ne sont codés qu'au premier jeu qui en a besoin (Slime Rancher est en `manual`) |
| 10 | Mot de passe dans la config du mod | accepté et documenté ; préférence `env`, puis `config_file`, puis `cli_args` quand le mod laisse le choix |
| 11 | Jeux Steam | on lance l'exécutable directement |
| 12 | Plusieurs copies d'un jeu | une seule installation par `game_id` |

### Où vivent les données

| Quoi | Où |
| --- | --- |
| Catalogue | `profiles/*.toml`, embarqué dans le binaire |
| Configuration utilisateur | `%APPDATA%\Raftup\config.toml` |
| Reçus d'installation | `%LOCALAPPDATA%\Raftup\installs\<game_id>.toml` |
| Originaux sauvegardés | `%LOCALAPPDATA%\Raftup\backups\<game_id>\` |
| Cache des téléchargements | `%LOCALAPPDATA%\Raftup\cache\downloads\<sha256>` |
| Apworlds | `%LOCALAPPDATA%\Raftup\worlds\<version_ap>\` |
| Lockfile de session | à côté de la seed |

### Réversibilité (F11)

1. Le launcher calcule le plan complet avant d'écrire : archive lue, pas encore extraite.
2. Il sauvegarde chaque fichier existant qui va être remplacé.
3. Il écrit le reçu (état `InProgress`), de façon atomique (fichier `.tmp` puis renommage).
4. Il extrait, puis passe le reçu à `Complete` en notant le hash de chaque fichier écrit.
5. La désinstallation suit l'ordre inverse : fichiers créés (si leur hash n'a pas changé), sauvegardes restaurées, dossiers vides supprimés, reçu supprimé en dernier.
6. Un reçu resté `InProgress` après un plantage se désinstalle par le même chemin.

### Configuration qui survit aux mises à jour (F07)

- Elle vit hors du dossier du binaire, avec un `format_version` et des migrations.
- Un champ nouveau se lit avec une valeur par défaut.
- Un fichier illisible est mis de côté (`config.toml.illisible`), jamais écrasé en silence.
- La configuration des mods est protégée par `[preserve]` : une mise à jour ne l'écrase jamais.
- Aucun mot de passe n'est écrit dans `config.toml` (NF09).

### Détection des jeux (F01)

- **Steam** : `libraryfolders.vdf` puis `appmanifest_<appid>.acf` dans **toutes** les bibliothèques ; parseur `keyvalues-parser` (MIT ou Apache-2.0).
- **Epic** : manifestes `.item` (JSON) ; lecture tolérante, le format n'est pas documenté.
- **Manuel** : présence de l'exécutable attendu ; un hash n'est qu'un avertissement.
- Registre Windows : `winreg`, derrière `cfg(windows)` et une petite fonction.
- `detect_games` reçoit ses dossiers en paramètre : les tests utilisent de faux dossiers, sans Steam installé.
- Un jeu en cours de mise à jour ou lancé bloque l'installation de mod.

## Conséquences

- Un profil faux est arrêté par la CI, pas chez le joueur.
- Deux joueurs de versions différentes peuvent avoir des lockfiles incompatibles : message « mettez Raftup à jour ».
- Nouvelles dépendances de `launcher-core` : `toml`, `zip`, `ureq`, `directories`, `keyvalues-parser`, `winreg` (Windows), `client-core` ; `tempfile` en test. Licences à confirmer à l'ajout de chacune. Le `unsafe` interne de `winreg` ne contredit pas la règle (pas d'`unsafe` dans **notre** code).
- Slimipelago n'a pas de licence : autorisation écrite de l'auteur avant d'écrire le profil réel de Slime Rancher.
- Un mod est du code arbitraire : la seule confiance est la relecture du catalogue.
- Un hash ne protège pas d'un profil malveillant fusionné : relecture des pull requests et liste blanche indispensables.
- Jamais de jeu, de ROM ou d'asset distribué : cibles limitées à `{game_dir}` et `{worlds_dir}`, aucun mod ré-hébergé.
- À vérifier avant d'adopter : clés du registre, valeurs de `StateFlags`, absence d'`unsafe` dans `keyvalues-parser`, identifiants Epic des cinq jeux.
