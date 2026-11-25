"""STL (from the FreeCAD step) -> game GLB for the Space Needle.

    blender --background --python scripts/stl_to_glb.py -- <in.stl> <out.glb> [target_height_m]

Blender is Z-up and its glTF exporter converts to Y-up, so the FreeCAD Y-up STL
is rotated back +90 deg about X on import. The model is scaled so the top of the
spire is TARGET (default 184 m), origin at the base, and decimated only if it
exceeds the game budget.
"""

import sys

import os

import bpy
import mathutils

args = sys.argv[sys.argv.index("--") + 1 :]
stl_path, out_path = args[0], args[1]
target = float(args[2]) if len(args) > 2 else 184.0

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.wm.stl_import(filepath=stl_path)
obj = bpy.context.selected_objects[0]
obj.rotation_euler = (3.141592653589793 / 2.0, 0.0, 0.0)
bpy.ops.object.transform_apply(location=False, rotation=True, scale=True)

# Scale to the real total height, origin at the base.
height = obj.dimensions.z
scale = target / height if height > 0 else 1.0
obj.scale = (scale, scale, scale)
bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)

triangles = sum(len(p.vertices) - 2 for p in obj.data.polygons)
print(f"RECEIPT stl_tris={triangles} height_before={height:.2f} scale={scale:.4f}", flush=True)

budget = int(os.environ.get("TARGET_TRIS", "6000"))
if triangles > budget:
    ratio = budget / triangles
    mod = obj.modifiers.new("decimate", "DECIMATE")
    mod.ratio = ratio
    bpy.ops.object.modifier_apply(modifier=mod.name)
    triangles = sum(len(p.vertices) - 2 for p in obj.data.polygons)
    print(f"RECEIPT decimated_to={triangles}", flush=True)

material = bpy.data.materials.new("needle_shell")
material.use_nodes = True
bsdf = material.node_tree.nodes.get("Principled BSDF")
if bsdf:
    bsdf.inputs["Base Color"].default_value = (0.86, 0.87, 0.88, 1.0)
    bsdf.inputs["Roughness"].default_value = 0.55
obj.data.materials.append(material)

bpy.ops.object.select_all(action="DESELECT")
obj.select_set(True)
bpy.context.view_layer.objects.active = obj
bpy.ops.export_scene.gltf(
    filepath=out_path,
    export_format="GLB",
    use_selection=True,
    export_yup=True,
    export_apply=True,
)
print(f"RECEIPT wrote={out_path} tris={triangles}", flush=True)
