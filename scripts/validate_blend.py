import bpy, os
ROOT = "/home/orbit9/Work/games/Manhattan"
print("BLEND", bpy.data.filepath)
missing = []
for im in bpy.data.images:
    if im.source != 'FILE':
        continue
    p = bpy.path.abspath(im.filepath)
    ok = os.path.exists(p) and im.size[0] > 0
    print(f"  {'OK' if ok else 'MISSING'} {im.name} {im.size[:]} {im.filepath}")
    if not ok:
        missing.append(im.name)
for ob in bpy.data.objects:
    if ob.type == 'MESH':
        print(f"  OBJECT {ob.name}: {len(ob.data.vertices)} verts, {len(ob.data.polygons)} polys, "
              f"materials={len(ob.data.materials)}")
print("MATERIALS", sorted(m.name for m in bpy.data.materials))
print("MISSING", missing)
assert not missing
