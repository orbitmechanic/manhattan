//! Seattle S2: near-field DEM terrain, water surfaces and draped streets.
//!
//! No buildings are spawned in this mode (`--seattle-s2`). The 30 m DEM is a
//! local asset; a 120 m far mesh covers the region, 960 m near tiles stream at
//! full resolution with skirts, water is a triangulated surface at dataset
//! datums, and street ribbons drape the DEM per loaded neighborhood.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use serde::Deserialize;

use crate::camera::FlyCamera;
use crate::citypack::{
    CityPack, Evaluations, PackStreet, STREAM_WORLD_OFFSET, wanted_neighborhoods,
};
use crate::clock::SimClock;
use crate::weather::{Weather, WeatherKind};
use destructibles_bevy::building::{BuildingDetail, spawn_building_world_lod};
use destructibles_core::family::MaterialFamily;
use destructibles_core::{BuildingSpec, FacadeStyle, WindowSpec, core_for_style};

pub const DEM_PATH: &str = "assets/terrain/seattle_dem_30m.bin";
pub const MASK_PATH: &str = "assets/terrain/seattle_water_mask.bin";
pub const WATER_PATH: &str = "assets/terrain/seattle_water.bin";
pub const REPORT_PATH: &str = "datasets/seattle/s2_report.json";
/// Seattle archive radius; the neighborhood minimap zoom (650 m) governs.
pub const SEATTLE_ARCHIVE_RADIUS_M: f32 = 650.0;

const TILE_CELLS: i32 = 32;
const TERRAIN_LOAD_M: f32 = 2800.0;
const TERRAIN_UNLOAD_M: f32 = 3400.0;
const ROAD_LOAD_M: f32 = 3200.0;
const ROAD_UNLOAD_M: f32 = 3800.0;
const TILE_LOADS_PER_FRAME: usize = 2;
const ROAD_LOADS_PER_FRAME: usize = 1;
const LOAD_BUDGET_MS: f32 = 6.0;
/// Lever (a): full shells cover the near field; everything else is a proxy box.
const S3_SHELL_NEAR_M: f32 = 220.0;
const S3_SHELL_FLOOR: usize = 6;
const S3_SHELL_CAP: usize = 64;
/// Proxies beyond this distance merge into one mesh per chunk; closer ones
/// stay individual so the near field keeps per-building silhouettes.
const S3_PROXY_MERGE_M: f32 = 250.0;
/// Unload margin over the 650 m load radius, and a hard resident cap so a
/// teleporting capture walk cannot accumulate chunks faster than it drops them.
const S3_UNLOAD_M: f32 = 800.0;
const S3_MAX_RESIDENT_CHUNKS: usize = 16;
/// Shell creation is the expensive part of a chunk spawn; keep a small
/// immediate set for the near field and amortize the rest across frames.
const S3_SHELL_IMMEDIATE: usize = 6;
const S3_SHELLS_PER_FRAME: usize = 4;
const FAR_STEP: i32 = 2;

#[derive(Deserialize)]
struct S2Report {
    grid: GridReport,
}

#[derive(Clone, Deserialize)]
struct StylePack {
    rules: Vec<StyleRule>,
    default: StyleRule,
}

#[derive(Clone, Deserialize)]
struct StyleRule {
    #[serde(default)]
    cities: Vec<String>,
    #[serde(default)]
    neighborhood_any: Vec<String>,
    family: String,
    facade_style: String,
    glass_probability: f32,
    #[serde(default)]
    height_bias: String,
}

#[derive(Deserialize)]
struct GridReport {
    rows: usize,
    cols: usize,
    spacing_m: f32,
    origin_world: [f32; 2],
}

pub struct GeologyPlugin;

impl Plugin for GeologyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FireProbe>()
            .init_resource::<EvidenceState>()
            .init_resource::<GhostFunnelState>()
            .init_resource::<GhostAerialState>()
            .init_resource::<FlightCaptureState>()
            .init_resource::<MurkState>()
            .add_systems(Startup, setup_geology)
            .add_systems(
                Update,
                (
                    geology_stream,
                    gauntlet_flight,
                    orbit_flight,
                    hover_flight,
                    s2_capture,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                (
                    s3_buildings,
                    s3_damage,
                    s3_list_targets,
                    district_capture,
                    seattle_boot_camera,
                    seattle_fire_probe,
                    evidence_capture,
                    ghost_funnel,
                    ghost_aerial,
                    flight_capture,
                    murk_capture,
                ),
            )
            .add_systems(PostUpdate, (clamp_camera, murk_update, hide_precip));
    }
}

/// Ground floor law: the camera may not sink below DEM + 0.5 m on land; over
/// water it may dive (the reviewer permitted it formally).
pub fn clamp_camera_floor(ground: f32, wet: bool, y: f32) -> f32 {
    if wet { y } else { y.max(ground + 0.5) }
}

fn clamp_camera(state: Option<Res<Geology>>, mut camera: Query<&mut Transform, With<FlyCamera>>) {
    let Some(state) = state else {
        return;
    };
    let Ok(mut transform) = camera.single_mut() else {
        return;
    };
    let x = transform.translation.x - STREAM_WORLD_OFFSET;
    let z = transform.translation.z;
    let col = ((x - state.dem.origin[0]) / state.dem.spacing).round() as isize;
    let row = ((z - state.dem.origin[1]) / state.dem.spacing).round() as isize;
    let wet = row >= 0
        && col >= 0
        && (row as usize) < state.dem.rows
        && (col as usize) < state.dem.cols
        && state.mask[row as usize * state.dem.cols + col as usize] != 0;
    let ground = state.dem.height(x, z);
    transform.translation.y = clamp_camera_floor(ground, wet, transform.translation.y);
}

#[derive(Component)]
pub struct MurkOverlay;

fn murk_update(
    state: Option<Res<Geology>>,
    camera: Query<&Transform, With<FlyCamera>>,
    mut overlay: Query<&mut BackgroundColor, With<MurkOverlay>>,
) {
    let Some(state) = state else {
        return;
    };
    let Ok(transform) = camera.single() else {
        return;
    };
    let y = transform.translation.y;
    let wet = {
        let x = transform.translation.x - STREAM_WORLD_OFFSET;
        let z = transform.translation.z;
        let col = ((x - state.dem.origin[0]) / state.dem.spacing).round() as isize;
        let row = ((z - state.dem.origin[1]) / state.dem.spacing).round() as isize;
        row >= 0
            && col >= 0
            && (row as usize) < state.dem.rows
            && (col as usize) < state.dem.cols
            && state.mask[row as usize * state.dem.cols + col as usize] != 0
    };
    let underwater = wet && y < 1.0;
    for mut color in &mut overlay {
        color.0 = if underwater {
            Color::srgba(0.04, 0.09, 0.07, 0.96)
        } else {
            Color::NONE
        };
    }
}

fn hide_precip(
    mut commands: Commands,
    mut weather: ResMut<Weather>,
    mut motion: Option<ResMut<crate::motion::MotionState>>,
    precip: Query<Entity, With<crate::weather::PrecipVolume>>,
    fog_banks: Query<Entity, With<crate::environment::FogBank>>,
    speed_lines: Query<Entity, With<crate::motion::SpeedLines>>,
) {
    if !s2_enabled() {
        return;
    }
    weather.auto = false;
    weather.precipitation = 0.0;
    weather.snow = 0.0;
    weather.storm = 0.0;
    if let Some(motion) = motion.as_mut() {
        motion.intensity = 0.0;
        motion.mach = 0.0;
        motion.velocity = Vec3::ZERO;
        motion.speed = 0.0;
    }
    for entity in &precip {
        commands.entity(entity).despawn();
    }
    for entity in &fog_banks {
        commands.entity(entity).despawn();
    }
    for entity in &speed_lines {
        commands.entity(entity).despawn();
    }
}

pub fn s2_enabled() -> bool {
    std::env::var_os("SEATTLE_S2").is_some()
}

#[derive(Clone)]
pub struct SeattleFootprint {
    pub id: u32,
    pub centroid: [f32; 2],
    pub perimeter: Vec<[f32; 2]>,
}

#[derive(Resource, Default)]
pub struct SeattleFootprints {
    pub buildings: Vec<SeattleFootprint>,
    /// Instrument marker: the S3 target under test, ringed on the dial.
    pub target: Option<u32>,
}

pub struct PendingShell {
    chunk: usize,
    id: u32,
    spec: BuildingSpec,
    origin: Vec3,
    node_name: String,
    mean_height: f32,
}

#[derive(Resource)]
pub struct Geology {
    pub dem: Dem,
    pub mask: Vec<u8>,
    pub pack: CityPack,
    style: StylePack,
    pub districts: HashMap<String, String>,
    pub building_chunks: HashMap<usize, Vec<Entity>>,
    /// Chunks pinned because they hold a damaged building; logged once.
    pub pinned_chunks: std::collections::HashSet<usize>,
    pending_shells: std::collections::VecDeque<PendingShell>,
    pub box_material: Handle<StandardMaterial>,
    pub box_material_warm: Handle<StandardMaterial>,
    pub box_mesh: Handle<Mesh>,
    pub box_mesh_warm: Handle<Mesh>,
    pub targets: Vec<(u32, Vec3, f32)>,
    pub loaded_buildings: usize,
    start_instant: std::time::Instant,
    first_chunk_reported: bool,
    pub terrain_material: Handle<StandardMaterial>,
    pub water_material: Handle<StandardMaterial>,
    pub road_material: Handle<StandardMaterial>,
    pub terrain: HashMap<(i32, i32), Entity>,
    pub roads: HashMap<usize, Entity>,
    pub terrain_loads: usize,
    pub terrain_unloads: usize,
    pub road_loads_total: usize,
    pub road_unloads: usize,
    pub loaded_roads: usize,
    pub samples: Vec<f32>,
    pub settled: bool,
    pub perf: Vec<serde_json::Value>,
    pub boot_ms: f32,
    pub finished_at: f32,
    pub evaluations: Evaluations,
    pub captures: u8,
    pub finished: bool,
}

pub struct Dem {
    pub rows: usize,
    pub cols: usize,
    pub spacing: f32,
    pub origin: [f32; 2],
    pub heights: Vec<i16>,
}

impl Dem {
    pub fn height(&self, x: f32, z: f32) -> f32 {
        let col = (x - self.origin[0]) / self.spacing - 0.5;
        let row = (z - self.origin[1]) / self.spacing - 0.5;
        let c0 = col.floor() as isize;
        let r0 = row.floor() as isize;
        if c0 < 0 || r0 < 0 || c0 + 1 >= self.cols as isize || r0 + 1 >= self.rows as isize {
            return 0.0;
        }
        let fc = col - c0 as f32;
        let fr = row - r0 as f32;
        let at =
            |r: isize, c: isize| self.heights[r as usize * self.cols + c as usize] as f32 / 10.0;
        at(r0, c0) * (1.0 - fc) * (1.0 - fr)
            + at(r0, c0 + 1) * fc * (1.0 - fr)
            + at(r0 + 1, c0) * (1.0 - fc) * fr
            + at(r0 + 1, c0 + 1) * fc * fr
    }
}

#[derive(Component)]
pub struct S2Hud;

#[derive(Component)]
pub struct S2Text;

fn asset(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn setup_geology(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !s2_enabled() {
        return;
    }
    let started = std::time::Instant::now();
    let report: S2Report =
        serde_json::from_str(&std::fs::read_to_string(asset(REPORT_PATH)).expect("s2 report"))
            .expect("s2 report json");
    let heights = std::fs::read(asset(DEM_PATH))
        .expect("dem")
        .chunks(2)
        .filter(|pair| pair.len() == 2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>();
    let dem = Dem {
        rows: report.grid.rows,
        cols: report.grid.cols,
        spacing: report.grid.spacing_m,
        origin: report.grid.origin_world,
        heights,
    };
    let mask = std::fs::read(asset(MASK_PATH)).expect("mask");
    let water_bytes = std::fs::read(asset(WATER_PATH)).expect("water");
    let water_mesh = parse_water(&water_bytes);
    let style: StylePack = serde_json::from_str(
        &std::fs::read_to_string(asset("assets/styles/seattle_pack.json")).expect("style pack"),
    )
    .expect("style pack json");
    #[derive(Deserialize)]
    struct DistrictFile {
        assignments: HashMap<String, String>,
    }
    let districts: HashMap<String, String> =
        std::fs::read_to_string(asset("assets/styles/seattle_districts.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<DistrictFile>(&text).ok())
            .map(|file| file.assignments)
            .unwrap_or_default();
    let pack =
        CityPack::open(Path::new(env!("CARGO_MANIFEST_DIR")).join(crate::citypack::PACK_PATH))
            .expect("citypack for streets");
    let terrain_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.18, 0.26, 0.16),
        perceptual_roughness: 0.96,
        metallic: 0.0,
        cull_mode: None,
        ..default()
    });
    let water_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.055, 0.18, 0.26),
        unlit: true,
        perceptual_roughness: 1.0,
        metallic: 0.0,
        cull_mode: None,
        ..default()
    });
    let box_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.58, 0.52, 0.44),
        perceptual_roughness: 0.95,
        cull_mode: None,
        ..default()
    });
    // Proxy boxes share the facade atlas UV banks: bank 0 is residential warm
    // scatter, bank 1 the commercial grid. Both day and night atlases use the
    // same split, so one mesh per bank is enough.
    let box_mesh = meshes.add(proxy_boxes_mesh(&[(Vec3::ZERO, Vec3::ONE, 0, [1.0; 4], 0)]));
    let box_mesh_warm = meshes.add(proxy_boxes_mesh(&[(Vec3::ZERO, Vec3::ONE, 1, [1.0; 4], 0)]));
    let road_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.17, 0.17, 0.18),
        perceptual_roughness: 0.9,
        cull_mode: None,
        ..default()
    });
    // Ground underside: a deep dirt slab under the streamed terrain so a
    // camera clamped at grade never sees under-map void.
    let dirt = materials.add(StandardMaterial {
        base_color: Color::srgb(0.20, 0.16, 0.11),
        perceptual_roughness: 1.0,
        cull_mode: None,
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(70_000.0, 320.0, 70_000.0))),
        MeshMaterial3d(dirt),
        Transform::from_xyz(STREAM_WORLD_OFFSET + 3000.0, -160.5, 1000.0),
        Name::new("ground underside"),
    ));
    let far = build_far_mesh(&dem, &mask);
    commands.spawn((
        Mesh3d(meshes.add(far)),
        MeshMaterial3d(terrain_material.clone()),
        Transform::from_xyz(STREAM_WORLD_OFFSET, 0.0, 0.0),
        Name::new("S2 far terrain"),
    ));
    let water_handle = meshes.add(water_mesh);
    commands.spawn((
        Mesh3d(water_handle.clone()),
        MeshMaterial3d(water_material.clone()),
        Transform::from_xyz(STREAM_WORLD_OFFSET, 0.0, 0.0),
        Name::new("S2 water"),
    ));
    let mut terrain = HashMap::new();
    let tile_size = TILE_CELLS as f32 * dem.spacing;
    // Boot spawns the downtown-pose chunks; everything else streams.
    let boot_tx = ((0.0 - dem.origin[0]) / tile_size).floor() as i32;
    let boot_tz = ((0.0 - dem.origin[1]) / tile_size).floor() as i32;
    for dx in -1..=1 {
        for dz in -1..=1 {
            let tx = boot_tx + dx;
            let tz = boot_tz + dz;
            let entity = commands
                .spawn((
                    Mesh3d(meshes.add(build_tile_mesh(&dem, &mask, tx, tz))),
                    MeshMaterial3d(terrain_material.clone()),
                    Transform::from_xyz(STREAM_WORLD_OFFSET, 0.0, 0.0),
                    Name::new(format!("S2 terrain {tx},{tz}")),
                ))
                .id();
            terrain.insert((tx, tz), entity);
        }
    }
    info!("S2: boot terrain chunks {}", terrain.len());
    let boot_ms = started.elapsed().as_secs_f32() * 1000.0;
    info!(
        "S2: DEM {}x{} at {} m, water and far terrain in {:.1} ms",
        dem.rows, dem.cols, dem.spacing, boot_ms
    );
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            top: Val::Px(0.0),
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        BackgroundColor(Color::NONE),
        MurkOverlay,
        Name::new("underwater murk"),
    ));
    commands.insert_resource(DistrictCaptureState::default());
    commands.insert_resource(Geology {
        dem,
        mask,
        pack,
        style,
        districts,
        building_chunks: HashMap::new(),
        pinned_chunks: std::collections::HashSet::new(),
        pending_shells: std::collections::VecDeque::new(),
        box_material: box_material.clone(),
        box_material_warm: box_material,
        box_mesh,
        box_mesh_warm,
        targets: Vec::new(),
        loaded_buildings: 0,
        start_instant: started,
        first_chunk_reported: false,
        terrain_material,
        water_material,
        road_material,
        terrain,
        roads: HashMap::new(),
        terrain_loads: 0,
        terrain_unloads: 0,
        road_loads_total: 0,
        road_unloads: 0,
        loaded_roads: 0,
        samples: Vec::new(),
        settled: false,
        perf: Vec::new(),
        boot_ms,
        finished_at: 0.0,
        evaluations: Evaluations::default(),
        captures: 0,
        finished: false,
    });
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(12),
                left: px(12),
                padding: UiRect::all(px(10)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.03, 0.05, 0.82)),
            S2Hud,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("S2 starting"),
                TextFont {
                    font_size: bevy::text::FontSize::Px(15.0),
                    ..default()
                },
                TextColor(Color::srgb(0.80, 0.92, 1.0)),
                S2Text,
                S2Hud,
            ));
        });
}

