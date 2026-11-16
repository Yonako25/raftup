"""Scénarios fixes joués sur la seed MIT (Alice et Bob sur ChecksFinder, liés
par le groupe « Lien », Chloé sur Yacht Dice).

Un scénario est une liste d'étapes (client, paquets envoyés). Le client est un
nom court (« a », « b »…) : une connexion WebSocket ouverte au début.

Utilisé par les scripts de capture (`connect_vectors.py`, `game_vectors.py`) et
par le pilote des tests différentiels (`tests/differentiel/differentiel.py`).
"""

V = {"major": 0, "minor": 6, "build": 8, "class": "Version"}


def connect(name, game, handling=7, tags=None, version=V, password=None, slot_data=None, uuid="raftup-test"):
    p = {"cmd": "Connect", "password": password, "game": game, "name": name, "uuid": uuid,
         "version": version, "items_handling": handling, "tags": tags or []}
    if slot_data is not None:
        p["slot_data"] = slot_data
    return [p]


def checks(locations):
    return [{"cmd": "LocationChecks", "locations": locations}]


def update(**fields):
    return [{"cmd": "ConnectUpdate", **fields}]


def status(value):
    return [{"cmd": "StatusUpdate", "status": value}]


def bounce(**fields):
    return [{"cmd": "Bounce", **fields}]


def setp(key, ops, **fields):
    return [{"cmd": "Set", "key": key, "operations": [{"operation": o, "value": v} for o, v in ops], **fields}]


CF = list(range(81000, 81025))              # lieux d'Alice et de Bob (ChecksFinder)
YD = list(range(16871244501, 16871244511))  # 10 premiers lieux de Chloé (Yacht Dice)

# ----- Connexion (connexion-seed-mit.json) : un serveur neuf par scénario -----

CONNEXION = {
    "alice": [("a", connect("Alice", "ChecksFinder"))],
    "chloe_puis_alice": [
        ("c", connect("Chloé", "Yacht Dice", slot_data=False)),
        ("a", connect("Alice", "ChecksFinder", tags=["DeathLink"])),
    ],
    "chloe_sans_inventaire_distant": [("c", connect("Chloé", "Yacht Dice", handling=0b011))],
    "chloe_sans_objets": [("c", connect("Chloé", "Yacht Dice", handling=0))],
    "suivi_textonly": [("t", connect("Bob", "", tags=["AP", "TextOnly"], version={"major": 0, "minor": 5, "build": 0, "class": "Version"}))],
    "refus_nom": [("a", connect("Personne", "ChecksFinder"))],
    "refus_jeu": [("a", connect("Alice", "Yacht Dice"))],
    "refus_version": [("a", connect("Alice", "ChecksFinder", version={"major": 0, "minor": 4, "build": 9, "class": "Version"}))],
    "refus_items_handling": [("a", connect("Alice", "ChecksFinder", handling=0b110))],
    "refus_jeu_et_version": [("a", connect("Alice", "Yacht Dice", version={"major": 0, "minor": 1, "build": 0, "class": "Version"}))],
    "connect_sans_game": [("a", [{"cmd": "Connect", "password": None, "name": "Alice", "uuid": "x",
                                  "version": V, "items_handling": 7, "tags": []}])],
    "reconnexion_meme_socket": [
        ("a", connect("Alice", "ChecksFinder")),
        ("a", connect("Alice", "ChecksFinder", tags=["DeathLink"])),
    ],
    "changement_de_slot": [
        ("a", connect("Alice", "ChecksFinder")),
        ("a", connect("Bob", "ChecksFinder")),
    ],
    "datapackage_avant_connect": [("a", [{"cmd": "GetDataPackage", "games": ["ChecksFinder", "Inconnu"]}])],
    "datapackage_sans_games": [("a", [{"cmd": "GetDataPackage"}])],
    "groupe": [("g", connect("Lien", "ChecksFinder"))],
}

