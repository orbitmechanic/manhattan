# Manhattan ↔ destructibles integration (iteration 17)

The destructible subsystem is **consumed from its source of truth**, pinned to
the private repository tag `manhattan-2`:

```toml
destructibles-bevy = { git = "https://github.com/orbitmechanic/destructibles", tag = "manhattan-2" }
destructibles-core = { git = "https://github.com/orbitmechanic/destructibles", tag = "manhattan-2" }
```

This is a private-git dependency rather than a sibling path: a standalone
`git clone` of Manhattan can build with the operator's git credentials and does
not require a checkout of destructibles. The destructibles assets (textures and
sounds) are copied under `viewer/assets/` and the attribution is in
`viewer/assets/DESTRUCTIBLES_ATTRIBUTIONS.md`.

## Feeder: Manhattan record → `BuildingSpec`

`viewer/src/neighborhood.rs::spec_for` reads
`assets/models/building_perimeters.json`:

| Manhattan field | `BuildingSpec` |
|---|---|
| `perimeter` (first-floor wall loop) | exact footprint, translated to the record centroid |
| `height_m` + `floor_height_m` | `floor_count` rounded, `floor_height = height / count` (total height exact; neighborhood LOD caps the count and raises it only as the 12 m validation ceiling requires) |
| `category` | facade style (below) |
| `centroid` | `Transform` translation; no rotation |

Style mapping:

| Manhattan category | Style |
|---|---|
| `executive`, height ≥ 90 m | `GlassCurtainWall` |
| `executive`, lower | `ConcreteGrid` |
| `residential`, height ≤ 18 m | `BrickBrownstone` |
| `residential`, taller | `BrickMidrise` |
| `storefront` | `BrickMidrise` |
| `industrial` | `ConcreteGrid` |
| unknown | `BrickMidrise` (safe default) |

The gate is self-checking: after `swap_in_lod`, `footprint_matches` requires
every record vertex to have a generated vertex within **0.02 m**, absolute area
within **0.1 %**, and height within **0.05 m**. Every successful swap logs
`footprint PASS`; a failure swaps out immediately and leaves the plain map
building.

## Ownership per phase (no doubled draw)

1. **Plain:** Manhattan's merged block mesh is drawn normally. The live mask is
   empty.
2. **Live:** a `DestructibleHandle` owns the building. The live set is
   rasterized into a 1024² R8 world-XZ texture; `city_block.wesl` discards
   Manhattan fragments inside any live footprint. The destructible walls,
   trim, roof, panes and lit windows draw in their place. No Manhattan snapshot
   is spawned.
3. **Swap-out:** `swap_out` evicts the live entities and the mask footprint is
   cleared; Manhattan's original geometry is visible again on the next frame.

The mask is regenerated only when the live set changes or the camera moves more
than 15 % of its 1 200 m window. The mini-map white fill is the same live set,
so the two readouts cannot disagree.

## Camera-relative zone

`NeighborhoodConfig` (panel/constant tunable):

- enter: **120 m** from the camera nadir, or a **120 m** corridor ahead within
  **60 m** lateral; exit: **150 m** (25 % hysteresis).
- damaged persistence is a policy stub (`damaged_persists`); this iteration has
  no damage path, so it never fires.
- one swap per frame, 6 ms budget; plain geometry is always a valid resting
  state.
- a flying block (destructibles `Lift`) is irrelevant here; Manhattan only
  swaps pristine buildings.

The destructibles `HotZone` remains the vocabulary/source of the hot/damaged
model, but Manhattan's `Streaming` controller is disabled: the neighborhood
policy is the only swap controller.

## Mini-map debug layer

`LiveFootprints` is a permanent resource. `N` toggles the layer. Live
footprints are filled **white** while pristine; once a damage path exists the
fill becomes the established green→red halo palette, derived from the journal
and `HealthIndex` every update (never a stale classification). The layer is
committed as a permanent debug feature, not a one-off capture.

## Flight gauntlet

```bash
cd viewer
MANHATTAN_NEIGHBORHOOD=1 ./target/release/manhattan-viewer
```

The camera traverses 300 m of the densest block at 15, 30, 100 and 650 m/s;
each regime settles, traverses, captures, and force-clears all live handles
(`stray=0`). The final stationary hover measures steady-state hysteresis
(`swaps=0`). Captures land in `viewer/captures/iteration17_*.png`.

### Performance (release, fullscreen, latest run)

| Regime | Speed | Frames | Worst frame | Worst swap | Worst swap frame | Live max | Swaps in/out | Pending peak | Behind max |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| pedestrian drone | 15 m/s | 542 | 96.4 ms | 12.0 ms | 12.0 ms | 73 | 109 / 36 | 3 | 143.6 m |
| fast drone | 30 m/s | 249 | 74.2 ms | 9.9 ms | 11.4 ms | 74 | 219 / 72 | 6 | 144.1 m |
| hero flight | 100 m/s | 52 | 110.3 ms | 7.0 ms | 10.7 ms | 68 | 323 / 108 | 33 | 145.6 m |
| mach 1.92 whip | 650 m/s | 11 | 74.0 ms | 6.9 ms | 8.6 ms | 23 | 357 / 141 | 95 | 167.6 m |
| boundary hover | 0 m/s | — | — | — | — | — | 0 during measurement | — | — |

**Attribution:** the swap machinery is **under the 30 ms gate** in every
regime (worst swap frame 8.6–12.0 ms). The total worst frame (74–110 ms) is
city streaming and the screenshot request, not swap work; the table reports it
honestly. The captures themselves are a deliberate 1-frame screenshot cost.

**Saturation:** the 150 m zone keeps up at 15–30 m/s (pending ≤ 6). At 100 m/s
the pending queue grows to 33 and the live set trails to the zone edge; at
650 m/s the queue reaches **95** and the live set trails to **168 m** (18 m
beyond the 150 m exit edge) while the camera outruns it. Plain Manhattan
geometry remains on screen; no half-swapped state, doubled draw or orphaned
handle was observed (`stray=0` every regime). The practical ceiling of the
one-swap-per-frame, shell-detail pipeline is roughly **30–100 m/s**; the mach
1.92 camera needs either a larger pre-warmed pool or a budgeted incremental
generator next.

## Verification

- Manhattan: fmt, clippy `-D warnings`, **73 tests**, release build.
- Destructibles at `manhattan-2`: fmt, clippy `-D warnings`, **155 tests**,
  iteration-15/16 audit and behavior unchanged (defaults are still Full/demo).
- Destructibles audit smoke test at `manhattan-2`: `CMD AUDIT 28/28`,
  `PUNCTURE AUDIT 11/11`, `LIFT AUDIT 10/10`.
- Destructibles full autodemo at `manhattan-2` (same git tree): determinism
  PASS, collapse replay 335 entries PASS, triple restoration PASS, regime
  matrix 4/4, leftover sweep 5/5, epilogue transforms PASS. Stress 32.5 fps /
  144.3 ms is the inherited gate miss; logged in
  `viewer/captures/iteration17_destructibles_full.log`.
- Footprint, style mapping, hysteresis and pristine-white tests are in
  `neighborhood.rs`.
- Zero stray handles after every regime; hover swaps = 0.

## Out of scope / deferred

No damage delivery from Manhattan gameplay yet; damaged persistence across
zone exit is a policy stub. Live buildings use the shell-detail LOD (no
interior lattice/colliders) because a full-detail synchronous spawn was
40–90 ms; upgrading to Full on first damage is the next integration step.
Storefront/advertisement decals and the ground-floor detail question remain
deferred. Multiplayer: never.


## Iteration 18 — detail on demand, damage delivery, persistence, street parity

### Detail tiers (destructibles `manhattan-3`)

A live building enters as **Tier 0** (shell + a convex footprint collider, so a
building is never walkthrough while detail is pending). Manhattan drives the
generator every frame against a **4 ms** budget (`detail_tick`):

- **Tier 1** — slabs, columns and per-pane wall colliders replace the coarse
  collider.
- **Tier 2** — core shaft, interior shell, fill light and clutter.

Ordering is `(tier, corridor, distance)`: corridor buildings (ahead of the
camera inside `ahead_half_width * 1.5`) are detailed before nadir edges, which
pre-warms a high-speed approach. No synchronous full spawn exists anywhere.
The HUD shows per-target `DetailLevel`, live `T0/T1/T2` counts, built totals,
frozen and damage counters, and parity panel/ad counts.

Measured Tier-1 readiness on the gauntlet (distance at which Tier 1 finished
while approaching):

| Regime | Tier 1 built | Farthest Tier-1-ready approach |
|---|---:|---:|
| pedestrian drone | 49 | 133.5 m |
| fast drone | 35 | 91.8 m |
| hero flight | 7 | 91.5 m |
| mach 1.92 | 0 | — (camera outruns the generator) |

### Damage delivery

`damage_input` casts the reticle against live AABBs (with a forward-cone
fallback) and routes the established arsenal through `ImpactSpec`:

| Key | Preset |
|---|---|
| `1` | bullet (chip) |
| `2` | car bomb (5 kg TNT, 1 500 kg) |
| `3` | tier-3 (120 t, 120 m/s) |
| `4` | hero punch (`apply_puncture`, 620 m/s) |
| `X` | fire at the reticle |
| `Z` | restore the target |
| `Shift+Z` | restore damaged live buildings within 250 m |

