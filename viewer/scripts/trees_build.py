#!/usr/bin/env python3
"""Seattle trees: acquire inventory + canopy, build the per-chunk tree dataset.

Sources
-------
- SDOT Trees (Seattle GeoData ArcGIS FeatureServer, EPSG:2926): inventoried
  street trees with genus/common name, height and diameter.
- ESA WorldCover 2021 v200 10 m (classes 10/20) for canopy-guided residential
  scatter, windowed from the public COG.
- City of Seattle parks polygons (already in raw/seattle_parks.geojson) for park
  canopy placement.

Outputs
-------
- raw/seattle_trees.jsonl.gz (cached inventory), source SHA-256.
- assets/datasets/seattle_trees.bin: per-neighborhood records keyed by the
  CityPack node index; record = x,z (decimetres, chunk-centre relative),
  height (cm), species, tint, regime.
- datasets/seattle/trees_report.json: counts, thresholds, exclusions, hashes.
- datasets/seattle/manifest.json: sources + derived entry.

Run:  python3 scripts/trees_build.py [--refresh]
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import math
import os
import struct
import time
from datetime import datetime, timezone

import numpy as np
import requests
from pyproj import Transformer
from shapely import STRtree
from shapely.geometry import LineString, Point, Polygon, box

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PACK = os.path.join(ROOT, "assets/datasets/seattle.citypack")
PACK_NODES = 1096
TREE_BIN = os.path.join(ROOT, "assets/datasets/seattle_trees.bin")
REPORT = os.path.join(ROOT, "datasets/seattle/trees_report.json")
MANIFEST = os.path.join(ROOT, "datasets/seattle/manifest.json")
RAW = os.path.join(ROOT, "datasets/seattle/raw")
TREES_JSONL = os.path.join(RAW, "seattle_trees.jsonl.gz")
PARKS = os.path.join(RAW, "seattle_parks.geojson")
PARKS_POLY = os.path.join(RAW, "seattle_parks_polygons.geojson")
WATER = os.path.join(RAW, "water_areas.geojson")
CANOPY_NPZ = os.path.join(RAW, "worldcover_canopy.npz")

TREES_SERVICE = (
    "https://services.arcgis.com/ZOyb2t4B0UYuYNYH/arcgis/rest/services/"
    "SDOT_Trees_CDL/FeatureServer/0/query"
)
WORLDCOVER = (
    "https://esa-worldcover.s3.eu-central-1.amazonaws.com/v200/2021/map/"
    "ESA_WorldCover_10m_2021_v200_N45W123_Map.tif"
)
US_FOOT_M = 0.304800609601219

# Species palette: index, key, archetype (0 conifer, 1 broadleaf, 2 ornamental),
# height range in metres.
SPECIES = [
    ("douglas_fir", 0, (22.0, 45.0)),
    ("western_red_cedar", 0, (18.0, 38.0)),
    ("bigleaf_maple", 1, (12.0, 26.0)),
    ("pacific_madrone", 1, (8.0, 20.0)),
    ("cherry", 2, (5.0, 11.0)),
    ("london_plane", 1, (12.0, 24.0)),
]

GENUS_SPECIES = {
    "Pseudotsuga": 0,
    "Tsuga": 1,
    "Abies": 1,
    "Picea": 1,
    "Pinus": 1,
    "Thuja": 1,
    "Calocedrus": 1,
    "Sequoia": 1,
    "Acer": 2,
    "Arbutus": 3,
    "Prunus": 4,
    "Platanus": 5,
    "Fraxinus": 5,
    "Quercus": 5,
    "Tilia": 5,
    "Ulmus": 5,
    "Zelkova": 5,
    "Pyrus": 4,
    "Malus": 4,
    "Magnolia": 4,
    "Crataegus": 4,
    "Cornus": 4,
    "Cercidiphyllum": 5,
    "Ginkgo": 5,
    "Betula": 5,
    "Carpinus": 5,
    "Fagus": 5,
    "Liquidambar": 5,
    "Populus": 5,
    "Salix": 5,
    "Sorbus": 4,
    "Styrax": 4,
    "Stewartia": 5,
    "Nyssa": 5,
    "Metasequoia": 1,
    "Taxodium": 1,
}

REGIME_STREET = 0
REGIME_PARK = 1
REGIME_RESIDENTIAL = 2
REGIME_SYNTHETIC_STREET = 3

CAP_STREET = 300
CAP_PARK = 320
CAP_RESIDENTIAL = 160
CAP_SYNTHETIC = 160


def log(*args: object) -> None:
    print(*args, flush=True)


def sha256(path: str) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def parse_pack_nodes() -> list[dict]:
    """Read the CityPack node directory (version 2)."""
    with open(PACK, "rb") as handle:
        data = handle.read()
    if data[:4] != b"CDPK":
        raise SystemExit("bad citypack magic")
    version, header_bytes, node_count, _buildings, _streets, _chunks = struct.unpack_from(
        "<HIIIII", data, 4
    )
    if version != 2:
        raise SystemExit(f"unsupported citypack version {version}")
    nodes = []
    pos = 26
    for _ in range(node_count):
        level = data[pos]
        parent = struct.unpack_from("<i", data, pos + 1)[0]
        first_child = struct.unpack_from("<I", data, pos + 5)[0]
        child_count = struct.unpack_from("<H", data, pos + 9)[0]
        buildings = struct.unpack_from("<I", data, pos + 11)[0]
        streets = struct.unpack_from("<I", data, pos + 15)[0]
        center = struct.unpack_from("<ff", data, pos + 19)
        bounds = struct.unpack_from("<ffff", data, pos + 27)
        mean_height, p95, glass = struct.unpack_from("<HHB", data, pos + 43)
        pos += 50  # fixed node fields before the name
        name_len = data[pos]
        name = data[pos + 1 : pos + 1 + name_len].decode("utf-8", "replace")
        pos += 1 + name_len
        chunk_offset, chunk_bytes = struct.unpack_from("<II", data, pos)
        pos += 8
        nodes.append(
            {
                "level": level,
                "parent": parent,
                "first_child": first_child,
                "child_count": child_count,
                "buildings": buildings,
                "streets": streets,
                "center": center,
                "bounds": bounds,
                "name": name,
                "chunk_offset": chunk_offset,
                "chunk_bytes": chunk_bytes,
            }
        )
    if len(nodes) != node_count:
        raise SystemExit("node directory truncated")
    return nodes


def parse_chunk(data: bytes, offset: int, size: int) -> dict:
    blob = data[offset : offset + size]
    _chunk_bytes, building_count, street_count = struct.unpack_from("<III", blob, 0)
    center = struct.unpack_from("<ff", blob, 12)
    pos = 20 + 16 + 7 + 1
    string_count = struct.unpack_from("<H", blob, pos)[0]
    pos += 2
    strings = []
    for _ in range(string_count):
        length = struct.unpack_from("<H", blob, pos)[0]
        strings.append(blob[pos + 2 : pos + 2 + length].decode("utf-8", "replace"))
        pos += 2 + length
    buildings = []
    for _ in range(building_count):
        _id, qx, qz, height_cm, _fhc, _base, _fc, _year, _levels, _src, _cat, vcount = (
            struct.unpack_from("<IhhHHHHHBBBH", blob, pos)
        )
        pos += 23
        cx = center[0] + qx * 0.25
        cz = center[1] + qz * 0.25
        ring = []
        for _v in range(vcount):
            vx, vz = struct.unpack_from("<hh", blob, pos)
            pos += 4
            ring.append((cx + vx * 0.25, cz + vz * 0.25))
        if len(ring) >= 3:
            buildings.append((height_cm / 100.0, Polygon(ring)))
    streets = []
    for _ in range(street_count):
        _id, kind, _name_idx, pcount = struct.unpack_from("<IBHH", blob, pos)
        pos += 9
        pts = []
        for _p in range(pcount):
            vx, vz = struct.unpack_from("<hh", blob, pos)
            pos += 4
            pts.append((center[0] + vx * 0.25, center[1] + vz * 0.25))
        if len(pts) >= 2:
            streets.append((kind, LineString(pts)))
    return {"center": center, "buildings": buildings, "streets": streets}


def fetch_sdot(refresh: bool) -> tuple[int, str]:
    if os.path.exists(TREES_JSONL) and not refresh:
        with gzip.open(TREES_JSONL, "rt") as handle:
            return sum(1 for _ in handle), sha256(TREES_JSONL)
    fields = ",".join(
        [
            "OBJECTID",
            "COMMON_NAME",
            "SCIENTIFIC_NAME",
            "GENUS",
            "TREEHEIGHT",
            "DIAM",
            "CONDITION",
            "CURRENT_STATUS",
            "SITETYPE",
            "OWNERSHIP",
            "SHAPE_LNG",
            "SHAPE_LAT",
        ]
    )
    started = time.time()
    count_params = {"where": "1=1", "returnCountOnly": "true", "f": "json"}
    count_response = requests.get(TREES_SERVICE, params=count_params, timeout=120)
    count_response.raise_for_status()
    total = int(count_response.json().get("count", 0))
    offset = 0
    kept = 0
    log(f"sdot tree count {total}")
    tmp = TREES_JSONL + ".tmp"
    with gzip.open(tmp, "wt") as out:
        while total is None or offset < total:
            params = {
                "where": "1=1",
                "outFields": fields,
                "returnGeometry": "true",
                "outSR": "4326",
                "resultOffset": offset,
                "resultRecordCount": 2000,
                "f": "geojson",
            }
            for attempt in range(4):
                response = requests.get(TREES_SERVICE, params=params, timeout=120)
                if response.status_code == 200:
                    break
                time.sleep(3 * (attempt + 1))
            response.raise_for_status()
            payload = response.json()
            features = payload.get("features", [])
            for feature in features:
                props = feature.get("properties") or {}
                coords = (feature.get("geometry") or {}).get("coordinates")
                if not coords or coords[0] is None or coords[1] is None:
                    continue
                out.write(
                    json.dumps(
                        {
                            "id": props.get("OBJECTID"),
                            "lon": coords[0],
                            "lat": coords[1],
                            "common": props.get("COMMON_NAME"),
                            "scientific": props.get("SCIENTIFIC_NAME"),
                            "genus": props.get("GENUS"),
                            "height_ft": props.get("TREEHEIGHT"),
                            "diam_in": props.get("DIAM"),
                            "status": props.get("CURRENT_STATUS"),
                            "site": props.get("SITETYPE"),
                            "owner": props.get("OWNERSHIP"),
                        },
                        separators=(",", ":"),
                    )
                    + "\n"
                )
                kept += 1
            offset += len(features)
            if not features:
                break
            if offset % 20000 < 2000:
                log(f"sdot trees {offset}/{total} kept {kept}")
    os.replace(tmp, TREES_JSONL)
    log(f"sdot trees fetched {kept} in {time.time() - started:.1f}s")
    return kept, sha256(TREES_JSONL)


def load_canopy(nodes: list[dict]):
    """Window-read WorldCover over the pack bounds; keep the 10 m WGS84 window."""
    xs = [n["bounds"][0] for n in nodes if n["level"] == 2]
    zs = [n["bounds"][1] for n in nodes if n["level"] == 2]
    xe = [n["bounds"][2] for n in nodes if n["level"] == 2]
    ze = [n["bounds"][3] for n in nodes if n["level"] == 2]
    xmin, zmin, xmax, zmax = min(xs), min(zs), max(xe), max(ze)
    manifest = json.load(open(MANIFEST))
    anchor_lon, anchor_lat = manifest["region"]["anchor_wgs84"]
    to_2926 = Transformer.from_crs("EPSG:4326", "EPSG:2926", always_xy=True)
    origin = to_2926.transform(anchor_lon, anchor_lat)
    to_wgs = Transformer.from_crs("EPSG:2926", "EPSG:4326", always_xy=True)
    e0 = origin[0] + xmin / US_FOOT_M
    n0 = origin[1] - zmax / US_FOOT_M
    e1 = origin[0] + xmax / US_FOOT_M
    n1 = origin[1] - zmin / US_FOOT_M
    lon_min, lat_min = to_wgs.transform(e0, n0)
    lon_max, lat_max = to_wgs.transform(e1, n1)
    if os.path.exists(CANOPY_NPZ):
        cached = np.load(CANOPY_NPZ)
        return {
            "cover": cached["cover"],
            "left": float(cached["left"]),
            "top": float(cached["top"]),
            "px": float(cached["px"]),
            "py": float(cached["py"]),
        }
    import rasterio
    from rasterio.windows import from_bounds

    started = time.time()
    with rasterio.open(f"/vsicurl/{WORLDCOVER}") as src:
        window = from_bounds(lon_min, lat_min, lon_max, lat_max, transform=src.transform)
        cover = src.read(1, window=window)
        left, _bottom, _right, top = src.window_bounds(window)
        px = src.transform.a
        py = src.transform.e
    np.savez_compressed(
        CANOPY_NPZ, cover=cover.astype(np.uint8), left=left, top=top, px=px, py=py
    )
    log(
        f"worldcover canopy {cover.shape} tree cells "
        f"{int(np.isin(cover, (10, 20)).sum())} in {time.time() - started:.1f}s"
    )
    return {"cover": cover, "left": left, "top": top, "px": px, "py": py}


def species_for(genus: str | None, common: str | None, height_m: float) -> int:
    if genus and genus in GENUS_SPECIES:
        return GENUS_SPECIES[genus]
    text = f"{genus or ''} {common or ''}".lower()
    if any(word in text for word in ("fir", "cedar", "hemlock", "spruce", "pine")):
        return 0 if "fir" in text else 1
    if any(word in text for word in ("cherry", "plum", "crabapple", "pear", "magnolia")):
        return 4
    if "maple" in text:
        return 2
    if "madrone" in text:
        return 3
    if "plane" in text or "linden" in text or "oak" in text or "elm" in text:
        return 5
    if height_m >= 18.0:
        return 2
    if height_m <= 9.0:
        return 4
    return 5


def tint_for(key: int) -> int:
    return (key * 2654435761) % 3


def height_for(species: int, source_height_m: float | None, key: int) -> float:
    lo, hi = SPECIES[species][2]
    if source_height_m and source_height_m > 1.5:
        return float(min(max(source_height_m, lo * 0.7), hi))
    frac = ((key * 2246822519) % 1000) / 1000.0
    return lo + (hi - lo) * (0.35 + 0.65 * frac)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--refresh", action="store_true")
    args = parser.parse_args()

    started = time.time()
    nodes = parse_pack_nodes()
    log(f"pack nodes {len(nodes)}, neighborhoods {sum(1 for n in nodes if n['level'] == 2)}")
    tree_count, trees_sha = fetch_sdot(args.refresh)
    with open(PACK, "rb") as handle:
        pack_bytes = handle.read()
    canopy = load_canopy(nodes)
    log(f"canopy window ready {canopy['cover'].shape}")

    manifest = json.load(open(MANIFEST))
    anchor_lon, anchor_lat = manifest["region"]["anchor_wgs84"]
    to_2926 = Transformer.from_crs("EPSG:4326", "EPSG:2926", always_xy=True)
    to_wgs = Transformer.from_crs("EPSG:2926", "EPSG:4326", always_xy=True)
    origin = to_2926.transform(anchor_lon, anchor_lat)

    def world(lon: float, lat: float) -> tuple[float, float]:
        e, n = to_2926.transform(lon, lat)
        return (e - origin[0]) * US_FOOT_M, -(n - origin[1]) * US_FOOT_M

    # Global water polygons in world space.
    import geopandas as gpd

    water = gpd.read_file(WATER).to_crs("EPSG:2926")
    water_polys = []
    for geom in water.geometry:
        if geom is None or geom.is_empty or geom.geom_type not in ("Polygon", "MultiPolygon"):
            continue
        parts = list(geom.geoms) if geom.geom_type == "MultiPolygon" else [geom]
        for part in parts:
            coords = [((x - origin[0]) * US_FOOT_M, -(y - origin[1]) * US_FOOT_M) for x, y in part.exterior.coords]
            if len(coords) >= 4:
                water_polys.append(Polygon(coords))
    water_tree = STRtree(water_polys) if water_polys else None
    log(f"water polygons {len(water_polys)}")
    s2_report = json.load(open(os.path.join(ROOT, "datasets/seattle/s2_report.json")))
    wgrid = s2_report["grid"]
    wmask = np.frombuffer(
        open(os.path.join(ROOT, "assets/terrain/seattle_water_mask.bin"), "rb").read(),
        dtype=np.uint8,
    )
    w_rows, w_cols = int(wgrid["rows"]), int(wgrid["cols"])
    w_spacing = float(wgrid["spacing_m"])
    w_origin = wgrid["origin_world"]

    def wet(x: float, z: float) -> bool:
        col = int(round((x - w_origin[0]) / w_spacing))
        row = int(round((z - w_origin[1]) / w_spacing))
        if row < 0 or col < 0 or row >= w_rows or col >= w_cols:
            return False
        return int(wmask[row * w_cols + col]) != 0

    parks = gpd.read_file(PARKS_POLY).to_crs("EPSG:2926")
    park_polys = []
    for geom in parks.geometry:
        if geom is None or geom.is_empty or geom.geom_type not in ("Polygon", "MultiPolygon"):
            continue
        parts = list(geom.geoms) if geom.geom_type == "MultiPolygon" else [geom]
        for part in parts:
            coords = [((x - origin[0]) * US_FOOT_M, -(y - origin[1]) * US_FOOT_M) for x, y in part.exterior.coords]
            if len(coords) >= 4 and Polygon(coords).area > 200.0:
                park_polys.append(Polygon(coords))
    park_tree_global = STRtree(park_polys) if park_polys else None
    log(f"park polygons {len(park_polys)}")

    # Inventory points in world space, bucketed by neighborhood bounds.
    street_points: dict[int, list[tuple[float, float, dict]]] = {}
    street_records = 0
    kept_outside = 0
    neighborhoods = [i for i, n in enumerate(nodes) if n["level"] == 2]
    bounds_index = STRtree([box(n["bounds"][0], n["bounds"][1], n["bounds"][2], n["bounds"][3]) for n in nodes])
    with gzip.open(TREES_JSONL, "rt") as handle:
        for line in handle:
            record = json.loads(line)
            x, z = world(record["lon"], record["lat"])
            hits = bounds_index.query(Point(x, z))
            owner = None
            for hit in hits:
                node = nodes[int(hit)]
                if node["level"] != 2:
                    continue
                b = node["bounds"]
                if b[0] <= x <= b[2] and b[1] <= z <= b[3]:
                    owner = int(hit)
                    break
            if owner is None:
                kept_outside += 1
                continue
            street_points.setdefault(owner, []).append((x, z, record))
            street_records += 1
    log(f"street inventory placed {street_records}, outside {kept_outside}")

    # Build per-neighborhood tree lists.
    per_node: dict[int, list[tuple[float, float, float, int, int, int]]] = {}
    stats = {
        "street_inventory": 0,
        "synthetic_street": 0,
        "park": 0,
        "residential": 0,
        "rejected_building": 0,
        "rejected_road": 0,
        "rejected_water": 0,
        "rejected_spacing": 0,
    }
    road_half = {1: 9.0, 2: 7.0, 3: 5.0, 4: 4.0, 5: 3.5, 6: 3.0}
    road_offset = 4.5

    for node_index in neighborhoods:
        node = nodes[node_index]
        x0, z0, x1, z1 = node["bounds"]
        chunk = parse_chunk(pack_bytes, node["chunk_offset"], node["chunk_bytes"])
        buildings = chunk["buildings"]
        streets = chunk["streets"]
        building_tree = (
            STRtree([geom.buffer(0.6) for _h, geom in buildings]) if buildings else None
        )
        road_geoms = []
        for kind, line in streets:
            half = road_half.get(kind, 4.0)
            road_geoms.append(line.buffer(half + 0.5))
        road_tree = STRtree(road_geoms) if road_geoms else None

        park_hits = (
            park_tree_global.query(box(x0, z0, x1, z1)) if park_tree_global else []
        )
        park_local = [park_polys[int(hit)] for hit in park_hits]
        park_tree_res = STRtree(park_local) if park_local else None
        entries: list[tuple[float, float, float, int, int, int]] = []

        def excluded(px: float, pz: float) -> str | None:
            point = Point(px, pz)
            if wet(px, pz):
                return "water"
            if water_tree is not None and len(water_tree.query(point, predicate="intersects")):
                return "water"
            if building_tree is not None and len(building_tree.query(point, predicate="intersects")):
                return "building"
            if road_tree is not None and len(road_tree.query(point, predicate="intersects")):
                return "road"
            return None

        # (a) street trees: the real inventory.
        bucket = street_points.get(node_index, [])
        bucket.sort(key=lambda item: item[2]["id"] or 0)
        taken = 0
        for x, z, record in bucket:
            if taken >= CAP_STREET:
                break
            reason = excluded(x, z)
            if reason:
                stats[f"rejected_{reason}"] += 1
                continue
            height_m = (record["height_ft"] or 0.0) * 0.3048
            species = species_for(record["genus"], record["common"], height_m)
            h = height_for(species, height_m if height_m > 3 else None, record["id"] or 0)
            entries.append((x, z, h, species, tint_for(record["id"] or 0), REGIME_STREET))
            taken += 1
        stats["street_inventory"] += taken

        # (b) synthetic street trees on SND segments where the inventory is thin.
        if taken < 20 and streets:
            synthetic = []
            for kind, line in streets:
                if len(synthetic) >= CAP_SYNTHETIC:
                    break
                spacing = 12.0 if kind <= 2 else 10.0
                length = line.length
                if length < spacing:
                    continue
                count = int(length // spacing)
                for step in range(count):
                    distance = (step + 0.5) * spacing
                    point = line.interpolate(distance)
                    # Offset to the planting strip side (+normal), deterministic.
                    p0 = line.interpolate(max(0.0, distance - 1.0))
                    p1 = line.interpolate(min(length, distance + 1.0))
                    dx, dz = p1.x - p0.x, p1.y - p0.y
                    norm = math.hypot(dx, dz) or 1.0
                    nx, nz = -dz / norm, dx / norm
                    px = point.x + nx * road_offset
                    pz = point.y + nz * road_offset
                    if not (x0 <= px <= x1 and z0 <= pz <= z1):
                        continue
                    key = int(px * 13.0) * 73856093 ^ int(pz * 7.0) * 19349663
                    species = 2 if key % 5 else 4
                    h = height_for(species, None, key)
                    synthetic.append((px, pz, h, species, tint_for(key), REGIME_SYNTHETIC_STREET))
            synthetic.sort(key=lambda item: (item[0], item[1]))
            added = 0
            for px, pz, h, species, tint, regime in synthetic:
                if added >= CAP_SYNTHETIC:
                    break
                reason = excluded(px, pz)
                if reason:
                    stats[f"rejected_{reason}"] += 1
                    continue
                entries.append((px, pz, h, species, tint, regime))
                added += 1
            stats["synthetic_street"] += added
        # Remove street entries that fall inside park polygons (park regime owns them).
        if park_local:
            park_tree = STRtree(park_local)
            keep = []
            for entry in entries:
                if entry[5] == REGIME_STREET and len(park_tree.query(Point(entry[0], entry[1]), predicate="intersects")):
                    continue
                keep.append(entry)
            entries = keep

        # (c) park canopy: seeded jittered grid inside parks, denser than streets.
        park_added = 0
        if park_local:
            park_tree = STRtree(park_local)
            minx = max(x0, min(p.bounds[0] for p in park_local))
            maxx = min(x1, max(p.bounds[2] for p in park_local))
            minz = max(z0, min(p.bounds[1] for p in park_local))
            maxz = min(z1, max(p.bounds[3] for p in park_local))
            step = 11.5
            gx = minx
            while gx <= maxx and park_added < CAP_PARK:
                gz = minz
                while gz <= maxz and park_added < CAP_PARK:
                    key = int(gx * 73856093) ^ int(gz * 19349663) ^ node_index
                    frac = ((key * 2654435761) % 1000) / 1000.0
                    px = gx + (frac - 0.5) * step
                    pz = gz + (((key * 2246822519) % 1000) / 1000.0 - 0.5) * step
                    if len(park_tree.query(Point(px, pz), predicate="intersects")):
                        if excluded(px, pz) is None:
                            roll = (key * 1597334677) % 100
                            if roll < 42:
                                species = 0 if roll % 3 else 1
                            elif roll < 68:
                                species = 2
                            elif roll < 80:
                                species = 3
                            elif roll < 92:
                                species = 5
                            else:
                                species = 4
                            h = height_for(species, None, key)
                            entries.append((px, pz, h, species, tint_for(key), REGIME_PARK))
                            park_added += 1
                        else:
                            stats["rejected_spacing"] += 1
                    gz += step
                gx += step
        stats["park"] += park_added

        # (d) residential canopy scatter guided by WorldCover.
        cover = canopy["cover"]
        left = canopy["left"]
        top = canopy["top"]
        px = canopy["px"]
        py = canopy["py"]
        corners = [
            to_wgs.transform(origin[0] + x / US_FOOT_M, origin[1] - z / US_FOOT_M)
            for x, z in ((x0, z0), (x0, z1), (x1, z0), (x1, z1))
        ]
        lon_a = min(c[0] for c in corners)
        lon_b = max(c[0] for c in corners)
        lat_a = min(c[1] for c in corners)
        lat_b = max(c[1] for c in corners)
        col0 = max(0, int((lon_a - left) / px))
        col1 = min(cover.shape[1], int((lon_b - left) / px) + 1)
        row0 = max(0, int((top - lat_b) / (-py)))
        row1 = min(cover.shape[0], int((top - lat_a) / (-py)) + 1)
        residential_added = 0
        if col1 > col0 and row1 > row0:
            sub = cover[row0:row1:2, col0:col1:2]
            rows, cols = np.nonzero(np.isin(sub, (10, 20)))
            keys = (rows.astype(np.int64) * 73856093) ^ (cols.astype(np.int64) * 19349663) ^ node_index
            order = np.argsort(keys)
            for pick in order:
                if residential_added >= CAP_RESIDENTIAL:
                    break
                key = int(keys[pick])
                # Deterministic one-in-three thinning.
                if key % 3 != 0:
                    continue
                r = row0 + int(rows[pick]) * 2
                c = col0 + int(cols[pick]) * 2
                lon = left + (c + 0.5) * px
                lat = top + (r + 0.5) * py
                e, n = to_2926.transform(lon, lat)
                pxw = (e - origin[0]) * US_FOOT_M
                pzw = -(n - origin[1]) * US_FOOT_M
                if not (x0 <= pxw <= x1 and z0 <= pzw <= z1):
                    continue
                if park_local and len(park_tree_res.query(Point(pxw, pzw), predicate="intersects")):
                    continue
                if excluded(pxw, pzw) is not None:
                    stats["rejected_spacing"] += 1
                    continue
                roll = (key * 2654435761) % 100
                if roll < 34:
                    species = 0
                elif roll < 46:
                    species = 1
                elif roll < 72:
                    species = 2
                elif roll < 84:
                    species = 3
                elif roll < 94:
                    species = 5
                else:
                    species = 4
                h = height_for(species, None, key)
                entries.append((pxw, pzw, h, species, tint_for(key), REGIME_RESIDENTIAL))
                residential_added += 1
        stats["residential"] += residential_added

        entries.sort(key=lambda item: (item[5], item[0], item[1]))
        if node["name"] in ("Ballard 51", "Fremont 18", "Capitol Hill 32"):
            log("NODE", node["name"], "entries", len(entries))
        per_node[node_index] = entries

    # Serialise.
    directory = []
    body = bytearray()
    total = 0
    for index in range(len(nodes)):
        entries = per_node.get(index, [])
        if not entries:
            directory.append((0, 0))
            continue
        node = nodes[index]
        cx, cz = node["center"]
        offset = 18 + 8 * len(nodes) + len(body)
        for x, z, height, species, tint, regime in entries:
            qx = int(round((x - cx) * 10.0))
            qz = int(round((z - cz) * 10.0))
            qx = max(-32000, min(32000, qx))
            qz = max(-32000, min(32000, qz))
            body += struct.pack(
                "<hhHBBBB",
                qx,
                qz,
                int(round(height * 100.0)),
                species,
                tint,
                regime,
                0,
            )
            total += 1
        directory.append((offset, len(entries)))
    with open(TREE_BIN, "wb") as handle:
        handle.write(b"TRES")
        handle.write(struct.pack("<HIII", 1, len(nodes), total, 0))
        for offset, count in directory:
            handle.write(struct.pack("<II", offset, count))
        handle.write(body)
    log(f"wrote {TREE_BIN}: {total} trees in {sum(1 for c in directory if c[1])} chunks")

    by_species = {SPECIES[i][0]: 0 for i in range(len(SPECIES))}
    with open(TREE_BIN, "rb") as handle:
        handle.seek(18 + 8 * len(nodes))
        for _ in range(total):
            _qx, _qz, _h, species, _tint, _regime, _pad = struct.unpack("<hhHBBBB", handle.read(10))
            by_species[SPECIES[species][0]] += 1

    report = {
        "generated_utc": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "pack": {
            "path": "assets/datasets/seattle.citypack",
            "sha256": sha256(PACK),
            "nodes": len(nodes),
            "neighborhoods": len(neighborhoods),
        },
        "output": {
            "path": "assets/datasets/seattle_trees.bin",
            "sha256": sha256(TREE_BIN),
            "bytes": os.path.getsize(TREE_BIN),
            "trees": total,
            "record_bytes": 10,
            "record": "x i16 dm, z i16 dm (chunk-centre relative), height u16 cm, species u8, tint u8, regime u8, pad u8",
        },
        "sources": {
            "sdot_trees": {
                "service": TREES_SERVICE,
                "records": tree_count,
                "sha256": trees_sha,
                "license": "City of Seattle open data (data.seattle.gov terms)",
                "attribution": "City of Seattle, SDOT Trees (Seattle GeoData)",
            },
            "worldcover": {
                "url": WORLDCOVER,
                "product": "ESA WorldCover 2021 v200, 10 m, classes 10/20",
                "license": "CC BY 4.0 (ESA WorldCover)",
            },
            "parks": {
                "path": "datasets/seattle/raw/seattle_parks_polygons.geojson",
                "sha256": sha256(PARKS_POLY),
                "service": "https://services.arcgis.com/ZOyb2t4B0UYuYNYH/arcgis/rest/services/Park_Boundaries/FeatureServer/1",
                "license": "City of Seattle open data",
                "attribution": "City of Seattle GeoData - Park Boundaries (outline)",
            },
            "water": {
                "path": "datasets/seattle/raw/water_areas.geojson",
                "sha256": sha256(WATER),
                "license": "US Census TIGER/Line",
            },
        },
        "regimes": {
            "street_inventory": stats["street_inventory"],
            "synthetic_street": stats["synthetic_street"],
            "park": stats["park"],
            "residential": stats["residential"],
        },
        "caps": {
            "street": CAP_STREET,
            "park": CAP_PARK,
            "residential": CAP_RESIDENTIAL,
            "synthetic": CAP_SYNTHETIC,
        },
        "rejections": {k: v for k, v in stats.items() if k.startswith("rejected")},
        "species": by_species,
        "species_palette": [
            {"key": key, "archetype": arch, "height_m": [lo, hi]}
            for key, arch, (lo, hi) in SPECIES
        ],
        "seconds": round(time.time() - started, 1),
    }
    with open(REPORT, "w") as handle:
        json.dump(report, handle, indent=2)
        handle.write("\n")

    manifest["sources"].extend(
        [
            {
                "layer": "seattle_trees",
                "service": TREES_SERVICE,
                "attribution": "City of Seattle, SDOT Trees (Seattle GeoData)",
                "license": "City of Seattle open data (data.seattle.gov terms)",
                "crs": "EPSG:4326",
                "fetched_utc": report["generated_utc"],
                "records": tree_count,
                "sha256": trees_sha,
            },
            {
                "layer": "seattle_parks_polygons",
                "service": "https://services.arcgis.com/ZOyb2t4B0UYuYNYH/arcgis/rest/services/Park_Boundaries/FeatureServer/1",
                "attribution": "City of Seattle GeoData - Park Boundaries (outline)",
                "license": "City of Seattle open data",
                "crs": "EPSG:4326",
                "fetched_utc": report["generated_utc"],
                "sha256": sha256(PARKS_POLY),
            },
            {
                "layer": "worldcover_canopy",
                "service": WORLDCOVER,
                "attribution": "ESA WorldCover 2021 v200 (10 m)",
                "license": "CC BY 4.0",
                "crs": "EPSG:4326",
                "fetched_utc": report["generated_utc"],
            },
        ]
    )
    manifest.setdefault("derived", {})["trees"] = {
        "path": "assets/datasets/seattle_trees.bin",
        "report": "datasets/seattle/trees_report.json",
        "sha256": report["output"]["sha256"],
        "trees": total,
        "keyed_by": "CityPack node index (neighborhood level)",
    }
    manifest["counts"]["seattle_trees"] = total
    with open(MANIFEST, "w") as handle:
        json.dump(manifest, handle, indent=2)
        handle.write("\n")
    log(json.dumps(report["regimes"], indent=1))
    log(json.dumps(by_species, indent=1))
    log(f"done in {time.time() - started:.1f}s")


if __name__ == "__main__":
    main()
