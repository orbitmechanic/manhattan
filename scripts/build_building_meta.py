"""Export block footprints, heights and ground-floor categories.

Run on the block master:
  blender -b manhattan_blocks.blend --python scripts/build_building_meta.py

Output:
  build/building_perimeters.json
  viewer/assets/models/building_perimeters.json
"""
import json
import math
import os

import bpy
from mathutils import Vector

ROOT = "/home/orbit9/Work/games/Manhattan"
OUT_BUILD = os.path.join(ROOT, "build", "building_perimeters.json")
OUT_VIEWER = os.path.join(ROOT, "viewer", "assets", "models", "building_perimeters.json")


def hull_2d(points):
    pts = sorted(set((round(float(p.x), 3), round(float(p.y), 3)) for p in points))
    if len(pts) <= 3:
        return pts

    def cross(o, a, b):
        return (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])

    lower = []
    for p in pts:
        while len(lower) >= 2 and cross(lower[-2], lower[-1], p) <= 0:
            lower.pop()
        lower.append(p)
    upper = []
    for p in reversed(pts):
        while len(upper) >= 2 and cross(upper[-2], upper[-1], p) <= 0:
            upper.pop()
        upper.append(p)
    return lower[:-1] + upper[:-1]


city = bpy.data.collections["City_Blocks"]
blocks = [o for o in city.objects if o.type == "MESH" and o.name.startswith("Block_")]
print("blocks", len(blocks))
buildings = []
for ob in blocks:
    mw = ob.matrix_world
    verts = [mw @ v.co for v in ob.data.vertices]
    if not verts:
        continue
    zs = [v.z for v in verts]
    zmin, zmax = min(zs), max(zs)
    bottom = [v for v in verts if v.z <= zmin + 3.1]
    if len(bottom) < 3:
        bottom = verts
    hull = hull_2d(bottom)
    if len(hull) < 3:
        continue
    cx = sum(v.x for v in verts) / len(verts)
    cy = sum(v.y for v in verts) / len(verts)
    height = zmax - zmin
    # Viewer axes: x, z = -Blender-y.
    pts = [[round(x, 3), round(-y, 3)] for x, y in hull]
    # Category rules: tall downtown = executive, low waterfront = industrial,
    # low/mid residential, remaining mid-rise storefront.
    if height >= 150.0:
        category = "executive"
    elif height < 45.0 and (cx < -1200.0 or cx > 3800.0 or cy < -1900.0 or cy > 1900.0):
        category = "industrial"
    elif height < 45.0:
        category = "residential"
    else:
        category = "storefront"
    buildings.append({
        "id": int(ob.get("block_index", len(buildings))),
        "height_m": round(height, 2),
        "centroid": [round(cx, 3), round(-cy, 3)],
        "hull": pts,
        "category": category,
    })

buildings.sort(key=lambda b: b["id"])
counts = {}
for b in buildings:
    counts[b["category"]] = counts.get(b["category"], 0) + 1
print("categories", counts, "total", len(buildings))
payload = {"count": len(buildings), "categories": counts, "buildings": buildings}
for path in (OUT_BUILD, OUT_VIEWER):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        json.dump(payload, f, separators=(",", ":"))
    print("wrote", path)
