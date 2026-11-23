"""Tests du worker Python (unittest, bibliotheque standard seule).

Lancer : python -I crates/py-bridge/worker/test_worker.py
"""
import json
import os
import subprocess
import sys
import tempfile
import unittest

ICI = os.path.dirname(os.path.abspath(__file__))
with open(os.path.join(ICI, "raftup_worker.py"), encoding="utf-8") as f:
    SCRIPT = f.read()


def lancer(script=SCRIPT):
    """Demarre le worker exactement comme le fait le pont Rust."""
    return subprocess.Popen(
        [sys.executable, "-I", "-X", "utf8", "-c", script],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def echanger(octets, script=SCRIPT):
    """Envoie des octets, ferme stdin, rend (lignes JSON de stdout, code)."""
    p = lancer(script)
    out = p.communicate(octets, timeout=30)[0]
    lignes = [json.loads(x) for x in out.decode("utf-8").splitlines()]
    return lignes, p.returncode


def req(id_, method, params=None):
    msg = {"id": id_, "method": method, "params": params or {}}
    return (json.dumps(msg) + "\n").encode("utf-8")


class TestWorker(unittest.TestCase):
    def test_taille_script(self):
        self.assertLess(len(SCRIPT.encode("utf-8")), 16 * 1024)

    def test_ping_version_echo(self):
        sortie, code = echanger(
            req(1, "ping")
            + req(2, "version")
            + req(3, "echo", {"texte": "été ✓", "n": [1, 2]})
        )
        self.assertEqual(code, 0)
        self.assertEqual(sortie[0], {"id": 1, "result": "pong"})
        v = sortie[1]["result"]
        self.assertEqual(v["protocol"], 1)
        self.assertEqual(v["implementation"], "CPython")
        self.assertEqual(v["python"], "%d.%d.%d" % sys.version_info[:3])
        self.assertEqual(sortie[2]["result"], {"texte": "été ✓", "n": [1, 2]})

    def test_sortie_ascii(self):
        p = lancer()
        out = p.communicate(req(1, "echo", {"t": "é"}), timeout=30)[0]
        self.assertTrue(out.isascii())

    def test_stdin_des_sous_process_est_devnull(self):
        # Le handler injecte lance un sous-process qui lit son stdin (herite du fd 0) :
        # il doit recevoir EOF tout de suite, sans voler la requete suivante du pont.
        cible = 'return "pong"'
        self.assertIn(cible, SCRIPT)
        enfant = "import sys; print(len(sys.stdin.buffer.read()))"
        script = SCRIPT.replace(
            cible,
            "import subprocess; return subprocess.run("
            "[sys.executable, '-I', '-c', %r], capture_output=True, timeout=20)"
            ".stdout.decode().strip()" % enfant,
        )
        sortie, code = echanger(req(1, "ping") + req(2, "ping"), script)
        self.assertEqual(code, 0)
        self.assertEqual(
            sortie, [{"id": 1, "result": "0"}, {"id": 2, "result": "0"}]
        )

    def test_json_trop_imbrique(self):
        profond = b"[" * 100000 + b"]" * 100000 + b"\n"
        sortie, code = echanger(profond + req(2, "ping"))
        self.assertEqual(sortie[0]["id"], None)
        self.assertEqual(sortie[0]["error"]["code"], -32700)
        self.assertEqual(sortie[1], {"id": 2, "result": "pong"})
        self.assertEqual(code, 0)

    def test_nan_donne_erreur_interne(self):
        # json.loads accepte NaN en entree ; echo le renverrait tel quel, ce qui
        # n'est pas du JSON valide : le worker doit repondre -32603 et rester en vie.
        nan = b'{"id":1,"method":"echo","params":{"x":NaN}}\n'
        sortie, code = echanger(nan + req(2, "ping"))
        self.assertEqual(sortie[0]["id"], 1)
        self.assertEqual(sortie[0]["error"]["code"], -32603)
        self.assertEqual(sortie[1], {"id": 2, "result": "pong"})
        self.assertEqual(code, 0)

    def test_bruit_ne_pollue_pas(self):
        # On injecte des print() et une ecriture directe sur le fd 1 dans
        # la methode ping ; le protocole ne doit pas les voir.
        cible = 'return "pong"'
        self.assertIn(cible, SCRIPT)
        script = SCRIPT.replace(
            cible,
            'print("bruit"); os.write(1, b"bruit fd\\n"); ' + cible,
        )
        sortie, code = echanger(req(1, "ping") + req(2, "ping"), script)
        self.assertEqual(code, 0)
        self.assertEqual(
            sortie, [{"id": 1, "result": "pong"}, {"id": 2, "result": "pong"}]
        )

    def test_methode_inconnue(self):
        sortie, _ = echanger(req(7, "nope"))
        self.assertEqual(sortie[0]["id"], 7)
        self.assertEqual(sortie[0]["error"]["code"], -32601)

    def test_json_illisible(self):
        sortie, code = echanger(b"{pas du json\n" + req(2, "ping"))
        self.assertEqual(sortie[0]["id"], None)
        self.assertEqual(sortie[0]["error"]["code"], -32700)
        # le flux reste synchronise apres une ligne invalide
        self.assertEqual(sortie[1], {"id": 2, "result": "pong"})
        self.assertEqual(code, 0)

    def test_requete_mal_formee(self):
        sortie, _ = echanger(b"[1,2]\n" + b'{"id":5,"method":3}\n')
        self.assertEqual(sortie[0]["error"]["code"], -32600)
        self.assertEqual(sortie[0]["id"], None)
        self.assertEqual(sortie[1]["error"]["code"], -32600)
        self.assertEqual(sortie[1]["id"], 5)

    def test_ligne_trop_longue(self):
        enorme = (
            b'{"id":1,"method":"echo","params":{"x":"'
            + b"a" * (1024 * 1024)
            + b'"}}\n'
        )
        sortie, code = echanger(enorme + req(2, "ping"))
        self.assertEqual(sortie[0]["error"]["code"], -32600)
        self.assertEqual(sortie[0]["id"], None)
        self.assertEqual(len(sortie), 1)  # arret, pas de reponse au ping
        self.assertEqual(code, 2)  # code d'erreur : le flux est desynchronise

    def test_eof_code_zero(self):
        sortie, code = echanger(b"")
        self.assertEqual(sortie, [])
        self.assertEqual(code, 0)


FAUX_AP = os.path.join(ICI, "..", "tests", "fixtures", "fake_archipelago")


class TestGenerate(unittest.TestCase):
    """`generate` avec le faux Archipelago (tests/fixtures/fake_archipelago)."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.joueurs = os.path.join(self.tmp.name, "joueurs")
        self.sortie = os.path.join(self.tmp.name, "out")
        os.makedirs(self.joueurs)

    def yaml(self, nom, jeu="FakeGame", fichier="p1.yaml"):
        with open(os.path.join(self.joueurs, fichier), "w", encoding="utf-8") as f:
            f.write("name: %s\ngame: %s\n" % (nom, jeu))

    def params(self, **extra):
        p = {
            "archipelago_dir": os.path.abspath(FAUX_AP),
            "site_packages": None,
            "player_files_dir": self.joueurs,
            "output_dir": self.sortie,
            "seed": 42,
            "spoiler": 3,
            "race": False,
        }
        p.update(extra)
        return p

    def gen(self, id_=1, **extra):
        return req(id_, "generate", self.params(**extra))

    def test_normal(self):
        self.yaml("Alice")
        sortie, code = echanger(self.gen() + req(2, "ping"))
        self.assertEqual(code, 0)
        r = sortie[0]["result"]
        self.assertEqual(r["seed_name"], "42")
        self.assertEqual(r["ap_version"], "0.0.0-fake")
        self.assertEqual(r["players"], [{"slot": 1, "name": "Alice", "game": "FakeGame"}])
        self.assertTrue(os.path.isfile(r["zip"]))
        self.assertEqual(os.path.dirname(r["zip"]), os.path.realpath(self.sortie))
        self.assertIsInstance(r["seconds"], float)
        self.assertEqual(sortie[1], {"id": 2, "result": "pong"})  # worker vivant

    def test_seed_absente(self):
        self.yaml("Alice")
        sortie, _ = echanger(self.gen(seed=None))
        self.assertEqual(sortie[0]["result"]["seed_name"], "12345")

    def test_fill_error(self):
        self.yaml("FillError")
        sortie, code = echanger(self.gen() + req(2, "ping"))
        e = sortie[0]["error"]
        self.assertEqual(e["code"], -32010)
        self.assertEqual(e["data"]["type"], "FillError")
        self.assertIn("FillError", e["data"]["traceback"])
        self.assertLessEqual(len(e["data"]["traceback"].encode("utf-8")), 64 * 1024)
        self.assertEqual(sortie[1], {"id": 2, "result": "pong"})
        self.assertEqual(code, 0)

    def test_sys_exit(self):
        self.yaml("Exit")
        sortie, code = echanger(self.gen() + req(2, "ping"))
        self.assertEqual(sortie[0]["error"]["code"], -32010)
        self.assertEqual(sortie[0]["error"]["data"]["type"], "SystemExit")
        self.assertEqual(sortie[1], {"id": 2, "result": "pong"})
        self.assertEqual(code, 0)

    def test_input_recoit_eof(self):
        self.yaml("Input")
        sortie, code = echanger(self.gen() + req(2, "ping"))  # communicate() a un delai : pas de blocage
        self.assertEqual(sortie[0]["error"]["code"], -32010)
        self.assertEqual(sortie[0]["error"]["data"]["type"], "EOFError")
        self.assertEqual(sortie[1], {"id": 2, "result": "pong"})

    def test_print_ne_casse_pas_le_protocole(self):
        self.yaml("Print")
        p = lancer()
        out, err = p.communicate(self.gen(), timeout=30)
        sortie = [json.loads(x) for x in out.decode("utf-8").splitlines()]
        self.assertEqual(len(sortie), 1)
        self.assertIn("result", sortie[0])
        self.assertIn("ligne imprimee par le generateur", err.decode("utf-8"))
        self.assertIn("faux generateur", err.decode("utf-8"))  # logging -> stderr

    def test_import_impossible(self):
        self.yaml("BadImport")
        sortie, _ = echanger(self.gen())
        self.assertEqual(sortie[0]["error"]["code"], -32011)

    def test_archipelago_absent(self):
        self.yaml("Alice")
        vide = os.path.join(self.tmp.name, "vide")
        os.makedirs(vide)
        sortie, _ = echanger(self.gen(archipelago_dir=vide))
        self.assertEqual(sortie[0]["error"]["code"], -32011)

    def test_deuxieme_appel(self):
        self.yaml("Alice")
        sortie, code = echanger(self.gen(1) + self.gen(2) + req(3, "ping"))
        self.assertIn("result", sortie[0])
        self.assertEqual(sortie[1]["error"]["code"], -32012)
        self.assertEqual(sortie[2], {"id": 3, "result": "pong"})
        self.assertEqual(code, 0)

    def test_params_invalides(self):
        self.yaml("Alice")
        mauvais = [
            {"spoiler": 4},
            {"spoiler": True},
            {"spoiler": "3"},
            {"race": 1},
            {"seed": -1},
            {"seed": "42"},
            {"seed": True},
            {"output_dir": ""},
            {"site_packages": 5},
            {"archipelago_dir": os.path.join(self.tmp.name, "nope")},
            {"player_files_dir": os.path.join(self.tmp.name, "nope")},
        ]
        octets = b"".join(self.gen(i + 1, **m) for i, m in enumerate(mauvais))
        octets += self.gen(99)  # un appel valide derriere : les refus ne consomment pas le generate
        sortie, code = echanger(octets)
        for i, m in enumerate(mauvais):
            self.assertEqual(sortie[i]["error"]["code"], -32602, m)
        self.assertIn("result", sortie[-1])
        self.assertEqual(code, 0)

    def test_site_packages_inexistant(self):
        self.yaml("Alice")
        absent = os.path.join(self.tmp.name, "pas-de-site-packages")
        existant = os.path.join(self.tmp.name, "site-packages")
        os.makedirs(existant)
        octets = self.gen(1, site_packages=absent) + self.gen(2, site_packages=existant)
        sortie, code = echanger(octets)
        self.assertEqual(sortie[0]["error"]["code"], -32602)
        self.assertIn("introuvable", sortie[0]["error"]["message"])
        self.assertIn("result", sortie[1])  # le refus n'a pas consomme le generate
        self.assertEqual(code, 0)

    def test_params_manquants(self):
        sortie, _ = echanger(req(1, "generate", {}))
        self.assertEqual(sortie[0]["error"]["code"], -32602)

    def test_zip_hors_output_dir(self):
        self.yaml("Escape")
        sortie, code = echanger(self.gen() + req(2, "ping"))
        self.assertEqual(sortie[0]["error"]["code"], -32010)
        self.assertIn("output_dir", sortie[0]["error"]["message"] + str(sortie[0]["error"]["data"]))
        self.assertEqual(sortie[1], {"id": 2, "result": "pong"})


FAUX_EXPORT = """
import json
def export(mw, path, max_bytes):
    if mw.player_name[1] == "ExportPlante":
        raise ValueError("export impossible")
    text = json.dumps({"format_version": 1, "joueurs": [mw.player_name[i] for i in mw.player_ids]})
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)
    return {"bytes": len(text), "opaque_rules": 3}
