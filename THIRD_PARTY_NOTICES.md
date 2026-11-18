# Notices des projets tiers

Raftup est distribué en double licence MIT / Apache-2.0 (voir `LICENSE-MIT` et `LICENSE-APACHE`).

Ce fichier regroupe les notices de copyright du code **porté** (copié ou adapté) depuis d'autres projets.
Les dépendances Cargo ordinaires n'ont pas besoin d'être listées ici : leurs licences voyagent avec leurs crates.

Règle : dès qu'un fichier de Raftup contient du code porté, recopier ici, **mot pour mot**, la notice
de copyright et le texte de licence du fichier `LICENSE` du dépôt amont, au commit d'où vient le code.

---

## Archipelago

- Dépôt : https://github.com/ArchipelagoMW/Archipelago
- Licence : MIT
- Commit d'origine : `54803be064fc7e80c4628777ed0b46a9390f255f` (version 0.6.8)
- Code porté :
  - `crates/generate/src/worlds/apquest.rs` : monde APQuest (`worlds/apquest` : noms et numéros, régions, règles, options, `slot_data`), réécrit en Rust ;
  - `crates/generate/src/datapackage.rs` et `crates/generate/src/multidata_build.rs` : calcul du `checksum` du datapackage (`worlds/AutoWorld.py`) et contenu de la seed (`Main.py`), réécrits en Rust
- **Données de test** produites par Archipelago 0.6.8 (pas du code, mais elles contiennent du texte et des noms issus du dépôt) :
  - `crates/multidata/tests/data/seed-mit.archipelago` et son `.attendu.json` : seed générée avec les jeux ChecksFinder et Yacht Dice du dépôt officiel ;
  - `crates/ap-protocol/tests/captures/` : paquets envoyés par le serveur officiel et paquets écrits par son encodeur ;
  - `crates/server/tests/data/` : trames envoyées par le serveur officiel pour la seed ci-dessus (`RoomInfo`, scénarios de connexion) ;
  - `crates/generate/tests/data/apquest-*.archipelago` : seeds APQuest générées par Archipelago 0.6.8 (références du générateur Rust).

Notice (fichier `LICENSE` du dépôt amont, au commit ci-dessus) :

```text
MIT License

Copyright (c) 2017 LLCoolDave
Copyright (c) 2025 Berserker66
Copyright (c) 2022 CaitSith2
Copyright (c) 2021 LegendaryLinux

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

Subdirectories of this repository may contain their own LICENSE file. Files within
subdirectories containing their own LICENSE file are exempt from the above license
and are subject to the license contained within the LICENSE file in their containing
directory.
```

---

## pahoa

- Dépôt : https://github.com/mooinglemur/pahoa
- Licence : MIT
- Commit d'origine : _aucun code porté pour l'instant (utilisé en dépendance Git épinglée)_
- Fichiers de Raftup concernés : _aucun pour l'instant_

Notice : _à recopier depuis le `LICENSE` du dépôt amont au premier portage._

---

## Polices intégrées au launcher (Chakra Petch et Inter)

- Fichiers : `crates/launcher-ui/fonts/` (TTF copiés depuis le dépôt https://github.com/google/fonts, dossier `ofl/`)
- Licence : SIL Open Font License 1.1 (les textes complets sont dans `crates/launcher-ui/fonts/OFL-ChakraPetch.txt` et `OFL-Inter.txt`)
- Chakra Petch (SemiBold et Bold) : Copyright 2018 The Chakra Petch Project Authors (https://github.com/m4rc1e/Chakra-Petch.git)
- Inter (police variable) : Copyright 2020 The Inter Project Authors (https://github.com/rsms/inter)
- Inter SemiBold (`Inter-SemiBold.ttf`, statique, version 4.001) : même licence et mêmes auteurs ; copiée telle quelle depuis l'archive officielle `Inter-4.1.zip` (https://github.com/rsms/inter/releases/tag/v4.1, dossier `extras/ttf/`)
- Fichiers de Raftup concernés : `crates/launcher-ui/src/theme.rs` (les polices sont intégrées au binaire par `include_bytes!`)

Les polices ne sont pas modifiées ; elles ne sont pas vendues seules.

---

## Composants téléchargés au premier usage (non redistribués)

Pour générer une seed, le launcher télécharge ces composants depuis leur source d'origine, vérifie leur sha256
puis les installe sur le PC du joueur (ADR 0017). Raftup ne les héberge ni ne les distribue : ils ne figurent
ni dans le dépôt ni dans le binaire. Adresses, tailles et hash : `crates/launcher-core/generator-pins.toml`.
Les licences ci-dessous ont été lues dans les métadonnées et les fichiers de licence des archives téléchargées.

| Composant | Version | Licence | Source |
| --- | --- | --- | --- |
| Python (python-build-standalone) | 3.12.16, build 20261110 | PSF-2.0 pour CPython ; le projet python-build-standalone est sous MPL-2.0 ; composants tiers : voir `python/LICENSE.txt` dans l'archive | https://github.com/astral-sh/python-build-standalone |
| Archipelago | 0.6.8 (commit `54803be064fc7e80c4628777ed0b46a9390f255f`) | MIT (voir la notice de la section Archipelago ci-dessus) ; certains dossiers `worlds/*` ont leur propre licence | https://github.com/ArchipelagoMW/Archipelago |
| colorama | 0.4.6 | BSD-3-Clause (texte : copyright Jonathan Hartley) | https://pypi.org/project/colorama/ |
| websockets | 13.1 | BSD-3-Clause | https://pypi.org/project/websockets/ |
| PyYAML | 6.0.3 | MIT | https://pypi.org/project/PyYAML/ |
| jellyfish | 1.2.1 | MIT | https://pypi.org/project/jellyfish/ |
| Jinja2 | 3.1.6 | BSD-3-Clause (texte : copyright Pallets) | https://pypi.org/project/Jinja2/ |
| MarkupSafe | 3.0.3 | BSD-3-Clause | https://pypi.org/project/MarkupSafe/ |
| schema | 0.7.8 | MIT | https://pypi.org/project/schema/ |
| platformdirs | 4.10.1 | MIT | https://pypi.org/project/platformdirs/ |
| certifi | 2026.2.25 | MPL-2.0 | https://pypi.org/project/certifi/ |
| orjson | 3.11.7 | MPL-2.0 AND (Apache-2.0 OR MIT) | https://pypi.org/project/orjson/ |
| typing_extensions | 4.15.0 | PSF-2.0 | https://pypi.org/project/typing-extensions/ |
| pathspec | 1.0.4 | MPL-2.0 (texte du fichier `LICENSE` ; pas de champ de licence dans les métadonnées) | https://pypi.org/project/pathspec/ |
| bsdiff4 | 1.2.6 | « BSD » d'après les métadonnées (copyright Ilan Schnell) | https://pypi.org/project/bsdiff4/ |

Chaque wheel porte son propre fichier de licence dans son dossier `.dist-info`, qui reste installé avec elle.
