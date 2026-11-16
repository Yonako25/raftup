"""Dessine l'icône du launcher (le logo de la maquette) en PNG et en ICO.

Usage : python -I make_icon.py   (écrit icon.png et icon.ico à côté du script)
Sans bibliothèque : le PNG est écrit à la main (zlib), l'ICO contient des PNG.
"""
import os, struct, zlib

HERE = os.path.dirname(os.path.abspath(__file__))
BACKGROUND = (0x16, 0x18, 0x1B, 255)
BLUE = (0x4C, 0x9A, 0xFF, 255)


def draw(size):
    """Fond acier, deux traits horizontaux, deux verticaux, un carré au centre."""
    px = [[BACKGROUND] * size for _ in range(size)]
    s = size / 28  # le logo SVG fait 28 × 28
    width = max(1, round(2 * s))

    def hline(y, x0, x1):
        for yy in range(round(y * s - width / 2), round(y * s + width / 2)):
            for xx in range(round(x0 * s), round(x1 * s)):
                if 0 <= yy < size and 0 <= xx < size:
                    px[yy][xx] = BLUE

    def vline(x, y0, y1):
        for xx in range(round(x * s - width / 2), round(x * s + width / 2)):
            for yy in range(round(y0 * s), round(y1 * s)):
                if 0 <= yy < size and 0 <= xx < size:
                    px[yy][xx] = BLUE

    hline(9, 4, 24); hline(19, 4, 24)
    vline(9, 4, 24); vline(19, 4, 24)
    hline(11, 11, 17); hline(17, 11, 17)
    vline(11, 11, 17); vline(17, 11, 17)
    return px


def png(px):
    size = len(px)
    raw = b"".join(b"\x00" + b"".join(bytes(c) for c in row) for row in px)

    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    header = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")


def ico(images):
    """ICO dont chaque image est un PNG (accepté depuis Windows Vista)."""
    header = struct.pack("<HHH", 0, 1, len(images))
    entries, data = b"", b""
    offset = 6 + 16 * len(images)
    for size, blob in images:
        dim = 0 if size >= 256 else size
        entries += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(blob), offset)
        data += blob
        offset += len(blob)
    return header + entries + data


pngs = [(size, png(draw(size))) for size in (16, 32, 48, 256)]
with open(os.path.join(HERE, "icon.png"), "wb") as f:
    f.write(pngs[-1][1])
with open(os.path.join(HERE, "icon.ico"), "wb") as f:
    f.write(ico(pngs))
print("ok")
