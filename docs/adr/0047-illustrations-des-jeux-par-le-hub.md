# ADR 0047 — Illustrations des jeux servies par le hub (IGDB)

- **Statut** : remplacé le 22 novembre 2026 par l'ADR 0051 (images choisies par le créateur, installées avec le world) ; accepté le 21 novembre 2026
- Modifie la SPEC (§ 7, « Jaquettes ») et l'ADR 0001 (« jamais de jaquette officielle ») ; prolonge les ADR 0033 (API `/v1` du hub), 0035 (client du hub) et 0036 (réglages en base).

## Contexte

Dans la Bibliothèque, un jeu n'a qu'un aplat coloré avec son titre, et une pastille d'initiales ailleurs. L'utilisatrice veut de vraies illustrations (jaquette, image de fond de la fiche). Deux règles tiennent toujours : aucune image de jeu dans le dépôt ni dans le binaire, et `launcher-core` (MIT / Apache) ne dépend jamais du hub (AGPL).

Comparaison du 21 novembre 2026 :

| Source | Clé | Cache et service par un serveur | Verdict |
| --- | --- | --- | --- |
| **IGDB (Twitch)** | Client ID + secret, jeton OAuth « client credentials » | Permis par écrit (FAQ : « we prefer if you store and serve the data to your end users ») ; 4 requêtes/s, 8 ouvertes au plus | **Retenu** |
| CDN Steam | Aucune | Rien d'écrit sur les images | Plus tard, sur décision à part |
| SteamGridDB | Clé perso | Usage personnel seulement, pas de redistribution ni d'usage automatisé | Écarté |
| RAWG | Oui | Redistribution interdite, lien vers RAWG sur chaque page | Écarté |

## Décision

### Source et clé

- **IGDB seul.** Le hub l'appelle ; le launcher ne parle jamais à IGDB.
- Deux réglages du hub (ADR 0036, base > environnement > défaut) : `igdb_client_id` (`RAFTUP_HUB_IGDB_CLIENT_ID`) et `igdb_client_secret` (`RAFTUP_HUB_IGDB_CLIENT_SECRET`, secret jamais réaffiché), dans un cadre « Illustrations des jeux » de la console. Chaque hub a sa propre application Twitch. Rien dans le launcher, rien dans le dépôt.
- Le jeton OAuth est demandé et renouvelé par le hub (`POST https://id.twitch.tv/oauth2/token`), gardé en mémoire seulement.
- Sans clé : les routes répondent 503 `art_disabled`, et le launcher garde ses aplats.

### Routes du hub (publiques, sans compte)

| Méthode et chemin | Réponse |
| --- | --- |
| `GET /v1/art/steam/{appid}` | JSON `{ igdb_id, cover, hero, attribution, attribution_url }` du jeu trouvé par son appid Steam (`external_games`, source Steam) |
| `GET /v1/art/igdb/{id}` | même JSON, par identifiant IGDB |
| `GET /v1/art/images/{image_id}/{kind}` | l'image JPEG ; `kind` = `cover` (`t_cover_big_2x`, portrait) ou `hero` (`t_screenshot_huge`, 1280 × 720) |

- `cover` et `hero` sont des identifiants d'image IGDB (ou `null`). `hero` : premier « artwork » du jeu, sinon première capture.
- Erreurs (codes stables) : 404 `art_not_found` (jeu inconnu d'IGDB), 503 `art_disabled`, 502 `art_unavailable` (IGDB ne répond pas et rien en cache), 429 `art_rate_limited`, 400 `bad_request` (identifiant d'image hors `[a-z0-9]`, `kind` inconnu).
- **Cache disque** dans `<cache du hub>/art/` : fiches de jeu en JSON, images en JPEG. Valable 30 jours (IGDB garde 30 jours une image retirée) ; un jeu introuvable est retenu 7 jours. Si IGDB ne répond pas, une entrée périmée est encore servie. Taille plafonnée (256 Mio par défaut) : au-delà, les fichiers les plus anciens sont retirés.
- Image reçue d'IGDB : 4 Mio au plus, et doit commencer comme un JPEG.
- **Débit** : tous les appels à IGDB passent par une file à 4 par seconde. Les demandes qui ratent le cache sont limitées à 120 par minute pour tout le hub (429 au-delà) ; les réponses du cache ne sont pas limitées. Le hub n'est donc pas un proxy ouvert vers IGDB.
- Le hub ne parle à IGDB qu'à travers un trait (`ArtSource`) : un faux dans les tests, comme Discord et la distribution.

### Lien entre un profil et IGDB

- Par le `steam_appid` déjà présent dans `[game.detect]`.
- Champ **facultatif** `igdb_id` dans `[game]` pour corriger une mauvaise correspondance ou couvrir un jeu sans Steam ; il passe avant l'appid. Champ facultatif : `format_version` reste à 1 (comme `seconds_per_location`, ADR 0030).
- Un jeu sans les deux (monde natif, APQuest…) garde son aplat.

### Côté launcher

- Module `launcher_core::art` (MIT / Apache) : types JSON **redéfinis**, aucune dépendance au hub (vérifié par `crates/hub/tests/licence.rs`). Appels `ureq` comme `hub_client` (pas de redirection, délai court, réponse bornée), toujours hors du thread d'interface.
- **Cache disque** du launcher dans `<données>/cache/art/<hôte du hub>/` : fiches et images, valables 30 jours. Hors ligne, une entrée périmée sert quand même.
- L'interface décode le JPEG (crate `image`, déjà utilisée pour le PNG) et l'affiche en texture egui.

### Sans hub, hors ligne ou jeu inconnu

Image du cache si elle existe, sinon **l'aplat et la pastille actuels**. Aucun message d'erreur, rien ne bloque.

### Attribution

« Illustrations : IGDB.com », renvoyée par le hub (`attribution`, `attribution_url`) et affichée au pied de la Bibliothèque quand au moins une illustration est montrée.

### Périmètre

La Bibliothèque (jaquettes des cartes) et la fiche d'un jeu (image de fond en haut). Les pastilles de « Partie en cours », du Salon et des parties (repérées par le nom Archipelago du jeu) restent pour plus tard.

## Conséquences

- La SPEC § 7 devient : « Jaquettes : illustrations servies par le hub depuis une API qui permet le cache (IGDB, ADR 0047), jamais dans le dépôt ni dans le binaire ; aplats colorés avec le titre du jeu sinon. » L'ADR 0001 garde « jamais de jaquette dans le dépôt ».
- Risque restant : les images appartiennent aux éditeurs. IGDB permet de les mettre en cache et de les servir, mais ce n'est pas une licence de l'éditeur. Un éditeur qui le demande voit ses images retirées (identifiant mis en liste d'exclusion du hub, à faire si besoin).
- Les conditions de Twitch visent un usage non commercial : Raftup est gratuit ; un hub commercial devrait passer un accord avec IGDB.
- Un test réel (`#[ignore]`) contre IGDB demande une application Twitch (compte avec double authentification).
- La couverture de chaque jeu du catalogue se vérifie au test réel ; un jeu mal trouvé se corrige par `igdb_id`.
