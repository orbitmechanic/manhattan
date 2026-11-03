"""Build the irregular city-block art master and viewer GLB.

Inputs (from scripts/segment_blocks.py):
  build/blocks_labels.npy
  build/blocks.json
  build/ground_cells.npz

Art master:
  City_Blocks/Block_0000 ... one object per building block (all its faces)
  City_Blocks/Ground_Blocks  Voronoi ground mesh with per-face block ids
  Parks/Park_0000 ... empties for open-space blocks
  _Source_HIDDEN/ original single building + ground

The viewer GLB is produced separately by scripts/export_blocks_viewer.py
(11 per-material meshes + ground, block ids baked into COLOR_0 alpha).
"""
import bpy, bmesh, os, json
import numpy as np
from collections import defaultdict

ROOT = "/home/orbit9/Work/games/Manhattan"
BUILD = os.path.join(ROOT, "build")
MAX_BLOCKS = 1536
OUT_BLEND = os.path.join(ROOT, "manhattan_blocks.blend")
OUT_GLB = os.path.join(ROOT, "viewer", "assets", "models", "manhattan_blocks.glb")

meta = json.load(open(os.path.join(BUILD, "blocks.json")))
labels = np.load(os.path.join(BUILD, "blocks_labels.npy"))
H, W = labels.shape
MINX, MAXX = meta["bounds"]["min_x"], meta["bounds"]["max_x"]
MINY, MAXY = meta["bounds"]["min_y"], meta["bounds"]["max_y"]
GMINX = meta["ground_bounds"]["min_x"]
GMAXX = meta["ground_bounds"]["max_x"]
GMINY = meta["ground_bounds"]["min_y"]
GMAXY = meta["ground_bounds"]["max_y"]
N_BUILDING = meta["building_blocks"]
N_PARK = meta["park_blocks"]
N_BLOCK = meta["block_count"]
print("blocks", N_BLOCK, "buildings", N_BUILDING, "parks", N_PARK)


def block_at(x, y):
    px = int((x - MINX) / (MAXX - MINX) * W)
    py = int((MAXY - y) / (MAXY - MINY) * H)
    px = min(max(px, 0), W - 1)
    py = min(max(py, 0), H - 1)
    lab = int(labels[py, px])
    if lab:
        return lab - 1
    # search a small window for a non-zero label
    for rad in (2, 5, 10):
        y0, y1 = max(0, py - rad), min(H, py + rad + 1)
        x0, x1 = max(0, px - rad), min(W, px + rad + 1)
        patch = labels[y0:y1, x0:x1]
        nz = patch[patch > 0]
        if nz.size:
            return int(nz[0]) - 1
    return 0


# ---------------------------------------------------------------------------
# collections / source
# ---------------------------------------------------------------------------
scene = bpy.context.scene
src_bld = bpy.data.objects["Manhattan_Buildings"]
src_gnd = bpy.data.objects["Manhattan_Ground"]

for name in ("City_Blocks", "Parks", "_Source_HIDDEN", "_Viewer_Export"):
    c = bpy.data.collections.get(name)
    if c:
        bpy.data.collections.remove(c)
city_coll = bpy.data.collections.new("City_Blocks")
parks_coll = bpy.data.collections.new("Parks")
source = bpy.data.collections.new("_Source_HIDDEN")
scene.collection.children.link(city_coll)
scene.collection.children.link(parks_coll)
scene.collection.children.link(source)

for ob in (src_bld, src_gnd):
    for c in list(ob.users_collection):
        c.objects.unlink(ob)
    source.objects.link(ob)
    ob.hide_set(True)
    ob.hide_viewport = True
    ob.hide_render = True

# ---------------------------------------------------------------------------
# face -> block assignment
# ---------------------------------------------------------------------------
me = src_bld.data
uv_layer = me.uv_layers.active
uv_data = uv_layer.data
face_block = np.zeros(len(me.polygons), dtype=np.int32)
for i, p in enumerate(me.polygons):
    c = p.center
    face_block[i] = block_at(c.x, c.y)
print("faces", len(face_block), "unassigned", int((face_block < 0).sum()))

blocks_faces = defaultdict(list)
for i, b in enumerate(face_block):
    blocks_faces[int(b)].append(i)

# ---------------------------------------------------------------------------
# art master: one object per block
# ---------------------------------------------------------------------------
for idx in range(N_BUILDING):
    face_ids = blocks_faces.get(idx, [])
    if not face_ids:
        continue
    verts = []
    faces = []
    uvs = []
    mat_ids = []
    smooths = []
    mats = []
    mat_map = {}
    for fi in face_ids:
        p = me.polygons[fi]
        base = len(verts)
        for li in p.loop_indices:
            vi = me.loops[li].vertex_index
            verts.append(tuple(me.vertices[vi].co))
            uvs.append(tuple(uv_data[li].uv))
        faces.append(tuple(range(base, base + len(p.loop_indices))))
        mi = p.material_index
        if mi not in mat_map:
            mat_map[mi] = len(mats)
            mats.append(me.materials[mi])
        mat_ids.append(mat_map[mi])
        smooths.append(p.use_smooth)
    name = f"Block_{idx:04d}"
    new_me = bpy.data.meshes.new(name)
    new_me.from_pydata(verts, [], faces)
    new_me.update()
    ul = new_me.uv_layers.new(name="UVMap")
    for k, uv in enumerate(uvs):
        ul.data[k].uv = uv
    for m in mats:
        new_me.materials.append(m)
    for k, poly in enumerate(new_me.polygons):
        poly.material_index = mat_ids[k]
        poly.use_smooth = smooths[k]
    new_me.validate(verbose=False)
    ob = bpy.data.objects.new(name, new_me)
    ob["block_index"] = idx
    ob["block_type"] = "building"
    b = meta["blocks"][idx]
    ob["centroid_x"] = b["centroid"][0]
    ob["centroid_y"] = b["centroid"][1]
    ob["area_px"] = b["area_px"]
    city_coll.objects.link(ob)
