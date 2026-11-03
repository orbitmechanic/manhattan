import bpy, bmesh, math, os
from mathutils import Vector, geometry

ROOT = "/home/orbit9/Work/games/Manhattan"
FLOOR_DAY = os.path.join(ROOT, "Textures/day/floors")
ROOF_DAY = os.path.join(ROOT, "Textures/day/roofs/roof_concrete_gravel.jpg")
MAX_BLOCKS = 1536
FLOOR_H = 3.0
MODULE_W = 12.0
ROOF_TILE = 8.0

city = bpy.data.collections["City_Blocks"]
src = bpy.data.objects["Block_0501"]

# --- materials --------------------------------------------------------------
lod0_mats = {}
facade_map = {}
for m in bpy.data.materials:
    if m.name.startswith(("LOD0_", "TEST_")):
        continue
    if m.name.startswith("Facade_"):
        path = os.path.join(FLOOR_DAY, m.name.lower() + "_floor.jpg")
        if not os.path.exists(path):
            print("missing floor", path)
            continue
        mat = bpy.data.materials.new("LOD0_" + m.name)
        nt = mat.node_tree
        nt.nodes.clear()
        out = nt.nodes.new("ShaderNodeOutputMaterial")
        bsdf = nt.nodes.new("ShaderNodeBsdfPrincipled")
        tex = nt.nodes.new("ShaderNodeTexImage")
        tex.image = bpy.data.images.load(path, check_existing=True)
        nt.links.new(tex.outputs["Color"], bsdf.inputs["Base Color"])
        nt.links.new(bsdf.outputs["BSDF"], out.inputs["Surface"])
        bsdf.inputs["Roughness"].default_value = 0.55
        lod0_mats["LOD0_" + m.name] = mat
        facade_map[m.name] = "LOD0_" + m.name
# roof
mat = bpy.data.materials.new("LOD0_Roof_Concrete_Gravel")
nt = mat.node_tree
nt.nodes.clear()
out = nt.nodes.new("ShaderNodeOutputMaterial")
bsdf = nt.nodes.new("ShaderNodeBsdfPrincipled")
tex = nt.nodes.new("ShaderNodeTexImage")
tex.image = bpy.data.images.load(ROOF_DAY, check_existing=True)
nt.links.new(tex.outputs["Color"], bsdf.inputs["Base Color"])
nt.links.new(bsdf.outputs["BSDF"], out.inputs["Surface"])
bsdf.inputs["Roughness"].default_value = 0.9
lod0_mats["LOD0_Roof_Concrete_Gravel"] = mat

# --- floor mesh -------------------------------------------------------------
me = src.data
bm = bmesh.new()
bm.from_mesh(me)
bm.normal_update()
zmax = max(v.co.z for v in bm.verts)
zmin = min(v.co.z for v in bm.verts)
levels = [k * FLOOR_H for k in range(1, int(math.ceil(zmax / FLOOR_H)))]
for z in levels:
    geom = list(bm.verts) + list(bm.edges) + list(bm.faces)
    bmesh.ops.bisect_plane(
        bm, geom=geom, dist=1e-4,
        plane_co=(0.0, 0.0, z), plane_no=(0.0, 0.0, 1.0),
        clear_inner=False, clear_outer=False,
    )
bm.normal_update()
# split horizontal floor-level edges so bands are separate islands
cut_edges = []
for e in bm.edges:
    a, b = e.verts
    if abs(a.co.z - b.co.z) < 1e-4:
        z = (a.co.z + b.co.z) * 0.5
        if abs(z / FLOOR_H - round(z / FLOOR_H)) < 1e-3:
            fs = list(e.link_faces)
            if len(fs) == 2:
                f0, f1 = fs
                if math.floor(f0.calc_center_median().z / FLOOR_H) != math.floor(f1.calc_center_median().z / FLOOR_H):
                    cut_edges.append(e)
bmesh.ops.split_edges(bm, edges=cut_edges)
bm.normal_update()
bm.normal_update()
bm.faces.ensure_lookup_table()
# Cap each disconnected floor shell with a convex-hull floor/ceiling plate.
seen = set()
components = []
for f in bm.faces:
    if f.index in seen:
        continue
    stack = [f]
    seen.add(f.index)
    comp = []
    while stack:
        cur = stack.pop()
        comp.append(cur)
        for e in cur.edges:
            for n in e.link_faces:
                if n.index not in seen:
                    seen.add(n.index)
                    stack.append(n)
    components.append(comp)
