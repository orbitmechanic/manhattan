"""Turn the active floor-master Block_XXXX object into one object per floor.

Run from Blender's Text Editor / Python console with the block object active:

    exec(open('/home/orbit9/Work/games/Manhattan/scripts/separate_block_floors.py').read())

The original object is hidden and left in place as a source.  New objects are
named Block_XXXX_Floor_NN and keep the materials, UVs, COLOR_0 and the
`floor` face attribute.
"""
import bpy, bmesh
from collections import defaultdict

ob = bpy.context.active_object
assert ob and ob.type == 'MESH', "select a mesh block object first"
me = ob.data
if 'floor' not in me.attributes:
    raise RuntimeError("active mesh has no 'floor' face attribute")

groups = defaultdict(list)
for i, p in enumerate(me.polygons):
    groups[int(me.attributes['floor'].data[i].value)].append(i)

coll = ob.users_collection[0]
base_name = ob.name
for floor, face_ids in sorted(groups.items()):
    bm = bmesh.new()
    bm.from_mesh(me)
    bm.faces.ensure_lookup_table()
    delete = [f for f in bm.faces if f.index not in set(face_ids)]
    if delete:
        bmesh.ops.delete(bm, geom=delete, context='FACES')
    new_me = bpy.data.meshes.new(f"{base_name}_Floor_{floor:03d}")
    bm.to_mesh(new_me)
    bm.free()
    for mat in me.materials:
        new_me.materials.append(mat)
    new_ob = bpy.data.objects.new(new_me.name, new_me)
    new_ob.location = ob.location.copy()
    new_ob.rotation_euler = ob.rotation_euler.copy()
    new_ob.scale = ob.scale.copy()
    new_ob["block_index"] = ob.get("block_index", 0)
    new_ob["block_type"] = "floor"
    new_ob["floor"] = floor
    coll.objects.link(new_ob)
    print("created", new_ob.name, "faces", len(new_me.polygons))

ob.hide_set(True)
ob.hide_render = True
print("done; floor objects linked in", coll.name)