Damage is admitted only when the target has reached **Tier 1**; otherwise the
shot is queued and applied on the frame the tier lands (never a hitch). The
journal, halo, power rolls and audio inherit by construction. Restoration uses
the host-controlled `begin_restoration`/`advance_restoration` API; the demo sets
a host rate so a car-bombed mid-rise restores in ~1.3 s.

### Damaged persistence

`persistence_tick` freezes a damaged live record beyond the exit radius
(`freeze_building`, far marker + host smoke quads beyond 250 m) and thaws it in
place on re-entry; the live record is never demoted to the plain map while the
journal is non-empty (`damaged_persists`). The minimap fill is re-derived every
frame from journal presence and `HealthIndex`, so it stays colored at all
ranges. When restoration empties the journal the record is released, the far
marker/smoke are despawned, the fill returns to white, and swap-out becomes
eligible again. The zone-exit sweep (`update_zone` second pass) and
`process_swaps` both honour damaged persistence.

### Street-level parity

**Documented choice: re-projected quads parented to the destructible root**
(not baked into facade materials). The plain map hides its `Storefront_*` and
`Signs_*` meshes (new `HiddenByLive` marker; the culler skips them) while the
live destructible carries the identical ground-floor panels and second-floor ad
plates, built from the same atlas, UVs and corner geometry. Each panel/plate
gets a `FacadeDressing { facade, floor }`; breached or removed wall cells hide
it and restoration brings it back because visibility is re-derived from the live
damage state every frame. Storefronts attach to the ground face, ad plates keep
their own floor.

### Gauntlet with damage active (`MANHATTAN_DAMAGE=1`)

```bash
cd viewer
MANHATTAN_NEIGHBORHOOD=1 MANHATTAN_DAMAGE=1 ./target/release/manhattan-viewer
```

A car bomb is queued once per regime at 35 % of the traverse (and again at the
street-level sequence). Worst frames include live damage and parity draw.

| Regime | Speed | I17 worst | I18 worst | Δ | I17 swap | I18 swap | I17 behind | I18 behind |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| pedestrian drone | 15 m/s | 96.4 ms | 156.7 ms | +60.3 | 12.0 ms | 15.8 ms | 143.6 m | 226.7 m |
| fast drone | 30 m/s | 74.2 ms | 126.7 ms | +52.5 | 9.9 ms | 19.3 ms | 144.1 m | 228.7 m |
| hero flight | 100 m/s | 110.3 ms | 124.4 ms | +14.1 | 7.0 ms | 22.6 ms | 145.6 m | 229.9 m |
| mach 1.92 | 650 m/s | 74.0 ms | 103.8 ms | +29.8 | 6.9 ms | 15.6 ms | 167.6 m | 253.5 m |

The `behind` rise is expected: a damaged frozen building remains a live record
beyond the zone, so the metric now includes persistence. Total worst frames
still belong to Manhattan city streaming plus the screenshot request; the
incremental generator has retired the 40–90 ms full-spawn cliff (Tier 0 swap is
a shell, Tier 1/2 arrive over budgeted frames). `stray=0` every regime,
`swaps=0` during the steady hover.

### Captures (iteration 18)

- `iteration18_detail_buildout_<regime>.png` — HUD `DetailLevel`/tier counts
  during build-out.
- `iteration18_storefront_pristine.png` — street-level storefront panel on a
  live destructible.
- `iteration18_storefront_damage.png` — the same block after a car bomb: the
  storefront panel disappears with its breached face.
- `iteration18_damage_nadir.png` — elevated view with halo and the green
  damaged minimap footprint.
- `iteration18_horizon_smoke.png` — three blocks away: host smoke column and
  colored minimap fill while the record is frozen.
- `iteration18_storefront_return.png` / `iteration18_storefront_restored.png` —
  re-entry continuity and full restoration (journal 0, fill back to white,
  swap-out eligible).

### Verification (iteration 18)

- Manhattan: fmt, clippy `-D warnings`, **78 tests**, release build.
- Destructibles: fmt, clippy `-D warnings`, **157 tests**.
- New tests: `detail_tiers.rs` (Tier-0 footprint collider present; Tier-1
  removes it and spawns wall colliders; tier ordering/budget), Manhattan
  `minimap_color_*`, `storefront panels/ad attachment`, `dressing damage
  participation`, `detail_priority`, `damaged_persistence_round_trip`.
- Determinism/journal/audit battery unchanged (destructibles workspace suite).
- Zero stray handles after every regime; hover swaps = 0.

## Iteration 19 — city time-reversal controller and one far plume

### CityRestorationController

A single city-level invocation assigns every damaged building within a radius a
per-building restoration rate through the destructibles volume ledger:

```text
restore_city(epicenter, radius, amplitude, profile)
rate(b) = amplitude * profile(distance(b, epicenter) / radius)   for distance <= radius
```

Buildings outside the radius are untouched. The command works on the journal,
not the render regime, so HOT, FROZEN and DORMANT records all receive an
assignment. Overlapping invocations merge: a running assignment keeps its rate
unless the new epicenter is strictly nearer; completed assignments are always
replaceable; a completed ledger still resident during the animation settle is
re-armed for fresh damage. No thrash.

Falloff profiles (documented math, panel-selectable):

| Profile | `f(t)` for `t = d / r` in 0..1 |
|---|---|
| flat | `1` |
| linear (default) | `1 - t` |
| inverse-square | `(1 / (1 + 15 t²) - 1/16) / (1 - 1/16)` |

All profiles are clamped to 0..1; linear and inverse-square reach exactly zero
at the radius. Because the nearest buildings receive the highest rate, they
complete first and the healing front visibly radiates outward. Panes
reassemble, rubble un-collapses, wounds seal in reverse choreography, halos
sweep shut, and each building's terminal epilogue fires as its journal empties.

### Scheduler and budget

Destructibles owns the ledger; Manhattan owns the frame budget. Every update
calls `advance_restoration_budget(5 ms)`:

1. **Fair minimum pass** — every restoring building gets at most one journal
   entry. A rotating cursor changes the starting building each frame.
2. **Priority pass** — the remaining budget is spent highest-effective-rate
   first, which is the wave front.
3. Credit accrues for every building every frame, so an unserviced frame only
   defers pops; it does not slow the mathematical schedule.

A building can lag its scheduled completion by a frame, but cannot be starved
permanently. `RestorationStep` reports serviced/completed/starved and measured
slice time. The controller logs throughput (buildings/s), worst slice, and the
number of budget-deferred (`starved`) frames per wave.

### Controls and minimap

| Input | Action |
|---|---|
| reticle | epicenter (building hit, or ground-plane intersection) |
| `G` | fire city wave |
| `[` / `]` | radius 80 / 150 / 250 / 400 / 600 / 900 / 1 400 m |
| `-` / `=` | amplitude 100 / 250 / 500 / 1 000 / 2 000 / 4 000 m³/s |
| `P` | profile flat / linear / inverse-square |
| `Z` | single-building restore (unchanged) |
| `Shift+Z` | **delegates** to the city controller: radius 250 m at the reticle, current amplitude/profile |

The WAVE panel shows the configuration, epicenter, active count, contracting
front radius, measured slice/peak, starved frames and last throughput. The
minimap draws the front as a translucent green ring that contracts from the
assignment radius to zero as buildings complete; live damaged footprints inside
it bleed back to white.

### Regime × restoration (wave variant)

`wave_regimes.rs` spawns one building in each regime, damages all three, then
evicts the DORMANT one and freezes the FROZEN one. One wave restores all three;
per-building checksums return to pristine, and the DORMANT record reconstructs
pristine from its spec on approach. `wave_scheduler.rs` covers tiny-budget
fairness (no watchdog), high-rate-first order, extension preserving progress,
overlap rate adoption, and damage arriving mid-wave: the first wave leaves
exactly the newer journal entries, the second restores to pristine.

### One far plume

The host crossed-quad column is the **single** far-plume mechanism
(`FAR_PLUME_OWNER = "manhattan-host-quads"`). Manhattan no longer spawns the
destructibles far sprite for FAR duty; the destructibles sprite remains for the
standalone demo only. The host plume is journal-derived (frozen damaged
records only) and despawned on thaw/release. Edges are softened with a
procedural radial-alpha texture (Gaussian-like falloff plus deterministic
noise), replacing the hard card edges deferred since iteration 13. The softened
column is still 10 quads per record.

### Wave performance (release, damage active)

Measured by `MANHATTAN_WAVE=1 MANHATTAN_WAVE_NOCAPTURE=1`; total frames include
Manhattan city streaming (out of scope), the slice column isolates restoration
machinery.

| Requested / assigned | Radius | Duration | Throughput | Worst total frame | Worst restoration slice | Deferred frames |
|---|---:|---:|---:|---:|---:|---:|
| 5 / 5 | 150 m | 1.41 s | 3.56 buildings/s | 184.0 ms | 5.03 ms | 1 |
| 20 / 20 | 150 m | 1.99 s | 10.07 buildings/s | 230.9 ms | 5.23 ms | 12 |
| 50 / 38 (live cap) | 150 m | 5.95 s | 6.38 buildings/s | 250.0 ms | 5.69 ms | 35 |
| 50 / 38 | 900 m | 5.56 s | 6.84 buildings/s | 250.0 ms | 6.65 ms | 32 |
| 10 (capture run) | 150 m | 1.81 s | 5.52 buildings/s | 234.0 ms | 5.18 ms | 8 |

