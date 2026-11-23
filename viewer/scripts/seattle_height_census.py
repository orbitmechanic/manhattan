#!/usr/bin/env python3
"""Chimney census: which pack records exceed the human-scale governor?

Reads the CityPack, the district map and the style pack, replays the same rule
order and governor as `viewer/src/geology.rs::building_spec`, and reports, per
neighborhood:

  records            total building records in the partition
  residential        records whose category string is "residential"
  over_cap           records whose recorded height exceeds the governor-applied
                     height by more than 5 cm
  max_recorded       tallest recorded height
  max_applied        tallest governor-applied height
  families           family -> count (from the matched style rule)
  sources            source string -> count (enriched / audited / ...)

Usage: python3 scripts/seattle_height_census.py [--json out.json]
"""

from __future__ import annotations

import json
import struct
import sys
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PACK = ROOT / "assets/datasets/seattle.citypack"
DISTRICTS = ROOT / "assets/styles/seattle_districts.json"
STYLE = ROOT / "assets/styles/seattle_pack.json"

CATEGORY_RESIDENTIAL = "residential"


def parse_nodes(data: bytes) -> list[dict]:
    version, _header, node_count, *_ = struct.unpack_from("<HIIIII", data, 4)
    if version != 2:
        raise SystemExit(f"unsupported citypack version {version}")
    nodes = []
    pos = 26
    for _ in range(node_count):
        level = data[pos]
        parent = struct.unpack_from("<i", data, pos + 1)[0]
        buildings = struct.unpack_from("<I", data, pos + 11)[0]
        center = struct.unpack_from("<ff", data, pos + 19)
        bounds = struct.unpack_from("<ffff", data, pos + 27)
        pos += 50
        name_len = data[pos]
        name = data[pos + 1 : pos + 1 + name_len].decode("utf-8", "replace")
        pos += 1 + name_len
        offset, size = struct.unpack_from("<II", data, pos)
        pos += 8
        nodes.append(
            {
                "level": level,
                "parent": parent,
                "buildings": buildings,
                "center": center,
                "bounds": bounds,
                "name": name,
                "offset": offset,
                "size": size,
            }
        )
    return nodes


def parse_buildings(data: bytes, node: dict) -> list[dict]:
    blob = data[node["offset"] : node["offset"] + node["size"]]
    _chunk_bytes, building_count, _street_count = struct.unpack_from("<III", blob, 0)
    center = struct.unpack_from("<ff", blob, 12)
    pos = 20 + 16 + 7 + 1
    string_count = struct.unpack_from("<H", blob, pos)[0]
    pos += 2
    strings = []
    for _ in range(string_count):
        length = struct.unpack_from("<H", blob, pos)[0]
        strings.append(blob[pos + 2 : pos + 2 + length].decode("utf-8", "replace"))
        pos += 2 + length
    out = []
    for _ in range(building_count):
        (
            building_id,
            qx,
            qz,
            height_cm,
            _floor_height_cm,
            _base_cm,
            _floor_count,
            _year,
            _levels,
            source_idx,
            category_idx,
            vertex_count,
        ) = struct.unpack_from("<IhhHHHHHBBBH", blob, pos)
        pos += 23
        cx = center[0] + qx * 0.25
        cz = center[1] + qz * 0.25
        pos += 4 * vertex_count
        out.append(
            {
                "id": building_id,
                "height": height_cm / 100.0,
                "centroid": [cx, cz],
                "category": strings[category_idx] if category_idx < len(strings) else "unknown",
                "source": strings[source_idx] if source_idx < len(strings) else "unknown",
            }
        )
    return out


def match_rule(style: dict, city: str, district: str) -> dict:
    for rule in style["rules"]:
        cities = rule.get("cities") or []
        if cities and city not in cities:
            continue
        if any(
            district == value or value in district
            for value in rule.get("neighborhood_any", [])
        ):
            return rule
    return style["default"]


def governor(record: dict, rule: dict) -> dict:
    residential = record["category"].lower() == CATEGORY_RESIDENTIAL
    floor_height = 3.0 if residential else 4.0
    floor_count = max(1, round(record["height"] / floor_height))
    if residential:
        floor_count = min(floor_count, 3)
    if rule.get("height_bias") == "low":
        floor_count = min(floor_count, 6)
    return {
        "residential": residential,
        "floor_height": floor_height,
        "floor_count": floor_count,
        "applied": floor_count * floor_height,
    }


