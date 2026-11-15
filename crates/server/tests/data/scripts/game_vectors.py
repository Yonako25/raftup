"""Rejoue une partie sur le serveur officiel (seed MIT) et écrit les trames reçues.

Usage : python -I game_vectors.py <scénario> <sortie.json>   (scénario : partie, objectif ou echanges)
Serveur officiel neuf attendu sur ws://127.0.0.1:38281 (la partie modifie son état).
Même format que connect_vectors.py : {scénario: [{"client", "envoi", "recu"}]}.
"""
import asyncio, json, sys
import websockets

URL = "ws://127.0.0.1:38281"
V = {"major": 0, "minor": 6, "build": 8, "class": "Version"}


def connect(name, game, handling=7, tags=None):
    return [{"cmd": "Connect", "password": None, "game": game, "name": name, "uuid": "raftup-test",
             "version": V, "items_handling": handling, "tags": tags or []}]


def checks(locations):
    return [{"cmd": "LocationChecks", "locations": locations}]


def update(**fields):
    return [{"cmd": "ConnectUpdate", **fields}]


CF = list(range(81000, 81025))           # lieux d'Alice et de Bob (ChecksFinder)
YD = list(range(16871244501, 16871244511))  # 10 premiers lieux de Chloé (Yacht Dice)

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



def status(value):
    return [{"cmd": "StatusUpdate", "status": value}]


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

def bounce(**fields):
    return [{"cmd": "Bounce", **fields}]


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

def setp(key, ops, **fields):
    return [{"cmd": "Set", "key": key, "operations": [{"operation": o, "value": v} for o, v in ops], **fields}]


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
SCENARIOS = {"partie": PARTIE, "objectif": OBJECTIF, "echanges": ECHANGES, "stockage": STOCKAGE}

async def drain(ws, wait):
    out = []
    while True:
        try:
            out.append(await asyncio.wait_for(ws.recv(), timeout=wait))
        except asyncio.TimeoutError:
            return out
        except websockets.ConnectionClosed:
            out.append("<fermé>")
            return out


async def run(steps):
    sockets, result = {}, []
    names = sorted({c for c, _ in steps})
    for c in names:
        sockets[c] = await websockets.connect(URL, max_size=None)
        await drain(sockets[c], 0.5)  # RoomInfo
    for c, packets in steps:
        await sockets[c].send(json.dumps(packets, ensure_ascii=False))
        await asyncio.sleep(0.3)
        recu = {}
        for n in names:
            recu[n] = await drain(sockets[n], 0.7)
        result.append({"client": c, "envoi": packets, "recu": recu})
    for ws in sockets.values():
        await ws.close()
    return result


async def main():
    out = {sys.argv[1]: await run(SCENARIOS[sys.argv[1]])}
    with open(sys.argv[2], "w", encoding="utf-8", newline="\n") as f:
        json.dump(out, f, ensure_ascii=False, indent=1)
        f.write("\n")
    print("ok")

asyncio.run(main())