No restoration slice exceeded 6.65 ms (one indivisible journal entry can
overshoot the 5 ms target); nothing attributable to restoration approaches
30 ms. The largest radius tested, **900 m**, completed every assigned building
with no permanent starvation. Total worst frames remain city streaming and the
screenshot request, as logged since iteration 17.

Gauntlet spot-check with a wave running at hero speed
(`MANHATTAN_GAUNTLET_WAVE=1`): worst slice 3.32 ms, hero worst frame 154.5 ms
(vs 124.4 ms without the wave), `stray=0`, hover swaps 0.

### Captures (iteration 19)

- `iteration19_wave_mixed.png` — HOT/FROZEN mixed district before firing.
- `iteration19_wave_ring_0.png` / `_1.png` — altitude signature: the green
  minimap front contracts 125 m → 57 m as the white fills expand.
- `iteration19_wave_hot_heal.png` — on-camera HOT record mid-restoration.
- `iteration19_mid_wave_damage.png` — damage arriving during the wave (front
  42 m, new smoke, remaining assignments).
- `iteration19_wave_restored.png` — after the second (fresh-damage) wave.
- `iteration19_soft_plume.png` — the single softened host plume from range.

### Verification (iteration 19)

- Manhattan: fmt, clippy `-D warnings`, **84 tests**, release build.
- Destructibles: fmt, clippy `-D warnings`, **163 tests** (scheduler, mixed
  regime wave, mid-wave journal arithmetic).
- Determinism/audit/triple-restore/regime matrix battery unchanged.
- Zero stray handles in every gauntlet regime; hover swaps 0.
- Networking note: a city wave is a journaled, serializable rate assignment
  (epicenter, radius, amplitude, profile, per-building rates and cutoffs). It
  replicates under the same contract as damage; no new network semantics.

## Iteration 20 — visual parity: one skyline, two generators

### Interior envelope and reveal

**Root cause found:** the structural lattice was generated as an axis-aligned
AABB grid over the footprint bounds and never clipped to the polygon. Almost
every Manhattan footprint is rotated or concave, so slab rectangles, boundary
columns, core columns and the whole-building interior shell extruded through
the facade. Stacked across floors they read as balcony/chimney protrusions.

**Fix:** slabs, columns, core columns and the shell now shrink to the largest
inscribed rectangle around their centre (`fit_rect`); boundary supports keep a
thin 0.03 m fallback so the support graph does not lose a ring. Clutter keeps a
rotated-AABB containment filter, and service pipes are placed inside the
footprint instead of at a fixed coordinate. Full-city audit over
`building_perimeters.json`:

| Check | Before | After |
|---|---:|---:|
| generated records | 9,678 | 9,678 |
| slab out of envelope | 9,643 | 0 |
| column out of envelope | 4,790 | 0 |
| shell out of envelope | — (AABB shell) | 0 |
| clutter out of envelope | 9,541* | 0 |
| roof / height-above-roof / antenna allowance | 0 | 0 |

\*before counts use the AABB containment check; after counts use the same
rotated-AABB invariant as the generator. The remaining 1,354 records are
rejected by pre-existing `BuildingSpec` validation (perimeter > 64 vertices,
tiny footprints, floor/window mismatch) and never generated; the test asserts
generated + rejected = 11,032.

**Reveal model:** every interior component (slabs, columns, core shaft, shell,
fill light, clutter) spawns `Visibility::Hidden`. A broken pane, removed wall
cell or breach exposes exactly the storey it belongs to; shell/core/light reveal
with any open storey. Restoration removes the breach and the same query hides
everything again, so the pristine epilogue is naturally fully hidden — tested
by `interior_reveal.rs` (pristine → hidden, floor-1 pane → only floor 1 shown,
restoration → hidden).

**Measured cost delta** (`MANHATTAN_PERF_PROBE=1`, fixed dense pose, 52 live
buildings, 5 s window after zone settle):

| Mode | avg frame | interior mesh entities | hidden |
|---|---:|---:|---:|
| pristine interiors hidden | 50.43 ms | 2,661 | 2,661 |
| legacy always-visible | 57.01 ms | 2,318 | 13 |

≈ **6.6 ms / 11.6 %** saved on the dense pristine neighborhood; roughly 2.3k
interior mesh draws are removed at that pose.

### Phantom-tower audit

The height-derivation candidates (bulkhead/core/antenna divides) have **zero**
offenders across the generated city; antenna masts stay within the documented
6–12 % allowance. The chimney/balcony read was the out-of-envelope shell and
stacked lattice described above. The batch invariant now checks footprint
containment, shell containment, roof height, column top, core height and
antenna allowance for every record.

### Unified material strategy

- **One palette, one source of truth:** `destructibles_core::palette` holds the
  base colours both generators use. The map's lighter palette wins:
  brick `0.82/0.75/0.65`, concrete `0.70/0.68/0.63`, glass `0.63/0.71/0.77`,
  roof tan `0.64/0.60/0.51`, trim `0.36/0.29/0.23`, equipment `0.46/0.47/0.49`.
  Destructibles' WorldAssets, map roof furniture and generated roofs all import
  these constants.
- **Textures:** both trees already load the same brick/concrete/glass texture
  files; map facades additionally use their baked facade/roof atlas. Destructible
  geometry keeps tiled UVs and trim detail on top of the same palette rather
  than a second darker material family. Trade-off: map facade atlas cells and
  destructible tiled walls are not byte-identical textures, so a swap still
  reads as a **style/LOD shift** (accepted since iteration 17), never a dark→light
  palette jump. The live↔plain pair shares palette, roof tan and furniture.

### Roof recipe (both generators)

- **Roof surface:** plain tan, metallic 0, roughness 1.0 — no directional
  specular wedge. Map roofs keep their baked atlas within the same tan family.
- **Parapet curb:** generated roofs bake a distance-from-edge vertex-colour
  band (`curb_shade`: darker within ~1 m of the edge, ~0.62 deep on the field).
  No extra geometry. Map roofs rely on the atlas' baked edge band; documented
  trade-off (the band is not re-baked into the uploaded glTF).
- **Equipment:** `roof_equipment(building, footprint, height)` is a seeded,
  journal-transparent generation parameter (1–6 grey boxes scaled by roof area,
  parapet-aware, deterministic). Generated buildings merge them into one mesh;
  plain map buildings get one merged equipment mesh plus one merged antenna
  mesh, hidden while the live record owns the footprint.
- **Antennae/nav lights:** the destructible recipe wins. `antenna_heights` moved
  to core and the map's pyramidal "pointy tree" antennas are retired; map
  furniture uses the same deterministic mast heights and thin masts.
- **Alignment:** both roofs are built from the true footprint polygon; the
  generated roof curb and equipment use footprint-local coordinates, so roof
  edges align on both sides.

### Captures (iteration 20)

- `iteration20_roof_live.png` — live generated roof: tan field, curb band,
  equipment boxes, masts.
- `iteration20_roof_plain.png` — plain map roof with the same furniture recipe.
- `iteration20_roof_restored.png` — the same live roof after damage+restoration
  (identical equipment arrangement; regeneration invariance).
- `iteration20_roof_aerial.png` — mixed liveness block from altitude.
- `iteration20_interior_hidden_street.png` — street face with zero protruding
  slabs/shell.
- `iteration20_mid_swap.png` / `_detail.png` — in-flight live/plain mix from the
  gauntlet capture; palette reads as a style/LOD shift.

### Verification (iteration 20)

- Destructibles: fmt, clippy `-D warnings`, **168 tests** (envelope clipping,
  reveal transitions/re-hide, equipment determinism, palette, antennas).
- Manhattan: fmt, clippy `-D warnings`, **87 tests** (full-city audit 11,032
  records, shared roof recipe/palette).
- Gauntlet unchanged in structure: worst frames 160.1 / 141.1 / 125.3 / 97.6 ms,
  `stray=0` each regime, hover swaps 0. Journal, audit, triple-restore and wave
  battery untouched and green.

## Iteration 21 — atmosphere, composition and the heartbeat

### Lighting contract

- **Sun** is the only shadow caster (shadow maps enabled). Colour grades from
  pink-orange at the horizon to strong yellow at the zenith by sun elevation;
  illuminance fades to zero below the horizon. Direction comes from the existing
  `sun_position` time-of-day parameter.
- **Moon** is always far weaker (160 vs 16 000 lux at full), always bluish-white
  at baseline, and mirrors the real lunar altitude/azimuth. Its colour is
  modulated by a **read-only journal consumer**:
  `pollution = 1 - exp(-burning / 26)`, `tint = mix(blue, orange-red, pollution)`
  where `burning` is the live destructibles fire count (`Hazards`). Ten burning
  blocks measurably reddens the moon; full restoration returns the blue value.
- **Day ambient** is sky-blue top-down and reddens at sunrise/sunset by the sun
  elevation; shadowed facades therefore read blue, not black.