print("block objects:", len([o for o in city_coll.objects if o.type == 'MESH']) - 0)

# park empties
for j in range(N_PARK):
    idx = N_BUILDING + j
    b = meta["blocks"][idx]
    emp = bpy.data.objects.new(f"Park_{j:04d}", None)
    emp.empty_display_type = "PLAIN_AXES"
    emp.empty_display_size = 60.0
    emp.location = (b["centroid"][0], b["centroid"][1], 0.0)
    emp["block_index"] = idx
    emp["block_type"] = "park"
    emp["area_px"] = b["area_px"]
    parks_coll.objects.link(emp)
print("park empties:", N_PARK)

# ---------------------------------------------------------------------------
# ground mesh: Voronoi cells, with block id in COLOR_0 alpha
# ---------------------------------------------------------------------------
gd = np.load(os.path.join(BUILD, "ground_cells.npz"))
gverts = gd["vertices"]
goffsets = gd["offsets"]
gblocks = gd["block_ids"]
faces = [
    tuple(range(int(goffsets[i]), int(goffsets[i + 1])))
    for i in range(len(gblocks))
]
gme = bpy.data.meshes.new("Ground_Blocks")
gme.from_pydata([(float(x), float(y), 0.0) for x, y in gverts], [], faces)
gme.update()
guv = gme.uv_layers.new(name="rastUVmap")
uv_min_u = 0.1823
for poly in gme.polygons:
    for li in poly.loop_indices:
        vi = gme.loops[li].vertex_index
        x, y = gverts[vi]
        guv.data[li].uv = (
            uv_min_u + (float(x) - GMINX) / (GMAXX - GMINX) * (1.0 - uv_min_u),
            (float(y) - GMINY) / (GMAXY - GMINY),
        )
col = gme.color_attributes.new(name="block_id", type="FLOAT_COLOR", domain="CORNER")
for i, poly in enumerate(gme.polygons):
    a = (float(gblocks[i]) + 0.5) / MAX_BLOCKS
    for li in poly.loop_indices:
        col.data[li].color = (1.0, 1.0, 1.0, a)
gme.color_attributes.active_color = col
gmat = bpy.data.materials["Ground_Satellite_Manhattan"]
gme.materials.append(gmat)
gme.validate(verbose=False)
gob = bpy.data.objects.new("Ground_Blocks", gme)
gob["block_type"] = "ground"
city_coll.objects.link(gob)
print("ground polygons:", len(gblocks), "verts:", len(gverts))

# ---------------------------------------------------------------------------
# notes / save art master
# ---------------------------------------------------------------------------
text = bpy.data.texts.get("BLOCK_FRAMEWORK") or bpy.data.texts.new("BLOCK_FRAMEWORK")
text.clear()
text.write(
    "Manhattan irregular city-block framework\n"
    "========================================\n\n"
    f"Building blocks: {N_BUILDING}\n"
    f"Park/open-space blocks: {N_PARK}\n"
    f"Total: {N_BLOCK}\n\n"
    "Blocks were derived from building footprints: footprints were dilated by\n"
    "8 world units and connected components became blocks.  Large interior\n"
    "open areas became park blocks.  Block boundaries therefore follow streets\n"
    "and alleys, not a regular grid.\n\n"
    "Art master layout:\n"
    "  City_Blocks/Block_0000 ...   one mesh per building block (multi-material)\n"
    "  City_Blocks/Ground_Blocks    Voronoi ground, boundaries mid-street\n"
    "  Parks/Park_0000 ...          empties at park block centres\n"
    "  _Source_HIDDEN/              original single building + ground\n\n"
    "Each object has block_index / block_type / centroid custom properties.\n"
    "The viewer GLB bakes block ids into COLOR_0 alpha and merges geometry back\n"
    "to 11 materials + 1 ground mesh; the shader reads the id from vertex alpha.\n"
    "The art-master Block objects keep their index in the block_index property.\n"
    "Re-run scripts/segment_blocks.py then this script to regenerate.\n"
)

scene["block_count"] = N_BLOCK
scene["building_blocks"] = N_BUILDING
scene["park_blocks"] = N_PARK
scene["max_blocks"] = MAX_BLOCKS
scene["block_min_x"] = MINX
scene["block_max_x"] = MAXX
scene["block_min_y"] = MINY
scene["block_max_y"] = MAXY

bpy.ops.wm.save_as_mainfile(filepath=OUT_BLEND, compress=True)
print("saved", OUT_BLEND)

print("art master saved; run scripts/export_blocks_viewer.py to rebuild the viewer GLB")
