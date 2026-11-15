"""Rejoue des scénarios de connexion sur le serveur officiel et écrit les trames reçues.

Usage : python -I connect_vectors.py <sortie.json>
Serveur officiel attendu sur ws://127.0.0.1:38281 avec la seed MIT.
Chaque scénario : liste d'étapes {"client", "envoi", "recu": {client: [trames brutes]}}.
"""
import asyncio, json, sys
import websockets

URL = "ws://127.0.0.1:38281"
V = {"major": 0, "minor": 6, "build": 8, "class": "Version"}


def connect(name, game, handling=7, tags=None, version=V, password=None, slot_data=None, uuid="raftup-test"):
    p = {"cmd": "Connect", "password": password, "game": game, "name": name, "uuid": uuid,
         "version": version, "items_handling": handling, "tags": tags or []}
    if slot_data is not None:
        p["slot_data"] = slot_data
    return [p]


SCENARIOS = {
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


async def drain(ws, wait=1.0):
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
        sockets[c] = await websockets.connect(URL)
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
    await asyncio.sleep(0.3)
    return result


async def main():
    out = {}
    for name, steps in SCENARIOS.items():
        out[name] = await run(steps)
        print(name, "ok")
    with open(sys.argv[1], "w", encoding="utf-8", newline="\n") as f:
        json.dump(out, f, ensure_ascii=False, indent=1)
        f.write("\n")

asyncio.run(main())
