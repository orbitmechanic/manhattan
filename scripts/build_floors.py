"""Generate human-scale floors and the real-geometry viewer mesh.

From manhattan_blocks.blend:
  manhattan_floors.blend                    floor-detail art master
  viewer/assets/models/manhattan_city.glb   per-block floor geometry
  viewer/assets/models/manhattan_ground.glb always-visible ground

There are no merged-region or box-proxy LOD meshes: the viewer renders these
real floor meshes and only culls them at the fog distance.

Floor height 3 world units; facade U module 12 units; roof tile 8 units.
One-floor texture crops come from scripts/make_floor_textures.py.
"""
import bpy, bmesh, math, os
from mathutils import Vector

ROOT = "/home/orbit9/Work/games/Manhattan"
FLOOR_DAY = os.path.join(ROOT, "Textures/day/floors")
ROOF_DAY = os.path.join(ROOT, "Textures/day/roofs/roof_concrete_gravel.jpg")
OUT_BLEND = os.path.join(ROOT, "manhattan_floors.blend")
OUT_DIR = os.path.join(ROOT, "viewer", "assets", "models")
MAX_BLOCKS = 1536
FLOOR_H = 3.0


MODULE_W = 12.0
ROOF_TILE = 8.0

city = bpy.data.collections["City_Blocks"]
block_objs = [
    o for o in city.objects
    if o.type == "MESH" and o.name.startswith("Block_") and "block_index" in o
]
ground = bpy.data.objects["Ground_Blocks"]
parks = bpy.data.collections["Parks"]
print("block objects", len(block_objs))

# ---------------------------------------------------------------------------
# material sets
# ---------------------------------------------------------------------------
def new_image_material(name, path, roughness=0.6):
    mat = bpy.data.materials.new(name)
    nt = mat.node_tree
    nt.nodes.clear()
    out = nt.nodes.new("ShaderNodeOutputMaterial")
    bsdf = nt.nodes.new("ShaderNodeBsdfPrincipled")
    tex = nt.nodes.new("ShaderNodeTexImage")
    tex.image = bpy.data.images.load(path, check_existing=True)
    nt.links.new(tex.outputs["Color"], bsdf.inputs["Base Color"])
    nt.links.new(bsdf.outputs["BSDF"], out.inputs["Surface"])
    bsdf.inputs["Roughness"].default_value = roughness
    return mat


lod0_mats = {}      # original name -> floor material
for m in bpy.data.materials:
    if m.name.startswith(("LOD0_", "LOD1_", "LOD2_")):
        continue
    if m.name.startswith("Facade_"):
        path = os.path.join(FLOOR_DAY, m.name.lower() + "_floor.jpg")
        if not os.path.exists(path):
            print("missing floor texture", path)
            continue
        lod0_mats[m.name] = new_image_material("LOD0_" + m.name, path, 0.55)
lod0_roof = new_image_material("LOD0_Roof_Concrete_Gravel", ROOF_DAY, 0.92)
print("floor facade mats", len(lod0_mats))

# ---------------------------------------------------------------------------
# floor geometry
# ---------------------------------------------------------------------------
old_floor = bpy.data.collections.get("Floor_Blocks")
if old_floor:
    bpy.data.collections.remove(old_floor)
floor_coll = bpy.data.collections.new("Floor_Blocks")
bpy.context.scene.collection.children.link(floor_coll)


