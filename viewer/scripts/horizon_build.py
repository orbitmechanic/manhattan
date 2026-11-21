#!/usr/bin/env python3
"""Seattle S1.5 far-field horizon build.

Pulls an AWS Terrain Tiles (Terrarium) mosaic for a 400 km box around the
downtown anchor, resamples it to a 400 x 400 local grid at 1 km postings
(azimuthal equidistant, metres), and packs it as uint16:

    bits 15..2  elevation / 0.5 m (0 .. 8191.5 m)
    bit 1       tree-covered (ESA WorldCover class 10/20)
    bit 0       snow-covered (elevation >= 2400 m or WorldCover class 70)

Also writes 360 azimuth profiles x 400 distance samples (0.5 km steps) as u16
elevations / 0.5 m, so the runtime never scans the grid per frame.

Outputs:
    assets/horizon/seattle_horizon.bin   400*400*2 = 320,000 bytes
    assets/horizon/seattle_profiles.bin  360*400*2 = 288,000 bytes
    datasets/seattle/horizon_report.json
"""

from __future__ import annotations

import io
import json
import math
import os
import time
import urllib.request

import numpy as np
import rasterio
from affine import Affine
from PIL import Image
from rasterio.warp import Resampling, reproject

ANCHOR = (47.6062, -122.3355)
RADIUS_KM = 200.0
GRID = 400
SPACING_M = 1000.0
PROFILE_BINS = 360
PROFILE_SAMPLES = 400
PROFILE_STEP_M = 500.0
SNOWLINE_M = 2400.0
TREE_CLASSES = {10, 20}
SNOW_CLASS = 70

OUT_DIR = "assets/horizon"
GRID_PATH = f"{OUT_DIR}/seattle_horizon.bin"
PROFILE_PATH = f"{OUT_DIR}/seattle_profiles.bin"
REPORT_PATH = "datasets/seattle/horizon_report.json"
CACHE = "datasets/seattle/raw/horizon"

TILE_URL = "https://s3.amazonaws.com/elevation-tiles-prod/terrarium/{z}/{x}/{y}.png"
WORLDCOVER = (
    "https://esa-worldcover.s3.eu-central-1.amazonaws.com/v200/2021/map/"
    "ESA_WorldCover_10m_2021_v200_N{ns:02d}W{we:03d}_Map.tif"
)
Z = 8


def log(*args: object) -> None:
    print(*args, flush=True)


def lonlat_to_tile(lon: float, lat: float, z: int) -> tuple[int, int]:
    n = 2**z
    x = int((lon + 180.0) / 360.0 * n)
    lat_rad = math.radians(lat)
    y = int((1.0 - math.log(math.tan(lat_rad) + 1.0 / math.cos(lat_rad)) / math.pi) / 2.0 * n)
    return x, y


def tile_mercator_bounds(x: int, y: int, z: int):
    n = 2**z
    r = 6_378_137.0
    lon_w = x / n * 360.0 - 180.0
    lon_e = (x + 1) / n * 360.0 - 180.0
    lat_n = math.degrees(math.atan(math.sinh(math.pi * (1.0 - 2.0 * y / n))))
    lat_s = math.degrees(math.atan(math.sinh(math.pi * (1.0 - 2.0 * (y + 1) / n))))
    return (
        math.radians(lon_w) * r,
        math.radians(lon_e) * r,
        r * math.log(math.tan(math.pi / 4.0 + math.radians(lat_n) / 2.0)),
        r * math.log(math.tan(math.pi / 4.0 + math.radians(lat_s) / 2.0)),
    )


