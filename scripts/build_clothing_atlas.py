#!/usr/bin/env python3
"""Generate a compact clothing-pattern atlas for pedestrian outfits.

8 x 8 tiles of 128 x 128 on a 1024 x 1024 atlas.  Tile ranges are used by the
viewer: shirts 0-15, pants 16-31, skirts 32-39, jackets 40-47, hats 48-55,
bags 56-61.
"""
import json
import math
import os
import random

from PIL import Image, ImageDraw

ROOT = "/home/orbit9/Work/games/Manhattan"
OUT_TEX = os.path.join(ROOT, "viewer", "assets", "textures", "people")
OUT_JSON = os.path.join(ROOT, "viewer", "assets", "models", "clothing_catalog.json")
OUT_BUILD = os.path.join(ROOT, "build")
os.makedirs(OUT_TEX, exist_ok=True)
os.makedirs(OUT_BUILD, exist_ok=True)

TILE = 128
COLS, ROWS = 8, 8
ATLAS = TILE * COLS

PALETTE = [
    (196, 58, 54), (44, 92, 148), (54, 118, 82), (218, 172, 44),
    (108, 64, 128), (32, 32, 38), (224, 224, 216), (90, 62, 44),
    (212, 120, 150), (80, 146, 164), (150, 150, 150), (238, 132, 54),
    (60, 70, 90), (176, 196, 96), (120, 30, 70), (230, 200, 160),
]


def shade(c, f):
    return tuple(max(0, min(255, int(v * f))) for v in c)


def pattern(rng, kind, base, accent):
    img = Image.new("RGB", (TILE, TILE), base)
    d = ImageDraw.Draw(img)
    if kind == "solid":
        for _ in range(500):
            x, y = rng.randrange(TILE), rng.randrange(TILE)
            d.point((x, y), fill=shade(base, rng.uniform(0.92, 1.06)))
    elif kind == "stripe_v":
        for x in range(0, TILE, rng.choice([8, 12, 16, 24])):
            d.rectangle((x, 0, x + rng.randint(3, 7), TILE), fill=accent)
    elif kind == "stripe_h":
        for y in range(0, TILE, rng.choice([8, 12, 16, 24])):
            d.rectangle((0, y, TILE, y + rng.randint(3, 7)), fill=accent)
    elif kind == "check":
        step = rng.choice([12, 16, 20, 24])
        for x in range(0, TILE, step):
            d.rectangle((x, 0, x + 3, TILE), fill=accent)
        for y in range(0, TILE, step):
            d.rectangle((0, y, TILE, y + 3), fill=shade(accent, 0.8))
    elif kind == "dots":
        step = rng.choice([12, 16, 20])
        r = rng.randint(2, 4)
        for y in range(0, TILE, step):
            for x in range(0, TILE, step):
                d.ellipse((x - r, y - r, x + r, y + r), fill=accent)
    elif kind == "denim":
        for y in range(TILE):
            d.line((0, y, TILE, y), fill=shade(base, 0.92 + (y % 5) * 0.012))
        for _ in range(80):
            x, y = rng.randrange(TILE), rng.randrange(TILE)
            d.line((x, y, x + rng.randint(4, 18), y), fill=shade(base, rng.uniform(0.75, 1.1)))
    elif kind == "plaid":
        step = rng.choice([10, 14, 18])
        for x in range(0, TILE, step):
            d.rectangle((x, 0, x + 2, TILE), fill=accent)
        for y in range(0, TILE, step):
            d.rectangle((0, y, TILE, y + 2), fill=shade(accent, 0.7))
        for x in range(0, TILE, step * 2):
            d.rectangle((x, 0, x + 1, TILE), fill=shade(base, 0.75))
    elif kind == "floral":
        for _ in range(26):
            x, y = rng.randrange(TILE), rng.randrange(TILE)
            r = rng.randint(3, 7)
            for k in range(5):
                a = k / 5.0 * math.tau
                px = x + int(math.cos(a) * r)
                py = y + int(math.sin(a) * r)
                d.ellipse((px - 3, py - 3, px + 3, py + 3), fill=accent)
            d.ellipse((x - 2, y - 2, x + 2, y + 2), fill=(250, 230, 120))
    else:
        for _ in range(80):
            x, y = rng.randrange(TILE), rng.randrange(TILE)
            d.line((x, y, x + rng.randint(2, 10), y + rng.randint(-3, 3)),
                   fill=shade(base, rng.uniform(0.8, 1.15)), width=1)
    return img


KINDS = ["solid", "stripe_v", "stripe_h", "check", "dots", "denim", "plaid", "floral"]
atlas = Image.new("RGB", (ATLAS, ATLAS), (24, 24, 26))
tiles = []
for index in range(COLS * ROWS):
    rng = random.Random(0xC107_0000 + index * 7919)
    base = rng.choice(PALETTE)
    accent = rng.choice([c for c in PALETTE if c != base])
    kind = KINDS[index % len(KINDS)]
    img = pattern(rng, kind, base, accent)
    col, row = index % COLS, index // COLS
    atlas.paste(img, (col * TILE, row * TILE))
    tiles.append({"id": index, "col": col, "row": row, "kind": kind})

path = os.path.join(OUT_TEX, "clothing_atlas.jpg")
atlas.save(path, quality=92, optimize=True)
atlas.save(os.path.join(OUT_BUILD, "clothing_contact.png"))
catalog = {
    "tile_px": TILE,
    "cols": COLS,
    "rows": ROWS,
    "atlas": "textures/people/clothing_atlas.jpg",
    "ranges": {
        "shirts": [0, 16],
        "pants": [16, 32],
        "skirts": [32, 40],
        "jackets": [40, 48],
        "hats": [48, 56],
        "bags": [56, 62],
    },
    "tiles": tiles,
}
with open(OUT_JSON, "w") as handle:
    json.dump(catalog, handle, separators=(",", ":"))
print("wrote", path, atlas.size, os.path.getsize(path))
print("wrote", OUT_JSON, os.path.getsize(OUT_JSON))
