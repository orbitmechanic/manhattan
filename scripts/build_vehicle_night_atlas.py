#!/usr/bin/env python3
"""Generate the emissive vehicle night-light atlas.

4 x 4 tiles of 128 px on a 512 px atlas:
  tile 0  warm cabin windows
  tile 1  white headlight
  tile 2  red taillight
  tile 3  amber marker
  tile 4  red/blue emergency bar
  tiles 5-15 additional variants
"""
import math
import os
import random

from PIL import Image, ImageDraw, ImageFilter

ROOT = "/home/orbit9/Work/games/Manhattan"
OUT = os.path.join(ROOT, "viewer", "assets", "textures", "vehicles")
os.makedirs(OUT, exist_ok=True)
TILE = 128
COLS = 4
ATLAS = TILE * COLS


def glow(img, cx, cy, radius, color, strength=1.0):
    px = img.load()
    for y in range(TILE):
        for x in range(TILE):
            d = math.hypot(x - cx, y - cy)
            if d > radius:
                continue
            t = max(0.0, 1.0 - d / radius)
            t = t * t * strength
            r, g, b = px[x, y]
            px[x, y] = (
                min(255, int(r + color[0] * t)),
                min(255, int(g + color[1] * t)),
                min(255, int(b + color[2] * t)),
            )


def tile(index):
    rng = random.Random(0xA17E_0000 + index * 7919)
    img = Image.new("RGB", (TILE, TILE), (8, 9, 12))
    d = ImageDraw.Draw(img)
    if index % 5 == 0:
        # Warm cabin window pattern with interior silhouettes.
        for row in range(2):
            y0 = 26 + row * 42
            for col in range(2):
                x0 = 18 + col * 50
                d.rectangle((x0, y0, x0 + 40, y0 + 30), fill=(28, 24, 16))
                d.rectangle((x0 + 3, y0 + 3, x0 + 37, y0 + 27),
                            fill=(235, 178, 96))
                # seats / interior shadows
                d.rectangle((x0 + 8, y0 + 17, x0 + 22, y0 + 27), fill=(94, 62, 34))
                d.rectangle((x0 + 25, y0 + 21, x0 + 33, y0 + 27), fill=(70, 48, 30))
                glow(img, x0 + 20, y0 + 15, 26, (60, 42, 18))
    elif index % 5 == 1:
        # White headlight with radial glare.
        glow(img, 52, 64, 58, (240, 236, 210), 0.9)
        d.ellipse((34, 46, 70, 82), fill=(255, 250, 226))
        d.ellipse((42, 54, 62, 74), fill=(255, 255, 248))
        glow(img, 100, 64, 28, (160, 150, 110), 0.5)
    elif index % 5 == 2:
        # Red taillight.
        glow(img, 64, 64, 56, (170, 20, 18), 0.9)
        d.rounded_rectangle((26, 40, 102, 88), radius=18, fill=(190, 22, 20))
        d.rounded_rectangle((36, 50, 92, 78), radius=12, fill=(255, 70, 48))
        d.rectangle((58, 50, 70, 78), fill=(255, 180, 120))
    elif index % 5 == 3:
        # Amber marker / safety light.
        glow(img, 64, 64, 52, (200, 120, 20), 0.9)
        d.rounded_rectangle((36, 42, 92, 86), radius=20, fill=(230, 140, 28))
        d.rounded_rectangle((46, 52, 82, 76), radius=13, fill=(255, 196, 70))
    else:
        # Emergency red/blue bar.
        d.rounded_rectangle((14, 44, 62, 84), radius=10, fill=(210, 26, 28))
        d.rounded_rectangle((66, 44, 114, 84), radius=10, fill=(36, 88, 230))
        glow(img, 38, 64, 34, (140, 12, 14), 0.7)
        glow(img, 90, 64, 34, (18, 50, 150), 0.7)
        d.rectangle((36, 48, 40, 80), fill=(255, 180, 160))
        d.rectangle((88, 48, 92, 80), fill=(160, 200, 255))
    # scanlines and rim
    for y in range(0, TILE, 8):
        d.line((0, y, TILE, y), fill=(12, 13, 17), width=1)
    d.rectangle((0, 0, TILE - 1, TILE - 1), outline=(22, 23, 28), width=2)
    return img


atlas = Image.new("RGB", (ATLAS, ATLAS), (8, 9, 12))
for index in range(16):
    atlas.paste(tile(index), ((index % COLS) * TILE, (index // COLS) * TILE))
path = os.path.join(OUT, "vehicle_night_atlas.png")
atlas.save(path)
atlas.save(os.path.join(ROOT, "build", "vehicle_night_contact.png"))
print("wrote", path, atlas.size, os.path.getsize(path))
