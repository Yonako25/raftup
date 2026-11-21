# Tests de l'interface du launcher

Les écrans du launcher sont testés **sans ouvrir de fenêtre** avec [egui_kittest](https://docs.rs/egui_kittest) :
l'écran est dessiné en mémoire, puis le test cherche un bouton ou un texte par son libellé, clique et tape comme un joueur.

## Où sont les tests

`launcher-ui` est un binaire sans `lib.rs` : un fichier de ce dossier `tests/` ne verrait pas ses écrans.
Les tests d'interface sont donc dans `src/ui_tests/`, compilés seulement pour `cargo test` :

| Fichier | Ce qui est vérifié |
| --- | --- |
| `salon.rs` | Salon de préparation : affichage, ajout d'un joueur, pseudo en double refusé, refus annoncé par « Lancer la partie » |
| `bibliotheque.rs` | Bibliothèque avec le catalogue de `profiles/` : carte et fiche du jeu, « Se connecter à la main » replié |
| `ecrans.rs` | Paramètres, Rejoindre (et un lien illisible), Amis sans compte |
| `menu.rs` | Barre latérale : aucune entrée « Bientôt », un clic ouvre l'écran |
| `parcours.rs` | Accueil, Catalogue, Mon profil, assistant du premier lancement, « Lancer et connecter » de la partie hébergée |
| `captures.rs` | Captures de référence du Salon et de la Bibliothèque (images) |

Ces tests n'ouvrent aucune fenêtre, n'écoutent sur aucun port, ne lisent ni le presse-papiers ni Steam et n'appellent pas le réseau.

## Lancer les tests

Depuis **PowerShell** (pas Git Bash : son `link.exe` n'est pas celui de Visual Studio) :

```powershell
cargo test -p launcher-ui ui_tests            # tous les tests d'interface
cargo test -p launcher-ui ui_tests::salon     # un seul fichier
```

Ils tournent aussi avec `cargo test --workspace` et dans la CI (Windows, macOS, Linux).

## Les captures de référence

`captures.rs` dessine le Salon et la Bibliothèque en image (rendu wgpu : carte graphique, ou moteur logiciel WARP de Windows)
et les compare aux PNG de `tests/snapshots/`.
Ce test ne tourne que **sous Windows** : les références y ont été faites, et le rendu change un peu d'un système à l'autre.
Ailleurs, il est marqué « ignoré » ; on peut le forcer avec `cargo test -p launcher-ui captures -- --ignored`.

Si une image ne correspond plus, le test échoue et laisse à côté de la référence :

- `<nom>.new.png` : l'image du jour ;
- `<nom>.diff.png` : les pixels qui diffèrent.

Ces deux fichiers ne sont jamais commités (`.gitignore`).

### Mettre à jour les images de référence

Après un changement voulu de l'interface (ou de la version de Raftup, écrite dans le Salon) :

```powershell
$env:UPDATE_SNAPSHOTS = "1"
cargo test -p launcher-ui captures
Remove-Item Env:UPDATE_SNAPSHOTS
```

Regarder les nouveaux `tests/snapshots/*.png` avant de les commiter : une capture de référence fausse rend le test inutile.
