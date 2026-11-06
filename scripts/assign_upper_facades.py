"""Bake upper-floor facade and roof choices into the city mesh.

Reads `viewer/assets/models/building_perimeters.json` (real first-floor wall
loops) and `upper_facade_catalog.json`, assigns each building one unique
facade template and one roof template, then writes the choice into COLOR_0:

  R, G, B = 24-bit packed value
    bits 0..8   facade tile index (0..511)
    bits 9..16  roof tile index (0..255)
    bits 17..19 facade tint variation (0..7)
    bits 20..23 facade U offset (0..15)
  A = block id / 1536 (unchanged)

The custom shader decodes this per fragment.  Every wall face of a building
uses the same facade template; every horizontal face uses the same roof
template.

Run:
  blender -b manhattan_floors.blend --python scripts/assign_upper_facades.py
"""
import json
import math
import os
import random
from collections import Counter, defaultdict

import bpy

ROOT = "/home/orbit9/Work/games/Manhattan"
BUILDINGS = os.path.join(ROOT, "viewer", "assets", "models", "building_perimeters.json")
CATALOG = os.path.join(ROOT, "viewer", "assets", "models", "upper_facade_catalog.json")
OUT_BLEND = os.path.join(ROOT, "manhattan_floors.blend")
OUT_GLB = os.path.join(ROOT, "viewer", "assets", "models", "manhattan_city.glb")
OUT_ASSIGN = os.path.join(ROOT, "build", "upper_facade_assignments.json")
MAX_BLOCKS = 1536
SEED = 0x5EED_2026
ROOF_UV_SCALE = 0.5


def splitmix64(value):
    value = (value + 0x9E3779B97F4A7C15) & 0xFFFFFFFFFFFFFFFF
    value = ((value ^ (value >> 30)) * 0xBF58476D1CE4E5B9) & 0xFFFFFFFFFFFFFFFF
    value = ((value ^ (value >> 27)) * 0x94D049BB133111EB) & 0xFFFFFFFFFFFFFFFF
    return value ^ (value >> 31)


def hash01(value):
    return (splitmix64(value) & 0xFFFFFFFF) / 0xFFFFFFFF


def signed_area(poly):
    return 0.5 * sum(
        poly[i][0] * poly[(i + 1) % len(poly)][1]
        - poly[(i + 1) % len(poly)][0] * poly[i][1]
        for i in range(len(poly))
    )


def point_in_poly(x, y, poly):
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


def segment_distance(px, py, ax, ay, bx, by):
    dx, dy = bx - ax, by - ay
    length_sq = dx * dx + dy * dy
    if length_sq <= 1e-12:
        return math.hypot(px - ax, py - ay)
    t = ((px - ax) * dx + (py - ay) * dy) / length_sq
    t = max(0.0, min(1.0, t))
    return math.hypot(px - (ax + dx * t), py - (ay + dy * t))


def loop_distance(px, py, poly):
    return min(
        segment_distance(px, py, poly[i][0], poly[i][1],
                         poly[(i + 1) % len(poly)][0], poly[(i + 1) % len(poly)][1])
        for i in range(len(poly))
    )


def load_data():
    buildings = json.load(open(BUILDINGS))["buildings"]
    catalog = json.load(open(CATALOG))
    return buildings, catalog


class Loop:
    __slots__ = ("building", "poly", "minx", "maxx", "miny", "maxy", "area")

    def __init__(self, building):
        self.building = building
        # Viewer is (x, z); Blender XY is (x, -z).
        self.poly = [(float(p[0]), -float(p[1])) for p in building["loop"]]
        xs = [p[0] for p in self.poly]
        ys = [p[1] for p in self.poly]
        self.minx, self.maxx = min(xs), max(xs)
        self.miny, self.maxy = min(ys), max(ys)
        self.area = abs(signed_area(self.poly))


