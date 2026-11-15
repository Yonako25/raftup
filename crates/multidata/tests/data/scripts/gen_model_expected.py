"""Écrit le modèle de partie attendu pour une seed, calculé par le code officiel.

Usage : python -I gen_model_expected.py <dossier source Archipelago> <seed .archipelago> <sortie .json>

La forme JSON est celle que reproduit `crates/multidata/tests/modele.rs`.
"""
import json, sys, zlib

AP, SEED, OUT = sys.argv[1], sys.argv[2], sys.argv[3]
sys.path.insert(0, AP)
import Utils  # noqa: E402
from NetUtils import encode  # noqa: E402

raw = open(SEED, "rb").read()
d = Utils.restricted_loads(zlib.decompress(raw[1:]))


def via_encode(value):
    """Ce que le serveur officiel envoie en JSON pour cette valeur."""
    return json.loads(encode(value))


options = d.get("server_options", {})
expected = {
    "seed_name": d["seed_name"],
    "generator_version": list(d["version"]),
    "minimum_server_version": list(d["minimum_versions"]["server"]),
    "minimum_client_versions": {str(k): list(v) for k, v in d["minimum_versions"].get("clients", {}).items()},
    "tags": list(d.get("tags", [])),
    "race_mode": bool(d.get("race_mode", 0)),
    "slots": {str(k): via_encode(v) for k, v in d["slot_info"].items()},
    "connect_names": {k: list(v) for k, v in d["connect_names"].items()},
    "slot_data": {str(k): via_encode(v) for k, v in d["slot_data"].items()},
    "locations": {str(s): {str(l): list(t) for l, t in locs.items()} for s, locs in d["locations"].items()},
    "precollected_items": {str(k): list(v) for k, v in d.get("precollected_items", {}).items()},
    # Ensemble sans ordre : trié pour comparer.
    "precollected_hints": {str(k): sorted((via_encode(h) for h in v), key=lambda h: json.dumps(h, sort_keys=True))
                           for k, v in d.get("precollected_hints", {}).items()},
    "er_hint_data": {str(s): {str(l): e for l, e in v.items()} for s, v in d.get("er_hint_data", {}).items()},
    "spheres": [{str(s): sorted(locs) for s, locs in sphere.items()} for sphere in d.get("spheres", [])],
    "server_options": {k: options.get(k) for k in
                       ("password", "server_password", "hint_cost", "location_check_points", "release_mode",
                        "collect_mode", "remaining_mode", "countdown_mode", "disable_item_cheat", "compatibility")},
    "datapackage": {game: {"item_name_to_id": p.get("item_name_to_id", {}),
                           "location_name_to_id": p.get("location_name_to_id", {}),
                           "item_name_groups": {g: list(m) for g, m in p.get("item_name_groups", {}).items()},
                           "location_name_groups": {g: list(m) for g, m in p.get("location_name_groups", {}).items()},
                           "checksum": p.get("checksum", "")}
                    for game, p in d.get("datapackage", {}).items()},
}
with open(OUT, "w", encoding="utf-8", newline="\n") as f:
    json.dump(expected, f, ensure_ascii=False, indent=1)
    f.write("\n")
print("ok")
