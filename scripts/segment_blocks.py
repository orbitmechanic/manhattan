#!/usr/bin/env python3
"""Geometry-driven city-block segmentation for Manhattan.

Inputs
  build/building_mask.png           top-down building alpha mask rendered by Blender
  Textures/day/ground/satellite_manhattan_ground.jpg

Outputs (in build/)
  blocks_labels.npy     uint16 block label per mask pixel (building blocks)
  blocks.json           block metadata (index, type, centroid, area, counts)
  ground_cells.npz      Voronoi ground polygons + block ids
  blocks_overlay.png    visual validation over the satellite

Method
  1. Dilate the building footprints by ~8 m and label connected clusters.
     Buildings in the same block merge; buildings across a street do not.
     These are the building blocks.
  2. Morphologically open the empty space by ~30 m; large interior open
     components (away from the map edge) become park / open-space blocks.
  3. Seeds: one per building component + block centroids + park grids.
     A Voronoi partition of the ground plane around those seeds gives ground
     tiles whose boundaries lie between blocks, i.e. down the middle of the
     street.  Polygons are clipped to the ground plane.
  4. Sort blocks for stable indices and write the data.

Run with a Python that has scipy/numpy (the analysis venv on this machine):
  /tmp/opencode/geoenv/bin/python scripts/segment_blocks.py
"""
import json
import os
import numpy as np
from PIL import Image
from scipy import ndimage as ndi

ROOT = "/home/orbit9/Work/games/Manhattan"
MASK = os.path.join(ROOT, "build", "building_mask.png")
SATELLITE = os.path.join(ROOT, "Textures/day/ground/satellite_manhattan_ground.jpg")
OUT = os.path.join(ROOT, "build")
MAX_BLOCKS = 1536
RES = 2.0  # world units per mask pixel

# mask bounds (union of building + ground), from render_building_mask.py
MINX, MAXX = -2446.01171875, 4683.123046875
MINY, MAXY = -2558.729736328125, 2621.252685546875
# visible ground plane bounds
GMINX, GMAXX = -2446.01171875, 4554.634
GMINY, GMAXY = -2373.2578125, 2518.712

DILATE_PX = 4       # 8 world units
PARK_OPEN_PX = 15   # 30 world units
PARK_MIN_PX = 5000  # 20,000 square world units
PARK_EDGE_PX = 100  # keep away from map edge (water)


def disk(r):
    y, x = np.ogrid[-r:r+1, -r:r+1]
    return x*x + y*y <= r*r


def pixel_to_world(x, y):
    return (
        MINX + (x + 0.5) / mask_w * (MAXX - MINX),
        MAXY - (y + 0.5) / mask_h * (MAXY - MINY),
    )


def world_to_pixel(x, y):
    return (
        int((x - MINX) / (MAXX - MINX) * mask_w),
        int((MAXY - y) / (MAXY - MINY) * mask_h),
    )


def clip_polygon(poly, xmin, xmax, ymin, ymax):
    """Sutherland-Hodgman clipping of a convex polygon to a rectangle."""
    def clip(points, inside, intersect):
        out = []
        if not points:
            return out
        prev = points[-1]
        prev_in = inside(prev)
        for cur in points:
            cur_in = inside(cur)
            if cur_in:
                if not prev_in:
                    out.append(intersect(prev, cur))
                out.append(cur)
            elif prev_in:
                out.append(intersect(prev, cur))
            prev, prev_in = cur, cur_in
        return out

    def ix_vertical(p, q, x):
        t = (x - p[0]) / (q[0] - p[0])
        return (x, p[1] + t * (q[1] - p[1]))

    def ix_horizontal(p, q, y):
        t = (y - p[1]) / (q[1] - p[1])
        return (p[0] + t * (q[0] - p[0]), y)

    poly = clip(poly, lambda p: p[0] >= xmin, lambda p, q: ix_vertical(p, q, xmin))
    poly = clip(poly, lambda p: p[0] <= xmax, lambda p, q: ix_vertical(p, q, xmax))
    poly = clip(poly, lambda p: p[1] >= ymin, lambda p, q: ix_horizontal(p, q, ymin))
    poly = clip(poly, lambda p: p[1] <= ymax, lambda p, q: ix_horizontal(p, q, ymax))
    return poly


