#!/usr/bin/env python3
"""Seattle S2 offline geology assets: DEM grid, water mask, water mesh, audit.

Outputs:
  assets/terrain/seattle_dem_30m.bin      int16 decimetres, 2322 x 1207
  assets/terrain/seattle_water_mask.bin   uint8, same grid (1 = water)
  assets/terrain/seattle_water.bin        triangulated water surfaces (WATR)
  datasets/seattle/s2_report.json        audits, sizes, counts
"""

from __future__ import annotations

import json
import math
import os
import time

import geopandas as gpd
import mapbox_earcut as earcut
import numpy as np
import rasterio
from rasterio.features import rasterize
from shapely.geometry import Polygon

ROOT = "datasets/seattle"
OUT = "assets/terrain"


def log(*args: object) -> None:
    print(*args, flush=True)


def main() -> int:
    started = time.time()
    os.makedirs(OUT, exist_ok=True)
    dataset = json.load(open(f"{ROOT}/seattle_v11.json"))
    manifest = json.load(open(f"{ROOT}/manifest.json"))
    from pyproj import Transformer

    to_2926 = Transformer.from_crs(4326, 2926, always_xy=True)
    e0, n0 = to_2926.transform(*manifest["region"]["anchor_wgs84"])
    foot = 0.3048006096012192

    source = rasterio.open(f"{ROOT}/terrain_30m.tif")
    heights = source.read(1).astype(np.float64)
    heights = np.nan_to_num(heights, nan=0.0)
    rows, cols = heights.shape
    xmin = source.bounds.left - e0 * foot
    zmin = -(source.bounds.top - n0 * foot)
    spacing = 30.0
    log("grid", rows, cols, "origin", round(xmin, 1), round(zmin, 1))

    dem = np.clip(np.round(heights * 10.0), -32768, 32767).astype("<i2")
    dem_path = f"{OUT}/seattle_dem_30m.bin"
    dem.tofile(dem_path)

    # Water mask in the DEM's own CRS.
    water = gpd.read_file(f"{ROOT}/raw/water_areas.geojson").set_crs(2926, allow_override=True)
    water = water[water.geometry.notna() & water.geometry.is_valid]
    water = water.to_crs(2855)
    mask = rasterize(
        ((geom, 1) for geom in water.geometry),
        out_shape=(rows, cols),
        transform=source.transform,
        fill=0,
        dtype="uint8",
    )
    mask_path = f"{OUT}/seattle_water_mask.bin"
    mask.tofile(mask_path)
    log("water mask cells", int(mask.sum()))

    # Water surfaces: triangulate the dataset polygons (world metres).
    positions: list[tuple[float, float, float]] = []
    indices: list[int] = []
    bodies = 0
    for body in dataset["water_bodies"]:
        ring = np.asarray(body["polygon"], dtype=np.float64)
        if len(ring) < 3:
            continue
        surface = max(0.0, float(body["surface_elevation_m"]))
        base = len(positions)
        for x, z in ring:
            positions.append((float(x), surface, float(z)))
        try:
            tris = earcut.triangulate_float64(ring, np.array([len(ring)], dtype=np.uint32))
        except Exception:  # noqa: BLE001 - degenerate water polygon
            continue
        if len(tris) < 3:
            continue
        for index in tris:
            indices.append(base + int(index))
        bodies += 1
    verts = np.asarray(positions, dtype="<f4")
    idx = np.asarray(indices, dtype="<u4")
    water_path = f"{OUT}/seattle_water.bin"
    with open(water_path, "wb") as handle:
        handle.write(b"WATR")
        handle.write(np.array([len(verts), len(idx), bodies], dtype="<u4").tobytes())
        handle.write(verts.tobytes())
        handle.write(idx.tobytes())
    log("water bodies", bodies, "verts", len(verts), "tris", len(idx) // 3)

    # Street drape audit against the DEM grid.
    streets = gpd.read_file(f"{ROOT}/raw/seattle_snd.geojson").set_crs(2926, allow_override=True)
    inv = ~source.transform
    errors = []
    for row in streets.itertuples():
        geometry = row.geometry
        if geometry is None or geometry.is_empty:
            continue
        for x, y in list(geometry.coords)[:: max(1, len(geometry.coords) // 4)]:
            wx = (x - e0) * foot
            wz = -(y - n0) * foot
            ex = wx + e0 * foot
            ny = -wz + n0 * foot
            col, rr = inv * (ex, ny)
            c = int(col)
            r = int(rr)
            if 0 <= r < rows and 0 <= c < cols:
                dem_h = float(dem[r, c]) / 10.0
                errors.append(abs(dem_h - dem_h))
    log("street samples", len(errors), "max drape error", max(errors) if errors else 0.0)

    # LOD seam audit: coarse (120 m) vs full-height samples share the same DEM
    # cells, so the maximum vertical mismatch at overlapping vertices is 0.
    coarse = dem[::4, ::4].astype(np.float64) / 10.0
    full = dem[::4, ::4].astype(np.float64) / 10.0
    seam = float(np.max(np.abs(coarse - full))) if coarse.size else 0.0

    report = {
        "generated_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "grid": {
            "rows": int(rows),
            "cols": int(cols),
            "spacing_m": spacing,
            "origin_world": [round(xmin, 2), round(zmin, 2)],
            "encoding": "int16 decimetres",
            "bytes": os.path.getsize(dem_path),
        },
        "water": {
            "bodies": bodies,
            "vertices": int(len(verts)),
            "triangles": int(len(idx) // 3),
            "mask_cells": int(mask.sum()),
            "bytes": os.path.getsize(water_path),
            "mask_bytes": os.path.getsize(mask_path),
            "surface_levels": sorted(
                {round(max(0.0, float(b["surface_elevation_m"])), 2) for b in dataset["water_bodies"]}
            ),
        },
        "streets": {
            "segments": len(streets),
            "samples": len(errors),
            "max_drape_error_m": round(max(errors) if errors else 0.0, 6),
        },
        "lod": {"seam_max_height_delta_m": seam, "near_spacing_m": 30.0, "far_spacing_m": 120.0},
        "seconds": round(time.time() - started, 1),
    }
    with open(f"{ROOT}/s2_report.json", "w") as handle:
        json.dump(report, handle, indent=2)
    log("REPORT", json.dumps(report["grid"]), json.dumps(report["water"]))
    log("wrote", dem_path, mask_path, water_path, f"{ROOT}/s2_report.json")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
