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
from scipy.spatial import cKDTree
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

def prune_spurs(mask, max_len_px):
    """Remove short dead-end hairs from a skeleton; keep junctions."""
    work = mask.copy()
    for _ in range(200):
        ys, xs = np.nonzero(work)
        pix = set(zip(ys.tolist(), xs.tolist()))

        def neigh(p):
            y, x = p
            return [
                (y + dy, x + dx)
                for dy in (-1, 0, 1)
                for dx in (-1, 0, 1)
                if (dy or dx) and (y + dy, x + dx) in pix
            ]

        deg = {p: len(neigh(p)) for p in pix}
        removed = 0
        for p in [q for q in pix if deg[q] == 1]:
            if p not in pix:
                continue
            path = [p]
            prev = None
            cur = p
            while len(path) <= max_len_px:
                nxt = [q for q in neigh(cur) if q != prev]
                if not nxt:
                    break
                if len(nxt) > 1:
                    break
                prev, cur = cur, nxt[0]
                path.append(cur)
                if deg.get(cur, 0) != 2:
                    break
            # Keep the junction pixel; drop the hair.
            if len(path) <= max_len_px and deg.get(path[-1], 0) != 2:
                for q in path[:-1] if deg.get(path[-1], 0) > 2 else path:
                    if q in pix:
                        work[q] = False
                        removed += 1
        if removed == 0:
            break
    return work