"""


class TestExport(unittest.TestCase):
    """`export_worlds` (ADR 0028) : arret juste avant le placement, module d'export recu en texte."""

    # Memes preparatifs que TestGenerate, sans heriter de ses tests.
    setUp = TestGenerate.setUp
    yaml = TestGenerate.yaml
    params = TestGenerate.params

    def exp(self, id_=1, **extra):
        p = self.params()
        p.update({"export_path": os.path.join(self.sortie, "export.json"), "exporter": FAUX_EXPORT})
        p.update(extra)
        return req(id_, "export_worlds", p)

    def test_export_normal(self):
        self.yaml("Alice")
        sortie, code = echanger(self.exp() + req(2, "ping"))
        r = sortie[0]["result"]
        self.assertEqual(r["players"], [{"slot": 1, "name": "Alice", "game": "FakeGame"}])
        self.assertEqual(r["opaque_rules"], 3)
        with open(r["export"], encoding="utf-8") as f:
            self.assertEqual(json.load(f)["joueurs"], ["Alice"])
        # Le placement est en attente : pas de zip ecrit.
        self.assertEqual([n for n in os.listdir(self.sortie) if n.endswith(".zip")], [])
        self.assertEqual(sortie[1], {"id": 2, "result": "pong"})
        self.assertEqual(code, 0)

    def test_export_puis_finish(self):
        self.yaml("Alice")
        placement = os.path.join(self.tmp.name, "placement.json")
        with open(placement, "w", encoding="utf-8") as f:
            f.write("[]")
        fin = req(2, "finish_worlds", {"placement_path": placement})
        sortie, code = echanger(self.exp() + fin + req(3, "ping"))
        r = sortie[1]["result"]
        # Main.main a repris apres le placement et ecrit la seed.
        self.assertEqual(r["seed_name"], "42")
        self.assertTrue(os.path.isfile(r["zip"]))
        self.assertEqual(sortie[2], {"id": 3, "result": "pong"})
        self.assertEqual(code, 0)

    def test_finish_sans_export(self):
        sortie, _ = echanger(req(1, "finish_worlds", {"placement_path": os.path.abspath("x")}))
        self.assertEqual(sortie[0]["error"]["code"], -32014)

    def test_finish_placement_illisible(self):
        self.yaml("Alice")
        sortie, _ = echanger(self.exp() + req(2, "finish_worlds", {"placement_path": "relatif"}))
        self.assertEqual(sortie[1]["error"]["code"], -32602)

    def test_export_qui_plante(self):
        self.yaml("ExportPlante")
        sortie, _ = echanger(self.exp())
        self.assertEqual(sortie[0]["error"]["code"], -32013)
        self.assertIn("export impossible", sortie[0]["error"]["message"])

    def test_placement_jamais_atteint(self):
        self.yaml("NoFill")
        sortie, _ = echanger(self.exp())
        self.assertEqual(sortie[0]["error"]["code"], -32010)

    def test_params_export(self):
        self.yaml("Alice")
        sortie, _ = echanger(self.exp(1, exporter="") + self.exp(2, export_path="relatif.json")
                             + self.exp(3))
        self.assertEqual(sortie[0]["error"]["code"], -32602)
        self.assertEqual(sortie[1]["error"]["code"], -32602)
        self.assertIn("result", sortie[2])  # les refus n'ont pas consomme le process


