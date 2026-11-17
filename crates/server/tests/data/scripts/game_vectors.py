"""Rejoue une partie sur le serveur officiel (seed MIT) et écrit les trames reçues.

Usage : python -I game_vectors.py <scénario> <sortie.json>
(scénario : partie, objectif, echanges ou stockage ; voir `scenarios.py`)
Serveur officiel neuf attendu sur ws://127.0.0.1:38281 (la partie modifie son état).
Même format que connect_vectors.py : {scénario: [{"client", "envoi", "recu"}]}.
"""
import asyncio, json, os, sys

# `python -I` n'ajoute pas le dossier du script au chemin : on le fait à la main
# pour trouver `scenarios.py`, qui est à côté.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import websockets  # noqa: E402
from scenarios import PARTIES  # noqa: E402

URL = "ws://127.0.0.1:38281"


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
    closed = set()  # clients dont le serveur a fermé la connexion
    for c in names:
        sockets[c] = await websockets.connect(URL, max_size=None)
        await drain(sockets[c], 0.5)  # RoomInfo
    for c, packets in steps:
        # Un client fermé n'envoie plus rien ; chaque étape note de nouveau
        # `<fermé>`, comme les captures précédentes.
        if c not in closed:
            try:
                await sockets[c].send(json.dumps(packets, ensure_ascii=False))
            except websockets.ConnectionClosed:
                closed.add(c)
        await asyncio.sleep(0.3)
        recu = {}
        for n in names:
            recu[n] = ["<fermé>"] if n in closed else await drain(sockets[n], 0.7)
            if recu[n][-1:] == ["<fermé>"]:
                closed.add(n)
        result.append({"client": c, "envoi": packets, "recu": recu})
    for ws in sockets.values():
        await ws.close()
    return result


async def main():
    out = {sys.argv[1]: await run(PARTIES[sys.argv[1]])}
    with open(sys.argv[2], "w", encoding="utf-8", newline="\n") as f:
        json.dump(out, f, ensure_ascii=False, indent=1)
        f.write("\n")
    print("ok")

asyncio.run(main())
