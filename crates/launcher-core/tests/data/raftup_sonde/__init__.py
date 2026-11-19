"""Monde de test de Raftup (ADR 0028, etape 2) : des regles en lambdas Python variees,
pour essayer le sondage sur le vrai Archipelago. Ecrit pour Raftup (MIT / Apache-2.0),
aucun code tiers. Jamais distribue aux joueurs : il ne sert qu'aux tests.

Menu --(Sword)--> Forest --(Lamp ou 2 Gem)--> Cave
Menu --(Sword et 2 Key)--> Castle (Victory)

Options `early_items` et `item_rules` : objets a placer tot et regles d'objet (ADR 0029).
"""
from dataclasses import dataclass

from BaseClasses import Item, ItemClassification, Location, LocationProgressType, Region
from Options import PerGameCommonOptions, Toggle
from worlds.AutoWorld import World

GAME = "Raftup Sonde"
ITEMS = {"Sword": 1, "Key": 2, "Lamp": 3, "Gem": 4, "Coin": 5}
PROGRESSION = {"Sword", "Key", "Lamp", "Gem"}
LOCATIONS = {
    "Start Chest": 1, "Forest Stump": 2, "Forest Tree": 3, "Cave Pool": 4,
    "Cave Crystal": 5, "Castle Gate": 6, "Castle Throne": 7, "Castle Vault": 8,
    "Bad Spot": 9,
}


class BadRule(Toggle):
    """Ajoute un lieu a regle non monotone : le sondage doit la refuser."""
    display_name = "Bad Rule"


class EarlyItems(Toggle):
    """Objets a placer tot (ADR 0029) : Coin n'importe ou, Sword chez le joueur."""
    display_name = "Early Items"


class ItemRules(Toggle):
    """Regles d'objet (ADR 0029) : pas de Coin dans Cave Pool ; Forest Stump (exclu)
    accepte toujours une Gem du joueur."""
    display_name = "Item Rules"


@dataclass
class SondeOptions(PerGameCommonOptions):
    bad_rule: BadRule
    early_items: EarlyItems
    item_rules: ItemRules


class SondeItem(Item):
    game = GAME


class SondeLocation(Location):
    game = GAME


def has_light(state, player):
    """Petite fonction d'aide, comme dans beaucoup de mondes."""
    return state.has("Lamp", player) or state.has("Gem", player, 2)


class SondeWorld(World):
    """Monde de test du sondage des regles (Raftup)."""
    game = GAME
    options_dataclass = SondeOptions
    options: SondeOptions
    item_name_to_id = ITEMS
    location_name_to_id = LOCATIONS
    item_name_groups = {"Shiny": {"Gem", "Lamp"}}

    def generate_early(self):
        if self.options.early_items:
            # Une seule place en sphere 0 (Start Chest, prioritaire) : Sword (local) la
            # prend ; Coin (pas de progression) n'a pas droit aux lieux prioritaires.
            self.multiworld.early_items[self.player]["Coin"] = 1
            self.multiworld.local_early_items[self.player]["Sword"] = 1

    def create_item(self, name):
        kind = ItemClassification.progression if name in PROGRESSION else ItemClassification.filler
        return SondeItem(name, kind, ITEMS[name], self.player)

    def create_regions(self):
        p, mw = self.player, self.multiworld
        regions = {}
        for name in ("Menu", "Forest", "Cave", "Castle"):
            regions[name] = Region(name, p, mw)
            mw.regions.append(regions[name])

        def add(region, name):
            regions[region].locations.append(SondeLocation(p, name, LOCATIONS[name], regions[region]))

        for region, name in (("Menu", "Start Chest"), ("Forest", "Forest Stump"),
                             ("Forest", "Forest Tree"), ("Cave", "Cave Pool"),
                             ("Cave", "Cave Crystal"), ("Castle", "Castle Gate"),
                             ("Castle", "Castle Throne"), ("Castle", "Castle Vault")):
            add(region, name)
        if self.options.bad_rule:
            add("Menu", "Bad Spot")
        regions["Menu"].connect(regions["Forest"], "Menu -> Forest", lambda s: s.has("Sword", p))
        regions["Forest"].connect(regions["Cave"], "Forest -> Cave", lambda s: has_light(s, p))
        regions["Menu"].connect(regions["Castle"], "Menu -> Castle",
                                lambda s: s.has_all(["Key", "Sword"], p) and s.count("Key", p) >= 2)
        victory = SondeLocation(p, "Victory", None, regions["Castle"])
        victory.place_locked_item(SondeItem("Victory", ItemClassification.progression, None, p))
        regions["Castle"].locations.append(victory)

    def create_items(self):
        names = ["Sword", "Key", "Key", "Lamp", "Gem", "Gem", "Gem"]
        total = len(LOCATIONS) - (0 if self.options.bad_rule else 1)
        names += ["Coin"] * (total - len(names))
        self.multiworld.itempool += [self.create_item(n) for n in names]

    def set_rules(self):
        p, mw = self.player, self.multiworld
        mw.get_location("Forest Tree", p).access_rule = lambda s: s.has_any(["Lamp", "Key"], p)
        mw.get_location("Cave Crystal", p).access_rule = lambda s: s.has_group("Shiny", p, 3)
        mw.get_location("Castle Throne", p).access_rule = \
            lambda s: s.can_reach("Cave", "Region", p) or s.has_from_list(["Gem"], p, 3)
        mw.get_location("Castle Vault", p).access_rule = \
            lambda s: s.count("Gem", p) > 1 and (s.has("Lamp", p) or s.can_reach_location("Cave Pool", p))
        if self.options.bad_rule:
            # Non monotone pour le sondeur, mais toujours vraie pour Archipelago : la piece
            # n'est pas de progression, elle n'entre jamais dans l'inventaire logique.
            mw.get_location("Bad Spot", p).access_rule = lambda s: not s.has("Coin", p)
        mw.get_location("Victory", p).access_rule = lambda s: s.has("Key", p, 2)
        # Un lieu prioritaire et un lieu exclu (LocationProgressType).
        mw.get_location("Start Chest", p).progress_type = LocationProgressType.PRIORITY
        mw.get_location("Forest Stump", p).progress_type = LocationProgressType.EXCLUDED
        mw.completion_condition[p] = lambda s: s.has("Victory", p)
        if self.options.item_rules:
            pool = mw.get_location("Cave Pool", p)
            # (Interdire Key ici rendait le monde a un joueur impossible a remplir.)
            pool.item_rule = lambda item: not (item.name == "Coin" and item.player == p)
            mw.get_location("Forest Stump", p).always_allow = \
                lambda state, item: item.name == "Gem" and item.player == p

    def fill_slot_data(self):
        return {"bad_rule": bool(self.options.bad_rule)}
