# Manhattan viewer (Bevy 0.20)

```bash
cargo run --release
# or build once and run the release binary from this directory:
cargo build --release && ./target/release/manhattan-viewer
```

The viewer loads two real-geometry GLBs generated from
`../manhattan_floors.blend` and `../manhattan_blocks.blend`:

- `manhattan_city.glb` — 1223 per-block 3-unit floor meshes, one-floor textures
- `manhattan_ground.glb` — always-visible satellite ground

There are no merged-region, reduced-facade or box-proxy LOD meshes.

There are 1242 geometry-derived blocks (1223 building clusters + 19 parks).
Block ids are baked into vertex COLOR_0 alpha; `assets/models/blocks.json`
contains the block count/metadata.  `assets/textures/night/` holds the night
variants, including `floors/` for the building floors.

## Controls

- **Fly**: `WASD`, `Q`/`E` down/up, hold **right mouse button** to look,
  arrow keys also look, `Shift` boost, `Alt` precision, wheel changes speed.
- **Camera floor**: the camera is clamped to `y >= 3.0`, ground is `y = 0`.
- **Collision**: movement is swept against a baked 4 m building-height grid.
  Walls remove the normal component and slide the camera along the facade; the
  camera can rest on a roof but never enter a building.
- **Calendar/clock**: `[`/`]` day, `-`/`=` hour, `;`/`'` 5 minutes,
  `,`/`.` month, `Space` play/pause, `B` reverse time, `R` reset.  The
  top-right panel has clickable buttons for all of these, including reverse.
- **Views**: `--view street`, `--view mid`, `--view overview`, `--view close`
  only move the camera; all render the same full-detail floor geometry.
- **Stationing**: `L` centrelines, `T` 100-ft vertical ticks, `C` civil
  coordinates, `G` switches IMAGERY ↔ BUILDING FRONTS.  The same toggles are
  buttons in the STATIONING panel.  `--lines`, `--ticks`, `--set fronts` and
  web `?lines=1&ticks=1&set=fronts` start with them enabled.
- **Weather**: `V` cycles clear / fair / overcast / rain / thunderstorm (snow
  replaces rain in cold months).  `--weather rain` or web `?weather=rain`
  starts at a fixed front; the HUD shows cloud cover, precipitation, wind and
  temperature.
- **Gamepad**: left stick moves, right stick looks, right trigger boosts, left
  trigger descends, South (A) ascends, and D-pad up/down changes fly speed.
  A connected pad is additive with keyboard/mouse.

## Day/night

`src/astro.rs` computes the NOAA solar position and a Meeus lunar position for
New York (EST).  `src/environment.rs` turns those into:

- a sun `DirectionalLight` (illuminance/colour/rotation)
- a moon `DirectionalLight` (cool colour, moonlight scaled by altitude × phase)
- ambient light, sky/fog colour, sun/moon discs, stars

`src/blocks.rs` owns the city lighting response:

- Twilight progress is a function of the sun's altitude: **+1° = 0.0**,
  **−6° = 1.0** (civil twilight).  Being altitude-driven makes dusk, dawn and
  reverse time automatic and symmetric.
- Each block's switch threshold is drawn from N(0.5, 0.18), so blocks turn on
  in a bell-curve wave.
- **Each new penumbra re-randomises every block's threshold and style.**
  ~55% of blocks are **Flicker** (fluorescent 40–120 ms toggles for 0.25–1.1 s,
  then settle); ~45% are **Fade** (smooth crossfade over a random 0.06–0.18
  progress window).  Dusk, dawn and reverse time all trigger fresh styles.
- The real per-date twilight duration (+1° to −6°) is computed from the solar
  ephemeris and shown in the status panel (e.g. 38 min on 2026-11-01, 45 min
  near the June solstice).  The clock defaults to 6 in-game minutes per real
  second, so that twilight wave plays over ~6 s; `B` reverses it for dawn.
- `src/city_material.rs` + `assets/shaders/city_block.wesl` read the block id
  from COLOR_0 alpha, mix the day base texture with the night texture and add
  the night emissive map.  The result keeps PBR lighting, fog, tonemapping and
  bloom.
- The shader derives a per-floor threshold/width pair from the block id, the
  floor index (`world_y / 3`) and the penumbra salt.  Each floor crosses from
  day to night at its own randomised time while the block blends, so a
  transitioning block shows some floors already lit and others still dark.
  The ground disables the per-floor variation with a material uniform.