def voronoi_finite_polygons_2d(vor, radius):
    """Reconstruct infinite Voronoi regions (standard recipe)."""
    new_regions = []
    new_vertices = vor.vertices.tolist()
    center = vor.points.mean(axis=0)
    all_ridges = {}
    for (p1, p2), (v1, v2) in zip(vor.ridge_points, vor.ridge_vertices):
        all_ridges.setdefault(p1, []).append((p2, v1, v2))
        all_ridges.setdefault(p2, []).append((p1, v1, v2))
    for p1, region in enumerate(vor.point_region):
        vertices = list(vor.regions[region])
        if all(v >= 0 for v in vertices):
            new_regions.append(vertices)
            continue
        ridges = all_ridges.get(p1, [])
        new_region = [v for v in vertices if v >= 0]
        for p2, v1, v2 in ridges:
            if v2 < 0:
                v1, v2 = v2, v1
            if v1 >= 0:
                continue
            tangent = vor.points[p2] - vor.points[p1]
            tangent /= np.linalg.norm(tangent)
            normal = np.array([-tangent[1], tangent[0]])
            midpoint = vor.points[[p1, p2]].mean(axis=0)
            direction = np.sign(np.dot(midpoint - center, normal)) * normal
            new_vertex = midpoint + direction * radius
            new_region.append(len(new_vertices))
            new_vertices.append(new_vertex.tolist())
        if not new_region:
            continue
        vs = np.asarray([new_vertices[v] for v in new_region])
        c = vs.mean(axis=0)
        angles = np.arctan2(vs[:, 1] - c[1], vs[:, 0] - c[0])
        new_regions.append(np.array(new_region)[np.argsort(angles)].tolist())
    return new_regions, np.asarray(new_vertices)


# ---------------------------------------------------------------------------
os.makedirs(OUT, exist_ok=True)
mask = np.asarray(Image.open(MASK))[..., 3] > 128
mask_h, mask_w = mask.shape
print("mask", mask.shape, "occupancy", round(mask.mean(), 4))

# --- building blocks -------------------------------------------------------
dil = ndi.binary_dilation(mask, structure=disk(DILATE_PX))
blab, n_building = ndi.label(dil)
sizes = ndi.sum(dil, blab, range(1, n_building + 1))
print("building blocks after dilation:", n_building)

# building components and seeds
comp, n_comp = ndi.label(mask)
ys, xs = np.nonzero(mask)
cl = comp[ys, xs]
counts = np.bincount(cl)
sumx = np.bincount(cl, weights=xs)
sumy = np.bincount(cl, weights=ys)
comp_cx = sumx[1:] / np.maximum(counts[1:], 1)
comp_cy = sumy[1:] / np.maximum(counts[1:], 1)
comp_area = counts[1:]

# decide each component's block from the dilated label at its centroid
block_of_comp = np.zeros(n_comp + 1, dtype=np.int32)
for i in range(1, n_comp + 1):
    cx = int(round(comp_cx[i - 1]))
    cy = int(round(comp_cy[i - 1]))
    lab = blab[min(max(cy, 0), mask_h - 1), min(max(cx, 0), mask_w - 1)]
    if lab == 0:
        # local majority fallback
        y0, y1 = max(0, cy - 3), min(mask_h, cy + 4)
        x0, x1 = max(0, cx - 3), min(mask_w, cx + 4)
        vals = blab[y0:y1, x0:x1]
        vals = vals[vals > 0]
        lab = int(np.bincount(vals).argmax()) if vals.size else 1
    block_of_comp[i] = lab