- **Night ambient** is deliberately **ground-up**: a rotating pool of 24
  block-coloured point lights at 6 m height, one per nearby block, with a
  deterministic halogen/neon mix from `block_ambient_color(block)`. Building
  feet are lit, tower tops fall dark. The pool follows the camera (24 lights,
  shadow-free, 0.25 s refresh; cost is in the canonical-moment table). Power
  loss cannot be expressed per building in a block-wide map fill; destructibles'
  generated facade uplights remain the journal-correct per-building source and
  turn off with power, so the two never contradict.
- Tests: `moon_tint` shifts and clears; `night_ambient` base/roof ratio > 2 and
  adjacent blocks differ; overcast direct factor dims.

### Times Square projection

The shipped Blender geometry stops at ~40.731 N; the real Times Square is ~4 km
further north and is not in the map. All radial fields use the **nearest mapped
point at Times Square's longitude** (`1711.638, -2000`), documented as the
epicentre proxy so the field is visible inside the rendered city.

### Facade probability field

`P(glass) = clamp(0.04 + 0.90 · exp(-(d/900 m)²) · year_factor, 0.04, 0.945)`

`year_factor = 0.18 + 0.82 / (1 + exp(-(year-1965)/20))`. Height is **not** an
input. Buildings roll `hash(id, block) < P` and become concrete/glass by era, or
stay brick/brownstone; tall brick and short glass are both legal. Core glass
probability is 0.94 (never 1); holdout brick survives at the epicentre.

Construction dates: the source binary has no date or stable OSM id, so an OSM
spatial join would be guesswork. A deterministic heuristic
(`scripts/bake_year_built.py`, radial + category + hash) is baked into
`building_perimeters.json` for all **11,032** records. Coverage: **0 real /
11,032 estimated** (`captures/iteration21_year_built.log`). Style assignment is
computed from that baked meta by both the map furniture path and `spec_for`, so
a swap cannot change a building's era.

### Advertising saturation

Ad density is `1.0` within 250 m of the epicentre and falls to zero at 2.5 km
(power 1.35). `spawn_signs` multiplies sign count by that field and skips
buildings below 0.02. Result: **254,284 → 44,179** ad quads, **10,723 → 5,909**
ad-bearing buildings (**−82.6 %**), with zero ads at the map edges. Times Square
street level is the saturated worst case (see capture).

### Two-beat daily rhythm

`daily_pace(hour)` = 0.05 + 0.90·max(AM 8:30, PM 17:00, midday) Gaussian; 3 AM
is the 5 % floor. Lane occupancy is
`0.05 + (pace-0.05) · toward_factor`, with toward Times Square at 0.90 / away
0.40 at the AM peak, **flipped** at PM. Lanes rebuild when the target occupancy
changes by >0.12. Pedestrian target = perimeter × `0.03 + 0.97 · ad_density ·
daily_pace` (ads drive population). Unit tests cover the four canonical values
and both asymmetries.

### Cloud deck

The existing four drifting soft decks remain. A fifth **overcast-only low deck**
is added; its altitude is data-driven from the tallest tower:
`deck = 960 + ((0.8·tallest).clamp(260,420) - 960)·cover^1.4`, i.e. ~330 m at
full overcast — low enough to swallow the top 20 % of the 410 m downtown towers.
Clouds are billboard/noise impostors, not volumetrics. A new camera uniform
drives a soft-particle fade (`smoothstep(10,95 m)`), so flying through the deck
dissolves puffs instead of slicing a hard sheet. Overcast also dims direct sun
(`1 - 0.80·cover - 0.14·storm`), flattening shadows.

### Performance (canonical moments, fixed 320 m pose, 44 live)

| Moment | avg frame | worst | pedestrians | ad meshes |
|---|---:|---:|---:|---:|
| AM rush, epicentre | 49.14 ms | 128.3 ms | 1000 | 19,417 |
| PM rush, epicentre | 184.48 ms* | 250 ms | 1000 | 19,385 |
| Midday, outer | 48.49 ms | 87.7 ms | 1000 | 19,348 |
| 3 AM | 47.62 ms | 77.2 ms | 1000 | 19,384 |
| Noon clear | 46.79 ms | 96.8 ms | 1000 | 19,387 |
| Noon overcast | 46.06 ms | 82.9 ms | 1000 | 19,428 |

\*the PM run hit a city-streaming spike (28 frames averaged 184 ms); the
other moments and the overcast delta are within noise. Pedestrian counts sit at
the 1,000 cap at this camera because the initial fill happens before the clock
override and the 0.65 s despawn cadence cannot drain fast enough inside the 5 s
window; the daily/radial targets are unit-tested and the count difference is a
measurement-window limitation, not a model failure. Vehicle entity counts read
0 in the probe for the same streaming-window reason; lane occupancy is
unit-tested.

### Captures (iteration 21)

`iteration21_times_square_am_rush.png`, `_pm_rush`, `_graveyard_shift`,
`_distant_no_ads`, `_glass_core_aerial`, `_brick_edge_aerial`, `_dawn`,
`_noon`, `_dusk`, `_night`, `_overcast_flythrough`, `_smoke_moon`; logs in
`iteration21_golden.log`, `iteration21_gauntlet.log`, `iteration21_perf_*.log`,
`iteration21_year_built.log`.

### Verification (iteration 21)

- Manhattan: fmt, clippy `-D warnings`, **100 tests**, release build.
- Destructibles: unchanged this iteration; **168 tests** still green.
- Gauntlet: worst 156.9 / 140.1 / 107.1 / 86.4 ms, `stray=0` every regime,
  hover swaps 0. Journal/audit/determinism/wave battery untouched.
- Year metadata is generation-only; no checksum changes.

## Iteration 22 — visual correctness sweep and swap-brain surgery

### Traveling shadow (fixed)

Root cause: the camera-following cloud decks (15–27 km discs at 360–2,180 m)
became shadow casters when iteration 21 enabled sun shadows, painting a moving
dark disc over the city beneath the camera. All five decks now carry
`NotShadowCaster`; only the sun/moon directional fixtures shadow the scene.
Before/after: `iteration22_shadow_before.png` / `_shadow_after.png`. The camera
no longer changes any building's direct light.

### Storefront illumination gradation

Storefront atlas materials are now four **emissive bands** selected by the
shared radial ad-density field: 0 (lit-only, shuttered), 0.45, 1.0, and 1.7
(Times Square). This is the **fourth consumer** of the same field (ads, people,
storefront light, facade probability), and the material handle is stored per
building so the live re-projection keeps the same band. Epicentre storefronts
stay self-lit; outer-borough bands are mostly dark and lit only by the sun.

### Glass grammar inversion

`spec_for` previously widened every bay for LOD, including curtain walls, so an
all-glass tower became 1.36 m panes in 4.2 m bays (~32 % glass). Curtain walls
now get height-scaled panes (`1.35 + min(h/300, 0.45)` m wide, floor-height
tall), 0.10 m mullions and 0.05 m edge margins. Batch audit over the generated
city: tall (≥60 m) glass buildings with a realised pane-to-facade area ratio
below **0.60** → **0 offenders**. (One edge case sits at 0.632; the rest are
higher.) The old synchronous LOD convention still applies to brick/concrete.

### Ghost shells and orphaned antennas

- **Height integrity:** `height_matches` is the twin of the footprint check
  (0.05 m tolerance). A swap whose generated height differs is rejected and
  plain geometry stays. Gauntlet run: one record (b6159) rejected, no ghost
  shells; `height_failures` counter added.
- **Furniture ownership:** map antennae/equipment are hidden with the rest of
  the plain building while live (`RoofFurniture` in `set_plain_hidden`).
  Antenna containment audit: every mast ≤ 12 % of the map record height, 0
  failures across 9,799 generated records.
- **Windows-only shell:** Tier-1 regression test now asserts the wall shell is
  present and visible plus non-empty slab/column sets at Tier 1.

### Clock decoupling

`ProcessingClock` advances with wall time and never pauses; `SimClock` drives
time-of-day, journal-adjacent choreography and the heartbeat. When the game
clock is frozen, swap budget and max swaps double and the detail budget rises
4 → 8 ms, so background catch-up uses the gift frame budget. Processing time is
never read by the journal. Test: processing time advances while `SimClock` is
paused and `catch_up` is set.

### Swap prioritization: current-position, LOS-weighted, idle GC

- Candidate set and priority queue are rebuilt every tick from the camera's
  **current** position; stale pending entries behind the camera are dropped
  before instantiation. The instantiate→collect cycle is structurally gone.
- Priority classes: **visible ahead (0) < visible/occluded ahead or visible
  behind (1) < occluded behind (2)**, distance as tie-break. LOS is a coarse
  ray vs candidate AABBs.
- GC is idle-time only: exits are queued only below 3 m/s (or while the clock is
  frozen); moving frames clear the exit queue rather than mass-despawn.
- Teleport envelope metric: **345 ms** until ≥8 live buildings at the new pose
  (`iteration22_perf_am.log`). Gauntlet: `stray=0` every regime, hover swaps 0.
  Mach 1.92 remains unreachable ahead (documented since iteration 17); behind is
  no longer spawned-then-collected.

### Performance refresh

