"""Worker Python de Raftup, protocole v1.

Lance par `python -I -X utf8 -c <script>`. Bibliotheque standard seule.
Une ligne JSON par message sur stdin/stdout :
  requete : {"id": int, "method": str, "params": objet}
  reponse : {"id": ..., "result": ...} ou {"id": ..., "error": {"code", "message"}}
"""
import json
import os
import sys

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