# sort building blocks by centroid for stable, human-friendly indices
meta = []
for lab in range(1, n_building + 1):
    m = blab == lab
    yy, xx = np.nonzero(m)
    if len(xx) == 0:
        continue
    wx, wy = pixel_to_world(xx.mean(), yy.mean())
    meta.append((lab, "building", wx, wy, len(xx)))
meta.sort(key=lambda t: (round(t[3] / 100), t[2]))
old_to_new = {lab: i for i, (lab, *_rest) in enumerate(meta)}
n_building_sorted = len(meta)
print("building blocks sorted:", n_building_sorted)

# remap building labels
blab_new = np.zeros_like(blab, dtype=np.uint16)
for old, new in old_to_new.items():
    blab_new[blab == old] = new + 1
blab = blab_new

# --- parks / large open space ---------------------------------------------
empty = ~mask
opened = ndi.binary_opening(empty, structure=disk(PARK_OPEN_PX))
plab, n_open = ndi.label(opened)
print("open components:", n_open)
parks = []
for i in range(1, n_open + 1):
    m = plab == i
    area = int(m.sum())
    if area < PARK_MIN_PX:
        continue
    yy, xx = np.nonzero(m)
    edge = min(xx.min(), yy.min(), mask_w - 1 - xx.max(), mask_h - 1 - yy.max())
    if edge < PARK_EDGE_PX:
        continue
    wx, wy = pixel_to_world(xx.mean(), yy.mean())
    parks.append((i, wx, wy, area, edge))
parks.sort(key=lambda t: (round(t[2] / 100), t[1]))
n_park = len(parks)
print("park/open-space blocks:", n_park)
for p in parks:
    print("  park", p[1], p[2], "area px", p[3], "edge", p[4])

# --- block metadata --------------------------------------------------------
blocks = []
for idx, (lab, typ, wx, wy, area) in enumerate(meta):
    blocks.append({
        "index": idx,
        "type": typ,
        "centroid": [float(round(wx, 2)), float(round(wy, 2))],
        "area_px": int(area),
    })
for j, (lab, wx, wy, area, edge) in enumerate(parks):
    idx = n_building_sorted + j
    blocks.append({
        "index": idx,
        "type": "park",
        "centroid": [float(round(wx, 2)), float(round(wy, 2))],
        "area_px": int(area),
        "edge_px": int(edge),
        "open_component": lab,
    })
block_count = n_building_sorted + n_park
assert block_count <= MAX_BLOCKS, block_count

blocks_json = {
    "max_blocks": MAX_BLOCKS,
    "block_count": block_count,
    "building_blocks": n_building_sorted,
    "park_blocks": n_park,
    "bounds": {"min_x": MINX, "max_x": MAXX, "min_y": MINY, "max_y": MAXY},
    "ground_bounds": {"min_x": GMINX, "max_x": GMAXX, "min_y": GMINY, "max_y": GMAXY},
    "resolution": RES,
    "blocks": blocks,
}
with open(os.path.join(OUT, "blocks.json"), "w") as f:
    json.dump(blocks_json, f, indent=2)

np.save(os.path.join(OUT, "blocks_labels.npy"), blab)

# --- ground Voronoi seeds --------------------------------------------------
seeds = []
seed_blocks = []

# one seed per building component, plus its block centroid
for i in range(1, n_comp + 1):
    lab = block_of_comp[i]
    if lab not in old_to_new:
        continue
    bx, by = pixel_to_world(comp_cx[i - 1], comp_cy[i - 1])
    if GMINX <= bx <= GMAXX and GMINY <= by <= GMAXY:
        seeds.append((bx, by))
        seed_blocks.append(old_to_new[lab])
# block centroids help elongated blocks cover their own ground
for lab, typ, wx, wy, area in meta:
    if GMINX <= wx <= GMAXX and GMINY <= wy <= GMAXY:
        seeds.append((wx, wy))
        seed_blocks.append(old_to_new[lab])
