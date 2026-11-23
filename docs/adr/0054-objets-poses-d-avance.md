# 0054 — Objets posés d'avance dans un lieu à numéro (format de monde natif, version 3)

- Statut : **accepté** le 23 novembre 2026 par l'utilisatrice (option A) ; codé le même jour
- Date : 2026-11-23
- Prolonge l'ADR 0041 (format natif), l'ADR 0042 (conversion, § 5 : « version 3 quand un vrai apworld en aura besoin »), l'ADR 0028 (export et moteur « export ») et l'ADR 0053 (lots de conversion)

## Contexte

### Ce que fait Archipelago

Un monde Python peut poser lui-même un objet dans un de ses lieux, avant le remplissage : `location.place_locked_item(item)`. Le lieu est alors **verrouillé** : le remplissage n'y touche pas, l'équilibrage de la progression ne l'échange pas, et l'objet n'est pas dans le pool. Deux sortes de lieux le font :

- **sans numéro** (`address = None`) : c'est un événement. Le format natif les connaît déjà (`[[events]]`, versions 1 et 2) ;
- **avec numéro** : un vrai lieu du datapackage, que le mod coche, qui porte un vrai objet (avec un numéro) toujours le même. Le serveur l'envoie au joueur comme n'importe quel objet. Archipelago interdit le cas mixte (lieu à numéro avec un objet sans numéro : `assert` dans `Main.py`).

Exemples relevés dans les sources d'Archipelago 0.6.8 :

| Jeu | Lieu → objet posé |
| --- | --- |
| DOOM 1993, DOOM II, Heretic | « E1M1 - Exit » → « E1M1 - Complete » (chaque niveau) |
| Meritous | « Place of Power » → « Cursed Seal », « The Last Place You'll Look » → « Agate Knife » |
| Aquaria | objets de départ posés à leur place d'origine (`__pre_fill_item`) |
| Celeste 64, Celeste (Open World) | points de passage → objet du même nom ; « House Keys » au lieu de l'objectif |
| Faxanadu, Landstalker, Mario & Luigi | boutiques (potions, EkeEke, champignons…) |
| Super Mario 64 | « BoB: Bob-omb Buddy » → « Cannon Unlock BoB » (sans `buddy_checks`) |
| Dark Souls III | certains objets posés dans un lieu **tiré au hasard** (`self.random.choice`) |

### Ce que fait Raftup aujourd'hui

- **Export (ADR 0028)** : chaque lieu est exporté avec `locked` et son `item` ; le pool exporté n'a pas les objets posés. Rien à changer.
- **Moteur « export »** (`generate::imported`) : il pose déjà ces objets (`world.place`) avant le remplissage. Le placement (`fill::fill`) ne remplit que les lieux vides : il ne les touche pas.
- **Convertisseur** (`generate::convert`) : refuse tout lieu à numéro qui porte un objet (« le format 1 ne connaît que les événements »). C'est la **première cause d'échec de 24 mondes, et la seule pour 14** (Aquaria, DOOM 1993, DOOM II, Dark Souls III, Heretic, Meritous, Noita ; Celeste 64 et Celeste (Open World), licence restrictive ; Castlevania - Circle of the Moon, Faxanadu, Landstalker, Mario & Luigi Superstar Saga, Super Mario 64, à ROM). Un 25ᵉ (Super Mario Land 2) échoue sur un cas voisin : un objet d'événement qui porte le nom d'un objet à numéro.
- **Format natif** : un `[[locations]]` n'a pas de champ `item` ; un `[[events]]` crée toujours un objet nouveau (le nom d'un objet de `[[items]]` y est refusé).
- **`logic` et `fill`** : `logic::Location` sait déjà porter un objet dans un lieu à numéro (`address` et `item` sont indépendants), le serveur et `multidata` l'envoient sans rien changer. Mais **les deux équilibrages** (`fill::balance` et `fill::time_balance`) prennent pour « verrouillé » un lieu **sans numéro** seulement : un objet posé dans un lieu à numéro peut être échangé avec un objet d'un autre joueur. C'est déjà un **bogue du moteur « export »** : sur une partie avec DOOM, « E1M1 - Complete » peut partir chez un autre joueur si l'équilibrage est allumé.

## Options

### A. Un champ `item` sur `[[locations]]`, en version 3 (recommandée)

```toml
format_version = 3

[[locations]]
name = "E1M1 - Exit"
id = 360000
region = "Hangar (E1M1) Main"
rule = 'has("Hangar (E1M1)")'
item = "E1M1 - Complete"     # objet posé d'avance : toujours ici, jamais dans le pool
```

