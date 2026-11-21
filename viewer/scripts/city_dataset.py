#!/usr/bin/env python3
"""CityDataset v1.1 adapters. Manhattan is data, not code assumptions.

Adapters:
    manhattan   legacy map records -> assets/datasets/manhattan_v11.json
    synthetic   invented ten-building test city
    seattle     Seattle S1: GeoData/3DEP raw -> full regional dataset, water,
                terrain grids, report, and a pilot-belt extract

Run from ``viewer/``:
    python3 scripts/city_dataset.py seattle
"""

import json
import math
import os
import sys

TS = [1711.638, -2000.0]
US_FOOT_M = 0.3048006096012192
SEATTLE_ROOT = "datasets/seattle"
SEATTLE_RAW = f"{SEATTLE_ROOT}/raw"


def manhattan():
    src = json.load(open("assets/models/building_perimeters.json"))
    buildings = []
    for b in src["buildings"]:
        cx, cz = b["centroid"]
        buildings.append(
            {
                "id": b["id"],
                "block": b["block"],
                "centroid": [cx, cz],
                "perimeter": b["loop"],
                "height_m": b["height_m"],
                "floor_height_m": b["floor_height_m"],
                "category": b["category"],
                "year_built": b.get("year_built", 0),
                "base_elevation_m": 0.0,
            }
        )
    dataset = {
        "version": "1.1",
        "city_id": "manhattan",
        "attribution": "manhattan_floors.blend; construction years estimated (see scripts/bake_year_built.py)",
        "license": "project data",
        "fields": {
            "epicenter": TS,
            "ad_radius_m": 2500.0,
            "ad_full_m": 250.0,
            "glass_radius_m": 900.0,
            "times_square_proxy_note": "dataset stops near 40.731N; epicenter is the nearest mapped point",
        },
        "terrain": None,
        "water_bodies": None,
        "buildings": buildings,
    }
    json.dump(dataset, open("assets/datasets/manhattan_v11.json", "w"), separators=(",", ":"))
    print("manhattan records", len(buildings))


def synthetic():
    # invented ten-building city; no Manhattan coupling
    buildings = []
    for i in range(10):
        x = i * 40.0
        z = (i % 3) * 30.0
        w = 10 + (i % 4) * 2
        d = 8 + (i % 3) * 2
        buildings.append(
            {
                "id": i,
                "block": i // 5,
                "centroid": [x, z],
                "perimeter": [
                    [x - w / 2, z - d / 2],
                    [x + w / 2, z - d / 2],
                    [x + w / 2, z + d / 2],
                    [x - w / 2, z + d / 2],
                ],
                "height_m": 12 + (i % 5) * 9,
                "floor_height_m": 3.2,
                "category": "residential" if i % 2 else "industrial",
                "year_built": 1900 + i * 11,
                "base_elevation_m": 0.0,
            }
        )
    json.dump(
        {
            "version": "1.1",
            "city_id": "synthetic",
            "attribution": "hand-written test",
            "license": "test",
            "fields": {"epicenter": [150.0, 30.0]},
            "terrain": None,
            "water_bodies": None,
            "buildings": buildings,
        },
        open("assets/datasets/synthetic_v11.json", "w"),
        separators=(",", ":"),
    )
    print("synthetic records", len(buildings))


# --------------------------------------------------------------------------
# Seattle S1 adapter
# --------------------------------------------------------------------------


def _dem_sampler(paths):
    import numpy as np
    import rasterio

    datasets = [rasterio.open(path) for path in paths]
    arrays = [dataset.read(1) for dataset in datasets]

    def sample(lon, lat):
        values = np.full(len(lon), np.nan, dtype=np.float32)
        for dataset, array in zip(datasets, arrays):
            cols, rows = (~dataset.transform) * (lon, lat)
            rows = np.round(rows).astype(int)
            cols = np.round(cols).astype(int)
            inside = (
                (rows >= 0)
                & (rows < array.shape[0])
                & (cols >= 0)
                & (cols < array.shape[1])
                & np.isnan(values)
            )
            values[inside] = array[rows[inside], cols[inside]]
        values[values < -1000.0] = np.nan
        return values

    return sample


