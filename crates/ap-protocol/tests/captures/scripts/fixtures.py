"""Découpe les trames capturées en un fichier de test par type de paquet."""
import json, sys, os
src, out = sys.argv[1], sys.argv[2]
os.makedirs(out, exist_ok=True)
seen, multi = set(), None
def dump(obj):
    return json.dumps(obj, ensure_ascii=False, separators=(",", ":"))
for line in open(src, encoding="utf-8"):
    raw = json.loads(line)["raw"]
    frame = json.loads(raw)
    if multi is None and len(frame) > 1:
        multi = raw
    for p in frame:
        name = p["cmd"] + ("_" + p["type"] if p["cmd"] == "PrintJSON" and "type" in p else "")
        if name in seen:
            continue
        seen.add(name)
        if p["cmd"] == "DataPackage":
            for g in p["data"]["games"].values():
                for k in ("item_name_to_id", "location_name_to_id"):
                    g[k] = dict(list(g[k].items())[:3])
        with open(os.path.join(out, name + ".json"), "w", encoding="utf-8", newline="\n") as f:
            f.write(dump([p]) + "\n")
if multi:
    with open(os.path.join(out, "trame_multiple.json"), "w", encoding="utf-8", newline="\n") as f:
        f.write(multi + "\n")
print(sorted(seen), "multi:", multi is not None)
