import bpy, bmesh, math
from mathutils import Vector

for ob in bpy.data.objects:
    if ob.type != 'MESH':
        continue
    me = ob.data
    print(f"=== {ob.name}: {len(me.vertices)} verts {len(me.polygons)} polys")
    changed = me.validate(verbose=True, clean_customdata=False)
    print("  validate changed:", changed, "->", len(me.vertices), len(me.polygons))
    bm = bmesh.new()
    bm.from_mesh(me)
    zero = [f for f in bm.faces if f.calc_area() < 1e-9]
    print("  zero-area faces:", len(zero))
    loose = [v for v in bm.verts if not v.link_faces]
    print("  loose verts:", len(loose))
    nonmanifold = [e for e in bm.edges if not e.is_manifold]
    print("  non-manifold edges:", len(nonmanifold))
    # duplicate faces by sorted vertex indices
    seen = {}
    dup = 0
    for f in bm.faces:
        key = tuple(sorted(v.index for v in f.verts))
        if key in seen:
            dup += 1
        else:
            seen[key] = f
    print("  duplicate faces:", dup)
    bm.free()
