# Generic City Builder — A.2b design (awaiting reviewer approval)

## Scope and contract
Input is `CityDataset v1`: versioned JSON with `buildings[]` (world polygon in
local metres, anchor, height, floor count or pitch, style/era hint, optional
year_built), `roads{}`, and `fields{}` (epicenter points, radial/style parameters,
palette id). Manhattan ships an adapter from `building_perimeters.json`; no
downstream code reads raw Manhattan files. A ten-building synthetic dataset is
the generality test.

## Module inventory (smallest expressive kit)
- `bay_window`: one window-unit wide facade strip, parameterised glass fill,
  frame thickness, sill; designed for per-instance width/height/tint.
- `bay_blank`: blank wall strip for solid/punched facades.
- `floor_band`: spandrel/floor line strip, style-parameterised.
- `corner`: L-shaped corner post at bay resolution.
- `cornice`, `parapet`, `roof_cap`: roofline pieces.
- `storefront_bay`, `entrance_unit`, `stoop`.
Styles compose: glass curtain = tall bay_window strips + thin floor_band +
parapet; brick mid-rise = bay_window at 35–45% glass + brick bay_blank +
cornice; brownstone = narrow bay_window + stoop + brick; concrete grid =
recessed bay_window + heavy floor_band. Variation is instance transform,
width-in-bays, floors and per-instance tint only — never unique per-building
meshes.

## LOD ladder
- Near: full kit composition (bays/floors/corners/trim), one instanced mesh per
  kit piece + material.
- Mid: iteration-12 LOD box (one instanced box per building).
- Far: billboard quad pool.
Thresholds are dataset field parameters; per-building tier exposed on HUD.

## Damage readiness
Pristine buildings stay in the batched kit. On JIT activation the instance is
hidden (per-instance visibility) and the existing destructible generator spawns
a dedicated damaged copy for the wounded building; guts/damage theater remain
as built. This preserves batch rendering and the no-ghost property by
construction; alternatives (submesh range hide, per-instance damage geometry)
are costlier and not needed.

## Materials
Atlas parity is formally dropped. The kit owns a coherent palette (A.1 palette
constants) for all styles; no city atlas dependency. A second city supplies its
own palette id and optional textures with matching UV conventions.

## Generality clauses
Every style rule, LOD threshold, width-in-bays range and palette entry is a
dataset field. City-specific constants (epicenter proxy, radial coefficients)
live only in `fields{}`. A second city needs: polygon/height/style records,
road graph, field parameters, palette.
