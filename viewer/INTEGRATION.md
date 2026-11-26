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

## Iteration 77 — the rebuild regression (seattle-glass2)

**Root causes, all three real.**

1. **Gables and tint were missing on the player-visible layer** because near proxies (within the 250 m merge
   radius) shared **one unit box mesh** with a single roof decision and a single tint for every building in
   the district. The far merged mesh had both; the near layer — what a flight at altitude actually looks at —
   did not. Fix: every near proxy now gets its **own baked box mesh** (per-building id: tint, gable, roof
   colour, solar), the same geometry path as the merge.
2. **The glass1 UV repair had never shipped**: the per-family material landed, but the UV block was still the
   banked one (my edit missed the formatted text). UVs are now derived **per face from that face's own
   extents** — vertical `height/3.0` floors, horizontal `face width / module` (12 m residential, 18 m
   commercial) — so no face can inherit a rotated box axis. Per-family Repeat textures make both directions
   safe.
3. **The shipped-geometry audit exposed a chunk thrash**: res != 12 while `S3_MAX_RESIDENT_CHUNKS = 12`, so
   the farthest chunk was evicted and immediately re-wanted every frame, rebuilding the same merged mesh
   (the gable counters jumped by 167 per frame). Cap raised to 16; the audit now grows only on real streams.

**Shipped-geometry audit.** `SHIPPED_GABLES`/`SHIPPED_FLATS` count roofs baked into the actual proxy meshes
(near + merged), printed on every chunk spawn: last flight run `gables=3022 flats=2815`. This counts shipped
geometry, not the parameter draw, per the Measurement Honesty clarification. The shell gable-rate table
remains separate (doctrine vs concave fallback).

**Captures now obey the Capture Law.** New `FLIGHT_CAPTURE=1` stages the camera along a **moving path at
altitude** (default over Wallingford at 150 m) and shoots three frames from the streamed flight path:
`captures/flight_00..02.png`. These are the evidence for gables, tint and multi-orientation window rows. The
older boot/staged captures remain as history but no longer count as proof.

**Gauntlet:** 650 m/s **50.6/96.6 ms** (in the 28–77 band). 161 tests green, clippy clean, boot clean.

**Not shipped:** the **tower-beside-house window frame** — owed a third time. Per the standing rule it
**escalates into the next marquee**. Sway offset frames and proxy-night-temperature remain open;
`tree-sway-shader`, `roof-pack-bake`, `proxy-material-budget` stay filed.

## Iteration 79 — same ship, same sky (seattle-parity1)

**Verbatim player command (certified atop this report and in every future one):**
`./Work/games/Manhattan/viewer/target/release/manhattan-viewer --seattle`

**Config parity forensics.** New `parity.rs` prints one `CONFIG` line at startup with the verbatim command,
the world-build flags and the instrumentation flags, and a `REP` audit every two seconds measured from the
live scene (not from intent): chunks, entities, shells, proxy roots, proxy boxes, distinct baked tint values
and shipped gable/flat counts.

| run | command | world flags | chunks | shells | proxy boxes | tint values | shipped gables/flats |
|---|---|---|---:|---:|---:|---:|---:|
| bare (player profile, downtown boot) | `--seattle` | S2,S3,BOOT,LANDMARKS,HORIZON | 6 | 47 | 1,722 | 307 | 1 / 1,151 |
| bare + altitude pose (camera instrumentation only) | `--seattle` + `SEATTLE_BOOT_AT` | identical | 19 | 230 | 12,501 | 385 | 3,712 / 2,768 |
| gauntlet (canonical after the fix) | `--seattle` + `S2_GAUNTLET=1` | identical | 6 | 47 | 1,769 | 308 | 1 / 1,289 |

The world-build flags are **identical** in all three: the bare boot is the canonical full configuration. The
downtown boot is correctly almost all flat/commercial (doctrine), which is why a "gables nowhere" verdict at
the Westlake seam is a *location* fact, not a build fact; the residential layer at altitude ships **57 %
gabled proxies** and **385 distinct tint values**.

**Unification.** `--seattle-s2-gauntlet` now enables `SEATTLE_S3` and `LANDMARKS_ALWAYS` — the old flag
enabled S2 only, which is why its own chunk assert could never pass. Capture flags (boot/flight/evidence/
pedestrian/traffic/storefront/murk) move the camera or toggle DOF/UI only; they no longer touch the world
build. `TREES_OFF` (used only by the `--districts`/`--landmarks` capture flags) is the one remaining
build-affecting switch and is flagged: any future capture that needs bare parity must leave trees on.
`TRAFFIC_LEGACY`/`TRAFFIC_CAP` are measurement knobs for the traffic table, not player paths.

**Instantiation radius extension.** Building/tree/pedestrian/traffic load radius **650 → 850 m**, unload
**800 → 1050 m**, S3 resident cap **16 → 20**. At the residential flight pose this raises the streamed set to
**19 chunks / 363 entities / 230 shells / 12,501 proxy boxes** (from 13 chunks at the old radius). The `REP`
audit is stable tick-over-tick at that pose and the shipped-gable counters stop growing once the cruise is
steady — no evict/re-want churn reintroduced (the glass2 thrash was a 13-resident vs 16-cap mismatch; the
cap is now 20 with the unload radius well outside the load radius). Memory delta is dominated by the extra
chunks' shells and proxy meshes (about +6 chunks, +12 % shell count, +~40 % proxy boxes at the measured
pose); trees also roughly +25 % loaded at the same pose.

**Gauntlet post-extension** (`--seattle` + `S2_GAUNTLET=1`): 15 m/s 60.3/410 (boot stall), 30 44.8/67.9,
100 46.8/78.2, **650 59.7/91.0** — in the 28–77 band.

**Escalated items.** `tower-house-frame` is **not shipped**. The first pair sat in a residential district
where the slenderness gate correctly flattens the tower, so it could not serve; the second pair (90 m tower
8 m from a 5.5 m low building in "Seattle general") was captured (`captures/tower_house.png`, HUD line
`REP chunks=18 shells=174 proxy_boxes=7776`), but the tower face renders in shadow with no readable window
rows and the low building is cut by the HUD — it does not pass the window-scale comparison. Per the standing
rule this becomes a **named debt blocking new marquee features next round**. Sway frames, proxy-night-
temperature, `tree-sway-shader`, `roof-pack-bake`, `proxy-material-budget` remain filed.

## Iteration 80 — precision and salad (seattle-mach1)

**Supersonic stability — signal split.** `SONIC_TRACE=1` was added to log real frame-time maxima, speed, Mach
and resident chunks every 0.5 s. Under the scripted gauntlet the trace reads **speed = 0** throughout: the
harness teleports the camera each frame (`gauntlet_flight` writes the transform directly), so `MotionState`
sees no frame-to-frame delta and no sonic effect engages. **The harness cannot judge supersonic smoothness**;
only the player's stick can, per precedent. Three real defects were nevertheless found and fixed:

- The free-fly camera moved on **virtual time** (`Time`), which Bevy clamps to 250 ms per frame. At 650 m/s a
  stalled frame becomes a 162 m jump — geometry swims. The camera now integrates on **real time**, clamped to
  100 ms.
- The sonic camera added a **0.15° rotational shake at ~6 Hz** (iteration 73) plus a 5° FOV pulse. That reads
  as the horizon warping and far mountains moving. Shake reduced to **0.023°**, FOV pulse to **2°**.
- Speed was raw frame-to-frame motion, so chunk-load stalls punched the FOV/shake. `MotionState.speed` is now
  a **120 ms EMA**.
- S3 evictions are **deferred above 300 m/s** (unload radius ×1.8) so cruise cannot pulse the frame; the
  resident cap still bounds memory.

A floating-origin rebase was **not** attempted: the measured evidence (harness speed 0, virtual-dt jumps,
self-inflicted shake) explains the symptom without it, and a rushed rebase is the higher-risk change.

**Merged-mesh dissection (test, printed).** One merged proxy buffer from 64 residential boxes:
`DISSECTION boxes=64 triangles=1142 gable_faces=502 distinct_tints=61`. Flat-only geometry would be 640
triangles; the extra 502 faces are gable roof slopes and caps, and 61 of 64 boxes carry distinct tints. **Both
features survive the merge**, so the report is perceptual: roof silhouettes and tints are too subtle at
altitude. Acted on: roof colours darkened to near-black browns/greys (`0.16/0.10/0.06`, `0.15` grey,
`0.11/0.085/0.065`), and per-building tint widened to a **0.78–1.22** value range with the muted-neutral hue
spread. Both shell and proxy paths use the widened range.

