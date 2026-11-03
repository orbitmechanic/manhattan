"""Build a clean, relinked Manhattan master file.

- relinks every image to Textures/day/<logical name>
- renames images, materials and objects descriptively
- removes orphan/duplicate data
- adds a preview camera + sun and a procedural sky
- saves manhattan.blend, then exports manhattan.fbx and manhattan.obj
"""
import bpy, os, math
from mathutils import Vector

ROOT = "/home/orbit9/Work/games/Manhattan"
TEX = os.path.join(ROOT, "Textures", "day")

# old image datablock name -> new path (relative to the .blend)
IMAGE_MAP = {
    "TexturesCom_HighRiseResidential0197_2_seamless_S.jpg": "Textures/day/facades/facade_residential_beige_0197.jpg",
    "TexturesCom_HighRiseGlass0110_4_seamless_S.jpg": "Textures/day/facades/facade_glass_white_0110.jpg",
    "TexturesCom_HighRiseGlass0053_1_seamless_S.jpg": "Textures/day/facades/facade_glass_bronze_0053.jpg",
    "TexturesCom_HighRiseGlass0056_1_seamless_S.jpg": "Textures/day/facades/facade_glass_blue_0056.jpg",
    "images (21).jpg": "Textures/day/facades/facade_glass_skyblue_grid.jpg",
    "TexturesCom_BuildingsHighRise0564_1_S.jpg": "Textures/day/facades/facade_glass_striped_0564.jpg",
    "images (4).jpg": "Textures/day/facades/facade_glass_dark_vertical.jpg",
    "images (19).jpg": "Textures/day/facades/facade_glass_blue_horizontal.jpg",
    "TexturesCom_BuildingsHighRise0607_1_S.jpg": "Textures/day/facades/facade_office_white_0607.jpg",
    "images (20).jpg": "Textures/day/facades/facade_masonry_beige_classic.jpg",
    "roof5.jpg": "Textures/day/roofs/roof_concrete_gravel.jpg",
    "EXPORT_GOOGLE_SAT_WM": "Textures/day/ground/satellite_manhattan_ground.jpg",
    "GOOGLE_SAT_WM.tif": "Textures/day/ground/satellite_manhattan_ground.jpg",
    "syferfontein_1d_clear_2k.hdr": "Textures/day/environment/sky_clear_2k.hdr",
}

MATERIAL_MAP = {
    "Material.001": "Facade_Residential_Beige_0197",
    "Material.002": "Facade_Glass_White_0110",
    "Material.003": "Facade_Glass_Bronze_0053",
    "Material.004": "Facade_Glass_Blue_0056",
    "Material.005": "Facade_Glass_SkyBlue_Grid",
    "Material.006": "Facade_Glass_Striped_0564",
    "Material.007": "Facade_Glass_Dark_Vertical",
    "Material.008": "Facade_Glass_Blue_Horizontal",
    "Material.009": "Facade_Office_White_0607",
    "Material.010": "Facade_Masonry_Beige_Classic",
    "Material.011": "Roof_Concrete_Gravel",
    "rastMat": "Ground_Satellite_Manhattan",
}

OBJECT_MAP = {
    "Areas:building": "Manhattan_Buildings",
    "EXPORT_GOOGLE_SAT_WM": "Manhattan_Ground",
    "GOOGLE_SAT_WM": "Manhattan_Origin",
}


def relink_images():
    # the ground image exists twice in the original file; drop the duplicate
    dup = bpy.data.images.get("GOOGLE_SAT_WM.tif")
    if dup:
        bpy.data.images.remove(dup)
        print("  removed duplicate GOOGLE_SAT_WM.tif image")
    for im in list(bpy.data.images):
        if im.name in ("Render Result", "Viewer Node"):
            continue
        rel = IMAGE_MAP.get(im.name)
        if rel is None:
            # already mapped by filepath basename (e.g. after a re-run)
            base = os.path.basename(im.filepath.replace("\\", "/"))
            for rel2 in IMAGE_MAP.values():
                if os.path.basename(rel2) == base:
                    rel = rel2
                    break
        if rel is None:
            print("  WARNING unmapped image:", im.name, im.filepath)
            continue
        abs_path = os.path.join(ROOT, rel)
        if not os.path.exists(abs_path):
            raise RuntimeError(f"missing target texture {abs_path}")
        # drop stale packed payload so the file genuinely links to disk
        if im.packed_file:
            try:
                im.unpack(method='REMOVE')
            except Exception as e:
                print("  unpack failed", im.name, e)
        # reload from the absolute path first: this script may be run from a
        # backup copy of the .blend whose // root is not the project root
        im.filepath = abs_path
        im.source = 'FILE'
        im.reload()
        new_name = os.path.splitext(os.path.basename(rel))[0]
        if new_name != im.name:
            im.name = new_name
        print(f"  image {new_name}: {im.size[0]}x{im.size[1]} -> {im.filepath}")


