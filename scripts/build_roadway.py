#!/usr/bin/env python3
"""Build smooth asphalt, sidewalk and road-marking ribbons.

The centreline network is the cleaned COMPLETED set.  Roads use a constant
width per street run (instead of wobbling per vertex), mitered joins, free-run
trimming at building footprints, filled junction patches, and painted edge and
centre lines.  Geometry is exported as 500 m tiles with an AABB manifest.

Outputs
  viewer/assets/models/roadway_chunks/chunk_<x>_<z>.glb
  viewer/assets/models/roadway_manifest.json

Run with the roadway venv:
  /tmp/opencode/roadenv/bin/python scripts/build_roadway.py
"""
import json
import math
import os

import numpy as np
import trimesh
from PIL import Image

ROOT = "/home/orbit9/Work/games/Manhattan"
BUILD = os.path.join(ROOT, "build")
STATIONS = os.path.join(ROOT, "viewer", "assets", "models", "station_lines_completed.json")
MASK = os.path.join(BUILD, "building_mask.png")
OUT_DIR = os.path.join(ROOT, "viewer", "assets", "models", "roadway_chunks")
OUT_MANIFEST = os.path.join(ROOT, "viewer", "assets", "models", "roadway_manifest.json")
FEET = 3.280839895
LANE_M = 12.0 / FEET
SIDEWALK_M = 12.0 / FEET
MAX_SCAN_M = 34.0
CURB_H = 0.14
ASPHALT_Y = 0.05
SIDEWALK_Y = ASPHALT_Y + CURB_H
CHUNK_M = 500.0

meta = json.load(open(os.path.join(BUILD, "blocks.json")))
bounds = meta["bounds"]
minx, maxx = bounds["min_x"], bounds["max_x"]
miny, maxy = bounds["min_y"], bounds["max_y"]
mask = np.asarray(Image.open(MASK))[..., 3] > 128
H, W = mask.shape


def pixel(x, z):
    px = int(np.clip((x - minx) / (maxx - minx) * W, 0, W - 1))
    py = int(np.clip((maxy + z) / (maxy - miny) * H, 0, H - 1))
    return px, py


def inside(x, z):
    px, py = pixel(x, z)
    return bool(mask[py, px])


def clearance(x, z, nx, nz):
    d = 0.25
    while d <= MAX_SCAN_M:
        if inside(x + nx * d, z + nz * d):
            return d
        d += 0.25
    return MAX_SCAN_M


def segment_inside(a, b, step=0.75):
    length = math.hypot(b[0] - a[0], b[1] - a[1])
    count = max(2, int(length / step) + 1)
    for i in range(count + 1):
        t = i / count
        x = a[0] + (b[0] - a[0]) * t
        z = a[1] + (b[1] - a[1]) * t
        if inside(x, z):
            return True
    return False


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


def smooth(points, passes=1):
    if len(points) < 3:
        return points
    original = list(points)
    for _ in range(passes):
        out = [original[0]]
        for i in range(1, len(original) - 1):
            a = np.asarray(original[i - 1], dtype=float)
            b = np.asarray(original[i], dtype=float)
            c = np.asarray(original[i + 1], dtype=float)
            q = a * 0.25 + b * 0.5 + c * 0.25
            out.append((float(q[0]), float(q[1])))
        out.append(original[-1])
        original = rdp(out, 0.12)
    if any(segment_inside(a, b) for a, b in zip(original, original[1:])):
        return points
    return original


def split_free_runs(points):
    runs = []
    current = []
    for a, b in zip(points, points[1:]):
        if segment_inside(a, b):
            if len(current) >= 2:
                runs.append(current)
            current = []
        else:
            if not current:
                current = [a]
            current.append(b)
    if len(current) >= 2:
        runs.append(current)
    return runs


def polyline_normals(points):
    n = len(points)
    normals = []
    for i in range(n):
        if i == 0:
            dx = points[1][0] - points[0][0]
            dz = points[1][1] - points[0][1]
        elif i == n - 1:
            dx = points[-1][0] - points[-2][0]
            dz = points[-1][1] - points[-2][1]
        else:
            dx = points[i + 1][0] - points[i - 1][0]
            dz = points[i + 1][1] - points[i - 1][1]
        length = math.hypot(dx, dz)
        normals.append((1.0, 0.0) if length < 1e-9 else (-dz / length, dx / length))
    return normals


