#!/usr/bin/env python3
"""Seattle S1.5/S1c hierarchy partitioner + CityPack v2 writer.

Hierarchy: county -> city -> neighborhood (the S1.5 1,096 partitions) ->
leaf records (buildings and street segments). County boundaries come from the
Census TIGER/Line 2024 county layer; cities use the per-record source-agency
provenance (Seattle/Bellevue/Tacoma) plus TIGER 2024 places point-in-polygon.
Neighborhood names are the existing committed overlay; ancestry paths are
`County -> City -> Neighborhood`.

CityPack v2 (little endian):

    header  b"CDPK" u16 2 u32 header_bytes u32 node_count u32 buildings
            u32 streets u32 chunks
    nodes   u8 level i32 parent u32 first_child u16 child_count
            u32 buildings u32 streets f32 cx f32 cz
            f32 minx f32 minz f32 maxx f32 maxz
            u16 mean_h_cm u16 p95_h_cm u8 glass u8 dominant_category
            u8 dominant_source u8 name_len name
            u32 chunk_offset u32 chunk_bytes
    chunks  (neighborhoods only)
            u32 chunk_bytes u32 building_count u32 street_count
            f32 cx f32 cz f32 bounds[4]
            u8 dominant_category u8 dominant_source u16 mean_h u16 p95 u8 glass u8 flags
            u16 string_count [u16 len bytes]*
            buildings: as v1
            streets: u32 id u8 kind_idx u16 name_idx u16 point_count
                     [i16 qx i16 qz]*
"""

from __future__ import annotations

import json
import math
import os
import struct
import time
from collections import Counter, defaultdict

import geopandas as gpd
import numpy as np
from pyproj import Transformer
from shapely.geometry import Point, Polygon
from shapely.ops import transform

ROOT = "datasets/seattle"
RAW = f"{ROOT}/raw"
PACK = "assets/datasets/seattle.citypack"
PACK_TEST = "assets/datasets/seattle_test.citypack"
PARTITIONS = f"{ROOT}/partitions.json"
REPORT = f"{ROOT}/partition_report.json"
TARGET_MAX = 750
MIN_CELL = 250.0
ROOT_CELL = 2000.0
Q = 0.25
US_FOOT_M = 0.3048006096012192

NAMED_AREAS = {
    "Downtown": (-122.3355, 47.6062),
    "Capitol Hill": (-122.3197, 47.6232),
    "Ballard": (-122.3860, 47.6680),
    "Queen Anne": (-122.3570, 47.6370),
    "Fremont": (-122.3500, 47.6510),
    "University District": (-122.3040, 47.6550),
    "West Seattle": (-122.3870, 47.5710),
    "Beacon Hill": (-122.3110, 47.5790),
    "Rainier Valley": (-122.2830, 47.5530),
    "Bellevue CBD": (-122.2015, 47.6101),
    "Redmond": (-122.1215, 47.6740),
    "Tacoma Core": (-122.4443, 47.2529),
    "Tacoma North": (-122.4600, 47.3000),
    "South Corridor": (-122.3300, 47.3800),
}

SRC_CITY = [(0, 187_053, "Seattle"), (187_053, 243_909, "Bellevue"), (243_909, 330_330, "Tacoma")]


def q(value: float) -> int:
    return max(-32768, min(32767, int(round(value / Q))))


def cm(value: float) -> int:
    return max(0, min(65535, int(round(value * 100.0))))


def log(*args: object) -> None:
    print(*args, flush=True)


def context():
    manifest = json.load(open(f"{ROOT}/manifest.json"))
    to_2926 = Transformer.from_crs(4326, 2926, always_xy=True)
    origin = to_2926.transform(*manifest["region"]["anchor_wgs84"])
    return manifest, origin


def to_world(origin):
    def apply(x, y, z=None):
        return (x - origin[0]) * US_FOOT_M, -(y - origin[1]) * US_FOOT_M

    return apply


