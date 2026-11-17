"""Tests du worker Python (unittest, bibliotheque standard seule).

Lancer : python -I crates/py-bridge/worker/test_worker.py
"""
import json
import os
import subprocess
import sys
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


if __name__ == "__main__":
    unittest.main()