def choose_building(px, py, loops, prefer_contains=True):
    """Pick the loop that best owns a plan point."""
    best = None
    best_key = None
    for loop in loops:
        if px < loop.minx - 2.0 or px > loop.maxx + 2.0:
            continue
        if py < loop.miny - 2.0 or py > loop.maxy + 2.0:
            continue
        inside = point_in_poly(px, py, loop.poly)
        distance = loop_distance(px, py, loop.poly)
        key = (0 if inside else 1, round(distance, 3), loop.area)
        if best_key is None or key < best_key:
            best_key = key
            best = loop
    if best is not None:
        return best
    # Coarse fallback: nearest loop anywhere in the block.
    for loop in loops:
        distance = loop_distance(px, py, loop.poly)
        key = (1, round(distance, 3), loop.area)
        if best_key is None or key < best_key:
            best_key = key
            best = loop
    return best


def category_decks(catalog, rng):
    decks = {}
    for name, (start, end) in catalog["facade"]["categories"].items():
        deck = list(range(start, end))
        rng.shuffle(deck)
        decks[name] = deck
    return decks


def build_assignments(buildings, catalog):
    rng = random.Random(SEED)
    decks = category_decks(catalog, rng)
    roof_deck = list(range(catalog["roof"]["count"]))
    rng.shuffle(roof_deck)
    mapping = catalog["category_mapping"]
    assignments = {}
    for building in sorted(buildings, key=lambda b: (b["block"], b["id"])):
        category = mapping.get(building["category"], "commercial")
        if not decks[category]:
            start, end = catalog["facade"]["categories"][category]
            decks[category] = list(range(start, end))
            rng.shuffle(decks[category])
        facade_tile = decks[category].pop()
        if not roof_deck:
            roof_deck = list(range(catalog["roof"]["count"]))
            rng.shuffle(roof_deck)
        roof_tile = roof_deck.pop()
        h = splitmix64(SEED ^ (building["id"] * 0x9E3779B97F4A7C15))
        tint = (h >> 17) & 7
        offset = (h >> 20) & 15
        assignments[building["id"]] = {
            "block": building["block"],
            "category": category,
            "facade_tile": facade_tile,
            "roof_tile": roof_tile,
            "tint": tint,
            "offset": offset,
            "height_m": building["height_m"],
        }
    return assignments


def pack(assignment):
    value = (
        assignment["facade_tile"]
        | (assignment["roof_tile"] << 9)
        | (assignment["tint"] << 17)
        | (assignment["offset"] << 20)
    )
    r = ((value >> 16) & 255) / 255.0
    g = ((value >> 8) & 255) / 255.0
    b = (value & 255) / 255.0
    return r, g, b


def set_roof_uvs(mesh, poly, mw, uv_layer):
    """Align the roof texture grid to this roof face's longest edge."""
    coords = []
    for li in poly.loop_indices:
        idx = mesh.loops[li].vertex_index
        coords.append((li, mw @ mesh.vertices[idx].co))
    if len(coords) < 3:
        return
    # Longest plan edge of the face, in world XY.
    best = None
    count = len(coords)
    for i in range(count):
        a = coords[i][1]
        b = coords[(i + 1) % count][1]
        dx, dy = b.x - a.x, b.y - a.y
        length = math.hypot(dx, dy)
        if length < 1e-6:
            continue
        if best is None or length > best[0]:
            best = (length, a, dx / length, dy / length)
    if best is None:
        return
    _, origin, ux, uy = best
    vx, vy = -uy, ux
    for li, p in coords:
        dx, dy = p.x - origin.x, p.y - origin.y
        u = (dx * ux + dy * uy) / 8.0
        v = (dx * vx + dy * vy) / 8.0
        uv_layer.data[li].uv = (u, v)


