"""Génère fuzzy-vectors.json : cas de test de get_intended_text (Utils.py, serveur officiel 0.6.8).

Usage : python -I fuzzy_vectors.py <sortie.json>

jellyfish n'est pas installé ici : le Damerau-Levenshtein NON restreint
(Lowrance-Wagner, comme jellyfish) est réécrit ci-dessous. get_fuzzy_results et
get_intended_text sont copiés tels quels depuis Utils.py (seul l'import de
jellyfish est remplacé par la version locale).
"""
import json
import random
import sys
import typing


def damerau_levenshtein_distance(a: str, b: str) -> int:
    # Lowrance-Wagner : transpositions de deux caractères voisins, même si
    # d'autres éditions s'intercalent (contrairement à la variante OSA).
    da = {}
    maxdist = len(a) + len(b)
    d = [[0] * (len(b) + 2) for _ in range(len(a) + 2)]
    d[0][0] = maxdist
    for i in range(len(a) + 1):
        d[i + 1][0] = maxdist
        d[i + 1][1] = i
    for j in range(len(b) + 1):
        d[0][j + 1] = maxdist
        d[1][j + 1] = j
    for i in range(1, len(a) + 1):
        db = 0
        for j in range(1, len(b) + 1):
            i1 = da.get(b[j - 1], 0)
            j1 = db
            if a[i - 1] == b[j - 1]:
                cost = 0
                db = j
            else:
                cost = 1
            d[i + 1][j + 1] = min(
                d[i][j] + cost,
                d[i + 1][j] + 1,
                d[i][j + 1] + 1,
                d[i1][j1] + (i - i1 - 1) + 1 + (j - j1 - 1),
            )
        da[a[i - 1]] = i
    return d[len(a) + 1][len(b) + 1]


class jellyfish:
    damerau_levenshtein_distance = staticmethod(damerau_levenshtein_distance)


# ---- copie de Utils.py ----
def get_fuzzy_results(input_word: str, word_list: typing.Collection[str], limit: typing.Optional[int] = None) \
        -> typing.List[typing.Tuple[str, int]]:

    def get_fuzzy_ratio(word1: str, word2: str) -> float:
        if word1 == word2:
            return 1.01
        return (1 - jellyfish.damerau_levenshtein_distance(word1.lower(), word2.lower())
                / max(len(word1), len(word2)))

    limit = limit if limit else len(word_list)
    return list(
        map(
            lambda container: (container[0], int(container[1]*100)),  # convert up to limit to int %
            sorted(
                map(lambda candidate: (candidate, get_fuzzy_ratio(input_word, candidate)), word_list),
                key=lambda element: element[1],
                reverse=True
            )[0:limit]
        )
    )


def get_intended_text(input_text: str, possible_answers) -> typing.Tuple[str, bool, str]:
    picks = get_fuzzy_results(input_text, possible_answers, limit=2)
    if len(picks) > 1:
        dif = picks[0][1] - picks[1][1]
        if picks[0][1] == 101:
            return picks[0][0], True, "Perfect Match"
        elif picks[0][1] == 100:
            return picks[0][0], True, "Case Insensitive Perfect Match"
        elif picks[0][1] < 75:
            return picks[0][0], False, f"Didn't find something that closely matches '{input_text}', " \
                                       f"did you mean '{picks[0][0]}'? ({picks[0][1]}% sure)"
        elif dif > 5:
            return picks[0][0], True, "Close Match"
        else:
            return picks[0][0], False, f"Too many close matches for '{input_text}', " \
                                       f"did you mean '{picks[0][0]}'? ({picks[0][1]}% sure)"
    else:
        if picks[0][1] > 90:
            return picks[0][0], True, "Only Option Match"
        else:
            return picks[0][0], False, f"Didn't find something that closely matches '{input_text}', " \
                                       f"did you mean '{picks[0][0]}'? ({picks[0][1]}% sure)"
# ---- fin de la copie ----

CHECKS = ["Map Width", "Map Height", "Map Bombs", "Nothing", "Tile 1", "Tile 2", "Tile 10", "Tile 25"]
YACHT = ["Dice", "Dice Fragment", "Roll", "Roll Fragment", "Fixed Score Multiplier",
         "Step Score Multiplier", "Category Ones", "Category Twos", "Category Threes",
         "Category Full House", "Category Large Straight", "Category Yacht", "1 Point", "10 Points"]
PLAYERS = ["Alice", "Alicia", "Bob", "Bobby", "Émilie", "Emilie", "Zoë", "Zoe", "ÉLODIE", "Straße", "STRASSE"]
GAMES = ["Ocarina of Time", "Ocarina of Time 2", "Yacht Dice", "ChecksFinder", "Factorio", "Hollow Knight"]


