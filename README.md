# Manhattan — cleaned model, day/night textures, and fly-through viewer

A New York City diorama (188k vertices / 105k polygons, 7 km across) recovered
from a 2020 Blender scene, relinked to a clean texture library, given generated
night variants, exported to OBJ/FBX, validated in FreeCAD, and wrapped in a
Bevy 0.20 fly-through viewer with real sun/moon lighting, geometry-derived
city blocks, human-scale building floors, full-detail fog culling, depth of
field, and a twilight wave that lights blocks up like fluorescent tubes.

## Quick start

```bash
cd viewer
cargo run --release
# or build once and run the release binary from the viewer directory:
cargo build --release && ./target/release/manhattan-viewer
```

First launch takes a few seconds while the 130 MB city mesh and its textures
load.  A 1440×900 window opens on the overview camera.

### Viewer controls

| Input | Action |
|---|---|
| `W` `A` `S` `D` | fly horizontally |
| `Q` / `E` | descend / ascend |
| hold right mouse button | look around (drag) |
| arrow keys | look around (no mouse) |
| `Shift` | 5× boost |
| `Alt` | 0.2× precision |
| mouse wheel | change fly speed (20–8000 u/s) |
| `[` / `]` | calendar day − / + |
| `-` / `=` | clock hour − / + |
| `;` / `'` | clock 5 minutes − / + |
| `,` / `.` | calendar month − / + |
| `Space` | play / pause the clock |
| `B` | reverse the clock direction (dusk ↔ dawn) |
| `R` | reset to 2026-06-21 12:00 |
| `L` | show/hide street centreline stationing |
| `T` | show/hide the vertical 100-ft station ticks |
| `C` | show/hide the camera civil-coordinate panel |

The top-right panel has clickable buttons for the same calendar/clock actions.
The camera is hard-clamped to `y >= 3.0` (the city ground line is `y = 0`) and
swept against the building height grid: it slides along facades instead of
flying through them, and can land on a roof but never enter a building.

### Browser build / GitHub Pages

Live: <https://orbitmechanic.github.io/manhattan/>

The browser build is produced by `viewer/tools/build_web.sh`:

```bash
cd viewer
tools/build_web.sh
rm -rf ../docs && cp -R web/dist ../docs
touch ../docs/.nojekyll
```

The Pages site is served from `docs/` on `main`:

- `docs/manhattan_bg.wasm` — wasm release build (27 MB after `wasm-opt -Oz`)
- `docs/assets/models/manhattan_city.glb` — 93 MB (< GitHub Pages' 100 MB
  per-file limit), plus ground, night textures and the city shader
- `docs/index.html` — loading screen + canvas host
- URL options: `?view=street|mid|overview`, `?time=HH:MM`,
  `?lines=1`, `?ticks=1`

WebGL2 limitations handled in the build:

- only **one directional light** is supported, so the browser build uses a
  single combined sun/moon fixture (the native build keeps both);
- depth textures are not reliable, so **depth of field is disabled** on the
  web; fog, full geometry, twilight and per-floor transitions remain;
- `VisibilityRange` dithering cannot bind on WebGL2, so the web build culls
  real building geometry with distance visibility instead.

First load downloads the 93 MB city mesh; the loading screen explains this.
Desktop with mouse and keyboard is recommended.

## Screenshot / self-test modes

```bash
# fixed-time screenshot (waits for the scene + textures to load)
cargo run --release -- --shot day.png   --date 2026-06-21 --time 12:00
cargo run --release -- --shot night.png --date 2026-11-01 --time 22:00 --view street

# runtime self-test: camera clamp/movement + building collision, clock wrap,
# day/night mode, block registry, fog culling and twilight targets;
# writes JSON and exits
cargo run --release -- --selftest --report selftest_report.json
```

## What was wrong with the original files

- Every texture link in `manhattan.blend` and `manhattan.fbx` was a Windows
  absolute path such as
  `C:\Users\AIDAN_~1\OneDrive\Desktop\textures\...`.  The images were packed
  inside the `.blend`, which is why it still rendered, but the on-disk links
  were unusable.
- The huge ground plane used `GOOGLE_SAT_WM.tif` from a Blender temp folder.
  That file was not in `Textures/`; it was recovered from the packed data.
- The texture dump had 111 files with names like `images (19).jpg`,
  `download (4).jpg` and `roof5.jpg`, plus two duplicate nested folders.
- `.blend` and `.fbx` were the same model: identical geometry, object names,
  material names and material→texture assignments (verified by re-importing
  the FBX and comparing vertex/poly counts and bounds).

## Deliverables

