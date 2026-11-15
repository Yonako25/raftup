"""Faux joueur Slime Rancher : se connecte, visite quelques lieux, écrit dans le chat."""
import asyncio, json
import websockets

V = {"major": 0, "minor": 6, "build": 8, "class": "Version"}


async def main():
    async with websockets.connect("ws://127.0.0.1:38281") as ws:
        await ws.recv()
        await ws.send(json.dumps([{"cmd": "Connect", "password": None, "name": "Rancher1", "game": "Slime Rancher",
                                   "uuid": "faux-joueur", "version": V, "items_handling": 7, "tags": ["AP"],
                                   "slot_data": False}]))
        connected = json.loads(await ws.recv())[0]
        missing = sorted(connected["missing_locations"])
        await asyncio.sleep(1)
        await ws.send(json.dumps([{"cmd": "LocationChecks", "locations": missing[:4]}]))
        await asyncio.sleep(1)
        await ws.send(json.dumps([{"cmd": "Say", "text": "Coucou depuis le faux joueur"}]))
        await asyncio.sleep(1)
    print("faux joueur : fini")


asyncio.run(main())
