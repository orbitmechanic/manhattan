# Seattle S1 — data round

Tag: `seattle-s1`. Branch: `seattle`. Charter (Iteration 30 / Iteration 35):
downtown Seattle focus, priority south toward Tacoma, east across Lake
Washington to Bellevue, Puget Sound as the western shore, with the northern
suburban fade clipped.

Everything here is scripted HTTP. The fetch tool is `scripts/seattle_data.py`
(Python because the pinned geospatial stack — shapely 2.2.0, pyproj 3.8.0,
rasterio 1.5.2/GDAL 3.12.2, geopandas 1.2.0 — lives there; the 3D side stays
Rust). The adapter is the `seattle` mode of `scripts/city_dataset.py`.

Regenerate from `viewer/`:

```sh
python3 scripts/seattle_data.py fetch     # resumable; caches raw/
python3 scripts/city_dataset.py seattle   # dataset + report + belt extract
PILOT_DATASET=assets/datasets/seattle_belt_v11.json \
  PILOT_BUNDLE_OUT=seattle_belt_bundle.json ./target/release/pilot_gen
```

## 1. Region

| Item | Value |
|---|---|
| Name | `puget-sound-downtown-to-tacoma` |
| Anchor (world origin) | `-122.3355, 47.6062` (downtown Seattle), EPSG:2926 `1269879.86, 224737.70` |
| Frame | x = east metres, z = south metres, y = `base_elevation_m` |
| Polygon | `datasets/seattle/region.geojson` / `region.polygon_wgs84` |
| North | 47.72, narrowed to the -122.40…-122.20 band (suburban fade clipped) |
| South | 47.10 (south Tacoma), widening through the corridor |
| East | -122.08 (Lake Washington crossed; Bellevue/Redmond edge) |
| West | -122.55 (Puget Sound) |

## 2. Fetch: sources, counts, provenance

All raw files and their SHA-256 checksums are listed in
`datasets/seattle/manifest.json`; every source's service URL, license and
attribution is embedded again in the dataset's `provenance` array.

| Layer | Raw features | Source |
|---|---:|---|
| Seattle Building Outlines 2023 | 205,864 | City of Seattle GeoData (open data) |
| Seattle Building Outline 2015 (BP99 apex) | 254,464 centroids | City of Seattle GeoData |
| Seattle Street Network Database (SND) | 29,833 | City of Seattle GeoData |
| Seattle zoning (generalized) | 3,465 | City of Seattle GeoData |
| Seattle parks | 488 | City of Seattle GeoData |
| Seattle shoreline | 349 | City of Seattle GeoData |
| Bellevue Buildings | 63,574 | City of Bellevue GIS |
| Tacoma 2024 Building Footprints | 91,000 | City of Tacoma Open Data |
| Water polygons (TIGER AREAWATER) | 432 | US Census Bureau (public domain) |
| 3DEP DEM tiles | 2 tiles (`n48w123`, `n48w122`) | USGS 3DEP 1/3 arc-second (public domain) |

2015 apex elevations are fetched as centroids only (`returnCentroid=true`) to
avoid a 250k-polygon download; they are joined to the 2023 outlines by nearest
centroid. Tacoma is queried with its own `maxRecordCount` (1000); the first
attempt returned only the first page and was fixed before the final run.

## 3. Adapter: parsed / rejected

| Source | Parsed (after region clip) | Rejected | Reason accounting |
|---|---:|---:|---|
| Seattle 2023 | 187,053 | 18,811 | 3 invalid/empty, 0 outside region, 18,808 below 300 sq ft |
| Bellevue | 56,856 | 6,718 | 5 invalid, 0 outside region, 6,713 inactive/non-building |
| Tacoma 2024 | 86,421 | 4,579 | 0 invalid, 16 outside region, 4,563 below 300 sq ft |
| **Total buildings** | **330,330** | 30,108 | |

The 300 sq ft floor drops sheds/garages; Seattle's 2023 layer has no height or
elevation attribute at all.

## 4. Height and elevation coverage (actuals)

| Source | Roof/elevation attribute | Real height | Neighbour-inferred | Area-estimated | Base elevation |
|---|---|---:|---:|---:|---:|
| Seattle 2023 | none in layer; `BP99_APEX` from 2015 within 25 ft | **40.3 %** | 17.3 % | 42.4 % | 100 % (3DEP) |
| Bellevue | none | **0 %** | — | 100 % | 100 % (3DEP) |
| Tacoma 2024 | `average_z` (lidar roof height) | **100 %** | — | — | 100 % (3DEP) |

- Seattle: 74.7 % of 2023 outlines find a 2015 `BP99_APEX` within 25 ft, but only
  40.3 % produce a physically plausible `apex - base` (2.5–400 m). The rest fall
  through to a 100 ft neighbour inference or a footprint-area estimate. Height
  percentiles (real/inferred/estimated mix): p05 6.0 m, p50 10.4 m, p95 85.6 m.
