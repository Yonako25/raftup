"""Faux Main.main(erargs, seed) : ecrit AP_<seed>.zip et rend un objet type MultiWorld."""
import logging
import os
import zipfile


class FillError(Exception):
    pass


class FauxMultiWorld:
    def __init__(self, seed_name, joueurs):
        self.seed_name = seed_name
        self.player_ids = list(range(1, len(joueurs) + 1))
        self.player_name = {i: j.get("name", "?") for i, j in zip(self.player_ids, joueurs)}
        self.game = {i: j.get("game", "?") for i, j in zip(self.player_ids, joueurs)}
        # Rien a placer dans le faux monde (finish_worlds avec un placement vide).
        self.itempool = []

    def get_unfilled_locations(self):
        return []


def distribute_items_restrictive(multiworld, *args, **kwargs):
    """Faux placement : le worker le remplace pour export_worlds (ADR 0028)."""


def main(args, seed):
    noms = [j.get("name") for j in args.players]
    logging.info("faux generateur : seed %s", seed)
    if "FillError" in noms:
        raise FillError("impossible de placer les objets")
    if "BadImport" in noms:
        import module_qui_nexiste_pas  # noqa: F401
    seed_name = str(seed)
    if "NoFill" not in noms:
        distribute_items_restrictive(FauxMultiWorld(seed_name, args.players))
    if "Escape" in noms:
        # Un nom qui remonte hors de output_dir ("AP_x/../../escape.zip").
        os.makedirs(os.path.join(args.outputpath, "AP_x"), exist_ok=True)
        seed_name = "x/../../escape"
    os.makedirs(args.outputpath, exist_ok=True)
    with zipfile.ZipFile(os.path.join(args.outputpath, "AP_%s.zip" % seed_name), "w") as z:
        z.writestr("fake.archipelago", "fake")
    return FauxMultiWorld(seed_name, args.players)
