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
# or use the prebuilt release binary (assets/ sits beside it):
./manhattan-viewer
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
- URL options: `?view=street|mid|overview` and `?time=HH:MM`

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
| `build/blocks.json`, `blocks_labels.npy`, `ground_cells.npz` | segmentation output: block metadata, per-pixel labels, Voronoi ground polygons. |
| `viewer/assets/models/blocks.json` | block metadata embedded into the viewer binary at compile time. |
| `viewer/assets/models/manhattan_city.glb`, `manhattan_ground.glb` | the real per-block floor geometry and the ground; no LOD proxies. |
| `viewer/assets/models/collision_grid.bin` | 4 m building-height grid baked from the 1223 blocks, used for camera collision. |
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
- Each facade material has a **one-floor texture**: a single window-row crop of
  the original day/night facade, in `Textures/day/floors/` and
  `Textures/night/floors/`.
- Roofs use the roof texture at a **world-space 8-unit tile**, so the gravel /
  concrete scale is human.
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
  entities**; `manhattan_city.glb` is 128 MB / 1223 nodes.
- `--view street`, `--view mid` and `--view overview` only change the camera
  position; they all show the same full-detail geometry.

### Atmosphere, focus and camera collision

- **Variable haze**: `DistanceFog` uses an exponential-squared falloff whose
  visibility derives from camera altitude
  (`base = 6 km + 3 m per world unit of height`), night air (25% thicker) and a
  slow weather wave.  Visibility is clamped to 2.5–24 km.  The status panel
  shows the current fog visibility.
- **Fog sheets**: 18 soft, horizontal mist quads drift slowly over the city,
  are tinted to the current sky colour and fade with the haze.  They add
  spatial variation on top of the distance fog without hard silhouettes.
- **Focus blur**: Gaussian `DepthOfField` with a deliberately long “sensor”
  (2.5 m) so the city-scale lens has visible blur.  Focus is
  `clamp(y × 1.35 + 70, 90..3200)`, aperture opens from f/1.0 at street level
  to f/3.6 at altitude, and the circle of confusion goes from 18 px to 9 px.
  `max_depth` is the fog visibility, so blur saturates exactly where the fog
  takes over.
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
- `renders/blocks_day.png` — irregular block-master render.
- `renders/blocks_overlay.png` — geometry-derived blocks over the satellite.
- `renders/blocks_ground_voronoi.png` — mid-street Voronoi ground partition.
- `renders/floor_textures.jpg` — one-floor day/night facade crops.
- `renders/floors_facade.png`, `floors_closeup.png`, `floors_roof.png` —
  floor-band and human-scale roof verification.
- `renders/viewer_full_street.png`, `viewer_full_mid.png`,
  `viewer_full_overview.png` — the same full floor geometry from street, mid
  and overview cameras (no LOD proxies).
- `renders/viewer_twilight_early.png`, `viewer_twilight.png`,
  `viewer_twilight_late.png`, `viewer_dawn.png` — the wave at 79/1242,
  564/1242, 1133/1242 and 810/1242 lit.  The street-level twilight frames also
  show individual floors switching at different times.
- `build/manhattan_freecad.FCStd` — FreeCAD import of the OBJ (259,767
  triangles, Z-up, bounds −2446…4683 × −2559…2621 × 0…410).
- `viewer/selftest_report.json` — runtime report: 1242 blocks (1223 building,
  19 park), noon targets 0, night targets 1242, randomised flicker/fade split
  (one run 702/540), 3,861 visibility-range entities, fog cull 4.3 km, focus
  blur active, collision grid loaded and resolved, camera clamp at y=3,
  hour-wrap date correct.
- `viewer` tests: **42 passed**, including the camera collision/slide maths,
  one-floor texture paths, fog-cull distance, the actual Bevy fly camera
  system, calendar keys, mixed flicker/fade re-randomisation, Gaussian
  thresholds and per-date twilight duration.
- `viewer/selftest_report.json` additionally reports the collision grid,
  fog cull distance, fog visibility and focus blur state.

## Notes / caveats

- Times are local **Eastern Standard Time**; daylight-saving is ignored so the
  demo is deterministic.
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
- `manhattan_city.glb` is intentionally large (128 MB / 739k faces) because it
  is the full-detail floor city.  Fog culling hides real meshes beyond the
  visibility distance; no geometry is replaced or simplified.  The ground is
  always visible and the DOF softens the far field.
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