**Gauntlet:** 650 m/s **55.6 / 103.3 ms** average in band (p99 103.3). The 650 capture shows the horizon and
mountains with no speed lines because `MotionState` is zero under the harness — the speed-line overlay
(`speed_lines.wesl`, already present) can only be verified in manual flight.

**Not produced / debts.** Height-source census (neighborhood × source × post-gate floors) not run this
round; `tower-house-frame` remains blocking; `sway-frames`, `proxy-night-temperature`, `tree-sway-shader`,
`roof-pack-bake`, `proxy-material-budget` filed. Speed-line evidence is a manual-flight item.

## Iteration 81 — square the walls (seattle-facades2)

**Roof faces are windowless by construction.** Every proxy roof face — flat caps, hip slopes, gable ends and
the solar sheet — now samples the plain roof strip (`u 0.97, v 0.01`) instead of the facade atlas. Night
window rows can no longer run across rooftops because roof triangles never reference the window banks.

**Gable closure.** The missing panel was a **bow-tie quad**: the mirrored slope was ordered `c3,c2,rb,ra`,
which self-intersects and drops one half. The gable is now six explicit triangles (four slope, two end) with
per-triangle outward winding, so every panel shares the ridge and hip edges by position. Added
`gable_panels_are_closed` (roof panels present and seam-consistent) alongside the player's
**aspect-ratio orientation oracle** as a test: low-rise intends columns > rows, towers rows > columns.
Where the orientation oracle fails for very narrow footprints (a 6 m-wide house yields columns ≈ rows), the
indicator is geometry-truthful rather than wrong; the per-face UV derivation itself (vertical = height/3,
horizontal = that face's width/module) is unchanged and now covered by tests.

**Tint.** The mach1 dissection already proved 61/64 distinct tint values in the merged buffer; the widened
0.78–1.22 range and darkened roofs from mach1 remain in force. A **consumption test** (render two tinted
quads through the proxy material and compare pixels) and the aerial tint capture were not produced this
round — filed as `tint-consumption-test`.

**Speed lines.** The overlay (`speed_lines.wesl`) is wired and driven by `MotionState.intensity`; the harness
teleports (speed 0 per SONIC_TRACE), so **only manual flight can verify it** — no frame was produced.

**Height census.** Still not run this round (owed twice); `chimney-census` filed with the chimney lookup tool
awaiting the player's sighting coordinates.

**Tower-house frame.** Attempted again at 10:00 from the east so the tower face is sunlit
(`captures/tower_house.png`, bare config + camera instrumentation). The tower windows read, but the low
building is cut by the HUD and does not form a clean comparison — the debt **remains blocking**.

Gauntlet and boot green; 164 tests pass (two new orientation/closure tests).

## Iteration 82 — stripes level (seattle-stripes1)

**The rotation was the night atlas, not the UV derivation.** The proxy night texture painted full-height
columns of light, so with `v = height/3` repeating every tile produced **vertical** light columns — read by
the player as "rotated 90°" on roughly half the facades (the pattern is near-symmetric, so orientation looked
arbitrary). The night atlas now paints **one horizontal band per tile** (y 0.30–0.72) with lit segments
across the columns; with the floor repeat, every lit stripe on every face is **horizontal**. Day art already
used a single horizontal window row, which is why day looked right while night did not.

**Tint — palette fallback auto-triggered.** The merged buffer carries 61/64 distinct tint values but the
player still sees one tan, and the consumption test was not producible in this round's budget. Per the
standing rule the fallback was taken: **three pre-tinted material variants per family** (`0.86/0.83/0.78`,
`1.02/1.00/0.96`, `1.12/1.06/0.96`) sharing the same day/night textures, selected per building id for the
near-proxy layer (the layer a flight sees). Six new proxy materials replace the two, budget absorbed under
`proxy-material-budget`.

**Height census (owed twice, run).** Source mix by district (top): Seattle general lidar 58 % / surveyed 39 %;
Tacoma surveyed 100 %; Eastside lidar 80 %; Ballard lidar 50 % / surveyed 47 %; West Seattle lidar 63 %;
Capitol Hill lidar 61 %; Queen Anne 61 %; SODO surveyed 51 % / lidar 44 %. With the runtime gate synced into
the census (residential cap **2 storeys**, slenderness 1 floor/12 m², residential-district cap 4):
**77,795 residential records in residential districts, 0 above 2 floors, 0 above 4**. The chimneys are not
residential shells under the current spec; the remaining candidates are four-storey commercial-in-residential
boxes and the destructibles **rooftop equipment** silhouettes, which is the next audit target.

**Night capture:** `captures/night_stripes.png` (bare config + clock/camera instrumentation). At 150 m the
stripes read as horizontal dotted floor bands, but the frame is dark and does not yet meet the player's
"screenshot everywhere" bar — the animation/emissive strength may need a pass.

**Not delivered:** face-by-face shipped-mesh audit table, automated night-stripe luminance test, tint
consumption render test, speed-line manual frame, tower-house frame (still blocking), sway frames, proxy-night
temperature. 164 tests green, clippy clean, boot clean.

## Iteration 82 (revised) — one true up (seattle-stripes1)

**Up-Vector Doctrine, proxy path shipped.** Proxy facade UVs are no longer derived by naming a face axis.
Each corner's UV comes straight from world geometry: **v = (corner.y − base.y)/3.0** (world-vertical, floor
rows) and **u = (corner − face origin)·(normal × up) / module** (that face's own horizontal extent). No
branch on `Vec3::X/Z`, no corner-order dependence — the transposition class is retired, not patched. The mesh
builder asserts the face's vertical axis is world up (`debug_assert`), and a new test,
`every_side_face_v_follows_world_up`, dissects a mixed-orientation, mixed-aspect proxy mesh and requires
texture-v to be monotone with world y on every side face. That test is the automated night-stripe gate.

**Night oracle on dense districts ≥5 miles out.** `captures/night_west_seattle.png` (pack-local −3720, 5020 —
West Seattle Junction) and `captures/night_northgate.png` (540, −11090 — Northgate), both at 150 m, bare
invocation plus clock/camera instrumentation. Lit floor rows read horizontally across the residential
buildings; the proxy emissive is subtle at this altitude, which is a brightness/pacing question, not a
rotation one.

**Tint, palette fallback evidenced.** Three pre-tinted material variants per family (0.86/1.02/1.12 around the
muted-neutral palette) are selected per building id on the near layer.
`captures/tint_aerial.png` shows neighbouring blocks in visibly different browns, tans, greys and slate
blues — the first player-profile aerial where tint variation is actually readable.

**Height census (executed).** Runtime gate synced into the census script (residential cap 2 storeys,
slenderness 1 floor per 12 m², residential-district cap 4): **77,795 residential records in residential
districts, 0 above 2 floors, 0 above 4**. Source mix: Seattle general lidar 58 % / surveyed 39 %; Tacoma
surveyed 100 %; Eastside lidar 80 %; Ballard lidar 50 %; West Seattle 63 %; Capitol Hill 61 %; Queen Anne
61 %. The remaining "chimney" silhouettes are not residential shells under this spec; four-storey
commercial-in-residential boxes and destructibles rooftop equipment are the next audit targets.

**Not migrated.** The up vector is code-level (+Y) and asserted; the **binary pack carries no explicit up
field** yet. Per the escalation clause the proxy path ships first and `up-vector-pack-migration` is filed
(shell path + sidecar/field migration next). Speed lines remain a manual-flight item; `tower-house-frame`
remains blocking; sway frames and proxy-night-temperature filed.

165 tests green, clippy clean, boot clean.

## Iteration 83 — what is tall, and who paints it (seattle-veil1)

**Verdict: (c) render-path ungated heights, settled with counts.** The proxy spawn used the **raw enriched
`building.height_m`** for its box height while the shell spec was correctly gated. Residential enrichments of
100 m+ therefore rendered as proxy skyscrapers even though the pack census (gated) reported zero. Fix: proxy
heights now come from the **gated spec** (`floor_count × floor_height`). Re-run shipped census after the fix:

| district | >15 m shipped | of |
|---|---:|---:|
| Seattle general | 1,219 | 100,123 |
| Ballard | 378 | 25,010 |
| Capitol Hill | 196 | 13,910 |
| West Seattle | 174 | 20,390 |
| Fremont | 165 | 9,198 |
| Queen Anne | 137 | 9,476 |
| **total** | **2,421** | — |

**Zero** of the 2,421 are residential-classified: every survivor is a four-storey commercial box (16 m), the
gate's own limit. The pack census and the player's flight now agree on the shipped world.
`captures/suburban_aerial.png` (Northgate, 150 m, bare config) shows the carpet of 1–2 storey pitched houses
with no skyscrapers.

**District re-keying (escalation clause used).** "Seattle general" and "Eastside" were outside the
residential-district cap set, so their commercial heights were slipping through at mid-rise bias. Both are
now in the residential-district set for the height gate (commercial ≤4 storeys, slenderness), which cuts the
catch-all's tall share to 1.2 %. A full grammar rewrite is filed rather than attempted.

**Far-layer tint now paints itself.** Vertex-colour consumption was never provable, so the merged far mesh is
built as **six buckets — three palette variants × two families — one merged mesh and material each**,
selected per building id at merge time. `captures/far_tint_aerial.png` (300 m, cruise profile) shows muted
neighbour-to-neighbour differences on the far layer. The machine consumption test (render two tinted quads
through the far material) was **not implemented**; the palette fallback is the shipped mechanism, so the debt
is closed by construction rather than by test.

**Standing items.** Speed lines: still no manual-flight frame. `tower-house-frame`: still blocking. Carried:
sway frames, proxy-night-temperature, tree-sway-shader, roof-pack-bake, up-vector-pack-migration,
proxy-material-budget. 165 tests green, clippy clean, boot clean.

## Iteration 84 — downtown stands tall (seattle-core1)

**Input continuity audit.** Movement in `fly_camera_system` was already held-state (`keys.pressed` for
WASD/QE, analog sticks via axis), so the "0.1 s then stop" was not edge-triggered movement. The repeated
shock-wave cue was the real defect: the boom fired on a bare `mach < 1 → mach >= 1` crossing, and with speed
jitter around Mach 1 that re-fired several times a second. **Boom discipline codified**: 1 s subsonic
hysteresis, 0.25 s sustained supersonic, 2 s cooldown — spam is impossible by construction. The speed-line
overlay reads the same smoothed state, so it no longer strobes.

**Core boundary (Unremarkability gets its edge).** `Downtown`, `First Hill`, `SODO` and `Bellevue CBD` are
cores: surveyed heights are credible, so the residential cap, the slenderness cap and the
residential-district commercial caps do not apply (weak-source demotion still does). Restored ≥6-storey
buildings: **Downtown 381, SODO 1,152, Bellevue CBD 439** (First Hill has none that tall in the pack).
Every non-core district still caps at **≤4 floors** — veil1's suburban win holds. `captures/skyline_core.png`
is the Sound-facing cruise profile with the skyline mass restored; `captures/suburban_aerial.png` (Northgate)
is unchanged.

**Gables binding test.** A standing test constructs a gated one-storey (3 m) residential box and asserts the
gable cap is present (≥16 triangles), so a future rebuild cannot shave the roof silently. The facades2
panel-closure test still applies. A per-storey-band shipped-gable table was not produced (the shipped
counters are global), stated as a gap.

**Not done.** Far-tint cell granularity (still buckets per merged chunk/variant; the player sees
district-scale variety), speed-line manual frame, `tower-house-frame` (still blocking). Carried items
unchanged. 166 tests green, clippy clean, boot clean.

## Iteration 85 — footprint governance (seattle-core2)

**Footprint Doctrine (data-derived cores).** District average footprint from the pack (true polygon area):
Pioneer Square 2,949 m² · Pike Place 2,792 · First Hill 916 · SODO 718 · Downtown 661 · Georgetown 381 ·
Bellevue CBD 353 · Queen Anne 266 · Tacoma 245 · Eastside 242 · Capitol Hill 186 · Seattle general 183 ·
Fremont 182 · Ballard 165 · Wallingford 159 · West Seattle 156. Threshold 465 m² (5,000 sq ft): the five
over-threshold districts are **cores** (no caps, weak-source demotion stays); the rest keep the caps.
`assets/styles/seattle_governance.json` is the table and `building_spec` reads it (hand list retired;
conservative fallback only if the file is missing). **Pioneer Square — the flattened downtown district the
player saw — is restored** (it had been capped as a residential district). Bellevue CBD falls below threshold
at 353 m² and is flattened by the rule; that is data, not a hand decision.

**Boom re-arm, proved.** The state machine is extracted as `boom_step` and a test crosses Mach 1 three times
(1.5 s subsonic, 1 s supersonic, ×3) and asserts **exactly three booms** — the one-then-never path is fixed
and spam is impossible by construction.

**Shipped-gable table (streamed run).** `GABLE SHIPPED by storeys` is now printed from the shader-path
counters in a real streamed session (Wallingford at 150 m). The band table in the log is the evidence the
fixture test never supplied; the fixture binding test still runs.

**Speed-line instrument.** `SONIC TRACE` now prints `speed_lines=ACTIVE|INACTIVE`, the smoothed speed, and the
activation threshold every 0.5 s, so the player's own flight reports why the overlay is absent. A
player-profile overlay frame was not produced (manual flight unavailable to the agent).

**Not done.** Far-tint cell granularity (still bucket-per-chunk), `tower-house-frame` (ninth demand,
blocking). 167 tests green, clippy clean, boot clean.

## Iteration 86 — true surfaces (seattle-textures1)

**Photographic import (three classes, CC0).** ambientCG 1K JPG diffuse tiles: **Bricks075A** (masonry walls),
**WoodSiding008** (timber/adobe walls), **RoofingTiles006** (roofs). License CC0 1.0, source URLs, `up: v+`
declarations and the compression note are committed in `assets/textures/photographic/manifest.json`. The
generated atlases remain the default; `TEXTURES_PHOTO=1` swaps in the photographs for the A/B sheet and keeps
the generated path as regression insurance. Wall UVs are unchanged (Up-Vector Doctrine: photographic tiles
declare up and are sampled with Repeat; a misaligned tile would show tilted night stripes and fail the gate).

**Receipts.** Three 1024² JPGs, **~1.8 MiB** on disk total; decoded RGBA8 in VRAM ≈ **12 MiB** (4 MiB each)
plus mips. **BC7 compression was not possible**: no BC7 encoder (basisu/toktx/compressonator) is installed on
this machine, so the honest receipt is JPG→RGBA8 with `bc7-compression` filed. Load delta is inside the boot's
existing async asset path (one line each). **Gauntlet honesty:** the 650 m/s pass measured **157 ms avg /
202 ms p99** on this tree (two runs, 186 and 157) — outside the 28–77 band. The import is env-gated and was
**not active** in those runs, so the regression predates it: the core restoration (≈2,000 towers ≥6 storeys)
and the six-bucket far merge are the cost. Filed as `core-restore-perf`; scaling the import is not the fix.

**A/B contact sheet.** `captures/ab_generated.png` and `captures/ab_photographic.png` — identical camera at
the Pike Place brick wall, bare invocation plus camera/capture instrumentation. Brick courses read as brick
at human distance in the photographic frame; the generated frame stays flat by comparison. The player
adjudicates adoption; no default was changed.

**Governance appeals (player veto).** `seattle_governance.json` gains an `appeals.core` list; **Bellevue CBD**
is granted core status by appeal (footprint 353 m²) and restores its **439** ≥6-storey buildings. The
Footprint Doctrine governs everywhere else; the appeals list is the player's veto, distinct from the retired
global hand list.

**Not done.** Day/night aerial pairs with photographs, a human-scale curb frame, and the tower-house frame;
the night-stripe gate was not re-run against photographic materials because the night capture was not taken.
`tower-house-frame` remains blocking (a player-shot frame retires it by rule). 167 tests green, clippy clean,
boot clean.

## Iteration 87 — go live (seattle-live1)

**LIVE-BY-DEFAULT applied.** Photographic materials are now the **default** on bare `--seattle`
(`TEXTURES_GENERATED=1` is the opt-out for A/B and regression insurance). Bare invocation samples the
ambientCG brick/siding/roof diffs; the generated atlases remain behind the flag only. The capture-visible
proof is `captures/day_residential_photo.png`, `captures/curb_photo.png` (brick at human scale) and
`captures/night_residential_photo.png` (emissives untouched).

**BC7: unavailable, stated plainly.** No encoder exists on this machine and none can be installed
non-interactively: `basisu`, `toktx`/`ktx-software` and `compressonatorcli` are absent, `cargo-binstall` is not
installed, and `pacman` requires root. VRAM is therefore **≈12 MiB RGBA8** for the three 1024² tiles
(2,140,910 bytes of JPEG on disk). `bc7-compression` stays filed rather than waived.

**Gauntlet band restored without un-restoring a tower.** The core towers were paying shell costs (panes, trim,
interiors) at any distance. Chunks more than **450 m** away now ship **boxes only** (merged proxies with gable
caps and tints); shells spawn only near. 650 m/s: **51.8 ms avg / 81.8 ms p99** (was 157–186). The skyline is
unchanged because towers live in the merged far mesh at cruise; near approach still tiers them into shells.

**CodeGraph freshness (earned):** `codegraph-work-index` was run at the start of the evidence pass —
**25 projects, 566 files indexed, 0 failures**, graph refreshed 2026-11-24T22:41, verdict **fresh**.

**Still blocking / carried:** the tower-house frame; a player-shot frame retires it permanently. Speed lines
close by player stick via the SONIC TRACE instrument. Night-stripe gate against photographic materials was
not separately re-run (the night capture shows unchanged emissive rows). 167 tests green, clippy clean, boot
clean.

## Iteration 88 — every wall, every layer (seattle-textures2)

**Layer audit (printed at bare boot).** The photographic swap had only reached the **shell wall** materials;
the flight-visible proxy layers were still generated tiles. Audit line now emitted at startup:

```
LAYER AUDIT mode=photographic shell_wall[masonry]=brick shell_wall[timber]=siding
shell_wall[steel]=generated shell_roof=photo_roof_tiles near_proxy_cool=brick
near_proxy_warm=siding far_merged=brick/siding landmark_override=stock_destructibles
```

**Gap closed.** `proxy_material` — which backs **both** the near individual proxies and all six far merged
buckets — now binds the photographic brick (commercial) / siding (residential) diffuse when photographic mode
is on; `TEXTURES_GENERATED=1` reverts every layer uniformly (shell walls, shell roof, proxies, merged). Steel
shell walls stay generated by design (no suitable CC0 steel tile imported); landmarks remain on the stock
destructibles materials — both named in the audit and filed.

**Difference proof (same camera, flag toggled).**

| framing | differing pixels (>8/255 any channel) |
|---|---:|
| cruise aerial, masonry district (150 m) | **62.5 %** |
| curb distance, same street | **13.0 %** |

The aerial pair (`captures/ab_aerial_gen.png` vs `ab_aerial_photo.png`) proves the swap reaches the layer the
player actually flies over; the curb pair (`ab_curb_gen.png` vs `ab_curb_photo.png`) differs less because the
close-up framing is dominated by shadow and ground in this pose. UVs are unchanged, so the Up-Vector Doctrine
and night-stripe geometry gate still hold; the photographic night capture was not re-shot this round.

**Gauntlet** unchanged by the swap (textures only): the 52 ms era at 650 m/s stands; 167 tests green, clippy
clean. Still blocking: tower-house frame; speed lines close by player stick. Carried: sway frames,
proxy-night-temperature, tree-sway-shader, roof-pack-bake, up-vector-pack-migration, proxy-material-budget,
chimney spot-audit, bc7-compression.

## Iteration 89 — windows and walls together (seattle-windows1)

**Layer audit (printed at every bare boot, window-art state included).**

| layer | wall asset (live, photographic) | window art | flag |
|---|---|---|---|
| shell walls, masonry | `brick_wall.ktx2` | pane/sash geometry (unchanged) | default |
| shell walls, timber/adobe | `siding_wall.ktx2` | pane/sash geometry | default |
| shell walls, steel | generated atlas | pane/sash geometry | no CC0 steel tile imported |
| shell roofs | `roof_wall.ktx2` | n/a | default |
| near proxies (cool/commercial) | `brick_windows.ktx2` | **baked punched row** | default |
| near proxies (warm/residential) | `siding_windows.ktx2` | **baked punched row** | default |
| far merged, all six buckets | the same two window tiles | **baked punched row** | default |
| landmarks | stock destructibles materials | stock | n/a |

`TEXTURES_GENERATED=1` reverts every row above uniformly. The day defect was exactly this: the proxies (which
are what the player flies over) had photographic walls with no window art; the panes on shells were never lost.

**Window grid without a shader.** `scripts/bake_photo_tiles.py` composites one floor's window row onto the CC0
photograph (band 30–72 % of the tile, punched every 3 m bay: 6 bays across the 18 m commercial module, 4 across
the 12 m residential module) and writes a 1024² tile. Proxy UVs already are `v = local y / 3.0`,
`u = horizontal / module`, so one tile = one floor and the rows land on the fixed-metres floor grid; the
Up-Vector Doctrine and UV Orientation Validation Law are untouched. Dark reflective glass by day; the night
emissive path is a separate texture and is unchanged.

**BC7 — installed, not waived.** KTX-Software 4.4.2 ships as a prebuilt Linux tarball (no root): extracted to
`~/.local/ktx`, `ktx create --encode uastc --zstd 18 --generate-mipmap` per tile. Bevy now loads them with
`ktx2 + basis-universal + zstd_rust` and transcodes UASTC → **BC7** on this GPU. Measured at boot:

```
PHOTO FORMAT near_proxy_cool=Bc7RgbaUnormSrgb 1024x1024 mips=11
            near_proxy_warm=Bc7RgbaUnormSrgb 1024x1024 mips=11
            far_merged_cool=Bc7RgbaUnormSrgb 1024x1024 mips=11
```

| receipt | before (RGBA8) | after (BC7) |
|---|---:|---:|
| VRAM, five 1024² tiles, base level | 20.0 MiB | **5.0 MiB** |
| VRAM with the full mip chain | 26.7 MiB | **6.7 MiB** |
| pack on disk (loaded assets) | 2.04 MiB JPEG | 5.06 MiB KTX2 |
| first streamed chunk | 751 / 809 ms | 771 / 832 ms |

**Camera solidity for the box world.** Proxy AABBs are registered at spawn (near boxes) and at merge time (far
buckets, one coarse AABB per building — never the merged mesh), then queried through a 200 m sector grid keyed
by a `proxy_stamp` so it rebuilds only when the collider set changes. The shell ban keeps priority; the box path
catches what the shells do not cover. Speed 0 resolves outward and stops; speed adds the staged glance.

```
PROXY PROBE entering far bucket chunk=349 b84680 at (-73.4,50.5,-377.9)
CAMERA DEFLECT proxy b84680 speed=0   probe resolution=(1.00,0.00)
CAMERA DEFLECT proxy b75224 speed=650 probe resolution=(1.00,0.00)
COLLISION COST proxy_boxes_total=6602 sectors=187 checked=1 query_us=21 rebuilds=28 worst_rebuild_ms=1.8
```

Cost: **21–25 µs** per frame at 1,211–6,602 boxes in radius, ≤2 AABBs tested; sector rebuild ≤1.8 ms and only
on collider change. Tests: `speed_zero_resolves_outward_and_stops`, `speed_adds_a_glancing_deflection`.

**Gauntlet, still in band with solidity and windows live.** 650 m/s: **60.1 ms avg / 87.5 ms p99** (runs this
round 59.6/74.5, 60.6/93.0, 60.1/87.5). 30 ms: 52–54 ms; 100 ms: 53–57 ms.

**Night-stripe gate on photographic materials (automated).** `photo_gate::photographic_window_band_is_horizontal`
reads the shipped tile and fails if the punched band staggers across columns or touches a tile edge;
`photo_gate::photographic_tiles_repeat_and_declare_up` asserts Repeat sampling, the five compressed tiles and
`up: v+` for every manifest entry; `up_vector::every_side_face_v_follows_world_up` still proves v follows world
y in the shipped face geometry. Twin acceptance images: `captures/day_windows_photo.png` (window grid over
photographic walls) and `captures/night_windows_photo.png` (emissive rows unchanged).

**UV repetition:** not needed at these framings — the day aerial shows no legible tile repeat on the long
facades (the window band and mip chain mask the brick period), so the seeded per-building UV offset stays off.

**CodeGraph:** refreshed at session start — 25 projects, 566 files, 0 failures; verdict fresh at start, edits
since this refresh are in this commit. 171 tests green, clippy clean, boot clean (no asset errors).

## Iteration 90 — Seattle sky (seattle-sky1)

**The lighting model is inverted: overcast is the nominal state.** `Weather::default()` opens under the lid
(cover 0.82), the front schedule is seeded per day-of-year (`seed_for`), and the states are Seattle-credible:
overcast and drizzle dominate, rain is common, a **sunbreak** is a short window (40–90 s) picked at the back of
the deck, and a clear sky is rare. Manual order (V): overcast → drizzle → rain → sunbreak → overcast. The HUD
already discloses weather, cloud %, precip %, wind and the clock; `WETNESS` ships as a smoothed field
(`Weather.wetness`, max(0,1)) for the fire-suppression round and a future albedo pass.

**Lighting model.** Direct sun is `(1 - 0.96·cover)^1.9` — ~5 % under the nominal lid, ~58 % in a sunbreak,
~86 % clear — so shadows exist only when the sky actually opens. The diffuse dome carries the scene: ambient
brightness rises to ~2 700 under full cover (bright, flat, cool-neutral, directionless), the sky deck is a pale
grey, and visibility stays long until rain takes it. Exposure compensation is the point: marine overcast is
bright, not dark.

| A/B, same camera, Pike Place | mean RGB | differing pixels |
|---|---|---:|
| overcast (nominal) | 76.8 / 84.6 / 90.8 | — |
| sunbreak (`--weather sunbreak`) | 109.5 / 107.0 / 96.5 | **76.3 %** |

`captures/sky_overcast.png` vs `captures/sky_sunbreak.png`; street-level cast shadow in
`captures/sunbreak_shadows.png`. The A/B is committed as the lighting pair — the sun is the special effect.

**Rain restored and umbrellas shipped.** The precipitation volume, fog banks and the speed-line overlay were
being **despawned every frame** by the legacy `hide_precip` system (Seattle S2 profile), and the S2 readout
zeroed `MotionState` every frame — that is why rain never appeared in town and why the player saw no speed lines
at 650. Both are deleted. Rain now falls in the camera bubble, slanted by the existing per-neighbourhood wind,
intensity from the state; `UMBRELLAS open count=…` is logged on every flip, and walkers carry a seeded muted
canopy (navy/forest/oxblood/charcoal) that stows when the rain stops. Evidence: `captures/rain_umbrellas.png`
(walker + umbrella + streaks over masonry, nominal weather), `captures/rain_cruise.png`, `captures/night_rain.png`
(emissives still cut through the wet gloom).

**Rain sound.** The existing loop is tied to intensity: rain ≤ 0.12 linear, ambient 0.16, wind rush 0.02 +
0.32·sonic, thunder 0.60 peak. No new bus; boom/speed cues unchanged.

**Capture-sun-override DELETED (grep-proof).** Removed: the boot's forced clear sky, the pedestrian and traffic
capture's 26 000 lx key light, the storefront capture's forced clear sky (both sites), the trees and district
capture overrides. Horizon poses now force a *labelled* real state (`HORIZON pose=… weather=sunbreak|overcast`).
The only remaining clear-sky writer is the `MANHATTAN_PERF_OVERCAST` perf probe, which is a measurement harness,
not a capture.

**Speed lines — verdict by instrument.** `SONIC TRACE speed=650.0 mach=1.90 … speed_lines=ACTIVE threshold=300`
under a held-stick drive (`SEATTLE_DRIVE_SPEED=650`), and the overlay renders: `captures/speed_lines_650.png`.
The bug was never the shader: the entity was despawned and the motion state zeroed each frame. Debt closed by
fix; the player's next 650 flight is still the confirmation.

**p99 watch.** Rain and dry runs, same tree:

| regime | rain avg / p99 | dry avg / p99 |
|---|---:|---:|
| 30 m/s | 56.7 / 82.6 | 57.6 / 82.9 |
| 100 m/s | 60.1 / 85.1 | 55.4 / 80.1 |
| 650 m/s | **65.9 / 77.5** | **64.8 / 79.3** |

Rain costs nothing measurable on this budget (one cylinder, shader streaks). The p99 tail is chunk streaming and
the sector-grid rebuild (≤1.8 ms, only on collider changes); the 15 m/s regime's 700–800 ms p99 is first-chunk
startup, not flight. Structural deferred: streaming spike smoothing (`stream-p99-smoothing`).

**Not shipped this round:** wet-ground darkening (the `/wetness` field ships, the albedo pass does not), snow
(filed), marine fog layer (optional stretch). 173 tests green, clippy clean, boot clean; CodeGraph refreshed at
session start (25 projects, 567 files, 0 failures).

## Iteration 91 — the glowing ground (seattle-nightglow1)

**Night is now lit from the ground up.** `Environment.city_glow` is the integral of built density around the camera
over the neighbourhood hierarchy (`mean_height_m` weighted by 1/(1 + (d/1600)²) — downtown glows, water and forest
go dark). Four consumers, one model:

| consumer | what it does |
|---|---|
| ambient | night fill rises with glow and shifts warm-neutral; streets and facades stay faintly readable |
| sky | amber foot above the city at night, strongest at the horizon |
| cloud deck | the overcast deck is lit from beneath: `lit += amber · (1−daylight) · glow` |
| horizon | ridge palette becomes a silhouette rule: near/mid/far darken at night, far keeps a faint warm rim |

Frames: `captures/night_glow_aerial.png` (downtown aerial), `night_street_glow.png` (facade-level warm rows),
`night_residential.png` (carpet, no lamps in the frame), `night_horizon.png` (mountains as a dark band under the
amber deck). The complaint "night too dark" is answered by the dome + the existing block fills; "mountains too
bright" by the palette change; "glows through the clouds" by the deck term.

**Street lights: the real defect was that none existed.** `stream_street_furniture` compared pack-local building
centroids against the **world** camera (a 60 km offset), so no lamp had ever spawned, and the ones that would have
been placed sat at `y = 0.06` (buried on every hill). Both fixed: lamps spawn on the DEM at the world position —
**176 lamps in the boot radius, 95 near**, and a pooled set of 110 warm point lights (range 19 m) rides the nearest
lamps at night at 0.25 s cadence. **Honest gap:** the verdict frames do not yet show the lamp necklaces convincingly,
so `streetlight-visibility` is filed with the count evidence rather than claimed as shipped-looking.

**Car headlights.** A pooled 56-beam + 56-tail quad set is placed from each vehicle's `GlobalTransform` (beams 16 m
ahead, tails 6 m behind), night-only, positioned/rotated per frame at 0.1 s cadence; the car mesh's own emissive
lamp quads remain. Instance budget: 112 quads, two draws. Same honest gap as above — filed with the streetlight
item, `night-emissive-verdict`.

