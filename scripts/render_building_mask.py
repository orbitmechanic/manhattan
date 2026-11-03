import bpy, os, math

ROOT = "/home/orbit9/Work/games/Manhattan"
OUT = "/home/orbit9/Work/games/Manhattan/build/building_mask.png"
bld = bpy.data.objects["Manhattan_Buildings"]
gnd = bpy.data.objects["Manhattan_Ground"]
gnd.hide_render = True

# union bounds of building + ground
xs = [v.co.x for v in bld.data.vertices] + [v.co.x for v in gnd.data.vertices]
ys = [v.co.y for v in bld.data.vertices] + [v.co.y for v in gnd.data.vertices]
minx, maxx = min(xs), max(xs)
miny, maxy = min(ys), max(ys)
w, h = maxx - minx, maxy - miny
res_x = int(round(w / 2.0))
res_y = int(round(h / 2.0))

cd = bpy.data.cameras.new("MaskCam")
cd.type = 'ORTHO'
cd.ortho_scale = w
cd.clip_start = 1.0
cd.clip_end = 10000.0
cam = bpy.data.objects.new("MaskCam", cd)
bpy.context.scene.collection.objects.link(cam)
cam.location = ((minx + maxx) / 2.0, (miny + maxy) / 2.0, 2000.0)
cam.rotation_euler = (0.0, 0.0, 0.0)
sc = bpy.context.scene
sc.camera = cam
sc.render.engine = 'BLENDER_EEVEE'
sc.render.resolution_x = res_x
sc.render.resolution_y = res_y
sc.render.resolution_percentage = 100
sc.render.film_transparent = True
sc.render.image_settings.file_format = 'PNG'
sc.render.image_settings.color_mode = 'RGBA'
sc.view_settings.view_transform = 'Standard'
sc.render.filepath = OUT
bpy.ops.render.render(write_still=True)
print("MASK", OUT, res_x, res_y, "bounds", minx, maxx, miny, maxy)
