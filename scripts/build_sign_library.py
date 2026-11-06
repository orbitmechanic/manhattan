#!/usr/bin/env python3
"""Generate abstract illuminated shop-sign designs.

64 tiles of 256 x 128 on a 2048 x 1024 atlas.  The designs are deliberately
abstract blocky glyphs, not readable text.  Bright glyphs on dark backgrounds
work both as unlit signboards and as an emissive texture for illuminated or
blinking signs.
"""
import json
import math
import os
import random

from PIL import Image, ImageDraw

ROOT = "/home/orbit9/Work/games/Manhattan"
OUT_TEX = os.path.join(ROOT, "viewer", "assets", "textures", "signage")
OUT_JSON = os.path.join(ROOT, "viewer", "assets", "models", "signage_catalog.json")
OUT_BUILD = os.path.join(ROOT, "build")
os.makedirs(OUT_TEX, exist_ok=True)
os.makedirs(OUT_BUILD, exist_ok=True)

TILE_W, TILE_H = 256, 128
COLS, ROWS = 8, 8
COUNT = COLS * ROWS
ATLAS_W, ATLAS_H = COLS * TILE_W, ROWS * TILE_H

BACKGROUNDS = [
    (18, 22, 30), (28, 18, 24), (16, 26, 26), (30, 24, 14),
    (24, 18, 36), (12, 12, 14), (36, 20, 18), (14, 24, 32),
]
COLORS = [
    (255, 220, 120), (255, 120, 90), (120, 220, 255), (255, 140, 220),
    (160, 255, 170), (255, 250, 235), (255, 180, 70), (120, 170, 255),
]


def glyph(draw, rng, x, y, size, color):
    kind = rng.choice(["bars", "ring", "box", "zig", "arc", "cross"])
    if kind == "bars":
        for i in range(rng.randint(2, 4)):
            xx = x + i * size * 0.28
            draw.rectangle((xx, y, xx + size * rng.uniform(0.08, 0.18), y + size),
                           fill=color)
    elif kind == "ring":
        draw.ellipse((x, y + size * 0.1, x + size * 0.85, y + size * 0.95),
                     outline=color, width=max(2, int(size * 0.13)))
        if rng.random() < 0.5:
            draw.ellipse((x + size * 0.3, y + size * 0.4, x + size * 0.55, y + size * 0.65),
                         fill=color)
    elif kind == "box":
        draw.rectangle((x, y + size * 0.08, x + size * 0.75, y + size * 0.92), fill=color)
        if rng.random() < 0.5:
            draw.rectangle((x + size * 0.18, y + size * 0.3, x + size * 0.57, y + size * 0.7),
                           fill=BACKGROUNDS[0])
    elif kind == "zig":
        pts = []
        for i in range(5):
            pts.append((x + i * size * 0.2, y + (size * 0.85 if i % 2 else size * 0.15)))
        draw.line(pts, fill=color, width=max(2, int(size * 0.12)))
    elif kind == "arc":
        draw.arc((x, y, x + size, y + size * 1.2), 200, 340,
                 fill=color, width=max(2, int(size * 0.13)))
        draw.line((x + size * 0.18, y + size * 0.82, x + size * 0.18, y + size * 1.0),
                  fill=color, width=max(2, int(size * 0.12)))
    else:
        draw.line((x + size * 0.08, y + size * 0.5, x + size * 0.8, y + size * 0.5),
                  fill=color, width=max(2, int(size * 0.13)))
        draw.line((x + size * 0.44, y + size * 0.1, x + size * 0.44, y + size * 0.92),
                  fill=color, width=max(2, int(size * 0.13)))


def design(rng, index):
    bg = rng.choice(BACKGROUNDS)
    img = Image.new("RGB", (TILE_W, TILE_H), bg)
    draw = ImageDraw.Draw(img)
    # subtle panel framing
    border = rng.choice(COLORS)
    if rng.random() < 0.75:
        draw.rectangle((5, 5, TILE_W - 6, TILE_H - 6), outline=border, width=2)
    color = rng.choice(COLORS)
    # 3..5 abstract glyphs across the sign
    count = rng.randint(2, 4)
    size = rng.randint(50, 74)
    gap = (TILE_W - 30 - count * size) / max(1, count - 1)
    base_y = (TILE_H - size) // 2
    for i in range(count):
        x = 15 + i * (size + gap)
        c = color if rng.random() < 0.75 else rng.choice(COLORS)
        glyph(draw, rng, int(x), base_y, size, c)
    # occasional arrow / dot / underline
    if rng.random() < 0.45:
        y = TILE_H - 20
        draw.line((16, y, TILE_W - 16, y), fill=rng.choice(COLORS), width=2)
    if rng.random() < 0.35:
        for i in range(3):
            x = TILE_W - 46 + i * 10
            draw.ellipse((x, 12, x + 5, 17), fill=rng.choice(COLORS))
    # light noise / scanline texture
    for _ in range(40):
        x = rng.randrange(TILE_W)
        y = rng.randrange(TILE_H)
        draw.point((x, y), fill=tuple(min(255, c + 25) for c in bg))
    return img


atlas = Image.new("RGB", (ATLAS_W, ATLAS_H), (8, 8, 10))
tiles = []
for index in range(COUNT):
    rng = random.Random(0x51_6E_0000 + index * 7919)
    img = design(rng, index)
    col, row = index % COLS, index // COLS
    atlas.paste(img, (col * TILE_W, row * TILE_H))
    tiles.append({"id": index, "col": col, "row": row})

path = os.path.join(OUT_TEX, "signage_atlas.jpg")
atlas.save(path, quality=92, optimize=True)
atlas.save(os.path.join(OUT_BUILD, "signage_contact.png"))
catalog = {
    "tile_w": TILE_W,
    "tile_h": TILE_H,
    "cols": COLS,
    "rows": ROWS,
    "count": COUNT,
    "atlas": "textures/signage/signage_atlas.jpg",
    "tiles": tiles,
}
with open(OUT_JSON, "w") as handle:
    json.dump(catalog, handle, separators=(",", ":"))
print("wrote", path, atlas.size, os.path.getsize(path))
print("wrote", OUT_JSON, os.path.getsize(OUT_JSON))
