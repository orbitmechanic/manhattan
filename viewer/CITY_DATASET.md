# CityDataset v1.1

JSON: `version`, `city_id`, `attribution`, `license`, optional `fields`
(epicenter, ad/glass radii), optional `terrain` (elevation grid), optional
`water_bodies` (polygons + datum). `buildings[]`: id, block, centroid, perimeter
(world metres), height_m, floor_height_m, category, optional year_built,
optional base_elevation_m (default 0). Adapters: `scripts/city_dataset.py
manhattan|synthetic|seattle`. The pilot generator consumes only this file
(`PILOT_DATASET`), never `building_perimeters.json`.

## Optional fields exercised by Seattle S1

All of these are additive; v1.1 consumers that only read `buildings[]` are
unaffected.

```jsonc
{
  "region": {
    "name": "puget-sound-downtown-to-tacoma",
    "anchor_wgs84": [-122.3355, 47.6062],     // world (0, 0)
    "anchor_epsg2926": [1269879.86, 224737.70],
    "polygon_wgs84": [[lon, lat], ...],
    "world_frame": "x=east metres, z=south metres, y=base_elevation_m",
    "charter": "downtown focus; south toward Tacoma; east to Bellevue; ..."
  },
  "provenance": [                              // one entry per source layer
    { "layer": "seattle_buildings_2023", "service": "...", "attribution": "...", "license": "..." }
  ],
  "terrain": {
    "source": "USGS 3DEP 1/3 arc-second (n48w123, n48w122)",
    "vertical_datum": "NAVD88 (approx.)",
    "crs": "EPSG:2855",
    "spacing_m": 30.0, "rows": 2322, "cols": 1207,
    "world_origin_m": [x, z],                  // world coords of grid corner
    "path": "datasets/seattle/terrain_30m.tif",
    "belt": { "path": "...", "spacing_m": 10.0, "center_world": [x, z], "half_extent_m": 600.0 }
  },
  "water_bodies": [
    { "kind": "sound|lake|stream|canal|water", "name": "Puget Sound",
      "mtfcc": "H2051", "datum": "NAVD88 (approx., 3DEP-derived)",
      "surface_elevation_m": 0.0, "polygon": [[x, z], ...] }
  ]
}
```

Terrain grid addressing: row 0 is the north edge, row increases south; column 0
is the west edge, column increases east; cell centre is
`world_origin_m + ((col + 0.5) * spacing, (row + 0.5) * spacing)` in the local
world frame.

The pilot preview also accepts `PILOT_DATASET`, `PILOT_CENTER=x,z`,
`PILOT_BUNDLE_OUT=name.json`, `PILOT_BUNDLE=name.json` (`--pilot` runtime) and
`PILOT_CLEAN=1` (omit instrument UI for evidence captures). Manhattan bundles
do not emit a `view` block, so A.2a camera poses are unchanged.
