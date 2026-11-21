#!/usr/bin/env python3
"""Seattle S3 district style pack + completeness audit.

Rules are matched against the CityPack ancestry (`City -> Neighborhood`) and the
enriched height. The pack is the authoritative grammar; the runtime mirrors it
in `src/geology.rs` (S3 path).
"""

from __future__ import annotations

import json
import time
from collections import Counter

ROOT = "datasets/seattle"
OUT = "assets/styles/seattle_pack.json"

PACK = {
    "schema": "city-style-pack/1",
    "city_id": "seattle",
    "epicenter": {
        "name": "Downtown / Columbia Center",
        "world": [0.0, 0.0],
        "note": "replaces the Manhattan Times Square proxy for Seattle",
    },
    "rules": [
        {
            "id": "downtown-core",
            "cities": ["Seattle", "Bellevue"],
            "neighborhood_any": ["Downtown", "Bellevue CBD"],
            "family": "steel_frame",
            "facade_style": "glass_curtain_wall",
            "glass_probability": 0.82,
            "height_bias": "tower",
            "roof_recipe": "tan_parapet_equipment",
        },
        {
            "id": "pioneer-square",
            "cities": ["Seattle"],
            "neighborhood_any": ["Pioneer Square"],
            "family": "masonry_stone",
            "facade_style": "brick_brownstone",
            "glass_probability": 0.05,
            "height_bias": "low",
            "roof_recipe": "tan_parapet_curb",
        },
        {
            "id": "timber-lowrise",
            "cities": ["Seattle"],
            "neighborhood_any": ["Ballard", "Fremont", "Georgetown"],
            "family": "timber",
            "facade_style": "brick_midrise",
            "glass_probability": 0.12,
            "height_bias": "low",
            "roof_recipe": "tan_parapet_equipment",
        },
        {
            "id": "residential-hills",
            "cities": ["Seattle"],
            "neighborhood_any": ["Capitol Hill", "Queen Anne", "Beacon Hill", "West Seattle", "Rainier Valley"],
            "family": "mixed",
            "facade_style": "brick_brownstone",
            "glass_probability": 0.08,
            "height_bias": "mid",
            "roof_recipe": "tan_parapet_curb",
        },
        {
            "id": "sodo-industrial",
            "cities": ["Seattle"],
            "neighborhood_any": ["SODO", "South King"],
            "family": "steel_frame",
            "facade_style": "concrete_grid",
            "glass_probability": 0.18,
            "height_bias": "industrial",
            "roof_recipe": "flat_equipment",
        },
        {
            "id": "tacoma-core",
            "cities": ["Tacoma"],
            "neighborhood_any": ["Tacoma Core", "Tacoma North", "Tacoma South"],
            "family": "masonry_stone",
            "facade_style": "concrete_grid",
            "glass_probability": 0.20,
            "height_bias": "mid",
            "roof_recipe": "tan_parapet_curb",
        },
        {
            "id": "eastside",
            "cities": ["Bellevue"],
            "neighborhood_any": ["Eastside"],
            "family": "mixed",
            "facade_style": "brick_midrise",
            "glass_probability": 0.15,
            "height_bias": "mid",
            "roof_recipe": "tan_parapet_equipment",
        },
    ],
    "default": {
        "id": "generic-seattle",
        "family": "masonry_stone",
        "facade_style": "brick_midrise",
        "glass_probability": 0.10,
        "height_bias": "mid",
        "roof_recipe": "tan_parapet_curb",
    },
}


def main() -> int:
    partitions = json.load(open(f"{ROOT}/partitions.json"))["nodes"]
    neighborhoods = [node for node in partitions if node["level"] == 2]
    parent = {index: node for index, node in enumerate(partitions)}
    cities = Counter()
    matched = Counter()
    unmatched = []
    for node in neighborhoods:
        city = parent[node["parent"]]["name"]
        cities[city] += 1
        hit = None
        for rule in PACK["rules"]:
            if city not in rule["cities"]:
                continue
            if any(key in node["name"] for key in rule["neighborhood_any"]):
                hit = rule["id"]
                break
        if hit:
            matched[hit] += 1
        else:
            unmatched.append(f"{city} -> {node['name']}")
            matched["default"] += 1
    report = {
        "generated_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "neighborhoods": len(neighborhoods),
        "cities": dict(cities),
        "rule_counts": dict(matched),
        "unmatched": unmatched,
        "completeness": round(sum(matched.values()) / len(neighborhoods), 6),
        "epicenter": PACK["epicenter"],
        "families": sorted({rule["family"] for rule in PACK["rules"]} | {PACK["default"]["family"]}),
    }
    PACK["audit"] = report
    with open(OUT, "w") as handle:
        json.dump(PACK, handle, indent=2)
    with open(f"{ROOT}/style_pack_report.json", "w") as handle:
        json.dump(report, handle, indent=2)
    print(json.dumps(report, indent=1))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
