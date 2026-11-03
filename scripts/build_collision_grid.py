"""Bake a 2D building-height grid for camera collision.

Run on the block art master:

    blender -b manhattan_blocks.blend --python scripts/build_collision_grid.py

Output: viewer/assets/models/collision_grid.bin

Format (little endian):
    magic  b"MCG1"
    u32    width, height
    f32    min_x, min_z (glTF/viewer axes)
    f32    cell size (metres)
    u16    width*height heights in decimetres, row-major, 0 = open

Blender is Z-up, glTF is Y-up, so a Blender point (x, y, z) becomes
(x, z, -y).  The grid stores the top of every building column; the camera
collision code treats a column as solid from the ground up to that height.
"""
import os
import struct

import bpy
import numpy as np

ROOT = "/home/orbit9/Work/games/Manhattan"
OUT = os.path.join(ROOT, "viewer", "assets", "models", "collision_grid.bin")
CELL = 4.0  # metres

city = bpy.data.collections["City_Blocks"]
blocks = [
    o for o in city.objects
    if o.type == "MESH" and o.name.startswith("Block_")
]
print("building objects", len(blocks))

# Gather world-space triangles in viewer axes.
tris = []
minx = miny = float("inf")
maxx = maxy = float("-inf")
for ob in blocks:
    mw = ob.matrix_world
    me = ob.data
    me.calc_loop_triangles()
    coords = np.empty(len(me.vertices) * 3, dtype=np.float64)
    me.vertices.foreach_get("co", coords)
    coords = coords.reshape(-1, 3)
    # Blender -> glTF axes: (x, y, z) -> (x, z, -y)
    world = np.empty_like(coords)
    world[:, 0] = coords[:, 0] * mw[0][0] + coords[:, 1] * mw[0][1] + coords[:, 2] * mw[0][2] + mw[0][3]
    world[:, 1] = coords[:, 0] * mw[1][0] + coords[:, 1] * mw[1][1] + coords[:, 2] * mw[1][2] + mw[1][3]
    world[:, 2] = coords[:, 0] * mw[2][0] + coords[:, 1] * mw[2][1] + coords[:, 2] * mw[2][2] + mw[2][3]
    vx = world[:, 0]
    vy = world[:, 2]          # viewer height
    vz = -world[:, 1]         # viewer z
    idx = np.empty(len(me.loop_triangles) * 3, dtype=np.int32)
    me.loop_triangles.foreach_get("vertices", idx)
    idx = idx.reshape(-1, 3)
    # (n, 3 verts, (x, z, height))
    tri = np.stack([vx[idx], vz[idx], vy[idx]], axis=2)
    tris.append(tri)
    minx = min(minx, float(vx.min()))
    maxx = max(maxx, float(vx.max()))
    miny = min(miny, float(vz.min()))
    maxy = max(maxy, float(vz.max()))

tris = np.concatenate(tris, axis=0)
print("triangles", len(tris), "bounds", (minx, maxx, miny, maxy))

pad = CELL * 4.0
minx -= pad
miny -= pad
maxx += pad
maxy += pad
w = int(np.ceil((maxx - minx) / CELL)) + 1
h = int(np.ceil((maxy - miny) / CELL)) + 1
heights = np.zeros((h, w), dtype=np.float32)

# Rasterise every projected triangle and keep the highest top.
for tri in tris:
    x = (tri[:, 0] - minx) / CELL
    z = (tri[:, 1] - miny) / CELL
    hz = tri[:, 2]
    x0 = max(0, int(np.floor(x.min())) - 1)
    x1 = min(w - 1, int(np.ceil(x.max())) + 1)
    z0 = max(0, int(np.floor(z.min())) - 1)
    z1 = min(h - 1, int(np.ceil(z.max())) + 1)
    if x0 > x1 or z0 > z1:
        continue
    gx = np.arange(x0, x1 + 1, dtype=np.float64)
    gz = np.arange(z0, z1 + 1, dtype=np.float64)
    gx, gz = np.meshgrid(gx, gz)
    px = gx.ravel()
    pz = gz.ravel()
    # barycentric point-in-triangle
    ax, ay = x[0], z[0]
    bx, by = x[1], z[1]
    cx, cy = x[2], z[2]
    den = (by - cy) * (ax - cx) + (cx - bx) * (ay - cy)
    if abs(den) < 1e-12:
        continue
    l1 = ((by - cy) * (px - cx) + (cx - bx) * (pz - cy)) / den
    l2 = ((cy - ay) * (px - cx) + (ax - cx) * (pz - cy)) / den
    l3 = 1.0 - l1 - l2
    inside = (l1 >= -0.05) & (l2 >= -0.05) & (l3 >= -0.05)
    if not inside.any():
        continue
    top = float(hz.max())
    sel_x = px[inside].astype(np.int32)
    sel_z = pz[inside].astype(np.int32)
    heights[sel_z, sel_x] = np.maximum(heights[sel_z, sel_x], top)

# Also fill cells whose four neighbours are all buildings.  This closes the
# small gaps left where roof triangles meet walls at a grid corner.
solid = heights > 0.5
for _ in range(2):
    up = np.zeros_like(solid)
    up[1:, :] = solid[:-1, :]
    dn = np.zeros_like(solid)
    dn[:-1, :] = solid[1:, :]
    lf = np.zeros_like(solid)
    lf[:, 1:] = solid[:, :-1]
    rt = np.zeros_like(solid)
    rt[:, :-1] = solid[:, 1:]
    fill = (~solid) & lf & rt & up & dn
    if not fill.any():
        break
    vals = np.where(up, heights, 0)
    vals = np.maximum(vals, np.where(dn, heights, 0))
    vals = np.maximum(vals, np.where(lf, heights, 0))
    vals = np.maximum(vals, np.where(rt, heights, 0))
    heights[fill] = vals[fill]
    solid = heights > 0.5

heights_dm = np.clip(np.round(heights * 10.0), 0, 65535).astype("<u2")
header = b"MCG1" + struct.pack("<IIfff", w, h, minx, miny, CELL)
with open(OUT, "wb") as f:
    f.write(header)
    f.write(heights_dm.tobytes())
print(
    "wrote", OUT,
    "grid", (w, h),
    "cell", CELL,
    "origin", (minx, miny),
    "max height", float(heights.max()),
    "buildings cells", int((heights > 0.5).sum()),
    "size", os.path.getsize(OUT),
)
