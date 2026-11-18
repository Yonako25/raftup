# ADR 0022 — Mise à jour automatique du launcher (F12)

- **Statut** : accepté le 18 novembre 2026

## Contexte

F12 demande que le launcher se mette à jour tout seul, avec une signature minisign. La règle de l'ADR 0015 s'applique : jamais de fichier téléchargé utilisé avant vérification. Cette première tâche ne couvre que le cœur (`crates/launcher-core/src/update.rs`) : lire le manifeste, comparer les versions, télécharger et vérifier. **Rien n'est exécuté ni installé** ; l'application de la mise à jour (remplacement du binaire, redémarrage) et l'écran du launcher viendront ensuite.

## Décision proposée

### Format du manifeste (`format_version = 1`)

JSON de moins de 64 Kio :

```json
{
  "format_version": 1,
  "version": "0.2.0",
  "url": "https://github.com/Yonako25/raftup/releases/download/v0.2.0/raftup-windows-x64.zip",
  "sha256": "<64 caractères hexadécimaux>",
  "max_bytes": 104857600,
  "signature": "<contenu complet du fichier .minisig>",
  "notes": "Texte facultatif affiché au joueur"
}
```

- `version` : semver ; seule une version strictement plus récente est proposée (jamais de retour en arrière, une préversion est plus ancienne que sa version finale).
- `url` : HTTPS, domaine de la liste blanche existante (`profile::ALLOWED_HOSTS`, donc GitHub), ni identifiants ni port. Même règle pour l'adresse du manifeste.
- `max_bytes` : entre 1 octet et 512 Mio ; compté pendant la lecture, pas via `Content-Length`.
- La signature porte sur les octets du fichier téléchargé (minisign pré-haché ; les signatures « legacy » sont refusées).
- Un `format_version` inconnu est refusé : un launcher ancien ne devine pas un format plus récent.

### Vérification

1. Le client HTTP est celui du téléchargeur existant (`ureq`, HTTPS seulement, redirections bornées) : pas de nouvelle pile.
2. `Downloader` contrôle la taille puis le sha256 (protège d'une corruption).
3. La signature minisign est vérifiée avec la clé publique compilée dans le launcher (crate `minisign-verify`) : elle protège d'un compte GitHub ou d'un serveur compromis, qui pourrait changer le fichier **et** le sha256 du manifeste, mais pas signer.
4. En cas d'échec de signature, le fichier est supprimé. Toute vérification échoue si aucune clé publique n'est configurée.

### Où publier

- Le fichier de mise à jour et `update.json` sont des assets de la release GitHub du dépôt ; le launcher lit `https://github.com/<dépôt>/releases/latest/download/update.json`.
- `update.json` est généré et signé par le script de publication (CI ou machine de la mainteneuse), jamais à la main.

### Clé de signature et rotation

- Une paire de clés minisign dédiée ; la clé publique est la constante `update::UPDATE_PUBLIC_KEY` (aujourd'hui vide, à renseigner après génération). La clé privée n'entre jamais dans le dépôt : secret de la CI et sauvegarde chiffrée hors ligne.
- **Rotation** (clé perdue ou compromise) : publier une version intermédiaire signée avec l'ancienne clé, qui embarque la nouvelle clé publique ; les versions suivantes sont signées avec la nouvelle. Une clé compromise ne se révoque que par une nouvelle installation manuelle depuis le site : à prévoir dans la page de téléchargement.
- Option à valider : accepter une liste de deux clés publiques (courante et suivante) pour que la rotation se fasse sans version intermédiaire.

## Conséquences

- Nouvelles dépendances MIT/Apache-2.0 : `minisign-verify` et `semver` (la crate `minisign`, qui sait signer, n'est utilisée que dans les tests).
- Pas de téléchargement en tâche de fond tant que l'utilisatrice n'a pas accepté (à décider dans l'écran de F12).
- Restent à faire : application de la mise à jour (Windows ne permet pas d'écraser le `.exe` en cours), écran du launcher, script de publication, génération de la clé.

## Réponses (18 novembre 2026)

1. Une seule clé publique au MVP ; la rotation passera par une version intermédiaire qui en accepte deux.
2. Version stable seulement, pas de canal bêta.
3. Vérification au démarrage seulement.

## Questions ouvertes (réglées ci-dessus)

1. Une ou deux clés publiques acceptées (rotation sans version intermédiaire) ?
2. Canal « bêta » (manifeste séparé) au MVP, ou seulement la version stable ?
3. Vérification au démarrage seulement, ou aussi périodique ?