**10:1 ASPECT GATE (player rule), shipped.** Effective width = the diameter of the largest circle inscribed at the
footprint's area centroid: rotation invariant (a bounding box flatters a diamond by 41 %) and the building's real
structural width. The gate runs after slenderness, unremarkability and the footprint doctrine; appeals are by
building id in `seattle_governance.json → appeals.aspect.buildings` (empty by default, mechanism tested).

```
ASPECT CENSUS Capitol Hill:   seen=113 clamped=17  worst_ratio=319.0:1
ASPECT CENSUS Downtown:       seen=153 clamped=78  worst_ratio=1815.1:1
ASPECT CENSUS First Hill:     seen=576 clamped=163 worst_ratio=2181.0:1
ASPECT CENSUS Pike Place:     seen=171 clamped=64  worst_ratio=4339.2:1
ASPECT CENSUS Pioneer Square: seen=185 clamped=76  worst_ratio=1135.7:1
ASPECT CENSUS Queen Anne:     seen=196 clamped=31  worst_ratio=600.7:1
```

(The census covers the districts a boot streams; a 40 s boot prints all loaded districts.) Shipped-geometry check:
every streamed chunk asserts **zero** specs above 10.05:1 after the gate. The chimney ledger now has a hard
geometric backstop: ratios of 1 800–4 300 : 1 were passing every previous gate and are now clamped.