| Pose | avg frame | worst | notes |
|---|---:|---:|---|
| AM rush epicentre | 77.65 ms | 136.3 ms | sun shadows + full ads + 1,000 peds |
| pedestrian 15 m/s | — | 250.0 ms | city-streaming spike |
| fast 30 m/s | — | 187.1 ms | |
| hero 100 m/s | — | 184.2 ms | |
| mach 1.92 | — | 126.7 ms | |

The shadow fix itself does not add cost; enabling sun shadows in iteration 21
did, and the camera-shadow bug is gone. City-streaming frames remain the
deferred item.

### Captures/evidence (iteration 22)

`iteration22_shadow_before/after`, `iteration22_swap_envelope[_detail]`,
`iteration22_perf_am.log`, `iteration22_gauntlet.log`, `iteration22_golden.log`
(golden set regenerated under the fixes).

### Verification (iteration 22)

- Manhattan: fmt, clippy `-D warnings`, **103 tests**, release build.
- Destructibles: **168 tests**, clippy clean (detail-tier shell test extended).
- Batch audits: all envelope/height/antenna/glass invariants 0 offenders;
  9,799 generated, 1,233 pre-existing spec rejections.
- Journal/audit/determinism/wave battery untouched.

## Iteration 23 — instrumentation, correctness, perf war

### Instruments

Always-visible FPS panel (F3): instant FPS, 5 s average, worst frame of the
last second, p50/p95/p99 per second, heading; plus the reviewer tuning line.
The minimap carries a compass ring with a north tick/N that rotates with heading
(`compass_angle(yaw) = yaw - 90°`; tested). Committed as a feature.

### 20-shot journal gate

`repeat_fire.rs` fires 20 tier-2 impacts twice: inline-drained and fully queued
before admission. Both journals grow monotonically, the head never regresses,
no restoration ledger leaks, and every entry belongs to the building. The
queued case drains in ≤25 frames. No journal mutation was required; the gate
guards the admission path.

### Shared floor pitch

The map record's `floor_height_m` (clamped 2.4–6 m) is ground truth for both
generators; the 8-floor LOD cap is gone (200-floor safety bound). Window rows
now match the map: batch `rows=0` mismatches over 9,761 generated records.
Cost: **cells 1,011,970 → 1,320,882 (+30.5 %)**, **panes 1,707,733 →
2,237,821 (+31.0 %)**, measured with the `MANHATTAN_LEGACY_FLOORS` hook.
Verification gate now includes window-row parity.

### Chimney / hybrid gate

Batch audit: tall/chimney 0, envelope 0, antenna 0, glass 0, rows 0 over 9,761
generated records (1,271 pre-existing spec rejections). Swaps require footprint,
height and window-row parity; failures stay plain (b6159 rejected in the
gauntlet, counted). Map furniture hides with the plain building, so the
antenna-orphan path is gone.

### Shadows: decision and cost

Research/codegraph showed the pipeline has no lightmap UV channel or bake pass
for 11k merged atlas buildings; a static lightmap hybrid would require a new
UV/bake pipeline outside this round. Decision: keep dynamic sun shadows (the
only global caster) and restore a **tiny damage-theatre budget** — the first
fire light per building casts, the second does not; night block fills stay
shadowless. Measured line item at the AM pose: **78.04 ms without sun shadows vs
87.12 ms with (≈9.1 ms)**. Firelight shadow cost is included in the burning
night capture; no measurable frame spike beyond city streaming was observed.
A future lightmap hybrid remains the documented recommendation with this
measured baseline.

### Plume rewrite (host FAR)

Far columns now use **7 recycled billboard puffs**: seeded phase per building,
slow rise, expansion, renewal, camera-facing, `NotShadowCaster`. This is 7
draws per frozen column (down from 10 crossed quads). Near fire columns remain
the destructibles Hanabi system. Epilogue removes the column; seed derives from
the journal/host state so replay reproduces it. Moon tint remains a read-only
journal consumer.

### Targeted optimization table

| Item | Before | After |
|---|---|---|
| AM dense frame | 77.65 ms | 87.12 ms |
| Sun-shadow line | 78.04 ms (no shadows) | 87.12 ms (≈9.1 ms) |
| Floor pitch cells/panes | 1,011,970 / 1,707,733 | 1,320,882 / 2,237,821 |
| Visible mesh entities (AM) | 89,834 | 92,239 |
| Teleport envelope | 345 ms | **178 ms** |
| Hybrid double-render | possible | **0** (gate rejects) |
| Admission budget | const 6 ms | host-wired 6 ms, 20-shot drain ≤25 frames |
| Plume draws / frozen column | 10 | **7** |

Shot saturation: one impact decomposition per frame ⇒ **60 impacts/s at 60
fps**; a 20-shot backlog drains in ~0.33 s. Damage therefore shows as slow
damage, not slow frames.

### Controls parity

`tuning.toml` is now in the tree and loaded at startup. The instrument panel
shows and edits: explosive yield, projectile mass, projectile velocity,
restoration rate, grip force, fire-stage pace and admission budget; F6 saves,
F7 reloads. Damage presets consume yield/mass/velocity, restoration consumes
the rate, admission budget is written into the destructibles Admission resource
each frame, and grip force into the destructibles Tuning resource. Fire-stage
pace is persisted/displayed (consumption is a destructibles-side follow-up).

### Verification

Manhattan **104 tests**, destructibles **170**, clippy clean both. Gauntlet:
worst 250.0 / 202.1 / 157.4 / 115.6 ms, worst swap 44.7 ms (more floors/cells
as expected), `stray=0` each regime, hover swaps 0. Batch audits 0 offenders.

## Iteration 24 — just-in-time destructibles

### Resting state and deletion

Camera-follow prefetch, proximity/nadir/corridor swap-in, hysteresis and swap
envelopes are **deleted**. `update_zone` now only performs idle GC/release; no
building becomes live from camera position, speed, altitude or clock. The
gauntlet confirms the resting city: `live=0, swaps=0` at 15/30/100/650 m/s.

### JIT impact router

`BuildingIndex::resolve_point` is the deterministic point→record resolver: a
1000 m candidate grid, point-in-polygon, boundary fallback within 1 m, nearest
centroid then id tie-break. Unit test: every sampled grid point matches the
manual footprint scan. `queue_damage` and the reticle JIT-swap a plain record
before damage admission; the same footprint/height/window-row geometry gate
rejects silently to plain. JIT is the only producer of live buildings.

### First-contact latency

Bullet JIT: swap-in **4.12 ms**, Tier 1 ready **0.32 s**, wound captured.
Car-bomb JIT: swap-in **7.90 ms**, Tier 1 **0.55 s**. JIT targets get the 8 ms
detail budget and front-of-queue priority until Tier 1. Repeat fire hits the
live record instantly; after full restoration (eviction) the next shot pays JIT
again.

### Damaged set and eviction

With zero damage, `live=0` and no interior/destructible meshes exist. Each
damaged record is live exactly once; frozen/dormant/restoration rules unchanged.
The restoration epilogue queues immediate eviction: journal empty → thaw/release
→ swap-out to plain, map furniture unhides, memory returns to baseline.

### Attribution and after-table

| Item | Before (iteration 23) | JIT after |
|---|---|---|
| AM dense frame (52 live) | 87.12 ms | 40.76 ms (0 live) |
| Visible mesh entities | 92,239 | 69,560 |
| Gauntlet 15/30/100/650 worst | 250 / 202 / 157 / 116 ms | **51.6 / 73.2 / 53.5 / 47.4 ms** |
| Swap machinery in flight | up to 44.7 ms/swap | **0 swaps** |
| Detail tiers in flight | 46 Tier-1 builds | 0 |
| Sun-shadow line | ≈9.1 ms | unchanged |
| JIT spawn | — | 4.1 ms bullet / 7.9 ms car bomb |
| Tier-1 first contact | — | 0.32 s / 0.55 s |

Ordinary flight now costs what idle costs; the remaining 40–73 ms is city
streaming/entity draw load, seeding round 25. Captures:
`iteration24_jit_before`, `_jit_bullet_wound`, `_restored_plain`; logs
`iteration24_jit.log`, `iteration24_perf_idle.log`, `iteration24_gauntlet.log`.

### Verification

Manhattan **105 tests** (resolver + JIT gate), destructibles **170**, clippy
clean. Gauntlet `stray=0`, hover swaps 0; batch audits unchanged 0 offenders.

## Iteration 25 — fire path repair and real-input acceptance

### Root cause

JIT created the live record in `damage_input`, after the frame's
`update_zone`/`process_swaps` pass. With camera prefetch deleted there is no
"desired set", so the next frame's idle GC saw the fresh JIT record as
out-of-zone and swapped it out **before Tier 1 admitted the queued shot** —
the journal stayed empty, no wound, no minimap fill. Fix: a building is exempt
from exit/GC while it is `jit_target`, has pending damage, or carries journal
damage; `process_swaps` honours the same guard.

### Fire trace and rejection counter

`damage_input` now logs every stage (key/mouse, ray origin, hit point, resolved
id, live/JIT, gate verdict, admission) and the DETAIL HUD shows
`rej=<n>` (`jit_rejected`). Left mouse also fires; X remains.

### Real-input path