print("floor components", len(components))
cap_count = 0
for comp in components:
    zs = [v.co.z for f in comp for v in f.verts]
    z0, z1 = min(zs), max(zs)
    if z1 - z0 < 0.01:
        continue
    for z in (z0, z1):
        vs = [v for f in comp for v in f.verts if abs(v.co.z - z) < 1e-3]
        uniq = {}
        for v in vs:
            uniq[(round(v.co.x, 4), round(v.co.y, 4))] = v
        if len(uniq) < 3:
            continue
        pts = [Vector((x, y)) for x, y in uniq.keys()]
        hull = geometry.convex_hull_2d(pts)
        hverts = []
        for p in hull:
            key = (round(p[0], 4), round(p[1], 4))
            if key in uniq:
                hverts.append(uniq[key])
        if len(hverts) >= 3:
            try:
                bm.faces.new(hverts)
                cap_count += 1
            except ValueError:
                pass
print("caps", cap_count)
bm.normal_update()
bm.faces.ensure_lookup_table()

# material slots
slot_names = []
slot_index = {}
for name in list(lod0_mats.keys()):
    slot_index[name] = len(slot_names)
    slot_names.append(name)

uv = bm.loops.layers.uv.verify()
cap_faces = set()
for f in bm.faces:
    c = f.calc_center_median()
    n = f.normal
    if abs(n.z) < 0.5:
        orig = me.materials[f.material_index].name
        name = facade_map.get(orig, "LOD0_Roof_Concrete_Gravel")
        f.material_index = slot_index[name]
        d = Vector((-n.y, n.x, 0.0)).normalized()
        z0 = math.floor(c.z / FLOOR_H) * FLOOR_H
        for loop in f.loops:
            co = loop.vert.co
            loop[uv].uv = ((co.x * d.x + co.y * d.y) / MODULE_W, (co.z - z0) / FLOOR_H)
    else:
        f.material_index = slot_index["LOD0_Roof_Concrete_Gravel"]
        for loop in f.loops:
            co = loop.vert.co
            loop[uv].uv = (co.x / ROOF_TILE, co.y / ROOF_TILE)

new_me = bpy.data.meshes.new("TEST_Floors")
bm.to_mesh(new_me)
bm.free()
for name in slot_names:
    new_me.materials.append(lod0_mats[name])
# per-face block color
col = new_me.color_attributes.new(name="block_id", type="FLOAT_COLOR", domain="CORNER")
a = (501 + 0.5) / MAX_BLOCKS
for p in new_me.polygons:
    for li in p.loop_indices:
        col.data[li].color = (1.0, 1.0, 1.0, a)
new_me.color_attributes.active_color = col

# origin at block centre
cx = sum(v.co.x for v in me.vertices) / len(me.vertices)
cy = sum(v.co.y for v in me.vertices) / len(me.vertices)
cz = (zmin + zmax) * 0.5
for v in new_me.vertices:
    v.co.x -= cx
    v.co.y -= cy
    v.co.z -= cz
ob = bpy.data.objects.new("TEST_Floors", new_me)
bpy.context.scene.collection.objects.link(ob)
ob.location = (cx, cy, cz)

# hide everything else for a close render
for o in bpy.data.objects:
    if o is not ob and o.type in ('MESH',):
        o.hide_render = True

cam = bpy.data.objects.get("Preview_Camera")
if cam:
    cam.location = (cx + 260, cy - 260, cz + 40)
    target = Vector((cx, cy, cz + 5))
    cam.rotation_euler = (target - cam.location).to_track_quat('-Z', 'Y').to_euler()
sc = bpy.context.scene
sc.camera = cam
sc.render.engine = 'BLENDER_EEVEE'
sc.render.resolution_x = 1200
sc.render.resolution_y = 900
sc.view_settings.view_transform = 'Standard'
sc.view_settings.exposure = -0.2
sc.render.filepath = "/home/orbit9/Work/games/Manhattan/renders/test_floors.png"
bpy.ops.render.render(write_still=True)
print("faces", len(new_me.polygons), "floor levels", len(levels), "render done")