# park grid seeds
for j, (lab, wx, wy, area, edge) in enumerate(parks):
    idx = n_building_sorted + j
    m = plab == lab
    yy, xx = np.nonzero(m)
    step = 40  # 80 world units
    for y in range(0, mask_h, step):
        for x in range(0, mask_w, step):
            if m[y, x]:
                sx, sy = pixel_to_world(x, y)
                if GMINX <= sx <= GMAXX and GMINY <= sy <= GMAXY:
                    seeds.append((sx, sy))
                    seed_blocks.append(idx)

seeds = np.asarray(seeds, dtype=np.float64)
seed_blocks = np.asarray(seed_blocks, dtype=np.int32)
# jitter duplicates (Voronoi needs distinct points)
rng = np.random.default_rng(20261102)
seen = {}
for i in range(len(seeds)):
    key = (round(seeds[i, 0] * 4), round(seeds[i, 1] * 4))
    if key in seen:
        seeds[i] += rng.normal(0, 0.35, 2)
    else:
        seen[key] = i
print("ground seeds:", len(seeds))

# Voronoi
from scipy.spatial import Voronoi
vor = Voronoi(seeds)
regions, vertices = voronoi_finite_polygons_2d(
    vor, radius=float(np.ptp(seeds, axis=0).max() * 2.0)
)
print("voronoi regions:", len(regions))

poly_vertices = []
poly_offsets = [0]
poly_blocks = []
for point_index, region in enumerate(regions):
    if point_index >= len(seed_blocks):
        continue
    poly = [tuple(vertices[v]) for v in region]
    poly = clip_polygon(poly, GMINX, GMAXX, GMINY, GMAXY)
    if len(poly) < 3:
        continue
    poly_vertices.extend(poly)
    poly_offsets.append(len(poly_vertices))
    poly_blocks.append(int(seed_blocks[point_index]))

poly_vertices = np.asarray(poly_vertices, dtype=np.float64)
poly_offsets = np.asarray(poly_offsets, dtype=np.int32)
poly_blocks = np.asarray(poly_blocks, dtype=np.int32)
np.savez_compressed(
    os.path.join(OUT, "ground_cells.npz"),
    vertices=poly_vertices,
    offsets=poly_offsets,
    block_ids=poly_blocks,
)
print("ground polygons:", len(poly_blocks), "vertices:", len(poly_vertices))

# --- validation overlay ----------------------------------------------------
sat = Image.open(SATELLITE).convert("RGB")
gx0, gy0 = GMINX, GMINY
gw, gh = GMAXX - GMINX, GMAXY - GMINY
x0 = int(round((gx0 - MINX) / (MAXX - MINX) * mask_w))
x1 = int(round((gx0 + gw - MINX) / (MAXX - MINX) * mask_w))
y1 = int(round((MAXY - gy0) / (MAXY - MINY) * mask_h))
y0 = int(round((MAXY - (gy0 + gh)) / (MAXY - MINY) * mask_h))
base = Image.new("RGB", (mask_w, mask_h), (12, 12, 16))
base.paste(sat.resize((x1 - x0, y1 - y0)), (x0, y0))

rng = np.random.default_rng(99)
colors = rng.integers(40, 255, size=(block_count + 1, 3), dtype=np.uint8)
colors[0] = (0, 0, 0)
overlay = colors[blab]
out = np.array(base).copy()
m = blab > 0
out[m] = (0.45 * out[m] + 0.55 * overlay[m]).astype(np.uint8)
for j, (lab, wx, wy, area, edge) in enumerate(parks):
    yy, xx = np.nonzero(plab == lab)
    out[yy, xx] = [0, 220, 120]
im = Image.fromarray(out)
im.thumbnail((1782, 1295))
im.save(os.path.join(OUT, "blocks_overlay.png"))
print("wrote outputs to", OUT)