| Path | Contents |
|---|---|
| `manhattan.blend` | clean single-mesh master: relinked images, descriptive material/object names, preview camera + sun, solid-blue day world. 11 building materials + ground. |
| `manhattan_blocks.blend` | block art master: 1223 geometry-derived block objects, 19 park empties, Voronoi ground, original source hidden. |
| `manhattan_floors.blend` | **floor art master**: 1223 block objects split into 3-unit floor bands with one-floor facade UVs, human-scale roofs, and a `floor` face attribute. |
| `manhattan.fbx` | re-exported from the clean master; textures embedded; same 188,170 verts / 105,330 polys as the blend. |
| `manhattan.obj` + `manhattan.mtl` | Z-up, triangulated OBJ for FreeCAD; MTL points at `Textures/day/...`. |
| `build/manhattan_freecad.FCStd` | native FreeCAD document produced by importing the OBJ (validated cleanly). |
| `build/blocks_report.txt` | irregular block counts, park/Voronoi details, wave measurements, runtime verification. |
| `build/floors_report.txt` | floor scale, one-floor textures, real-geometry assets, fog/focus/collision. |
| `build/stationing_report.txt` | street-centreline method, counts, data schema and camera civil coordinates. |
| `build/roadway_report.txt` | lane/sidewalk geometry method, counts and material notes. |
| `build/upper_facade_report.txt` | upper-floor facade/roof libraries, packed COLOR_0 schema and assignment counts. |
| `build/details_report.txt` | dawn timing, night-window variants, antennas, signs, trees and the 1,000 ft detail cull. |
| `build/people_report.txt` | pedestrian catalog, density, walk cycle, avoidance, homeless figures, corner ads and streaming limits. |
| `build/traffic_report.txt` | vehicle catalog, lane network, 85% occupancy, gridlock movement, collision grid and night lights. |
| `build/lighting_report.txt` | streetlamp/signal streaming, night-light atlas and the removed shadow pass. |
| `build/weather_report.txt` | weather state machine, procedural cloud/precipitation shaders and performance notes. |
| `build/blocks.json`, `blocks_labels.npy`, `ground_cells.npz` | segmentation output: block metadata, per-pixel labels, Voronoi ground polygons. |
| `viewer/assets/models/blocks.json` | block metadata embedded into the viewer binary at compile time. |
| `viewer/assets/models/manhattan_city.glb`, `manhattan_ground.glb` | the real per-block floor geometry and the ground; no LOD proxies. |
| `viewer/assets/models/collision_grid.bin` | 4 m building-height grid baked from the 1223 blocks, used for camera collision. |
| `viewer/assets/models/station_lines_imagery.json` | 2041 satellite/road-derived centrelines, 92.1 miles. |
| `viewer/assets/models/station_lines_fronts.json` | 2460 building-expansion-front centrelines, 185.5 miles. |
| `viewer/assets/models/station_lines_completed.json` | 4,445 connected, smoothed building-front + connector centrelines, 189.1 miles (default). |
| `viewer/assets/models/roadway_chunks/` | 129 x 500 m asphalt + sidewalk tiles generated from the completed baselines. |
| `viewer/assets/models/roadway_manifest.json` | chunk list with world-space AABBs for the 1 km detail cull. |
| `viewer/assets/models/building_perimeters.json` | 11,032 building first-floor wall loops, heights, floor heights and facade categories. |
| `viewer/assets/models/facade_catalog.json` | 600-image facade catalog (150 per category) with atlas tile coordinates. |
| `viewer/assets/textures/facades/` | four 4096px atlases: storefront, executive, residential, industrial (ground floor). |
| `viewer/assets/textures/upper/` | 320-template upper-floor facade atlases and 160-template roof atlases; three facade night variants. |
| `viewer/assets/textures/signage/` | 64-design abstract projecting-sign atlas. |
| `viewer/assets/textures/people/` | 8 x 8 clothing-pattern atlas for pedestrian outfits. |
| `viewer/assets/textures/vehicles/` | emissive vehicle night-light atlas (cabins, headlights, taillights, markers, emergency bars). |
| `viewer/assets/models/clothing_catalog.json` | clothing tile ranges (shirts, pants, skirts, jackets, hats, bags). |
| `viewer/assets/models/trees.json` | 3,000 park/open-ground tree placements. |
| `viewer/assets/models/traffic_lanes.json` | 24,175 lane centrelines exported by the roadway builder. |
| `viewer/assets/models/intersections.json` | 5,908 intersections with position, yaw and road half-width for signal placement. |
| `viewer/assets/models/upper_facade_catalog.json` | facade/roof atlas grids, category ranges and neighbourhood mapping. |
| `build/station_lines_imagery_overlay.png`, `station_lines_fronts_overlay.png` | validation overlays on the satellite. |
| `viewer/assets/textures/fog_bank.png` | soft mist texture for the drifting ground-fog sheets. |
| `Textures/day/` | canonical day textures, names describe content; `floors/` holds one-floor facade crops. |
| `Textures/night/` | generated night textures; `floors/` holds matching one-floor night crops. |
| `Textures/library/` | all unused source images, renamed descriptively; originals preserved in `backup/Textures_original.zip`. |
| `Textures/rename_manifest.json` | original filename → new path for all 111 source textures. |
| `viewer/` | Rust/Bevy 0.20 viewer; loads the real city + ground (12 materials, 3,862 primitives). |
| `renders/` | Blender and Bevy verification frames (day, night, street, twilight). |
| `backup/` | untouched original `.blend`, `.fbx`, and texture zip. |

### Texture library layout

```
Textures/
  day/
    facades/      10 building facade textures (facade_<style>_<descriptor>.jpg)
    ground/       satellite_manhattan_ground.jpg (recovered from packed TIF)
    roofs/        roof_concrete_gravel.jpg
    streets/      road_asphalt.jpg, sidewalk_concrete.jpg,
                  pavement_weave_roughness.jpg
    environment/  sky_clear_2k.hdr (original packed HDR, kept for reference)
  night/
    facades/      <same name>_night.jpg
    ground/       satellite_manhattan_ground_night.jpg
    roofs/        roof_concrete_gravel_night.jpg
    streets/      <street>_night.jpg
  library/        unused buildings, materials, roofs and grass/rock sets
```

The 11 building materials map to these day textures:

| Material | Day texture |
|---|---|
| Facade_Residential_Beige_0197 | facade_residential_beige_0197.jpg |
| Facade_Glass_White_0110 | facade_glass_white_0110.jpg |
| Facade_Glass_Bronze_0053 | facade_glass_bronze_0053.jpg |
| Facade_Glass_Blue_0056 | facade_glass_blue_0056.jpg |
| Facade_Glass_SkyBlue_Grid | facade_glass_skyblue_grid.jpg |
| Facade_Glass_Striped_0564 | facade_glass_striped_0564.jpg |
| Facade_Glass_Dark_Vertical | facade_glass_dark_vertical.jpg |
| Facade_Glass_Blue_Horizontal | facade_glass_blue_horizontal.jpg |
| Facade_Office_White_0607 | facade_office_white_0607.jpg |
| Facade_Masonry_Beige_Classic | facade_masonry_beige_classic.jpg |
| Roof_Concrete_Gravel | roof_concrete_gravel.jpg |
| Ground_Satellite_Manhattan | satellite_manhattan_ground.jpg |

## City-block framework

`manhattan_blocks.blend` is the editable art master.  The blocks are derived
from the building geometry, **not a grid**:

