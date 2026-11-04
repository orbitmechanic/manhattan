#!/usr/bin/env python3
"""Build two independent street-centreline sets.

A. imagery  — road-coloured pixels in the low-resolution ground texture,
              thinned on the free-space skeleton.
B. fronts   — simultaneous expansion fronts from every building: the locus
              where the expanded perimeter of one building meets another is
              taken as the road centreline.

Outputs
  build/station_lines_imagery.json
  build/station_lines_fronts.json
  viewer/assets/models/station_lines_imagery.json
  viewer/assets/models/station_lines_fronts.json
  build/station_lines_imagery_overlay.png
  build/station_lines_fronts_overlay.png

Stations/offsets/elevations are in feet; geometry is in metres.  Points are
written in viewer coordinates (x, z = -Blender-y), so they line up with the GLB.

Run with the geometry venv (needs scikit-image):
  /tmp/opencode/geoenv/bin/python scripts/build_station_lines.py
"""
import colorsys
import json
import math
import os
from collections import defaultdict

import numpy as np
from PIL import Image, ImageDraw
from scipy import ndimage as ndi
from skimage.morphology import closing, disk, remove_small_objects, skeletonize

ROOT = "/home/orbit9/Work/games/Manhattan"
BUILD = os.path.join(ROOT, "build")
SATELLITE = os.path.join(ROOT, "Textures/day/ground/satellite_manhattan_ground.jpg")
FEET = 3.280839895
TICK_FT = 100.0

meta = json.load(open(os.path.join(BUILD, "blocks.json")))
bounds = meta["bounds"]
ground = meta["ground_bounds"]
mask = np.asarray(Image.open(os.path.join(BUILD, "building_mask.png")))[..., 3] > 128
H, W = mask.shape
minx, maxx = bounds["min_x"], bounds["max_x"]
miny, maxy = bounds["min_y"], bounds["max_y"]

sat = Image.open(SATELLITE).convert("RGB")
sw, sh = sat.size


def world_from_pixel(px, py):
    return (
        minx + (px + 0.5) / W * (maxx - minx),
        maxy - (py + 0.5) / H * (maxy - miny),
    )


def image_uv(x, y):
    u = 0.1823 + (x - ground["min_x"]) / (ground["max_x"] - ground["min_x"]) * (1.0 - 0.1823)
    v = (y - ground["min_y"]) / (ground["max_y"] - ground["min_y"])
    return u * sw, (1.0 - v) * sh


def rdp(points, eps):
    if len(points) < 3:
        return points
    a = np.asarray(points[0], dtype=float)
    b = np.asarray(points[-1], dtype=float)
    ab = b - a
    ab2 = float(np.dot(ab, ab))
    best_i, best_d = 0, -1.0
    for i in range(1, len(points) - 1):
        p = np.asarray(points[i], dtype=float)
        if ab2 < 1e-12:
            d = float(np.linalg.norm(p - a))
        else:
            t = float(np.clip(np.dot(p - a, ab) / ab2, 0.0, 1.0))
            d = float(np.linalg.norm(p - (a + ab * t)))
        if d > best_d:
            best_i, best_d = i, d
    if best_d > eps:
        return rdp(points[: best_i + 1], eps)[:-1] + rdp(points[best_i:], eps)
    return [points[0], points[-1]]


def signed_turn(a, b, c):
    ab = np.asarray(b) - np.asarray(a)
    bc = np.asarray(c) - np.asarray(b)
    cross = ab[0] * bc[1] - ab[1] * bc[0]
    dot = float(np.dot(ab, bc))
    return math.degrees(math.atan2(cross, dot))


def circumradius(a, b, c):
    a = np.asarray(a, dtype=float)
    b = np.asarray(b, dtype=float)
    c = np.asarray(c, dtype=float)
    ab = float(np.linalg.norm(b - a))
    bc = float(np.linalg.norm(c - b))
    ca = float(np.linalg.norm(a - c))
    area2 = abs((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]))
    return 1e9 if area2 < 1e-9 else ab * bc * ca / (2.0 * area2)


