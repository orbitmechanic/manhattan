"""Export first-floor facade loops for every building.

The earlier metadata used convex hulls of whole block groups, which bridged
alleys and streets.  This version reads the actual first-floor wall polygons
from `manhattan_floors.blend` and chains their bottom edges into the closed
loops that form each building's exterior.

Run on the floor master:
  blender -b manhattan_floors.blend --python scripts/build_building_meta.py

Output:
  build/building_perimeters.json
  viewer/assets/models/building_perimeters.json
"""
import json
import math
import os
from collections import Counter, defaultdict

import bpy

ROOT = "/home/orbit9/Work/games/Manhattan"
OUT_BUILD = os.path.join(ROOT, "build", "building_perimeters.json")
OUT_VIEWER = os.path.join(ROOT, "viewer", "assets", "models", "building_perimeters.json")

# First-floor walls are the 3 m band starting at z=0.  Edges shorter than this
# are still kept if they close a loop; the facade paneling code merges them.
GROUND_EPS = 0.05
MIN_EDGE = 0.05
SIMPLIFY_EPS = 0.15
MIN_AREA = 1.0


def signed_area(poly):
    return 0.5 * sum(
        poly[i][0] * poly[(i + 1) % len(poly)][1]
        - poly[(i + 1) % len(poly)][0] * poly[i][1]
        for i in range(len(poly))
    )


def block_category(height, cx, cy):
    if height >= 150.0:
        return "executive"
    if height < 45.0 and (cx < -1200.0 or cx > 3800.0 or cy < -1900.0 or cy > 1900.0):
        return "industrial"
    if height < 45.0:
        return "residential"
    return "storefront"


def block_stats():
    stats = {}
    for ob in bpy.data.collections["City_Blocks"].objects:
        if ob.type != "MESH" or not ob.name.startswith("Block_"):
            continue
        bid = int(ob.name.split("_")[-1])
        mw = ob.matrix_world
        vs = [mw @ v.co for v in ob.data.vertices]
        if not vs:
            continue
        zs = [v.z for v in vs]
        cx = sum(v.x for v in vs) / len(vs)
        cy = sum(v.y for v in vs) / len(vs)
        height = max(zs) - min(zs)
        stats[bid] = {
            "height": height,
            "category": block_category(height, cx, cy),
        }
    return stats


def first_floor_edges(ob):
    """Bottom edges of the vertical ground-floor wall polygons."""
    mw = ob.matrix_world
    edges = []
    for poly in ob.data.polygons:
        vs = [mw @ ob.data.vertices[i].co for i in poly.vertices]
        zmin = min(v.z for v in vs)
        zmax = max(v.z for v in vs)
        if zmin > GROUND_EPS or zmax - zmin < 0.8:
            continue
        bottom = [v for v in vs if abs(v.z - zmin) < GROUND_EPS]
        if len(bottom) != 2:
            continue
        a, b = bottom
        if math.hypot(a.x - b.x, a.y - b.y) < MIN_EDGE:
            continue
        # 1 cm endpoints make the chain robust to float noise.
        edges.append(
            (
                (round(a.x, 2), round(a.y, 2)),
                (round(b.x, 2), round(b.y, 2)),
            )
        )
    return edges


def chain_loops(edges):
    """Chain wall bottom edges into closed loops."""
    adjacency = defaultdict(list)
    for index, (a, b) in enumerate(edges):
        adjacency[a].append((b, index))
        adjacency[b].append((a, index))
    used = set()
    loops = []
    for index, (a, b) in enumerate(edges):
        if index in used:
            continue
        loop = [a, b]
        used.add(index)
        cursor = b
        closed = False
        for _ in range(len(edges) + 2):
            if cursor == loop[0]:
                closed = True
                break
            step = None
            for other, edge_index in adjacency[cursor]:
                if edge_index not in used:
                    step = (other, edge_index)
                    break
            if step is None:
                break
            other, edge_index = step
            used.add(edge_index)
            cursor = other
            loop.append(cursor)
            if cursor == loop[0]:
                closed = True
                break
        if not closed or len(loop) < 4:
            continue
        poly = loop[:-1]
        if len(poly) < 3 or abs(signed_area(poly)) < MIN_AREA:
            continue
        loops.append(poly)
    return loops


def simplify_loop(poly, epsilon=SIMPLIFY_EPS):
    """Remove near-collinear micro-jogs while preserving real corners."""
    points = [(round(x, 3), round(y, 3)) for x, y in poly]
    changed = True
    while changed and len(points) > 4:
        changed = False
        out = []
        count = len(points)
        for i in range(count):
            a = points[(i - 1) % count]
            b = points[i]
            c = points[(i + 1) % count]
            ab = (c[0] - a[0], c[1] - a[1])
            length = math.hypot(*ab)
            if length < 1e-6:
                continue
            distance = abs(ab[0] * (b[1] - a[1]) - ab[1] * (b[0] - a[0])) / length
            if distance < epsilon:
                changed = True
                continue
            out.append(b)
        points = out
    return points


