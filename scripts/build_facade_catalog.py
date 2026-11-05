#!/usr/bin/env python3
"""Generate a 600-image ground-floor facade library and four atlases.

150 images per category:
  storefront, executive, residential, industrial

Each image is 256x256 and is packed into a 4096x4096 atlas (16x16 tiles).
Outputs under viewer/assets/textures/facades/ and a catalog JSON:
  viewer/assets/models/facade_catalog.json
"""
import json
import os
import random

from PIL import Image, ImageDraw, ImageFilter

ROOT = "/home/orbit9/Work/games/Manhattan"
OUT_TEX = os.path.join(ROOT, "viewer", "assets", "textures", "facades")
OUT_JSON = os.path.join(ROOT, "viewer", "assets", "models", "facade_catalog.json")
os.makedirs(OUT_TEX, exist_ok=True)

TILE = 256
ATLAS = 4096
GRID = ATLAS // TILE
PER_CATEGORY = 150
CATEGORIES = {
    "storefront": {"width_m": 8.0, "height_m": 3.2},
    "executive": {"width_m": 12.0, "height_m": 3.6},
    "residential": {"width_m": 6.0, "height_m": 3.0},
    "industrial": {"width_m": 10.0, "height_m": 3.4},
}


def shade(c, f):
    return tuple(max(0, min(255, int(v * f))) for v in c)


def rect(d, x0, y0, x1, y1, fill, outline=None):
    d.rectangle((x0, y0, x1, y1), fill=fill, outline=outline)


def noise_wall(img, rng, base):
    px = img.load()
    for y in range(TILE):
        for x in range(TILE):
            n = rng.randint(-7, 7)
            px[x, y] = shade(base, 1.0 + n / 255.0)


def storefront(rng):
    wall = rng.choice([(86, 68, 58), (72, 74, 78), (110, 82, 60), (56, 62, 68), (128, 104, 82)])
    img = Image.new("RGB", (TILE, TILE), wall)
    noise_wall(img, rng, wall)
    d = ImageDraw.Draw(img)
    # sign band
    rect(d, 0, 0, TILE, 42, rng.choice([(20, 24, 30), (60, 40, 30), (32, 42, 50)]))
    for i in range(rng.randint(3, 7)):
        x = rng.randint(12, 190)
        rect(d, x, 13, x + rng.randint(18, 52), 19, (230, 220, 180))
    # shop windows
    win_y = 78
    count = rng.randint(2, 4)
    gap = 8
    ww = (TILE - 24 - gap * (count - 1)) // count
    for i in range(count):
        x = 12 + i * (ww + gap)
        rect(d, x, win_y, x + ww, 220, (30, 45, 52), (22, 20, 18))
        rect(d, x + 5, win_y + 5, x + ww - 5, 214, (42, 66, 74))
        # interior shapes
        for _ in range(rng.randint(2, 5)):
            cx = rng.randint(x + 10, x + ww - 10)
            cy = rng.randint(win_y + 12, 205)
            rect(d, cx - 4, cy - 9, cx + 4, cy + 9, rng.choice([(190, 150, 90), (90, 110, 120), (170, 170, 160)]))
    # door
    dx = rng.choice([16, TILE - 62])
    rect(d, dx, 92, dx + 42, 235, (28, 34, 38), (15, 15, 15))
    rect(d, dx + 6, 100, dx + 36, 232, (44, 56, 62))
    # awning
    rect(d, 0, 52, TILE, 70, rng.choice([(110, 30, 28), (30, 68, 48), (40, 45, 90)]))
    for x in range(0, TILE, 24):
        rect(d, x, 52, x + 12, 70, (210, 200, 180))
    # pilasters
    for x in (0, TILE - 12):
        rect(d, x, 0, x + 12, TILE, shade(wall, 0.72))
    return img


def executive(rng):
    glass = rng.choice([(34, 64, 82), (30, 52, 60), (45, 55, 68), (52, 72, 90)])
    img = Image.new("RGB", (TILE, TILE), glass)
    noise_wall(img, rng, glass)
    d = ImageDraw.Draw(img)
    # marble/stone piers
    pier = rng.choice([(178, 168, 145), (145, 136, 120), (110, 108, 104), (198, 188, 170)])
    for x in (0, 62, 124, 186, 248):
        rect(d, x, 0, x + 8, TILE, pier)
    rect(d, 0, 0, TILE, 22, shade(pier, 0.8))
    # lobby glass
    for x in (12, 74, 136, 198):
        rect(d, x, 24, x + 44, 226, (20, 42, 58), (14, 30, 40))
        for yy in range(32, 220, 22):
            rect(d, x + 4, yy, x + 40, yy + 2, (90, 140, 160))
        # interior glow
        rect(d, x + 8, 175, x + 36, 220, (150, 120, 80))
    # revolving/entry door
    rect(d, 92, 92, 164, 236, (18, 30, 38), (10, 20, 28))
    for a in range(0, 360, 30):
        import math
        cx, cy, rr = 128, 165, 26
        x = cx + int(math.cos(math.radians(a)) * rr)
        y = cy + int(math.sin(math.radians(a)) * rr)
        d.line((cx, cy, x, y), fill=(180, 190, 190), width=2)
    # address / canopy
    rect(d, 0, 226, TILE, 242, shade(pier, 0.7))
    return img


