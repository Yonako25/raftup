# Répartition de la V2 en sous-sessions autonomes

Écrit le 19 novembre 2026. Consigne de l'utilisatrice : plusieurs sous-sessions travaillent en parallèle, chacune sur une partie, et **ne lui demandent rien tant que le projet n'est pas fini**.

## Règles communes (à recopier dans chaque brief)

- Lire au démarrage `CLAUDE.md`, `docs/regles-agents.md`, `docs/etat-du-projet.md`, les 5 dernières entrées de `docs/journal.md`, `docs/SPEC.md`.
- **Ne rien demander** : prendre l'option recommandée. Une décision hors SPEC s'écrit dans un ADR (numéro réservé ci-dessous), statut « accepté en autonomie le <date>, à relire par l'utilisatrice », puis on code.
- Jamais de push, de publication, ni de téléchargement non épinglé par sha256 ; le noter dans le journal si c'était nécessaire. Ne pas toucher `docs/profils-en-attente/`. Pas de test en jeu.
- Code en anglais ; commentaires et documentation en français, simples ; notions Rust non triviales expliquées ; pas d'`unwrap`/`expect`/`unsafe` en bibliothèque ; ordre déterministe ; un bug corrigé arrive avec son test.
- Avant chaque commit : `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (`CARGO_TARGET_DIR=target/dev-agents`). Commits locaux sur sa branche (worktree à soi).
- À la fin : entrée de 5 lignes dans `docs/journal.md`, mise à jour courte de `docs/etat-du-projet.md` (ajouts en bas des listes, pour limiter les conflits de fusion).
- Ne pas modifier les fichiers des autres sessions, sauf ajustement minimal (une ligne dans `Cargo.toml`, un `cfg`).

## Les sessions

| Nom | Partie | Fichiers | ADR réservé | Fini quand |
| --- | --- | --- | --- | --- |
| `ci-multi-os` | NF11 : CI Windows, macOS, Linux ; NF12 : test d'empreinte de seed APQuest identique partout | `.github/workflows/ci.yml`, `cfg(windows)` minimaux (`py-bridge/src/job.rs`, `winreg`, schéma `raftup://`) | — | Workflow multi-OS écrit, workspace propre pour les trois cibles (au moins par `cfg` relus ; `cargo check --target` seulement si la cible est déjà installée), test d'empreinte en place |
| `moteur-finitions` | Spoiler écrit par le moteur Rust seul ; moteur réellement utilisé affiché à la fin de la génération ; explication d'un échec du placement Raftup montrée dans l'interface (aujourd'hui seulement dans le journal avant le repli Python) | `crates/generate` (nouveau module de spoiler), `launcher-core/src/generation/{rust.rs,export.rs}`, `launcher-ui/src/screens/generation.rs` | — | Spoiler `AP_<seed>_Spoiler.txt` dans le zip du moteur Rust seul (sphères, placement), moteur et raison du repli visibles, tests |
| `early-items` | `early_items` et `item_rule` (dont `local_items` / `non_local_items`) dans le moteur export ; `accessibility: minimal` | `crates/logic`, `crates/fill` (hors nouveaux fichiers des autres), `generate/src/imported.rs`, `py-bridge/worker/raftup_export.py` | 0029 | Ces options ne forcent plus le repli Python ; tests de `fill` et d'import ; test réel avec le monde `raftup_sonde` |
| `duree-estimee` | Durée estimée par joueur (SPEC V2) ; plan de l'équilibrage par temps de jeu | nouveau module dans `launcher-core`, champ facultatif dans les profils (`format_version` inchangé si possible), widget `launcher-ui/src/screens/estimate.rs` + une ligne dans `create.rs` ; équilibrage dans un nouveau fichier `crates/fill/src/time_balance.rs` | 0030 | Estimation calculée depuis la seed (sphères, lieux par joueur) et affichée après génération ; équilibrage par temps écrit et testé derrière une option |
| `suivi-async` | Suivi asynchrone : résumé de retour, notifications, release par vote, mode absence | `crates/server` (commandes et état), `launcher-ui/src/screens/running.rs` | 0031 | Fonctions dans le serveur (sauvegardées dans le `.raftup`), compatibles avec le protocole (rien de propriétaire sans repli), visibles dans « Partie en cours », tests |
| `registre` | Registre de paquets signés (apworlds, profils) avec CLI de publication (minisign, comme ADR 0022) | nouvelle crate `crates/registry` + binaire CLI ; lecture côté `launcher-core` (catalogue) | 0032 | Format d'index versionné, signature et vérification, CLI `publish` qui produit les fichiers (sans rien envoyer), lecture et vérification par le launcher, tests |
| `hub` | Service `raftup-hub` : comptes, amis, présence (ADR 0002, 0003, AGPL) | nouvelle crate `crates/hub` (AGPL-3.0-or-later ; aucune crate MIT/Apache n'en dépend) | 0033 | Serveur HTTP (comptes e-mail + mot de passe argon2, amis, présence), stockage (SQLite en test, PostgreSQL prévu), tests ; pas de déploiement |
| `bac-a-sable` | Bac à sable par OS pour le code des apworlds (SPEC, risques) | `crates/py-bridge` (`job.rs`, `bridge.rs`) | 0034 | Worker lancé avec des droits réduits sous Windows (jeton restreint ou intégrité basse, sans `unsafe` dans Raftup : crate sûre ou API existante), repli documenté ailleurs ; tests réels `#[ignore]` qui passent toujours |

Hors répartition : les essais en jeu (valider un lieu de Slime Rancher, partie APQuest), qui demandent l'utilisatrice, et les essais de validation du MVP, reportés à la fin de la V3.
