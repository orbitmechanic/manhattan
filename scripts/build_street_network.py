#!/usr/bin/env python3
"""Build the real Manhattan street network used by the minimap.

Source geometry is an OpenStreetMap extract (BBBike New York).  The viewer's
world coordinates are the BlenderGIS import frame: EPSG:3857 Web Mercator at
zoom 15, one world unit per mercator metre.  The scene origin stored in
``manhattan.blend`` is

    lon -74.00087591003654, lat 40.713460624392276

and the final ground mesh maps world ``(x, z)`` onto the satellite with

    X_mercator = anchor_x + x
    Y_mercator = anchor_y - z

so the conversion below is exact for the model (and was checked against the
satellite streets, the Brooklyn Bridge and the shoreline).

Usage:
    python3 scripts/build_street_network.py /tmp/opencode/osm_highways.json \
        viewer/assets/models/street_network.json
"""
from __future__ import annotations

import json
import math
import sys

EARTH_RADIUS = 6_378_137.0
ANCHOR_X = -8237739.8245614935
ANCHOR_Y = 4970168.600037185

# Model ground bounds (viewer world units) plus a margin so the minimap never
# runs out of streets near the edge of the playable model.
MIN_X, MAX_X = -2446.0, 4554.6
MIN_Z, MAX_Z = -2518.7, 2373.3
MARGIN = 900.0

DRIVABLE = {
    "motorway",
    "trunk",
    "primary",
    "secondary",
    "tertiary",
    "residential",
    "unclassified",
    "living_street",
    "pedestrian",
    "motorway_link",
    "trunk_link",
    "primary_link",
    "secondary_link",
    "tertiary_link",
}
# Service ways are only useful when they have a name (alleys, driveways and
# parking aisles otherwise clutter a 360 px map).
PEDESTRIAN = {"pedestrian"}


def lon_lat_to_world(lon: float, lat: float) -> tuple[float, float]:
    x = EARTH_RADIUS * math.radians(lon) - ANCHOR_X
    y = EARTH_RADIUS * math.log(math.tan(math.pi / 4 + math.radians(lat) / 2))
    return x, ANCHOR_Y - y


def intersects(bbox: tuple[float, float, float, float]) -> bool:
    return not (
        bbox[2] < MIN_X - MARGIN
        or bbox[0] > MAX_X + MARGIN
        or bbox[3] < MIN_Z - MARGIN
        or bbox[1] > MAX_Z + MARGIN
    )


def main() -> int:
    if len(sys.argv) != 3:
        print(__doc__)
        return 2
    source, target = sys.argv[1], sys.argv[2]
    ways = json.load(open(source))
    streets = []
    for way in ways:
        tags = way.get("tags", {})
        highway = tags.get("highway", "")
        if highway not in DRIVABLE:
            continue
        name = tags.get("name", "").strip()
        if highway == "service" and not name:
            continue
        points = way.get("points", [])
        if len(points) < 2:
            continue
        xs = [p[0] for p in points]
        ys = [p[1] for p in points]
        if not intersects((min(xs), min(ys), max(xs), max(ys))):
            continue
        flat = []
        last = None
        for lon, lat in points:
            x, z = lon_lat_to_world(lon, lat)
            pair = (round(x), round(z))
            if pair != last:
                flat.extend(pair)
                last = pair
        if len(flat) < 4:
            continue
        streets.append(
            {
                "name": name,
                "highway": highway,
                "points": flat,
            }
        )

    # Deterministic, stable output: longest/major streets first so a future
    # renderer can stop early if it ever needs to.
    rank = {
        "motorway": 0,
        "trunk": 1,
        "primary": 2,
        "secondary": 3,
        "tertiary": 4,
        "residential": 5,
        "unclassified": 6,
        "living_street": 7,
        "pedestrian": 8,
    }
    streets.sort(key=lambda s: (rank.get(s["highway"], 9), s["name"]))

    named = sum(1 for s in streets if s["name"])
    payload = {
        "version": 1,
        "source": "OpenStreetMap (BBBike New York extract)",
        "license": "ODbL 1.0",
        "units": "EPSG:3857 mercator metres (viewer world units)",
        "anchor": {"x": ANCHOR_X, "y": ANCHOR_Y},
        "street_count": len(streets),
        "named_count": named,
        "streets": streets,
    }
    with open(target, "w") as handle:
        json.dump(payload, handle, separators=(",", ":"))
    print(
        f"wrote {target}: {len(streets)} streets ({named} named), "
        f"{sum(len(s['points']) // 2 for s in streets)} vertices"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
