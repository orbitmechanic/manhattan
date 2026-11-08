#!/usr/bin/env python3
"""Merge OSM street ways into continuous centrelines for the 3D roadway.

OSM splits a street into a way at every intersection and sometimes at
administrative boundaries.  The roadway builder needs long runs, so this
script chains ways that share an endpoint, the same name (or both unnamed),
the same travel direction and a shallow turn.  The geometry is unchanged:
only consecutive ways are concatenated.

Input:  viewer/assets/models/street_network.json
Output: viewer/assets/models/street_centrelines.json

Usage:
    python3 scripts/build_street_centrelines.py
"""
from __future__ import annotations

import json
import math
import os
from collections import defaultdict

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SOURCE = os.path.join(ROOT, "viewer", "assets", "models", "street_network.json")
TARGET = os.path.join(ROOT, "viewer", "assets", "models", "street_centrelines.json")

# Bridges and tunnels have no deck geometry in the city mesh; drawing asphalt
# for them would put a road on the river.  The minimap still shows them.
SKIP_BRIDGE_TUNNEL = True
MAX_TURN_DEG = 70.0

# The ground plane ends here; clipping keeps roadway meshes from floating over
# the void beyond the model.
GROUND_MIN_X, GROUND_MAX_X = -2446.0, 4554.6
GROUND_MIN_Z, GROUND_MAX_Z = -2518.7, 2373.3


def clip_segment(a, b):
    """Liang-Barsky clip of one segment to the ground rectangle."""
    x0, z0 = a
    x1, z1 = b
    dx, dz = x1 - x0, z1 - z0
    u0, u1 = 0.0, 1.0
    for p, q in (
        (-dx, x0 - GROUND_MIN_X),
        (dx, GROUND_MAX_X - x0),
        (-dz, z0 - GROUND_MIN_Z),
        (dz, GROUND_MAX_Z - z0),
    ):
        if abs(p) < 1e-12:
            if q < 0:
                return None
            continue
        r = q / p
        if p < 0:
            if r > u1:
                return None
            u0 = max(u0, r)
        else:
            if r < u0:
                return None
            u1 = min(u1, r)
    if u0 > u1:
        return None
    return (
        (x0 + dx * u0, z0 + dz * u0),
        (x0 + dx * u1, z0 + dz * u1),
    )


def clip_chain(points):
    runs = []
    current = []
    for a, b in zip(points, points[1:]):
        clipped = clip_segment(a, b)
        if clipped is None:
            if len(current) >= 2:
                runs.append(current)
            current = []
            continue
        start, end = clipped
        if not current:
            current = [start]
        current.append(end)
    if len(current) >= 2:
        runs.append(current)
    return runs


def length(points: list[tuple[int, int]]) -> float:
    return sum(math.dist(points[i], points[i + 1]) for i in range(len(points) - 1))


def direction(a: tuple[int, int], b: tuple[int, int]) -> tuple[float, float]:
    dx, dz = b[0] - a[0], b[1] - a[1]
    size = math.hypot(dx, dz)
    return (0.0, 0.0) if size < 1e-9 else (dx / size, dz / size)


def turn_angle(
    before: tuple[int, int],
    at: tuple[int, int],
    after: tuple[int, int],
) -> float:
    ax, az = direction(before, at)
    bx, bz = direction(at, after)
    dot = max(-1.0, min(1.0, ax * bx + az * bz))
    return math.degrees(math.acos(dot))


def road_group(highway: str) -> int:
    return 1 if highway == "pedestrian" else 0


class Way:
    __slots__ = ("name", "highway", "oneway", "lanes", "points", "used")

    def __init__(self, record: dict):
        self.name = record.get("name", "")
        self.highway = record.get("highway", "")
        flat = record.get("points", [])
        points = [(int(flat[i]), int(flat[i + 1])) for i in range(0, len(flat) - 1, 2)]
        cleaned = []
        for point in points:
            if not cleaned or point != cleaned[-1]:
                cleaned.append(point)
        if record.get("oneway", 0) < 0:
            cleaned.reverse()
            self.oneway = 1
        else:
            self.oneway = 1 if record.get("oneway", 0) > 0 else 0
        self.lanes = int(record.get("lanes", 0) or 0)
        self.points = cleaned
        self.used = False


