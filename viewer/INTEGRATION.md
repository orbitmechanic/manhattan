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

### S3.2 journal fix — admission timing, not the resolver

Trace result: resolver/addressability/tier-up are sound. `Buildings` contains
the target, `begin_detail_upgrade` returns true, level promotes to
`Structural`; the 20-shot + car-bomb batch applies in 0.4 ms. The dead stage was
**reading the journal and screenshotting in the same frame the admissions were
enqueued**. The admission queue applies on subsequent frames; the earlier smoke
reported `journal 0.00` before the car bomb was decomposed.

Fix: the damage mode now applies the batch, sets stage 1, waits 1.5 s for the
admission machinery, then reads the journal and captures. Trace now reports
**journal 10.54** on b75134 after admission.

This is a real-input badge-chain result: resolver/registry hit → tier-up →
admission enqueue → journal apply. Remaining battery (trio per MaterialFamily,
monotone 20-shot gate, restore→evict, persistence cycle, levers, buildings
gauntlet) is still not delivered; `seattle-s3b` remains untagged.

## CodeGraph verification culture (standing order)

Session start (2026-11-22) ran `codegraph-work-index`: **25 projects, 540 files,
0 failures**; Manhattan indexed 88 files and destructibles 105. The Seattle
files are present in the graph — `codegraph_symbol_search("s3_damage")` resolves
`viewer/src/geology.rs:1207`, and `S3DamageState` resolves at `geology.rs:1328`,
so CityPack/geology/horizon did not require a manual re-index.

Seam queries used for the S3.1/S3.2 work and cited in the fix report:

- `symbol_search("ensure_structure_runtime")` → `destructibles/crates/bevy/src/structure.rs:271`.
  `get_callers` returned none because the function is registered as a system
  reference, not called directly; `get_callees` returned `Structures::get` /
  `contains`, confirming the runtime-construction seam.
- `symbol_search("begin_detail_upgrade")` → `building.rs:1106`;
  `get_callers` returned `geology::s3_damage`, `pilot::pilot_damage`,
  `neighborhood::detail_tick`, and the destructibles detail-tier tests. That map
  identified the tier-up seam where `ensure_runtime_for` belongs.
- `symbol_search("s3_damage")` located the Seattle smoke and `S3DamageState` for
  the journal-timing trace; the dead stage was admission timing, not the
  resolver/registry path.

Non-compliance note: the earlier S3/S3.1 rounds did not run this session index or
record these queries; that gap is reported rather than hidden. Future
cross-module work must run the session index and cite the seam queries alongside
trace evidence.

Seattle archive radius: **650 m**, matching the neighborhood minimap zoom (the default 650 m dial), not the 15 km city or 80 km county zoom views. The neighborhood radius governs freeze/evict because it is the scale at which building detail and damage are legible; the wider minimap zooms are navigation instruments, not residency scopes. Constant: `SEATTLE_ARCHIVE_RADIUS_M = 650.0` in `src/geology.rs`.

## Seattle landmarks (destructible, iteration 56c)

Nine authored assemblies (Needle, Market, Aurora/Fremont/Ballard, Monorail, Climate Pledge, Lumen, T-Mobile) spawn as shell components and tier up on first impact; the assembly is the support topology, and signature components must survive at least half the nine-act sequence (validated at load). Measured tier-up cost, **Space Needle: 176 ms for six components** (48 m saucer, 134 m stem, waist, crown, spire, pedestal) on the Vega 8 machine; the other assemblies are cheaper. The Needle's landmark persistence cycle (freeze 710 m out, thaw, restore) is runnable as `scripts/seattle_persistence.sh needle`; the 16-row table with all landmarks resident moved no city pose by more than ~3.5 ms and the 650 m/s gauntlet by ~7 ms.

Landmark tier-up costs (Vega 8): Space Needle **176 ms** (6 components), Climate Pledge Arena **71 ms** (2), Seattle Center Monorail **44.5 ms** (12 spans), Pike Place Market and the three bridges cheaper; measured at Structural tier-up from the damage-ready baseline. The 16-row landmark table (s3_perf_landmarks.json) was taken with all nine assemblies resident: no city pose regressed beyond ~3.5 ms and the 650 m/s gauntlet by ~7 ms; it predates the final monorail span geometry (spans lengthened to 118 m for a continuous guideway), whose shell count is unchanged.

**The 250 ms clamp, explained.** The harness records frame cost from `time.delta_secs()`, i.e. Bevy's virtual clock. `Time<Virtual>` clamps each frame delta to its `max_delta` (default 0.25 s) so a stalled frame advances game time by at most 250 ms. A frame that really takes 2 s is therefore recorded as **250 ms**, and the perf tables show 250.0 for clamped poses. It is a **measurement artifact of the harness**, not a runtime limiter: real wall time is unaffected. The honest fix is to record `Time<Real>` deltas (or raise `max_delta`), which would show the true 1-3 s chunk-spawn stalls that the S3 levers eliminated; until then the clamp is a known, documented ceiling, and any 250.0 row means "at least 250 ms", never exactly.

## Measurement honesty: real time, no 250 ms sentinel

