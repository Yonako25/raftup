"""Export d'un MultiWorld d'Archipelago pour le moteur Rust de Raftup (ADR 0028).

Ce module n'est pas lance seul : le worker (raftup_worker.py) le recoit en texte dans
la requete `export_worlds`, l'execute, puis appelle `export(mw, path, max_bytes)` une
fois que les mondes ont fini `pre_fill`, juste avant le placement d'Archipelago.

Le fichier ecrit (JSON, `format_version` 2) decrit, pour chaque joueur : regions,
entrees, lieux, objets du pool, objets deja poses, inventaire de depart, condition de
victoire, accessibilite, objets a placer tot et datapackage ; et, pour les lieux, leurs
regles d'objet sous forme de listes d'objets du pool (ADR 0029). Les regles `rule_builder` sont traduites telles quelles ; une
regle Python ordinaire (lambda) est traduite par SONDAGE (classe Prober) ; si le sondage
echoue, elle est marquee `opaque` et le monde reste au generateur Python.

Pour le controle croise (ADR 0028, section 4), l'export contient aussi des inventaires
tires au hasard et, pour chaque regle, sa valeur calculee par le VRAI Archipelago ; Rust
evalue ses regles traduites sur les memes inventaires et compare.
Bibliotheque standard seule.
"""
import json
import random
from collections import Counter

MAX_PATHS = 4096  # feuilles de l'arbre de decision d'une regle
MAX_QUESTIONS = 64  # questions differentes sur un chemin
SAMPLES = 64  # inventaires du controle croise
SAMPLE_SEED = 0x52414654  # graine fixe : meme export, memes inventaires

FORMAT_VERSION = 2  # 2 : regles d'objet, objets a placer tot, accessibilite (ADR 0029)


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


class Refused(Exception):
    """Le sondage ne peut pas traduire cette regle : elle restera opaque."""


class ProbeCount:
    """Resultat de state.count(...) : seule une comparaison a un nombre est comprise."""

    def __init__(self, state, player, item):
        self.state, self.player, self.item = state, player, item

    def at_least(self, n):
        return True if n <= 0 else self.state.ask(("has", self.player, self.item, int(n)))

    def __ge__(self, n):
        return self.at_least(n)

    def __gt__(self, n):
        return self.at_least(n + 1)

    def __le__(self, n):
        return not self.at_least(n + 1)

    def __lt__(self, n):
        return not self.at_least(n)

    def __eq__(self, n):
        return self.at_least(n) and not self.at_least(n + 1)

    def __ne__(self, n):
        return not self.__eq__(n)

    def __bool__(self):
        return self.at_least(1)

    def __getattr__(self, name):
        raise Refused("calcul sur un compteur (%s)" % name)

    __hash__ = None


class ProbeState:
    """Faux CollectionState : chaque question posee par la regle est notee, et la
    reponse vient du sondeur. Toute autre methode ou attribut refuse le sondage."""

    def __init__(self, mw, answers):
        self._mw = mw
        self._answers = answers  # reponses imposees, dans l'ordre des nouvelles questions
        self._used = 0
        self.asked = {}  # question -> reponse, dans l'ordre (dict garde l'ordre)

    def ask(self, question):
        if question in self.asked:
            return self.asked[question]
        if len(self.asked) >= MAX_QUESTIONS:
            raise Refused("plus de %d questions" % MAX_QUESTIONS)
        if self._used < len(self._answers):
            answer = self._answers[self._used]
        else:
            answer = False
        self._used += 1
        self.asked[question] = answer
        return answer

    def __getattr__(self, name):
        raise Refused("state.%s" % name)

    def has(self, item, player, count=1):
        return True if count <= 0 else self.ask(("has", player, item, int(count)))

    def has_all(self, items, player):
        return self.ask(("has_all", player, tuple(items)))

    def has_any(self, items, player):
        return self.ask(("has_any", player, tuple(items)))

    def has_all_counts(self, item_counts, player):
        return self.ask(("has_all_counts", player, tuple((k, int(v)) for k, v in item_counts.items())))

    def has_any_count(self, item_counts, player):
        return self.ask(("has_any_count", player, tuple((k, int(v)) for k, v in item_counts.items())))

    def has_from_list(self, items, player, count):
        return True if count <= 0 else self.ask(("has_from_list", player, tuple(items), int(count)))

    def has_from_list_unique(self, items, player, count):
        return True if count <= 0 else self.ask(("has_from_list_unique", player, tuple(items), int(count)))

    def _group(self, group, player):
        return tuple(self._mw.worlds[player].item_name_groups[group])

    def has_group(self, item_name_group, player, count=1):
        if count <= 0:
            return True
        return self.ask(("has_from_list", player, self._group(item_name_group, player), int(count)))

    def has_group_unique(self, item_name_group, player, count=1):
        if count <= 0:
            return True
        return self.ask(("has_from_list_unique", player, self._group(item_name_group, player), int(count)))

    def count(self, item, player):
        return ProbeCount(self, player, item)

    def can_reach(self, spot, resolution_hint=None, player=None):
        if isinstance(spot, str):
            if not isinstance(player, int):
                raise Refused("can_reach sans joueur")
            kind = {"Location": "location", "Entrance": "entrance"}.get(resolution_hint, "region")
            return self.ask(("can_reach_" + kind, player, spot))
        import BaseClasses
        for cls, kind in ((BaseClasses.Location, "location"), (BaseClasses.Entrance, "entrance"),
                          (BaseClasses.Region, "region")):
            if isinstance(spot, cls):
                return self.ask(("can_reach_" + kind, int(spot.player), spot.name))
        raise Refused("can_reach sur %r" % type(spot).__name__)

    def can_reach_location(self, spot, player):
        return self.ask(("can_reach_location", player, spot))

    def can_reach_entrance(self, spot, player):
        return self.ask(("can_reach_entrance", player, spot))

    def can_reach_region(self, spot, player):
        return self.ask(("can_reach_region", player, spot))