def _world_xy(coords, origin_2926):
    """EPSG:2926 feet -> local world metres (x east, z south)."""
    ex, ny = origin_2926
    xs = (coords[:, 0] - ex) * US_FOOT_M
    zs = -(coords[:, 1] - ny) * US_FOOT_M
    return xs, zs


def _largest_polygon(geometry):
    if geometry.geom_type == "Polygon":
        return geometry
    if geometry.geom_type == "MultiPolygon":
        return max(geometry.geoms, key=lambda part: part.area)
    return None


def _ring_world(geometry, origin):
    import numpy as np

    geometry = _largest_polygon(geometry)
    if geometry is None:
        return []
    ring = np.asarray(geometry.exterior.coords, dtype=np.float64)
    xs, zs = _world_xy(ring, origin)
    return [[round(float(x), 2), round(float(z), 2)] for x, z in zip(xs, zs)][:-1]


def _height_from_area(area_m2, lo=6.0, hi=30.0):
    return max(lo, min(hi, 4.5 + area_m2 / 380.0))


def _category_from_zoning(desc):
    text = (desc if isinstance(desc, str) else "").lower()
    if "industrial" in text:
        return "industrial"
    if "single" in text or "multi" in text or "residential" in text:
        return "residential"
    if "commercial" in text or "downtown" in text or "mixed" in text:
        return "commercial"
    if "institution" in text:
        return "commercial"
    return "residential"


def _context():
    import geopandas as gpd
    from pyproj import Transformer
    from shapely.geometry import Polygon

    manifest = json.load(open(f"{SEATTLE_ROOT}/manifest.json"))
    polygon = Polygon(manifest["region"]["polygon_wgs84"])
    to_2926 = Transformer.from_crs("EPSG:4326", "EPSG:2926", always_xy=True)
    origin = to_2926.transform(*manifest["region"]["anchor_wgs84"])
    region_2926 = gpd.GeoSeries([polygon], crs="EPSG:4326").to_crs("EPSG:2926").iloc[0]
    to_lonlat = Transformer.from_crs("EPSG:2926", "EPSG:4326", always_xy=True)
    return manifest, polygon, origin, region_2926, to_lonlat


def _load_frame(name):
    import geopandas as gpd

    frame = gpd.read_file(f"{SEATTLE_RAW}/{name}.geojson")
    return frame.set_crs("EPSG:2926", allow_override=True)


def _clean(frame, region_2926):
    frame = frame[frame.geometry.notna() & ~frame.geometry.is_empty]
    frame = frame[frame.geometry.is_valid]
    return frame[frame.intersects(region_2926)]


def _sample_base(frame, dem, to_lonlat):
    import numpy as np

    centroids = frame.geometry.centroid
    lon, lat = to_lonlat.transform(centroids.x.to_numpy(), centroids.y.to_numpy())
    return centroids, dem(lon, lat)