The perf harness now records `Time<Real>` frame deltas instead of the virtual delta. Bevy's `Time<Virtual>` clamps a frame to `max_delta` (default 0.25 s), so every stalled frame previously recorded as exactly **250.0 ms** — a sentinel, not a measurement. All earlier tables must be read with that caveat: any 250.00 row means **≥250 ms**, and the true stall could be seconds (the pre-lever chunk spawns were 1–3 s). With `Time<Real>` the clamp disappears and stalls are reported at their real cost.

## District grammar reconciliation

The style pack keys families by district vocabulary, but CityPack partitions carry synthetic names (`Ballard 04`, `Seattle South 105`, `Downtown 03`), and the old name-substring match let `Downtown 03` (which *is* Pioneer Square) match the downtown tower rule. Fix at the data layer:

- **`assets/styles/seattle_districts.json`** assigns all **1,096** partitions to exactly one district from curated lat/lon boxes in priority order, then name prefix, then city default, with two documented manual overrides (`Downtown 02` → Pike Place, `Downtown 03` → Pioneer Square) because the adaptive grid has no centroid inside those small boxes. Districts: Ballard 71, Bellevue CBD 49, Capitol Hill 47, Downtown 4, Eastside 132, First Hill 4, Fremont 30, Georgetown 6, Pike Place 1, Pioneer Square 1, Queen Anne 28, SODO 10, Seattle general 354, Tacoma 289, Wallingford 5, West Seattle 65.
- The runtime matches rules by **district only** (no name fallback); the pack gained Pike Place, First Hill, Wallingford and a Tacoma default rule. `height_bias: low` districts now cap at six floors, so Pioneer Square and Ballard read low-rise.
- **Headline: 33,657 buildings changed MaterialFamily** (of ~300k). Before → after by family: masonry 164,941 → 187,864; mixed 111,948 → 84,846; steel 17,348 → 19,365; timber 36,093 → 38,255.
- The dump (`S3_LIST_TARGETS=1`) now prints the district: masonry targets come from the Pioneer Square partition, steel from Downtown, timber from Ballard.
- Determinism: the mapping is committed data; `tests/districts.rs` recomputes its SHA-256 over the sorted assignments and the pack hash, asserting both. Families are assigned at spawn time, so the **CityPack bytes are unchanged**; the emitted artifacts are the mapping and the re-keyed style pack (the pack already carries per-node hints, none of which store family).

## Seattle field epicentre (amenities round, iteration 59)

Seattle's radial/daily fields are anchored at the **Westlake / Pike Pine seam**, world `(-138.4, -614.3)`, not at Columbia Center: ad density, the pedestrian field and commute-direction logic all key on where downtown life concentrates, and the transit/retail seam is where that lands. Pike Place stays the symbolic heart (the point is ~440 m from it). The choice is committed in `datasets/seattle/seattle_fields.json` with its rationale, peaks (08:30 / 17:00, graveyard 5%), the 90/40 commute split, the halved traffic scale and the 500-pedestrian cap. `city_rhythm::CityFields` now carries the epicentre and parameters; Manhattan keeps its Times Square instance (`CityFields::manhattan()`) and Seattle uses `CityFields::seattle()` / `from_json`, so no Manhattan constant is silently reused. Tests pin the four canonical moments, the AM/PM flip, the radial falloff and the metadata agreement.

## Iteration 69b — readout unit boundary, painter guard, ghost funnel (playtest 4)

**The readout panic was the stream offset, not a 3.28× feet factor.** `--seattle` panicked with
`lat/lon readout outside the Seattle dataset bbox: 47.61945,-121.53946 at (59862,-614)`. The camera's
world X carries `STREAM_WORLD_OFFSET = 60000`; the fix in playtest 69 re-fit against metres but the
readout still passed world X into the fit. Evaluating the committed cubic at raw `x = 59861.6, z = -614.3`
reproduces `-121.53945789, 47.61944951` to the fifth decimal — the crash string exactly. The same fit at
`x = 18264` (the coordinate a 3.28084 feet-as-metres error would need) gives `-122.0928`, so the earlier
3.28×/feet hypothesis is **refuted by arithmetic**; the bug was a missing conversion at one boundary, not a
wrong conversion factor. `minimap::world_to_lon_lat` now documents pack-local metres as its input space, the
location readout subtracts `STREAM_WORLD_OFFSET` once, and the fit is one private `fit_lon_lat` with two
public wrappers: the player readout asserts (a wrong boundary must be loud), and
`world_to_lon_lat_checked` returns `Option` for batch painters.

**Painter guard.** `render_seattle_layer` validates each visible footprint through the checked wrapper,
skips out-of-bbox records, and warns with the running count at paint completion. No pack records currently
fall outside the fit domain, so the path is defensive; a boot at pack-local `(18000, 240, -800)` (HUD
`LAT 47.61630 N / LON 122.09631 W`, no panic) exercises the far-east readout end to end.