def line_components(lines, tol=3.0):
    """Connect line endpoints to any nearby line segment; return components."""
    segments = []
    for i, line in enumerate(lines):
        for a, b in zip(line, line[1:]):
            segments.append((i, a, b))
    cell = 20.0
    grid = defaultdict(list)
    for si, (i, a, b) in enumerate(segments):
        for gx in range(int(min(a[0], b[0]) // cell), int(max(a[0], b[0]) // cell) + 1):
            for gy in range(int(min(a[1], b[1]) // cell), int(max(a[1], b[1]) // cell) + 1):
                grid[(gx, gy)].append(si)

    def point_segment(p, a, b):
        dx, dy = b[0] - a[0], b[1] - a[1]
        L = dx * dx + dy * dy
        if L == 0:
            return math.dist(p, a), a
        t = max(0.0, min(1.0, ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / L))
        q = (a[0] + t * dx, a[1] + t * dy)
        return math.dist(p, q), q

    parent = list(range(len(lines)))

    def find(a):
        while parent[a] != a:
            parent[a] = parent[parent[a]]
            a = parent[a]
        return a

    def union(a, b):
        ra, rb = find(a), find(b)
        if ra != rb:
            parent[rb] = ra

    for i, line in enumerate(lines):
        for e in (line[0], line[-1]):
            gx, gy = int(e[0] // cell), int(e[1] // cell)
            seen = set()
            for dx in (-1, 0, 1):
                for dy in (-1, 0, 1):
                    for si in grid.get((gx + dx, gy + dy), []):
                        if si in seen:
                            continue
                        seen.add(si)
                        j, a, b = segments[si]
                        if j != i and point_segment(e, a, b)[0] <= tol:
                            union(i, j)
    comps = {}
    for i in range(len(lines)):
        comps.setdefault(find(i), []).append(i)
    return comps, segments, grid, point_segment


def connect_line_components(lines):
    comps, segments, _, point_segment = line_components(lines)
    if len(comps) <= 1:
        return lines
    # Largest component is the trunk.
    trunk = max(comps.values(), key=len)
    trunk_set = set(trunk)
    for _ in range(len(comps) + 5):
        comps, segments, _, point_segment = line_components(lines)
        if len(comps) <= 1:
            break
        trunk = max(comps.values(), key=len)
        trunk_set = set(trunk)
        # Add one connector from the nearest outside component to the trunk.
        best = None
        for root, members in comps.items():
            if root == max(comps, key=lambda r: len(comps[r])):
                continue
            for i in members:
                for e in (lines[i][0], lines[i][-1]):
                    for j in trunk:
                        for a, b in zip(lines[j], lines[j][1:]):
                            d, q = point_segment(e, a, b)
                            if best is None or d < best[0]:
                                best = (d, i, e, q)
        if best is None:
            break
        d, i, e, q = best
        lines.append([e, q])
        print("completed vector connector", round(d, 2), "m")
    return lines

def viewer_from_pixel(px, py):
    x, y = world_from_pixel(px, py)
    return (x, -y)


def viewer_to_pixel(x, z):
    px = int(np.clip((x - minx) / (maxx - minx) * W, 0, W - 1))
    py = int(np.clip((maxy + z) / (maxy - miny) * H, 0, H - 1))
    return py, px


def crosses_building(a, b):
    for t in np.linspace(0.0, 1.0, 12):
        x = a[0] + (b[0] - a[0]) * t
        z = a[1] + (b[1] - a[1]) * t
        py, px = viewer_to_pixel(x, z)
        if mask[py, px]:
            return True
    return False

# ---------------------------------------------------------------------------
# C. completed building-front network: connect all meaningful components
# ---------------------------------------------------------------------------
# Drop tiny disconnected hairs before connecting; then connect every remaining
# component with a minimum spanning tree of free-space paths.  Buildings are
# impassable and water/open ground is expensive, so connectors follow road
# corridors where possible.
labels_all, count_all = ndi.label(fronts_pixels, structure=np.ones((3, 3)))
sizes_all = ndi.sum(fronts_pixels, labels_all, range(1, count_all + 1))
keep_ids = [i + 1 for i, size in enumerate(sizes_all) if size >= 100]
base = np.isin(labels_all, keep_ids)
base_labels, base_count = ndi.label(base, structure=np.ones((3, 3)))
print("completed base components:", base_count)

# Component adjacency from nearest boundary pixels.
boundary = base & ~ndi.binary_erosion(base, iterations=1)
ys, xs = np.nonzero(base)
pts = np.column_stack([ys, xs])
pt_labels = base_labels[ys, xs]
tree = cKDTree(pts)
by, bx = np.nonzero(boundary)
bpts = np.column_stack([by, bx])
b_labels = base_labels[by, bx]
dists, idxs = tree.query(bpts, k=32, workers=-1)
comp_pts = {}
for i in range(1, base_count + 1):
    yy, xx = np.nonzero(base_labels == i)
    comp_pts[i] = np.column_stack([yy, xx])
comp_ids = sorted(comp_pts)
centroids = np.array(
    [
        [comp_pts[i][:, 1].mean(), comp_pts[i][:, 0].mean()]
        for i in comp_ids
    ],
    dtype=np.float64,
)
pair_edges = []
for ii in range(len(comp_ids)):
    for jj in range(ii + 1, len(comp_ids)):
        d = float(np.linalg.norm(centroids[ii] - centroids[jj]))
        pair_edges.append((d, ii, jj))
pair_edges.sort(key=lambda e: e[0])

parent = list(range(len(comp_ids)))


def find(a):
    while parent[a] != a:
        parent[a] = parent[parent[a]]
        a = parent[a]
    return a


def union(a, b):
    ra, rb = find(a), find(b)
    if ra == rb:
        return False
    parent[rb] = ra
    return True


mst_edges = []
for d, ii, jj in pair_edges:
    if not union(ii, jj):
        continue
    a, b = comp_ids[ii], comp_ids[jj]
    A, B = comp_pts[a], comp_pts[b]
    tree_a = cKDTree(A)
    dd, idx = tree_a.query(B)
    k = int(np.argmin(dd))
    sa = (int(A[idx[k], 0]), int(A[idx[k], 1]))
    sb = (int(B[k, 0]), int(B[k, 1]))
    mst_edges.append((a, b, sa, sb))
print("completed connector edges:", len(mst_edges))

# Cost: buildings impassable; roads cheap; water/open land expensive.
mx = rgb.max(axis=2)
mn = rgb.min(axis=2)
sat_ = np.where(mx > 0, (mx - mn) / np.maximum(mx, 1e-6), 0.0)
road = (sat_ < 0.16) & (mx > 0.22) & (mx < 0.82)
cost = np.ones((H, W), dtype=np.float64)
cost[road] = 0.25
cost += np.clip((dist_building - 20.0) / 10.0, 0.0, 15.0) * 1.5
# Water/open space is expensive, but a road-coloured bridge remains cheap.
water = (dist_building > 45.0) & ~road & ~base
cost[water] = 120.0
cost[base] = 0.10
cost[mask] = np.inf

completed = base.copy()
from skimage.graph import route_through_array
for i, j, s, e in mst_edges:
    try:
        path, _ = route_through_array(
            cost, s, e, fully_connected=False, geometric=True
        )
    except Exception as exc:
        print("connector failed", i, j, exc)
        continue
    yy = np.array([p[0] for p in path])
    xx = np.array([p[1] for p in path])
    if mask[yy, xx].any():
        print("connector rejected (crosses building)", i, j)
        continue
    if water[yy, xx].mean() > 0.05:
        print(
            "connector crosses water",
            i,
            j,
            "pixels",
            int(water[yy, xx].sum()),
            "route",
            len(path),
        )
    completed[yy, xx] = True
print("completed skeleton pixels:", int(completed.sum()))
completed = prune_spurs(completed, 10)
print("completed skeleton after spur pruning:", int(completed.sum()))
completed_lines = vectorize(completed, 2.0, 5.0)
completed_lines = connect_line_components(completed_lines)
# Remove any RDP/connector corner that clips a building: route those segments
# through the free-space cost field instead.
clear_lines = []
for line in completed_lines:
    if len(line) < 2:
        clear_lines.append(line)
        continue
    out = [line[0]]
    for a, b in zip(line, line[1:]):
        if not crosses_building(a, b):
            out.append(b)
            continue
        start = viewer_to_pixel(*a)
        end = viewer_to_pixel(*b)
        try:
            path, _ = route_through_array(
                cost, start, end, fully_connected=False, geometric=True
            )
            out.extend(viewer_from_pixel(x, y) for y, x in path[1:])
        except Exception:
            out.append(b)
    clear_lines.append(out)
completed_lines = [line for line in clear_lines if not any(
    crosses_building(a, b) for a, b in zip(line, line[1:])
)]
print("completed lines after clearance:", len(completed_lines))

# Reconnect the small number of components left after removing any line that
# still clipped a building.  Connectors are routed through free space.
def safe_path(a, b):
    if not crosses_building(a, b):
        return [a, b]
    start = viewer_to_pixel(*a)
    end = viewer_to_pixel(*b)
    try:
        path, _ = route_through_array(
            cost, start, end, fully_connected=False, geometric=True
        )
    except Exception:
        return None
    pts = [viewer_from_pixel(x, y) for y, x in path]
    for p, q in zip(pts, pts[1:]):
        if crosses_building(p, q):
            return None
    return pts

for _ in range(20):
    comps, _, _, _ = line_components(completed_lines)
    if len(comps) <= 1:
        break
    main = max(comps.values(), key=len)
    main_pts = [e for i in main for e in (completed_lines[i][0], completed_lines[i][-1])]
    main_tree = cKDTree(np.asarray(main_pts))
    best = None
    for members in comps.values():
        if members is main or set(members) == set(main):
            continue
        for i in members:
            for e in (completed_lines[i][0], completed_lines[i][-1]):
                d, k = main_tree.query(e)
                if best is None or d < best[0]:
                    best = (d, e, tuple(main_pts[int(k)]))
    if best is None:
        break
    _d, a, b = best
    connector = safe_path(a, b)
    if connector is None:
        break
    completed_lines.append(connector)
    print("completed final connector", round(float(_d), 2), "m")

completed_set = package(completed_lines, "completed", "CMP")
overlay(completed_lines, os.path.join(BUILD, "station_lines_completed_overlay.png"))
print(
    f"completed lines: {completed_set['line_count']}  "
    f"{completed_set['total_length_ft']/5280:.1f} miles"
)

# ---------------------------------------------------------------------------
for name, data in (("imagery", imagery), ("fronts", fronts), ("completed", completed_set)):
    for path in (
        os.path.join(BUILD, f"station_lines_{name}.json"),
        os.path.join(ROOT, "viewer", "assets", "models", f"station_lines_{name}.json"),
    ):
        with open(path, "w") as f:
            json.dump(data, f, separators=(",", ":"))
        print("wrote", path)