1. A 2-unit/pixel top-down building mask is rendered from `manhattan.blend`.
2. Building footprints are dilated by **8 world units** and connected
   components become building blocks.  Buildings in one block merge; buildings
   across a street do not.  **1223 building blocks.**
3. Empty space is morphologically opened by **30 world units**.  Large interior
   open components become **19 park / open-space blocks** (map-edge/water
   components are excluded).
4. The ground is partitioned into **3388 Voronoi cells** around building
   component centroids, block centroids and park seeds, clipped to the ground
   plane.  Cell boundaries fall between blocks, i.e. down the middle of streets
   and alleys.

```
City_Blocks/
  Block_0000 ...        1223 objects, one per building block (multi-material)
  Ground_Blocks         Voronoi ground; per-face block id in COLOR_0
Parks/
  Park_0000 ...         19 empties at open-space block centres
_Source_HIDDEN/
  Manhattan_Buildings   original single mesh, hidden
  Manhattan_Ground      original single plane, hidden
```

- No building faces are lost: **105,330** faces across the block objects.
- Each block object has `block_index`, `block_type`, `centroid_x/y` and
  `area_px` custom properties.
- To add block art, edit the matching `Block_XXXX` object (or add a park mesh
  with `block_index`) and re-run `scripts/build_floors.py`; it rebuilds the
  floor master and the real-geometry city mesh.  If you add/remove blocks, also
  regenerate `blocks.json` and rebuild the viewer.  New materials need a
  matching night texture.
- To edit floors directly, open `manhattan_floors.blend`, use
  `scripts/separate_block_floors.py` for per-floor objects, then re-export with
  `scripts/export_city_mesh.py`.
- `build/blocks_overlay.png` shows the building blocks over the satellite;
  `renders/blocks_ground_voronoi.png` shows the Voronoi ground partition.

### Viewer representation

The viewer renders the **real per-block floor geometry** from
`manhattan_city.glb`: 1223 block meshes / 1.53M triangles, plus one
always-visible ground mesh.  It does not merge regions, swap full-facade
textures, or draw box proxies at distance.  Every face/loop carries its block
id in **COLOR_0 alpha** (16-bit normalized, enough for 1536 ids).

`viewer/src/city_material.rs` + `assets/shaders/city_block.wesl` implement a
PBR material extension that:

- reads the block id from vertex alpha,
- reads the block's day/night blend from a 1536-entry uniform array,
- mixes the base (day) texture with a repeating night texture,
- drives the night emissive windows/street lights,
- keeps Bevy's PBR sun/moon lighting, fog, tonemapping and bloom.

### Twilight wave and randomised transition styles

`viewer/src/blocks.rs`:

- Twilight progress is a pure function of the sun's altitude:
  **+1° → 0.0**, **−6° (end of civil twilight) → 1.0**.  Because it is
  altitude-driven, it works identically for dusk, dawn and reverse time.
- Each block's switch threshold is drawn from **N(0.5, 0.18)**, producing a
  bell-curve wave of random blocks.
- **Every new penumbra** (each dusk, each dawn, including re-entry in reverse
  time) re-randomises every block's threshold **and its transition style**:
  - **Flicker** (~55%): hard on/off, fluorescent bursts of 40–120 ms for
    0.25–1.1 s, then settle.
  - **Fade** (~45%): a smooth day/night crossfade over a random 0.06–0.18
    progress window (2–7 min of twilight).
- The exact per-date twilight duration is computed from the solar ephemeris
  and shown in the UI: **38 min** on 2026-11-01, **45 min** near the June
  solstice.
- The clock defaults to **6 in-game minutes per real second**, so the ~38 min
  November twilight plays over ~6 s.  `B` / the reverse button runs the same
  wave backwards.

Measured on 2026-11-01: **79/1242** lit at 16:50 (progress 0.23),
**564/1242** at 17:00 (0.48), **1133/1242** at 17:10 (0.74), and **810/1242**
at dawn 06:15.  The 17:10 penumbra had **712 flicker / 530 fade** blocks; each
launch and each new penumbra re-randomises the mix.

Evidence: `renders/blocks_overlay.png`, `renders/blocks_ground_voronoi.png`,
`renders/viewer_twilight_early.png`, `renders/viewer_twilight.png`,
`renders/viewer_twilight_late.png`, `renders/viewer_dawn.png`.

## Building floors and human scale

`manhattan_floors.blend` is the floor art master.  Every block mesh is cut at
**3-unit floor levels**:

- **739,616** floor-band faces across 1223 block objects.
- Facade walls map `V = 0..1` inside each floor and repeat `U` around the wall
  with a **12-unit window module**.
- Each facade material still carries a **one-floor texture** (a single
  window-row crop in `Textures/day/floors/` and `Textures/night/floors/`), but
  the viewer now overrides those ten legacy crops with the 320-template
  per-building upper-floor atlas described below.
- Roof UVs are rebuilt per roof face from that face's **longest edge**.  The
  8-unit local UVs are mapped by the shader to a 16-unit roof-atlas module, so
  the garden/pool/AC grid follows the roof geometry instead of world axes.
- A FACE attribute `floor` records the floor index.  The helper
  `scripts/separate_block_floors.py` turns the active block's floor bands into
  individual `Block_XXXX_Floor_NN` objects for detailed editing; the viewer
  keeps floors merged per block for performance.
- Floor bands are shells (walls + roof where present), not watertight room
  boxes; the exterior floor-by-floor mapping and roof scale are the visual
  contract.  Caps can be added in the floor master if a watertight cut is
  needed.

### Full geometry and fog culling

There are no LOD proxies.  The viewer loads only `manhattan_city.glb` (1223
real per-block floor meshes, 1.53M triangles) and `manhattan_ground.glb`.
Nothing is merged into larger regions and no larger cubes are drawn over the
buildings.

- `VisibilityRange` culls the **real** building meshes at the fog distance:
  `cull = clamp(fog_visibility × 1.05, 2.5..24 km)`.  The ground has no range
  and is always visible.
