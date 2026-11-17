"""Vecteurs de test de `src/pyset.rs` : ordre de parcours d'un `set` Python.

Joue des suites d'opérations tirées au hasard sur des `set` d'indices et note,
après chaque opération, l'ordre dans lequel Python parcourt le `set`.
`src/pyset.rs` rejoue les mêmes opérations et doit trouver le même ordre.

`Hint` reprend la classe de `NetUtils.py` (Archipelago 0.6.8) : un tuple nommé
de 8 champs, comparé sur les 8, haché sur 5.

À lancer avec CPython 3.12 et `PYTHONHASHSEED=0` (le hachage des textes est
sinon tiré au hasard à chaque lancement). Pas d'option `-I` : elle ferait
ignorer cette variable.

    set PYTHONHASHSEED=0
    py -3.12 crates/server/tests/data/scripts/pyset_vectors.py > crates/server/tests/data/pyset-vectors.json
"""

import json
import os
import random
import sys
import typing


class Hint(typing.NamedTuple):
    receiving_player: int
    finding_player: int
    location: int
    item: int
    found: bool
    entrance: str = ""
    item_flags: int = 0
    status: int = 0

    def __hash__(self):
        return hash((self.receiving_player, self.finding_player, self.location, self.item, self.entrance))


ENTRANCES = ["", "", "", "", "", "Cave", "Ébène", "洞窟", "🏰", "Tour du nord-est"]
STATUSES = [0, 10, 20, 30, 40]


def random_hint(rng: random.Random, pool: list) -> Hint:
    # Souvent un indice déjà vu, parfois le même avec un autre statut (même
    # hachage, mais différent pour `==`).
    if pool and rng.random() < 0.4:
        base = rng.choice(pool)
        if rng.random() < 0.5:
            return base
        return base._replace(status=rng.choice(STATUSES), found=not base.found)
    location = rng.choice([
        rng.randrange(0, 64),
        rng.randrange(0, 8) * 8,
        0x10000 + rng.randrange(0, 300),
        rng.randrange(-2, 0),
        (1 << 61) + rng.randrange(0, 4),
    ])
    hint = Hint(
        receiving_player=rng.randrange(1, 5),
        finding_player=rng.randrange(1, 5),
        location=location,
        item=rng.choice([rng.randrange(0, 50), 77_000 + rng.randrange(0, 50)]),
        found=rng.random() < 0.3,
        entrance=rng.choice(ENTRANCES),
        item_flags=rng.choice([0, 1, 2, 4]),
        status=rng.choice(STATUSES),
    )
    pool.append(hint)
    return hint


def run_case(rng: random.Random, steps: int) -> dict:
    elements: list = []  # indices dans l'ordre de première apparition
    index: dict = {}

    def ref(hint: Hint) -> int:
        # Clé : les 8 champs. Un dict compare avec `==`, donc deux indices
        # égaux ont le même numéro.
        if hint not in index:
            index[hint] = len(elements)
            elements.append(hint)
        return index[hint]

    pool: list = []
    current: set = set()
    ops: list = []
    for _ in range(steps):
        op: dict
        roll = rng.random()
        if roll < 0.55:
            hint = random_hint(rng, pool)
            current.add(hint)
            op = {"op": "add", "h": ref(hint)}
        elif roll < 0.75:
            if current and rng.random() < 0.8:
                hint = rng.choice(sorted(current, key=lambda h: index.get(h, -1)))
            else:
                hint = random_hint(rng, pool)
            current.discard(hint)
            op = {"op": "discard", "h": ref(hint)}
        elif roll < 0.82:
            current = {hint for hint in current}
            op = {"op": "rebuild"}
        elif roll < 0.86:
            current = current.copy()
            op = {"op": "copy"}
        elif roll < 0.91:
            items = [random_hint(rng, pool) for _ in range(rng.randrange(0, 12))]
            current.update(set(items))
            op = {"op": "update", "items": [ref(h) for h in items]}
        elif roll < 0.96:
            items = [random_hint(rng, pool) for _ in range(rng.randrange(0, 6))]
            items += rng.sample(list(current), min(len(current), rng.randrange(0, 4)))
            current = current - set(items)
            op = {"op": "difference", "items": [ref(h) for h in items]}
        else:
            items = [random_hint(rng, pool) for _ in range(rng.randrange(0, 40))]
            current = set(items)
            op = {"op": "from_list", "items": [ref(h) for h in items]}
        op["order"] = [ref(h) for h in current]
        ops.append(op)
    return {
        "elements": [
            [h.receiving_player, h.finding_player, h.location, h.item, h.found, h.entrance, h.item_flags, h.status]
            for h in elements
        ],
        "hashes": [hash(h) for h in elements],
        "ops": ops,
    }


def main() -> None:
    if sys.version_info[:2] != (3, 12):
        raise SystemExit("il faut CPython 3.12")
    if os.environ.get("PYTHONHASHSEED") != "0":
        raise SystemExit("il faut PYTHONHASHSEED=0")
    rng = random.Random(18)
    cases = [run_case(rng, rng.randrange(20, 120)) for _ in range(30)]
    # Une longue suite pour passer par de grandes tables.
    cases.append(run_case(rng, 500))
    json.dump({"python": sys.version.split()[0], "cases": cases}, sys.stdout, ensure_ascii=True,
              separators=(",", ":"))
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
