# Règles pour les agents (développeur, testeur, relecteur…)

Ce fichier évite de recopier les mêmes consignes dans chaque prompt.
Un prompt d'agent dit : « suis `docs/regles-agents.md` », puis donne **seulement** la tâche, les fichiers permis et le critère de réussite.

## Règles communes

- Lire `CLAUDE.md` puis l'ADR de la tâche. Ne pas relire le reste du dépôt sans raison.
- Ne **jamais** commiter, ni supprimer de fichier hors de `target/` (sauf suppression bornée prévue par la tâche).
- Ne toucher qu'aux **fichiers permis** de la tâche. Pour un fichier partagé (`Cargo.toml`, `lib.rs`, `mod.rs`) : édition ponctuelle, jamais de réécriture complète.
- Code de bibliothèque : pas de `unwrap()`, `expect()` ni `unsafe`. Erreurs typées (`thiserror`).
- Commentaires et documentation en français ; un court commentaire pour toute notion Rust non triviale (la propriétaire est débutante en Rust).
- Ordre déterministe : `BTreeMap`, jamais d'itération sur une `HashMap` pour produire une sortie.
- Un bug corrigé arrive avec un test qui le reproduit.
- Jamais de jeu, de ROM, d'asset ou de clé dans le dépôt. Jamais de téléchargement ni d'exécution de fichier téléchargé sans accord explicite de l'utilisatrice ; les téléchargements vont dans un dossier neuf et vide du scratchpad.
- Les builds partagent le même `target` : `CARGO_TARGET_DIR=target/dev-agents`. Si la compilation échoue à cause d'un fichier qui n'est pas le sien, attendre une minute et recommencer.

## Vérifications

| Moment | Commande |
| --- | --- |
| Pendant l'itération | `cargo test -p <crate> --test <fichier>` |
| Fin de tâche | `cargo fmt --all`, `cargo clippy -p <crate> --all-targets -- -D warnings`, `cargo test -p <crate>` |
| Fin d'étape (une seule fois, par le chef d'orchestre) | `cargo clippy --workspace --all-targets -- -D warnings` et `cargo test --workspace` |

- Tests réels (vrai Python, vrai Archipelago) : `#[ignore]`, variable d'environnement, `--test-threads=1` ; jamais de téléchargement dans un test.
- Sous Windows, un hook réécrit les commandes contenant « python » : si la première tentative est refusée, la réémettre telle quelle.

## Rapports

- **150 mots au plus**, une ligne par fait : ce qui est fait, ce qui passe (commandes), ce qui n'est **pas** vérifié, les écarts par rapport à la consigne.
- Pas de recopie du code ni de la liste complète des fichiers : seulement ceux qui sortent du périmètre.
- Un relecteur classe ses points : bloquant / à corriger / mineur, avec `fichier:ligne` et la correction proposée.

## Découpage d'une étape

1. **Conception** (architecte, lecture seule) : approche, contrat (signatures, erreurs, formats), tâches, risques à vérifier (sécurité, concurrence, API publique).
2. **Validation** par l'utilisatrice des décisions hors SPEC (numérotées, avec une recommandation chacune), puis ADR.
3. **Test réel d'abord** quand l'étape dépend d'un programme externe (Python, Archipelago, un jeu) : le contrat se vérifie avant d'écrire le code qui le suppose.
4. **Tâches** : une tâche = un critère de réussite. Les tâches parallèles n'ont aucun fichier en commun ; le contrat qu'elles partagent est figé avant.
5. **Vérification** : un testeur ; un relecteur seulement pour le code sensible (archives, process, chemins, suppressions, concurrence). Un petit module se relit en une passe.
6. **Corrections** : un seul développeur, avec un test par correction, puis une vérification finale.
7. **Clôture** : mettre à jour `docs/etat-du-projet.md`, puis commiter par étape.

## Choix du modèle

- Modèle léger : lancer fmt / clippy / tests, mettre à jour un ADR ou l'état du projet.
- Modèle fort : conception, relecture de sécurité, tâches avec concurrence ou protocole.