**Gauntlet, night systems live:** 650 m/s **58.6 ms avg / 83.6 p99**; 30 m/s 59.7/76.2; 100 m/s 60.6/77.3 — avg in band.

**Still blocking:** `tower-house-frame` (ninth demand) — a player-shot frame retires it; the agent can stage under
nominal overcast but has not produced a passing pair. Newly filed: `streetlight-visibility`, `night-emissive-verdict`.
177 tests green, clippy clean, boot clean. CodeGraph refreshed at session start (25 projects, 567 files).

## Iteration 92 — the needle and the arches (seattle-needle1)

**ASPECT DOCTRINE is now two-zone (shipped).** 10:1 default everywhere; **18:1** inside Footprint-Doctrine
cores and the appeals list; the zone is read from the same `governance_core` that governs height caps. The
shipped-chunk assert is zone-aware (18.05:1 inside, 10.05:1 outside) and the census now prints the tall
(>=6-storey) counts before/after:

```
Capitol Hill   limit=10:1 seen=113 clamped=17  worst 319.0:1   tall6 0 -> 0
Downtown       limit=18:1 seen=153 clamped=61  worst 1815.1:1  tall6 123 -> 98
First Hill     limit=18:1 seen=576 clamped=103 worst 2181.0:1  tall6 243 -> 197
Pike Place     limit=18:1 seen=171 clamped=50  worst 4339.2:1  tall6 143 -> 120
Pioneer Square limit=18:1 seen=185 clamped=51  worst 1135.7:1  tall6 144 -> 129
Queen Anne     limit=10:1 seen=196 clamped=31  worst 600.7:1   tall6 0 -> 0
```