- Because geometry is never replaced, the city cannot show the old LOD2 region
  boxes or full-facade region merges at distance.
- Every block id lives in COLOR_0 alpha, so the twilight/flicker/fade behaviour
  continues across the whole city.
- Viewer totals: **12 materials, 3,862 primitives, 3,861 visibility-range
  entities**; `manhattan_city.glb` is 93 MB / 1223 nodes.
- `--view street`, `--view mid` and `--view overview` only change the camera
  position; they all show the same full-detail geometry.

### Street centreline stationing — two sets

`scripts/build_station_lines.py` produces two independent networks so they can
be compared:

| Set | Source | Lines | Length | 100-ft ticks |
|---|---|---:|---:|---:|
| **IMAGERY** | grey road pixels in the low-resolution satellite, thinned on the free-space skeleton | 2041 | 92.1 mi | 8003 |
| **BUILDING FRONTS** | simultaneous expansion fronts from every building; the meeting locus is the road centreline | 2460 | 185.5 mi | 13513 |
| **COMPLETED** | building fronts + free-space connectors, smoothed with corner preservation; default | 4445 | 189.1 mi | 16894 |

- **IMAGERY** is filtered by road colour, a 4–30 m distance from building
  footprints, and continues straight through junctions.
- **BUILDING FRONTS** uses the Voronoi boundary of individual building
  components: two expansion fronts meet where their distances are equal, and
  that locus is used as the centreline.  The front is limited to 35 m from a
  building so harbour/open-water boundaries are not included.
- Each line has `points`, plan **tangents**, **circular curves** (radius,
  left/right, reverse flag), and a **vertical tangent profile** for future
  grade changes.  The source ground is flat, so current elevations are 0.
- Stations, offsets and elevations are in **feet**; geometry is in metres.
- Offset is **negative left / positive right** when looking up-station.
- Press `L` for centrelines, `T` for the 100-ft vertical ticks (from −3 m to
  +30 m through the ground), `C` for the civil-coordinate panel, and **`G` to
  cycle IMAGERY → BUILDING FRONTS → COMPLETED**.  The same toggles are buttons
  in the STATIONING panel.  **COMPLETED is the default set.**
- The camera computes **one civil-coordinate array per set** (`imagery`,
  `fronts`, `completed`) with `station_ft`, `offset_ft`, `elevation_ft`, nearest tangent
  station/elevation, signed station/elevation from that tangent and plan
  distance.  The panel shows the three nearest lines for the selected set; the
  full arrays live in `StationingState` for future pavement/gutter/sidewalk/
  crosswalk/signal placement.
- `build/station_lines_imagery_overlay.png` and
  `station_lines_fronts_overlay.png` show the two networks on the satellite;
  `renders/viewer_station_*_street.png` / `_mid.png` show them in the viewer,
  including the completed set.

The extraction is a planning aid, not a survey; both sets should be field-
checked before construction.

### Roadway geometry — pavement and sidewalks

`scripts/build_roadway.py` generates 500 m tiles in
`viewer/assets/models/roadway_chunks/` from the **COMPLETED** street baselines
(plus an AABB manifest), so the viewer can hide pavement beyond the 1 km
embellishment radius:

- **12-ft nominal lane width**; the remaining traffic area is divided into an
  even 2–6 lanes, with a double yellow centreline.
- **12-ft concrete sidewalks** aligned to the building faces; they stop at
  open intersections where no building is within the corridor cap.
- Asphalt is dark grey, sidewalk concrete light grey, with a 0.14 m curb face.
- Pavement extends to the measured building perimeters instead of using a
  constant ribbon width.
- 4,445 baselines split into 4,472 free street runs; only 18 runs shorter
  than 4 m are omitted.
- Pavement now fills the **full building-to-building corridor**: the actual
  clearance is measured on each side of every centreline vertex (capped at
  15 m half-width in open intersections/plazas).
- The outer **12 ft** of each side is sidewalk aligned to the building face;
  the remaining space is divided into an even number of traffic lanes
  (2–6), each close to 12 ft.  Where the clearance hits the cap there is no
  building, so the sidewalk stops and the intersection stays asphalt.
- Markings: solid white edge lines, white dashed lane dividers, and a
  **double yellow centreline** with a 0.24 m gap.
- Mitred joins keep bends crack-free; 5,908 asphalt junction patches fill
  crossings and T-junctions.
- Geometry: 104,898 asphalt triangles, 60,220 sidewalk triangles, 122,892
  white markings and 68,004 yellow markings.
- 130 tiles of 500 m (25.8 MB); each scene root gets a `DetailCull` AABB and
  is hidden once the camera is more than 1,000 m from it.
- `--view close` provides a low street-level inspection camera.
- `renders/viewer_roadway_close.png` shows the result.

### Ground-floor facade catalog and dressing

`scripts/build_facade_catalog.py` generates a 600-image library (`150` per
category) and packs it into four 4096 x 4096 JPEG atlases:

| category | count | theme |
| --- | ---: | --- |
| `storefront` | 150 | mid-rise shopfronts: awnings, sign bands, display windows |
| `executive` | 150 | glass/ornate skyscraper lobbies, piers, revolving doors |
| `residential` | 150 | row houses and apartment fronts, stoops, fire escapes |
| `industrial` | 150 | warehouses and factories, roll-up doors, corrugation |

`scripts/build_building_meta.py` reads `manhattan_floors.blend`, takes the
bottom edges of every vertical first-floor wall polygon, and chains them into
the real exterior wall loop of each building. It exports **11,032 building
loops** (not block hulls), so facades follow the building outline and do not
bridge alleys, courtyards or streets. Loops with near-collinear micro-jogs are
simplified within 0.15 m; the small number of block-sized rows are simplified
further so each building stays under the 150-image deck.

Category rules use the building height and its block's neighbourhood:
`height >= 150 m` → executive; `height < 45 m` in an industrial block →
industrial; `height < 45 m` elsewhere → residential; otherwise storefront.