def _seattle_layer(dem, to_lonlat, region_2926, zones):
    import numpy as np
    import geopandas as gpd

    report = {}
    raw = _load_frame("seattle_buildings_2023")
    report["raw_features"] = len(raw)
    valid = raw[raw.geometry.notna() & ~raw.geometry.is_empty]
    valid = valid[valid.geometry.is_valid]
    report["valid_geometry"] = len(valid)
    inside = valid[valid.intersects(region_2926)]
    report["inside_region"] = len(inside)
    frame = inside[inside["Shape__Area"] >= 300.0]
    report["area_ge_300_sqft"] = len(frame)
    report["rejected"] = {
        "invalid_or_empty_geometry": report["raw_features"] - report["valid_geometry"],
        "outside_region": report["valid_geometry"] - report["inside_region"],
        "below_300_sqft": report["inside_region"] - report["area_ge_300_sqft"],
        "total": report["raw_features"] - len(frame),
    }

    centroids, base = _sample_base(frame, dem, to_lonlat)
    report["base_elevation_coverage"] = round(float(np.isfinite(base).mean()), 4)

    points = _load_frame("seattle_buildings_2015")
    points = points[points["BP99_APEX"].notna()]
    left = gpd.GeoDataFrame({"idx": np.arange(len(frame))}, geometry=centroids, crs=frame.crs)
    right = points[["BP99_APEX", "geometry"]].reset_index(names="r15")
    joined = gpd.sjoin_nearest(left, right, how="left", max_distance=25.0, distance_col="d")
    joined = joined.sort_values(["idx", "d"]).drop_duplicates("idx")
    apex = np.full(len(frame), np.nan)
    apex[joined["idx"].to_numpy()] = joined["BP99_APEX"].to_numpy(dtype=float)
    report["roof_apex_join_25ft"] = round(float(np.isfinite(apex).mean()), 4)

    height = apex * US_FOOT_M - base
    real = np.isfinite(height) & (height > 2.5) & (height < 400.0)
    report["real_height_coverage"] = round(float(real.mean()), 4)
    height = np.where(real, height, np.nan)

    unmatched = ~real
    fill = np.zeros(len(frame), dtype=bool)
    if unmatched.any():
        left = gpd.GeoDataFrame(
            {"idx": np.where(unmatched)[0]}, geometry=centroids[unmatched], crs=frame.crs
        )
        joined = gpd.sjoin_nearest(left, right, how="left", max_distance=100.0, distance_col="d")
        joined = joined.sort_values(["idx", "d"]).drop_duplicates("idx")
        neighbour = np.full(len(frame), np.nan)
        neighbour[joined["idx"].to_numpy()] = joined["BP99_APEX"].to_numpy(dtype=float)
        inferred = neighbour * US_FOOT_M - base
        fill = unmatched & np.isfinite(inferred) & (inferred > 2.5) & (inferred < 400.0)
        height[fill] = inferred[fill]
    report["neighbour_inferred_coverage"] = round(float(fill.mean()), 4)

    area_m2 = frame["Shape__Area"].to_numpy() * US_FOOT_M * US_FOOT_M
    estimate = ~np.isfinite(height)
    height[estimate] = [_height_from_area(a) for a in area_m2[estimate]]
    report["area_estimated_coverage"] = round(float(estimate.mean()), 4)
    report["height_note"] = (
        "Building Outlines 2023 has no height/elevation attributes. Roof apex comes "
        "from Building Outline 2015 (BP99_APEX) within 25 ft; neighbours within 100 ft "
        "fill some gaps; the remainder uses a footprint-area estimate."
    )

    category = np.array(["residential"] * len(frame), dtype=object)
    if zones is not None and len(zones):
        zleft = gpd.GeoDataFrame(
            {"idx": np.arange(len(frame))}, geometry=centroids, crs=frame.crs
        )
        zjoin = gpd.sjoin(
            zleft,
            zones[["CATEGORY_DESC", "ZONING", "geometry"]],
            how="left",
            predicate="within",
        )
        zjoin = zjoin.drop_duplicates("idx")
        for row in zjoin.itertuples():
            category[int(row.idx)] = _category_from_zoning(row.CATEGORY_DESC)
    report["category_counts"] = {}
    for value in category:
        report["category_counts"][value] = report["category_counts"].get(value, 0) + 1
    report["height_p05_p50_p95_m"] = [
        round(float(v), 1) for v in np.nanpercentile(height, [5, 50, 95])
    ]
    report["base_p05_p50_p95_m"] = [
        round(float(v), 1)
        for v in np.nanpercentile(base[np.isfinite(base)], [5, 50, 95])
    ]

    buildings = _buildings_from_frame(frame, base, height, category)
    return buildings, report