# ----- Parties (partie, objectif, echanges, stockage) : un serveur neuf chacune -----

PARTIE = [
    ("a", connect("Alice", "ChecksFinder")),
    ("b", connect("Bob", "ChecksFinder", handling=0b001)),
    ("c", connect("Chloé", "Yacht Dice")),
    ("t", connect("Bob", "", tags=["TextOnly"])),
    ("a", checks([81000, 81001, 81002, 99999])),
    ("a", checks([81000])),
    ("b", checks(CF)),
    ("c", checks(YD)),
    ("t", checks([81005])),
    ("b", [{"cmd": "Sync"}]),
    ("a", update(items_handling=0b011)),
    ("a", update(items_handling=0b110)),
    ("a", update(tags=["DeathLink"])),
    ("a", update(tags=["DeathLink"])),
    ("c", update(items_handling=0)),
    ("c", update(items_handling=0b111)),
    ("d", connect("Bob", "ChecksFinder")),
    ("a", checks(CF[3:])),
    ("a", connect("Alice", "ChecksFinder")),
    ("c", [{"cmd": "Sync"}, {"cmd": "LocationChecks", "locations": [16871244511, 16871244512]}]),
]

OBJECTIF = [
    ("a", connect("Alice", "ChecksFinder")),
    ("b", connect("Bob", "ChecksFinder")),
    ("c", connect("Chloé", "Yacht Dice")),
    ("t", connect("Bob", "", tags=["TextOnly"])),
    ("a", checks(CF[:5])),
    ("t", status(30)),
    ("c", status(10)),
    ("b", status(30)),
    ("b", status(20)),
    ("a", status(30)),
    ("c", status(30)),
    ("c", checks(YD)),
]

MORT = {"time": 1794764264.5, "source": "Alice", "cause": "Alice est tombée."}
ECHANGES = [
    ("a", connect("Alice", "ChecksFinder", tags=["DeathLink"])),
    ("b", connect("Bob", "ChecksFinder")),
    ("c", connect("Chloé", "Yacht Dice", tags=["DeathLink"])),
    ("a", [{"cmd": "LocationScouts", "locations": [81000, 81001]}]),
    ("a", [{"cmd": "LocationScouts", "locations": [81002], "create_as_hint": 0}]),
    ("a", [{"cmd": "LocationScouts", "locations": ["x"]}]),
    ("a", [{"cmd": "Say", "text": "Bonjour à tous ! 🎉"}]),
    ("c", [{"cmd": "Say", "text": "ligne\nsuite"}]),
    ("u", [{"cmd": "Say", "text": "coucou"}]),
    ("a", bounce(tags=["DeathLink"], data=MORT)),
    ("b", bounce(data={"x": 1})),
    ("b", bounce(slots=[1], data={"x": 2})),
    ("b", bounce(games=["Yacht Dice"], data={"x": 3})),
    ("c", bounce(tags=["DeathLink"], slots=[2], operator="and", data={"x": 4})),
    ("c", bounce(tags=["DeathLink"], slots=[2], operator="or", data={"x": 5})),
    ("b", bounce(teams=[0], tags=["Inexistant"], data={"x": 6})),
    ("b", bounce(tags="DeathLink", data={"x": 7})),
    ("a", bounce(slots=[1])),
    ("u", bounce(slots=[1], data={"x": 8})),
    ("a", [{"cmd": "LocationScouts", "locations": [99999]}]),
]