### Floors, full geometry and fog culling

- `manhattan_floors.blend` cuts each block into 3-unit floor bands.  Walls use
  V=0..1 per floor and a 12-unit U module; the shader overrides the ten legacy
  one-floor crops with the 320-template upper-floor atlas.  Roof UVs are
  rebuilt per roof face from its longest edge (8-unit local UVs), and the
  shader maps them to the 16-unit roof-atlas module.
- `VisibilityRange` culls the **real** block meshes at
  `clamp(fog_visibility*1.05, 2.5..24 km)`.  There are no merged regions or
  box proxies: geometry is never modified for rendering speed.  Ground has no
  range.
- 12 materials, 3,862 primitives, 3,861 visibility-range entities;
  `manhattan_city.glb` is 93 MB / 1223 nodes, ground 1.2 MB.

### Stationing — two selectable sets

Two JSON assets are generated by `../scripts/build_station_lines.py`:

- `station_lines_imagery.json` — 2041 lines / 92.1 miles, from grey road
  pixels in the low-resolution satellite (free-space skeleton, 4–30 m from
  buildings).
- `station_lines_fronts.json` — 2460 lines / 185.5 miles, from the meeting
  locus of simultaneous building-expansion fronts.
- `station_lines_completed.json` — 4445 lines / 189.1 miles: the fronts set
  connected across every meaningful component with free-space connectors.
  This is the default set.

Each line stores 100-ft tick positions, plan tangents, circular/reverse curves
(radius, direction) and a vertical tangent profile (currently grade 0 because
the model ground is flat).  Points are already in viewer coordinates.

`src/stationing.rs` draws the selected set as gizmos and keeps one compact
nearest-coordinate array for the active set in
`StationingState.imagery` / `.fronts` / `.completed`: station, signed offset
(negative left / positive right up-station), camera elevation relative to the
line, nearest vertical tangent station/elevation, signed station/elevation from
that tangent, and plan distance.  Up to eight entries are retained and the
panel shows the three nearest lines; the array refreshes ~4×/s while moving and
0.2×/s when stationary.  Switching sets recomputes immediately.  Press `G`
(or the **set [G]** button) to cycle IMAGERY → BUILDING FRONTS → COMPLETED.

### Roadway geometry

`assets/models/roadway_chunks/` is generated from the completed centreline set
by `../scripts/build_roadway.py`:

- Pavement fills the full building-to-building corridor, measured at every
  centreline vertex and capped at 15 m half-width in open intersections.
- 12-ft concrete sidewalks aligned to the building perimeters, with a 0.14 m
  curb face; sidewalks stop at open intersections.
- Traffic lanes split the remaining space into an even 2–6 lanes close to
  12 ft, with a double yellow centreline, dashed white lane dividers and
  solid white edge lines.
- 130 x 500 m tiles; `roadway_manifest.json` supplies each tile's AABB.
- Mitred joins and 5,908 junction patches.
- `--view close` is a low street-level inspection camera.

### Ground-floor facades

`assets/models/facade_catalog.json` describes four 4096px atlases
(`assets/textures/facades/`) with 150 images each: storefront, executive,
residential and industrial. `assets/models/building_perimeters.json` carries
11,032 building first-floor wall loops, heights, floor heights and categories,
extracted from the actual `Floor_Block_*` ground-floor wall polygons.

`src/storefront.rs` panelises each wall loop at roughly one image width.
Panels draw from a shuffled per-building deck, so no image repeats around a
building's perimeter. Each panel stores its image id, perimeter offset, width
and scale in `StorefrontState`. The seed changes per run but the assignment is
stable within a run.

### Upper-floor facades and roofs

`assets/textures/upper/` contains 320 one-floor facade templates (80 each:
residential, industrial, financial, commercial) and 160 roof templates
(gardens, pools, AC/HVAC, cooling towers, solar panels, tanks, patios).
Facade templates cover a 12 m x 3 m floor module; roof templates cover
16 m x 16 m.

`scripts/assign_upper_facades.py` packs a per-building facade tile, roof tile,
tint and U offset into COLOR_0 RGB (alpha keeps the block id). `city_block.wesl`
decodes it, so every wall face of a building uses the same facade template on
every floor, and every horizontal face uses the same roof template. The
category mapping is residential → residential, industrial → industrial,
storefront → commercial, executive → financial.

