"""Génère shlex-vectors.json : cas de test du découpage des commandes.

Usage : python -I shlex_vectors.py <sortie.json>

Reproduit CommandProcessor.__call__ (MultiServer.py) : shlex.split(raw,
comments=False), repli sur raw.split() en cas de ValueError ; plus
raw.split(maxsplit=1) pour l'argument brut.
"""
import json
import shlex
import sys

RAWS = [
    "/help", "/help   ", "   /help", "/countdown 10", "/countdown   10   ", "/send Alice Dice",
    "/send  Alice   'Dice Fragment'", '/send "Alice" "Dice Fragment"', "/send Alice 'Dice Fragment' 3",
    "/send 'Alice Dice", '/send "Alice Dice', "/send Alice\\", "/send Alice\\ Bob", "/send a\\\\b",
    '/send "a\\"b"', '/send "a\\\\b"', '/send "a\\nb"', "/send 'a\\nb'", "/send 'a\\'b'", "/send 'a'\"b\"c",
    "/send '' x", '/send "" x', "/send ''", "''", '""', "/send a''b", "/send a'  'b",
    "/forfeit Alice", "/forfeit 'Émilie Zoë'", "/forfeit Émilie", "/forfeit \"Émilie\"", "/forfeit Zoë\\ Bob",
    "/send 日本語 'テキスト 二'", "/send 😀 \"😀 😀\"", "/send_multiple 3 Alice 'Dice Fragment'",
    "hello world", "hello   world  ", "  hello", "hello 'world", "hello \"world", "hello\\",
    "a\tb", "a\nb", "a\r\nb", "a b", "a　b", "a b", "a\x1cb", "a\x0bb", "a\x0cb", "a\x85b",
    " a b", "  \t ", "", " ", "\\", "'", '"', "\\ ", "a\\ ", "# not a comment", "a#b", "a #b c",
    "/cmd 'it''s'", "/cmd \"it\"\"s\"", "/cmd it's", "/cmd it\"s", "/cmd \"a'b\"", "/cmd 'a\"b'",
    "/cmd \"a\\'b\"", "/cmd \"a\\$b\"", "/cmd \"\\\\\\\"\"", "/cmd a\\\"b", "/cmd a\\'b", "/cmd \\\\",
    "/cmd 'a\\\\b'", "/cmd \"unclosed\\\"", "/cmd 'x' 'y", "/cmd \"x\" \"y", "/cmd a\\\nb", "/cmd \"a\\\nb\"",
    "/cmd a  b c  d ", "/cmd (a) [b] {c} ;d |e &f <g >h", "/cmd a=b --flag=\"x y\"", "/cmd ​a", "/cmd é\\é",
    "/cmd \"é\\é\"", "ab\\", "\"ab\\", "'ab\\", "x \\\n", " x y",
]


def py_split(raw):
    try:
        return shlex.split(raw, comments=False)
    except ValueError:
        return raw.split()


def raw_arg(raw):
    arg = raw.split(maxsplit=1)
    return arg[1] if len(arg) > 1 else None


def main():
    cases = [{"raw": r, "split": py_split(r), "arg": raw_arg(r)} for r in RAWS]
    with open(sys.argv[1], "w", encoding="utf-8", newline="\n") as f:
        json.dump(cases, f, ensure_ascii=False, indent=1)
        f.write("\n")
    print(len(cases), "cas")


main()