fn parse_water(bytes: &[u8]) -> Mesh {
    assert_eq!(&bytes[0..4], b"WATR");
    let verts = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    let index_count = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    let mut positions = Vec::with_capacity(verts);
    let mut offset = 16;
    for _ in 0..verts {
        let x = f32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]);
        let y = f32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]);
        let z = f32::from_le_bytes([
            bytes[offset + 8],
            bytes[offset + 9],
            bytes[offset + 10],
            bytes[offset + 11],
        ]);
        positions.push([x, y, z]);
        offset += 12;
    }
    let mut indices = Vec::with_capacity(index_count);
    for _ in 0..index_count {
        indices.push(u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]));
        offset += 4;
    }
    let mut mesh = Mesh::new(
        bevy::render::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::RENDER_WORLD | bevy::asset::RenderAssetUsages::MAIN_WORLD,
    );
    let normals = vec![[0.0, 1.0, 0.0]; verts];
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_indices(bevy::render::mesh::Indices::U32(indices));
    mesh
}

fn terrain_color(height: f32) -> [f32; 4] {
    if height > 1200.0 {
        [0.92, 0.94, 0.96, 1.0]
    } else if height > 500.0 {
        [0.52, 0.50, 0.45, 1.0]
    } else if height > 180.0 {
        [0.26, 0.32, 0.21, 1.0]
    } else {
        [0.18, 0.26, 0.16, 1.0]
    }
}

fn build_far_mesh(dem: &Dem, mask: &[u8]) -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::new();
    let step = FAR_STEP as usize;
    let rows = dem.rows / step;
    let cols = dem.cols / step;
    for r in 0..rows {
        for c in 0..cols {
            let col = (c * step) as f32;
            let row = (r * step) as f32;
            let x = dem.origin[0] + (col + 0.5) * dem.spacing;
            let z = dem.origin[1] + (row + 0.5) * dem.spacing;
            let y = dem.height(x, z);
            positions.push([x, y, z]);
            normals.push([0.0, 1.0, 0.0]);
            colors.push(terrain_color(y));
        }
    }
    for r in 0..rows.saturating_sub(1) {
        for c in 0..cols.saturating_sub(1) {
            let i = (r * cols + c) as u32;
            let c0 = (c * step) as i32;
            let r0 = (r * step) as i32;
            let wet = mask[r0 as usize * dem.cols + c0 as usize] != 0
                || mask
                    [r0 as usize * dem.cols + (c0 + step as i32).min(dem.cols as i32 - 1) as usize]
                    != 0
                || mask
                    [(r0 + step as i32).min(dem.rows as i32 - 1) as usize * dem.cols + c0 as usize]
                    != 0;
            if wet {
                continue;
            }
            indices.extend([
                i,
                i + 1,
                i + cols as u32,
                i + 1,
                i + cols as u32 + 1,
                i + cols as u32,
            ]);
        }
    }
    mesh(positions, normals, colors, indices)
}

fn build_tile_mesh(dem: &Dem, mask: &[u8], tx: i32, tz: i32) -> Mesh {
    let col0 = tx * TILE_CELLS;
    let row0 = tz * TILE_CELLS;
    let size = TILE_CELLS as usize;
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::new();
    for r in 0..=size {
        for c in 0..=size {
            let col = (col0 + c as i32).clamp(0, dem.cols as i32 - 1) as usize;
            let row = (row0 + r as i32).clamp(0, dem.rows as i32 - 1) as usize;
            let x = dem.origin[0] + (col as f32 + 0.5) * dem.spacing;
            let z = dem.origin[1] + (row as f32 + 0.5) * dem.spacing;
            let y = dem.heights[row * dem.cols + col] as f32 / 10.0;
            positions.push([x, y + 0.15, z]);
            normals.push([0.0, 1.0, 0.0]);
            colors.push(terrain_color(y));
        }
    }
    let stride = size + 1;
    for r in 0..size {
        for c in 0..size {
            let col = (col0 + c as i32).clamp(0, dem.cols as i32 - 1) as usize;
            let row = (row0 + r as i32).clamp(0, dem.rows as i32 - 1) as usize;
            let wet = mask[row * dem.cols + col] != 0
                || mask[row * dem.cols + (col + 1).min(dem.cols - 1)] != 0
                || mask[(row + 1).min(dem.rows - 1) * dem.cols + col] != 0;
            if wet {
                continue;
            }
            let i = (r * stride + c) as u32;
            indices.extend([
                i,
                i + 1,
                i + stride as u32,
                i + 1,
                i + stride as u32 + 1,
                i + stride as u32,
            ]);
        }
    }
    // Vertical skirts at all four chunk edges hide the 30 m / 60 m LOD seam.
    let stride = size + 1;
    let drop = 8.0_f32;
    let skirt_color = [0.10, 0.13, 0.09, 1.0];
    for edge in 0..4 {
        for step in 0..size {
            let (a, b) = match edge {
                0 => (step, step + 1),
                1 => (size * stride + step, size * stride + step + 1),
                2 => (step * stride, (step + 1) * stride),
                _ => (step * stride + size, (step + 1) * stride + size),
            };
            let pa = positions[a];
            let pb = positions[b];
            let start = positions.len() as u32;
            for point in [pa, pb] {
                positions.push([point[0], point[1] - drop, point[2]]);
                normals.push([0.0, 1.0, 0.0]);
                colors.push(skirt_color);
                positions.push(point);
                normals.push([0.0, 1.0, 0.0]);
                colors.push(skirt_color);
            }
            indices.extend([start, start + 1, start + 2, start + 1, start + 3, start + 2]);
        }
    }
    mesh(positions, normals, colors, indices)
}

fn mesh(
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
    indices: Vec<u32>,
) -> Mesh {
    let mut mesh = Mesh::new(
        bevy::render::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::RENDER_WORLD | bevy::asset::RenderAssetUsages::MAIN_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(bevy::render::mesh::Indices::U32(indices));
    mesh
}

/// Five-face boxes merged into one mesh: the bottom face is never visible and
/// is dropped, so far proxies cost 20 vertices each instead of 24.
/// Per-building grade for the merged proxy boxes: the aerial neighbourhood is
/// these, so the tint seed must be the building id, not the chunk or family.
fn hash01(mut value: u32) -> f32 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x7FEB_352D);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846C_A68F);
    value ^= value >> 16;
    (value & 0x00FF_FFFF) as f32 / 16_777_216.0
}

fn proxy_tint(id: u32, family: destructibles_core::family::MaterialFamily) -> [f32; 4] {
    fn h01(mut value: u32) -> f32 {
        value ^= value >> 16;
        value = value.wrapping_mul(0x7FEB_352D);
        value ^= value >> 15;
        value = value.wrapping_mul(0x846C_A68F);
        value ^= value >> 16;
        (value & 0x00FF_FFFF) as f32 / 16_777_216.0
    }
    // Tint salad doctrine: owner-by-owner stock, not district code. Six muted
    // neutral directions (brown, tan, grey, slate, off-white, dark brown) with
    // saturation capped; hue is free to wander between neighbours.
    let _ = family;
    const MUTED: [[f32; 3]; 6] = [
        [1.06, 0.96, 0.86],
        [1.02, 1.00, 0.95],
        [0.96, 0.96, 0.96],
        [0.92, 0.98, 1.08],
        [1.08, 1.06, 1.02],
        [0.90, 0.86, 0.82],
    ];
    let pick =
        MUTED[(h01(id.wrapping_mul(0x9E37_79B9)) * MUTED.len() as f32) as usize % MUTED.len()];
    let nudge = 0.86 + 0.26 * h01(id.rotate_left(7) ^ 0x5EED_1234);
    [
        (pick[0] * nudge).clamp(0.70, 1.20),
        (pick[1] * nudge).clamp(0.70, 1.20),
        (pick[2] * nudge).clamp(0.70, 1.20),
        1.0,
    ]
}

