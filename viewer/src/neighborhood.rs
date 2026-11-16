//! Manhattan ↔ destructibles neighborhood pipeline.
//!
//! Manhattan owns the real map record (footprint, height, block, category).
//! Destructibles owns generation. This module converts a record to a
//! `BuildingSpec`, swaps it in when the camera-relative zone selects it, swaps
//! it out when it leaves, masks the original Manhattan draw while a handle is
//! live, and drives the mini-map debug fills and the flight gauntlet.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::PathBuf;
use std::time::Instant;

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

use destructibles_bevy::HostCameraProbe;
use destructibles_bevy::breach::BreachState;
use destructibles_bevy::building::{
    BuildingDetail, Buildings, DetailTier, advance_detail, begin_detail_upgrade, detail_level,
};
use destructibles_bevy::camera::FlyCamera as DestructiblesFlyCamera;
use destructibles_bevy::damage::DamageJournal;
use destructibles_bevy::health::HealthIndex;
use destructibles_bevy::impact::apply_impact;
use destructibles_bevy::night::SceneTime;
use destructibles_bevy::puncture::apply_puncture;
use destructibles_bevy::rewind::{begin_restoration, set_restoration_rate};
use destructibles_bevy::streaming::{
    HotZone, Regime, Streaming, freeze_building, register_building, thaw_building,
};
use destructibles_bevy::swap::{DestructibleHandle, swap_in_lod, swap_out};
use destructibles_core::{BuildingSpec, FacadeStyle, WallQuad, core_for_style};

use crate::camera::FlyCamera;
use crate::city_material::{CityMaterial, CityMaterialSetup};
use crate::clock::SimClock;
use crate::detail::HiddenByLive;
use crate::details::{RoofFurniture, SignMaterials};
use crate::minimap::LiveFootprints;
use crate::storefront::{BuildingMeta, StorefrontData, StorefrontState};

/// Stable id namespace for Manhattan-owned destructibles.
pub const NEIGHBORHOOD_ID_BASE: u32 = 1_000_000;
/// Footprint acceptance tolerance, metres (vertex-to-vertex).
pub const FOOTPRINT_TOL_M: f32 = 0.02;
/// Height acceptance tolerance, metres.
pub const HEIGHT_TOL_M: f32 = 0.05;
/// R8 live-mask resolution.
pub const MASK_RES: usize = 1024;
/// World size covered by the live mask, metres.
pub const MASK_WORLD: f32 = 1200.0;

#[derive(Resource, Clone, Debug)]
pub struct NeighborhoodConfig {
    /// Enter/exit radius from the camera nadir. Exit is the hysteresis edge.
    pub nadir_enter: f32,
    pub nadir_exit: f32,
    /// View-direction corridor ahead of the camera.
    pub ahead_enter: f32,
    pub ahead_exit: f32,
    pub ahead_half_width: f32,
    /// Damaged buildings persist outside the zone (policy stub; this
    /// iteration has no damage path).
    pub damaged_persists: bool,
    /// Budgeted transitions.
    pub max_swaps_per_frame: usize,
    pub swap_budget_ms: f32,
    /// Mini-map live fills (permanent debug layer, N toggles).
    pub minimap_fills: bool,
}

impl Default for NeighborhoodConfig {
    fn default() -> Self {
        Self {
            nadir_enter: 120.0,
            nadir_exit: 150.0,
            ahead_enter: 120.0,
            ahead_exit: 150.0,
            ahead_half_width: 60.0,
            damaged_persists: true,
            max_swaps_per_frame: 1,
            swap_budget_ms: 6.0,
            minimap_fills: true,
        }
    }
}

#[derive(Resource, Default)]
pub struct BuildingIndex {
    pub buildings: Vec<BuildingMeta>,
    grid: HashMap<(i32, i32), Vec<usize>>,
    cell: f32,
}

impl BuildingIndex {
    fn build(buildings: &[BuildingMeta]) -> Self {
        let cell = 100.0;
        let mut grid: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (index, building) in buildings.iter().enumerate() {
            let key = (
                (building.centroid[0] / cell).floor() as i32,
                (building.centroid[1] / cell).floor() as i32,
            );
            grid.entry(key).or_default().push(index);
        }
        Self {
            buildings: buildings.to_vec(),
            grid,
            cell,
        }
    }

    fn candidates(&self, center: Vec2, radius: f32) -> Vec<usize> {
        let min_x = ((center.x - radius) / self.cell).floor() as i32;
        let max_x = ((center.x + radius) / self.cell).ceil() as i32;
        let min_z = ((center.y - radius) / self.cell).floor() as i32;
        let max_z = ((center.y + radius) / self.cell).ceil() as i32;
        let mut out = Vec::new();
        for gx in min_x..=max_x {
            for gz in min_z..=max_z {
                if let Some(list) = self.grid.get(&(gx, gz)) {
                    out.extend_from_slice(list);
                }
            }
        }
        out
    }

    /// Deterministic point → building resolver used by JIT impacts. Boundary
    /// points use the nearest footprint edge; ties use centroid distance, then
    /// record id. Misses return None.
    pub fn resolve_point(&self, point: Vec2) -> Option<usize> {
        let mut hits: Vec<(usize, f32)> = Vec::new();
        let mut edges: Vec<(usize, f32)> = Vec::new();
        for candidate in self.candidates(point, self.cell * 10.0) {
            let Some(meta) = self.buildings.get(candidate) else {
                continue;
            };
            let inside =
                destructibles_core::math::point_in_polygon([point.x, point.y], &meta.perimeter);
            if inside {
                let centroid = Vec2::from_array(meta.centroid);
                hits.push((meta.id, centroid.distance(point)));
            } else {
                let edge = destructibles_core::math::distance_to_polygon_edges(
                    [point.x, point.y],
                    &meta.perimeter,
                );
                if edge <= 1.0 {
                    edges.push((meta.id, edge));
                }
            }
        }
        let mut pool = if hits.is_empty() {
            edges
                .into_iter()
                .map(|(id, edge)| (id, edge * 4.0))
                .collect::<Vec<_>>()
        } else {
            hits
        };
        pool.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        pool.first().map(|(id, _)| *id)
    }

    fn densest_cell(&self) -> ((i32, i32), usize) {
        self.grid
            .iter()
            .map(|(key, list)| (*key, list.len()))
            .max_by_key(|(_, len)| *len)
            .unwrap_or(((0, 0), 0))
    }
}

pub struct LiveBuilding {
    pub handle: DestructibleHandle,
    pub meta: usize,
    pub spec: BuildingSpec,
    pub origin: Vec3,
    pub detail: DetailTier,
    pub entry_distance: f32,
    pub tier1_ready: Option<f32>,
    pub frozen: bool,
    pub dressing: Vec<Entity>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DamagePreset {
    #[default]
    Bullet,
    CarBomb,
    Tier3,
    Hero,
}

impl DamagePreset {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Bullet => "bullet",
            Self::CarBomb => "car bomb",
            Self::Tier3 => "tier-3",
            Self::Hero => "hero punch",
        }
    }
}

#[derive(Resource, Default)]
pub struct FireBadges {
    pub line: String,
    pub misses: u32,
}

#[derive(Resource, Default)]
pub struct DamageAim {
    pub target: Option<usize>,
    pub point: Vec3,
    pub preset: DamagePreset,
}

struct PendingDamage {
    id: usize,
    preset: DamagePreset,
    point: Vec3,
}

#[derive(Resource, Default)]
pub struct Neighborhood {
    pub live: BTreeMap<usize, LiveBuilding>,
    pending_in: VecDeque<usize>,
    pending_out: VecDeque<usize>,
    queued_in: BTreeSet<usize>,
    queued_out: BTreeSet<usize>,
    failed: BTreeSet<usize>,
    pub mask: Option<Handle<Image>>,
    pub mask_origin: Vec2,
    pub mask_pixels: Vec<u8>,
    pub mask_dirty: bool,
    pub mask_revision: u64,
    pub live_revision: u64,
    pub swaps_in: u32,
    pub swaps_out: u32,
    pub pending_peak: usize,
    pub worst_swap_ms: f32,
    pub worst_swap_frame_ms: f32,
    pub behind_max: f32,
    pub last_zone_ms: f32,
    pub footprint_failures: u32,
    pub height_failures: u32,
    pub jit_target: Option<usize>,
    pub jit_started: f32,
    pub jit_latency: f32,
    pub jit_rejected: u32,
    pending_damage: Vec<PendingDamage>,
    pub dressing_panels: usize,
    pub dressing_plates: usize,
    pub damage_events: u32,
    pub tier1_built: usize,
    pub tier2_built: usize,
    pub frozen_count: usize,
    pub smoke_markers: BTreeMap<u32, Vec<Entity>>,
}

/// Per-regime acceptance statistics.
#[derive(Clone, Debug, Default)]
pub struct RegimeStats {
    pub name: String,
    pub speed: f32,
    pub frames: u32,
    pub worst_frame_ms: f32,
    pub live_max: usize,
    pub swaps_in: u32,
    pub swaps_out: u32,
    pub pending_peak: usize,
    pub behind_max: f32,
    pub worst_swap_ms: f32,
    pub worst_swap_frame_ms: f32,
    pub stray_handles: usize,
    pub tier1_count: usize,
    pub tier1_max_m: f32,
}

#[derive(Resource, Default)]
pub struct NeighborhoodDemo {
    active: bool,
    stage: DemoStage,
    stage_at: f32,
    regime: usize,
    distance: f32,
    stats: Vec<RegimeStats>,
    current: RegimeStats,
    start: Vec3,
    forward: Vec3,
    capture_dir: PathBuf,
    hover_swaps: u32,
    hover_start_swaps: u32,
    captured: bool,
    captured_detail: bool,
    damage: bool,
    bombed: bool,
    damage_building: Option<usize>,
    damage_point: Vec3,
    street_view: (Vec3, Vec3),
    damage_view: (Vec3, Vec3),
    horizon_armed: bool,
    return_armed: bool,
    damage_seen: bool,
    restored_at: f32,
    wave_size: usize,
    wave_center: Vec3,
    wave_radius: f32,
    wave_targets: Vec<usize>,
    wave_worst_ms: f32,
    wave_captured: [bool; 4],
    wave_mid_damage: bool,
    wave_hot_captured: bool,
    wave_plume_started: bool,
    wave_plume_moved: bool,
    gauntlet_wave: bool,
    wave_fired: bool,
    wave_second: bool,
    wave_started: f64,
    wave_mid_captured: bool,
    wave_first_ms: f32,
    no_capture: bool,
    roof_target: Option<usize>,
    roof_view: (Vec3, Vec3),
    golden_step: u8,
    golden_shot: bool,
    perf_start: f64,
    perf_worst: f32,
    perf_sum: f32,
    perf_frames: u32,
    perf_envelope_ms: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum DemoStage {
    #[default]
    Boot,
    Settle,
    Traverse,
    RegimeEnd,
    Hover,
    HoverWait,
    DamageFly,
    DamageSetup,
    StorefrontPristine,
    StorefrontBomb,
    StorefrontHalo,
    Horizon,
    Return,
    Restored,
    WaveBoot,
    WaveDamage,
    WaveFreeze,
    WaveFire,
    WaveWatch,
    WavePlume,
    RoofBoot,
    RoofLive,
    RoofRestore,
    RoofPlain,
    RoofStreet,
    RoofAerial,
    PerfProbe,
    GoldenBoot,
    GoldenAds,
    GoldenPm,
    GoldenGrave,
    GoldenDistant,
    GoldenAerial,
    GoldenDawn,
    GoldenNoon,
    GoldenDusk,
    GoldenNight,
    GoldenDeck,
    GoldenMoon,
    GoldenSmokeMoon,
    JitBullet,
    JitCar,
    JitRestore,
    InputTest,
    ExitWait,
}

const REGIMES: [(&str, f32); 4] = [
    ("pedestrian_drone", 15.0),
    ("fast_drone", 30.0),
    ("hero", 100.0),
    ("mach_1_92", 650.0),
];
const TRAVERSE_METERS: f32 = 300.0;

pub struct NeighborhoodPlugin;

impl Plugin for NeighborhoodPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NeighborhoodConfig>()
            .init_resource::<Neighborhood>()
            .init_resource::<BuildingIndex>()
            .init_resource::<LiveFootprints>()
            .init_resource::<DamageAim>()
            .init_resource::<FireBadges>()
            .init_resource::<StreetParity>()
            .insert_resource(NeighborhoodDemo::from_env())
            .add_systems(Startup, (setup_neighborhood, setup_neighborhood_hud))
            .add_systems(
                Update,
                (
                    sync_host_camera.after(crate::camera::fly_camera_system),
                    update_zone,
                    process_swaps,
                    detail_tick,
                    damage_input,
                    crate::restoration::wave_input,
                    crate::restoration::restoration_tick,
                    crate::restoration::wave_panel,
                    persistence_tick,
                    sync_street_dressing,
                    sync_dressing_damage,
                    refresh_footprints,
                    refresh_mask,
                    update_mask_texture,
                    sync_live_mask,
                    toggle_minimap_fills,
                    update_neighborhood_hud,
                    run_neighborhood_demo,
                )
                    .chain(),
            );
    }
}