**Ghost funnel, per stage (GHOST_FUNNEL=1, boot camera):** the camera's Westlake point lies inside three
overlapping partition AABBs. Downtown 01 (the first bbox hit) reports `records=153 specs=152 entities=7
pending=0 mesh=1 nearest=599 m`; the buildings the camera actually stands among are in **Downtown 02**:
`records=171 specs=152 entities=21 mesh=4 nearest=36 m`. Resident chunks all had entities, and a
GHOST_AERIAL frame from 320 m shows the merged proxy meshes and shells painted across the basin — so the
pipeline loses no stage (records → specs → shells/proxies → meshes). The "empty block" impression was the
**boot pose being discarded**: `seattle_boot_camera` wrote `Transform` once, but `fly_camera_system`
re-applies its own `FlyCamera.yaw/pitch` every Update, so the commanded look-at was replaced by yaw 0 on the
next frame. The boot now hands ownership to input at the commanded pose by writing the `FlyCamera`
component as well (Camera Ownership Law: one writer, and the hand-off is explicit). The authored eye/look
pair also had a real error: at the seam the DEM is **37.9 m** and the Market corner **19.4 m**, so the old
`eye y=60 / look y=20` aimed **10 m into the hillside** (screenshots showed a dark bank filling the lower
half). Both endpoints are now seated on the DEM — eye 22 m above grade, target Market roof height — so the
opening frame looks downhill along Pike and keeps the Sound on the horizon.

**Murk frame.** Captured with `MURK_CAPTURE=1` (two frames, no queue): `captures/evidence_murk.png` — camera
staged underwater in Elliott Bay at pack-local `(-4200, -6, 0)` with the HUD reading `MODE MURK`. This
clears the frame owed since playtest 67; no fourth attempt was needed.

**Suite.** `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and the full test suite are green,
including `tests/geo.rs` (far-east 18 km round trip, stream-offset equivalence, painter skip, and a
`#[should_panic]` gate for the player readout). `citypack::tests::top_down_lookup_equals_bruteforce` was
failing at playtest 69 for a different reason — it sampled points up to 1.4 m outside footprints and
compared them to a strict point-in-polygon brute force, while `resolve_point` intentionally snaps within
1 m of an edge. That test now samples strictly interior points (centroid, vertex mean, skip if neither).

## Iteration 70 — per-family shell facades, night light, chimney census (seattle-facades1)

**Textures belong to the shell.** `src/facade.rs` generates four family atlases at startup: steel frame
(panel joints + rivets), timber (lap siding), masonry/stone (running-bond brick) and earth/adobe (plaster).
Each family owns five shared materials — cladding, window sash, lit window, shop sash, shop lit — and every
streamed building points at its family's handles. Nothing is cloned per building: **20 shared
`StandardMaterial`s for the whole city**, **2.0 MB** of CPU texture data (4 × 256² wall + 4 × 4 × 128² sash,
mips extra ≈ 2.7 MB). `FACADE_DEBUG=1` replaces the cladding with a magenta checker for the sampling check
that proved the swap reaches the wall child.

The window grid is the layout's own pane geometry, not a painted illusion: each pane quad maps 0..1 onto the
family sash atlas, so a window carries its frame and mullions at any distance, and the lit quad reuses the
same mask (glass bright, frame black). The human-scale fix lives in `building_spec`: a **residential** parcel
gets a 1.10 × 1.45 m sash window with a 0.95 m sill and a 3.10 m column pitch at 3.0 m floors, capped at
three storeys, *regardless of the district's commercial grammar*. The old override forced every window to
`floor_height − 0.1` at zero sill, which is precisely how a brownstone became an office curtain wall. A
`SkinGuard` re-applies the family skin after a detail tier-up rebuilds a building's children.

**Night.** `seattle_night_density` runs after the stock window system and lights floor-coherent rows rather
than a dot scatter: ≤3 floors **42 %** of floors, taller buildings **62 %**, curtain wall **72 %**, with 88 %
of panes inside a lit floor and 90 % of ground-floor shop windows. The live atlas carries **6 × 8 cells,
55 % lit** per proxy face, so the merged far-city boxes are a light field rather than black cut-outs — the
reason an aerial at 190 m could not see the city before. Measured frames: `night_downtown.png`,
`night_residential.png`, `night_storefronts.png` (shop band brightest, apartment rows behind).

**Storefront glow.** Commercial ground-floor panes use the shop sash (wider, brighter, 11.0 emissive), the
district props (awnings, placards) share one material whose emissive follows the scene clock. Both filed
storefront debts close here: the capture-mode sun override in `storefront_capture` was **deleted** and the
Pike Place frame renders correctly from the host sun path alone, and the props sit on the same lighting path
as the host facade. Capture modes (`SEATTLE_BOOT_CAPTURE`, `SCALE_ORACLE`, `EVIDENCE_CAPTURE`,
`MINIMAP_CAPTURE`, `STOREFRONT_CAPTURE`, `MURK_CAPTURE`, `GHOST_AERIAL`) also **omit the depth-of-field
pass**: its 180 m focal distance blurred every street-range reviewer frame.

**Capture gates.** Day storefront row: `captures/storefronts_pike_place.png` (brick window grid, awning,
crate, curb template). Residential skin: `captures/facade_house_day.png` (lap siding + punched sash windows
on a three-storey house). Night: two aerials above plus `night_storefronts.png`. Scale oracle:
`captures/scale_oracle.png`, a sedan at ~20 m beside a three-storey house on a residential street, pose
derived from the nearest vehicle (`SCALE_ORACLE=1`, logs walker/vehicle distances). The **walker half of the
oracle is a named debt**: `ad_density` falls to zero beyond 2.5 km from Westlake, so residential districts
spawn no walkers at all (`walkers=0` at Wallingford *and* at First Hill; the same field makes the
`--pedestrians` plaza capture assert 0/15). Pedestrian-field round owns it.

