#!/usr/bin/env python3
"""STEP -> GLB for the player's Space Needle CAD (FreeCAD headless).

    STEP_PATH=/abs/model.STEP OUT_GLB=/abs/out.glb \
    TESS_LINEAR=0.08 TESS_ANGULAR=0.30 \
    freecadcmd scripts/step_to_glb.py

Paths come from the environment because freecadcmd treats bare arguments as
files it should also open. The tessellation receipt (facets at each stage) is
printed as RECEIPT lines for the ledger.

CAD is Z-up; the mesh is rotated -90 deg about X so the engine's Y is up.
"""

import math
import os

import FreeCAD
import Import
import Mesh
import MeshPart

step_path = os.environ["STEP_PATH"]
out_path = os.environ["OUT_GLB"]
linear = float(os.environ.get("TESS_LINEAR", "0.08"))
angular = float(os.environ.get("TESS_ANGULAR", "0.30"))

doc = FreeCAD.newDocument("needle")
Import.insert(step_path, doc.Name)
doc.recompute()

tessellated = 0
meshes = []
for obj in doc.Objects:
    shape = getattr(obj, "Shape", None)
    if shape is None or shape.isNull():
        continue
    mesh = MeshPart.meshFromShape(
        Shape=shape,
        LinearDeflection=linear,
        AngularDeflection=angular,
        Relative=False,
    )
    matrix = FreeCAD.Matrix()
    matrix.rotateX(-math.pi / 2.0)
    mesh.transform(matrix)
    tessellated += mesh.CountFacets
    feature = doc.addObject("Mesh::Feature", "needle_mesh")
    feature.Mesh = mesh
    meshes.append(feature)

doc.recompute()
if not meshes:
    raise SystemExit("no tessellatable shapes in " + step_path)

joined = Mesh.Mesh()
for feature in meshes:
    joined.addMesh(feature.Mesh)
print(
    f"RECEIPT tessellated_facets={tessellated} joined_facets={joined.CountFacets} "
    f"linear={linear} angular={angular}",
    flush=True,
)

# FreeCAD's glTF writer only takes Part shapes, not Mesh::Feature objects; the
# mesh path is STL, and Blender does STL -> decimated GLB in scripts/stl_to_glb.py.
joined.write(out_path.replace(".glb", ".stl"))
print(f"RECEIPT wrote={out_path.replace('.glb', '.stl')}", flush=True)
os._exit(0)