def _atom_json(q):
    """Question du sondeur -> regle JSON (le vocabulaire de rule_builder)."""
    kind, player = q[0], int(q[1])
    if kind == "has":
        return {"t": "has", "player": player, "item": q[2], "count": q[3]}
    if kind in ("has_all", "has_any"):
        return {"t": kind, "player": player, "items": list(q[2])}
    if kind in ("has_all_counts", "has_any_count"):
        return {"t": kind, "player": player, "items": [[k, v] for k, v in q[2]]}
    if kind in ("has_from_list", "has_from_list_unique"):
        return {"t": kind, "player": player, "items": list(q[2]), "count": q[3]}
    field = kind[len("can_reach_"):]
    return {"t": kind, "player": player, field: q[2]}


def _truth(result):
    """Vrai ou faux, comme Archipelago lit une regle (`if rule(state)`). Beaucoup de
    mondes renvoient `a or b` ou une valeur d'option (un entier) : on accepte booleens,
    entiers, None et compteurs ; tout autre type est suspect et refuse."""
    if isinstance(result, ProbeCount):
        return bool(result)
    if result is None or isinstance(result, (bool, int)):
        return bool(result)
    raise Refused("resultat %r" % type(result).__name__)


def probe(mw, rule):
    """Arbre de decision de `rule` par exploration en profondeur, puis regle « ou » de
    « et ». Leve Refused si la regle sort de ce qu'on sait traduire.

    Le sondeur impose les reponses aux questions dans l'ordre ou elles sont posees ;
    une question nouvelle recoit Faux, puis on revient essayer Vrai. Comme `and` et `or`
    de Python s'arretent des qu'ils connaissent le resultat, on suit exactement les
    chemins que la regle emprunte.
    """
    pending = [[]]
    leaves = []  # (vraies, fausses, resultat)
    while pending:
        answers = pending.pop()
        state = ProbeState(mw, answers)
        try:
            result = rule(state)
        except Refused:
            raise
        except Exception as exc:  # la regle a plante avec nos reponses
            raise Refused("%s: %s" % (type(exc).__name__, exc))
        result = _truth(result)
        asked = list(state.asked.items())
        # Meme chemin rejoue : meme resultat (pas de hasard ni d'etat cache).
        again = ProbeState(mw, [a for _, a in asked])
        if _truth(rule(again)) != result or list(again.asked.items()) != asked:
            raise Refused("resultat instable")
        leaves.append(({q for q, a in asked if a}, {q for q, a in asked if not a}, result))
        if len(leaves) > MAX_PATHS:
            raise Refused("plus de %d chemins" % MAX_PATHS)
        # Les questions posees au-dela des reponses imposees ont recu Faux : on
        # programme le meme debut avec Vrai a leur place.
        for i in range(len(answers), len(asked)):
            pending.append([a for _, a in asked[:i]] + [True])
    terms = [frozenset(t) for t, _, r in leaves if r]
    # Regle monotone (avoir un objet de plus ne la rend jamais fausse) : chaque feuille
    # fausse doit contredire chaque terme vrai par une de ses reponses Faux.
    for _, false_set, r in leaves:
        if not r:
            for term in terms:
                if not (term & false_set):
                    raise Refused("regle non monotone")
    # Termes minimaux seulement, dans un ordre stable.
    terms = sorted(set(terms), key=lambda t: (len(t), sorted(map(repr, t))))
    minimal = [t for t in terms if not any(o < t for o in terms)]
    if not minimal:
        return {"t": "false"}
    if any(len(t) == 0 for t in minimal):
        return {"t": "true"}
    ors = [{"t": "and", "of": [_atom_json(q) for q in sorted(t, key=repr)]} for t in minimal]
    return ors[0] if len(ors) == 1 else {"t": "or", "of": ors}


