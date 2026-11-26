#!/usr/bin/env python3
"""Seattle S1.5 height enrichment.

Joins Overture Maps buildings (USGS-Lidar / Microsoft-ML heights) and Microsoft
Global ML Building Footprints (fallback) to the CityDataset via centroid
containment + footprint area tolerance, then assigns one documented
``height_source`` per building:

    real_surveyed      City of Tacoma lidar roof height / Seattle 2015 BP99 apex
    lidar_overture     Overture height (USGS Lidar or Microsoft ML Buildings)
    osm_levels_derived Overture ``num_floors`` × 3.2 m (OSM-sourced levels)
    msml_estimate      Microsoft ML footprint height for uncovered originals
    area_heuristic     footprint-area estimate (pre-existing fallback)

Priority: real_surveyed > lidar_overture > osm_levels_derived >
msml_estimate > area_heuristic.  Tacoma survey data never loses to Overture.

Run from ``viewer/`` after the S1 fetch/adapter:

    python3 scripts/seattle_enrich.py
"""

from __future__ import annotations

import gzip
import hashlib
import json
import math
import os
import time

import geopandas as gpd
import numpy as np
import pandas as pd
from pyproj import Transformer
from shapely.geometry import Point, Polygon, shape
from shapely.ops import transform

ROOT = "datasets/seattle"
RAW = f"{ROOT}/raw"
US_FOOT_M = 0.3048006096012192
FLOOR_M = 3.2
AREA_RATIO_MIN = 0.40
AREA_RATIO_MAX = 2.50
IOU_MIN = 0.35
NEAR_M = 8.0

OVERTURE = f"{RAW}/overture_buildings.parquet"
MSML_GLOB = f"{RAW}/msml_"


def log(*args: object) -> None:
    print(*args, flush=True)


def sha256(path: str) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def context():
    manifest = json.load(open(f"{ROOT}/manifest.json"))
    to_2926 = Transformer.from_crs(4326, 2926, always_xy=True)
    e0, n0 = to_2926.transform(*manifest["region"]["anchor_wgs84"])
    return manifest, (e0, n0)


def to_world_xy(origin):
    e0, n0 = origin

    def apply(x, y, z=None):
        return (x - e0) * US_FOOT_M, -(y - n0) * US_FOOT_M

    return apply


def load_dataset():
    dataset = json.load(open(f"{ROOT}/seattle_v11.json"))
    buildings = dataset["buildings"]
    log("dataset buildings", len(buildings))
    return dataset


def original_shapes(buildings):
    polygons = []
    centroids = []
    for building in buildings:
        ring = building["perimeter"]
        if len(ring) < 3:
            polygons.append(None)
            centroids.append(Point(building["centroid"]))
            continue
        polygon = Polygon(ring)
        if not polygon.is_valid:
            polygon = polygon.buffer(0)
        polygons.append(polygon)
        centroids.append(Point(building["centroid"]))
    return polygons, centroids


def load_overture(origin):
    started = time.time()
    frame = gpd.read_parquet(
        OVERTURE,
        columns=["id", "geometry", "height", "num_floors", "class", "sources"],
    )
    raw_count = len(frame)
    frame = frame[frame.geometry.notna() & ~frame.geometry.is_empty]
    frame = frame.to_crs(2926)
    frame["geometry"] = frame.geometry.apply(lambda g: transform(to_world_xy(origin), g))
    frame = frame[frame.geometry.is_valid]
    usable = frame.height.fillna(0).between(1.5, 400.0) | frame.num_floors.fillna(0).between(
        1, 100
    )
    frame = frame[usable].reset_index(drop=True)
    log(
        f"overture raw={raw_count} usable={len(frame)} in {time.time() - started:.1f}s"
    )
    return frame