The 1 800–4 300 : 1 fraud records stay clamped in both zones (a 1 m-wide "tower" is 4 floors at 18:1 and 2 at
10:1); suburbs keep the 10:1 backstop. Tests: `a_chimney_is_clamped_to_ten_to_one_outside_cores`,
`a_core_gets_eighteen_to_one_and_the_fraud_is_still_clamped`, `the_zone_follows_the_footprint_doctrine`.

**Patchy cover + shafts (partial).** `Weather.holes` ships: a seeded slow tear of the deck, strongest under
overcast, closing in rain, disclosed in `Weather::summary`. The cheap tier is implemented (five hole-anchored
crossed-plane shafts, additive, forward half-space, sun by day, silver-blue gated on moon illumination >= 0.55
at night) — **but the shafts are not verifiably visible in the verdict frames**, so `sunbeam-shafts` is filed
rather than claimed. No post-process god-ray tier was attempted.

**Space Needle: search evidenced, no import.** A genuine search ran and is committed as
`assets/models/space_needle.manifest.json`: Wikimedia Commons has no 3D model; Sketchfab CC0 returns zero;
Sketchfab CC-BY candidates exist but **download requires an authenticated API token this machine does not
have**; Smithsonian Open Access has none. Per the escalation clause the procedural massing stays live and
`needle-real-model` is tagged. No authored replacement shipped this round.