class TestSondage(unittest.TestCase):
    """probe() de raftup_export.py (ADR 0028, etape 2), sans Archipelago."""

    @classmethod
    def setUpClass(cls):
        with open(os.path.join(ICI, "raftup_export.py"), encoding="utf-8") as f:
            cls.ns = {"__name__": "raftup_export"}
            exec(compile(f.read(), "raftup_export.py", "exec"), cls.ns)

    def sonde(self, rule):
        return self.ns["probe"](None, rule)

    def refus(self, rule, mot):
        with self.assertRaises(self.ns["Refused"]) as ctx:
            self.sonde(rule)
        self.assertIn(mot, str(ctx.exception))

    def test_et_ou(self):
        r = self.sonde(lambda s: s.has("A", 1) and (s.has("B", 1) or s.has("C", 1, 2)))
        self.assertEqual(r, {"t": "or", "of": [
            {"t": "and", "of": [{"t": "has", "player": 1, "item": "A", "count": 1},
                                {"t": "has", "player": 1, "item": "B", "count": 1}]},
            {"t": "and", "of": [{"t": "has", "player": 1, "item": "A", "count": 1},
                                {"t": "has", "player": 1, "item": "C", "count": 2}]},
        ]})

    def test_constantes(self):
        self.assertEqual(self.sonde(lambda s: True), {"t": "true"})
        self.assertEqual(self.sonde(lambda s: False), {"t": "false"})
        # Une question qui ne change rien au resultat disparait.
        self.assertEqual(self.sonde(lambda s: s.has("A", 1) or True), {"t": "true"})

    def test_compteur_compare(self):
        r = self.sonde(lambda s: s.count("Gem", 2) >= 3)
        self.assertEqual(r, {"t": "and", "of": [{"t": "has", "player": 2, "item": "Gem", "count": 3}]})
        r = self.sonde(lambda s: s.count("Gem", 2) > 1)
        self.assertEqual(r["of"][0]["count"], 2)

    def test_reach_et_listes(self):
        r = self.sonde(lambda s: s.can_reach("Cave", "Region", 1)
                       or s.has_from_list(["G", "H"], 1, 3))
        atomes = sorted(t["of"][0]["t"] for t in r["of"])
        self.assertEqual(atomes, ["can_reach_region", "has_from_list"])

    def test_refus(self):
        self.refus(lambda s: not s.has("A", 1), "non monotone")
        self.refus(lambda s: s.prog_items[1].items(), "prog_items")
        self.refus(lambda s: s.stale_cache, "state.stale_cache")
        # Un entier (valeur d'option), None ou un objet se lisent comme vrai / faux,
        # comme Archipelago le fait (`if rule(state)`).
        self.assertEqual(self.sonde(lambda s: s.has("A", 1) or 0),
                         {"t": "and", "of": [{"t": "has", "player": 1, "item": "A", "count": 1}]})
        self.assertEqual(self.sonde(lambda s: 1), {"t": "true"})
        self.assertEqual(self.sonde(lambda s: None), {"t": "false"})
        self.assertEqual(self.sonde(lambda s: "oui"), {"t": "true"})
        compteur = [0]

        def instable(s):
            compteur[0] += 1
            return compteur[0] % 2 == 0

        self.refus(instable, "instable")

    def test_trop_de_chemins(self):
        self.refus(lambda s: sum(s.has(str(i), 1) for i in range(13)) >= 13, "chemins")


