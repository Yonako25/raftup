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

### 2026-11-18 · Binaire release sans VCRUNTIME, premier lancement réel
- **Fait** : `.cargo/config.toml` compile en `crt-static` sous Windows : `raftup.exe` ne dépend plus de `VCRUNTIME140.dll` (absente d'un Windows neuf), seulement de DLL système (vérifié avec `dumpbin /dependents`). Binaire `--release` lancé comme un joueur : dossier de données et clé `HKCU\Software\Classes\raftup` créés au premier lancement, puis un vrai lien `raftup://` ouvert par Windows a été passé à la fenêtre déjà ouverte (un seul processus, boîte aux lettres vidée).
- **Pas vérifié** : écran affiché après le clic (le release n'a pas la macro de captures) ; machine sans bon pilote graphique (wgpu) ; SmartScreen et pare-feu au premier lancement (il faut une machine propre).
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : test sur une machine propre (VirtualBox ou autre PC) juste avant la première diffusion.

### 2026-11-18 · Catalogue Slime Rancher et fonctions V1 restantes
- **Fait** : Slime Rancher au catalogue (a48e924, ADR 0021) ; F10 cœur+branchement (e630fbc, eaaa533) ; F12 cœur (e630fbc) ; F06 délai seulement (18f8651) ; F08 salon côté hôte (commit suivant). Workspace, clippy et fmt passent.
- **Pas vérifié** : onglet « Salon » et sélecteur de preset jamais vus à l'écran ; aucun vrai message Discord ; signature minisign non recoupée avec l'outil officiel ; Slime Rancher jamais joué.
- **Ne pas toucher** : `docs/profils-en-attente/` (reste le brouillon de demande).
- **Suite** : valider les ADR 0022 à 0024, puis J1 étape 9 (partie réelle de Slime Rancher).

### 2026-11-18 · F12 mise à jour terminée
- **Fait** : ADR 0022-0024 acceptés ; outil `update_tool`, vraie clé publique, bannière et remplacement de l'exe (4bc917b). Workspace, clippy et fmt passent.
- **Pas vérifié** : remplacement réel de l'exe sur un release ; bannière jamais vue ; signature recoupée avec l'outil minisign officiel.
- **Ne pas toucher** : `docs/profils-en-attente/`, la clé privée (`C:\Users\ionak\.raftup-keys`, hors dépôt).
- **Suite** : sauvegarder la clé privée ; J1 étape 9 (partie réelle de Slime Rancher).

### 2026-11-18 · Reste de la V1, première vague
- **Fait** : webhook dans « Créer » et mémorisé (ADR 0025), préréglage mémorisé, `meta.yaml` des préréglages (ADR 0023 étape 2), salon rafraîchi tout seul (ADR 0024), 7 mondes importés dont zillion par `[[source_packages]]` (ADR 0026 accepté, c2520cf). Workspace, clippy et fmt passent.
- **Pas vérifié** : test réel du `meta.yaml` ; aucun champ ni onglet vu à l'écran ; soe demande la ROM sans message clair ; kdl3 réclame `PyMemoryEditor`.
- **Ne pas toucher** : `docs/profils-en-attente/` ; `Z:\raftup-test\runtime2` (runtime de vérification).
- **Suite** : regarder l'interface avec la macro de captures, puis J1 étape 9.

### 2026-11-18 · Contrôle visuel de la V1 par captures
- **Fait** : scénario `salon` de la macro de captures ; onglet « Salon », préréglages, champ webhook et panneau d'installation du générateur regardés à l'écran ; pastilles (« Pas testé », « En ligne ») qui prenaient toute la hauteur de la ligne, dessinées à leur taille (18bd63d). Workspace, clippy et fmt passent.
- **Pas vérifié** : « Tout tester » cliqué ; bannière de mise à jour ; fin de la génération par la macro (le runtime `Z:
aftup-test
untime` est périmé depuis les nouveaux pins).
- **Ne pas toucher** : `docs/profils-en-attente/` ; `Z:
aftup-test
untime2`.
- **Suite** : J1 étape 9, partie réelle de Slime Rancher.

### 2026-11-18 · ROM absente à la génération
- **Fait** : sans ROM, soe bloquait la génération 5 min (boîte de fichier invisible d'Archipelago) ; le worker met `settings.no_gui = True` et l'erreur arrive en 6 s ; l'écran traduit ce `FileNotFoundError` en message clair (b14af43). Test réel `monde_avec_rom_absente`, workspace, clippy et fmt passent ; tests réels à lancer avec `--test-threads=1`.
- **Pas vérifié** : message vu à l'écran ; autres mondes à ROM (seul soe essayé). Vrai gras Inter SemiBold pas fait : il faut télécharger la police, accord à demander.
- **Ne pas toucher** : `docs/profils-en-attente/` ; `Z:\raftup-test\runtime2`.
- **Suite** : J1 étape 9, partie réelle de Slime Rancher (avec l'utilisatrice).

### 2026-11-18 · Vrai gras Inter SemiBold
- **Fait** : Inter SemiBold 4.001 (archive officielle `Inter-4.1.zip`, OFL) intégrée, famille `Bold` dans `theme.rs`, appliquée aux 18 textes `.strong()` ; gras vu sur captures (En cours, barre latérale). Workspace, clippy et fmt passent.
- **Pas vérifié** : les scénarios `salon` et `bibliotheque` de la macro se sont arrêtés sur « En cours » (onglet et Bibliothèque pas capturés cette fois).
- **Ne pas toucher** : `docs/profils-en-attente/` ; `Z:
aftup-test
untime2`.
- **Suite** : J1 étape 9, partie réelle de Slime Rancher (avec l'utilisatrice).

### 2026-11-18 · Partie réelle de Slime Rancher (J1 étape 9)
- **Fait** : avec l'utilisatrice devant le PC : installation par la Bibliothèque (MelonLoader, Slimipelago, apworld ; hash vérifiés), mod chargé au lancement, vrai générateur installé, partie générée par « Créer une partie » puis hébergée, mod connecté sans modification, lieux validés et objets reçus. J1 terminé.
- **Pas vérifié** : reprise de la sauvegarde `.raftup` après redémarrage ; partie à plusieurs joueurs ; DeathLink.
- **Ne pas toucher** : `docs/profils-en-attente/` ; `Z:
aftup-test
untime2`.
- **Suite** : « Créer une partie » reprend seul les apworlds installés par la Bibliothèque.

### 2026-11-18 · Apworlds installés pris automatiquement
- **Fait** : « Créer une partie » ajoute seul les apworlds posés par la Bibliothèque (`installed_apworlds`, relus à chaque clic, sans l'avertissement) et les liste sous « Pris automatiquement » ; un apworld de même nom choisi à la main passe avant. Partie Slime Rancher générée et hébergée par la macro avec le seul YAML. Workspace, clippy et fmt passent.
- **Pas vérifié** : apworld installé pour une autre version d'Archipelago que 0.6.8 (seul `worlds/0.6.8` est lu).
- **Ne pas toucher** : `docs/profils-en-attente/` ; `Z:
aftup-test
untime2`.
- **Suite** : « Lancer le jeu » sans adresse ni slot (premier lancement).

### 2026-11-18 · Lancer le jeu hors partie
- **Fait** : `plan_launch_offline` (méthode de connexion du profil ignorée) ; dans la Bibliothèque, « Lancer le jeu » marche avec adresse et slot vides (jeu sans connexion), reste grisé si un seul des deux est rempli, avec une mention sous le bouton. Tests du cœur et de l'écran, workspace, clippy et fmt passent ; bouton vu sur capture.
- **Pas vérifié** : clic réel sur « Lancer le jeu » sans adresse (Slime Rancher démarré par Raftup hors partie).
- **Ne pas toucher** : `docs/profils-en-attente/` ; `Z:
aftup-test
untime2`.
- **Suite** : reprise d'une partie hébergée après redémarrage de Raftup, en vrai.

### 2026-11-18 · Reprise réelle d'une partie
- **Fait** : avec l'utilisatrice : partie Slime Rancher réhébergée après redémarrage de Raftup, sauvegarde `.raftup` reprise (1/152), jeu reconnecté, nouvel emplacement validé (2/152) sans objet reçu en double.
- **Pas vérifié** : reprise après un arrêt brutal (processus tué) ; partie à plusieurs joueurs.
- **Ne pas toucher** : `docs/profils-en-attente/` ; `Z:
aftup-test
untime2`.
- **Suite** : clic sur un lien `raftup://` depuis Discord ou un navigateur avec le binaire `--release`.

### 2026-11-18 · Liens d'invitation et boutons « Copier » en vrai
- **Fait** : boutons « Copier » muets réparés (eframe sans « clipboard », 889f974, avec test) ; lien `raftup://join/?…` réécrit par le navigateur accepté, bulle « Copié » (910bbcd) ; avec l'utilisatrice : clic sur le lien depuis le navigateur → écran « Rejoindre » rempli. Discord ne rend pas le lien cliquable (ADR 0020 mis à jour).
- **Pas vérifié** : après réhébergement, le compteur affichait 1/152 alors que 2/152 avait été annoncé ; à recontrôler (deuxième emplacement peut-être jamais envoyé).
- **Ne pas toucher** : `docs/profils-en-attente/` ; `Z:
aftup-test
untime2`.
- **Suite** : recontrôler la sauvegarde (valider un emplacement, réhéberger, comparer le compteur).

### 2026-11-18 · Sauvegarde recontrôlée
- **Fait** : doute du 1/152 levé : le 2e emplacement annoncé n'avait jamais été envoyé (sauvegarde lue : un seul lieu). Nouvel essai avec l'utilisatrice : lieu validé (2/152), écrit sur le disque aussitôt, puis Raftup fermé, rouvert, partie réhébergée : 2/152 repris.
- **Pas vérifié** : reprise après un arrêt brutal (processus tué) ; partie à plusieurs joueurs.
- **Ne pas toucher** : `docs/profils-en-attente/` ; `Z:
aftup-test
untime2`.
- **Suite** : partie à deux joueurs (deux slots, deux jeux ou deux instances).


### 2026-11-18 · Démarrage de la V2 : ADR du générateur Rust
- **Fait** : ADR 0027 rédigé (`a1ec081`) puis accepté tel que recommandé : crates `logic` / `fill` / `generate`, sortie `.archipelago` (écrivain de pickle à ajouter), seed reproductible dans Raftup (ChaCha), APQuest en premier monde, Python en secours. Aucun code.
- **Pas vérifié** : calcul exact du `checksum` du datapackage d'Archipelago ; jeu réel d'APQuest avec son client.
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : étape 1 de l'ADR 0027 : créer la crate `logic` (modèle en arènes, règles, balayage par sphères), dans une nouvelle session.

### 2026-11-18 · V2 étape 1 : crate `logic`
- **Fait** : crate `logic` (`a3b2c81`) : `MultiWorld` en arènes à index `u32`, noms uniques par joueur, règles `rule_builder` sans « non », inventaire par sorte d'objet (progression seule), atteignabilité par point fixe, balayage par sphères, `validate`. 11 tests sur petits mondes ; workspace, clippy et fmt passent.
- **Pas vérifié** : performance sur un gros multiworld (point fixe qui repasse sur tous les lieux) ; vocabulaire comparé au vrai `rule_builder` seulement via l'ADR.
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : étape 2 de l'ADR 0027 : écrivain de pickle et `write_archipelago` dans `multidata`.

### 2026-11-18 · V2 étape 2 : écriture du `.archipelago`
- **Fait** : écrivain de pickle protocole 4 limité à la liste blanche (`PyValue`) et `write_archipelago` dans `multidata` (`bcc6a68`) : dictionnaire et types de `Main.py`, zlib 9, mêmes octets à chaque écriture. Seed MIT relue à l'identique par notre lecteur et par le vrai Python 0.6.8 (`relue_par_le_vrai_python`, `#[ignore]`, avec `RAFTUP_SITE_PACKAGES`). Workspace, clippy et fmt passent.
- **Pas vérifié** : seed écrite hébergée par le serveur officiel (`MultiServer`) ; `checks_in_area` et options serveur hors modèle écrits vides ou absents.
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : étape 3 de l'ADR 0027 : crate `fill` (placement supposé, faisabilité, erreur expliquée).

### 2026-11-18 · V2 étape 3 : crate `fill`
- **Fait** : crate `fill` (`73a2b03`) : placement supposé de la progression, puis utiles et remplissage ; nouvel essai (20 au plus) quand l'ordre tiré bloque, monde remis en état sur erreur ; faisabilité avant et après avec clause ou entrée qui bloque ; mélange maison sur `rand_core` 0.10 (ChaCha épinglé). 11 tests dont 1 000 seeds sur 2 joueurs ; workspace, clippy et fmt passent.
- **Pas vérifié** : performance sur un gros multiworld (un balayage complet par objet de progression) ; restrictions d'objets par lieu (`item_rule`) absentes du modèle.
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : étape 4 de l'ADR 0027 : crate `generate` + APQuest en Rust, branché sur `RustBackend` (sur option).

### 2026-11-18 · V2 étape 4 : `generate` + APQuest en Rust, moteur Rust dans le launcher
- **Fait** : crate `generate` (`6eee3f3`) : APQuest porté (notice MIT), datapackage et `checksum` identiques à l'officiel, seed `.archipelago` comparée à 4 seeds du vrai 0.6.8 (tout sauf le placement), 16 combinaisons × 60 seeds jouables et reproductibles, seed chargée par le serveur officiel. YAML, `RustBackend` et case « Moteur Rust (expérimental) » (`4da4bb1`) : 2 joueurs générés et hébergés par la macro sans Python installé. Workspace, clippy et fmt passent.
- **Pas vérifié** : partie jouée avec le vrai client APQuest sur une seed Rust ; seed Rust hébergée par `MultiServer` et comparée sur `Connect` (banc différentiel) ; spoiler et `meta.yaml` ignorés par le moteur Rust.
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : étape 5 de l'ADR 0027 : équilibrage de progression (`progression_balancing`).

### 2026-11-19 · V2 étape 5 : équilibrage de progression
- **Fait** : `fill::balance` (d'après `Fill.py` 0.6.8, parts comparées sans virgule), `progression_balancing` et `meta.yaml` (préréglage « Long ») compris par le moteur Rust (ADR 0027, étape 5). Bug trouvé : la description des préréglages (`Raftup : partie…`) n'était pas du YAML valide, même pour PyYAML ; corrigée. Effet mesuré sur 200 seeds. `cargo test --workspace` et clippy passent.
- **Pas vérifié** : comparaison avec le vrai `balance_multiworld_progression` (algorithme relu, pas rejoué) ; partie jouée avec le vrai client APQuest sur une seed Rust ; préréglage « Rapide » (`accessibility: minimal`) toujours au Python.
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : jouer une vraie partie APQuest sur une seed Rust, puis étape 6 (export des règles depuis un apworld, ADR à part).

### 2026-11-19 · ADR de l'export des mondes Python
- **Fait** : ADR 0028 rédigé (`5a1fb05`) puis accepté tel que recommandé : export en deux appels au worker, sondage des lambdas, contrôle croisé sur 64 inventaires, repli Python pour toute règle opaque, `progress_type` ajouté, `item_rule` pas encore. Aucun code.
- **Pas vérifié** : sondage jamais essayé sur une vraie lambda ; durée de l'aller-retour Python.
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : ADR 0028, étape 1 : `export_worlds` sur APQuest, relu en `logic::MultiWorld` et comparé au portage Rust.

### 2026-11-19 · ADR 0028, étape 1 : export des mondes
- **Fait** : méthode `export_worlds` du worker (arrêt avant le placement d'Archipelago, module `raftup_export.py` envoyé en texte), `generate::imported` (JSON lu comme une donnée hostile, `rule_builder` traduit, `checksum` vérifié), `generate::build`. En vrai : APQuest exporté identique au portage Rust (16 combinaisons, 2 joueurs). Faux Archipelago des tests Python réparé (`settings.py` manquait depuis `b14af43`). Workspace, clippy, 30 tests Python passent.
- **Pas vérifié** : un monde à lambdas (toutes opaques pour l'instant) ; reproductibilité de l'export (`PYTHONHASHSEED` sans effet avec `-I`).
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : ADR 0028, étape 2 : sondeur de lambdas et contrôle croisé, sur un monde de test du dépôt.

### 2026-11-19 · ADR 0028, étape 2 : sondage des lambdas
- **Fait** : sondeur (`probe`, arbre de décision, monotonie vérifiée exactement, plafonds), contrôle croisé de toutes les règles sur 64 inventaires du vrai Archipelago (vérifié côté Rust), refus des mondes à `collect` personnalisé. Monde de test `raftup_sonde` : en vrai, 9 lambdas sondées, contrôle réussi, règle non monotone refusée. 36 tests Python, 14 tests d'import Rust, 4 tests réels ; workspace et clippy passent.
- **Pas vérifié** : un vrai monde tiers à lambdas (Slime Rancher, étape 5) ; durée du sondage sur un gros monde.
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : ADR 0028, étape 3 : `progress_type` (lieux exclus et prioritaires) dans `logic` et `fill`.

### 2026-11-19 · ADR 0028, étape 3 : lieux prioritaires et exclus
- **Fait** : `logic::Progress` et `set_progress` ; `fill` place la progression d'abord dans les lieux prioritaires, jamais dans les exclus, met du remplissage dans les exclus (`TooManyExcluded` sinon) ; l'équilibrage ne touche pas aux prioritaires et ne remplit pas les exclus. Import de `progress_type`. 4 tests de `fill` (300 seeds), test d'import, monde `raftup_sonde` vérifié en vrai. Workspace et clippy passent.
- **Pas vérifié** : écart assumé avec la passe « Priority » séparée d'Archipelago (objets déprioritisés non distingués), mesuré seulement sur le monde de test.
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : ADR 0028, étape 4 : `finish_worlds` et branchement ; d'abord faire valider qui écrit la seed (Archipelago ou Rust) et le réglage de `PYTHONHASHSEED`.

### 2026-11-19 · ADR 0028, étape 4 : génération complète par export
- **Fait** : choix validés (Archipelago écrit la seed ; `-s -P` + `PYTHONHASHSEED=0`, ADR 0016 révisé) ; `finish_worlds` (Main.main en attente dans un thread, placement Raftup posé), `fill_imported`, `ExportBackend` avec repli Python, moteur « export » dans l'interface. En vrai : seed 2 joueurs placée par Raftup, écrite par Archipelago, reproductible ; repli vérifié. 39 tests Python, tests réels py-bridge, export et génération.
- **Pas vérifié** : un vrai monde tiers par ce chemin (Slime Rancher) ; écran « Créer une partie » regardé avec le moteur export ; durée totale comparée au Python seul.
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : ADR 0028, étape 5 : Slime Rancher exporté en vrai (apworld installée par la Bibliothèque), puis une partie.

### 2026-11-19 · ADR 0028, étape 5 : Slime Rancher par le moteur export
- **Fait** : sondeur qui lit la vérité d'un résultat entier (valeur d'option), comme Archipelago : 147 règles de Slime Rancher traduites, 0 opaque, contrôle croisé réussi. Seed complète (seed 11) identique à celle d'Archipelago seul sauf le placement, même `slot_data` ; 2,9 s contre 2,8 s. Tests réels `slime_rancher_exporte` et `slime_rancher_genere_par_export` (`RAFTUP_SLIME_APWORLD`). 39 tests Python, workspace et clippy passent.
- **Pas vérifié** : une vraie partie de Slime Rancher sur une seed placée par Raftup ; autres options du YAML de Slime Rancher (seulement les valeurs par défaut).
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : jouer une partie de Slime Rancher générée avec la case « Moteur Rust » (à faire par l'utilisatrice, devant le jeu).

### 2026-11-19 · Slime Rancher en jeu, piloté par macro (seed placée par Raftup)
- **Fait** : launcher en macro (`RAFTUP_DEV_CAPTURE`, `RAFTUP_DEV_RUST=1`, port 38281) : moteur « export » choisi seul, placement Raftup réussi (fichiers de travail supprimés, pas de repli), partie hébergée en `ws`. Jeu lancé et piloté par un script PowerShell (`SendInput`, captures d'écran ; pas de computer-use) : menu Archipelago de Slimipelago prérempli par `ApConnection.json`, connexion acceptée, nouvelle partie « Archipelago » avec l'`ap_uuid` du `slot_data`, objet de départ reçu en jeu (« Region Unlock: Mochi's Manor », from Server).
- **Pas vérifié** : aucun lieu validé en jeu (achat d'amélioration au terminal du ranch non trouvé à l'aveugle) ; la borne à droite du marché ne s'ouvre pas avec E.
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : valider un lieu en jeu (acheter « Heart Module lv.1 » : +75 newbucks attendus d'après le spoiler) et vérifier le compteur du launcher.

### 2026-11-19 · Hub `raftup-hub` : comptes, amis, présence (ADR 0033)
- **Fait** : ADR 0033 (axum 0.8, sqlx 0.8 + SQLite, PostgreSQL prévu dans `store.rs` seul, jetons opaques hachés SHA-256 révocables) ; crate `crates/hub` en AGPL-3.0-or-later, binaire `raftup-hub` ; inscription et connexion e-mail + mot de passe argon2id, pseudo unique (NFKC + minuscules), demandes d'ami, présence en mémoire avec expiration, limites de tentatives. 32 tests (API, NF09 sur base et journal, ADR 0003 : aucune crate n'en dépend) ; licences des dépendances vérifiées (toutes permissives).
- **Pas vérifié** : PostgreSQL (rien d'écrit) ; binaire seulement essayé en local (inscription, connexion) ; `LICENSE-AGPL` écrit sans téléchargement, à comparer avec le texte de gnu.org. Pas de Discord, de vérification d'e-mail, de blocage ni de RGPD (listés dans l'ADR).
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : relire l'ADR 0033 ; client du hub dans `launcher-core` (jeton dans `keyring`) et panneau d'amis.

### 2026-11-19 · Registre de paquets signés (session `registre`, ADR 0032)
- **Fait** : ADR 0032 (accepté en autonomie, à relire) ; crate `registry` (MIT OR Apache-2.0) : `index.json` versionné, trié, une version par paquet, URL + sha256 + taille + licence + auteurs (rien n'est hébergé) ; signature minisign vérifiée avant lecture ; binaire `raftup-registry` (`keygen`, `publish`, `verify`) qui écrit en local et n'envoie rien. `launcher-core::registry` : index et paquets téléchargés bornés et vérifiés, profils du registre ajoutés au catalogue sans jamais remplacer l'embarqué. 21 tests `registry`, 10 `launcher-core` ; workspace, clippy et fmt passent.
- **Pas vérifié** : vraie clé du registre (`REGISTRY_PUBLIC_KEY` vide : tout est refusé), adresse de publication, binaire jamais lancé sur une vraie apworld ; rien dans l'interface (Bibliothèque, installation d'une apworld du registre).
- **Ne pas toucher** : `docs/profils-en-attente/`.
- **Suite** : générer la clé (`raftup-registry keygen`, hors du dépôt), écrire le `registre.toml` source, brancher le registre dans la Bibliothèque.

### 2026-11-19 · NF11 / NF12 : CI multi-OS et empreinte de seed
- **Fait** : `ci.yml` en matrice Windows, macOS, Linux (`fail-fast: false`, en-têtes X11/Wayland sous Linux) ; test `generate/tests/empreinte.rs` : sha256 du `.archipelago` d'une seed APQuest à 2 joueurs, valeur écrite dans le test, mise à jour expliquée en tête. Relecture des `cfg(windows)` : rfd ne compilait pas sous Linux (aucun moteur de boîte de fichiers) → `xdg-portal` ; `detect::is_safe_relative` acceptait `..\x.exe` hors Windows → `\` et `:` refusés partout. Runtime Python documenté Windows seulement (repli : moteur Rust). fmt, clippy et workspace passent sous Windows.
- **Pas vérifié** : aucune compilation ni test sous macOS ou Linux (cibles non installées, pas de push) ; empreinte identique sur les trois OS pas encore prouvée.
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : pousser la branche pour lancer la CI, corriger ce qui casse sous macOS et Linux.

### 2026-11-19 · Bac à sable du générateur (ADR 0034)
- **Fait** : ADR 0034 (accepté en autonomie, à relire). Sous Windows, le worker est lancé en intégrité basse : copie `python-raftup-low.exe` étiquetée par `icacls`, écriture permise seulement dans `generations/<horodatage>/` (TEMP compris), remis à « moyen » après l'arrêt ; le worker annonce son niveau (`whoami /groups`) et le pont refuse un worker qui n'est pas en intégrité basse. Activé par défaut (`PythonBackend::sandbox`), aussi pour le moteur export. Sans `unsafe`, sans nouvelle crate. Tests `py-bridge/tests/sandbox.rs` (faux worker vraiment lancé en intégrité basse) ; tests réels de génération (5) et d'export (8, dont Slime Rancher) passent avec le bac à sable ; 39 tests Python.
- **Pas vérifié** : repli hors Windows (rien appliqué, seulement écrit et relu) ; disque sans étiquettes (FAT32, réseau : la génération échoue) ; runtime neuf (`.pyc` jamais écrits par le worker, coût non mesuré) ; durée comparée à avant (export réel : 846 s pour 8 tests, sans mesure avant). Builds dans un `target` à part (`agents-v2-bac-a-sable`, puis `v2-bac-a-sable`) : le `target` commun mélangeait les worktrees.
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : relire l'ADR 0034 ; plus tard, AppContainer (lecture et réseau) et bac à sable Linux/macOS.

### 2026-11-19 · Règles d'objet, objets à placer tôt et accessibilité (ADR 0029, session early-items)
- **Fait** : ADR 0029 (accepté en autonomie, à relire). `logic` : `ItemRule` (arène), `can_hold`, `Accessibility`, `must_reach` ; `fill` : `fill_with_early` (sphère 0 d'après `distribute_early_items`), règles d'objet respectées au placement et à l'équilibrage, `minimal` / `items` ; export format 2 : règles d'objet en listes calculées par Archipelago, contrôle croisé `can_fill`, `early_items`, `accessibility`. Monde `raftup_sonde` : options `early_items`, `item_rules`. Tests `fill` (8), import, Python (41), réels `export_real` (2 nouveaux).
- **Pas vérifié** : un monde tiers qui pose des règles d'objet ou des objets tôt ; gros multiworld (coût du calcul en Python) ; pas d'échange façon `remaining_fill` (échec ⇒ repli Python).
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : relecture de l'ADR 0029 par l'utilisatrice ; essai sur un apworld qui se sert de `local_items` ou d'`early_items`.

### 2026-11-19 · Finitions du moteur (session `moteur-finitions`)
- **Fait** : spoiler `AP_<seed>_Spoiler.txt` dans le zip du moteur Rust seul (`generate::spoiler` : en-tête, options, placement, parcours par sphères ; niveau 0 sans spoiler, 1 sans parcours) ; `GeneratorBackend::report` (`EngineReport`) : l'écran dit à la fin quel moteur a vraiment généré ; raison du repli du moteur export affichée en direct et à la fin (avant le message d'erreur). Tests `generate`, `launcher-core`, `launcher-ui` ; workspace et clippy passent.
- **Vu sur captures** (macro, port 38296 occupé) : APQuest « Moteur : Raftup seul » ; Slime Rancher + APQuest export « Archipelago a construit les mondes, Raftup a placé les objets » ; `local_items` → « Archipelago seul » + raison du repli en orange.
- **Pas vérifié** : test réel `repli_sur_archipelago_seul` (assertion du rapport ajoutée, non lancé) ; spoiler non relu par un outil tiers ; `CARGO_TARGET_DIR` partagé (`agents-v2`) inutilisable en parallèle (rlib d'une autre worktree liée) : construit dans `target/agents-v2-finitions`. `ignore_eof_shutdown_et_drop_rapides` (py-bridge, non touché) a échoué une fois sous charge, passe seul.
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : écrire dans le spoiler la version des apworlds (ADR 0028, § reproductibilité).

### 2026-11-19 · Durée estimée et équilibrage par temps (ADR 0030)
- **Fait** : ADR 0030 (accepté en autonomie, à relire) ; champ facultatif `game.seconds_per_location` des profils (Slime Rancher : 240 s, défaut 120 s, `format_version` inchangé) ; `launcher_core::estimate` (temps de jeu et fin au plus tard par sphères) ; panneau « Durée estimée » (`screens/estimate.rs`, une ligne dans `create.rs`) ; `fill::time_balance` et `simulate`, désactivé par défaut, 9 tests sur des mondes écrits à la main.
- **Pas vérifié** : temps par lieu jamais mesuré en vraie partie ; équilibrage par temps branché nulle part (ni `generate` ni interface) ; coût sur une grosse partie. Builds dans `target/agents-v2-duree` : le `target/agents-v2` partagé entre worktrees donnait un `fill` d'une autre branche.
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : brancher `time_balance` derrière une option (case ou option YAML), afficher l'estimation dans « Partie en cours ».

### 2026-11-19 · V2, suivi async (ADR 0031)
- **Fait** : ADR 0031 (accepté en autonomie, à relire). Option de salle `with_async_follow` (désactivée par défaut, `--async_follow`, toujours active dans le launcher) : `!away`, `!back`, `!summary`, `!vote_release` (majorité stricte des autres joueurs, passe outre `release_mode`), résumé de retour en `PrintJSON` après le tutoriel, réponse automatique quand le chat cite un absent, clés `_read_raftup_away_<équipe>` / `_read_raftup_release_votes_<équipe>`, fil d'activité borné. `.raftup` en `format_version` 2 (version 1 toujours lue, réécrite au compactage). Événements `Away`, `Back`, `ReleaseVote` vers le webhook et « Partie en cours ». 8 tests serveur, tests `multidata`, webhook et format ; workspace, clippy et fmt passent.
- **Pas vérifié** : tests différentiels contre le vrai serveur Python non relancés (comportement inchangé sans l'option, captures rejouées vertes) ; écran regardé à l'œil ; vrai mod qui envoie `!away` ; vrai webhook.
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : faire relire l'ADR 0031 (noms des commandes, vote qui passe outre `release_mode`) ; regarder « Partie en cours » avec un absent et un vote.

### 2026-11-19 · Fusion des 8 branches V2
- **Fait** : 8 branches d'agents fusionnées dans `main` (CI multi-OS, finitions du moteur, ADR 0029 à 0034) ; conflits du journal, de l'état, de `Cargo.toml`, d'`export.rs` et de `launcher-core/src/lib.rs` résolus. Sur le code fusionné : fmt, clippy `-D warnings`, 95 suites de tests, 41 tests Python, 15 tests réels (génération, export, Slime Rancher) passent. Repli du bac à sable ajouté : pose impossible (FAT32, réseau) ⇒ worker sans bac à sable, dit dans le journal (ADR 0034 révisé, 2 tests). `main` avancé en `2964870` ; copies de travail et branches des agents supprimées (plus 6 anciennes déjà fusionnées).
- **Pas vérifié** : CI sous macOS et Linux (rien poussé) ; `crates/hub/LICENSE-AGPL` écrit de mémoire par l'agent, à remplacer par le texte officiel de gnu.org ; les ADR 0029 à 0034 sont « acceptés en autonomie », à relire par l'utilisatrice.
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago).
- **Suite** : remplacer le texte de l'AGPL ; essais en jeu quand l'utilisatrice sera devant le PC.

### 2026-11-19 · Équilibrage par temps branché (session `v2-time-balance`, ADR 0030 révisé)
- **Fait** : case « Équilibrer par temps de jeu » sous « Moteur Rust » (décochée par défaut, grisée sans lui) ; `generate::GenerateRequest::time_balance` et `imported::fill_imported_with` (moteurs Rust et export), secondes par slot depuis les profils embarqués, ligne de journal avant → après ; échange refusé s'il viole une règle d'objet (`can_hold`). Tests : désactivé = même seed (empreinte inchangée), activé = reproductible et utile, chemin export, réglages des profils, moteur du launcher. Workspace, clippy et fmt passent.
- **Pas vérifié** : temps par lieu jamais mesuré en vraie partie ; coût sur une grosse partie ; case jamais regardée à l'écran (macro : `RAFTUP_DEV_TEMPS=1`) ; moteur export avec l'option jamais lancé en réel (test unitaire seulement) ; profils du registre pas pris en compte.
- **Ne pas toucher** : `docs/profils-en-attente/` ; `launcher-core::registry`, Bibliothèque, `crates/hub` (autres sessions).
- **Suite** : mesurer le temps par lieu en vraie partie (Slime Rancher, APQuest) et corriger les profils.

### 2026-11-19 · Registre de paquets signés terminé (session `v2-registre`)
- **Fait** : vraie clé du registre (`A3D76747B9BA50E2`, secrète dans le dossier des clés hors dépôt) ; `registre/registre.toml` (apworld Slime Rancher, références chez l'auteur) et procédure `registre/README.md` ; `publish` + `verify` lancés pour de vrai avec la clé, sha256 identique au profil ; `fetch_registry`, `apply_registry`, `install_apworld` ; Bibliothèque : jeux du registre marqués, « Mondes du registre », échec discret (ADR 0032 mis à jour). Workspace, clippy, fmt passent ; captures regardées (registre local signé, et 404 réel).
- **Pas vérifié** : clic « Installer » dans la vraie fenêtre (fonction testée seule) ; release GitHub `registre` pas créée (rien poussé) : à faire par l'utilisatrice.
- **Ne pas toucher** : `crates/hub`, `fill`, `generate`, écran « Créer une partie » (autres sessions V2).
- **Suite** : créer la release `registre` et y envoyer `index.json` + `index.json.minisig` (voir `registre/README.md`).

### 2026-11-19 · Hub côté serveur (session `v2-hub-serveur`)
- **Fait** : ADR 0033 complété (ajouts acceptés en autonomie, à relire). PostgreSQL par `sqlx::Any`, même SQL que SQLite (`a600c7b`) ; blocages (`171e060`) ; RGPD, export JSON et suppression du compte (`1001185`) ; e-mails de vérification et mot de passe oublié, trait `Mailer` + SMTP `lettre`, jetons à usage unique hachés (`9058792`) ; connexion Discord OAuth 2.0 + PKCE, trait `DiscordApi` (`1e51a38`). Schéma en version 4 avec migrations. Tests du hub passés sur SQLite et sur un PostgreSQL 18 jetable (`RAFTUP_HUB_TEST_POSTGRES`) ; workspace, clippy et fmt passent.
- **Pas vérifié** : vrai fournisseur SMTP (seulement un faux serveur SMTP local) ; vraie application Discord (seulement `FakeDiscord`) ; hub lancé à la main ; aucune route existante renommée, mais le client du launcher n'a pas été essayé contre ces ajouts.
- **Ne pas toucher** : `docs/profils-en-attente/` (attend l'autorisation de l'auteur de Slimipelago) ; client du hub et panneau d'amis (session `v2-hub-client`).
- **Suite** : relire les ajouts de l'ADR 0033, puis essayer avec une vraie application Discord et un vrai SMTP.

### 2026-11-19 · Client du hub et écran « Amis » (session `v2-hub-client`, ADR 0035)
- **Fait** : ADR 0035 (accepté en autonomie, à relire). `launcher_core::hub_client` : client `ureq` de toute l'API `/v1`, types redéfinis (aucune dépendance au hub AGPL), erreurs typées avec le code du hub, jeton par adresse de hub dans le trousseau (`keyring` 3.6), adresse par défaut `http://127.0.0.1:8787` (`hub_url` dans `config.toml`). Écran « Amis » dans `launcher-ui` (connexion, inscription, présence, demandes, ajout par pseudo, adresse du hub), appels dans un thread, présence toutes les 60 s ; cartes de la barre branchées. 14 tests contre un faux serveur, scénario réel contre `raftup-hub` (`#[ignore]`) et trousseau Windows réel passent ; vu sur captures (scénario `amis`, connecté et déconnecté). Workspace, clippy et fmt passent.
- **Pas vérifié** : trousseau macOS et Linux (keyutils) jamais essayés ; écran jamais cliqué à la main (macro seulement) ; hub injoignable regardé seulement par test ; présence « en partie » pas encore envoyée.
- **Ne pas toucher** : `crates/hub` (session hub-serveur en parallèle) ; `docs/profils-en-attente/`.
- **Suite** : envoyer la présence « en partie » au lancement d'un jeu (`launch`), puis l'adresse officielle du hub au déploiement.

### 2026-11-19 · Fusion des 4 chantiers V2 dans `main`
- **Fait** : adresse du registre fixée (`REGISTRY_INDEX_URL`, release GitHub `registre`) ; fusion de `v2-time-balance`, `v2-registre`, `v2-hub-serveur`, `v2-hub-client` ; conflits seulement dans le journal, l'état du projet et `Cargo.lock` (régénéré). Sur `main` : fmt, clippy `-D warnings` et `cargo test --workspace` passent.
- **Pas vérifié** : rien regardé à l'écran après la fusion ; vrai SMTP, vraie application Discord, release `registre` ; rien poussé.
- **Ne pas toucher** : `docs/profils-en-attente/`.
- **Suite** : créer la release `registre` ; relire les ADR 0029 à 0035 ; essais réels de la V2.

### 2026-11-19 · Launcher relié à Rafthub (ADR 0036, côté launcher)
- **Fait** : module `launcher_core::rafthub` (adoption unique de `rafthub.toml`, liens `raftup://hub/<hôte>`), boîte « Passer sur le hub … ? » et hôte du Rafthub dans « Amis » ; bandeau effacé par la reprise de session corrigé (trouvé en capture, test ajouté) ; tests, clippy, fmt et captures (`changer-de-hub`, lien refusé) passent.
- **Pas vérifié** : vrai clic sur un lien `raftup://hub/…` depuis un navigateur (schéma enregistré en release seulement) ; changement de hub contre deux vrais hubs.
- **Ne pas toucher** : `crates/hub` (sessions rafthub-socle et rafthub-donnees) ; `docs/profils-en-attente/`.
- **Suite** : côté hub, servir le zip `GET /telecharger/raftup-windows.zip` avec `rafthub.toml`.

### 2026-11-19 · Socle de la console d'admin Rafthub (session `rafthub-socle`, ADR 0036)
- **Fait** : binaire `rafthub` (+ `admin create|remove <e-mail>`) ; console `/admin` en askama (menu, connexion, tableau de bord avec vrais chiffres et état base/SMTP/Discord, pages « Bientôt »), CSS et polices servies par le hub ; schéma v5 (`users.role`, `admin_sessions`) ; cookie haché 12 h, anti-CSRF, essais limités, Bearer refusé. 10 tests `tests/admin.rs` ; workspace, clippy, fmt passent ; connexion, tableau de bord et déconnexion faits dans le vrai navigateur (`5fc7209`).
- **Pas vérifié** : PostgreSQL (seulement SQLite cette fois) ; vraie installation derrière un proxy HTTPS ; fusion avec `rafthub-donnees`, qui prend sans doute aussi la version 5 du schéma (renuméroter, voir la note de groupe `rafthub-socle.md`).
- **Ne pas toucher** : `launcher-core` / `launcher-ui` (session `rafthub-launcher`) ; `"service": "raftup-hub"` de `GET /v1` et le service du trousseau, que le launcher teste.
- **Suite** : après la fusion, pages Joueurs puis Réglages sur les données de `rafthub-donnees`.

### 2026-11-19 · Rafthub, couche données de la console (session `rafthub-donnees`, ADR 0036)
- **Fait** : schéma en version 5 ; suspension (sessions fermées, connexion refusée par mot de passe et Discord, caché des autres joueurs) ; réglages en base > environnement > défaut, secrets jamais relus, SMTP et Discord relus à chaque usage (`AppState::from_settings`) ; mode d'inscription `open` / `invite` / `closed` et codes d'invitation ; journal `admin_events` ; liste des joueurs et compteurs (`console.rs`, `store/admin.rs`, `settings.rs`). ADR 0036 précisé. Tests du hub sur SQLite et sur un PostgreSQL 18 jetable ; workspace (`--no-fail-fast`), clippy et fmt passent.
- **Pas vérifié** : migration 4 → 5 essayée sur SQLite seulement ; aucune page HTML (session `rafthub-socle`) ; vrai SMTP et vraie application Discord ; le launcher n'envoie pas encore de code d'invitation. `launcher-core/tests/host.rs` (`sauvegarde_puis_reprise`) échoue si deux `cargo test` tournent en même temps (dossier fixe).
- **Ne pas toucher** : `docs/profils-en-attente/` ; pages `/admin`, `users.role`, `admin_sessions` (session `rafthub-socle`) ; launcher (session `rafthub-launcher`).
- **Suite** : fusion avec `rafthub-socle` (renuméroter sa version de schéma en 6) puis brancher les pages sur `console.rs`.

### 2026-11-20 · Fusion des 3 branches Rafthub dans `main` (ADR 0036)
- **Fait** : fusion de `rafthub-launcher`, `rafthub-socle`, `rafthub-donnees` ; schéma renuméroté (5 : `users.role`, 6 : `users.suspended_at`, `SCHEMA_VERSION` = 6) ; `store/admin.rs` du socle renommé `store/admin_auth.rs` (rôles, sessions d'admin), un seul `dashboard_counts(now)` ; tableau de bord relit SMTP, Discord et adresse publique dans les réglages (`AppState::mailer` / `discord` en `pub(crate)`). Sur `main` : fmt, clippy `-D warnings` et `cargo test --workspace` passent.
- **Pas vérifié** : console jamais ouverte dans un navigateur ; PostgreSQL ; migration d'une base réelle de la version 4 à 6 (seulement par les tests).
- **Ne pas toucher** : `docs/profils-en-attente/` ; le stash « retouches hub trouvées dans main avant fusion Rafthub » (non commité, inutile à la compilation, à jeter sur accord de l'utilisatrice).
- **Suite** : pages Joueurs, Distribution du launcher (zip + `rafthub.toml`), Réglages et Journaux de la console.

### 2026-11-20 · Rafthub, page « Joueurs » de la console (session `rafthub-joueurs`, ADR 0036)
- **Fait** : `web/players.rs` + `templates/admin/players.html` remplacent le bouchon : liste paginée (25 par page), e-mail masqué, date de création, vérifié, suspendu, présence ; recherche et filtres ; `POST` suspendre / réactiver / créer et révoquer un code d'invitation (bloc visible en mode `invite`, code montré une seule fois), tous avec CSRF et journal `admin_events` via `console.rs`. Section « Page Joueurs » ajoutée à `rafthub.css` (avant « Connexion »). Tests `tests/admin_players.rs` ; fmt, clippy workspace et `cargo test -p hub` passent.
- **Pas vérifié** : page jamais ouverte dans un navigateur ; PostgreSQL ; export et suppression RGPD depuis la page (pas demandés).
- **Ne pas toucher** : `docs/profils-en-attente/` ; dans `web/mod.rs` et `soon.rs`, les lignes des autres pages (sessions parallèles). La ligne de doc `GET /admin/joueurs, …` en tête de `web/mod.rs` reste à mettre à jour à la fusion.
- **Suite** : fusion avec les sessions Distribution, Réglages et Journaux.

### 2026-11-20 · Page « Journaux » de la console Rafthub (session `rafthub-journaux`, ADR 0036)
- **Fait** : `GET /admin/journaux` (`web/logs.rs`, `admin/logs.html`) : `admin_events` du plus récent au plus ancien (date UTC lisible, acteur, type en français, cible, détail repassé dans `clean_detail`), filtres GET sans JS (type, pseudo acteur ou cible, période `du`/`au` incluse), 50 lignes par page ; `Store::list_events_filtered` + `EventFilter` (ordre date puis id) ; bouchon `logs` retiré de `soon.rs` ; quelques règles CSS « Journaux ». 4 tests `tests/admin_logs.rs` + 4 tests unitaires ; fmt, clippy `-D warnings`, `cargo test -p hub` passent.
- **Pas vérifié** : page jamais ouverte dans un navigateur ; PostgreSQL (SQLite seulement) ; heures en UTC, pas dans le fuseau de l'admin.
- **Ne pas toucher** : `docs/profils-en-attente/` ; journaux techniques `tracing` (pas décidé, ADR 0036).
- **Suite** : fusion avec les sessions Joueurs, Distribution, Réglages (conflits attendus dans `web/mod.rs`, `soon.rs`, `rafthub.css`, `store.rs`).

### 2026-11-20 · Page « Réglages » de la console Rafthub (session `rafthub-reglages`, ADR 0036)
- **Fait** : `/admin/reglages` remplace le bouchon (`web/settings.rs`, `settings.html`, CSS ajouté en fin de `rafthub.css`) : un formulaire POST + CSRF par groupe (général, inscriptions, e-mail, Discord, page publique), source affichée (base, environnement, défaut), « Revenir au défaut », secrets jamais réaffichés (hôte SMTP seul, champ vide = rien ne change), groupe vérifié en entier avant d'écrire, valeur inchangée non réécrite, e-mail de test à l'admin connecté. 7 tests dans `tests/settings_page.rs` ; fmt, clippy `-D warnings` et `cargo test -p hub` passent.
- **Pas vérifié** : page jamais ouverte dans un navigateur ; vrai serveur SMTP pour l'e-mail de test ; PostgreSQL.
- **Ne pas toucher** : `docs/profils-en-attente/` ; pages Joueurs, Distribution, Journaux (autres sessions) ; le tableau de la doc de `web/mod.rs` cite encore `/admin/reglages` comme « Bientôt » (laissé pour éviter un conflit, à corriger à la fusion).
- **Suite** : fusionner les quatre pages dans `main`, puis ouvrir la console dans un navigateur.

### 2026-11-20 · Rafthub, distribution du launcher (session `rafthub-distribution`, ADR 0036)
- **Fait** : `hub::distribution` (manifeste ADR 0022, liste blanche, sha256 + signature minisign, cache disque revérifié à chaque service, zip exe + `rafthub.toml`, pas de retour en arrière) ; page `/admin/distribution` (état du cache, « Rafraîchir » en POST + CSRF, lien `raftup://hub/…` à copier), page publique `/telecharger`, zips public et admin ; `copy.js` ; clé publique recopiée de `launcher-core` (test d'égalité) plutôt qu'une dépendance, consigné dans l'ADR 0036. 12 tests `tests/distribution.rs` sans réseau (clé générée dans le test) ; fmt, clippy `-D warnings` et `cargo test --workspace` passent.
- **Pas vérifié** : vrai téléchargement depuis la release GitHub (aucun appel réseau fait) ; pages jamais ouvertes dans un navigateur ; zip jamais décompressé à côté d'un vrai launcher.
- **Ne pas toucher** : `docs/profils-en-attente/` ; pages Joueurs, Réglages, Journaux (sessions parallèles).
- **Suite** : publier une release avec `update.json`, puis essayer « Rafraîchir » sur un vrai hub et lancer l'exe du zip.

### 2026-11-20 · Fusion des 4 pages de la console Rafthub dans `main` (ADR 0036)
- **Fait** : fusion de `rafthub-joueurs`, `rafthub-journaux`, `rafthub-reglages`, `rafthub-distribution` ; conflits dans `web/mod.rs` (routes des quatre pages), `rafthub.css` et le journal (les deux côtés gardés) ; tous les bouchons retirés, `mod soon` enlevé du routeur. Sur `main` : fmt, clippy `-D warnings` et `cargo test --workspace` passent.
- **Pas vérifié** : console jamais ouverte dans un navigateur après fusion ; PostgreSQL pour les nouvelles pages ; vrai téléchargement GitHub.
- **Ne pas toucher** : `docs/profils-en-attente/` ; le stash « retouches hub… » (à jeter sur accord) ; `web/soon.rs` et `templates/admin/soon.html` ne servent plus (à supprimer sur accord).
- **Suite** : ouvrir la console dans un navigateur et la parcourir ; champ « code d'invitation » dans le launcher ; déploiement.

### 2026-11-20 · Rafthub : base intégrée et premier admin depuis le navigateur (ADR 0037)
- **Fait** : console parcourue dans le navigateur après la fusion (les 6 pages, suspension, réglage, journal). Sans `--db`, base SQLite dans `rafthub-data/rafthub.db` à côté de l'exe, cache du launcher dans `rafthub-data/cache` ; sans admin, code d'installation au journal et page `/admin/installation` (`web/setup.rs`, `setup.html`, `Store::has_admin`, `HubConfig::setup_code`), `/admin/login` y renvoie ; 5 tests dans `tests/admin.rs` ; essayé dans le navigateur sur un hub neuf. fmt, clippy `-D warnings`, `cargo test --workspace` passent.
- **Pas vérifié** : exe lancé depuis un autre dossier que `target/debug` ; PostgreSQL avec la page d'installation ; affichage sur grand écran.
- **Ne pas toucher** : `docs/profils-en-attente/` ; stash « retouches hub… » et `web/soon.rs` + `soon.html` (à jeter sur accord).
- **Suite** : champ « code d'invitation » dans le launcher ; vrai téléchargement depuis la release GitHub ; déploiement.

### 2026-11-20 · Rafthub : assistant de premier lancement, choix de la base (ADR 0037)
- **Fait** : sans `--db` ni `rafthub-data/rafthub.toml`, le hub ne sert que l'assistant `/admin/installation` (`web/install.rs`, `install.html`) : base intégrée SQLite ou externe PostgreSQL (MySQL grisé), essayée puis gardée dans `rafthub.toml` (`server_config.rs`, chemin SQLite relatif) ; le vrai hub redémarre sur le même port et enchaîne sur le premier admin, même code. CSS et JS en `no-cache` (ancienne CSS vue dans le navigateur). 5 tests `tests/install.rs` + 4 unitaires ; parcours complet fait dans le navigateur sur un hub neuf. fmt, clippy `-D warnings`, `cargo test --workspace` passent.
- **Pas vérifié** : une vraie base PostgreSQL par l'assistant (seulement le refus d'un port fermé) ; exe lancé hors de `target/debug`.
- **Ne pas toucher** : `docs/profils-en-attente/` ; stash « retouches hub… », `web/soon.rs` + `soon.html` (à jeter sur accord).
- **Suite** : essayer l'assistant avec un vrai PostgreSQL ; champ « code d'invitation » dans le launcher ; déploiement.

### 2026-11-20 · Rafthub : MySQL et MariaDB (ADR 0037)
- **Fait** : `Backend::Mysql` (sqlx `mysql`), requêtes traduites par `Store::q` (`$n` -> `?`), paramètres numérotés dans l'ordre partout (test `parametres_dans_l_ordre`), formes MySQL de `RETURNING` / `ON CONFLICT`, clés étrangères en fin de table pour les trois bases, schéma MySQL (`VARCHAR`, InnoDB `utf8mb4_unicode_ci`, sans `CHECK`) ; assistant : PostgreSQL, MySQL, MariaDB ; `RAFTUP_HUB_TEST_MYSQL`. Toute la suite du hub passe sur MariaDB 11.4 et MySQL 8.4 (conteneurs Docker `raftup-test-mariadb`, `raftup-test-mysql`, ports 33061 et 33062 sur 127.0.0.1) ; assistant + premier admin faits dans le navigateur sur MariaDB. fmt, clippy, `cargo test --workspace` passent.
- **Vérifié ensuite** : toute la suite du hub passe aussi sur PostgreSQL 18 (conteneur `raftup-test-postgres`, port 33063).
- **Pas vérifié** : base PostgreSQL existante créée avec l'ancien schéma (même sens, non rejoué).
- **Ne pas toucher** : `docs/profils-en-attente/` ; stash « retouches hub… », `web/soon.rs` + `soon.html` (à jeter sur accord) ; conteneurs de test (à supprimer sur accord).
- **Suite** : champ « code d'invitation » dans le launcher (session `launcher-invitation`) ; déploiement.

### 2026-11-20 · Rafthub : confirmation du mot de passe admin à l'installation (ADR 0037)
- **Fait** : étape 2 de l'installation, champ « Confirmer le mot de passe » (refus si différent, `Password::same_as`), rappel « 8 caractères au moins » ; test `installation_refuse_deux_mots_de_passe_differents`. Bug trouvé dans le navigateur : sur le disque `Z:`, chaque écriture SQLite prend ~2,5 s et la création des tables dépassait la limite de 10 s de l'assistant ; la limite ne vaut plus que pour une base externe (15 s). Étape 1 SQLite et étape 2 refaites dans le navigateur. Tests du hub et clippy passent.
- **Pas vérifié** : lenteur du disque `Z:` (cause non cherchée : disque réseau ou virtuel ?).
- **Ne pas toucher** : `docs/profils-en-attente/` ; session `launcher-invitation` en cours.
- **Suite** : fusionner `launcher-invitation` ; déploiement.

### 2026-11-20 · Launcher : inscription sur un Rafthub « sur invitation » (session `launcher-invitation`)
- **Fait** : `register` du client prend un `invite_code` facultatif (vide = non envoyé) ; codes `registration_closed`, `invite_required`, `invalid_invite` typés ; champ « Code d'invitation » dans « Nouveau compte », trois messages en français ; 3 tests client (faux serveur), 2 tests d'interface ; capture vue (`RAFTUP_DEV_HUB_INSCRIPTION=1`). fmt, clippy, `cargo test --workspace` passent.
- **Pas vérifié** : inscription contre un vrai Rafthub en mode `invite`. Partie Discord non faite : le launcher n'a pas de connexion Discord, rien où brancher `invite_required`.
- **Ne pas toucher** : `crates/hub`, `docs/profils-en-attente/`.
- **Suite** : connexion Discord dans le launcher (lire `invite_required` de `needs_username`, envoyer `invite_code` à `/register`).

### 2026-11-20 · Fusion de `launcher-invitation` dans `main`
- **Fait** : branche du code d'invitation (`7f9b5a4`) fusionnée ; conflits seulement dans `etat-du-projet.md` et `journal.md` (les deux côtés gardés). Sur `main` : fmt, clippy `-D warnings` et `cargo test --workspace` passent.
- **Pas vérifié** : inscription avec code depuis le launcher contre un vrai Rafthub en mode `invite`.
- **Ne pas toucher** : `docs/profils-en-attente/` ; stash « retouches hub… », `web/soon.rs` + `soon.html`, conteneurs `raftup-test-*` (à supprimer sur accord).
- **Suite** : connexion Discord dans le launcher ; déploiement.

### 2026-11-20 · Rafthub officiel et auto-hébergé, connexion par pseudo (ADR 0038)
- **Fait** : option de compilation `officiel` (`HubConfig::discord_allowed`) : sans elle, Discord répond `discord_disabled` même réglé, et la console ne montre ni ne règle Discord (tableau de bord, Réglages, formulaire forgé refusé). Connexion API et console par e-mail **ou** pseudo (`LoginName`, `Store::password_identity_by_username`, limiteur par compte visé) ; launcher : « E-mail ou pseudo » à la connexion. Tests : `hub_auto_heberge_sans_discord`, `console_sans_discord_hors_version_officielle`, `connexion_par_pseudo`, `connexion_console_par_pseudo`. fmt, clippy (avec et sans `officiel`), `cargo test --workspace` et `cargo test -p hub --features officiel` passent.
- **Pas vérifié** : écran du launcher regardé (capture) ; console sans Discord ouverte dans le navigateur.
- **Ne pas toucher** : `docs/profils-en-attente/` ; stash, `web/soon.rs` + `soon.html`, conteneurs `raftup-test-*` (à supprimer sur accord).
- **Suite** : construire les deux binaires aux releases ; déploiement du Rafthub officiel.

### 2026-11-20 · Texte officiel de l'AGPL pour le hub (session `p4-agpl`)
- **Fait** : `crates/hub/LICENSE-AGPL` remplacé par le texte exact de <https://www.gnu.org/licenses/agpl-3.0.txt> (récupéré le 2026-11-20, LF) ; seule différence avec l'ancien fichier : une coupure de ligne dans « How to Apply ». Test `licence_agpl_est_le_texte_officiel_de_la_fsf` (en-tête, fin, LF, SHA-256) ; source et empreinte notées dans l'ADR 0003.
- **Vérifié** : `cargo test -p hub --test licence`, clippy du hub, `cargo test --workspace --no-fail-fast` (108 binaires ok). Au premier passage, `py-bridge` `ignore_eof_shutdown_et_drop_rapides` (limite de 2 s) a échoué sous charge ; il passe seul et au second passage.
- **Pas vérifié** : rien de plus ; le test de durée de `py-bridge` reste instable quand la machine est chargée.
- **Ne pas toucher** : `docs/profils-en-attente/`.

### 2026-11-20 · Rafthub officiel : déploiement Docker préparé (`deploy/rafthub/`)
- **Fait** : `Dockerfile` (build `--release --features officiel`, Debian 13 slim, uid 10001, volume `/data`, image `backup` avec sqlite3), `docker-compose.yml` (Traefik : `Host(${RAFTHUB_DOMAIN})`, certresolver et réseau en variables, pas de port sur l'hôte, lecture seule, healthcheck), `.env.example`, `backup.sh` (`sqlite3 .backup` + `integrity_check`, gzip, rotation `BACKUP_KEEP_DAYS`), `README.md` pas à pas. Hub : `RAFTUP_HUB_DATA_DIR` (`server_config::resolve_data_dir`), `rafthub healthcheck` (module `health`, sans curl), arrêt propre sur SIGTERM. Tests unitaires ajoutés ; fmt, clippy `-D warnings` et `cargo test --workspace` passent.
- **Vérifié en local (Docker 29)** : image construite (147 Mo), « healthy » pendant l'assistant, code d'installation au journal, `docker stop` en 1,5 s, sauvegarde à chaud hub en marche (données du `-wal` comprises), boucle nocturne arrêtée par signal.
- **Pas vérifié** : rien sur le serveur triomyno (interdit) ; Traefik réel et certificat ; restauration ; clippy du bloc `#[cfg(unix)]` (compilé sans avertissement dans l'image seulement).
- **Ne pas toucher** : `docs/profils-en-attente/` ; volumes `rafthub-p2-test-data`, `rafthub-p2-test-bk` et images `rafthub:officiel-test`, `rafthub-backup:officiel-test` (essais locaux, à supprimer sur accord).
- **Suite** : choisir le domaine, puis suivre `deploy/rafthub/README.md` sur le serveur.

### 2026-11-20 · Trois correctifs : code d'installation, durée dans « Partie en cours », captures (session `p1-correctifs`)
- **Fait** : l'étape 1 réussie de l'assistant pose le cookie `rafthub_setup_pass` (`HttpOnly`, `SameSite=Strict`, `/admin/installation`, 1 h) au jeton aléatoire que `main` passe au vrai hub (`HubConfig::setup_pass`) : l'étape 2 cache le champ du code ; sans cookie valable (hub relancé), le code est demandé ; cookie effacé une fois l'admin créé (ADR 0037, 3 tests). Panneau « Durée estimée » dans la colonne de droite de « Partie en cours » si la partie est la dernière seed générée (ADR 0030, 2 tests). Captures dans `Z:\raftup-test\captures-p1-correctifs` : connexion « E-mail ou pseudo », inscription avec code d'invitation, « Partie en cours » avec la durée (moteur Rust) ; aucun défaut vu. fmt, clippy `-D warnings`, `cargo test --workspace` passent.
- **Pas vérifié** : enchaînement des deux étapes dans un vrai navigateur ; l'estimation n'existe que si l'écran « Créer » a dessiné la seed prête avant le passage à « Partie en cours » (vrai sur la capture).
- **Ne pas toucher** : `docs/profils-en-attente/` ; le générateur Python de `Z:\raftup-test\runtime` est à réinstaller (versions épinglées changées), non fait (téléchargement).
- **Suite** : refaire l'installation d'un hub neuf dans le navigateur pour voir l'étape 2 sans code.

### 2026-11-20 · Binaires prêts à l'emploi dans les releases (session `p1-release`)
- **Fait** : workflow `release.yml` sur tag `v*` (tag comparé à la version de `Cargo.toml`) : `rafthub` public Linux et Windows x86_64, `rafthub-officiel-linux-x86_64` (`--features officiel`), launcher `raftup-windows-x86_64.exe` nu ; licence AGPL et lien du code source dans les archives de Rafthub ; `.sha256` à côté de chaque fichier ; brouillon de release, **sans signature**. Job CI `hub-officiel` (clippy + tests avec `officiel`). Procédure de signature à la main : `docs/publier-une-release.md`. YAML relu et analysé (yaml-rust2) ; fmt, clippy `-D warnings`, `cargo test --workspace` passent.
- **Pas vérifié** : workflow jamais lancé (rien poussé, pas de tag) ; actionlint absent ; téléchargement par le launcher et par Rafthub depuis une vraie release (redirection GitHub vers un autre domaine).
- **Ne pas toucher** : `docs/profils-en-attente/`.
- **Suite** : première vraie release `v0.1.0` (tag, signature, `update.json`) ; déploiement du Rafthub officiel.

### 2026-11-20 · Rafthub : HTTPS intégré, en option (ADR 0039, session `p1-https`)
- **Fait** : module `hub::tls` (rustls + ring) : `--tls-cert` / `--tls-key` (et `RAFTUP_HUB_TLS_CERT` / `_KEY`) et Let's Encrypt automatique `--tls-acme <domaine>` (crate `rustls-acme`, défi TLS-ALPN-01, port 443, `rafthub-data/acme/`, `--tls-acme-staging`) ; assistant d'installation servi pareil ; cookies `Secure` en HTTPS intégré (`HubConfig::https`) ; sans option, HTTP inchangé. Tests `tests/https.rs` (certificat `rcgen`, client qui lui fait confiance, port libéré à l'arrêt, messages d'erreur) ; licences des 28 nouvelles crates vérifiées (MIT/Apache) ; message d'un certificat absent vu avec le vrai binaire.
- **Pas vérifié** : Let's Encrypt jamais essayé (ni le vrai serveur ni celui d'essai) ; HTTPS dans un vrai navigateur ; launcher contre un hub en HTTPS.
- **Ne pas toucher** : `docs/profils-en-attente/` ; `api.rs`, `web/setup.rs`, `web/install.rs`, `.github/workflows/` (autres sessions).
- **Suite** : au déploiement, essayer `--tls-acme-staging` puis `--tls-acme` sur un vrai domaine.

### 2026-11-20 · Changement de pseudo (session `p1-pseudo`)
- **Fait** : `PATCH /v1/me` (`username`) : règles de l'inscription, `username_taken`, une fois par 24 h (`username_change_too_soon`), compté sur `admin_events` (`username_changed`, « ancien → nouveau », écrit dans la même transaction, sans migration) ; console : « Renommer » sur la page Joueurs (CSRF, sans limite, ne compte pas pour le joueur), libellé « Pseudo changé » dans Journaux ; suppression RGPD vide le détail ; launcher : `HubClient::change_username`, bouton « Changer de pseudo » sous « Connecté en tant que… », message clair pour les 24 h. ADR 0033 complété. Tests : `tests/username_change.rs` (5), `renommer_un_compte`, 2 tests client, 3 tests d'interface.
- **Vérifié** : fmt, clippy `-D warnings`, `cargo test --workspace` ; toute la suite du hub sur MariaDB 11.4 et PostgreSQL 18 (conteneurs `raftup-test-*`). **Pas vérifié** : écran du launcher et page Joueurs regardés à l'œil ; MySQL 8.4 non relancé.
- **Ne pas toucher** : `docs/profils-en-attente/`.
- **Suite** : capture de l'écran « Amis » avec le champ ouvert et de la page Joueurs.

### 2026-11-20 · Launcher : connexion Discord, route `GET /v1/info` (session `p3-discord`)
- **Fait** : hub, route publique `GET /v1/info` (nom, version, `login_methods` : `password`, plus `discord` si `discord_allowed` et Discord réglé, `registration_mode`), 4 tests `tests/info.rs`. Client : `hub_info` (404 = mot de passe seulement), `discord_start` / `discord_finish` / `discord_register`, serveur local `DiscordReturn` (127.0.0.1, port libre, 5 min, annulable), erreurs typées ; tests faux serveur `tests/hub_discord.rs`, bout en bout vrai routeur + `FakeDiscord` `hub/tests/launcher_discord.rs` (le hub dépend du launcher en test, pas l'inverse). Écran « Amis » : bouton « Se connecter avec Discord » seulement si proposé, attente (copier le lien, annuler), étape « Choisir un pseudo » (+ code d'invitation). ADR 0035 complété.
- **Pas vérifié** : vraie application Discord ; écran regardé à l'œil (pas de capture).
- **Ne pas toucher** : `docs/profils-en-attente/`.
- **Suite** : liaison de Discord à un compte existant depuis le launcher ; capture de l'écran Discord.

### 2026-11-20 · Fusion des sept sessions du plan dans `main`
- **Fait** : fusion de `p4-agpl`, `p2-deploiement`, `p1-correctifs`, `p1-release`, `p1-https`, `p1-pseudo`, `p3-discord`. Conflits réglés à la main : `hub/src/main.rs` (HTTPS intégré + arrêt sur SIGTERM, `shutdown_signal()` partagé par l'assistant et le hub ; sonde `healthcheck` placée avant la lecture du TLS), `hub/src/lib.rs` (`setup_pass` et `https`), écran « Amis » (changement de pseudo + Discord), `Cargo.lock`, `hub/Cargo.toml`, journal et état.
- **Vérifié** : `cargo clippy --workspace --all-targets -D warnings`, clippy du hub avec `--features officiel`, `cargo test --workspace --no-fail-fast` (113 binaires ok).
- **Pas vérifié** : écrans regardés à l'œil (pseudo, Discord) ; workflow de release jamais lancé ; Let's Encrypt, vrai SMTP, vraie application Discord ; rien poussé.
- **Ne pas toucher** : `docs/profils-en-attente/` ; stash, `web/soon.rs` + `soon.html`, conteneurs `raftup-test-*`, volumes et images `rafthub-p2-test-*` / `*:officiel-test` (à supprimer sur accord).
- **Suite** : pousser ; choisir le domaine du Rafthub officiel puis suivre `deploy/rafthub/README.md` ; poser un tag `v*` pour essayer la release.

### 2026-11-20 · CI verte sur Windows, macOS et Linux
- **Fait** : poussé `main` ; première CI hors Windows : `low_copy_name` (py-bridge) compilée seulement sous Windows et en test ; tests `launch.rs` sur `/usr/bin/true` (absent : `/bin/true` sous macOS) ; `missing_rom` coupe le chemin sur `/` et `\` à la main.
- **Vérifié** : CI GitHub verte (Windows, macOS, Linux, `hub-officiel`).
- **Pas vérifié** : launcher lancé à la main hors Windows ; workflow de release (aucun tag).
- **Ne pas toucher** : `docs/profils-en-attente/`.
- **Suite** : domaine du Rafthub officiel ; tag `v*` pour essayer la release.

### 2026-11-20 · Rafthub officiel : console invisible, Cloudflare, adresse officielle (ADR 0040)
- **Fait** : `--admin-listen` / `RAFTUP_HUB_ADMIN_LISTEN` : `hub::public_router` (API + `/telecharger` + `/static/`) sur `--listen`, `hub::admin_router` (console seule, HTTP clair, cookies sans `Secure` via `HubConfig::admin_separate`) sur l'autre adresse ; assistant d'installation sur l'adresse de la console ; page publique sur `/static/` ; `DEFAULT_HUB_URL` = `option_env!("RAFTUP_OFFICIAL_HUB_URL")`, lue par `release.yml` dans `vars` ; déploiement : port 8788 publié sur `RAFTHUB_ADMIN_BIND`, Traefik `!PathPrefix(/admin)` + `ipAllowList` `RAFTHUB_ALLOWED_SOURCES` (plages Cloudflare), README (Cloudflare, Full strict). Tests `tests/admin_separe.rs` (4).
- **Vérifié** : vrai binaire avec deux adresses (`/admin` 404 public, console 200 à part, `/v1` 404 côté console) ; clippy (avec et sans `officiel`), `cargo test --workspace` (114 binaires ok).
- **Pas vérifié** : déploiement réel, Cloudflare et `ipAllowList` dans Traefik ; launcher compilé avec l'adresse officielle (seulement `cargo check`).
- **Ne pas toucher** : `docs/profils-en-attente/`.
- **Suite** : acheter le domaine (idée : `raftup-games.com`), le confier à Cloudflare, créer la variable GitHub `RAFTUP_OFFICIAL_HUB_URL`, puis déployer sur triomyno.

### 2026-11-20 · Rafthub officiel en ligne sur triomyno (`hub.raftup-games.com`)
- **Fait** : dépôt cloné dans `~/rafthub` (compte `claude`), `.env` (Traefik `traefik-public`, `letsencrypt`, `websecure`, console sur `10.10.0.1:8788`), dossier de sauvegarde à l'uid 10001, `docker compose build` + `up -d` (`rafthub`, `rafthub-backup`). Installation faite par l'utilisatrice (base intégrée, premier admin). Correctif : la sonde de santé visait `/admin/static/…` (404 côté public) → `/static/rafthub.css`, servi aussi par l'assistant.
- **Vérifié** : conteneur « healthy », `https://hub.raftup-games.com/v1` 200 avec certificat Let's Encrypt, `/admin` 404 depuis Internet, console `http://10.10.0.1:8788/admin` par le VPN.
- **Pas vérifié** : la première sauvegarde nuitière (3 h) ; le formulaire d'installation refusé (cookie CSRF) dans le navigateur de l'utilisatrice, alors qu'il passe dans le navigateur intégré : cause non trouvée. Pas de Cloudflare (DNS chez feelb.net, IP visible).
- **Ne pas toucher** : `docs/profils-en-attente/` ; sur triomyno, `~/rafthub/deploy/rafthub/.env` (droits 600) et le volume `rafthub-data`.
- **Suite** : variable GitHub `RAFTUP_OFFICIAL_HUB_URL=https://hub.raftup-games.com` ; Réglages du hub (SMTP, Discord, inscriptions) ; site de la racine `raftup-games.com`.

### 2026-11-20 · Page d'attente de `raftup-games.com`
- **Fait** : `deploy/site/` : page statique (palette Acier, logo et polices du launcher, liens vers les releases et le code), image `nginx-unprivileged` en lecture seule, en-têtes de sécurité (CSP sans script), compose avec labels Traefik ; déployée sur triomyno (`raftup-site`). Variable GitHub `RAFTUP_OFFICIAL_HUB_URL=https://hub.raftup-games.com` créée.
- **Vérifié** : `https://raftup-games.com` 200 avec certificat Let's Encrypt, logo et polices servis, 404 sur une adresse inconnue ; vue dans le navigateur intégré.
- **Pas vérifié** : affichage sur téléphone ; `www.raftup-games.com` (pas de DNS ni de route).
- **Ne pas toucher** : `docs/profils-en-attente/`.
- **Suite** : vrai site officiel (présentation, jeux, téléchargements, doc) ; réglages SMTP, Discord et inscriptions du hub.

### 2026-11-20 · Site officiel de `raftup-games.com`
- **Fait** : `b0a94bd` : site statique écrit à la main (aucun script) dans `deploy/site/www` : accueil, fonctionnalités, jeux (Slime Rancher), téléchargements (launcher, Rafthub officiel et auto-hébergé), documentation (premiers pas, parties, héberger son Rafthub, FAQ), mentions, 404 ; captures du launcher en WebP ; CSP plus stricte (`style-src 'self'`) ; nginx : index par dossier, redirections relatives, page 404. Vu dans le navigateur intégré, ordinateur et téléphone (375 px), avec la même CSP : aucun défilement horizontal, aucun lien interne cassé.
- **Pas vérifié** : image Docker non construite et `nginx.conf` jamais chargé par un vrai nginx ; rien déployé ; liens des releases vides tant qu'aucun tag n'est publié ; pas de mentions légales d'éditeur ni d'hébergeur (choix de l'utilisatrice).
- **Ne pas toucher** : `docs/profils-en-attente/`.
- **Suite** : sur accord, `docker compose build && docker compose up -d` dans `~/rafthub/deploy/site` sur triomyno (après `git pull`).

### 2026-11-20 · Présence « en partie » et liaison Discord dans le launcher
- **Fait** : présence « en partie · <jeu> » tant qu'un jeu lancé depuis la Bibliothèque tourne (processus suivi) ou qu'une partie est hébergée (jeux de la seed), nom du jeu seul, nettoyé et coupé à 64 caractères (`launcher_core::activity`, `Presence::playing`), renvoyée dès qu'elle change ; panneau « Profil » de l'écran « Amis » : « Lier mon compte Discord » / « Délier Discord » (si `GET /v1/info` propose Discord), refus `discord_taken`, `already_linked`, `last_identity` en clair. Hub inchangé (liaison déjà là et testée). Tests client sur faux serveur et tests d'écran ; fmt, clippy (avec et sans `officiel`), `cargo test --workspace` passent.
- **Pas vérifié** : écran « Profil » jamais regardé (captures à faire) ; vraie liaison avec une vraie application Discord ; jeu passant par Steam (le processus rend la main tout de suite, la présence revient alors à « en ligne ») ; pendant l'attente de Discord (5 min au plus), la présence n'est pas renouvelée.
- **Ne pas toucher** : `docs/profils-en-attente/`.
- **Suite** : captures de l'écran « Amis » connecté (panneau « Profil ») avec la macro `RAFTUP_DEV_CAPTURE`.

### 2026-11-20 · V3 phase A : format de monde natif (ADR 0041)
- **Fait** : `193df45` ADR 0041 (accepté en autonomie, à relire) et spécification autonome `spec/monde-natif/` ; `b51a093` crate `worlds-native` (TOML borné, mini-langage de règles, erreurs ligne/colonne, compilation vers `logic`), `GameOptions::Native`, YAML des mondes natifs, APQuest natif identique au portage (même empreinte) ; launcher : dossier `mondes-natifs/` lu pour le moteur Rust.
- **Vérifié** : `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -D warnings`, `cargo test --workspace -j 4` (aucun échec).
- **Pas vérifié** : un monde natif joué en vrai depuis le launcher (seulement en test) ; affichage des mondes natifs dans la Bibliothèque (pas fait).
- **Ne pas toucher** : `docs/profils-en-attente/` ; `crates/registry`, `launcher-core::registry`, `.github/` (session confiance du registre) ; `launcher-ui/src/screens/friends.rs`, `hub_client` (session « Amis »).
- **Suite** : phase B, conversion apworld → natif à partir de l'export de l'ADR 0028 (note de relais dans le dossier partagé).

### 2026-11-21 · Analyse du projet
- **Fait** : analyse du dépôt sans rien coder : 13 crates, environ 64 000 lignes de Rust et 1 230 tests ; règles du CLAUDE.md respectées dans le code (`unsafe` interdit, aucun `unwrap` hors tests, aucune crate MIT/Apache qui dépend du hub). Points d'attention : MVP jamais validé en vrai, un seul jeu au catalogue, 12 ADR « acceptés en autonomie » à relire, beaucoup de fonctions jamais essayées en vrai, 6 commits non poussés.
- **Pas vérifié** : build, tests et clippy non lancés : sur ce PC il manque les Visual Studio Build Tools (`link.exe` introuvable ; dans Git Bash, `link` est celui de coreutils) ; clés de signature absentes de `C:\Users\ionak\.raftup-keys` et de `%USERPROFILE%\.raftup-keys` sur ce PC.
- **Ne pas toucher** : `docs/profils-en-attente/`.
- **Suite** : installer les Build Tools (« Développement Desktop en C++ »), puis relire les ADR acceptés en autonomie avant toute nouvelle fonction.

### 2026-11-21 · Interface, étape 1 : ménage et réglages par défaut
- **Fait** : `f683741` : entrées « Bientôt » cachées du menu ; « Options avancées » repliées dans Créer et Héberger (port, adresse, webhook, chiffrement ; apworlds à part), ouvertes seules sur le champ refusé ; jaquettes en aplats colorés avec le titre ; « Se connecter à la main » replié ; adresse du Rafthub dans un nouvel écran « Paramètres », texte « Aucun Rafthub public » retiré ; « Rejoindre » lit un lien `raftup://` du presse-papiers. fmt, clippy et `cargo test --workspace` (1 213 tests) passent ; captures regardées (macro, `RAFTUP_DEV_AVANCE=1` ouvre les blocs).
- **Pas vérifié** : bloc « Se connecter à la main » jamais vu à l'écran (Slime Rancher absent de ce PC) ; launcher compilé avec `RAFTUP_OFFICIAL_HUB_URL` ; presse-papiers hors Windows.
- **Ne pas toucher** : `docs/profils-en-attente/`.
- **Suite** : ADR 0043 accepté (certificat généré, port mixte, empreinte dans le lien) : essai réel d'abord (Slimipelago face au port mixte), puis le code ; d'ici là, wss PEM reste le défaut.

### 2026-11-21 · Salon de préparation, version locale (ADR 0044)
- **Fait** : maquettes comparées à des captures du launcher (identité visuelle fidèle, parcours différent) ; ADR 0044 validé (salon local, YAML écrits par Raftup) ; `launcher_core::crew` (pseudos vérifiés, un YAML par joueur) ; « Créer une partie » refait comme la maquette du salon (ton monde, préréglages à segments, équipage, profil de session, hébergement, « Générer et héberger », YAML à la main en options avancées) ; scénario de capture `salon-preparation`. fmt, clippy et `cargo test --workspace` (1 219 tests) passent ; captures regardées.
- **Pas vérifié** : vraie génération depuis l'équipage (aucun générateur installé sur ce PC) : la section de jeu vide `"Slime Rancher": {}` reste à valider par `Generate.py` 0.6.8 ; le bas de l'écran (ligne « Ajouter un joueur », panneau « Lancer la partie ») n'est vu qu'en partie sur la capture 1280×820.
- **Ne pas toucher** : `docs/profils-en-attente/`.
- **Suite** : générer une vraie partie depuis le salon (installer le générateur, puis Slime Rancher et APQuest), puis l'étape 3 du plan (« Partie en cours » comme la maquette).
