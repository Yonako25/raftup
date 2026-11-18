# Journal des sessions

> Ajout seulement, les plus récentes **en bas**. Une entrée = 5 lignes au plus. L'avancement global reste dans `etat-du-projet.md`, les décisions dans `docs/adr/`.
> Écrit avec la compétence `/cloture`. Au démarrage, lire les **5 dernières entrées**.

## En cours

_Une session qui touche à un dossier partagé (par exemple `crates/launcher-ui/`) écrit ici une ligne `session → fichiers` et la retire à la clôture._

_Aucune session en cours._

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

### 2026-11-17 · Écran « Créer une partie », étape 1
- **Fait** : état « Préparation » (`screens/create.rs`) : YAML par sélecteur (ajout sans doublon, retrait), apworlds derrière l'avertissement et « J'ai compris », réglages du serveur, `build_request` qui donne `GenerateRequest` + `PartyHostOptions` ou un message du contrat ; accès par « Créer depuis des YAML » et `RAFTUP_DEV_PAGE=creer`. 8 tests ; `cargo test -p launcher-ui` (31) et clippy passent. Pas de commit encore.
- **Pas vérifié** : rendu à l'œil (accès à la fenêtre refusé) ; boîtes de fichiers ; `cargo test --workspace` non relancé. Le bouton « Générer » ne fait que vérifier le formulaire.
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : étape 2, runtime absent : consentement au téléchargement (≈ 72 Mo), progression, annulation.

### 2026-11-17 · Écran « Créer une partie », étape 2
- **Fait** : installation du générateur annulable dans `launcher-core` (`Downloader::get_cancellable`, `ensure_runtime_cancellable`, `GeneratorPins::download_bytes`, `CancelToken` réexporté) avec 2 tests ; panneau `generator_setup` : consentement (sources, taille, hash), progression par étape et globale, « Annuler », erreurs du contrat, « Réessayer ». `cargo test --workspace`, clippy et fmt passent. Pas de commit encore.
- **Pas vérifié** : vrai téléchargement des ≈ 72 Mo (pas d'accord demandé) ; rendu à l'œil ; fermeture de la fenêtre pendant l'installation (thread détaché, restes nettoyés au prochain essai en théorie).
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : étape 3, la génération : barre indéterminée, journal en direct, « Annuler ».

### 2026-11-18 · Écran « Créer une partie », étape 3
- **Fait** : panneau `generation` : `generate_blocking` dans un thread `std` avec `PythonBackend` (réseau bloqué), barre indéterminée, temps écoulé, journal en direct (2 000 lignes au plus), « Annuler », messages d'erreur du contrat avec « Voir le détail » (traceback) et « Réessayer » ; seed générée gardée pour l'étape 4. 7 tests ; `cargo test --workspace`, clippy et fmt passent. Pas de commit encore.
- **Pas vérifié** : vraie génération depuis l'interface (générateur pas installé ici) ; rendu à l'œil ; fermeture de la fenêtre pendant une génération.
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : étape 4, l'hébergement de la seed générée (`host_generated`, « Réessayer » sur un autre port sans régénérer).

### 2026-11-18 · Écran « Créer une partie », étape 4
- **Fait** : module `hosting` : la seed générée est hébergée par le worker avec les réglages lus au clic sur « Générer » ; bascule vers « Partie en cours » ; en cas d'échec, « Impossible de démarrer le serveur : {raison}. » et « Réessayer » qui relit le formulaire sans régénérer ; refus si une partie tourne déjà. 6 tests ; `cargo test --workspace`, clippy et fmt passent. Pas de commit encore.
- **Pas vérifié** : enchaînement réel génération puis hébergement (générateur pas installé ici) ; rendu à l'œil ; port réellement occupé.
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : faire tourner l'écran en vrai (installer le générateur, générer une partie APQuest, l'héberger) et le regarder à l'œil.

### 2026-11-18 · Essai réel de « Créer une partie »
- **Fait** : générateur installé pour de vrai dans un dossier de test (`Z:\raftup-test\runtime`, via `setup_generator`) ; tests réels de `launcher-core` passés (3 générations, génération + hébergement) ; nouveau test réel `reel_generer_puis_heberger` (`#[ignore]`) qui suit le chemin de l'écran sans fenêtre : 2 joueurs APQuest générés, port occupé refusé, « Réessayer » sur un autre port sans régénérer, deuxième hébergement refusé. Message lisible pour un port déjà utilisé. `cargo test -p launcher-ui -- --include-ignored` (52) et clippy passent. Pas de commit encore.
- **Pas vérifié** : rendu à l'œil (accès à la fenêtre refusé, session à distance) ; installation par le panneau de consentement (faite par l'exemple) ; un ancien `raftup.exe` du 17 novembre tourne encore et bloque `target/debug` (compilé dans `target/essai`).
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : regarder l'écran « Créer une partie » à l'œil, devant le PC (`RAFTUP_DEV_RACINE=Z:\raftup-test\runtime`, `RAFTUP_DEV_PAGE=creer`).

