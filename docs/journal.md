# Journal des sessions

> Ajout seulement, les plus récentes **en bas**. Une entrée = 5 lignes au plus. L'avancement global reste dans `etat-du-projet.md`, les décisions dans `docs/adr/`.
> Écrit avec la compétence `/cloture`. Au démarrage, lire les **5 dernières entrées**.

## En cours

_Aucune session n'indique de fichiers en cours. Une session qui touche à un dossier partagé (par exemple `crates/launcher-ui/`) écrit ici une ligne `session → fichiers` et la retire à la clôture._

## Entrées

### 2026-11-17 · Launcher selon maquette (session parent)
- **Fait** : profil Slime Rancher en brouillon (`docs/profils-en-attente/`), `py-bridge` (ADR 0016), étape `generate` avec Python embarqué et Archipelago 0.6.8 épinglé (ADR 0017), `generate_and_host`, méthode de travail (`docs/regles-agents.md`), contrat de l'écran de génération, suivi entre sessions (ce journal, `/cloture`, hook). Commits `94df2cb` à `ef1c9f3`.
- **Pas vérifié** : interface de génération (pas écrite) ; déclenchement réel du hook de rappel ; 7 mondes d'Archipelago ne s'importent pas faute de dépendances ; rendu de l'interface jamais contrôlé à l'œil.
- **Ne pas toucher** : `crates/launcher-ui/**` et `docs/comparaison/**` (session ui-bibliotheque) ; `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : écran « Créer une partie » d'après `docs/contrat-ui-generation.md` une fois l'écran Bibliothèque fini ; décider la règle de conservation de `generations/`.

### 2026-11-17 · Captures différentielles J2
- **Fait** : scénarios `commandes` et `indices` capturés sur le serveur officiel 0.6.8 et rejoués dans `tests/connexion.rs` ; un écart corrigé (texte d'aide de `!status`) ; masque de `!admin login` normalisé ; `game_vectors.py` tolère une connexion fermée ; le pilote espace les `Say`. Différentiel : 0 écart sur 24 scénarios fixes, 150 au hasard (graine 7) sans écart. `cargo test -p server` et clippy passent. Commit `2fb5cd6`.
- **Pas vérifié** : `cargo test --workspace` (launcher-ui en cours dans une autre session) ; le générateur au hasard ne joue toujours ni commandes ni indices.
- **Ne pas toucher** : `crates/launcher-ui/**`, `docs/comparaison/**`, `Cargo.lock` (session ui-bibliotheque) ; `docs/profils-en-attente/`.
- **Suite** : ajouter commandes `!` et indices au générateur au hasard (`hasard.py`).

### 2026-11-17 · Point d'étape et questions
- **Fait** : point sur l'avancement ; comptage des langages (Rust 58 % du dépôt, 94 % du code) ; explication du Python (worker de `py-bridge`, scripts de tests) et du plan de génération en Rust (V2, V3) ; rien de codé.
- **Pas vérifié** : l'écran Bibliothèque (session ui-bibliotheque) n'est toujours pas commité.
- **Ne pas toucher** : `crates/launcher-ui/**`, `docs/comparaison/**`, `Cargo.lock` (session ui-bibliotheque) ; `docs/profils-en-attente/`.
- **Suite** : commandes `!` et indices dans `hasard.py`, tâche validée pour la prochaine session.

### 2026-11-17 · Commandes et indices au hasard
- **Fait** : `hasard.py` joue des commandes `!` (`help`, `players`, `status`, `remaining`, `missing`, `checked`, `hint_location`, `alias`, `getitem`, `release`, `collect`) et des indices (`create_as_hint`, `CreateHints`, `UpdateHint`) ; 100 scénarios au hasard (graine 7) : 0 écart. Nouvel écart relevé et déclaré : `!hint` sans argument liste les indices dans l'ordre d'un `set` Python. Pas de commit encore.
- **Pas vérifié** : scénarios fixes et `cargo test --workspace` non relancés (le serveur n'a pas changé) ; autres graines.
- **Ne pas toucher** : `crates/launcher-ui/**`, `docs/comparaison/**`, `Cargo.lock` (session ui-bibliotheque) ; `docs/profils-en-attente/`.
- **Suite** : décider si l'on reproduit l'ordre du `set` Python pour `!hint` seul, ou si l'écart reste déclaré.

### 2026-11-17 · Ordre des `set` Python
- **Fait** : décision de reproduire l'ordre (ADR 0018) ; module `pyset` (table et hachage de CPython 3.12) vérifié sur 31 suites d'opérations notées par le vrai Python ; indices et petits `set` de slots branchés dessus ; tri des indices retiré du différentiel ; `!hint` seul joué au hasard. Différentiel : 0 écart sur 24 fixes et 100 au hasard (graine 7). `cargo test --workspace`, clippy et fmt passent. Pas de commit encore.
- **Pas vérifié** : seed avec `entrance` non vide (la seed MIT n'en a pas) ; groupes avec numéros de slot qui se heurtent dans la table ; autres graines.
- **Ne pas toucher** : `crates/launcher-ui/**`, `docs/comparaison/**`, `Cargo.lock` (session ui-bibliotheque) ; `docs/profils-en-attente/`.
- **Suite** : écran « Créer une partie » (`docs/contrat-ui-generation.md`) une fois la Bibliothèque commitée.

### 2026-11-17 · Vérification de l'écran Bibliothèque
- **Fait** : écran Bibliothèque laissé non commité par la session ui-bibliotheque, vérifié puis commité : 23 tests, clippy et fmt passent ; regardé à l'œil (catalogue vide, profil en attente refusé faute d'autorisation, profil de démo). `Cargo.lock` réparé (celui du dépôt avait des lignes déplacées).
- **Pas vérifié** : installation, lancement et désinstallation réels ; boîte « Ajouter un dossier de jeu » ; le reste du code de `launcher-ui` n'est pas relu en détail.
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : écran « Créer une partie » d'après `docs/contrat-ui-generation.md`, dans une nouvelle session.

### 2026-11-17 · Décisions de l'écran « Créer une partie »
- **Fait** : trois décisions du contrat validées et consignées (ADR 0019) : purge manuelle de `generations/`, YAML par sélecteur seul, réseau du générateur bloqué par défaut (`PythonBackend::new`, champ `block_network`). Contrat et ADR 0017 mis à jour. `cargo test -p launcher-core`, clippy et fmt passent. Pas de commit encore.
- **Pas vérifié** : test réel de génération avec le nouveau défaut (déjà passé avec `RAFTUP_BLOCK_NETWORK=1`, ADR 0017) ; `cargo test --workspace` non relancé.
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : coder l'écran « Créer une partie » d'après `docs/contrat-ui-generation.md`, dans une nouvelle session.
