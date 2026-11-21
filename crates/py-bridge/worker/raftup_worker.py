"""Worker Python de Raftup, protocole v1.

Lance par `python -I -X utf8 -c <script>`. Bibliotheque standard seule.
Une ligne JSON par message sur stdin/stdout :
  requete : {"id": int, "method": str, "params": objet}
  reponse : {"id": ..., "result": ...} ou {"id": ..., "error": {"code", "message"}}
"""
import json
import os
import site
import sys
import time
import traceback

PROTOCOL = 1
MAX = 1024 * 1024  # taille maximale d'une ligne, en octets

# Le protocole passe par une COPIE du vrai stdout ; le fd 1 est ensuite
# redirige vers stderr pour que les print() du generateur ne cassent rien.
proto = os.fdopen(os.dup(1), "w", encoding="utf-8", newline="\n")
os.dup2(2, 1)
sys.stdout = sys.stderr
# stdin d'origine : on en garde une COPIE (lecture binaire bornee), puis le fd 0
# est remplace par devnull : input() et les sous-process du generateur recevront
# EOF au lieu de voler les requetes du pont.
inp = os.fdopen(os.dup(0), "rb")
os.dup2(os.open(os.devnull, os.O_RDONLY), 0)
sys.stdin = open(os.devnull, "r")


def send(msg):
    # allow_nan=False : NaN/Infinity ne sont pas du JSON valide ; dumps leve alors
    # ValueError, rattrape dans handle() et renvoye en -32603.
    proto.write(
        json.dumps(msg, ensure_ascii=True, separators=(",", ":"), allow_nan=False) + "\n"
    )
    proto.flush()


def fail(id_, code, message):
    send({"id": id_, "error": {"code": code, "message": message}})


class MethodNotFound(Exception):
    pass


class RpcFail(Exception):
    """Erreur applicative : code JSON-RPC, message et donnees optionnelles."""

    def __init__(self, code, message, data=None):
        Exception.__init__(self, message)
        self.code, self.message, self.data = code, message, data


used = False  # un seul generate par process (Archipelago garde un etat global)


def tb_text(limit=64 * 1024):
    """Traceback de l'exception en cours, tronquee a `limit` octets (on garde la fin)."""
    raw = traceback.format_exc().encode("utf-8", "replace")
    return raw[-limit:].decode("utf-8", "ignore")


def check_params(p):
    def text(k, nullable=False):
        v = p.get(k)
        if v is None and nullable:
            return None
        if not isinstance(v, str) or not v:
            raise RpcFail(-32602, "parametre invalide : " + k)
        return v

    def integer(k, lo, nullable=False):
        v = p.get(k)
        if v is None and nullable:
            return None
        if not isinstance(v, int) or isinstance(v, bool) or v < lo:
            raise RpcFail(-32602, "parametre invalide : " + k)
        return v

    race = p.get("race")
    if not isinstance(race, bool):
        raise RpcFail(-32602, "parametre invalide : race")
    spoiler = integer("spoiler", 0)
    if spoiler > 3:
        raise RpcFail(-32602, "parametre invalide : spoiler")
    out = {
        "ap": os.path.abspath(text("archipelago_dir")),
        "sp": os.path.abspath(text("site_packages", True)) if p.get("site_packages") is not None else None,
        "players": os.path.abspath(text("player_files_dir")),
        "out": os.path.abspath(text("output_dir")),
        "seed": integer("seed", 0, True),
        "spoiler": spoiler,
        "race": race,
    }
    for k in ("ap", "players", "sp"):
        # "sp" est optionnel (None) ; s'il est donne, ce doit etre un dossier existant.
        if out[k] is not None and not os.path.isdir(out[k]):
            raise RpcFail(-32602, "dossier introuvable : " + out[k])
    return out