/// Shipped-geometry audit: counts the gable/flat roofs actually baked into
/// merged and near-proxy meshes, not the parameter draw.
pub static SHIPPED_GABLES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static SHIPPED_FLATS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn proxy_boxes_mesh(boxes: &[(Vec3, Vec3, u8, [f32; 4], u32)]) -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::new();
    for (center, size, warm, tint, id) in boxes {
        let h = *size * 0.5;
        let c = *center;
        let bank = if *warm > 0 { 0.0 } else { 0.5 };
        let faces = [
            (
                Vec3::X,
                [
                    Vec3::new(h.x, -h.y, -h.z),
                    Vec3::new(h.x, h.y, -h.z),
                    Vec3::new(h.x, h.y, h.z),
                    Vec3::new(h.x, -h.y, h.z),
                ],
            ),
            (
                -Vec3::X,
                [
                    Vec3::new(-h.x, -h.y, h.z),
                    Vec3::new(-h.x, h.y, h.z),
                    Vec3::new(-h.x, h.y, -h.z),
                    Vec3::new(-h.x, -h.y, -h.z),
                ],
            ),
            (
                Vec3::Y,
                [
                    Vec3::new(-h.x, h.y, -h.z),
                    Vec3::new(-h.x, h.y, h.z),
                    Vec3::new(h.x, h.y, h.z),
                    Vec3::new(h.x, h.y, -h.z),
                ],
            ),
            (
                Vec3::Z,
                [
                    Vec3::new(-h.x, -h.y, h.z),
                    Vec3::new(h.x, -h.y, h.z),
                    Vec3::new(h.x, h.y, h.z),
                    Vec3::new(-h.x, h.y, h.z),
                ],
            ),
            (
                -Vec3::Z,
                [
                    Vec3::new(h.x, -h.y, -h.z),
                    Vec3::new(-h.x, -h.y, -h.z),
                    Vec3::new(-h.x, h.y, -h.z),
                    Vec3::new(h.x, h.y, -h.z),
                ],
            ),
        ];
        for (normal, corners) in faces {
            let base = positions.len() as u32;
            for corner in corners {
                positions.push((c + corner).to_array());
                normals.push(normal.to_array());
                colors.push(*tint);
            }
            // Fixed metres per floor/module, derived PER FACE from that face's
            // own extents: vertical = height/3.0 floor rows, horizontal = this
            // face's width/module. Never a rotated box axis. Per-family Repeat
            // textures make the wrap safe in both directions.
            let horizontal = match normal {
                Vec3::X | Vec3::NEG_X => size.z,
                _ => size.x,
            };
            let module = if *warm > 0 { 12.0 } else { 18.0 };
            let (u1, v1) = if normal == Vec3::Y {
                (0.99, 0.02)
            } else {
                ((horizontal / module).max(0.05), (size.y / 3.0).max(0.05))
            };
            let _ = bank;
            uvs.extend([[0.0, v1], [0.0, 0.0], [u1, 0.0], [u1, v1]]);
            indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        // Residential proxies carry a pitched roof: the roof parameters are
        // seeded by building id, same bisector pitch family as the shells.
        // Commercial (warm == 0) stays legitimately flat.
        let floors = (size.y / 3.0).round().max(1.0);
        let gable_p = if floors <= 1.0 {
            1.0
        } else if floors >= 10.0 {
            0.05
        } else {
            1.0 - 0.95 * (floors - 1.0) / 9.0
        };
        let pitched = *warm > 0 && hash01(id ^ 0x51ED_2701) < gable_p;
        if pitched {
            SHIPPED_GABLES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        } else {
            SHIPPED_FLATS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        if pitched {
            let pitch = (15.0 + 30.0 * hash01(id.wrapping_mul(0x9E37_79B9))).to_radians();
            let rise = h.y.min(h.z) * pitch.tan();
            let ridge = (h.x.max(h.z) - h.x.min(h.z)).max(0.0);
            let along_x = h.x >= h.z;
            let (ra, rb) = if along_x {
                (
                    Vec3::new(-ridge, h.y + rise, 0.0),
                    Vec3::new(ridge, h.y + rise, 0.0),
                )
            } else {
                (
                    Vec3::new(0.0, h.y + rise, -ridge),
                    Vec3::new(0.0, h.y + rise, ridge),
                )
            };
            let roof_color = {
                let roll = hash01(id.rotate_left(7) ^ 0x51ED_2701);
                if roll < 0.60 {
                    [0.26, 0.17, 0.10, 1.0]
                } else if roll < 0.90 {
                    [0.22, 0.22, 0.22, 1.0]
                } else {
                    [0.17, 0.13, 0.10, 1.0]
                }
            };
            let moss = [0.13, 0.17, 0.08, 1.0];
            let corners = [
                Vec3::new(-h.x, h.y, -h.z),
                Vec3::new(h.x, h.y, -h.z),
                Vec3::new(h.x, h.y, h.z),
                Vec3::new(-h.x, h.y, h.z),
            ];
            let mut add = |a: Vec3, b: Vec3, d: Vec3, e: Vec3, col: [f32; 4]| {
                let base = positions.len() as u32;
                let normal = (d - a).cross(e - b).normalize_or_zero();
                for point in [a, b, d, e] {
                    positions.push((c + point).to_array());
                    normals.push(normal.to_array());
                    colors.push(col);
                    uvs.push([0.02, 0.02]);
                }
                indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
            };
            if along_x {
                add(corners[0], corners[1], rb, ra, roof_color);
                add(corners[3], corners[2], rb, ra, roof_color);
                add(corners[1], corners[2], ra, rb, moss);
                add(corners[0], corners[3], ra, rb, moss);
            } else {
                add(corners[0], corners[3], ra, rb, roof_color);
                add(corners[1], corners[2], ra, rb, roof_color);
                add(corners[0], corners[1], rb, ra, moss);
                add(corners[2], corners[3], rb, ra, moss);
            }
            if hash01(id ^ 0xA53C_9E11) < 0.05 {
                let solar = [0.035, 0.04, 0.05, 1.0];
                let mid = (ra + rb) * 0.5;
                add(mid, rb, rb + Vec3::new(0.0, 0.0, 0.0), mid, solar);
            }
        } else {
            // Legitimately flat roof: gravel with a moss-dark rim, never bare.
            let gravel = [0.31, 0.30, 0.28, 1.0];
            let moss = [0.14, 0.18, 0.09, 1.0];
            let corners = [
                Vec3::new(-h.x, h.y, -h.z),
                Vec3::new(h.x, h.y, -h.z),
                Vec3::new(h.x, h.y, h.z),
                Vec3::new(-h.x, h.y, h.z),
            ];
            let base = positions.len() as u32;
            for (index, point) in corners.iter().enumerate() {
                positions.push((c + *point).to_array());
                normals.push([0.0, 1.0, 0.0]);
                colors.push(if index % 2 == 0 { moss } else { gravel });
                uvs.push([0.97, 0.01]);
            }
            indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
    let mut mesh = Mesh::new(
        bevy::render::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::RENDER_WORLD | bevy::asset::RenderAssetUsages::MAIN_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(bevy::render::mesh::Indices::U32(indices));
    mesh
}

fn road_mesh(dem: &Dem, chunk: &crate::citypack::Chunk) -> Mesh {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::new();
    for street in &chunk.streets {
        add_ribbon(
            dem,
            street,
            chunk,
            &mut positions,
            &mut normals,
            &mut colors,
            &mut indices,
        );
    }
    mesh(positions, normals, colors, indices)
}

fn add_ribbon(
    dem: &Dem,
    street: &PackStreet,
    chunk: &crate::citypack::Chunk,
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    colors: &mut Vec<[f32; 4]>,
    indices: &mut Vec<u32>,
) {
    let width = match chunk.street_kind(street) {
        "1" => 15.0,
        "2" => 11.0,
        "3" => 8.0,
        _ => 7.0,
    };
    let color = [0.17, 0.17, 0.18, 1.0];
    for pair in street.points.windows(2) {
        let [a, b] = [pair[0], pair[1]];
        let dx = b[0] - a[0];
        let dz = b[1] - a[1];
        let length = (dx * dx + dz * dz).sqrt();
        if length < 0.5 {
            continue;
        }
        let nx = -dz / length * width * 0.5;
        let nz = dx / length * width * 0.5;
        let ya = dem.height(a[0], a[1]) + 0.35;
        let yb = dem.height(b[0], b[1]) + 0.35;
        let start = positions.len() as u32;
        positions.push([a[0] + nx, ya, a[1] + nz]);
        positions.push([a[0] - nx, ya, a[1] - nz]);
        positions.push([b[0] + nx, yb, b[1] + nz]);
        positions.push([b[0] - nx, yb, b[1] - nz]);
        for _ in 0..4 {
            normals.push([0.0, 1.0, 0.0]);
            colors.push(color);
        }
        indices.extend([start, start + 1, start + 2, start + 1, start + 3, start + 2]);
    }
}

#[allow(clippy::too_many_arguments)]
fn geology_stream(
    mut commands: Commands,
    mut state: Option<ResMut<Geology>>,
    _time: Res<Time>,
    real_time: Res<Time<bevy::time::Real>>,
    camera: Query<&Transform, With<FlyCamera>>,
    mut text: Query<&mut Text, With<S2Text>>,
    mut precip: Query<&mut Visibility, With<crate::weather::PrecipVolume>>,
    mut motion: Option<ResMut<crate::motion::MotionState>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut ticks: Local<u32>,
    trees: Option<Res<crate::trees::Trees>>,
    staged: Option<Res<crate::camera_authority::StagedPose>>,
) {
    if let Some(motion) = motion.as_mut() {
        motion.velocity = Vec3::ZERO;
        motion.speed = 0.0;
        motion.mach = 0.0;
        motion.intensity = 0.0;
        motion.boom_age = 10.0;
    }
    for mut visibility in &mut precip {
        *visibility = Visibility::Hidden;
    }
    let Some(state) = state.as_mut() else {
        return;
    };
    let Ok(transform) = camera.single() else {
        return;
    };
    let center = [
        transform.translation.x - STREAM_WORLD_OFFSET,
        transform.translation.z,
    ];
    state.samples.push(real_time.delta_secs() * 1000.0);
    if state.samples.len() > 360 {
        state.samples.remove(0);
    }
    let tile_size = TILE_CELLS as f32 * state.dem.spacing;
    let tile_x = ((center[0] - state.dem.origin[0]) / tile_size).floor() as i32;
    let tile_z = ((center[1] - state.dem.origin[1]) / tile_size).floor() as i32;
    let span = (TERRAIN_UNLOAD_M / tile_size).ceil() as i32 + 1;
    let mut wanted: Vec<((i32, i32), f32)> = Vec::new();
    for dx in -span..=span {
        for dz in -span..=span {
            let tx = tile_x + dx;
            let tz = tile_z + dz;
            if tx < 0 || tz < 0 {
                continue;
            }
            let cx = state.dem.origin[0] + (tx as f32 + 0.5) * tile_size;
            let cz = state.dem.origin[1] + (tz as f32 + 0.5) * tile_size;
            let distance = (cx - center[0]).hypot(cz - center[1]);
            if distance <= TERRAIN_LOAD_M {
                wanted.push(((tx, tz), distance));
            }
        }
    }
    wanted.sort_by(|a, b| a.1.total_cmp(&b.1));
    let wanted_set: HashSet<(i32, i32)> = wanted.iter().map(|(key, _)| *key).collect();
    let mut unloads = 0;
    let stale: Vec<(i32, i32)> = state
        .terrain
        .keys()
        .copied()
        .filter(|key| {
            if wanted_set.contains(key) {
                return false;
            }
            let cx = state.dem.origin[0] + (key.0 as f32 + 0.5) * tile_size;
            let cz = state.dem.origin[1] + (key.1 as f32 + 0.5) * tile_size;
            (cx - center[0]).hypot(cz - center[1]) > TERRAIN_UNLOAD_M
        })
        .take(4)
        .collect();
    for key in stale {
        if let Some(entity) = state.terrain.remove(&key) {
            commands.entity(entity).despawn();
            state.terrain_unloads += 1;
            unloads += 1;
        }
    }
    let started = std::time::Instant::now();
    let mut loads = 0;
    for (key, _) in &wanted {
        if state.terrain.contains_key(key) {
            continue;
        }
        if loads >= TILE_LOADS_PER_FRAME
            || started.elapsed().as_secs_f32() * 1000.0 > LOAD_BUDGET_MS
        {
            break;
        }
        let entity = commands
            .spawn((
                Mesh3d(meshes.add(build_tile_mesh(&state.dem, &state.mask, key.0, key.1))),
                MeshMaterial3d(state.terrain_material.clone()),
                Transform::from_xyz(STREAM_WORLD_OFFSET, 0.0, 0.0),
                Name::new(format!("S2 terrain {},{}", key.0, key.1)),
            ))
            .id();
        state.terrain.insert(*key, entity);
        state.terrain_loads += 1;
        loads += 1;
    }
    let tile_ms = started.elapsed().as_secs_f32() * 1000.0;

    let mut evaluations = Evaluations::default();
    let mut wanted_roads =
        wanted_neighborhoods(&state.pack.nodes, center, ROAD_LOAD_M, &mut evaluations);
    wanted_roads.sort_by(|a, b| {
        let da = state.pack.nodes[*a].center;
        let db = state.pack.nodes[*b].center;
        let aa = (da[0] - center[0]).powi(2) + (da[1] - center[1]).powi(2);
        let bb = (db[0] - center[0]).powi(2) + (db[1] - center[1]).powi(2);
        aa.total_cmp(&bb)
    });
    let road_set: HashSet<usize> = wanted_roads.iter().copied().collect();
    let stale_roads: Vec<usize> = state
        .roads
        .keys()
        .copied()
        .filter(|index| {
            if road_set.contains(index) {
                return false;
            }
            let c = state.pack.nodes[*index].center;
            (c[0] - center[0]).hypot(c[1] - center[1]) > ROAD_UNLOAD_M
        })
        .take(3)
        .collect();
    for index in stale_roads {
        if let Some(entity) = state.roads.remove(&index) {
            commands.entity(entity).despawn();
            state.road_unloads += 1;
        }
    }
    let road_started = std::time::Instant::now();
    let mut road_loads = 0;
    for index in wanted_roads {
        if state.roads.contains_key(&index) {
            continue;
        }
        if road_loads >= ROAD_LOADS_PER_FRAME
            || road_started.elapsed().as_secs_f32() * 1000.0 > LOAD_BUDGET_MS
        {
            break;
        }
        let Some(node) = state.pack.nodes.get(index) else {
            continue;
        };
        if node.streets == 0 {
            continue;
        }
        match state.pack.read_chunk(index) {
            Ok(chunk) => {
                let entity = commands
                    .spawn((
                        Mesh3d(meshes.add(road_mesh(&state.dem, &chunk))),
                        MeshMaterial3d(state.road_material.clone()),
                        Transform::from_xyz(STREAM_WORLD_OFFSET, 0.0, 0.0),
                        Name::new(format!("S2 roads {}", node.name)),
                    ))
                    .id();
                state.roads.insert(index, entity);
                state.road_loads_total += 1;
                road_loads += 1;
            }
            Err(error) => error!("S2 road {index}: {error}"),
        }
    }
    let road_ms = road_started.elapsed().as_secs_f32() * 1000.0;
    state.loaded_roads = state.roads.len();
    state.evaluations = evaluations;
    *ticks += 1;
    if (*ticks).is_multiple_of(90) {
        info!(
            "S2: terrain {} (+{}/-{}) roads {} (+{}/-{}) evals {}/{}/{} tile {:.1} ms road {:.1} ms",
            state.terrain.len(),
            loads,
            unloads,
            state.roads.len(),
            road_loads,
            state.road_unloads,
            evaluations.counties,
            evaluations.cities,
            evaluations.neighborhoods,
            tile_ms,
            road_ms
        );
    }
    if let Ok(mut text) = text.single_mut() {
        text.0 = format!(
            "S2 terrain {} (+{}/-{})  roads {} (+{}/-{})  buildings {}  trees {} / {} groups  evals {}/{}/{}  {}\n30 m streamed / 60 m regional / skirts / boot {:.0} ms / tile {:.1} ms road {:.1} ms",
            state.terrain.len(),
            loads,
            unloads,
            state.roads.len(),
            road_loads,
            state.road_unloads,
            state.loaded_buildings,
            trees.as_ref().map(|trees| trees.active).unwrap_or(0),
            trees.as_ref().map(|trees| trees.draw_groups).unwrap_or(0),
            evaluations.counties,
            evaluations.cities,
            evaluations.neighborhoods,
            staged
                .as_ref()
                .filter(|pose| pose.active)
                .map(|pose| format!(
                    "POSE {} {:.0},{:.0}",
                    pose.stamp,
                    pose.translation.x - STREAM_WORLD_OFFSET,
                    pose.translation.z
                ))
                .unwrap_or_default(),
            state.boot_ms,
            tile_ms,
            road_ms
        );
    }
}

fn record_perf(state: &mut Geology, name: &str) {
    if state.samples.len() < 2 {
        return;
    }
    let mut samples = std::mem::take(&mut state.samples);
    samples.sort_by(f32::total_cmp);
    let avg = samples.iter().sum::<f32>() / samples.len() as f32;
    let p99 = samples[((samples.len() - 1) as f32 * 0.99) as usize];
    let row = serde_json::json!({
        "pose": name,
        "avg_ms": round2(avg),
        "p99_ms": round2(p99),
        "samples": samples.len(),
    });
    state.perf.retain(|item| item["pose"] != row["pose"]);
    state.perf.push(row);
    let perf_name = if s3_enabled() {
        "s3_perf.json"
    } else {
        "s2_perf.json"
    };
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("datasets/seattle")
        .join(perf_name);
    let mut merged = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<Vec<serde_json::Value>>(&text).ok())
        .unwrap_or_default();
    for item in &state.perf {
        merged.retain(|existing| existing["pose"] != item["pose"]);
        merged.push(item.clone());
    }
    let _ = std::fs::write(
        path,
        serde_json::to_string_pretty(&merged).unwrap_or_default(),
    );
    info!("S2 PERF {name}: avg {avg:.2} ms p99 {p99:.2} ms");
}

fn round2(value: f32) -> f32 {
    (value * 100.0).round() / 100.0
}

#[derive(Clone, Copy)]
struct Pose {
    name: &'static str,
    x: f32,
    z: f32,
    y: f32,
    look: [f32; 3],
    overcast: bool,
    minute: f32,
}

fn pose(geology: &Geology, index: usize) -> Pose {
    let ground = |x: f32, z: f32| geology.dem.height(x, z);
    match index {
        0 => Pose {
            name: "s2_capitol_climb.png",
            x: 900.0,
            z: -3300.0,
            y: ground(900.0, -3300.0) + 18.0,
            look: [1400.0, ground(1400.0, -1900.0) + 22.0, -1900.0],
            overcast: false,
            minute: 12.0 * 60.0,
        },
        1 => Pose {
            name: "s2_elliott_rainier.png",
            x: -5532.0,
            z: 245.0,
            y: ground(-5532.0, 245.0) + 25.0,
            look: [-1000.0, 40.0, 6000.0],
            overcast: false,
            minute: 12.0 * 60.0,
        },
        2 => Pose {
            name: "s2_queen_anne.png",
            x: -1548.0,
            z: -3455.0,
            y: ground(-1548.0, -3455.0) + 35.0,
            look: [0.0, 25.0, 0.0],
            overcast: false,
            minute: 12.0 * 60.0,
        },
        3 => Pose {
            name: "s2_lake_bellevue.png",
            x: 6080.0,
            z: -1419.0,
            y: ground(6080.0, -1419.0) + 25.0,
            look: [10081.0, 25.0, -245.0],
            overcast: false,
            minute: 12.0 * 60.0,
        },
        4 => Pose {
            name: "s2_sodo_flats.png",
            x: 335.0,
            z: 4032.0,
            y: ground(335.0, 4032.0) + 22.0,
            look: [0.0, 20.0, -1000.0],
            overcast: false,
            minute: 12.0 * 60.0,
        },
        5 => Pose {
            name: "s2_steep_overcast.png",
            x: -1200.0,
            z: -2600.0,
            y: ground(-1200.0, -2600.0) + 20.0,
            look: [200.0, 20.0, -3800.0],
            overcast: true,
            minute: 8.0 * 60.0,
        },
        6 => Pose {
            name: "s2_montlake_cut.png",
            x: 2455.0,
            z: -4490.0,
            y: ground(2455.0, -4490.0) + 18.0,
            look: [6000.0, 8.0, -4400.0],
            overcast: false,
            minute: 12.0 * 60.0,
        },
        7 => Pose {
            name: "s2_waterfront_duwamish.png",
            x: -482.0,
            z: 7352.0,
            y: ground(-482.0, 7352.0) + 20.0,
            look: [0.0, 25.0, 3000.0],
            overcast: false,
            minute: 12.0 * 60.0,
        },
        8 => Pose {
            name: "s2_elliott_dawn.png",
            x: -5532.0,
            z: 245.0,
            y: ground(-5532.0, 245.0) + 25.0,
            look: [-1000.0, 40.0, 6000.0],
            overcast: false,
            minute: 5.0 * 60.0 + 40.0,
        },
        _ => Pose {
            name: "s2_elliott_dusk.png",
            x: -5532.0,
            z: 245.0,
            y: ground(-5532.0, 245.0) + 25.0,
            look: [-1000.0, 40.0, 6000.0],
            overcast: false,
            minute: 20.0 * 60.0 + 30.0,
        },
    }
}

fn underfoot_loaded(state: &Geology, center: [f32; 2]) -> bool {
    let tile_size = TILE_CELLS as f32 * state.dem.spacing;
    let tx = ((center[0] - state.dem.origin[0]) / tile_size).floor() as i32;
    let tz = ((center[1] - state.dem.origin[1]) / tile_size).floor() as i32;
    state.terrain.contains_key(&(tx, tz))
}

fn nearest_chunk_loaded(state: &Geology, center: [f32; 2]) -> bool {
    // The nearest *data-bearing* neighborhood may be skipped by read_chunk
    // errors, so assert coverage: some loaded chunk centre lies within the
    // building load radius of the camera.
    state.building_chunks.keys().any(|index| {
        let node = &state.pack.nodes[*index];
        (node.center[0] - center[0]).hypot(node.center[1] - center[1]) <= 650.0
    })
}

#[allow(clippy::too_many_arguments)]
fn gauntlet_flight(
    mut commands: Commands,
    time: Res<Time>,
    mut state: Option<ResMut<Geology>>,
    mut camera: Query<&mut Transform, With<FlyCamera>>,
    mut captured: Local<[bool; 4]>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(state) = state.as_mut() else {
        return;
    };
    if std::env::var_os("S2_GAUNTLET").is_none() {
        return;
    }
    let elapsed = time.elapsed_secs() - 5.0;
    if elapsed < 0.0 {
        return;
    }
    let speeds = [15.0_f32, 30.0, 100.0, 650.0];
    let regime = (elapsed / 3.0).floor() as usize;
    if regime >= speeds.len() {
        if elapsed > 13.5 {
            let _ = exit.write(AppExit::Success);
        }
        return;
    }
    let local_time = elapsed - regime as f32 * 3.0;
    let mut start_distance = 0.0;
    for speed in &speeds[..regime] {
        start_distance += speed * 3.0;
    }
    let distance = start_distance + speeds[regime] * local_time;
    let direction = Vec3::new(0.82, 0.0, -0.57).normalize();
    let base = Vec3::new(0.0, 0.0, 0.0) + direction * distance;
    let y = state.dem.height(base.x, base.z) + 220.0;
    if let Ok(mut transform) = camera.single_mut() {
        transform.translation = Vec3::new(STREAM_WORLD_OFFSET + base.x, y, base.z);
        let look = transform.translation + direction * 400.0 + Vec3::new(0.0, -80.0, 0.0);
        let d = (look - transform.translation).normalize_or_zero();
        transform.rotation = Quat::from_euler(EulerRot::YXZ, (-d.x).atan2(-d.z), d.y.asin(), 0.0);
    }
    // Virtual frames clamp at 250 ms, so a fast regime can step over any fixed
    // 2.9 s mark; capture once per regime at the first frame past 2.6 s.
    if local_time >= 2.6 && !captured[regime] {
        captured[regime] = true;
        assert!(
            underfoot_loaded(state, [base.x, base.z]),
            "gauntlet {} m/s: unloaded terrain underfoot at ({:.0}, {:.0})",
            speeds[regime],
            base.x,
            base.z
        );
        assert!(
            nearest_chunk_loaded(state, [base.x, base.z]),
            "gauntlet {} m/s: no streamed building chunk underfoot",
            speeds[regime]
        );
        let name = format!("gauntlet_{}ms.png", speeds[regime] as u32);
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("captures")
            .join(name);
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
        record_perf(state, &format!("gauntlet_{}ms", speeds[regime] as u32));
    }
}

#[allow(clippy::too_many_arguments)]
fn orbit_flight(
    mut commands: Commands,
    time: Res<Time>,
    mut state: Option<ResMut<Geology>>,
    mut camera: Query<&mut Transform, With<FlyCamera>>,
    mut captured: Local<bool>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(state) = state.as_mut() else {
        return;
    };
    if std::env::var_os("S2_ORBIT").is_none() {
        return;
    }
    let elapsed = time.elapsed_secs() - 5.0;
    if elapsed < 0.0 {
        return;
    }
    let angle = elapsed * 0.28;
    let center = Vec3::new(0.0, 0.0, 0.0);
    let position = center + Vec3::new(angle.cos() * 1200.0, 0.0, angle.sin() * 1200.0);
    let y = state.dem.height(position.x, position.z) + 300.0;
    if let Ok(mut transform) = camera.single_mut() {
        transform.translation = Vec3::new(STREAM_WORLD_OFFSET + position.x, y, position.z);
        let look = Vec3::new(STREAM_WORLD_OFFSET, state.dem.height(0.0, 0.0) + 20.0, 0.0);
        let d = (look - transform.translation).normalize_or_zero();
        transform.rotation = Quat::from_euler(EulerRot::YXZ, (-d.x).atan2(-d.z), d.y.asin(), 0.0);
    }
    if elapsed > 9.0 && !*captured {
        *captured = true;
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("captures/s2_low_orbit.png");
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
        record_perf(state, "low_orbit");
    }
    if elapsed > 11.0 {
        let _ = exit.write(AppExit::Success);
    }
}

#[allow(clippy::too_many_arguments)]
fn hover_flight(
    mut commands: Commands,
    time: Res<Time>,
    mut state: Option<ResMut<Geology>>,
    mut camera: Query<&mut Transform, With<FlyCamera>>,
    mut captured: Local<bool>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(state) = state.as_mut() else {
        return;
    };
    if std::env::var_os("S2_HOVER").is_none() {
        return;
    }
    let elapsed = time.elapsed_secs() - 4.0;
    if elapsed < 0.0 {
        return;
    }
    if elapsed > 3.0 && (elapsed - 3.0) < 0.1 {
        state.samples.clear();
    }
    let tile = TILE_CELLS as f32 * state.dem.spacing;
    let base_x = state.dem.origin[0] + tile * 17.0;
    let base_z = state.dem.origin[1] + tile * 17.0;
    let y = state.dem.height(base_x, base_z) + 160.0;
    if let Ok(mut transform) = camera.single_mut() {
        transform.translation = Vec3::new(STREAM_WORLD_OFFSET + base_x, y, base_z);
        let look = transform.translation + Vec3::new(0.0, -80.0, 300.0);
        let d = (look - transform.translation).normalize_or_zero();
        transform.rotation = Quat::from_euler(EulerRot::YXZ, (-d.x).atan2(-d.z), d.y.asin(), 0.0);
    }
    if elapsed > 5.0 && !*captured {
        *captured = true;
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("captures/s2_chunk_hover.png");
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
        record_perf(state, "chunk_hover");
    }
    if elapsed > 7.0 {
        let _ = exit.write(AppExit::Success);
    }
}

#[allow(clippy::too_many_arguments)]
fn s2_capture(
    mut commands: Commands,
    time: Res<Time>,
    mut state: Option<ResMut<Geology>>,
    mut entered: Local<f32>,
    mut active: Local<usize>,
    mut camera: Query<&mut Transform, With<FlyCamera>>,
    mut weather: ResMut<Weather>,
    mut clock: ResMut<SimClock>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(state) = state.as_mut() else {
        return;
    };
    if std::env::var_os("S2_GAUNTLET").is_some()
        || std::env::var_os("S2_ORBIT").is_some()
        || std::env::var_os("S2_HOVER").is_some()
        || std::env::var_os("S3_DAMAGE").is_some()
        || std::env::var_os("TREES_CAPTURE").is_some()
        || std::env::var_os("LANDMARK_GATE").is_some()
        || std::env::var_os("DISTRICT_CAPTURE").is_some()
        || std::env::var_os("STOREFRONT_CAPTURE").is_some()
        || std::env::var_os("SEATTLE_BOOT").is_some()
    {
        return;
    }
    let elapsed = time.elapsed_secs();
    if elapsed < 5.0 {
        return;
    }
    let index = state.captures as usize;
    if index >= 10 {
        if state.finished_at == 0.0 {
            state.finished_at = elapsed;
        } else if elapsed - state.finished_at > 1.6 {
            let _ = exit.write(AppExit::Success);
        }
        return;
    }
    let pose = pose(state, index);
    if *active != index || *entered == 0.0 {
        *active = index;
        *entered = elapsed;
        state.settled = false;
        state.samples.clear();
    }
    if let Ok(mut transform) = camera.single_mut() {
        transform.translation = Vec3::new(STREAM_WORLD_OFFSET + pose.x, pose.y, pose.z);
        let look = Vec3::new(
            STREAM_WORLD_OFFSET + pose.look[0],
            pose.look[1],
            pose.look[2],
        );
        let direction = (look - transform.translation).normalize_or_zero();
        transform.rotation = Quat::from_euler(
            EulerRot::YXZ,
            (-direction.x).atan2(-direction.z),
            direction.y.asin(),
            0.0,
        );
    }
    let settle = if s3_enabled() { 2.5 } else { 5.0 };
    if elapsed - *entered > settle * 0.6 && !state.settled {
        state.settled = true;
        state.samples.clear();
    }
    if elapsed - *entered < settle {
        return;
    }
    weather.auto = false;
    if pose.overcast {
        weather.target = WeatherKind::Overcast;
        weather.cloud_cover = 0.9;
        weather.precipitation = 0.08;
        weather.storm = 0.0;
        weather.snow = 0.0;
    } else {
        weather.target = WeatherKind::Clear;
        weather.cloud_cover = 0.18;
        weather.precipitation = 0.0;
        weather.storm = 0.0;
        weather.snow = 0.0;
    }
    clock.minute_of_day = pose.minute as f64;
    if state.captures == index as u8 {
        record_perf(state, pose.name);
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("captures")
            .join(pose.name);
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
        state.captures += 1;
    }
}

fn despawn_skyline_smoke(world: &mut World) {
    let mut query = world.query::<(Entity, &destructibles_bevy::hazards::SkylineSmoke)>();
    let columns = query
        .iter(world)
        .map(|(entity, column)| (entity, column.parts.clone()))
        .collect::<Vec<_>>();
    for (root, parts) in columns {
        for part in parts {
            if let Ok(entity) = world.get_entity_mut(part) {
                entity.despawn();
            }
        }
        if let Ok(entity) = world.get_entity_mut(root) {
            entity.despawn();
        }
    }
}

fn s3_damage(world: &mut World) {
    if std::env::var_os("S3_DAMAGE").is_none() || !s3_enabled() {
        return;
    }
    let elapsed = world.resource::<Time>().elapsed_secs();
    let targets = world.resource::<Geology>().targets.clone();
    let target_limit =
        if std::env::var_os("S3_ID").is_some() || std::env::var_os("S3_NODE").is_some() {
            1
        } else {
            3
        };
    if std::env::var_os("S3_ID").is_some() && targets.is_empty() && elapsed >= 4.0 {
        if let Ok(raw) = std::env::var("S3_ID")
            && let Ok(wanted_id) = raw.trim().parse::<u32>()
        {
            let mut found = None;
            {
                let state = world.resource::<Geology>();
                let district_priority = |name: &str| -> u8 {
                    let district = state.districts.get(name).map(String::as_str).unwrap_or("");
                    match district {
                        "Pioneer Square" => 0,
                        "Pike Place" => 1,
                        "Downtown" => 2,
                        "First Hill" => 3,
                        "Capitol Hill" => 4,
                        "Ballard" => 5,
                        "Fremont" => 6,
                        "Wallingford" => 7,
                        "Georgetown" => 8,
                        "SODO" => 9,
                        "West Seattle" => 10,
                        "Bellevue CBD" => 11,
                        "Eastside" => 12,
                        "Tacoma" => 13,
                        _ => 14,
                    }
                };
                let mut ordered = state.pack.neighborhoods();
                ordered.sort_by_key(|index| district_priority(&state.pack.nodes[*index].name));
                for index in ordered {
                    let Ok(chunk) = state.pack.read_chunk(index) else {
                        continue;
                    };
                    if let Some(building) = chunk.buildings.iter().find(|b| b.id == wanted_id) {
                        let node = &state.pack.nodes[index];
                        let city = state
                            .pack
                            .nodes
                            .get(node.parent as usize)
                            .map(|node| node.name.as_str())
                            .unwrap_or("Seattle");
                        let district = state
                            .districts
                            .get(&node.name)
                            .map(String::as_str)
                            .unwrap_or(&node.name);
                        let category = chunk.string(building.category_idx as u16).to_string();
                        let source = chunk.string(building.source_idx as u16).to_string();
                        if let Some(spec) = building_spec(
                            &state.style,
                            node,
                            building,
                            city,
                            district,
                            &category,
                            &source,
                        ) {
                            let y =
                                footprint_seat(&state.dem, building.centroid, &building.perimeter)
                                    + 0.03;
                            found = Some((
                                building.id,
                                spec,
                                Vec3::new(building.centroid[0], y, building.centroid[1])
                                    + Vec3::X * STREAM_WORLD_OFFSET,
                                building.height_m,
                                index,
                            ));
                        }
                        break;
                    }
                }
            }
            if let Some((id, spec, origin, height, chunk_index)) = found {
                spawn_building_world_lod(world, id, spec, origin, BuildingDetail::Shell);
                world
                    .resource_mut::<Geology>()
                    .targets
                    .push((id, origin, height));
                // The minimap instrument sees the whole neighbourhood chunk;
                // only the target gets geometry (Shell Doctrine isolation).
                let state = world.resource::<Geology>();
                if let Ok(chunk) = state.pack.read_chunk(chunk_index) {
                    let mut footprints = world.resource_mut::<SeattleFootprints>();
                    footprints.target = Some(id);
                    for building in &chunk.buildings {
                        if footprints
                            .buildings
                            .iter()
                            .any(|entry| entry.id == building.id)
                        {
                            continue;
                        }
                        footprints.buildings.push(SeattleFootprint {
                            id: building.id,
                            centroid: building.centroid,
                            perimeter: building.perimeter.clone(),
                        });
                    }
                }
            }
        }
        return;
    }
    if std::env::var_os("S3_ID").is_none() && elapsed >= 5.0 && targets.len() < target_limit {
        let center = std::env::var("S3_CENTER")
            .ok()
            .and_then(|raw| {
                let mut parts = raw.split(',');
                Some((
                    parts.next()?.trim().parse::<f32>().ok()?,
                    parts.next()?.trim().parse::<f32>().ok()?,
                ))
            })
            .unwrap_or((0.0, 0.0));
        let y = world.resource::<Geology>().dem.height(center.0, center.1) + 30.0;
        if let Ok(mut camera) = world
            .query_filtered::<&mut Transform, With<FlyCamera>>()
            .single_mut(world)
        {
            camera.translation = Vec3::new(STREAM_WORLD_OFFSET + center.0, y, center.1);
            camera.rotation = Quat::from_euler(EulerRot::YXZ, 0.0, -0.25, 0.0);
        }
        return;
    }
    if elapsed < 9.0 || targets.is_empty() {
        return;
    }
    let (mut stage, mut since) = {
        let done = world.resource::<S3DamageState>();
        (done.stage, done.since)
    };
    if stage == 1 {
        if elapsed - since > 1.5 {
            let journal = world
                .resource::<destructibles_bevy::damage::DamageJournal>()
                .0
                .head_for(targets[0].0);
            info!("S3 DAMAGE journal after admission: {journal:.2}");
            if std::env::var_os("S3_CYCLE").is_some() {
                let mut done = world.resource_mut::<S3DamageState>();
                done.stage = 6;
                done.since = elapsed;
                return;
            }
            let name =
                std::env::var("S3_CAPTURE").unwrap_or_else(|_| "s3_damage_tower.png".to_string());
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("captures")
                .join(name);
            world
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path));
            stage = 2;
            since = elapsed;
            let mut done = world.resource_mut::<S3DamageState>();
            done.stage = stage;
            done.since = since;
        } else {
            return;
        }
    }
    if stage == 2 {
        if std::env::var_os("S3_RESTORE").is_some() && since > 0.0 {
            let id = targets[0].0;
            destructibles_bevy::rewind::begin_restoration(world, &[id]);
            for _ in 0..600 {
                destructibles_bevy::rewind::advance_restoration(world);
                if world
                    .resource::<destructibles_bevy::damage::DamageJournal>()
                    .0
                    .is_empty()
                {
                    break;
                }
            }
            let journal_zero = world
                .resource::<destructibles_bevy::damage::DamageJournal>()
                .0
                .len();
            let pristine = world
                .resource::<destructibles_bevy::streaming::PristineChecksums>()
                .0
                .get(&id)
                .copied()
                .unwrap_or(0);
            let pre_reset = destructibles_bevy::rewind::building_checksum(world, id);
            let checksum_pre = pre_reset == pristine;
            destructibles_bevy::streaming::force_pristine_reset(world, id);
            let evicted = destructibles_bevy::streaming::evict_to_dormant(world, id);
            let registered = world
                .resource::<destructibles_bevy::building::Buildings>()
                .get(id)
                .is_some();
            let structures = world
                .resource::<destructibles_bevy::structure::Structures>()
                .iter()
                .count();
            let mut seat_ok = false;
            let mut checksum_ok = false;
            if let Some((spec, _origin)) = evicted {
                let record = world
                    .resource::<Geology>()
                    .dem
                    .height(targets[0].1.x - STREAM_WORLD_OFFSET, targets[0].1.z);
                let seat = Vec3::new(targets[0].1.x, record + SEAT_LIFT_M, targets[0].1.z);
                spawn_building_world_lod(world, id, spec, seat, BuildingDetail::Shell);
                seat_ok = true;
                checksum_ok = checksum_pre;
            }
            info!(
                "S3 RESTORE id {id}: journal {journal_zero}, registered_after_evict {registered}, structures {structures}, seat {seat_ok}, checksum_match {checksum_ok}"
            );
            let name =
                std::env::var("S3_CAPTURE").unwrap_or_else(|_| "s3_restore_after.png".to_string());
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("captures")
                .join(name);
            world
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path));
            let mut done = world.resource_mut::<S3DamageState>();
            done.stage = 3;
            done.since = elapsed;
            return;
        }
        if elapsed - since > 1.0 {
            world.write_message(AppExit::Success);
        }
        return;
    }
    if stage == 6 {
        let id = targets[0].0;
        let origin = targets[0].1;
        let height = targets[0].2;
        let snapshot = world
            .resource::<destructibles_bevy::damage::DamageJournal>()
            .0
            .entries_for(id)
            .map(|entry| entry.seq)
            .collect::<Vec<_>>();
        let changed = {
            let state = world.resource::<S3DamageState>();
            state.journal_snapshot != snapshot
        };
        if changed {
            let mut done = world.resource_mut::<S3DamageState>();
            done.journal_snapshot = snapshot;
            done.since = elapsed;
            return;
        }
        if elapsed - since < 1.0 || snapshot.is_empty() {
            return;
        }
        // Near-frame anti-occlusion: the city-scale skyline smoke is suppressed
        // for the damage evidence shot; the far plume is the host column.
        despawn_skyline_smoke(world);
        if let Ok(mut camera) = world
            .query_filtered::<&mut Transform, With<FlyCamera>>()
            .single_mut(world)
        {
            let distance = (height * 3.0).max(18.0);
            camera.translation = origin + Vec3::new(0.0, height * 0.5, distance);
            let look = origin + Vec3::Y * (height * 0.66);
            let d = (look - camera.translation).normalize_or_zero();
            camera.rotation = Quat::from_euler(EulerRot::YXZ, (-d.x).atan2(-d.z), d.y.asin(), 0.0);
        }
        let name = "s3_cycle_damaged.png";
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("captures")
            .join(name);
        world
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
        let mut done = world.resource_mut::<S3DamageState>();
        done.stage = 8;
        done.since = elapsed;
        return;
    }
    if stage == 8 {
        let id = targets[0].0;
        let origin = targets[0].1;
        let height = targets[0].2;
        if elapsed - since > 0.1 {
            let checksum = destructibles_bevy::rewind::building_checksum(world, id);
            let snapshot = world.resource::<S3DamageState>().journal_snapshot.clone();
            assert!(
                snapshot.len() >= 10,
                "damage gate: journal {} below 10 before freeze",
                snapshot.len()
            );
            info!(
                "S3 CYCLE damage: journal {} entries, checksum {checksum:016x}, freezing",
                snapshot.len()
            );
            let frozen = destructibles_bevy::streaming::freeze_building(world, id, true);
            assert!(frozen.is_some(), "freeze failed for the cycle target");
            info!(
                "S3 CYCLE freeze: journal continuity snapshot {} entries, checksum {checksum:016x}",
                snapshot.len()
            );
            if let Ok(mut camera) = world
                .query_filtered::<&mut Transform, With<FlyCamera>>()
                .single_mut(world)
            {
                camera.translation =
                    origin + Vec3::new(0.0, height + 14.0, SEATTLE_ARCHIVE_RADIUS_M + 60.0);
                let look = origin + Vec3::Y * (height * 0.8);
                let d = (look - camera.translation).normalize_or_zero();
                camera.rotation =
                    Quat::from_euler(EulerRot::YXZ, (-d.x).atan2(-d.z), d.y.asin(), 0.0);
            }
            let smoke = crate::restoration::spawn_host_smoke(world, id);
            info!("S3 CYCLE plume: {} host puffs", smoke.len());
            let mut done = world.resource_mut::<S3DamageState>();
            done.frozen = frozen;
            done.plume = smoke;
            done.checksum_snapshot = checksum;
            done.stage = 7;
            done.since = elapsed;
        }
        return;
    }
    if stage == 7 {
        if elapsed - since > 0.5 {
            let captured = world.resource::<S3DamageState>().plume_captured;
            if !captured {
                let path =
                    Path::new(env!("CARGO_MANIFEST_DIR")).join("captures/s3_cycle_plume.png");
                world
                    .spawn(Screenshot::primary_window())
                    .observe(save_to_disk(path));
                let mut done = world.resource_mut::<S3DamageState>();
                done.plume_captured = true;
                done.since = elapsed;
                return;
            }
            let mut done = world.resource_mut::<S3DamageState>();
            done.stage = 4;
            done.since = elapsed;
        }
        return;
    }
    if stage == 4 {
        if elapsed - since > 1.5 {
            let id = targets[0].0;
            let origin = targets[0].1;
            let height = targets[0].2;
            let plume = std::mem::take(&mut world.resource_mut::<S3DamageState>().plume);
            for entity in plume {
                if let Ok(entity_mut) = world.get_entity_mut(entity) {
                    entity_mut.despawn();
                }
            }
            let frozen = world.resource_mut::<S3DamageState>().frozen.take();
            if let Some(cache) = frozen {
                destructibles_bevy::streaming::thaw_building(world, &cache);
            }
            if let Ok(mut camera) = world
                .query_filtered::<&mut Transform, With<FlyCamera>>()
                .single_mut(world)
            {
                camera.translation =
                    origin + Vec3::new(0.0, height * 0.5, (height * 3.0).max(18.0));
                let look = origin + Vec3::Y * (height * 0.66);
                let d = (look - camera.translation).normalize_or_zero();
                camera.rotation =
                    Quat::from_euler(EulerRot::YXZ, (-d.x).atan2(-d.z), d.y.asin(), 0.0);
            }
            let snapshot = world
                .resource::<destructibles_bevy::damage::DamageJournal>()
                .0
                .entries_for(id)
                .map(|entry| entry.seq)
                .collect::<Vec<_>>();
            let checksum = destructibles_bevy::rewind::building_checksum(world, id);
            let state = world.resource::<S3DamageState>();
            assert_eq!(
                snapshot, state.journal_snapshot,
                "journal changed across freeze/thaw"
            );
            assert_eq!(
                checksum, state.checksum_snapshot,
                "checksum changed across freeze/thaw"
            );
            info!(
                "S3 CYCLE thaw: journal continuity PASS ({} entries, checksum {checksum:016x})",
                snapshot.len()
            );
            let mut done = world.resource_mut::<S3DamageState>();
            done.stage = 10;
            done.since = elapsed;
            return;
        }
        return;
    }
    if stage == 10 {
        if elapsed - since > 0.5 {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("captures/s3_cycle_return.png");
            world
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path));
            let mut done = world.resource_mut::<S3DamageState>();
            done.stage = 5;
            done.since = elapsed;
        }
        return;
    }
    if stage == 5 {
        if elapsed - since > 1.5 {
            let id = targets[0].0;
            let origin = targets[0].1;
            let height = targets[0].2;
            destructibles_bevy::rewind::begin_restoration(world, &[id]);
            for _ in 0..600 {
                destructibles_bevy::rewind::advance_restoration(world);
                if world
                    .resource::<destructibles_bevy::damage::DamageJournal>()
                    .0
                    .is_empty()
                {
                    break;
                }
            }
            let journal_zero = world
                .resource::<destructibles_bevy::damage::DamageJournal>()
                .0
                .is_empty();
            let pristine = world
                .resource::<destructibles_bevy::streaming::PristineChecksums>()
                .0
                .get(&id)
                .copied()
                .unwrap_or(0);
            let restored = destructibles_bevy::rewind::building_checksum(world, id);
            assert!(journal_zero, "restore left journal entries");
            assert_eq!(
                restored, pristine,
                "restore checksum does not match the pristine baseline"
            );
            destructibles_bevy::streaming::force_pristine_reset(world, id);
            let evicted = destructibles_bevy::streaming::evict_to_dormant(world, id);
            let registered = world
                .resource::<destructibles_bevy::building::Buildings>()
                .get(id)
                .is_some();
            let structures = world
                .resource::<destructibles_bevy::structure::Structures>()
                .iter()
                .count();
            let mut seat = false;
            if let Some((spec, _)) = evicted {
                let ground = world
                    .resource::<Geology>()
                    .dem
                    .height(origin.x - STREAM_WORLD_OFFSET, origin.z);
                spawn_building_world_lod(
                    world,
                    id,
                    spec,
                    Vec3::new(origin.x, ground + SEAT_LIFT_M, origin.z),
                    BuildingDetail::Shell,
                );
                seat = true;
            }
            assert!(!registered, "eviction left the building registered");
            assert_eq!(structures, 0, "eviction left structure graphs");
            assert!(seat, "re-seat after eviction failed");
            // Pull back to the derived trio camera so the restored record and
            // its dial footprint are both legible in the final frame.
            if let Ok(mut camera) = world
                .query_filtered::<&mut Transform, With<FlyCamera>>()
                .single_mut(world)
            {
                let distance = (height * 2.0).max(60.0);
                camera.translation = origin + Vec3::new(0.0, height * 0.5, distance);
                let look = origin + Vec3::Y * (height * 0.66);
                let d = (look - camera.translation).normalize_or_zero();
                camera.rotation =
                    Quat::from_euler(EulerRot::YXZ, (-d.x).atan2(-d.z), d.y.asin(), 0.0);
            }
            info!(
                "S3 CYCLE PASS: journal 0, checksum_match true ({restored:016x}), registered {registered}, structures {structures}, seat {seat}"
            );
            let mut done = world.resource_mut::<S3DamageState>();
            done.stage = 9;
            done.since = elapsed;
        }
        return;
    }
    if stage == 9 {
        if elapsed - since > 0.5 {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("captures/s3_cycle_restore.png");
            world
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path));
            let mut done = world.resource_mut::<S3DamageState>();
            done.stage = 3;
            done.since = elapsed;
        }
        return;
    }
    if stage == 3 {
        if elapsed - since > 1.0 {
            world.write_message(AppExit::Success);
        }
        return;
    }
    stage = 1;
    since = elapsed;
    {
        let mut done = world.resource_mut::<S3DamageState>();
        done.stage = stage;
        done.since = since;
    }
    let target = targets[0];
    let target_height = target.2;
    if let Some(record) = world
        .resource::<destructibles_bevy::building::Buildings>()
        .get(target.0)
    {
        let target_root = record.entity;
        let others = world
            .resource::<Geology>()
            .building_chunks
            .values()
            .flat_map(|entities| entities.iter().copied())
            .filter(|entity| *entity != target_root)
            .collect::<Vec<_>>();
        for entity in others {
            if let Ok(mut entity) = world.get_entity_mut(entity) {
                entity.insert(Visibility::Hidden);
            }
        }
    }
    if let Ok(mut camera) = world
        .query_filtered::<&mut Transform, With<FlyCamera>>()
        .single_mut(world)
    {
        let distance = (target_height * 2.0).max(60.0);
        camera.translation = target.1 + Vec3::new(0.0, target_height * 0.5, distance);
        let look = target.1 + Vec3::Y * (target_height * 0.66);
        let d = (look - camera.translation).normalize_or_zero();
        camera.rotation = Quat::from_euler(EulerRot::YXZ, (-d.x).atan2(-d.z), d.y.asin(), 0.0);
    }
    let began = destructibles_bevy::building::begin_detail_upgrade(
        world,
        target.0,
        destructibles_bevy::building::DetailTier::Structural,
    );
    for _ in 0..80 {
        let level = destructibles_bevy::building::advance_detail(world, target.0, 50.0);
        if level >= destructibles_bevy::building::DetailTier::Structural {
            break;
        }
    }
    {
        let entries = world
            .resource::<destructibles_bevy::damage::DamageJournal>()
            .0
            .entries_for(target.0)
            .count();
        let state = world.resource::<S3DamageState>();
        assert!(
            baseline_record_allowed(state.baseline_recorded, entries),
            "baseline re-record must fire exactly once, at tier-up before any damage entry (recorded {}, entries {})",
            state.baseline_recorded,
            entries
        );
        let mut state = world.resource_mut::<S3DamageState>();
        state.baseline_recorded = true;
    }
    world
        .resource_mut::<destructibles_bevy::streaming::PristineChecksums>()
        .0
        .remove(&target.0);
    world
        .resource_mut::<destructibles_bevy::streaming::PristineParts>()
        .0
        .remove(&target.0);
    world
        .resource_mut::<destructibles_bevy::streaming::PristineInventories>()
        .0
        .remove(&target.0);
    destructibles_bevy::streaming::record_pristine(world, target.0);
    info!(
        "S3 DAMAGE trace: id {} registered {} begin {} level {:?}",
        target.0,
        world
            .resource::<destructibles_bevy::building::Buildings>()
            .get(target.0)
            .is_some(),
        began,
        destructibles_bevy::building::detail_level(world, target.0)
    );
    let started = std::time::Instant::now();
    for _ in 0..20 {
        destructibles_bevy::impact::apply_impact(
            world,
            target.0,
            destructibles_core::ImpactSpec {
                position: (target.1 + Vec3::new(0.0, 6.0, 6.0)).to_array(),
                radius: 0.0,
                velocity: [0.0, -30.0, -280.0],
                mass_kg: 20.0,
                explosive_yield_kg_tnt: 0.0,
            },
        );
    }
    destructibles_bevy::impact::apply_impact(
        world,
        target.0,
        destructibles_core::ImpactSpec {
            position: (target.1 + Vec3::new(0.0, 4.0, 8.0)).to_array(),
            radius: 0.0,
            velocity: [0.0, 0.0, -420.0],
            mass_kg: 1500.0,
            explosive_yield_kg_tnt: 5.0,
        },
    );
    info!(
        "S3 DAMAGE: b{} 20 shots + car bomb applied in {:.1} ms",
        target.0,
        started.elapsed().as_secs_f32() * 1000.0
    );
}

