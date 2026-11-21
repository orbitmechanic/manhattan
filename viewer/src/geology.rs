//! Seattle S2: near-field DEM terrain, water surfaces and draped streets.
//!
//! No buildings are spawned in this mode (`--seattle-s2`). The 30 m DEM is a
//! local asset; a 120 m far mesh covers the region, 960 m near tiles stream at
//! full resolution with skirts, water is a triangulated surface at dataset
//! datums, and street ribbons drape the DEM per loaded neighborhood.

use std::collections::HashSet;
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

pub const DEM_PATH: &str = "assets/terrain/seattle_dem_30m.bin";
pub const MASK_PATH: &str = "assets/terrain/seattle_water_mask.bin";
pub const WATER_PATH: &str = "assets/terrain/seattle_water.bin";
pub const REPORT_PATH: &str = "datasets/seattle/s2_report.json";

const TILE_CELLS: i32 = 32;
const ROAD_RADIUS_M: f32 = 5200.0;
const FAR_STEP: i32 = 2;

#[derive(Deserialize)]
struct S2Report {
    grid: GridReport,
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
        app.add_systems(Startup, setup_geology)
            .add_systems(Update, (geology_stream, s2_capture).chain())
            .add_systems(PostUpdate, hide_precip);
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

#[derive(Resource)]
pub struct Geology {
    pub dem: Dem,
    pub mask: Vec<u8>,
    pub pack: CityPack,
    pub terrain_material: Handle<StandardMaterial>,
    pub water_material: Handle<StandardMaterial>,
    pub road_material: Handle<StandardMaterial>,
    pub loaded_roads: usize,
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
    let road_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.17, 0.17, 0.18),
        perceptual_roughness: 0.9,
        cull_mode: None,
        ..default()
    });
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
    let mut global_positions = Vec::new();
    let mut global_normals = Vec::new();
    let mut global_colors = Vec::new();
    let mut global_indices = Vec::new();
    for index in pack.neighborhoods() {
        if pack.nodes[index].streets == 0 {
            continue;
        }
        if let Ok(chunk) = pack.read_chunk(index) {
            let mesh = road_mesh(&dem, &chunk);
            if let Some(positions) = mesh
                .attribute(Mesh::ATTRIBUTE_POSITION)
                .and_then(|a| a.as_float3())
            {
                let base = global_positions.len() as u32;
                for p in positions {
                    global_positions.push(*p);
                    global_normals.push([0.0, 1.0, 0.0]);
                    global_colors.push([0.18, 0.18, 0.19, 1.0]);
                }
                if let Some(bevy::render::mesh::Indices::U32(values)) = mesh.indices() {
                    global_indices.extend(values.iter().map(|i| base + i));
                }
            }
        }
    }
    info!("S2: global roads {} verts", global_positions.len());
    commands.spawn((
        Mesh3d(meshes.add(mesh(
            global_positions,
            global_normals,
            global_colors,
            global_indices,
        ))),
        MeshMaterial3d(road_material.clone()),
        Transform::from_xyz(STREAM_WORLD_OFFSET, 0.0, 0.0),
        Name::new("S2 global roads"),
    ));
    let pose_centers = [
        (900.0, -3300.0),
        (-5532.0, 245.0),
        (-1548.0, -3455.0),
        (6080.0, -1419.0),
        (335.0, 4032.0),
        (-1200.0, -2600.0),
        (2455.0, -4490.0),
        (-482.0, 7352.0),
    ];
    let tile_size = TILE_CELLS as f32 * dem.spacing;
    let mut patches = HashSet::new();
    for (cx, cz) in pose_centers {
        let tx = ((cx - dem.origin[0]) / tile_size).floor() as i32;
        let tz = ((cz - dem.origin[1]) / tile_size).floor() as i32;
        for dx in -1..=1 {
            for dz in -1..=1 {
                patches.insert((tx + dx, tz + dz));
            }
        }
    }
    for (tx, tz) in &patches {
        let tile = build_tile_mesh(&dem, &mask, *tx, *tz);
        commands.spawn((
            Mesh3d(meshes.add(tile)),
            MeshMaterial3d(terrain_material.clone()),
            Transform::from_xyz(STREAM_WORLD_OFFSET, 0.0, 0.0),
            Name::new(format!("S2 near terrain {tx},{tz}")),
        ));
    }
    info!("S2: {} near terrain patches", patches.len());
    let boot_ms = started.elapsed().as_secs_f32() * 1000.0;
    info!(
        "S2: DEM {}x{} at {} m, water and far terrain in {:.1} ms",
        dem.rows, dem.cols, dem.spacing, boot_ms
    );
    commands.insert_resource(Geology {
        dem,
        mask,
        pack,
        terrain_material,
        water_material,
        road_material,
        loaded_roads: 0,
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
    mut state: Option<ResMut<Geology>>,
    time: Res<Time>,
    camera: Query<&Transform, With<FlyCamera>>,
    mut text: Query<&mut Text, With<S2Text>>,
    mut precip: Query<&mut Visibility, With<crate::weather::PrecipVolume>>,
    mut motion: Option<ResMut<crate::motion::MotionState>>,
    mut ticks: Local<u32>,
    mut frame: Local<(f32, u32)>,
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
    let mut evaluations = Evaluations::default();
    let wanted = wanted_neighborhoods(&state.pack.nodes, center, ROAD_RADIUS_M, &mut evaluations);
    state.evaluations = evaluations;
    state.loaded_roads = wanted.len();
    frame.0 += time.delta_secs() * 1000.0;
    frame.1 += 1;
    if frame.1 >= 60 {
        info!("S2 frame avg {:.2} ms", frame.0 / frame.1 as f32);
        frame.0 = 0.0;
        frame.1 = 0;
    }
    *ticks += 1;
    if (*ticks).is_multiple_of(90) {
        info!(
            "S2: road neighborhoods {} evals {}/{}/{}",
            state.loaded_roads, evaluations.counties, evaluations.cities, evaluations.neighborhoods
        );
    }
    if let Ok(mut text) = text.single_mut() {
        text.0 = format!(
            "S2  geology boot {:.0} ms  road nbhds {}  evals {}/{}/{}\n60 m regional + 30 m pose patches + global draped roads",
            state.boot_ms,
            state.loaded_roads,
            evaluations.counties,
            evaluations.cities,
            evaluations.neighborhoods
        );
    }
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
    if *active != index {
        *active = index;
        *entered = elapsed;
    }
    if elapsed - *entered < 3.0 {
        return;
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
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("captures")
            .join(pose.name);
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
        state.captures += 1;
    }
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
        for p in tile_positions.iter().step_by(13) {
            let ground = dem.height(p[0], p[2]);
            assert!((p[1] - ground - 0.15).abs() < 0.5);
        }
    }
}
