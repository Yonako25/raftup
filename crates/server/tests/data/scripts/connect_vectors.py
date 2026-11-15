"""Rejoue des scénarios de connexion sur le serveur officiel et écrit les trames reçues.

Usage : python -I connect_vectors.py <sortie.json>
Serveur officiel attendu sur ws://127.0.0.1:38281 avec la seed MIT.
Chaque scénario : liste d'étapes {"client", "envoi", "recu": {client: [trames brutes]}}.
Les scénarios sont dans `scenarios.py` (CONNEXION).
"""
import asyncio, json, os, sys

# `python -I` n'ajoute pas le dossier du script au chemin : on le fait à la main
# pour trouver `scenarios.py`, qui est à côté.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import websockets  # noqa: E402
from scenarios import CONNEXION  # noqa: E402

URL = "ws://127.0.0.1:38281"


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
    for name, steps in CONNEXION.items():
        out[name] = await run(steps)
        print(name, "ok")
    with open(sys.argv[1], "w", encoding="utf-8", newline="\n") as f:
        json.dump(out, f, ensure_ascii=False, indent=1)
        f.write("\n")

asyncio.run(main())
