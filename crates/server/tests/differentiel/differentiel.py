"""Tests différentiels : le serveur officiel et Raftup jouent les mêmes scénarios (ADR 0008).

Usage (depuis la racine du dépôt) :

    python -I crates/server/tests/differentiel/differentiel.py [--fixes] [--hasard N] [--graine G]
                                                             [--longueur L] [--sortie DOSSIER]

Variables d'environnement :
- RAFTUP_ARCHIPELAGO : dossier source d'Archipelago 0.6.8 (celui de MultiServer.py) ;
- RAFTUP_PYTHON : Python qui lance le serveur officiel (par défaut, celui qui lance ce script).

Sans option, joue les scénarios fixes puis 50 scénarios tirés au hasard.
Chaque écart est écrit dans le dossier de sortie, au format des vecteurs de
`tests/data/` (« recu » : trames du serveur officiel ; « recu_raftup » : les nôtres).
"""
import argparse, asyncio, json, os, random, socket, subprocess, sys, time, zlib
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[3]
# `python -I` n'ajoute pas le dossier du script au chemin : on ajoute nos deux dossiers.
sys.path.insert(0, str(HERE))
sys.path.insert(0, str(ROOT / "crates/server/tests/data/scripts"))
import websockets  # noqa: E402
import hasard  # noqa: E402
import scenarios  # noqa: E402

SEED = ROOT / "crates/multidata/tests/data/seed-mit.archipelago"
CLOSED = "<fermé>"
# Barrière : un `GetDataPackage` vide. Sa réponse prouve que le serveur a fini
# de traiter tout ce qui précède sur cette connexion.
BARRIER = json.dumps([{"cmd": "GetDataPackage", "games": []}])
# Barrières envoyées au plus par client et par étape.
BARRIER_ROUNDS = 3
# Rythme minimal d'une étape : chaque client y envoie jusqu'à 3 barrières, et
# Raftup ferme une connexion au-delà de 50 paquets/s (ADR 0007).
STEP_SECONDS = 0.1
TIMEOUT = 10


def is_barrier_reply(frame):
    packets = json.loads(frame)
    return packets == [{"cmd": "DataPackage", "data": {"games": {}}}]


# ----- Comparaison -----

def text(value):
    """Texte JSON canonique : 1 et 1.0 restent différents, l'ordre des clés ne compte pas."""
    return json.dumps(value, sort_keys=True, ensure_ascii=False)


def normalize(frame):
    """Même normalisation que `tests/connexion.rs`, plus l'heure de `RoomInfo`."""
    if frame == CLOSED:
        return CLOSED
    packets = json.loads(frame)
    for packet in packets:
        if isinstance(packet.get("errors"), list):
            packet["errors"].sort(key=text)
        if packet.get("cmd") == "RoomUpdate" and isinstance(packet.get("checked_locations"), list):
            packet["checked_locations"].sort()
        if packet.get("cmd") == "SetReply" and str(packet.get("key", "")).startswith("_read_hints_"):
            if isinstance(packet.get("value"), list):
                packet["value"].sort(key=text)
        if packet.get("cmd") == "Retrieved":
            for key, value in packet.get("keys", {}).items():
                if key.startswith("_read_hints_") and isinstance(value, list):
                    value.sort(key=text)
        if packet.get("cmd") == "RoomInfo":
            packet.pop("time", None)
    return text(packets)


# ----- Serveurs -----

def free_port():
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def wait_port(port, process, name):
    deadline = time.time() + 90
    while time.time() < deadline:
        if process.poll() is not None:
            raise SystemExit(f"{name} s'est arrêté au démarrage (code {process.returncode})")
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.5):
                return
        except OSError:
            time.sleep(0.2)
    raise SystemExit(f"{name} ne répond pas sur le port {port}")