def main() -> None:
    data = PACK.read_bytes()
    nodes = parse_nodes(data)
    districts = json.loads(DISTRICTS.read_text())["assignments"]
    style = json.loads(STYLE.read_text())
    by_name = {node["name"]: node for node in nodes}

    rows = []
    for node in nodes:
        if node["level"] != 2 or node["buildings"] == 0:
            continue
        district = districts.get(node["name"], "Seattle")
        parent = nodes[node["parent"]] if 0 <= node["parent"] < len(nodes) else None
        city_name = parent["name"] if parent and parent["level"] == 1 else "Seattle"
        rule = match_rule(style, city_name, district)
        buildings = parse_buildings(data, node)
        counts = Counter()
        sources = Counter()
        families = Counter()
        over = []
        max_recorded = 0.0
        max_applied = 0.0
        max_res_over = 0.0
        for building in buildings:
            gov = governor(building, rule)
            counts["records"] += 1
            if gov["residential"]:
                counts["residential"] += 1
            sources[building["source"]] += 1
            families[rule.get("family", "unknown")] += 1
            max_recorded = max(max_recorded, building["height"])
            max_applied = max(max_applied, gov["applied"])
            excess = building["height"] - gov["applied"]
            if excess > 0.05:
                counts["over_cap"] += 1
                if gov["residential"]:
                    counts["over_cap_residential"] += 1
                    max_res_over = max(max_res_over, excess)
                over.append(
                    {
                        "id": building["id"],
                        "height": round(building["height"], 2),
                        "applied": round(gov["applied"], 2),
                        "category": building["category"],
                        "source": building["source"],
                        "family": rule.get("family", "unknown"),
                        "district": district,
                    }
                )
        if not over and counts["records"] == 0:
            continue
        rows.append(
            {
                "neighborhood": node["name"],
                "district": district,
                "records": counts["records"],
                "residential": counts["residential"],
                "over_cap": counts["over_cap"],
                "over_cap_residential": counts["over_cap_residential"],
                "max_recorded": round(max_recorded, 2),
                "max_applied": round(max_applied, 2),
                "max_residential_excess": round(max_res_over, 2),
                "families": dict(families),
                "sources": dict(sources),
                "worst": sorted(over, key=lambda item: -item["height"])[:3],
            }
        )

    rows.sort(key=lambda row: (-row["over_cap"], row["neighborhood"]))
    totals = Counter()
    for row in rows:
        totals["neighborhoods"] += 1
        totals["records"] += row["records"]
        totals["residential"] += row["residential"]
        totals["over_cap"] += row["over_cap"]
        totals["over_cap_residential"] += row["over_cap_residential"]
    sources = Counter()
    for row in rows:
        sources.update(row["sources"])
    families = Counter()
    for row in rows:
        families.update(row["families"])

    print(
        f"CENSUS neighborhoods={totals['neighborhoods']} records={totals['records']} "
        f"residential={totals['residential']} over_cap={totals['over_cap']} "
        f"over_cap_residential={totals['over_cap_residential']}"
    )
    print(f"CENSUS sources {dict(sources)}")
    print(f"CENSUS families {dict(families)}")
    print()
    print(
        f"{'neighborhood':24} {'district':16} {'rec':>5} {'res':>5} {'over':>5} "
        f"{'over_res':>8} {'max_rec':>8} {'max_gov':>8} {'excess':>7}  sources"
    )
    for row in rows[:30]:
        print(
            f"{row['neighborhood']:24} {row['district']:16} {row['records']:5} "
            f"{row['residential']:5} {row['over_cap']:5} {row['over_cap_residential']:8} "
            f"{row['max_recorded']:8.1f} {row['max_applied']:8.1f} "
            f"{row['max_residential_excess']:7.1f}  {row['sources']}"
        )

    district_rows = defaultdict(
        lambda: {
            "records": 0,
            "residential": 0,
            "residential_over": 0,
            "commercial_over_12m": 0,
            "max_recorded": 0.0,
            "max_applied": 0.0,
            "families": Counter(),
            "sources": Counter(),
        }
    )
    for row in rows:
        target = district_rows[row["district"]]
        target["records"] += row["records"]
        target["residential"] += row["residential"]
        target["residential_over"] += row["over_cap_residential"]
        target["max_recorded"] = max(target["max_recorded"], row["max_recorded"])
        target["max_applied"] = max(target["max_applied"], row["max_applied"])
        target["families"].update(row["families"])
        target["sources"].update(row["sources"])
        for worst in row["worst"]:
            if worst["category"] != "residential" and worst["applied"] >= 12.0:
                target["commercial_over_12m"] += 1
    print()
    print("BY DISTRICT (worst three commercial records per neighborhood counted)")
    print(
        f"{'district':18} {'rec':>7} {'res':>7} {'res_over':>9} {'com>12m':>8} "
        f"{'max_rec':>8} {'max_gov':>8}  families"
    )
    for name, target in sorted(district_rows.items(), key=lambda item: -item[1]["residential_over"]):
        print(
            f"{name:18} {target['records']:7} {target['residential']:7} "
            f"{target['residential_over']:9} {target['commercial_over_12m']:8} "
            f"{target['max_recorded']:8.1f} {target['max_applied']:8.1f}  {dict(target['families'])}"
        )

    focus = {"Ballard", "Wallingford", "Fremont", "Capitol Hill", "Queen Anne", "West Seattle", "First Hill", "Pike Place", "Pioneer Square", "Downtown"}
    print()
    print("FOCUS (residential-sighting districts)")
    for row in rows:
        if row["district"] not in focus:
            continue
        if row["over_cap"] == 0:
            continue
        print(
            f"  {row['neighborhood']:22} {row['district']:14} over={row['over_cap']:4} "
            f"(res {row['over_cap_residential']:3}) max_recorded={row['max_recorded']:6.1f} "
            f"applied={row['max_applied']:5.1f} families={row['families']} worst={row['worst']}"
        )

    if "--json" in sys.argv:
        target = Path(sys.argv[sys.argv.index("--json") + 1])
        target.write_text(json.dumps({"totals": dict(totals), "rows": rows}, indent=2))
        print(f"\nwrote {target}")


if __name__ == "__main__":
    main()
