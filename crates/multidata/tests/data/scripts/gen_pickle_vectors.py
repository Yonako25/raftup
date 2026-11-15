"""Écrit les vecteurs de test du lecteur pickle de Raftup, avec le résultat attendu.

Usage : python -I gen_pickle_vectors.py <dossier source Archipelago> <seed .archipelago> <dossier de sortie>

Le résultat attendu est calculé par Python lui-même (la fonction officielle
`restricted_loads` pour la seed), puis écrit dans une forme JSON « canonique »
que le test Rust reproduit.
"""
import collections, copyreg, json, os, pickle, struct, sys, zlib

AP, SEED, OUT = sys.argv[1], sys.argv[2], sys.argv[3]
sys.path.insert(0, AP)
import Utils  # noqa: E402

SCALAR_TYPES = (type(None), bool, int, float, str, bytes, list, tuple, dict, set, frozenset)


def canon(o):
    if type(o) not in SCALAR_TYPES:
        # Objet d'une classe autorisée : on reproduit ce que pickle écrit.
        r = o.__reduce_ex__(4)
        if r[0] is copyreg.__newobj__:
            cls, args = r[1][0], r[1][1:]
        else:
            cls, args = r[0], r[1]
        state = r[2] if len(r) > 2 else None
        return {"object": f"{cls.__module__}.{cls.__qualname__}",
                "args": [canon(a) for a in args],
                "state": None if state is None else canon(state)}
    if o is None or isinstance(o, (bool, str)):
        return o
    if isinstance(o, int):
        if -2**63 <= o < 2**63:
            return o
        return {"bigint": pickle.encode_long(o).hex()}
    if isinstance(o, float):
        return {"float": struct.pack("<d", o).hex()}
    if isinstance(o, bytes):
        return {"bytes": o.hex()}
    if isinstance(o, list):
        return {"list": [canon(x) for x in o]}
    if isinstance(o, tuple):
        return {"tuple": [canon(x) for x in o]}
    if isinstance(o, dict):
        return {"dict": [[canon(k), canon(v)] for k, v in o.items()]}
    if isinstance(o, frozenset):
        return {"frozenset": [canon(x) for x in o]}
    if isinstance(o, set):
        return {"set": [canon(x) for x in o]}
    raise TypeError(type(o))


def write(name, data, obj):
    with open(os.path.join(OUT, name), "wb") as f:
        f.write(data)
    with open(os.path.join(OUT, name + ".attendu.json"), "w", encoding="utf-8", newline="\n") as f:
        json.dump(canon(obj), f, ensure_ascii=False, indent=1)
        f.write("\n")


os.makedirs(OUT, exist_ok=True)

# 1. La seed : lue avec la fonction officielle du serveur.
seed = open(SEED, "rb").read()
write("seed-mit.archipelago", seed, Utils.restricted_loads(zlib.decompress(seed[1:])))

# 2. Petits pickles fabriqués exprès (données simples, sans classe).
shared = [1, 2]
VECTORS = {
    "scalaires": [None, True, False, 0, 1, -1, 255, 256, 65535, 65536, -2**31, 2**31 - 1, 2**31,
                  2**63 - 1, -2**63, 2**63, 2**100, -2**100, 1.5, -0.0, 1e300,
                  "", "é漢😀", "x" * 300, b"\x00\xff", b"y" * 300],
    "conteneurs": {"liste": [1, [2, [3]]], "tuple": (1, (2,)), "ensemble": {1, 2, 3},
                   "gele": frozenset({"a"}), "vides": ((), [], {}, set(), frozenset()),
                   7: "clé entière", (1, 2): "clé tuple"},
    "partage": {"a": shared, "b": shared},
    "compteur": collections.Counter({"a": 2, "b": 1}),
}
for name, obj in VECTORS.items():
    for protocol in (4, 5):
        write(f"{name}-p{protocol}.pickle", pickle.dumps(obj, protocol=protocol), obj)
# Protocole 3 : ensembles écrits avec `builtins.set` (REDUCE), opcode GLOBAL.
sets = {"ensemble": {1, 2}, "gele": frozenset({3}), "texte": "é"}
write("ensembles-p3.pickle", pickle.dumps(sets, protocol=3), sets)

# Protocole 2 : écrit `__builtin__.set` (nom Python 2), que le serveur officiel
# refuse. Le résultat attendu est donc l'erreur de `restricted_loads`.
data = pickle.dumps(sets, protocol=2)
try:
    Utils.restricted_loads(data)
    raise SystemExit("le serveur officiel aurait dû refuser le protocole 2")
except pickle.UnpicklingError as e:
    with open(os.path.join(OUT, "ensembles-p2.pickle"), "wb") as f:
        f.write(data)
    with open(os.path.join(OUT, "ensembles-p2.pickle.attendu.json"), "w", encoding="utf-8", newline="\n") as f:
        json.dump({"erreur": str(e)}, f, ensure_ascii=False)
        f.write("\n")
print("ok")
