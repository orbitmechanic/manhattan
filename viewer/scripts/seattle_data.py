#!/usr/bin/env python3
"""Seattle S1 fetch: scripted HTTP only, no browsing.

Pulls the layers the CityDataset v1.1 adapter needs for the Puget Sound pilot
region (downtown Seattle focus, south through Tacoma, east across Lake
Washington to Bellevue, west to Puget Sound):

* City of Seattle GeoData FeatureServer: Building Outlines 2023, Building
  Outline 2015 (roof apex attributes), Street Network Database, zoning, parks,
  shoreline.
* City of Bellevue Buildings (footprints; regional coverage east of the lake).
* City of Tacoma 2024 Building Footprints (roof ``average_z``; south extent).
* US Census TIGER/Line AREAWATER (King/Pierce/Snohomish) for water polygons.
* USGS 3DEP 1/3 arc-second DEM tiles for the metro region.

Everything is cached under ``viewer/datasets/seattle/raw`` and resumable; the
manifest records source URLs, licenses, counts and checksums so provenance
travels with the data.

Usage:
    python3 scripts/seattle_data.py fetch [--force]
    python3 scripts/city_dataset.py seattle      # adaptation + report
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
import time
import urllib.parse
import urllib.request

ROOT = "datasets/seattle"
RAW = f"{ROOT}/raw"
MANIFEST = f"{ROOT}/manifest.json"

SEATTLE_BASE = "https://services.arcgis.com/ZOyb2t4B0UYuYNYH/arcgis/rest/services"
BELLEVUE_BASE = "https://services1.arcgis.com/EYzEZbDhXZjURPbP/arcgis/rest/services"
TACOMA_BASE = "https://services3.arcgis.com/SCwJH1pD8WSn5T5y/arcgis/rest/services"

# Pilot region, WGS84 lon/lat. Narrow at the northern fade, wide south through
# Tacoma, east across Lake Washington to Bellevue, west across Puget Sound.
REGION_WGS84 = [
    [-122.400, 47.720],
    [-122.200, 47.700],
    [-122.080, 47.580],
    [-122.100, 47.350],
    [-122.180, 47.100],
    [-122.500, 47.100],
    [-122.550, 47.400],
    [-122.500, 47.600],
    [-122.400, 47.720],
]
ANCHOR_WGS84 = [-122.3355, 47.6062]  # downtown Seattle = dataset world origin

SEATTLE_LAYERS = {
    "seattle_buildings_2023": {
        "service": f"{SEATTLE_BASE}/Building_Outlines_2023/FeatureServer/0",
        "where": "1=1",
        "attribution": "City of Seattle GeoData - Building Outlines 2023",
        "license": "City of Seattle open data (see data.seattle.gov terms)",
    },
    "seattle_buildings_2015": {
        "service": f"{SEATTLE_BASE}/Building_Outline_2015/FeatureServer/0",
        "where": "BP99_TYPE='BLD'",
        "attribution": "City of Seattle GeoData - Building Outline 2015 (BP99 apex elevations)",
        "license": "City of Seattle open data",
        "centroids": True,
    },
    "seattle_snd": {
        "service": f"{SEATTLE_BASE}/Street_Network_Database_SND/FeatureServer/0",
        "where": "1=1",
        "attribution": "City of Seattle GeoData - Street Network Database (SND)",
        "license": "City of Seattle open data",
    },
    "seattle_zoning": {
        "service": f"{SEATTLE_BASE}/Current_Land_Use_Zoning_Generalized_new/FeatureServer/0",
        "where": "1=1",
        "attribution": "City of Seattle GeoData - Current Land Use Zoning (generalized)",
        "license": "City of Seattle open data",
    },
    "seattle_parks": {
        "service": f"{SEATTLE_BASE}/Park_Boundaries/FeatureServer/0",
        "where": "1=1",
        "attribution": "City of Seattle GeoData - Park Boundaries",
        "license": "City of Seattle open data",
    },
    "seattle_shoreline": {
        "service": f"{SEATTLE_BASE}/Shoreline/FeatureServer/0",
        "where": "1=1",
        "attribution": "City of Seattle GeoData - Shoreline",
        "license": "City of Seattle open data",
    },
    "bellevue_buildings": {
        "service": f"{BELLEVUE_BASE}/Buildings/FeatureServer/0",
        "where": "1=1",
        "attribution": "City of Bellevue GIS - Buildings",
        "license": "City of Bellevue open data",
    },
    "tacoma_buildings_2024": {
        "service": f"{TACOMA_BASE}/Tacoma_2024_BuildingFootprints/FeatureServer/0",
        "where": "1=1",
        "attribution": "City of Tacoma Open Data - Building Footprints 2024 (lidar average_z)",
        "license": "City of Tacoma open data",
    },
}

CENSUS_WATER = {
    "king": ("53033", "https://www2.census.gov/geo/tiger/TIGER2024/AREAWATER/tl_2024_53033_areawater.zip"),
    "pierce": ("53053", "https://www2.census.gov/geo/tiger/TIGER2024/AREAWATER/tl_2024_53053_areawater.zip"),
    "snohomish": ("53061", "https://www2.census.gov/geo/tiger/TIGER2024/AREAWATER/tl_2024_53061_areawater.zip"),
}

DEM_TILES = ["n48w123", "n48w122"]


def log(*args: object) -> None:
    print(*args, flush=True)


def get_json(url: str, tries: int = 4) -> dict:
    last: Exception | None = None
    for attempt in range(tries):
        try:
            with urllib.request.urlopen(url, timeout=90) as response:
                return json.load(response)
        except Exception as exc:  # noqa: BLE001 - transient HTTP/JSON errors
            last = exc
            time.sleep(1.5 * (attempt + 1))
    raise RuntimeError(f"GET failed after {tries} tries: {url} ({last})")


def sha256(path: str) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def load_manifest() -> dict:
    if os.path.exists(MANIFEST):
        with open(MANIFEST) as handle:
            return json.load(handle)
    return {
        "city_id": "seattle",
        "region": {
            "name": "puget-sound-downtown-to-tacoma",
            "polygon_wgs84": REGION_WGS84,
            "anchor_wgs84": ANCHOR_WGS84,
            "charter": "downtown focus; south priority toward Tacoma; east across "
            "Lake Washington to Bellevue; Puget Sound western shore; northern "
            "suburban fade clipped",
        },
        "sources": [],
        "counts": {},
        "raw": {},
    }


def save_manifest(manifest: dict) -> None:
    os.makedirs(ROOT, exist_ok=True)
    manifest["generated_utc"] = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    with open(MANIFEST, "w") as handle:
        json.dump(manifest, handle, indent=2, sort_keys=True)


def region_ring_2926(step: float = 5.0) -> list[list[float]]:
    """Region polygon densified into EPSG:2926 feet for the ArcGIS envelope."""
    from pyproj import Transformer

    transformer = Transformer.from_crs("EPSG:4326", "EPSG:2926", always_xy=True)
    dense: list[list[float]] = []
    for start, end in zip(REGION_WGS84, REGION_WGS84[1:]):
        for i in range(int(step) + 1):
            t = i / step
            lon = start[0] + (end[0] - start[0]) * t
            lat = start[1] + (end[1] - start[1]) * t
            x, y = transformer.transform(lon, lat)
            dense.append([round(x, 2), round(y, 2)])
    return dense


def fetch_layer(name: str, spec: dict, ring: list[list[float]], force: bool) -> None:
    out = f"{RAW}/{name}.geojson"
    if os.path.exists(out) and not force:
        with open(out) as handle:
            count = len(json.load(handle).get("features", []))
        log(f"cached {name}: {count} features")
        return
    features: list[dict] = []
    offset = 0
    meta = get_json(f"{spec['service']}?f=pjson")
    page = min(2000, int(meta.get("maxRecordCount") or 2000))
    log(f"{name}: maxRecordCount {meta.get('maxRecordCount')} -> page {page}")
    while True:
        query = {
            "where": spec["where"],
            "outFields": "*",
            "f": "geojson",
            "outSR": "2926",
            "resultOffset": offset,
            "resultRecordCount": page,
            "orderByFields": "objectid ASC",
        }
        if spec.get("centroids"):
            query.update(
                {
                    "returnGeometry": "false",
                    "returnCentroid": "true",
                    "f": "json",
                }
            )
        else:
            query.update(
                {
                    "returnGeometry": "true",
                    "geometry": json.dumps({"rings": [ring]}),
                    "geometryType": "esriGeometryPolygon",
                    "inSR": "2926",
                    "spatialRel": "esriSpatialRelIntersects",
                }
            )
        url = f"{spec['service']}/query?" + urllib.parse.urlencode(query)
        data = get_json(url)
        batch = data.get("features", [])
        if spec.get("centroids"):
            converted = []
            for feature in batch:
                centroid = feature.get("centroid") or {}
                if "x" not in centroid:
                    continue
                converted.append(
                    {
                        "type": "Feature",
                        "geometry": {
                            "type": "Point",
                            "coordinates": [centroid["x"], centroid["y"]],
                        },
                        "properties": feature.get("attributes", {}),
                    }
                )
            features.extend(converted)
        else:
            features.extend(batch)
        log(f"{name}: {len(features)} features (offset {offset})")
        if not batch:
            break
        if len(batch) < page and not data.get("exceededTransferLimit"):
            break
        offset += len(batch)
    os.makedirs(RAW, exist_ok=True)
    with open(out, "w") as handle:
        json.dump({"type": "FeatureCollection", "features": features}, handle)
    log(f"wrote {out}: {len(features)} features, {os.path.getsize(out) / 1e6:.1f} MB")


def fetch_water(ring: list[list[float]], force: bool) -> None:
    """Census AREAWATER clipped to the region, as one 2926 GeoJSON."""
    import geopandas as gpd
    from shapely.geometry import Polygon, shape

    out = f"{RAW}/water_areas.geojson"
    if os.path.exists(out) and not force:
        with open(out) as handle:
            log(f"cached water_areas: {len(json.load(handle)['features'])} polygons")
        return
    region = Polygon(REGION_WGS84)
    frames = []
    for county, (_fips, url) in CENSUS_WATER.items():
        zipped = f"{RAW}/tl_2024_{_fips}_areawater.zip"
        if not os.path.exists(zipped):
            log(f"download {url}")
            urllib.request.urlretrieve(url, zipped)
        frame = gpd.read_file(f"zip://{zipped}")
        frame = frame[frame.intersects(region)]
        log(f"water {county}: {len(frame)} polygons intersect region")
        frames.append(frame)
    water = gpd.GeoDataFrame(
        __import__("pandas").concat(frames, ignore_index=True), crs="EPSG:4269"
    ).to_crs("EPSG:2926")
    water["geometry"] = water.geometry.intersection(
        gpd.GeoSeries([region], crs="EPSG:4326").to_crs("EPSG:2926").iloc[0]
    )
    water = water[~water.geometry.is_empty]
    water["geometry"] = water.geometry.simplify(3.0)
    water = water[water.geometry.area > 2000.0]
    water[["MTFCC", "FULLNAME", "AWATER", "geometry"]].to_file(out, driver="GeoJSON")
    log(f"wrote {out}: {len(water)} water polygons, {os.path.getsize(out) / 1e6:.1f} MB")


def fetch_dem(force: bool) -> None:
    for tile in DEM_TILES:
        out = f"{RAW}/USGS_13_{tile}.tif"
        if os.path.exists(out) and not force:
            log(f"cached {out}")
            continue
        url = (
            "https://prd-tnm.s3.amazonaws.com/StagedProducts/Elevation/13/TIFF/"
            f"current/{tile}/USGS_13_{tile}.tif"
        )
        log(f"download {url}")
        urllib.request.urlretrieve(url, out)
        log(f"wrote {out}: {os.path.getsize(out) / 1e6:.1f} MB")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", nargs="?", default="fetch", choices=["fetch"])
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--layer", action="append", default=None)
    args = parser.parse_args()

    os.makedirs(RAW, exist_ok=True)
    manifest = load_manifest()
    ring = region_ring_2926()
    known = {source["layer"] for source in manifest["sources"]}
    for name, spec in SEATTLE_LAYERS.items():
        if args.layer and name not in args.layer:
            continue
        if name in known and not args.force:
            path = f"{RAW}/{name}.geojson"
            if os.path.exists(path):
                log(f"cached {name}")
                continue
        started = time.time()
        fetch_layer(name, spec, ring, args.force)
        path = f"{RAW}/{name}.geojson"
        with open(path) as handle:
            count = len(json.load(handle)["features"])
        manifest["counts"][name] = count
        manifest["raw"][os.path.basename(path)] = {
            "bytes": os.path.getsize(path),
            "sha256": sha256(path),
        }
        manifest["sources"] = [
            source for source in manifest["sources"] if source["layer"] != name
        ] + [
            {
                "layer": name,
                "service": spec["service"],
                "where": spec["where"],
                "attribution": spec["attribution"],
                "license": spec["license"],
                "crs": "EPSG:2926",
                "fetched_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                "seconds": round(time.time() - started, 1),
            }
        ]
        save_manifest(manifest)

    started = time.time()
    fetch_water(ring, args.force)
    manifest["counts"]["water_areas"] = len(
        json.load(open(f"{RAW}/water_areas.geojson"))["features"]
    )
    manifest["raw"]["water_areas.geojson"] = {
        "bytes": os.path.getsize(f"{RAW}/water_areas.geojson"),
        "sha256": sha256(f"{RAW}/water_areas.geojson"),
    }
    manifest["sources"] = [
        source for source in manifest["sources"] if source["layer"] != "water_areas"
    ] + [
        {
            "layer": "water_areas",
            "service": "; ".join(url for _fips, url in CENSUS_WATER.values()),
            "attribution": "US Census Bureau TIGER/Line 2024 AREAWATER (King, Pierce, Snohomish)",
            "license": "Public domain (US Census Bureau)",
            "crs": "EPSG:2926",
            "fetched_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "seconds": round(time.time() - started, 1),
        }
    ]
    save_manifest(manifest)

    started = time.time()
    fetch_dem(args.force)
    for tile in DEM_TILES:
        path = f"{RAW}/USGS_13_{tile}.tif"
        manifest["raw"][os.path.basename(path)] = {
            "bytes": os.path.getsize(path),
            "sha256": sha256(path),
        }
    manifest["sources"] = [
        source for source in manifest["sources"] if source["layer"] != "dem_3dep"
    ] + [
        {
            "layer": "dem_3dep",
            "service": "https://prd-tnm.s3.amazonaws.com/StagedProducts/Elevation/13/"
            "TIFF/current/{tile}/USGS_13_{tile}.tif",
            "tiles": DEM_TILES,
            "attribution": "USGS 3D Elevation Program (3DEP) 1/3 arc-second",
            "license": "Public domain (USGS)",
            "crs": "EPSG:4269 + NAVD88 (approx.)",
            "fetched_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
            "seconds": round(time.time() - started, 1),
        }
    ]
    save_manifest(manifest)
    log("FETCH OK", json.dumps(manifest["counts"], sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