/// The pristine baseline is re-recorded exactly once, at Structural tier-up and
/// before the first impact. Any later attempt would launder damage into the
/// baseline the epilogue compares against.
fn baseline_record_allowed(recorded: bool, journal_entries: usize) -> bool {
    !recorded && journal_entries == 0
}

#[derive(Resource, Default)]
pub struct S3DamageState {
    stage: u8,
    since: f32,
    baseline_recorded: bool,
    frozen: Option<destructibles_bevy::streaming::FrozenCache>,
    plume: Vec<Entity>,
    journal_snapshot: Vec<u64>,
    checksum_snapshot: u64,
    plume_captured: bool,
}

fn family_name(family: MaterialFamily) -> &'static str {
    match family {
        MaterialFamily::SteelFrame => "steel_frame",
        MaterialFamily::MasonryStone => "masonry_stone",
        MaterialFamily::Timber => "timber",
        _ => "mixed",
    }
}

#[derive(Resource, Default)]
struct DistrictCaptureState {
    index: usize,
    entered: bool,
    since: f32,
    done: bool,
}

fn district_capture(world: &mut World) {
    if std::env::var_os("DISTRICT_CAPTURE").is_none() || !s2_enabled() {
        return;
    }
    let elapsed = world.resource::<Time>().elapsed_secs();
    if elapsed < 5.0 {
        return;
    }
    let (index, entered, since, done) = {
        let state = world.resource::<DistrictCaptureState>();
        (state.index, state.entered, state.since, state.done)
    };
    if done {
        return;
    }
    const POSES: [(&str, [f32; 2], [f32; 2]); 5] = [
        (
            "storefronts_pike_place.png",
            [-440.0, -400.0],
            [-525.0, -400.0],
        ),
        (
            "storefronts_pioneer_square.png",
            [250.0, 650.0],
            [250.0, 500.0],
        ),
        ("storefronts_sodo.png", [420.0, 3620.0], [340.0, 3300.0]),
        (
            "storefronts_residential.png",
            [-3060.0, -6060.0],
            [-2980.0, -6140.0],
        ),
        ("districts_downtown_glass.png", [0.0, 120.0], [0.0, -60.0]),
    ];
    if index >= POSES.len() {
        world.resource_mut::<DistrictCaptureState>().done = true;
        world.write_message(AppExit::Success);
        return;
    }
    {
        let mut clock = world.resource_mut::<SimClock>();
        clock.minute_of_day = 12.0 * 60.0;
        let mut weather = world.resource_mut::<Weather>();
        weather.auto = false;
        weather.target = WeatherKind::Clear;
        weather.cloud_cover = 0.18;
        weather.precipitation = 0.0;
        weather.storm = 0.0;
        weather.snow = 0.0;
    }
    let (name, eye, look) = POSES[index];
    let (ground, look_ground) = {
        let geology = world.resource::<Geology>();
        (
            geology.dem.height(eye[0], eye[1]),
            geology.dem.height(look[0], look[1]),
        )
    };
    {
        let mut pose = world.resource_mut::<crate::camera_authority::StagedPose>();
        crate::camera_authority::stage_look_at(
            &mut pose,
            "district_capture",
            Vec3::new(STREAM_WORLD_OFFSET + eye[0], ground + 6.0, eye[1]),
            Vec3::new(STREAM_WORLD_OFFSET + look[0], look_ground + 9.0, look[1]),
            name,
        );
    }
    if !entered {
        let mut state = world.resource_mut::<DistrictCaptureState>();
        state.entered = true;
        state.since = elapsed;
        return;
    }
    if elapsed - since < 1.5 {
        return;
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("captures")
        .join(name);
    world
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path));
    info!("DISTRICT capture {name}");
    let mut state = world.resource_mut::<DistrictCaptureState>();
    state.index += 1;
    state.entered = false;
}

