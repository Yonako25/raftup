"""Options du monde du joueur 1, pour le fuzzing du registre de Raftup (ADR 0045).

Envoye par Raftup dans la requete `export_worlds` du worker (comme raftup_export.py) et
lance sur le MultiWorld construit jusqu'au placement. Ecrit un JSON :
  {"game": str, "options": [{"name", "kind": toggle|choice|range|other, ...}]}
"""
import json

MAX_OPTIONS = 500
MAX_CHOICES = 200
MAX_BYTES = 1024 * 1024
MAX_INT = 2 ** 53


def _clamp(value):
    return max(-MAX_INT, min(MAX_INT, int(value)))


def _option(Options, name, cls):
    try:
        if issubclass(cls, Options.Toggle):
            return {"name": name, "kind": "toggle"}
        if issubclass(cls, Options.Choice):
            names = sorted({str(v) for v in cls.name_lookup.values()})[:MAX_CHOICES]
            return {"name": name, "kind": "choice", "choices": names}
        if issubclass(cls, Options.Range):
            return {"name": name, "kind": "range",
                    "start": _clamp(cls.range_start), "end": _clamp(cls.range_end)}
    except Exception:
        pass
    return {"name": name, "kind": "other"}


def export(mw, path, max_bytes):
    import Options

    world = mw.worlds[1]
    hints = list(type(world).options_dataclass.type_hints.items())[:MAX_OPTIONS]
    data = {"game": str(mw.game[1]),
            "options": [_option(Options, str(name), cls) for name, cls in hints]}
    raw = json.dumps(data, ensure_ascii=True).encode("ascii")
    if len(raw) > min(max_bytes, MAX_BYTES):
        raise ValueError("description des options trop grosse")
    with open(path, "wb") as f:
        f.write(raw)
    return {"bytes": len(raw), "opaque_rules": 0}