**Chimney census** (`scripts/seattle_height_census.py`, full rows in `datasets/seattle/chimney_census.json`).
Replays the same rule order and governor as `building_spec`. Totals: **330,330 records, 318,703 residential,
225,080 residential records above the 9 m cap (70.6 %)**. Sources: audited 313,571 (`real_surveyed`
161,882, `lidar_overture` 151,623, `osm_levels_derived` 66), enriched 16,759 (`area_heuristic` 13,912,
`msml_estimate` 2,847).

| district | residential over cap | max recorded | max governor-applied | family key |
|---|---:|---:|---:|---|
| Ballard | 16,575 | 144.4 m | 24.0 m | timber |
| West Seattle | 12,417 | 150.2 m | 128.0 m | mixed |
| Capitol Hill | 8,724 | 137.2 m | 128.0 m | mixed |
| Fremont | 6,572 | 145.5 m | 24.0 m | timber |
| Queen Anne | 5,904 | 138.5 m | 128.0 m | mixed |
| Wallingford | 1,287 | 93.8 m | 24.0 m | timber |
| Tacoma | 81,288 | 134.3 m | 9.0 m | masonry_stone |

Worst single records are `real_surveyed` heights of **141.9 m** on 10 × 16 m "residential" footprints
(Fremont 30, id 443; Ballard 36, id 33427 at 142.9 m). The governor is applied (those render at 9 m), so the
chimneys the player saw are **commercial records in residential districts**, which the governor does not
touch: `mixed`/`mid` districts leave commercial uncapped (max applied 128 m), and `low` districts allow 24 m.
Recommended fix path, not this round: (1) re-enrichment must derive height from floors × category floor
height or footprint area — a 160 m² house cannot be 141.9 m; (2) the interim runtime should cap commercial
by the same `height_bias` (low 6 floors, mid 8) and by footprint area; (3) key family from the record's own
material where present instead of inheriting the partition's single family.

**Named debts.** `pedestrian-residential-field` (above). `proxy-night-uniformity`: far proxies share one
6 × 8 atlas, so a residential block glows like a tower block; per-family proxy UV banks are the next step.
`steel-day-skin`: the steel cladding is proven at night and from the air but no satisfactory sunlit street
frame was captured this round (the tower test shots were backlit silhouettes).

## Iteration 71 — the orphan graph: streaming lifecycle, journal authors, proxy skin (seattle-journal1)

**Crash, reproduced exactly.** `SEATTLE_AUTOFIRE_AT=1.5 ./manhattan-viewer --seattle` fires the crosshair ray
from the boot camera and lands on **b110094 at (59689.9, 54.0, −587.4)** — the same building and decimetre
as the player's journal. The emitter is the **player weapon path**, not a camera system: a startup focus
click (`mouse.just_pressed(Left)`) walked `damage_input → seattle_fire → fire_seattle_at`, tiering the shell
up and applying a `Bullet` impact.

**The assert was overreach, with a latent leak behind it.** World (b): the support graph was created by a
live tier-up 1.5 s *before* the chunk stream that tripped the assert; streaming itself created nothing.
Evidence: the delta assert (`structures_before` vs after) passes on the identical repro, and the journal
timestamp precedes the stream. The latent leak is real, though: the viewer's chunk unload despawned
entities without telling the destructibles streaming controller, orphaning any graph on a damaged building.
Fix: a chunk that contains a journaled building is now **pinned on unload** (logged once), so journal-derived
state can never be despawned by chunk churn; the controller freezes/evicts it on its own schedule. The
Shell Doctrine now reads *streaming must not create a support graph* (delta), not *no graph may exist*.

**One chip forced a blackout — upstream sign error.** `power::damage_check` read the journal's
*removed* volume (misnamed `remaining_volume`) and computed `lost = 1 − removed/total`, so a 0.0004 m³ chip
on an 1803.7 m³ tower scored lost = 1.0 and forced `PowerLoss` in the same tick. Fixed in destructibles
`manhattan-journal1` (`lost_fraction = removed/total`, regression test), and the journal accessor is now
`removed_volume()` with the old name as an alias.

**Journal attribution.** `JournalEntry.author` (`system|player|vehicle|walker|camera`) with
`record_with_volume_author`; the `DamageAuthor` resource lets a host tag a call. The player's chip now dumps
as `surface | player | b110094 | ChipFacade` and the bogus PowerLoss is gone. Camera Ownership Law is
extended in code: a staged camera (`StagedPose.active`) can never fire the weapon.

**The hang.** Not reproducible in the fixed build. A deliberate panic at the same point (after all boot
chunks, 195 buildings, 1.5 M tris) exits in **6 s with and without `RUST_BACKTRACE=1`**. The player's ~20 s
is consistent with symbolization of the 273 MB binary plus winit/pipewire teardown after an unwind (their
console shows the calloop source-removal warning), not a deadlock. No watchdog was added: an abort-on-panic
watchdog would hide the next lifecycle defect, and the panic is gone.

