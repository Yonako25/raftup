# ADR 0053 — Mondes d'Archipelago en natif : périmètre, licence, repli et badge

- Statut : **accepté** le 23 novembre 2026 par l'utilisatrice, avec les 4 recommandations de l'inventaire ; rien n'est codé
- Date : 2026-11-23
- Prolonge l'ADR 0041 (format natif), l'ADR 0042 (conversion), l'ADR 0046 (mondes natifs dans le registre) et l'ADR 0028 (moteur « export », repli Python)

## Contexte

- L'objectif est que les jeux pris en charge par Archipelago aient un monde « Converti » ou « Natif », générés par le moteur Rust sans Python.
- L'inventaire du 23 novembre 2026 (`docs/inventaire-conversion.md`) a mesuré les 81 mondes intégrés d'Archipelago 0.6.8 : 14 convertis, tous avec des points à vérifier, et 67 échecs.
- 32 jeux demandent une ROM ou un ISO. Pour eux, le fichier que lit le jeu (le patch) est écrit par le code Python du monde (`generate_output`), que le format natif ne décrit pas.
- Les mondes du dépôt d'Archipelago sont sous MIT. 12 ont en plus leur propre fichier de licence, dont 4 (PoryGone) sous une « Modified MIT » qui interdit la relicence et la vente sans accord écrit.
- Une conversion est un instantané des options par défaut, sans `slot_data` (ADR 0042, § 1).

## Décision

1. **Périmètre** : l'objectif « tous convertis » vise d'abord les **49 jeux PC** (sans ROM ni ISO). Les jeux à ROM ou ISO gardent le générateur Python, au mieux le moteur « export » (placement par Raftup). Ils ne comptent pas dans l'objectif.
2. **Licence et distribution** : un monde converti est publié comme paquet du registre (ADR 0046) **sous la licence de son monde d'origine**, pas sous la double licence MIT / Apache-2.0 de Raftup. Le paquet contient la notice MIT d'Archipelago et le fichier `LICENSE` du monde s'il en a un. Les 4 mondes « Modified MIT » de PoryGone (Celeste 64, Celeste (Open World), Sonic Adventure 2 Battle, Super Mario World) ne sont **pas publiés** sans l'accord écrit de leurs auteurs : conversion sur le PC seulement, comme Slime Rancher (ADR 0042, § 6).
3. **Mondes qui ne se convertissent pas** : le **repli sur le générateur Python reste permanent** (badge « Archipelago »). On étend le format et le sondeur par lots (lots 0 à 3 de l'inventaire). Le portage à la main est réservé à quelques jeux PC très joués, un par un, sur demande.
4. **Badge « Converti »** : un jeu ne l'affiche qu'une fois son monde converti complété : `slot_data` repris, options principales écrites avec des `when`, tirage des pièges vérifié, et une vraie partie jouée avec le mod. Avant, il garde le badge « Archipelago ».

## Conséquences

- L'objectif devient mesurable : 13 jeux PC sur 49 se convertissent aujourd'hui, et l'inventaire se relance après chaque lot (`inventaire_real`).
- Les extensions du format (objets posés d'avance, somme d'exemplaires, `at_least`, compteurs) restent chacune à proposer et valider avant d'être codées, car elles ouvrent une version 3 du format.
- Python reste embarqué dans le launcher, pour les jeux à ROM et les mondes non convertis.
- Le registre doit porter la licence propre à chaque monde publié, pas une licence unique.
- Un monde converti n'apparaît comme « Converti » qu'après un travail par jeu : c'est plus lent, mais le badge promet une partie qui marche.
