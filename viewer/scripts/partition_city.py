#!/usr/bin/env python3
"""Seattle S1.5 neighborhood partition + CityPack binary writer.

Partition scheme: adaptive quadtree over the region. Start at 2 km cells and
split a cell while it holds more than 750 building centroids and is larger than
250 m. Sparse cells stay large (below the 250 target); dense downtown cells
bottom out near 500 buildings. Each partition gets a name from the committed
named-area overlay (nearest anchor within 3 km, else a cardinal fallback) plus
aggregate style hints.

CityPack v1 layout (little endian):

    header  b"CDPK" u16 version u32 header_bytes u32 partitions u32 buildings
    dir     [u32 offset u32 bytes u8 name_len name]
    chunks  u32 chunk_bytes u32 building_count
            f32 center_x f32 center_z f32 min_x f32 min_z f32 max_x f32 max_z
            u8 dominant_category u8 dominant_source u16 mean_height_cm
            u16 p95_height_cm u8 glass_pct u8 reserved
            u16 string_count [u16 len bytes]*
            buildings:
              u32 id i16 qx i16 qz u16 height_cm u16 floor_height_cm
              u16 base_cm u16 floor_count u16 year_built u8 levels
              u8 source_idx u8 category_idx u16 vertex_count
              [i16 qx i16 qz]*vertex_count

Coordinates are quantized at 0.25 m relative to the partition centre (centroid)
or building centroid (vertices); heights are integer centimetres.
"""

from __future__ import annotations

import json
import math
import os
import struct
import time

ROOT = "datasets/seattle"
PACK = "assets/datasets/seattle.citypack"
PACK_TEST = "assets/datasets/seattle_test.citypack"
PARTITIONS = f"{ROOT}/partitions.json"
REPORT = f"{ROOT}/partition_report.json"
TARGET_MAX = 750
TARGET_MIN = 250
MIN_CELL = 250.0
ROOT_CELL = 2000.0
Q = 0.25

# WGS84 anchors; converted to the dataset world frame at run time.
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


def q(value: float) -> int:
    return max(-32768, min(32767, int(round(value / Q))))


def clamp_cm(value: float) -> int:
    return max(0, min(65535, int(round(value * 100.0))))


def load():
    dataset = json.load(open(f"{ROOT}/seattle_v11_enriched.json"))
    return dataset


def world_anchors():
    from pyproj import Transformer

    manifest = json.load(open(f"{ROOT}/manifest.json"))
    to_2926 = Transformer.from_crs(4326, 2926, always_xy=True)
    e0, n0 = to_2926.transform(*manifest["region"]["anchor_wgs84"])
    out = {}
    for name, (lon, lat) in NAMED_AREAS.items():
        e, n = to_2926.transform(lon, lat)
        out[name] = ((e - e0) * 0.3048006096012192, -(n - n0) * 0.3048006096012192)
    return out