STOCKAGE = [
    ("a", connect("Alice", "ChecksFinder")),
    ("b", connect("Bob", "ChecksFinder")),
    ("c", connect("Chloé", "Yacht Dice")),
    ("u", [{"cmd": "Get", "keys": ["x"]}]),
    ("a", [{"cmd": "SetNotify", "keys": ["compteur", "_read_client_status_0_2", "_read_hints_0_1", "_read_hints_0_2"]}]),
    ("b", [{"cmd": "SetNotify", "keys": ["compteur"]}]),
    ("a", [{"cmd": "Get", "rid": 7, "keys": [
        "compteur", "_read_race_mode", "_read_client_status_0_1", "_read_client_status_0_3",
        "_read_slot_data_3", "_read_item_name_groups_ChecksFinder", "_read_location_name_groups_Yacht Dice",
        "_read_hints_0_1", "_read_inconnu", "compteur", "_read_slot_data_04", "_read_hints_1_1"]}]),
    ("b", setp("compteur", [("add", 5)])),
    ("c", setp("compteur", [("mul", 3), ("max", 10), ("mod", 4)], want_reply=True, tag="x")),
    ("c", setp("liste", [("update", [1, 2]), ("update", [2, 3, 3]), ("remove", 1), ("pop", 0)], default=[], want_reply=True)),
    ("c", setp("texte", [("add", "b"), ("mul", 3)], default="a", want_reply=True)),
    ("c", setp("dico", [("update", {"x": 1}), ("or", {"y": 2}), ("pop", "x")], default={}, want_reply=True)),
    ("c", setp("nombres", [("pow", 2), ("floor", None), ("left_shift", 2), ("right_shift", 1), ("xor", 5),
                           ("and", 12), ("or", 1), ("min", 3), ("ceil", 0), ("add", 0.5)], default=7, want_reply=True)),
    ("c", setp("negatif", [("mod", 3), ("default", 1), ("replace", -2.5), ("floor", 0)], default=-7, want_reply=True)),
    ("a", setp("_read_race_mode", [("replace", 1)])),
    ("a", [{"cmd": "Get", "keys": "pas une liste"}]),
    ("a", [{"cmd": "SetNotify", "keys": "x"}]),
    ("b", status(10)),
    ("b", checks([81004])),
    ("a", checks([81003])),
    ("a", [{"cmd": "Get", "keys": ["_read_hints_0_1", "_read_hints_0_2", "_read_client_status_0_2"]}]),
    ("c", setp("compteur", [("add", "texte")])),
    ("a", [{"cmd": "Get", "keys": ["compteur", "liste", "nombres"]}]),
]

# Paquets mal formés : écarts trouvés par les tests différentiels (graine 1).
MALFORMES = [
    ("a", connect("Alice", "ChecksFinder")),
    ("t", connect("Bob", "", tags=["Tracker"])),
    ("a", [{"cmd": 5}, {"cmd": None}, {"cmd": 1.5}, {"cmd": True}, {"cmd": [1]}, {"cmd": {"x": 1}}]),
    ("t", [{"cmd": "LocationChecks", "locations": "pas une liste"}]),
    ("t", [{"cmd": "LocationChecks", "locations": [81000]}]),
    ("b", [{"pas": "de cmd", "n": 1e300, "f": 0.5, "z": -0.0, "e": [None, True, "l'eau"], "o": {}}]),
    ("c", ["texte"]),
    ("d", [5]),
    ("e", [[1, 2.0]]),
]

# Un `Connect` refusé applique quand même son `items_handling` à un client déjà
# connecté (écart trouvé par les tests différentiels, graine 2).
OLD = {"major": 0, "minor": 4, "build": 9, "class": "Version"}
REFUS_CONNECTE = [
    ("c", connect("Chloé", "Yacht Dice", handling=0b001)),
    ("b", connect("Bob", "ChecksFinder")),
    ("b", checks(CF)),
    ("c", connect("Chloé", "Yacht Dice", handling=0b111, version=OLD)),
    ("c", checks(YD[:2])),
    ("c", connect("Chloé", "Inconnu", handling=0b011)),
    ("c", [{"cmd": "Sync"}]),
]


def say(text):
    return [{"cmd": "Say", "text": text}]


