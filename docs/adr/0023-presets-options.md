# ADR 0023 — Presets « Rapide / Standard / Long » (F06)

- **Statut** : proposé

## Contexte

F06 demande des presets avant tout éditeur avancé. `GenerateRequest` n'expose que `seed`, `spoiler`, `race` et `timeout` : aucun réglage générique de durée ou de progression. Les options de jeu (`progression_balancing`, `accessibility`, options propres à un apworld) vivent dans les YAML des joueurs, que Raftup ne réécrit pas.

## Décision proposée

1. **Sous-ensemble livré** : un preset ne règle que `GenerateRequest.timeout` (Rapide 5 min, Standard 15 min = défaut, Long 45 min). Module pur `screens/presets.rs`, sélecteur dans « Créer une partie ». Standard ne change rien.
2. **Suite, à valider** : faire accepter par le worker un `meta.yaml` (réglages communs à tous les joueurs : `progression_balancing`, `accessibility`), généré par le launcher à partir du preset. Rapide = progression plus équilibrée et accès minimal ; Long = l'inverse. Rien de propre à un jeu, aucune modification des YAML des joueurs.
3. Un éditeur avancé (YAML par joueur) reste hors périmètre.

## Conséquences

- Aujourd'hui, le preset n'agit pas sur la longueur de la partie, seulement sur le délai de génération : à dire à l'utilisatrice avant de s'en servir comme argument produit.
- L'étape 2 demande un changement dans `py-bridge` / `generation` (contrat du worker) : décision de la mainteneuse nécessaire.
