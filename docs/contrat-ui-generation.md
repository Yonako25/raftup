# Contrat de l'interface « Créer une partie »

> Brouillon à valider avant de coder l'écran. Il fixe ce que l'interface appelle dans `launcher-core`, les états de l'écran et les messages à l'utilisatrice. Il ne dit rien du dessin : celui-ci vient de la maquette (`docs/maquette/`).

## Ce que l'écran appelle (`launcher-core`)

| Besoin | Appel | Remarque |
| --- | --- | --- |
| Le runtime est-il installé ? | `generation::runtime_status(paths, &pins()?)` | `Missing { download_bytes }` donne la taille à montrer (environ 72 Mo) |
| Installer le runtime | `generation::ensure_runtime(paths, &pins, &downloader, progress)` | Bloquant : thread à part ; `progress` reçoit `SetupStep` et des octets |
| Générer puis héberger | `party::generate_and_host(...)` ou `generate_blocking` puis `host_generated` | Depuis un thread à part (voir ci-dessous) |
| Annuler | `CancelToken::cancel()` | Annule la génération en moins d'une seconde |
| Journal de génération | `Sender<String>` passé à `generate` | Lignes de stderr du générateur, en direct |
| Suivre la partie | `Host::status()`, `Host::events_since()` | Déjà utilisés par l'écran « Partie en cours » |

Règles d'appel :

- L'interface **n'appelle jamais** `generate` dans le thread de dessin. Elle utilise un thread `std` : `generate_blocking`, puis `host_generated` dans le runtime tokio.
- `PythonBackend` et `GeneratorBackend` doivent être `Send`.
- Une seule génération à la fois : `GenerationError::Busy` est un message, pas un plantage.
- `PartyError::Host { generated, .. }` : garder `generated` et proposer « Réessayer » (autre port) sans régénérer.

## États de l'écran

1. **Préparation** : choix des fichiers YAML (un par joueur), mot de passe facultatif, port, chiffrement.
2. **Runtime absent** : bouton « Installer le générateur (≈ 72 Mo) » avec le **consentement** avant tout téléchargement. Progression par étape (Python, Archipelago, bibliothèques), annulable.
3. **Génération** : barre indéterminée, journal en direct, bouton « Annuler ». Délai maximal : `DEFAULT_TIMEOUT` (15 min).
4. **Hébergement** : adresse, nombre de slots, bouton « Copier l'adresse » ; bascule vers l'écran « Partie en cours ».
5. **Erreur** : message en français (tableau ci-dessous), bouton « Réessayer », lien « Voir le journal ».

## Messages d'erreur à écrire

| Cas | Message |
| --- | --- |
| `NoPlayers` | « Ajoutez au moins un fichier YAML. » |
| `InvalidPlayerFile` | « Le fichier {nom} n'est pas utilisable : {raison}. » |
| `GeneratorFailed` | « Archipelago n'a pas pu générer la partie : {message}. » + « Voir le détail » (traceback) |
| `GeneratorFailed` (`FileNotFoundError` sur une ROM) | « Un des jeux demande votre copie de sa ROM ({fichier}), introuvable. Raftup ne fournit aucune ROM : ajoutez votre copie, ou retirez ce joueur. » + « Voir le détail » |
| `IncompatibleApworld` | « L'apworld {nom} demande une autre version d'Archipelago. » |
| `Cancelled` | « Génération annulée. » |
| `Busy` | « Une génération est déjà en cours. » |
| `Download` | « Le téléchargement a échoué ({raison}). Vérifiez votre connexion. » |
| `EmptyPublicHost` | « Indiquez l'adresse que les autres joueurs utiliseront. » |
| `Host` (port occupé…) | « Impossible de démarrer le serveur : {raison}. » + « Réessayer » |

## Avertissements obligatoires

- **Apworld** : « Un apworld est du code qui s'exécute sur votre PC, sans bac à sable. N'ajoutez que ceux que vous jugez sûrs. » (avant toute copie d'un apworld)
- **Premier téléchargement** : sources et taille affichées (Python, Archipelago, bibliothèques), vérifiées par hash.

## Décisions prises (ADR 0019)

1. Conservation de `generations/` : **purge manuelle** (bouton « Supprimer » par partie, avec confirmation) ; rien d'automatique.
2. YAML : **sélecteur de fichiers seul**, pas de dossier surveillé.
3. Durcissement réseau du worker : **activé par défaut** (`PythonBackend::new` → `RAFTUP_BLOCK_NETWORK=1`).