- `item` nomme un objet de `[[items]]` (à numéro, du même monde). Il accepte la même forme que `region` : un nom, ou une liste de cas `{ when, value }` (un monde dont l'objet posé dépend d'une option l'écrit sans dupliquer le lieu).
- Le lieu est construit, puis l'objet est **posé et verrouillé** ; il n'entre ni dans le pool ni dans le compte des lieux à remplir (`[filler]` comble le reste, comme aujourd'hui).
- C'est ce que fait Archipelago, champ pour champ. L'export en apworld écrit `location.place_locked_item(self.create_item(...))`.

### B. Une table à part `[[placed]] location = …, item = …`

Même effet, mais l'information est loin du lieu, et il faut vérifier à part qu'un lieu n'est posé qu'une fois. Plus lourd à lire pour un auteur, sans rien de plus.

### C. Ne rien ajouter : mettre l'objet dans l'inventaire de départ et marquer le lieu `excluded`

Faux : l'objet arriverait dès le début au lieu d'être reçu en cochant le lieu (le DOOM de Raftup ouvrirait tous les niveaux), et le lieu recevrait un objet de remplissage que le mod n'attend pas. Écarté.

## Décision proposée (option A)

### 1. Format, version 3

- `FORMAT_VERSION` passe à 3 ; `MIN_FORMAT_VERSION` reste 1.
- Nouveau champ facultatif `item` sur `[[locations]]`, **à partir de la version 3** ; écrit en version 1 ou 2, il est refusé avec un message qui cite la version 3 (comme le format 2 l'a fait pour les événements partagés).
- Refusé : objet inconnu, objet d'événement (« un lieu à numéro porte un objet à numéro », comme le veut Archipelago), objet d'un autre monde.
- Permis : un lieu posé avec `progress = "excluded"` ou `"priority"` (Archipelago l'accepte ; l'objet posé passe outre) ; plusieurs lieux qui posent le même objet (boutiques de Faxanadu) ; un objet posé qui figure aussi dans le pool.
- **Événement qui donne un objet à numéro** (cas de Super Mario Land 2) : en version 3, `[[events]] item` peut nommer un objet de `[[items]]`. L'événement en pose un exemplaire, compté par la logique comme les autres ; il n'est jamais envoyé (un événement n'a pas de numéro). C'est exactement ce que fait Archipelago, où les objets se comptent par leur nom.

### 2. Compatibilité

- Un fichier de version 1 ou 2 est lu **exactement comme avant** : aucun champ n'y change de sens.
- Le convertisseur écrit la **plus petite version** qui suffit : 1, puis 2 si un objet d'événement est partagé, puis 3 si un lieu à numéro porte un objet ou si un événement donne un objet à numéro. Les 16 mondes déjà convertis gardent leur version.
- Un Raftup plus ancien refuse un fichier de version 3 avec le message existant (« format_version inconnu ») : c'est voulu, il ne saurait pas poser l'objet et donnerait un monde faux.
- La Bibliothèque affiche déjà `format_version` (rien à changer). Le registre (ADR 0046) ne regarde pas la version : un paquet de version 3 se publie comme les autres.

### 3. Logique et placement

- `logic::Location` gagne un champ `locked: bool`, et `MultiWorld` une méthode `place_locked(lieu, objet)` qui pose et verrouille. Les événements sont posés avec elle aussi (un événement est toujours verrouillé).
- `fill::balance` et `fill::time_balance` ne déplacent plus un lieu `locked` (aujourd'hui : « sans numéro »). Comme chez Archipelago (`Fill.py`, qui écarte les lieux `locked` de ses comptes), un lieu verrouillé à numéro reste dans les sphères mais ne compte pas dans le total de lieux de son joueur ; pour l'équilibrage par le temps, il prend quand même du temps à visiter (il a un numéro).
- `fill::fill` ne change pas : il ne remplit que les lieux vides, et les objets posés comptent déjà dans la logique.
- Le moteur « export » (`generate::imported`) pose avec `place_locked` les lieux que l'export marque `locked` : **corrige le bogue** de l'équilibrage décrit plus haut, avec un test qui le reproduit (deux joueurs, équilibrage à 99, un objet posé de progression qui ne doit pas bouger).
- `worlds_native::build` pose les `item` des lieux avec `place_locked`, avant de compter les lieux à remplir.

### 4. Convertisseur

- Un lieu à numéro avec un objet du même joueur devient `[[locations]] … item = "…"` ; le lieu n'entre pas dans le compte « pool = lieux » (les objets posés ne sont pas dans le pool exporté).
- La classification des objets est prise aussi sur les objets posés (aujourd'hui : pool et départ seulement) : un objet qui n'existe que posé (« E1M1 - Complete ») n'est plus écrit `filler` par défaut, et l'union des drapeaux s'applique (Faxanadu pose des « Progressive Shield » de progression).
- Toujours refusés : objet posé d'un autre joueur (aucun monde intégré ne le fait), objet sans numéro dans un lieu à numéro (impossible chez Archipelago).
- Nouveau point « À vérifier » `pose`, par jeu : « N objets posés d'avance : vérifier qu'ils ne dépendent pas des options ou du hasard ». C'est un instantané (ADR 0042, § 1) : DOOM pose un objet par niveau **des épisodes choisis**, Dark Souls III tire certains lieux au hasard.

### 5. Inventaire : repérer les objets posés au hasard

La conversion est déterministe. `inventaire_real` convertira donc chaque jeu **avec deux seeds** (1 et 2, mêmes options) et comparera les deux textes : s'ils diffèrent, le monde tire quelque chose au hasard pendant la génération (objet posé, pool, pièges), et le jeu est marqué `hasard` dans le tableau. Coût : une seconde exportation par jeu (environ 7 minutes de plus pour les 81).

### 6. Export en apworld et aller-retour

`worlds_native::apworld` écrit, pour un lieu qui a un `item`, `location.place_locked_item(w.create_item(...))` après sa création (dans le `if` de son `when`, cas par cas comme pour `region`). Le monde Python obtenu repasse au contrôle croisé de l'inventaire comme les autres.

## Conséquences

- En bien : jusqu'à **14 mondes** de plus se convertissent si rien ne se cache derrière cette cause (7 jeux PC, dont DOOM 1993, DOOM II, Heretic et Meritous) ; 10 autres passent à leur cause suivante (règles opaques). Le moteur « export » cesse d'échanger des objets posés.
- En bien : un auteur de monde natif écrit enfin un objet vanilla, une boutique fixe ou une fin de niveau sans tricher avec un événement.
- En moins bien : trois crates touchées (`logic`, `fill`, `worlds-native`) en plus de `generate` ; un champ de plus dans `logic::Location` (tous les constructeurs de tests qui écrivent un `Location` à la main devront le remplir).
- En moins bien : un monde converti qui pose des objets d'après ses options reste un instantané faux pour d'autres options tant que l'auteur n'a pas écrit les `when` : le point `pose` et la comparaison à deux seeds le disent, ils ne le corrigent pas.
- Hors de cet ADR : les règles d'objet, `has_from_list`, `at_least` et les compteurs (lots 2 et 3) ; ils ouvriront une version 4 ou entreront en version 3 s'ils sont validés avant la publication d'un monde en version 3.

## Résultat (23 novembre 2026)

- Inventaire relancé sur les 81 mondes : **29 convertis** (16 avant), dont 21 jeux PC sur 49 ; seed Rust et aller-retour réussis pour les 29 (`docs/inventaire-conversion.md`).
- 13 des 14 jeux visés passent. Aquaria bute sur une cause qui était cachée : son pool a plus d'objets que de lieux (Archipelago laisse le surplus non placé).
- Celeste 64 et Celeste (Open World) demandaient en plus une correction du convertisseur : une règle qui cite un objet jamais créé (« CANNOT ACCESS ») est écrite `false`, comme Archipelago le compte (point « À vérifier » `jamais`).
- La seconde seed montre que Dark Souls III, Faxanadu et Noita tirent leurs objets posés au hasard : leur instantané fige un tirage.

## Plan de travail, une fois validé

1. `logic` : `locked`, `place_locked` ; `fill` : les deux équilibrages ; test du bogue (moteur « export »).
2. `worlds-native` : version 3, `item` sur les lieux et objet à numéro dans les événements, build, apworld ; tests de lecture (refus en version 2, refus d'un objet d'événement, cas `when`) et de construction.
3. `generate::convert` : écriture, classification, point `pose`, version minimale ; tests dans `generate/tests/conversion.rs`.
4. `inventaire_real` : seconde seed et colonne `hasard` ; relance sur les 81 mondes, mise à jour de `docs/inventaire-conversion.md`.