At startup `viewer/src/storefront.rs` gives every building a **fixed per-run**
dressing. Each wall loop is divided into panels roughly one catalog image wide
(8 m storefront, 12 m executive, 6 m residential, 10 m industrial). Panels are
assigned from a shuffled per-building deck, so **no image repeats around a
building's perimeter**; each panel records its image id, perimeter offset,
width and scale in `StorefrontState`. The assignment is stored once and reused,
so a building looks the same for the whole session; the seed comes from the
system clock (native) or `Math.random()` (wasm), so it changes between runs.
The quads sit 0.08 m proud of the first-floor wall, use `ClampToEdge` sampling
to avoid atlas bleed, and carry a `DetailCull` so they only render inside
1,000 m.

### Upper-floor facades and roofs

`scripts/build_upper_facade_library.py` generates a second library for every
floor above the street:

| facade category | templates | theme |
| --- | ---: | --- |
| `residential` | 80 | brick, stucco and brownstone, punched windows, balconies, window AC units |
| `industrial` | 80 | corrugated metal, concrete panels, steel-framed ribbon windows, vents |
| `financial` | 80 | dark/blue glass curtain walls, stone piers, vertical fins, spandrels |
| `commercial` | 80 | masonry/stone mid-rise upper floors, large windows, cornices |

Each template covers one **12 m x 3 m floor module** and tiles seamlessly
around the building. Day and night versions share window positions. The
library is packed into a 3840 x 3072 day atlas and a 1920 x 1536 night atlas.

Each roof face's UV frame is rebuilt from its longest edge, so the texture grid
follows the roof geometry rather than the world axes; the 16 m module then
tiles within that frame.

It also generates **160 roof templates** on a 16 m x 16 m module: gardens,
planters and trees, pools with loungers, AC/HVAC units, cooling towers, ducts,
solar panels, skylights, water tanks and wooden patios with tables and
umbrellas. Day and night roof atlases are 4096 x 2560 and 2048 x 1280.

`scripts/assign_upper_facades.py` matches every city face to its building
using the first-floor wall loops, then bakes a 24-bit choice into COLOR_0 RGB:

- bits 0–8: facade template
- bits 9–16: roof template
- bits 17–19: per-building tint variation
- bits 20–23: per-building horizontal U offset
- alpha: block id (unchanged)

One template is used for **every wall face of a building, on every floor**;
one roof template is used for all of its horizontal faces. The tint and U
offset make neighbouring buildings differ even when they draw the same
template. Neighbourhood mapping is `residential → residential`,
`industrial → industrial`, `storefront → commercial`,
`executive → financial`.

`viewer/assets/shaders/city_block.wesl` decodes the packed value per fragment,
samples the facade atlas with U tiled around the building and V mapped to one
floor, and samples the roof atlas on horizontal faces. Day/night blending and
the per-floor twilight wave are unchanged. Current build: 739,453 city faces,
728,210 wall faces, 11,243 roof faces, 0 unmatched faces.

### Dawn timing and night windows

At dusk the city still fades into night during the +1° to −6° penumbra.  At
dawn it now **holds the night set for the whole penumbra** and only starts the
night-to-day switch after the sun reaches +1°.  The switch is spread over the
next 1.5° so the per-block and per-floor thresholds still stagger the change.

Night windows no longer show the same lit strip on every floor.  Three facade
night atlases are generated: two independent random lit subsets and one
all-lit variant.  Each floor picks one of the random subsets by hash.  Lit
probabilities are 28% residential, 20% industrial, 30% commercial and 42%
financial.  Tall financial buildings also give each floor a 35% chance of
using the all-lit atlas, producing random fully lit floors.

### Rooftop antennas, shop signs and trees

`viewer/src/details.rs` adds the near-detail layer.  All of it uses a
**1,000 ft (304.8 m)** cull radius; roads and facades keep the 1 km radius.

- **Antennas**: one per floor per 10,000 sq ft of floor area
  (`footprint_m2 × 10.7639 × floors / 10,000`), with a minimum of 1 and a
  safety cap of 64 per building.  Heights are randomised between **1% and 8%
  of the building's total height**.  Each mast is rooted at the baked
  collision grid's roof height for its random plan position, so setback and
  lower-wing roofs get antennas on the actual roof instead of floating.
- **Signs**: every building gets four times its antenna count.  Half are flat
  wall signs parallel to the facade and half are blade signs mounted
  perpendicular to the facade, projecting 2.4–4.0 m.  Boards are 0.9–2.6 m
  tall at the second floor.  The 64-design atlas has abstract blocky glyphs in
  random colours; roughly 25% blink, 25% are steadily illuminated, the rest are
  unlit.  Signs are no longer limited to commercial/industrial areas.
- **Trees**: 3,000 low-poly park trees in the 19 park/open-space blocks, at
  least 9 m from street centrelines and outside building footprints.  Three
  variants, random scale and rotation.

`DetailCull` now carries a per-entity radius, so the new objects disappear
past 1,000 ft while the roadway and facade dressing remain visible to 1 km.

### Pedestrians, homeless figures and corner ads

`viewer/src/people.rs` adds a streaming crowd layer.  The catalog is built
once at startup from a 64-pattern clothing atlas plus male/female mannequin
part meshes; individual outfits choose shared clothing materials.

- Walkers follow mitered tracks **1–12 ft outside** each building's first-floor
  perimeter, starting at random points and walking clockwise or
  counter-clockwise at 0.85–1.5 m/s.
- **50% male / 50% female**, **10% juvenile** at 30–80% adult scale.
- Randomized shirts, pants, skirts, jackets, hats and bags; arms and legs are
  articulated and animate a walk cycle, with body bob and basic avoidance
  (slow/stop behind and lateral passing for oncoming walkers).
- Density is **1 person per 5 ft of perimeter** nominally and surges to
  **1 person per 1 ft** around the 08:00 and 17:30 rush hours.
- **One prone homeless figure per 20 ft** of perimeter lies against the
  building and does not move.
