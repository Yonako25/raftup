# ADR 0023 — Presets « Rapide / Standard / Long » (F06)

- **Statut** : accepté le 18 novembre 2026 (étapes 1 et 2 faites)

## Contexte

F06 demande des presets avant tout éditeur avancé. `GenerateRequest` n'expose que `seed`, `spoiler`, `race` et `timeout` : aucun réglage générique de durée ou de progression. Les options de jeu (`progression_balancing`, `accessibility`, options propres à un apworld) vivent dans les YAML des joueurs, que Raftup ne réécrit pas.

## Décision proposée

1. **Sous-ensemble livré** : un preset ne règle que `GenerateRequest.timeout` (Rapide 5 min, Standard 15 min = défaut, Long 45 min). Module pur `screens/presets.rs`, sélecteur dans « Créer une partie ». Standard ne change rien.
2. **`meta.yaml` (fait)** : `GenerateRequest.meta_yaml: Option<String>` ; `generate` l'écrit tel quel sous `meta.yaml` dans le dossier des joueurs, où `Generate.py` 0.6.8 le lit par défaut (`--meta_file_path`) : le worker Python et `GenerateParams` ne changent pas. Format vérifié dans le source 0.6.8 : `meta_description` obligatoire, catégorie `null:` = tous les jeux, options communes seulement. Valeurs écrasant celles des YAML des joueurs : Rapide `progression_balancing: 80` + `accessibility: minimal` ; Long `20` + `full` ; Standard : aucun `meta.yaml`. Fonction pure `presets::meta_yaml(preset)`, appliquée par `Preset::apply`. Test réel `#[ignore]` : `generation_reelle_avec_meta_yaml` (`RAFTUP_REAL_RUNTIME`), non lancé faute de runtime installé.
3. Un éditeur avancé (YAML par joueur) reste hors périmètre.

## Conséquences

- Aujourd'hui, le preset n'agit pas sur la longueur de la partie, seulement sur le délai de génération : à dire à l'utilisatrice avant de s'en servir comme argument produit.
- Le preset impose ses valeurs à tous les joueurs, même celles qu'un joueur a écrites dans son YAML ; `progression_balancing` et `accessibility` ne sont pas la « longueur » exacte d'une partie.