def match_frames(points, originals, frame, tolerance=(AREA_RATIO_MIN, AREA_RATIO_MAX), near=NEAR_M):
    """Centroid containment + area ratio; nearest + IoU fallback."""
    n = len(points)
    areas = np.array(
        [poly.area if poly is not None else 0.0 for poly in originals], dtype=np.float64
    )
    candidate = gpd.sjoin(
        points,
        frame[["geometry", "height", "num_floors", "id"]],
        how="left",
        predicate="within",
    )
    candidate = candidate[candidate["index_right"].notna()].copy()
    accepted = []
    if len(candidate):
        target = candidate["index_right"].to_numpy(dtype=int)
        source_area = frame.geometry.area.to_numpy()[target]
        ratio = areas[candidate["bi"].to_numpy(dtype=int)] / np.maximum(source_area, 1e-6)
        score = np.abs(np.log(ratio))
        candidate["score"] = score
        keep = (ratio >= tolerance[0]) & (ratio <= tolerance[1])
        candidate = candidate[keep]
        candidate = (
            candidate.sort_values(["bi", "score"]).drop_duplicates("bi").reset_index(drop=True)
        )
        accepted.append(candidate)
    matched = (
        gpd.GeoDataFrame(pd.concat(accepted, ignore_index=True), crs=frame.crs)
        if accepted
        else gpd.GeoDataFrame(columns=["bi", "index_right"])
    )
    matched_ids = set(int(v) for v in matched["bi"]) if len(matched) else set()
    remaining = np.array([i for i in range(n) if i not in matched_ids], dtype=int)

    near_rows = []
    if len(remaining):
        subset = points.iloc[remaining].copy()
        nearest = gpd.sjoin_nearest(
            subset,
            frame[["geometry", "height", "num_floors", "id"]],
            how="left",
            max_distance=near,
            distance_col="d",
        )
        nearest = nearest[nearest["index_right"].notna()]
        keep_rows = []
        for row in nearest.itertuples():
            bi = int(row.bi)
            oid = int(row.index_right)
            polygon = originals[bi]
            other = frame.geometry.iloc[oid]
            if polygon is None or other is None or other.is_empty:
                continue
            try:
                inter = polygon.intersection(other).area
            except Exception:  # noqa: BLE001 - degenerate geometry
                continue
            union = polygon.area + other.area - inter
            iou = inter / union if union > 0 else 0.0
            if iou >= IOU_MIN:
                keep_rows.append((bi, oid, iou))
        if keep_rows:
            best = {}
            for bi, oid, iou in keep_rows:
                if bi not in best or iou > best[bi][1]:
                    best[bi] = (oid, iou)
            near_rows = [(bi, oid, iou) for bi, (oid, iou) in best.items()]

    return matched, near_rows


def assign_matches(n, matched, near_rows, frame, prefix):
    """Return per-building height/floors/source for a matched frame."""
    heights = np.full(n, np.nan)
    floors = np.full(n, np.nan)
    source = np.full(n, "", dtype=object)
    ratio_ok = 0
    if len(matched):
        ids = matched["index_right"].to_numpy(dtype=int)
        bis = matched["bi"].to_numpy(dtype=int)
        heights[bis] = frame.height.to_numpy(dtype=float)[ids]
        floors[bis] = frame.num_floors.to_numpy(dtype=float)[ids]
        source[bis] = prefix
        ratio_ok = len(bis)
    for bi, oid, _iou in near_rows:
        heights[bi] = frame.height.iloc[oid]
        floors[bi] = frame.num_floors.iloc[oid]
        source[bi] = prefix
    return heights, floors, source, ratio_ok, len(near_rows)


