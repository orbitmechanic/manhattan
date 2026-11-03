import bpy, os, sys
bpy.ops.wm.read_factory_settings(use_empty=True)
p = "/home/orbit9/Work/games/Manhattan/manhattan.fbx"
bpy.ops.import_scene.fbx(filepath=p)
print("=== FBX IMPORT ===")
print("objects:", len(bpy.data.objects), "meshes:", len(bpy.data.meshes),
      "materials:", len(bpy.data.materials), "images:", len(bpy.data.images))
for ob in bpy.data.objects:
    extra=""
    if ob.type=='MESH':
        extra=f" verts={len(ob.data.vertices)} polys={len(ob.data.polygons)} mats={[m.name if m else None for m in ob.data.materials]}"
    print(f"  {ob.name!r} {ob.type} parent={ob.parent.name if ob.parent else None} loc={tuple(round(v,2) for v in ob.location)} scale={tuple(round(v,3) for v in ob.scale)}{extra}")
print("--- images")
for im in bpy.data.images:
    print(f"  {im.name!r} {im.filepath!r} size={tuple(im.size)} packed={bool(im.packed_file)}")
print("--- materials")
for m in bpy.data.materials:
    print(" ", m.name, "users", m.users, "nodes", [(n.type,n.name) for n in m.node_tree.nodes] if m.use_nodes else None)
    if m.use_nodes:
        for n in m.node_tree.nodes:
            if n.type=='TEX_IMAGE':
                print("    tex", n.image.name if n.image else None, n.image.filepath if n.image else None)
# dimensions
import mathutils
for ob in bpy.data.objects:
    if ob.type=='MESH':
        vs=[ob.matrix_world @ v.co for v in ob.data.vertices]
        if vs:
            xs=[v.x for v in vs]; ys=[v.y for v in vs]; zs=[v.z for v in vs]
            print("BBOX", ob.name, (min(xs),min(ys),min(zs)), (max(xs),max(ys),max(zs)))