def process_block(ob):
    idx = int(ob.get("block_index", 0))
    me = ob.data
    bm = bmesh.new()
    bm.from_mesh(me)
    bm.normal_update()
    zmax = max(v.co.z for v in bm.verts)
    zmin = min(v.co.z for v in bm.verts)
    levels = [k * FLOOR_H for k in range(1, int(math.ceil(zmax / FLOOR_H)))]
    for z in levels:
        bmesh.ops.bisect_plane(
            bm,
            geom=list(bm.verts) + list(bm.edges) + list(bm.faces),
            dist=1e-4,
            plane_co=(0.0, 0.0, z),
            plane_no=(0.0, 0.0, 1.0),
            clear_inner=False,
            clear_outer=False,
        )
    bm.normal_update()
    bm.faces.ensure_lookup_table()
    uv = bm.loops.layers.uv.verify()

    # Build the floor material slot list actually used.
    slot_names = []
    name_to_slot = {}
    for name in list(lod0_mats.keys()):
        name_to_slot[name] = len(slot_names)
        slot_names.append(name)
    name_to_slot["Roof_Concrete_Gravel"] = len(slot_names)
    slot_names.append("Roof_Concrete_Gravel")

    for f in bm.faces:
        c = f.calc_center_median()
        normal = f.normal
        if abs(normal.z) < 0.5:
            orig = me.materials[f.material_index].name if me.materials else ""
            target = orig if orig in lod0_mats else "Roof_Concrete_Gravel"
            f.material_index = name_to_slot[target]
            d = Vector((-normal.y, normal.x, 0.0))
            if d.length < 1e-5:
                d = Vector((1.0, 0.0, 0.0))
            d.normalize()
            z0 = math.floor(c.z / FLOOR_H) * FLOOR_H
            for loop in f.loops:
                co = loop.vert.co
                loop[uv].uv = (
                    (co.x * d.x + co.y * d.y) / MODULE_W,
                    (co.z - z0) / FLOOR_H,
                )
        else:
            f.material_index = name_to_slot["Roof_Concrete_Gravel"]
            for loop in f.loops:
                co = loop.vert.co
                loop[uv].uv = (co.x / ROOF_TILE, co.y / ROOF_TILE)

    new_me = bpy.data.meshes.new(f"Block_{idx:04d}_Floors")
    bm.to_mesh(new_me)
    bm.free()
    for n in slot_names:
        mat = lod0_mats.get(n) or lod0_roof
        new_me.materials.append(mat)

    # block id in COLOR_0, green faces per floor in a FACE attribute
    col = new_me.color_attributes.new(name="block_id", type="FLOAT_COLOR", domain="CORNER")
    a = (float(idx) + 0.5) / MAX_BLOCKS
    for p in new_me.polygons:
        for li in p.loop_indices:
            col.data[li].color = (1.0, 1.0, 1.0, a)
    new_me.color_attributes.active_color = col
    floor_attr = new_me.attributes.new(name="floor", type="FLOAT", domain="FACE")
    for i, p in enumerate(new_me.polygons):
        floor_attr.data[i].value = math.floor(p.center.z / FLOOR_H)

    # object origin at building-block centre
    cx = float(ob.get("centroid_x", 0.0))
    cy = float(ob.get("centroid_y", 0.0))
    cz = (zmin + zmax) * 0.5
    for v in new_me.vertices:
        v.co.x -= cx
        v.co.y -= cy
        v.co.z -= cz
    new_me.validate(verbose=False)
    new_ob = bpy.data.objects.new(f"Floor_Block_{idx:04d}", new_me)
    new_ob.location = (cx, cy, cz)
    new_ob["block_index"] = idx
    new_ob["block_type"] = "building"
    new_ob["floor_height"] = FLOOR_H
    floor_coll.objects.link(new_ob)
    return new_ob, len(new_me.polygons)


total_faces = 0
for ob in block_objs:
    _, nf = process_block(ob)
    total_faces += nf
print("floor objects", len(floor_coll.objects), "faces", total_faces)

# hide the block-master buildings; leave ground/parks visible
for ob in city.objects:
    if ob.type == "MESH" and ob.name != "Ground_Blocks":
        ob.hide_viewport = True
        ob.hide_render = True
        ob.hide_set(True)

# art master collections
floor_coll.name = "Floor_Blocks"
bpy.context.scene["floor_height"] = FLOOR_H
bpy.context.scene["floor_texture_module"] = MODULE_W
bpy.context.scene["roof_tile"] = ROOF_TILE
bpy.context.scene["lod_levels"] = 3
text = bpy.data.texts.get("FLOOR_FRAMEWORK") or bpy.data.texts.new("FLOOR_FRAMEWORK")
text.clear()
text.write(
    "Human-scale floor master\n"
    "========================\n"
    f"Floor height: {FLOOR_H} world units\n"
    f"Facade U module: {MODULE_W} units (one window module)\n"
    f"Roof UV tile: {ROOF_TILE} units\n\n"
    "Each block object is divided horizontally into floor bands.  Facade walls\n"
    "use a one-floor texture crop (one row of windows), V=0..1 per floor and U\n"
    "wrapped around the perimeter.  Roofs use world-scale UVs.\n\n"
    "A face attribute 'floor' records the floor index.  Select a block and use\n"
    "Mesh > Separate > By Loose Parts to get individual floor shells for detailed\n"
    "editing.\n\n"
    "Viewer GLBs (exported separately):\n"
    "  manhattan_city.glb  real per-block floor geometry\n"
    "  manhattan_ground.glb always visible ground\n"
    "The viewer culls real geometry at the fog distance; it never substitutes\n"
    "merged regions or larger box proxies.\n"
)
bpy.ops.wm.save_as_mainfile(filepath=OUT_BLEND, compress=True)
print("saved", OUT_BLEND)

# ---------------------------------------------------------------------------
# export helpers
# ---------------------------------------------------------------------------
def export_glb(objs, name):
    path = os.path.join(OUT_DIR, name)
    bpy.ops.object.select_all(action="DESELECT")
    for ob in objs:
        if ob.name in bpy.data.objects:
            ob.select_set(True)
    if not objs:
        print("no objects for", name)
        return
    bpy.context.view_layer.objects.active = objs[0]
    bpy.ops.export_scene.gltf(
        filepath=path,
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
    print("wrote", path, os.path.getsize(path))


export_glb(list(floor_coll.objects), "manhattan_city.glb")
export_glb([ground], "manhattan_ground.glb")

print("done")