def load_admin(origin, region_polygon):
    apply = to_world(origin)
    counties = gpd.read_file(f"{RAW}/admin/tl_2024_us_county.zip")
    counties = counties[(counties["STATEFP"] == "53") & counties["NAME"].isin(["King", "Pierce", "Snohomish"])]
    counties = counties.to_crs(2926)
    counties["geometry"] = counties.geometry.apply(lambda g: transform(apply, g))
    places = gpd.read_file(f"{RAW}/admin/tl_2024_53_place.zip").to_crs(2926)
    places["geometry"] = places.geometry.apply(lambda g: transform(apply, g))
    places = places[places.intersects(region_polygon)]
    log("counties", [(r.NAME, round(r.geometry.area)) for r in counties.itertuples()])
    log("places in region", len(places))
    return counties, places


def load_streets(origin):
    frame = gpd.read_file(f"{RAW}/seattle_snd.geojson").set_crs(2926, allow_override=True)
    apply = to_world(origin)
    streets = []
    for row in frame.itertuples():
        geometry = row.geometry
        if geometry is None or geometry.is_empty:
            continue
        coords = list(geometry.coords)
        points = [apply(x, y) for x, y in coords]
        name = (row.ORD_STNAME_CONCAT or row.ORD_STREET_NAME or "").strip()
        streets.append(
            {
                "id": int(row.OBJECTID),
                "name": name,
                "kind": str(row.SEGMENT_TYPE),
                "points": points,
                "mid": [
                    sum(p[0] for p in points) / len(points),
                    sum(p[1] for p in points) / len(points),
                ],
            }
        )
    log("streets", len(streets))
    return streets


