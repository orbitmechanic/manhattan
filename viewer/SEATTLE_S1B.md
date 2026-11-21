# Seattle S1.5 — height enrichment, partition/streaming format, horizon

Tag: `seattle-s1b`. Branch: `seattle`. Builds on S1 (`seattle-s1`); S2
terrain/water/roads remain out of scope.

Regenerate from `viewer/`:

```sh
python3 scripts/seattle_enrich.py     # Overture + MSML join -> enriched JSON
python3 scripts/partition_city.py     # partitions overlay + CityPack binary
python3 scripts/horizon_build.py      # far-field grid + 360 profiles
python3 scripts/test_s1b.py           # Python matcher/accounting/pack checks
cargo test --release citypack horizon
```

## 1. Height enrichment

Sources fetched by scripted HTTP:

| Source | Records | License |
|---|---:|---|
| Overture Maps buildings, release **2026-11-18.0** (bbox of the S1 region) | 934,201 raw / 732,908 with height or floors | ODbL-1.0 |
| Microsoft Global ML Building Footprints, quadkeys `021230021/23/30/32` | 758,000 in-region parsed | CDLA-Permissive-2.0 |

Overture's height provenance is explicit: 276,560 records cite `USGS Lidar` and
441,828 cite `Microsoft ML Buildings` (also lidar-derived in the US). Both are
recorded as `lidar_overture`.

Matcher: original centroid inside the Overture footprint, footprint area ratio
0.40–2.50 (choose ratio closest to 1 among candidates); unmatched originals try
nearest ≤ 8 m with IoU ≥ 0.35; then MSML with ratio 0.30–3.00, nearest ≤ 10 m,
IoU ≥ 0.35. **291,726 / 330,330 (88.3 %)** matched Overture (287,916 by
containment, 3,810 by IoU); **13,460** additional buildings matched MSML.

Priority: `real_surveyed` > `lidar_overture` > `osm_levels_derived` >
`msml_estimate` > `area_heuristic`. Tacoma survey heights (100 % real) were
never replaced; Seattle 2015 apex heights stayed `real_surveyed`.

### Coverage: baseline → after

| City | Baseline real | After real_surveyed | lidar_overture | osm_levels | msml_estimate | area_heuristic |
|---|---:|---:|---:|---:|---:|---:|
| Seattle (187,053) | 40.3 % | 75,461 (40.3 %) | **105,522 (56.4 %)** | 56 | 2,346 | 3,668 |
| Bellevue (56,856) | **0 %** | 0 | **46,101 (81.1 %)** | 10 | 501 | 10,244 |
| Tacoma (86,421) | 100 % | 86,421 (100 %) | 0 | 0 | 0 | 0 |
| **Total (330,330)** | **49.0 %** | 161,882 (49.0 %) | **151,623 (45.9 %)** | 66 | 2,847 (0.9 %) | 13,912 (4.2 %) |

Real + lidar coverage is **94.9 %** overall; Bellevue moved from 0 % to 81.1 %
lidar, and Seattle from 40.3 % to 96.7 % real+lidar.

`levels` (Overture `num_floors`) coverage is **1.92 %**. Overture release
2026-11-18.0 has no `start_date` column: **start-date coverage 0 %** and the
field is absent. Both are reported as actuals, not assumed.

### Downtown spot-check (140 m around the anchor, world 0,0)

| Building | Before m | After m | Source |
|---:|---:|---:|---|
| 80153 | 66.87 | 161.50 | area → lidar_overture |
| 84429 | 93.41 | 15.00 | area → lidar_overture |
| 87345 | 11.16 | 75.30 | area → lidar_overture |
| 110265 | 7.75 | 54.30 | area → lidar_overture |

Full machine-readable samples: `assets/datasets/seattle_enrich_samples.json`
(20 pairs with both footprints); report `datasets/seattle/height_report.json`.

## 2. Neighborhood partitions + CityPack v1

Scheme: adaptive quadtree, 2 km root cells split while > 750 buildings and
larger than 250 m; no merge. **1,096 partitions / 330,330 buildings**, mean
301.4, max 749; 703 partitions in the 250–750 target band, 393 sparse
(< 250), none over 750. Three float-boundary records were rescued into their
nearest partition. Named overlay includes Downtown, Capitol Hill, Ballard,
Queen Anne, Fremont, University District, West Seattle, Beacon Hill, Rainier
Valley, Bellevue CBD, Redmond/Eastside, Tacoma Core/North/South, South King;
every partition carries centre, AABB, building count, dominant category/source,
mean/p95 height and glass percentage (`datasets/seattle/partitions.json`).

CityPack v1 (spec in `CITY_DATASET.md`, reader in `src/citypack.rs`): chunks are
independently loadable; coordinates are i16 at 0.25 m relative to the partition
centre (vertices relative to the building centroid); heights are u16
centimetres; category/source strings are interned per chunk.

