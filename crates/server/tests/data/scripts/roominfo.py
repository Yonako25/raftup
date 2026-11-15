"""Capture le RoomInfo brut du serveur officiel. Usage : python -I roominfo.py <sortie.json>"""
import asyncio, sys
import websockets


async def main():
    async with websockets.connect("ws://127.0.0.1:38281") as ws:
        raw = await asyncio.wait_for(ws.recv(), timeout=5)
    with open(sys.argv[1], "w", encoding="utf-8", newline="\n") as f:
        f.write(raw + "\n")
    print("ok")

asyncio.run(main())