def main():
    buildings, catalog = load_data()
    by_block = defaultdict(list)
    for building in buildings:
        by_block[building["block"]].append(Loop(building))
    assignments = build_assignments(buildings, catalog)

    faces_total = 0
    faces_unmatched = 0
    faces_roof = 0
    faces_wall = 0
    matched_buildings = set()
    tile_use = Counter()
    roof_use = Counter()

    for obj in bpy.data.collections["Floor_Blocks"].objects:
        if obj.type != "MESH" or not obj.name.startswith("Floor_Block_"):
            continue
        block_id = int(obj.name.split("_")[-1])
        loops = by_block.get(block_id, [])
        if not loops:
            print("no loops for block", block_id)
            continue
        mw = obj.matrix_world
        normal_matrix = mw.to_3x3()
        mesh = obj.data
        uv_layer = mesh.uv_layers.active or (mesh.uv_layers[0] if mesh.uv_layers else None)
        if uv_layer is None:
            uv_layer = mesh.uv_layers.new(name="UVMap")
            mesh.uv_layers.active = uv_layer
        color = mesh.color_attributes.get("block_id")
        if color is None:
            color = mesh.color_attributes.new(name="block_id", type="FLOAT_COLOR", domain="CORNER")
            mesh.color_attributes.active_color = color

        for poly in mesh.polygons:
            faces_total += 1
            center = mw @ poly.center
            normal = normal_matrix @ poly.normal
            if normal.length > 1e-8:
                normal.normalize()
            roof = abs(normal.z) > 0.55
            chosen = None
            if roof:
                chosen = choose_building(center.x, center.y, loops)
                faces_roof += 1
            else:
                # The first-floor loops are the footprint; step in from the
                # wall on either normal direction so the correct building
                # contains the sample point.
                for sign in (-1.0, 1.0):
                    px = center.x + normal.x * 0.35 * sign
                    py = center.y + normal.y * 0.35 * sign
                    for loop in loops:
                        if px < loop.minx or px > loop.maxx or py < loop.miny or py > loop.maxy:
                            continue
                        if point_in_poly(px, py, loop.poly):
                            chosen = loop
                            break
                    if chosen is not None:
                        break
                if chosen is None:
                    chosen = choose_building(center.x, center.y, loops)
                faces_wall += 1

            if chosen is None:
                faces_unmatched += 1
                chosen = loops[0]
            assignment = assignments[chosen.building["id"]]
            r, g, b = pack(assignment)
            a = (block_id + 0.5) / MAX_BLOCKS
            for li in poly.loop_indices:
                color.data[li].color = (r, g, b, a)
            if roof:
                set_roof_uvs(mesh, poly, mw, uv_layer)
            matched_buildings.add(chosen.building["id"])
            tile_use[assignment["facade_tile"]] += 1
            roof_use[assignment["roof_tile"]] += 1

        # Ensure this attribute is the one the exporter uses.
        mesh.color_attributes.active_color = color

    print("faces", faces_total, "wall", faces_wall, "roof", faces_roof,
          "unmatched", faces_unmatched)
    print("buildings matched", len(matched_buildings), "of", len(buildings))
    print("facade tiles used", len(tile_use), "roof tiles used", len(roof_use))
    print("facade use min/max", min(tile_use.values()), max(tile_use.values()))
    print("roof use min/max", min(roof_use.values()), max(roof_use.values()))

    with open(OUT_ASSIGN, "w") as handle:
        json.dump({
            "seed": SEED,
            "roof_uv_scale": ROOF_UV_SCALE,
            "assignments": assignments,
        }, handle, separators=(",", ":"))
    print("wrote", OUT_ASSIGN, os.path.getsize(OUT_ASSIGN))

    bpy.ops.wm.save_as_mainfile(filepath=OUT_BLEND, compress=True)
    print("saved", OUT_BLEND)

    objects = [o for o in bpy.data.collections["Floor_Blocks"].objects if o.type == "MESH"]
    bpy.ops.object.select_all(action="DESELECT")
    for obj in objects:
        obj.select_set(True)
    bpy.context.view_layer.objects.active = objects[0]
    bpy.ops.export_scene.gltf(
        filepath=OUT_GLB,
        export_format="GLB",
        use_selection=True,
        export_apply=True,
        export_materials="EXPORT",
        export_image_format="AUTO",
        export_texcoords=True,
        export_normals=False,
        export_vertex_color="ACTIVE",
        export_all_vertex_colors=True,
        export_cameras=False,
        export_lights=False,
        export_extras=False,
        export_yup=True,
    )
    print("wrote", OUT_GLB, os.path.getsize(OUT_GLB))


main()