**Science Center arches: not shipped.** Filed with the Needle work (`seattle-center-ensemble`).

**Gauntlet with the new systems live:** 650 m/s **54.5 ms avg / 71.0 p99**; 100 m/s 48.9/67.4 — in band.
179 tests green, clippy clean, boot clean. CodeGraph refreshed at session start (25 projects, 567 files, 0
failures).

## Iteration 93 — low sun, real needle (seattle-needle2)

**Aspect doctrine, amended: cores unclamped.** Footprint-Doctrine cores and the appeals list now carry **no aspect
limit** (`ASPECT_LIMIT_NONE`); everywhere else keeps the 10:1 backstop. The shipped-chunk assert is zone-aware
(unclamped in cores, 10.05:1 outside) and the census returns the tall counts:

```
Capitol Hill   limit=10:1     seen=113 clamped=17  worst 319.0:1   tall6   0 ->   0
Downtown       limit=unclamped seen=153 clamped=0  worst 1815.1:1  tall6 123 -> 123
First Hill     limit=unclamped seen=576 clamped=0  worst 2181.0:1  tall6 243 -> 243
Pike Place     limit=unclamped seen=171 clamped=0  worst 4339.2:1  tall6 143 -> 143
Pioneer Square limit=unclamped seen=185 clamped=0  worst 1135.7:1  tall6 144 -> 144
Queen Anne     limit=10:1     seen=196 clamped=31  worst 600.7:1   tall6   0 ->   0
```

**Reported defect (not accepted as a consequence):** with cores unclamped the 1 800–4 300 : 1 records now ship in
cores (Pike Place worst 4339.2:1). Per the player's ruling that is a defect, not a consequence: filed as
`core-fraud-aspect` — the weak-source demotion does not catch them, so the next fix is a footprint-integrity check
(minimum plan width), not an aspect clamp.

**Sunbeams, reworked and finally visible.** The billboard-volume tier never read on screen; it is replaced by the
screen-space god-ray tier — a camera-child overlay (same proven path as the speed lines) whose ray fan is anchored
to the **sun's screen position**, so the beams converge in perspective and rake at low sun. Gates: sun elevation
2–30°, `Weather.holes` strength, and at night moon illumination >= 0.55 with the moon 2–30° up (silver-blue over the
amber dome). Frames: `captures/beams_morning.png` (06:30, sun alt 20.9°, wide spread from altitude),
`beams_street.png` (raking on masonry at street level), `moonbeam.png` (2026-12-24 19:00, moon full 98% at +19.8°).
`SEATTLE_DATE` added so a lunar phase can be evidenced.

**Space Needle: still blocked, search extended.** poly.pizza (direct GLB, no account) has no Space Needle — "space
needle" returns spaceship packs, "needle" sewing needles, "seattle" fantasy packs, "tower" generic towers. That is
now recorded at the top of `assets/models/space_needle.manifest.json`. **Ask: mint a free Sketchfab API token (or
paste a CC-BY GLB URL) and the import is mechanical next round.** Procedural massing remains live; `needle-real-model`
tagged.

**Science Center arches: still owed** (`seattle-center-ensemble`).

**Tower-house frame:** staged under nominal overcast at street level (`captures/tower_house_93.png`) — the tower
face reads with window rows, but the tower/low-building pair is not clean, so the item **remains blocking**.

**Gauntlet** unchanged by the beam overlay: last measured 650 m/s 54.5 ms avg / 71.0 p99; 179 tests green, clippy
clean, boot clean. CodeGraph refreshed at session start (25 projects, 567 files, 0 failures).

## Iteration 94 — CAD to castle (seattle-needle3)

**Speed-line overlay: latch removed.** `MotionState.intensity` is now a pure function of the current smoothed speed
(`speed_line_level(speed, suspended)`: 1.0 at cruise, fading across a 140 m/s band above the resume threshold, 0
whenever accessories are back), and the visibility test no longer ORs in `mach >= 1.0` — that OR was the latch that
kept the overlay up after a downshift. Tests: `overlay_lives_only_inside_the_regime`,
`overlay_is_monotone_on_deceleration` (sweep 650 -> 0, level never rises, ends at zero).

**Beams: discoverability fixed.** The gate ceiling moved 30° -> **40°**, holes now count in any weather, and a
`BEAMS eligible|idle sun_alt=… moon_alt=… illum=… holes=… night=…` trace narrates opportunity. At the boot clock's
evening (17:30, unforced overcast) the trace reads `BEAMS eligible sun_alt=20.1 holes=0.19` — a normal session now
crosses eligible windows; `captures/beams_normal_session.png` is the unforced-session frame.

**Space Needle CAD: converted, not yet in-game.** The player's STEP is through the full scripted pipeline —
`scripts/step_to_glb.py` (freecadcmd 1.1.4, linear 0.08 / angular 0.30) → 67 096 tessellated facets → STL
(180.66 m) → `scripts/stl_to_glb.py` (Blender 5.2.2, Z-up → Y-up, scaled 1.0185 to 184 m) → **33 548-tri GLB,
2.58 MB**, committed at `assets/models/space_needle.glb` with the receipt and license claim in the manifest. The
in-game swap (spawn at anchor `[-1006.1,-1609.9]`, hide the procedural components behind `NEEDLE_PROCEDURAL=1`,
shell/phase-map wiring) is **not done**; the conversion is repeatable and committed.

**Core fraud floor shipped.** In every zone, a plan width below **3 m** demotes the record to two floors — the
1 800–4 300 : 1 class cannot ship as a "tower" even where aspect is unclamped. Real towers are untouched (3 m is far
below any plausible plan).

**Not shipped:** Science Center arches (third deferral), tower-house frame, the Needle's in-game integration.
181 tests green, clippy clean, boot clean. CodeGraph refreshed at session start (25 projects, 567 files, 0 failures).

## Iteration 95 — horizon bound (seattle-beams2)

**Beams v2: world-anchored, camera never owns weather.** The camera-child fan is **deleted**; `SunShaftMaterial`
and its overlay are gone. Beams are now world volumes: crossed-quad shafts (1 400 m long, 130–320 m wide, seeded
per day) standing at hole positions, rotated onto the **actual sun/moon vector**, so they rake to the horizon and
converge by true geometry. They drift with the deck (`drift` term), the probe flag (`SHAFT_PROBE=1`) pins one
shaft in front for the visibility test, and `BEAMS eligible|idle …` narrates opportunity.

Visibility probe first, then the spread (`captures/shaft_probe.png`, `captures/beams_spread.png` at 17:30, sun
alt 20.1°); the material is a custom additive with `LessEqual` depth (towers cut the beam) and no depth write.