def block_network():
    """Durcissement optionnel (ADR 0017, decision 12) : refuse toute connexion sortante.

    Active par RAFTUP_BLOCK_NETWORK=1, avant l'import d'Archipelago. Les adresses de
    boucle locale restent permises (asyncio sous Windows s'en sert en interne). Ce n'est
    pas un bac a sable : ctypes ou un sous-process le contournent ; il evite seulement
    qu'un monde ou une dependance appelle le reseau par inadvertance.
    """
    import socket

    def refuse(what):
        sys.stderr.write("RAFTUP: acces reseau refuse (%s)\n" % what)
        sys.stderr.flush()
        raise OSError("reseau desactive par Raftup : %s" % what)

    def local(address):
        host = address[0] if isinstance(address, tuple) and address else address
        return host in ("127.0.0.1", "::1", "localhost")

    real_socket = socket.socket

    class NoNetSocket(real_socket):
        def connect(self, address):
            if self.family != getattr(socket, "AF_UNIX", -1) and not local(address):
                refuse("connect %r" % (address,))
            return real_socket.connect(self, address)

        def connect_ex(self, address):
            if self.family != getattr(socket, "AF_UNIX", -1) and not local(address):
                refuse("connect_ex %r" % (address,))
            return real_socket.connect_ex(self, address)

    def no_resolve(host, *a, **k):
        if not local((host,)):
            refuse("resolution de %r" % (host,))
        return real_getaddrinfo(host, *a, **k)

    real_getaddrinfo = socket.getaddrinfo
    socket.socket = NoNetSocket
    socket.getaddrinfo = no_resolve
    socket.gethostbyname = lambda host: refuse("gethostbyname %r" % (host,))
    socket.gethostbyname_ex = lambda host: refuse("gethostbyname_ex %r" % (host,))


class Abandon(BaseException):
    """Arrete Main.main en attente si Raftup ne donne pas de placement."""


# Export en cours (ADR 0028) : Main.main tourne dans un thread, en attente juste avant
# le placement ; finish_worlds lui donne le placement de Raftup et le laisse finir.
SESSION = {}


def integrity_level():
    """Integrite Windows (ADR 0034) d'apres `whoami /groups` (SID S-1-16-x), ou None."""
    if os.name != "nt" or not os.environ.get("RAFTUP_SANDBOX"):
        return None
    import re
    import subprocess
    try:
        exe = os.path.join(os.environ.get("SystemRoot", "C:\\Windows"), "System32", "whoami.exe")
        out = subprocess.run([exe, "/groups"], capture_output=True, stdin=subprocess.DEVNULL,
                             timeout=10, creationflags=0x08000000).stdout  # sans fenetre
        m = re.search(r"S-1-16-(\d+)", out.decode("ascii", "replace"))
        rid = int(m.group(1))
    except Exception:
        return None
    return "untrusted" if rid < 4096 else "low" if rid < 8192 else "medium" if rid < 12288 else "high"


def failed(exc, tb):
    return RpcFail(-32010, "generation echouee : %s: %s" % (type(exc).__name__, exc),
                   {"type": type(exc).__name__, "traceback": tb})


def result_of(mw, out, Utils, t0):
    try:
        name = str(mw.seed_name)
        zip_path = os.path.realpath(os.path.join(out, "AP_%s.zip" % name))
        root = os.path.realpath(out)
        if os.path.commonpath([zip_path, root]) != root or not os.path.isfile(zip_path):
            raise ValueError("zip introuvable sous output_dir : " + zip_path)
        players = [{"slot": int(i), "name": str(mw.player_name[i]), "game": str(mw.game[i])}
                   for i in sorted(mw.player_ids)]
        ver = str(getattr(Utils, "__version__", "inconnue"))
    except BaseException as exc:
        raise RpcFail(-32010, "resultat illisible : %s: %s" % (type(exc).__name__, exc),
                      {"type": type(exc).__name__, "traceback": tb_text()})
    return {"zip": zip_path, "seed_name": name, "ap_version": ver, "players": players,
            "seconds": round(time.time() - t0, 3)}


