# CityDataset v1.1 (+ S1.5 extensions)

JSON: `version`, `city_id`, `attribution`, `license`, optional `fields`
(epicenter, ad/glass radii), optional `terrain` (elevation grid), optional
`water_bodies` (polygons + datum), optional `region` / `provenance`. `buildings[]`:
id, block, centroid, perimeter (world metres), height_m, floor_height_m,
category, optional year_built, optional base_elevation_m (default 0), optional
`height_source`, optional `levels`. Adapters: `scripts/city_dataset.py
manhattan|synthetic|seattle`. The pilot generator consumes only this file
(`PILOT_DATASET`).

## Height provenance (S1.5)

Every enriched building carries one documented `height_source`:

| Value | Meaning |
|---|---|
| `real_surveyed` | City of Tacoma lidar roof height (`average_z`) or Seattle 2015 `BP99_APEX` roof-elevation join |
| `lidar_overture` | Overture Maps height (USGS Lidar or Microsoft ML Buildings, both lidar-derived) |
| `osm_levels_derived` | Overture `num_floors` × 3.2 m where no height is published |
| `msml_estimate` | Microsoft Global ML Building Footprints height for originals Overture does not cover |
| `area_heuristic` | footprint-area estimate (pre-existing fallback) |

Conflict priority, applied in `scripts/seattle_enrich.py`:

`real_surveyed` > `lidar_overture` > `osm_levels_derived` > `msml_estimate` >
`area_heuristic`

Tacoma survey data never loses to Overture. The matcher is centroid-in-footprint
plus footprint area ratio 0.40–2.50; unmatched originals try nearest ≤8 m with
IoU ≥ 0.35, then MSML with area ratio 0.30–3.00 / IoU ≥ 0.35 / ≤10 m.

`levels` is copied from Overture `num_floors` where present. Overture release
2026-11-18.0 has no `start_date` column, so start-date coverage is 0 % in this
round and the field is absent.

## Binary CityPack v2 (S1c)

`assets/datasets/seattle.citypack` is the compiled hierarchy. Little-endian:

```
header  b"CDPK" u16 2 u32 header_bytes u32 nodes u32 buildings u32 streets u32 chunks
nodes   u8 level i32 parent u32 first_child u16 child_count
        u32 buildings u32 streets f32 cx f32 cz f32 bounds[4]
        u16 mean_height_cm u16 p95_height_cm u8 glass_pct
        u8 dominant_category u8 dominant_source u8 name_len name
        u32 chunk_offset u32 chunk_bytes
chunks  (neighborhood nodes only)
        u32 chunk_bytes u32 buildings u32 streets f32 center[2] f32 bounds[4]
        u8 dominant_category u8 dominant_source u16 mean_h u16 p95 u8 glass u8 flags
        u16 string_count [u16 len bytes]*
        buildings: u32 id i16 qx i16 qz u16 height_cm u16 floor_height_cm
                   u16 base_cm u16 floor_count u16 year_built u8 levels
                   u8 source_idx u8 category_idx u16 vertex_count
                   [i16 qx i16 qz]*vertex_count
        streets:   u32 id u8 kind_idx u16 name_idx u16 point_count
                   [i16 qx i16 qz]*point_count
```

Levels: 0 county, 1 city, 2 neighborhood; leaves are buildings and street
segments. Parent AABBs are tight unions of children (strict containment),
and names/ancestry are `County -> City -> Neighborhood`. Chunk coordinates are
i16 at 0.25 m relative to the neighborhood centre (vertices relative to the
building centroid); heights are u16 centimetres; category/source/street-kind/
street-name strings are interned per chunk. `src/citypack.rs` owns the reader,
four-level descent, gated streamer and point resolver; `src/bin/hierarchy_eval.rs`
prints the evaluation-count table; `scripts/partition_city.py` writes the pack
and `datasets/seattle/partitions.json` (all nodes with centre, AABB, counts,
hints and parent).

## Horizon assets (S1.5)

- `assets/horizon/seattle_horizon.bin` — 400×400 uint16, 1 km local postings,
  bits 15–2 elevation / 0.5 m, bit 1 tree (WorldCover 10/20), bit 0 snow
  (elevation ≥ 2400 m or WorldCover 70).
- `assets/horizon/seattle_profiles.bin` — 360 azimuth bins × 400 samples at
  500 m steps, uint16 elevation / 0.5 m.
- `datasets/seattle/horizon_report.json` — source, flags counts, Rainier cell,
  contracted test cells.

## Pilot / runtime seams

The pilot preview accepts `PILOT_DATASET`, `PILOT_CENTER=x,z`,
`PILOT_BUNDLE_OUT=name.json`, `PILOT_BUNDLE=name.json`, and `PILOT_CLEAN=1`.
`--seattle-stream` runs the full binary streamer; `--seattle-horizon` runs the
horizon capture sequence. Manhattan bundles do not emit a `view` block, so A.2a
camera poses are unchanged.
