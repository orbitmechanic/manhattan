import bpy, os, math
from mathutils import Vector

ROOT = "/home/orbit9/Work/games/Manhattan"
NIGHT = os.path.join(ROOT, "Textures", "night")
sc = bpy.context.scene

def night_path_for(name):
    n = name.lower()
    if "ground" in n or "satellite" in n:
        return os.path.join(NIGHT, "ground", "satellite_manhattan_ground_night.jpg")
    if "roof" in n:
        return os.path.join(NIGHT, "roofs", "roof_concrete_gravel_night.jpg")
    # facade
    return os.path.join(NIGHT, "facades", name + "_night.jpg")

for m in bpy.data.materials:
    if not m.use_nodes:
        continue
    tex = next((n for n in m.node_tree.nodes if n.type == 'TEX_IMAGE'), None)
    p = next((n for n in m.node_tree.nodes if n.type == 'BSDF_PRINCIPLED'), None)
    if not tex or not p or not tex.image:
        continue
    day_name = tex.image.name
    # derive logical name without _night
    logical = day_name
    pth = night_path_for(logical)
    if not os.path.exists(pth):
        print("MISSING NIGHT", pth)
        continue
    nim = bpy.data.images.get("night_" + logical)
    if not nim:
        nim = bpy.data.images.load(pth, check_existing=True)
        nim.name = "night_" + logical
    tex.image = nim
    # make lit windows emissive in the preview
    p.inputs['Emission Strength'].default_value = 1.6
    if not p.inputs['Emission Color'].is_linked:
        m.node_tree.links.new(tex.outputs['Color'], p.inputs['Emission Color'])
    print(m.name, "->", os.path.basename(pth))

# moon light
sun = bpy.data.objects.get('Preview_Sun')
if sun:
    sun.data.energy = 0.25
    sun.data.color = (0.62, 0.72, 1.0)
    sun.rotation_euler = (math.radians(58), 0, math.radians(210))

# dark blue world
w = sc.world
bg = w.node_tree.nodes.get('Background')
if bg:
    for l in list(bg.inputs['Color'].links):
        w.node_tree.links.remove(l)
    bg.inputs['Color'].default_value = (0.008, 0.012, 0.030, 1)
    bg.inputs['Strength'].default_value = 0.35

sc.view_settings.view_transform = 'Standard'
sc.view_settings.exposure = 0.2
sc.render.resolution_x = 1600
sc.render.resolution_y = 900

sc.render.filepath = os.path.join(ROOT, "renders", "clean_night_wide.png")
bpy.ops.render.render(write_still=True)

# close-up street view
cd = bpy.data.cameras.new("NightCam")
cd.lens = 35
cd.clip_end = 40000
cam = bpy.data.objects.new("NightCam", cd)
sc.collection.objects.link(cam)
cam.location = (900, -900, 110)
target = Vector((1100, 400, 150))
cam.rotation_euler = (target - cam.location).to_track_quat('-Z', 'Y').to_euler()
sc.camera = cam
sc.render.filepath = os.path.join(ROOT, "renders", "clean_night_street.png")
bpy.ops.render.render(write_still=True)
print("done")
