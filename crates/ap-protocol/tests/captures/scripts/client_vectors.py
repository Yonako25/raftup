"""Écrit des paquets client avec l'encodeur officiel, puis les envoie au vrai serveur.

Usage : python -I client_vectors.py <dossier source Archipelago> <dossier de sortie>
"""
import asyncio, json, os, sys

AP, OUT = sys.argv[1], sys.argv[2]
sys.path.insert(0, AP)
import Utils  # noqa: E402
from NetUtils import encode, ClientStatus, HintStatus  # noqa: E402
import websockets  # noqa: E402

NAME = "Rancher1"
# Mêmes formes que CommonClient.py (Archipelago 0.6.8).
VECTORS = {
    "Connect": [{'cmd': 'Connect', 'password': None, 'name': NAME, 'version': Utils.version_tuple,
                 'tags': ['AP'], 'items_handling': 0b111, 'uuid': 'raftup-test-uuid',
                 'game': 'Slime Rancher', "slot_data": True}],
    "ConnectUpdate": [{"cmd": "ConnectUpdate", "tags": ["AP", "DeathLink"]}],
    "Sync": [{"cmd": "Sync"}],
    "LocationChecks": [{"cmd": 'LocationChecks', "locations": tuple([1, 2])}],
    "LocationScouts": [{"cmd": "LocationScouts", "locations": [3, 5], "create_as_hint": 0}],
    "CreateHints": [{"cmd": "CreateHints", "locations": [6], "status": HintStatus.HINT_PRIORITY}],
    "UpdateHint": [{"cmd": "UpdateHint", "location": 6, "player": 1, "status": HintStatus.HINT_AVOID}],
    "Say": [{"cmd": "Say", "text": "Salut, ça va ?"}],
    "GetDataPackage": [{"cmd": "GetDataPackage", "games": ["Slime Rancher"]}],
    "Get_SetNotify": [{"cmd": "Get", "keys": ["_read_race_mode", "raftup_test"]},
                      {"cmd": "SetNotify", "keys": ["raftup_test"]}],
    "Set": [{"cmd": "Set", "key": "raftup_test", "default": 0, "want_reply": True,
             "operations": [{"operation": "add", "value": 5}, {"operation": "max", "value": 3}]}],
    "Bounce_DeathLink": [{"cmd": "Bounce", "tags": ["DeathLink"],
                          "data": {"time": 1731670000.123456, "source": NAME, "cause": "Rancher1 a glissé"}}],
    "Bounce_Ping": [{"cmd": "Bounce", "slots": [1]}],
    "StatusUpdate": [{"cmd": "StatusUpdate", "status": ClientStatus.CLIENT_GOAL}],
}

os.makedirs(OUT, exist_ok=True)
frames = {}
for name, packets in VECTORS.items():
    text = encode(packets)
    frames[name] = text
    with open(os.path.join(OUT, name + ".json"), "w", encoding="utf-8", newline="\n") as f:
        f.write(text + "\n")


async def check():
    """Envoie chaque trame (Connect d'abord) et relève les réponses InvalidPacket."""
    problems = []
    async with websockets.connect("ws://127.0.0.1:38281", max_size=None) as ws:
        await ws.recv()  # RoomInfo
        for name, text in frames.items():
            await ws.send(text)
            while True:
                try:
                    raw = await asyncio.wait_for(ws.recv(), timeout=1.0)
                except asyncio.TimeoutError:
                    break
                for p in json.loads(raw):
                    if p["cmd"] in ("InvalidPacket", "ConnectionRefused"):
                        problems.append((name, p))
    return problems


print("refus du serveur :", asyncio.run(check()))
print("vecteurs :", sorted(frames))