class Servers:
    """Les deux serveurs, sur la même seed."""

    def __init__(self, raftup_exe, logs):
        self.raftup_exe = raftup_exe
        self.logs = logs
        self.processes = []
        self.urls = {}
        self.starts = 0

    def start(self):
        self.stop()
        self.starts += 1
        archipelago = os.environ.get("RAFTUP_ARCHIPELAGO")
        if not archipelago:
            raise SystemExit("RAFTUP_ARCHIPELAGO doit donner le dossier source d'Archipelago 0.6.8")
        python = os.environ.get("RAFTUP_PYTHON", sys.executable)
        official_port, raftup_port = free_port(), free_port()
        log = open(self.logs / f"officiel-{self.starts}.log", "w", encoding="utf-8")
        # stdin reste ouvert : le serveur officiel y lit ses commandes de console.
        # SKIP_REQUIREMENTS_UPDATE : sinon il propose d'installer des paquets
        # et attend une réponse au clavier.
        env = dict(os.environ, SKIP_REQUIREMENTS_UPDATE="1", PYTHONUTF8="1")
        official = subprocess.Popen(
            [python, "MultiServer.py", "--host", "127.0.0.1", "--port", str(official_port),
             "--disable_save", "--use_embedded_options", str(SEED)],
            cwd=archipelago, env=env, stdin=subprocess.PIPE, stdout=log, stderr=subprocess.STDOUT)
        log = open(self.logs / f"raftup-{self.starts}.log", "w", encoding="utf-8")
        raftup = subprocess.Popen(
            [str(self.raftup_exe), str(SEED), "--no-tls", "--host", "127.0.0.1", "--port", str(raftup_port)],
            stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT)
        self.processes = [official, raftup]
        wait_port(official_port, official, "le serveur officiel")
        wait_port(raftup_port, raftup, "raftup-server")
        self.urls = {"officiel": f"ws://127.0.0.1:{official_port}", "raftup": f"ws://127.0.0.1:{raftup_port}"}

    def stop(self):
        for process in self.processes:
            process.kill()
            process.wait()
        self.processes = []


# ----- Jouer un scénario sur un serveur -----

class Player:
    """Une connexion du scénario, avec ce qu'elle a reçu."""

    def __init__(self, ws):
        self.ws = ws
        self.closed = False
        self.bytes = 0

    async def one_barrier(self, frames):
        """Envoie une barrière ; ajoute à `frames` ce qui arrive avant sa réponse.

        Renvoie le nombre de trames ajoutées.
        """
        before = len(frames)
        await self.ws.send(BARRIER)
        while True:
            frame = await asyncio.wait_for(self.ws.recv(), timeout=TIMEOUT)
            self.bytes += len(frame.encode("utf-8"))
            if is_barrier_reply(frame):
                return len(frames) - before
            frames.append(frame)

    async def until_barrier(self):
        """Trames reçues jusqu'à une barrière « vide », ou la fermeture.

        Le serveur officiel envoie certains messages (`Join`, `Tutorial`…) par des
        tâches `asyncio` à part : ils peuvent arriver après la réponse à la
        barrière. On fait donc toujours au moins deux tours, puis on continue
        tant qu'un tour apporte encore des trames.
        """
        frames = []
        if self.closed:
            return frames
        try:
            for round_number in range(BARRIER_ROUNDS):
                added = await self.one_barrier(frames)
                if round_number > 0 and added == 0:
                    break
                await asyncio.sleep(0.02)
            return frames
        except websockets.ConnectionClosed:
            self.closed = True
            frames.append(CLOSED)
            return frames


async def play(url, steps):
    """Joue les étapes ; renvoie {"accueil": {client: trame}, "etapes": [{client: [trames]}]}, octets."""
    names = sorted({c for c, _ in steps})
    players = {}
    welcome = {}
    for name in names:
        ws = await websockets.connect(url, max_size=None)
        players[name] = Player(ws)
        welcome[name] = await asyncio.wait_for(ws.recv(), timeout=TIMEOUT)
    received = []
    for client, packets in steps:
        started = time.monotonic()
        sender = players[client]
        if not sender.closed:
            try:
                await sender.ws.send(json.dumps(packets, ensure_ascii=False))
            except websockets.ConnectionClosed:
                sender.closed = True
        # D'abord la barrière de l'émetteur : quand elle revient, l'étape est
        # traitée et tout ce qu'elle envoie aux autres est déjà parti.
        step = {client: await sender.until_barrier()}
        for name in names:
            if name != client:
                step[name] = await players[name].until_barrier()
        received.append(step)
        rest = STEP_SECONDS - (time.monotonic() - started)
        if rest > 0:
            await asyncio.sleep(rest)
    for player in players.values():
        await player.ws.close()
    total = sum(p.bytes for p in players.values())
    return {"accueil": welcome, "etapes": received}, total


def first_difference(steps, official, raftup):
    """Première différence, en texte ; `None` si les deux serveurs sont d'accord."""
    for name in official["accueil"]:
        if normalize(official["accueil"][name]) != normalize(raftup["accueil"][name]):
            return f"accueil de « {name} »"
    for number, (theirs, ours) in enumerate(zip(official["etapes"], raftup["etapes"])):
        for name in theirs:
            if [normalize(f) for f in theirs[name]] != [normalize(f) for f in ours[name]]:
                client, packets = steps[number]
                return (f"étape {number} (« {client} » envoie {json.dumps(packets, ensure_ascii=False)[:200]}), "
                        f"trames reçues par « {name} »")
    return None


