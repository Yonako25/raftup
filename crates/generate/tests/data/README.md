# Seeds de référence d'APQuest

Seeds produites par le vrai Archipelago 0.6.8 (runtime installé par Raftup), graine `42`,
à partir des YAML des dossiers `yaml-*` :

| Fichier | Options |
| --- | --- |
| `apquest-defaut` | toutes par défaut |
| `apquest-tout` | toutes activées, `trap_chance` 50, sprite `duck` |
| `apquest-mixte` | `hard_mode`, `trap_chance` 100, sprite `kitty` |
| `apquest-deux` | Alice (« tout ») et Bob (par défaut) |

Pour les refaire (l'archive `.archipelago` est dans le zip produit) :

```bash
python -I generer_reference.py <archipelago> <site-packages> yaml-tout <sortie> 42
```

`<archipelago>` et `<site-packages>` : dossiers `generator/0.6.8/…` du runtime de Raftup.
Le test `tests/apquest.rs` compare tout sauf le placement (ADR 0027, § 8).