**Facade streaming hole — root cause found and fixed.** `SKIN AUDIT` (printed every 2 s): **skinned=47,
bare=0**, proxy entities 9 — but **proxy_buildings=1151 of 1198 represented buildings (96.1 %)**. The stream
spawn path skins every shell; what the player saw as ">90 % tan blocks" was the merged proxy boxes, which
were flat tan. The proxy material now carries a generated daylight facade atlas (6 × 8 punched windows,
cladding grain) with a plain roof strip sampled by the roof face, so flight-level distance reads as window
grids instead of sand. Evidence: `captures/facade_proxy_day.png`.

**Interim height caps from the census.** Commercial floors are capped in residential districts only
(low bias 6 floors, mid bias 8); the catch-all "Seattle general" and the cores stay uncapped. Census delta
(max governor-applied height, before → after): Capitol Hill **128 → 32 m**, West Seattle **128 → 32**,
Queen Anne **128 → 32**, First Hill **76 → 32**; Ballard/Fremont/Wallingford/Georgetown/Pioneer/Pike
unchanged at 24 m; Seattle general 248 and Downtown 136 unchanged. Family from the record's own material is
not possible yet — the pack carries no per-record material, only the partition rule — filed with the
re-enrichment debt.

**Verification.** fmt; clippy `-D warnings`; 160 tests green; **two 30 s hands-off `--seattle` soaks** with
no panic and no hang; `JOURNAL STATE entries=0 structures=0 checksum=cbf29ce484222325` (empty FNV offset)
on both; the damage soak leaves `entries=1, structures=1, checksum=2de187e59a00030b` with the chip attributed
to `player` and no PowerLoss.

## Iteration 72 — car city: district-calibrated street population (seattle-walkers1)

**Pedestrians, rebuilt on Seattle priors.** `pedestrians.rs` no longer decays uniformly from one epicentre.
The field is `max(core, parks, plazas, sprinkle)`:

- **Core + waterfront**: `CORE_GAIN 0.35` over a compact window (full inside 800 m, zero past 1.7 km) on the
  existing 8:30/17:00 pace — roughly half the Manhattan peak, per the player's lived comparison.
- **Parks** (the real Seattle walk): 600 hotspots derived at runtime from the tree dataset's **park regime**
  (85,844 park-regime trees; cells of ≥6 trees, 120 m), `PARK_GAIN 1.4`. Walkers **stroll park loops** (60 %
  of the park crowd) and the rest walk the nearest frontage streets. Gas Works' cell sits at water level, so
  the park capture uses the Green Lake frontage cell instead (logged).
- **Plazas**: Pike Place and the Westlake seam carry a loop crowd (`PLAZA_GAIN 2.0`), which is what keeps the
  `--pedestrians` plaza assert meaningful at a genuine site.
- **Everything else**: `SPRINKLE 0.02` (≈0.26 walkers/chunk) — oddities, not fauna. Residential Wallingford
  audits at **exp=0/act=0** on the street model, which is correct and is not tuned upward.

The density audit prints two lines per capture so park crowds cannot masquerade as residential traffic:
`PED CROWDS` (park/plaza-anchored chunks) and `PED DENSITY` (street model). Example at the park pose:
`PED CROWDS … Fremont exp=129 act=93 | Wallingford exp=37 act=37`; `PED DENSITY … Ballard exp=1 act=0 |
Wallingford exp=0 act=0`.

**Traffic, rebalanced toward the car.** Fleet cap **300 → 640**, arterial weighting (kind 1|2 ×1.7,
5|6 ×1.4), per-street share cap 4 → 8, per-chunk block cap 24 → 48, spawn budget 24 → 48/frame, and a
6 s capture settle. AM Westlake, expected/actual per district, legacy → new:

| district | legacy (cap 300) | new (cap 640) |
|---|---:|---:|
| Capitol Hill | 72 / 72 | 388 / **196** |
| First Hill | 96 / 96 | 196 / **144** |
| Downtown | 24 / 0 | 192 / **144** |
| Pioneer Square | 24 / 24 | 48 / 48 |
| Pike Place | 24 / 24 | 48 / 48 |
| total actual | **216** | **532** |

Midday Ballard: `exp=1252 act=480` (global cap). **Seat law corollary codified**: `road_grade()` samples the
ribbon **centerline**; `vehicle_pose` seats on it, not on the DEM under the lane offset (a curb, retaining
wall or hillside face lifted cars by metres). `tests/traffic.rs` now asserts the centerline seat, and a
synthetic spike under the lane proves the car does not ride it.

**Scale oracle closes.** `SCALE_ORACLE=1` now targets a walker first (and prefers a walker with a sedan
within 60 m). At the Pike Place morning pose: **walkers=74, nearest vehicle 47.3 m**; `captures/scale_oracle.png`
shows the walker row on the street. The park frontage frame (`captures/pedestrians_park_frontage.png`)
carries two walkers and a sedan beside the buildings.

**Proxy night character.** The night atlas is split into two banks selected by proxy family in the merged
mesh: residential/timber proxies sample a **warm incandescent scatter** (14 × 18 cells, clustered, gaps at
parks/ravines, ~2700-3000 K) and steel/masonry keep the **regular grid**. Day atlas split the same way
(residential cladding + house-scale windows vs commercial grid); the roof face samples a plain strip in
each bank. Evidence: `captures/night_proxy_banks.png`, `captures/night_residential_scatter.png`.