def _bellevue_layer(dem, to_lonlat, region_2926):
    import numpy as np

    report = {}
    raw = _load_frame("bellevue_buildings")
    report["raw_features"] = len(raw)
    valid = raw[raw.geometry.notna() & ~raw.geometry.is_empty]
    valid = valid[valid.geometry.is_valid]
    inside = valid[valid.intersects(region_2926)]
    report["inside_region"] = len(inside)
    active = inside["LifeCycleStatus"].isin(["Active", "Pending", "Proposed"])
    usable = inside["StructureType"].isin(["Occupiable", "Not Occupiable", "Not Verified"])
    frame = inside[active & usable]
    report["active_usable"] = len(frame)
    report["rejected"] = {
        "invalid_or_empty_geometry": report["raw_features"] - len(valid),
        "outside_region": len(valid) - len(inside),
        "inactive_or_nonbuilding": len(inside) - len(frame),
        "total": report["raw_features"] - len(frame),
    }
    _centroids, base = _sample_base(frame, dem, to_lonlat)
    area_m2 = frame.geometry.area.to_numpy() * US_FOOT_M * US_FOOT_M
    height = np.array([_height_from_area(a, lo=6.0, hi=36.0) for a in area_m2])
    report["real_height_coverage"] = 0.0
    report["base_elevation_coverage"] = round(float(np.isfinite(base).mean()), 4)
    report["height_note"] = (
        "Bellevue Buildings has no roof elevation/height attribute; all heights are "
        "footprint-area estimates and are never counted as real."
    )
    report["category_counts"] = {"residential": len(frame)}
    report["height_p05_p50_p95_m"] = [
        round(float(v), 1) for v in np.nanpercentile(height, [5, 50, 95])
    ]
    report["base_p05_p50_p95_m"] = [
        round(float(v), 1)
        for v in np.nanpercentile(base[np.isfinite(base)], [5, 50, 95])
    ]
    category = np.array(["residential"] * len(frame), dtype=object)
    return _buildings_from_frame(frame, base, height, category), report


def _tacoma_layer(dem, to_lonlat, region_2926):
    import numpy as np

    report = {}
    raw = _load_frame("tacoma_buildings_2024")
    report["raw_features"] = len(raw)
    valid = raw[raw.geometry.notna() & ~raw.geometry.is_empty]
    valid = valid[valid.geometry.is_valid]
    inside = valid[valid.intersects(region_2926)]
    report["inside_region"] = len(inside)
    frame = inside[inside["area_sqft"].fillna(0) >= 300.0]
    report["area_ge_300_sqft"] = len(frame)
    report["rejected"] = {
        "invalid_or_empty_geometry": report["raw_features"] - len(valid),
        "outside_region": len(valid) - len(inside),
        "below_300_sqft": len(inside) - len(frame),
        "total": report["raw_features"] - len(frame),
    }
    _centroids, base = _sample_base(frame, dem, to_lonlat)
    raw_height = frame["average_z"].to_numpy(dtype=float)
    height = raw_height.copy()
    plausible = np.isfinite(height) & (height > 2.5) & (height < 200.0)
    report["real_height_coverage"] = round(float(plausible.mean()), 4)
    area_m2 = frame["area_sqft"].to_numpy(dtype=float) * US_FOOT_M * US_FOOT_M
    height[~plausible] = [
        _height_from_area(a, lo=6.0, hi=40.0) for a in area_m2[~plausible]
    ]
    report["height_note"] = (
        "Tacoma 2024 average_z is roof height above lidar ground (metres): lidar "
        "'lag' matches the 3DEP base within ~1 m."
    )
    report["base_elevation_coverage"] = round(float(np.isfinite(base).mean()), 4)
    report["category_counts"] = {"residential": len(frame)}
    report["height_p05_p50_p95_m"] = [
        round(float(v), 1) for v in np.nanpercentile(height, [5, 50, 95])
    ]
    report["base_p05_p50_p95_m"] = [
        round(float(v), 1)
        for v in np.nanpercentile(base[np.isfinite(base)], [5, 50, 95])
    ]
    category = np.array(["residential"] * len(frame), dtype=object)
    return _buildings_from_frame(frame, base, height, category), report


def _buildings_from_frame(frame, base, height, category):
    import numpy as np

    _manifest, _polygon, origin, _region, _to_lonlat = _context()
    buildings = []
    simple = frame.geometry.simplify(1.0)  # feet; ~0.3 m
    for i, geometry in enumerate(simple):
        ring = _ring_world(geometry, origin)
        if len(ring) < 3:
            continue
        centroid = geometry.centroid
        cx, cz = _world_xy(np.array([[centroid.x, centroid.y]], dtype=np.float64), origin)
        buildings.append(
            {
                "id": 0,
                "block": 0,
                "centroid": [round(float(cx[0]), 2), round(float(cz[0]), 2)],
                "perimeter": ring,
                "height_m": round(float(height[i]), 2),
                "floor_height_m": 3.4,
                "category": str(category[i]),
                "year_built": 0,
                "base_elevation_m": round(
                    float(base[i]) if np.isfinite(base[i]) else 0.0, 2
                ),
            }
        )
    return buildings


