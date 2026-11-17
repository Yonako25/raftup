"""Générateur de scénarios tirés au hasard pour les tests différentiels (ADR 0008).

Même format que `scenarios.py` : une liste d'étapes (client, paquets).
Tout dépend d'un `random.Random` : la même graine donne le même scénario.

Le générateur évite ce qui ne doit pas être comparé :
- les écarts connus et acceptés avec le serveur officiel (voir
  `docs/etat-du-projet.md`) : entiers plus grands qu'un i64, `mul` géant,
  changement de slot sur une même connexion, connexion à un groupe, lieu
  inconnu dans `LocationScouts` (le serveur officiel plante) ;
- les écarts connus des commandes `!` et des indices (ADR 0012, 0013) :
  `!options` (mot de passe), `!license`, `!admin` (commandes serveur pas
  faites), `!countdown` (messages en différé), `!hint <objet>` (tirage entre
  ex æquo), noms approchés (l'ordre des suggestions est au hasard en Python),
  arguments qui font planter le serveur officiel ;
- les limites propres à Raftup (ADR 0007) : plus de 5 messages de chat par
  connexion, messages de plus de 2 000 caractères.

Les commandes `!` comptent comme des messages de chat (même limite de 5).
"""

from scenarios import CF, V, YD, bounce, checks, connect, setp, status, update

CLIENTS = ["a", "b", "c", "d", "e"]

# Slot -> (jeu, lieux connus du slot).
SLOTS = {
    "Alice": ("ChecksFinder", CF),
    "Bob": ("ChecksFinder", CF),
    "Chloé": ("Yacht Dice", YD),
}
OLD_VERSION = {"major": 0, "minor": 4, "build": 9, "class": "Version"}
TEXTS = ["Bonjour", "ça va ?", "ligne\nsuite", "🎉", "", "  espaces  ", "a" * 300, "x;y=z"]
MAX_SAY = 5
# Numéro de slot de chaque nom (seed MIT).
SLOT_NUMBERS = {"Alice": 1, "Bob": 2, "Chloé": 3}
# Noms exacts seulement : un nom approché fait proposer des suggestions dont
# l'ordre est tiré au hasard par le serveur officiel.
CF_ITEMS = ["Map Width", "Map Height", "Map Bombs"]
CF_LOCATION_NAMES = [f"Tile {n}" for n in range(1, 26)]
ALIASES = ["Lulu", "Zed", "Mi ka"]
STATUS_TAGS = ["DeathLink", "Tracker", "AP", "Inconnu"]
HINT_STATUSES = [0, 10, 20, 30]