- Only buildings within 140 m (100 m on web) are populated; crowds are
  despawned when they leave that radius.  Visible limits are 1,000 walkers /
  300 homeless native (500 / 150 web); catalogs stay in memory, individual
  accessories do not.
- **12% of signs** are now vertical banners running up building corners; the
  remaining split is 50% perpendicular blade signs and 38% flat wall signs.

### Vehicle traffic

`viewer/src/traffic.rs` streams low-poly traffic from the lane centrelines
exported with the roadway:

- **150 catalog variants**, 30 each of tractor-trailer, cargo van, SUV,
  passenger sedan and emergency (ambulance/fire/police).  Civilian colours are
  muted; emergency vehicles use white/red, fire-red and police dark/white
  schemes.
- Spawn shares: **10% tractor-trailer, 20% cargo van, 40% SUV, 20% sedan,
  10% emergency**.
- Traffic covers the same **1,000 m radius as the roadway**.  Within 250 m
  native / 180 m web, lanes use individually animated vehicles at **85%
  occupancy** (1,200 / 600 active).  Beyond that, each lane gets a merged
  far-proxy mesh of solid vehicle boxes at 85% occupancy, so every visible
  road is covered out to 1 km.  Proxies and near vehicles are despawned as
  lanes leave range.
- Vehicles align to the lane tangent (parallel to the road, perpendicular to
  adjacent building faces), are rejected if they do not fit inside the lane,
  and therefore stay off the 12-ft sidewalk tracks.  Body and light materials
  are double-sided, so each vehicle is a closed solid from any angle.
- Gridlock movement: small creep moves only when space opens, with a
  per-vehicle stop/go oscillator.  A spatial grid of AABBs prevents
  cross-lane overlaps at intersections.
- `viewer/assets/textures/vehicles/vehicle_night_atlas.png` supplies warm cabin
  windows, white headlights, red taillights, amber markers and emergency bars.
  The light meshes have per-face atlas UVs; one shared emissive material turns
  the texture on at night.  Vehicle bodies switch to an unlit vertex-coloured
  presentation, dimmed to 0.72, so each vehicle keeps its own muted paint colour
  instead of washing to a uniform grey.  Far proxies carry a roof light too.

### Streetlamps and traffic signals

`viewer/src/street_furniture.rs` streams near-field street furniture and
despawns it again as the camera moves:

- **Streetlamps** every 24 m along each building's first-floor perimeter,
  offset 2.8 m onto the sidewalk.  Each has a 6.25 m pole/arm, a lamp head and
  a shared emissive panel that turns on at night.
- **Traffic signals** at the 5,908 exported intersections, two poles per
  intersection on opposite corners.  Each has red/amber/green lenses on a
  shared 14-second cycle (6 s red, 5 s green, 3 s amber); the active lens is
  bright by day and emissive at night.
- Both streams use a **330 m native / 240 m web** radius; buildings and
  intersections outside it are despawned, so the city does not keep furniture
  for the whole map.

### Weather

`viewer/src/weather.rs` runs a small front-based weather state machine
(**clear → fair → overcast → rain → thunderstorm**, and snow in cold months)
with smooth transitions over roughly half a minute.  `V` cycles the front and
`?weather=rain` / `--weather rain` starts at a fixed one.

- **Clouds**: two camera-following planes at 1.18 km and 2.05 km use
  `assets/shaders/clouds.wesl`, a three-octave world-space noise shader.  Cover,
  wind drift, sun/sky colours and lightning flashes are plain uniforms, so the
  CPU cost is two transforms and a few material writes per frame.
- **Rain and snow**: one open cylinder around the camera uses
  `assets/shaders/precipitation.wesl` to draw wind-slanted streaks or drifting
  flakes per fragment.  It overlays the scene without depth testing, and the
  depth of field stops down while precipitation is active so the streaks stay
  readable.  A temperature curve for New York turns rain into snow below
  freezing.
- **Lightning**: storms fire isolated flashes that brighten the sky, ambient
  light and cloud shader for a few frames.
- **Integration**: cloud cover darkens the sky, attenuates the sun by up to
  ~88%, thickens the fog (visibility 1.2–22 km), hides stars and the sun/moon
  discs, and drives the HUD weather line.  No particle simulation or extra
  shadow maps are used.

### 1 km embellishment cull

`viewer/src/detail.rs` defines `EMBELLISHMENT_RADIUS = 1_000 m`. Roadway
chunks, facade meshes and station gizmos are tested against that radius every
frame (3D distance to the entity AABB, 2D distance for stationing), and hidden
outside it. The camera near plane stays at `1.0` and the far plane at
`45,000`, so close geometry is never clipped.

### Atmosphere, focus and camera collision

- **Variable haze**: `DistanceFog` uses an exponential-squared falloff whose
  visibility derives from camera altitude
  (`base = 5 km + 2.2 m per world unit of height`), night air (25% thicker),
  cloud cover, precipitation and a slow weather wave.  Visibility is clamped
  to 1.2–22 km.  The status panel shows the current fog visibility.
- **Weather**: `src/weather.rs` owns cloud cover, precipitation, wind,
  temperature and lightning; it feeds the sky/fog/light values above and
  drives the two procedural cloud planes and the shader rain/snow volume.
- **Fog sheets**: 18 soft, horizontal mist quads drift slowly over the city,
  are tinted to the current sky colour and fade with the haze.  They add
  spatial variation on top of the distance fog without hard silhouettes.
- **Focus blur**: Gaussian `DepthOfField` with a deliberately long “sensor”
  (2.5 m) so the city-scale lens has visible blur.  Focus is
  `clamp(y × 1.35 + 70, 90..3200)`, aperture opens from f/1.0 at street level
  to f/3.6 at altitude, and the circle of confusion goes from 18 px to 9 px.
  `max_depth` is the fog visibility, so blur saturates exactly where the fog
  takes over.  Rain/snow stops the lens down (up to f/17, CoC ~0.7 px) so
  precipitation streaks remain legible.