- Bellevue publishes footprints only; every height is an area estimate and is
  labelled as such, never counted as real.
- Tacoma `average_z` was validated against the DEM: the layer's `lag` (lowest
  adjacent ground) matches 3DEP within ~1 m, and `average_z` resolves to roof
  height above that ground. Tacoma heights p05/p50/p95 are 8.8 / 13.1 / 22.6 m.
- Base elevations are 3DEP-derived (10 m grid, NAVD88 approx.), not survey
  grade. They are the project's first non-zero elevations; Manhattan supplied 0.

## 5. Terrain and water

| Item | Value |
|---|---|
| Regional DEM grid | 2322 × 1207 cells at 30 m, EPSG:2855, NAVD88 (approx.) |
| Coverage | 100 % of the region polygon |
| Elevation range | -1.5 m to 489.9 m |
| Pilot-belt grid | 120 × 120 at 10 m, 119.4 m relief across the 1.2 km window |
| Water | 153 polygons ≥ 1 ha, 719.7 km² total, 103 named |
| Water kinds | 104 lake, 31 sound, 11 other, 4 stream, 3 canal |
| Datum | NAVD88 (approx., 3DEP-derived surface elevation per polygon) |

## 6. Zone counts (programmatic sanity check)

| Zone | Buildings |
|---|---:|
| Downtown core (≤ 2 km of anchor) | 2,828 |
| Bellevue (≤ 3 km of Bellevue centre) | 9,685 |
| Tacoma (≤ 4 km of Tacoma centre) | 23,676 |
| South corridor (between Seattle and Tacoma) | 63,051 |
| Rest of region | 231,090 |

Bellevue and the southern extent are present as data, not only as a polygon.

## 7. Contract changelog

v1.1 fields were sufficient for the per-building data (`base_elevation_m`) and
for `terrain` / `water_bodies`. Seattle forced **optional, backward-compatible**
metadata shapes, documented in `CITY_DATASET.md`:

- top-level `region` (polygon, WGS84 + EPSG:2926 anchor, world frame, charter)
  because "region = dataset metadata";
- top-level `provenance` so attribution/licence/service travel with the data;
- a defined `terrain` object (path, CRS, spacing, rows/cols, world origin, belt window);
- `water_bodies[]` gains `datum`, `surface_elevation_m`, `mtfcc`.

No version bump: consumers that read only `buildings[]` are unaffected.

## 8. Preview render (loadability proof, not S2)

Belt selection: densest candidate with 20–60 m p95–p5 base relief inside 420 m
(a Queen Anne/Capitol-Hill-style hillside, not a cliff).

| Item | Value |
|---|---|
| Belt centre (world) | `-2273.2, -3741.8` |
| Buildings within 420 m | 1,197 |
| Base range in belt | 58.6 – 125.5 m |
| p95–p5 relief | 36.0 m |
| Generated pilot records | **140 / 140, 0 rejects, 0 envelope/row offenders** |
| Batches | 112 brick brownstone, 20 brick mid-rise, 8 concrete grid |
| Triangles (estimate) | 96,966 |
| Field wiring | 840 storefront panels, 560 ad panels |
| Load time | 0.13 s (Shell tier; structure graphs deferred) |

Preview ground is a **flat placeholder** at the belt's minimum base (101.7 m);
each building spawns a stepped plinth up to its surveyed 3DEP base, so the
hillside steps uphill without an S2 terrain mesh. Captures:

- `captures/seattle_s1_street.png` — street canyon, storefront/ad bands visible
- `captures/seattle_s1_aerial.png` — belt at real relative base elevations

## 9. Honest labels

| Claim | Label |
|---|---|
| HTTP fetch, pagination, retries, checksums | **real execution** |
| Adapter counts, rejections, coverage, zone counts | **programmatic** |
| 3DEP terrain/water coverage | **programmatic** |
| Preview generation + captures | **real execution** |
| Seattle buildings with real roof heights | **40.3 % actual**, 17.3 % neighbour-inferred, 42.4 % estimated |
| Bellevue heights | **0 % real** — area estimates |
| Base elevations | 3DEP-derived (approx. NAVD88), **not survey grade** |
| Roads/terrain/water rendering, SND placement | **not implemented (S2+)** |

## 10. Known limitations

- The Seattle 2023 outlines carry no roof/base attributes; the 2015 apex join is
  the only real height source and covers 40 % after plausibility filtering.
- 3DEP is a surface model in dense cover; base elevations can sit a few metres
  above bare ground under canopy/roofs. S2 should prefer bare-earth/lidar ground
  (`lag`-style) where available.
- Bellevue has no height attribute in this source; S3 needs a height-bearing
  footprint source for the east side.
- The preview uses a flat plane + stepped plinths, not terrain, water or roads;
  those are S2.
- Raw and full derived data (~1.4 GB) are intentionally not committed; the
  manifest, report, region polygon, belt extract and preview bundle are.
