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
- Code porté : _aucun pour l'instant_
- **Données de test** produites par Archipelago 0.6.8 (pas du code, mais elles contiennent du texte et des noms issus du dépôt) :
  - `crates/multidata/tests/data/seed-mit.archipelago` et son `.attendu.json` : seed générée avec les jeux ChecksFinder et Yacht Dice du dépôt officiel ;
  - `crates/ap-protocol/tests/captures/` : paquets envoyés par le serveur officiel et paquets écrits par son encodeur ;
  - `crates/server/tests/data/` : trames envoyées par le serveur officiel pour la seed ci-dessus (`RoomInfo`, scénarios de connexion).

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
- Fichiers de Raftup concernés : `crates/launcher-ui/src/theme.rs` (les polices sont intégrées au binaire par `include_bytes!`)

Les polices ne sont pas modifiées ; elles ne sont pas vendues seules.