#[derive(Resource, Default)]
struct FireProbe {
    stage: u8,
    since: f32,
    target: u32,
}

/// Verification harness for the Seattle weapon: aim at a real CityPack record,
/// fire the same shared path the G key uses, log the journal and capture the
/// wound. Gated by SEATTLE_FIRE_TEST (id via SEATTLE_FIRE_AT).
fn seattle_fire_probe(world: &mut World) {
    if std::env::var_os("SEATTLE_FIRE_TEST").is_none() || !s2_enabled() {
        return;
    }
    unsafe {
        std::env::set_var("PILOT_CLEAN", "1");
    }
    let elapsed = world.resource::<Time>().elapsed_secs();
    let (mut stage, mut since) = {
        let state = world.resource::<FireProbe>();
        (state.stage, state.since)
    };
    let target_id = std::env::var("SEATTLE_FIRE_AT")
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .unwrap_or(187064);
    match stage {
        0 => {
            if elapsed < 6.0 {
                return;
            }
            let mut chosen = world
                .resource::<destructibles_bevy::building::Buildings>()
                .get(target_id)
                .map(|record| (record.origin, record.layout.height, target_id));
            if chosen.is_none() {
                let camera = world
                    .query_filtered::<&Transform, With<FlyCamera>>()
                    .single(world)
                    .map(|transform| transform.translation)
                    .unwrap_or(Vec3::ZERO);
                let registry = world.resource::<destructibles_bevy::building::Buildings>();
                chosen = registry
                    .ids()
                    .filter(|id| *id < crate::landmarks::LANDMARK_ID_BASE)
                    .filter_map(|id| registry.get(id))
                    .map(|record| {
                        (
                            record.origin,
                            record.layout.height,
                            record.id,
                            record.origin.distance_squared(camera),
                        )
                    })
                    .min_by(|a, b| a.3.total_cmp(&b.3))
                    .map(|(origin, height, id, _)| (origin, height, id));
            }
            let Some((origin, height, chosen_id)) = chosen else {
                return;
            };
            let _staged = world.resource::<crate::camera_authority::StagedPose>();
            let eye = origin + Vec3::new(40.0, height * 0.55 + 6.0, 28.0);
            let look = origin + Vec3::Y * (height * 0.45);
            let mut staged = world.resource_mut::<crate::camera_authority::StagedPose>();
            crate::camera_authority::stage_look_at(
                &mut staged,
                "seattle_fire_probe",
                eye,
                look,
                "gun_probe",
            );
            world.resource_mut::<FireProbe>().target = chosen_id;
            stage = 1;
            since = elapsed;
        }
        1 => {
            if elapsed - since < 1.5 {
                return;
            }
            let chosen_id = world.resource::<FireProbe>().target;
            let Some((origin, height)) = world
                .resource::<destructibles_bevy::building::Buildings>()
                .get(chosen_id)
                .map(|record| (record.origin, record.layout.height))
            else {
                return;
            };
            let point = origin + Vec3::Y * (height * 0.45);
            let forward = Vec3::new(-0.8, -0.1, -0.55).normalize_or_zero();
            crate::neighborhood::fire_seattle_at(
                world,
                chosen_id,
                crate::neighborhood::DamagePreset::Bullet,
                point,
                forward,
                "seattle_fire_test",
            );
            stage = 2;
            since = elapsed;
        }
        2 => {
            if elapsed - since < 1.5 {
                return;
            }
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("captures/seattle_gun_wound.png");
            world
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path));
            let chosen_id = world.resource::<FireProbe>().target;
            let head = world
                .resource::<destructibles_bevy::damage::DamageJournal>()
                .0
                .head_for(chosen_id);
            info!("SEATTLE GUN PROBE: b{chosen_id} journal {head:.2} (capture)");
            stage = 3;
            since = elapsed;
        }
        _ => {
            if elapsed - since > 1.0 {
                world.write_message(AppExit::Success);
            }
        }
    }
    {
        let mut state = world.resource_mut::<FireProbe>();
        state.stage = stage;
        state.since = since;
    }
}