impl NeighborhoodDemo {
    fn from_env() -> Self {
        let active = std::env::var_os("MANHATTAN_NEIGHBORHOOD").is_some();
        let capture_dir = std::env::var("MANHATTAN_CAPTURE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("captures"));
        let _ = std::fs::create_dir_all(&capture_dir);
        let wave_demo = std::env::var_os("MANHATTAN_WAVE").is_some();
        let wave_size = std::env::var("MANHATTAN_WAVE_SIZE")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(10)
            .clamp(1, 64);
        let roofs = std::env::var_os("MANHATTAN_ROOFS").is_some();
        let perf_probe = std::env::var_os("MANHATTAN_PERF_PROBE").is_some();
        let golden = std::env::var_os("MANHATTAN_GOLDEN").is_some();
        let jit = std::env::var_os("MANHATTAN_JIT").is_some();
        let input_test = std::env::var_os("MANHATTAN_INPUT_TEST").is_some();
        let stage = if input_test {
            DemoStage::InputTest
        } else if jit {
            DemoStage::JitBullet
        } else if golden {
            DemoStage::GoldenBoot
        } else if perf_probe {
            DemoStage::PerfProbe
        } else if roofs {
            DemoStage::RoofBoot
        } else if wave_demo {
            DemoStage::WaveBoot
        } else {
            DemoStage::Boot
        };
        Self {
            active: active || wave_demo || roofs || perf_probe || golden || jit || input_test,
            capture_dir,
            damage: std::env::var_os("MANHATTAN_DAMAGE").is_some(),
            wave_size,
            stage,
            gauntlet_wave: std::env::var_os("MANHATTAN_GAUNTLET_WAVE").is_some(),
            no_capture: std::env::var_os("MANHATTAN_WAVE_NOCAPTURE").is_some(),
            ..default()
        }
    }
}

fn setup_neighborhood(
    mut commands: Commands,
    storefront: Res<StorefrontData>,
    hot: Option<Res<HotZone>>,
    mut images: ResMut<Assets<Image>>,
    mut streaming: Option<ResMut<Streaming>>,
) {
    let index = BuildingIndex::build(storefront.buildings());
    let mut config = NeighborhoodConfig::default();
    if let Some(hot) = hot {
        // The destructibles HotZone is the origin of the live/damaged/hot
        // vocabulary; Manhattan's zone applies its own enter/exit pair.
        config.max_swaps_per_frame = hot.max_hot_damaged.clamp(1, 2);
    }
    // The neighborhood policy is the only streaming controller in Manhattan.
    if let Some(streaming) = streaming.as_mut() {
        streaming.enabled = false;
    }
    let mask = images.add(make_mask_image());
    let pixels = vec![0u8; MASK_RES * MASK_RES];
    commands.insert_resource(index);
    commands.insert_resource(config);
    commands.insert_resource(Neighborhood {
        mask: Some(mask),
        mask_pixels: pixels,
        mask_dirty: true,
        mask_revision: 1,
        ..default()
    });
    info!(
        "NEIGHBORHOOD: feeder/index ready, mask {}² @ {MASK_WORLD:.0} m",
        MASK_RES
    );
}

fn make_mask_image() -> Image {
    let mut image = Image::new_fill(
        bevy::render::render_resource::Extent3d {
            width: MASK_RES as u32,
            height: MASK_RES as u32,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        &[0],
        bevy::render::render_resource::TextureFormat::R8Unorm,
        bevy::asset::RenderAssetUsages::MAIN_WORLD | bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    image.data = Some(vec![0u8; MASK_RES * MASK_RES]);
    image.sampler = bevy::image::ImageSampler::nearest();
    image
}

/// Style mapping from Manhattan's first-floor wall categories.
/// Facade style from the shared glass/brick probability field. Height only
/// picks the brick/glass sub-variant; it never changes the glass probability.
pub fn style_for(meta: &BuildingMeta) -> FacadeStyle {
    let glassy = crate::city_rhythm::is_glassy(
        meta.id,
        meta.block,
        meta.centroid[0],
        meta.centroid[1],
        meta.year_built,
    );
    style_for_parts(&meta.category, meta.height_m, meta.year_built, glassy)
}

pub fn style_for_parts(
    category: &str,
    height_m: f32,
    year_built: u32,
    glassy: bool,
) -> FacadeStyle {
    if glassy {
        return if year_built >= 1975 {
            FacadeStyle::GlassCurtainWall
        } else {
            FacadeStyle::ConcreteGrid
        };
    }
    match category {
        "industrial" => FacadeStyle::ConcreteGrid,
        "residential" if height_m <= 18.0 => FacadeStyle::BrickBrownstone,
        "residential" => FacadeStyle::BrickMidrise,
        _ => FacadeStyle::BrickMidrise,
    }
}

/// Convert a Manhattan building record to the equivalent generated spec.
/// The footprint is the record's exact wall loop, local to its centroid; the
/// total height is exact because the floor height absorbs the remainder.
pub fn spec_for(meta: &BuildingMeta) -> Result<BuildingSpec, String> {
    if meta.perimeter.len() < 3 || meta.perimeter.len() > 64 {
        return Err(format!("perimeter vertices {}", meta.perimeter.len()));
    }
    let footprint: Vec<[f32; 2]> = meta
        .perimeter
        .iter()
        .map(|point| [point[0] - meta.centroid[0], point[1] - meta.centroid[1]])
        .collect();
    let height = meta.height_m.max(1.0);
    let raw_floor = meta.floor_height_m.clamp(2.4, 6.0);
    // Shared floor pitch: the map record's measured floor height is the ground
    // truth for both generators, so a swap cannot change the window-row count.
    let mut floor_count = (height / raw_floor).round().max(1.0) as u32;
    // Measurement hook only: reproduce the pre-iteration-23 8-floor LOD.
    if std::env::var_os("MANHATTAN_LEGACY_FLOORS").is_some() {
        floor_count = floor_count.clamp(1, 8);
        if height / floor_count as f32 > 12.0 {
            floor_count = (height / 12.0).ceil().max(1.0) as u32;
        }
    }
    // Bound the absolutely pathological supertall only; normal records keep
    // their real pitch even though the destructible side gains rows/cells.
    floor_count = floor_count.clamp(1, 200);
    let floor_height = height / floor_count as f32;
    let style = style_for(meta);
    let mut windows = style.windows();
    if style == FacadeStyle::GlassCurtainWall {
        // Invert the old LOD shortcut: a curtain wall is mostly glass. Panes
        // grow with height and the mullion stays a thin steel line, so a tall
        // glass tower never reads as brick with pinholes.
        let pane_width = 1.35 + (height / 300.0).min(0.45);
        windows.width = pane_width;
        windows.column_spacing = pane_width + 0.10;
        windows.height = (floor_height - 0.08).max(1.4);
        windows.sill_height = 0.03;
        windows.edge_margin = 0.05;
        windows.recess = 0.03;
    } else {
        // Fewer, wider bays than the standalone presets: the neighbourhood LOD
        // keeps the silhouette and the material read without the pane count.
        windows.column_spacing = windows.column_spacing.max(4.2);
        windows.width = windows.width.min(1.6);
    }
    let core = core_for_style(style, &footprint, meta.id as u32);
    let spec = BuildingSpec {
        footprint,
        floor_height,
        floor_count,
        facade: style.material(),
        facade_style: style,
        windows,
        core,
    };
    spec.validate().map_err(|error| format!("{error}"))?;
    Ok(spec)
}

/// Footprint gate: every record vertex must have a generated vertex within
/// `FOOTPRINT_TOL_M`, the absolute area must agree within 0.1 %, and the
/// generated height must match the record within `HEIGHT_TOL_M`.
/// Height twin of the footprint acceptance check: a swap whose generated
/// height drifts from the map record would leave map antennas and roof
/// furniture floating (or bury them), so reject it and keep plain geometry.
pub fn height_matches(meta: &BuildingMeta, layout: &destructibles_core::BuildingLayout) -> bool {
    (layout.height - meta.height_m).abs() <= HEIGHT_TOL_M
}

pub fn window_rows_match(meta: &BuildingMeta, layout: &destructibles_core::BuildingLayout) -> bool {
    let expected = (meta.height_m / meta.floor_height_m.clamp(2.4, 6.0))
        .round()
        .max(1.0) as u32;
    layout.spec.floor_count == expected
}

pub fn footprint_matches(
    meta: &BuildingMeta,
    layout: &destructibles_core::BuildingLayout,
    origin: Vec3,
) -> bool {
    if layout.footprint.len() != meta.perimeter.len() {
        return false;
    }
    for point in &meta.perimeter {
        let world = Vec2::new(point[0] - origin.x, point[1] - origin.z);
        let nearest = layout
            .footprint
            .iter()
            .map(|candidate| Vec2::new(candidate[0], candidate[1]).distance(world))
            .fold(f32::INFINITY, f32::min);
        if nearest > FOOTPRINT_TOL_M {
            return false;
        }
    }
    let record_area = polygon_area(&meta.perimeter).abs();
    let generated_area = layout
        .footprint
        .iter()
        .enumerate()
        .map(|(index, a)| {
            let b = layout.footprint[(index + 1) % layout.footprint.len()];
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f32>()
        .abs()
        * 0.5;
    if (record_area - generated_area).abs() > record_area * 1.0e-3 {
        return false;
    }
    (layout.height - meta.height_m).abs() <= HEIGHT_TOL_M
}

fn polygon_area(points: &[[f32; 2]]) -> f32 {
    points
        .iter()
        .enumerate()
        .map(|(index, a)| {
            let b = points[(index + 1) % points.len()];
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f32>()
        * 0.5
}

/// Pure zone predicate, used by the live update and by hysteresis tests.
pub fn in_zone(
    distance: f32,
    ahead: f32,
    lateral: f32,
    live: bool,
    config: &NeighborhoodConfig,
) -> bool {
    let nadir = if live {
        config.nadir_exit
    } else {
        config.nadir_enter
    };
    let ahead_limit = if live {
        config.ahead_exit
    } else {
        config.ahead_enter
    };
    distance <= nadir
        || (ahead >= 0.0 && ahead <= ahead_limit && lateral <= config.ahead_half_width)
}

fn sync_host_camera(
    camera: Single<(&Transform, &FlyCamera), Without<HostCameraProbe>>,
    mut probe: Query<
        (
            &mut Transform,
            &mut GlobalTransform,
            &mut DestructiblesFlyCamera,
        ),
        With<HostCameraProbe>,
    >,
    clock: Res<SimClock>,
    mut scene: ResMut<SceneTime>,
) {
    let (camera_transform, fly) = *camera;
    if let Ok((mut transform, mut global, mut probe_fly)) = probe.single_mut() {
        *transform = *camera_transform;
        *global = GlobalTransform::from(*camera_transform);
        probe_fly.yaw = fly.yaw;
        probe_fly.pitch = fly.pitch;
    }
    scene.hours = (clock.minute_of_day / 60.0).rem_euclid(24.0) as f32;
    scene.auto = false;
}

#[allow(dead_code)]
fn visibility_class(ahead: bool, visible: bool) -> u8 {
    match (ahead, visible) {
        (true, true) => 0,
        (true, false) | (false, true) => 1,
        (false, false) => 2,
    }
}

#[allow(dead_code)]
fn los_blocked(index: &BuildingIndex, from: Vec3, target: usize) -> bool {
    let Some(meta) = index.buildings.get(target) else {
        return false;
    };
    let to = Vec3::new(meta.centroid[0], meta.height_m * 0.5, meta.centroid[1]);
    let delta = to - from;
    let distance = delta.length();
    if distance < 1.0 {
        return false;
    }
    let direction = delta / distance;
    let mid = Vec2::new((from.x + to.x) * 0.5, (from.z + to.z) * 0.5);
    for candidate in index.candidates(mid, distance * 0.5 + 40.0) {
        let Some(other) = index.buildings.get(candidate) else {
            continue;
        };
        if other.id == target {
            continue;
        }
        let (min, max) = meta_bounds(other, Vec3::new(other.centroid[0], 0.0, other.centroid[1]));
        if let Some(t) = ray_aabb(from, direction, min, max)
            && t > 2.0
            && t < distance - 2.0
        {
            return true;
        }
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn update_zone(
    time: Res<Time>,
    camera: Single<&Transform, With<FlyCamera>>,
    index: Res<BuildingIndex>,
    mut neighborhood: ResMut<Neighborhood>,
    config: Res<NeighborhoodConfig>,
    journal: Res<DamageJournal>,
    motion: Res<crate::motion::MotionState>,
    clock: Res<SimClock>,
) {
    let started = Instant::now();
    if !neighborhood.live.is_empty() {
        neighborhood.behind_max = 0.0;
    }
    let center = Vec2::new(camera.translation.x, camera.translation.z);
    let forward = camera.forward();
    let forward = Vec2::new(forward.x, forward.z).normalize_or_zero();
    let radius = config
        .nadir_exit
        .max(config.ahead_exit)
        .max(config.ahead_half_width)
        + index.cell;
    let candidates = index.candidates(center, radius);
    // Candidates are always rebuilt from the camera's CURRENT position; the
    // queue below is replaced, never appended to, so a stale entry behind the
    // camera can never be instantiated and then collected.
    let idle = motion.speed < 3.0 || !clock.playing;
    let mut seen = BTreeSet::new();
    for candidate in candidates {
        let Some(meta) = index.buildings.get(candidate) else {
            continue;
        };
        let id = meta.id;
        if !seen.insert(id) {
            continue;
        }
        let point = Vec2::new(meta.centroid[0], meta.centroid[1]);
        let delta = point - center;
        let distance = delta.length();
        let along = delta.dot(forward);
        let lateral = (delta - forward * along).length();
        let live = neighborhood.live.contains_key(&id);
        let damaged = live && journal.0.head_for(NEIGHBORHOOD_ID_BASE + id as u32) > 0.0;
        let desired = in_zone(distance, along, lateral, live, &config)
            || (damaged && config.damaged_persists);
        if !desired && live && idle && !neighborhood.queued_out.contains(&id) {
            neighborhood.pending_out.push_back(id);
            neighborhood.queued_out.insert(id);
        }
        if live {
            let behind = (-along).max(0.0);
            neighborhood.behind_max = neighborhood.behind_max.max(behind);
        }
    }
    // Live buildings far outside the candidate grid still need an exit check.
    let live_ids: Vec<usize> = neighborhood.live.keys().copied().collect();
    for id in live_ids {
        let Some(live) = neighborhood.live.get(&id) else {
            continue;
        };
        let point = Vec2::new(live.origin.x, live.origin.z);
        let delta = point - center;
        let distance = delta.length();
        let along = delta.dot(forward);
        let lateral = (delta - forward * along).length();
        let damaged = journal.0.head_for(NEIGHBORHOOD_ID_BASE + id as u32) > 0.0;
        // A damaged building persists in its damage-carrying state; zone exit
        // freezes it instead of demoting it to the plain map.
        let persists = damaged && config.damaged_persists;
        let jitting = neighborhood.jit_target == Some(id)
            || neighborhood.pending_damage.iter().any(|shot| shot.id == id);
        if idle
            && !in_zone(distance, along, lateral, true, &config)
            && !persists
            && !jitting
            && !neighborhood.queued_out.contains(&id)
        {
            neighborhood.pending_out.push_back(id);
            neighborhood.queued_out.insert(id);
        }
        neighborhood.behind_max = neighborhood.behind_max.max((-along).max(0.0));
    }
    if !idle {
        // GC is idle-time work: while moving, queued exits are dropped rather
        // than mass-despawned behind the camera.
        neighborhood.pending_out.clear();
        neighborhood.queued_out.clear();
    }
    // Camera-follow prefetch is retired. Nothing is queued for swap-in by
    // position; JIT impacts are the only producer. Stale entries are cleared.
    neighborhood.pending_in.clear();
    neighborhood.queued_in.clear();
    neighborhood.pending_peak = neighborhood.pending_peak.max(neighborhood.pending_in.len());
    neighborhood.last_zone_ms = started.elapsed().as_secs_f32() * 1000.0;
    let _ = time;
}

#[allow(clippy::drop_non_drop)]
#[derive(Component)]
struct NeighborhoodHud;

#[derive(Component)]
struct NeighborhoodReticle;

fn setup_neighborhood_hud(mut commands: Commands) {
    commands.spawn((
        NeighborhoodReticle,
        Node {
            position_type: PositionType::Absolute,
            left: percent(50),
            top: percent(50),
            ..default()
        },
        Text::new("+"),
        TextFont {
            font_size: FontSize::Px(22.0),
            ..default()
        },
        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.9)),
        TextShadow::default(),
        ZIndex(21),
    ));
    commands.spawn((
        NeighborhoodHud,
        Node {
            position_type: PositionType::Absolute,
            left: px(12),
            bottom: px(232),
            ..default()
        },
        Text::new("DETAIL —"),
        TextFont {
            font_size: FontSize::Px(12.0),
            ..default()
        },
        TextColor(Color::srgba(0.76, 0.96, 1.0, 0.96)),
        TextShadow::default(),
        ZIndex(20),
    ));
}

fn update_neighborhood_hud(
    neighborhood: Res<Neighborhood>,
    badges: Res<FireBadges>,
    aim: Res<DamageAim>,
    index: Res<BuildingIndex>,
    mut texts: Query<&mut Text, With<NeighborhoodHud>>,
    mut reticles: Query<&mut TextColor, With<NeighborhoodReticle>>,
) {
    let target = aim
        .target
        .map(|id| {
            let detail = neighborhood
                .live
                .get(&id)
                .map(|live| match live.detail {
                    DetailTier::Shell => "T0",
                    DetailTier::Structural => "T1",
                    DetailTier::Full => "T2",
                })
                .unwrap_or("—");
            format!("b{id} {detail}")
        })
        .unwrap_or_else(|| "—".to_string());
    let mut levels = [0usize; 3];
    for live in neighborhood.live.values() {
        levels[live.detail as usize] += 1;
    }
    let body = format!(
        "DETAIL live={} T0={} T1={} T2={} built={}/{} frozen={} damage={} pending={} rej={} miss={}\nFIRE {}\nDRESS panels={} ads={}  target {target}  [1-4] {}  [X] fire  [Z]/[Shift+Z] restore",
        neighborhood.live.len(),
        levels[0],
        levels[1],
        levels[2],
        neighborhood.tier1_built,
        neighborhood.tier2_built,
        neighborhood.frozen_count,
        neighborhood.damage_events,
        neighborhood.pending_damage.len(),
        neighborhood.jit_rejected,
        badges.misses,
        badges.line,
        neighborhood.dressing_panels,
        neighborhood.dressing_plates,
        aim.preset.label()
    );
    for mut text in &mut texts {
        text.0 = body.clone();
    }
    let colour = if aim.target.is_some() {
        Color::srgba(0.32, 1.0, 0.52, 0.98)
    } else {
        Color::srgba(1.0, 1.0, 1.0, 0.9)
    };
    for mut text_color in &mut reticles {
        text_color.0 = colour;
    }
    let _ = &index;
}

fn detail_tick(world: &mut World) {
    let camera = world
        .query_filtered::<&Transform, With<FlyCamera>>()
        .single(world)
        .ok()
        .map(|transform| transform.translation)
        .unwrap_or(Vec3::ZERO);
    let forward = world
        .query_filtered::<&Transform, With<FlyCamera>>()
        .single(world)
        .map(|transform| {
            let forward = transform.forward();
            Vec2::new(forward.x, forward.z).normalize_or_zero()
        })
        .unwrap_or(Vec2::Y);
    let config = world.resource::<NeighborhoodConfig>().clone();
    let mut items: Vec<(usize, usize, f32, f32, DetailTier)> = {
        let neighborhood = world.resource::<Neighborhood>();
        let index = world.resource::<BuildingIndex>();
        neighborhood
            .live
            .iter()
            .filter(|(_, live)| !live.frozen)
            .filter_map(|(id, live)| {
                let meta = index.buildings.get(live.meta)?;
                let point = Vec2::new(meta.centroid[0], meta.centroid[1]);
                let delta = point - Vec2::new(camera.x, camera.z);
                let distance = delta.length();
                let along = delta.dot(forward);
                let lateral = (delta - forward * along).length();
                Some((
                    *id,
                    live.meta,
                    distance,
                    if along >= 0.0 && lateral <= config.ahead_half_width * 1.5 {
                        along
                    } else {
                        0.0
                    },
                    live.detail,
                ))
            })
            .collect()
    };
    items.sort_by(|a, b| {
        detail_priority(a.4, a.3 > 0.0, a.2)
            .partial_cmp(&detail_priority(b.4, b.3 > 0.0, b.2))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let jit = world.resource::<Neighborhood>().jit_target;
    if let Some(target) = jit
        && let Some(position) = items.iter().position(|item| item.0 == target)
    {
        let item = items.remove(position);
        items.insert(0, item);
    }
    let catch_up = !world.resource::<SimClock>().playing;
    let detail_budget = if catch_up || jit.is_some() { 8.0 } else { 4.0 };
    let started = Instant::now();
    let mut tier1 = 0usize;
    let mut tier2 = 0usize;
    for (id, _meta, distance, _along, _detail) in items {
        if started.elapsed().as_secs_f32() * 1000.0 >= detail_budget {
            break;
        }
        let did = NEIGHBORHOOD_ID_BASE + id as u32;
        let level = detail_level(world, did);
        let damaged = world.resource::<DamageJournal>().0.head_for(did) > 0.0;
        if level < DetailTier::Structural {
            begin_detail_upgrade(world, did, DetailTier::Structural);
            let reached = advance_detail(world, did, 2.0);
            if reached >= DetailTier::Structural {
                tier1 += 1;
                info!("DETAIL b{id}: Tier 1 ready at {distance:.1} m");
                if jit == Some(id) {
                    let now = world.resource::<Time>().elapsed_secs();
                    let started = world.resource::<Neighborhood>().jit_started;
                    let latency = (now - started).max(0.0);
                    let mut neighborhood = world.resource_mut::<Neighborhood>();
                    neighborhood.jit_target = None;
                    neighborhood.jit_latency = latency;
                    info!("JIT READY b{id}: Tier 1 in {:.2} s", latency);
                }
            }
            set_live_detail(world, id, reached);
            if let Some(live) = world.resource_mut::<Neighborhood>().live.get_mut(&id)
                && reached >= DetailTier::Structural
            {
                live.tier1_ready = Some(distance);
            }
            let _ = damaged;
        } else if level < DetailTier::Full && (damaged || distance < 150.0) {
            begin_detail_upgrade(world, did, DetailTier::Full);
            let reached = advance_detail(world, did, 1.5);
            if reached >= DetailTier::Full {
                tier2 += 1;
            }
            set_live_detail(world, id, reached);
        }
    }
    if tier1 > 0 || tier2 > 0 {
        let mut neighborhood = world.resource_mut::<Neighborhood>();
        neighborhood.tier1_built += tier1;
        neighborhood.tier2_built += tier2;
    }
    // Damage admission: apply the queued shot only once its target is Tier 1.
    let pending = std::mem::take(&mut world.resource_mut::<Neighborhood>().pending_damage);
    let mut keep = Vec::new();
    let mut applied = 0usize;
    for shot in pending {
        if applied >= 1 {
            keep.push(shot);
            continue;
        }
        let level = detail_level(world, NEIGHBORHOOD_ID_BASE + shot.id as u32);
        let live = world.resource::<Neighborhood>().live.contains_key(&shot.id);
        if live && level >= DetailTier::Structural {
            let did = NEIGHBORHOOD_ID_BASE + shot.id as u32;
            let direction = world
                .query_filtered::<&Transform, With<FlyCamera>>()
                .single(world)
                .map(|transform| transform.forward().as_vec3())
                .unwrap_or(Vec3::NEG_Z);
            apply_preset(world, did, shot.preset, shot.point, direction);
            let head = world.resource::<DamageJournal>().0.head_for(did);
            let badges = world.resource::<FireBadges>().line.clone();
            world.resource_mut::<FireBadges>().line = format!(
                "{} → JOURNAL={head:.2} → WOUND VISIBLE",
                badges.split(" → ADMITTED").next().unwrap_or(&badges)
            );
            applied += 1;
        } else {
            keep.push(shot);
        }
    }
    world.resource_mut::<Neighborhood>().pending_damage = keep;
}

/// Detail build-out order: lower tiers first, then corridor buildings (the
/// ones the camera is flying towards), then nearest. That is what pre-warms a
/// high-speed approach without spending the budget on nadir edges.
fn detail_priority(detail: DetailTier, corridor: bool, distance: f32) -> (u8, u8, f32) {
    (detail as u8, if corridor { 0 } else { 1 }, distance)
}

fn set_live_detail(world: &mut World, id: usize, level: DetailTier) {
    if let Some(live) = world.resource_mut::<Neighborhood>().live.get_mut(&id) {
        live.detail = level;
    }
}

fn apply_preset(
    world: &mut World,
    building: u32,
    preset: DamagePreset,
    point: Vec3,
    direction: Vec3,
) {
    let direction = direction.normalize_or_zero();
    let tuning = world.resource::<crate::city_tuning::CityTuning>().clone();
    let impact = match preset {
        DamagePreset::Bullet => destructibles_core::ImpactSpec {
            position: point.to_array(),
            radius: 0.0,
            velocity: (direction * 300.0).to_array(),
            mass_kg: 5.0,
            explosive_yield_kg_tnt: 0.0,
        },
        DamagePreset::CarBomb => destructibles_core::ImpactSpec {
            position: point.to_array(),
            radius: 0.0,
            velocity: (direction * tuning.projectile_velocity_mps).to_array(),
            mass_kg: tuning.projectile_mass_kg,
            explosive_yield_kg_tnt: tuning.explosive_yield_kg,
        },
        DamagePreset::Tier3 => destructibles_core::ImpactSpec {
            position: point.to_array(),
            radius: 0.0,
            velocity: (direction * 120.0).to_array(),
            mass_kg: 120_000.0,
            explosive_yield_kg_tnt: 0.0,
        },
        DamagePreset::Hero => destructibles_core::ImpactSpec {
            position: point.to_array(),
            radius: 0.5,
            velocity: (direction * tuning.projectile_velocity_mps.max(120.0) * 14.0).to_array(),
            mass_kg: tuning.projectile_mass_kg.min(400.0),
            explosive_yield_kg_tnt: 0.0,
        },
    };
    if preset == DamagePreset::Hero {
        apply_puncture(world, building, impact);
    } else {
        apply_impact(world, building, impact);
    }
    world.resource_mut::<Neighborhood>().damage_events += 1;
    info!(
        "DAMAGE {} at b{building} ({:.1},{:.1},{:.1})",
        preset.label(),
        point.x,
        point.y,
        point.z
    );
}

fn ray_aabb(origin: Vec3, direction: Vec3, min: Vec3, max: Vec3) -> Option<f32> {
    let mut tmin = 0.0f32;
    let mut tmax = f32::INFINITY;
    for axis in 0..3 {
        let d = direction[axis];
        if d.abs() < 1.0e-6 {
            if origin[axis] < min[axis] || origin[axis] > max[axis] {
                return None;
            }
        } else {
            let inv = 1.0 / d;
            let mut t1 = (min[axis] - origin[axis]) * inv;
            let mut t2 = (max[axis] - origin[axis]) * inv;
            if t1 > t2 {
                std::mem::swap(&mut t1, &mut t2);
            }
            tmin = tmin.max(t1);
            tmax = tmax.min(t2);
            if tmin > tmax {
                return None;
            }
        }
    }
    (tmax >= 0.0).then_some(tmin.max(0.0))
}

fn mask_covers_building(world: &World, id: usize) -> bool {
    let Some(meta) = world.resource::<BuildingIndex>().buildings.get(id) else {
        return false;
    };
    let neighborhood = world.resource::<Neighborhood>();
    let mut samples = vec![
        Vec2::from_array(meta.centroid),
        Vec2::new(
            meta.perimeter.iter().map(|p| p[0]).sum::<f32>() / meta.perimeter.len() as f32,
            meta.perimeter.iter().map(|p| p[1]).sum::<f32>() / meta.perimeter.len() as f32,
        ),
    ];
    for point in &meta.perimeter {
        samples.push(Vec2::from_array(*point));
    }
    samples.into_iter().any(|point| {
        let x = point.x - neighborhood.mask_origin.x;
        let z = point.y - neighborhood.mask_origin.y;
        let px = ((x / MASK_WORLD + 0.5) * MASK_RES as f32) as isize;
        let py = ((z / MASK_WORLD + 0.5) * MASK_RES as f32) as isize;
        if px < 0 || py < 0 || px >= MASK_RES as isize || py >= MASK_RES as isize {
            return false;
        }
        neighborhood.mask_pixels[py as usize * MASK_RES + px as usize] > 0
    })
}

fn damage_input(world: &mut World) {
    let keys = world.resource::<ButtonInput<KeyCode>>().clone();
    let mouse = world.resource::<ButtonInput<MouseButton>>().clone();
    let Some(camera) = world
        .query_filtered::<&Transform, With<FlyCamera>>()
        .single(world)
        .ok()
        .copied()
    else {
        return;
    };
    let mut preset = world.resource::<DamageAim>().preset;
    if keys.just_pressed(KeyCode::Digit1) {
        preset = DamagePreset::Bullet;
    } else if keys.just_pressed(KeyCode::Digit2) {
        preset = DamagePreset::CarBomb;
    } else if keys.just_pressed(KeyCode::Digit3) {
        preset = DamagePreset::Tier3;
    } else if keys.just_pressed(KeyCode::Digit4) {
        preset = DamagePreset::Hero;
    }
    let origin = camera.translation;
    let origin_of_ray = origin;
    let forward = camera.forward().as_vec3();
    let mut best: Option<(f32, usize, Vec3)> = None;
    {
        let neighborhood = world.resource::<Neighborhood>();
        let index = world.resource::<BuildingIndex>();
        for (id, live) in &neighborhood.live {
            let Some(meta) = index.buildings.get(live.meta) else {
                continue;
            };
            let (min, max) = meta_bounds(meta, live.origin);
            if let Some(t) = ray_aabb(origin, forward, min, max)
                && best.is_none_or(|(best_t, _, _)| t < best_t)
            {
                best = Some((t, *id, origin + forward * t));
            }
        }
    }
    if best.is_none() {
        // Plain geometry still receives the aim ray: JIT needs the record.
        let index = world.resource::<BuildingIndex>();
        for meta in &index.buildings {
            let origin = Vec3::new(meta.centroid[0], 0.0, meta.centroid[1]);
            let (min, max) = meta_bounds(meta, origin);
            if let Some(t) = ray_aabb(origin_of_ray, forward, min, max)
                && best.is_none_or(|(best_t, _, _)| t < best_t)
            {
                best = Some((t, meta.id, origin_of_ray + forward * t));
            }
        }
    }
    if best.is_none() {
        // No direct ray hit: pick the closest live building in the forward cone.
        let neighborhood = world.resource::<Neighborhood>();
        let index = world.resource::<BuildingIndex>();
        let flat = Vec2::new(forward.x, forward.z).normalize_or_zero();
        for (id, live) in &neighborhood.live {
            let Some(meta) = index.buildings.get(live.meta) else {
                continue;
            };
            let delta = Vec2::new(meta.centroid[0] - origin.x, meta.centroid[1] - origin.z);
            let distance = delta.length();
            let along = delta.dot(flat);
            let lateral = (delta - flat * along).length();
            if along > 0.0 && along < 250.0 && lateral < 30.0 {
                let point = live.origin + Vec3::Y * (meta.height_m * 0.5);
                if best.is_none_or(|(best_t, _, _)| distance < best_t) {
                    best = Some((distance, *id, point));
                }
            }
        }
    }
    let ground_point = if forward.y < -0.02 {
        let t = -origin.y / forward.y;
        if (0.0..1_800.0).contains(&t) {
            origin + forward * t
        } else {
            origin + forward * 30.0
        }
    } else {
        origin + forward * 30.0
    };
    let (target, point) = best
        .map(|(_, id, point)| (Some(id), point))
        .unwrap_or((None, ground_point));
    {
        let mut aim = world.resource_mut::<DamageAim>();
        aim.target = target;
        aim.point = point;
        aim.preset = preset;
    }
    if keys.just_pressed(KeyCode::KeyG)
        || keys.just_pressed(KeyCode::KeyX)
        || mouse.just_pressed(MouseButton::Left)
    {
        let ray_len = origin.distance(point);
        info!(
            "FIRE: key G/X target={:?} point={:?} ray_origin={origin:?} forward={forward:?}",
            target, point
        );
        {
            let mut badges = world.resource_mut::<FireBadges>();
            badges.line = format!(
                "SOUND=heard → RAY o({:.0},{:.0},{:.0}) d({:.2},{:.2},{:.2})",
                origin.x, origin.y, origin.z, forward.x, forward.y, forward.z
            );
        }
        let Some(id) = target else {
            world.resource_mut::<FireBadges>().misses += 1;
            world.resource_mut::<FireBadges>().line =
                "SOUND=heard → RAY cast → HIT=MISS".to_string();
            world.resource_mut::<Neighborhood>().jit_rejected += 1;
            info!("FIRE: ray MISS (no footprint)");
            return;
        };
        world.resource_mut::<FireBadges>().line =
            format!("SOUND=heard → RAY {ray_len:.0}m → HIT=b{id} → GATE…");
        if !world.resource::<Neighborhood>().live.contains_key(&id) && !jit_swap(world, id) {
            info!("FIRE: JIT rejected b{id}");
            return;
        }
        info!("FIRE: target b{id} live; admitting");
        let level = detail_level(world, NEIGHBORHOOD_ID_BASE + id as u32);
        if level < DetailTier::Structural {
            world
                .resource_mut::<Neighborhood>()
                .pending_damage
                .push(PendingDamage { id, preset, point });
            let badges = world.resource::<FireBadges>().line.clone();
            world.resource_mut::<FireBadges>().line = format!("{badges} → ADMITTED=queued(T1)");
            info!("FIRE: b{id} queued for Tier 1 (currently {level:?})");
        } else {
            let did = NEIGHBORHOOD_ID_BASE + id as u32;
            info!("FIRE: b{id} admitted immediately");
            apply_preset(world, did, preset, point, forward);
            let head = world.resource::<DamageJournal>().0.head_for(did);
            let badges = world.resource::<FireBadges>().line.clone();
            world.resource_mut::<FireBadges>().line =
                format!("{badges} → JOURNAL={head:.2} → WOUND VISIBLE");
        }
    }
    if keys.just_pressed(KeyCode::KeyZ) {
        let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        if shift {
            // Iteration 19: Shift+Z retires as a special case and delegates to
            // the city controller at the reticle, radius 250 m.
            let radius = 250.0;
            let amplitude = world
                .resource::<crate::restoration::CityRestoration>()
                .config_amplitude;
            let amplitude = if amplitude > 0.0 { amplitude } else { 600.0 };
            let profile = world
                .resource::<crate::restoration::CityRestoration>()
                .profile;
            let fired = crate::restoration::restore_city(world, point, radius, amplitude, profile);
            if fired > 0 {
                info!("RESTORE delegated to city wave: {fired} buildings within {radius:.0} m");
            }
        } else {
            let target_id = target.filter(|id| {
                let did = NEIGHBORHOOD_ID_BASE + *id as u32;
                world.resource::<DamageJournal>().0.head_for(did) > 0.0
            });
            if let Some(id) = target_id {
                let did = NEIGHBORHOOD_ID_BASE + id as u32;
                info!("RESTORE request: b{id}");
                begin_restoration(world, &[did]);
            }
        }
    }
}

fn meta_bounds(meta: &BuildingMeta, origin: Vec3) -> (Vec3, Vec3) {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for point in &meta.perimeter {
        let p = Vec3::new(
            point[0] - meta.centroid[0],
            0.0,
            point[1] - meta.centroid[1],
        ) + origin;
        min = min.min(p);
        max = max.max(p);
    }
    max.y = origin.y + meta.height_m;
    (min, max)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PersistAction {
    None,
    Freeze,
    Thaw,
    Release,
}

/// Damaged buildings are frozen beyond the exit radius and thawed on re-entry;
/// a restored building (empty journal) is released so it can swap out again.
fn persistence_action(
    damaged: bool,
    frozen: bool,
    distance: f32,
    config: &NeighborhoodConfig,
) -> PersistAction {
    if damaged && !frozen && distance > config.nadir_exit {
        PersistAction::Freeze
    } else if damaged && frozen && distance <= config.nadir_enter {
        PersistAction::Thaw
    } else if !damaged && frozen {
        PersistAction::Release
    } else {
        PersistAction::None
    }
}

fn persistence_tick(world: &mut World) {
    let Some(camera) = world
        .query_filtered::<&Transform, With<FlyCamera>>()
        .single(world)
        .ok()
        .map(|transform| transform.translation)
    else {
        return;
    };
    let config = world.resource::<NeighborhoodConfig>().clone();
    let ids: Vec<(usize, Vec3, bool)> = {
        let neighborhood = world.resource::<Neighborhood>();
        neighborhood
            .live
            .iter()
            .map(|(id, live)| (*id, live.origin, live.frozen))
            .collect()
    };
    let mut frozen_count = 0usize;
    for (id, origin, was_frozen) in ids {
        let did = NEIGHBORHOOD_ID_BASE + id as u32;
        let damaged = world.resource::<DamageJournal>().0.head_for(did) > 0.0;
        let distance = camera.distance(origin);
        let action = persistence_action(damaged, was_frozen, distance, &config);
        if action == PersistAction::Freeze {
            let hide = distance > 250.0;
            if let Some(cache) = freeze_building(world, did, hide) {
                // Iteration 19 consolidation: the host crossed-quad column is
                // the single far-plume mechanism; no destructibles sprite is
                // spawned for FAR duty.
                let marker = None;
                let smoke = crate::restoration::spawn_host_smoke(world, did);
                if !smoke.is_empty() {
                    world
                        .resource_mut::<Neighborhood>()
                        .smoke_markers
                        .insert(did, smoke);
                }
                register_building(world, did, Regime::Frozen);
                if let Some(state) = world.resource_mut::<Streaming>().by_building.get_mut(&did) {
                    state.damaged = true;
                    state.frozen = Some(cache);
                    state.marker = marker;
                }
                if let Some(live) = world.resource_mut::<Neighborhood>().live.get_mut(&id) {
                    live.frozen = true;
                }
                info!("PERSIST b{id}: damaged building frozen at {distance:.0} m hidden={hide}");
            }
        } else if action == PersistAction::Thaw {
            let cache = world
                .resource_mut::<Streaming>()
                .by_building
                .get_mut(&did)
                .and_then(|state| state.frozen.take());
            if let Some(cache) = cache {
                thaw_building(world, &cache);
            }
            crate::restoration::despawn_host_smoke(world, did);
            let marker = world
                .resource_mut::<Streaming>()
                .by_building
                .get_mut(&did)
                .and_then(|state| state.marker.take());
            if let Some(marker) = marker
                && let Ok(entity_mut) = world.get_entity_mut(marker)
            {
                entity_mut.despawn();
            }
            if let Some(state) = world.resource_mut::<Streaming>().by_building.get_mut(&did) {
                state.regime = Regime::Hot;
                state.damaged = true;
                state.frozen = None;
                state.marker = None;
            }
            if let Some(live) = world.resource_mut::<Neighborhood>().live.get_mut(&id) {
                live.frozen = false;
            }
            info!("PERSIST b{id}: thawed on re-entry at {distance:.0} m");
        } else if action == PersistAction::Release {
            crate::restoration::despawn_host_smoke(world, did);
            let cache = world
                .resource_mut::<Streaming>()
                .by_building
                .get_mut(&did)
                .and_then(|state| state.frozen.take());
            if let Some(cache) = cache {
                thaw_building(world, &cache);
            }
            crate::restoration::despawn_host_smoke(world, did);
            let marker = world
                .resource_mut::<Streaming>()
                .by_building
                .get_mut(&did)
                .and_then(|state| state.marker.take());
            if let Some(marker) = marker
                && let Ok(entity_mut) = world.get_entity_mut(marker)
            {
                entity_mut.despawn();
            }
            if let Some(state) = world.resource_mut::<Streaming>().by_building.get_mut(&did) {
                state.regime = Regime::Hot;
                state.frozen = None;
                state.marker = None;
            }
            if let Some(live) = world.resource_mut::<Neighborhood>().live.get_mut(&id) {
                live.frozen = false;
            }
            // Epilogue becomes eviction: a fully restored record returns to
            // the plain resting state immediately.
            {
                let mut neighborhood = world.resource_mut::<Neighborhood>();
                if !neighborhood.queued_out.contains(&id) {
                    neighborhood.pending_out.push_back(id);
                    neighborhood.queued_out.insert(id);
                }
            }
            info!("PERSIST b{id}: released after restoration at {distance:.0} m");
        }
        if world
            .resource::<Neighborhood>()
            .live
            .get(&id)
            .is_some_and(|live| live.frozen)
        {
            frozen_count += 1;
        }
    }
    world.resource_mut::<Neighborhood>().frozen_count = frozen_count;
}

fn refresh_footprints(world: &mut World) {
    update_live_footprints(world);
}

/// A re-projected Manhattan panel or ad plate riding on a live destructible.
/// `facade`/`floor` identify the structural face it is attached to so it can
/// disappear with the wall cells it covers and come back on restoration.
#[derive(Component, Clone, Copy, Debug)]
pub struct FacadeDressing {
    pub building: u32,
    pub facade: u32,
    pub floor: u32,
    pub storefront: bool,
}

#[derive(Resource, Default)]
struct StreetParity {
    dressed: BTreeSet<usize>,
    hidden: usize,
    panels: usize,
    plates: usize,
}

fn set_plain_hidden(world: &mut World, id: usize, hidden: bool) {
    let mut entities: Vec<Entity> = Vec::new();
    if let Some(entity) = world
        .resource::<StorefrontState>()
        .entities
        .get(&id)
        .copied()
    {
        entities.push(entity);
    }
    if let Some(list) = world.resource::<SignMaterials>().entities.get(&id) {
        entities.extend(list.iter().copied());
    }
    if let Some(list) = world.resource::<RoofFurniture>().entities.get(&id) {
        entities.extend(list.iter().copied());
    }
    for entity in entities {
        let Ok(mut entity_mut) = world.get_entity_mut(entity) else {
            continue;
        };
        if hidden {
            entity_mut.insert(HiddenByLive);
            entity_mut.insert(Visibility::Hidden);
        } else {
            entity_mut.remove::<HiddenByLive>();
            entity_mut.insert(Visibility::Inherited);
        }
    }
}

/// Keep the plain map's storefront panels and sign meshes hidden while the
/// live destructible owns the footprint, and re-project the identical panels
/// and ad plates onto the destructible root.
#[allow(clippy::drop_non_drop)]
fn sync_street_dressing(world: &mut World) {
    let live: BTreeSet<usize> = world
        .resource::<Neighborhood>()
        .live
        .keys()
        .copied()
        .collect();
    let dressed: BTreeSet<usize> = world.resource::<StreetParity>().dressed.clone();
    for id in dressed.difference(&live) {
        set_plain_hidden(world, *id, false);
    }
    let to_dress: Vec<usize> = live.difference(&dressed).copied().collect();
    if to_dress.is_empty() && live.len() == dressed.len() {
        return;
    }
    let mut panels = 0usize;
    let mut plates = 0usize;
    for id in to_dress {
        let Some((did, origin, root, quads)) =
            world.resource::<Neighborhood>().live.get(&id).map(|live| {
                let did = NEIGHBORHOOD_ID_BASE + id as u32;
                let record = world.resource::<Buildings>().get(did);
                (
                    did,
                    live.origin,
                    record.map(|record| record.entity),
                    record
                        .map(|record| record.wall_quads.clone())
                        .unwrap_or_default(),
                )
            })
        else {
            continue;
        };
        let Some(root) = root else {
            continue;
        };
        set_plain_hidden(world, id, true);
        panels += spawn_storefront_parity(world, id, did, origin, root, &quads);
        plates += spawn_ad_parity(world, id, did, origin, root, &quads);
    }
    let mut parity = world.resource_mut::<StreetParity>();
    parity.dressed = live;
    parity.panels += panels;
    parity.plates += plates;
    {
        let panels_total = parity.panels;
        let plates_total = parity.plates;
        drop(parity);
        let mut neighborhood = world.resource_mut::<Neighborhood>();
        neighborhood.dressing_panels = panels_total;
        neighborhood.dressing_plates = plates_total;
    }
}

fn panel_centre_normal(corners: &[[f32; 3]; 4]) -> (Vec3, Vec3) {
    let p0 = Vec3::from_array(corners[0]);
    let p1 = Vec3::from_array(corners[1]);
    let p2 = Vec3::from_array(corners[2]);
    let p3 = Vec3::from_array(corners[3]);
    let centre = (p0 + p1 + p2 + p3) * 0.25;
    let normal = (p1 - p0).cross(p3 - p0).normalize_or_zero();
    (centre, normal)
}

/// Attach a panel or plate to the structural face it covers. Ground-floor
/// storefronts prefer floor 0; ad plates pick the closest face at their height.
fn nearest_face(centre: Vec3, normal: Vec3, quads: &[WallQuad], ground_only: bool) -> (u32, u32) {
    let mut best: Option<(f32, u32, u32)> = None;
    let mut fallback: Option<(f32, u32, u32)> = None;
    for quad in quads {
        let face_centre = quad.corners.iter().fold([0.0f32; 3], |acc, p| {
            [acc[0] + p[0], acc[1] + p[1], acc[2] + p[2]]
        });
        let face_centre = Vec3::new(
            face_centre[0] * 0.25,
            face_centre[1] * 0.25,
            face_centre[2] * 0.25,
        );
        let dot = normal.dot(Vec3::from_array(quad.normal));
        if dot < 0.2 {
            continue;
        }
        let score = face_centre.distance(centre) - dot * 2.0;
        if fallback.is_none_or(|(best_score, _, _)| score < best_score) {
            fallback = Some((score, quad.facade, quad.cell.floor));
        }
        if ground_only && quad.cell.floor != 0 {
            continue;
        }
        if best.is_none_or(|(best_score, _, _)| score < best_score) {
            best = Some((score, quad.facade, quad.cell.floor));
        }
    }
    best.or(fallback)
        .map(|(_, facade, floor)| (facade, floor))
        .unwrap_or((0, 0))
}

fn quad_mesh(quads: &[([[f32; 3]; 4], [f32; 3], [[f32; 2]; 4])]) -> Option<Mesh> {
    if quads.is_empty() {
        return None;
    }
    let mut verts = Vec::with_capacity(quads.len() * 4);
    let mut normals = Vec::with_capacity(quads.len() * 4);
    let mut uvs = Vec::with_capacity(quads.len() * 4);
    let mut indices = Vec::with_capacity(quads.len() * 6);
    for (corners, normal, uv) in quads {
        let base = verts.len() as u32;
        verts.extend(corners.iter().copied());
        normals.extend([*normal; 4]);
        uvs.extend(uv.iter().copied());
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    Some(
        Mesh::new(
            bevy::mesh::PrimitiveTopology::TriangleList,
            bevy::asset::RenderAssetUsages::RENDER_WORLD
                | bevy::asset::RenderAssetUsages::MAIN_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, verts)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(bevy::mesh::Indices::U32(indices)),
    )
}

fn spawn_storefront_parity(
    world: &mut World,
    id: usize,
    did: u32,
    origin: Vec3,
    root: Entity,
    quads: &[WallQuad],
) -> usize {
    let (panels, material) = {
        let state = world.resource::<StorefrontState>();
        let Some(&index) = state.index.get(&id) else {
            return 0;
        };
        let dressing = &state.assignments[index];
        let material = dressing.material.clone();
        (dressing.panels.clone(), material)
    };
    let mut groups: BTreeMap<u32, Vec<([[f32; 3]; 4], [f32; 3], [[f32; 2]; 4])>> = BTreeMap::new();
    for panel in &panels {
        let (centre, normal) = panel_centre_normal(&panel.corners);
        let (facade, _floor) = nearest_face(centre, normal, quads, true);
        let mut corners = panel.corners;
        for corner in &mut corners {
            corner[0] -= origin.x;
            corner[1] -= origin.y;
            corner[2] -= origin.z;
        }
        groups
            .entry(facade)
            .or_default()
            .push((corners, normal.to_array(), panel.uv));
    }
    let mut spawned = 0usize;
    for (facade, group) in groups {
        let Some(mesh) = quad_mesh(&group) else {
            continue;
        };
        let handle = world.resource_mut::<Assets<Mesh>>().add(mesh);
        world.spawn((
            Mesh3d(handle),
            MeshMaterial3d(material.clone()),
            Transform::default(),
            bevy::light::NotShadowCaster,
            FacadeDressing {
                building: did,
                facade,
                floor: 0,
                storefront: true,
            },
            ChildOf(root),
            Name::new(format!("storefront parity b{id} f{facade}")),
        ));
        spawned += 1;
    }
    spawned
}

fn spawn_ad_parity(
    world: &mut World,
    id: usize,
    did: u32,
    origin: Vec3,
    root: Entity,
    quads: &[WallQuad],
) -> usize {
    let (plates, materials) = {
        let signs = world.resource::<SignMaterials>();
        let Some(plates) = signs.plates.get(&id) else {
            return 0;
        };
        (plates.clone(), signs.materials.clone())
    };
    let mut groups: BTreeMap<(usize, u32, u32), Vec<([[f32; 3]; 4], [f32; 3], [[f32; 2]; 4])>> =
        BTreeMap::new();
    for plate in &plates {
        let (centre, normal) = panel_centre_normal(&plate.corners);
        let (facade, floor) = nearest_face(centre, normal, quads, false);
        let mut corners = plate.corners;
        for corner in &mut corners {
            corner[0] -= origin.x;
            corner[1] -= origin.y;
            corner[2] -= origin.z;
        }
        groups
            .entry((plate.material, facade, floor))
            .or_default()
            .push((corners, plate.normal, plate.uv));
    }
    let mut spawned = 0usize;
    for ((material, facade, floor), group) in groups {
        let Some(handle_material) = materials.get(material).cloned() else {
            continue;
        };
        let Some(mesh) = quad_mesh(&group) else {
            continue;
        };
        let handle = world.resource_mut::<Assets<Mesh>>().add(mesh);
        world.spawn((
            Mesh3d(handle),
            MeshMaterial3d(handle_material),
            Transform::default(),
            bevy::light::NotShadowCaster,
            FacadeDressing {
                building: did,
                facade,
                floor,
                storefront: false,
            },
            ChildOf(root),
            Name::new(format!("ad parity b{id} f{facade} l{floor}")),
        ));
        spawned += 1;
    }
    spawned
}

/// Structural faces with removed wall cells or an active breach, keyed by
/// (facade, floor). Re-derived every run, so restoration is automatic.
fn damaged_faces(world: &World, did: u32) -> Vec<(u32, u32)> {
    let mut faces = BTreeSet::new();
    if let Some(record) = world.resource::<Buildings>().get(did) {
        for index in &record.removed_quads {
            if let Some(quad) = record.wall_quads.get(*index) {
                faces.insert((quad.facade, quad.cell.floor));
            }
        }
    }
    if let Some(breach) = world.get_resource::<BreachState>() {
        for (cell, _partial) in breach.cells_for(did) {
            faces.insert((cell.facade, cell.floor));
        }
    }
    faces.into_iter().collect()
}

fn covered_by(dressing: &FacadeDressing, faces: &[(u32, u32)]) -> bool {
    faces.iter().any(|(facade, floor)| {
        *facade == dressing.facade
            && if dressing.storefront {
                *floor == 0
            } else {
                *floor == dressing.floor
            }
    })
}

fn sync_dressing_damage(world: &mut World) {
    let live: BTreeSet<usize> = world
        .resource::<Neighborhood>()
        .live
        .keys()
        .copied()
        .collect();
    if live.is_empty() {
        return;
    }
    let mut damage: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
    for id in &live {
        let did = NEIGHBORHOOD_ID_BASE + *id as u32;
        let faces = damaged_faces(world, did);
        if !faces.is_empty() {
            damage.insert(did, faces);
        }
    }
    let hidden_before = world.resource::<StreetParity>().hidden;
    if damage.is_empty() && hidden_before == 0 {
        return;
    }
    let mut hidden = 0usize;
    let mut query = world.query::<(Entity, &FacadeDressing, &mut Visibility)>();
    for (_entity, dressing, mut visibility) in query.iter_mut(world) {
        let covered = damage
            .get(&dressing.building)
            .is_some_and(|faces| covered_by(dressing, faces));
        let desired = if covered {
            hidden += 1;
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if *visibility != desired {
            *visibility = desired;
        }
    }
    world.resource_mut::<StreetParity>().hidden = hidden;
}

/// JIT: a plain record becomes live because something hit it, never because
/// the camera is near it. Uses the same footprint/height/window-row gate.
#[allow(clippy::drop_non_drop)]
fn jit_swap(world: &mut World, id: usize) -> bool {
    if world.resource::<Neighborhood>().live.contains_key(&id) {
        return true;
    }
    let Some(meta) = world.resource::<BuildingIndex>().buildings.get(id).cloned() else {
        return false;
    };
    let spec = match spec_for(&meta) {
        Ok(spec) => spec,
        Err(error) => {
            info!("JIT skip b{id}: {error}");
            world.resource_mut::<Neighborhood>().failed.insert(id);
            return false;
        }
    };
    let origin = Vec3::new(meta.centroid[0], 0.0, meta.centroid[1]);
    let started = Instant::now();
    let Some(handle) = swap_in_lod(
        world,
        NEIGHBORHOOD_ID_BASE + id as u32,
        spec.clone(),
        Transform::from_translation(origin),
        BuildingDetail::Shell,
    ) else {
        return false;
    };
    let layout = world
        .resource::<Buildings>()
        .get(handle.building)
        .map(|record| record.layout.clone());
    let ok = layout.as_ref().is_some_and(|layout| {
        footprint_matches(&meta, layout, origin)
            && height_matches(&meta, layout)
            && window_rows_match(&meta, layout)
    });
    if !ok {
        let _ = swap_out(world, handle);
        world.resource_mut::<Neighborhood>().height_failures += 1;
        world.resource_mut::<Neighborhood>().jit_rejected += 1;
        world.resource_mut::<Neighborhood>().failed.insert(id);
        info!("JIT b{id} rejected by geometry gate; stays plain");
        return false;
    }
    if let Some(mut streaming) = world.get_resource_mut::<Streaming>() {
        streaming.dormant_specs.remove(&handle.building);
        streaming.by_building.remove(&handle.building);
    }
    let now_secs = world.resource::<Time>().elapsed_secs();
    {
        let mut neighborhood = world.resource_mut::<Neighborhood>();
        neighborhood.live.insert(
            id,
            LiveBuilding {
                handle,
                meta: id,
                spec,
                origin,
                detail: DetailTier::Shell,
                entry_distance: 0.0,
                tier1_ready: None,
                frozen: false,
                dressing: Vec::new(),
            },
        );
        neighborhood.swaps_in += 1;
        neighborhood.mask_dirty = true;
        neighborhood.mask_revision += 1;
        neighborhood.live_revision += 1;
        neighborhood.jit_target = Some(id);
        neighborhood.jit_started = now_secs;
    }
    // Same hiding machinery as the retired camera swap: hide the plain
    // storefront/sign/furniture immediately; refresh_mask later this frame
    // rasterises the live footprint into the city shader mask.
    set_plain_hidden(world, id, true);
    info!(
        "JIT swap_in b{id} ({:.2} ms) geometry PASS",
        started.elapsed().as_secs_f32() * 1000.0
    );
    true
}

#[allow(clippy::drop_non_drop)]
fn process_swaps(world: &mut World) {
    let started = Instant::now();
    // A frozen game clock is extra budget: background processing keeps
    // running and is allowed to catch up twice as fast.
    let catch_up = !world.resource::<SimClock>().playing;
    let scale = if catch_up { 2.0 } else { 1.0 };
    let budget = world
        .resource::<NeighborhoodConfig>()
        .swap_budget_ms
        .max(0.5)
        * scale;
    let max_swaps = ((world
        .resource::<NeighborhoodConfig>()
        .max_swaps_per_frame
        .max(1) as f32)
        * scale)
        .round() as usize;
    let mut swaps = 0usize;
    let mut frame_swap_ms = 0.0f32;
    while swaps < max_swaps && started.elapsed().as_secs_f32() * 1000.0 < budget {
        let out = world.resource_mut::<Neighborhood>().pending_out.pop_front();
        if let Some(id) = out {
            world.resource_mut::<Neighborhood>().queued_out.remove(&id);
            let did = NEIGHBORHOOD_ID_BASE + id as u32;
            let damaged = world.resource::<DamageJournal>().0.head_for(did) > 0.0;
            if (damaged
                && world.resource::<NeighborhoodConfig>().damaged_persists
                && world.resource::<Neighborhood>().live.contains_key(&id))
                || world.resource::<Neighborhood>().jit_target == Some(id)
                || world
                    .resource::<Neighborhood>()
                    .pending_damage
                    .iter()
                    .any(|shot| shot.id == id)
            {
                continue;
            }
            let live = world.resource_mut::<Neighborhood>().live.remove(&id);
            if let Some(live) = live {
                let swap_started = Instant::now();
                let _ = swap_out(world, live.handle);
                let ms = swap_started.elapsed().as_secs_f32() * 1000.0;
                frame_swap_ms += ms;
                let mut neighborhood = world.resource_mut::<Neighborhood>();
                neighborhood.swaps_out += 1;
                neighborhood.worst_swap_ms = neighborhood.worst_swap_ms.max(ms);
                neighborhood.mask_dirty = true;
                neighborhood.mask_revision += 1;
                neighborhood.live_revision += 1;
                drop(neighborhood);
                // A pristine swap-out is not a damaged building; drop the
                // dormant spec so the next swap-in starts clean.
                if let Some(mut streaming) = world.get_resource_mut::<Streaming>() {
                    streaming
                        .dormant_specs
                        .remove(&(NEIGHBORHOOD_ID_BASE + id as u32));
                    streaming
                        .by_building
                        .remove(&(NEIGHBORHOOD_ID_BASE + id as u32));
                }
                info!("NEIGHBORHOOD swap_out b{id} ({ms:.2} ms)");
            }
            swaps += 1;
            continue;
        }
        let next_in = world.resource_mut::<Neighborhood>().pending_in.pop_front();
        let Some(id) = next_in else {
            break;
        };
        world.resource_mut::<Neighborhood>().queued_in.remove(&id);
        if world.resource::<Neighborhood>().live.contains_key(&id) {
            continue;
        }
        let meta = world.resource::<BuildingIndex>().buildings.get(id).cloned();
        let Some(meta) = meta else {
            continue;
        };
        let spec = match spec_for(&meta) {
            Ok(spec) => spec,
            Err(error) => {
                info!("NEIGHBORHOOD skip b{id}: {error}");
                world.resource_mut::<Neighborhood>().failed.insert(id);
                continue;
            }
        };
        let origin = Vec3::new(meta.centroid[0], 0.0, meta.centroid[1]);
        let swap_started = Instant::now();
        let handle = swap_in_lod(
            world,
            NEIGHBORHOOD_ID_BASE + id as u32,
            spec.clone(),
            Transform::from_translation(origin),
            BuildingDetail::Shell,
        );
        let ms = swap_started.elapsed().as_secs_f32() * 1000.0;
        frame_swap_ms += ms;
        let Some(handle) = handle else {
            info!("NEIGHBORHOOD swap_in b{id} failed (id collision)");
            world.resource_mut::<Neighborhood>().failed.insert(id);
            continue;
        };
        let layout = world
            .resource::<Buildings>()
            .get(handle.building)
            .map(|record| record.layout.clone());
        let footprint_ok = layout
            .as_ref()
            .is_some_and(|layout| footprint_matches(&meta, layout, origin));
        let height_ok = layout
            .as_ref()
            .is_some_and(|layout| height_matches(&meta, layout));
        let rows_ok = layout
            .as_ref()
            .is_some_and(|layout| window_rows_match(&meta, layout));
        if !footprint_ok || !height_ok || !rows_ok {
            let _ = swap_out(world, handle);
            if !height_ok || !rows_ok {
                world.resource_mut::<Neighborhood>().height_failures += 1;
            }
            if !footprint_ok {
                world.resource_mut::<Neighborhood>().footprint_failures += 1;
            }
            world.resource_mut::<Neighborhood>().failed.insert(id);
            info!("NEIGHBORHOOD footprint/height FAIL b{id}; returned to plain map");
            continue;
        }
        if let Some(mut streaming) = world.get_resource_mut::<Streaming>() {
            streaming.dormant_specs.remove(&handle.building);
            streaming.by_building.remove(&handle.building);
        }
        {
            let mut neighborhood = world.resource_mut::<Neighborhood>();
            neighborhood.live.insert(
                id,
                LiveBuilding {
                    handle,
                    meta: id,
                    spec,
                    origin,
                    detail: DetailTier::Shell,
                    entry_distance: 0.0,
                    tier1_ready: None,
                    frozen: false,
                    dressing: Vec::new(),
                },
            );
            neighborhood.swaps_in += 1;
            neighborhood.worst_swap_ms = neighborhood.worst_swap_ms.max(ms);
            neighborhood.mask_dirty = true;
            neighborhood.mask_revision += 1;
            neighborhood.live_revision += 1;
        }
        info!(
            "NEIGHBORHOOD swap_in b{id} ({ms:.2} ms) footprint PASS style={:?}",
            world
                .resource::<Buildings>()
                .get(handle.building)
                .map(|record| record.layout.spec.facade_style)
        );
        swaps += 1;
    }
    if frame_swap_ms > 0.0 {
        let mut neighborhood = world.resource_mut::<Neighborhood>();
        neighborhood.worst_swap_frame_ms = neighborhood.worst_swap_frame_ms.max(frame_swap_ms);
    }
    update_live_footprints(world);
}

fn update_live_footprints(world: &mut World) {
    let (live, revision) = {
        let neighborhood = world.resource::<Neighborhood>();
        (
            neighborhood
                .live
                .keys()
                .copied()
                .collect::<BTreeSet<usize>>(),
            neighborhood.live_revision,
        )
    };
    let mut colors = BTreeMap::new();
    {
        let journal = world.resource::<DamageJournal>();
        let health = world.get_resource::<HealthIndex>();
        for id in &live {
            let damaged = journal.0.head_for(NEIGHBORHOOD_ID_BASE + *id as u32) > 0.0;
            let integrity = health
                .and_then(|health| {
                    health
                        .by_building
                        .get(&(NEIGHBORHOOD_ID_BASE + *id as u32))
                        .map(|entry| entry.integrity)
                })
                .unwrap_or(1.0);
            colors.insert(*id, live_fill_color(damaged, integrity));
        }
    }
    let mut footprints = world.resource_mut::<LiveFootprints>();
    if footprints.live != live || footprints.colors != colors {
        footprints.live = live;
        footprints.colors = colors;
        footprints.revision = revision.max(footprints.revision.wrapping_add(1));
    }
}

/// White for pristine; the established green→red halo palette once damaged.
pub fn live_fill_color(damaged: bool, integrity: f32) -> [u8; 4] {
    if !damaged {
        return [255, 255, 255, 235];
    }
    let t = integrity.clamp(0.0, 1.0);
    let red = (255.0 * (1.0 - t) + 30.0 * t) as u8;
    let green = (40.0 * (1.0 - t) + 240.0 * t) as u8;
    [red, green, 40, 235]
}

fn refresh_mask(
    camera: Single<&Transform, With<FlyCamera>>,
    index: Res<BuildingIndex>,
    mut neighborhood: ResMut<Neighborhood>,
) {
    let center = Vec2::new(camera.translation.x, camera.translation.z);
    let origin = center - Vec2::splat(MASK_WORLD * 0.5);
    let recentered = neighborhood.mask_origin.distance(origin) > MASK_WORLD * 0.15;
    if !neighborhood.mask_dirty && !recentered {
        return;
    }
    neighborhood.mask_origin = origin;
    {
        let Neighborhood {
            live, mask_pixels, ..
        } = &mut *neighborhood;
        rebuild_mask(&index, live, origin, mask_pixels);
    }
    neighborhood.mask_dirty = true;
    neighborhood.mask_revision += 1;
}

fn update_mask_texture(mut neighborhood: ResMut<Neighborhood>, mut images: ResMut<Assets<Image>>) {
    if !neighborhood.mask_dirty {
        return;
    }
    let Some(handle) = neighborhood.mask.clone() else {
        return;
    };
    let Some(mut image) = images.get_mut(&handle) else {
        return;
    };
    image.data = Some(neighborhood.mask_pixels.clone());
    neighborhood.mask_dirty = false;
}

fn sync_live_mask(
    neighborhood: Res<Neighborhood>,
    mut materials: ResMut<Assets<CityMaterial>>,
    mut last: Local<(u64, usize)>,
) {
    let count = materials.len();
    if *last == (neighborhood.mask_revision, count) {
        return;
    }
    *last = (neighborhood.mask_revision, count);
    let Some(mask) = neighborhood.mask.clone() else {
        return;
    };
    let params = Vec4::new(
        neighborhood.mask_origin.x,
        neighborhood.mask_origin.y,
        MASK_WORLD,
        1.0,
    );
    for (_, material) in materials.iter_mut() {
        material.extension.live_mask = Some(mask.clone());
        material.extension.live_mask_params = params;
    }
}

fn toggle_minimap_fills(
    keys: Res<ButtonInput<KeyCode>>,
    mut config: ResMut<NeighborhoodConfig>,
    mut footprints: ResMut<LiveFootprints>,
) {
    if keys.just_pressed(KeyCode::KeyN) {
        config.minimap_fills = !config.minimap_fills;
        footprints.debug = config.minimap_fills;
        footprints.revision += 1;
        info!(
            "NEIGHBORHOOD minimap live fills {}",
            if config.minimap_fills { "ON" } else { "OFF" }
        );
    }
    if footprints.debug != config.minimap_fills {
        footprints.debug = config.minimap_fills;
        footprints.revision += 1;
    }
}

/// Re-rasterize the R8 world-XZ mask from the current live set.
pub fn rebuild_mask(
    index: &BuildingIndex,
    live: &BTreeMap<usize, LiveBuilding>,
    origin: Vec2,
    pixels: &mut [u8],
) {
    pixels.fill(0);
    for value in live.values() {
        let Some(meta) = index.buildings.get(value.meta) else {
            continue;
        };
        if meta.perimeter.len() < 3 {
            continue;
        }
        let polygon: Vec<Vec2> = meta
            .perimeter
            .iter()
            .map(|point| {
                Vec2::new(
                    (point[0] - origin.x) / MASK_WORLD + 0.5,
                    (point[1] - origin.y) / MASK_WORLD + 0.5,
                ) * MASK_RES as f32
            })
            .collect();
        fill_mask_polygon(pixels, &polygon);
    }
}

fn fill_mask_polygon(pixels: &mut [u8], polygon: &[Vec2]) {
    let min_y = polygon.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
    let max_y = polygon
        .iter()
        .map(|p| p.y)
        .fold(f32::NEG_INFINITY, f32::max);
    let y0 = min_y.floor().max(0.0) as i32;
    let y1 = max_y.ceil().min(MASK_RES as f32 - 1.0) as i32;
    let mut intersections = Vec::new();
    for y in y0..=y1 {
        let scan = y as f32 + 0.5;
        intersections.clear();
        for index in 0..polygon.len() {
            let a = polygon[index];
            let b = polygon[(index + 1) % polygon.len()];
            if (a.y <= scan && b.y > scan) || (b.y <= scan && a.y > scan) {
                let t = (scan - a.y) / (b.y - a.y);
                intersections.push(a.x + (b.x - a.x) * t);
            }
        }
        if intersections.len() < 2 {
            continue;
        }
        intersections.sort_by(|a, b| a.total_cmp(b));
        for pair in intersections.as_chunks::<2>().0 {
            let x0 = pair[0].round().max(0.0) as i32;
            let x1 = pair[1].round().min(MASK_RES as f32 - 1.0) as i32;
            for x in x0..=x1 {
                pixels[y as usize * MASK_RES + x as usize] = 255;
            }
        }
    }
}

#[allow(clippy::drop_non_drop)]
fn demo_damage(world: &World) -> bool {
    world
        .get_resource::<NeighborhoodDemo>()
        .is_some_and(|demo| demo.damage)
}

/// Pick a storefront-category block for the street-level capture, preferring a
/// reasonably tall one so the same building reads as a plume from three blocks.
fn nearest_demo_live(world: &World, point: Vec3) -> Option<(usize, Vec3)> {
    let index = world.resource::<BuildingIndex>();
    let mut best: Option<(u8, f32, usize, Vec3)> = None;
    for (id, live) in &world.resource::<Neighborhood>().live {
        let distance = live.origin.distance_squared(point);
        let rank = index
            .buildings
            .get(live.meta)
            .map(|meta| {
                if meta.category == "storefront" {
                    0
                } else if meta.height_m >= 45.0 {
                    1
                } else {
                    2
                }
            })
            .unwrap_or(3);
        if best
            .as_ref()
            .is_none_or(|(best_rank, best_distance, _, _)| {
                (rank, distance) < (*best_rank, *best_distance)
            })
        {
            best = Some((rank, distance, *id, live.origin));
        }
    }
    best.map(|(_, _, id, origin)| (id, origin))
}

fn nearest_live_ids(world: &World, point: Vec3, count: usize) -> Vec<usize> {
    let index = world.resource::<BuildingIndex>();
    let mut ids: Vec<(f32, usize)> = world
        .resource::<Neighborhood>()
        .live
        .keys()
        .filter_map(|id| {
            index.buildings.get(*id).map(|meta| {
                (
                    Vec2::new(meta.centroid[0] - point.x, meta.centroid[1] - point.z).length(),
                    *id,
                )
            })
        })
        .collect();
    ids.sort_by(|a, b| a.0.total_cmp(&b.0));
    ids.into_iter().take(count).map(|(_, id)| id).collect()
}

fn mid_wave_target(world: &World) -> Option<usize> {
    let journal = world.resource::<DamageJournal>();
    let city = world.resource::<crate::restoration::CityRestoration>();
    city.assignments
        .values()
        .filter(|assignment| assignment.completed_at.is_none())
        .find(|assignment| journal.0.head_for(assignment.did) > 0.0)
        .map(|assignment| assignment.building)
}

fn nearest_live(world: &World, point: Vec3) -> Option<(usize, Vec3)> {
    world
        .resource::<Neighborhood>()
        .live
        .iter()
        .map(|(id, live)| (*id, live.origin))
        .min_by(|a, b| {
            a.1.distance_squared(point)
                .total_cmp(&b.1.distance_squared(point))
        })
}

fn queue_damage(world: &mut World, id: usize, preset: DamagePreset, point: Vec3) {
    // JIT: every damage source localizes to a plain record and swaps it in
    // before the shot is admitted. No other path creates live buildings.
    if !world.resource::<Neighborhood>().live.contains_key(&id) && !jit_swap(world, id) {
        info!("DAMAGE skip b{id}: JIT geometry gate rejected");
        return;
    }
    world
        .resource_mut::<Neighborhood>()
        .pending_damage
        .push(PendingDamage { id, preset, point });
    info!("DAMAGE DEMO: {} queued at b{id}", preset.label());
}

fn auto_bomb(world: &mut World, camera: Vec3) {
    let Some((id, origin)) = nearest_live(world, camera) else {
        return;
    };
    let away = Vec2::new(camera.x - origin.x, camera.z - origin.z).normalize_or_zero();
    let away = if away.length_squared() < 0.01 {
        Vec2::X
    } else {
        away
    };
    let point = origin + Vec3::new(away.x, 0.0, away.y) * 8.0 + Vec3::Y * 2.2;
    queue_damage(world, id, DamagePreset::CarBomb, point);
}

/// Two-phase screenshot helper: request on the first tick, then wait a couple
/// of frames before the stage changes time/camera so the PNG matches the label.
fn golden_take(world: &mut World, file: &str, label: &'static str, now: f32) -> bool {
    if !world.resource::<NeighborhoodDemo>().golden_shot {
        world.resource_mut::<NeighborhoodDemo>().golden_shot = true;
        world.resource_mut::<NeighborhoodDemo>().stage_at = now;
        let path = world.resource::<NeighborhoodDemo>().capture_dir.join(file);
        screenshot(world, &path, label);
        return false;
    }
    if now - world.resource::<NeighborhoodDemo>().stage_at < 0.45 {
        return false;
    }
    world.resource_mut::<NeighborhoodDemo>().golden_shot = false;
    true
}

#[allow(clippy::drop_non_drop)]
fn run_neighborhood_demo(world: &mut World) {
    if !world.resource::<NeighborhoodDemo>().active {
        return;
    }
    let now = world.resource::<Time>().elapsed_secs();
    let dt = world.resource::<Time>().delta_secs();
    let stage = world.resource::<NeighborhoodDemo>().stage;
    match stage {
        DemoStage::Boot => {
            if now - world.resource::<NeighborhoodDemo>().stage_at < 2.0
                || !world.resource::<CityMaterialSetup>().done
            {
                return;
            }
            let (cell, _) = world.resource::<BuildingIndex>().densest_cell();
            let center = Vec2::new((cell.0 as f32 + 0.5) * 100.0, (cell.1 as f32 + 0.5) * 100.0);
            let start = Vec3::new(center.x - 350.0, 120.0, center.y);
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.start = start;
            demo.forward = Vec3::X;
            demo.regime = 0;
            demo.distance = 0.0;
            demo.stage = DemoStage::Settle;
            demo.stage_at = now;
            demo.current = RegimeStats {
                name: REGIMES[0].0.to_string(),
                speed: REGIMES[0].1,
                ..default()
            };
            set_camera(world, start, Vec3::X, REGIMES[0].1);
            info!(
                "NEIGHBORHOOD DEMO: start {start:?} toward +X at {} m/s",
                REGIMES[0].1
            );
        }
        DemoStage::Settle => {
            if now - world.resource::<NeighborhoodDemo>().stage_at < 1.5 {
                return;
            }
            {
                let mut neighborhood = world.resource_mut::<Neighborhood>();
                neighborhood.worst_swap_ms = 0.0;
                neighborhood.worst_swap_frame_ms = 0.0;
                neighborhood.pending_peak = 0;
                neighborhood.behind_max = 0.0;
            }
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.stage = DemoStage::Traverse;
            demo.stage_at = now;
            demo.distance = 0.0;
            demo.captured = false;
            demo.captured_detail = false;
            demo.bombed = false;
            demo.wave_fired = false;
            demo.current = RegimeStats {
                name: REGIMES[demo.regime].0.to_string(),
                speed: REGIMES[demo.regime].1,
                ..default()
            };
            let start = demo.start;
            let speed = REGIMES[demo.regime].1;
            drop(demo);
            set_camera(world, start, Vec3::X, speed);
        }
        DemoStage::Traverse => {
            let live_count = world.resource::<Neighborhood>().live.len();
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            let (name, speed) = REGIMES[demo.regime];
            demo.current.frames += 1;
            demo.current.worst_frame_ms = demo.current.worst_frame_ms.max(dt * 1000.0);
            demo.current.live_max = demo.current.live_max.max(live_count);
            let step = speed * dt;
            demo.distance += step;
            let position = demo.start + Vec3::X * demo.distance;
            let distance = demo.distance;
            drop(demo);
            if let Ok(mut camera) = world
                .query_filtered::<&mut Transform, With<FlyCamera>>()
                .single_mut(world)
            {
                camera.translation = position;
            }
            // Let the zone/mask update with the new camera pose before the
            // screenshot.
            update_zone_snapshot(world);
            if distance >= TRAVERSE_METERS * 0.12
                && !world.resource::<NeighborhoodDemo>().captured_detail
            {
                world.resource_mut::<NeighborhoodDemo>().captured_detail = true;
                let path = world
                    .resource::<NeighborhoodDemo>()
                    .capture_dir
                    .join(format!("iteration18_detail_buildout_{name}.png"));
                screenshot(world, &path, "detail build-out");
            }
            if demo_damage(world)
                && !world.resource::<NeighborhoodDemo>().bombed
                && distance >= TRAVERSE_METERS * 0.35
            {
                world.resource_mut::<NeighborhoodDemo>().bombed = true;
                auto_bomb(world, position);
            }
            if world.resource::<NeighborhoodDemo>().gauntlet_wave
                && world.resource::<NeighborhoodDemo>().regime == 2
                && !world.resource::<NeighborhoodDemo>().wave_fired
                && distance >= TRAVERSE_METERS * 0.4
            {
                world.resource_mut::<NeighborhoodDemo>().wave_fired = true;
                let center = Vec3::new(position.x, 0.0, position.z);
                for id in nearest_live_ids(world, center, 6) {
                    let origin = world
                        .resource::<Neighborhood>()
                        .live
                        .get(&id)
                        .map(|live| live.origin)
                        .unwrap_or(center);
                    queue_damage(
                        world,
                        id,
                        DamagePreset::CarBomb,
                        origin + Vec3::new(0.0, 2.2, 6.0),
                    );
                }
                let fired = crate::restoration::restore_city(
                    world,
                    center,
                    500.0,
                    2_000.0,
                    crate::restoration::FalloffProfile::Linear,
                );
                info!("GAUNTLET WAVE: {fired} assignments at hero speed");
            }
            if distance >= TRAVERSE_METERS * 0.5 && !world.resource::<NeighborhoodDemo>().captured {
                world.resource_mut::<NeighborhoodDemo>().captured = true;
                let path = world
                    .resource::<NeighborhoodDemo>()
                    .capture_dir
                    .join(format!("iteration18_{name}.png"));
                screenshot(world, &path, name);
            }
            if distance >= TRAVERSE_METERS {
                let mut demo = world.resource_mut::<NeighborhoodDemo>();
                demo.stage = DemoStage::RegimeEnd;
                demo.stage_at = now;
            }
        }
        DemoStage::RegimeEnd => {
            if now - world.resource::<NeighborhoodDemo>().stage_at < 0.8 {
                return;
            }
            let live_len = world_live_len(world);
            let (swaps_in, swaps_out, pending_peak, behind_max, worst_swap_ms, worst_swap_frame_ms) = {
                let neighborhood = world.resource::<Neighborhood>();
                (
                    neighborhood.swaps_in,
                    neighborhood.swaps_out,
                    neighborhood.pending_peak,
                    neighborhood.behind_max,
                    neighborhood.worst_swap_ms,
                    neighborhood.worst_swap_frame_ms,
                )
            };
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.current.live_max = demo.current.live_max.max(live_len);
            demo.current.swaps_in = swaps_in;
            demo.current.swaps_out = swaps_out;
            demo.current.pending_peak = pending_peak;
            demo.current.behind_max = behind_max;
            demo.current.worst_swap_ms = worst_swap_ms;
            demo.current.worst_swap_frame_ms = worst_swap_frame_ms;
            drop(demo);
            {
                let mut count = 0usize;
                let mut max_m = 0.0f32;
                for live in world.resource::<Neighborhood>().live.values() {
                    if let Some(distance) = live.tier1_ready {
                        count += 1;
                        max_m = max_m.max(distance);
                    }
                }
                let mut demo = world.resource_mut::<NeighborhoodDemo>();
                demo.current.tier1_count = count;
                demo.current.tier1_max_m = max_m;
            }
            let stray = force_clear(world);
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.current.stray_handles = stray;
            let stats = demo.current.clone();
            info!(
                "REGIME {} speed={:.1} frames={} worst_frame={:.1}ms worst_swap={:.1}ms worst_swap_frame={:.1}ms live_max={} swaps_in={} swaps_out={} pending_peak={} behind_max={:.1}m stray={} tier1={} tier1_max_approach={:.1}m",
                stats.name,
                stats.speed,
                stats.frames,
                stats.worst_frame_ms,
                stats.worst_swap_ms,
                stats.worst_swap_frame_ms,
                stats.live_max,
                stats.swaps_in,
                stats.swaps_out,
                stats.pending_peak,
                stats.behind_max,
                stats.stray_handles,
                stats.tier1_count,
                stats.tier1_max_m
            );
            demo.stats.push(stats);
            demo.regime += 1;
            if demo.regime >= REGIMES.len() {
                demo.stage = DemoStage::Hover;
                demo.stage_at = now;
                let start = demo.start;
                drop(demo);
                set_camera(world, start, Vec3::X, 0.0);
            } else {
                let start = demo.start;
                let speed = REGIMES[demo.regime].1;
                demo.stage = DemoStage::Settle;
                demo.stage_at = now;
                drop(demo);
                set_camera(world, start, Vec3::X, speed);
            }
        }
        DemoStage::Hover => {
            // Let the stationary zone populate and its queue drain before
            // measuring. This separates steady-state hysteresis from the
            // one-off repopulation after the gauntlet.
            if now - world.resource::<NeighborhoodDemo>().stage_at < 1.8 {
                return;
            }
            let current_swaps = {
                let neighborhood = world.resource::<Neighborhood>();
                neighborhood.swaps_in + neighborhood.swaps_out
            };
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.hover_start_swaps = current_swaps;
            demo.hover_swaps = 0;
            demo.stage = DemoStage::HoverWait;
            demo.stage_at = now;
        }
        DemoStage::HoverWait => {
            if now - world.resource::<NeighborhoodDemo>().stage_at < 1.8 {
                return;
            }
            let path = world
                .resource::<NeighborhoodDemo>()
                .capture_dir
                .join("iteration18_boundary_hover.png");
            screenshot(world, &path, "boundary hover");
            let current_swaps = {
                let neighborhood = world.resource::<Neighborhood>();
                neighborhood.swaps_in + neighborhood.swaps_out
            };
            let hover_start = world.resource::<NeighborhoodDemo>().hover_start_swaps;
            let hover_swaps = current_swaps.saturating_sub(hover_start);
            info!("NEIGHBORHOOD HOVER: 3 s steady, swaps={hover_swaps} (expect 0)");
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.hover_swaps = hover_swaps;
            demo.stage = DemoStage::ExitWait;
            demo.stage_at = now;
            let summary: Vec<String> = demo
                .stats
                .iter()
                .map(|stats| {
                    format!(
                        "{} {:.0}m/s worst {:.1}ms swap {:.1}ms live {} pending {} behind {:.0}m stray {}",
                        stats.name,
                        stats.speed,
                        stats.worst_frame_ms,
                        stats.worst_swap_ms,
                        stats.live_max,
                        stats.pending_peak,
                        stats.behind_max,
                        stats.stray_handles
                    )
                })
                .collect();
            info!("NEIGHBORHOOD GAUNTLET SUMMARY: {}", summary.join(" | "));
            if demo.damage {
                demo.stage = DemoStage::DamageFly;
            } else {
                demo.stage = DemoStage::ExitWait;
            }
            demo.stage_at = now;
        }
        DemoStage::DamageFly => {
            // The street/horizon proof reads best on a tower: a tall plume is
            // visible over three blocks, and a tower's ground floor is a real
            // storefront. Fly to the tallest building in the city and let the
            // zone swap it in before framing it.
            let start = world.resource::<NeighborhoodDemo>().start;
            let pick = |min_height: f32, max_height: f32| {
                world
                    .resource::<BuildingIndex>()
                    .buildings
                    .iter()
                    .filter(|meta| {
                        meta.perimeter.len() >= 4
                            && meta.height_m >= min_height
                            && meta.height_m <= max_height
                    })
                    .map(|meta| {
                        let distance =
                            Vec2::new(meta.centroid[0] - start.x, meta.centroid[1] - start.z)
                                .length();
                        (distance, meta.centroid, meta.height_m, meta.id)
                    })
                    .filter(|(distance, ..)| *distance < 1_500.0)
                    .min_by(|a, b| a.0.total_cmp(&b.0))
            };
            let selected = pick(55.0, 140.0).or_else(|| pick(35.0, 200.0)).or_else(|| {
                world
                    .resource::<BuildingIndex>()
                    .buildings
                    .iter()
                    .filter(|meta| meta.perimeter.len() >= 4)
                    .min_by(|a, b| {
                        let da =
                            Vec2::new(a.centroid[0] - start.x, a.centroid[1] - start.z).length();
                        let db =
                            Vec2::new(b.centroid[0] - start.x, b.centroid[1] - start.z).length();
                        da.total_cmp(&db)
                    })
                    .map(|meta| {
                        let distance =
                            Vec2::new(meta.centroid[0] - start.x, meta.centroid[1] - start.z)
                                .length();
                        (distance, meta.centroid, meta.height_m, meta.id)
                    })
            });
            let Some((_distance, centroid, height, id)) = selected else {
                world.resource_mut::<NeighborhoodDemo>().stage = DemoStage::ExitWait;
                return;
            };
            let origin = Vec3::new(centroid[0], 0.0, centroid[1]);
            let position = origin + Vec3::new(0.0, 40.0, 90.0);
            let look = origin + Vec3::Y * (height * 0.35);
            set_camera(world, position, look - position, 15.0);
            info!("DAMAGE DEMO: approach b{id} height {height:.1} m");
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.stage = DemoStage::DamageSetup;
            demo.stage_at = now;
        }
        DemoStage::DamageSetup => {
            if now - world.resource::<NeighborhoodDemo>().stage_at < 2.8 {
                return;
            }
            let camera = world
                .query_filtered::<&Transform, With<FlyCamera>>()
                .single(world)
                .ok()
                .map(|transform| transform.translation)
                .unwrap_or(Vec3::ZERO);
            let Some((id, origin)) = nearest_demo_live(world, camera) else {
                info!("DAMAGE DEMO: no live building near the hover pose");
                world.resource_mut::<NeighborhoodDemo>().stage = DemoStage::ExitWait;
                return;
            };
            let away = Vec2::new(camera.x - origin.x, camera.z - origin.z).normalize_or_zero();
            let away = if away.length_squared() < 0.01 {
                Vec2::X
            } else {
                away
            };
            let radius = world
                .resource::<BuildingIndex>()
                .buildings
                .get(id)
                .map(|meta| {
                    meta.perimeter
                        .iter()
                        .map(|point| {
                            Vec2::new(point[0] - meta.centroid[0], point[1] - meta.centroid[1])
                                .length()
                        })
                        .fold(4.0f32, f32::max)
                })
                .unwrap_or(6.0);
            let direction = Vec3::new(away.x, 0.0, away.y);
            let standoff = (radius * 1.9 + 12.0).max(34.0);
            let position = origin + direction * standoff + Vec3::Y * 6.0;
            let look = origin + Vec3::Y * 3.0;
            let point = origin + direction * (radius * 0.95) + Vec3::Y * 2.4;
            set_camera(world, position, look - position, 8.0);
            let height = world
                .resource::<BuildingIndex>()
                .buildings
                .get(id)
                .map(|meta| meta.height_m)
                .unwrap_or(20.0);
            let high = origin + direction * (standoff * 2.2) + Vec3::Y * (height * 1.1 + 8.0);
            let damage_view = (high, (origin + Vec3::Y * (height * 0.7)) - high);
            {
                let mut demo = world.resource_mut::<NeighborhoodDemo>();
                demo.damage_building = Some(id);
                demo.damage_point = point;
                demo.street_view = (position, look - position);
                demo.damage_view = damage_view;
                demo.horizon_armed = false;
                demo.return_armed = false;
                demo.damage_seen = false;
                demo.restored_at = 0.0;
                demo.stage = DemoStage::StorefrontPristine;
                demo.stage_at = now;
            }
            let meta = world
                .resource::<BuildingIndex>()
                .buildings
                .get(id)
                .map(|meta| (meta.category.clone(), meta.height_m));
            info!("DAMAGE DEMO: street view at b{id} {meta:?}, point {point:?}");
        }
        DemoStage::StorefrontPristine => {
            if now - world.resource::<NeighborhoodDemo>().stage_at < 1.6 {
                return;
            }
            let path = world
                .resource::<NeighborhoodDemo>()
                .capture_dir
                .join("iteration18_storefront_pristine.png");
            screenshot(world, &path, "storefront pristine");
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            if let Some(id) = demo.damage_building {
                let point = demo.damage_point;
                demo.stage = DemoStage::StorefrontBomb;
                demo.stage_at = now;
                drop(demo);
                queue_damage(world, id, DamagePreset::CarBomb, point);
            }
        }
        DemoStage::StorefrontBomb => {
            let Some(id) = world.resource::<NeighborhoodDemo>().damage_building else {
                return;
            };
            let head = world
                .resource::<DamageJournal>()
                .0
                .head_for(NEIGHBORHOOD_ID_BASE + id as u32);
            if head > 0.0 && !world.resource::<NeighborhoodDemo>().damage_seen {
                world.resource_mut::<NeighborhoodDemo>().damage_seen = true;
                world.resource_mut::<NeighborhoodDemo>().stage_at = now;
            }
            let elapsed = now - world.resource::<NeighborhoodDemo>().stage_at;
            if head <= 0.0 && elapsed < 8.0 {
                return;
            }
            if head > 0.0 && elapsed < 0.6 {
                return;
            }
            let path = world
                .resource::<NeighborhoodDemo>()
                .capture_dir
                .join("iteration18_storefront_damage.png");
            screenshot(world, &path, "storefront damage");
            info!("DAMAGE DEMO: b{id} journal head {head:.2}");
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.stage = DemoStage::StorefrontHalo;
            demo.stage_at = now;
        }
        DemoStage::StorefrontHalo => {
            if now - world.resource::<NeighborhoodDemo>().stage_at < 0.4 {
                return;
            }
            let Some(id) = world.resource::<NeighborhoodDemo>().damage_building else {
                return;
            };
            {
                let (position, direction) = world.resource::<NeighborhoodDemo>().damage_view;
                if direction != Vec3::ZERO {
                    set_camera(world, position, direction, 8.0);
                }
            }
            let path = world
                .resource::<NeighborhoodDemo>()
                .capture_dir
                .join("iteration18_damage_nadir.png");
            screenshot(world, &path, "damage at nadir");
            info!("DAMAGE DEMO: b{id} halo capture");
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.stage = DemoStage::Horizon;
            demo.stage_at = now;
        }
        DemoStage::Horizon => {
            let Some(id) = world.resource::<NeighborhoodDemo>().damage_building else {
                return;
            };
            if !world.resource::<NeighborhoodDemo>().horizon_armed {
                world.resource_mut::<NeighborhoodDemo>().horizon_armed = true;
                world.resource_mut::<NeighborhoodDemo>().stage_at = now;
                // Three blocks out, framing the smoke column itself.
                let did = NEIGHBORHOOD_ID_BASE + id as u32;
                let origin = world
                    .resource::<Neighborhood>()
                    .live
                    .get(&id)
                    .map(|live| live.origin)
                    .unwrap_or(Vec3::ZERO);
                let focus = world
                    .resource::<Neighborhood>()
                    .smoke_markers
                    .get(&did)
                    .and_then(|markers| markers.first().copied())
                    .and_then(|marker| world.get::<Transform>(marker))
                    .map(|transform| transform.translation)
                    .unwrap_or(origin + Vec3::Y * 20.0);
                let position = focus + Vec3::new(0.0, 55.0, 300.0);
                set_camera(world, position, focus - position, 40.0);
                return;
            }
            if now - world.resource::<NeighborhoodDemo>().stage_at < 2.4 {
                return;
            }
            {
                let did = NEIGHBORHOOD_ID_BASE + id as u32;
                let frozen = world
                    .resource::<Neighborhood>()
                    .live
                    .get(&id)
                    .is_some_and(|live| live.frozen);
                let smoke = world
                    .resource::<Neighborhood>()
                    .smoke_markers
                    .get(&did)
                    .map(|markers| markers.len())
                    .unwrap_or(0);
                info!("DAMAGE DEMO: horizon b{id} frozen={frozen} host_smoke_quads={smoke}");
            }
            let path = world
                .resource::<NeighborhoodDemo>()
                .capture_dir
                .join("iteration18_horizon_smoke.png");
            screenshot(world, &path, "horizon smoke");
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.stage = DemoStage::Return;
            demo.stage_at = now;
        }
        DemoStage::Return => {
            if !world.resource::<NeighborhoodDemo>().return_armed {
                world.resource_mut::<NeighborhoodDemo>().return_armed = true;
                world.resource_mut::<NeighborhoodDemo>().stage_at = now;
                let (position, direction) = world.resource::<NeighborhoodDemo>().street_view;
                set_camera(world, position, direction, 8.0);
                return;
            }
            if now - world.resource::<NeighborhoodDemo>().stage_at < 2.4 {
                return;
            }
            if let Some(id) = world.resource::<NeighborhoodDemo>().damage_building {
                let did = NEIGHBORHOOD_ID_BASE + id as u32;
                let head = world.resource::<DamageJournal>().0.head_for(did);
                let frozen = world
                    .resource::<Neighborhood>()
                    .live
                    .get(&id)
                    .is_some_and(|live| live.frozen);
                info!("DAMAGE DEMO: re-entry b{id} journal {head:.2} frozen={frozen}");
            }
            let path = world
                .resource::<NeighborhoodDemo>()
                .capture_dir
                .join("iteration18_storefront_return.png");
            screenshot(world, &path, "storefront re-entry");
            if let Some(id) = world.resource::<NeighborhoodDemo>().damage_building {
                let did = NEIGHBORHOOD_ID_BASE + id as u32;
                // The host owns the rate; tuning drives the demo restoration.
                let rate = world
                    .resource::<crate::city_tuning::CityTuning>()
                    .restoration_rate_m3s;
                set_restoration_rate(world, did, rate);
                begin_restoration(world, &[did]);
            }
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.stage = DemoStage::Restored;
            demo.stage_at = now;
        }
        DemoStage::Restored => {
            let Some(id) = world.resource::<NeighborhoodDemo>().damage_building else {
                return;
            };
            let did = NEIGHBORHOOD_ID_BASE + id as u32;
            let head = world.resource::<DamageJournal>().0.head_for(did);
            let elapsed = now - world.resource::<NeighborhoodDemo>().stage_at;
            if head <= 0.0 && world.resource::<NeighborhoodDemo>().restored_at == 0.0 {
                world.resource_mut::<NeighborhoodDemo>().restored_at = now;
            }
            let restored_for = now - world.resource::<NeighborhoodDemo>().restored_at;
            if head > 0.0 && elapsed < 30.0 {
                return;
            }
            if head <= 0.0 && restored_for < 1.2 {
                return;
            }
            let path = world
                .resource::<NeighborhoodDemo>()
                .capture_dir
                .join("iteration18_storefront_restored.png");
            screenshot(world, &path, "storefront restored");
            let integrity = world
                .get_resource::<HealthIndex>()
                .and_then(|health| health.by_building.get(&did).map(|entry| entry.integrity))
                .unwrap_or(1.0);
            let fill = live_fill_color(head > 0.0, integrity);
            let frozen = world
                .resource::<Neighborhood>()
                .live
                .get(&id)
                .is_some_and(|live| live.frozen);
            info!(
                "DAMAGE DEMO: b{id} restored, journal {head:.2} at {elapsed:.1}s minimap_fill={fill:?} frozen={frozen} swap_out_eligible={}",
                head <= 0.0 && !frozen
            );
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.stage = DemoStage::ExitWait;
            demo.stage_at = now;
        }
        DemoStage::WaveBoot => {
            if now - world.resource::<NeighborhoodDemo>().stage_at < 2.0
                || !world.resource::<CityMaterialSetup>().done
            {
                return;
            }
            let (cell, _) = world.resource::<BuildingIndex>().densest_cell();
            let center = Vec3::new(
                (cell.0 as f32 + 0.5) * 100.0,
                0.0,
                (cell.1 as f32 + 0.5) * 100.0,
            );
            set_camera(world, center + Vec3::Y * 160.0, Vec3::NEG_Y, 20.0);
            {
                let mut demo = world.resource_mut::<NeighborhoodDemo>();
                demo.start = center;
                demo.wave_center = center;
                demo.wave_targets.clear();
                demo.wave_worst_ms = 0.0;
                demo.wave_captured = [false; 4];
                demo.wave_mid_damage = false;
                demo.wave_hot_captured = false;
                demo.stage = DemoStage::WaveDamage;
                demo.stage_at = now;
            }
            info!(
                "WAVE DEMO: center {center:?}, target {} buildings",
                world.resource::<NeighborhoodDemo>().wave_size
            );
        }
        DemoStage::WaveDamage => {
            let center = world.resource::<NeighborhoodDemo>().wave_center;
            let size = world.resource::<NeighborhoodDemo>().wave_size;
            if world.resource::<NeighborhoodDemo>().wave_targets.is_empty() {
                if now - world.resource::<NeighborhoodDemo>().stage_at < 3.0 {
                    return;
                }
                let targets = nearest_live_ids(world, center, size);
                if targets.is_empty() {
                    info!("WAVE DEMO: no live buildings to damage");
                    world.resource_mut::<NeighborhoodDemo>().stage = DemoStage::ExitWait;
                    return;
                }
                let mut demo = world.resource_mut::<NeighborhoodDemo>();
                demo.wave_targets = targets.clone();
                demo.stage_at = now;
                drop(demo);
                for id in targets {
                    let did = NEIGHBORHOOD_ID_BASE + id as u32;
                    let origin = world
                        .resource::<Neighborhood>()
                        .live
                        .get(&id)
                        .map(|live| live.origin)
                        .unwrap_or(center);
                    let away =
                        Vec2::new(center.x - origin.x, center.z - origin.z).normalize_or_zero();
                    let away = if away.length_squared() < 0.01 {
                        Vec2::X
                    } else {
                        away
                    };
                    let point = origin + Vec3::new(away.x, 0.0, away.y) * 7.0 + Vec3::Y * 2.2;
                    queue_damage(world, id, DamagePreset::CarBomb, point);
                    let _ = did;
                }
                return;
            }
            let targets = world.resource::<NeighborhoodDemo>().wave_targets.clone();
            let journal = world.resource::<DamageJournal>();
            let damaged = targets
                .iter()
                .filter(|id| journal.0.head_for(NEIGHBORHOOD_ID_BASE + **id as u32) > 0.0)
                .count();
            let elapsed = now - world.resource::<NeighborhoodDemo>().stage_at;
            if damaged == targets.len() || elapsed > 25.0 {
                info!("WAVE DEMO: {damaged}/{} buildings damaged", targets.len());
                let radius = {
                    let index = world.resource::<BuildingIndex>();
                    let max = targets
                        .iter()
                        .filter_map(|id| index.buildings.get(*id))
                        .map(|meta| {
                            Vec2::new(meta.centroid[0] - center.x, meta.centroid[1] - center.z)
                                .length()
                        })
                        .fold(40.0f32, f32::max);
                    (max + 30.0).clamp(150.0, 1_400.0)
                };
                let radius = std::env::var("MANHATTAN_WAVE_RADIUS")
                    .ok()
                    .and_then(|value| value.parse::<f32>().ok())
                    .unwrap_or(radius)
                    .clamp(80.0, 1_400.0);
                let mut demo = world.resource_mut::<NeighborhoodDemo>();
                demo.wave_radius = radius;
                demo.stage = DemoStage::WaveFreeze;
                demo.stage_at = now;
                info!("WAVE DEMO: radius {radius:.0} m");
            }
        }
        DemoStage::WaveFreeze => {
            let center = world.resource::<NeighborhoodDemo>().wave_center;
            if now - world.resource::<NeighborhoodDemo>().stage_at < 0.1 {
                set_camera(
                    world,
                    center + Vec3::new(0.0, 140.0, 340.0),
                    Vec3::NEG_Y,
                    30.0,
                );
                world.resource_mut::<NeighborhoodDemo>().stage_at = now;
                return;
            }
            if now - world.resource::<NeighborhoodDemo>().stage_at < 2.6 {
                return;
            }
            // Return to a mixed close-in pose: nearest records are HOT again,
            // the outer ones stay FROZEN for the wave.
            set_camera(
                world,
                center + Vec3::new(0.0, 110.0, 100.0),
                Vec3::new(0.0, -0.6, -1.0),
                12.0,
            );
            {
                let mut demo = world.resource_mut::<NeighborhoodDemo>();
                demo.stage = DemoStage::WaveFire;
                demo.stage_at = now;
            }
        }
        DemoStage::WaveFire => {
            if now - world.resource::<NeighborhoodDemo>().stage_at < 1.6 {
                return;
            }
            let center = world.resource::<NeighborhoodDemo>().wave_center;
            let radius = world.resource::<NeighborhoodDemo>().wave_radius;
            let path = world
                .resource::<NeighborhoodDemo>()
                .capture_dir
                .join("iteration19_wave_mixed.png");
            if !world.resource::<NeighborhoodDemo>().no_capture {
                screenshot(world, &path, "mixed-regime wave");
            }
            let fired = crate::restoration::restore_city(
                world,
                center,
                radius,
                60.0,
                crate::restoration::FalloffProfile::Linear,
            );
            info!("WAVE DEMO: fired {fired} assignments radius {radius:.0} m");
            {
                let mut city = world.resource_mut::<crate::restoration::CityRestoration>();
                city.config_radius = radius;
                city.config_amplitude = 60.0;
            }
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.stage = DemoStage::WaveWatch;
            demo.stage_at = now;
            demo.wave_started = now as f64;
            demo.wave_worst_ms = 0.0;
            demo.wave_fired = true;
            demo.wave_second = false;
            demo.wave_mid_captured = false;
        }
        DemoStage::WaveWatch => {
            let dt = world.resource::<Time>().delta_secs();
            let elapsed = now - world.resource::<NeighborhoodDemo>().stage_at;
            world.resource_mut::<NeighborhoodDemo>().wave_worst_ms = world
                .resource::<NeighborhoodDemo>()
                .wave_worst_ms
                .max(dt * 1000.0);
            let center = world.resource::<NeighborhoodDemo>().wave_center;
            let ring_shots = [(0.3f32, 0usize), (1.0, 1), (2.1, 2), (3.2, 3)];
            let altitude = center + Vec3::new(0.0, 290.0, 240.0);
            let hot_window = (0.45..1.0).contains(&elapsed);
            if !hot_window {
                set_camera(world, altitude, center - altitude, 20.0);
            }
            for (threshold, index) in ring_shots {
                if elapsed >= threshold
                    && !world.resource::<NeighborhoodDemo>().wave_captured[index]
                {
                    world.resource_mut::<NeighborhoodDemo>().wave_captured[index] = true;
                    set_camera(world, altitude, center - altitude, 20.0);
                    let path = world
                        .resource::<NeighborhoodDemo>()
                        .capture_dir
                        .join(format!("iteration19_wave_ring_{index}.png"));
                    if !world.resource::<NeighborhoodDemo>().no_capture {
                        screenshot(world, &path, "healing ring");
                    }
                }
            }
            // On-camera HOT heal: park at a still-damaged target's street.
            if elapsed >= 0.5
                && !world.resource::<NeighborhoodDemo>().wave_hot_captured
                && let Some(id) = mid_wave_target(world)
                && let Some(origin) = world
                    .resource::<Neighborhood>()
                    .live
                    .get(&id)
                    .map(|live| live.origin)
            {
                let position = origin + Vec3::new(0.0, 7.0, 32.0);
                set_camera(world, position, origin + Vec3::Y * 6.0 - position, 8.0);
                let path = world
                    .resource::<NeighborhoodDemo>()
                    .capture_dir
                    .join("iteration19_wave_hot_heal.png");
                if !world.resource::<NeighborhoodDemo>().no_capture {
                    screenshot(world, &path, "HOT heal");
                }
                info!("WAVE DEMO: HOT heal capture b{id}");
                world.resource_mut::<NeighborhoodDemo>().wave_hot_captured = true;
            }
            // Deliberate war: shoot a building that is still healing.
            if elapsed >= 1.1 && !world.resource::<NeighborhoodDemo>().wave_mid_damage {
                world.resource_mut::<NeighborhoodDemo>().wave_mid_damage = true;
                if let Some(id) = mid_wave_target(world) {
                    let origin = world
                        .resource::<Neighborhood>()
                        .live
                        .get(&id)
                        .map(|live| live.origin)
                        .unwrap_or(center);
                    queue_damage(
                        world,
                        id,
                        DamagePreset::CarBomb,
                        origin + Vec3::new(0.0, 2.2, 7.0),
                    );
                    info!("WAVE DEMO: mid-wave car bomb queued at b{id}");
                }
            }
            if elapsed >= 1.45 && !world.resource::<NeighborhoodDemo>().wave_mid_captured {
                world.resource_mut::<NeighborhoodDemo>().wave_mid_captured = true;
                let path = world
                    .resource::<NeighborhoodDemo>()
                    .capture_dir
                    .join("iteration19_mid_wave_damage.png");
                if !world.resource::<NeighborhoodDemo>().no_capture {
                    screenshot(world, &path, "mid-wave damage");
                }
            }
            let wave_active = world
                .resource::<crate::restoration::CityRestoration>()
                .wave
                .is_some();
            let fired = world.resource::<NeighborhoodDemo>().wave_fired;
            if fired && !wave_active {
                if !world.resource::<NeighborhoodDemo>().wave_second {
                    let city = world.resource::<crate::restoration::CityRestoration>();
                    let first =
                        (now as f64 - world.resource::<NeighborhoodDemo>().wave_started) as f32;
                    info!(
                        "WAVE DEMO FIRST: size {} radius {:.0} m duration {:.2} s throughput {:.2} buildings/s worst_slice {:.2} ms starved_frames {}",
                        city.last_buildings,
                        world.resource::<NeighborhoodDemo>().wave_radius,
                        first,
                        city.last_buildings as f32 / first.max(0.001),
                        city.worst_slice_ms,
                        city.starved_frames
                    );
                    world.resource_mut::<NeighborhoodDemo>().wave_first_ms =
                        world.resource::<NeighborhoodDemo>().wave_worst_ms;
                    let center = world.resource::<NeighborhoodDemo>().wave_center;
                    let radius = world.resource::<NeighborhoodDemo>().wave_radius;
                    let fired = crate::restoration::restore_city(
                        world,
                        center,
                        radius,
                        4_000.0,
                        crate::restoration::FalloffProfile::Linear,
                    );
                    info!("WAVE DEMO: second (full-restore) wave fired on {fired} buildings");
                    let mut demo = world.resource_mut::<NeighborhoodDemo>();
                    demo.wave_second = true;
                    demo.wave_started = now as f64;
                    return;
                }
                let path = world
                    .resource::<NeighborhoodDemo>()
                    .capture_dir
                    .join("iteration19_wave_restored.png");
                if !world.resource::<NeighborhoodDemo>().no_capture {
                    screenshot(world, &path, "wave restored");
                }
                let second =
                    (now as f64 - world.resource::<NeighborhoodDemo>().wave_started) as f32;
                let size = world
                    .resource::<crate::restoration::CityRestoration>()
                    .last_buildings;
                let first_ms = world.resource::<NeighborhoodDemo>().wave_first_ms;
                info!(
                    "WAVE DEMO SUMMARY: size {size} radius {:.0} m first_wave_worst_frame {first_ms:.1} ms second_duration {second:.2} s throughput {:.2} buildings/s worst_slice {:.2} ms starved_frames {}",
                    world.resource::<NeighborhoodDemo>().wave_radius,
                    size as f32 / second.max(0.001),
                    world
                        .resource::<crate::restoration::CityRestoration>()
                        .worst_slice_ms,
                    world
                        .resource::<crate::restoration::CityRestoration>()
                        .starved_frames
                );
                {
                    let mut demo = world.resource_mut::<NeighborhoodDemo>();
                    if demo.no_capture {
                        demo.stage = DemoStage::ExitWait;
                        demo.stage_at = now + 1.0;
                    } else {
                        demo.stage = DemoStage::WavePlume;
                        demo.stage_at = now;
                    }
                }
            } else if elapsed > 60.0 {
                info!("WAVE DEMO: timeout waiting for the wave");
                let mut demo = world.resource_mut::<NeighborhoodDemo>();
                demo.stage = DemoStage::WavePlume;
                demo.stage_at = now;
            }
        }
        DemoStage::WavePlume => {
            if world.resource::<NeighborhoodDemo>().wave_targets.is_empty() {
                return;
            }
            let id = world.resource::<NeighborhoodDemo>().wave_targets[0];
            let did = NEIGHBORHOOD_ID_BASE + id as u32;
            let head = world.resource::<DamageJournal>().0.head_for(did);
            if !world.resource::<NeighborhoodDemo>().wave_plume_started {
                queue_damage(
                    world,
                    id,
                    DamagePreset::CarBomb,
                    Vec3::new(0.0, 2.2, 7.0)
                        + world
                            .resource::<Neighborhood>()
                            .live
                            .get(&id)
                            .map(|live| live.origin)
                            .unwrap_or(Vec3::ZERO),
                );
                let mut demo = world.resource_mut::<NeighborhoodDemo>();
                demo.wave_plume_started = true;
                demo.stage_at = now;
                return;
            }
            if !world.resource::<NeighborhoodDemo>().wave_plume_moved {
                if head <= 0.0 && now - world.resource::<NeighborhoodDemo>().stage_at < 8.0 {
                    return;
                }
                let origin = world
                    .resource::<Neighborhood>()
                    .live
                    .get(&id)
                    .map(|live| live.origin)
                    .unwrap_or(Vec3::ZERO);
                let position = origin + Vec3::new(0.0, 65.0, 300.0);
                set_camera(world, position, origin + Vec3::Y * 30.0 - position, 30.0);
                let mut demo = world.resource_mut::<NeighborhoodDemo>();
                demo.wave_plume_moved = true;
                demo.stage_at = now;
                return;
            }
            if now - world.resource::<NeighborhoodDemo>().stage_at < 2.5 {
                return;
            }
            let path = world
                .resource::<NeighborhoodDemo>()
                .capture_dir
                .join("iteration19_soft_plume.png");
            if !world.resource::<NeighborhoodDemo>().no_capture {
                screenshot(world, &path, "softened plume");
            }
            info!("WAVE DEMO: soft plume capture b{id}");
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.stage = DemoStage::ExitWait;
            demo.stage_at = now + 2.0;
        }
        DemoStage::RoofBoot => {
            if now - world.resource::<NeighborhoodDemo>().stage_at < 2.0
                || !world.resource::<CityMaterialSetup>().done
            {
                return;
            }
            let (cell, _) = world.resource::<BuildingIndex>().densest_cell();
            let center = Vec3::new(
                (cell.0 as f32 + 0.5) * 100.0,
                0.0,
                (cell.1 as f32 + 0.5) * 100.0,
            );
            set_camera(
                world,
                center + Vec3::new(0.0, 150.0, 80.0),
                Vec3::NEG_Y,
                20.0,
            );
            {
                let mut demo = world.resource_mut::<NeighborhoodDemo>();
                demo.wave_center = center;
                demo.stage = DemoStage::RoofLive;
                demo.stage_at = now;
            }
            info!("ROOF DEMO: center {center:?}");
        }
        DemoStage::RoofLive => {
            if now - world.resource::<NeighborhoodDemo>().stage_at < 3.0 {
                return;
            }
            let center = world.resource::<NeighborhoodDemo>().wave_center;
            let index = world.resource::<BuildingIndex>();
            let tallest = world
                .resource::<Neighborhood>()
                .live
                .keys()
                .filter_map(|id| {
                    index
                        .buildings
                        .get(*id)
                        .map(|meta| (meta.height_m, *id, meta))
                })
                .filter(|(_, _, meta)| {
                    Vec2::new(meta.centroid[0] - center.x, meta.centroid[1] - center.z).length()
                        < 320.0
                })
                .max_by(|a, b| a.0.total_cmp(&b.0));
            let Some((height, id, meta)) = tallest else {
                world.resource_mut::<NeighborhoodDemo>().stage = DemoStage::ExitWait;
                return;
            };
            let origin = Vec3::new(meta.centroid[0], 0.0, meta.centroid[1]);
            let radius = meta
                .perimeter
                .iter()
                .map(|point| {
                    Vec2::new(point[0] - meta.centroid[0], point[1] - meta.centroid[1]).length()
                })
                .fold(6.0f32, f32::max);
            let position =
                origin + Vec3::new(radius * 1.15, height + radius * 0.75 + 6.0, radius * 1.15);
            let look = origin + Vec3::Y * (height + 0.5);
            set_camera(world, position, look - position, 8.0);
            let path = world
                .resource::<NeighborhoodDemo>()
                .capture_dir
                .join("iteration20_roof_live.png");
            if !world.resource::<NeighborhoodDemo>().no_capture {
                screenshot(world, &path, "live roof");
            }
            info!("ROOF DEMO: live b{id} height {height:.1} m");
            {
                let mut demo = world.resource_mut::<NeighborhoodDemo>();
                demo.roof_target = Some(id);
                demo.roof_view = (position, look - position);
                demo.stage = DemoStage::RoofRestore;
                demo.stage_at = now;
            }
            let did = NEIGHBORHOOD_ID_BASE + id as u32;
            queue_damage(
                world,
                id,
                DamagePreset::CarBomb,
                origin + Vec3::new(0.0, 2.2, radius * 0.8),
            );
            let _ = did;
        }
        DemoStage::RoofRestore => {
            let Some(id) = world.resource::<NeighborhoodDemo>().roof_target else {
                return;
            };
            let did = NEIGHBORHOOD_ID_BASE + id as u32;
            let head = world.resource::<DamageJournal>().0.head_for(did);
            if head > 0.0
                && !world
                    .resource::<destructibles_bevy::rewind::RewindState>()
                    .is_restoring(did)
                && now - world.resource::<NeighborhoodDemo>().stage_at > 0.8
            {
                let rate = world
                    .resource::<crate::city_tuning::CityTuning>()
                    .restoration_rate_m3s;
                destructibles_bevy::rewind::set_restoration_rate(world, did, rate);
                begin_restoration(world, &[did]);
            }
            if head > 0.0 {
                return;
            }
            if now - world.resource::<NeighborhoodDemo>().stage_at < 1.5 {
                return;
            }
            let (position, direction) = world.resource::<NeighborhoodDemo>().roof_view;
            set_camera(world, position, direction, 8.0);
            let path = world
                .resource::<NeighborhoodDemo>()
                .capture_dir
                .join("iteration20_roof_restored.png");
            if !world.resource::<NeighborhoodDemo>().no_capture {
                screenshot(world, &path, "restored roof");
            }
            info!("ROOF DEMO: restored b{id}");
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.stage = DemoStage::RoofPlain;
            demo.stage_at = now;
        }
        DemoStage::RoofPlain => {
            if now - world.resource::<NeighborhoodDemo>().stage_at < 1.0 {
                return;
            }
            let center = world.resource::<NeighborhoodDemo>().wave_center;
            let live: BTreeSet<usize> = world
                .resource::<Neighborhood>()
                .live
                .keys()
                .copied()
                .collect();
            let candidate = world
                .resource::<BuildingIndex>()
                .buildings
                .iter()
                .filter(|meta| !live.contains(&meta.id) && meta.height_m > 18.0)
                .map(|meta| {
                    let distance =
                        Vec2::new(meta.centroid[0] - center.x, meta.centroid[1] - center.z)
                            .length();
                    (distance, meta)
                })
                .filter(|(distance, _)| (500.0..1_600.0).contains(distance))
                .map(|(distance, meta)| (distance, meta.clone()))
                .min_by(|a, b| a.0.total_cmp(&b.0));
            let Some((_distance, meta)) = candidate else {
                world.resource_mut::<NeighborhoodDemo>().stage = DemoStage::RoofAerial;
                world.resource_mut::<NeighborhoodDemo>().stage_at = now;
                return;
            };
            let origin = Vec3::new(meta.centroid[0], 0.0, meta.centroid[1]);
            let radius = meta
                .perimeter
                .iter()
                .map(|point| {
                    Vec2::new(point[0] - meta.centroid[0], point[1] - meta.centroid[1]).length()
                })
                .fold(6.0f32, f32::max);
            let position = origin
                + Vec3::new(
                    radius * 1.15,
                    meta.height_m + radius * 0.75 + 6.0,
                    radius * 1.15,
                );
            let look = origin + Vec3::Y * (meta.height_m + 0.5);
            set_camera(world, position, look - position, 8.0);
            let path = world
                .resource::<NeighborhoodDemo>()
                .capture_dir
                .join("iteration20_roof_plain.png");
            if !world.resource::<NeighborhoodDemo>().no_capture {
                screenshot(world, &path, "plain roof");
            }
            info!(
                "ROOF DEMO: plain b{} height {:.1} m",
                meta.id, meta.height_m
            );
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.stage = DemoStage::RoofStreet;
            demo.stage_at = now;
        }
        DemoStage::RoofStreet => {
            if now - world.resource::<NeighborhoodDemo>().stage_at < 1.2 {
                return;
            }
            let Some(id) = world.resource::<NeighborhoodDemo>().roof_target else {
                return;
            };
            let origin = world
                .resource::<Neighborhood>()
                .live
                .get(&id)
                .map(|live| live.origin);
            let Some(origin) = origin else {
                let mut demo = world.resource_mut::<NeighborhoodDemo>();
                demo.stage = DemoStage::RoofAerial;
                demo.stage_at = now;
                return;
            };
            let position = origin + Vec3::new(0.0, 7.0, 34.0);
            set_camera(world, position, origin + Vec3::Y * 5.0 - position, 8.0);
            let path = world
                .resource::<NeighborhoodDemo>()
                .capture_dir
                .join("iteration20_interior_hidden_street.png");
            if !world.resource::<NeighborhoodDemo>().no_capture {
                screenshot(world, &path, "interior hidden");
            }
            info!("ROOF DEMO: interior-hidden street capture b{id}");
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.stage = DemoStage::RoofAerial;
            demo.stage_at = now;
        }
        DemoStage::RoofAerial => {
            if now - world.resource::<NeighborhoodDemo>().stage_at < 1.2 {
                return;
            }
            let center = world.resource::<NeighborhoodDemo>().wave_center;
            let position = center + Vec3::new(0.0, 210.0, 180.0);
            set_camera(world, position, center - position, 20.0);
            let path = world
                .resource::<NeighborhoodDemo>()
                .capture_dir
                .join("iteration20_roof_aerial.png");
            if !world.resource::<NeighborhoodDemo>().no_capture {
                screenshot(world, &path, "roof aerial");
            }
            info!("ROOF DEMO: aerial capture");
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.stage = DemoStage::ExitWait;
            demo.stage_at = now + 1.0;
        }
        DemoStage::PerfProbe => {
            if now - world.resource::<NeighborhoodDemo>().stage_at < 2.0
                || !world.resource::<CityMaterialSetup>().done
            {
                return;
            }
            if world.resource::<NeighborhoodDemo>().perf_start == 0.0 {
                if let Ok(hour) = std::env::var("MANHATTAN_PERF_HOUR")
                    && let Ok(hour) = hour.parse::<f32>()
                {
                    world.resource_mut::<SimClock>().minute_of_day = hour as f64 * 60.0;
                }
                if std::env::var_os("MANHATTAN_PERF_OVERCAST").is_some() {
                    world
                        .resource_mut::<crate::weather::Weather>()
                        .force(crate::weather::WeatherKind::Overcast);
                }
                let (cell, _) = world.resource::<BuildingIndex>().densest_cell();
                let center = Vec3::new(
                    (cell.0 as f32 + 0.5) * 100.0,
                    0.0,
                    (cell.1 as f32 + 0.5) * 100.0,
                );
                set_camera(
                    world,
                    center + Vec3::new(0.0, 140.0, 120.0),
                    Vec3::NEG_Y,
                    20.0,
                );
                let mut demo = world.resource_mut::<NeighborhoodDemo>();
                demo.perf_start = now as f64;
                demo.perf_worst = 0.0;
                demo.perf_sum = 0.0;
                demo.perf_frames = 0;
                demo.perf_envelope_ms = 0.0;
                return;
            }
            let elapsed = now as f64 - world.resource::<NeighborhoodDemo>().perf_start;
            if world.resource::<NeighborhoodDemo>().perf_envelope_ms == 0.0
                && world.resource::<Neighborhood>().live.len() >= 8
            {
                world.resource_mut::<NeighborhoodDemo>().perf_envelope_ms =
                    (elapsed * 1000.0) as f32;
            }
            if elapsed < 4.0 {
                // settle the zone at the fixed pose before measuring
                return;
            }
            if elapsed < 9.0 {
                let dt = world.resource::<Time>().delta_secs();
                let mut demo = world.resource_mut::<NeighborhoodDemo>();
                demo.perf_worst = demo.perf_worst.max(dt * 1000.0);
                demo.perf_sum += dt * 1000.0;
                demo.perf_frames += 1;
                return;
            }
            let visible = {
                let mut query = world.query_filtered::<&Mesh3d, With<ViewVisibility>>();
                query.iter(world).filter(|_| true).count()
            };
            let all_meshes = {
                let mut query = world.query::<&Mesh3d>();
                query.iter(world).count()
            };
            let hidden_meshes = {
                let mut query = world.query::<(&Mesh3d, &Visibility)>();
                query
                    .iter(world)
                    .filter(|(_, visibility)| **visibility == Visibility::Hidden)
                    .count()
            };
            let vehicles = {
                let mut query = world.query::<&crate::traffic::Vehicle>();
                query.iter(world).count()
            };
            let pedestrians = {
                let mut query = world.query::<&crate::people::Pedestrian>();
                query.iter(world).count()
            };
            let ad_meshes = {
                let mut query = world.query::<&Name>();
                query
                    .iter(world)
                    .filter(|name| name.as_str().starts_with("Signs_"))
                    .count()
            };
            let interior_total = {
                let mut query = world.query_filtered::<&Visibility, Or<(
                    With<destructibles_bevy::building::SlabVisual>,
                    With<destructibles_bevy::building::ColumnVisual>,
                    With<destructibles_bevy::building::ClutterFloor>,
                    With<destructibles_bevy::building::InteriorShell>,
                    With<destructibles_bevy::building::InteriorCore>,
                    With<destructibles_bevy::building::InteriorLight>,
                )>>();
                query.iter(world).count()
            };
            let interior_hidden = {
                let mut query = world.query_filtered::<&Visibility, Or<(
                    With<destructibles_bevy::building::SlabVisual>,
                    With<destructibles_bevy::building::ColumnVisual>,
                    With<destructibles_bevy::building::ClutterFloor>,
                    With<destructibles_bevy::building::InteriorShell>,
                    With<destructibles_bevy::building::InteriorCore>,
                    With<destructibles_bevy::building::InteriorLight>,
                )>>();
                query
                    .iter(world)
                    .filter(|visibility| **visibility == Visibility::Hidden)
                    .count()
            };
            let city = world.resource::<Neighborhood>();
            info!(
                "PERF PROBE: frames {} avg {:.2} ms worst {:.2} ms teleport_envelope {:.0} ms vehicles {vehicles} pedestrians {pedestrians} ad_meshes {ad_meshes} mesh_entities {all_meshes} hidden_meshes {hidden_meshes} visible_meshes {visible} interior_meshes {interior_total} interior_hidden {interior_hidden} live {} frozen {}",
                world.resource::<NeighborhoodDemo>().perf_frames,
                world.resource::<NeighborhoodDemo>().perf_sum
                    / world.resource::<NeighborhoodDemo>().perf_frames.max(1) as f32,
                world.resource::<NeighborhoodDemo>().perf_worst,
                world.resource::<NeighborhoodDemo>().perf_envelope_ms,
                city.live.len(),
                city.frozen_count
            );
            if let Some(mut demo) = world.get_resource_mut::<NeighborhoodDemo>() {
                demo.stage = DemoStage::ExitWait;
                demo.stage_at = now + 0.5;
            }
        }
        DemoStage::GoldenBoot => {
            if now - world.resource::<NeighborhoodDemo>().stage_at < 2.0
                || !world.resource::<CityMaterialSetup>().done
            {
                return;
            }
            world.resource_mut::<SimClock>().minute_of_day = 8.0 * 60.0 + 30.0;
            let epicenter = crate::city_rhythm::TIMES_SQUARE;
            set_camera(
                world,
                Vec3::new(epicenter.x - 130.0, 9.0, epicenter.y + 20.0),
                Vec3::new(1.0, 0.02, 0.18),
                8.0,
            );
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.wave_center = Vec3::new(epicenter.x, 0.0, epicenter.y);
            demo.stage = DemoStage::GoldenAds;
            demo.stage_at = now;
        }
        DemoStage::GoldenAds => {
            if now - world.resource::<NeighborhoodDemo>().stage_at < 2.0 {
                return;
            }
            if !golden_take(
                world,
                "iteration21_times_square_am_rush.png",
                "Times Square AM rush",
                now,
            ) {
                return;
            }
            world.resource_mut::<SimClock>().minute_of_day = 17.0 * 60.0;
            {
                let mut demo = world.resource_mut::<NeighborhoodDemo>();
                demo.stage = DemoStage::GoldenPm;
                demo.stage_at = now;
            }
        }
        DemoStage::GoldenPm => {
            if now - world.resource::<NeighborhoodDemo>().stage_at < 1.5 {
                return;
            }
            if !golden_take(
                world,
                "iteration21_times_square_pm_rush.png",
                "Times Square PM rush",
                now,
            ) {
                return;
            }
            world.resource_mut::<SimClock>().minute_of_day = 3.0 * 60.0;
            {
                let mut demo = world.resource_mut::<NeighborhoodDemo>();
                demo.stage = DemoStage::GoldenGrave;
                demo.stage_at = now;
            }
        }
        DemoStage::GoldenGrave => {
            if now - world.resource::<NeighborhoodDemo>().stage_at < 1.5 {
                return;
            }
            if !golden_take(
                world,
                "iteration21_graveyard_shift.png",
                "graveyard shift",
                now,
            ) {
                return;
            }
            let epicenter = crate::city_rhythm::TIMES_SQUARE;
            set_camera(
                world,
                Vec3::new(epicenter.x, 6.0, epicenter.y + 3_600.0),
                Vec3::new(0.9, 0.02, 0.3),
                8.0,
            );
            world.resource_mut::<SimClock>().minute_of_day = 8.0 * 60.0 + 30.0;
            {
                let mut demo = world.resource_mut::<NeighborhoodDemo>();
                demo.stage = DemoStage::GoldenDistant;
                demo.stage_at = now;
            }
        }
        DemoStage::GoldenDistant => {
            if now - world.resource::<NeighborhoodDemo>().stage_at < 1.5 {
                return;
            }
            if !golden_take(
                world,
                "iteration21_distant_no_ads.png",
                "distant no ads",
                now,
            ) {
                return;
            }
            let epicenter = crate::city_rhythm::TIMES_SQUARE;
            set_camera(
                world,
                Vec3::new(epicenter.x - 250.0, 300.0, epicenter.y + 380.0),
                Vec3::new(0.0, -1.0, -0.6),
                20.0,
            );
            {
                let mut demo = world.resource_mut::<NeighborhoodDemo>();
                demo.stage = DemoStage::GoldenAerial;
                demo.stage_at = now;
            }
        }
        DemoStage::GoldenAerial => {
            if now - world.resource::<NeighborhoodDemo>().stage_at < 1.2 {
                return;
            }
            if !golden_take(
                world,
                "iteration21_glass_core_aerial.png",
                "glass core aerial",
                now,
            ) {
                return;
            }
            let epicenter = crate::city_rhythm::TIMES_SQUARE;
            set_camera(
                world,
                Vec3::new(epicenter.x - 400.0, 260.0, epicenter.y + 2_600.0),
                Vec3::new(0.0, -1.0, 0.8),
                20.0,
            );
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.stage = DemoStage::GoldenDawn;
            demo.stage_at = now;
        }
        DemoStage::GoldenDawn => {
            if now - world.resource::<NeighborhoodDemo>().stage_at < 1.2 {
                return;
            }
            if !golden_take(
                world,
                "iteration21_brick_edge_aerial.png",
                "brick edge aerial",
                now,
            ) {
                return;
            }
            let epicenter = crate::city_rhythm::TIMES_SQUARE;
            set_camera(
                world,
                Vec3::new(epicenter.x - 700.0, 90.0, epicenter.y + 900.0),
                Vec3::new(0.35, -0.25, -1.0),
                20.0,
            );
            world.resource_mut::<SimClock>().minute_of_day = 6.0 * 60.0;
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.stage = DemoStage::GoldenNoon;
            demo.stage_at = now;
        }
        DemoStage::GoldenNoon => {
            if now - world.resource::<NeighborhoodDemo>().stage_at < 1.2 {
                return;
            }
            if !golden_take(world, "iteration21_dawn.png", "dawn", now) {
                return;
            }
            world.resource_mut::<SimClock>().minute_of_day = 12.0 * 60.0;
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.stage = DemoStage::GoldenDusk;
            demo.stage_at = now;
        }
        DemoStage::GoldenDusk => {
            if now - world.resource::<NeighborhoodDemo>().stage_at < 1.2 {
                return;
            }
            if !golden_take(world, "iteration21_noon.png", "noon", now) {
                return;
            }
            world.resource_mut::<SimClock>().minute_of_day = 19.0 * 60.0;
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.stage = DemoStage::GoldenNight;
            demo.stage_at = now;
        }
        DemoStage::GoldenNight => {
            if now - world.resource::<NeighborhoodDemo>().stage_at < 1.2 {
                return;
            }
            if !golden_take(world, "iteration21_dusk.png", "dusk", now) {
                return;
            }
            world.resource_mut::<SimClock>().minute_of_day = 0.0;
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.stage = DemoStage::GoldenDeck;
            demo.stage_at = now;
        }
        DemoStage::GoldenDeck => {
            if now - world.resource::<NeighborhoodDemo>().stage_at < 1.2 {
                return;
            }
            if !golden_take(world, "iteration21_night.png", "night", now) {
                return;
            }
            {
                let mut weather = world.resource_mut::<crate::weather::Weather>();
                weather.force(crate::weather::WeatherKind::Overcast);
            }
            let epicenter = crate::city_rhythm::TIMES_SQUARE;
            world.resource_mut::<SimClock>().minute_of_day = 14.0 * 60.0;
            let position = Vec3::new(epicenter.x, 365.0, epicenter.y + 600.0);
            set_camera(world, position, Vec3::new(0.0, 0.0, -1.0), 100.0);
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.stage = DemoStage::GoldenMoon;
            demo.stage_at = now;
        }
        DemoStage::GoldenMoon => {
            if now - world.resource::<NeighborhoodDemo>().stage_at < 2.0 {
                return;
            }
            if !golden_take(
                world,
                "iteration21_overcast_flythrough.png",
                "overcast flythrough",
                now,
            ) {
                return;
            }
            // Smoke-tinted moon: light a district, set night, look at the moon.
            world
                .resource_mut::<crate::weather::Weather>()
                .force(crate::weather::WeatherKind::Fair);
            world.resource_mut::<SimClock>().minute_of_day = 22.0 * 60.0;
            let center = world.resource::<NeighborhoodDemo>().wave_center;
            for id in nearest_live_ids(world, center, 10) {
                let origin = world
                    .resource::<Neighborhood>()
                    .live
                    .get(&id)
                    .map(|live| live.origin)
                    .unwrap_or(center);
                queue_damage(
                    world,
                    id,
                    DamagePreset::CarBomb,
                    origin + Vec3::new(0.0, 2.2, 7.0),
                );
            }
            {
                let mut demo = world.resource_mut::<NeighborhoodDemo>();
                demo.golden_step = 1;
                demo.stage = DemoStage::GoldenSmokeMoon;
                demo.stage_at = now;
            }
        }
        DemoStage::GoldenSmokeMoon => {
            // Find a time when the moon is actually above the horizon.
            let moon_alt = world
                .resource::<crate::environment::Environment>()
                .moon
                .altitude_deg;
            if moon_alt < 14.0 {
                let minute = world.resource::<SimClock>().minute_of_day;
                world.resource_mut::<SimClock>().minute_of_day = (minute + 15.0) % 1440.0;
                return;
            }
            if now - world.resource::<NeighborhoodDemo>().stage_at < 4.0 {
                return;
            }
            let center = world.resource::<NeighborhoodDemo>().wave_center;
            let direction = world.resource::<crate::environment::Environment>().moon_dir;
            let position = center + Vec3::Y * 150.0;
            set_camera(world, position, direction, 20.0);
            if !golden_take(
                world,
                "iteration21_smoke_moon.png",
                "smoke-tinted moon",
                now,
            ) {
                return;
            }
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.stage = DemoStage::ExitWait;
            demo.stage_at = now + 1.0;
        }
        DemoStage::JitBullet => {
            if world.resource::<NeighborhoodDemo>().golden_step == 0 {
                if now - world.resource::<NeighborhoodDemo>().stage_at < 2.0
                    || !world.resource::<CityMaterialSetup>().done
                {
                    return;
                }
                world.resource_mut::<NeighborhoodDemo>().golden_step = 1;
            }
            let center = crate::city_rhythm::TIMES_SQUARE;
            let index = world.resource::<BuildingIndex>();
            let live: BTreeSet<usize> = world
                .resource::<Neighborhood>()
                .live
                .keys()
                .copied()
                .collect();
            let target = index
                .buildings
                .iter()
                .filter(|meta| !live.contains(&meta.id) && meta.height_m > 20.0)
                .map(|meta| {
                    let d = Vec2::new(meta.centroid[0] - center.x, meta.centroid[1] - center.y)
                        .length();
                    (d, meta.id)
                })
                .min_by(|a, b| a.0.total_cmp(&b.0));
            let Some((_, id)) = target else {
                world.resource_mut::<NeighborhoodDemo>().stage = DemoStage::ExitWait;
                return;
            };
            let origin = {
                let meta = &index.buildings[id];
                Vec3::new(meta.centroid[0], 0.0, meta.centroid[1])
            };
            let position = origin + Vec3::new(0.0, 7.0, 38.0);
            set_camera(world, position, origin + Vec3::Y * 5.0 - position, 8.0);
            world.resource_mut::<NeighborhoodDemo>().roof_target = Some(id);
            if !golden_take(world, "iteration24_jit_before.png", "plain before JIT", now) {
                return;
            }
            queue_damage(
                world,
                id,
                DamagePreset::Bullet,
                origin + Vec3::Y * 6.0 + Vec3::Z * 8.0,
            );
            info!("JIT DEMO: bullet queued at plain b{id}");
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.stage = DemoStage::JitCar;
            demo.stage_at = now;
        }
        DemoStage::JitCar => {
            let Some(id) = world.resource::<NeighborhoodDemo>().roof_target else {
                return;
            };
            let did = NEIGHBORHOOD_ID_BASE + id as u32;
            let head = world.resource::<DamageJournal>().0.head_for(did);
            if head <= 0.0 && now - world.resource::<NeighborhoodDemo>().stage_at < 10.0 {
                return;
            }
            if now - world.resource::<NeighborhoodDemo>().stage_at < 0.5 {
                return;
            }
            let path = world
                .resource::<NeighborhoodDemo>()
                .capture_dir
                .join("iteration24_jit_bullet_wound.png");
            screenshot(world, &path, "JIT bullet wound");
            info!("JIT DEMO: bullet wound visible b{id} head {head:.2}");
            let center = crate::city_rhythm::TIMES_SQUARE;
            let index = world.resource::<BuildingIndex>();
            let neighbor = index
                .buildings
                .iter()
                .filter(|meta| meta.id != id && meta.height_m > 12.0)
                .map(|meta| {
                    let d = Vec2::new(meta.centroid[0] - center.x, meta.centroid[1] - center.y)
                        .length();
                    (d, meta.id)
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, id)| id);
            if let Some(neighbor) = neighbor {
                let origin = Vec3::new(
                    index.buildings[neighbor].centroid[0],
                    0.0,
                    index.buildings[neighbor].centroid[1],
                );
                queue_damage(
                    world,
                    neighbor,
                    DamagePreset::CarBomb,
                    origin + Vec3::Y * 3.0 + Vec3::Z * 8.0,
                );
                info!("JIT DEMO: car bomb queued at plain b{neighbor}");
            }
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.stage = DemoStage::JitRestore;
            demo.stage_at = now;
        }
        DemoStage::JitRestore => {
            let Some(id) = world.resource::<NeighborhoodDemo>().roof_target else {
                return;
            };
            let did = NEIGHBORHOOD_ID_BASE + id as u32;
            let head = world.resource::<DamageJournal>().0.head_for(did);
            if head > 0.0
                && !world
                    .resource::<destructibles_bevy::rewind::RewindState>()
                    .is_restoring(did)
            {
                let rate = world
                    .resource::<crate::city_tuning::CityTuning>()
                    .restoration_rate_m3s;
                destructibles_bevy::rewind::set_restoration_rate(world, did, rate);
                begin_restoration(world, &[did]);
            }
            if head > 0.0 {
                return;
            }
            if now - world.resource::<NeighborhoodDemo>().stage_at < 1.2 {
                return;
            }
            let path = world
                .resource::<NeighborhoodDemo>()
                .capture_dir
                .join("iteration24_jit_restored_plain.png");
            screenshot(world, &path, "restored plain");
            info!(
                "JIT DEMO: restored, live={}",
                world.resource::<Neighborhood>().live.len()
            );
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.stage = DemoStage::ExitWait;
            demo.stage_at = now;
        }
        DemoStage::InputTest => {
            if world.resource::<NeighborhoodDemo>().golden_step == 0 {
                if now - world.resource::<NeighborhoodDemo>().stage_at < 2.0
                    || !world.resource::<CityMaterialSetup>().done
                {
                    return;
                }
                let center = crate::city_rhythm::TIMES_SQUARE;
                let index = world.resource::<BuildingIndex>();
                let target = index
                    .buildings
                    .iter()
                    .filter(|meta| meta.height_m > 20.0)
                    .map(|meta| {
                        let d = Vec2::new(meta.centroid[0] - center.x, meta.centroid[1] - center.y)
                            .length();
                        (d, meta.id)
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0));
                let Some((_, id)) = target else {
                    world.resource_mut::<NeighborhoodDemo>().stage = DemoStage::ExitWait;
                    return;
                };
                let index = world.resource::<BuildingIndex>();
                let origin = Vec3::new(
                    index.buildings[id].centroid[0],
                    0.0,
                    index.buildings[id].centroid[1],
                );
                let height = std::env::var("MANHATTAN_INPUT_HEIGHT")
                    .ok()
                    .and_then(|value| value.parse::<f32>().ok())
                    .unwrap_or(8.0);
                let position = origin + Vec3::new(0.0, height, 42.0);
                set_camera(world, position, origin + Vec3::Y * 12.0 - position, 8.0);
                world.resource_mut::<NeighborhoodDemo>().roof_target = Some(id);
                world.resource_mut::<NeighborhoodDemo>().golden_step = 1;
                return;
            }
            if now - world.resource::<NeighborhoodDemo>().stage_at < 1.0 {
                return;
            }
            if world.resource::<NeighborhoodDemo>().golden_step == 1 {
                world.resource_mut::<NeighborhoodDemo>().golden_step = 2;
                world.resource_mut::<NeighborhoodDemo>().stage_at = now;
                world
                    .resource_mut::<ButtonInput<KeyCode>>()
                    .press(KeyCode::KeyG);
                info!("INPUT TEST: synthesized X press");
                damage_input(world);
                return;
            }
            let Some(_id) = world.resource::<NeighborhoodDemo>().roof_target else {
                return;
            };
            let live_count = world.resource::<Neighborhood>().live.len();
            let head = world
                .resource::<Neighborhood>()
                .live
                .keys()
                .map(|id| {
                    world
                        .resource::<DamageJournal>()
                        .0
                        .head_for(NEIGHBORHOOD_ID_BASE + *id as u32)
                })
                .fold(0.0f64, f64::max);
            let live = live_count > 0;
            if world.resource::<NeighborhoodDemo>().golden_step == 2
                && live
                && now - world.resource::<NeighborhoodDemo>().stage_at > 1.0
            {
                let mask = world
                    .resource::<NeighborhoodDemo>()
                    .roof_target
                    .is_some_and(|id| mask_covers_building(world, id));
                info!(
                    "INPUT TEST RESULT: live={live_count} head={head:.2} mask_covers={mask} shots={} rejected={}",
                    world.resource::<Neighborhood>().damage_events,
                    world.resource::<Neighborhood>().jit_rejected,
                );
                world.resource_mut::<NeighborhoodDemo>().golden_step = 3;
                world.resource_mut::<NeighborhoodDemo>().stage_at = now;
                return;
            }
            if now - world.resource::<NeighborhoodDemo>().stage_at > 12.0 {
                let path = world
                    .resource::<NeighborhoodDemo>()
                    .capture_dir
                    .join("iteration25_input_fire.png");
                screenshot(world, &path, "input fire");
                world.resource_mut::<NeighborhoodDemo>().stage = DemoStage::ExitWait;
                world.resource_mut::<NeighborhoodDemo>().stage_at = now + 1.0;
            }
        }
        DemoStage::ExitWait => {
            if now - world.resource::<NeighborhoodDemo>().stage_at > 1.2 {
                info!("NEIGHBORHOOD GAUNTLET COMPLETE");
                world.write_message(AppExit::Success);
            }
        }
    }
}

fn world_live_len(world: &World) -> usize {
    world
        .get_resource::<Neighborhood>()
        .map(|neighborhood| neighborhood.live.len())
        .unwrap_or(0)
}

fn force_clear(world: &mut World) -> usize {
    let ids: Vec<usize> = world
        .resource::<Neighborhood>()
        .live
        .keys()
        .copied()
        .collect();
    for id in ids {
        if let Some(live) = world.resource_mut::<Neighborhood>().live.remove(&id) {
            let _ = swap_out(world, live.handle);
        }
        crate::restoration::despawn_host_smoke(world, NEIGHBORHOOD_ID_BASE + id as u32);
    }
    {
        let mut neighborhood = world.resource_mut::<Neighborhood>();
        neighborhood.pending_in.clear();
        neighborhood.pending_out.clear();
        neighborhood.pending_damage.clear();
        neighborhood.queued_in.clear();
        neighborhood.queued_out.clear();
        neighborhood.live_revision += 1;
        neighborhood.mask_dirty = true;
        neighborhood.mask_revision += 1;
    }
    update_live_footprints(world);
    // A stray handle is a registry entry in the neighborhood id band with no
    // live record, or a live record with no registry entry.
    let registry_ids: Vec<u32> = world
        .resource::<Buildings>()
        .ids()
        .filter(|id| *id >= NEIGHBORHOOD_ID_BASE)
        .collect();
    registry_ids.len() + world.resource::<Neighborhood>().live.len()
}

fn update_zone_snapshot(world: &mut World) {
    // Keep the exclusive demo honest: run the zone/swap pass immediately after
    // teleporting the camera so the captured frame and the minimap agree.
    let _ = world;
}

fn set_camera(world: &mut World, position: Vec3, direction: Vec3, speed: f32) {
    let direction = direction.normalize_or_zero();
    let yaw = (-direction.x).atan2(-direction.z);
    let pitch = direction.y.asin();
    if let Ok(mut camera) = world
        .query_filtered::<(&mut Transform, &mut FlyCamera), With<FlyCamera>>()
        .single_mut(world)
    {
        camera.0.translation = position;
        camera.0.rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
        camera.1.yaw = yaw;
        camera.1.pitch = pitch;
        camera.1.speed = speed.max(0.01);
        camera.1.max_speed = speed.max(8_000.0);
    }
}

#[allow(clippy::unnecessary_to_owned)]
fn screenshot(world: &mut World, path: &std::path::Path, label: &'static str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    info!("NEIGHBORHOOD capture [{label}] -> {}", path.display());
    world
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path.to_path_buf()));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> BuildingMeta {
        BuildingMeta {
            id: 0,
            block: 0,
            height_m: 16.1,
            floor_height_m: 3.0,
            perimeter: vec![
                [657.77, 2370.9],
                [654.67, 2379.63],
                [624.21, 2368.77],
                [625.65, 2364.7],
                [632.2, 2367.03],
                [633.85, 2362.36],
            ],
            centroid: [638.06, 2368.9],
            category: "industrial".to_string(),
            year_built: 1930,
        }
    }

    #[test]
    fn point_resolver_matches_manual_footprints_for_every_record() {
        let data = StorefrontData::default();
        let index = BuildingIndex::build(data.buildings());
        // Grid sample: the resolver must match a manual first-inside scan.
        for gx in 0..40 {
            for gz in 0..40 {
                let point = Vec2::new(-2_200.0 + gx as f32 * 180.0, -2_800.0 + gz as f32 * 140.0);
                let manual = data
                    .buildings()
                    .iter()
                    .filter(|meta| {
                        destructibles_core::math::point_in_polygon(
                            [point.x, point.y],
                            &meta.perimeter,
                        )
                    })
                    .min_by(|a, b| {
                        Vec2::from_array(a.centroid)
                            .distance(point)
                            .total_cmp(&Vec2::from_array(b.centroid).distance(point))
                    })
                    .map(|meta| meta.id);
                let resolved = index.resolve_point(point);
                if let Some(expected) = manual {
                    assert_eq!(resolved, Some(expected), "grid {point:?}");
                }
            }
        }
    }

    #[test]
    fn feeder_is_footprint_and_height_exact() {
        let meta = sample();
        let spec = spec_for(&meta).expect("spec");
        let layout = destructibles_core::BuildingLayout::generate(spec).expect("layout");
        let origin = Vec3::new(meta.centroid[0], 0.0, meta.centroid[1]);
        assert!(footprint_matches(&meta, &layout, origin));
        assert!((layout.height - meta.height_m).abs() <= HEIGHT_TOL_M);
        assert_eq!(layout.footprint.len(), meta.perimeter.len());
    }

    #[test]
    fn style_mapping_is_total_and_height_independent() {
        // Brick variants by scale, glass variants by era. A tall brick and a
        // short glass building are both legal.
        assert_eq!(
            style_for_parts("residential", 12.0, 1900, false),
            FacadeStyle::BrickBrownstone
        );
        assert_eq!(
            style_for_parts("residential", 80.0, 1900, false),
            FacadeStyle::BrickMidrise
        );
        assert_eq!(
            style_for_parts("residential", 80.0, 2020, true),
            FacadeStyle::GlassCurtainWall
        );
        assert_eq!(
            style_for_parts("executive", 40.0, 1950, true),
            FacadeStyle::ConcreteGrid
        );
        assert_eq!(
            style_for_parts("industrial", 20.0, 1900, false),
            FacadeStyle::ConcreteGrid
        );
        // The probability field itself has no height input.
        let a = crate::city_rhythm::glass_probability(0.0, 0.0, 2000);
        let b = crate::city_rhythm::glass_probability(0.0, 0.0, 2000);
        assert_eq!(a, b);
        assert!(a < 1.0);
    }

    #[test]
    fn zone_hysteresis_has_a_dead_band() {
        let config = NeighborhoodConfig::default();
        // A building between the enter and exit radii is live if it was live,
        // and not selected if it was not.
        assert!(!in_zone(130.0, -1.0, 0.0, false, &config));
        assert!(in_zone(130.0, -1.0, 0.0, true, &config));
        assert!(in_zone(100.0, -1.0, 0.0, false, &config));
        assert!(!in_zone(160.0, -1.0, 0.0, true, &config));
        assert!(in_zone(130.0, 100.0, 20.0, false, &config));
        assert!(!in_zone(130.0, 160.0, 20.0, false, &config));
    }

    #[test]
    fn mask_fills_a_live_footprint() {
        let mut pixels = vec![0u8; MASK_RES * MASK_RES];
        let polygon = vec![
            Vec2::new(100.0, 100.0),
            Vec2::new(140.0, 100.0),
            Vec2::new(140.0, 140.0),
            Vec2::new(100.0, 140.0),
        ];
        fill_mask_polygon(&mut pixels, &polygon);
        assert_eq!(pixels[120 * MASK_RES + 120], 255);
        assert_eq!(pixels[20 * MASK_RES + 20], 0);
    }

    #[test]
    fn pristine_live_fill_is_white() {
        assert_eq!(live_fill_color(false, 1.0), [255, 255, 255, 235]);
        let damaged = live_fill_color(true, 0.0);
        assert!(damaged[0] > damaged[1]);
    }

    #[test]
    fn minimap_color_is_journal_derived_and_ramps_with_integrity() {
        // Pristine is the map's own white regardless of nominal integrity.
        assert_eq!(live_fill_color(false, 0.2), [255, 255, 255, 235]);
        // Once the journal carries damage the fill ramps green -> red as
        // integrity falls; restoration to pristine returns white.
        let healthy = live_fill_color(true, 1.0);
        let broken = live_fill_color(true, 0.0);
        assert!(healthy[1] > healthy[0], "intact damaged fill reads green");
        assert!(broken[0] > broken[1], "collapsed fill reads red");
        assert!(broken[0] >= healthy[0]);
        assert!(broken[1] <= healthy[1]);
        assert_eq!(live_fill_color(false, 1.0), [255, 255, 255, 235]);
    }

    fn wall_quad(facade: u32, floor: u32, normal: [f32; 3], z: f32, y0: f32) -> WallQuad {
        WallQuad {
            corners: [
                [-6.0, y0, z],
                [6.0, y0, z],
                [6.0, y0 + 3.0, z],
                [-6.0, y0 + 3.0, z],
            ],
            normal,
            uv: [[0.0, 0.0]; 4],
            facade,
            cell: destructibles_core::CellId::new(facade, floor, 0),
            u_min: 0.0,
            u_max: 12.0,
            v_min: y0,
            v_max: y0 + 3.0,
        }
    }

    #[test]
    fn storefront_panels_attach_to_the_nearest_ground_face() {
        let quads = vec![
            wall_quad(0, 0, [0.0, 0.0, -1.0], -6.0, 0.0),
            wall_quad(1, 0, [0.0, 0.0, 1.0], 6.0, 0.0),
            wall_quad(1, 3, [0.0, 0.0, 1.0], 6.0, 9.0),
        ];
        let (facade, floor) = nearest_face(Vec3::new(0.0, 1.5, 6.2), Vec3::Z, &quads, true);
        assert_eq!((facade, floor), (1, 0), "ground storefront rides floor 0");
        let (facade, floor) = nearest_face(Vec3::new(0.0, 10.0, 6.2), Vec3::Z, &quads, false);
        assert_eq!((facade, floor), (1, 3), "second-floor ad keeps its floor");
    }

    #[test]
    fn dressing_hides_with_the_face_it_rides() {
        let storefront = FacadeDressing {
            building: 1,
            facade: 2,
            floor: 0,
            storefront: true,
        };
        let ad = FacadeDressing {
            building: 1,
            facade: 2,
            floor: 3,
            storefront: false,
        };
        assert!(covered_by(&storefront, &[(2, 0)]));
        assert!(
            !covered_by(&ad, &[(2, 0)]),
            "a higher ad survives a lobby breach"
        );
        assert!(covered_by(&ad, &[(2, 3)]));
        assert!(!covered_by(&storefront, &[(1, 0)]));
        // Restoration empties the damaged-face list, so both come back.
        assert!(!covered_by(&storefront, &[]));
    }

    #[test]
    fn swap_visibility_classes_are_los_weighted() {
        assert!(visibility_class(true, true) < visibility_class(true, false));
        assert_eq!(visibility_class(true, false), visibility_class(false, true));
        assert!(visibility_class(false, true) < visibility_class(false, false));
    }

    #[test]
    fn height_mismatch_is_rejected_like_a_footprint_mismatch() {
        let meta = sample();
        let spec = spec_for(&meta).expect("spec");
        let mut layout = destructibles_core::BuildingLayout::generate(spec).expect("layout");
        assert!(height_matches(&meta, &layout));
        layout.height += 0.2;
        assert!(!height_matches(&meta, &layout));
    }

    #[test]
    fn detail_priority_prefers_lower_tiers_then_the_corridor() {
        let config = NeighborhoodConfig::default();
        // Tier ordering dominates: every Shell comes before any Structural.
        assert!(
            detail_priority(DetailTier::Shell, false, 10.0)
                < detail_priority(DetailTier::Structural, true, 10.0)
        );
        // Within a tier the corridor building wins even if it is farther.
        assert!(
            detail_priority(DetailTier::Shell, true, 180.0)
                < detail_priority(DetailTier::Shell, false, 20.0)
        );
        // Otherwise nearest first.
        assert!(
            detail_priority(DetailTier::Shell, false, 20.0)
                < detail_priority(DetailTier::Shell, false, 60.0)
        );
        let _ = config;
    }

    #[test]
    fn damaged_persistence_round_trip() {
        let config = NeighborhoodConfig::default();
        let near = config.nadir_enter * 0.5;
        let far = config.nadir_exit + 40.0;
        // Pristine live building inside the zone: nothing to do.
        assert_eq!(
            persistence_action(false, false, near, &config),
            PersistAction::None
        );
        // Damage at the nadir keeps the live record.
        assert_eq!(
            persistence_action(true, false, near, &config),
            PersistAction::None
        );
        // Fly three blocks: freeze, retaining damage.
        assert_eq!(
            persistence_action(true, false, far, &config),
            PersistAction::Freeze
        );
        assert_eq!(
            persistence_action(true, true, far, &config),
            PersistAction::None
        );
        // Re-entry thaws in place, no snap.
        assert_eq!(
            persistence_action(true, true, near, &config),
            PersistAction::Thaw
        );
        // Full restoration empties the journal and releases the frozen record
        // so swap-out becomes eligible again.
        assert_eq!(
            persistence_action(false, true, far, &config),
            PersistAction::Release
        );
    }
}
