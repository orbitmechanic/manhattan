#!/usr/bin/env python3
"""Build 100-ft stationing data from the real OSM street centrelines.

The old stationing sets were derived from building-expansion fronts.  The
viewer's default overlay and civil-coordinate panel now use this OSM set, so
nothing in the UI still points at the guessed geometry.  Curves and vertical
profiles are flat/empty because the model ground is flat.

Input:  viewer/assets/models/street_centrelines.json
Output: viewer/assets/models/station_lines_osm.json

Usage:
    python3 scripts/build_station_osm.py
"""
from __future__ import annotations

import json
import math
import os

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SOURCE = os.path.join(ROOT, "viewer", "assets", "models", "street_centrelines.json")
TARGET = os.path.join(ROOT, "viewer", "assets", "models", "station_lines_osm.json")
FEET = 3.280839895
TICK_FT = 100.0


def point_at(points, cumulative, station_m):
    if station_m <= 0.0:
        return points[0]
    for i in range(len(points) - 1):
        if cumulative[i + 1] >= station_m:
            span = max(1e-9, cumulative[i + 1] - cumulative[i])
            t = (station_m - cumulative[i]) / span
            a, b = points[i], points[i + 1]
            return [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
    return points[-1]


def main() -> int:
    data = json.load(open(SOURCE))
    lines = []
    total_ft = 0.0
    for index, chain in enumerate(data["lines"]):
        points = [[float(p[0]), float(p[1])] for p in chain["points"]]
        if len(points) < 2:
            continue
        cumulative = [0.0]
        for a, b in zip(points, points[1:]):
            cumulative.append(cumulative[-1] + math.dist(a, b))
        length_m = cumulative[-1]
        if length_m < 3.0:
            continue
        length_ft = length_m * FEET
        ticks = []
        stations = []
        station = 0.0
        while station < length_ft - 0.01:
            stations.append(station)
            ticks.append(point_at(points, cumulative, station / FEET))
            station += TICK_FT
        stations.append(length_ft)
        ticks.append(point_at(points, cumulative, length_m))
        total_ft += length_ft
        lines.append(
            {
                "id": f"OSM_{index + 1:05d}",
                "points": points,
                "length_m": round(length_m, 2),
                "length_ft": round(length_ft, 1),
                "station_start_ft": 0.0,
                "station_end_ft": round(length_ft, 1),
                "tick_interval_ft": TICK_FT,
                "ticks_ft": [round(s, 1) for s in stations],
                "tick_points": ticks,
                "plan_curves": [],
                "plan_tangents": [],
                "vertical": {
                    "tangent_points_ft": [0.0, round(length_ft, 1)],
                    "grades": [0.0, 0.0],
                    "curves": [],
                    "start_elevation_ft": 0.0,
                    "end_elevation_ft": 0.0,
                },
            }
        )

    payload = {
        "version": 1,
        "source": "OpenStreetMap centerlines (street_centrelines.json)",
        "units": "feet",
        "tick_interval_ft": TICK_FT,
        "line_count": len(lines),
        "total_length_ft": round(total_ft, 1),
        "lines": lines,
    }
    with open(TARGET, "w") as handle:
        json.dump(payload, handle, separators=(",", ":"))
    print(
        f"wrote {TARGET}: {len(lines)} lines, "
        f"{total_ft / 5280.0:.1f} miles, {sum(len(l['ticks_ft']) for l in lines)} ticks"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
