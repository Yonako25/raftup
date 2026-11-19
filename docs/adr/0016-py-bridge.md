# 0016 — `py-bridge` : piloter le générateur Python

- Statut : accepté
- Date : 2026-11-17

## Contexte

- Le MVP génère avec le générateur officiel d'Archipelago, dans un process Python séparé (SPEC § 3), en JSON-RPC sur stdin/stdout, sans port réseau.
- Cette première étape livre seulement le pont (`ping`, `version`, `echo`) ; l'appel au vrai générateur viendra ensuite.

## Décision

| # | Sujet | Décision |
| --- | --- | --- |
| 1 | Modèle d'exécution | Synchrone, threads `std`, sans tokio : un seul appel à la fois. Depuis du code async : `spawn_blocking` |
| 2 | Protocole | JSON-RPC simplifié : `{id, method, params}` → `{id, result}` ou `{id, error:{code, message, data?}}`, une ligne JSON chacun. Pas de champ `"jsonrpc"`, ni lots, ni notifications. Codes d'erreur de JSON-RPC 2.0 |
| 3 | Version | `version` renvoie `protocol` ; le pont refuse un worker d'une autre version au démarrage |
| 4 | Livraison du script | `python -s -P -X utf8 -c <script>` : rien n'est écrit sur le disque. *Révisé le 19 novembre 2026 (ADR 0028) :* au départ `-I` ; `-I` = `-E -P -s`, et `-E` empêchait de poser `PYTHONHASHSEED`. Le pont retire lui-même toutes les variables `PYTHON*` (héritées ou passées dans `env`), puis pose `PYTHONHASHSEED=0`. L'isolation est la même, et l'ordre des `set` Python devient reproductible |
| 5 | Erreur de flux | Délai dépassé, JSON invalide, ligne trop longue ou mauvais id : le process est tué et le pont est fermé. Seule une erreur renvoyée par le worker laisse le pont utilisable |
| 6 | Valeurs par défaut | 1 Mio par ligne, 10 s par appel, 30 s au démarrage, 2 s de grâce à l'arrêt |
| 7 | Tests sans Python | Faux worker Rust (`[[bin]] fake-worker`), à exclure d'un futur paquet |
| 8 | stdout du worker | Le protocole utilise une copie du vrai stdout ; le fd 1 est redirigé vers stderr (les `print` du générateur ne cassent rien), stdin remplacé par `devnull` |
| 9 | Dépendances | Seul `launcher-core` dépendra de `py-bridge` |

## Conséquences

- Aucune dépendance externe nouvelle (`serde`, `serde_json`, `thiserror`, `tracing` du workspace).
- Sous Windows : `CREATE_NO_WINDOW`, sans `unsafe`.
- Le worker quitte à la fermeture de stdin, donc pas de process orphelin si le launcher plante. Réserve : c'est vrai seulement quand le worker lit stdin ; pendant un long `generate` (boucle bloquée dans le générateur), il ne voit pas la fermeture et peut survivre au launcher.
- `child.kill()` ne tue pas les petits-enfants sous Windows : le worker est donc placé dans un Job Object `KILL_ON_JOB_CLOSE` (crate `win32job`, API sûre, MIT OR Apache-2.0, `crates/py-bridge/src/job.rs`). Le job est détenu par `PyBridge` et fermé à l'arrêt, à l'annulation et au Drop. Hors Windows : rien pour l'instant.
- Sous Windows, `python` doit être un `.exe` : les `.bat`/`.cmd` sont refusés (`BridgeError::UnsupportedPython`, testé). Quand le pont sera branché sur python-build-standalone, utiliser un chemin absolu.
- `-I` (mode isolé) ignore les variables `PYTHON*` (`PYTHONPATH`, `PYTHONHOME`...) passées via `BridgeOptions::env` ; les autres variables passent. Documenté sur le champ.
- L'écriture des requêtes se fait depuis un thread dédié, avec le même délai que la lecture : un worker qui ne lit pas stdin donne `Timeout`, jamais un blocage. Une requête plus grande que `max_line_bytes` est refusée avant l'écriture (`RequestTooLarge`), sans fermer le pont.
- Une réponse `id: null` avec `error` (le worker n'a pas pu lire la requête) devient `BridgeError::Protocol(RpcError)` : le message du worker est conservé, mais le pont se ferme (décision 5).
- Le worker Python quitte avec le code 2 (pas 0) après « ligne trop longue ».
- Étapes suivantes : téléchargement de python-build-standalone vérifié par hash, environnement par version d'Archipelago, méthode `generate`, notifications de progression (protocole 2).