`MANHATTAN_INPUT_TEST=1` synthesizes a genuine `ButtonInput<KeyCode>` X press
and drives the real keybind → reticle → resolver → JIT gate → spawn → Tier-1 →
journal chain. Result: target b9634, JIT 13.5 ms geometry PASS, Tier-1 1.07 s,
`DAMAGE bullet` logged, `live=1`, and the capture shows the wound with the
white minimap fill (`iteration25_input_fire.png`). OS-level focus itself could
not be automated on this Wayland host (`wtype`/xdotool unavailable), so that
link is reported UNVERIFIED; every stage after the key event is exercised.

### Report discipline amendment

`README.md` now states that each iteration report must label features
**exercised via real input** or **exercised via programmatic path**, and mark
anything without an input-driven test as **UNVERIFIED**. Iteration 25 status:
bullet JIT = real-input (mocked) PASS; car-bomb JIT = programmatic PASS;
20-shot monotone = programmatic PASS, real-input UNVERIFIED; hero/lift/thrown
neighbour = programmatic only / UNVERIFIED real-input; restoration eviction =
programmatic PASS; minimap fill = real-input capture PASS.

### Performance

No regression: idle flight remains 40.76 ms AM / 47–73 ms gauntlet; JIT spawns
3.7–14.9 ms and Tier 1 0.27–1.26 s in these runs. Rejection counter stays 0
for accepted records.

## Iteration 26 — the shot that never landed

### Root cause

G was never bound to the fire handler. In iteration 25 G still fired the city
restoration wave (`restoration.rs`) and cycled the stationing set
(`stationing.rs`); the UI key-click audio made it sound like a gunshot. X/left
mouse were the only damage bindings, so the reviewer's G press had no shot.

### Fix

- **G is the authoritative fire key** (X and left mouse remain aliases).
- City restoration wave moved to **B**; stationing set moved to **Y**.
- The full badge chain is on the HUD and in stdout:
  `SOUND=heard → RAY origin/direction → HIT=b####/MISS → GATE=PASS/REJECT →
  JIT=spawned/live → ADMITTED[=queued(T1)] → JOURNAL=n → WOUND VISIBLE`.
- `MISS` increments its own counter (`miss=`), alongside `rej=`.

### G-path input test

`MANHATTAN_INPUT_TEST=1` injects a real `ButtonInput<KeyCode>` **G** press and
drives keybind → ray → resolver → gate → JIT → admission. Street pose:
`HIT=b9634`, JIT 4.69 ms PASS, Tier 1 1.18 s, `DAMAGE bullet` journaled,
wound + white minimap capture. Aerial pose (180 m): `HIT=b9675`, JIT 6.54 ms,
damage journaled — the JIT target survives idle GC. Logs:
`iteration26_input_g.log`, `iteration26_input_g_aerial.log`.

### Reporting contract amendment

The binding the reviewer uses is the binding tests use. G is now encoded;
tests must inject G. A fire key that only produces audio without lighting the
badge chain is a failure.

### Status

| Feature | Status |
|---|---|
| G bullet street → wound + white fill | real input (mocked) PASS |
| G bullet aerial → wound | real input (mocked) PASS |
| Car bomb / 20-shot / hero / lift / thrown | programmatic path only; real-input **UNVERIFIED** |
| Badge chain + miss/reject counters | permanent HUD + logs |
| Performance | unchanged from iteration 24 (40.76 ms AM idle, 47–73 ms gauntlet) |

## Iteration 27 — invisible JIT swap

### City twin hiding

JIT now runs the same hiding machinery as the retired camera swap *inside the
JIT frame*: the live record is added, `set_plain_hidden` hides storefront, sign
and roof-furniture meshes immediately, and `refresh_mask` rasterises the
footprint into the live mask before the frame renders. Previously the mask was
painted, but JIT did not invoke the furniture/storefront hide on the spot, so
the plain street face could survive the swap.

Permanent acceptance evidence: the mocked-G input test samples the rebuilt
mask under the live footprint and reports `mask_covers=true` with the wound and
white fill (`iteration27_input.log`, `iteration27_jit_no_ghost.png`). A
programmatic mask helper samples centroid, vertex average and every perimeter
vertex; the test fails visibly (`false`) if the city twin is not suppressed.

### Height and halo

The gate compares footprint, height and window rows against the record; the
live roofline equals the record height after the floor-pitch alignment. The
halo anchor was raised from roof+3 m to roof+5 m (destructibles
`manhattan-8`) so it clears the rendered roof/parapet skin and can never sit
inside either the live or city shell. Antenna allowance still uses the
journal-derived mast tips; the halo uses the taller of mast tip and roof.

### Texture parity — partial, honest

The existing iteration-18 storefront panel and ad re-projection already samples
the map atlas at close range. Full exterior wall/floor atlas sampling was not
implemented this round: destructible walls keep tiled brick/concrete/glass
materials on the shared palette, so a muzzle-range swap still reads as a
style/LOD shift rather than an identical texture repaint. No crossfade was
added. This acceptance criterion is **UNVERIFIED** by the reviewer's eye; the
mask/no-ghost and height criteria are covered.

### Performance

JIT spawn on the verification run: **9.67–14.84 ms** (mask raster + furniture
hide included), Tier-1 unchanged. Flight/idle baselines from iteration 24 are
unchanged.

### Verification

Manhattan: fmt, clippy clean, tests green; destructibles **170 tests**, clippy
clean; batch audits 0 offenders. Real-input (mocked G) street shot: `HIT`,
`GATE=PASS`, `JIT`, `ADMITTED`, `JOURNAL`, `WOUND VISIBLE`, `mask_covers=true`.
Texture parity at muzzle range remains UNVERIFIED.

## Iteration 28 — partial delivery, explicit UNVERIFIED list

This round was not completed to the requested bar. Implemented and measured:

- **Traffic halved everywhere:** lane occupancy is multiplied by 0.5 at every
  daily point (AM toward ≈0.43, away ≈0.20), pedestrians unchanged. Tested by
  the existing occupancy function values.
- **Archive radius = minimap radius:** `NeighborhoodConfig.archive_radius =
  MAP_RADIUS_WORLD (650 m)`; damaged persistence freezes at the minimap edge
  instead of 150 m. Minimap/fill behavior unchanged.
- **Storm deck height bug found and fixed numerically:** the low-deck curve used
  `cover^1.4` against a 960 m high deck, so a 0.96-cover thunderstorm parked the
  deck at ~925 m. It now collapses to the 328 m low deck for cover ≥ 0.70
  (storm included, tested). **Visual presence is still UNVERIFIED:** the golden
  overcast capture at 365 m does not show the deck, so the render/culling cause
  remains open.

Not implemented (UNVERIFIED, not claimed):

1. Copy-paste city exterior: the host city is merged glTF chunks, not one mesh
   per building, so there is no per-building city mesh handle to clone. JIT
   still uses the generated exterior + live-mask suppression.
2. Host-profile pane retirement: panes remain in the host; debris ratio and
   reduced command set not done.
3. 20-ft guts: guts still follow the facade floor pitch; cell/collider reduction
   not delivered.
4. Tiara health band: halo is still the existing flat washer at roof+5 m.
5. Off-screen animation gating: not implemented; burning off-screen buildings
   still tick their presentation systems.
6. Real-input G battery under JIT after these changes: only the earlier mocked
   bullet path was re-run; car bomb/20-shot/hero/lift/thrown remain UNVERIFIED.

Performance headline table with 0/1/3/5 damaged buildings was therefore not
produced. What can be stated: traffic entity work is halved by construction;
archive GC now happens at 650 m rather than 150 m.

## Iteration 29a — pilot generator (Milestone A, partial gate)

**Tool:** `viewer/src/bin/pilot_gen.rs` (`cargo run --release --bin pilot_gen`).
It wraps the runtime `spec_for`/layout generator, selects a 840 m-wide mixed
region centred at the mapped Times Square proxy, writes
`assets/pilot/pilot_bundle.json`, and self-audits: selected 140, generated 131,
rejected 9 → 9 fallback boxes, envelope 0, window rows 0, per-style batch counts
(brick brownstone 2, brick mid-rise 2, concrete grid 12, glass curtain wall 115),
triangles ≈234,026.

**Runtime:** `--pilot` skips the merged city glTF (ground and roadways stay),
spawns the bundle through `spawn_building_world`, disables JIT/zone/damage
input, and places the camera at the pilot region.

**Run/capture:** street and aerial captures at 12:00 and a night capture were
produced (`captures/pilot_street.png`, `pilot_aerial.png`, `pilot_night.png`).
Frame readings at capture time are startup-heavy (≈4 fps / 250 ms worst) and are
not a valid pilot performance number.

**Gate status:** the pilot exists and runs, but it does **not** satisfy the
visual gate: the generated region is overwhelmingly glass (115/131), storefront
and advertising placement are not part of the bundle, façades are per-building
draws rather than batched per style, and the generated exterior still uses
tiled destructible materials rather than the city atlas. Per the standing rule,
Milestone A halts here: no B/C work.

## Iteration 29a2a — pilot gate evidence

| Pose | avg FPS | p50 ms | p99 ms | source |
|---|---:|---:|---:|---|
| street day | 21 | 46.8 | 55.0 | HUD in `pilot_a2a_street_day.png` |
| aerial day | 17 | 60.0 | 68.9 | HUD in `pilot_a2a_aerial_day.png` |
| street night | ~19 | 53.1 | 57.2 | HUD in `pilot_a2a_street_night.png` |
| aerial night | ~14 | 71.1 | 74.8 | HUD in `pilot_a2a_aerial_night.png` |