def as_vector(steps, official, raftup):
    """Écart au format des vecteurs de `tests/data/`."""
    return [{"client": c, "envoi": p, "recu": theirs, "recu_raftup": ours}
            for (c, p), theirs, ours in zip(steps, official["etapes"], raftup["etapes"])]


# ----- Mesure de la bande passante (ADR 0005) -----

async def measure(url):
    async with websockets.connect(url, max_size=None, compression=None) as ws:
        await ws.recv()
        # Sans « games », le serveur officiel ne renvoie rien d'utile : on nomme les jeux.
        games = ["Archipelago", "ChecksFinder", "Yacht Dice"]
        await ws.send(json.dumps([{"cmd": "GetDataPackage", "games": games}]))
        frame = await asyncio.wait_for(ws.recv(), timeout=TIMEOUT)
    raw = frame.encode("utf-8")
    return len(raw), len(zlib.compress(raw))


# ----- Programme principal -----

async def run_all(args, servers):
    differences = 0
    played = 0
    totals = {"officiel": 0, "raftup": 0}

    async def compare(name, steps):
        nonlocal differences, played
        played += 1
        # Les deux serveurs jouent en même temps : ils ne se voient pas.
        (official, b1), (raftup, b2) = await asyncio.gather(
            play(servers.urls["officiel"], steps), play(servers.urls["raftup"], steps))
        totals["officiel"] += b1
        totals["raftup"] += b2
        where = first_difference(steps, official, raftup)
        if where is None:
            return True
        differences += 1
        path = args.sortie / f"ecart-{name.replace('/', '-')}.json"
        with open(path, "w", encoding="utf-8", newline="\n") as f:
            json.dump({name: as_vector(steps, official, raftup)}, f, ensure_ascii=False, indent=1)
            f.write("\n")
        print(f"ÉCART {name} : {where}\n       → {path}")
        return False

    if args.fixes:
        for name, steps in scenarios.all_fixed().items():
            servers.start()  # un serveur neuf par scénario fixe
            if await compare(name, steps):
                print(f"ok    {name}")

    if args.hasard:
        servers.start()
        rng = random.Random(args.graine)
        for number in range(args.hasard):
            steps = hasard.scenario(rng, number, args.longueur)
            name = f"hasard-g{args.graine}-{number}"
            if not await compare(name, steps):
                # Les états ont divergé : on repart de deux serveurs neufs.
                servers.start()
            elif (number + 1) % 10 == 0:
                print(f"ok    {number + 1} scénarios au hasard")

    sizes = {name: await measure(url) for name, url in servers.urls.items()}
    print()
    print(f"Scénarios joués : {played}, écarts : {differences}")
    print(f"Octets reçus (sans compression) : officiel {totals['officiel']}, raftup {totals['raftup']}")
    for name, (raw, packed) in sizes.items():
        print(f"DataPackage complet ({name}) : {raw} octets, {packed} une fois compressé (zlib)")
    return differences


def main():
    # `python -I` ignore PYTHONUTF8 : sous Windows, la console serait en cp1252.
    sys.stdout.reconfigure(encoding="utf-8")
    parser = argparse.ArgumentParser(description="Tests différentiels Raftup / serveur officiel")
    parser.add_argument("--fixes", action="store_true", help="scénarios fixes de scenarios.py")
    parser.add_argument("--hasard", type=int, default=None, help="nombre de scénarios tirés au hasard")
    parser.add_argument("--graine", type=int, default=1)
    parser.add_argument("--longueur", type=int, default=25, help="étapes par scénario au hasard")
    parser.add_argument("--sortie", type=Path, default=ROOT / "target/differentiel")
    args = parser.parse_args()
    if not args.fixes and args.hasard is None:
        args.fixes, args.hasard = True, 50
    args.sortie.mkdir(parents=True, exist_ok=True)

    print("Compilation de raftup-server…")
    subprocess.run(["cargo", "build", "-q", "-p", "server", "--bin", "raftup-server"], cwd=ROOT, check=True)
    exe = ROOT / "target/debug" / ("raftup-server.exe" if os.name == "nt" else "raftup-server")

    servers = Servers(exe, args.sortie)
    try:
        differences = asyncio.run(run_all(args, servers))
    finally:
        servers.stop()
    sys.exit(1 if differences else 0)


if __name__ == "__main__":
    main()