def _has(item, count, player=1):
    return {"t": "has", "player": player, "item": item, "count": count}


class TestSondageLot2(unittest.TestCase):
    """Lot 2 de l'inventaire de conversion : calculs sur les compteurs, prog_items,
    multiworld, reachable_regions, methodes de LogicMixin, plafonds."""

    def setUp(self):
        # Un namespace neuf par test : certains changent les plafonds (globales).
        with open(os.path.join(ICI, "raftup_export.py"), encoding="utf-8") as f:
            self.ns = {"__name__": "raftup_export"}
            exec(compile(f.read(), "raftup_export.py", "exec"), self.ns)

    def sonde(self, rule, copies=None, mw=None):
        return self.ns["probe"](mw, rule, copies)

    def refus(self, rule, mot, copies=None, mw=None):
        with self.assertRaises(self.ns["Refused"]) as ctx:
            self.sonde(rule, copies, mw)
        self.assertIn(mot, str(ctx.exception))

    def test_seuil_flottant(self):
        # `count >= 1.5` veut dire « au moins 2 » (avant : tronque a 1, faux).
        self.assertEqual(self.sonde(lambda s: s.count("Gem", 1) >= 1.5),
                         {"t": "and", "of": [_has("Gem", 2)]})
        self.assertEqual(self.sonde(lambda s: s.count("Gem", 1) > 1.5),
                         {"t": "and", "of": [_has("Gem", 2)]})
        self.assertEqual(self.sonde(lambda s: not s.count("Gem", 1) < 2.5),
                         {"t": "and", "of": [_has("Gem", 3)]})

    def test_somme_de_compteurs(self):
        # Overcooked! 2 : etoiles + etoiles bonus >= 3, avec 2 exemplaires de chaque.
        copies = {(1, "Star"): 2, (1, "Bonus"): 2}
        r = self.sonde(lambda s: s.count("Star", 1) + s.count("Bonus", 1) >= 3, copies)
        termes = sorted(sorted((a["item"], a["count"]) for a in t["of"]) for t in r["of"])
        self.assertEqual(termes, [[("Bonus", 1), ("Star", 2)], [("Bonus", 2), ("Star", 1)]])

    def test_range_et_produit(self):
        # shapez : `for _ in range(count)` et `count * 10`.
        copies = {(1, "Up"): 3}

        def regle(s):
            m = 1
            for _ in range(s.count("Up", 1)):
                m *= 2
            return m >= 4

        self.assertEqual(self.sonde(regle, copies), {"t": "and", "of": [_has("Up", 2)]})
        self.assertEqual(self.sonde(lambda s: s.count("Up", 1) * 10 >= 25, copies),
                         {"t": "and", "of": [_has("Up", 3)]})
        # Objet absent de la partie : le compteur vaut 0 sans question.
        self.assertEqual(self.sonde(lambda s: s.count("Absent", 1) * 2 >= 1, copies),
                         {"t": "false"})

    def test_compteur_cle_de_cache(self):
        # Zillion : un compteur passe a une fonction en cache (cle de dictionnaire).
        copies = {(1, "Gun"): 3}
        cache = {}

        def niveau(n):
            return cache.setdefault(n, int(n) >= 2)

        self.assertEqual(self.sonde(lambda s: niveau(s.count("Gun", 1)), copies),
                         {"t": "and", "of": [_has("Gun", 2)]})

    def test_prog_items(self):
        r = self.sonde(lambda s: s.prog_items[1]["A"] > 0)
        self.assertEqual(r, {"t": "and", "of": [_has("A", 1)]})
        r = self.sonde(lambda s: s.prog_items[1].get("A", 0) >= 3)
        self.assertEqual(r, {"t": "and", "of": [_has("A", 3)]})
        self.refus(lambda s: s.prog_items[1].__setitem__("A", 3), "ecriture")
        self.refus(lambda s: list(s.prog_items[1]), "parcours")

    def test_count_from_list_unique(self):
        # Yu-Gi-Oh! 2006 : nombre d'objets differents d'une liste.
        r = self.sonde(lambda s: s.count_from_list_unique(["A", "B", "C"], 1) >= 2)
        self.assertEqual(r, {"t": "and", "of": [
            {"t": "has_from_list_unique", "player": 1, "items": ["A", "B", "C"], "count": 2}]})
        r = self.sonde(lambda s: s.count_from_list(["A", "B"], 1) >= 4)
        self.assertEqual(r["of"][0]["t"], "has_from_list")
        # En calcul, le plafond est le nombre d'objets differents qui existent.
        copies = {(1, "A"): 5, (1, "B"): 1}
        r = self.sonde(lambda s: s.count_from_list_unique(["A", "B", "C"], 1) * 2 >= 3, copies)
        self.assertEqual(r["of"][0]["count"], 2)

    def test_multiworld_en_lecture(self):
        class Options:
            goal = 3

        class MW:
            options = Options()

        r = self.sonde(lambda s: s.has("A", 1, s.multiworld.options.goal), mw=MW())
        self.assertEqual(r, {"t": "and", "of": [_has("A", 3)]})

        def ecrit(s):
            s.multiworld.seed = 1
            return True

        self.refus(ecrit, "ecriture dans state.multiworld", mw=MW())

    def test_regions_atteintes(self):
        # Region.can_reach d'Archipelago : `stale` puis `region in reachable_regions`.
        class Region:
            name, player = "Cave", 1

            def can_reach(self, state):
                if state.stale[self.player]:
                    state.update_reachable_regions(self.player)
                return self in state.reachable_regions[self.player]

        r = self.sonde(lambda s: Region().can_reach(s))
        self.assertEqual(r, {"t": "and", "of": [
            {"t": "can_reach_region", "player": 1, "region": "Cave"}]})
        self.refus(lambda s: len(list(s.reachable_regions[1])) > 3, "parcours")

    def test_methode_logic_mixin(self):
        # Wargroove : methode ajoutee a CollectionState par un LogicMixin.
        import types
        base = types.ModuleType("BaseClasses")

        class CollectionState:
            def collect(self, item):  # methode d'Archipelago : refusee
                raise AssertionError

        def _wargroove_has_item(self, player, item):
            return self.has(item, player)

        _wargroove_has_item.__module__ = "worlds.wargroove.Rules"
        CollectionState._wargroove_has_item = _wargroove_has_item
        CollectionState.collect.__module__ = "BaseClasses"
        CollectionState._cache = {}  # attribut de donnees : refuse
        base.CollectionState = CollectionState
        sys.modules["BaseClasses"] = base
        try:
            r = self.sonde(lambda s: s._wargroove_has_item(1, "Mage"))
            self.assertEqual(r, {"t": "and", "of": [_has("Mage", 1)]})
            self.refus(lambda s: s.collect(None), "state.collect")
            self.refus(lambda s: s._cache, "state._cache")
        finally:
            del sys.modules["BaseClasses"]

    def test_plafonds_reglables(self):
        regle = lambda s: sum(s.has(str(i), 1) for i in range(13)) >= 13  # noqa: E731
        self.refus(regle, "chemins")
        self.ns["MAX_PATHS"] = 10000
        r = self.sonde(regle)
        self.assertEqual(len(r["of"]), 13)  # un seul « et » de 13 objets
        self.ns["MAX_QUESTIONS"] = 5
        self.refus(regle, "plus de 5 questions")