| Artifact | Size | Win |
|---|---:|---:|
| Enriched JSON | 146.6 MB | — |
| CityPack binary | **23.84 MB** | **6.15×** smaller |
| One partition JSON | 50,467 B | — |
| One partition chunk | **7,861 B** | **6.42×** smaller |

Runtime streamer (`--seattle-stream`): 1,096 partitions, radius 2.6 km, at most
2 loads/frame and a 6 ms load budget, 4 unloads/frame, one merged mesh per
partition. HUD captures:

| Capture | Loaded | Buildings | Cumulative +/− | Last event |
|---|---:|---:|---:|---|
| `stream_downtown.png` | 56/1096 | 17,045 | +63 / −7 | − Downtown 03 |
| `stream_lake.png` | 13/1096 | 2,457 | +72 / −59 | − Capitol Hill 37 |
| `stream_bellevue.png` | 23/1096 | 6,542 | +91 / −68 | + Bellevue CBD 29 |

Point resolution is two-level: partition AABB first, then footprints inside
candidate partitions (`PartitionLookup::resolve_point`). The Seattle streaming
path does not build or consult the old 1,000 m uniform grid; Manhattan's
in-memory path is unchanged. Tests (Rust) cover at-scale lookup vs brute force,
fixture round-trip quantisation, and budget planning with an exaggerated radius.

## 3. Horizon mountain system

Far-field grid: AWS Open Data Terrain Tiles (Mapzen Terrarium z8, a
SRTM/3DEP/GMTED composite), resampled to **400×400 at 1 km** in a local
azimuthal-equidistant frame around downtown; the Rainier cell is refined with
z13 local max (contract requires 4,390 ± 3 m). Tree flag from ESA WorldCover
2021 v200 classes 10/20; snow flag = elevation ≥ 2,400 m or WorldCover class 70.

| Item | Value |
|---|---|
| Packed grid | **320,000 B** (312.5 KiB), uint16: 14-bit elevation/0.5 m, tree bit, snow bit |
| Profiles | **288,000 B** (281.25 KiB), 360 bins × 400 samples @ 500 m |
| Elevation range | 0 – 4,393.0 m |
| Tree / snow / water cells | 89,932 / 578 / 24,027 |
| Rainier cell | row 283, col 243, **4,393.0 m**, snow set, tree clear |
| Rainier bearing / distance / angle from 250 m | **152.48° / 94.2 km / 2.52°** |
| Summit refinement | z13 tiles: 4,392.9 m (1 km grid was 4,180.5 m) |

Renderer: 1,440 segments (0.25° by interpolation), three passes per azimuth —
nearest ridge (black fill), taller mid terrain ≤ 90 km (grey-blue fill), distant
summits > 90 km (whitecap). The ring sits at 30 km, uses vertex colours and
`fog_enabled=false`, one draw. Targets are recomputed at 2.5 Hz or after 150 m
of camera motion and interpolated between updates; the grid is never scanned
per frame. Captures: Rainier noon/dawn/dusk + downtown, Olympics west, Cascades
east, Cascade zoom, overcast.

Measured on this machine: horizon profile update **4.3–6.7 ms** at 2.5 Hz
(amortised ≈ 0.1–0.2 ms per 60 Hz frame), **3,780–6,954 triangles, 1 draw
call**. Render submission is negligible; no separate GPU timer is claimed.

## 4. Verification and honest labels

- `cargo test --release citypack horizon`: 6 new tests (fixture round-trip,
  at-scale lookup vs brute force, budget planner, packed cell contracts,
  profile raycast comparison, Rainier apex/bearing).
- `python3 scripts/test_s1b.py`: 8 tests (provenance accounting, matcher sample
  recomputation, partition accounting/size, pack fixture, horizon packing,
  Rainier quadrant/angle, profile shape).
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, full viewer
  test suite; destructibles untouched.

| Claim | Label |
|---|---|
| Fetch, adapter, enrichment, partition build, pack, horizon build | **real execution** |
| Coverage tables, matcher ratios, partition/size report | **programmatic** |
| Overture/MSML heights | lidar/ML-derived source data, not this project's survey |
| Seattle real heights | 40.3 % `real_surveyed`; 56.4 % lidar; rest estimates |
| Bellevue | 81.1 % lidar; no city-surveyed heights |
| `levels` 1.92 %, `start_date` 0 % | actuals from this Overture release |
| Terrain rendering, roads, water meshes, style packs | **not implemented (S2/S3)** |
| Horizon render cost | 1 draw, ≤ ~7k tris; update timed, render not GPU-timed |

Limitations: 3DEP/Terrarium is a coarse surface (summit refined only for
Rainier); the snowline is a coarse elevation rule, not seasonal snow data;
partitions below 250 buildings are accepted in sparse areas; the streamer's
placeholders are flat-shaded prisms with no facades, and the ground is a plane.