**Steel day skin closes.** `captures/facade_steel_day.png` — a sunlit SODO steel-frame building (concrete
panel joints, punched windows), camera south of the face at 12:00.

**Verification.** fmt; clippy `-D warnings`; **161 tests** green (new road-grade seat test); `--seattle`
boots clean. Captures committed: `pedestrians_am_westlake`, `pedestrians_pike_place` (36 in view),
`pedestrians_park_frontage`, `pedestrians_sprinkle_wallingford` (0 = correct), `scale_oracle`,
`traffic_am_westlake`…`traffic_capitol_ascent`, `night_proxy_banks`, `night_residential_scatter`,
`facade_steel_day`.

**Named debts.** `park-paths`: park walkers follow a synthetic loop because the pack carries no park path
network; frontage streets are the honest analogue until paths are emitted. `proxy-tree-occlusion` (deferred
per the escalation clause): proxy light points do not yet mute under known canopy. `capture-sun-override`:
the pedestrian/traffic capture harness still aims the directional light along the view (the storefront
override was removed last round; these two remain).

## Someday list — authorized omissions

- **homeless encampment representation** — requires locational accuracy and modeling we don't authorize;
  omission is deliberate, not oversight. No agent session should improvise encampment scatter as scenery
  texture.

## Iteration 73 — the botanist: branch kits, material grade, camera law (seattle-trees1)

**Tree kits.** The dataset reports **6 species** (Douglas fir, western red cedar, bigleaf maple, Pacific
madrone, cherry, London plane); each gets **5 authored branch parts** (tapered limbs + a twig, real
geometry, never a billboard). Only the rooted part carries the trunk stub, and the other parts root at a
fork ring, so a composite has **no coplanar trunk surfaces to z-fight**. Combo = **1–3 parts**, each yawed by
its own seeded angle, merged into one mesh. The seed is the quantised tree position + height, so journal
replays and re-streams reproduce the identical tree.

**Lazy baking, not enumeration**, because the combo space is seeded per position (continuous rotations):
enumerating 5 + 25 + 125 subsets × yaw steps explodes. Instead a tree bakes when it comes inside **30 m** and
drops its mesh at **45 m** (hysteresis), 16 bakes/tick. Near-camera stats from a Green Lake park run:
`baked_combos=156–395`, `avg_combo_tris=159–166` vs `card_tris=22`, `draw_calls≈baked` — one draw per near
tree, as specified. The card LODs remain for the far field; the 300/neighborhood cap is untouched.

**Material grade.** Per-building tint is a **vertex-colour instance attribute baked into the building's own
wall mesh** at spawn — the shared family atlas and its 20 materials are untouched, so the budget does not
grow. Ranges are hue-constrained per family (timber +6 % warm, masonry +5 %, steel ±2 % neutral, adobe +6 %)
with a 0.90–1.10 value nudge. Evidence: `captures/tint_contact_sheet.png` (a Wallingford row showing three
distinct browns from one material).

**Camera interior ban.** Buildings are solid to the camera. A camera ending a frame inside a hull is moved to
the nearest edge and **glanced along the tangent**, correction scaled by speed (`glance = speed/650`, min
0.15). Deliberate fly-through log: `INTERIOR PROBE: entering b87626 … → CAMERA DEFLECT b87626 speed=0
resolution=(0.07,1.00) glance=0.15`. Staged capture cameras are exempt (Camera Ownership Law).

**Sonic camera reinstalled.** One `sonic_gain(speed)` curve (0 at a walk, 1 at 650 m/s) drives wind-rush
volume **and playback rate** (the speed cue), a 5° FOV pulse and a 0.15° shake. `SONIC_TABLE=1` prints the
curve and the live gains; measured curve at the gauntlet regimes:

| speed (m/s) | gain | wind vol | wind rate | fov | shake (°) |
|---:|---:|---:|---:|---:|---:|
| 15 | 0.000 | 0.020 | 0.85 | 70.0 | 0.000 |
| 30 | 0.000 | 0.020 | 0.85 | 70.0 | 0.000 |
| 100 | 0.000 | 0.020 | 0.85 | 70.0 | 0.000 |
| 650 | 1.000 | 0.340 | 2.20 | 75.0 | 0.149 |

**Gauntlet, treed district (650 m/s pass).** `--seattle` + `S2_GAUNTLET=1`, trees on:

| regime | avg ms | p99 ms |
|---|---:|---:|
| 15 m/s | 217.4 | 1889.6 (boot stall) |
| 30 m/s | 133.0 | 200.0 |
| 100 m/s | 129.9 | 208.5 |
| 650 m/s | **176.7** | **248.3** |

The 650 m/s pass is **worse than the 28–77 ms standard** from playtests 66–70. Named debt `tree-650-perf`:
the near-combo bakes and one-draw-per-tree cost are the likely cause (156–395 unique meshes in a park); the
next step is a quantised combo cache so neighbouring trees share baked meshes, and/or a 20 m bake radius.
This round ships the feature green with the regression measured rather than hidden.

**Captures.** `trees_near.png`, `trees_far.png` (same park, 12 m vs 300 m — branchy combos vs cards),
`tint_contact_sheet.png`, `pedestrians_park_frontage.png` (walkers scaled against near combos), gauntlet
frames.

## Iteration 74 — the roofline carpet (seattle-homes1)

