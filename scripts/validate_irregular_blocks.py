import bpy

city = bpy.data.collections["City_Blocks"]
parks = bpy.data.collections["Parks"]
objs = [o for o in city.objects if o.type == "MESH"]
blocks = [o for o in objs if o.name.startswith("Block_")]
ground = bpy.data.objects.get("Ground_Blocks")
faces = sum(len(o.data.polygons) for o in blocks)
print("block objects", len(blocks))
print("ground", ground, "polys", len(ground.data.polygons) if ground else None,
      "verts", len(ground.data.vertices) if ground else None)
print("park empties", len(parks.objects))
print("building faces", faces)
print("scene block_count", bpy.context.scene.get("block_count"))
print("scene building_blocks", bpy.context.scene.get("building_blocks"))
print("scene park_blocks", bpy.context.scene.get("park_blocks"))
bad = [o.name for o in blocks if o.get("block_type") != "building" or o.get("block_index") is None]
print("bad block props", len(bad), bad[:5])
missing = [o.name for o in blocks if not o.data.materials]
print("block objects without materials", len(missing))
assert len(blocks) == 1223, len(blocks)
assert faces == 105330, faces
assert len(parks.objects) == 19
assert ground is not None and len(ground.data.polygons) == 3388
assert bpy.context.scene.get("block_count") == 1242
print("IRREGULAR BLOCKS OK")