### 2026-11-18 · Macro de captures du launcher
- **Fait** : macro de debug `devscript.rs` (`RAFTUP_DEV_CAPTURE`, `RAFTUP_DEV_YAML`, `RAFTUP_DEV_PORT`) qui remplit « Créer une partie », clique et enregistre ses propres captures PNG, sans boîte de validation ni faux clics Windows ; écran regardé sur 4 captures (préparation, génération, port occupé, partie en ligne après « Réessayer ») ; chemin de la seed affiché en double retiré. 53 tests de `launcher-ui` (réel compris), clippy et build `--release` passent. Pas de commit encore.
- **Pas vérifié** : panneau de consentement de l'installation (générateur déjà installé) ; boîtes de fichiers ; le bas de la barre latérale (« Compatible avec… ») est un peu serré à 820 px de haut.
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : capturer les autres écrans avec la macro (consentement du générateur, Bibliothèque, Héberger) pour finir le contrôle visuel.

### 2026-11-18 · Relecture de launcher-ui
- **Fait** : `launcher-ui` relu en entier ; corrigé : génération et hébergement suivis à chaque image même hors de l'écran « Créer » (`create::background`), bascule vers la partie qui était effacée par la barre latérale (trouvé par la macro), erreur effacée au changement d'écran et montrée aussi sur « En cours » et Bibliothèque, état de « Partie en cours » remis à zéro à l'arrêt, `auto-enabled` bien décrit, « Lancement interrompu », interface toute au vouvoiement. 54 tests (réel compris), clippy, build `--release` et macro de captures passent. Pas de commit encore.
- **Pas vérifié** : redimensionnement de la fenêtre sans bordure par ses bords (à regarder devant le PC).
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : rejoindre par lien d'invitation (F06).

### 2026-11-18 · Travail en parallèle (3 agents)
- **Fait** : commandes serveur après `!admin login` (`/players`, `/status`, `/send`, `/send_location`, `/release`, `/collect`, `/allow_release`, `/hint`, `/save`…, textes de 0.6.8, 12 tests) ; fenêtre redimensionnable par ses bords (`resize.rs`, coins en L pour épargner « Fermer ») ; macro de captures étendue (`RAFTUP_DEV_SCENARIO` : bibliotheque, heberger, en-cours, generateur-absent), barre latérale non coupée ; ADR 0020 proposé pour rejoindre par lien (F04, et non F06). `cargo test --workspace`, test réel, clippy, build `--release` et macro passent.
- **Pas vérifié** : textes `!admin` comparés au code Python, pas à un serveur officiel lancé ; redimensionnement à la souris ; la poignée du bord droit (5 px) chevauche un peu la barre de défilement des pages.
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : faire valider les 7 décisions de l'ADR 0020, puis coder `invite.rs`, `instance.rs` et `url_scheme.rs` en parallèle.

### 2026-11-18 · Rejoindre par lien (F04)
- **Fait** : ADR 0020 accepté (rust-version 1.89) ; `launcher-core` : `invite` (liens `raftup://join`, 55 cas), `instance` (verrou + boîte aux lettres), `url_scheme` (HKCU, idempotent) ; `launcher-ui` : liens par joueur et avertissement d'adresse locale dans « Partie en cours », écran « Rejoindre » (aussi via « Invitations »), démarrage (`startup.rs` : lien en argument, 2e instance qui passe la main, fenêtre ramenée au premier plan) ; recopie de l'adresse hébergée qui n'écrase plus un lien. 70 tests de `launcher-ui`, workspace, clippy et `--release` passent ; parcours essayé en vrai avec la macro (`RAFTUP_DEV_SCENARIO=rejoindre`).
- **Pas vérifié** : clic sur un vrai lien `raftup://` (schéma enregistré seulement par le binaire `--release`, pas installé) ; Discord rend-il le lien cliquable ? ; un `.link` impossible à supprimer serait relu en boucle.
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : test sur une VM Windows propre avec le binaire `--release` (critère de fin de J3), clic sur un lien compris.