# Commandes du chat (cadre et commandes d'information) ; pas de `!options` :
# l'officiel tire le nombre d'étoiles du mot de passe au hasard.
COMMANDES = [
    ("a", connect("Alice", "ChecksFinder", tags=["DeathLink"])),
    ("b", connect("Bob", "ChecksFinder")),
    ("a", say("!help")),
    ("a", say("!Inconnue x")),
    ("a", say("!")),
    ("a", say("bonjour")),
    ("a", say("\"!PLAYERS\"")),
    ("a", say("!status")),
    ("a", say("!status DeathLink")),
    ("a", say("!remaining")),
    ("a", checks([81000, 81001])),
    ("a", say("!checked")),
    ("a", say("!checked 1")),
    ("a", say("!missing")),
    ("a", say("!missing zzz")),
    ("b", say("!admin login secret")),
    ("b", say("!admin")),
]


# Indices (T6) : LocationScouts avec create_as_hint, CreateHints, UpdateHint. Alice (1) trouve
# 81001 pour Bob (2) ; Bob a 81018 pour Alice. À capturer sur le serveur officiel, puis
# comparer (ordre LocationInfo / messages d'indices, textes d'erreur exacts).
def create_hints(locations, **fields):
    return [{"cmd": "CreateHints", "locations": locations, **fields}]


def update_hint(player, location, status):
    return [{"cmd": "UpdateHint", "player": player, "location": location, "status": status}]


INDICES = [
    ("a", connect("Alice", "ChecksFinder")),
    ("b", connect("Bob", "ChecksFinder")),
    ("t", connect("Bob", "", tags=["TextOnly", "NoText"])),
    ("a", [{"cmd": "SetNotify", "keys": ["_read_hints_0_1", "_read_hints_0_2"]}]),
    ("b", [{"cmd": "SetNotify", "keys": ["_read_hints_0_1", "_read_hints_0_2"]}]),
    ("a", [{"cmd": "LocationScouts", "locations": [81001], "create_as_hint": 0}]),
    ("a", [{"cmd": "LocationScouts", "locations": [81001], "create_as_hint": 1}]),
    ("a", [{"cmd": "LocationScouts", "locations": [81001], "create_as_hint": 2}]),
    ("a", [{"cmd": "LocationScouts", "locations": [81001, 81002], "create_as_hint": 2}]),
    ("a", create_hints([81018], player=2, status=30)),
    ("a", create_hints([81018], player=2, status=10)),
    ("a", create_hints([81002])),
    ("a", create_hints([])),
    ("a", create_hints([81000], status=7)),
    ("a", create_hints([81000], status=300)),
    ("a", create_hints([81000], status=30)),
    ("a", create_hints([99999], player=2)),
    ("a", create_hints([81002], player=2)),
    ("a", create_hints([81000], player=9)),
    ("b", update_hint(1, 81001, 30)),
    ("b", update_hint(1, 81001, 30)),
    ("b", update_hint(1, 81002, 30)),
    ("b", update_hint(1, 81001, None)),
    ("b", update_hint(1, 81001, 7)),
    ("b", update_hint(1, 81001, 40)),
    ("b", update_hint("1", 81001, 30)),
    ("a", update_hint(1, 81001, 20)),
    ("a", checks([81001])),
    ("a", [{"cmd": "Get", "keys": ["_read_hints_0_1", "_read_hints_0_2"]}]),
    ("a", [{"cmd": "CreateHints", "locations": [81000], "player": 9}]),
]

PARTIES = {"partie": PARTIE, "objectif": OBJECTIF, "echanges": ECHANGES, "stockage": STOCKAGE,
           "malformes": MALFORMES, "refus_connecte": REFUS_CONNECTE, "commandes": COMMANDES,
           "indices": INDICES}


def all_fixed():
    """Tous les scénarios fixes, chacun à jouer sur un serveur neuf : {nom: étapes}."""
    out = {f"connexion/{name}": steps for name, steps in CONNEXION.items()}
    out.update(PARTIES)
    return out
