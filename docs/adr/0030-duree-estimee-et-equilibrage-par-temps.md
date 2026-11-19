# 0030 — Durée estimée par joueur et équilibrage par temps de jeu

- Statut : **accepté en autonomie le 19 novembre 2026, à relire par l'utilisatrice**
- Date : 2026-11-19

## Contexte

- La SPEC (V2) demande une « durée estimée par joueur » et un « équilibrage par temps de jeu ».
- Une seed `.archipelago` contient déjà ce qu'il faut pour une estimation grossière : les lieux de chaque joueur et les **sphères** (`spheres` : sphère 0 = ouvert au départ, sphère 1 = ce que la sphère 0 ouvre…). Le moteur Rust les écrit aussi (`generate::multidata_build`).
- Il manque le **temps d'un lieu**, qui dépend du jeu : un coffre d'APQuest prend quelques secondes, un achat de Slime Rancher demande de l'élevage.
- L'équilibrage de progression d'Archipelago (`fill::balance`, ADR 0027 étape 5) compare des **parts de lieux**, pas du temps : un joueur à 50 % de ses lieux est « à jour », même si ses lieux durent dix fois plus longtemps que ceux des autres.

## Décision

### 1. Temps par lieu dans le profil du jeu

- Nouveau champ **facultatif** `game.seconds_per_location` (entier, 1 à 3600) dans `profiles/*.toml`. Absent : **120 s** (`DEFAULT_SECONDS_PER_LOCATION`), un ordre de grandeur.
- `format_version` **inchangé** (1) : un champ facultatif ne casse aucun profil, et les profils sont embarqués dans le launcher (un vieux launcher ne lit jamais un profil récent).
- Slime Rancher : 240 s, valeur estimée (achats et capsules demandent de l'élevage), à mesurer pendant une vraie partie.
- Un jeu sans profil (APQuest) prend la valeur par défaut.

### 2. Estimation depuis la seed (`launcher_core::estimate`)

- Par joueur (les groupes « item link » sont ignorés) :
  - **temps de jeu** = lieux × secondes par lieu (borne basse, sans attente) ;
  - **fin au plus tard** = modèle « synchrone » : la sphère `s` ne s'ouvre que quand **tous** les joueurs ont fini la sphère `s - 1` ; la durée d'une sphère est celle du joueur le plus long dans cette sphère. Un lieu qu'aucune sphère ne cite (seed sans sphères, accessibilité minimale) est compté à la fin. C'est une borne haute.
- Tout en secondes entières (`u64`), sans virgule, dans l'ordre des slots (`BTreeMap`).
- `estimate_file` relit la seed par la lecture sûre de `multidata` (NF08).

### 3. Affichage

- Panneau « Durée estimée » (`launcher-ui/src/screens/estimate.rs`) sur l'écran « Créer une partie », relié par une ligne dans `create.rs`. Il est calculé une fois, quand la seed est prête, et gardé dans la mémoire d'egui : il reste visible pour la dernière partie générée.
- Pas encore dans « Partie en cours » (`running.rs`, fichier d'une autre session) : à ajouter ensuite, avec le même module.

### 4. Équilibrage par temps (`fill::time_balance`), désactivé par défaut

- **Simulation** (`fill::simulate`) : tous les joueurs jouent en même temps, un lieu à la fois, dans l'ordre de création de leurs lieux atteignables ; un lieu du joueur `p` prend `secondes[p]` ; un objet de progression est reçu dès que son lieu est visité ; un événement ne prend pas de temps. Un joueur sans lieu atteignable attend.
- **Échanges** : au plus `max_swaps` (32 par défaut) fois, on prend le joueur qui attend le plus, on cherche un objet de progression à lui visité **après** le début de son attente (16 candidats au plus, du plus tôt au plus tard) et un lieu sans progression visité **avant** (4 au plus, du plus tard au plus tôt). L'échange n'est gardé que s'il fait baisser `(attente totale, heure de fin)` ; sinon il est défait.
- Mêmes garde-fous que `balance` : un lieu prioritaire garde son objet, un lieu exclu ne reçoit pas de progression, `skip_balancing` est respecté, le pool ne change pas, balayage complet à la fin et retour à l'état d'avant si la partie n'était plus jouable.
- Pas de hasard : ordre fixe des candidats, même monde et mêmes réglages ⇒ même placement (NF12).
- **Désactivé par défaut** (`TimeBalance::default().enabled == false`) et **pas encore branché** dans `generate` : il le sera derrière une option (case de l'interface ou option YAML propre à Raftup, à décider), les secondes par slot venant des profils par le launcher (`generate` ne dépend pas de `launcher-core`).

## Conséquences

- L'hôte voit, après la génération, un ordre de grandeur de la durée de chaque joueur et de l'attente possible.
- Les chiffres ne valent que ce que vaut le temps par lieu : sans mesure en vraie partie, c'est une estimation. Le texte du panneau le dit.
- L'équilibrage par temps coûte une simulation par essai (au plus 32 × 16 × 4 simulations, chacune en O(lieux²) au pire) : acceptable pour quelques milliers de lieux, à mesurer avant de l'activer sur une grosse partie.
- Limites connues : la simulation suppose qu'un joueur joue sans pause et visite tous ses lieux ; elle ignore les règles d'objet par lieu (`item_rule`, en cours dans la session `early-items`) : quand elles arrivent dans `logic`, `time_balance` devra refuser un échange qui les viole.
