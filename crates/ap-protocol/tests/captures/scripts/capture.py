"""Capture les trames brutes envoyées par le serveur officiel (sens serveur -> client)."""
import asyncio, json, sys, os
import websockets

URL = "ws://127.0.0.1:38281"
OUT = sys.argv[1]
SLOTS = sys.argv[2:] or ["Rancher", "Rancher1"]
frames = []  # (étiquette de l'action, texte brut de la trame)


async def drain(ws, label, wait=1.5):
    while True:
        try:
            raw = await asyncio.wait_for(ws.recv(), timeout=wait)
        except asyncio.TimeoutError:
            return
        frames.append((label, raw))


async def send(ws, label, packets):
    await ws.send(json.dumps(packets))
    await drain(ws, label)


VERSION = {"major": 0, "minor": 6, "build": 8, "class": "Version"}


def connect_pkt(name):
    return [{"cmd": "Connect", "password": "", "game": "Slime Rancher", "name": name,
             "uuid": "raftup-capture", "version": VERSION, "items_handling": 7,
             "tags": ["AP"], "slot_data": True}]


async def main():
    # 1. Connexion refusée (slot inconnu).
    async with websockets.connect(URL, max_size=None) as ws:
        await drain(ws, "accueil")
        await send(ws, "getdatapackage", [{"cmd": "GetDataPackage", "games": ["Slime Rancher"]}])
        await send(ws, "connect_refuse", connect_pkt("SlotInexistant"))
    # 2. Vraie connexion.
    connected = None
    for name in SLOTS:
        async with websockets.connect(URL, max_size=None) as ws:
            await drain(ws, "accueil2")
            await ws.send(json.dumps(connect_pkt(name)))
            raw = await ws.recv()
            if '"Connected"' not in raw:
                continue
            frames.append(("connect_ok", raw))
            await drain(ws, "connect_ok")
            connected = next(p for p in json.loads(raw) if p["cmd"] == "Connected")
            missing = sorted(connected["missing_locations"])
            await send(ws, "scouts", [{"cmd": "LocationScouts", "locations": missing[:2], "create_as_hint": 0}])
            await send(ws, "checks", [{"cmd": "LocationChecks", "locations": missing[:3]}])
            await send(ws, "say", [{"cmd": "Say", "text": "Bonjour depuis Raftup"}])
            await send(ws, "say_hint", [{"cmd": "Say", "text": "!hint"}])
            await send(ws, "setnotify", [{"cmd": "SetNotify", "keys": ["raftup_test"]}])
            await send(ws, "set", [{"cmd": "Set", "key": "raftup_test", "default": 0, "want_reply": True,
                                    "operations": [{"operation": "add", "value": 5}]}])
            await send(ws, "get", [{"cmd": "Get", "keys": ["raftup_test", "_read_hints_0_1", "_read_race_mode"]}])
            await send(ws, "bounce", [{"cmd": "Bounce", "slots": [connected["slot"]], "data": {"msg": "ping"}}])
            await send(ws, "sync", [{"cmd": "Sync"}])
            await send(ws, "update_tags", [{"cmd": "ConnectUpdate", "tags": ["AP", "DeathLink"]}])
            await send(ws, "deathlink", [{"cmd": "Bounce", "tags": ["DeathLink"],
                                          "data": {"time": 1700000000.0, "source": name, "cause": "test"}}])
            await send(ws, "invalid_cmd", [{"cmd": "CommandeInconnue"}])
            await send(ws, "invalid_args", [{"cmd": "LocationScouts", "locations": "pas une liste"}])
            await send(ws, "goal", [{"cmd": "StatusUpdate", "status": 30}])
        break
    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        for label, raw in frames:
            f.write(json.dumps({"action": label, "raw": raw}, ensure_ascii=False) + "\n")
    print("trames:", len(frames), "connecte:", connected is not None)


asyncio.run(main())