def main() -> int:
    data = json.load(open(SOURCE))
    ways = []
    for record in data["streets"]:
        if SKIP_BRIDGE_TUNNEL and (record.get("bridge") or record.get("tunnel")):
            continue
        if len(record.get("points", [])) < 4:
            continue
        way = Way(record)
        if len(way.points) >= 2:
            ways.append(way)

    by_endpoint: dict[tuple[int, int], list[tuple[int, int]]] = defaultdict(list)
    for index, way in enumerate(ways):
        by_endpoint[way.points[0]].append((index, 0))
        by_endpoint[way.points[-1]].append((index, 1))

    def best_extension(
        chain: list[tuple[int, int]],
        name: str,
        highway: str,
        oneway: int,
        at_start: bool,
    ) -> tuple[int, list[tuple[int, int]]] | None:
        end = chain[0] if at_start else chain[-1]
        adjacent = chain[1] if at_start else chain[-2]
        best = None  # (turn, way_index, oriented_points)
        for other_index, endpoint in by_endpoint.get(end, ()):
            other = ways[other_index]
            if other.used:
                continue
            if other.oneway != oneway or road_group(other.highway) != road_group(highway):
                continue
            if (name or other.name) and name != other.name:
                continue

            # Orient the candidate so it joins the chain at `end`.
            if at_start:
                if endpoint == 1:
                    oriented = other.points
                elif other.oneway == 0:
                    oriented = list(reversed(other.points))
                else:
                    continue
                turn = turn_angle(oriented[-2], end, adjacent)
            else:
                if endpoint == 0:
                    oriented = other.points
                elif other.oneway == 0:
                    oriented = list(reversed(other.points))
                else:
                    continue
                turn = turn_angle(adjacent, end, oriented[1])

            if turn > MAX_TURN_DEG:
                continue
            if best is None or turn < best[0]:
                best = (turn, other_index, oriented)
        return None if best is None else (best[1], best[2])

    chains = []
    for index, way in enumerate(ways):
        if way.used:
            continue
        way.used = True
        points = list(way.points)
        name = way.name
        highway = way.highway
        oneway = way.oneway
        lanes = way.lanes

        while True:
            found = best_extension(points, name, highway, oneway, at_start=False)
            if found is None:
                break
            other_index, oriented = found
            ways[other_index].used = True
            points.extend(oriented[1:])
            lanes = max(lanes, ways[other_index].lanes)

        while True:
            found = best_extension(points, name, highway, oneway, at_start=True)
            if found is None:
                break
            other_index, oriented = found
            ways[other_index].used = True
            points = oriented + points[1:]
            lanes = max(lanes, ways[other_index].lanes)

        if len(points) >= 2:
            for run in clip_chain(points):
                run_length = length(run)
                if run_length < 3.0:
                    continue
                chains.append(
                    {
                        "name": name,
                        "highway": highway,
                        "oneway": oneway,
                        "lanes": lanes,
                        "points": run,
                        "length_m": round(run_length, 1),
                    }
                )

    chains.sort(key=lambda c: (-c["length_m"], c["name"]))

    payload = {
        "version": 1,
        "source": "OpenStreetMap (BBBike New York extract), merged by name + continuity",
        "license": "ODbL 1.0",
        "units": "EPSG:3857 mercator metres (viewer world units)",
        "line_count": len(chains),
        "lines": chains,
    }
    with open(TARGET, "w") as handle:
        json.dump(payload, handle, separators=(",", ":"))
    total = sum(c["length_m"] for c in chains)
    named = sum(1 for c in chains if c["name"])
    print(
        f"wrote {TARGET}: {len(chains)} chains ({named} named), "
        f"{total / 1000.0:.1f} km from {len(ways)} ways"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
