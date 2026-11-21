# Publier une release

Ce guide explique comment sortir une nouvelle version de Raftup et de Rafthub.
La CI construit les binaires ; **toi seule** signes le launcher, avec ta clé, sur ton PC.
La clé privée ne va jamais dans le dépôt ni dans la CI (ADR 0022).

## Ce que la CI fabrique

Le workflow `.github/workflows/release.yml` démarre quand un tag `v*` est poussé (par exemple `v0.2.0`).
Il crée un **brouillon** de release (invisible des joueurs) avec ces fichiers :

| Fichier | Contenu |
| --- | --- |
| `raftup-windows-x86_64.exe` | le launcher Windows, exe nu (ADR 0022) |
| `rafthub-linux-x86_64.tar.gz` | Rafthub auto-hébergé pour Linux, sans Discord (ADR 0038) |
| `rafthub-windows-x86_64.zip` | Rafthub auto-hébergé pour Windows, sans Discord |
| `rafthub-officiel-linux-x86_64.tar.gz` | Rafthub officiel (`--features officiel`), pour ton serveur |

Chaque fichier a son empreinte à côté : `<fichier>.sha256`.
Les archives de Rafthub contiennent aussi `LICENSE-AGPL`, `THIRD_PARTY_NOTICES.md` et un `LISEZMOI.txt` avec le lien vers le code source (obligation de l'AGPL).

Le workflow refuse de construire si le tag ne correspond pas à la version de `Cargo.toml` (`[workspace.package] version`).

**Adresse du Rafthub officiel dans le launcher** (ADR 0040) : le workflow lit la variable de dépôt `RAFTUP_OFFICIAL_HUB_URL` (GitHub → Settings → Secrets and variables → Actions → onglet **Variables**), par exemple `https://hub.raftup-games.com`. Sans elle, le launcher de la release garde l'adresse locale `http://127.0.0.1:8787`.

## Étapes

### 1. Préparer la version

1. Mettre la nouvelle version dans `Cargo.toml` (section `[workspace.package]`), par exemple `0.2.0`.
2. Lancer `cargo build --workspace` pour mettre `Cargo.lock` à jour, puis commiter les deux sur `main`.
3. Vérifier que la CI de `main` est verte.

### 2. Poser le tag

```bash
git tag v0.2.0
```

```bash
git push origin v0.2.0
```

Le workflow « Release » se lance (onglet **Actions** du dépôt GitHub). Attendre qu'il soit vert.

### 3. Télécharger le launcher et vérifier son empreinte

Dans un dossier vide (par exemple `C:\raftup-release\v0.2.0`) :

```bash
gh release download v0.2.0 --repo Yonako25/raftup --pattern "raftup-windows-x86_64.exe*"
```

```bash
sha256sum -c raftup-windows-x86_64.exe.sha256
```

La commande doit afficher `raftup-windows-x86_64.exe: OK`. Sinon, ne pas signer : le fichier est abîmé.

### 4. Signer le launcher

Depuis le dossier du dépôt, en remplaçant le chemin du dossier de release :

```bash
cargo run -p launcher-core --example update_tool -- sign C:\raftup-release\v0.2.0\raftup-windows-x86_64.exe C:\Users\Yonako\.raftup-keys\raftup-update.key
```

Cela écrit `raftup-windows-x86_64.exe.minisig` à côté de l'exe. La clé n'est jamais affichée.

### 5. Écrire le manifeste `update.json`

L'adresse est celle du fichier **dans cette release** (pas `latest`) :

```bash
cargo run -p launcher-core --example update_tool -- manifest C:\raftup-release\v0.2.0\raftup-windows-x86_64.exe 0.2.0 https://github.com/Yonako25/raftup/releases/download/v0.2.0/raftup-windows-x86_64.exe C:\raftup-release\v0.2.0\raftup-windows-x86_64.exe.minisig
```

L'outil relit le manifeste comme le ferait le launcher : s'il le refuse, rien n'est écrit.

### 6. Joindre la signature et le manifeste à la release

Depuis le dossier de release :

```bash
gh release upload v0.2.0 raftup-windows-x86_64.exe.minisig update.json --repo Yonako25/raftup
```

### 7. Publier

Sur la page de la release (onglet **Releases**), relire la liste des fichiers, puis « Publish release ».
Une fois publiée, c'est la « dernière release » : les launchers installés trouvent `update.json` à l'adresse
`https://github.com/Yonako25/raftup/releases/latest/download/update.json`, et les Rafthub y prennent l'exe à distribuer (bouton « Rafraîchir » de la page « Distribution du launcher »).

**Ne publie jamais une release sans `update.json`** : elle deviendrait la « dernière », et les launchers ne trouveraient plus de mise à jour.

## Si quelque chose casse

- **Le workflow échoue sur « Comparer le tag »** : le tag et `Cargo.toml` ne disent pas la même version. Supprimer le tag (`git push origin :refs/tags/v0.2.0` puis `git tag -d v0.2.0`), corriger, recommencer.
- **Une cible ne compile pas** : les autres continuent, mais le brouillon n'est pas créé. Corriger sur `main`, puis reposer le tag.
- **Relancer le workflow sur une release qui existe déjà** : il remplace seulement les fichiers construits ; il faut alors **re-signer** le launcher (étapes 3 à 6), car l'exe a changé.

## À vérifier au premier vrai essai

- GitHub sert les fichiers des releases par une redirection vers un autre domaine (`objects.githubusercontent.com` ou `release-assets.githubusercontent.com`). Le launcher et Rafthub n'acceptent que du HTTPS dans les redirections, sans liste de domaines : ça devrait passer, mais rien n'a encore été essayé sur une vraie release.
- Le Rafthub Linux est construit sur Ubuntu 22.04 : il tourne sur une distribution au moins aussi récente (glibc 2.35).
