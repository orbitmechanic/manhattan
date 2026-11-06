#!/usr/bin/env python3
"""Generate the upper-floor facade and roof texture libraries.

Facades
  320 one-floor templates, 80 each for residential, industrial, financial and
  commercial.  A template covers one 12 m x 3 m floor module and tiles
  horizontally.  Day and night versions share the same window layout.

Roofs
  160 top-down 16 m x 16 m templates with gardens, pools, HVAC/AC equipment,
  solar panels, water tanks, patios and skylights at human scale.

Atlas output
  viewer/assets/textures/upper/facade_atlas_day.jpg   3840 x 3072 (10 x 32)
  viewer/assets/textures/upper/facade_atlas_night.jpg 1920 x 1536 (10 x 32)
  viewer/assets/textures/upper/roof_atlas_day.jpg     4096 x 2560 (16 x 10)
  viewer/assets/textures/upper/roof_atlas_night.jpg   2048 x 1280 (16 x 10)

Catalog
  viewer/assets/models/upper_facade_catalog.json
"""
import json
import math
import os
import random

from PIL import Image, ImageDraw, ImageFilter

ROOT = "/home/orbit9/Work/games/Manhattan"
OUT_TEX = os.path.join(ROOT, "viewer", "assets", "textures", "upper")
OUT_JSON = os.path.join(ROOT, "viewer", "assets", "models", "upper_facade_catalog.json")
OUT_BUILD = os.path.join(ROOT, "build")
os.makedirs(OUT_TEX, exist_ok=True)
os.makedirs(OUT_BUILD, exist_ok=True)

FACADE_W, FACADE_H = 384, 96
FACADE_COLS, FACADE_ROWS = 10, 32
FACADE_COUNT = FACADE_COLS * FACADE_ROWS  # 320
PER_CATEGORY = FACADE_COUNT // 4           # 80

ROOF_PX = 256
ROOF_COLS, ROOF_ROWS = 16, 10
ROOF_COUNT = ROOF_COLS * ROOF_ROWS          # 160

SS = 2  # supersampling factor

CATEGORIES = ["residential", "industrial", "financial", "commercial"]
CATEGORY_RANGES = {
    name: [CATEGORIES.index(name) * PER_CATEGORY, (CATEGORIES.index(name) + 1) * PER_CATEGORY]
    for name in CATEGORIES
}


def clamp(value, low, high):
    return max(low, min(high, value))


def shade(color, factor):
    return tuple(clamp(int(channel * factor), 0, 255) for channel in color)


def mix(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))