def miter_offsets(normals):
    out = []
    count = len(normals)
    for i, n in enumerate(normals):
        if i == 0 or i == count - 1:
            out.append((n[0], n[1], 1.0))
            continue
        prev = normals[i - 1]
        nx, nz = prev[0] + n[0], prev[1] + n[1]
        length = math.hypot(nx, nz)
        if length < 1e-6:
            out.append((n[0], n[1], 1.0))
            continue
        nx, nz = nx / length, nz / length
        dot = nx * prev[0] + nz * prev[1]
        scale = 1.0 / max(0.35, dot)
        out.append((nx, nz, min(2.5, scale)))
    return out


def add_quad(verts, faces, a, b, c, d):
    base = len(verts)
    verts.extend([a, b, c, d])
    faces.extend([(base, base + 1, base + 2), (base, base + 2, base + 3)])


def add_vertical_quad(verts, faces, a, b, h0, h1):
    base = len(verts)
    verts.extend([
        (a[0], h0, a[1]),
        (b[0], h0, b[1]),
        (b[0], h1, b[1]),
        (a[0], h1, a[1]),
    ])
    faces.extend([(base, base + 1, base + 2), (base, base + 2, base + 3)])


def add_disk(verts, faces, cx, cz, radius, y, segments=12):
    base = len(verts)
    verts.append((cx, y, cz))
    for i in range(segments):
        angle = 2.0 * math.pi * i / segments
        verts.append((cx + math.cos(angle) * radius, y, cz + math.sin(angle) * radius))
    for i in range(segments):
        a = base + 1 + i
        b = base + 1 + (i + 1) % segments
        faces.append((base, a, b))


def segment_intersection(a, b, c, d):
    ax, az = a
    bx, bz = b
    cx, cz = c
    dx, dz = d
    r_x, r_z = bx - ax, bz - az
    s_x, s_z = dx - cx, dz - cz
    denom = r_x * s_z - r_z * s_x
    if abs(denom) < 1e-9:
        return None
    t = ((cx - ax) * s_z - (cz - az) * s_x) / denom
    u = ((cx - ax) * r_z - (cz - az) * r_x) / denom
    if -0.001 <= t <= 1.001 and -0.001 <= u <= 1.001:
        return (ax + r_x * t, az + r_z * t)
    return None