def load_msml(origin):
    started = time.time()
    files = sorted(
        os.path.join(RAW, name)
        for name in os.listdir(RAW)
        if name.startswith("msml_") and name.endswith(".csv.gz")
    )
    if not files:
        log("msml: no files (fallback disabled)")
        return gpd.GeoDataFrame(geometry=[], crs=2926)
    rows = []
    region = Polygon(
        json.load(open(f"{ROOT}/manifest.json"))["region"]["polygon_wgs84"]
    ).bounds
    minx, miny, maxx, maxy = region
    for path in files:
        kept = 0
        with gzip.open(path, "rt") as handle:
            for line in handle:
                try:
                    feature = json.loads(line)
                except json.JSONDecodeError:
                    continue
                geometry = feature.get("geometry")
                if not geometry:
                    continue
                coords = geometry["coordinates"]
                if geometry["type"] == "MultiPolygon":
                    coords = coords[0]
                x = coords[0][0][0]
                y = coords[0][0][1]
                if not (minx <= x <= maxx and miny <= y <= maxy):
                    continue
                try:
                    polygon = shape(geometry)
                except Exception:  # noqa: BLE001 - malformed feature
                    continue
                if polygon.is_empty:
                    continue
                rows.append(
                    (
                        float(feature["properties"].get("height") or 0.0),
                        polygon,
                    )
                )
                kept += 1
        log(f"msml {os.path.basename(path)}: kept {kept} in-region")
    frame = gpd.GeoDataFrame(
        {"height": [r[0] for r in rows], "geometry": [r[1] for r in rows]}, crs=4326
    )
    frame = frame.to_crs(2926)
    frame["geometry"] = frame.geometry.apply(lambda g: transform(to_world_xy(origin), g))
    frame = frame[frame.geometry.is_valid].reset_index(drop=True)
    frame["id"] = [f"msml-{i}" for i in range(len(frame))]
    frame["num_floors"] = np.nan
    log(f"msml usable {len(frame)} in {time.time() - started:.1f}s")
    return frame


def block_spot_check(buildings, enriched, center=(0.0, 0.0), radius=140.0):
    out = []
    for index, building in enumerate(buildings):
        dx = building["centroid"][0] - center[0]
        dz = building["centroid"][1] - center[1]
        if dx * dx + dz * dz > radius * radius:
            continue
        old = building
        new = enriched[index]
        if abs(old["height_m"] - new["height_m"]) < 0.1:
            continue
        out.append(
            {
                "id": old["id"],
                "centroid": old["centroid"],
                "height_before_m": old["height_m"],
                "height_after_m": new["height_m"],
                "source_before": old.get("height_source", "area_heuristic"),
                "source_after": new["height_source"],
            }
        )
    out.sort(key=lambda row: abs(row["height_after_m"] - row["height_before_m"]), reverse=True)
    return out[:10]