`--perf` walker remains non-emitting (scheduling bug); the table is produced by
the screenshot/HUD instrumentation the reviewer accepted as fallback.
Pilot load: 140 Shell buildings in 0.73 s; structures/interiors are deferred
until damage. `--pilot-damage`: b7228 tiered to Structural in 27 ms, bullet
admitted 0.3 ms, car bomb journal 5.63; captures `pilot_damage_bullet.png`,
`pilot_damage_carbomb.png`. Other 139 buildings remain shell-tier.

## CityPack v2 — hierarchical index

**Level contract.** `county -> city -> neighborhood -> leaf records`:

| Level | Count | Membership |
|---|---:|---|
| 0 county | 2 | King (Seattle + Bellevue = 243,909), Pierce (Tacoma = 86,421) |
| 1 city | 3 | Seattle 187,053, Bellevue 56,856, Tacoma 86,421 (source-agency provenance) |
| 2 neighborhood | 1,096 | Existing S1.5 quadtree partitions, re-parented |
| leaves | 330,330 buildings + 29,833 street segments | Assigned to their neighborhood |

Every node stores centre, AABB, roll-up count and style hints, parent index and
child range. Child AABBs are strictly inside parent AABBs (max overflow 0.0 m).
Names preserve the overlay with full ancestry (`King -> Seattle -> Capitol
Hill 04`). County boundaries are Census TIGER/Line 2024; cities are the three
source agencies, verified point-in-polygon against TIGER places (302,443
centroids inside the agency polygon, 27,887 provenance_outside_polygon retained
by source rather than re-bucketed; there are no unincorporated records in this
extent). Snohomish has no records in the region polygon and is omitted.

**Format.** CityPack v2 adds a node table (level, parent, first_child,
child_count, roll-up counts, centre/AABB, hints, name, chunk offset/size) and
street leaves in each neighborhood chunk. Per-chunk interning is unchanged.
Binary 25.26 MB vs 146.6 MB JSON (5.8x); growth from v1 is +1.42 MB (6.0%),
mostly the node table, street records and hierarchy indices.

**Runtime descent and evaluation counts.** Streaming and point lookup start at
county level; an out-of-range county terminates its subtree. Measured distance
evaluations at 2.6 km radius (flat = 1,096 neighborhood tests):

| Pose | flat | county/city/nb | total | ratio | 10× flat | 10× hier | 10× ratio |
|---|---:|---:|---:|---:|---:|---:|---:|
| downtown | 1,096 | 2/2/626 | 630 | 1.74× | 10,960 | 648 | **16.9×** |
| mid-lake | 1,096 | 2/2/807 | 811 | 1.35× | 10,960 | 829 | **13.2×** |
| Bellevue | 1,096 | 2/2/807 | 811 | 1.35× | 10,960 | 829 | **13.2×** |
| Tacoma corridor | 1,096 | 2/1/289 | 292 | 3.75× | 10,960 | 310 | **35.4×** |
| far-south | 1,096 | 2/0/0 | 2 | 548× | 10,960 | 20 | **548×** |

The 10× column builds ten spatial copies of the hierarchy and repeats the same
descent; it is the megaregion extrapolation, not an adjective. At far-south no
city or neighborhood is enumerated — the Tacoma gate proves subtree termination.
The runtime HUD now shows `county/city/neighborhood` evaluations per frame and
the loaded partition's full ancestry.

**Minimap zooms.** `MapZoom` adds neighborhood (650 m) → city (15 km) → county
(80 km); `O` cycles and `MINIMAP_ZOOM`/`MINIMAP_CAPTURE` drive evidence runs.
Neighborhood AABBs aggregate to city outlines and county outlines, so the
instrument renders the hierarchy as geography. Captures:
`captures/minimap_neighborhood.png`, `minimap_city.png`, `minimap_county.png`.

**Provenance alignment.** Counties and cities mirror the three source agencies
(Seattle, Bellevue, Tacoma) that produced the footprints, so the hierarchy is
also the correct scoping unit for future city-level LOD and style packs: a
package can be authored/validated against one city without scanning the
megaregion. Skyline-billboard far-LOD remains a future consumer, not built.

## Seattle S2 — geology round (terrain / water / draped roads, no buildings)

**Terrain.** 30 m DEM (2322×1207, int16 decimetres) is the S2 near-field source.
The runtime builds a 60 m regional mesh over the whole dataset and 30 m near
patches around the S2 poses (64 patches, overlapping the regional mesh at
+0.15 m as the seam treatment; no explicit vertical skirts). Water cells are
removed from every terrain triangle, giving the hard shoreline edge.
Resolution tiers: 30 m near / 60 m regional; triangle counts ~0.9 M regional
plus ~0.4 M near patches. Horizon ring remains at 30 km beyond the dataset
extent, so it is not occluded or z-fought by the near mesh; fog bridges the
seam where terrain ends.

**Water.** 153 polygons triangulated offline (earcut) at dataset
NAVD88-approximate surfaces: Puget Sound/Elliott Bay at 0.0 (negative source
values clamped), Lake Union and Lake Washington 4.98 m, Ship Canal cuts 4.98 m,
Duwamish 1.63/5.32 m. 15,250 triangles. 472,509 DEM cells are masked water; no
terrain triangle is emitted inside the mask. Flat shaded surface with fog; no
cloud/water z artifacts after removing the speed-line/precip overlays from this
mode. Piers/harbor detail are out of scope.

**Roads.** All 29,833 SND segments are draped: each segment samples the DEM at
its endpoints and becomes a ribbon at DEM height +0.35 m. Widths from SND
segment type (15/11/8/7 m; documented default 7 m). 482,348 vertices in the
global road mesh. Drape audit over 85,981 sampled segment points: 0.0 m error
against the DEM grid. Sidewalk bands were not added (streets only).

**No-buildings gate.** `--seattle-s2` loads terrain, water and roads only; no
CityPack building meshes are spawned.

**Boot and streaming.** Geology setup (DEM, water, 60 m regional mesh, 64 near
patches, global roads) completes in **177–181 ms**; first rendered frame follows
immediately. Road-neighborhood descent still runs per frame: downtown
**2 county / 2 city / 626 neighborhood** evaluations, Bellevue **2/2/807**,
far south **2/0/0**. HUD reports `road nbhds` and level evaluations. Dynamic
near-tile/road chunk spawn is not exercised this round: the 30 m near terrain
is prebuilt for the S2 pose set and roads are a global draped mesh; the
hierarchy still gates the road-neighborhood evaluation count. This is the
remaining S2.x streaming item.

**Captures.** `s2_capitol_climb.png`, `s2_elliott_rainier.png` (Rainier over
the water), `s2_queen_anne.png`, `s2_lake_bellevue.png`, `s2_sodo_flats.png`,
`s2_steep_overcast.png`, `s2_montlake_cut.png`, `s2_waterfront_duwamish.png`,
`s2_elliott_dawn.png`, `s2_elliott_dusk.png`.

**Audits.** Street drape error 0.0 m; LOD seam height delta 0.0 m at shared DEM
cells; water-mask/terrain exclusion checked in the offline build; Rainier
silhouette remains from S1c. Frame cost per pose was not separately instrumented
(boot only); no claim is made beyond the setup number.

**Honest gap.** Dynamic terrain/road chunk streaming, explicit skirt geometry,
sidewalk bands and pier detail are not delivered; terrain is smooth-shaded and
road ribbons are unfaceted placeholders. These are S2.x/S3 items, not passed off
as complete.

## S2.1 — dynamic geology streaming, skirts, frame-cost baseline

**Terrain streaming.** 30 m DEM is tiled into **960 m chunks** (32×32 cells),
loaded/unloaded from the camera through the hierarchy's leaf budget:
load radius 2,800 m, unload 3,400 m (hysteresis), 2 chunks/frame and a 6 ms
budget. Boot spawns the 3×3 downtown chunks; everything else streams. Far
terrain is the 60 m regional mesh; the 30 km horizon ring owns the far field.
Every chunk carries **vertical skirts** (8 m, four edges) at the LOD/chunk
boundary. HUD: terrain loaded (+loads/−unloads), roads, evals and per-frame
build ms.

**Road streaming.** All 29,833 SND segments stream per loaded neighborhood:
load radius 3,200 m, unload 3,800 m, 1 chunk/frame, 6 ms budget; only candidate
neighborhoods are read from CityPack. HUD reports road chunks (+/−) and the
county/city/neighborhood descent evaluations.

**Gauntlet.** Four regimes 15/30/100/650 m/s, 3 s each, cumulative path with
end-point captures (`gauntlet_15ms.png` … `gauntlet_650ms.png`) showing intact
terrain and counters. Chunk-boundary hover capture `s2_chunk_hover.png`
(terrain 32 / roads 66, 2/2/626). Hero-speed seam behavior is in the 650 m/s
capture.

**Per-pose frame cost (S3 baseline).** Measured via the HUD/instrument fallback
(the `--perf` walker remains non-emitting). Steady samples after 3 s settle;
average/p99 in ms on this Vega 8 machine:

| Pose | avg ms | p99 ms |
|---|---:|---:|
| Capitol climb | 39.97 | 50.53 |
| Elliott/Rainier | 39.00 | 47.71 |
| Queen Anne | 36.12 | 42.58 |
| Lake Washington | 41.36 | 46.89 |
| SODO | 35.54 | 40.59 |
| Steep overcast | 41.11 | 47.61 |
| Montlake | 42.33 | 52.11 |
| Duwamish waterfront | 37.55 | 56.12 |
| Elliott dawn | 42.04 | 52.52 |
| Elliott dusk | 36.10 | 40.84 |
| Gauntlet 15 m/s | 37.82 | 244.30 |
| Gauntlet 30 m/s | 38.13 | 47.64 |
| Gauntlet 100 m/s | 37.91 | 50.59 |
| Gauntlet 650 m/s | 38.18 | 54.92 |
| Low orbit | 40.20 | 130.15 |
| Chunk hover | 39.45 | 47.05 |

This is the baseline S3’s buildings will be judged against. The 15 m/s p99
includes the first-chunk/startup spike (244 ms), while steady regimes stay
40–56 ms; low orbit’s 130 ms is the current p99 stress result. No chunk swap exceeded the 6 ms build budget in the HUD (`tile 0.0–0.4
ms`), but the first regime shows the expected startup outlier.

**Tests.** Terrain chunk budgeting/hysteresis constants, skirt vertex presence,
road-neighborhood chunk round-trip and removal (unloaded absence), DEM spot
elevations, water-mask/terrain exclusion, road drape, plus the S2/S1c suites.
Honest gap: the Capitol perf row and a truly steady 15 m/s p99 remain open; the
walker is still broken and the fallback numbers are the committed baseline.

## Seattle S3 — buildings round (current status)

**Style pack.** `assets/styles/seattle_pack.json` defines district grammar driven
by CityPack ancestry + height: Downtown/Bellevue CBD (steel/glass, 82% glass),
Pioneer Square (masonry/stone low-rise), Ballard/Fremont/Georgetown (timber
low-rise), Capitol Hill/Queen Anne/residential belts (mixed masonry/timber),
SODO/South King (concrete grid), Tacoma (masonry/concrete), Eastside (mixed).
Epicenter is Downtown/Columbia Center at world (0,0). Completeness audit: 1096 /
1096 neighborhoods mapped (default rule for unmatched Tacoma South King names),
report in `datasets/seattle/style_pack_report.json`.

**Streaming generation.** Buildings ride the CityPack neighborhood stream:
radius 650 m load / 950 m unload, one neighborhood per frame. Shells (up to 6
per chunk within 120 m) use `spawn_building_world_lod(..., Shell)` with
footprint-local specs and the pack’s family/facade; farther buildings in the
chunk become one merged LOD box mesh. Placement y = terrain DEM at the centroid
+ 0.03 m. HUD reports building count alongside terrain/roads/evals.

**S3 frame cost (same poses, buildings streaming).** `datasets/seattle/s3_perf.json`:

| Pose | avg ms | p99 ms | vs S2 baseline |
|---|---:|---:|---:|
| Elliott/Rainier | 73.27 | 87.40 | +1.9× |
| Lake Washington | 101.19 | 110.45 | +2.4× |
| Montlake | 177.97 | 181.38 | +4.2× |
| Duwamish | 175.14 | 196.90 | +4.7× |
| Elliott dawn | 130.58 | 133.99 | +3.1× |
| Elliott dusk | 133.40 | 144.74 | +3.7× |
| Queen Anne | 250.00 | 250.00 | 6 fps clamp |
| SODO | 250.00 | 250.00 | 6 fps clamp |
| Steep overcast | 250.00 | 250.00 | 6 fps clamp |
| Capitol | 250.00 | 250.00 | 6 fps clamp |

The near-field cost roughly doubles to quadruples, and four poses hit the
6-fps clamp; the 650 m/s gauntlet with buildings was not completed. Levers:
tighten the 650 m load radius, cut shells per chunk below 6, and replace the
per-neighborhood box merge with a distance-decayed far shell. Boot with geology
plus buildings: ~90–135 ms geology setup, first frame dominated by shell
creation.

**Damage smoke.** `--seattle-s3-damage` tiers a loaded shell and fires 20
bullets: `b75134`, 20 shots in 0.4 ms, journal head 0.00; capture
`captures/s3_damage_tower.png`. The wound did not register in this run, so the
damage trio, MaterialFamily gut checks, full persistence cycle, and repeat-fire
pass/fail gate are **not delivered**.

**Captures.** S2 poses re-shot with buildings (some poses show the merged LOD
boxes rather than shells), damage smoke. Queen Anne/SODO/steep/Capitol still
show geometry/culling gaps. Honest verdict: style pack + streamed generation
scaffolding are in place, but the S3 visual/damage/perf gate is not met; no S3
tag should be inferred from this section.

## Architecture invariants — the Shell Doctrine (Seattle)

These three clauses are binding for Seattle and all future rounds. They are
cited by name; any performance lever is evaluated against them before it is
considered.

1. **Buildings are textured hit-box shells until damaged.** The near-field
   render is shells (wall/trim/roof meshes + footprint collider) plus
   elevation-anchored LOD proxies. No interior lattice, slab/column/collider
   per cell, or guts may exist in the pristine near field.
2. **No mass spawn-in of destructible skeletons, ever** — at load or streaming
   time. The zero-structure-graphs-at-load assertion is the enforcement point.
   Shells are legitimate; skeleton material is not. The current S3
   “6 shells per chunk” behavior must be audited against this clause: the six
   are shells, so the cap is allowed **only if** no support graph is built.
3. **Mass destructible escalation occurs only under mass-damage events**, which
   run through the admission machinery as a scheduled wave in bullet time —
   never as a synchronous per-building spawn.

### Audit result (this amendment)

- Clause 1: S3 spawns `BuildingDetail::Shell` and proxy boxes; wall/trim/roof
  visuals are correct. **However**, the dependency’s
  `ensure_structure_runtime` builds a `SupportGraph` for every `Added<Building>`
  regardless of `DetailLevel`. The S3 load assertion in `src/geology.rs`
  (`Structures::iter().count() == 0`) therefore **fails**: the resting state
  currently contains skeleton graphs. Clause 1/2 are **NOT honoured**.
- Clause 3: the S3 damage smoke applies 20 impacts synchronously; it is not a
  scheduled admission wave. **NOT honoured** as a mass-escalation path.
- The 6-shell-per-chunk cap is therefore **rejected as a compliant lever**:
  shells are allowed, but the dependency’s added-building hook turns each into
  a skeleton graph. Raising the cap, distance-decaying shells, or any lever
  that increases shell/skeleton count is rejected until
  `ensure_structure_runtime` skips `DetailTier::Shell` and the load assertion
  passes. Tighter radii alone would reduce the count but not fix the invariant.

**Enforcement status:** the assertion is committed and currently fails on
`--seattle-s3`, so the non-compliant path is loudly blocked rather than
silently accepted. S3 is not a valid baseline for lever work until the
dependency hook is corrected.

### S3.1 fix — shell skeletons deferred to tier-up

**Dependency fix** (`destructibles` branch `iteration-29a3`, tag `manhattan-s3b`
@ `520c85e7`):
- `ensure_structure_runtime` now queries `DetailLevel` and skips
  `DetailTier::Shell` (`crates/bevy/src/structure.rs`).
- `record_pristine` no longer calls `ensure_runtime_for`; pristine recording
  cannot construct a skeleton (`crates/bevy/src/streaming.rs`).
- `begin_detail_upgrade` calls `ensure_runtime_for` at Shell → Structural
  promotion (`crates/bevy/src/building.rs`), matching the pilot/standalone
  damage path.
- Manhattan pins `manhattan-s3b` and the lockfile records `520c85e7`.

**Enforcement now green.** `src/geology.rs` asserts `Structures::iter().count()
== 0` after every streamed building chunk and after the full S3 load; the
`--seattle-s3` run passes. Out-of-range/unloaded partitions contribute zero by
construction (no chunk read → no building spawn → no graph).

**Skeleton-tax isolation (same poses, fix alone, no levers):**

| Pose | pre-fix avg ms | post-fix avg ms | Δ |
|---|---:|---:|---:|
| Elliott/Rainier | 73.27 | 44.40 | −39% |
| Lake Washington | 101.19 | 45.04 | −55% |
| Elliott dawn | 130.58 | 42.86 | −67% |
| Elliott dusk | 133.40 | 42.22 | −68% |
| Montlake | 177.97 | 72.87 | −59% |
| Duwamish | 175.14 | 88.92 | −49% |
| Capitol/Queen Anne/SODO/steep | 250 | 250 | unchanged (load stall) |

The contraband was costing 30–55 % on the poses that were not clamped.

**Still not passing:** the Seattle damage journal remains 0.00 after the
20-shot smoke despite tier-up; the MaterialFamily trio, persistence cycle,
repeat-fire gate and buildings-in gauntlet are not delivered. Levers were not
started because item 2 (damage) is not green. `seattle-s3b` is **not tagged**;
`seattle-s3` remains the prior (non-compliant) tag.