def vertical_walls(ob):
    """All vertical wall polygons: midpoint, top height, outward normal."""
    mw = ob.matrix_world
    out = []
    for poly in ob.data.polygons:
        vs = [mw @ ob.data.vertices[i].co for i in poly.vertices]
        zmin = min(v.z for v in vs)
        zmax = max(v.z for v in vs)
        if zmax - zmin < 0.8:
            continue
        cx = sum(v.x for v in vs) / len(vs)
        cy = sum(v.y for v in vs) / len(vs)
        normal = mw.to_3x3() @ poly.normal
        length = math.hypot(normal.x, normal.y)
        out.append((cx, cy, zmin, zmax, length, normal.x, normal.y))
    return out


def point_in_polygon(x, y, poly):
    inside = False
    count = len(poly)
    j = count - 1
    for i in range(count):
        xi, yi = poly[i]
        xj, yj = poly[j]
        if ((yi > y) != (yj > y)) and (x < (xj - xi) * (y - yi) / (yj - yi) + xi):
            inside = not inside
        j = i
    return inside


def assign_heights(loops, walls, block_height):
    heights = [0.0] * len(loops)
    floors = [0.0] * len(loops)
    boxes = [
        (min(p[0] for p in poly), max(p[0] for p in poly),
         min(p[1] for p in poly), max(p[1] for p in poly))
        for poly in loops
    ]
    for cx, cy, zmin, zmax, nlen, nx, ny in walls:
        if nlen < 1e-4:
            continue
        found = None
        for direction in (0.08, -0.08):
            px = cx - nx / nlen * direction
            py = cy - ny / nlen * direction
            for index, (x0, x1, y0, y1) in enumerate(boxes):
                if px < x0 - 0.1 or px > x1 + 0.1 or py < y0 - 0.1 or py > y1 + 0.1:
                    continue
                if point_in_polygon(px, py, loops[index]):
                    found = index
                    break
            if found is not None:
                break
        if found is None:
            continue
        heights[found] = max(heights[found], zmax)
        if zmin <= GROUND_EPS:
            floors[found] = max(floors[found], zmax)
    for index in range(len(heights)):
        if heights[index] <= 0.0:
            heights[index] = block_height
        if floors[index] <= 0.0:
            floors[index] = 3.0
    return heights, floors


def main():
    blocks = block_stats()
    buildings = []
    next_id = 0
    dropped = 0
    category_counts = Counter()
    no_loops = []

    for ob in bpy.data.collections["Floor_Blocks"].objects:
        if ob.type != "MESH" or not ob.name.startswith("Floor_Block_"):
            continue
        block_id = int(ob.name.split("_")[-1])
        info = blocks.get(block_id, {"height": 20.0, "category": "residential"})
        loops = chain_loops(first_floor_edges(ob))
        if not loops:
            no_loops.append(block_id)
            dropped += 1
            continue
        walls = vertical_walls(ob)
        heights, floors = assign_heights(loops, walls, info["height"])
        for poly, height, floor_height in zip(loops, heights, floors):
            simplified = simplify_loop(poly)
            # Keep the edge count below the facade deck so every panel can be
            # a unique image.  This only affects a handful of very complex
            # block-sized loops; normal buildings stay at 0.15 m tolerance.
            if len(simplified) > 140:
                for epsilon in (0.5, 1.0, 1.5, 2.0):
                    simplified = simplify_loop(poly, epsilon)
                    if len(simplified) <= 140:
                        break
            if len(simplified) < 3:
                dropped += 1
                continue
            if abs(signed_area(simplified)) < MIN_AREA:
                dropped += 1
                continue
            if height < 2.0:
                dropped += 1
                continue
            # Viewer coordinates: x stays, z = -Blender y; CCW in x/z.
            viewer = [[round(x, 2), round(-y, 2)] for x, y in simplified]
            if signed_area(viewer) < 0.0:
                viewer.reverse()
            if floor_height < 30.0:
                floor_height = min(floor_height, 4.2)
            if height >= 150.0:
                category = "executive"
            elif height < 45.0 and info["category"] == "industrial":
                category = "industrial"
            elif height < 45.0:
                category = "residential"
            else:
                category = "storefront"
            cx = sum(p[0] for p in viewer) / len(viewer)
            cz = sum(p[1] for p in viewer) / len(viewer)
            buildings.append(
                {
                    "id": next_id,
                    "block": block_id,
                    "height_m": round(height, 2),
                    "floor_height_m": round(floor_height, 2),
                    "centroid": [round(cx, 2), round(cz, 2)],
                    "loop": viewer,
                    "category": category,
                }
            )
            category_counts[category] += 1
            next_id += 1

    payload = {
        "count": len(buildings),
        "categories": dict(category_counts),
        "source": "manhattan_floors.blend / Floor_Block_* first-floor wall loops",
        "buildings": buildings,
    }
    for path in (OUT_BUILD, OUT_VIEWER):
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w") as handle:
            json.dump(payload, handle, separators=(",", ":"))
        print("wrote", path, os.path.getsize(path))
    print("buildings", len(buildings), "categories", dict(category_counts))
    print("blocks without first-floor loops", len(no_loops), no_loops[:8])
    print("dropped loops", dropped)


main()
