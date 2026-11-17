"""Faux Utils : juste de quoi imiter ce que le worker lit."""
import os

__version__ = "0.0.0-fake"


def local_path(*path):
    return os.path.join(os.getcwd(), *path)