- **Camera collision**: `scripts/build_collision_grid.py` bakes a 4 m
  building-height grid (1685 × 1304 cells, 528k solid cells, 4.4 MB) from the
  block master.  Every frame the camera movement is swept against it; a wall
  impact removes the normal component and leaves the tangential motion, so the
  camera slides along the facade.  Descending over a roof rests on it, and a
  final push-out guarantees the camera never finishes inside a building.

## Night textures

`scripts/make_night_textures.py` generates every night variant from the day
image at the same resolution, so UVs stay aligned:

- **Facades** — detects window rectangles (dark regions against a blurred local
  wall estimate) and/or the repeating glazing grid, then lights a random share
  of windows with warm/cool interior colours, some curtained/dim.  Walls get a
  dark blue ambient tint.  This works for punched masonry and glass curtain
  walls.
- **Ground satellite** — darkens the daytime city, keeps water black-blue, adds
  warm lamp glows sampled along the road network and saturated neon bars near
  roads.
- **Streets** — `road_asphalt_night.jpg` has lamp pools along both kerbs plus
  neon signage and a faint reflection; the sidewalk/pavement variants get lamp
  glows.
- **Roofs** — dim blue ambient with a few warm penthouses/skylights.

Night variants exist for all 15 surface textures used by the model plus the
street set (10 facades, ground, roof, 3 streets).  The environment HDR and the
unused `library/` images are not surface textures and were not converted.

