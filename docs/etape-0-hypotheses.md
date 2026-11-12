# Étape 0 : vérification des hypothèses (§9 de la spec)

Date : 12 novembre 2026. Sources : API GitHub publique et pages web. Rien n'a été téléchargé ni exécuté.

## 1. Comment Slimipelago reçoit hôte, slot et mot de passe

**Vérifié (lecture du code source).**

- Le mod lit un fichier `ApConnection.json` dans le dossier du jeu (chemin relatif au dossier de travail).
- Champs lus : `AddressPort` (ex. `archipelago.gg:12345`), `Password`, `SlotName`, plus des options : `DeathLink`, `TrapLink`, `MusicRando`…
- Un ancien format `ApConnection.txt` (3 lignes : adresse, mot de passe, slot) est converti en JSON puis supprimé.
- Le menu de connexion dans les options du jeu reste la voie normale.

**Conséquence pour le profil.** On peut déclarer `connect = { method = "config_file" }` : le launcher écrit `ApConnection.json` dans le dossier du jeu avant de le lancer. Il reste à vérifier, en jouant, que le menu affiche bien ces valeurs sans les retaper.

## 2. Licence de Slimipelago

**Vérifié : aucune licence.**

- L'API GitHub renvoie `license: null` et le dépôt ne contient pas de fichier `LICENSE`.
- Sans licence, tous les droits sont réservés à l'auteur.
- **Action** : demander à SWCreeperKing l'autorisation d'installer le mod automatiquement (URL + hash, sans ré-hébergement), avant d'ajouter le profil au catalogue.

## 3. L'apworld `slime_rancher`

**Vérifié sur le manifeste, pas encore à l'usage.**

- Dernière release : `v0.3.2.1` (16 août 2026), qui contient trois fichiers : `slime_rancher.apworld` (22 Ko), `Slimipelago.zip` (2,2 Mo) et `Slime.Rancher.yaml`.
- Téléchargé et ouvert comme une archive zip, sans rien exécuter. Contenu : un dossier `slime_rancher/` avec 7 fichiers Python.
- Manifeste `archipelago.json` : `minimum_ap_version` = **0.6.5**, `world_version` = 0.3.2.
- **Conséquence** : l'Archipelago épinglé par Raftup doit être en 0.6.5 ou plus.
- **Génération testée avec Archipelago 0.6.8** (dernière version officielle, 4 octobre 2026) : une seed solo Slime Rancher se génère en environ 5 secondes (35 objets, 315 lieux, 152 objets placés). Le fichier `.archipelago` produit fait 8 Ko.
- **Non vérifié** : la durée d'une partie complète et le comportement en jeu.

## 4. python-build-standalone

**Vérifié pour l'installation, pas encore pour la génération.**

- Licence : MPL-2.0.
- Archive testée : `cpython-3.12.16+20261110-x86_64-pc-windows-msvc-install_only` (47 Mo). Son empreinte SHA-256 correspond à celle publiée dans `SHA256SUMS`.
- Extraite dans un dossier utilisateur, sans droits administrateur : 151 Mo une fois dézippée.
- Python 3.12.16 se lance sans droits administrateur.
- Le générateur officiel (`Generate.py`, Archipelago 0.6.8) tourne avec ce Python seul, sans Python installé sur la machine.
- **Dépendances minimales à installer pour générer** (sans kivy) : colorama, websockets 13.1, PyYAML, jellyfish, jinja2, schema, platformdirs, certifi, orjson, typing_extensions, pathspec et bsdiff4.
- **À savoir** : sans la variable `SKIP_REQUIREMENTS_UPDATE=1`, Archipelago tente d'installer lui-même toutes ses dépendances. Le `py-bridge` devra l'activer, et fermer l'entrée standard pour éviter le « Press enter to close ».
- **Dépendances** : elles viennent de PyPI à l'installation, donc le premier usage demande internet.

## Bilan

| Hypothèse | État |
| --- | --- |
| Réception de l'hôte, du slot et du mot de passe | Levée : fichier `ApConnection.json` |
| Licence de Slimipelago | Confirmée absente : demander l'accord de l'auteur |
| Apworld compatible avec Archipelago épinglé | Levée : seed générée avec Archipelago 0.6.8 |
| Python autonome sans droits admin | Levée : génération réussie |
| Durée d'une partie complète, comportement en jeu | À tester avec le jeu (J1) |
| Autres hypothèses (`multidata`, serveur, `client-core`, licences des autres mods) | Hors étape 0 : après le code |