def residential(rng):
    brick = rng.choice([(116, 62, 48), (88, 70, 56), (76, 74, 68), (128, 92, 64), (98, 52, 44)])
    img = Image.new("RGB", (TILE, TILE), brick)
    noise_wall(img, rng, brick)
    d = ImageDraw.Draw(img)
    # floors / cornice
    rect(d, 0, 0, TILE, 18, shade(brick, 0.65))
    rect(d, 0, 18, TILE, 26, shade(brick, 1.2))
    # windows
    cols = rng.choice([3, 4])
    for c in range(cols):
        x = 18 + c * (TILE - 36) // cols
        w = (TILE - 36) // cols - 16
        rect(d, x, 44, x + w, 108, (30, 38, 42), (20, 20, 20))
        rect(d, x + 4, 48, x + w - 4, 104, (70, 94, 105))
        rect(d, x + w // 2 - 1, 48, x + w // 2 + 1, 104, (200, 190, 170))
        rect(d, x - 3, 40, x + w + 3, 44, shade(brick, 0.75))
    # stoop/door
    dx = rng.choice([18, TILE - 76])
    rect(d, dx, 130, dx + 52, 230, shade(brick, 0.7))
    rect(d, dx + 8, 138, dx + 44, 230, (34, 34, 30))
    rect(d, dx + 13, 145, dx + 39, 230, (72, 52, 36))
    rect(d, dx - 8, 230, dx + 60, 243, (150, 140, 125))
    # fire escape
    if rng.random() < 0.5:
        fy = 96
        for k in range(3):
            y = fy + k * 46
            rect(d, 4, y, TILE - 4, y + 3, (22, 24, 26))
            rect(d, 8, y + 3, 10, y + 40, (22, 24, 26))
            rect(d, TILE - 10, y + 3, TILE - 8, y + 40, (22, 24, 26))
    return img


def industrial(rng):
    metal = rng.choice([(88, 92, 96), (72, 80, 76), (105, 95, 82), (66, 70, 74), (120, 110, 96)])
    img = Image.new("RGB", (TILE, TILE), metal)
    noise_wall(img, rng, metal)
    d = ImageDraw.Draw(img)
    # corrugation
    for x in range(0, TILE, 12):
        rect(d, x, 0, x + 3, TILE, shade(metal, 0.8))
    # roll-up doors
    for x in (18, 120):
        rect(d, x, 82, x + 88, 238, (45, 48, 50), (20, 22, 24))
        for yy in range(90, 236, 9):
            rect(d, x + 4, yy, x + 84, yy + 2, (75, 78, 80))
    # loading dock / vents
    rect(d, 0, 238, TILE, TILE, (58, 58, 54))
    for i in range(5):
        x = rng.randint(10, 220)
        rect(d, x, 20, x + 18, 42, (35, 38, 40))
        rect(d, x + 3, 23, x + 15, 39, (90, 92, 90))
    # sign
    rect(d, 0, 0, TILE, 16, (38, 42, 44))
    for i in range(6):
        rect(d, 12 + i * 36, 3, 30 + i * 36, 13, (180, 160, 80))
    return img


GENERATORS = {
    "storefront": storefront,
    "executive": executive,
    "residential": residential,
    "industrial": industrial,
}

catalog = {
    "tile_px": TILE,
    "atlas_px": ATLAS,
    "tile_uv": TILE / ATLAS,
    "categories": {},
}

for category, info in CATEGORIES.items():
    atlas = Image.new("RGB", (ATLAS, ATLAS), (24, 24, 26))
    images = []
    for i in range(PER_CATEGORY):
        rng = random.Random(0x51A7_0000 + sum(map(ord, category)) * 1000 + i)
        tile = GENERATORS[category](rng)
        col = i % GRID
        row = i // GRID
        atlas.paste(tile, (col * TILE, row * TILE))
        images.append({
            "id": f"{category}_{i:04d}",
            "col": col,
            "row": row,
            "width_m": info["width_m"],
            "height_m": info["height_m"],
        })
    name = f"{category}_atlas.jpg"
    atlas.save(os.path.join(OUT_TEX, name), quality=88, optimize=True)
    catalog["categories"][category] = {
        "atlas": name,
        "images": images,
    }
    print(category, len(images), name, os.path.getsize(os.path.join(OUT_TEX, name)))

with open(OUT_JSON, "w") as f:
    json.dump(catalog, f, separators=(",", ":"))
print("wrote", OUT_JSON)