def point_segment_distance(p, a, b):
    dx, dz = b[0] - a[0], b[1] - a[1]
    length_sq = dx * dx + dz * dz
    if length_sq < 1e-12:
        return math.hypot(p[0] - a[0], p[1] - a[1]), a
    t = ((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / length_sq
    t = max(0.0, min(1.0, t))
    q = (a[0] + dx * t, a[1] + dz * t)
    return math.hypot(p[0] - q[0], p[1] - q[1]), q


def percentile(values, fraction):
    if not values:
        return 0.0
    ordered = sorted(values)
    index = int(round((len(ordered) - 1) * fraction))
    return ordered[max(0, min(len(ordered) - 1, index))]


def make_mesh(vertices, faces, name, color, roughness):
    mesh = trimesh.Trimesh(
        vertices=np.asarray(vertices, dtype=np.float64),
        faces=np.asarray(faces, dtype=np.int64),
        process=False,
    )
    material = trimesh.visual.material.PBRMaterial(
        name=name,
        baseColorFactor=color,
        metallicFactor=0.0,
        roughnessFactor=roughness,
        doubleSided=True,
    )
    mesh.visual = trimesh.visual.TextureVisuals(
        uv=np.zeros((len(mesh.vertices), 2), dtype=np.float64),
        material=material,
    )
    return mesh


def face_buckets(vertices, faces):
    buckets = {}
    for f in faces:
        cx = sum(vertices[i][0] for i in f) / 3.0
        cz = sum(vertices[i][2] for i in f) / 3.0
        key = (int(math.floor(cx / CHUNK_M)), int(math.floor(cz / CHUNK_M)))
        buckets.setdefault(key, []).append(f)
    return buckets


def submesh(vertices, faces):
    used = sorted({i for f in faces for i in f})
    remap = {old: new for new, old in enumerate(used)}
    return [vertices[i] for i in used], [tuple(remap[i] for i in f) for f in faces]


def add_line_quad(verts, faces, a, b, width, y):
    dx = b[0] - a[0]
    dz = b[1] - a[1]
    length = math.hypot(dx, dz)
    if length < 1e-6:
        return
    nx, nz = -dz / length, dx / length
    w = width * 0.5
    add_quad(
        verts, faces,
        (a[0] - nx * w, y, a[1] - nz * w),
        (a[0] + nx * w, y, a[1] + nz * w),
        (b[0] + nx * w, y, b[1] + nz * w),
        (b[0] - nx * w, y, b[1] - nz * w),
    )


def point_at(section, offset):
    return (
        section["a"][0] + section["n"][0] * offset,
        section["a"][1] + section["n"][1] * offset,
    )


stations = json.load(open(STATIONS))["lines"]
asphalt_v, asphalt_f = [], []
walk_v, walk_f = [], []
mark_v, mark_f = [], []
mark_y_v, mark_y_f = [], []
runs = []
built_lines = 0
built_runs = 0
dropped_runs = 0
dropped_short = 0
dropped_blocked = 0

MAX_HALF_WIDTH = 15.0
MIN_ROAD_HALF = LANE_M * 0.5

for line_index, line in enumerate(stations):
    points = [(float(p[0]), float(p[1])) for p in line["points"]]
    points = [p for i, p in enumerate(points) if i == 0 or math.dist(p, points[i - 1]) > 0.05]
    points = rdp(points, 0.5)
    points = smooth(points, passes=1)
    free_runs = split_free_runs(points)
    for run in free_runs:
        run = rdp(run, 0.6)
        length = sum(math.dist(run[i], run[i + 1]) for i in range(len(run) - 1))
        if len(run) < 2 or length < 4.0:
            dropped_runs += 1
            dropped_short += 1
            continue
        normals = polyline_normals(run)
        miter = miter_offsets(normals)
        lefts, rights = [], []
        for (x, z), (nx, nz, _) in zip(run, miter):
            if inside(x, z):
                lefts.append(0.0)
                rights.append(0.0)
                continue
            left = min(MAX_HALF_WIDTH, clearance(x, z, -nx, -nz))
            right = min(MAX_HALF_WIDTH, clearance(x, z, nx, nz))
            lefts.append(left)
            rights.append(right)
        if max(left + right for left, right in zip(lefts, rights)) < 1.0:
            dropped_runs += 1
            dropped_blocked += 1
            continue

        y0 = ASPHALT_Y + (line_index % 97) * 0.00002
        y1 = SIDEWALK_Y + (line_index % 97) * 0.00002
        sections = []
        for (x, z), (nx, nz, _), left, right in zip(run, miter, lefts, rights):
            # The sidewalk occupies the outer 12 ft against each building.
            # If the scan hit the corridor cap there is no building on that
            # side: this is an intersection/open corner, so leave the asphalt
            # open instead of drawing a sidewalk across the junction.
            side_l = (
                0.0
                if left >= MAX_HALF_WIDTH - 0.05
                else min(SIDEWALK_M, max(0.0, left - MIN_ROAD_HALF))
            )
            side_r = (
                0.0
                if right >= MAX_HALF_WIDTH - 0.05
                else min(SIDEWALK_M, max(0.0, right - MIN_ROAD_HALF))
            )
            road_l_off = -(left - side_l)
            road_r_off = right - side_r
            road_w = road_r_off - road_l_off
            sections.append({
                "a": (x, z),
                "n": (nx, nz),
                "left": left,
                "right": right,
                "side_l": side_l,
                "side_r": side_r,
                "road_l_off": road_l_off,
                "road_r_off": road_r_off,
                "road_w": road_w,
                "ol": (x - nx * left, z - nz * left),
                "rl": (x + nx * road_l_off, z + nz * road_l_off),
                "rr": (x + nx * road_r_off, z + nz * road_r_off),
                "or": (x + nx * right, z + nz * right),
            })
        if min(section["road_w"] for section in sections) < 1.0:
            dropped_runs += 1
            dropped_blocked += 1
            continue

        widths = [section["road_w"] for section in sections]
        median_width = percentile(widths, 0.5)
        # Even lane count so the double-yellow line is a true centreline.
        lanes = int(2.0 * round(median_width / (2.0 * LANE_M)))
        lanes = max(2, min(6, lanes))
        for section in sections:
            section["lanes"] = lanes
            section["lane_w"] = section["road_w"] / lanes

        cumulative = [0.0]
        for j in range(len(run) - 1):
            cumulative.append(cumulative[-1] + math.dist(run[j], run[j + 1]))

        for i in range(len(sections) - 1):
            a = sections[i]
            b = sections[i + 1]
            add_quad(asphalt_v, asphalt_f,
                     (a["rl"][0], y0, a["rl"][1]),
                     (a["rr"][0], y0, a["rr"][1]),
                     (b["rr"][0], y0, b["rr"][1]),
                     (b["rl"][0], y0, b["rl"][1]))
            if a["side_r"] > 0.02 or b["side_r"] > 0.02:
                add_quad(walk_v, walk_f,
                         (a["rr"][0], y1, a["rr"][1]),
                         (a["or"][0], y1, a["or"][1]),
                         (b["or"][0], y1, b["or"][1]),
                         (b["rr"][0], y1, b["rr"][1]))
                add_vertical_quad(walk_v, walk_f, a["rr"], b["rr"], y0, y1)
            if a["side_l"] > 0.02 or b["side_l"] > 0.02:
                add_quad(walk_v, walk_f,
                         (a["ol"][0], y1, a["ol"][1]),
                         (a["rl"][0], y1, a["rl"][1]),
                         (b["rl"][0], y1, b["rl"][1]),
                         (b["ol"][0], y1, b["ol"][1]))
                add_vertical_quad(walk_v, walk_f, a["rl"], b["rl"], y0, y1)

            seg_len = math.hypot(b["a"][0] - a["a"][0], b["a"][1] - a["a"][1])
            distance = cumulative[i] + seg_len * 0.5

            # Solid white edge lines just inside the curbs.
            add_line_quad(
                mark_v, mark_f,
                point_at(a, a["road_l_off"] + 0.18),
                point_at(b, b["road_l_off"] + 0.18),
                0.09, y0 + 0.0045,
            )
            add_line_quad(
                mark_v, mark_f,
                point_at(a, a["road_r_off"] - 0.18),
                point_at(b, b["road_r_off"] - 0.18),
                0.09, y0 + 0.0045,
            )

            centre = lanes // 2
            for boundary in range(1, lanes):
                left_i = a["road_l_off"] + boundary * a["lane_w"]
                left_j = b["road_l_off"] + boundary * b["lane_w"]
                if boundary == centre:
                    # Double yellow down the middle of the traffic area.
                    for shift in (-0.12, 0.12):
                        add_line_quad(
                            mark_y_v, mark_y_f,
                            point_at(a, left_i + shift),
                            point_at(b, left_j + shift),
                            0.09, y0 + 0.0048,
                        )
                elif distance % 7.0 <= 3.2:
                    add_line_quad(
                        mark_v, mark_f,
                        point_at(a, left_i),
                        point_at(b, left_j),
                        0.09, y0 + 0.0045,
                    )

        max_half = max(section["road_w"] for section in sections) * 0.5
        runs.append((run, max_half))
        built_runs += 1
    built_lines += 1

print("lines processed", built_lines, "of", len(stations))
print("road runs", built_runs, "dropped", dropped_runs,
      "(short", dropped_short, "blocked", dropped_blocked, ")")

# ---------------------------------------------------------------------------
# junction patches
# ---------------------------------------------------------------------------
cell = 30.0
grid = {}
segments = []
for run_index, (run, road_half) in enumerate(runs):
    for a, b in zip(run, run[1:]):
        sid = len(segments)
        segments.append((run_index, a, b, road_half))
        x0, x1 = sorted((a[0], b[0]))
        z0, z1 = sorted((a[1], b[1]))
        pad = road_half + 2.0
        for gx in range(int((x0 - pad) // cell), int((x1 + pad) // cell) + 1):
            for gz in range(int((z0 - pad) // cell), int((z1 + pad) // cell) + 1):
                grid.setdefault((gx, gz), []).append(sid)

junctions = {}
for members in grid.values():
    for i in range(len(members)):
        for j in range(i + 1, len(members)):
            si = segments[members[i]]
            sj = segments[members[j]]
            if si[0] == sj[0]:
                continue
            p = segment_intersection(si[1], si[2], sj[1], sj[2])
            if p is None:
                continue
            key = (round(p[0] / 3.0), round(p[1] / 3.0))
            half = max(si[3], sj[3])
            if key not in junctions or half > junctions[key][2]:
                junctions[key] = (p[0], p[1], half)

endpoints = []
for run_index, (run, road_half) in enumerate(runs):
    endpoints.append((run_index, run[0], road_half))
    endpoints.append((run_index, run[-1], road_half))
for run_index, point, road_half in endpoints:
    gx, gz = int(point[0] // cell), int(point[1] // cell)
    best = None
    for dx in (-1, 0, 1):
        for dz in (-1, 0, 1):
            for sid in grid.get((gx + dx, gz + dz), []):
                other, a, b, half = segments[sid]
                if other == run_index:
                    continue
                distance, q = point_segment_distance(point, a, b)
                if distance <= max(1.6, road_half * 0.45):
                    if best is None or distance < best[0]:
                        best = (distance, q, half)
    if best is not None:
        _, q, half = best
        key = (round(q[0] / 3.0), round(q[1] / 3.0))
        if key not in junctions or half > junctions[key][2]:
            junctions[key] = (q[0], q[1], half)

for index, (key, (x, z, half)) in enumerate(junctions.items()):
    add_disk(asphalt_v, asphalt_f, x, z, min(11.5, half + 0.6),
             ASPHALT_Y + 0.0001 + (index % 31) * 0.000001)
print("junction patches", len(junctions))

# ---------------------------------------------------------------------------
# chunk export
# ---------------------------------------------------------------------------
os.makedirs(OUT_DIR, exist_ok=True)
for name in os.listdir(OUT_DIR):
    if name.endswith(".glb"):
        os.remove(os.path.join(OUT_DIR, name))

a_buckets = face_buckets(asphalt_v, asphalt_f)
w_buckets = face_buckets(walk_v, walk_f)
m_buckets = face_buckets(mark_v, mark_f)
y_buckets = face_buckets(mark_y_v, mark_y_f)
keys = sorted(set(a_buckets) | set(w_buckets) | set(m_buckets) | set(y_buckets))
manifest = []
for key in keys:
    scene = trimesh.Scene()
    bounds_min = [math.inf, math.inf, math.inf]
    bounds_max = [-math.inf, -math.inf, -math.inf]
    total_tris = 0
    for source_v, source_f, buckets, name, color, roughness in (
        (asphalt_v, asphalt_f, a_buckets, "Road_Asphalt", [0.03, 0.03, 0.035, 1.0], 0.92),
        (walk_v, walk_f, w_buckets, "Sidewalk_Concrete", [0.78, 0.78, 0.75, 1.0], 0.86),
        (mark_v, mark_f, m_buckets, "Road_Marking_White", [0.93, 0.93, 0.89, 1.0], 0.7),
        (mark_y_v, mark_y_f, y_buckets, "Road_Marking_Yellow", [0.95, 0.72, 0.08, 1.0], 0.7),
    ):
        if key not in buckets:
            continue
        vs, fs = submesh(source_v, buckets[key])
        mesh = make_mesh(vs, fs, name, color, roughness)
        scene.add_geometry(mesh, geom_name=name.lower())
        total_tris += len(fs)
        for f in buckets[key]:
            for i in f:
                v = source_v[i]
                for k in range(3):
                    bounds_min[k] = min(bounds_min[k], v[k])
                    bounds_max[k] = max(bounds_max[k], v[k])
    filename = f"chunk_{key[0]}_{key[1]}.glb"
    path = os.path.join(OUT_DIR, filename)
    scene.export(path, include_normals=True)
    manifest.append({
        "file": f"models/roadway_chunks/{filename}",
        "min": [round(v, 3) for v in bounds_min],
        "max": [round(v, 3) for v in bounds_max],
        "tris": total_tris,
    })

manifest.sort(key=lambda m: m["file"])
with open(OUT_MANIFEST, "w") as handle:
    json.dump({"chunk_m": CHUNK_M, "chunks": manifest}, handle, separators=(",", ":"))

print("asphalt triangles", len(asphalt_f))
print("sidewalk triangles", len(walk_f))
print("white marking triangles", len(mark_f))
print("yellow marking triangles", len(mark_y_f))
print("wrote", len(manifest), "chunks to", OUT_DIR)
print("wrote", OUT_MANIFEST, os.path.getsize(OUT_MANIFEST))