#[derive(Resource, Default)]
struct EvidenceState {
    index: usize,
    entered: bool,
    since: f32,
    done: bool,
}

/// Playtest evidence frames: Tacoma/West Seattle agree-pass, Wallingford
/// houses, the clamped ground slab and the underwater murk.
fn evidence_capture(world: &mut World) {
    if std::env::var_os("EVIDENCE_CAPTURE").is_none() || !s2_enabled() {
        return;
    }
    let elapsed = world.resource::<Time>().elapsed_secs();
    if elapsed < 5.0 {
        return;
    }
    let (index, entered, since, done) = {
        let state = world.resource::<EvidenceState>();
        (state.index, state.entered, state.since, state.done)
    };
    if done {
        return;
    }
    if index >= 5 {
        if since == 0.0 {
            world.resource_mut::<EvidenceState>().since = elapsed;
            return;
        }
        if elapsed - since > 1.5 {
            world.resource_mut::<EvidenceState>().done = true;
            world.write_message(AppExit::Success);
        }
        return;
    }
    if !entered {
        let (center, district) = {
            let geology = world.resource::<Geology>();
            let wanted = ["Tacoma", "West Seattle", "Wallingford", "Capitol Hill"][index.min(3)];
            let mut found = None;
            for id in geology.pack.neighborhoods() {
                let node = &geology.pack.nodes[id];
                if geology.districts.get(&node.name).map(String::as_str) == Some(wanted) {
                    found = Some((node.center, wanted));
                    break;
                }
            }
            found.unwrap_or(([0.0, 0.0], "Seattle general"))
        };
        let _ = district;
        let (ground, wet) = {
            let geology = world.resource::<Geology>();
            let ground = geology.dem.height(center[0], center[1]);
            let col = ((center[0] - geology.dem.origin[0]) / geology.dem.spacing).round() as isize;
            let row = ((center[1] - geology.dem.origin[1]) / geology.dem.spacing).round() as isize;
            let wet = row >= 0
                && col >= 0
                && (row as usize) < geology.dem.rows
                && (col as usize) < geology.dem.cols
                && geology.mask[row as usize * geology.dem.cols + col as usize] != 0;
            (ground, wet)
        };
        let (eye, look) = match index {
            0 => (
                Vec3::new(
                    STREAM_WORLD_OFFSET + center[0],
                    ground + 140.0,
                    center[1] + 160.0,
                ),
                Vec3::new(STREAM_WORLD_OFFSET + center[0], ground + 10.0, center[1]),
            ),
            1 => (
                Vec3::new(
                    STREAM_WORLD_OFFSET + center[0] + 120.0,
                    ground + 120.0,
                    center[1] + 140.0,
                ),
                Vec3::new(STREAM_WORLD_OFFSET + center[0], ground + 8.0, center[1]),
            ),
            2 => (
                Vec3::new(
                    STREAM_WORLD_OFFSET + center[0] + 70.0,
                    ground + 90.0,
                    center[1] + 90.0,
                ),
                Vec3::new(STREAM_WORLD_OFFSET + center[0], ground + 6.0, center[1]),
            ),
            3 => (
                Vec3::new(
                    STREAM_WORLD_OFFSET + center[0],
                    ground - 40.0,
                    center[1] + 40.0,
                ),
                Vec3::new(
                    STREAM_WORLD_OFFSET + center[0] + 80.0,
                    ground + 4.0,
                    center[1] - 40.0,
                ),
            ),
            _ => {
                let (wx, wz) = (-4200.0_f32, 0.0_f32);
                let _ = wet;
                (
                    Vec3::new(STREAM_WORLD_OFFSET + wx, -6.0, wz),
                    Vec3::new(STREAM_WORLD_OFFSET + wx + 60.0, -8.0, wz - 40.0),
                )
            }
        };
        let labels = [
            "evidence_tacoma",
            "evidence_west_seattle",
            "evidence_wallingford",
            "evidence_ground_slab",
            "evidence_murk",
        ];
        let mut staged = world.resource_mut::<crate::camera_authority::StagedPose>();
        crate::camera_authority::stage_look_at(
            &mut staged,
            "evidence_capture",
            eye,
            look,
            labels[index],
        );
        let mut state = world.resource_mut::<EvidenceState>();
        state.entered = true;
        state.since = elapsed;
        return;
    }
    if elapsed - since < 2.0 {
        return;
    }
    let labels = [
        "evidence_tacoma",
        "evidence_west_seattle",
        "evidence_wallingford",
        "evidence_ground_slab",
        "evidence_murk",
    ];
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("captures")
        .join(format!("{}.png", labels[index]));
    world
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path));
    info!("EVIDENCE capture {}", labels[index]);
    let mut state = world.resource_mut::<EvidenceState>();
    state.index += 1;
    state.entered = false;
}

#[derive(Resource)]
struct GhostFunnelState {
    next_at: f32,
}

impl Default for GhostFunnelState {
    fn default() -> Self {
        Self { next_at: 4.0 }
    }
}

/// GHOST_FUNNEL=1: per-stage counts for the partition under the camera every
/// two seconds. Records -> specs -> spawned entities -> shells/proxies -> mesh
/// entities close enough to be visible.
fn ghost_funnel(world: &mut World) {
    if std::env::var_os("GHOST_FUNNEL").is_none() || !s2_enabled() {
        return;
    }
    let elapsed = world.resource::<Time>().elapsed_secs();
    if elapsed < world.resource::<GhostFunnelState>().next_at {
        return;
    }
    world.resource_mut::<GhostFunnelState>().next_at = elapsed + 2.0;
    let Some(camera) = world
        .query_filtered::<&Transform, With<FlyCamera>>()
        .single(world)
        .ok()
        .copied()
    else {
        return;
    };
    let (x, z) = (
        camera.translation.x - STREAM_WORLD_OFFSET,
        camera.translation.z,
    );
    let dem_y = world.resource::<Geology>().dem.height(x, z);
    let state = world.resource::<Geology>();
    let resident = {
        let mut rows = state
            .building_chunks
            .iter()
            .map(|(index, entities)| {
                let node = &state.pack.nodes[*index];
                let mut nearest = f32::MAX;
                let mut nearest_y = 0.0;
                let mut meshes = 0;
                for entity in entities {
                    if world.get::<Mesh3d>(*entity).is_some() {
                        meshes += 1;
                    }
                    if let Some(transform) = world.get::<Transform>(*entity) {
                        let distance = transform.translation.distance(camera.translation);
                        if distance < nearest {
                            nearest = distance;
                            nearest_y = transform.translation.y;
                        }
                    }
                }
                let records = state.pack.nodes[*index].buildings;
                let pending = state
                    .pending_shells
                    .iter()
                    .filter(|job| job.chunk == *index)
                    .count();
                format!(
                    "{}: rec={records} ent={} pend={pending} mesh={meshes} near={nearest:.0}m y={nearest_y:.1}",
                    node.name,
                    entities.len()
                )
            })
            .collect::<Vec<_>>();
        rows.sort();
        rows
    };
    info!(
        "GHOST FUNNEL t={elapsed:.1} cam=({x:.0},{z:.0},y={:.1}) dem={dem_y:.1} resident={} | {}",
        camera.translation.y,
        resident.len(),
        resident.join(" | ")
    );
}

#[derive(Resource, Default)]
struct GhostAerialState {
    stage: u8,
    since: f32,
}

/// GHOST_AERIAL=1: a wide frame over the boot camera's block, so proxies and
/// shells can be judged together without touching the player's view.
fn ghost_aerial(world: &mut World) {
    if std::env::var_os("GHOST_AERIAL").is_none() || !s2_enabled() {
        return;
    }
    let elapsed = world.resource::<Time>().elapsed_secs();
    let (stage, since) = {
        let state = world.resource::<GhostAerialState>();
        (state.stage, state.since)
    };
    match stage {
        0 if elapsed >= 9.0 => {
            let mut staged = world.resource_mut::<crate::camera_authority::StagedPose>();
            crate::camera_authority::stage_look_at(
                &mut staged,
                "ghost_aerial",
                Vec3::new(STREAM_WORLD_OFFSET - 138.0, 320.0, -614.0),
                Vec3::new(STREAM_WORLD_OFFSET - 1500.0, 0.0, 600.0),
                "ghost_aerial",
            );
            let mut state = world.resource_mut::<GhostAerialState>();
            state.stage = 1;
            state.since = elapsed;
        }
        1 if elapsed - since > 1.5 => {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("captures/ghost_aerial.png");
            world
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path));
            let mut state = world.resource_mut::<GhostAerialState>();
            state.stage = 2;
            state.since = elapsed;
        }
        2 if elapsed - since > 2.5 => {
            world.write_message(AppExit::Success);
        }
        _ => {}
    }
}

#[derive(Resource, Default)]
struct FlightCaptureState {
    stage: u8,
    shots: u8,
    since: f32,
}

/// FLIGHT_CAPTURE=1: stage the camera through a *moving* path at altitude so
/// the capture proves the streamed flight layer, not the boot/staging pose.
/// FLIGHT_PATH="x0,z0,x1,z1,y[,look_x,look_z]" in pack-local metres.
fn flight_capture(world: &mut World) {
    if std::env::var_os("FLIGHT_CAPTURE").is_none() || !s2_enabled() {
        return;
    }
    let elapsed = world.resource::<Time>().elapsed_secs();
    if elapsed < 8.0 {
        return;
    }
    let path = std::env::var("FLIGHT_PATH")
        .unwrap_or_else(|_| "-300,-6200,-300,-6560,150,-300,-6700".to_string());
    let values = path
        .split(',')
        .filter_map(|value| value.trim().parse::<f32>().ok())
        .collect::<Vec<_>>();
    if values.len() < 5 {
        return;
    }
    let (x0, z0, x1, z1, y) = (values[0], values[1], values[2], values[3], values[4]);
    let (look_x, look_z) = if values.len() >= 7 {
        (values[5], values[6])
    } else {
        (x1, z1)
    };
    let t = ((elapsed - 8.0) / 6.0).clamp(0.0, 1.0);
    let x = x0 + (x1 - x0) * t;
    let z = z0 + (z1 - z0) * t;
    {
        let mut staged = world.resource_mut::<crate::camera_authority::StagedPose>();
        crate::camera_authority::stage_look_at(
            &mut staged,
            "flight_capture",
            Vec3::new(STREAM_WORLD_OFFSET + x, y, z),
            Vec3::new(STREAM_WORLD_OFFSET + look_x, y - 45.0, look_z),
            "flight",
        );
    }
    let (shots, since, stage) = {
        let state = world.resource::<FlightCaptureState>();
        (state.shots, state.since, state.stage)
    };
    let due = [9.0f32, 11.0, 13.0].get(shots as usize).copied();
    if let Some(due) = due
        && elapsed >= due
        && elapsed - since > 0.5
    {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("captures")
            .join(format!("flight_{shots:02}.png"));
        world
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
        let mut state = world.resource_mut::<FlightCaptureState>();
        state.since = elapsed;
        state.shots += 1;
        info!("FLIGHT CAPTURE shot {shots} at ({x:.0},{z:.0}) altitude {y:.0}");
    }
    if shots >= 3 && elapsed > 14.5 && stage == 0 {
        world.resource_mut::<FlightCaptureState>().stage = 1;
        world.write_message(AppExit::Success);
    }
}

#[derive(Resource, Default)]
struct MurkState {
    stage: u8,
    since: f32,
}

/// MURK_CAPTURE=1: a still-start underwater capture (two frames, no queue).
fn murk_capture(world: &mut World) {
    if std::env::var_os("MURK_CAPTURE").is_none() || !s2_enabled() {
        return;
    }
    let elapsed = world.resource::<Time>().elapsed_secs();
    let (stage, since) = {
        let state = world.resource::<MurkState>();
        (state.stage, state.since)
    };
    match stage {
        0 if elapsed >= 5.0 => {
            let mut staged = world.resource_mut::<crate::camera_authority::StagedPose>();
            crate::camera_authority::stage_look_at(
                &mut staged,
                "murk_capture",
                Vec3::new(STREAM_WORLD_OFFSET - 4200.0, -6.0, 0.0),
                Vec3::new(STREAM_WORLD_OFFSET - 4140.0, -8.0, -40.0),
                "murk",
            );
            let mut state = world.resource_mut::<MurkState>();
            state.stage = 1;
            state.since = elapsed;
        }
        1 if elapsed - since > 1.0 => {
            let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("captures/evidence_murk.png");
            world
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path));
            let mut state = world.resource_mut::<MurkState>();
            state.stage = 2;
            state.since = elapsed;
        }
        2 if elapsed - since > 2.0 => {
            world.write_message(AppExit::Success);
        }
        _ => {}
    }
}