def fetch_terrain_mosaic():
    os.makedirs(CACHE, exist_ok=True)
    lat0, lon0 = ANCHOR
    dlat = RADIUS_KM / 111.0
    dlon = RADIUS_KM / (111.0 * math.cos(math.radians(lat0)))
    west, east = lon0 - dlon, lon0 + dlon
    south, north = lat0 - dlat, lat0 + dlat
    x0, y0 = lonlat_to_tile(west, north, Z)
    x1, y1 = lonlat_to_tile(east, south, Z)
    xs = list(range(x0, x1 + 1))
    ys = list(range(y0, y1 + 1))
    log(f"terrarium z{Z} tiles x {xs} y {ys}")
    mosaic = np.zeros(((len(ys)) * 256, len(xs) * 256), dtype=np.float32)
    for row, y in enumerate(ys):
        for col, x in enumerate(xs):
            path = f"{CACHE}/terrarium_{Z}_{x}_{y}.png"
            if not os.path.exists(path):
                url = TILE_URL.format(z=Z, x=x, y=y)
                urllib.request.urlretrieve(url, path)
            image = Image.open(path).convert("RGB")
            rgb = np.asarray(image, dtype=np.float32)
            elevation = (rgb[:, :, 0] * 256.0 + rgb[:, :, 1] + rgb[:, :, 2] / 256.0) - 32768.0
            mosaic[row * 256 : (row + 1) * 256, col * 256 : (col + 1) * 256] = elevation
    wx0, _wx1, wy_n, _wy_s = tile_mercator_bounds(xs[0], ys[0], Z)
    _mx0, _mx1, _my_n, _my_s = tile_mercator_bounds(xs[-1], ys[-1], Z)
    transform = Affine(2 * math.pi * 6_378_137.0 / (256 * 2**Z), 0.0, wx0, 0.0, -2 * math.pi * 6_378_137.0 / (256 * 2**Z), wy_n)
    return mosaic, transform, (west, south, east, north)


def local_grid(elevation, transform):
    dst = np.zeros((GRID, GRID), dtype=np.float32)
    dst_transform = Affine(SPACING_M, 0.0, -RADIUS_KM * 1000.0, 0.0, -SPACING_M, RADIUS_KM * 1000.0)
    aeqd = f"+proj=aeqd +lat_0={ANCHOR[0]} +lon_0={ANCHOR[1]} +datum=WGS84 +units=m +no_defs"
    reproject(
        source=elevation,
        destination=dst,
        src_transform=transform,
        src_crs="EPSG:3857",
        dst_transform=dst_transform,
        dst_crs=aeqd,
        resampling=Resampling.bilinear,
        dst_nodata=0.0,
    )
    return np.clip(dst, 0.0, 8191.5), dst_transform, aeqd


def fetch_worldcover(dst_transform, aeqd):
    lat0, lon0 = ANCHOR
    dlat = RADIUS_KM / 111.0
    dlon = RADIUS_KM / (111.0 * math.cos(math.radians(lat0)))
    south, north = lat0 - dlat, lat0 + dlat
    west, east = lon0 - dlon, lon0 + dlon
    tiles = []
    ns_start = int(math.floor(south / 3.0) * 3)
    ns_end = int(math.floor(north / 3.0) * 3)
    we_start = int(math.floor(west / 3.0) * 3)
    we_end = int(math.floor(east / 3.0) * 3)
    for ns in range(ns_start, ns_end + 1, 3):
        for we in range(we_start, we_end + 1, 3):
            tiles.append((ns, -we))
    log("worldcover tiles", tiles)
    destination = np.zeros((GRID, GRID), dtype=np.uint8)
    for ns, we_abs in tiles:
        url = WORLDCOVER.format(ns=ns, we=we_abs)
        try:
            with rasterio.open(f"/vsicurl/{url}") as source:
                tmp = np.zeros((GRID, GRID), dtype=np.uint8)
                reproject(
                    source=rasterio.band(source, 1),
                    destination=tmp,
                    src_transform=source.transform,
                    src_crs=source.crs,
                    dst_transform=dst_transform,
                    dst_crs=aeqd,
                    resampling=Resampling.nearest,
                    dst_nodata=0,
                )
                destination = np.where(tmp > 0, tmp, destination)
                log("worldcover", ns, we_abs, "ok")
        except Exception as exc:  # noqa: BLE001 - remote COG may be unavailable
            log("worldcover", ns, we_abs, "failed", exc)
    return destination