def vectorize(skel, eps, min_len_m):
    ys, xs = np.nonzero(skel)
    pix = set(zip(ys.tolist(), xs.tolist()))
    if not pix:
        return []

    def neigh(p):
        y, x = p
        out = []
        for dy in (-1, 0, 1):
            for dx in (-1, 0, 1):
                if dy == 0 and dx == 0:
                    continue
                q = (y + dy, x + dx)
                if q in pix:
                    out.append(q)
        return out

    deg = {p: len(neigh(p)) for p in pix}
    seen = set()
    chains = []

    def trace(start, nxt):
        path = [start, nxt]
        seen.add(frozenset((start, nxt)))
        prev, cur = start, nxt
        dy, dx = nxt[0] - start[0], nxt[1] - start[1]
        norm = math.hypot(dy, dx)
        dir_y, dir_x = dy / norm, dx / norm
        while True:
            candidates = [
                q for q in neigh(cur)
                if q != prev and frozenset((cur, q)) not in seen
            ]
            if not candidates:
                break
            best = None
            best_dot = -2.0
            for q in candidates:
                vy, vx = q[0] - cur[0], q[1] - cur[1]
                norm = math.hypot(vy, vx)
                dot = (vy * dir_y + vx * dir_x) / norm
                if dot > best_dot:
                    best_dot, best = dot, q
            # Continue straight through intersections; do not turn onto a
            # cross street just because the skeleton has a branch there.
            if best is None or best_dot < 0.42:
                break
            seen.add(frozenset((cur, best)))
            path.append(best)
            vy, vx = best[0] - cur[0], best[1] - cur[1]
            norm = math.hypot(vy, vx)
            dir_y, dir_x = vy / norm, vx / norm
            prev, cur = cur, best
        return path

    # Start from true endpoints first so branches become separate streets.
    for p in sorted(q for q in pix if deg[q] == 1):
        for q in neigh(p):
            if frozenset((p, q)) not in seen:
                chains.append(trace(p, q))
    for p in pix:
        for q in neigh(p):
            if frozenset((p, q)) not in seen:
                chains.append(trace(p, q))

    lines = []
    for chain in chains:
        pts = [world_from_pixel(x, y) for y, x in chain]
        # Viewer coordinates: x, z = -Blender-y.
        pts = [(x, -y) for x, y in pts]
        pts = rdp(pts, eps)
        if len(pts) < 2:
            continue
        length = sum(math.dist(pts[i], pts[i + 1]) for i in range(len(pts) - 1))
        if length < min_len_m:
            continue
        lines.append(pts)
    return lines


def plan_geometry(points, stations_m):
    curves = []
    prev_dir = 0
    for i in range(1, len(points) - 1):
        turn = signed_turn(points[i - 1], points[i], points[i + 1])
        if abs(turn) < 1.5:
            continue
        radius = circumradius(points[i - 1], points[i], points[i + 1])
        if radius > 3_000.0:
            continue
        direction = "left" if turn > 0 else "right"
        start = (stations_m[i - 1] + stations_m[i]) * 0.5
        end = (stations_m[i] + stations_m[i + 1]) * 0.5
        curves.append(
            {
                "start_station_ft": round(start * FEET, 2),
                "end_station_ft": round(end * FEET, 2),
                "radius_ft": round(radius * FEET, 2),
                "turn_deg": round(turn, 3),
                "direction": direction,
                "reverse": prev_dir != 0 and direction != prev_dir,
            }
        )
        prev_dir = 1 if direction == "left" else -1
    return curves


def package(lines, source, prefix):
    lines.sort(key=lambda pts: (round(pts[0][0] / 50), round(pts[0][1] / 50), len(pts)))
    out_lines = []
    for index, pts in enumerate(lines):
        seg = [math.dist(pts[i], pts[i + 1]) for i in range(len(pts) - 1)]
        length_m = sum(seg)
        if length_m < 1e-6:
            continue
        stations_m = [0.0]
        for s in seg:
            stations_m.append(stations_m[-1] + s)
        length_ft = length_m * FEET
        ticks_ft = []
        s = 0.0
        while s <= length_ft + 1e-6:
            ticks_ft.append(round(s, 2))
            s += TICK_FT
        if ticks_ft[-1] < length_ft - 1e-6:
            ticks_ft.append(round(length_ft, 2))
        tick_points = []
        for tick in ticks_ft:
            target = tick / FEET
            j = 0
            while j + 1 < len(stations_m) and stations_m[j + 1] < target:
                j += 1
            if j + 1 >= len(stations_m):
                tick_points.append(pts[-1])
                continue
            span = max(stations_m[j + 1] - stations_m[j], 1e-9)
            t = (target - stations_m[j]) / span
            a = np.asarray(pts[j])
            b = np.asarray(pts[j + 1])
            q = a + (b - a) * t
            tick_points.append([round(float(q[0]), 3), round(float(q[1]), 3)])
        curves = plan_geometry(pts, stations_m)
        tangents = []
        cursor = 0.0
        for c in curves:
            if c["start_station_ft"] > cursor + 1e-6:
                tangents.append(
                    {"start_station_ft": round(cursor, 2), "end_station_ft": c["start_station_ft"]}
                )
            cursor = max(cursor, c["end_station_ft"])
        if length_ft > cursor + 1e-6:
            tangents.append(
                {"start_station_ft": round(cursor, 2), "end_station_ft": round(length_ft, 2)}
            )
        out_lines.append(
            {
                "id": f"{prefix}_{index + 1:04d}",
                "points": [[round(float(x), 3), round(float(y), 3)] for x, y in pts],
                "length_m": round(length_m, 3),
                "length_ft": round(length_ft, 2),
                "tick_interval_ft": TICK_FT,
                "ticks_ft": ticks_ft,
                "tick_points": tick_points,
                "plan_curves": curves,
                "plan_tangents": tangents,
                "station_start_ft": 0.0,
                "station_end_ft": round(length_ft, 2),
                "vertical": {
                    "tangent_points_ft": [0.0, round(length_ft, 2)],
                    "grades": [0.0],
                    "curves": [],
                    "start_elevation_ft": 0.0,
                    "end_elevation_ft": 0.0,
                },
            }
        )
    for i, line in enumerate(out_lines):
        line["id"] = f"{prefix}_{i + 1:04d}"
    return {
        "version": 2,
        "source": source,
        "units": {"geometry": "meters", "station": "feet", "offset": "feet", "elevation": "feet"},
        "tick_interval_ft": TICK_FT,
        "line_count": len(out_lines),
        "total_length_ft": round(sum(l["length_ft"] for l in out_lines), 2),
        "lines": out_lines,
    }


