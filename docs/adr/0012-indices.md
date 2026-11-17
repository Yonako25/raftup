# 0012 — Indices (hints) comme le serveur officiel

- Statut : accepté
- Date : 2026-11-16

## Contexte

Un indice (`!hint`) révèle où se trouve un objet de la partie : chez qui, et à quel lieu. Le serveur officiel Archipelago le fait avec un coût : chaque indice coûte des points d'indice, qui se gagnent en visitant des lieux.

La spec ne décrit pas ce mécanisme. Un mod de jeu doit recevoir les mêmes paquets (`LocationScouts`, `CreateHints`, `UpdateHint`, `SetReply`, `RoomUpdate`, `PrintJSON`) et afficher les mêmes textes qu'avec le serveur officiel.

Le serveur officiel calcule les indices à partir de sa propre logique Python. Raftup suit ce calcul pas à pas (références : `MultiServer.py` et `NetUtils.py` du dépôt Archipelago épinglé).

## Décision

### Coût et points d'indice

- Coût d'un indice pour un slot (`hint_cost(slot)`) :
  - 0 si le pourcentage `hint_cost` vaut 0 ;
  - sinon `max(1, int(hint_cost * 0.01 * nb_lieux))`, calcul en flottant, troncature finale.
- Points d'indice d'un slot :
  `location_check_points * lieux visités - coût * indices utilisés`.

Exemple : un slot a 100 lieux et `hint_cost` vaut 10 (10 %). Le coût d'un indice est `max(1, int(10 * 0.01 * 100))`, soit 10. Un joueur a visité 30 lieux (1 point par lieu) et a utilisé 2 indices : il lui reste 30 - 2 × 10 = 10 points.

Le calcul ancien, qui ignorait les indices utilisés, est remplacé.

### Création

- `LocationScouts` avec `create_as_hint` : crée les indices demandés.
- `CreateHints` : crée une liste d'indices pour un ou plusieurs lieux.
- `UpdateHint` : change le statut d'un indice existant.
- `!hint` : tire des indices pour le joueur, au coût indiqué ci-dessus.
- Statut automatique (quand le statut n'est pas donné) :
  - `AVOID` (20) si l'objet est un piège ;
  - `PRIORITY` (30) sinon.
- Un lieu déjà visité donne un indice `FOUND` (40) avec `found = true`.
- Statut par défaut d'un indice créé sans statut : `UNSPECIFIED` (0).

### Tirage de `!hint` entre ex æquo (DÉCISION de la porteuse)

Quand plusieurs objets sont aussi intéressants, le tirage suit la même règle que le serveur officiel : les lieux non locaux d'abord, puis la sphère la plus basse.

Le serveur officiel tire le reste au hasard avec le Mersenne Twister de Python. Raftup le remplace par un tirage déterministe :

1. Trier les candidats par `(finding, location)`.
2. Tirer avec un petit générateur pseudo-aléatoire dont la graine vient de `(seed_name, team, slot, hints_used)`.

Même partie, même joueur, même nombre d'indices déjà utilisés : même résultat, à chaque fois.

Écart déclaré : le tirage n'est pas celui du Mersenne Twister, donc un ex æquo peut tomber sur un autre objet qu'avec le serveur officiel. Ça n'a pas d'effet sur les mods : ils reçoivent un indice valide, de même forme.

### Diffusion

La diffusion suit `notify_hints` du serveur officiel, dans cet ordre :

1. Si la diffusion ne concerne que les nouveaux indices, on retire ceux que le joueur connaît déjà.
2. Tri stable : les indices `found` passent après les autres.
3. On enregistre les indices nouveaux (pas ceux déjà trouvés, sauf si `persist_even_if_found`) chez le joueur qui trouve et chez chaque membre du slot qui reçoit.
4. Pour chaque slot touché, dans l'ordre du `set` Python `new_hint_events` (ADR 0018, qui remplace « l'ordre croissant ») :
   - `SetReply` sur la clé `_read_hints_<équipe>_<slot>` avec la liste complète (sans `original_value` ni `slot`) ;
   - `RoomUpdate` avec les nouveaux `hint_points` aux clients de ce slot.
5. Ensuite, pour chaque slot concerné, aux clients qui n'ont pas `NoText` : une trame `PrintJSON` de type `Hint`. Les indices du slot concerné passent en premier.

Texte d'un indice affiché dans le chat, par exemple : `[Hint]: Bob's Épée is at Coffre 3 in Alice's World at Grotte. (priority)`.

Textes des statuts :

| Statut | Valeur | Texte |
| --- | --- | --- |
| `FOUND` | 40 | `(found)` |
| `UNSPECIFIED` | 0 | `(unspecified)` |
| `NO_PRIORITY` | 10 | `(no priority)` |
| `AVOID` | 20 | `(avoid)` |
| `PRIORITY` | 30 | `(priority)` |
| autre | | `(unknown)` |

## Conséquences

- Les indices et leur coût donnent les mêmes résultats que le serveur officiel, à l'exception du tirage entre ex æquo (ci-dessus).
- Le moteur d'indices est fait de fonctions pures : il renvoie des effets (paquets à envoyer), ce qui permet de tester chaque règle sans réseau.
- Le tirage déterministe rend les tests reproductibles, et un bug se rejoue à l'identique.
- Un client qui compare ses résultats au serveur officiel verra une différence seulement sur les ex æquo.
- Le coût dépend du nombre de lieux du slot : un jeu avec beaucoup de lieux rend les indices plus chers.
- Le calcul des indices à partir de la liste des objets non indiçables (T4 du plan J2) reste à faire : il demande le Python d'Archipelago.