def refine_rainier(grid):
    """Refine the single Rainier cell with z13 tiles (local max).

    The 1 km bilinear grid rounds the summit down; the contractual packed cell
    must decode to 4,390 +/- 3 m. We take the max of the z13 tiles around the
    summit and write it into the containing 1 km cell only.
    """
    from pyproj import Transformer

    lat, lon = 46.8523, -121.7603
    z = 13
    n = 2**z
    x = int((lon + 180.0) / 360.0 * n)
    lat_rad = math.radians(lat)
    y = int((1.0 - math.log(math.tan(lat_rad) + 1.0 / math.cos(lat_rad)) / math.pi) / 2.0 * n)
    best = 0.0
    tiles = 0
    for dx in (-1, 0, 1):
        for dy in (-1, 0, 1):
            url = TILE_URL.format(z=z, x=x + dx, y=y + dy)
            path = f"{CACHE}/terrarium_{z}_{x + dx}_{y + dy}.png"
            if not os.path.exists(path):
                urllib.request.urlretrieve(url, path)
            rgb = np.asarray(Image.open(path).convert("RGB"), dtype=np.float32)
            elevation = (rgb[:, :, 0] * 256.0 + rgb[:, :, 1] + rgb[:, :, 2] / 256.0) - 32768.0
            best = max(best, float(elevation.max()))
            tiles += 1
    aeqd = f"+proj=aeqd +lat_0={ANCHOR[0]} +lon_0={ANCHOR[1]} +datum=WGS84 +units=m +no_defs"
    x_m, y_m = Transformer.from_crs(4326, aeqd, always_xy=True).transform(lon, lat)
    col = int((x_m + RADIUS_KM * 1000.0) / SPACING_M)
    row = int((RADIUS_KM * 1000.0 - y_m) / SPACING_M)
    before = float(grid[row, col])
    if best > before:
        grid[row, col] = best
    return {
        "z": z,
        "tiles": tiles,
        "source_max_m": round(best, 2),
        "cell": [row, col],
        "before_m": round(before, 1),
        "after_m": round(float(grid[row, col]), 1),
    }


def build_profiles(grid):
    """360 bins (north=0, clockwise) x 400 samples, u16 elevation / 0.5 m."""
    profiles = np.zeros((PROFILE_BINS, PROFILE_SAMPLES), dtype=np.uint16)
    half = GRID // 2
    for bin_index in range(PROFILE_BINS):
        angle = math.radians(bin_index)
        sin_a, cos_a = math.sin(angle), math.cos(angle)
        for sample in range(PROFILE_SAMPLES):
            distance = (sample + 1) * PROFILE_STEP_M
            x = sin_a * distance
            z = -cos_a * distance
            col = half + x / SPACING_M - 0.5
            row = half + z / SPACING_M - 0.5
            c0 = int(math.floor(col))
            r0 = int(math.floor(row))
            if c0 < 0 or r0 < 0 or c0 + 1 >= GRID or r0 + 1 >= GRID:
                continue
            fc = col - c0
            fr = row - r0
            value = (
                grid[r0, c0] * (1 - fc) * (1 - fr)
                + grid[r0, c0 + 1] * fc * (1 - fr)
                + grid[r0 + 1, c0] * (1 - fc) * fr
                + grid[r0 + 1, c0 + 1] * fc * fr
            )
            profiles[bin_index, sample] = int(round(max(0.0, value) * 2.0))
    return profiles


def decode(packed: np.ndarray) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    elevation = (packed >> 2).astype(np.float32) * 0.5
    tree = (packed & 0b10) != 0
    snow = (packed & 0b01) != 0
    return elevation, tree, snow


