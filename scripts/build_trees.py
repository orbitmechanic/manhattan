#!/usr/bin/env python3
"""Pick random park/open-ground tree positions.

Uses the Voronoi ground cells belonging to the 19 park/open-space blocks and
rejects points inside buildings or within 9 m of a street centreline.  This
keeps trees on open ground, never on roads or sidewalks.

Output:
  viewer/assets/models/trees.json
  build/trees_overlay.png
"""
import json
import math
import os
import random

import numpy as np
from PIL import Image, ImageDraw
from scipy.spatial import cKDTree

ROOT = "/home/orbit9/Work/games/Manhattan"
BUILD = os.path.join(ROOT, "build")
OUT = os.path.join(ROOT, "viewer", "assets", "models", "trees.json")
OVERLAY = os.path.join(BUILD, "trees_overlay.png")
MAX_TREES = 3000
TREE_AREA_M2 = 340.0
ROAD_CLEAR_M = 9.0

meta = json.load(open(os.path.join(BUILD, "blocks.json")))
ground = np.load(os.path.join(BUILD, "ground_cells.npz"))
vertices = ground["vertices"]
offsets = ground["offsets"]
block_ids = ground["block_ids"]
building = np.asarray(Image.open(os.path.join(BUILD, "building_mask.png")))[..., 3] > 128
H, W = building.shape
minx, maxx = meta["bounds"]["min_x"], meta["bounds"]["max_x"]
miny, maxy = meta["bounds"]["min_y"], meta["bounds"]["max_y"]

park_ids = {block["index"] for block in meta["blocks"] if block["type"] == "park"}
park_cells = []
for i, block_id in enumerate(block_ids):
    if int(block_id) not in park_ids:
        continue
    poly = vertices[offsets[i]:offsets[i + 1]]
    if len(poly) < 3:
        continue
    area = abs(0.5 * sum(
        poly[j][0] * poly[(j + 1) % len(poly)][1]
        - poly[(j + 1) % len(poly)][0] * poly[j][1]
        for j in range(len(poly))
    ))
    if area < 20.0:
        continue
    x0, y0 = poly.min(axis=0)
    x1, y1 = poly.max(axis=0)
    park_cells.append((area, x0, x1, y0, y1, poly))
print("park cells", len(park_cells), "area m2", round(sum(c[0] for c in park_cells)))

# Road-centreline samples for the tree/street clearance test.
roads = json.load(open(os.path.join(ROOT, "viewer/assets/models/station_lines_completed.json")))["lines"]
road_points = []
for line in roads:
    pts = line["points"]
    for a, b in zip(pts, pts[1:]):
        ax, az = a
        bx, bz = b
        length = math.hypot(bx - ax, bz - az)
        steps = max(1, int(length / 2.0))
        for k in range(steps + 1):
            t = k / steps
            road_points.append((ax + (bx - ax) * t, az + (bz - az) * t))
road_tree = cKDTree(np.asarray(road_points))
print("road samples", len(road_points))


def point_in_poly(x, y, poly):
    inside = False
    n = len(poly)
    j = n - 1
    for i in range(n):
        xi, yi = poly[i]
        xj, yj = poly[j]
        if ((yi > y) != (yj > y)) and (x < (xj - xi) * (y - yi) / (yj - yi) + xi):
            inside = not inside
        j = i
    return inside


def inside_building(x, y):
    px = int((x - minx) / (maxx - minx) * W)
    py = int((maxy - y) / (maxy - miny) * H)
    px = max(0, min(W - 1, px))
    py = max(0, min(H - 1, py))
    return bool(building[py, px])


rng = random.Random(0x7EE5_2026)
total_area = sum(c[0] for c in park_cells)
count = int(min(MAX_TREES, max(300, total_area / TREE_AREA_M2)))
print("target trees", count)

trees = []
attempts = 0
while len(trees) < count and attempts < count * 12:
    attempts += 1
    r = rng.random() * total_area
    for area, x0, x1, y0, y1, poly in park_cells:
        r -= area
        if r > 0:
            continue
        x = rng.uniform(x0, x1)
        y = rng.uniform(y0, y1)
        if not point_in_poly(x, y, poly):
            break
        if inside_building(x, y):
            break
        # Viewer coordinates for the road KD-tree.
        if road_tree.query((x, -y))[0] < ROAD_CLEAR_M:
            break
        trees.append({
            "x": round(x, 2),
            "z": round(-y, 2),
            "scale": round(rng.uniform(0.72, 1.38), 3),
            "variant": rng.randrange(3),
            "rotation": round(rng.uniform(0.0, 6.283185), 3),
        })
        break

with open(OUT, "w") as handle:
    json.dump({"count": len(trees), "trees": trees}, handle, separators=(",", ":"))
print("wrote", OUT, os.path.getsize(OUT), "trees", len(trees))

sat = Image.open(os.path.join(ROOT, "Textures/day/ground/satellite_manhattan_ground.jpg")).convert("RGB")
sw, sh = sat.size
draw = ImageDraw.Draw(sat)
gb = meta["ground_bounds"]
for tree in trees:
    x, z = tree["x"], tree["z"]
    y = -z
    u = 0.1823 + (x - gb["min_x"]) / (gb["max_x"] - gb["min_x"]) * (1.0 - 0.1823)
    v = (y - gb["min_y"]) / (gb["max_y"] - gb["min_y"])
    if 0 <= u <= 1 and 0 <= v <= 1:
        xp, yp = u * sw, (1.0 - v) * sh
        draw.ellipse((xp - 2, yp - 2, xp + 2, yp + 2), fill=(40, 255, 80))
sat.save(OVERLAY)
print("wrote", OVERLAY)
