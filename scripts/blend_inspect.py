import bpy, os, sys

print("=== BLENDER", bpy.app.version_string, "===")
print("file:", bpy.data.filepath)
print("objects:", len(bpy.data.objects), " meshes:", len(bpy.data.meshes),
      " materials:", len(bpy.data.materials), " images:", len(bpy.data.images),
      " collections:", len(bpy.data.collections), " worlds:", len(bpy.data.worlds))

print("\n=== COLLECTIONS ===")
for c in bpy.data.collections:
    print(f"  {c.name!r}: objs={len(c.objects)} children={[x.name for x in c.children]}")

print("\n=== OBJECTS ===")
for ob in bpy.data.objects:
    extra = ""
    if ob.type == 'MESH':
        extra = f" verts={len(ob.data.vertices)} polys={len(ob.data.polygons)} mats={[m.name if m else None for m in ob.data.materials]}"
    print(f"  {ob.name!r} type={ob.type} parent={ob.parent.name if ob.parent else None} loc={tuple(round(v,2) for v in ob.location)} scale={tuple(round(v,2) for v in ob.scale)}{extra}")

print("\n=== IMAGES ===")
for im in bpy.data.images:
    print(f"  {im.name!r} file={im.filepath!r} src={im.source} size={tuple(im.size)} packed={bool(im.packed_file)} users={im.users}")

print("\n=== MATERIALS ===")
for m in bpy.data.materials:
    if not m.use_nodes:
        print(f"  {m.name!r} NO NODES users={m.users}")
        continue
    print(f"  {m.name!r} users={m.users} nodes={[ (n.type, n.name) for n in m.node_tree.nodes ]}")
    for n in m.node_tree.nodes:
        if n.type == 'TEX_IMAGE':
            print(f"     TEX_IMAGE node {n.name!r} image={n.image.name if n.image else None} file={n.image.filepath if n.image else None}")

print("\n=== WORLDS ===")
for w in bpy.data.worlds:
    print(" ", w.name, "use_nodes", w.use_nodes)
    if w.use_nodes:
        for n in w.node_tree.nodes:
            print("   ", n.type, n.name)

print("\n=== SCENE ===")
sc = bpy.context.scene
print("camera:", sc.camera, "engine:", sc.render.engine)
print("frame:", sc.frame_start, sc.frame_end)
print("unit scale:", sc.unit_settings.scale_length, sc.unit_settings.system)