def build_cells(buildings):
    xs = [b["centroid"][0] for b in buildings]
    zs = [b["centroid"][1] for b in buildings]
    minx, maxx = min(xs) - 100.0, max(xs) + 100.0
    minz, maxz = min(zs) - 100.0, max(zs) + 100.0
    cells = [
        (minx + gx * ROOT_CELL, minz + gz * ROOT_CELL, ROOT_CELL)
        for gx in range(int(math.ceil((maxx - minx) / ROOT_CELL)))
        for gz in range(int(math.ceil((maxz - minz) / ROOT_CELL)))
    ]
    buckets = defaultdict(list)
    for index, building in enumerate(buildings):
        x, z = building["centroid"]
        key = (int((x - minx) // ROOT_CELL), int((z - minz) // ROOT_CELL))
        buckets[key].append(index)
    final = []
    stack = []
    for cell in cells:
        key = (int((cell[0] - minx) // ROOT_CELL), int((cell[1] - minz) // ROOT_CELL))
        if buckets.get(key):
            stack.append((cell, buckets[key]))
    while stack:
        cell, members = stack.pop()
        x0, z0, size = cell
        if len(members) > TARGET_MAX and size > MIN_CELL:
            half = size / 2.0
            for dx in (0.0, half):
                for dz in (0.0, half):
                    child = (x0 + dx, z0 + dz, half)
                    sub = [
                        i
                        for i in members
                        if child[0] <= buildings[i]["centroid"][0] < child[0] + half
                        and child[1] <= buildings[i]["centroid"][1] < child[1] + half
                    ]
                    if sub:
                        stack.append((child, sub))
        else:
            final.append((cell, members))
    final = [item for item in final if item[1]]
    covered = {index for _cell, members in final for index in members}
    for index in range(len(buildings)):
        if index in covered:
            continue
        centroid = buildings[index]["centroid"]
        best = min(
            range(len(final)),
            key=lambda pos: min(
                math.dist(centroid, buildings[member]["centroid"]) for member in final[pos][1]
            ),
        )
        final[best][1].append(index)
    final.sort(key=lambda item: (item[0][1], item[0][0]))
    return final


def name_cells(cells, buildings, anchors):
    names, used = [], Counter()
    for _cell, members in cells:
        cx = sum(buildings[i]["centroid"][0] for i in members) / len(members)
        cz = sum(buildings[i]["centroid"][1] for i in members) / len(members)
        best = None
        for name, (ax, az) in anchors.items():
            distance = math.dist((cx, cz), (ax, az))
            limit = 4000.0 if name in ("Bellevue CBD", "Tacoma Core", "Tacoma North") else 3000.0
            if distance <= limit and (best is None or distance < best[0]):
                best = (distance, name)
        base = best[1] if best else cardinal_name(cz, cx)
        used[base] += 1
        names.append(f"{base} {used[base]:02d}")
    return names


def cardinal_name(z, x):
    if z < -13000:
        return "North"
    if z > 40000:
        return "Tacoma South"
    if x > 8000:
        return "Eastside"
    if z > 15000:
        return "South King"
    return "Seattle South"


def assign_city(building, index, counties, places):
    source = next((name for start, end, name in SRC_CITY if start <= index < end), "Seattle")
    point = Point(building["centroid"])
    for row in places.itertuples():
        if row.NAME == source and row.geometry.contains(point):
            return source, "provenance"
    # Source provenance remains authoritative; report the few records whose
    # centroid falls outside the agency polygon (coastline/rounding) instead of
    # re-bucketing them and breaking the S1 city totals.
    return source, "provenance_outside_polygon"


def aggregate(buildings, indices):
    categories, sources, heights = Counter(), Counter(), []
    glass = 0
    for index in indices:
        building = buildings[index]
        categories[building["category"]] += 1
        sources[building["height_source"]] += 1
        heights.append(building["height_m"])
        if building["height_m"] >= 40.0 and building["category"] != "residential":
            glass += 1
    heights.sort()
    p95 = heights[min(len(heights) - 1, int(len(heights) * 0.95))]
    return {
        "dominant_category": categories.most_common(1)[0][0],
        "dominant_source": sources.most_common(1)[0][0],
        "mean_height_m": round(sum(heights) / len(heights), 2),
        "p95_height_m": round(p95, 2),
        "glass_pct": int(round(100.0 * glass / len(heights))),
        "category_counts": dict(categories),
        "source_counts": dict(sources),
    }


def chunk_bytes(center, bounds, members, streets, buildings):
    categories = sorted({buildings[i]["category"] for i in members})
    sources = sorted({buildings[i]["height_source"] for i in members})
    kinds = sorted({street["kind"] for street in streets})
    names = sorted({street["name"] for street in streets})
    strings = categories + sources + kinds + names
    index = {value: pos for pos, value in enumerate(strings)}
    hints = aggregate(buildings, members)
    heights = sorted(buildings[i]["height_m"] for i in members)
    p95 = heights[min(len(heights) - 1, int(len(heights) * 0.95))]
    body = bytearray()
    body += struct.pack("<III", 0, len(members), len(streets))
    body += struct.pack("<ff", *center)
    body += struct.pack("<ffff", *bounds)
    body += struct.pack(
        "<BBHHBB",
        categories.index(hints["dominant_category"]),
        sources.index(hints["dominant_source"]),
        cm(hints["mean_height_m"]),
        cm(p95),
        hints["glass_pct"],
        0,
    )
    body += struct.pack("<H", len(strings))
    for text in strings:
        raw = text.encode("utf-8")
        body += struct.pack("<H", len(raw)) + raw
    for i in members:
        building = buildings[i]
        cx, cz = building["centroid"]
        body += struct.pack(
            "<IhhHHHHHBBBH",
            int(building["id"]),
            q(cx - center[0]),
            q(cz - center[1]),
            cm(building["height_m"]),
            cm(building["floor_height_m"]),
            cm(building["base_elevation_m"]),
            min(65535, int(building.get("floor_count", 1) or 1)),
            min(65535, int(building.get("year_built", 0) or 0)),
            min(255, int(building.get("levels", 0) or 0)),
            index[building["height_source"]],
            index[building["category"]],
            min(65535, len(building["perimeter"])),
        )
        for vx, vz in building["perimeter"]:
            body += struct.pack("<hh", q(vx - cx), q(vz - cz))
    for street in streets:
        body += struct.pack(
            "<IBHH",
            int(street["id"]),
            index[street["kind"]],
            index[street["name"]],
            min(65535, len(street["points"])),
        )
        for vx, vz in street["points"]:
            body += struct.pack("<hh", q(vx - center[0]), q(vz - center[1]))
    struct.pack_into("<I", body, 0, len(body))
    return bytes(body), hints


def write_pack(nodes, chunks):
    def build_header(offsets):
        header = bytearray()
        header += b"CDPK"
        header += struct.pack(
            "<HIIIII",
            2,
            0,
            len(nodes),
            sum(n["buildings"] for n in nodes if n["level"] == 2),
            sum(n["streets"] for n in nodes if n["level"] == 2),
            len(chunks),
        )
        for index, node in enumerate(nodes):
            raw = node["name"].encode("utf-8")
            offset = offsets.get(index, 0)
            header += struct.pack(
                "<BiIHIIff" + "ffff" + "HHBBBB",
                node["level"],
                node["parent"],
                node["first_child"],
                node["child_count"],
                node["buildings"],
                node["streets"],
                node["center"][0],
                node["center"][1],
                *node["aabb"],
                cm(node["hints"].get("mean_height_m", 0.0)),
                cm(node["hints"].get("p95_height_m", 0.0)),
                node["hints"].get("glass_pct", 0),
                0,
                0,
                len(raw),
            )
            header += raw
            header += struct.pack("<II", offset, node.get("chunk_bytes", 0))
        return header

    header = build_header({})
    header_bytes = len(header)
    offsets = {}
    cursor = header_bytes
    for index, node in enumerate(nodes):
        if node["level"] == 2:
            offsets[index] = cursor
            cursor += node["chunk_bytes"]
    header = build_header(offsets)
    assert len(header) == header_bytes
    struct.pack_into("<I", header, 6, header_bytes)
    with open(PACK, "wb") as handle:
        handle.write(header)
        for chunk in chunks:
            handle.write(chunk)
    return header_bytes


def main() -> int:
    started = time.time()
    dataset = json.load(open(f"{ROOT}/seattle_v11_enriched.json"))
    buildings = dataset["buildings"]
    manifest, origin = context()
    region = Polygon(manifest["region"]["polygon_wgs84"])
    to_2926 = Transformer.from_crs(4326, 2926, always_xy=True)
    e0, n0 = origin
    region_world = Polygon(
        [
            ((to_2926.transform(lon, lat)[0] - e0) * US_FOOT_M, -(to_2926.transform(lon, lat)[1] - n0) * US_FOOT_M)
            for lon, lat in manifest["region"]["polygon_wgs84"]
        ]
    )
    counties, places = load_admin(origin, region_world)
    streets = load_streets(origin)

    city_of = []
    city_method = Counter()
    for index, building in enumerate(buildings):
        name, method = assign_city(building, index, counties, places)
        city_of.append(name)
        city_method[method] += 1
    log("city methods", dict(city_method), "city counts", Counter(city_of))

    county_of_city = {"Seattle": "King", "Bellevue": "King", "Tacoma": "Pierce"}

    anchors = {}
    for name, (lon, lat) in NAMED_AREAS.items():
        e, n = to_2926.transform(lon, lat)
        anchors[name] = ((e - e0) * US_FOOT_M, -(n - n0) * US_FOOT_M)
    cells = build_cells(buildings)
    names = name_cells(cells, buildings, anchors)

    # Neighborhood assignment of streets (Seattle SND) by nearest Seattle cell.
    seattle_cells = [i for i, (_cell, members) in enumerate(cells) if city_of[members[0]] == "Seattle"]
    seattle_centers = np.array(
        [
            [
                sum(buildings[i]["centroid"][0] for i in cells[c][1]) / len(cells[c][1]),
                sum(buildings[i]["centroid"][1] for i in cells[c][1]) / len(cells[c][1]),
            ]
            for c in seattle_cells
        ]
    )
    street_buckets = defaultdict(list)
    for street in streets:
        mid = np.array(street["mid"])
        nearest = int(np.argmin(np.sum((seattle_centers - mid) ** 2, axis=1)))
        street_buckets[seattle_cells[nearest]].append(street)
    log("streets assigned to Seattle neighborhoods", sum(len(v) for v in street_buckets.values()))

    # Neighborhood records.
    neighborhoods = []
    for position, (cell, members) in enumerate(cells):
        xs = [buildings[i]["centroid"][0] for i in members]
        zs = [buildings[i]["centroid"][1] for i in members]
        for i in members:
            xs.extend(p[0] for p in buildings[i]["perimeter"])
            zs.extend(p[1] for p in buildings[i]["perimeter"])
        for street in street_buckets.get(position, []):
            xs.extend(p[0] for p in street["points"])
            zs.extend(p[1] for p in street["points"])
        bounds = [min(xs), min(zs), max(xs), max(zs)]
        center = [(bounds[0] + bounds[2]) / 2.0, (bounds[1] + bounds[3]) / 2.0]
        majority_city = Counter(city_of[i] for i in members).most_common(1)[0][0]
        neighborhoods.append(
            {
                "level": 2,
                "name": names[position],
                "city": majority_city,
                "members": members,
                "street_records": street_buckets.get(position, []),
                "streets": len(street_buckets.get(position, [])),
                "aabb": bounds,
                "center": center,
                "hints": aggregate(buildings, members),
                "buildings": len(members),
            }
        )

    # City/county nodes, ordered counties -> cities -> neighborhoods.
    counties_present = sorted({county_of_city[city] for city in set(city_of)})
    nodes = []
    county_index = {}
    city_index = {}
    for county in counties_present:
        county_index[county] = len(nodes)
        nodes.append(
            {
                "level": 0,
                "name": county,
                "parent": -1,
                "first_child": 0,
                "child_count": 0,
                "buildings": 0,
                "streets": 0,
                "center": [0.0, 0.0],
                "aabb": [0.0, 0.0, 0.0, 0.0],
                "hints": {},
                "chunk_offset": 0,
                "chunk_bytes": 0,
            }
        )
    for county in counties_present:
        cities = sorted({city for city in set(city_of) if county_of_city[city] == county})
        for city in cities:
            city_index[city] = len(nodes)
            nodes.append(
                {
                    "level": 1,
                    "name": city,
                    "parent": county_index[county],
                    "first_child": 0,
                    "child_count": 0,
                    "buildings": 0,
                    "streets": 0,
                    "center": [0.0, 0.0],
                    "aabb": [0.0, 0.0, 0.0, 0.0],
                    "hints": {},
                    "chunk_offset": 0,
                    "chunk_bytes": 0,
                }
            )
    neighborhoods.sort(key=lambda n: (city_index[n["city"]], n["name"]))
    neighborhood_base = len(nodes)
    for neighborhood in neighborhoods:
        neighborhood["parent"] = city_index[neighborhood["city"]]
        neighborhood["first_child"] = 0
        neighborhood["child_count"] = 0
        neighborhood["chunk_offset"] = 0
        neighborhood["chunk_bytes"] = 0
        nodes.append(neighborhood)

    # Parent AABBs from children; place/county polygon bounds are added where available.
    def parent_bounds(children, extra=None):
        b = [
            min(nodes[i]["aabb"][0] for i in children),
            min(nodes[i]["aabb"][1] for i in children),
            max(nodes[i]["aabb"][2] for i in children),
            max(nodes[i]["aabb"][3] for i in children),
        ]
        # Keep parent AABBs tight to the data extent so culling prunes early.
        let_unused = extra
        _ = let_unused
        pad = 0.5
        return [b[0] - pad, b[1] - pad, b[2] + pad, b[3] + pad]

    for city, index in city_index.items():
        children = [i for i, node in enumerate(nodes) if node["parent"] == index]
        rows = [row for row in places.itertuples() if row.NAME == city]
        extra = rows[0].geometry.bounds if rows else None
        nodes[index]["aabb"] = parent_bounds(children, extra)
        nodes[index]["first_child"] = min(children)
        nodes[index]["child_count"] = len(children)
        nodes[index]["buildings"] = sum(nodes[i]["buildings"] for i in children)
        nodes[index]["streets"] = sum(nodes[i]["streets"] for i in children)
        nodes[index]["center"] = [
            (nodes[index]["aabb"][0] + nodes[index]["aabb"][2]) / 2.0,
            (nodes[index]["aabb"][1] + nodes[index]["aabb"][3]) / 2.0,
        ]
        member_indices = [i for child in children for i in nodes[child]["members"]]
        nodes[index]["hints"] = aggregate(buildings, member_indices)
    for county in counties_present:
        children = [i for i, node in enumerate(nodes) if node["parent"] == county_index[county]]
        nodes[county_index[county]]["aabb"] = parent_bounds(children)
        nodes[county_index[county]]["first_child"] = min(children)
        nodes[county_index[county]]["child_count"] = len(children)
    def leaf_members(index):
        node = nodes[index]
        if node["level"] == 2:
            return node["members"]
        return [
            member
            for child in range(node["first_child"], node["first_child"] + node["child_count"])
            for member in leaf_members(child)
        ]

    for county in counties_present:
        index = county_index[county]
        children = [i for i, node in enumerate(nodes) if node["parent"] == index]
        nodes[index]["buildings"] = sum(nodes[i]["buildings"] for i in children)
        nodes[index]["streets"] = sum(nodes[i]["streets"] for i in children)
        nodes[index]["center"] = [
            (nodes[index]["aabb"][0] + nodes[index]["aabb"][2]) / 2.0,
            (nodes[index]["aabb"][1] + nodes[index]["aabb"][3]) / 2.0,
        ]
        member_indices = leaf_members(index)
        nodes[index]["hints"] = aggregate(buildings, member_indices)

    # Chunks.
    chunks = []
    for neighborhood in neighborhoods:
        raw, hints = chunk_bytes(
            neighborhood["center"],
            neighborhood["aabb"],
            neighborhood["members"],
            neighborhood["street_records"],
            buildings,
        )
        neighborhood["chunk_index"] = len(chunks)
        chunks.append(raw)
    offset = 0
    header_estimate = 0
    # header bytes are computed in write_pack; offsets need it first.
    for node in nodes:
        if node["level"] == 2:
            node["chunk_bytes"] = len(chunks[node["chunk_index"]])
    # First pass to get header size.
    header_probe = bytearray()
    header_probe += b"CDPK"
    header_probe += struct.pack("<HIIIII", 2, 0, len(nodes), 330_330, len(streets), len(chunks))
    for node in nodes:
        raw = node["name"].encode("utf-8")
        header_probe += struct.pack(
            "<BiIHIIff" + "ffff" + "HHBBBB",
            node["level"], node["parent"], node["first_child"], node["child_count"],
            node["buildings"], node["streets"], node["center"][0], node["center"][1],
            *node["aabb"], cm(node["hints"].get("mean_height_m", 0.0)),
            cm(node["hints"].get("p95_height_m", 0.0)), node["hints"].get("glass_pct", 0),
            0, 0, len(raw),
        ) + raw + struct.pack("<II", 0, 0)
    header_estimate = len(header_probe)
    offset = header_estimate
    for node in neighborhoods:
        source = chunks[node["chunk_index"]]
        node["chunk_offset"] = offset
        offset += len(source)
    header_bytes = write_pack(nodes, chunks)
    assert header_bytes == header_estimate, (header_bytes, header_estimate)

    # Fixture: first three neighborhoods.
    selected = [n for n in nodes if n["level"] == 2][:3]
    fixture_nodes = []
    fixture_chunks = []
    fixture_header = 0
    # Rebuild a miniature pack using the same writer would require index remap;
    # emit a standalone v2 pack with parent = -1 for the three neighborhoods.
    fixture_nodes = []
    for node in selected:
        fixture_nodes.append(
            {
                **node,
                "parent": -1,
                "level": 2,
                "first_child": 0,
                "child_count": 0,
                "chunk_offset": 0,
                "chunk_bytes": len(chunks[node["chunk_index"]]),
            }
        )
    # Use the main writer into a temp path by temporarily swapping PACK.
    global PACK
    saved = PACK
    PACK = PACK_TEST
    write_pack(fixture_nodes, [chunks[n["chunk_index"]] for n in selected])
    PACK = saved

    first = selected[0]
    samples = [
        {
            "id": buildings[i]["id"],
            "centroid": buildings[i]["centroid"],
            "height_m": buildings[i]["height_m"],
            "source": buildings[i]["height_source"],
            "perimeter": buildings[i]["perimeter"],
        }
        for i in first["members"][:12]
    ]
    expected = {
        "version": 2,
        "nodes": len(selected),
        "buildings": sum(n["buildings"] for n in selected),
        "streets": sum(n["streets"] for n in selected),
        "names": [n["name"] for n in selected],
        "samples": samples,
    }
    with open("assets/datasets/seattle_test_expected.json", "w") as handle:
        json.dump(expected, handle, indent=2)

    # Integrity: strict AABB containment.
    max_overflow = 0.0
    for node in nodes:
        if node["parent"] < 0:
            continue
        parent = nodes[node["parent"]]["aabb"]
        child = node["aabb"]
        overflow = max(
            parent[0] - child[0],
            parent[1] - child[1],
            child[2] - parent[2],
            child[3] - parent[3],
            0.0,
        )
        max_overflow = max(max_overflow, overflow)
    if max_overflow > 0.01:
        raise SystemExit(f"hierarchy containment failed: {max_overflow}")

    cities_report = Counter(city_of)
    counties_report = Counter(county_of_city[city] for city in city_of)
    street_total = sum(len(v) for v in street_buckets.values())
    report = {
        "generated_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "levels": {
            "counties": len(counties_present),
            "cities": len(city_index),
            "neighborhoods": len(neighborhoods),
            "buildings": 330_330,
            "streets": street_total,
        },
        "county_counts": dict(counties_report),
        "city_counts": dict(cities_report),
        "provenance_methods": dict(city_method),
        "names": {
            "counties": {county: [city for city in city_index if county_of_city[city] == county] for county in counties_present},
            "sample_paths": [
                f"{county_of_city[n['city']]} -> {n['city']} -> {n['name']}" for n in neighborhoods[:8]
            ],
        },
        "containment_max_overflow_m": round(max_overflow, 6),
        "binary_bytes": os.path.getsize(PACK),
        "json_bytes": os.path.getsize(f"{ROOT}/seattle_v11_enriched.json"),
        "single_partition_bytes": len(chunks[0]),
        "single_partition_json_bytes": len(
            json.dumps([buildings[i] for i in neighborhoods[0]["members"]], separators=(",", ":"))
        ),
        "partition_size_histogram": dict(
            Counter(
                "<250" if n["buildings"] < 250 else "250-750" if n["buildings"] <= 750 else ">750"
                for n in neighborhoods
            )
        ),
        "mean_partition": round(sum(n["buildings"] for n in neighborhoods) / len(neighborhoods), 1),
        "max_partition": max(n["buildings"] for n in neighborhoods),
        "min_partition": min(n["buildings"] for n in neighborhoods),
    }
    with open(REPORT, "w") as handle:
        json.dump(report, handle, indent=2)
    with open(PARTITIONS, "w") as handle:
        json.dump(
            {
                "generated_utc": report["generated_utc"],
                "scheme": "county -> city -> neighborhood (adaptive quadtree 2 km -> 250 m)",
                "nodes": [
                    {
                        "level": node["level"],
                        "name": node["name"],
                        "parent": node["parent"],
                        "center": [round(v, 2) for v in node["center"]],
                        "aabb": [round(v, 2) for v in node["aabb"]],
                        "buildings": node["buildings"],
                        "streets": node["streets"],
                        "hints": node["hints"],
                    }
                    for node in nodes
                ],
            },
            handle,
            indent=2,
        )
    log("LEVELS", report["levels"])
    log("COUNTIES", dict(counties_report), "CITIES", dict(cities_report))
    log("METHODS", dict(city_method))
    log("CONTAINMENT", max_overflow)
    log("SIZES", report["binary_bytes"], report["json_bytes"], report["single_partition_bytes"], report["single_partition_json_bytes"])
    log("wrote", PACK, PACK_TEST, PARTITIONS, REPORT)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
