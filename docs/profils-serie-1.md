# Profils d'installation, série 1 (25 novembre 2026)

Dix profils ajoutés dans `profiles/`, tous en `status = "untested"` (ADR 0049) : aucun n'a été essayé avec le vrai jeu. Chaque fichier téléchargé a été récupéré dans le scratchpad pour calculer son sha256 (jamais dans le dépôt) ; toutes les empreintes sont identiques aux digest publiés par GitHub quand il y en a un. Les mondes sont ceux intégrés à Archipelago 0.6.8 : aucun apworld installé.

## Jeux ajoutés

| Jeu | Mod (dépôt) | Version | Licence | Connexion |
| --- | --- | --- | --- | --- |
| DOOM 1993 | Archipelago Doom (ArchipelagoDoom/APDoom) | 2.0.0 | GPL-2.0 | arguments de ligne de commande |
| DOOM II | Archipelago Doom (ArchipelagoDoom/APDoom) | 2.0.0 | GPL-2.0 | arguments de ligne de commande |
| Heretic | Archipelago Doom (ArchipelagoDoom/APDoom) | 2.0.0 | GPL-2.0 | arguments de ligne de commande |
| Hylics 2 | ArchipelagoHylics2 (TRPG0) + BepInEx 5.4.23.5 x86 | 1.0.11 | MIT, LGPL-2.1 | manuelle (console du jeu) |
| Muse Dash | ArchipelagoMuseDash (DeamonHunter) + MelonLoader 0.7.2 | 1.5.38 | MIT, Apache-2.0 | manuelle |
| Noita | NoitaArchipelago (DaftBrit) | 1.5.0 | MIT | manuelle (Mod Settings) |
| Subnautica | ArchipelagoSubnauticaModSrc (Berserker66), BepInEx inclus | 1.9.3 | MIT, LGPL-2.1 | manuelle |
| Timespinner | TsRandomizer (Jarno458) | 1.38.6 | sans licence (ADR 0050) | manuelle |
| TUNIC | TUNIC Randomizer (silent-destroyer) + BepInEx 6.0.0-pre.1 IL2CPP | 5.0.2 | MIT, LGPL-2.1 | manuelle |
| VVVVVV | V6AP (N00byKing) + SDL2 2.32.10 x86 | AP0.5.1-3 | licence maison de VVVVVV (non commerciale), Zlib | arguments de ligne de commande |

Les 10 jeux ont un monde intégré à 0.6.8 ; parmi eux, DOOM 1993, DOOM II, Heretic, Hylics 2, Muse Dash, Noita, Subnautica, Timespinner et VVVVVV ont aussi un monde natif converti (`docs/inventaire-conversion.md`) ; TUNIC n'en a pas encore.

### Points à vérifier en vraie partie

- **DOOM** : le guide officiel 0.6.8 renvoie à la version 1.2.0 (Daivuk/apdoom), que ses auteurs disent abandonnée ; le profil prend la 2.0, qui dit jouer les mondes intégrés avec `-game compat_doom` (`compat_doom2`, `compat_heretic`). Les WAD ne sont jamais copiés : le jeu les trouve dans le dossier Steam, et le profil passe `-iwad {game_dir}/base/DOOM.WAD` (chemins tirés du code de la 2.0). DOOM 1993 et DOOM II partagent l'appid Steam 2280.
- **Hylics 2** : nom de l'exécutable (`Hylics 2.exe`) supposé, pas trouvé.
- **Muse Dash, TUNIC** : le premier lancement est long (MelonLoader et BepInEx IL2CPP génèrent leurs fichiers) ; les deux guides demandent de lancer le jeu une fois avant de poser le mod, le profil pose tout d'un coup.
- **VVVVVV** : V6AP remplace `VVVVVV.exe` et SDL2 remplace `SDL2.dll` (sauvegardés et restaurés par le launcher) ; `-v6ap_passwd` vide sans mot de passe, non vérifié.
- **TUNIC** : le mod 5.0.2 publie un apworld plus récent que celui de 0.6.8 ; compatibilité non vérifiée.
- **Noita, TUNIC** : appid Steam non confirmé par le guide (881100, 553420).

### Limites du format relevées (décision de l'utilisatrice, rien n'a été changé)

- `game.detect.expected_exe` n'accepte qu'un nom de fichier à la racine du jeu. Pour DOOM 1993, DOOM II et Heretic, l'exécutable racine de la version Steam n'est pas connu, et le fichier sûr (`base/DOOM.WAD`) est dans un sous-dossier : le champ est absent, donc seule la détection Steam marche (pas de choix manuel du dossier).
- Pas de prérequis système dans le format : Muse Dash demande le .NET Desktop Runtime 6, VVVVVV peut demander le Visual C++ x86 ; c'est écrit seulement en commentaire.

## Jeux écartés

| Jeu | Raison |
| --- | --- |
| Dark Souls III | il faut lancer `DS3Randomizer.exe` pour chaque partie (fichiers propres à la seed), puis lancer le jeu par un `.bat` : hors de portée du format |
| The Messenger | le chargeur Courier s'installe en exécutant son installateur, qui patche l'assembly du jeu |
| Undertale, Wargroove, Saving Princess | passent par le client Python d'Archipelago (patch du jeu ou pont obligatoire) |
| Inscryption | le mod n'a pas de release GitHub (seulement l'apworld) ; paquet Thunderstore avec dépendances à résoudre : prévu pour la série 2 |
| DLC Quest | licence du dépôt non identifiée (NOASSERTION) et installateur fourni : à regarder en série 2 |
| Overcooked! 2 | non regardé en détail (oc2-modding, MIT) : série 2 |
| Meritous, Bumper Stickers, ChecksFinder, Celeste 64 | ce sont des jeux gratuits complets, pas des mods d'un jeu possédé (décision déjà notée dans `docs/jeux-pc-catalogue.md`) |
| Lingo, Terraria, A Hat in Time | mod publié seulement sur le Steam Workshop |
| Civilization VI, Factorio | mod généré pour chaque partie |
| Celeste (Open World) | Everest s'installe en exécutant `MiniInstaller` |
| Slime Rancher | en attente de l'autorisation de l'auteur (`docs/profils-en-attente/`) |

Aucun jeu de console Nintendo actuelle ni de jeu à ROM n'a été retenu.
