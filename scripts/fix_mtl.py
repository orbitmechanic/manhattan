#!/usr/bin/env python3
"""Rewrite OBJ MTL texture paths to the canonical Textures/day tree."""
import os, re, sys

ROOT = "/home/orbit9/Work/games/Manhattan"
mtl = os.path.join(ROOT, "manhattan.mtl")

lookup = {}
for base, _dirs, files in os.walk(os.path.join(ROOT, "Textures", "day")):
    for f in files:
        lookup[f] = os.path.relpath(os.path.join(base, f), ROOT).replace(os.sep, "/")

lines = open(mtl).read().splitlines()
out, changed = [], 0
for line in lines:
    if line.startswith("map_Kd "):
        base = os.path.basename(line[7:].strip().replace("\\", "/"))
        target = lookup.get(base)
        if target:
            out.append("map_Kd " + target)
            changed += 1
        else:
            print("UNRESOLVED", base)
            out.append(line)
    else:
        out.append(line)
open(mtl, "w").write("\n".join(out) + "\n")
print("rewrote", changed, "texture paths")
for line in out:
    if line.startswith("map_Kd "):
        print(" ", line)