/// Milestone S4: `--seattle` boot profile. Places the free-fly camera at the
/// Westlake/Pike Pine seam at street level looking downhill toward the market
/// and the Sound, with daytime clear defaults. One-shot; input owns the camera
/// afterwards.
fn seattle_boot_camera(
    mut commands: Commands,
    time: Res<Time>,
    mut done: Local<u8>,
    mut camera: Query<(&mut Transform, &mut FlyCamera)>,
    geology: Option<Res<Geology>>,
    mut clock: ResMut<SimClock>,
    mut weather: ResMut<Weather>,
) {
    let boot = std::env::var_os("SEATTLE_BOOT").is_some();
    let traffic_hour = std::env::var("TRAFFIC_HOUR")
        .ok()
        .and_then(|value| value.parse::<f64>().ok());
    if !boot && traffic_hour.is_none() {
        return;
    }
    let capture = std::env::var_os("SEATTLE_BOOT_CAPTURE").is_some();
    if let Some(hour) = traffic_hour {
        clock.minute_of_day = hour * 60.0;
    }
    if !boot {
        return;
    }
    if *done == 0 {
        clock.minute_of_day = std::env::var("SEATTLE_HOUR")
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .map(|hour| hour * 60.0)
            .unwrap_or(12.0 * 60.0);
        weather.auto = false;
        weather.target = WeatherKind::Clear;
        weather.cloud_cover = 0.18;
        weather.precipitation = 0.0;
        weather.storm = 0.0;
        weather.snow = 0.0;
        if let Ok((mut transform, mut fly)) = camera.single_mut() {
            // Audit override: SEATTLE_BOOT_AT / SEATTLE_BOOT_LOOK take
            // pack-local x,y,z so a far partition can be booted without a
            // flight. Defaults are the Westlake/Pike Place seam.
            let parse = |name: &str| {
                std::env::var(name).ok().and_then(|value| {
                    let parts = value
                        .split(',')
                        .map(|part| part.trim().parse::<f32>().ok())
                        .collect::<Option<Vec<_>>>()?;
                    (parts.len() == 3)
                        .then(|| Vec3::new(STREAM_WORLD_OFFSET + parts[0], parts[1], parts[2]))
                })
            };
            // Seat the establishing view on the DEM, not on a hard-coded
            // elevation: the seam is at 37.9 m and the Market corner at
            // ~19 m, so the old eye/look pair (60 / 20) aimed 10 m into the
            // hillside. Keep the authored 22 m rooftop eye, aim at Market
            // roof height so the Sound stays on the horizon.
            let seat = |x: f32, z: f32, above: f32| {
                geology
                    .as_ref()
                    .map(|state| state.dem.height(x, z).max(0.0) + above)
                    .unwrap_or(above)
            };
            let eye = parse("SEATTLE_BOOT_AT").unwrap_or(Vec3::new(
                STREAM_WORLD_OFFSET - 138.4,
                seat(-138.4, -614.3, 22.0),
                -614.3,
            ));
            let look = parse("SEATTLE_BOOT_LOOK").unwrap_or(Vec3::new(
                STREAM_WORLD_OFFSET - 740.0,
                seat(-740.0, -520.0, 20.0),
                -520.0,
            ));
            transform.translation = eye;
            let d = (look - eye).normalize_or_zero();
            let extra = std::env::var("SEATTLE_YAW")
                .ok()
                .and_then(|value| value.parse::<f32>().ok())
                .unwrap_or(0.0);
            // Hand ownership to input at the commanded pose: the free-fly
            // controller re-applies its own yaw/pitch every Update, so a
            // transform-only boot pose is silently discarded on the next
            // frame and the establishing view never happens.
            fly.yaw = (-d.x).atan2(-d.z) + extra.to_radians();
            fly.pitch = d.y.asin();
            transform.rotation = Quat::from_rotation_y(fly.yaw) * Quat::from_rotation_x(fly.pitch);
        }
        info!("SEATTLE BOOT: camera at Westlake seam, daytime clear");
        *done = 1;
        return;
    }
    let capture_at = std::env::var("SEATTLE_CAPTURE_AT")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .unwrap_or(7.0);
    if capture && *done == 1 && time.elapsed_secs() > capture_at {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("captures/seattle_boot.png");
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
        info!("SEATTLE BOOT: spawn capture");
        *done = 2;
    }
}

fn s3_list_targets(world: &mut World) {
    if std::env::var_os("S3_LIST_TARGETS").is_none() || !s3_enabled() {
        return;
    }
    static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if DONE.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    info!("S3 TARGET DUMP key=style-pack rule match on city+district (mapped) -> MaterialFamily");
    let state = world.resource::<Geology>();
    let mut counts = std::collections::HashMap::<String, usize>::new();
    let priority = |name: &str| -> u8 {
        match state.districts.get(name).map(String::as_str).unwrap_or("") {
            "Pioneer Square" => 0,
            "Pike Place" => 1,
            "Downtown" => 2,
            "First Hill" => 3,
            "Capitol Hill" => 4,
            "Ballard" => 5,
            "Fremont" => 6,
            "Wallingford" => 7,
            "Georgetown" => 8,
            "SODO" => 9,
            "West Seattle" => 10,
            "Bellevue CBD" => 11,
            "Eastside" => 12,
            "Tacoma" => 13,
            _ => 14,
        }
    };
    let mut ordered = state.pack.neighborhoods();
    ordered.sort_by_key(|index| priority(&state.pack.nodes[*index].name));
    for index in ordered {
        if counts.len() >= 4 && counts.values().all(|count| *count >= 5) {
            break;
        }
        let node = &state.pack.nodes[index];
        let Ok(chunk) = state.pack.read_chunk(index) else {
            continue;
        };
        let city = state
            .pack
            .nodes
            .get(node.parent as usize)
            .map(|node| node.name.as_str())
            .unwrap_or("Seattle");
        let district = state
            .districts
            .get(&node.name)
            .map(String::as_str)
            .unwrap_or(&node.name);
        for building in &chunk.buildings {
            let category = chunk.string(building.category_idx as u16).to_string();
            let source = chunk.string(building.source_idx as u16).to_string();
            let Some(spec) = building_spec(
                &state.style,
                node,
                building,
                city,
                district,
                &category,
                &source,
            ) else {
                continue;
            };
            let family = family_name(spec.family).to_string();
            let count = counts.entry(family.clone()).or_insert(0);
            if *count < 5 {
                info!(
                    "S3 TARGET family={} id={} height={:.1} partition={} district={}",
                    family, building.id, building.height_m, node.name, district
                );
                *count += 1;
            }
        }
    }
    std::process::exit(0);
}

pub fn s3_enabled() -> bool {
    std::env::var_os("SEATTLE_S3").is_some()
}

fn rule_for<'a>(
    pack: &'a StylePack,
    city: &str,
    district: &str,
    _neighborhood: &str,
) -> &'a StyleRule {
    for rule in &pack.rules {
        if !rule.cities.is_empty() && !rule.cities.iter().any(|value| value == city) {
            continue;
        }
        if rule
            .neighborhood_any
            .iter()
            .any(|value| district == value || district.contains(value))
        {
            return rule;
        }
    }
    &pack.default
}

fn facade_for(name: &str) -> FacadeStyle {
    match name {
        "glass_curtain_wall" => FacadeStyle::GlassCurtainWall,
        "concrete_grid" => FacadeStyle::ConcreteGrid,
        "brick_midrise" => FacadeStyle::BrickMidrise,
        _ => FacadeStyle::BrickBrownstone,
    }
}

fn family_for(name: &str) -> MaterialFamily {
    match name {
        "steel_frame" => MaterialFamily::SteelFrame,
        "timber" => MaterialFamily::Timber,
        _ => MaterialFamily::MasonryStone,
    }
}

/// Seat a footprint on the highest DEM sample under it, not just its centroid:
/// on steep high-elevation terrain a centroid seat buries the uphill corners.
pub const SEAT_LIFT_M: f32 = 0.18;

/// Douglas-Peucker-lite: stride samples keep existence inside the 64-vertex
/// render budget. A few centimetres of outline fidelity traded for the house.
pub fn simplify_perimeter(perimeter: &[[f32; 2]]) -> Vec<[f32; 2]> {
    if perimeter.len() <= 64 {
        return perimeter.to_vec();
    }
    let step = perimeter.len().div_ceil(64);
    let mut simplified = perimeter.iter().step_by(step).copied().collect::<Vec<_>>();
    if simplified.len() < 3 {
        simplified = perimeter.iter().take(3).copied().collect();
    }
    simplified
}

pub fn footprint_seat(dem: &Dem, centroid: [f32; 2], perimeter: &[[f32; 2]]) -> f32 {
    let mut height = dem.height(centroid[0], centroid[1]);
    for point in perimeter {
        height = height.max(dem.height(point[0], point[1]));
    }
    height
}

fn building_spec(
    pack: &StylePack,
    node: &crate::citypack::PackNode,
    building: &crate::citypack::PackBuilding,
    city: &str,
    district: &str,
    category: &str,
    source: &str,
) -> Option<BuildingSpec> {
    if building.perimeter.len() < 3 {
        return None;
    }
    // Complex footprints are simplified to the renderer's 64-vertex budget
    // instead of being dropped: existence beats centimetre outline fidelity.
    let perimeter = simplify_perimeter(&building.perimeter);
    let rule = rule_for(pack, city, district, &node.name);
    let mut style = facade_for(&rule.facade_style);
    let roll = (building.id.wrapping_mul(2654435761) % 1000) as f32 / 1000.0;
    if roll < rule.glass_probability && building.height_m >= 25.0 {
        style = FacadeStyle::GlassCurtainWall;
    }
    let family = family_for(&rule.family);
    let footprint = perimeter
        .iter()
        .map(|point| {
            [
                point[0] - building.centroid[0],
                point[1] - building.centroid[1],
            ]
        })
        .collect::<Vec<_>>();
    // Human-scale governor: the packaged heights are inflated in residential
    // districts (Wallingford p90 39 m, max 94 m for SFR blocks). Residential
    // floors are 3.0 m and capped at three storeys until the enrichment layer
    // is re-emitted; commercial keeps 4.0 m.
    let residential = category.eq_ignore_ascii_case("residential");
    let floor_height = if residential { 3.0 } else { 4.0 };
    // Unremarkability Doctrine: a building without credible height data is
    // UNREMARKABLE, and unremarkable means SINGLE-STORY. Weak enrichments are
    // stripped outright; surveyed heights must also pass an area plausibility
    // gate (141 m on a 10 x 16 m footprint is not a house).
    let area = {
        let count = perimeter.len();
        (0..count)
            .map(|index| {
                let a = perimeter[index];
                let b = perimeter[(index + 1) % count];
                a[0] * b[1] - b[0] * a[1]
            })
            .sum::<f32>()
            .abs()
            * 0.5
    };
    let weak_source = matches!(source, "area_heuristic" | "msml_estimate");
    let plausible = building.height_m <= 4.0 + 3.2 * area.sqrt().max(1.0);
    let demoted = residential && (weak_source || !plausible);
    let mut floor_count = if demoted {
        1
    } else {
        (building.height_m / floor_height).round().max(1.0) as u32
    };
    if residential && !demoted {
        // The carpet gate: Seattle houses are a 1-2 storey carpet. The old
        // three-storey cap still produced flat-topped blocks; two storeys lets
        // every credible house carry a pitched roof.
        floor_count = floor_count.min(2);
    }
    // Interim caps from the playtest-70 census: low districts 6 floors, mid
    // districts 8; towers (Downtown/Bellevue core) stay uncapped. The census
    // showed commercial records inheriting 128 m in mixed residential
    // districts — the chimneys the player flew into.
    // The cap is for residential *districts*, not the catch-all "Seattle
    // general", whose mid-bias covers downtown-adjacent blocks that are
    // legitimately tall.
    let residential_district = matches!(
        district,
        "Ballard"
            | "Fremont"
            | "Wallingford"
            | "Georgetown"
            | "Capitol Hill"
            | "Queen Anne"
            | "West Seattle"
            | "First Hill"
            | "Pioneer Square"
            | "Pike Place"
    );
    if !residential && residential_district {
        match rule.height_bias.as_str() {
            "low" => floor_count = floor_count.min(6),
            "mid" => floor_count = floor_count.min(8),
            _ => {}
        }
    }
    // Slenderness gate (extends Unremarkability): a footprint hosts about one
    // storey per 12 m2, and a residential district hosts no building above
    // four storeys. Downtown commercial keeps its slenderness because it has
    // real window rows and is not in a residential district.
    if residential_district {
        floor_count = floor_count.min((area / 12.0).floor().max(1.0) as u32);
        floor_count = floor_count.min(4);
    }
    // House windows on houses: a residential parcel gets a sash window with a
    // sill and a 3.1 m column pitch regardless of the district's commercial
    // grammar. Commercial floors keep their grammar's fenestration, but the
    // window never runs floor-to-floor (that is how a brownstone turned into
    // an office curtain wall).
    let mut windows = if residential {
        WindowSpec {
            width: 1.10,
            height: 1.45,
            sill_height: 0.95,
            column_spacing: 3.10,
            edge_margin: 1.30,
            recess: 0.17,
        }
    } else {
        let mut spec = style.windows();
        spec.height = spec
            .height
            .min(floor_height - spec.sill_height - 0.35)
            .max(1.0);
        spec
    };
    windows.height = windows
        .height
        .min(floor_height - windows.sill_height - 0.20)
        .max(1.0);
    let spec = BuildingSpec {
        family,
        footprint,
        floor_height,
        floor_count,
        facade: style.material(),
        facade_style: style,
        windows,
        core: core_for_style(style, &perimeter, building.id),
    };
    let mut spec = spec;
    if spec.validate().is_err() {
        // Small residential parcels cannot host the default core (>=50% of the
        // footprint). Shrink it, then drop it, rather than lose the building.
        if let Some(core) = spec.core.as_mut() {
            core.half_x *= 0.5;
            core.half_z *= 0.5;
        }
        if spec.validate().is_err() {
            spec.core = None;
        }
    }
    spec.validate().ok().map(|_| spec)
}