def generate(params, capture=False):
    global used
    a = check_params(params)
    if used:
        raise RpcFail(-32012, "un seul generate par process")
    used = True
    t0 = time.time()
    try:
        os.makedirs(a["out"], exist_ok=True)
        if a["sp"]:
            site.addsitedir(a["sp"])
        # Avec -P, le dossier courant n'est pas dans sys.path.
        sys.path.insert(0, a["ap"])
        os.chdir(a["ap"])  # Utils.local_path en depend
        os.environ["SKIP_REQUIREMENTS_UPDATE"] = "1"
        if os.environ.get("RAFTUP_BLOCK_NETWORK") == "1":
            block_network()
        import logging
        import settings

        if os.environ.get("RAFTUP_SANDBOX"):
            # Bac a sable (ADR 0034) : le dossier d'Archipelago est en lecture seule pour
            # nous ; host.yaml ne doit pas etre reecrit a la sortie.
            settings.skip_autosave = True
        # Sans cela, un monde qui demande un fichier absent (ROM du joueur) ouvre une
        # boite de dialogue invisible et la generation attend jusqu'au delai : on veut
        # un FileNotFoundError tout de suite.
        settings.no_gui = True
        logging.basicConfig(stream=sys.stderr, level=logging.INFO)
        argv = ["Generate.py", "--player_files_path", a["players"],
                "--outputpath", a["out"], "--spoiler", str(a["spoiler"])]
        if a["seed"] is not None:
            argv += ["--seed", str(a["seed"])]
        if a["race"]:
            argv.append("--race")
        sys.argv = argv
        import Generate
        import Main
        import Utils

        erargs, seed = Generate.main()
        if capture:
            return pause_before_fill(Main, Utils, erargs, seed, a["out"], t0)
        mw = Main.main(erargs, seed)
    except RpcFail:
        raise
    except ImportError as exc:
        raise RpcFail(-32011, "import impossible : %s" % exc, {"type": type(exc).__name__, "traceback": tb_text()})
    except BaseException as exc:  # SystemExit, EOFError (input) et erreurs de generation
        raise failed(exc, tb_text())
    return result_of(mw, a["out"], Utils, t0)


def pause_before_fill(Main, Utils, erargs, seed, out, t0):
    """Lance Main.main dans un thread qui s'arrete au placement ; rend le MultiWorld."""
    import threading

    ready, go, box = threading.Event(), threading.Event(), {}

    def fill(multiworld, *args, **kwargs):
        box["mw"] = multiworld
        ready.set()
        go.wait()
        if "place" not in box:
            raise Abandon()
        box["place"](multiworld)

    Main.distribute_items_restrictive = fill
    Main.flood_items = fill
    # Ordre de creation (ADR 0042) : _raftup_n sur chaque region, entree et lieu.
    import BaseClasses as B
    n = [0]
    for c in (B.Region, B.Entrance, B.Location):
        def w(s, *a, _i=c.__init__, **k):
            n[0] += 1
            s._raftup_n = n[0]
            _i(s, *a, **k)
        c.__init__ = w
    # Raftup a deja equilibre la progression.
    Main.balance_multiworld_progression = lambda multiworld: None

    def run():
        try:
            box["result"] = Main.main(erargs, seed)
        except BaseException as exc:
            box["error"] = (exc, tb_text())
        finally:
            ready.set()

    thread = threading.Thread(target=run, daemon=True)
    thread.start()
    ready.wait()
    if "mw" not in box:  # fini ou plante avant le placement
        thread.join()
        if "error" in box:
            raise failed(*box["error"])
        raise RpcFail(-32010, "le placement n'a pas ete atteint")
    SESSION.update(box=box, go=go, thread=thread, out=out, utils=Utils, t0=t0)
    return box["mw"], Utils, t0


def finish_worlds(params):
    """Pose le placement de Raftup et laisse Archipelago ecrire la seed (ADR 0028)."""
    if not SESSION:
        raise RpcFail(-32014, "aucun export en cours")
    path = params.get("placement_path")
    if not isinstance(path, str) or not os.path.isabs(path):
        raise RpcFail(-32602, "parametre invalide : placement_path")
    with open(path, encoding="utf-8") as f:
        placement = json.load(f)
    box = SESSION["box"]

    def place(mw):
        pool = {}
        for item in mw.itempool:
            if item.location is None:
                pool.setdefault((item.player, item.name), []).append(item)
        for lp, lname, ip, iname in placement:
            loc = mw.get_location(lname, lp)
            items = pool.get((ip, iname))
            if loc.item is not None or not items:
                raise ValueError("placement impossible : %s <- %s" % (lname, iname))
            mw.push_item(loc, items.pop(0), collect=False)
        if any(pool.values()) or mw.get_unfilled_locations():
            raise ValueError("placement incomplet")

    box["place"] = place
    SESSION["go"].set()
    SESSION["thread"].join()
    if "error" in box:
        raise failed(*box["error"])
    return result_of(box["result"], SESSION["out"], SESSION["utils"], SESSION["t0"])


