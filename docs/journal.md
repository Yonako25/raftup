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
- **Suite** : pousser la branche pour lancer la CI, corriger ce qui casse sous macOS et Linux.
### 2026-11-19 · Bac à sable du générateur (ADR 0034)
- **Fait** : ADR 0034 (accepté en autonomie, à relire). Sous Windows, le worker est lancé en intégrité basse : copie `python-raftup-low.exe` étiquetée par `icacls`, écriture permise seulement dans `generations/<horodatage>/` (TEMP compris), remis à « moyen » après l'arrêt ; le worker annonce son niveau (`whoami /groups`) et le pont refuse un worker qui n'est pas en intégrité basse. Activé par défaut (`PythonBackend::sandbox`), aussi pour le moteur export. Sans `unsafe`, sans nouvelle crate. Tests `py-bridge/tests/sandbox.rs` (faux worker vraiment lancé en intégrité basse) ; tests réels de génération (5) et d'export (8, dont Slime Rancher) passent avec le bac à sable ; 39 tests Python.
- **Pas vérifié** : repli hors Windows (rien appliqué, seulement écrit et relu) ; disque sans étiquettes (FAT32, réseau : la génération échoue) ; runtime neuf (`.pyc` jamais écrits par le worker, coût non mesuré) ; durée comparée à avant (export réel : 846 s pour 8 tests, sans mesure avant). Builds dans un `target` à part (`agents-v2-bac-a-sable`, puis `v2-bac-a-sable`) : le `target` commun mélangeait les worktrees.
- **Suite** : relire l'ADR 0034 ; plus tard, AppContainer (lecture et réseau) et bac à sable Linux/macOS.
### 2026-11-19 · Règles d'objet, objets à placer tôt et accessibilité (ADR 0029, session early-items)
- **Fait** : ADR 0029 (accepté en autonomie, à relire). `logic` : `ItemRule` (arène), `can_hold`, `Accessibility`, `must_reach` ; `fill` : `fill_with_early` (sphère 0 d'après `distribute_early_items`), règles d'objet respectées au placement et à l'équilibrage, `minimal` / `items` ; export format 2 : règles d'objet en listes calculées par Archipelago, contrôle croisé `can_fill`, `early_items`, `accessibility`. Monde `raftup_sonde` : options `early_items`, `item_rules`. Tests `fill` (8), import, Python (41), réels `export_real` (2 nouveaux).
- **Pas vérifié** : un monde tiers qui pose des règles d'objet ou des objets tôt ; gros multiworld (coût du calcul en Python) ; pas d'échange façon `remaining_fill` (échec ⇒ repli Python).
- **Suite** : relecture de l'ADR 0029 par l'utilisatrice ; essai sur un apworld qui se sert de `local_items` ou d'`early_items`.
