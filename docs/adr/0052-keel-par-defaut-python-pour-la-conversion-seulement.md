# 0052 — Keel par défaut, Python seulement pour convertir (Ferry)

- Statut : **accepté le 23 novembre 2026** (choix de l'utilisatrice, mise en œuvre à faire par étapes)
- Date : 2026-11-23
- Modifie la SPEC § 3 (« Génération au MVP », « Process Python ») et § 4 (V2). Prolonge les ADR 0027 (générateur Rust), 0041 (format de monde natif), 0042 (conversion apworld ↔ natif) et 0046 (mondes natifs dans la Bibliothèque).
- Les ADR 0028 et 0029 (moteur « export ») et 0034 (bac à sable) restent valables pour la conversion, plus pour la génération.

## Contexte

- Aujourd'hui, une partie est générée par le générateur officiel d'Archipelago dans un process Python (par défaut), par le moteur « export » (mondes Python exportés, placement par Raftup) ou par le moteur Rust (case « Moteur Rust (expérimental) », décochée).
- Le moteur Rust sait déjà générer APQuest, Slime Rancher converti (sur le PC, ADR 0042 § 6) et The Planet Crafter natif (ADR 0048), sans Python.
- L'utilisatrice veut que le moteur Rust soit **le** moteur, que Python disparaisse de la génération, et qu'un apworld d'Archipelago puisse être **converti** en un vrai monde Raftup.
- Limite technique : un apworld est du code Python ; ses règles sont des fonctions. Le convertir demande de l'exécuter une fois (export de l'ADR 0028, puis `generate::convert`). Écrire un interpréteur Python en Rust est hors de portée.

## Décision

### 1. Noms techniques

| Nom | Ce que c'est | Dans le code |
| --- | --- | --- |
| **Keel** | Le moteur de génération de Raftup (la quille du radeau) | crates `logic`, `fill`, `generate` (noms de crates inchangés) |
| **Raftworld** | Un monde de jeu Raftup : le format natif déclaratif (ADR 0041) | fichier `world.toml` ; paquet `.raftworld` (le zip de l'ADR 0046 § 2) |
| **Ferry** | Le convertisseur apworld → Raftworld (il fait traverser les mondes) | binaire `raftup-ferry` (ex-`raftup-monde`), module `generate::convert` |
| **Knot** | Le mini-langage des règles d'un Raftworld | `worlds_native::expr` |

- Ces noms sont ceux de l'interface et de la documentation. Les crates gardent leur nom (renommer coûte beaucoup et n'apporte rien au joueur).
- Aucun ne contient « Archipelago » ni « AP ». Le texte reste « compatible avec Archipelago ».

### 2. Keel est le seul moteur de génération

- « Créer une partie » génère toujours avec Keel. La case « Moteur Rust (expérimental) » disparaît, ainsi que le moteur « export » et le repli sur le générateur officiel **à la génération**.
- Le résultat reste une seed `.archipelago` que le serveur Raftup héberge : les mods de jeu Archipelago se connectent toujours sans modification.
- Un joueur dont le jeu n'a pas de Raftworld ne peut pas être ajouté à l'équipage ; le message dit comment en obtenir un (registre ou Ferry).

### 3. Python ne sert plus qu'à Ferry, en deux endroits

- **Chez l'auteur ou dans la CI du registre** : `raftup-ferry` convertit un apworld, l'auteur relit les points à vérifier et écrit les `when` des options (ADR 0042 § 1), puis publie le Raftworld au registre (signé, niveau de confiance par `raftup-registry check`).
- **Chez le joueur**, pour un apworld qui n'est pas encore sur Raftup : « Convertir un apworld » (ADR 0046 § 5) reste dans la Bibliothèque. Python n'est téléchargé qu'à ce moment-là, avec le même consentement, le même bac à sable et le réseau bloqué. Le Raftworld obtenu est un instantané des options par défaut, marqué « Converti sur ce PC », avec la liste des points à vérifier.
- Un joueur qui n'a que des jeux du registre n'installe jamais Python.
- Python reste aussi l'oracle des tests (différentiels du serveur, contrôle croisé de Ferry), hors du launcher livré.

### 4. Catalogue : jeux sans Raftworld masqués

- Un profil dont le jeu n'a pas de Raftworld (ni au registre, ni installé, ni converti sur ce PC) n'apparaît plus dans le launcher. Son fichier reste dans `profiles/`.
- Il réapparaît dès qu'un Raftworld de son jeu est disponible.
- Aujourd'hui visibles : Slime Rancher (une fois converti sur le PC : son apworld n'a pas de licence, ADR 0021), The Planet Crafter. Masqués : A Short Hike, Aquaria, Blasphemous, Bomb Rush Cyberfunk, Choo-Choo Charles, Hollow Knight, Risk of Rain 2, Stardew Valley.

## Mise en œuvre, par étapes (une session par étape)

1. **Noms** : binaire `raftup-ferry`, extension `.raftworld` acceptée (le zip et le `world.toml` seul restent lus), textes de l'interface (« Raftworld », « Keel », « Ferry »).
2. **Keel par défaut** : retrait de la case et du chemin de génération par `py-bridge` dans « Créer une partie » ; erreurs de Keel affichées telles quelles.
3. **Catalogue filtré** : un profil n'est visible que si un Raftworld de son jeu est disponible ; test qui le vérifie.
4. **Ferry chez le joueur** : Python installé à la demande seulement par « Convertir un apworld » ; étiquette « Converti sur ce PC ».
5. **Ménage** : retrait du moteur « export » et du repli Python de `launcher-core::generation` ; SPEC et état du projet à jour.

## Conséquences

- Plus de Python au premier lancement : installation plus légère et plus rapide, moins de surface d'attaque.
- Génération plus rapide, reproductible sur les trois systèmes (NF12), erreurs expliquées par Keel.
- **Perte immédiate** : 8 jeux du catalogue ne sont plus jouables tant qu'ils ne sont pas convertis. Certains mondes (Stardew Valley, Hollow Knight) ont une logique que Ferry ne sait pas encore traduire (règles d'objet, `has_from_list`, entrées mélangées : ADR 0042 § 5) ; il faudra de nouvelles versions du format Raftworld.
- Une conversion faite sur le PC du joueur ne couvre que les options par défaut : le YAML ne peut pas régler les options de ce jeu tant que l'auteur n'a pas écrit les `when`.
- Une seed Keel n'est pas la seed qu'Archipelago aurait produite avec la même graine ; ce n'était déjà pas le cas avec le moteur Rust.
- `py-bridge` et le runtime Python restent dans le code, mais servent seulement à Ferry.