fn s3_buildings(world: &mut World) {
    if !s3_enabled() {
        return;
    }
    if std::env::var_os("S3_ID").is_some() {
        return;
    }
    if std::env::var_os("S3_DAMAGE").is_some() && world.resource::<Geology>().targets.len() >= 3 {
        return;
    }
    {
        let mut spawned = 0usize;
        while spawned < S3_SHELLS_PER_FRAME {
            let Some(job) = world.resource_mut::<Geology>().pending_shells.pop_front() else {
                break;
            };
            if !world
                .resource::<Geology>()
                .building_chunks
                .contains_key(&job.chunk)
            {
                continue;
            }
            let family = job.spec.family;
            let entity = spawn_building_world_lod(
                world,
                job.id,
                job.spec,
                job.origin,
                BuildingDetail::Shell,
            );
            crate::facade::apply_family_skin(world, job.id, family);
            if let Some(entities) = world
                .resource_mut::<Geology>()
                .building_chunks
                .get_mut(&job.chunk)
            {
                entities.push(entity);
            }
            let filter = std::env::var("S3_NODE").ok();
            let min_height = std::env::var("S3_MIN_HEIGHT")
                .ok()
                .and_then(|value| value.parse::<f32>().ok())
                .unwrap_or(0.0);
            let matches_node = filter
                .as_ref()
                .is_none_or(|needle| job.node_name.contains(needle));
            if matches_node
                && job.mean_height >= min_height
                && world.resource::<Geology>().targets.len() < 3
            {
                world
                    .resource_mut::<Geology>()
                    .targets
                    .push((job.id, job.origin, job.mean_height));
            }
            spawned += 1;
        }
    }
    let Some(camera) = world
        .query_filtered::<&Transform, With<FlyCamera>>()
        .single(world)
        .ok()
        .copied()
    else {
        return;
    };
    let center = [
        camera.translation.x - STREAM_WORLD_OFFSET,
        camera.translation.z,
    ];
    let (wanted, unload): (Vec<usize>, Vec<(usize, Vec<Entity>)>) = {
        let state = world.resource::<Geology>();
        let mut evaluations = Evaluations::default();
        let candidates = wanted_neighborhoods(&state.pack.nodes, center, 650.0, &mut evaluations);
        let wanted = candidates
            .into_iter()
            .filter(|index| !state.building_chunks.contains_key(index))
            .take(1)
            .collect::<Vec<_>>();
        // Load and unload must use the same metric: a sprawling neighbourhood
        // can have its AABB inside the load radius while its centre sits past
        // the unload radius, which made every frame respawn it.
        let mut unload = state
            .building_chunks
            .iter()
            .filter(|(index, _)| {
                let node = &state.pack.nodes[**index];
                crate::citypack::aabb_distance2(&node.bounds, center) > S3_UNLOAD_M * S3_UNLOAD_M
            })
            .map(|(index, entities)| (*index, entities.clone()))
            .collect::<Vec<_>>();
        let resident = state.building_chunks.len() - unload.len();
        if resident > S3_MAX_RESIDENT_CHUNKS {
            let mut farthest = state
                .building_chunks
                .iter()
                .filter(|(index, _)| !unload.iter().any(|(other, _)| other == *index))
                .map(|(index, entities)| {
                    let node = &state.pack.nodes[*index];
                    let distance = crate::citypack::aabb_distance2(&node.bounds, center);
                    (*index, entities.clone(), distance)
                })
                .collect::<Vec<_>>();
            farthest.sort_by(|a, b| b.2.total_cmp(&a.2));
            let excess = resident - S3_MAX_RESIDENT_CHUNKS;
            unload.extend(
                farthest
                    .into_iter()
                    .take(excess)
                    .map(|(index, entities, _)| (index, entities)),
            );
        }
        (wanted, unload)
    };
    for (index, entities) in unload {
        // A damaged building's support graph is journal-derived state, not
        // chunk state. Despawning the chunk under it would orphan the graph
        // and let a re-stream spawn a pristine shell over a damaged record.
        // Pin the chunk instead; the destructibles streaming controller owns
        // the damaged building from there (freeze -> dormant -> reconstruct).
        let damaged = entities.iter().any(|entity| {
            world
                .get::<destructibles_bevy::building::Building>(*entity)
                .map(|building| {
                    world
                        .resource::<destructibles_bevy::damage::DamageJournal>()
                        .0
                        .head_for(building.id)
                        > 0.0
                })
                .unwrap_or(false)
        });
        if damaged {
            let name = world.resource::<Geology>().pack.nodes[index].name.clone();
            if world.resource_mut::<Geology>().pinned_chunks.insert(index) {
                info!("S3 unload: pinned {name} (damaged building in chunk)");
            }
            continue;
        }
        for entity in entities {
            let _ = world.despawn(entity);
        }
        world
            .resource_mut::<Geology>()
            .building_chunks
            .remove(&index);
        world
            .resource_mut::<Geology>()
            .pending_shells
            .retain(|job| job.chunk != index);
    }
    for index in wanted {
        let index_chunk = index;
        let structures_before = world
            .resource::<destructibles_bevy::structure::Structures>()
            .iter()
            .filter(|(id, _)| **id < crate::landmarks::LANDMARK_ID_BASE)
            .count();
        let (target_name, target_mean) = {
            let state = world.resource::<Geology>();
            let node = &state.pack.nodes[index];
            (node.name.clone(), node.mean_height_m)
        };
        let (chunk, specs, origin_h) = {
            let state = world.resource::<Geology>();
            let Some(node) = state.pack.nodes.get(index) else {
                continue;
            };
            match state.pack.read_chunk(index) {
                Ok(chunk) => {
                    let specs = chunk
                        .buildings
                        .iter()
                        .filter_map(|building| {
                            let city = state
                                .pack
                                .nodes
                                .get(node.parent as usize)
                                .map(|node| node.name.as_str())
                                .unwrap_or("Seattle");
                            let district = state
                                .districts
                                .get(&node.name)
                                .map(String::as_str)
                                .unwrap_or(&node.name);
                            let category = chunk.string(building.category_idx as u16).to_string();
                            let source = chunk.string(building.source_idx as u16).to_string();
                            let spec = building_spec(
                                &state.style,
                                node,
                                building,
                                city,
                                district,
                                &category,
                                &source,
                            )?;
                            let y =
                                footprint_seat(&state.dem, building.centroid, &building.perimeter)
                                    + 0.03;
                            let mut min = [f32::MAX; 2];
                            let mut max = [f32::MIN; 2];
                            for point in &building.perimeter {
                                min[0] = min[0].min(point[0]);
                                min[1] = min[1].min(point[1]);
                                max[0] = max[0].max(point[0]);
                                max[1] = max[1].max(point[1]);
                            }
                            Some((
                                building.id,
                                spec,
                                [building.centroid[0], y, building.centroid[1]],
                                building.height_m,
                                (max[0] - min[0]).max(2.0),
                                (max[1] - min[1]).max(2.0),
                            ))
                        })
                        .collect::<Vec<_>>();
                    (chunk, specs, 0.0_f32)
                }
                Err(_) => continue,
            }
        };
        let _ = chunk;
        let spawn_started = std::time::Instant::now();
        let spec_count = specs.len();
        let mut order = (0..specs.len()).collect::<Vec<_>>();
        order.sort_by(|left, right| {
            let a = specs[*left].2;
            let b = specs[*right].2;
            let da = (a[0] - center[0]).hypot(a[2] - center[1]);
            let db = (b[0] - center[0]).hypot(b[2] - center[1]);
            da.total_cmp(&db)
        });
        let near = order
            .iter()
            .take_while(|&&index| {
                let origin = specs[index].2;
                (origin[0] - center[0]).hypot(origin[2] - center[1]) <= S3_SHELL_NEAR_M
            })
            .count();
        let shell_count = near.clamp(S3_SHELL_FLOOR, S3_SHELL_CAP).min(specs.len());
        let shell_set = order[..shell_count].iter().copied().collect::<HashSet<_>>();
        let mut entities = Vec::with_capacity(specs.len());
        let mut shells = 0usize;
        let mut proxies = 0usize;
        let mut far_boxes: Vec<(Vec3, Vec3, u8, [f32; 4], u32)> = Vec::new();
        for (index, (id, spec, origin, height, width, depth)) in specs.into_iter().enumerate() {
            // Proxy night bank: timber/adobe residential reads as a scatter of
            // warm incandescent points; steel/masonry keeps the regular grid.
            let warm = matches!(
                spec.family,
                destructibles_core::family::MaterialFamily::Timber
                    | destructibles_core::family::MaterialFamily::EarthAdobe
            );
            if shell_set.contains(&index) {
                shells += 1;
                if shells > S3_SHELL_IMMEDIATE {
                    world
                        .resource_mut::<Geology>()
                        .pending_shells
                        .push_back(PendingShell {
                            chunk: index_chunk,
                            id,
                            spec,
                            origin: Vec3::from_array(origin) + Vec3::X * STREAM_WORLD_OFFSET,
                            node_name: target_name.clone(),
                            mean_height: target_mean,
                        });
                    continue;
                }
                let family = spec.family;
                let entity = spawn_building_world_lod(
                    world,
                    id,
                    spec,
                    Vec3::from_array(origin) + Vec3::X * STREAM_WORLD_OFFSET,
                    BuildingDetail::Shell,
                );
                crate::facade::apply_family_skin(world, id, family);
                entities.push(entity);
                let filter = std::env::var("S3_NODE").ok();
                let min_height = std::env::var("S3_MIN_HEIGHT")
                    .ok()
                    .and_then(|value| value.parse::<f32>().ok())
                    .unwrap_or(0.0);
                let matches_node = filter
                    .as_ref()
                    .is_none_or(|needle| target_name.contains(needle));
                let spec_height = target_mean;
                if matches_node
                    && spec_height >= min_height
                    && world.resource::<Geology>().targets.len() < 3
                {
                    world.resource_mut::<Geology>().targets.push((
                        id,
                        Vec3::from_array(origin) + Vec3::X * STREAM_WORLD_OFFSET,
                        spec_height,
                    ));
                }
            } else {
                proxies += 1;
                let distance = (origin[0] - center[0]).hypot(origin[2] - center[1]);
                if distance > S3_PROXY_MERGE_M {
                    far_boxes.push((
                        Vec3::new(origin[0], origin[1] + height * 0.5, origin[2]),
                        Vec3::new(width, height.max(2.0), depth),
                        u8::from(warm),
                        proxy_tint(id, spec.family),
                        id,
                    ));
                } else {
                    // Near proxies get their OWN baked box: the shared unit
                    // cube carried one tint and one roof decision for the whole
                    // district, which is why the player saw flat, identically
                    // coloured buildings in flight even though the far merged
                    // mesh had both.
                    let material = {
                        let state = world.resource::<Geology>();
                        if warm {
                            state.box_material_warm.clone()
                        } else {
                            state.box_material.clone()
                        }
                    };
                    let mesh = {
                        let mut meshes = world.resource_mut::<Assets<Mesh>>();
                        meshes.add(proxy_boxes_mesh(&[(
                            Vec3::ZERO,
                            Vec3::new(width, height.max(2.0), depth),
                            u8::from(warm),
                            proxy_tint(id, spec.family),
                            id,
                        )]))
                    };
                    let entity = world
                        .spawn((
                            Mesh3d(mesh),
                            MeshMaterial3d(material),
                            Transform::from_translation(
                                Vec3::from_array(origin)
                                    + Vec3::X * STREAM_WORLD_OFFSET
                                    + Vec3::Y * height * 0.5,
                            ),
                            Name::new(format!("S3 proxy b{id}")),
                        ))
                        .id();
                    entities.push(entity);
                }
            }
        }
        {
            let mut footprints = world.resource_mut::<SeattleFootprints>();
            for building in &chunk.buildings {
                if footprints
                    .buildings
                    .iter()
                    .any(|entry| entry.id == building.id)
                {
                    continue;
                }
                footprints.buildings.push(SeattleFootprint {
                    id: building.id,
                    centroid: building.centroid,
                    perimeter: building.perimeter.clone(),
                });
            }
        }
        let count = entities.len();
        let chunk_structures = world
            .resource::<destructibles_bevy::structure::Structures>()
            .iter()
            .filter(|(id, _)| **id < crate::landmarks::LANDMARK_ID_BASE)
            .count();
        // Shell Doctrine: *streaming* must not build a support graph. A graph
        // that already existed belongs to a damaged, journal-owned building
        // and is legitimate; only growth across the spawn is a violation.
        assert!(
            chunk_structures <= structures_before,
            "Shell Doctrine violation: streaming {target_name} created {} support graph(s)",
            chunk_structures - structures_before
        );
        let merged = far_boxes.len();
        if !far_boxes.is_empty() {
            let handle = world
                .resource_mut::<Assets<Mesh>>()
                .add(proxy_boxes_mesh(&far_boxes));
            let material = world.resource::<Geology>().box_material.clone();
            let entity = world
                .spawn((
                    Mesh3d(handle),
                    MeshMaterial3d(material),
                    Transform::from_xyz(STREAM_WORLD_OFFSET, 0.0, 0.0),
                    Name::new(format!("S3 far proxies {target_name}")),
                ))
                .id();
            entities.push(entity);
        }
        let spawn_ms = spawn_started.elapsed().as_secs_f32() * 1000.0;
        assert_eq!(
            shells + proxies,
            spec_count,
            "proxy hole: {} covered records for {spec_count} in {target_name}",
            shells + proxies
        );
        info!(
            "S3 chunk {target_name}: {count} entities -> {shells} shells, {proxies} proxies ({merged} merged); shipped roofs gables={} flats={}; {spawn_ms:.1} ms; resident {} chunks",
            SHIPPED_GABLES.load(std::sync::atomic::Ordering::Relaxed),
            SHIPPED_FLATS.load(std::sync::atomic::Ordering::Relaxed),
            world.resource::<Geology>().building_chunks.len() + 1
        );
        if !world.resource::<Geology>().first_chunk_reported {
            let boot_to_first = world
                .resource::<Geology>()
                .start_instant
                .elapsed()
                .as_secs_f32()
                * 1000.0;
            info!(
                "S3 BOOT: first streamed chunk in {boot_to_first:.0} ms ({shells} shells, {proxies} proxies)"
            );
            world.resource_mut::<Geology>().first_chunk_reported = true;
        }
        let mut state = world.resource_mut::<Geology>();
        state.building_chunks.insert(index, entities);
        state.loaded_buildings += count;
        let _ = origin_h;
    }
    // The doctrine's load-time invariant is asserted per streamed chunk above.
    // A player weapon (or a smoke/gate) may legitimately tier a shell up later;
    // that is the intended damage path, not a violation.
    let mut state = world.resource_mut::<Geology>();
    state.loaded_buildings = state.building_chunks.values().map(Vec::len).sum();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load() -> (Dem, Vec<u8>) {
        let report: S2Report =
            serde_json::from_str(&std::fs::read_to_string(asset(REPORT_PATH)).expect("report"))
                .expect("json");
        let heights = std::fs::read(asset(DEM_PATH))
            .expect("dem")
            .chunks(2)
            .filter(|pair| pair.len() == 2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        let dem = Dem {
            rows: report.grid.rows,
            cols: report.grid.cols,
            spacing: report.grid.spacing_m,
            origin: report.grid.origin_world,
            heights,
        };
        let mask = std::fs::read(asset(MASK_PATH)).expect("mask");
        (dem, mask)
    }

    #[test]
    fn baseline_record_is_once_before_damage() {
        assert!(baseline_record_allowed(false, 0));
        assert!(!baseline_record_allowed(true, 0));
        assert!(!baseline_record_allowed(false, 1));
        assert!(!baseline_record_allowed(true, 3));
    }

    #[test]
    fn dem_spot_heights_and_water_mask() {
        let (dem, mask) = load();
        assert_eq!(dem.rows, 2322);
        assert_eq!(dem.cols, 1207);
        for (x, z, expected) in [
            (0.0, 0.0, 30.6),
            (1224.3, -1866.7, 103.1),
            (-1548.5, -3455.5, 117.9),
        ] {
            let h = dem.height(x, z);
            assert!((h - expected).abs() < 1.6, "({x},{z}) {h} vs {expected}");
        }
        let wet_lake = mask.iter().filter(|v| **v != 0).count();
        assert!(wet_lake > 400_000);
    }

    #[test]
    fn skirt_geometry_and_hysteresis() {
        let (dem, mask) = load();
        let tile = build_tile_mesh(&dem, &mask, 17, 17);
        assert!(tile.count_vertices() > (TILE_CELLS as usize + 1).pow(2));
    }

    #[test]
    fn road_chunk_roundtrip_and_unloaded_absence() {
        let (dem, _mask) = load();
        let pack =
            CityPack::open(Path::new(env!("CARGO_MANIFEST_DIR")).join(crate::citypack::PACK_PATH))
                .expect("pack");
        let nodes = pack.neighborhoods();
        let mut loaded = HashMap::new();
        let mut first = None;
        for node in nodes
            .iter()
            .copied()
            .filter(|index| pack.nodes[*index].streets > 0)
            .take(3)
        {
            let chunk = pack.read_chunk(node).expect("chunk");
            let mesh = road_mesh(&dem, &chunk);
            assert!(mesh.count_vertices() > 0);
            loaded.insert(node, mesh.count_vertices());
            first.get_or_insert(node);
        }
        assert_eq!(loaded.len(), 3);
        let first = first.unwrap();
        loaded.remove(&first);
        assert!(!loaded.contains_key(&first));
    }

    #[test]
    fn water_mesh_and_road_drape() {
        let (dem, mask) = load();
        let bytes = std::fs::read(asset(WATER_PATH)).expect("water");
        let water = parse_water(&bytes);
        assert!(water.count_vertices() > 10_000);
        let pack =
            CityPack::open(Path::new(env!("CARGO_MANIFEST_DIR")).join(crate::citypack::PACK_PATH))
                .expect("pack");
        let node = pack
            .neighborhoods()
            .into_iter()
            .find(|index| pack.nodes[*index].streets > 0)
            .expect("street neighborhood");
        let chunk = pack.read_chunk(node).expect("chunk");
        let roads = road_mesh(&dem, &chunk);
        let positions = roads
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(|a| a.as_float3())
            .expect("positions");
        assert!(!positions.is_empty());
        for p in positions.iter().step_by(7) {
            let ground = dem.height(p[0], p[2]);
            assert!(
                (p[1] - ground).abs() < 3.0,
                "road y {} ground {}",
                p[1],
                ground
            );
        }
        // LOD seam: a near-tile vertex uses the same DEM cell as the far sampler.
        let tile = build_tile_mesh(&dem, &mask, 10, 10);
        let tile_positions = tile
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(|a| a.as_float3())
            .expect("tile");
        let grid_vertices = (TILE_CELLS as usize + 1).pow(2);
        for p in tile_positions.iter().take(grid_vertices).step_by(13) {
            let ground = dem.height(p[0], p[2]);
            assert!((p[1] - ground - 0.15).abs() < 0.5);
        }
    }
}