def build_cells(buildings):
    xs = [b["centroid"][0] for b in buildings]
    zs = [b["centroid"][1] for b in buildings]
    minx, maxx = min(xs) - 100.0, max(xs) + 100.0
    minz, maxz = min(zs) - 100.0, max(zs) + 100.0
    root_count = (
        int(math.ceil((maxx - minx) / ROOT_CELL)),
        int(math.ceil((maxz - minz) / ROOT_CELL)),
    )
    cells = [
        (minx + gx * ROOT_CELL, minz + gz * ROOT_CELL, ROOT_CELL)
        for gx in range(root_count[0])
        for gz in range(root_count[1])
    ]

    buckets = {}
    for index, building in enumerate(buildings):
        x, z = building["centroid"]
        key = (int((x - minx) // ROOT_CELL), int((z - minz) // ROOT_CELL))
        buckets.setdefault(key, []).append(index)

    final = []
    stack = []
    for cell in cells:
        x0, z0, _size = cell
        key = (int((x0 - minx) // ROOT_CELL), int((z0 - minz) // ROOT_CELL))
        members = buckets.get(key, [])
        if members:
            stack.append((cell, members))
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
    missing = [index for index in range(len(buildings)) if index not in covered]
    for index in missing:
        centroid = buildings[index]["centroid"]
        best = min(
            range(len(final)),
            key=lambda pos: min(
                math.dist(centroid, buildings[member]["centroid"])
                for member in final[pos][1]
            ),
        )
        final[best][1].append(index)
    if missing:
        print(f"partition boundary rescue: {len(missing)} buildings")
    final.sort(key=lambda item: (item[0][1], item[0][0]))
    return final


def name_cells(cells, buildings, anchors):
    names = []
    used = {}
    for cell, members in cells:
        cx = sum(buildings[i]["centroid"][0] for i in members) / len(members)
        cz = sum(buildings[i]["centroid"][1] for i in members) / len(members)
        best = None
        for name, (ax, az) in anchors.items():
            distance = math.dist((cx, cz), (ax, az))
            limit = 4000.0 if name in ("Bellevue CBD", "Tacoma Core", "Tacoma North") else 3000.0
            if distance <= limit and (best is None or distance < best[0]):
                best = (distance, name)
        base = best[1] if best else cardinal_name(cz, cx)
        used[base] = used.get(base, 0) + 1
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


def aggregate(buildings, members):
    categories = {}
    sources = {}
    heights = []
    glass = 0
    for index in members:
        building = buildings[index]
        categories[building["category"]] = categories.get(building["category"], 0) + 1
        sources[building["height_source"]] = sources.get(building["height_source"], 0) + 1
        heights.append(building["height_m"])
        if building["height_m"] >= 40.0 and building["category"] != "residential":
            glass += 1
    heights.sort()
    p95 = heights[min(len(heights) - 1, int(len(heights) * 0.95))]
    dominant_category = max(categories, key=categories.get)
    dominant_source = max(sources, key=sources.get)
    return {
        "dominant_category": dominant_category,
        "dominant_source": dominant_source,
        "mean_height_m": round(sum(heights) / len(heights), 2),
        "p95_height_m": round(p95, 2),
        "glass_pct": int(round(100.0 * glass / len(heights))),
        "category_counts": categories,
        "source_counts": sources,
    }


def chunk_bytes(name, center, bounds, members, buildings):
    categories = sorted({buildings[i]["category"] for i in members})
    sources = sorted({buildings[i]["height_source"] for i in members})
    strings = categories + sources
    index = {value: pos for pos, value in enumerate(strings)}
    hints = aggregate(buildings, members)
    heights = sorted(buildings[i]["height_m"] for i in members)
    p95 = heights[min(len(heights) - 1, int(len(heights) * 0.95))]
    body = bytearray()
    body += struct.pack("<II", 0, len(members))  # size patched later
    body += struct.pack("<ff", *center)
    body += struct.pack("<ffff", *bounds)
    body += struct.pack(
        "<BBHHBB",
        categories.index(hints["dominant_category"]),
        sources.index(hints["dominant_source"]),
        clamp_cm(hints["mean_height_m"]),
        clamp_cm(p95),
        hints["glass_pct"],
        0,
    )
    body += struct.pack("<H", len(strings))
    for text in strings:
        raw = text.encode("utf-8")
        body += struct.pack("<H", len(raw)) + raw
    for index_in_members in members:
        building = buildings[index_in_members]
        cx, cz = building["centroid"]
        lx, lz = cx - center[0], cz - center[1]
        body += struct.pack(
            "<IhhHHHHHBBBH",
            int(building["id"]),
            q(lx),
            q(lz),
            clamp_cm(building["height_m"]),
            clamp_cm(building["floor_height_m"]),
            clamp_cm(building["base_elevation_m"]),
            min(65535, int(building.get("floor_count", 1) or 1)),
            min(65535, int(building.get("year_built", 0) or 0)),
            min(255, int(building.get("levels", 0) or 0)),
            index[building["height_source"]],
            index[building["category"]],
            min(65535, len(building["perimeter"])),
        )
        for vx, vz in building["perimeter"]:
            body += struct.pack("<hh", q(vx - cx), q(vz - cz))
    struct.pack_into("<I", body, 0, len(body))
    return bytes(body), hints


def main() -> int:
    started = time.time()
    dataset = load()
    buildings = dataset["buildings"]
    anchors = world_anchors()
    cells = build_cells(buildings)
    names = name_cells(cells, buildings, anchors)
    log = lambda *a: print(*a, flush=True)
    log(f"partitions {len(cells)} buildings {len(buildings)}")

    chunks = []
    directory = []
    partition_rows = []
    offset = 0
    sizes = [len(members) for _, members in cells]
    names_unique = []
    for (cell, members), name in zip(cells, names):
        xs = [buildings[i]["centroid"][0] for i in members]
        zs = [buildings[i]["centroid"][1] for i in members]
        for i in members:
            xs.extend(p[0] for p in buildings[i]["perimeter"])
            zs.extend(p[1] for p in buildings[i]["perimeter"])
        bounds = (min(xs), min(zs), max(xs), max(zs))
        center = (
            (bounds[0] + bounds[2]) / 2.0,
            (bounds[1] + bounds[3]) / 2.0,
        )
        raw, hints = chunk_bytes(name, center, bounds, members, buildings)
        chunk_offset = offset
        chunks.append(raw)
        directory.append((chunk_offset, len(raw), name))
        offset += len(raw)
        names_unique.append(name)
        partition_rows.append(
            {
                "name": name,
                "index": len(partition_rows),
                "center": [round(center[0], 2), round(center[1], 2)],
                "aabb": [round(v, 2) for v in bounds],
                "buildings": len(members),
                "hints": hints,
            }
        )

    header = bytearray()
    header += b"CDPK"
    header += struct.pack("<HIIII", 1, 0, len(chunks), len(buildings), 0)
    for _chunk_offset, chunk_size, name in directory:
        raw = name.encode("utf-8")
        header += struct.pack("<IIB", 0, chunk_size, len(raw)) + raw
    header_bytes = len(header)
    # Directory offsets are absolute from the file start.
    header = bytearray()
    header += b"CDPK"
    header += struct.pack("<HIIII", 1, 0, len(chunks), len(buildings), 0)
    for chunk_offset, chunk_size, name in directory:
        raw = name.encode("utf-8")
        header += struct.pack("<IIB", header_bytes + chunk_offset, chunk_size, len(raw)) + raw
    assert len(header) == header_bytes
    struct.pack_into("<I", header, 6, header_bytes)
    with open(PACK, "wb") as handle:
        handle.write(header)
        for chunk in chunks:
            handle.write(chunk)

    # Small fixture: first three partitions + expected assertions.
    test_header = bytearray()
    test_header += b"CDPK"
    selected = partition_rows[:3]
    fixture = bytearray()
    test_dir = []
    test_offset = 0
    for row in selected:
        source = chunks[row["index"]]
        test_dir.append((test_offset, len(source), row["name"]))
        fixture += source
        test_offset += len(source)
    test_header += struct.pack("<HIIII", 1, 0, len(selected), sum(row["buildings"] for row in selected), 0)
    for _chunk_offset, chunk_size, name in test_dir:
        raw = name.encode("utf-8")
        test_header += struct.pack("<IIB", 0, chunk_size, len(raw)) + raw
    test_header_bytes = len(test_header)
    test_header = bytearray()
    test_header += b"CDPK"
    test_header += struct.pack("<HIIII", 1, 0, len(selected), sum(row["buildings"] for row in selected), 0)
    for chunk_offset, chunk_size, name in test_dir:
        raw = name.encode("utf-8")
        test_header += struct.pack("<IIB", test_header_bytes + chunk_offset, chunk_size, len(raw)) + raw
    assert len(test_header) == test_header_bytes
    struct.pack_into("<I", test_header, 6, test_header_bytes)
    with open(PACK_TEST, "wb") as handle:
        handle.write(test_header + fixture)

    first = selected[0]
    first_members = cells[0][1][:12]
    expected = {
        "partitions": len(selected),
        "buildings": sum(row["buildings"] for row in selected),
        "names": [row["name"] for row in selected],
        "first_partition_center": first["center"],
        "samples": [
            {
                "id": buildings[i]["id"],
                "centroid": buildings[i]["centroid"],
                "height_m": buildings[i]["height_m"],
                "source": buildings[i]["height_source"],
                "perimeter": buildings[i]["perimeter"],
            }
            for i in first_members
        ],
    }
    with open("assets/datasets/seattle_test_expected.json", "w") as handle:
        json.dump(expected, handle, indent=2)

    from collections import Counter

    histogram = Counter()
    for size in sizes:
        if size < 250:
            histogram["<250"] += 1
        elif size <= 750:
            histogram["250-750"] += 1
        else:
            histogram[">750"] += 1
    report = {
        "generated_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "scheme": "adaptive quadtree 2 km -> 250 m; split >750; no merge",
        "partitions": len(cells),
        "buildings": len(buildings),
        "size_histogram": dict(histogram),
        "min_partition": min(sizes),
        "max_partition": max(sizes),
        "mean_partition": round(sum(sizes) / len(sizes), 1),
        "named_areas": sorted(set(row["name"].rsplit(" ", 1)[0] for row in partition_rows)),
        "binary_bytes": os.path.getsize(PACK),
        "json_bytes": os.path.getsize(f"{ROOT}/seattle_v11_enriched.json"),
        "single_partition_bytes": len(chunks[0]),
        "single_partition_json_bytes": len(
            json.dumps([buildings[i] for i in cells[0][1]], separators=(",", ":"))
        ),
        "first_partition": partition_rows[0],
        "seconds": round(time.time() - started, 1),
    }
    with open(REPORT, "w") as handle:
        json.dump(report, handle, indent=2)
    with open(PARTITIONS, "w") as handle:
        json.dump(
            {
                "generated_utc": report["generated_utc"],
                "scheme": report["scheme"],
                "partitions": partition_rows,
            },
            handle,
            indent=2,
        )
    log("HISTOGRAM", dict(histogram), "min/max", min(sizes), max(sizes))
    log(
        "SIZES binary",
        report["binary_bytes"],
        "json",
        report["json_bytes"],
        "ratio",
        round(report["json_bytes"] / report["binary_bytes"], 2),
        "first chunk",
        report["single_partition_bytes"],
    )
    log("wrote", PACK, PACK_TEST, PARTITIONS, REPORT)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
