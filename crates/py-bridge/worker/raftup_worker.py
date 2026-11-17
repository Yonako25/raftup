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


def generate(params):
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
        # Avec -I, le dossier courant n'est pas dans sys.path.
        sys.path.insert(0, a["ap"])
        os.chdir(a["ap"])  # Utils.local_path en depend
        os.environ["SKIP_REQUIREMENTS_UPDATE"] = "1"
        if os.environ.get("RAFTUP_BLOCK_NETWORK") == "1":
            block_network()
        import logging

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
        mw = Main.main(erargs, seed)
    except ImportError as exc:
        raise RpcFail(-32011, "import impossible : %s" % exc, {"type": type(exc).__name__, "traceback": tb_text()})
    except BaseException as exc:  # SystemExit, EOFError (input) et erreurs de generation
        raise RpcFail(-32010, "generation echouee : %s: %s" % (type(exc).__name__, exc),
                      {"type": type(exc).__name__, "traceback": tb_text()})
    try:
        name = str(mw.seed_name)
        zip_path = os.path.realpath(os.path.join(a["out"], "AP_%s.zip" % name))
        root = os.path.realpath(a["out"])
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


def call(method, params):
    if method == "ping":
        return "pong"
    if method == "version":
        impl = sys.implementation.name
        return {
            "python": "%d.%d.%d" % sys.version_info[:3],
            "implementation": "CPython" if impl == "cpython" else impl,
            "protocol": PROTOCOL,
        }
    if method == "echo":
        return params
    if method == "generate":
        return generate(params)
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