def noise_grid(rng, width, height, step, low, high):
    grid = [[rng.randint(low, high) for _ in range(width // step + 2)]
            for _ in range(height // step + 2)]
    return grid


def sample_grid(grid, x, y, step):
    gx, gy = x // step, y // step
    fx, fy = (x % step) / step, (y % step) / step
    a = grid[gy][gx]
    b = grid[gy][gx + 1]
    c = grid[gy + 1][gx]
    d = grid[gy + 1][gx + 1]
    top = a + (b - a) * fx
    bottom = c + (d - c) * fx
    return top + (bottom - top) * fy


def textured_base(img, rng, color, step=12, amp=14):
    width, height = img.size
    grid = noise_grid(rng, width, height, step, -amp, amp)
    pixels = img.load()
    for y in range(height):
        for x in range(width):
            n = sample_grid(grid, x, y, step)
            pixels[x, y] = shade(color, 1.0 + n / 255.0)


# ---------------------------------------------------------------------------
# facades
# ---------------------------------------------------------------------------
def facade_palette(category, rng):
    if category == "residential":
        return rng.choice([
            (146, 88, 66), (122, 84, 66), (170, 132, 100), (118, 112, 100),
            (156, 142, 122), (104, 72, 58), (184, 154, 118), (132, 96, 78),
        ])
    if category == "industrial":
        return rng.choice([
            (112, 118, 112), (96, 104, 100), (128, 120, 104), (86, 96, 96),
            (142, 132, 112), (104, 96, 84),
        ])
    if category == "financial":
        return rng.choice([
            (52, 76, 94), (44, 62, 76), (66, 84, 96), (38, 58, 70),
            (78, 88, 96), (56, 70, 82),
        ])
    return rng.choice([
        (152, 140, 124), (132, 124, 116), (170, 156, 136), (112, 108, 104),
        (144, 130, 110), (120, 120, 118),
    ])


def draw_wall_texture(draw, category, rng, wall, width, height):
    if category == "residential":
        # brick courses
        course = 12
        for y in range(0, height, course):
            draw.line((0, y, width, y), fill=shade(wall, 0.82), width=1)
            offset = 0 if (y // course) % 2 == 0 else 12
            for x in range(offset, width, 24):
                draw.line((x, y, x, y + course), fill=shade(wall, 0.88), width=1)
        for _ in range(70):
            x = rng.randrange(width)
            y = rng.randrange(height)
            draw.rectangle((x, y, x + 3, y + 2), fill=shade(wall, rng.uniform(0.75, 1.12)))
    elif category == "industrial":
        for x in range(0, width, 10):
            draw.line((x, 0, x, height), fill=shade(wall, 0.86), width=2)
            draw.line((x + 2, 0, x + 2, height), fill=shade(wall, 1.08), width=1)
        for _ in range(10):
            x = rng.randrange(width)
            draw.line((x, rng.randrange(height // 2), x + rng.randint(-4, 4), height),
                      fill=shade(wall, rng.uniform(0.58, 0.78)), width=rng.randint(1, 3))
    elif category == "financial":
        for x in range(0, width, 48):
            draw.line((x, 0, x, height), fill=shade(wall, 0.7), width=3)
            draw.line((x + 4, 0, x + 4, height), fill=shade(wall, 1.15), width=1)
        for y in range(0, height, 32):
            draw.line((0, y, width, y), fill=shade(wall, 0.78), width=1)
    else:
        # stone blocks
        for y in range(0, height, 24):
            draw.line((0, y, width, y), fill=shade(wall, 0.8), width=1)
            offset = 0 if (y // 24) % 2 == 0 else 20
            for x in range(offset, width, 40):
                draw.line((x, y, x, y + 24), fill=shade(wall, 0.84), width=1)


def draw_window(draw, rect, rng, category, wall, lit_night=False, night=False):
    x0, y0, x1, y1 = rect
    w, h = x1 - x0, y1 - y0
    if night:
        frame = (20, 22, 24)
        glass = (18, 24, 30)
    else:
        frame = shade(wall, 0.55)
        glass = rng.choice([(120, 160, 178), (96, 132, 152), (150, 176, 186), (78, 106, 126)])
    # recess shadow + frame
    draw.rectangle((x0 - 3, y0 - 3, x1 + 3, y1 + 3), fill=frame)
    draw.rectangle((x0, y0, x1, y1), fill=glass)
    if not night:
        # sky reflection gradient
        for i in range(h):
            t = i / max(1, h - 1)
            color = mix(shade(glass, 1.18), shade(glass, 0.7), t)
            draw.line((x0 + 2, y0 + i, x1 - 2, y0 + i), fill=color)
        # mullions
        cols = rng.choice([1, 1, 2, 3])
        for c in range(1, cols + 1):
            x = x0 + w * c // (cols + 1)
            draw.line((x, y0 + 2, x, y1 - 2), fill=shade(frame, 1.15), width=max(1, w // 34))
        rows = rng.choice([1, 1, 2])
        for r in range(1, rows + 1):
            y = y0 + h * r // (rows + 1)
            draw.line((x0 + 2, y, x1 - 2, y), fill=shade(frame, 1.05), width=1)
    else:
        if lit_night:
            warm = rng.random() < (0.8 if category in ("residential", "commercial") else 0.45)
            if warm:
                glow = rng.choice([(255, 206, 138), (255, 224, 170), (240, 184, 110)])
            else:
                glow = rng.choice([(180, 220, 245), (208, 232, 255), (160, 200, 226)])
            for i in range(h):
                t = i / max(1, h - 1)
                color = mix(shade(glow, 1.0), shade(glow, 0.55), t)
                draw.line((x0 + 2, y0 + i, x1 - 2, y0 + i), fill=color)
            # blinds / interior silhouettes
            if rng.random() < 0.35:
                draw.rectangle((x0 + 2, y0 + 2, x1 - 2, y0 + h // 3), fill=shade(glow, 0.35))
            if rng.random() < 0.2:
                draw.rectangle((x0 + w // 4, y0 + h // 2, x0 + w // 3, y0 + h // 2 + 3),
                               fill=(30, 30, 32))
        else:
            draw.rectangle((x0 + 2, y0 + 2, x1 - 2, y1 - 2), fill=(12, 18, 24))
            if rng.random() < 0.15:
                draw.rectangle((x0 + 2, y0 + 2, x1 - 2, y0 + h // 4), fill=(26, 34, 40))


def make_facade(category, rng):
    width, height = FACADE_W * SS, FACADE_H * SS
    wall = facade_palette(category, rng)
    day = Image.new("RGB", (width, height), wall)
    textured_base(day, rng, wall, step=10, amp=16)
    draw = ImageDraw.Draw(day)
    draw_wall_texture(draw, category, rng, wall, width, height)

    night = Image.new("RGB", (width, height), shade(wall, 0.16))
    nd = ImageDraw.Draw(night)
    draw_wall_texture(nd, category, rng, shade(wall, 0.16), width, height)

    if category == "financial":
        # glass curtain wall: wide bays, thin mullions, spandrel band
        bays = rng.choice([3, 4])
        cell = width / bays
        top = int(height * 0.14)
        bottom = int(height * 0.86)
        for b in range(bays):
            x0 = int(b * cell + cell * 0.06)
            x1 = int((b + 1) * cell - cell * 0.06)
            rect = (x0, top, x1, bottom)
            draw_window(draw, rect, rng, category, wall, night=False)
            lit = rng.random() < 0.72
            draw_window(nd, rect, rng, category, wall, lit_night=lit, night=True)
        # spandrels
        draw.rectangle((0, 0, width, int(height * 0.1)), fill=shade(wall, 0.7))
        draw.rectangle((0, int(height * 0.9), width, height), fill=shade(wall, 0.66))
        nd.rectangle((0, 0, width, int(height * 0.1)), fill=shade(wall, 0.1))
        nd.rectangle((0, int(height * 0.9), width, height), fill=shade(wall, 0.1))
    else:
        count = rng.choice([3, 4, 5, 6]) if category != "industrial" else rng.choice([3, 4, 5])
        cell = width / count
        top = int(height * rng.uniform(0.16, 0.24))
        bottom = int(height * rng.uniform(0.72, 0.84))
        for index in range(count):
            x0 = int(index * cell + cell * rng.uniform(0.16, 0.26))
            x1 = int((index + 1) * cell - cell * rng.uniform(0.16, 0.26))
            rect = (x0, top, x1, bottom)
            draw_window(draw, rect, rng, category, wall, night=False)
            lit = rng.random() < (0.5 if category == "residential" else 0.42)
            draw_window(nd, rect, rng, category, wall, lit_night=lit, night=True)
            # sills / lintels
            draw.rectangle((x0 - 4, bottom, x1 + 4, bottom + 4), fill=shade(wall, 1.16))
            draw.line((x0 - 2, top - 4, x1 + 2, top - 4), fill=shade(wall, 0.72), width=2)
            nd.rectangle((x0 - 4, bottom, x1 + 4, bottom + 4), fill=shade(wall, 0.12))

            if category == "residential" and rng.random() < 0.45:
                # balcony rail and planter
                rail_y = bottom + 8
                draw.rectangle((x0 - 10, rail_y, x1 + 10, rail_y + 3), fill=(50, 46, 42))
                for bx in range(x0 - 8, x1 + 8, 10):
                    draw.line((bx, rail_y, bx, rail_y + 16), fill=(60, 56, 52), width=1)
                draw.rectangle((x0 - 6, rail_y + 16, x1 + 6, rail_y + 22), fill=(70, 52, 38))
                nd.rectangle((x0 - 10, rail_y, x1 + 10, rail_y + 3), fill=(20, 18, 16))
            if category == "residential" and rng.random() < 0.3:
                # window AC unit
                ac_w = max(20, (x1 - x0) // 3)
                ax = x0 + (x1 - x0) // 2 - ac_w // 2
                ay = bottom + 6
                draw.rectangle((ax, ay, ax + ac_w, ay + 12), fill=(112, 114, 112))
                draw.rectangle((ax + 2, ay + 2, ax + ac_w - 2, ay + 10), fill=(86, 88, 88))
                for k in range(3):
                    draw.line((ax + 4, ay + 4 + k * 2, ax + ac_w - 4, ay + 4 + k * 2),
                              fill=(70, 72, 72), width=1)
            if category == "industrial":
                # steel frame rivets and a vent
                for ry in range(top, bottom, 18):
                    draw.ellipse((x0 - 4, ry, x0 - 1, ry + 3), fill=(72, 70, 66))
                    draw.ellipse((x1 + 1, ry, x1 + 4, ry + 3), fill=(72, 70, 66))
                if rng.random() < 0.5:
                    vx = x0 + (x1 - x0) // 2 - 8
                    draw.rectangle((vx, top - 10, vx + 16, top - 3), fill=(64, 68, 68))
                    for k in range(4):
                        draw.line((vx + 2, top - 8 + k * 2, vx + 14, top - 8 + k * 2),
                                  fill=(40, 42, 42), width=1)
            if category == "commercial" and rng.random() < 0.35:
                # cornice / lintel band
                draw.rectangle((x0 - 6, top - 12, x1 + 6, top - 5), fill=shade(wall, 1.2))
                nd.rectangle((x0 - 6, top - 12, x1 + 6, top - 5), fill=shade(wall, 0.14))

    # floor separation lines
    draw.line((0, 0, width, 0), fill=shade(wall, 0.55), width=2)
    draw.line((0, height - 1, width, height - 1), fill=shade(wall, 0.6), width=2)
    nd.line((0, 0, width, 0), fill=(8, 10, 12), width=2)
    nd.line((0, height - 1, width, height - 1), fill=(8, 10, 12), width=2)

    # gentle vertical grime / highlights
    for _ in range(22):
        x = rng.randrange(width)
        draw.line((x, rng.randrange(height // 2), x, height),
                  fill=shade(wall, rng.uniform(0.82, 1.08)), width=1)

    day = day.resize((FACADE_W, FACADE_H), Image.LANCZOS)
    night = night.resize((FACADE_W, FACADE_H), Image.LANCZOS)
    return day, night


# ---------------------------------------------------------------------------
# roofs
# ---------------------------------------------------------------------------
def roof_base(rng, size):
    kind = rng.choices(
        ["tar", "gravel", "concrete", "green", "pavers"],
        weights=[24, 24, 22, 16, 14],
    )[0]
    if kind == "tar":
        base = rng.choice([(62, 62, 60), (70, 68, 64), (56, 58, 58)])
    elif kind == "gravel":
        base = rng.choice([(128, 126, 118), (142, 138, 128), (116, 116, 110)])
    elif kind == "concrete":
        base = rng.choice([(150, 148, 142), (138, 138, 132), (164, 160, 152)])
    elif kind == "green":
        base = rng.choice([(78, 98, 62), (66, 88, 54), (92, 108, 70)])
    else:
        base = rng.choice([(132, 128, 120), (148, 142, 132)])
    img = Image.new("RGB", (size, size), base)
    textured_base(img, rng, base, step=14, amp=18 if kind != "concrete" else 8)
    draw = ImageDraw.Draw(img)
    if kind == "concrete":
        for y in range(0, size, 64):
            draw.line((0, y, size, y), fill=shade(base, 0.78), width=3)
        for x in range(0, size, 64):
            draw.line((x, 0, x, size), fill=shade(base, 0.82), width=2)
    elif kind == "pavers":
        for y in range(0, size, 24):
            draw.line((0, y, size, y), fill=shade(base, 0.76), width=2)
            offset = 0 if (y // 24) % 2 == 0 else 24
            for x in range(offset, size, 48):
                draw.line((x, y, x, y + 24), fill=shade(base, 0.8), width=2)
    elif kind == "green":
        for _ in range(180):
            x, y = rng.randrange(size), rng.randrange(size)
            r = rng.randint(2, 8)
            draw.ellipse((x - r, y - r, x + r, y + r),
                         fill=rng.choice([(72, 104, 56), (94, 124, 66), (58, 86, 46)]))
    elif kind == "gravel":
        for _ in range(450):
            x, y = rng.randrange(size), rng.randrange(size)
            r = rng.randint(0, 2)
            draw.ellipse((x - r, y - r, x + r, y + r),
                         fill=shade(base, rng.uniform(0.72, 1.22)))
    return img, kind


def free_rect(occupied, rect, margin=10, size=512):
    x0, y0, x1, y1 = rect
    if x0 < margin or y0 < margin or x1 > size - margin or y1 > size - margin:
        return False
    for ox0, oy0, ox1, oy1 in occupied:
        if not (x1 < ox0 or x0 > ox1 or y1 < oy0 or y0 > oy1):
            return False
    return True


def draw_ac(draw, x, y, w, h, rng):
    draw.rectangle((x + 3, y + 4, x + w + 3, y + h + 4), fill=(36, 38, 38))
    draw.rectangle((x, y, x + w, y + h), fill=rng.choice([(146, 148, 148), (122, 126, 128), (164, 164, 160)]))
    draw.rectangle((x + 2, y + 2, x + w - 2, y + h - 2), outline=(88, 90, 92), width=2)
    cx, cy = x + w // 2, y + h // 2
    r = min(w, h) // 3
    draw.ellipse((cx - r, cy - r, cx + r, cy + r), fill=(64, 68, 70), outline=(40, 42, 44))
    for a in range(0, 360, 60):
        ex = cx + int(math.cos(math.radians(a)) * r * 0.7)
        ey = cy + int(math.sin(math.radians(a)) * r * 0.7)
        draw.line((cx, cy, ex, ey), fill=(30, 32, 34), width=2)
    for k in range(3):
        draw.line((x + 3, y + h - 4 - k * 3, x + w - 3, y + h - 4 - k * 3),
                  fill=(70, 72, 74), width=1)


def draw_garden(draw, rng, x, y, w, h):
    draw.rectangle((x, y, x + w, y + h), fill=(74, 96, 58))
    for _ in range(16):
        px = rng.randint(x + 4, x + w - 4)
        py = rng.randint(y + 4, y + h - 4)
        r = rng.randint(3, 9)
        draw.ellipse((px - r, py - r, px + r, py + r),
                     fill=rng.choice([(82, 118, 58), (98, 132, 70), (62, 94, 48)]))
    # planter boxes
    for _ in range(2):
        px = rng.randint(x + 6, max(x + 6, x + w - 30))
        py = rng.randint(y + 6, max(y + 6, y + h - 18))
        draw.rectangle((px, py, px + 24, py + 12), fill=(104, 74, 48), outline=(72, 50, 34))
        for k in range(3):
            cx = px + 6 + k * 7
            draw.ellipse((cx - 4, py - 2, cx + 4, py + 7), fill=(90, 126, 62))
    # tree canopy
    tx, ty = x + w // 2, y + h // 2
    tr = min(w, h) // 4
    draw.ellipse((tx - tr + 4, ty - tr + 6, tx + tr + 4, ty + tr + 6), fill=(32, 44, 28))
    for _ in range(7):
        ox = rng.randint(-tr // 2, tr // 2)
        oy = rng.randint(-tr // 2, tr // 2)
        rr = rng.randint(tr // 2, tr)
        draw.ellipse((tx + ox - rr, ty + oy - rr, tx + ox + rr, ty + oy + rr),
                     fill=rng.choice([(58, 92, 44), (76, 110, 52), (44, 76, 38)]))


def draw_pool(draw, rng, x, y, w, h):
    # deck
    draw.rectangle((x - 10, y - 10, x + w + 10, y + h + 10), fill=(176, 172, 160))
    draw.rectangle((x - 6, y - 6, x + w + 6, y + h + 6), fill=(198, 194, 182))
    # water
    draw.rounded_rectangle((x, y, x + w, y + h), radius=10, fill=(54, 126, 164))
    for i in range(h):
        t = i / max(1, h - 1)
        color = mix((84, 164, 196), (38, 98, 138), t)
        draw.line((x + 4, y + i, x + w - 4, y + i), fill=color)
    for _ in range(18):
        px = rng.randint(x + 6, x + w - 6)
        py = rng.randint(y + 6, y + h - 6)
        draw.line((px, py, px + rng.randint(6, 22), py), fill=(160, 214, 228), width=1)
    # loungers
    for k in range(rng.choice([2, 3, 4])):
        lx = x - 24 + k * 20
        ly = y + h + 14
        draw.rectangle((lx, ly, lx + 14, ly + 5), fill=(226, 222, 210), outline=(150, 146, 136))


def draw_patio(draw, rng, x, y, w, h):
    draw.rectangle((x, y, x + w, y + h), fill=(150, 112, 74))
    for yy in range(y, y + h, 8):
        draw.line((x, yy, x + w, yy), fill=(122, 88, 58), width=2)
    # table + umbrella
    cx, cy = x + w // 2, y + h // 2
    r = min(w, h) // 5
    draw.ellipse((cx - r, cy - r, cx + r, cy + r), fill=rng.choice([(206, 72, 58), (58, 118, 96), (72, 92, 148)]))
    draw.ellipse((cx - r // 3, cy - r // 3, cx + r // 3, cy + r // 3), fill=(84, 58, 40))
    for a in range(0, 360, 90):
        ex = cx + int(math.cos(math.radians(a)) * (r + 10))
        ey = cy + int(math.sin(math.radians(a)) * (r + 10))
        draw.ellipse((ex - 5, ey - 5, ex + 5, ey + 5), fill=(96, 74, 56))


def draw_equipment(draw, rng, x, y, w, h):
    if rng.random() < 0.5:
        r = min(w, h) // 2
        cx, cy = x + w // 2, y + h // 2
        draw.ellipse((cx - r + 4, cy - r + 6, cx + r + 4, cy + r + 6), fill=(40, 42, 42))
        draw.ellipse((cx - r, cy - r, cx + r, cy + r), fill=(148, 150, 148), outline=(92, 94, 94), width=3)
        draw.ellipse((cx - r // 2, cy - r // 2, cx + r // 2, cy + r // 2), fill=(92, 94, 96))
        for a in range(0, 360, 45):
            ex = cx + int(math.cos(math.radians(a)) * r * 0.45)
            ey = cy + int(math.sin(math.radians(a)) * r * 0.45)
            draw.line((cx, cy, ex, ey), fill=(44, 46, 48), width=3)
    else:
        draw_ac(draw, x, y, w, h, rng)
        draw.rectangle((x + w + 8, y + 8, x + w + 20, y + h - 8), fill=(118, 120, 120))
        draw.rectangle((x + w + 10, y + 10, x + w + 18, y + h - 10), fill=(72, 74, 76))


def draw_solar(draw, rng, x, y, w, h):
    draw.rectangle((x, y, x + w, y + h), fill=(28, 42, 62))
    cols = max(2, w // 18)
    rows = max(2, h // 12)
    for c in range(cols):
        for r in range(rows):
            x0 = x + c * w // cols + 2
            y0 = y + r * h // rows + 2
            x1 = x + (c + 1) * w // cols - 2
            y1 = y + (r + 1) * h // rows - 2
            draw.rectangle((x0, y0, x1, y1), fill=(40, 66, 96), outline=(80, 110, 144))
    draw.line((x + w // 2, y, x + w // 2, y + h), fill=(120, 126, 130), width=2)


def draw_tank(draw, x, y, r):
    draw.ellipse((x - r + 5, y - r + 7, x + r + 5, y + r + 7), fill=(38, 40, 40))
    draw.ellipse((x - r, y - r, x + r, y + r), fill=(176, 172, 160), outline=(108, 106, 100), width=3)
    draw.ellipse((x - r // 2, y - r // 2, x + r // 2, y + r // 2), fill=(152, 150, 142))
    for a in range(0, 360, 90):
        ex = x + int(math.cos(math.radians(a)) * r * 0.8)
        ey = y + int(math.sin(math.radians(a)) * r * 0.8)
        draw.ellipse((ex - 4, ey - 4, ex + 4, ey + 4), fill=(72, 70, 66))


def make_roof(rng):
    size = ROOF_PX * SS
    img, kind = roof_base(rng, size)
    draw = ImageDraw.Draw(img)
    night = Image.new("RGB", (size, size), (10, 12, 14))
    nd = ImageDraw.Draw(night)

    occupied = []
    wants = []
    if kind == "green":
        wants.append("garden")
    wants.extend(rng.choices(
        ["garden", "pool", "equipment", "patio", "solar", "tank", "ac"],
        k=rng.randint(2, 5),
        weights=[20, 14, 24, 16, 12, 6, 8],
    ))
    for _ in range(rng.randint(1, 3)):
        wants.append(rng.choice(["ac", "ac", "equipment", "solar"]))

    for kind_name in wants:
        for _ in range(8):
            if kind_name == "garden":
                w = rng.randint(90, 190)
                h = rng.randint(70, 150)
            elif kind_name == "pool":
                w = rng.randint(130, 210)
                h = rng.randint(80, 130)
            elif kind_name == "patio":
                w = rng.randint(100, 170)
                h = rng.randint(90, 150)
            elif kind_name == "solar":
                w = rng.randint(80, 150)
                h = rng.randint(55, 95)
            elif kind_name == "tank":
                w = h = rng.randint(50, 90)
            elif kind_name == "equipment":
                w = rng.randint(45, 90)
                h = rng.randint(40, 80)
            else:
                w = rng.randint(30, 58)
                h = rng.randint(26, 46)
            x = rng.randint(16, size - w - 16)
            y = rng.randint(16, size - h - 16)
            rect = (x, y, x + w, y + h)
            if not free_rect(occupied, rect, margin=14, size=size):
                continue
            occupied.append(rect)
            if kind_name == "garden":
                draw_garden(draw, rng, x, y, w, h)
                # night: dark planting with a few warm garden lights
                nd.rectangle((x, y, x + w, y + h), fill=(18, 26, 18))
                for _ in range(10):
                    px = rng.randint(x + 4, x + w - 4)
                    py = rng.randint(y + 4, y + h - 4)
                    rr = rng.randint(2, 6)
                    nd.ellipse((px - rr, py - rr, px + rr, py + rr), fill=(28, 40, 26))
                for _ in range(3):
                    px = rng.randint(x + 4, x + w - 4)
                    py = rng.randint(y + 4, y + h - 4)
                    nd.ellipse((px - 2, py - 2, px + 2, py + 2), fill=(220, 180, 100))
            elif kind_name == "pool":
                draw_pool(draw, rng, x, y, w, h)
                nd.rectangle((x - 10, y - 10, x + w + 10, y + h + 10), fill=(28, 30, 32))
                nd.rounded_rectangle((x, y, x + w, y + h), radius=10, fill=(30, 84, 110))
                for i in range(h):
                    t = i / max(1, h - 1)
                    color = mix((48, 132, 160), (18, 62, 88), t)
                    nd.line((x + 4, y + i, x + w - 4, y + i), fill=color)
                for _ in range(10):
                    px = rng.randint(x + 6, x + w - 6)
                    py = rng.randint(y + 6, y + h - 6)
                    nd.line((px, py, px + rng.randint(6, 18), py), fill=(72, 160, 180), width=1)
            elif kind_name == "patio":
                draw_patio(draw, rng, x, y, w, h)
                nd.rectangle((x, y, x + w, y + h), fill=(30, 24, 20))
                for k in range(5):
                    px = x + 8 + k * (w - 16) // 4
                    nd.ellipse((px - 2, y + 6, px + 2, y + 10), fill=(240, 200, 120))
            elif kind_name == "solar":
                draw_solar(draw, rng, x, y, w, h)
                nd.rectangle((x, y, x + w, y + h), fill=(14, 20, 30))
            elif kind_name == "tank":
                draw_tank(draw, x + w // 2, y + h // 2, min(w, h) // 2)
                nd.ellipse((x + w // 2 - min(w, h) // 2, y + h // 2 - min(w, h) // 2,
                            x + w // 2 + min(w, h) // 2, y + h // 2 + min(w, h) // 2),
                           fill=(24, 26, 28))
            else:
                draw_ac(draw, x, y, w, h, rng)
                nd.rectangle((x, y, x + w, y + h), fill=(20, 22, 24))
                nd.ellipse((x + w // 2 - 2, y + h // 2 - 2, x + w // 2 + 2, y + h // 2 + 2),
                           fill=(180, 60, 40))
            break

    # a few skylights / vents on the open roof
    for _ in range(rng.randint(1, 4)):
        w = rng.randint(30, 70)
        h = rng.randint(22, 45)
        x = rng.randint(16, size - w - 16)
        y = rng.randint(16, size - h - 16)
        if not free_rect(occupied, (x, y, x + w, y + h), margin=10, size=size):
            continue
        occupied.append((x, y, x + w, y + h))
        draw.rectangle((x, y, x + w, y + h), fill=(96, 118, 130), outline=(60, 68, 72), width=3)
        draw.rectangle((x + 4, y + 4, x + w - 4, y + h - 4), fill=(120, 148, 160))
        nd.rectangle((x, y, x + w, y + h), fill=(18, 26, 32))

    img = img.resize((ROOF_PX, ROOF_PX), Image.LANCZOS)
    night = night.resize((ROOF_PX, ROOF_PX), Image.LANCZOS)
    return img, night


# ---------------------------------------------------------------------------
# build the atlases
# ---------------------------------------------------------------------------
def build_facades():
    day_atlas = Image.new("RGB", (FACADE_COLS * FACADE_W, FACADE_ROWS * FACADE_H), (20, 20, 22))
    night_atlas = Image.new("RGB", (FACADE_COLS * FACADE_W // 2, FACADE_ROWS * FACADE_H // 2), (10, 10, 12))
    tiles = []
    for index in range(FACADE_COUNT):
        category = CATEGORIES[index // PER_CATEGORY]
        rng = random.Random(0xFA_CADE_0000 + index * 7919)
        day, night = make_facade(category, rng)
        col, row = index % FACADE_COLS, index // FACADE_COLS
        day_atlas.paste(day, (col * FACADE_W, row * FACADE_H))
        night_small = night.resize((FACADE_W // 2, FACADE_H // 2), Image.LANCZOS)
        night_atlas.paste(night_small, (col * FACADE_W // 2, row * FACADE_H // 2))
        tiles.append({"id": index, "category": category, "col": col, "row": row})
        if (index + 1) % 40 == 0:
            print("facade", index + 1, "/", FACADE_COUNT)
    day_path = os.path.join(OUT_TEX, "facade_atlas_day.jpg")
    night_path = os.path.join(OUT_TEX, "facade_atlas_night.jpg")
    day_atlas.save(day_path, quality=90, optimize=True)
    night_atlas.save(night_path, quality=88, optimize=True)
    # contact sheet
    contact = day_atlas.resize((FACADE_COLS * 96, FACADE_ROWS * 24), Image.LANCZOS)
    contact.save(os.path.join(OUT_BUILD, "upper_facade_contact.png"))
    print("facade atlas", day_atlas.size, os.path.getsize(day_path), night_atlas.size, os.path.getsize(night_path))
    return tiles, day_path, night_path


def build_roofs():
    day_atlas = Image.new("RGB", (ROOF_COLS * ROOF_PX, ROOF_ROWS * ROOF_PX), (20, 20, 22))
    night_atlas = Image.new("RGB", (ROOF_COLS * ROOF_PX // 2, ROOF_ROWS * ROOF_PX // 2), (8, 8, 10))
    tiles = []
    for index in range(ROOF_COUNT):
        rng = random.Random(0x600D_0000 + index * 104729)
        day, night = make_roof(rng)
        col, row = index % ROOF_COLS, index // ROOF_COLS
        day_atlas.paste(day, (col * ROOF_PX, row * ROOF_PX))
        night_small = night.resize((ROOF_PX // 2, ROOF_PX // 2), Image.LANCZOS)
        night_atlas.paste(night_small, (col * ROOF_PX // 2, row * ROOF_PX // 2))
        tiles.append({"id": index, "col": col, "row": row})
        if (index + 1) % 20 == 0:
            print("roof", index + 1, "/", ROOF_COUNT)
    day_path = os.path.join(OUT_TEX, "roof_atlas_day.jpg")
    night_path = os.path.join(OUT_TEX, "roof_atlas_night.jpg")
    day_atlas.save(day_path, quality=90, optimize=True)
    night_atlas.save(night_path, quality=88, optimize=True)
    day_atlas.save(os.path.join(OUT_BUILD, "roof_contact.png"))
    print("roof atlas", day_atlas.size, os.path.getsize(day_path), night_atlas.size, os.path.getsize(night_path))
    return tiles, day_path, night_path


facade_tiles, facade_day, facade_night = build_facades()
roof_tiles, roof_day, roof_night = build_roofs()

catalog = {
    "facade": {
        "tile_w": FACADE_W,
        "tile_h": FACADE_H,
        "cols": FACADE_COLS,
        "rows": FACADE_ROWS,
        "count": FACADE_COUNT,
        "module_m": [12.0, 3.0],
        "atlas_day": "textures/upper/facade_atlas_day.jpg",
        "atlas_night": "textures/upper/facade_atlas_night.jpg",
        "categories": CATEGORY_RANGES,
        "tiles": facade_tiles,
    },
    "roof": {
        "tile_px": ROOF_PX,
        "cols": ROOF_COLS,
        "rows": ROOF_ROWS,
        "count": ROOF_COUNT,
        "module_m": 16.0,
        "atlas_day": "textures/upper/roof_atlas_day.jpg",
        "atlas_night": "textures/upper/roof_atlas_night.jpg",
        "tiles": roof_tiles,
    },
    "category_mapping": {
        "residential": "residential",
        "industrial": "industrial",
        "storefront": "commercial",
        "executive": "financial",
    },
}
with open(OUT_JSON, "w") as handle:
    json.dump(catalog, handle, separators=(",", ":"))
print("wrote", OUT_JSON, os.path.getsize(OUT_JSON))