Each block blends toward the night JPEG independently when the twilight
progress crosses its own threshold; the night map drives both base colour and
emissive windows/street lights.  Blocks are either hard fluorescent flicks or
smooth fades, re-randomised each penumbra; see
[City-block framework](#city-block-framework).

## Lighting model

`viewer/src/astro.rs` implements:

- NOAA solar position for New York (40.7580 N, 73.9855 W, EST/UTC−5).
- Meeus low-precision lunar position plus illuminated fraction.
- Real date arithmetic (leap years, month lengths, day wrap).

The sun is a `DirectionalLight` whose illuminance, colour (warm near the
horizon), and rotation follow the computed altitude/azimuth.  The moon is a
second `DirectionalLight` with a cool colour, scaled by its altitude and
illuminated fraction.  Ambient light, sky colour/fog, sun/moon discs and the
star field all follow the same state.  The calendar/clock controls feed the
same `SimClock` used by the UI and tests.

## Reproducing the pipeline

```bash
# 1. texture audit / rename (one-off; originals archived first)
#    scripts/organize_textures.py is the historical run that produced the
#    current layout; do not re-run it on an already-organised folder.

# 2. generate night variants and publish them to the viewer assets
python3 scripts/make_night_textures.py
cp -r Textures/night viewer/assets/textures/

# 3. rebuild clean blend + FBX + OBJ
blender -b backup/manhattan_original.blend --python scripts/build_clean.py

# 4. fix OBJ MTL texture paths after Blender's exporter
python3 scripts/fix_mtl.py

# 5. validate the OBJ in FreeCAD headless
freecadcmd scripts/freecad_check.py

# 6. geometry-derived blocks
#    (segmentation needs scipy; this machine uses a venv at /tmp/opencode/geoenv)
blender -b manhattan.blend --python scripts/render_building_mask.py
/tmp/opencode/geoenv/bin/python scripts/segment_blocks.py
cp build/blocks.json viewer/assets/models/blocks.json
blender -b manhattan.blend --python scripts/build_irregular_blocks.py
blender -b manhattan_blocks.blend --python scripts/validate_irregular_blocks.py

# 6b. human-scale floor textures + floor master + real-geometry city mesh
python3 scripts/make_floor_textures.py
cp -r Textures/night/floors viewer/assets/textures/night/
blender -b manhattan_blocks.blend --python scripts/build_floors.py

# 6b2. atmosphere sprite + camera-collision height grid
python3 scripts/make_fog_texture.py
blender -b manhattan_blocks.blend --python scripts/build_collision_grid.py

# 6b3. street centrelines + stationing data
/tmp/opencode/geoenv/bin/python scripts/build_station_lines.py

# 6b4. asphalt + sidewalk roadway tiles (500 m)
/tmp/opencode/roadenv/bin/python scripts/build_roadway.py

# 6b5. ground-floor facade catalog (600 images, four atlases)
python3 scripts/build_facade_catalog.py

# 6b6. block footprints, heights and ground-floor categories
blender -b manhattan_floors.blend --python scripts/build_building_meta.py

# 6b7. upper-floor facade + roof library
python3 scripts/build_upper_facade_library.py

# 6b8. bake per-building upper facade/roof choices into the city mesh
blender -b manhattan_floors.blend --python scripts/assign_upper_facades.py

# 6b9. signage atlas, clothing atlas, vehicle night atlas and trees
python3 scripts/build_sign_library.py
python3 scripts/build_clothing_atlas.py
python3 scripts/build_vehicle_night_atlas.py
/tmp/opencode/geoenv/bin/python scripts/build_trees.py

# 6c. after floor-master edits, re-export the city mesh
blender -b manhattan_floors.blend --python scripts/export_city_mesh.py

# 7. tests / lint
cd viewer && cargo test && cargo clippy --all-targets -- -D warnings
```

`scripts/build_clean.py` starts from the archived original and is the
authoritative rebuild; the manual world change made after the last export only
affected presentation and is already reflected in the script.

## Verification evidence

- `renders/clean_day.png` — Blender, clean blend, all textures linked.
- `renders/clean_night_wide.png`, `renders/clean_night_street.png` — Blender
  night material preview (night textures + emissive windows).
- `renders/viewer_day.png`, `viewer_day_street.png` — Bevy daylight.
- `renders/viewer_night.png`, `viewer_night_street.png` — Bevy night with all
  blocks switched to the night material set at 22:00.
- `renders/viewer_weather_clear.png`, `viewer_weather_rain.png`,
  `viewer_weather_snow.png`, `viewer_weather_storm_night.png` — clear, rain,
  snow and thunderstorm fronts from the weather presets.
- `renders/blocks_day.png` — irregular block-master render.
- `renders/blocks_overlay.png` — geometry-derived blocks over the satellite.
- `renders/blocks_ground_voronoi.png` — mid-street Voronoi ground partition.
- `renders/floor_textures.jpg` — one-floor day/night facade crops.
- `renders/floors_facade.png`, `floors_closeup.png`, `floors_roof.png` —
  floor-band and human-scale roof verification.
- `renders/viewer_full_street.png`, `viewer_full_mid.png`,
  `viewer_full_overview.png` — the same full floor geometry from street, mid
  and overview cameras (no LOD proxies).
- `renders/viewer_station_imagery_street.png`,
  `viewer_station_fronts_street.png`,
  `viewer_station_completed_street.png`, and the matching `_mid.png` frames —
  the three centreline sets with vertical 100-ft ticks and civil coordinates.
- `renders/viewer_roadway_close.png` — asphalt pavement, 12-ft sidewalks and
  curb faces at street level.
- `renders/viewer_upper_facades.png` and
  `renders/viewer_upper_facades_night.png` — per-building upper-floor facades
  in daylight and with lit windows at 22:00.
- `renders/viewer_roofs.png` — oblique roof detail with gardens, pools, HVAC,
  tanks, solar panels and patios.
- `renders/viewer_upper_facades_web.png` — the same building dressing in the
  browser build (1440 x 900).
- `renders/viewer_trees.png` — low-poly park trees at human scale.
- `renders/viewer_people.png` — streamed pedestrians, randomized outfits,
  articulated walk poses and second-floor/corner signage.
- `renders/viewer_traffic.png` and `renders/viewer_traffic_night.png` —
  gridlocked traffic by day and cabin/headlight/taillight/marker lights at 22:00.
- `renders/viewer_streetlamps.png` — streamed streetlamps glowing along the
  sidewalks at night.
- `renders/viewer_upper_facades_night.png` — random per-floor night windows
  and all-lit financial floors.
- `renders/viewer_facade_close.png` — randomised residential ground-floor
  facade wrapped around a block corner.
- `build/station_lines_imagery_overlay.png`,
  `build/station_lines_fronts_overlay.png` — the two networks over the
  satellite.
- `renders/viewer_twilight_early.png`, `viewer_twilight.png`,
  `viewer_twilight_late.png`, `viewer_dawn.png` — the wave at 79/1242,
  564/1242, 1133/1242 and 810/1242 lit.  The street-level twilight frames also
  show individual floors switching at different times.
- `build/manhattan_freecad.FCStd` — FreeCAD import of the OBJ (259,767
  triangles, Z-up, bounds −2446…4683 × −2559…2621 × 0…410).
- `viewer/selftest_report.json` — runtime report: 1242 blocks (1223 building,
  19 park), noon targets 0, night targets 1242, randomised flicker/fade split
  (one run 702/540), 3,861 visibility-range entities, fog cull 4.3 km, focus
  blur active, collision grid loaded and resolved, stationing imagery 2041 /
  fronts 2460 / completed 4445 lines with matching camera arrays, camera clamp
  at y=3, hour-wrap date correct.
- `viewer` tests: **50 passed**, including station/offset sign projection and
  station data loading, camera collision/slide maths, one-floor texture paths,
  fog-cull distance, the actual Bevy fly camera system, calendar keys, mixed
  flicker/fade re-randomisation, Gaussian thresholds and per-date twilight
  duration.
- `viewer/selftest_report.json` additionally reports the collision grid,
  fog cull distance, fog visibility, focus blur state and stationing counts.

## Notes / caveats

- Times are local **Eastern Standard Time**; daylight-saving is ignored so the
  demo is deterministic.
- The archived originals in `backup/` are kept local only (not committed); the
  public repo ships the generated models and all source scripts.
- The viewer uses the real-geometry `manhattan_city.glb` + `manhattan_ground.glb`
  exported from the floor and block masters.  `manhattan.blend` / `.fbx` /
  `.obj` remain the single-mesh interchange deliverables for FreeCAD and other
  DCC tools.
- `manhattan.blend` uses a solid-blue world for a clean studio-day preview;
  lighting is not baked, so the viewer and Blender can light the same materials
  independently.
- The diorama's ground plane is the satellite image, so the city ends at the
  edge of the model; distance fog softens the boundary in the viewer.
- Floor bands are separate horizontal shells with a `floor` attribute, not
  watertight room boxes; use `scripts/separate_block_floors.py` for per-floor
  objects and add caps if a fully closed solid is required.
- `manhattan_city.glb` is intentionally large (93 MB / 739k faces) because it
  is the full-detail floor city.  Fog culling hides real meshes beyond the
  visibility distance; no geometry is replaced or simplified.  The ground is
  always visible and the DOF softens the far field.
- Street centrelines are heuristic: Voronoi block boundaries + satellite road
  colour + building distance.  They are a coordinate framework for future
  graphics, not a legal survey; re-run `build_station_lines.py` after block
  edits.
- The vertical profiles are flat (grade 0) because the source city ground is
  flat; the data schema already supports tangent points, grades and vertical
  curves for future elevation work.
- The segmentation is heuristic: 8-unit dilation groups buildings into blocks,
  and the 30-unit empty-space opening finds large parks/plazas.  Very narrow
  streets may merge two blocks and some wide avenues can become open-space
  blocks; individual `Block_XXXX` objects can be edited or split later without
  changing the viewer (block ids are baked per face, not by name).
- The segmentation script requires scipy/numpy/Pillow; the generated block
  data is committed under `build/` so the art master and viewer can be rebuilt
  without re-running scipy.
- FreeCAD imports the OBJ successfully (259,767 triangles, Z-up).  When the
  saved FCStd is re-opened, FreeCAD prints a benign "mesh data structure has
  some defects" notice because the city is a set of open building shells
  (94,085 boundary edges, zero non-manifold multi-face edges, zero degenerate
  or duplicate faces).  This does not affect display or the import; closing
  every shell would add geometry that was never in the source model.