def main() -> int:
    manifest, origin = context()
    dataset = load_dataset()
    buildings = dataset["buildings"]
    n = len(buildings)
    baseline = {}
    for index, building in enumerate(buildings):
        city = (
            "seattle"
            if index < 187_053
            else "bellevue"
            if index < 187_053 + 56_856
            else "tacoma"
        )
        baseline.setdefault(city, {})
        source = building.get("height_source", "area_heuristic")
        baseline[city][source] = baseline[city].get(source, 0) + 1

    originals, centroids = original_shapes(buildings)
    points = gpd.GeoDataFrame({"bi": np.arange(n)}, geometry=centroids, crs=2926)
    overture = load_overture(origin)
    overture_usable = len(overture)
    matched, near = match_frames(points, originals, overture)
    log(f"overture matched={len(matched)} iou_fallback={len(near)}")
    ov_h, ov_f, ov_s, ov_ratio, ov_near = assign_matches(
        n, matched, near, overture, "lidar_overture"
    )
    matcher_samples = []
    if len(matched):
        for row in matched.head(20).itertuples():
            bi = int(row.bi)
            oid = int(row.index_right)
            polygon = overture.geometry.iloc[oid]
            matcher_samples.append(
                {
                    "building_id": int(buildings[bi]["id"]),
                    "original_perimeter": buildings[bi]["perimeter"],
                    "original_area_m2": round(float(originals[bi].area), 2),
                    "overture_id": str(overture.id.iloc[oid]),
                    "overture_perimeter": [
                        [round(float(x), 2), round(float(y), 2)]
                        for x, y in polygon.exterior.coords
                    ],
                    "overture_area_m2": round(float(polygon.area), 2),
                    "overture_height_m": (
                        round(float(overture.height.iloc[oid]), 2)
                        if not math.isnan(float(overture.height.iloc[oid]))
                        else None
                    ),
                    "overture_floors": (
                        int(overture.num_floors.iloc[oid])
                        if not math.isnan(float(overture.num_floors.iloc[oid]))
                        else None
                    ),
                    "area_ratio": round(
                        float(originals[bi].area) / max(float(polygon.area), 1e-6), 3
                    ),
                }
            )
    del overture, matched, near

    overture_hit = np.isfinite(ov_h) | np.isfinite(ov_f)
    remaining = np.flatnonzero(~overture_hit)
    log(f"overture coverage {overture_hit.mean():.3f}; remaining {len(remaining)}")
    msml = load_msml(origin)
    msml_h = np.full(n, np.nan)
    msml_ratio = msml_near = 0
    if len(msml) and len(remaining):
        rem_points = points.iloc[remaining].copy()
        rem_points["bi"] = np.arange(len(remaining))
        rem_originals = [originals[i] for i in remaining]
        m_matched, m_near = match_frames(
            rem_points,
            rem_originals,
            msml,
            tolerance=(0.30, 3.00),
            near=10.0,
        )
        if len(m_matched):
            ids = m_matched["index_right"].to_numpy(dtype=int)
            local = m_matched["bi"].to_numpy(dtype=int)
            msml_h[remaining[local]] = msml.height.to_numpy(dtype=float)[ids]
            msml_ratio = len(ids)
        for bi, oid, _iou in m_near:
            global_bi = remaining[bi]
            msml_h[global_bi] = msml.height.iloc[oid]
            msml_near += 1
    log(f"msml matched={msml_ratio} iou_fallback={msml_near}")

    priority = {
        "real_surveyed": 5,
        "lidar_overture": 4,
        "osm_levels_derived": 3,
        "msml_estimate": 2,
        "area_heuristic": 1,
    }
    enriched = []
    source_counts = {}
    leveled = 0
    for index, building in enumerate(buildings):
        record = dict(building)
        source = record.get("height_source", "area_heuristic")
        height = float(record["height_m"])
        if source != "real_surveyed":
            if np.isfinite(ov_h[index]) and 1.5 <= ov_h[index] <= 400.0:
                height = float(ov_h[index])
                source = "lidar_overture"
            elif np.isfinite(ov_f[index]) and 1 <= ov_f[index] <= 100:
                height = float(ov_f[index]) * FLOOR_M
                source = "osm_levels_derived"
            elif np.isfinite(msml_h[index]) and 1.5 <= msml_h[index] <= 400.0:
                height = float(msml_h[index])
                source = "msml_estimate"
        priority.setdefault(source, 1)
        record["height_m"] = round(max(1.0, height), 2)
        record["height_source"] = source
        if np.isfinite(ov_f[index]) and ov_f[index] > 0:
            record["levels"] = int(ov_f[index])
            leveled += 1
        # Overture 2026-11 releases no start_date column; keep it absent.
        source_counts[source] = source_counts.get(source, 0) + 1
        enriched.append(record)

    with open("assets/datasets/seattle_enrich_samples.json", "w") as handle:
        json.dump(
            {
                "note": "Sampled matcher pairs for scripts/test_s1b.py; world metres.",
                "samples": matcher_samples,
            },
            handle,
            indent=2,
        )

    enriched_path = f"{ROOT}/seattle_v11_enriched.json"
    dataset["buildings"] = enriched
    dataset["height_enrichment"] = {
        "generated_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "sources": [
            "Overture Maps buildings (release 2026-11-18.0)",
            "Microsoft Global ML Building Footprints quadkeys 021230021/23/30/32",
        ],
        "priority": list(priority),
        "matcher": "centroid-in-footprint + area ratio 0.40-2.50; nearest <=8 m + IoU>=0.35 fallback",
        "start_date": "not present in the Overture 2026-11 building schema; 0% coverage",
    }
    with open(enriched_path, "w") as handle:
        json.dump(dataset, handle, separators=(",", ":"))
    log("wrote", enriched_path, f"{os.path.getsize(enriched_path) / 1e6:.1f} MB")

    # Refresh the committed belt extract so the preview uses enriched heights.
    belt = json.load(open("assets/datasets/seattle_belt_v11.json"))
    center = belt["fields"]["epicenter"]
    radius = 500.0
    belt_records = [
        record
        for record in enriched
        if math.dist(record["centroid"], center) <= radius
    ]
    belt["buildings"] = belt_records
    with open("assets/datasets/seattle_belt_v11.json", "w") as handle:
        json.dump(belt, handle, separators=(",", ":"))

    after = {}
    for index, record in enumerate(enriched):
        city = (
            "seattle"
            if index < 187_053
            else "bellevue"
            if index < 187_053 + 56_856
            else "tacoma"
        )
        after.setdefault(city, {})
        after[city][record["height_source"]] = (
            after[city].get(record["height_source"], 0) + 1
        )

    report = {
        "generated_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
        "overture": {
            "release": "2026-11-18.0",
            "raw": 934201,
            "usable": int(overture_usable),
            "matched_ratio": int(ov_ratio),
            "matched_iou": int(ov_near),
            "coverage": round(float(overture_hit.mean()), 4),
        },
        "msml": {
            "matched_ratio": int(msml_ratio),
            "matched_iou": int(msml_near),
        },
        "baseline": baseline,
        "after": after,
        "height_source_counts": source_counts,
        "levels_coverage": round(leveled / n, 4),
        "start_date_coverage": 0.0,
        "spot_check_downtown": block_spot_check(buildings, enriched),
        "sizes_mb": {
            "enriched_json": round(os.path.getsize(enriched_path) / 1e6, 1),
            "overture_parquet": round(os.path.getsize(OVERTURE) / 1e6, 1),
            "msml_raw": round(
                sum(
                    os.path.getsize(os.path.join(RAW, name))
                    for name in os.listdir(RAW)
                    if name.startswith("msml_")
                )
                / 1e6,
                1,
            ),
        },
    }
    with open(f"{ROOT}/height_report.json", "w") as handle:
        json.dump(report, handle, indent=2)

    # Manifest provenance.
    manifest["counts"]["overture_buildings"] = 934201
    manifest["counts"]["msml_buildings"] = int(len(msml))
    manifest["raw"]["overture_buildings.parquet"] = {
        "bytes": os.path.getsize(OVERTURE),
        "sha256": sha256(OVERTURE),
    }
    for name in sorted(
        name for name in os.listdir(RAW) if name.startswith("msml_") and name.endswith(".gz")
    ):
        path = os.path.join(RAW, name)
        manifest["raw"][name] = {"bytes": os.path.getsize(path), "sha256": sha256(path)}
    manifest["sources"] = [
        source
        for source in manifest["sources"]
        if source["layer"] not in ("overture_buildings", "msml_buildings")
    ] + [
        {
            "layer": "overture_buildings",
            "service": "overturemaps download --type=building (Overture release 2026-11-18.0)",
            "attribution": "Overture Maps Foundation - Buildings; heights from USGS Lidar "
            "and Microsoft ML Buildings, footprints also OpenStreetMap/Esri",
            "license": "ODbL-1.0 (Overture Maps Foundation)",
            "crs": "EPSG:2926 (matched in world metres)",
        },
        {
            "layer": "msml_buildings",
            "service": "https://minedbuildings.z5.web.core.windows.net/global-buildings/2026-02-03/ "
            "(quadkeys 021230021/23/30/32)",
            "attribution": "Microsoft Global ML Building Footprints",
            "license": "CDLA-Permissive-2.0",
            "crs": "EPSG:2926",
        },
    ]
    with open(f"{ROOT}/manifest.json", "w") as handle:
        json.dump(manifest, handle, indent=2, sort_keys=True)

    log("BASELINE", json.dumps(baseline, sort_keys=True))
    log("AFTER", json.dumps(after, sort_keys=True))
    log("SOURCES", json.dumps(source_counts, sort_keys=True))
    log("LEVELS", report["levels_coverage"], "START_DATE", report["start_date_coverage"])
    log("SPOT", json.dumps(report["spot_check_downtown"], indent=1)[:1400])
    log("wrote", f"{ROOT}/height_report.json")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
