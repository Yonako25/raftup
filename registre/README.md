# Registre de paquets signés

Le registre complète le catalogue embarqué du launcher (ADR 0032) : il ajoute des jeux
(profils) et des apworlds sans nouvelle version de Raftup. Il ne contient que des
**références** (adresse chez l'auteur + sha256) : rien n'est redistribué.

- `registre.toml` : la source, relue à la main avant chaque publication.
- `fichiers/` : copies locales des paquets, pour le calcul des sha256. **Jamais commité.**
- `sortie/` : `index.json` et `index.json.minisig` produits par `publish`. **Jamais commité.**

Le launcher lit l'index à l'adresse `launcher_core::registry::REGISTRY_INDEX_URL` :
`https://github.com/Yonako25/raftup/releases/download/registre/index.json`, et vérifie la
signature avec `REGISTRY_PUBLIC_KEY` (clé `A3D76747B9BA50E2`).

L'outil est le binaire `raftup-registry` de la crate `registry-tool` (ADR 0045).

## Clés

La paire de clés du registre est dans `C:\Users\ionak\.raftup-keys\` :
`raftup-registry.key` (secrète, sans mot de passe) et `raftup-registry.pub`. Elle est
distincte de celle des mises à jour (`raftup-update.*`). La clé secrète ne va **jamais**
dans le dépôt, ni dans un message, ni dans la CI ; en faire une copie de sauvegarde hors
ligne (clé USB). Si elle est perdue ou fuit : nouvelle paire avec `keygen`, nouvelle
`REGISTRY_PUBLIC_KEY`, nouvelle version du launcher (même règle que l'ADR 0022).

## Niveaux de confiance (ADR 0045)

Chaque paquet de l'index porte un niveau, calculé par la CI puis signé avec l'index :

| Niveau | Ce que la CI a vérifié |
| --- | --- |
| Non vérifié | rien (pas de rapport, ou rapport d'un autre fichier) |
| Structure valide | zip sûr avec `<name>/__init__.py`, ou profil lisible |
| Générable | le générateur officiel épinglé génère une partie solo sur 5 graines |
| Testé en profondeur | en plus, 40 cas de fuzzing (options, YAML abîmés) sans plantage ni boucle |

Mondes natifs (`kind = "native_world"`, ADR 0046) : « Structure valide » = paquet lisible
(`world.toml` seul, ou zip qui le contient) dont le monde est relu et annonce le jeu de
`game` ; « Générable » = le **moteur Rust** de Raftup génère une partie solo sur N graines,
sans Python (même avec `--structure-seule`, même hors Windows). Pas encore de fuzzing :
le niveau s'arrête à « Générable ». Un index qui contient un monde natif est écrit en
`format_version` 3 ; sans monde natif, `publish` écrit toujours le format 2.

Le workflow `.github/workflows/registre.yml` lance `raftup-registry check` à chaque
modification de `registre/`, sous Windows, et range `rapport.json` et `rapport.md` dans
l'artefact `rapport-registre` du run. Il ne signe ni ne publie rien.

Vérifier en local (sur les copies de `fichiers/`) :

```powershell
cargo run -p registry-tool --bin raftup-registry -- check registre\registre.toml registre\rapport
```

Options utiles : `--paquet <nom>`, `--telecharger` (prend les fichiers chez l'auteur, comme
la CI), `--graines N`, `--fuzz K`, `--limite <secondes>`, `--delai-cas <secondes>`,
`--structure-seule`. Le fuzzing est tiré à graine fixe : les mêmes réglages donnent les
mêmes cas.

## Ajouter ou mettre à jour un paquet

1. Télécharger le fichier **chez l'auteur**, à l'adresse exacte qui ira dans `url`, et le
   ranger dans `registre/fichiers/`.
2. Comparer son sha256 à celui que GitHub affiche pour l'asset (ou à celui du profil
   embarqué, s'il y en a un) :

   ```powershell
   Get-FileHash registre\fichiers\slime_rancher.apworld -Algorithm SHA256
   ```

3. Ajouter ou modifier l'entrée `[[package]]` de `registre.toml` (licence, auteurs, version).
   Pour un profil : `kind = "profile"`, `name` = `game.id` du profil, et le profil doit
   passer les mêmes règles que ceux de `profiles/`. Pour une apworld, `game` est le nom du
   jeu côté Archipelago : sans lui, la CI ne peut pas générer et le niveau s'arrête à
   « Structure valide ».
4. Pousser la modification (branche puis pull request) : le workflow `registre.yml` vérifie
   le paquet. Attendre qu'il soit fini sur `main`.

## Publier

1. Télécharger l'artefact `rapport-registre` du **dernier run du workflow sur `main`**, le
   décompresser dans `registre\rapport\`, et lire `rapport.md` : le niveau de chaque paquet
   et, s'il n'est pas au plus haut, l'étape qui a échoué.

2. Produire l'index signé avec ce rapport (rien n'est envoyé) :

   ```powershell
   cargo run -p registry-tool --bin raftup-registry -- publish registre\registre.toml C:\Users\ionak\.raftup-keys\raftup-registry.key registre\sortie --rapport registre\rapport\rapport.json
   ```

   Le niveau d'un paquet n'est repris que si le rapport décrit **le même fichier** (même
   version, même sha256) que celui de `fichiers/`. Sinon le paquet est publié « Non
   vérifié » et la commande affiche un avertissement. Sans `--rapport`, tout est « Non
   vérifié ».

3. Vérifier ce qui a été produit, avec la clé publique du launcher (la commande affiche
   aussi le niveau de chaque paquet) :

   ```powershell
   cargo run -p registry-tool --bin raftup-registry -- verify registre\sortie\index.json registre\sortie\index.json.minisig RWTiULq5R2fXo7ZsQfse7x6oTpfcKN7HDCRlg4v7c1JyGe0EZuTjWszy
   ```

4. **À la main**, sur GitHub, dans la release d'étiquette `registre` du dépôt
   `Yonako25/raftup` : remplacer (ou ajouter, la première fois) les deux fichiers
   `index.json` et `index.json.minisig` de `registre\sortie\`. Toujours les deux ensemble :
   un index sans sa nouvelle signature est refusé par tous les launchers.

   Première fois seulement : créer la release `registre` (étiquette `registre`, cochée
   « pre-release » pour qu'elle ne passe pas devant les releases du launcher en
   « latest »). Avec `gh` :

   ```powershell
   gh release create registre registre\sortie\index.json registre\sortie\index.json.minisig --title "Registre de paquets" --notes "Index signé du registre Raftup (ADR 0032)." --prerelease
   ```

   Les fois suivantes :

   ```powershell
   gh release upload registre registre\sortie\index.json registre\sortie\index.json.minisig --clobber
   ```

5. Ouvrir la Bibliothèque du launcher : le bas de l'écran dit « Registre : N paquet(s) », et
   chaque monde de « Mondes du registre » porte son badge de niveau.

## Ce que fait le launcher

Au démarrage de la Bibliothèque, un thread à part télécharge l'index et sa signature, vérifie
la signature avant de lire le JSON, puis télécharge les profils des jeux **absents** du
catalogue embarqué (le catalogue embarqué gagne toujours). Les jeux ajoutés portent la
mention « Registre ». Une erreur de réseau ou de signature n'empêche rien : la Bibliothèque
marche avec son catalogue embarqué et le dit en une ligne discrète. Les apworlds du registre
sont proposées dans « Mondes du registre », chacune avec son badge de niveau (explication au
survol) : « Installer » télécharge le fichier chez l'auteur, contrôle taille et sha256, et
le pose dans le dossier des mondes, sans jamais écraser un autre fichier du même nom. Pour un
monde « Non vérifié » ou « Structure valide », une confirmation est demandée avant.