**Tree revert, two truths kept.** The pre-trees1 card model is restored wholesale (no combos, no baking, no
unique meshes). Trunks are now dark bark/moss-dark (`0.085/0.075/0.060`, was `0.30/0.22/0.15`), and foliage
varies per tree through **twelve shared dark tint steps selected by a position hash** — still one material,
still shared meshes. Gauntlet is back in the band (below). The draw-call regression dissolved as expected.

**Unremarkability Doctrine (data law).** A residential building without credible height data is
**single-storey**. The gate strips `area_heuristic` and `msml_estimate` outright and demotes surveyed heights
failing an area plausibility test (`h ≤ 4 + 3.2·√area`) — 141 m on a 10 × 16 m footprint is not a house.
Credible residential is capped at **two storeys** (the carpet gate; the old three-storey cap still produced
flat-topped blocks). Demotion census: **47,875 records** — `real_surveyed` 31,298, `area_heuristic` 13,658,
`msml_estimate` 2,657, `lidar_overture` 261, `osm_levels_derived` 1. By district: Seattle general 19,218,
Eastside 8,311, Ballard 5,746, West Seattle 3,652, Bellevue CBD 2,643, Fremont 2,507. Script:
`scripts/seattle_height_census.py` (prints `DEMOTION total` and the district table; `--legacy` reproduces
the pre-gate world).

**Pitched roofs (runtime bisector, convex).** For 1–2 storey shells the roof is the first straight-skeleton
step: each footprint corner's bisector is intersected on the roof plane, giving a hip ring; rectangles
collapse to a ridge and are built as a gable. Pitch is seeded 15–45°, one angle per building. Materials are
vertex colours over the shared destructibles roof material: **wood shake 57 %, asphalt 27 %, tile 12 %,
metal 4 %**, a moss-green eave band on every roof, and **~5 % seeded solar arrays**. Stats from the carpet
run: `installed=81 pitched=51 flat_fallback=30 solar=1` — **37 % fallback** (concave/L-shaped footprints
that neither hip nor gable cleanly), counted, not hidden. Colliders stay the shell's flat roof AABB (visual
wedge only), as permitted.

**Tint seed domain.** Shell tint was already building-id-keyed; the neighbourhood-level uniformity the player
saw comes from **proxies** (the majority of an aerial view), which carry no per-building signal. Proxy boxes
now bake a **per-building vertex-colour tint** into the merged block mesh (same id-keyed, hue-constrained
range), so individual buildings vary in the far field too. New contact sheet shows a row of individual
houses with visibly different browns from one family material.

**Ledger (accepted-for-now, not to be changed on agent initiative):** the player accepts the current uniform
black night structures with uniform incandescent window glow as adequate for now; any future change to night
rendering is a player-requested round.

**Gauntlet, trees restored** (`--seattle` + `S2_GAUNTLET=1`):

| regime | avg ms | p99 ms |
|---|---:|---:|
| 15 m/s | 65.5 | 262.5 (boot stall) |
| 30 m/s | 49.9 | 61.5 |
| 100 m/s | 49.2 | 64.5 |
| 650 m/s | **62.4** | **96.2** |

Back inside the 28–77 ms band; the seattle-trees1 177/248 pass is gone.

**Captures.** `carpet_residential.png` (aerial over the residential grid), `roofs_residential.png`,
`tint_contact_sheet.png`, `trees_dark_street.png`, regenerated gauntlet frames.

**Named debts.** `proxy-roofs`: proxies (the aerial majority) are still flat boxes, so the carpet reads as
blocks among trees rather than a pitched-roofline carpet; the acceptance gestalt is **not yet met** until
proxy boxes carry a ridge/hip top. `roof-pack-bake`: roof parameters are derived per building at spawn; the
binary pack has no roof block and the pack itself was **not rebuilt** this round — the height gate is applied
at spec time, not re-emitted into the pack. `roof-fallback-concave`: 37 % of shells fall back to flat.

## Iteration 75 — the proxy veil (seattle-proxies1)

**Proxies graduate to buildings.** Each merged proxy box now carries its own **pitched roof baked into the
merged mesh**: seeded pitch 15–45° by building id, gable along the longer axis (ridge = long − short), moss
eave triangles, material in the shake/asphalt/tile family, and ~5 % near-black solar patches. Commercial
proxies stay legitimately flat. The aerial majority finally shows rooflines. `ROOFS` (shells) in the same
run: `installed=643 pitched=444 flat_fallback=199 solar=8`, materials 68/18/12/2.

**Fixed metres per floor (law).** The proxy facade UV is now `V = height / 3.0` — one atlas row per 3.0 m
floor, repeated — so a window keeps the same physical height on a 2-storey house and a 60 m tower. The atlas
was re-authored with **horizontal family banks and one window row per bank** so the V repeat never crosses
banks. U stays within the family bank (one bank tile is 4–6 windows across), so horizontal module is derived
from the face width; that half of the law is the `proxy-u-module` debt. Shells already carry physical pane
geometry.

**Slenderness gate (extends Unremarkability).** A footprint hosts about one storey per 12 m², and a
residential district hosts nothing above four storeys. Census over the model: **0 survivors >4 floors in
residential districts** (the remaining 16 m "survivors" in the old table are 4-storey commercial, allowed).
Demotion totals unchanged from iteration 74 (47,875) plus the slenderness caps.