def prune():
    # duplicate satellite datablock is merged/removed
    for name in ["GOOGLE_SAT_WM.tif", "GOOGLE_SAT_WM"]:
        im = bpy.data.images.get(name)
        if im and im.users == 0:
            bpy.data.images.remove(im)
    # default Grease Pencil material left behind by the original file
    dots = bpy.data.materials.get("Dots Stroke")
    if dots:
        dots.use_fake_user = False
        bpy.data.materials.remove(dots)
        print("  removed Dots Stroke")
    for m in list(bpy.data.materials):
        if m.users == 0:
            print("  removing unused material", m.name)
            bpy.data.materials.remove(m)


def rename_materials():
    for old, new in MATERIAL_MAP.items():
        m = bpy.data.materials.get(old)
        if not m:
            continue
        m.name = new
        p = next((n for n in m.node_tree.nodes if n.type == 'BSDF_PRINCIPLED'), None)
        if p:
            name = new.lower()
            if "glass" in name:
                p.inputs['Metallic'].default_value = 0.15
                p.inputs['Roughness'].default_value = 0.22
            elif "ground" in name:
                p.inputs['Roughness'].default_value = 0.85
                p.inputs['Metallic'].default_value = 0.0
            elif "roof" in name:
                p.inputs['Roughness'].default_value = 0.92
                p.inputs['Metallic'].default_value = 0.0
            else:
                p.inputs['Roughness'].default_value = 0.72
                p.inputs['Metallic'].default_value = 0.0


def rename_objects():
    for old, new in OBJECT_MAP.items():
        ob = bpy.data.objects.get(old)
        if ob:
            ob.name = new
            if ob.data:
                ob.data.name = new + "_Mesh"
            print(f"  object {old} -> {new}")
    sc = bpy.context.scene
    sc["ground_z"] = 0.0
    sc["city_bounds_min"] = (-2019.29, -2558.73, 0.0)
    sc["city_bounds_max"] = (4683.12, 2621.25, 410.0)
    sc["texture_day_dir"] = "Textures/day"
    sc["texture_night_dir"] = "Textures/night"
    for ob in bpy.data.objects:
        if ob.type == 'Mesh':
            ob["manhattan_role"] = "buildings" if "Buildings" in ob.name else "ground"


def setup_world():
    sc = bpy.context.scene
    w = bpy.data.worlds.new("Manhattan_Sky") if not bpy.data.worlds else bpy.data.worlds[0]
    sc.world = w
    w.name = "Manhattan_Sky"
    nt = w.node_tree
    nt.nodes.clear()
    out = nt.nodes.new("ShaderNodeOutputWorld")
    bg = nt.nodes.new("ShaderNodeBackground")
    # clean studio-day sky: a solid blue background reads better than a
    # physical sky in a stylised city view
    bg.inputs['Color'].default_value = (0.38, 0.55, 0.80, 1.0)
    bg.inputs['Strength'].default_value = 1.0
    nt.links.new(bg.outputs['Background'], out.inputs['Surface'])
    # remove the orphan HDR image once the world no longer uses it
    hdr = bpy.data.images.get("sky_clear_2k")
    if hdr and hdr.users == 0:
        bpy.data.images.remove(hdr)


