"""Faux Generate.main() : lit les YAML du dossier, rend (erargs, seed).

Le nom du joueur pilote le comportement :
  Exit -> sys.exit(1) ; Input -> input() ; Print -> print() ; BadImport -> ImportError (dans Main).
"""
import argparse
import glob
import os
import sys

import ModuleUpdate  # noqa: F401  (comme le vrai : verifie qu'il est trouvable)


def lire_yaml(chemin):
    infos = {}
    with open(chemin, encoding="utf-8") as f:
        for ligne in f:
            if ":" in ligne:
                cle, valeur = ligne.split(":", 1)
                infos[cle.strip()] = valeur.strip()
    return infos


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--player_files_path")
    p.add_argument("--outputpath")
    p.add_argument("--seed", type=int)
    p.add_argument("--spoiler", type=int, default=3)
    p.add_argument("--race", action="store_true")
    args = p.parse_args(sys.argv[1:])
    args.players = [lire_yaml(c) for c in sorted(glob.glob(os.path.join(args.player_files_path, "*.yaml")))]
    for joueur in args.players:
        nom = joueur.get("name", "")
        if nom == "Exit":
            sys.exit(1)
        if nom == "Input":
            input("Appuyez sur Entree : ")
        if nom == "Print":
            print("ligne imprimee par le generateur")
    seed = args.seed if args.seed is not None else 12345
    return args, seed