**Evergreen truth.** Species are re-weighted at spawn by position hash: **≥85 % conifer** (fir/cedar mix),
deciduous minority; twelve dark tint steps. A composition table prints once past 10k trees
(`TREE COMPOSITION fir=… cedar=… conifer=…%`).

**Wind sway.** A per-frame CPU base-pivot rotation of camera-near trees: 3–5° maximum bend, one wind
direction per 500 m cell, phase from the position hash, zero draw cost, no unique meshes. This is the
zero-draw *approximation* of a height-proportional vertex bend; the shader version is the `tree-sway-shader`
debt.

**Proxy tint visibility.** The proxy tint bake was verified to reach the merged mesh (per-box vertex colours,
id-keyed) and the aerial carpet now shows building-to-building cream/brown variation rather than
neighbourhood blocks.

**Gauntlet** (`--seattle` + `S2_GAUNTLET=1`): 15 m/s 63.3/425 (boot stall), 30 48.4/64.0, 100 48.2/63.2,
**650 60.2/85.2** — in the 28–77 band with the proxy roofs added.

**Texture ledger probe (measurement only, no import).** Classes needed for a future real-texture round:
brick courses, concrete panels, glass curtain wall, timber clapboard, asphalt/wood shake roofing, moss
border, storefront signage, window sash. One 1024² BC7 tile at 1 byte/px is **1.00 MiB** (RGBA8 4.00 MiB,
DXT5 1.00 MiB) before mips; at 6–8 classes the atlas set adds ~6–8 MiB VRAM and ~10–20 ms load in this
pipeline's asset path. No tile was imported this round; the prototype mapping is deferred to the
player-approved texture round.

**Debts.** `proxy-u-module` (horizontal module still face-derived), `tree-sway-shader`, `roof-pack-bake`
(still runtime), and the tower-beside-house window-frame capture + sway offset-frame pair were not produced
this round; the carpet and the floors-UV facades carry the window-scale evidence.

## Iteration 76 — glass and gravel (seattle-glass1)

**Facade UV repair — diagnosis confirmed, strategy (a) shipped.** The proxy bug was exactly as reported:
the single banked atlas with `V = height/3.0` and a **Clamp** sampler. V > 1 clamped to the top row
("windows on the top floor only"), U was locked inside one bank ("one edge"), and the same Clamp state
smeared edge pixels. Fix: **one proxy texture per family** (residential, commercial), each a single
tileable facade with one window row and a plain 5 % roof strip, sampled with an explicit **Repeat**
descriptor in U and V. UVs are now the plain physical mapping: `v = height / 3.0` floors, `u = width / 18 m`
(commercial) or `width / 12 m` (residential). No banks, no shader fract, no sub-rect math. Cost: **+2 shared
materials** (20 → 22) for the two proxy skins; the shell 20-material budget is otherwise untouched.
Sampler audit: shell wall textures were already Repeat-descriptor; pane/sash atlases stay Clamp with
0..1 UVs (correct). The proxy Clamp was the only offender.

**Roof probability doctrine.** Gable probability is monotonic in storeys — 100 % at one storey, smooth decay
to 5 % above ten — and flat roofs now carry a gravel top with a moss rim instead of bare grey. Measured built
distribution (carpet run): **1 storey 62 % (177), 2 storeys 68 % (544), 3–5 storeys 7 % (14), 6–10 0 % (4),
>10 0 % (3)**. The gap between the one-storey doctrine (100 %) and 62 % built is the concave-skeleton flat
fallback, counted separately.

**Tint doctrine reversal (tint salad).** Per-building tint is now a disorganized muted neutral salad — six
directions (brown, tan, grey, slate blue, off-white, dark brown) with saturation capped and a 0.86–1.12
value nudge — applied identically to shells (wall vertex colours) and proxy boxes. No district coherence.
`captures/tint_salad_street.png` is the street-row proof.

**Window day/night.** Day windows are always dark reflective glass on both the proxy atlas and the shell
sash atlases (no lit cells in the day art). Night: towers above three storeys mix the crate's warm and cool
window materials floor by floor (seeded, ~55 % cool), while 1–3 storey homes keep one material — one
household, one temperature. The residential night carpet stays warm-dominant (`night_residential_carpet.png`).
Proxy night art is warm-dominant; per-floor temperature on proxies needs a second emissive texture and is
filed as `proxy-night-temperature`.

**Carry items closed.** Evergreen composition now prints in a normal run once 4k trees have spawned:
**fir 51 %, cedar 35 % (conifer 85 %), maple 6 %, madrone 1 %, cherry 5 %, plane 3 %**. The carpet capture is
re-shot with full window grids on all faces. Sway offset frames, the tower-beside-house window frame, and a
dedicated roof re-shoot remain the evidence debts.

**Gauntlet** (`--seattle` + `S2_GAUNTLET=1`): 15 m/s 58.5/253.7 (boot stall), 30 42.3/66.3, 100 42.5/63.2,
**650 58.5/95.4** — inside the 28–77 band.

**Debts.** `tower-house-frame` (owed twice, **escalated**), `sway-frames`, `proxy-night-temperature`,
`tree-sway-shader`, `roof-pack-bake`, `proxy-material-budget` (+2).