### Dawn timing, night windows and near detail

At dawn the night texture set is held for the whole +1°..−6° penumbra; the
night-to-day switch begins at +1° and completes over the next 1.5°.  Night
facades use two random lit-window atlases selected per floor, plus an all-lit
atlas used on random floors of tall financial buildings.

`src/details.rs` adds volume-based rooftop masts (one per floor per 10,000
sq ft, heights 1–8% of the building, rooted on the collision-grid roof),
four-times-density shop signs on every building (half flat to the facade,
half perpendicular blade signs) and 3,000 park trees.  These use a
**1,000 ft (304.8 m)** `DetailCull` radius; roads and facades keep the 1 km
radius.

### Pedestrians and homeless figures

`src/people.rs` builds a mannequin/clothing catalog once, then streams
pedestrians and prone homeless figures around buildings within 140 m
(100 m on web).  Walkers follow tracks offset 1–12 ft from the first-floor
perimeter, are 50/50 male/female with 10% juveniles, animate an articulated
walk cycle with avoidance, and are dressed from the shared clothing atlas.
Density is 1 per 5 ft nominally and 1 per ft at the morning/evening rush
peaks; one homeless figure per 20 ft lies against the perimeter.  Crowds are
despawned when a building leaves the near radius.  Twelve percent of signs are
vertical corner banners.

### Vehicle traffic

`src/traffic.rs` streams 150 low-poly vehicle variants (30 each of
tractor-trailer, cargo van, SUV, sedan and emergency) from the lane centrelines
in `assets/models/traffic_lanes.json`.  Traffic covers the full 1,000 m roadway
radius: individually animated vehicles within **150 m native / 100 m web** and
merged solid far-proxy boxes beyond, all at 85% occupancy (max 600 / 300 near
vehicles).  Individual vehicle light meshes are hidden past 130 m native /
90 m web; proxy lights remain.  Vehicles are double-sided solids, align to the
lane, keep clear of sidewalks, creep forward unevenly in gridlock, and use an
AABB grid to avoid cross-lane overlaps.  A shared emissive night-light atlas
(cabin windows, headlights, taillights, amber markers and emergency bars) turns
on at night.  Body paint is preserved at night with an unlit, dimmed
vertex-colour presentation rather than a uniform grey emissive.

Building shadow maps are disabled.  `src/street_furniture.rs` streams
streetlamps along building sidewalks and traffic signals at exported
intersections within a 330 m native / 240 m web radius; both turn on at night
and are despawned as the camera moves.

### Weather

`src/weather.rs` owns a front-based state machine (clear, fair, overcast, rain,
thunderstorm, snow) with ~30 s transitions, a seasonal/diurnal temperature
curve, wind and lightning.  `V` cycles the front.

- **Clouds**: two camera-following planes at 1.18 km and 2.05 km use
  `assets/shaders/clouds.wesl`.  The shader evaluates three octaves of
  world-space value noise; cover, wind, sky colour, sunlit/shadow colour and
  lightning are uniforms, so the two layers cost only a few material writes.
- **Precipitation**: one open cylinder around the camera uses
  `assets/shaders/precipitation.wesl` to generate wind-slanted rain streaks or
  drifting snow flakes per fragment.  Rain combines two rotated, jittered
  world-space layers with a slow fall rate to avoid the old raster-grid look.
  It is a lens-style overlay (no depth test/write, no prepass) and is hidden
  above 750 m.  Below freezing the rain automatically becomes snow.
- **Lightning**: storm fronts produce short flashes that add to the sky,
  ambient light and cloud colour, plus a jagged gizmo bolt drawn from the cloud
  deck to a building rooftop in the forward view; thunder uses that same roof
  position.
- **Atmosphere**: cloud cover replaces the clear-sky gradient with a grey deck,
  attenuates the sun, thickens the fog (1.2–22 km visibility), hides stars and
  the sun/moon discs, and shifts the ambient fill.  Rain/snow stops the camera
  lens down so the streaks remain readable.
- No particles, shadow maps or extra render passes are used; the whole system
  is two cloud planes, one precipitation mesh and three small custom shaders.

### Runtime throttling

- City block uniforms and road tints are change-driven; steady day/night
  frames do not rewrite material assets.