class NoState:
    """Inventaire donne a `always_allow` : la regle ne doit pas le lire (Raftup en fait
    une liste d'objets, sans inventaire). Toute lecture refuse l'export du monde."""

    def __getattr__(self, name):
        raise Refused("always_allow lit l'inventaire (state.%s)" % name)


def _func_key(f):
    """Identite d'une fonction de regle : meme fonction (meme objet) => meme resultat.
    Une methode liee change d'objet a chaque lecture : on prend sa fonction et son objet."""
    return (id(getattr(f, "__func__", f)), id(getattr(f, "__self__", None)))


class Exporter:
    def __init__(self, mw):
        import BaseClasses

        self.mw = mw
        self.default_rule = getattr(BaseClasses.Location, "access_rule", None)
        self.default_entrance_rule = getattr(BaseClasses.Entrance, "access_rule", None)
        self.default_can_fill = BaseClasses.Location.can_fill
        self.table = _rule_table()
        self.opaque = 0
        self.probed = 0
        # Regles a recontroler par Rust : (genre, joueur, nom, fonction Python).
        self.checked = []
        # Sortes d'objets du pool, dans l'ordre : (joueur, nom) -> exemplaires.
        self.kinds = {}
        for item in mw.itempool:
            self.kinds.setdefault((int(item.player), str(item.name)), []).append(item)
        # Regles d'objet (ADR 0029) : tables partagees entre les lieux.
        self.item_rules = []
        self._rule_index = {}  # contenu de la regle (JSON) -> index
        self._rule_cache = {}  # identite des fonctions -> index ou None
        self.item_rule_errors = {}  # joueur -> premiere erreur
        self.ruled = []  # lieux vides a regle d'objet, pour le controle croise

    def item_rule(self, p, loc):
        """Index de la regle d'objet du lieu dans `item_rules`, ou None si tout objet du
        pool passe. La regle est calculee par Archipelago lui-meme sur un exemplaire de
        chaque sorte d'objet du pool, comme `Location.can_fill` sans l'acces :
        `always_allow` (sauf objet non local) ou `item_rule`."""
        if type(loc).can_fill is not self.default_can_fill:
            self.item_rule_errors.setdefault(p, "lieu %s : can_fill redefini" % loc.name)
            return None
        rule = loc.item_rule
        allow = loc.always_allow if "always_allow" in vars(loc) else None
        key = (_func_key(rule), None if allow is None else _func_key(allow))
        if key in self._rule_cache:
            index = self._rule_cache[key]
        else:
            try:
                index = self._compute_item_rule(rule, allow)
            except Exception as exc:  # la regle plante, ou lit l'inventaire
                self.item_rule_errors.setdefault(
                    p, "lieu %s : %s: %s" % (loc.name, type(exc).__name__, exc))
                return None
            self._rule_cache[key] = index
        if index is not None and loc.item is None:
            self.ruled.append(loc)
        return index

    def _compute_item_rule(self, rule, allow):
        allowed, forbidden, always = [], [], []
        for (player, name), items in self.kinds.items():
            item = items[0]
            (allowed if rule(item) else forbidden).append([player, name])
            if allow is not None and allow(NoState(), item):
                if name not in self.mw.worlds[player].options.non_local_items.value:
                    always.append([player, name])
        if not forbidden and not always:
            return None
        # La liste la plus courte : refuses, ou seuls acceptes.
        allow_only = len(allowed) < len(forbidden)
        entry = {"allow_only": allow_only, "list": allowed if allow_only else forbidden,
                 "always": always}
        text = json.dumps(entry, sort_keys=True)
        if text not in self._rule_index:
            self._rule_index[text] = len(self.item_rules)
            self.item_rules.append(entry)
        return self._rule_index[text]

    def item_checks(self):
        """Controle croise des regles d'objet : `Location.can_fill` du vrai Archipelago
        (sans l'acces) sur des paires (lieu, exemplaire) tirees au hasard, graine fixe.
        Rust refait le calcul avec ses listes. On tire surtout parmi les lieux a regle."""
        from BaseClasses import CollectionState

        mw = self.mw
        empty = [loc for loc in mw.get_locations() if loc.item is None]
        if not empty or not mw.itempool:
            return []
        state = CollectionState(mw)
        rng = random.Random(SAMPLE_SEED + 1)
        out = []
        for i in range(4 * SAMPLES):
            pick = self.ruled if self.ruled and i % 4 else empty
            loc = rng.choice(pick)
            item = rng.choice(mw.itempool)
            try:
                bit = "1" if loc.can_fill(state, item, False) else "0"
            except Exception:
                bit = "e"
            out.append([int(loc.player), str(loc.name), int(item.player), str(item.name), bit])
        return out

    def rule(self, rule):
        if rule is None or rule is self.default_rule or rule is self.default_entrance_rule:
            return {"t": "true"}
        kind = self.table.get(type(rule))
        if kind is None:
            if callable(rule):
                try:
                    out = probe(self.mw, rule)
                    self.probed += 1
                    return out
                except Refused as why:
                    self.opaque += 1
                    return {"t": "opaque", "why": "%s (%s)" % (_describe(rule), why)}
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

    @staticmethod
    def options(world):
        """Options propres au monde, avec leur valeur (ADR 0042, conversion en monde natif).

        Lecture seule : rien n'est cree ni tire au hasard. Les options communes a tous
        les jeux (PerGameCommonOptions) sont laissees de cote."""
        import Options

        common = set(Options.PerGameCommonOptions.type_hints)
        out = []
        for name, cls in getattr(world.options_dataclass, "type_hints", {}).items():
            if name in common:
                continue
            entry = {"name": str(name), "display": str(getattr(cls, "display_name", name))}
            try:
                entry["value"] = int(getattr(world.options, name).value)
                entry["default"] = int(cls.default)
            except Exception:
                entry["kind"] = "other"
                out.append(entry)
                continue
            if issubclass(cls, Options.Toggle):
                entry["kind"] = "toggle"
            elif issubclass(cls, Options.Range):
                entry.update(kind="range", min=int(cls.range_start), max=int(cls.range_end),
                             special=sorted(getattr(cls, "special_range_names", {})))
            elif issubclass(cls, Options.Choice):
                entry.update(kind="choice",
                             choices=[[int(k), str(v)] for k, v in sorted(cls.name_lookup.items())],
                             aliases=sorted([str(k), int(v)] for k, v in cls.aliases.items()))
            else:
                entry["kind"] = "other"
            out.append(entry)
        return out

    def player(self, p):
        mw = self.mw
        world = mw.worlds[p]
        regions, entrances, locations = [], [], []
        from worlds.AutoWorld import World

        # Un monde qui change ce que « ramasser un objet » veut dire (objets progressifs
        # renommes, compteurs maison) ne colle pas au modele de Raftup : refuse.
        custom_collect = any(getattr(type(world), name, None) is not getattr(World, name, None)
                             for name in ("collect", "collect_item", "remove"))
        for region in mw.get_regions(p):
            regions.append(region.name)
            for exit_ in region.exits:
                target = exit_.connected_region
                entrances.append({
                    "name": str(exit_.name),
                    "from": region.name,
                    "to": None if target is None else target.name,
                    "rule": self.rule(exit_.access_rule),
                    "n": getattr(exit_, "_raftup_n", None),
                })
                self.check("entrance", p, exit_.name, exit_.access_rule)
            for loc in region.locations:
                locations.append({
                    "name": str(loc.name),
                    "address": loc.address if isinstance(loc.address, int) else None,
                    "region": region.name,
                    "rule": self.rule(loc.access_rule),
                    "progress_type": int(loc.progress_type),
                    "locked": bool(loc.locked),
                    "item": None if loc.item is None else self.item(loc.item),
                    # Regle d'objet (item_rule, always_allow, local_items...) : index dans
                    # `item_rules`, ou None.
                    "item_rule": self.item_rule(p, loc),
                    # Ordre de creation (ADR 0042), note par le worker.
                    "n": getattr(loc, "_raftup_n", None),
                })
                self.check("location", p, loc.name, loc.access_rule)
        return {
            "slot": int(p),
            "name": str(mw.player_name[p]),
            "game": str(world.game),
            "origin": str(getattr(world, "origin_region_name", "Menu")),
            "regions": regions,
            "region_n": [getattr(r, "_raftup_n", None) for r in mw.get_regions(p)],
            "entrances": entrances,
            "locations": locations,
            "precollected": [self.item(i) for i in mw.precollected_items[p]],
            "goal": self.rule(mw.completion_condition[p]),
            "custom_collect": custom_collect,
            "progression_balancing": int(getattr(world.options, "progression_balancing", 50)),
            # 0 full, 1 items, 2 minimal (Options.Accessibility).
            "accessibility": int(getattr(world.options, "accessibility", 0)),
            # Objets a placer tot (ADR 0029) : nom -> nombre, apres les retouches de
            # Main.py (inventaire de depart tire du pool deja retire).
            "early_items": {str(k): int(v) for k, v in mw.early_items[p].items() if v > 0},
            "local_early_items": {str(k): int(v) for k, v in mw.local_early_items[p].items()
                                  if v > 0},
            "item_rule_error": self.item_rule_errors.get(p),
            "options": self.options(world),
            "required_client_version": list(getattr(world, "required_client_version", (0, 1, 6))),
        }

    def check(self, kind, player, name, rule):
        if rule is None or rule is self.default_rule or rule is self.default_entrance_rule:
            return
        self.checked.append((kind, int(player), str(name), rule))

    def samples(self):
        """Inventaires tires au hasard (graine fixe) et valeur de chaque regle calculee par
        le vrai Archipelago : Rust refera le calcul avec ses regles traduites."""
        from BaseClasses import CollectionState

        mw = self.mw
        names = {}
        for p in sorted(mw.player_ids):
            world = mw.worlds[p]
            known = set(world.item_name_to_id)
            for region in mw.get_regions(p):
                for loc in region.locations:
                    if loc.item is not None:
                        names.setdefault(loc.item.player, set()).add(loc.item.name)
            names.setdefault(p, set()).update(known)
        rng = random.Random(SAMPLE_SEED)
        samples, results = [], [[] for _ in self.checked]
        for _ in range(SAMPLES):
            density = rng.choice((0.05, 0.2, 0.5, 0.8, 0.95))
            sample = []
            for p in sorted(names):
                for name in sorted(names[p]):
                    if rng.random() < density:
                        roll = rng.random()
                        count = 1 if roll < 0.6 else rng.randint(2, 5) if roll < 0.85 else rng.randint(6, 40)
                        sample.append([p, name, count])
            samples.append(sample)
            state = CollectionState(mw)
            state.prog_items = {p: Counter() for p in mw.get_all_ids()}
            for p, name, count in sample:
                state.prog_items[p][name] = count
            for p in state.stale:
                state.stale[p] = True
            for i, (_, _, _, rule) in enumerate(self.checked):
                try:
                    results[i].append("1" if rule(state) else "0")
                except Exception:
                    results[i].append("e")
        rules = [{"kind": kind, "player": player, "name": name, "results": "".join(bits)}
                 for (kind, player, name, _), bits in zip(self.checked, results)]
        return {"samples": samples, "rules": rules, "item_checks": self.item_checks()}

    def export(self):
        mw = self.mw
        players = [self.player(p) for p in sorted(mw.player_ids)]
        for p in sorted(mw.player_ids):
            self.check("goal", p, mw.player_name[p], mw.completion_condition[p])
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
            "item_rules": self.item_rules,
            "datapackage": packages,
            "opaque_rules": self.opaque,
            "probed_rules": self.probed,
            "checks": self.samples(),
        }


def export(mw, path, max_bytes):
    data = Exporter(mw).export()
    text = json.dumps(data, ensure_ascii=True, separators=(",", ":"), allow_nan=False)
    if len(text) > max_bytes:
        raise ValueError("export trop gros : %d octets (max %d)" % (len(text), max_bytes))
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    return {"bytes": len(text), "opaque_rules": data["opaque_rules"],
            "probed_rules": data["probed_rules"]}
