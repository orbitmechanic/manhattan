import sys, os, time
sys.path.insert(0, "/usr/lib/freecad/Mod/BIM")
sys.path.insert(0, "/usr/lib/freecad/Mod/BIM/importers")
import FreeCAD
import importOBJ

path = "/home/orbit9/Work/games/Manhattan/manhattan.obj"
t0 = time.time()
doc = importOBJ.open(path)
print("import time", round(time.time() - t0, 2), "s")
print("documents:", [d.Name for d in FreeCAD.listDocuments().values()])
print("objects:", len(doc.Objects))
total_f = 0
total_v = 0
mins = [1e30] * 3
maxs = [-1e30] * 3
for o in doc.Objects:
    print(" obj", o.Name, o.TypeId, "label", o.Label)
    if hasattr(o, "Mesh") and o.Mesh:
        m = o.Mesh
        total_f += m.CountFacets
        total_v += m.CountPoints
        bb = m.BoundBox
        print("    facets", m.CountFacets, "points", m.CountPoints,
              "bbox", tuple(round(v, 2) for v in (bb.XMin, bb.YMin, bb.ZMin)),
              tuple(round(v, 2) for v in (bb.XMax, bb.YMax, bb.ZMax)))
        mins = [min(mins[i], [bb.XMin, bb.YMin, bb.ZMin][i]) for i in range(3)]
        maxs = [max(maxs[i], [bb.XMax, bb.YMax, bb.ZMax][i]) for i in range(3)]
print("TOTAL facets", total_f, "points", total_v)
print("BBOX", tuple(round(v, 2) for v in mins), tuple(round(v, 2) for v in maxs))
assert total_f == 259767, f"facet count mismatch: {total_f}"
assert total_v > 150000, f"vertex count suspicious: {total_v}"
assert maxs[2] > 400, f"city height wrong (Z-up expected): {maxs}"
out = "/home/orbit9/Work/games/Manhattan/build/manhattan_freecad.FCStd"
doc.saveAs(out)
print("saved", out, os.path.getsize(out))
print("FREECAD IMPORT OK")
sys.stdout.flush()
# FreeCAD 1.1.4 sometimes segfaults while tearing down a large imported mesh
# on process exit; the document is already saved, so exit immediately.
os._exit(0)