class TestReglesObjet(unittest.TestCase):
    """Regles d'objet de raftup_export.py (ADR 0029), sans Archipelago."""

    @classmethod
    def setUpClass(cls):
        with open(os.path.join(ICI, "raftup_export.py"), encoding="utf-8") as f:
            cls.ns = {"__name__": "raftup_export"}
            exec(compile(f.read(), "raftup_export.py", "exec"), cls.ns)

    def exporteur(self, non_local=()):
        class Objet:
            def __init__(self, player, name):
                self.player, self.name = player, name

        class Valeur:
            value = set(non_local)

        class Options:
            non_local_items = Valeur()

        class Monde:
            options = Options()

        class MW:
            worlds = {1: Monde()}

        e = self.ns["Exporter"].__new__(self.ns["Exporter"])
        e.mw = MW()
        e.kinds = {(1, n): [Objet(1, n)] for n in ("Key", "Coin", "Gem")}
        e.item_rules, e._rule_index = [], {}
        return e

    def test_listes(self):
        e = self.exporteur()
        self.assertIsNone(e._compute_item_rule(lambda i: True, None))
        a = e._compute_item_rule(lambda i: i.name != "Key", None)
        self.assertEqual(e.item_rules[a], {"allow_only": False, "list": [[1, "Key"]], "always": []})
        b = e._compute_item_rule(lambda i: i.name == "Coin", None)
        self.assertEqual(e.item_rules[b], {"allow_only": True, "list": [[1, "Coin"]], "always": []})
        # Meme contenu, meme index.
        self.assertEqual(e._compute_item_rule(lambda i: i.name not in ("Key",), None), a)

    def test_toujours_acceptes(self):
        e = self.exporteur(non_local={"Gem"})
        r = e._compute_item_rule(lambda i: False, lambda state, i: i.name in ("Key", "Gem"))
        # Gem est non local : always_allow ne le laisse pas passer (Location.can_fill).
        self.assertEqual(e.item_rules[r]["always"], [[1, "Key"]])
        with self.assertRaises(self.ns["Refused"]):
            e._compute_item_rule(lambda i: True, lambda state, i: state.has("Key", 1))


class TestBlocageReseau(unittest.TestCase):
    """block_network() (RAFTUP_BLOCK_NETWORK=1) : refuse le sortant, garde la boucle locale."""

    def test_refus_sortant_et_boucle_locale(self):
        chemin = os.path.join(ICI, "raftup_worker.py")
        code = "\n".join([
            "import re, socket, sys",
            "src = open(%r, encoding='utf-8').read()" % chemin,
            "exec(re.search(r'def block_network\\(\\):.*?(?=\\n\\ndef generate)', src, re.S).group(0))",
            "block_network()",
            "for f in (lambda: socket.gethostbyname('example.com'),",
            "          lambda: socket.socket().connect(('93.184.216.34', 80))):",
            "    try:",
            "        f(); print('PAS BLOQUE')",
            "    except OSError:",
            "        print('bloque')",
            "a, b = socket.socketpair()",
            "a.send(b'x'); print(b.recv(1))",
        ])
        r = subprocess.run([sys.executable, "-I", "-c", code], capture_output=True, timeout=30)
        self.assertEqual(r.stdout.decode().split(), ["bloque", "bloque", "b'x'"], r.stderr)


if __name__ == "__main__":
    unittest.main()
