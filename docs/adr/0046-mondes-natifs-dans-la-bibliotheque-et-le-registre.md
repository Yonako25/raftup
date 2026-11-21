# ADR 0046 — Mondes natifs dans la Bibliothèque et le registre (V3)

- **Statut** : accepté en autonomie le 21 novembre 2026, à relire par l'utilisatrice
- Prolonge les ADR 0032 (registre signé), 0041 (format de monde natif), 0042 (conversion apworld ↔ natif) et 0045 (niveaux de confiance).

## Contexte

Dernier morceau de la V3 d'après l'état du projet : afficher les mondes natifs dans la Bibliothèque, les installer par le registre, et rendre l'export JSON (ADR 0028 / 0042) accessible depuis le launcher. Le format de l'index (ADR 0032, 0045) ne connaît que `apworld` et `profile`.

## Décisions

### 1. Index de format 3 : sorte `native_world`

- `PackageKind::NativeWorld` (`"native_world"` dans le JSON), triée **après** les apworlds et les profils : un index sans monde natif garde les mêmes octets.
- `format_version` 3 = format 2 + cette sorte. Le launcher lit 1, 2 et 3. Un index 1 ou 2 qui contient `native_world` est refusé (« permise à partir du format 3 »).
- `publish` écrit **le plus petit format qui suffit** (`Index::minimal_format`) : 2 sans monde natif, 3 sinon. Ainsi un index sans monde natif reste lisible par un launcher qui ne connaît que le format 2 (règle de l'ADR 0045 sur les nouveaux formats). En mémoire, un index lu est toujours au dernier format.
- Le fichier source de `publish` ne change pas (`kind = "native_world"`).

### 2. Paquet de monde natif

- Soit un `world.toml` seul (texte UTF-8), soit un zip qui le contient, à la racine ou dans un dossier `<name>/`, avec au plus 32 fichiers simples à côté (licence, lisez-moi) : noms `[A-Za-z0-9._-]`, sans point au début, sans sous-dossier, 8 Mio décompressés au plus. Le nom `raftup-paquet.toml` est réservé.
- Le monde est relu par `worlds-native` ; si le registre annonce `game`, ce doit être celui du monde.
- Même lecture au launcher (installation) et à la CI (`launcher_core::native_worlds::read_native_package`).

### 3. Installation et désinstallation (`launcher_core::native_worlds`)

- Posé dans `mondes-natifs/<name>/` avec un **reçu** `raftup-paquet.toml` (versionné : origine registre ou conversion, version, sha256, niveau, licence, auteurs, liste des fichiers posés).
- Fichiers d'abord écrits dans `mondes-natifs.partiel/<name>/` (à côté, jamais lu par le moteur), puis dossier renommé.
- Refus : un autre monde du même nom (jamais écrasé), un monde installé qui fournit déjà le jeu. Même paquet (même sha256 dans le reçu) : « déjà là », rien n'est téléchargé.
- Désinstallation depuis la fiche, en deux clics : un `<clé>.toml` posé à la main est effacé ; pour un dossier, seuls `world.toml`, le reçu et les fichiers listés par le reçu sont effacés, puis le dossier s'il est vide. Un fichier ajouté à la main reste (et le message le dit). Aucun lien n'est suivi, aucune descente dans un sous-dossier.

### 4. Bibliothèque

- Les mondes de `mondes-natifs/` (mêmes règles de lecture que le moteur Rust, ADR 0041) sont des cartes de la grille, après les jeux, avec le marqueur « Monde natif » ; la fiche donne nom, version (celle du registre, sinon « non précisée » : un `world.toml` n'a pas de version), `format_version`, lieux, événements, objets, régions, options, origine, badge de confiance (registre), emplacement, et l'export JSON pour un monde converti.
- « Mondes natifs du registre » liste les mondes natifs de l'index, avec le même badge et la même confirmation que « Mondes du registre » (ADR 0045), et un texte adapté : un monde natif est une donnée, sans code.

### 5. Export JSON et conversion depuis le launcher

- Options avancées de la Bibliothèque : « Convertir un apworld en monde natif ». On choisit un apworld installé (`worlds/<version>`), le jeu est pré-rempli depuis son `archipelago.json`.
- Le générateur officiel construit le monde pour **un joueur, options par défaut, graine 1** (`generation::export_worlds_json`, mêmes protections que `inspect_worlds` : verrou, bac à sable, réseau bloqué, dossier de travail effacé), puis `generate::convert` (ADR 0042) écrit le monde. Le dossier `mondes-natifs/<nom de l'apworld>/` reçoit `world.toml`, `export.json` (celui que lit `raftup-monde importer`) et le reçu.
- Générateur absent : le même panneau de consentement et d'installation que « Créer une partie » (`generator_setup`).
- Pas de `slot_data` (il faudrait générer la seed entière) : le convertisseur le signale déjà dans ses points à vérifier.

### 6. Niveaux de confiance d'un monde natif (`raftup-registry check`)

- « Structure valide » : la lecture du § 2.
- « Générable » : le **moteur Rust** de Raftup génère une partie solo, options par défaut, sur N graines. Pas de Python ni de bac à sable : un monde natif est une donnée. Cette étape tourne même avec `--structure-seule` (qui ne concerne que le générateur Python) et hors Windows.
- Pas encore de fuzzing des mondes natifs : le niveau s'arrête à « Générable ». Le texte du niveau dit maintenant « le générateur (officiel pour une apworld, moteur Rust pour un monde natif) ».

## Conséquences

- `NativeWorld` garde son `format_version` (affiché par la fiche).
- `registry-tool` dépend de `generate` (déjà dans le workspace).
- Un monde natif du registre n'est jamais exécuté ; seule la conversion lance Python, dans le bac à sable de l'ADR 0034.
- Reste : fuzzing des options d'un monde natif ; capture de la fiche d'un monde installé depuis un vrai registre ; conversion essayée de bout en bout depuis le launcher avec le vrai générateur (le chemin sans Python est testé avec l'export réel d'APQuest).