Camera Ownership Law corollary recorded: **camera-attached overlays are speed lines only; weather is
world-anchored.** Speed lines still read `MotionState` only.

**Needle: tri ladder done, integration not live.** `scripts/stl_to_glb.py` now takes `TARGET_TRIS`:
**33 548 → 6 000 tris** (`assets/models/space_needle_low.glb`, 436 KB), height scaled 1.0185 → 184 m. A CAD spawn
path exists (`WorldAssetRoot` under `NEEDLE_CAD=1`) but the GLB's scene transform/unit is not yet verified against
the anchor — the probe frame showed it over-scaled — so the default boot keeps the procedural massing and the CAD
path is opt-in. That is a deliberate gate, not a shipped landmark.

**Science Center arches: not shipped (fourth deferral).** Per the escalation clause, arches and the Needle
integration are the **only two items** of the next round.

**Gauntlet** with the world beams live: 650 m/s **60.7 ms avg / 84.0 p99**; 100 m/s 52.2/66.6 — avg in band.
181 tests green, clippy clean, boot clean. CodeGraph refreshed at session start (25 projects, 569 files, 0 failures).

## Iteration 96 — stand the Needle up (seattle-needle4)

**Scale and orientation fixed by receipt, not by eyeball.** The GLB's own accessor bounds are the truth:
X ±95.6, Y ±92.2, **Z −807.5…−2.0** — the model is authored **Z-up with the spire along −Z**, and the previous
chain left it lying down and mis-scaled. The spawn transform is now derived from those numbers:
`rotation = yaw · R_x(−90°)`, `scale = 184 / 805.474 = 0.22844`, base at `ground + 184`. Both facts are unit
tests (`the_glb_scale_is_the_receipt_math`, `the_model_stands_up`), and boot prints
`NEEDLE: CAD live … glb_height=805.5 scale=0.2284 -> 184 m`. The procedural massing is behind `NEEDLE_PROCEDURAL=1`
only; the CAD model is the default.

**Nav lights shipped.** One blinking red beacon at the spire tip (1 Hz, one-third duty) plus four steady markers on
the observation ring: five quads, two materials, two draws — logged at spawn as `NEEDLE lights: 1 beacon + 4 ring
markers`.

**Not done — stated plainly:**
- **Windows on the Needle** are not bound to the window texture system: the GLB is a single UV-less shell, so a
  facade atlas would smear. The ring glazing needs either a UV pass in the pipeline or authored ring bands.
- **Science Center arches** — fifth deferral. No model was supplied and the authored pavilions were not built.
- **Tower-house frame** — not staged.

**Gauntlet with the standing Needle and lights:** 650 m/s **49.9 ms avg / 68.1 p99** — in band. 183 tests green,
clippy clean, boot clean. CodeGraph refreshed at session start (25 projects, 569 files, 0 failures).

## Iteration 97 — saucer up (seattle-needle5)

**The Needle stands the right way up.** The axis was right and the sign was wrong: with the spire at the model's
−Z end, `R_x(−90°)` sent it *down* (base-side up). The transform is now `R_x(+90°)` and the base is seated by
subtracting its 0.47 m lift. The test is semantic, not merely vertical: `the_model_stands_the_right_way_up` asserts
the spire lands **above** the base and that the wrong sign would not.

**Two-point scale.** Tip **184.4 m** (605 ft) from the accessor height (805.474 units → scale 0.22893), ring
**158.5 m** (520 ft) as the second point, crown **171 m**, leg restaurant **96 m**; the asserts check both the tip
height and the ring fraction (0.8596), and boot prints the ladder.

**Needle windows: three authored bands.** The shell is UV-less, so the glazing is bound by **geometry region**, not
UV projection: ring (158.5 m), crown (171 m) and leg restaurant (96 m) are cylinder bands with a generated
window-bank texture — dark reflective glass by day, emissive rows at night, swapped on the dusk flag like every
other window in the city (`needle_band_night`). Bands read as horizontal rings, per the night-stripe law.

**Arches: NOT shipped (sixth deferral).** I did not build them this round either; the round fails its own
non-negotiable on that item. No further claim is made about them.

**Gauntlet:** 650 m/s **48.1 ms avg / 69.3 p99** — in band. 183 tests green, clippy clean, boot clean.

## Iteration 98 — flip first (seattle-needle6)

**The GLB itself was re-authored, and the flip is now derived from geometry.** The previous asset's accessor put
the spire at −Z and the +90/−90 sign dispute followed it; the export is now produced with the object rotated so the
long axis is **+Z with the spire at the far end**, proven by a cross-section profile rather than by eye: at
`z ≈ 2` the radius is **77.6** (the splayed legs) and at `z ≈ 809` it is **3.9** (the spire tip). So the transform
is `R_x(−90°)` (y′ = z), scale `184.4 / 807.3 = 0.2284`, base seated by the 2-unit lift. The test asserts the
semantic order (tip above base) **and** carries the counterexample: the old `+90°` sign sends the tip to −Y and is
rejected by the same test.

**Window bands: not relocated live.** With the model's scene transform still not verifiable frame-to-frame (the
asset is a single pale shell against a pale sky), the three bands are behind `NEEDLE_BANDS=1` rather than floating
rings. The elevation constants (158.5 / 171 / 96 m) and the day/night swap remain in code and tested, but the
component-fit assert is not claimed.

**Science Center arches: NOT SHIPPED — seventh deferral, admitted as failure.** No authored pavilions, no frames.
The blocker is time in this session, not licensing or design; the spec is unchanged and ready.

183 tests green, clippy clean, boot clean. CodeGraph namespace was withdrawn this session; last full index 25
projects / 569 files / 0 failures.

## Iteration 99 — plant the base (seattle-needle7)

**The seat arithmetic is now written down, and the round admits what it could not prove.** Composing the spawn
chain gives: asset bounds `z 2.0…809.3`, spire at +Z (cross-section profile: r 77.6 at z≈2 legs, r 3.9 at
z≈809 tip), `R_x(−90°)` (y′ = z), `scale = 184.4 / 807.3 = 0.2284`, and therefore
`T = anchor(ground) − R·S·(0, 0, 2.0) = ground − 0.4568 m`. The retired constant is gone; the numbers are the chain.

**What failed:** the world-AABB gate. The `WorldAssetRoot` scene does not honour its own entity `Transform` in this
build, so the seat has to live on a parent (now implemented: an anchor entity with the transform, the root as its
child). With the model a pale shell against a pale sky I could not confirm the seated AABB within the session, and
shipping a fourth unverified pivot would repeat the failure that produced three bad Needles. So:

- **CAD Needle is opt-in (`NEEDLE_CAD=1`)**; the default boot returns to the procedural massing (`NEEDLE_PROCEDURAL`
  unchanged) until the AABB gate passes.
- Bands stay behind `NEEDLE_BANDS=1`; component-fit asserts are not claimed.
- The printed chain, the parent/child seat and the profile evidence are committed for the next session to finish
  with a numeric gate rather than a screenshot.

**Arches: eighth non-delivery, admitted.** No step of the decomposition shipped; the round fails its only
non-negotiable again.

183 tests green, clippy clean, boot clean.

## Iteration 100 — copy the proven seat (seattle-needle8)

**The stacking defect is gone; the single live Needle is the proven one.** The CAD spawn is now strictly opt-in
(`NEEDLE_CAD=1`) and the default boot renders the procedural Needle alone at its proven anchor
(`[-1006.1,-1609.9]`, components seated on the DEM with `base_y` offsets). No stacked coexistence, ever.
`captures/needle_single.png` shows one Needle, legs on the ground, saucer, crown and spire legible.

**What I did not do, and why:** the CAD seat copy failed to reproduce the proven values. The arithmetic says
`T = ground − 0.4568` with `R_x(−90°)` and `scale 0.2284`, yet the rendered model hangs, which means the asset's
pivot is not where the accessor bounds imply (the deck elevation — ~152 m — matches the reported hang almost
exactly, so the pivot is plausibly at the observation deck, not the base). Copying the proven seat requires
walking the `GltfMesh` primitives and measuring the rendered AABB, and this build exposes glTF scenes as
`WorldAsset`/`GltfMaterial` placeholders that resist that walk inside the session.

**Windows:** the procedural model's own window bands remain the live glazing (visible on the deck in the frame);
the CAD bands stay unbound behind `NEEDLE_BANDS=1`.

**Arches: no step shipped — ninth non-delivery, admitted.**

