#!/usr/bin/env python3
"""Seattle district reconciliation: assign every CityPack neighborhood to one
curated district, key the style pack by district, and report the family shifts.

The CityPack names are synthetic ('Ballard 04', 'Seattle South 105'); the style
pack speaks district vocabulary ('Pioneer Square'). This script maps partitions
to districts from curated lat/lon boxes (checked in priority order), falls back
to name prefixes and city, and writes assets/styles/seattle_districts.json plus
datasets/seattle/district_report.json.

Run: python3 scripts/districts_seattle.py
"""
from __future__ import annotations

import hashlib
import json
import struct
import sys
from datetime import datetime, timezone

from pyproj import Transformer

PACK = "assets/datasets/seattle.citypack"
PACK_STYLE = "assets/styles/seattle_pack.json"
OUT = "assets/styles/seattle_districts.json"
REPORT = "datasets/seattle/district_report.json"

US_FOOT_M = 0.304800609601219
ANCHOR = (-122.3355, 47.6062)

# Priority-ordered curated districts (lat_min, lat_max, lon_min, lon_max).
BOXES = [
    ("Pioneer Square", 47.5985, 47.6045, -122.3380, -122.3270),
    ("Pike Place", 47.6075, 47.6115, -122.3460, -122.3375),
    ("Downtown", 47.6030, 47.6140, -122.3420, -122.3260),
    ("First Hill", 47.6050, 47.6135, -122.3260, -122.3120),
    ("Capitol Hill", 47.6130, 47.6330, -122.3300, -122.3080),
    ("Queen Anne", 47.6180, 47.6450, -122.3650, -122.3370),
    ("Ballard", 47.6500, 47.6750, -122.4000, -122.3650),
    ("Fremont", 47.6420, 47.6600, -122.3650, -122.3400),
    ("Wallingford", 47.6540, 47.6660, -122.3400, -122.3220),
    ("Georgetown", 47.5400, 47.5620, -122.3320, -122.3100),
    ("SODO", 47.5620, 47.5950, -122.3450, -122.3150),
    ("West Seattle", 47.5400, 47.6000, -122.4300, -122.3600),
]
PREFIXES = [
    "Pioneer Square", "Pike Place", "Downtown", "First Hill", "Capitol Hill",
    "Queen Anne", "Ballard", "Fremont", "Wallingford", "Georgetown", "SODO",
    "West Seattle", "Bellevue CBD", "Eastside", "Tacoma",
]


def pack_nodes():
    import importlib.util
    spec = importlib.util.spec_from_file_location(
        "trees_build", "scripts/trees_build.py"
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.parse_pack_nodes()


def rule_family(pack, city, needle):
    for rule in pack["rules"]:
        if rule.get("cities") and city not in rule["cities"]:
            continue
        if any(value in needle for value in rule.get("neighborhood_any", [])):
            return rule["family"]
    return pack["default"]["family"]


def main():
    t = Transformer.from_crs("EPSG:4326", "EPSG:2926", always_xy=True)
    origin = t.transform(*ANCHOR)

    def world(lon, lat):
        e, n = t.transform(lon, lat)
        return (e - origin[0]) * US_FOOT_M, -(n - origin[1]) * US_FOOT_M

    def to_lonlat(x, z):
        e = origin[0] + x / US_FOOT_M
        n = origin[1] - z / US_FOOT_M
        return Transformer.from_crs("EPSG:2926", "EPSG:4326", always_xy=True).transform(e, n)

    nodes = pack_nodes()
    cities = {}
    for i, node in enumerate(nodes):
        if node["level"] == 1:
            cities[i] = node["name"]
    assignments, district_counts = {}, {}
    city_of = {}
    for node in nodes:
        if node["level"] != 2:
            continue
        city = cities.get(node["parent"], "Seattle")
        lon, lat = to_lonlat(*node["center"])
        district = None
        for name, lat0, lat1, lon0, lon1 in BOXES:
            if lat0 <= lat <= lat1 and lon0 <= lon <= lon1:
                district = name
                break
        if district is None:
            for prefix in PREFIXES:
                if node["name"].startswith(prefix) or prefix in node["name"]:
                    district = prefix
                    break
        if district is None:
            district = "Eastside" if city == "Bellevue" else ("Tacoma" if city == "Tacoma" else "Seattle general")
        assignments[node["name"]] = district
        city_of[node["name"]] = city
        district_counts[district] = district_counts.get(district, 0) + 1

    # Manual overrides: the adaptive partition grid has no centroid inside the
    # small Pioneer Square / Pike Place boxes, so the nearest partition is
    # assigned by hand and recorded.
    overrides = {}
    used = set()
    for name, lat, lon in (("Pioneer Square", 47.6015, -122.3330), ("Pike Place", 47.6095, -122.3420)):
        if name in district_counts:
            continue
        tx, tz = world(lon, lat)
        best = min(
            ((n["name"], (n["center"][0] - tx) ** 2 + (n["center"][1] - tz) ** 2)
             for n in nodes
             if n["level"] == 2 and n["name"] not in used and assignments[n["name"]] != name),
            key=lambda pair: pair[1],
        )
        used.add(best[0])
        old = assignments[best[0]]
        assignments[best[0]] = name
        district_counts[old] -= 1
        district_counts[name] = 1
        overrides[best[0]] = name
    pack = json.load(open(PACK_STYLE))
    before, after = {}, {}
    changed = 0
    for node in nodes:
        if node["level"] != 2:
            continue
        city = city_of[node["name"]]
        old = rule_family(pack, city, node["name"])
        new = rule_family(pack, city, assignments[node["name"]])
        before[old] = before.get(old, 0) + node["buildings"]
        after[new] = after.get(new, 0) + node["buildings"]
        if old != new:
            changed += node["buildings"]

    digest = hashlib.sha256(
        json.dumps(sorted(assignments.items()), separators=(",", ":")).encode()
    ).hexdigest()
    out = {
        "schema": "seattle-districts/1",
        "generated_utc": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "method": "curated lat/lon boxes in priority order, then name prefix, then city default",
        "pack_sha256": hashlib.sha256(open(PACK, "rb").read()).hexdigest(),
        "assignments": assignments,
        "counts": district_counts,
        "manual_overrides": overrides,
        "mapping_sha256": digest,
    }
    json.dump(out, open(OUT, "w"), indent=1, sort_keys=True)
    report = {
        "schema": "seattle-district-report/1",
        "partitions": len(assignments),
        "districts": district_counts,
        "families_before_buildings": before,
        "families_after_buildings": after,
        "changed_buildings": changed,
        "mapping_sha256": digest,
    }
    json.dump(report, open(REPORT, "w"), indent=2, sort_keys=True)
    print(json.dumps(report, indent=1, sort_keys=True))
    if len(assignments) != 1096:
        print(f"ERROR: {len(assignments)} partitions mapped", file=sys.stderr)
        raise SystemExit(1)


if __name__ == "__main__":
    main()