- Detail and web building culling are throttled (6.7 Hz moving / 5 s idle for
  details) and only write `Visibility` when the result changes.
- Stationing keeps the nearest eight coordinates for the active set only and
  skips work when the panel is hidden.
- Traffic near radius is 150 m native / 100 m web, with individual vehicle
  lights hidden beyond 130 m / 90 m.
- Pedestrian avoidance uses a sorted ±2 m station window (max 32 checks per
  walker) instead of all-pairs comparison.
- Fog sheets are reduced to 8 quads and their shared material updates only on
  a tint/strength change.
- Vehicle lights, traffic signals and blinking signs only update their shared
  materials on a state change.
- Traffic/people/street-furniture streaming is staggered at 0.5 / 0.65 / 0.85 s
  with spawn/despawn budgets, removing the old simultaneous twice-per-second
  boundary burst.
- Above 520 u/s the streamed accessories are hidden and paused, and a radial
  speed-line shader is shown; they resume below 300 u/s.
- The web build omits the DOF pass; native keeps the cinematic blur.

### Audio

`src/audio.rs` loads synthesized OGG loops from `assets/sounds/` (see
`../scripts/generate_audio.py`).  Ambient city, wind and rain are stereo loops
around the listener; four traffic loops follow the nearest vehicles and use
spatial panning; distant horns and thunder are spatial one-shots.  Wind volume
tracks camera speed, traffic volume tracks proximity, rain volume tracks
precipitation, and thunder is delayed by distance / 343 after a lightning
flash.  Native starts immediately; web audio unlocks on the first key/click.

### 1 km embellishment cull

`src/detail.rs` (`EMBELLISHMENT_RADIUS = 1_000 m`) hides roadway tiles, facade
meshes and station gizmos beyond 1 km from the camera. The camera near/far
planes stay at `1.0` / `45,000`.

### Atmosphere, focus and collision

- `DistanceFog` is exponential-squared, with visibility from camera altitude
  (`5 km + 2.2 m per unit`), night air (25% denser), cloud cover, precipitation
  and a slow weather wave, clamped to 1.2–22 km.  The HUD shows the current
  visibility.
- 8 horizontal, soft mist quads drift over the city; they are tinted to the
  sky and fade with the haze.
- Gaussian `DepthOfField` uses a 2.5 m sensor (a cinematic city-scale cheat),
  focus `clamp(y*1.35 + 70, 90..3200)`, aperture f/1.0→f/3.6 and CoC 18→9 px.
  `max_depth` equals fog visibility, so the blur saturates at the fog cutoff.
  The web build omits the DOF pass.
- `collision_grid.bin` (1685×1304 cells, 4 m, 528k solid cells) is baked from
  the block master by `../scripts/build_collision_grid.py`.  `src/collision.rs`
  sweeps camera motion and slides along impact normals; `push_out` is the
  final guarantee that the camera never finishes inside a building.

Night textures are loaded with a repeating sampler because facade UVs are in
tile units well outside 0..1 — this is what makes the window pattern tile across
each building correctly.

## Useful commands

```bash
cargo test                                      # 45 tests, no GPU needed
cargo clippy --all-targets -- -D warnings
cargo run --release -- --shot out.png --date 2026-11-01 --time 22:00 --view street
cargo run --release -- --selftest --report selftest_report.json
```

`--view street` starts at street level; the default `overview` starts above the
city.  `--shot` waits 600 frames (~10 s) so the GLB, textures and shaders are
ready, captures the primary window and exits.

## Layout

```
src/main.rs         app setup, CLI, capture/self-test modes
src/astro.rs        date arithmetic + solar/lunar ephemeris (unit tests)
src/clock.rs        SimClock resource + calendar actions (unit tests)
src/camera.rs       FlyCamera, ground clamp, collision-aware movement (tests)
src/collision.rs    baked height grid, swept collision + wall sliding (tests)
src/environment.rs  lighting, sky, fog, focus, sun/moon discs, stars
src/weather.rs      fronts, clouds, precipitation, wind, lightning
src/stationing.rs   centrelines, 100-ft ticks, station/offset/elevation array
src/blocks.rs       block metadata, twilight wave, mixed flicker/fade (tests)
src/city_material.rs custom PBR extension + fog culling + per-floor uniforms
src/ui.rs           status panel, buttons, key handler (unit tests)
```
