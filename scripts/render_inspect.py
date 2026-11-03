import bpy, math, os
from mathutils import Vector

sc = bpy.context.scene
sc.render.engine = 'BLENDER_EEVEE'
sc.render.resolution_x = 1280
sc.render.resolution_y = 720
sc.render.film_transparent = False
sc.view_settings.view_transform = 'Standard'

# world sky
w = bpy.data.worlds[0]
w.use_nodes = True
bg = w.node_tree.nodes.get('Background')
if bg:
    bg.inputs[0].default_value = (0.48, 0.62, 0.82, 1)
    bg.inputs[1].default_value = 1.0

# sun
light_data = bpy.data.lights.new('SunInspect', type='SUN')
light_data.energy = 4.0
sun = bpy.data.objects.new('SunInspect', light_data)
sc.collection.objects.link(sun)
sun.rotation_euler = (math.radians(50), 0, math.radians(35))

def make_cam(name, loc, look):
    cd = bpy.data.cameras.new(name)
    cd.lens = 35
    cd.clip_end = 30000
    cam = bpy.data.objects.new(name, cd)
    sc.collection.objects.link(cam)
    cam.location = loc
    d = Vector(look) - Vector(loc)
    cam.rotation_euler = d.to_track_quat('-Z', 'Y').to_euler()
    return cam

views = {
    'iso': ((3800, -5200, 2600), (1200, 200, 100)),
    'top': ((900, 0, 6000), (900, 0, 0)),
    'street': ((900, -1200, 25), (900, 800, 60)),
    'close': ((1500, -900, 150), (1600, 0, 130)),
}
for name, (loc, look) in views.items():
    cam = make_cam(name, loc, look)
    sc.camera = cam
    sc.render.filepath = f"/home/orbit9/Work/games/Manhattan/renders/inspect_{name}.png"
    bpy.ops.render.render(write_still=True)
    print("rendered", sc.render.filepath)