def export_worlds(params):
    """Mondes construits jusqu'a pre_fill, decrits dans un fichier JSON (ADR 0028)."""
    code = params.get("exporter")
    if not isinstance(code, str) or not code:
        raise RpcFail(-32602, "parametre invalide : exporter")
    path = params.get("export_path")
    if not isinstance(path, str) or not os.path.isabs(path):
        raise RpcFail(-32602, "parametre invalide : export_path")
    mw, Utils, t0 = generate(params, capture=True)
    try:
        ns = {"__name__": "raftup_export"}
        exec(compile(code, "raftup_export.py", "exec"), ns)
        info = ns["export"](mw, path, 64 * 1024 * 1024)
        players = [{"slot": int(i), "name": str(mw.player_name[i]), "game": str(mw.game[i])}
                   for i in sorted(mw.player_ids)]
    except BaseException as exc:
        raise RpcFail(-32013, "export echoue : %s: %s" % (type(exc).__name__, exc),
                      {"type": type(exc).__name__, "traceback": tb_text()})
    return {"export": path, "seed_name": str(mw.seed_name),
            "ap_version": str(getattr(Utils, "__version__", "inconnue")), "players": players,
            "bytes": info["bytes"], "opaque_rules": info["opaque_rules"],
            "probed_rules": info.get("probed_rules", 0),
            "seconds": round(time.time() - t0, 3)}


def call(method, params):
    if method == "ping":
        return "pong"
    if method == "version":
        impl = sys.implementation.name
        return {
            "python": "%d.%d.%d" % sys.version_info[:3],
            "implementation": "CPython" if impl == "cpython" else impl,
            "protocol": PROTOCOL,
            # 0 si PYTHONHASHSEED=0 : ordre des set de chaines reproductible (ADR 0028).
            "hash_randomization": int(sys.flags.hash_randomization),
            # Bac a sable (ADR 0034) : "low" attendu quand le pont l'a demande.
            "integrity": integrity_level(),
        }
    if method == "echo":
        return params
    if method == "generate":
        return generate(params)
    if method == "export_worlds":
        return export_worlds(params)
    if method == "finish_worlds":
        return finish_worlds(params)
    raise MethodNotFound(method)


def handle(line):
    try:
        req = json.loads(line.decode("utf-8"))
    except (ValueError, RecursionError):  # JSON/UTF-8 invalide, ou imbrication trop profonde
        return fail(None, -32700, "JSON illisible")
    is_obj = isinstance(req, dict)
    id_ = req.get("id") if is_obj else None
    if not isinstance(id_, int) or isinstance(id_, bool):
        id_ = None
    params = req.get("params", {}) if is_obj else None
    if (
        not is_obj
        or id_ is None
        or not isinstance(req.get("method"), str)
        or not isinstance(params, dict)
    ):
        return fail(id_, -32600, "requete mal formee")
    try:
        send({"id": id_, "result": call(req["method"], params)})
    except RpcFail as exc:
        err = {"code": exc.code, "message": exc.message}
        if exc.data is not None:
            err["data"] = exc.data
        send({"id": id_, "error": err})
    except MethodNotFound:
        fail(id_, -32601, "methode inconnue : " + req["method"])
    except Exception as exc:  # erreur interne : on reste en vie
        fail(id_, -32603, "erreur interne : %s: %s" % (type(exc).__name__, exc))


def main():
    while True:
        line = inp.readline(MAX + 1)
        if not line:
            return 0  # EOF : arret propre
        # Limite stricte : readline(MAX + 1) coupe avant le "\n" d'une ligne de MAX octets
        # terminee par CRLF (MAX + 2 octets), qui est donc refusee. Le pont n'envoie que "\n".
        if len(line) > MAX and not line.endswith(b"\n"):
            # Flux desynchronise : on refuse puis on s'arrete avec un code d'erreur.
            fail(None, -32600, "ligne trop longue")
            return 2
        if not line.strip():
            continue
        handle(line)


sys.exit(main())