def _water_bodies(origin, dem, to_lonlat):
    import geopandas as gpd
    import numpy as np

    report = {}
    frame = _load_frame("water_areas")
    frame = frame[frame.geometry.notna() & frame.geometry.is_valid]
    frame = frame[frame.geometry.area * US_FOOT_M * US_FOOT_M >= 10000.0]
    report["polygons_ge_1ha"] = len(frame)
    report["total_area_km2"] = round(
        float(frame.geometry.area.sum() * US_FOOT_M * US_FOOT_M) / 1.0e6, 2
    )
    kinds = {"H2030": "lake", "H2051": "sound", "H3010": "stream", "H3020": "canal"}
    bodies = []
    for row in frame.itertuples():
        geometry = _largest_polygon(row.geometry.simplify(5.0))
        if geometry is None or geometry.is_empty:
            continue
        point = geometry.representative_point()
        lon, lat = to_lonlat.transform(point.x, point.y)
        surface = float(dem(np.array([lon]), np.array([lat]))[0])
        if not np.isfinite(surface):
            surface = 0.0
        ring = _ring_world(geometry, origin)
        if len(ring) < 3:
            continue
        bodies.append(
            {
                "kind": kinds.get(row.MTFCC, "water"),
                "name": (
                    row.FULLNAME.strip()
                    if isinstance(row.FULLNAME, str) and row.FULLNAME.strip()
                    else None
                ),
                "mtfcc": row.MTFCC,
                "datum": "NAVD88 (approx., 3DEP-derived)",
                "surface_elevation_m": round(surface, 2),
                "polygon": ring,
            }
        )
    report["bodies"] = len(bodies)
    report["named"] = sum(1 for body in bodies if body["name"])
    report["kinds"] = {}
    for body in bodies:
        report["kinds"][body["kind"]] = report["kinds"].get(body["kind"], 0) + 1
    return bodies, report


