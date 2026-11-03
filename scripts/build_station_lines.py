#!/usr/bin/env python3
"""Build street centrelines and stationing data from block geometry + satellite hints.

Inputs
  build/ground_cells.npz           Voronoi ground tiles (boundaries are mid-street)
  build/blocks_labels.npy          building-block labels, used for footprint distance
  build/blocks.json                ground/building bounds
  Textures/day/ground/satellite_manhattan_ground.jpg

Outputs
  build/station_lines.json         centrelines, plan curves, vertical tangents
  viewer/assets/models/station_lines.json
  build/station_lines_overlay.png  validation overlay on the satellite

Method
  1. Keep Voronoi edges shared by two different blocks (these are mid-street).
  2. Sample the low-res satellite at several points on each edge and keep only
     edges whose samples look like grey road surface; reject water/open ground.
  3. Reject edges far from any building footprint (removes basin/water spokes).
  4. Trace the surviving edges through degree-2 nodes, then join collinear
     chain ends within 12 m across intersections.
  5. Simplify each centreline and derive plan curves (tangent/arc/reverse) and
     a flat vertical tangent profile.  Stations are stored in feet; geometry is
     in metres.

Run with the geometry venv:
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

ROOT = "/home/orbit9/Work/games/Manhattan"
BUILD = os.path.join(ROOT, "build")
SATELLITE = os.path.join(ROOT, "Textures/day/ground/satellite_manhattan_ground.jpg")
OUT_JSON = os.path.join(BUILD, "station_lines.json")
OUT_VIEWER = os.path.join(ROOT, "viewer", "assets", "models", "station_lines.json")
OUT_OVERLAY = os.path.join(BUILD, "station_lines_overlay.png")
FEET = 3.280839895
TICK_FT = 100.0
ROAD_DIST_M = 35.0
JOIN_DIST_M = 12.0

meta = json.load(open(os.path.join(BUILD, "blocks.json")))
gb = meta["ground_bounds"]
cells = np.load(os.path.join(BUILD, "ground_cells.npz"))
verts = cells["vertices"]
offsets = cells["offsets"]
block_ids = cells["block_ids"]
labels = np.load(os.path.join(BUILD, "blocks_labels.npy"))
bh, bw = labels.shape
minx, maxx = meta["bounds"]["min_x"], meta["bounds"]["max_x"]
miny, maxy = meta["bounds"]["min_y"], meta["bounds"]["max_y"]

sat = Image.open(SATELLITE).convert("RGB")
sat_arr = np.asarray(sat, dtype=np.float32) / 255.0
sh, sw = sat_arr.shape[:2]


def ground_uv(x, y):
    """The ground mesh uses u=0.1823..1 of the satellite."""
    u = 0.1823 + (x - gb["min_x"]) / (gb["max_x"] - gb["min_x"]) * (1.0 - 0.1823)
    v = (y - gb["min_y"]) / (gb["max_y"] - gb["min_y"])
    return u * sw, (1.0 - v) * sh


def road_like(x, y):
    px, py = ground_uv(x, y)
    px = int(np.clip(px, 0, sw - 1))
    py = int(np.clip(py, 0, sh - 1))
    r, g, b = sat_arr[py, px]
    mx = max(r, g, b)
    mn = min(r, g, b)
    sat_ = 0.0 if mx == 0 else (mx - mn) / mx
    return sat_ < 0.18 and 0.22 < mx < 0.85


def building_distance(x, y):
    px = int(np.clip((x - minx) / (maxx - minx) * bw, 0, bw - 1))
    py = int(np.clip((maxy - y) / (maxy - miny) * bh, 0, bh - 1))
    return float(dist[py, px])


# 2 m per label pixel
dist = ndi.distance_transform_edt(labels == 0) * 2.0


def nk(p):
    return (round(float(p[0]), 1), round(float(p[1]), 1))


# ---- candidate Voronoi street edges ---------------------------------------
edge_map = defaultdict(list)
for i in range(len(block_ids)):
    poly = verts[offsets[i]:offsets[i + 1]]
    for a, b in zip(poly, np.roll(poly, -1, axis=0)):
        ka, kb = nk(a), nk(b)
        if ka == kb:
            continue
        key = (min(ka, kb), max(ka, kb))
        edge_map[key].append(int(block_ids[i]))

edges = []
for key, ids in edge_map.items():
    if len(set(ids)) < 2:
        continue
    a = np.asarray(key[0], dtype=np.float64)
    b = np.asarray(key[1], dtype=np.float64)
    length = float(np.linalg.norm(b - a))
    if length < 1.0:
        continue
    samples = [road_like(*(a + (b - a) * t)) for t in np.linspace(0.15, 0.85, 7)]
    if sum(samples) < 4:
        continue
    if building_distance(*((a + b) * 0.5)) > ROAD_DIST_M:
        continue
    edges.append((a, b))
print(f"street edges: {len(edges)}  length {sum(np.linalg.norm(b-a) for a,b in edges)/1000:.1f} km")

# ---- trace through degree-2 nodes -----------------------------------------
adj = defaultdict(list)
for ei, (a, b) in enumerate(edges):
    adj[nk(a)].append(ei)
    adj[nk(b)].append(ei)


def orient(ei, node):
    a, b = edges[ei]
    return [tuple(a), tuple(b)] if nk(a) == node else [tuple(b), tuple(a)]


used = [False] * len(edges)
chains = []
for start in range(len(edges)):
    if used[start]:
        continue
    used[start] = True
    a, b = edges[start]
    pts = [tuple(a), tuple(b)]
    end = nk(b)
    while len(adj[end]) == 2:
        cand = [e for e in adj[end] if not used[e]]
        if len(cand) != 1:
            break
        ne = cand[0]
        used[ne] = True
        o = orient(ne, end)
        pts.extend(o[1:])
        end = nk(o[-1])
    end = nk(a)
    rev = []
    while len(adj[end]) == 2:
        cand = [e for e in adj[end] if not used[e]]
        if len(cand) != 1:
            break
        ne = cand[0]
        used[ne] = True
        o = orient(ne, end)
        rev.append(o[-1])
        end = nk(o[-1])
    if rev:
        pts = list(reversed(rev)) + pts
    chains.append(pts)

# ---- join chain ends across intersections --------------------------------
ends = []
for ci, chain in enumerate(chains):
    p0 = np.asarray(chain[0])
    p1 = np.asarray(chain[1])
    pn = np.asarray(chain[-1])
    pp = np.asarray(chain[-2])
    ends.append([ci, 0, p0, p1 - p0])
    ends.append([ci, 1, pn, pp - pn])

candidates = []
for i in range(len(ends)):
    for j in range(i + 1, len(ends)):
        ca, sa, pa, da = ends[i]
        cb, sb, pb, db = ends[j]
        if ca == cb:
            continue
        d = float(np.linalg.norm(pa - pb))
        if d > JOIN_DIST_M:
            continue
        na = float(np.linalg.norm(da))
        nb = float(np.linalg.norm(db))
        if na < 1e-6 or nb < 1e-6:
            continue
        dot = float(np.dot(da, db) / (na * nb))
        if dot < -0.82:
            candidates.append((d + 5.0 * (1.0 + dot), i, j))
candidates.sort()

pair = {}
used_end = set()
for _score, i, j in candidates:
    if i in used_end or j in used_end:
        continue
    ca, sa = ends[i][0], ends[i][1]
    cb, sb = ends[j][0], ends[j][1]
    pair[(ca, sa)] = (cb, sb)
    pair[(cb, sb)] = (ca, sa)
    used_end.add(i)
    used_end.add(j)

seen = set()
merged = []
for ci in range(len(chains)):
    if ci in seen:
        continue
    seen.add(ci)
    pts = list(chains[ci])
    cur, side = ci, 1
    while (cur, side) in pair:
        nxt, ns = pair[(cur, side)]
        if nxt in seen:
            break
        seen.add(nxt)
        c = chains[nxt]
        if ns == 0:
            pts += c[1:]
            cur, side = nxt, 1
        else:
            pts += list(reversed(c))[1:]
            cur, side = nxt, 0
    cur, side = ci, 0
    front = []
    while (cur, side) in pair:
        nxt, ns = pair[(cur, side)]
        if nxt in seen:
            break
        seen.add(nxt)
        c = chains[nxt]
        if ns == 1:
            front = list(reversed(c))[:-1] + front
            cur, side = nxt, 0
        else:
            front = c[:-1] + front
            cur, side = nxt, 1
    merged.append(front + pts)
print(f"merged centreline candidates: {len(merged)}")


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
        left = rdp(points[:best_i + 1], eps)
        right = rdp(points[best_i:], eps)
        return left[:-1] + right
    return [points[0], points[-1]]


def dedupe(points):
    out = []
    for p in points:
        if not out or np.linalg.norm(np.asarray(p) - np.asarray(out[-1])) > 1e-4:
            out.append(p)
    return out


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
    if area2 < 1e-9:
        return 1e9
    return ab * bc * ca / (2.0 * area2)


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
        reverse = prev_dir != 0 and direction != prev_dir
        curves.append(
            {
                "start_station_ft": round(start * FEET, 2),
                "end_station_ft": round(end * FEET, 2),
                "radius_ft": round(radius * FEET, 2),
                "turn_deg": round(turn, 3),
                "direction": direction,
                "reverse": reverse,
            }
        )
        prev_dir = 1 if direction == "left" else -1
    return curves


lines = []
for pts in merged:
    pts = dedupe(pts)
    if len(pts) < 2:
        continue
    pts = rdp(pts, 0.8)
    pts = dedupe(pts)
    if len(pts) < 2:
        continue
    seg = [float(np.linalg.norm(np.asarray(pts[i + 1]) - np.asarray(pts[i]))) for i in range(len(pts) - 1)]
    length_m = sum(seg)
    if length_m < 8.0:
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
        tangents.append({"start_station_ft": round(cursor, 2), "end_station_ft": round(length_ft, 2)})
    lines.append(
        {
            "id": "",
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

lines.sort(key=lambda l: (round(l["points"][0][0] / 50), round(l["points"][0][1] / 50), l["length_ft"]))
for i, line in enumerate(lines):
    line["id"] = f"STG_{i + 1:04d}"
print(f"station lines: {len(lines)}  length {sum(l['length_ft'] for l in lines)/5280:.1f} miles")
print(f"ticks: {sum(len(l['ticks_ft']) for l in lines)}")
print(f"plan curves: {sum(len(l['plan_curves']) for l in lines)}")

out = {
    "version": 1,
    "units": {"geometry": "meters", "station": "feet", "offset": "feet", "elevation": "feet"},
    "tick_interval_ft": TICK_FT,
    "line_count": len(lines),
    "total_length_ft": round(sum(l["length_ft"] for l in lines), 2),
    "lines": lines,
}
with open(OUT_JSON, "w") as f:
    json.dump(out, f, separators=(",", ":"))
os.makedirs(os.path.dirname(OUT_VIEWER), exist_ok=True)
with open(OUT_VIEWER, "w") as f:
    json.dump(out, f, separators=(",", ":"))

# overlay
over = sat.copy()
draw = ImageDraw.Draw(over)
for i, line in enumerate(lines):
    hue = (i * 0.618) % 1.0
    col = tuple(int(v * 255) for v in colorsys.hsv_to_rgb(hue, 0.9, 1.0))
    p = [ground_uv(*pt) for pt in line["points"]]
    draw.line([coord for xy in p for coord in xy], fill=col, width=2)
over.save(OUT_OVERLAY)
print("wrote", OUT_JSON, OUT_VIEWER, OUT_OVERLAY)