class Generator:
    """Garde ce que chaque client a déjà fait, pour rester dans les cas comparables."""

    def __init__(self, rng, number):
        self.rng = rng
        # Clés de stockage propres au scénario : les valeurs ne grossissent pas
        # d'un scénario à l'autre (les serveurs gardent leur état).
        self.prefix = f"s{number}_"
        self.slot = {}       # client -> slot visé par son dernier Connect valide
        self.says = {c: 0 for c in CLIENTS}
        self.closed = set()  # clients qui ont envoyé de quoi fermer leur connexion

    # ----- Choix de base -----

    def pick(self, items):
        return self.rng.choice(items)

    def sample(self, items, low=1, high=4):
        count = self.rng.randint(low, min(high, len(items)))
        return self.rng.sample(items, count)

    def tags(self):
        tags = [t for t in ("AP", "DeathLink", "Tracker", "TextOnly") if self.rng.random() < 0.2]
        return tags

    # ----- Paquets -----

    def connect(self, client):
        # Une connexion déjà acceptée ne change jamais de slot (le serveur
        # officiel plante) : on reprend le même nom.
        if client in self.slot:
            name = self.slot[client]
        else:
            name = self.pick(list(SLOTS) + ["Personne"])
        game = SLOTS.get(name, ("ChecksFinder", []))[0]
        roll = self.rng.random()
        version = V
        if roll < 0.1:
            game = self.pick(["Yacht Dice", "ChecksFinder", "Inconnu"])
        elif roll < 0.2:
            version = OLD_VERSION
        handling = self.pick([7, 7, 7, 0, 1, 3, 5, 6])
        tags = self.tags()
        no_game = "TextOnly" in tags or "Tracker" in tags
        if no_game:
            game = self.pick([game, ""])
        # Mêmes règles que le serveur officiel : un outil sans jeu (`game` vide
        # et tag TextOnly ou Tracker) est accepté avec n'importe quel slot.
        ignore_game = no_game and game == ""
        valid = (name in SLOTS and version == V and handling != 6
                 and (ignore_game or game == SLOTS[name][0]))
        packet = connect(name, game, handling=handling, tags=tags, version=version,
                         slot_data=self.pick([None, True, False]))
        if valid and client not in self.slot:
            self.slot[client] = name
        return packet

    def locations(self, client, unknown=True):
        name = self.slot.get(client, self.pick(list(SLOTS)))
        known = SLOTS[name][1]
        chosen = self.sample(known, 1, 6)
        if unknown and self.rng.random() < 0.15:
            chosen.append(self.pick([99999, CF[0], YD[0]]))
        return chosen

    def scouts(self, client):
        # Seulement les lieux du slot du client : un lieu inconnu fait planter
        # le serveur officiel (écart connu).
        if client not in self.slot:
            return [{"cmd": "LocationScouts", "locations": []}]
        known = SLOTS[self.slot[client]][1]
        return [{"cmd": "LocationScouts", "locations": self.sample(known, 1, 4), "create_as_hint": 0}]

    def say(self, client):
        self.says[client] += 1
        return [{"cmd": "Say", "text": self.pick(TEXTS)}]

    def command(self, client):
        """Une commande `!` du chat, parmi celles qui se comparent (voir en tête)."""
        self.says[client] += 1
        return [{"cmd": "Say", "text": self.command_text(client)}]

    def command_text(self, client):
        name = self.slot.get(client)
        # Noms d'objets et de lieux : seulement pour ChecksFinder, dont on
        # connaît les noms exacts ; pour Yacht Dice, des numéros.
        on_cf = name in ("Alice", "Bob")
        variant = self.rng.randint(0, 14)
        if variant == 0:
            return "!help"
        if variant == 1:
            return "!players"
        if variant == 2:
            return self.pick(["!status", "!status " + self.pick(STATUS_TAGS)])
        if variant == 3:
            return "!remaining"
        if variant == 4:
            return self.pick(["!missing", "!missing 3", "!missing Tile 1", "!missing zzz"])
        if variant == 5:
            return self.pick(["!checked", "!checked 2", "!checked Tile"])
        if variant in (6, 7, 8):
            # Un lieu donne un seul indice : pas de tirage entre ex æquo.
            if on_cf and self.rng.random() < 0.7:
                return "!hint_location " + self.pick(CF_LOCATION_NAMES)
            known = SLOTS.get(name, ("", CF))[1]
            return f"!hint_location {self.pick(known)}"
        if variant == 9:
            return self.pick(["!alias " + self.pick(ALIASES), "!alias"])
        if variant == 10:
            # Sans argument, le serveur officiel plante (écart connu).
            if on_cf:
                return "!getitem " + self.pick(CF_ITEMS)
            return "!players"
        if variant == 11:
            # Rares : changent beaucoup l'état de la partie.
            if self.rng.random() < 0.3:
                return self.pick(["!release", "!collect"])
            return "!remaining"
        if variant == 12:
            return self.pick(["!Inconnue", "!", "\"!players\""])
        if variant == 13:
            # Ordre du `set` Python des indices, reproduit par Raftup (ADR 0018).
            return "!hint"
        return "!hint_location 99999"

    # ----- Indices (ADR 0012) -----

    def other_slot(self):
        """Un slot au hasard et un de ses lieux connus."""
        name = self.pick(list(SLOTS))
        return SLOT_NUMBERS[name], self.pick(SLOTS[name][1])

    def hint_scouts(self, client):
        if client not in self.slot:
            return [{"cmd": "LocationScouts", "locations": [], "create_as_hint": 1}]
        known = SLOTS[self.slot[client]][1]
        return [{"cmd": "LocationScouts", "locations": self.sample(known, 1, 3),
                 "create_as_hint": self.pick([1, 2])}]

    def create_hints(self, client):
        packet = {"cmd": "CreateHints"}
        if self.rng.random() < 0.5 or client not in self.slot:
            # Lieux de son propre slot (champ `player` absent).
            known = SLOTS.get(self.slot.get(client), ("", CF))[1]
            packet["locations"] = self.sample(known, 1, 3)
        else:
            player, location = self.other_slot()
            packet["player"] = player
            packet["locations"] = [location]
        if self.rng.random() < 0.6:
            packet["status"] = self.pick(HINT_STATUSES)
        return [packet]

    def update_hint(self):
        player, location = self.other_slot()
        return [{"cmd": "UpdateHint", "player": player, "location": location,
                 "status": self.pick(HINT_STATUSES + [40, 7])}]

    def bounce(self):
        data = {"n": self.rng.randint(0, 99)}
        variant = self.rng.randint(0, 6)
        if variant == 0:
            return bounce(tags=["DeathLink"], data={"time": 1700000000.5, "source": "x", "cause": "y"})
        if variant == 1:
            return bounce(slots=self.sample([1, 2, 3, 4], 1, 2), data=data)
        if variant == 2:
            return bounce(games=self.sample(["ChecksFinder", "Yacht Dice", "Inconnu"], 1, 2), data=data)
        if variant == 3:
            return bounce(tags=["DeathLink"], slots=[self.rng.randint(1, 3)],
                          operator=self.pick(["and", "or"]), data=data)
        if variant == 4:
            return bounce(teams=[0], data=data)
        if variant == 5:
            return bounce(data=data)
        return bounce(slots=[1])

    def key(self):
        base = self.pick(["n", "n", "l", "t", "d"])
        return self.prefix + base, base

    def set(self):
        key, kind = self.key()
        # Petites valeurs seulement : jamais plus grand qu'un i64, jamais de
        # `mul` sur un texte ou une liste (écarts connus).
        if kind == "n":
            ops = [(self.pick(["add", "max", "min", "mod", "xor", "and", "or", "replace", "floor", "ceil"]),
                    self.pick([0, 1, 2, 3, -1, 5, 0.5]))
                   for _ in range(self.rng.randint(1, 3))]
            ops = [(o, 1 if o == "mod" and v in (0, 0.5) else v) for o, v in ops]
            if self.rng.random() < 0.2:
                ops.append(("mul", self.pick([-1, 0, 1, 2])))
            default = self.pick([0, 7, -3, 1.5])
        elif kind == "l":
            ops = [(self.pick(["update", "update", "remove", "pop", "add"]), self.pick([[1], [2, 3], 1, 0]))
                   for _ in range(self.rng.randint(1, 2))]
            ops = [(o, v if o != "pop" else 0) for o, v in ops]
            default = []
        elif kind == "t":
            ops = [("add", self.pick(["a", "é", ""])), ("replace", "z")][: self.rng.randint(1, 2)]
            default = "x"
        else:
            ops = [(self.pick(["update", "or"]), {self.pick(["x", "y"]): self.rng.randint(0, 3)})]
            default = {}
        fields = {"default": default}
        if self.rng.random() < 0.5:
            fields["want_reply"] = True
        return setp(key, ops, **fields)

    def get(self, client):
        keys = [self.key()[0] for _ in range(self.rng.randint(1, 3))]
        keys += self.sample(["_read_race_mode", "_read_client_status_0_1", "_read_client_status_0_3",
                             "_read_hints_0_1", "_read_slot_data_2", "_read_inconnu"], 0, 2)
        return [{"cmd": "Get", "keys": keys}]

    def malformed(self, client):
        variant = self.rng.randint(0, 3)
        if variant == 0:
            return [{"cmd": "CommandeInconnue"}]
        if variant == 1:
            return [{"cmd": "LocationChecks", "locations": "pas une liste"}]
        if variant == 2:
            return [{"cmd": 5}]
        # Paquet sans « cmd » : le serveur ferme la connexion.
        self.closed.add(client)
        return [{"pas": "de cmd"}]

    def step(self, client):
        choices = [
            (8, lambda: self.connect(client)),
            (8, lambda: checks(self.locations(client))),
            (3, lambda: self.scouts(client)),
            (3, lambda: status(self.pick([0, 5, 10, 20, 30]))),
            (3 if self.says[client] < MAX_SAY else 0, lambda: self.say(client)),
            (4 if self.says[client] < MAX_SAY else 0, lambda: self.command(client)),
            (2, lambda: self.hint_scouts(client)),
            (2, lambda: self.create_hints(client)),
            (2, self.update_hint),
            (3, self.bounce),
            (5, self.set),
            (3, lambda: self.get(client)),
            (2, lambda: [{"cmd": "SetNotify", "keys": [self.key()[0]]}]),
            (2, lambda: [{"cmd": "Sync"}]),
            (2, lambda: update(items_handling=self.pick([0, 1, 3, 7, 6]))),
            (2, lambda: update(tags=self.tags())),
            (1, lambda: [{"cmd": "GetDataPackage", "games": self.sample(["ChecksFinder", "Yacht Dice", "Archipelago"])}]),
            (1, lambda: self.malformed(client)),
        ]
        weights = [w for w, _ in choices]
        _, make = self.rng.choices(choices, weights=weights)[0]
        return make()


def scenario(rng, number, length):
    """Un scénario de `length` étapes au plus, pour le numéro `number`."""
    gen = Generator(rng, number)
    count = rng.randint(2, len(CLIENTS))
    clients = CLIENTS[:count]
    steps = []
    # Au début, la plupart des clients se connectent.
    for client in clients:
        if rng.random() < 0.8:
            steps.append((client, gen.connect(client)))
    while len(steps) < length:
        alive = [c for c in clients if c not in gen.closed]
        if not alive:
            break
        client = rng.choice(alive)
        steps.append((client, gen.step(client)))
    return steps