def overlay(lines, path):
    out = sat.copy()
    draw = ImageDraw.Draw(out)
    for i, line in enumerate(lines):
        hue = (i * 0.618) % 1.0
        col = (255, 120, 20) if path.endswith("fronts_overlay.png") else (0, 255, 120)
        pts = [(x, -z) for x, z in line]
        draw.line([coord for p in pts for coord in image_uv(*p)], fill=col, width=2)
    out.save(path)


# ---------------------------------------------------------------------------
# A. imagery: free-space skeleton whose pixels are road-coloured
# ---------------------------------------------------------------------------
crop = sat.crop((int(0.1823 * sw), 0, sw, sh)).resize((W, H), Image.BILINEAR)
rgb = np.asarray(crop, dtype=np.float32) / 255.0
mx = rgb.max(axis=2)
mn = rgb.min(axis=2)
sat_ = np.where(mx > 0, (mx - mn) / np.maximum(mx, 1e-6), 0.0)
dist_building = ndi.distance_transform_edt(~mask) * 2.0
road = (sat_ < 0.16) & (mx > 0.22) & (mx < 0.82)
road = ndi.binary_dilation(road, iterations=1)
free_skel = skeletonize(~mask)
imagery_pixels = free_skel & road & (dist_building >= 4.0) & (dist_building <= 30.0)
imagery_pixels = remove_small_objects(imagery_pixels, max_size=30, connectivity=2)
print("imagery skeleton pixels:", int(imagery_pixels.sum()))
imagery_lines = vectorize(imagery_pixels, 3.0, 30.0)
imagery = package(imagery_lines, "imagery", "IMG")
overlay(imagery_lines, os.path.join(BUILD, "station_lines_imagery_overlay.png"))
print(
    f"imagery lines: {imagery['line_count']}  "
    f"{imagery['total_length_ft']/5280:.1f} miles"
)

# ---------------------------------------------------------------------------
# B. building expansion fronts: Voronoi boundary of individual buildings
# ---------------------------------------------------------------------------
components, n_components = ndi.label(mask, structure=np.ones((3, 3)))
free = ~mask
dist, nearest = ndi.distance_transform_edt(free, return_indices=True)
labels = np.zeros((H, W), dtype=np.int32)
labels[free] = components[nearest[0][free], nearest[1][free]]
boundary = np.zeros((H, W), dtype=bool)
for dy, dx in ((0, 1), (1, 0), (1, 1), (1, -1)):
    shifted = np.roll(labels, (-dy, -dx), axis=(0, 1))
    valid = np.ones((H, W), dtype=bool)
    if dy:
        valid[-dy:, :] = False
    if dx > 0:
        valid[:, -dx:] = False
    if dx < 0:
        valid[:, :-dx] = False
    boundary |= (labels != shifted) & (labels > 0) & (shifted > 0) & valid & free
# The expansion front is useful while the two fronts are within street width.
boundary &= (dist * 2.0 >= 1.0) & (dist * 2.0 <= 35.0)
fronts_pixels = skeletonize(boundary)
print("fronts skeleton pixels:", int(fronts_pixels.sum()))
fronts_lines = vectorize(fronts_pixels, 2.0, 25.0)
fronts = package(fronts_lines, "building_fronts", "FRT")
overlay(fronts_lines, os.path.join(BUILD, "station_lines_fronts_overlay.png"))
print(
    f"fronts lines: {fronts['line_count']}  "
    f"{fronts['total_length_ft']/5280:.1f} miles"
)

# ---------------------------------------------------------------------------
for name, data in (("imagery", imagery), ("fronts", fronts)):
    for path in (
        os.path.join(BUILD, f"station_lines_{name}.json"),
        os.path.join(ROOT, "viewer", "assets", "models", f"station_lines_{name}.json"),
    ):
        with open(path, "w") as f:
            json.dump(data, f, separators=(",", ":"))
        print("wrote", path)
