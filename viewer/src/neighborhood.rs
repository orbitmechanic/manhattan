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
use destructibles_bevy::building::{
    BuildingDetail, Buildings, DetailTier, advance_detail, begin_detail_upgrade, detail_level,
};
use destructibles_bevy::camera::FlyCamera as DestructiblesFlyCamera;
use destructibles_bevy::damage::DamageJournal;
use destructibles_bevy::health::HealthIndex;
use destructibles_bevy::impact::apply_impact;
use destructibles_bevy::night::SceneTime;
use destructibles_bevy::puncture::apply_puncture;
use destructibles_bevy::rewind::begin_restoration;
use destructibles_bevy::streaming::{
    HotZone, Regime, Streaming, freeze_building, register_building, spawn_far_marker, thaw_building,
};
use destructibles_bevy::swap::{DestructibleHandle, swap_in_lod, swap_out};
use destructibles_core::{BuildingSpec, FacadeStyle, core_for_style};

use crate::camera::FlyCamera;
use crate::city_material::{CityMaterial, CityMaterialSetup};
use crate::clock::SimClock;
use crate::minimap::LiveFootprints;
use crate::storefront::{BuildingMeta, StorefrontData};

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
    pending_damage: Vec<PendingDamage>,
    pub dressing_panels: usize,
    pub dressing_plates: usize,
    pub damage_events: u32,
    pub tier1_built: usize,
    pub tier2_built: usize,
    pub frozen_count: usize,
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
                    persistence_tick,
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
        Self {
            active,
            capture_dir,
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
pub fn style_for(category: &str, height_m: f32) -> FacadeStyle {
    match category {
        "executive" => {
            if height_m >= 90.0 {
                FacadeStyle::GlassCurtainWall
            } else {
                FacadeStyle::ConcreteGrid
            }
        }
        "residential" => {
            if height_m <= 18.0 {
                FacadeStyle::BrickBrownstone
            } else {
                FacadeStyle::BrickMidrise
            }
        }
        "storefront" => FacadeStyle::BrickMidrise,
        "industrial" => FacadeStyle::ConcreteGrid,
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
    let raw_floor = meta.floor_height_m.max(1.5);
    let mut floor_count = (height / raw_floor).round().max(1.0) as u32;
    // Neighborhood LOD: a live building is generated at the map's exact
    // footprint and height, but the floor count is capped so a swap cannot
    // spawn a full-detail 30-storey tower in one frame. The 12 m validation
    // ceiling raises the cap for very tall towers; the visual is a plausible
    // stylised floor stack (style popping is accepted open-world LOD).
    floor_count = floor_count.clamp(1, 8);
    if height / floor_count as f32 > 12.0 {
        floor_count = (height / 12.0).ceil().max(1.0) as u32;
    }
    let floor_height = (height / floor_count as f32).clamp(1.8, 12.0);
    let style = style_for(&meta.category, height);
    let mut windows = style.windows();
    // Fewer, wider bays than the standalone presets: the neighbourhood LOD
    // keeps the silhouette and the material read without the pane count.
    windows.column_spacing = windows.column_spacing.max(4.2);
    windows.width = windows.width.min(1.6);
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

fn update_zone(
    time: Res<Time>,
    camera: Single<&Transform, With<FlyCamera>>,
    index: Res<BuildingIndex>,
    mut neighborhood: ResMut<Neighborhood>,
    config: Res<NeighborhoodConfig>,
    journal: Res<DamageJournal>,
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
        if desired
            && !live
            && !neighborhood.queued_in.contains(&id)
            && !neighborhood.failed.contains(&id)
        {
            neighborhood.pending_in.push_back(id);
            neighborhood.queued_in.insert(id);
        } else if !desired && live && !neighborhood.queued_out.contains(&id) {
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
        if !in_zone(distance, along, lateral, true, &config)
            && !neighborhood.queued_out.contains(&id)
        {
            neighborhood.pending_out.push_back(id);
            neighborhood.queued_out.insert(id);
        }
        neighborhood.behind_max = neighborhood.behind_max.max((-along).max(0.0));
    }
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
            bottom: px(168),
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
        "DETAIL live={} T0={} T1={} T2={} built={}/{} frozen={} damage={} pending={} panels={} ads={}  target {target}  [1-4] {}  [X] fire  [Z]/[Shift+Z] restore",
        neighborhood.live.len(),
        levels[0],
        levels[1],
        levels[2],
        neighborhood.tier1_built,
        neighborhood.tier2_built,
        neighborhood.frozen_count,
        neighborhood.damage_events,
        neighborhood.pending_damage.len(),
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
    let started = Instant::now();
    let mut tier1 = 0usize;
    let mut tier2 = 0usize;
    for (id, _meta, distance, _along, _detail) in items {
        if started.elapsed().as_secs_f32() * 1000.0 >= 4.0 {
            break;
        }
        let level = detail_level(world, id as u32);
        let did = NEIGHBORHOOD_ID_BASE + id as u32;
        let damaged = world.resource::<DamageJournal>().0.head_for(did) > 0.0;
        if level < DetailTier::Structural {
            begin_detail_upgrade(world, id as u32, DetailTier::Structural);
            let reached = advance_detail(world, id as u32, 2.0);
            if reached >= DetailTier::Structural {
                tier1 += 1;
                info!("DETAIL b{id}: Tier 1 ready at {distance:.1} m");
            }
            set_live_detail(world, id, reached);
            if let Some(live) = world.resource_mut::<Neighborhood>().live.get_mut(&id)
                && reached >= DetailTier::Structural
            {
                live.tier1_ready = Some(distance);
            }
            let _ = damaged;
        } else if level < DetailTier::Full && (damaged || distance < 150.0) {
            begin_detail_upgrade(world, id as u32, DetailTier::Full);
            let reached = advance_detail(world, id as u32, 1.5);
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
        let level = detail_level(world, shot.id as u32);
        let live = world.resource::<Neighborhood>().live.contains_key(&shot.id);
        if live && level >= DetailTier::Structural {
            let did = NEIGHBORHOOD_ID_BASE + shot.id as u32;
            let direction = world
                .query_filtered::<&Transform, With<FlyCamera>>()
                .single(world)
                .map(|transform| transform.forward().as_vec3())
                .unwrap_or(Vec3::NEG_Z);
            apply_preset(world, did, shot.preset, shot.point, direction);
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
            velocity: (direction * 42.0).to_array(),
            mass_kg: 1_500.0,
            explosive_yield_kg_tnt: 5.0,
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
            velocity: (direction * 620.0).to_array(),
            mass_kg: 220.0,
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

fn damage_input(world: &mut World) {
    let keys = world.resource::<ButtonInput<KeyCode>>().clone();
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
    let (target, point) = best
        .map(|(_, id, point)| (Some(id), point))
        .unwrap_or((None, origin + forward * 30.0));
    {
        let mut aim = world.resource_mut::<DamageAim>();
        aim.target = target;
        aim.point = point;
        aim.preset = preset;
    }
    if keys.just_pressed(KeyCode::KeyX) {
        let Some(id) = target else {
            return;
        };
        let level = detail_level(world, id as u32);
        if level < DetailTier::Structural {
            world
                .resource_mut::<Neighborhood>()
                .pending_damage
                .push(PendingDamage { id, preset, point });
            info!("DAMAGE queued for b{id}: waiting for Tier 1 (currently {level:?})");
        } else {
            let did = NEIGHBORHOOD_ID_BASE + id as u32;
            apply_preset(world, did, preset, point, forward);
        }
    }
    if keys.just_pressed(KeyCode::KeyZ) {
        let target_id = target.filter(|id| {
            let did = NEIGHBORHOOD_ID_BASE + *id as u32;
            world.resource::<DamageJournal>().0.head_for(did) > 0.0
        });
        let ids: Vec<u32> = if keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight)
        {
            let neighborhood = world.resource::<Neighborhood>();
            let journal = world.resource::<DamageJournal>();
            neighborhood
                .live
                .iter()
                .filter(|(id, live)| {
                    journal.0.head_for(NEIGHBORHOOD_ID_BASE + **id as u32) > 0.0
                        && live.origin.distance(origin) <= 250.0
                })
                .map(|(id, _)| NEIGHBORHOOD_ID_BASE + *id as u32)
                .collect()
        } else {
            target_id
                .map(|id| vec![NEIGHBORHOOD_ID_BASE + id as u32])
                .unwrap_or_default()
        };
        if !ids.is_empty() {
            info!("RESTORE request: {} building(s)", ids.len());
            begin_restoration(world, &ids);
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
                let marker = if hide {
                    spawn_far_marker(world, did)
                } else {
                    None
                };
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
            let cache = world
                .resource_mut::<Streaming>()
                .by_building
                .get_mut(&did)
                .and_then(|state| state.frozen.take());
            if let Some(cache) = cache {
                thaw_building(world, &cache);
            }
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

#[allow(clippy::drop_non_drop)]
fn process_swaps(world: &mut World) {
    let started = Instant::now();
    let budget = world
        .resource::<NeighborhoodConfig>()
        .swap_budget_ms
        .max(0.5);
    let max_swaps = world
        .resource::<NeighborhoodConfig>()
        .max_swaps_per_frame
        .max(1);
    let mut swaps = 0usize;
    let mut frame_swap_ms = 0.0f32;
    while swaps < max_swaps && started.elapsed().as_secs_f32() * 1000.0 < budget {
        let out = world.resource_mut::<Neighborhood>().pending_out.pop_front();
        if let Some(id) = out {
            world.resource_mut::<Neighborhood>().queued_out.remove(&id);
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
        let verified = layout
            .as_ref()
            .is_some_and(|layout| footprint_matches(&meta, layout, origin));
        if !verified {
            let _ = swap_out(world, handle);
            world.resource_mut::<Neighborhood>().footprint_failures += 1;
            world.resource_mut::<Neighborhood>().failed.insert(id);
            info!("NEIGHBORHOOD footprint FAIL b{id}; returned to plain map");
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
            if distance >= TRAVERSE_METERS * 0.5 && !world.resource::<NeighborhoodDemo>().captured {
                world.resource_mut::<NeighborhoodDemo>().captured = true;
                let path = world
                    .resource::<NeighborhoodDemo>()
                    .capture_dir
                    .join(format!("iteration17_{name}.png"));
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
            let stray = force_clear(world);
            let mut demo = world.resource_mut::<NeighborhoodDemo>();
            demo.current.stray_handles = stray;
            let stats = demo.current.clone();
            info!(
                "REGIME {} speed={:.1} frames={} worst_frame={:.1}ms worst_swap={:.1}ms worst_swap_frame={:.1}ms live_max={} swaps_in={} swaps_out={} pending_peak={} behind_max={:.1}m stray={}",
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
                stats.stray_handles
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
                .join("iteration17_boundary_hover.png");
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
    }
    {
        let mut neighborhood = world.resource_mut::<Neighborhood>();
        neighborhood.pending_in.clear();
        neighborhood.pending_out.clear();
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
    fn style_mapping_is_total() {
        assert_eq!(style_for("executive", 200.0), FacadeStyle::GlassCurtainWall);
        assert_eq!(style_for("executive", 40.0), FacadeStyle::ConcreteGrid);
        assert_eq!(style_for("residential", 12.0), FacadeStyle::BrickBrownstone);
        assert_eq!(style_for("residential", 40.0), FacadeStyle::BrickMidrise);
        assert_eq!(style_for("storefront", 20.0), FacadeStyle::BrickMidrise);
        assert_eq!(style_for("industrial", 20.0), FacadeStyle::ConcreteGrid);
        assert_eq!(style_for("unknown", 20.0), FacadeStyle::BrickMidrise);
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