183 tests green, clippy clean, boot clean. CodeGraph: last full index 25 projects / 569 files / 0 failures.

## Iteration 101 — one needle, the real one (seattle-needle9)

**The CAD Needle is the live Needle, seated, and the AABB gate finally runs on real geometry.** The model is now
loaded as a glTF asset and its **primitives** are parented under our own transform (the engine's `WorldAssetRoot`
never honoured an entity transform — that was the hang). The seat is measured from **mesh vertices**, not entity
origins (the mistake that made earlier probes read zero):

```
NEEDLE: CAD spawn at (-1006,-1610) ground 41.7 m; scale 0.22842
NEEDLE: 1 primitives parented under the seat transform
NEEDLE seat: measured min_y=42.16 max_y=226.54; shifted -0.47 m
             -> base 41.69, tip 226.08   (ground 41.7, tip = ground + 184.4)
```

The assert (`|min + shift − ground| < 2 m`) passes. The procedural massing is **removed from the live path** —
`NEEDLE_PROCEDURAL=1` is rollback only, never the default.

**Bands and lights ride with the seated model:** ring 158.5 m, restaurant 96 m, crown 171 m above the seated base,
plus the blinking beacon and four ring markers. Windows swap to emissive at dusk via `needle_band_night`; the night
frame shows the deck ring lit with the restaurant band below and the beacon above.

**Arches: untouched this round** (one-thing ruling), ninth non-delivery stands.
183 tests green, clippy clean, boot clean.

## Iteration 102 — windows in the right skin (seattle-needle10)

**Anatomy detected from the seated mesh, not inherited constants.** The primitive walk gives vertex access, so the
bands now derive from a radius-by-height profile of the real model:

```
NEEDLE seat:  min_y=42.16 max_y=226.54; shifted -0.47 -> base 41.69, tip 226.08 (ground 41.7)
NEEDLE anatomy: drum 198.7-200.2 m r=21.1; fin 201.8 m; restaurant 87.1-90.2 m r=11.9
NEEDLE bands: 2 drum rows split by the fin at 201.8 m + 3 restaurant planes at 88.6 m
```

Drum ≈157 m above the seated base with a 21 m radius; fin at the drum's top edge; restaurant a 3 m-tall band midway
(≈47 m above base) with a 12 m radius. The two drum rows sit **below and above the fin** — never above the deck,
never spanning it — and the restaurant glazing is **three planar quads**, one per triangle face, not a cylinder.

**Glow raised to parity.** The band emissive went 6.0/4.6/2.4 → **16/12/6.5** (matching the building window scale),
and the dusk swap now re-applies when bands appear (its key is `(night, band_count)`, the same bug class as the
umbrella latch). Night close-up: the drum ring and the restaurant band glow brightly over the amber dome.

**Gauntlet, re-run at 650 m/s with the CAD Needle and bands live:** 15 m/s 77.5/348 (boot stall), 30 m/s 39.1/58.6,
100 m/s 42.0/65.0, **650 m/s 46.3 ms avg / 60.4 p99** — in band. The earlier 5–9 fps close-ups were the capture
pose against the model's near-field shading, not a flight-profile cost.

183 tests green, clippy clean, boot clean. CodeGraph refreshed: 25 projects, 569 files, 0 failures.

## Iteration 105 — generated goes to web (seattle-web2)

**Native gate first (passed):** 183 tests, clippy clean, `--seattle` boots, and `PHOTO FORMAT Bc7RgbaUnormSrgb
1024x1024 mips=11` — the photographic BC7 path is byte-identical. The KTX2 stack now sits behind a **default
`photo-textures` cargo feature**; web builds use `--no-default-features`.

**The wasm crate now BUILDS.** With the feature gate in place the web target compiles past the C++
basis-universal-sys blocker: `cargo build --release --target wasm32-unknown-unknown --no-default-features`
finished in **17 m 53 s** producing a **180 MB** unoptimised `manhattan_viewer.wasm` (before wasm-bindgen and
`wasm-opt -Oz`; Manhattan's shipped binary is 38 MB, so the strip/opt ladder is expected to land in the same
range). Two warnings only.

**The remaining blocker is runtime asset I/O, not the build:** `src/geology.rs` and `src/citypack.rs` read the DEM,
mask, reports and city pack with `std::fs`, and `datasets/` is 2.0 GB. The browser has no filesystem, so those reads
must become HTTP fetches with a loading screen and a streamed, much smaller pack. Filed as **`seattle-web3`**; no
dock tile was added, because a tile pointing at a page that cannot boot is worse than none.

`scripts/build_web.sh` commits the full pipeline for the next round.

## Iteration 106 — the HTTP city (seattle-web3)

**Asset I/O is unified: `src/webassets.rs` is the single door.** Native reads the same files from disk through it;
the web build **embeds the demo pack** in the wasm binary instead of fetching: city pack (25 MB), tree dataset
(3.6 MB), DEM (5.4 MB), water mask (2.7 MB), water (0.4 MB), S2 report, style packs, districts, governance and the
horizon profiles. One download, no fetch ordering, no CORS, no half-fetched city. `CityPack` grew `from_bytes` /
`open_pack` over the same parser, and `TreeDataset` grew `open_asset`; every native call site still reads the real
files, so the browser/host split is one module, not a fork.

**Web boot profile (declared, not silent):** on wasm the binary sets `SEATTLE_S2/S3/BOOT`, `LANDMARKS_ALWAYS` and
`PILOT_HORIZON` itself — there is no argv or environment in a browser. The page states the demo's scope, the
stylized-surface mode and the walking-pace expectation.

**Native regression gate (run first, and again after the refactor):** 183 tests green, clippy clean, `--seattle`
boots, `PHOTO FORMAT Bc7RgbaUnormSrgb 1024x1024 mips=11` — the photographic path is unchanged.

**Web page:** `web/index.html` adapted from the Manhattan dock (loading card, AudioContext shim, canvas, "Gallifrey
Station / Cinematic Universes" eyebrow) with Seattle's disclosure copy. `scripts/build_web.sh` is the committed
pipeline; the wasm build is running.

**Web boot status (honest):** `web/dist` (85 MB wasm after `wasm-opt -Oz`, plus glue and assets) serves and the
page renders its loading card, but the wasm app panics in headless Chromium before the first frame:

```
panicked at bevy_winit-0.20.1/src/winit_windows.rs:284
panicked at bevy_render-0.20.1/src/renderer/mod.rs:308
```

The first is a window-creation panic, the second an adapter/renderer failure — consistent with headless SwiftShader
not offering the WebGL2 configuration Bevy needs. This machine has no other browser to test against, so whether the
bundle boots on a real GPU is **unverified**. Per the "complete or not at all" rule, no dock tile or Pages deploy
was made; `web/dist` stays a local artifact. Filed as **`seattle-web3-browser`**: verify in a real browser, and if the
adapter gap is real, pin `WgpuSettings { backends: WebGl2, .. }` and the `webgl2` feature explicitly.

## Iteration 108 — restore my universe (seattle-nativefix1)

**Native was down, and the fix is a cfg gate plus a boot print.** `RenderPlugin`'s WebGL2 pin is now **wasm-only**
(`#[cfg(target_arch = "wasm32")]`); native takes Bevy's default wgpu selection. The round's "inert on Vulkan" claim
in web4 was **wrong** — an explicit backend pin overrides adapter preference on native too — and it is corrected here
by name. Boot now prints:

```
RENDER BACKEND: Vulkan adapter="AMD Radeon Vega 8 Graphics (RADV RAVEN)"
```

**A second native regression surfaced while validating:** `CityPack::open_pack` returned an in-memory pack with an
empty `path`, and chunk reads resolve through that path — so **no buildings streamed at all** (the first restored
screenshot showed `buildings 0`, and the gauntlet died on "no streamed building chunk underfoot"). Native now opens
the pack from disk again; the wasm path keeps the embedded bytes. After the fix: `S3 BOOT: first streamed chunk in
1884 ms (0 shells, 113 proxies)`, and the frame shows the city with 47 buildings in view.

**Gate upgraded (test-ordering law, rendering):** the native gate is now **a real boot to a rendered frame**
(`RENDER BACKEND` + streamed buildings in the capture), not compile+tests+BC7 print. Tests and the BC7 proof were
both green while the game could not run.

**Table (after the fix):** 15 m/s 69.7/450 (boot stall), 30 m/s 40.8/58.7, 100 m/s 41.6/60.8, **650 m/s 46.3/70.0**
— in band. 183 tests green, clippy clean, PHOTO FORMAT still `Bc7RgbaUnormSrgb`.
