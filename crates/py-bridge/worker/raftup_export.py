"""Export d'un MultiWorld d'Archipelago pour le moteur Rust de Raftup (ADR 0028).

Ce module n'est pas lance seul : le worker (raftup_worker.py) le recoit en texte dans
la requete `export_worlds`, l'execute, puis appelle `export(mw, path, max_bytes)` une
fois que les mondes ont fini `pre_fill`, juste avant le placement d'Archipelago.

Le fichier ecrit (JSON, `format_version` 1) decrit, pour chaque joueur : regions,
entrees, lieux, objets du pool, objets deja poses, inventaire de depart, condition de
victoire et datapackage. Les regles `rule_builder` sont traduites telles quelles ; une
regle Python ordinaire (lambda) est marquee `opaque` (le sondage viendra a l'etape 2).
Bibliotheque standard seule.
"""
import json

FORMAT_VERSION = 1


def _rule_table():
    """Classes `Resolved` de rule_builder connues, par type exact.

    Un monde peut definir ses propres regles (sous-classes) : leur sens nous est
    inconnu, elles restent opaques. On compare donc le type exact, pas isinstance.
    """
    try:
        from rule_builder import rules as rb
    except ImportError:
        return {}
    names = ["True_", "False_", "And", "Or", "AtLeast", "Has", "HasAll", "HasAny",
             "HasAllCounts", "HasAnyCount", "HasFromList", "HasFromListUnique",
             "HasGroup", "HasGroupUnique", "CanReachLocation", "CanReachRegion",
             "CanReachEntrance"]
    table = {}
    for name in names:
        cls = getattr(rb, name, None)
        resolved = getattr(cls, "Resolved", None)
        if resolved is not None:
            table[resolved] = name
    return table


def _describe(rule):
    name = getattr(rule, "__qualname__", None) or type(rule).__qualname__
    module = getattr(rule, "__module__", None) or type(rule).__module__
    return "%s.%s" % (module, name)


class Exporter:
    def __init__(self, mw):
        import BaseClasses

        self.mw = mw
        self.default_rule = getattr(BaseClasses.Location, "access_rule", None)
        self.default_entrance_rule = getattr(BaseClasses.Entrance, "access_rule", None)
        self.table = _rule_table()
        self.opaque = 0

    def rule(self, rule):
        if rule is None or rule is self.default_rule or rule is self.default_entrance_rule:
            return {"t": "true"}
        kind = self.table.get(type(rule))
        if kind is None:
            self.opaque += 1
            return {"t": "opaque", "why": _describe(rule)}
        player = int(getattr(rule, "player", 0))
        if kind == "True_":
            return {"t": "true"}
        if kind == "False_":
            return {"t": "false"}
        if kind in ("And", "Or"):
            return {"t": kind.lower(), "of": [self.rule(c) for c in rule.children]}
        if kind == "AtLeast":
            return {"t": "at_least", "count": int(rule.count),
                    "of": [self.rule(c) for c in rule.children]}
        if kind == "Has":
            return {"t": "has", "player": player, "item": rule.item_name, "count": int(rule.count)}
        if kind in ("HasAll", "HasAny"):
            return {"t": "has_all" if kind == "HasAll" else "has_any", "player": player,
                    "items": list(rule.item_names)}
        if kind in ("HasAllCounts", "HasAnyCount"):
            return {"t": "has_all_counts" if kind == "HasAllCounts" else "has_any_count",
                    "player": player,
                    "items": [[name, int(count)] for name, count in rule.item_counts]}
        if kind in ("HasFromList", "HasGroup"):
            # Somme des exemplaires de la liste (ou du groupe) >= count.
            return {"t": "has_from_list", "player": player, "items": list(rule.item_names),
                    "count": int(rule.count)}
        if kind in ("HasFromListUnique", "HasGroupUnique"):
            # Nombre d'objets DIFFERENTS de la liste >= count.
            return {"t": "has_from_list_unique", "player": player,
                    "items": list(rule.item_names), "count": int(rule.count)}
        if kind == "CanReachLocation":
            return {"t": "can_reach_location", "player": player, "location": rule.location_name}
        if kind == "CanReachRegion":
            return {"t": "can_reach_region", "player": player, "region": rule.region_name}
        if kind == "CanReachEntrance":
            return {"t": "can_reach_entrance", "player": player, "entrance": rule.entrance_name}
        self.opaque += 1
        return {"t": "opaque", "why": _describe(rule)}

    @staticmethod
    def item(item):
        return {"player": int(item.player), "name": str(item.name),
                "code": item.code if isinstance(item.code, int) else None,
                "classification": int(item.classification)}

    def player(self, p):
        mw = self.mw
        world = mw.worlds[p]
        regions, entrances, locations = [], [], []
        for region in mw.get_regions(p):
            regions.append(region.name)
            for exit_ in region.exits:
                target = exit_.connected_region
                entrances.append({
                    "name": str(exit_.name),
                    "from": region.name,
                    "to": None if target is None else target.name,
                    "rule": self.rule(exit_.access_rule),
                })
            for loc in region.locations:
                # Regle d'objet propre au lieu (item_rule ou always_allow) : non prise en charge.
                default_item_rule = not ({"item_rule", "always_allow"} & set(vars(loc)))
                locations.append({
                    "name": str(loc.name),
                    "address": loc.address if isinstance(loc.address, int) else None,
                    "region": region.name,
                    "rule": self.rule(loc.access_rule),
                    "progress_type": int(loc.progress_type),
                    "locked": bool(loc.locked),
                    "item": None if loc.item is None else self.item(loc.item),
                    "default_item_rule": default_item_rule,
                })
        return {
            "slot": int(p),
            "name": str(mw.player_name[p]),
            "game": str(world.game),
            "origin": str(getattr(world, "origin_region_name", "Menu")),
            "regions": regions,
            "entrances": entrances,
            "locations": locations,
            "precollected": [self.item(i) for i in mw.precollected_items[p]],
            "goal": self.rule(mw.completion_condition[p]),
            "required_client_version": list(getattr(world, "required_client_version", (0, 1, 6))),
        }

    def export(self):
        mw = self.mw
        players = [self.player(p) for p in sorted(mw.player_ids)]
        games = sorted({str(mw.worlds[p].game) for p in mw.player_ids})
        packages = {}
        for game in games:
            cls = type(next(mw.worlds[p] for p in mw.player_ids if mw.worlds[p].game == game))
            packages[game] = cls.get_data_package_data()
        return {
            "format_version": FORMAT_VERSION,
            "seed": int(mw.seed),
            "seed_name": str(mw.seed_name),
            "players": players,
            "pool": [self.item(i) for i in mw.itempool],
            "datapackage": packages,
            "opaque_rules": self.opaque,
        }


def export(mw, path, max_bytes):
    data = Exporter(mw).export()
    text = json.dumps(data, ensure_ascii=True, separators=(",", ":"), allow_nan=False)
    if len(text) > max_bytes:
        raise ValueError("export trop gros : %d octets (max %d)" % (len(text), max_bytes))
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    return {"bytes": len(text), "opaque_rules": data["opaque_rules"]}
