# ADR 0042 — Niveaux de confiance des paquets du registre, attribués par la CI

- **Statut** : accepté en autonomie le 20 novembre 2026, à relire par l'utilisatrice

## Contexte

La SPEC (V3) demande des « niveaux de confiance des paquets attribués par CI (tests, fuzzing) ». Le registre signé de la V2 (ADR 0032) prouve qu'un index vient de la mainteneuse et qu'un fichier téléchargé est bien celui qu'elle a relu (sha256). Il ne dit rien de ce que fait le paquet : une apworld qui plante à la génération, ou qui boucle sur une option rare, a la même place dans l'index qu'une apworld solide.

On veut un repère simple pour le joueur, calculé par une machine de façon reproductible, et impossible à falsifier sans la clé du registre.

## Décision

### Quatre niveaux, chacun inclut le précédent

| Niveau (`trust.level`) | Nom affiché | Ce que la CI a vérifié | Ce que ça ne prouve pas |
| --- | --- | --- | --- |
| `unverified` | Non vérifié | rien (paquet publié sans rapport, ou rapport d'un autre fichier) | — |
| `valid` | Structure valide | apworld : zip lisible, chemins sûrs (pas de `..`, pas de chemin absolu), un seul dossier `<name>/` avec `__init__.py`, `archipelago.json` lisible s'il est là, tailles bornées ; profil : `Profile::parse` passe et `game.id` = `name` | que le code Python fasse quoi que ce soit d'utile ou d'honnête |
| `generates` | Générable | le générateur officiel épinglé (ADR 0017), dans son bac à sable (ADR 0034), génère une partie solo avec les options par défaut sur **N graines** (5 par défaut) sans erreur | que les autres combinaisons d'options marchent, que la partie soit finissable en vrai, que le mod du jeu soit compatible |
| `fuzzed` | Testé en profondeur | en plus : **K cas de fuzzing** (40 par défaut) tirés à graine fixe — options valides au hasard, valeurs limites ou hors bornes, YAML abîmés — sans **plantage** (process Python mort, réponse illisible, erreur interne du worker) ni **boucle** (délai par cas dépassé) | que le code soit sain (aucun audit de sécurité), que chaque combinaison valide génère (un refus propre du générateur est permis et noté dans le rapport) |

Les niveaux 2 et 3 ne concernent que les apworlds : un profil s'arrête à `valid` (l'apworld qu'il installe a son propre niveau).

Aucun niveau ne remplace la relecture humaine : le bac à sable limite ce qu'une apworld peut faire pendant la vérification, il ne rend pas son code digne de confiance. Le texte du launcher le dit.

### Où le niveau est rangé : dans l'index signé (format 2)

- Chaque paquet de `index.json` gagne un champ `trust` : `{ "level": "generates", "detail": "Archipelago 0.6.8 · 5 graines · …" }` (`detail` facultatif, texte court affiché en survol).
- Le niveau est couvert par la signature minisign de l'index : le changer sans la clé du registre fait refuser tout l'index.
- `format_version` passe à **2**. Le launcher lit les formats 1 et 2 : un index 1 est relu comme un index 2 dont tous les paquets sont `unverified` (et un index 1 qui contiendrait déjà `trust` est refusé). `raftup-registry publish` écrit toujours le format 2. Un launcher qui ne connaît que le format 1 refuserait un index 2 (« Registre indisponible ») : aucun launcher n'a encore été publié, ce n'est donc pas un problème aujourd'hui ; à l'avenir, un nouveau format attendra que les launchers en circulation sachent le lire.

### Comment la CI le calcule : `raftup-registry check`

- Le binaire `raftup-registry` passe de la crate `registry` à une nouvelle crate **`registry-tool`** (MIT OR Apache-2.0) : `check` a besoin de `launcher-core` (générateur épinglé, bac à sable, téléchargements), qui dépend lui-même de `registry` ; Cargo refuse ce cycle. `registry` reste la bibliothèque du format, de la vérification et de la signature.
- `raftup-registry check <source.toml> <dossier-du-rapport> [--paquet <nom>] [--telecharger] [--graines N] [--fuzz K] [--limite <secondes>] [--delai-cas <secondes>] [--donnees <dossier>] [--structure-seule]` :
  1. prend chaque paquet du fichier source, depuis son fichier local, ou, avec `--telecharger`, chez son auteur (`url`, hôtes de la liste blanche, taille bornée) : la CI n'a pas les copies locales ;
  2. calcule son sha256, puis les vérifications dans l'ordre des niveaux ; la première qui échoue arrête la montée ;
  3. écrit `rapport.json` (format versionné, `format_version = 1`) et `rapport.md` (lisible), et affiche le résumé.
- Génération : `launcher_core::generation::generate` avec `PythonBackend` (réseau bloqué, bac à sable), une partie solo `name: Testeur`, l'apworld posée dans `custom_worlds` comme dans « Créer une partie ». Chaque génération a son délai (`--delai-cas`, 300 s par défaut) ; son dossier de travail est effacé après lecture.
- Fuzzing : le worker gagne une méthode `describe_options` (liste des options du jeu : interrupteur, choix, intervalle, autre). Les cas sont tirés par un générateur pseudo-aléatoire écrit dans Raftup (splitmix64), à graine fixe : la même version de l'outil et du paquet donne les mêmes cas. Une **limite de temps totale** (`--limite`, 30 min par défaut) arrête le fuzzing ; s'il n'a pas fini, le niveau reste `generates` et le rapport le dit.
- Classement d'un cas : réussite ; refus propre (exception du générateur, code -32010) ; plantage (process mort, JSON ou réponse illisible, erreur interne) ; boucle (délai dépassé). Une erreur de l'environnement (téléchargement du runtime, disque, bac à sable impossible) n'est pas la faute du paquet : `check` s'arrête en erreur sans attribuer de niveau.
- Hors Windows, le runtime Python n'existe pas (ADR 0017) : le niveau s'arrête à `valid`, avec la raison dans le rapport. Le workflow tourne donc sous Windows.

### Publication : `raftup-registry publish … --rapport rapport.json`

- `publish` prend en option le `rapport.json` produit par la CI. Pour chaque paquet, il reprend le niveau du rapport **seulement si** sorte, nom, version et sha256 sont identiques au fichier local publié ; sinon le paquet est `unverified`, avec un avertissement. Sans rapport, tout est `unverified`.
- La signature reste locale, chez la mainteneuse, avec sa clé (ADR 0032). La CI ne signe ni ne publie rien.

### Workflow `.github/workflows/registre.yml`

- Déclenché par une modification de `registre/**` (push sur `main` et pull request) ou à la main (`workflow_dispatch`).
- Sous Windows : `cargo run -p registry-tool --bin raftup-registry -- check registre/registre.toml rapport --telecharger`, puis envoi de `rapport/` en artefact (`actions/upload-artifact`).
- Permissions en lecture seule, aucun secret, aucune publication. La mainteneuse télécharge l'artefact, le relit, puis lance `publish` chez elle.

### Dans le launcher

- « Mondes du registre » (Bibliothèque) : un badge par apworld (`Non vérifié`, `Structure valide`, `Générable`, `Testé en profondeur`), couleur croissante (gris, ambre, bleu, vert), avec une explication courte au survol (ce que le niveau prouve, puis `detail`).
- Avant d'installer un paquet de niveau `unverified` ou `valid`, une confirmation : « Ce monde n'a pas été généré par la CI de Raftup… Installer quand même ? ». Les niveaux `generates` et `fuzzed` s'installent directement.

## Conséquences

- Un paquet publié avant ce changement, ou sans rapport, s'affiche « Non vérifié » : rien ne monte de niveau sans passer par la CI puis par la signature.
- La confiance repose toujours sur la mainteneuse : elle signe le niveau que la CI a calculé. Le rapport est un artefact GitHub, pas un document signé ; l'ADR demande de prendre celui du run sur `main`.
- Coût : un monde moyen prend de l'ordre de 10 s par génération ; 5 graines et 40 cas font quelques minutes de CI par paquet, bornées par `--limite`.
- Reproductibilité : les cas de fuzzing sont fixes ; le résultat peut quand même varier si Archipelago lui-même n'est pas déterministe (`PYTHONHASHSEED=0` est posé par le pont).
- Nouvelle crate `registry-tool`, nouvelle méthode du worker (`describe_options`) ; aucune nouvelle dépendance hors workspace.
