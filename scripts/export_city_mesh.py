"""Re-export the real city mesh + ground after floor edits.

  blender -b manhattan_floors.blend --python scripts/export_city_mesh.py
"""
import bpy, os

ROOT = "/home/orbit9/Work/games/Manhattan"
OUT = os.path.join(ROOT, "viewer", "assets", "models")


def export(objs, name):
    path = os.path.join(OUT, name)
    bpy.ops.object.select_all(action="DESELECT")
    for ob in objs:
        ob.select_set(True)
    bpy.context.view_layer.objects.active = objs[0]
    bpy.ops.export_scene.gltf(
        filepath=path,
        export_format="GLB",
        use_selection=True,
        export_apply=True,
        export_materials="EXPORT",
        export_image_format="AUTO",
        export_texcoords=True,
        export_normals=False,
        export_vertex_color="ACTIVE",
        export_all_vertex_colors=True,
        export_cameras=False,
        export_lights=False,
        export_extras=False,
        export_yup=True,
    )
    print("wrote", path, os.path.getsize(path))


floor = bpy.data.collections["Floor_Blocks"]
export([o for o in floor.objects if o.type == "MESH"], "manhattan_city.glb")
export([bpy.data.objects["Ground_Blocks"]], "manhattan_ground.glb")
