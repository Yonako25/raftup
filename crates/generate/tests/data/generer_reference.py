"""Génère une seed avec le vrai Archipelago (comme le worker de Raftup)."""
import os, site, sys
ap, sp, players, out, seed = sys.argv[1:6]
site.addsitedir(sp)
sys.path.insert(0, ap)
os.chdir(ap)
os.environ["SKIP_REQUIREMENTS_UPDATE"] = "1"
import settings
settings.no_gui = True
sys.argv = ["Generate.py", "--player_files_path", players, "--outputpath", out, "--spoiler", "2", "--seed", seed]
import Generate, Main
erargs, s = Generate.main()
mw = Main.main(erargs, s)
print("OK", mw.seed_name)
