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
