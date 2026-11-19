# ADR 0032 — Registre de paquets signés (apworlds et profils)

- **Statut** : accepté en autonomie le 19 novembre 2026, à relire par l'utilisatrice

## Contexte

La SPEC (V2) demande un « registre de paquets avec CLI de publication » : les apworlds sont du code arbitraire, distribué sans signature ni registre (SPEC § 2). L'ADR 0001 garde le catalogue public dans le dépôt et embarqué dans le launcher ; l'ADR 0015 épingle chaque téléchargement par sha256 ; l'ADR 0022 signe déjà les mises à jour avec minisign. Le registre réutilise ces trois briques.

Règle inchangée : Raftup ne redistribue **jamais** de jeu, de ROM, d'asset ni de mod. Le registre ne contient que des **références** (URL chez l'auteur + sha256).

## Décision

### Crate `registry` (MIT OR Apache-2.0)

- Bibliothèque sans réseau : format de l'index, validation, vérification de la signature (`minisign-verify`), contrôle d'un paquet téléchargé (taille + sha256).
- Fonction `publish` et binaire `raftup-registry` derrière la fonctionnalité `publish` (active par défaut) : c'est la seule partie qui dépend de `minisign` (qui sait signer). `launcher-core` prend la crate **sans** cette fonctionnalité.
- Le binaire **n'envoie rien** : il écrit `index.json` et `index.json.minisig` dans un dossier local. La mise en ligne (asset d'une release GitHub du dépôt) reste un geste manuel de la mainteneuse.

### Format de l'index (`index.json`, `format_version = 1`)

```json
{
  "format_version": 1,
  "registry": "raftup",
  "packages": [
    {
      "kind": "apworld",
      "name": "slime_rancher",
      "version": "0.3.2",
      "game": "Slime Rancher",
      "url": "https://github.com/<auteur>/<dépôt>/releases/download/v0.3.2/slime_rancher.apworld",
      "sha256": "<64 caractères hexadécimaux minuscules>",
      "size": 123456,
      "license": "unlicensed",
      "authors": ["<auteur>"]
    }
  ]
}
```

| Champ | Règle |
| --- | --- |
| `format_version` | lu en premier ; une version inconnue est refusée (un launcher ancien ne devine pas un format récent) |
| `kind` | `apworld` ou `profile` ; rien d'autre (jamais `rom`, `asset`, `game`) |
| `name` | `[a-z0-9_-]`, 64 caractères au plus, commence par une lettre ou un chiffre ; pour un profil, égal à `game.id` |
| `version` | texte libre court (`[0-9A-Za-z.+-]`, 32 au plus) : les apworlds n'utilisent pas toujours semver (« 0.3.2.1 ») |
| `url` | HTTPS, sans identifiants ni port, domaine de la liste blanche de l'ADR 0015 (copie dans `registry::ALLOWED_HOSTS`, un test vérifie l'égalité) |
| `sha256`, `size` | obligatoires ; `size` entre 1 octet et 512 Mio |
| `license` | obligatoire (identifiant SPDX, ou `unlicensed` comme l'ADR 0021) |
| `authors` | au moins un ; crédit affiché, pas de lien de confiance |
| `game` | facultatif : nom du jeu Archipelago |

- **Une seule version par paquet** (`kind` + `name`) dans un index, comme la décision 6 de l'ADR 0015. Les paquets sont triés par (`kind`, `name`) : deux publications du même contenu donnent le même fichier, octet pour octet. Pas de date dans l'index.
- Index de 4 Mio et 10 000 paquets au plus ; champs inconnus refusés (`deny_unknown_fields`).

### Signature

- **Une signature pour tout l'index** (minisign pré-haché, signatures « legacy » refusées, comme l'ADR 0022), sur les octets exacts de `index.json`. Chaque paquet est signé **par transitivité** : son sha256 est dans l'index signé. Pas de signature par paquet au MVP du registre.
- Paire de clés **distincte** de celle des mises à jour : une fuite de l'une n'ouvre pas l'autre. `raftup-registry keygen <dossier>` la crée hors du dépôt et refuse d'écraser.
- La clé publique est la constante `launcher_core::registry::REGISTRY_PUBLIC_KEY`, **vide aujourd'hui** : tant qu'elle l'est, toute vérification échoue (`PublicKeyNotConfigured`) plutôt que d'accepter n'importe quoi.
- `publish` relit l'index écrit et le vérifie avec la clé publique tirée de la clé secrète : un index que le launcher refuserait n'est jamais produit.
- Rotation : même règle que l'ADR 0022 (une version du launcher qui embarque la nouvelle clé).

### Publication

`raftup-registry publish <source.toml> <clé-secrète> <dossier-de-sortie>`. Le fichier source liste les paquets avec un champ `file` (chemin local, relatif au fichier source) : l'outil calcule sha256 et taille depuis ce fichier, valide tout, trie, écrit l'index puis la signature (fichier temporaire puis renommage). La clé secrète n'est jamais affichée.

Adresse de publication (décidée le 19 novembre 2026) : une release GitHub du dépôt à étiquette fixe `registre`, distincte des releases du launcher (`launcher_core::registry::REGISTRY_INDEX_URL` = `https://github.com/Yonako25/raftup/releases/download/registre/index.json`). Republier = remplacer les deux fichiers de cette release ; l'adresse ne change pas. Pourquoi : gratuit, déjà dans la liste blanche (ADR 0015), même hébergeur que les mises à jour (ADR 0022) ; une adresse en `latest` obligerait à joindre l'index à chaque release du launcher. L'envoi reste manuel : `raftup-registry` n'envoie rien.

### Côté launcher (`launcher-core::registry`)

- `fetch_verified_index` : télécharge `index.json` et `index.json.minisig` (mêmes règles d'adresse, taille bornée), vérifie la signature **avant** de lire le JSON.
- `add_registry_profiles` : pour chaque profil de l'index dont on a les octets, contrôle taille et sha256, lit le profil (`Profile::parse`, mêmes règles que le catalogue embarqué) et vérifie `game.id == name`. **Le catalogue embarqué gagne** (ADR 0001) : un profil du registre ne remplace jamais un profil embarqué, il ne fait qu'ajouter un jeu absent. Un paquet faux est écarté avec sa raison, sans casser le reste du catalogue.
- `verify_apworld` : contrôle des octets d'une apworld par rapport à son entrée.

## Conséquences

- La relecture humaine reste la seule vraie confiance : la signature prouve que l'index vient de la mainteneuse, pas que le code d'une apworld est sain. Les niveaux de confiance par CI viendront en V3.
- Pas encore fait : génération de la vraie clé, création de la release `registre` sur GitHub, téléchargement et installation d'une apworld du registre depuis l'interface, affichage des jeux venus du registre dans la Bibliothèque.
- Nouvelle dépendance : aucune hors workspace (`minisign`, `minisign-verify`, `sha2`, `serde_json`, `toml`, `anyhow` y sont déjà).