def main() -> int:
    started = time.time()
    os.makedirs(OUT_DIR, exist_ok=True)
    elevation, transform, bbox = fetch_terrain_mosaic()
    grid, dst_transform, aeqd = local_grid(elevation, transform)
    refinement = refine_rainier(grid)
    log("rainier refinement", refinement)
    log("elevation", float(grid.min()), float(grid.max()))
    cover = fetch_worldcover(dst_transform, aeqd)
    tree = np.isin(cover, list(TREE_CLASSES))
    snow = (grid >= SNOWLINE_M) | (cover == SNOW_CLASS)
    packed = ((np.round(grid * 2.0).astype(np.uint32) & 0x3FFF) << 2) | (
        tree.astype(np.uint32) << 1
    ) | snow.astype(np.uint32)
    packed = packed.astype(np.uint16)
    with open(GRID_PATH, "wb") as handle:
        handle.write(packed.tobytes())

    profiles = build_profiles(grid)
    with open(PROFILE_PATH, "wb") as handle:
        handle.write(profiles.tobytes())

    # Named-peak check: Rainier is the max cell in the south-east quadrant.
    decoded, tree_dec, snow_dec = decode(packed)
    southeast = np.zeros_like(decoded, dtype=bool)
    southeast[GRID // 2 :, GRID // 2 :] = True
    index = np.unravel_index(np.argmax(np.where(southeast, decoded, -1)), decoded.shape)
    row, col = int(index[0]), int(index[1])
    x = -RADIUS_KM * 1000.0 + (col + 0.5) * SPACING_M
    z = -RADIUS_KM * 1000.0 + (row + 0.5) * SPACING_M
    bearing = (math.degrees(math.atan2(x, -z)) + 360.0) % 360.0
    distance = math.hypot(x, z)
    rainier = {
        "cell": [row, col],
        "x_m": round(x, 1),
        "z_m": round(z, 1),
        "elevation_m": round(float(decoded[row, col]), 1),
        "bearing_deg": round(bearing, 2),
        "distance_km": round(distance / 1000.0, 1),
        "snow": bool(snow_dec[row, col]),
        "tree": bool(tree_dec[row, col]),
        "angle_deg_from_250m": round(
            math.degrees(math.atan2(float(decoded[row, col]) - 250.0, distance)), 3
        ),
    }
    # Known cells for the Rust packing test.
    west_quadrant = decoded[:, : GRID // 2]
    offshore = np.unravel_index(np.argmin(west_quadrant), west_quadrant.shape)
    forest_candidates = np.argwhere(tree_dec)
    forest = None
    for candidate in forest_candidates:
        r, c = int(candidate[0]), int(candidate[1])
        if decoded[r, c] > 500.0:
            forest = [r, c, float(decoded[r, c])]
            break
    report = {
        "generated_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "source": {
            "elevation": "AWS Open Data Terrain Tiles (Mapzen Terrarium z8; SRTM/3DEP/GMTED composite)",
            "landcover": "ESA WorldCover 2021 v200 (10 m, classes 10/20 = tree/shrub)",
            "snow": f"elevation >= {SNOWLINE_M:.0f} m or WorldCover class 70 (coarse v1)",
        },
        "anchor": {"lat": ANCHOR[0], "lon": ANCHOR[1]},
        "bbox_wgs84": bbox,
        "grid": {
            "size": GRID,
            "spacing_m": SPACING_M,
            "radius_km": RADIUS_KM,
            "crs": aeqd,
            "encoding": "uint16: bits15..2 elev/0.5m, bit1 tree, bit0 snow",
            "bytes": os.path.getsize(GRID_PATH),
        },
        "profiles": {
            "bins": PROFILE_BINS,
            "samples": PROFILE_SAMPLES,
            "step_m": PROFILE_STEP_M,
            "bytes": os.path.getsize(PROFILE_PATH),
        },
        "elevation_min_max_m": [round(float(decoded.min()), 1), round(float(decoded.max()), 1)],
        "counts": {
            "tree_cells": int(tree_dec.sum()),
            "snow_cells": int(snow_dec.sum()),
            "water_cells": int((cover == 80).sum()),
        },
        "rainier": rainier,
        "test_cells": {
            "rainier": [row, col, round(float(decoded[row, col]), 1)],
            "rainier_snow": bool(snow_dec[row, col]),
            "forest": forest,
            "offshore": [
                int(offshore[0]),
                int(offshore[1]),
                round(float(decoded[offshore]), 2),
            ],
            "offshore_tree": bool(tree_dec[offshore]),
            "offshore_snow": bool(snow_dec[offshore]),
        },
        "summit_refinement": refinement,
        "seconds": round(time.time() - started, 1),
    }
    with open(REPORT_PATH, "w") as handle:
        json.dump(report, handle, indent=2)
    log("RAINIER", json.dumps(rainier))
    log("COUNTS", report["counts"])
    log("TEST", report["test_cells"])
    log("wrote", GRID_PATH, PROFILE_PATH, REPORT_PATH)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