def add_preview():
    sc = bpy.context.scene
    for name in ("Preview_Camera", "Preview_Sun"):
        ob = bpy.data.objects.get(name)
        if ob:
            bpy.data.objects.remove(ob, do_unlink=True)

    cd = bpy.data.cameras.new("Preview_Camera")
    cd.lens = 35
    cd.clip_start = 1.0
    cd.clip_end = 40000
    cam = bpy.data.objects.new("Preview_Camera", cd)
    sc.collection.objects.link(cam)
    cam.location = (3600, -4700, 2300)
    target = Vector((1000, 100, 90))
    cam.rotation_euler = (target - cam.location).to_track_quat('-Z', 'Y').to_euler()
    sc.camera = cam

    ld = bpy.data.lights.new("Preview_Sun", type='SUN')
    ld.energy = 3.0
    ld.angle = math.radians(1.5)
    ld.color = (1.0, 0.96, 0.90)
    sun = bpy.data.objects.new("Preview_Sun", ld)
    sc.collection.objects.link(sun)
    sun.rotation_euler = (math.radians(48), 0, math.radians(35))

    sc.render.engine = 'BLENDER_EEVEE'
    sc.render.resolution_x = 1600
    sc.render.resolution_y = 900
    sc.render.resolution_percentage = 100
    sc.render.film_transparent = False
    sc.view_settings.view_transform = 'Standard'
    sc.view_settings.exposure = -0.2


def validate():
    print("\n=== VALIDATION ===")
    missing = []
    for im in bpy.data.images:
        if im.source != 'FILE':
            continue
        p = bpy.path.abspath(im.filepath)
        ok = os.path.exists(p) and im.size[0] > 0
        print(f"  {'OK ' if ok else 'BAD'} {im.name:<38} {im.size[0]}x{im.size[1]} {im.filepath}")
        if not ok:
            missing.append(im.name)
    if missing:
        raise RuntimeError(f"missing/unreadable textures: {missing}")
    for ob in bpy.data.objects:
        if ob.type == 'MESH':
            print(f"  mesh {ob.name}: {len(ob.data.vertices)} verts, "
                  f"{len(ob.data.polygons)} polys, {len(ob.data.materials)} materials")
    total_v = sum(len(o.data.vertices) for o in bpy.data.objects if o.type == 'MESH')
    total_p = sum(len(o.data.polygons) for o in bpy.data.objects if o.type == 'MESH')
    print(f"  TOTAL {total_v} verts / {total_p} polys")
    assert not missing


def main():
    print("=== BUILD CLEAN MANHATTAN ===")
    relink_images()
    prune()
    rename_materials()
    rename_objects()
    setup_world()
    add_preview()
    validate()

    sc = bpy.context.scene
    # save master, then make every texture path relative to it and save again
    bpy.ops.wm.save_as_mainfile(filepath=os.path.join(ROOT, "manhattan.blend"), compress=True)
    for im in bpy.data.images:
        if im.source == 'FILE' and os.path.isabs(im.filepath):
            im.filepath = bpy.path.relpath(im.filepath)
            print("  relative:", im.name, im.filepath)
    bpy.ops.wm.save_as_mainfile(filepath=os.path.join(ROOT, "manhattan.blend"), compress=True)
    validate()
    print("saved manhattan.blend")

    # export a consolidated FBX (textures embedded, same model as the blend)
    bpy.ops.object.select_all(action='SELECT')
    bpy.ops.export_scene.fbx(
        filepath=os.path.join(ROOT, "manhattan.fbx"),
        use_selection=False,
        path_mode='COPY',
        embed_textures=True,
        mesh_smooth_type='FACE',
        use_mesh_modifiers=True,
        object_types={'MESH', 'EMPTY'},
        bake_anim=False,
        apply_unit_scale=False,
        axis_forward='-Z',
        axis_up='Y',
    )
    print("saved manhattan.fbx", os.path.getsize(os.path.join(ROOT, "manhattan.fbx")))

    # export OBJ for FreeCAD (Z up, triangulated, relative texture paths)
    bpy.ops.wm.obj_export(
        filepath=os.path.join(ROOT, "manhattan.obj"),
        export_selected_objects=False,
        apply_modifiers=True,
        export_uv=True,
        export_normals=True,
        export_materials=True,
        export_pbr_extensions=False,
        export_triangulated_mesh=True,
        path_mode='RELATIVE',
        forward_axis='Y',
        up_axis='Z',
    )
    print("saved manhattan.obj", os.path.getsize(os.path.join(ROOT, "manhattan.obj")))


main()