def _terrain_grids(world_bounds, origin, belt_center):
    """Reproject 3DEP to EPSG:2855: 30 m regional grid + 10 m belt window."""
    import numpy as np
    import rasterio
    from affine import Affine
    from rasterio.warp import Resampling, reproject

    tiles = [f"{SEATTLE_RAW}/USGS_13_n48w123.tif", f"{SEATTLE_RAW}/USGS_13_n48w122.tif"]
    datasets = [rasterio.open(tile) for tile in tiles]
    ex_m = origin[0] * US_FOOT_M
    ny_m = origin[1] * US_FOOT_M
    xmin, xmax, zmin, zmax = world_bounds
    report = {}

    def window(spacing, half):
        cols = int(math.ceil(2 * half / spacing))
        transform = Affine(
            spacing,
            0.0,
            belt_center[0] + ex_m - half,
            0.0,
            -spacing,
            -belt_center[1] + ny_m + half,
        )
        return np.full((cols, cols), np.nan, dtype=np.float32), transform, cols

    def fill(destination, transform):
        for dataset in datasets:
            tile = np.full(destination.shape, np.nan, dtype=np.float32)
            reproject(
                source=rasterio.band(dataset, 1),
                destination=tile,
                src_transform=dataset.transform,
                src_crs=dataset.crs,
                dst_transform=transform,
                dst_crs="EPSG:2855",
                resampling=Resampling.bilinear,
                src_nodata=-999999.0,
                dst_nodata=np.nan,
            )
            np.copyto(destination, tile, where=np.isfinite(tile))

    pad = 200.0
    cols = int(math.ceil((xmax - xmin + 2 * pad) / 30.0))
    rows = int(math.ceil((zmax - zmin + 2 * pad) / 30.0))
    transform = Affine(30.0, 0.0, xmin + ex_m - pad, 0.0, -30.0, -zmin + ny_m + pad)
    destination = np.full((rows, cols), np.nan, dtype=np.float32)
    fill(destination, transform)
    path = f"{SEATTLE_ROOT}/terrain_30m.tif"
    with rasterio.open(
        path,
        "w",
        driver="GTiff",
        height=rows,
        width=cols,
        count=1,
        dtype="float32",
        crs="EPSG:2855",
        transform=transform,
        nodata=np.nan,
        compress="deflate",
    ) as out:
        out.write(destination, 1)
    valid = np.isfinite(destination)
    report["coverage_pct"] = round(float(valid.mean() * 100.0), 2)
    report["grid"] = {"rows": rows, "cols": cols, "spacing_m": 30.0}
    report["elevation_min_max_m"] = [
        round(float(np.nanmin(destination)), 1),
        round(float(np.nanmax(destination)), 1),
    ]

    belt, belt_transform, belt_cols = window(10.0, 600.0)
    fill(belt, belt_transform)
    belt_path = f"{SEATTLE_ROOT}/terrain_belt_10m.tif"
    with rasterio.open(
        belt_path,
        "w",
        driver="GTiff",
        height=belt_cols,
        width=belt_cols,
        count=1,
        dtype="float32",
        crs="EPSG:2855",
        transform=belt_transform,
        nodata=np.nan,
        compress="deflate",
    ) as out:
        out.write(belt, 1)
    report["belt_relief_m"] = round(float(np.nanmax(belt) - np.nanmin(belt)), 1)
    report["belt_grid"] = {"rows": belt_cols, "cols": belt_cols, "spacing_m": 10.0}

    manifest = json.load(open(f"{SEATTLE_ROOT}/manifest.json"))
    terrain = {
        "source": "USGS 3DEP 1/3 arc-second (n48w123, n48w122)",
        "vertical_datum": "NAVD88 (approx.)",
        "crs": "EPSG:2855",
        "spacing_m": 30.0,
        "rows": rows,
        "cols": cols,
        "world_origin_m": [
            round(xmin - pad, 2),
            round(zmin - pad, 2),
        ],
        "path": path,
        "belt": {
            "path": belt_path,
            "spacing_m": 10.0,
            "center_world": [round(belt_center[0], 2), round(belt_center[1], 2)],
            "half_extent_m": 600.0,
            "rows": belt_cols,
            "cols": belt_cols,
            "relief_m": report["belt_relief_m"],
        },
        "region_polygon_wgs84": manifest["region"]["polygon_wgs84"],
    }
    return terrain, report