def mutate(rng, word):
    chars = list(word)
    for _ in range(rng.randint(0, 3)):
        if not chars:
            break
        op = rng.choice("sdit")
        k = rng.randrange(len(chars))
        if op == "s":
            chars[k] = rng.choice("abcdeé xyz")
        elif op == "d":
            del chars[k]
        elif op == "i":
            chars.insert(k, rng.choice("abcdeé xyz"))
        elif k + 1 < len(chars):
            chars[k], chars[k + 1] = chars[k + 1], chars[k]
    s = "".join(chars)
    return rng.choice([s, s.lower(), s.upper(), s])


def main():
    rng = random.Random(20260616)
    cases = []

    def add(inp, cands):
        cases.append({"input": inp, "candidates": cands, "expected": list(get_intended_text(inp, cands))})

    # Cas fixes : égalité, casse, accents, ex aequo, une seule option.
    add("Alice", PLAYERS)
    add("alice", PLAYERS)
    add("ALICE", PLAYERS)
    add("Emilie", PLAYERS)
    add("émilie", PLAYERS)
    add("Zoe", PLAYERS)
    add("zoë", PLAYERS)
    add("élodie", PLAYERS)
    add("strasse", PLAYERS)
    add("Bob", ["Bob"])
    add("bob", ["Bob"])
    add("Bobb", ["Bob"])
    add("Bo", ["Bob"])
    add("xyz", ["Bob"])
    add("Dice", ["Dice", "Dice"])        # doublons exacts
    add("dice", ["Dice", "DICE"])        # ex aequo à 100
    add("Alic", ["Alice", "Alicia"])     # ex aequo à 80
    add("Alicx", ["Alicia", "Alice"])    # ex aequo, l'ordre compte
    add("", ["Bob"])
    add("", ["Bob", "Al"])
    add("a", ["a", "A"])
    add("ab", ["ba", "ab "])
    add("ca", ["abc"])                   # transposition non restreinte (OSA donnerait 3)
    add("abc", ["ca"])
    add("İstanbul", ["istanbul", "Istanbul"])  # lower() qui change la longueur
    add("日本語", ["日本", "日本語テキスト"])
    add("naïve", ["naive", "naïve!"])
    add("😀smile", ["😀smile", "smile"])
    # Noms réels (ChecksFinder, Yacht Dice) et fautes de frappe.
    for pool, mots in ((CHECKS, ["map widht", "Map Hieght", "map bomb", "nothin", "tile 1", "Tile 3", "tile 100", "Mapp Width", "bombs"]),
                       (YACHT, ["dice", "dice fragmnt", "roll fragment", "Fixed Score Multiplier", "fixed score multiplier", "step score multipler",
                                "category yacth", "Category Full Hosue", "Category One", "1 point", "10 point", "100 points", "Category Large Straigth"]),
                       (GAMES, ["ocarina of time", "Ocarina of Tim", "yacht dice", "Yatch Dice", "checksfinder", "Checks Finder", "factoro", "hollow knigt"])):
        for m in mots:
            add(m, pool)
    # Scores à la limite : 75 %, 90 %, écart de 5 points, etc.
    add("abcd", ["abcx", "abxx"])        # 75 % puis 50 %
    add("abcde", ["abcdx", "abxyz"])     # 80 %
    add("abcdefgh", ["abcdefxx", "abcdexxx"])  # 75 % / 62 %
    add("abcdefgh", ["abcdefgx", "abcdefxx"])  # 87 % / 75 %
    add("abcdefghij", ["abcdefghix", "abcdefgxxx"])  # 90 % / 70 %
    add("abcdefghij", ["abcdefghix"])    # 90 % : pas > 90
    add("abcdefghijk", ["abcdefghijx"])  # 90 % (int(0.909..*100))
    add("abcdefghijklmnopqrst", ["abcdefghijklmnopqrsx"])  # 95 %
    add("abcdefghijklmnopqrstu", ["abcdefghijklmnopqrstx"])  # 95 %
    add("abcdefghijkl", ["abcdefghijkx", "abcdefghikxx"])  # 91 % / 83 %
    add("abcdefghijklmnopqrst", ["abcdefghijklmnopqrxx", "abcdefghijklmnopqxxx"])  # 90 % / 85 % : écart de 5
    add("abcdefghijklmnopqrst", ["abcdefghijklmnopqrxx", "abcdefghijklmnopxxxx"])  # 90 % / 80 %
    add("abcdefghijklmnopqrst", ["abcdefghijklmnopqrst0"])  # 95 %
    add("abcdefghijklmnopqrst", ["abcdefghijklmnopqrxx", "abcdefghijklmnopqrxy"])  # ex aequo
    # Cas aléatoires, graine fixe.
    base_pools = [CHECKS, YACHT, GAMES, PLAYERS]
    while len(cases) < 170:
        pool = rng.choice(base_pools)
        k = rng.randint(1, min(len(pool), 6))
        cands = rng.sample(pool, k)
        add(mutate(rng, rng.choice(pool)), cands)

    with open(sys.argv[1], "w", encoding="utf-8", newline="\n") as f:
        json.dump(cases, f, ensure_ascii=False, indent=1)
        f.write("\n")
    print(len(cases), "cas")


main()
