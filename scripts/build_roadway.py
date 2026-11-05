#!/usr/bin/env python3
"""Build asphalt and sidewalk ribbons from the completed street baselines.

Uses 12-ft lanes and 12-ft sidewalks.  Lane count is reduced where the
building corridor is too narrow, and widths are clamped by the building mask so
pavement never crosses a footprint.

Output: viewer/assets/models/roadway.glb

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
OUT = os.path.join(ROOT, "viewer", "assets", "models", "roadway.glb")
FEET = 3.280839895
LANE_M = 12.0 / FEET
SIDEWALK_M = 12.0 / FEET
MAX_SCAN_M = 30.0
CURB_H = 0.14
ASPHALT_Y = 0.05
SIDEWALK_Y = ASPHALT_Y + 0.14

meta = json.load(open(os.path.join(BUILD, "blocks.json")))
b = meta["bounds"]
minx, maxx = b["min_x"], b["max_x"]
miny, maxy = b["min_y"], b["max_y"]
mask = np.asarray(Image.open(MASK))[..., 3] > 128
H, W = mask.shape


def pixel(x, z):
    px = int(np.clip((x - minx) / (maxx - minx) * W, 0, W - 1))
    # Viewer z = -Blender y.
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
        if length < 1e-9:
            normals.append((1.0, 0.0))
        else:
            # Right normal in the viewer's x/z plane.
            normals.append((-dz / length, dx / length))
    return normals


def add_quad(verts, faces, a, b, c, d):
    base = len(verts)
    verts.extend([a, b, c, d])
    faces.extend([(base, base + 1, base + 2), (base, base + 2, base + 3)])


def add_vertical_quad(verts, faces, a, b, h0, h1):
    # a,b are plan points; create a vertical curb face between heights.
    base = len(verts)
    verts.extend([
        (a[0], h0, a[1]),
        (b[0], h0, b[1]),
        (b[0], h1, b[1]),
        (a[0], h1, a[1]),
    ])
    faces.extend([(base, base + 1, base + 2), (base, base + 2, base + 3)])


stations = json.load(open(STATIONS))["lines"]
asphalt_v = []
asphalt_f = []
walk_v = []
walk_f = []
built = 0
for li, line in enumerate(stations):
    points = [(float(p[0]), float(p[1])) for p in line["points"]]
    if len(points) < 2:
        continue
    # Skip a line whose centre falls inside a building.
    if any(inside(x, z) for x, z in points):
        continue
    normals = polyline_normals(points)
    widths = []
    for (x, z), (nx, nz) in zip(points, normals):
        if inside(x, z):
            widths.append(None)
            continue
        left = clearance(x, z, -nx, -nz)
        right = clearance(x, z, nx, nz)
        avail = min(left, right)
        lanes = int(math.floor(avail / LANE_M))
        if lanes < 1:
            widths.append(None)
            continue
        lanes = min(lanes, 4)
        road_half = lanes * LANE_M * 0.5
        side_l = min(SIDEWALK_M, max(0.0, left - road_half))
        side_r = min(SIDEWALK_M, max(0.0, right - road_half))
        widths.append((road_half, side_l, side_r))
    # Fill occasional skipped vertices from neighbours, then skip the line if
    # it is still badly constrained.
    for i in range(len(widths)):
        if widths[i] is None:
            neighbours = [widths[j] for j in (i - 1, i + 1) if 0 <= j < len(widths) and widths[j]]
            widths[i] = neighbours[0] if neighbours else None
    if sum(w is not None for w in widths) < 2:
        continue
    # A tiny vertical offset keeps crossing streets from z-fighting.
    y0 = ASPHALT_Y + (li % 97) * 0.00004
    y1 = SIDEWALK_Y + (li % 97) * 0.00004
    for i in range(len(points) - 1):
        w0, w1 = widths[i], widths[i + 1]
        if w0 is None or w1 is None:
            continue
        a = points[i]
        bb = points[i + 1]
        n0 = normals[i]
        n1 = normals[i + 1]
        r0, sl0, sr0 = w0
        r1, sl1, sr1 = w1

        # Asphalt strip.
        add_quad(
            asphalt_v, asphalt_f,
            (a[0] + n0[0] * r0, y0, a[1] + n0[1] * r0),
            (a[0] - n0[0] * r0, y0, a[1] - n0[1] * r0),
            (bb[0] - n1[0] * r1, y0, bb[1] - n1[1] * r1),
            (bb[0] + n1[0] * r1, y0, bb[1] + n1[1] * r1),
        )
        # Sidewalk tops and inner curbs.
        if sl0 > 0.02 or sl1 > 0.02:
            add_quad(
                walk_v, walk_f,
                (a[0] + n0[0] * (r0 + sl0), y1, a[1] + n0[1] * (r0 + sl0)),
                (a[0] + n0[0] * r0, y1, a[1] + n0[1] * r0),
                (bb[0] + n1[0] * r1, y1, bb[1] + n1[1] * r1),
                (bb[0] + n1[0] * (r1 + sl1), y1, bb[1] + n1[1] * (r1 + sl1)),
            )
            add_vertical_quad(
                walk_v, walk_f,
                (a[0] + n0[0] * r0, a[1] + n0[1] * r0),
                (bb[0] + n1[0] * r1, bb[1] + n1[1] * r1),
                y0, y1,
            )
        if sr0 > 0.02 or sr1 > 0.02:
            add_quad(
                walk_v, walk_f,
                (a[0] - n0[0] * r0, y1, a[1] - n0[1] * r0),
                (a[0] - n0[0] * (r0 + sr0), y1, a[1] - n0[1] * (r0 + sr0)),
                (bb[0] - n1[0] * (r1 + sr1), y1, bb[1] - n1[1] * (r1 + sr1)),
                (bb[0] - n1[0] * r1, y1, bb[1] - n1[1] * r1),
            )
            add_vertical_quad(
                walk_v, walk_f,
                (a[0] - n0[0] * r0, a[1] - n0[1] * r0),
                (bb[0] - n1[0] * r1, bb[1] - n1[1] * r1),
                y0, y1,
            )
    built += 1

print("lines with roadway geometry:", built, "of", len(stations))
print("asphalt verts/faces", len(asphalt_v), len(asphalt_f))
print("walk verts/faces", len(walk_v), len(walk_f))


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


asphalt = make_mesh(asphalt_v, asphalt_f, "Road_Asphalt", [0.075, 0.078, 0.085, 1.0], 0.92)
sidewalk = make_mesh(walk_v, walk_f, "Sidewalk_Concrete", [0.62, 0.62, 0.59, 1.0], 0.86)
scene = trimesh.Scene()
scene.add_geometry(asphalt, geom_name="road_asphalt")
scene.add_geometry(sidewalk, geom_name="sidewalk_concrete")
scene.export(OUT, include_normals=True)
print("wrote", OUT, os.path.getsize(OUT))