def _pick_belt(buildings):
    """Pick a 140-building, high-relief belt (Capitol Hill / Queen Anne style)."""
    import numpy as np

    usable = [b for b in buildings if b["base_elevation_m"] > 0.0]
    points = np.array([b["centroid"] for b in usable], dtype=np.float64)
    bases = np.array([b["base_elevation_m"] for b in usable], dtype=np.float64)
    heights = np.array([b["height_m"] for b in usable], dtype=np.float64)
    if len(points) < 200:
        raise RuntimeError("not enough buildings for a pilot belt")
    stride = max(1, len(points) // 320)
    best = None
    for index in range(0, len(points), stride):
        center = points[index]
        distance = np.sum((points - center) ** 2, axis=1)
        within = np.flatnonzero(distance <= 420.0 * 420.0)
        if len(within) < 140:
            continue
        nearest = np.argpartition(distance, 140)[:140]
        if np.median(heights[nearest]) < 6.0:
            continue
        # The generator keeps the 140 nearest, so score the slope they span,
        # not the whole 420 m disc: a readable hillside, not a cliff.
        relief = np.percentile(bases[nearest], 95) - np.percentile(bases[nearest], 5)
        if not (18.0 <= relief <= 60.0):
            continue
        score = len(within) + relief
        if best is None or score > best[0]:
            best = (float(score), center, nearest, relief)
    if best is None:
        raise RuntimeError("belt search failed")
    _score, center, nearest, relief = best
    return [float(center[0]), float(center[1])], {
        "center_world": [round(float(center[0]), 1), round(float(center[1]), 1)],
        "relief_p95_p5_m": round(relief, 1),
        "buildings_within_420m": int(len(within)),
        "selected_140_relief_m": round(float(relief), 1),
        "median_height_m": round(float(np.median(heights[nearest])), 1),
        "min_base_m": round(float(np.min(bases[nearest])), 1),
        "max_base_m": round(float(np.max(bases[nearest])), 1),
        "selection": "densest candidate whose 140 nearest buildings span 18-60 m p95-p5 relief",
    }


def seattle():
    import geopandas as gpd
    import numpy as np

    manifest, polygon, origin, region_2926, to_lonlat = _context()
    dem = _dem_sampler(
        [f"{SEATTLE_RAW}/USGS_13_n48w123.tif", f"{SEATTLE_RAW}/USGS_13_n48w122.tif"]
    )
    zones = _load_frame("seattle_zoning")
    zones = zones[zones.geometry.notna() & zones.geometry.is_valid]

    layers = {}
    buildings = []
    seattle_buildings, layers["seattle_buildings_2023"] = _seattle_layer(
        dem, to_lonlat, region_2926, zones
    )
    buildings.extend(seattle_buildings)
    bellevue, layers["bellevue_buildings"] = _bellevue_layer(dem, to_lonlat, region_2926)
    buildings.extend(bellevue)
    tacoma, layers["tacoma_buildings_2024"] = _tacoma_layer(dem, to_lonlat, region_2926)
    buildings.extend(tacoma)
    for index, building in enumerate(buildings):
        building["id"] = index
        building["block"] = index // 24

    water, water_report = _water_bodies(origin, dem, to_lonlat)
    belt_center, belt_report = _pick_belt(buildings)

    region_2926_bounds = region_2926.bounds
    world_bounds = [
        (region_2926_bounds[0] - origin[0]) * US_FOOT_M,
        (region_2926_bounds[2] - origin[0]) * US_FOOT_M,
        -(region_2926_bounds[3] - origin[1]) * US_FOOT_M,
        -(region_2926_bounds[1] - origin[1]) * US_FOOT_M,
    ]
    terrain, terrain_report = _terrain_grids(world_bounds, origin, belt_center)

    def world_point(lon, lat):
        from pyproj import Transformer

        ex, ny = Transformer.from_crs("EPSG:4326", "EPSG:2926", always_xy=True).transform(
            lon, lat
        )
        return np.array([(ex - origin[0]) * US_FOOT_M, -(ny - origin[1]) * US_FOOT_M])

    anchor = np.array([0.0, 0.0])
    bellevue_anchor = world_point(-122.2015, 47.6101)
    tacoma_anchor = world_point(-122.4443, 47.2529)
    zone_counts = {
        "downtown_core_le_2km": 0,
        "bellevue_le_3km": 0,
        "tacoma_le_4km": 0,
        "south_corridor": 0,
        "rest_of_region": 0,
    }
    for building in buildings:
        point = np.array(building["centroid"])
        if np.linalg.norm(point - anchor) <= 2000.0:
            zone_counts["downtown_core_le_2km"] += 1
        elif np.linalg.norm(point - bellevue_anchor) <= 3000.0:
            zone_counts["bellevue_le_3km"] += 1
        elif np.linalg.norm(point - tacoma_anchor) <= 4000.0:
            zone_counts["tacoma_le_4km"] += 1
        elif point[1] > 12000.0:
            zone_counts["south_corridor"] += 1
        else:
            zone_counts["rest_of_region"] += 1

    attribution = "; ".join(
        sorted({source["attribution"] for source in manifest["sources"]})
    )
    licenses = "; ".join(sorted({source["license"] for source in manifest["sources"]}))
    dataset = {
        "version": "1.1",
        "city_id": "seattle",
        "attribution": attribution,
        "license": licenses,
        "region": {
            "name": manifest["region"]["name"],
            "anchor_wgs84": manifest["region"]["anchor_wgs84"],
            "anchor_epsg2926": [round(origin[0], 2), round(origin[1], 2)],
            "polygon_wgs84": [list(point) for point in polygon.exterior.coords],
            "world_frame": "x=east metres, z=south metres, y=base_elevation_m",
            "charter": manifest["region"]["charter"],
        },
        "fields": {
            "epicenter": [0.0, 0.0],
            "ad_radius_m": 2500.0,
            "ad_full_m": 250.0,
            "glass_radius_m": 900.0,
        },
        "provenance": [
            {key: source[key] for key in ("layer", "service", "attribution", "license")}
            for source in manifest["sources"]
        ],
        "terrain": terrain,
        "water_bodies": water,
        "buildings": buildings,
    }
    os.makedirs(SEATTLE_ROOT, exist_ok=True)
    full_path = f"{SEATTLE_ROOT}/seattle_v11.json"
    with open(full_path, "w") as handle:
        json.dump(dataset, handle, separators=(",", ":"))
    region_path = f"{SEATTLE_ROOT}/region.geojson"
    gpd.GeoSeries([polygon], crs="EPSG:4326").to_file(region_path, driver="GeoJSON")

    belt_radius = 430.0
    belt_buildings = [
        building
        for building in buildings
        if math.dist(building["centroid"], belt_center) <= belt_radius
    ]
    belt_dataset = {
        "version": "1.1",
        "city_id": "seattle",
        "attribution": attribution,
        "license": licenses,
        "region": dataset["region"],
        "fields": {
            "epicenter": [round(belt_center[0], 2), round(belt_center[1], 2)],
            "ad_radius_m": 800.0,
            "ad_full_m": 120.0,
            "glass_radius_m": 350.0,
        },
        "provenance": dataset["provenance"],
        "terrain": terrain,
        "water_bodies": None,
        "buildings": belt_buildings,
    }
    belt_path = "assets/datasets/seattle_belt_v11.json"
    os.makedirs("assets/datasets", exist_ok=True)
    with open(belt_path, "w") as handle:
        json.dump(belt_dataset, handle, separators=(",", ":"))

    report = {
        "generated_utc": manifest.get("generated_utc"),
        "layers": layers,
        "water": water_report,
        "terrain": terrain_report,
        "belt": belt_report,
        "zones": zone_counts,
        "buildings_total": len(buildings),
        "belt_buildings": len(belt_buildings),
        "sizes_mb": {
            "seattle_v11.json": round(os.path.getsize(full_path) / 1e6, 1),
            "seattle_belt_v11.json": round(os.path.getsize(belt_path) / 1e6, 1),
            "terrain_30m.tif": round(os.path.getsize(terrain["path"]) / 1e6, 1),
            "terrain_belt_10m.tif": round(os.path.getsize(terrain["belt"]["path"]) / 1e6, 1),
            "raw_total": round(
                sum(
                    os.path.getsize(os.path.join(SEATTLE_RAW, name))
                    for name in os.listdir(SEATTLE_RAW)
                )
                / 1e6,
                1,
            ),
        },
        "contract": {
            "fields_used": [
                "base_elevation_m",
                "terrain",
                "water_bodies",
                "fields.epicenter",
            ],
            "extensions": [
                "top-level region (polygon, anchor, world frame, charter)",
                "top-level provenance (per-source attribution/license/service)",
                "terrain.grid object (path, spacing, rows, cols, crs, world_origin_m, belt window)",
                "water_bodies[].datum + surface_elevation_m + mtfcc",
            ],
            "note": "All extensions are optional/backward-compatible; v1.1 consumers "
            "that only read buildings keep working.",
        },
    }
    report_path = f"{SEATTLE_ROOT}/report.json"
    with open(report_path, "w") as handle:
        json.dump(report, handle, indent=2)
    print("LAYERS", json.dumps(layers, indent=1))
    print("ZONES", zone_counts)
    print("BELT", belt_report)
    print("TERRAIN", terrain_report)
    print("WATER", water_report)
    print("BUILDINGS", len(buildings), "belt", len(belt_buildings))
    print("SIZES", report["sizes_mb"])
    print("wrote", full_path, belt_path, report_path)


if __name__ == "__main__":
    command = sys.argv[1] if len(sys.argv) > 1 else "manhattan"
    {"manhattan": manhattan, "synthetic": synthetic, "seattle": seattle}[command]()
