//! Seattle trees: per-neighborhood canopy streaming.
//!
//! The dataset is built offline by `scripts/trees_build.py` from the SDOT street
//! tree inventory, Seattle parks polygons and ESA WorldCover canopy, keyed by
//! CityPack neighborhood index. This module only reads that file and streams
//! instanced trees with the same load/unload hysteresis as buildings, plus a
//! three-tier LOD ladder (near cards + trunk, mid cards, far single card).
//!
//! Trees are static props: no structure graphs, no damage journals.

use std::collections::{HashMap, HashSet, VecDeque};

use bevy::asset::RenderAssetUsages;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::render::mesh::{Indices, Mesh, PrimitiveTopology};

use crate::camera::FlyCamera;
use crate::citypack::{STREAM_WORLD_OFFSET, wanted_neighborhoods};
use crate::geology::{Geology, s2_enabled};

pub const TREES_PATH: &str = "assets/datasets/seattle_trees.bin";
const LOAD_M: f32 = 650.0;
const UNLOAD_M: f32 = 800.0;
pub const SPAWN_PER_FRAME: usize = 320;
pub const MAX_ACTIVE: usize = 6500;
const UNLOAD_PER_FRAME: usize = 450;
pub const LOD_NEAR_M: f32 = 80.0;
pub const LOD_MID_M: f32 = 180.0;
const LOD_TICK_SECS: f32 = 0.3;

pub const SPECIES: [&str; 6] = [
    "douglas_fir",
    "western_red_cedar",
    "bigleaf_maple",
    "pacific_madrone",
    "cherry",
    "london_plane",
];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TreeRecord {
    pub x: f32,
    pub z: f32,
    pub height: f32,
    pub species: u8,
    pub tint: u8,
    pub regime: u8,
}

#[derive(Resource)]
pub struct TreeDataset {
    bytes: Vec<u8>,
    directory: Vec<(u32, u32)>,
    pub total: u32,
}

impl TreeDataset {
    pub fn open(path: &std::path::Path) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        if bytes.len() < 18 || &bytes[0..4] != b"TRES" {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "bad tree dataset magic",
            ));
        }
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version != 1 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "unsupported tree dataset version",
            ));
        }
        let node_count = u32::from_le_bytes(bytes[6..10].try_into().unwrap()) as usize;
        let total = u32::from_le_bytes(bytes[10..14].try_into().unwrap());
        let need = 18 + node_count * 8;
        if bytes.len() < need {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "tree directory truncated",
            ));
        }
        let mut directory = Vec::with_capacity(node_count);
        for index in 0..node_count {
            let at = 18 + index * 8;
            let offset = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
            let count = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap());
            directory.push((offset, count));
        }
        Ok(Self {
            bytes,
            directory,
            total,
        })
    }

    pub fn neighborhoods(&self) -> usize {
        self.directory
            .iter()
            .filter(|(_offset, count)| *count > 0)
            .count()
    }

    pub fn count(&self, node_index: usize) -> u32 {
        self.directory
            .get(node_index)
            .map(|(_offset, count)| *count)
            .unwrap_or(0)
    }

    /// Decode one neighborhood's trees. `center` is the pack node centre.
    pub fn read_chunk(&self, node_index: usize, center: [f32; 2]) -> Vec<TreeRecord> {
        let Some((offset, count)) = self.directory.get(node_index).copied() else {
            return Vec::new();
        };
        let mut out = Vec::with_capacity(count as usize);
        let mut at = offset as usize;
        for _ in 0..count {
            if at + 10 > self.bytes.len() {
                break;
            }
            let qx = i16::from_le_bytes(self.bytes[at..at + 2].try_into().unwrap());
            let qz = i16::from_le_bytes(self.bytes[at + 2..at + 4].try_into().unwrap());
            let height_cm = u16::from_le_bytes(self.bytes[at + 4..at + 6].try_into().unwrap());
            out.push(TreeRecord {
                x: center[0] + qx as f32 * 0.1,
                z: center[1] + qz as f32 * 0.1,
                height: height_cm as f32 / 100.0,
                species: self.bytes[at + 6],
                tint: self.bytes[at + 7],
                regime: self.bytes[at + 8],
            });
            at += 10;
        }
        out
    }
}

pub fn lod_for(distance: f32) -> u8 {
    if distance < LOD_NEAR_M {
        0
    } else if distance < LOD_MID_M {
        1
    } else {
        2
    }
}

#[derive(Resource)]
pub struct TreeAssets {
    /// [species][tint][lod]
    meshes: Vec<Vec<Vec<Handle<Mesh>>>>,
    pub material: Handle<StandardMaterial>,
}

impl TreeAssets {
    pub fn mesh(&self, species: u8, tint: u8, lod: u8) -> Handle<Mesh> {
        let species = (species as usize).min(self.meshes.len() - 1);
        let tint = (tint as usize).min(self.meshes[species].len() - 1);
        let lod = (lod as usize).min(self.meshes[species][tint].len() - 1);
        self.meshes[species][tint][lod].clone()
    }

    pub fn draw_groups(&self) -> usize {
        self.meshes.iter().map(|tints| tints.len() * 3).sum()
    }
}

#[derive(Component)]
pub struct TreeVisual {
    pub species: u8,
    pub tint: u8,
    pub lod: u8,
}

#[derive(Resource, Default)]
pub struct Trees {
    pub loaded: HashMap<usize, Vec<Entity>>,
    pending: VecDeque<(usize, Vec<TreeRecord>, usize)>,
    unloading: VecDeque<(usize, Vec<Entity>)>,
    pub active: usize,
    pub draw_groups: usize,
    pub last_lod: f32,
    pub disabled: bool,
}

pub struct TreesPlugin;

impl Plugin for TreesPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_trees)
            .add_systems(Update, (tree_stream, tree_lod_update, tree_capture));
    }
}

fn asset(relative: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn setup_trees(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !s2_enabled() {
        return;
    }
    let dataset = match TreeDataset::open(&asset(TREES_PATH)) {
        Ok(dataset) => dataset,
        Err(error) => {
            warn!("TREES: dataset unavailable: {error}");
            return;
        }
    };
    let material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.95,
        metallic: 0.0,
        cull_mode: None,
        ..default()
    });
    let mut all = Vec::new();
    for species in 0..SPECIES.len() {
        let mut tints = Vec::new();
        for tint in 0..3 {
            let mut lods = Vec::new();
            for lod in 0..3 {
                let mesh = tree_mesh(species, tint, lod);
                lods.push(meshes.add(mesh));
            }
            tints.push(lods);
        }
        all.push(tints);
    }
    commands.insert_resource(TreeAssets {
        meshes: all,
        material,
    });
    commands.insert_resource(Trees {
        disabled: std::env::var_os("TREES_OFF").is_some(),
        ..default()
    });
    commands.init_resource::<TreeCaptureState>();
    info!(
        "TREES: dataset {} trees in {} neighborhoods",
        dataset.total,
        dataset.neighborhoods()
    );
    commands.insert_resource(dataset);
}

/// Crossed tapered cards for the canopy plus a trunk box on the near LOD.
fn tree_mesh(species: usize, tint: u8, lod: u8) -> Mesh {
    let (cards, width, conifer, base_color): (usize, f32, bool, [f32; 3]) = match species {
        0 => (3, 0.30, true, [0.055, 0.14, 0.065]),
        1 => (3, 0.36, true, [0.045, 0.115, 0.060]),
        2 => (3, 0.85, false, [0.10, 0.22, 0.075]),
        3 => (3, 0.60, false, [0.14, 0.24, 0.09]),
        4 => (3, 0.70, false, [0.20, 0.24, 0.11]),
        _ => (3, 0.80, false, [0.13, 0.23, 0.08]),
    };
    let cards = match lod {
        0 => cards,
        1 => 2,
        _ => 1,
    };
    let tint_scale: f32 = [1.0, 0.86, 1.12][tint as usize % 3];
    let color = [
        (base_color[0] * tint_scale).clamp(0.0, 1.0),
        (base_color[1] * tint_scale).clamp(0.0, 1.0),
        (base_color[2] * tint_scale).clamp(0.0, 1.0),
        1.0,
    ];
    let trunk = [0.30, 0.22, 0.15, 1.0];
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut colors: Vec<[f32; 4]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut push_quad = |a: Vec3, b: Vec3, c: Vec3, d: Vec3, normal: Vec3, color: [f32; 4]| {
        let base = positions.len() as u32;
        for point in [a, b, c, d] {
            positions.push(point.to_array());
            normals.push(normal.to_array());
            colors.push(color);
        }
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    };
    if lod == 0 {
        // Trunk box up to 40% of the height, four sides, no caps.
        let half = 0.035;
        let top = 0.42;
        let corners = [
            Vec3::new(-half, 0.0, -half),
            Vec3::new(half, 0.0, -half),
            Vec3::new(half, 0.0, half),
            Vec3::new(-half, 0.0, half),
        ];
        for side in 0..4 {
            let a = corners[side];
            let b = corners[(side + 1) % 4];
            let normal = Vec3::new(b.z - a.z, 0.0, -(b.x - a.x)).normalize_or_zero();
            push_quad(
                a,
                b,
                Vec3::new(b.x, top, b.z),
                Vec3::new(a.x, top, a.z),
                normal,
                trunk,
            );
        }
    }
    let top_y = if conifer { 1.0 } else { 0.92 };
    let bottom_y = if conifer { 0.16 } else { 0.34 };
    for card in 0..cards {
        let angle = std::f32::consts::PI * card as f32 / cards as f32;
        let (sin, cos) = angle.sin_cos();
        let across = Vec3::new(cos, 0.0, sin) * (width * 0.5);
        let mid = Vec3::new(0.0, bottom_y + (top_y - bottom_y) * 0.55, 0.0);
        let mid_width = across * 1.15;
        let top_width = across * if conifer { 0.02 } else { 0.30 };
        let base = Vec3::new(-across.x, bottom_y, -across.z);
        let base_b = Vec3::new(across.x, bottom_y, across.z);
        let normal = Vec3::new(-sin, 0.35, cos).normalize_or_zero();
        // Two stacked tapered quads make a rounder silhouette.
        push_quad(
            base,
            base_b,
            mid + Vec3::new(mid_width.x, 0.0, mid_width.z),
            mid - Vec3::new(mid_width.x, 0.0, mid_width.z),
            normal,
            color,
        );
        push_quad(
            mid - Vec3::new(mid_width.x, 0.0, mid_width.z),
            mid + Vec3::new(mid_width.x, 0.0, mid_width.z),
            Vec3::new(top_width.x, top_y, top_width.z),
            Vec3::new(-top_width.x, top_y, -top_width.z),
            normal,
            color,
        );
    }
    // Horizontal canopy card: keeps tree crowns readable from above, where
    // vertical cards collapse to slivers.
    let top_y_h = top_y * 0.78;
    let across_h = Vec3::new(width * 0.42, 0.0, width * 0.34);
    push_quad(
        Vec3::new(-across_h.x, top_y_h, -across_h.z),
        Vec3::new(across_h.x, top_y_h, -across_h.z),
        Vec3::new(across_h.x, top_y_h, across_h.z),
        Vec3::new(-across_h.x, top_y_h, across_h.z),
        Vec3::Y,
        [color[0] * 0.9, color[1] * 0.9, color[2] * 0.9, 1.0],
    );
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

fn tree_stream(world: &mut World) {
    let Some(trees) = world.get_resource::<Trees>() else {
        return;
    };
    if !s2_enabled() || trees.disabled {
        return;
    }
    if world.get_resource::<TreeDataset>().is_none() {
        return;
    }
    // Despawn evicted chunks under a per-frame budget so a pose jump cannot
    // stall a frame on thousands of entity removals.
    {
        let mut budget = UNLOAD_PER_FRAME;
        loop {
            if budget == 0 {
                break;
            }
            let take = {
                let mut trees = world.resource_mut::<Trees>();
                let Some((_node, entities)) = trees.unloading.front_mut() else {
                    break;
                };
                budget.min(entities.len())
            };
            let mut batch = Vec::with_capacity(take);
            {
                let mut trees = world.resource_mut::<Trees>();
                if let Some((_node, entities)) = trees.unloading.front_mut() {
                    batch.extend(entities.drain(..take));
                }
            }
            for entity in batch {
                let _ = world.despawn(entity);
            }
            let empty = world
                .resource::<Trees>()
                .unloading
                .front()
                .map(|(_node, entities)| entities.is_empty())
                .unwrap_or(true);
            if empty {
                world.resource_mut::<Trees>().unloading.pop_front();
            }
            budget -= take;
        }
        let mut trees = world.resource_mut::<Trees>();
        let loaded_count: usize = trees.loaded.values().map(Vec::len).sum();
        let unloading_count: usize = trees.unloading.iter().map(|(_n, e)| e.len()).sum();
        trees.active = loaded_count + unloading_count;
    }
    // Drain the spawn queue first.
    {
        let mut budget = SPAWN_PER_FRAME;
        loop {
            if budget == 0 {
                break;
            }
            let front = world
                .resource::<Trees>()
                .pending
                .front()
                .map(|(node, records, _at)| (*node, records.len()));
            let Some((node, remaining)) = front else {
                break;
            };
            let take = budget.min(remaining);
            let mut records = Vec::with_capacity(take);
            {
                let mut trees = world.resource_mut::<Trees>();
                if let Some((_node, queue, _at)) = trees.pending.front_mut() {
                    for _ in 0..take {
                        if let Some(record) = queue.pop() {
                            records.push(record);
                        }
                    }
                    if queue.is_empty() {
                        trees.pending.pop_front();
                    }
                }
            }
            spawn_records(world, node, &records);
            budget -= take;
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
        let active = world.resource::<Trees>().active;
        let candidates = wanted_neighborhoods(
            &state.pack.nodes,
            center,
            LOAD_M,
            &mut crate::citypack::Evaluations::default(),
        );
        let loaded = &world.resource::<Trees>().loaded;
        let unloading = world.resource::<Trees>().unloading.clone();
        let wanted = if active >= MAX_ACTIVE {
            Vec::new()
        } else {
            candidates
                .into_iter()
                .filter(|index| !loaded.contains_key(index))
                .filter(|index| !unloading.iter().any(|(node, _)| node == index))
                .take(1)
                .collect::<Vec<_>>()
        };
        let unload = world
            .resource::<Trees>()
            .loaded
            .iter()
            .filter(|(index, _)| {
                let node = &state.pack.nodes[**index];
                crate::citypack::aabb_distance2(&node.bounds, center) > UNLOAD_M * UNLOAD_M
            })
            .map(|(index, entities)| (*index, entities.clone()))
            .collect();
        (wanted, unload)
    };
    for (index, entities) in unload {
        let mut trees = world.resource_mut::<Trees>();
        trees.loaded.remove(&index);
        trees.pending.retain(|(node, _records, _at)| *node != index);
        if !entities.is_empty() {
            trees.unloading.push_back((index, entities));
        }
    }
    for index in wanted {
        let records = {
            let state = world.resource::<Geology>();
            let center = state.pack.nodes[index].center;
            world.resource::<TreeDataset>().read_chunk(index, center)
        };
        if records.is_empty() {
            world
                .resource_mut::<Trees>()
                .loaded
                .insert(index, Vec::new());
            continue;
        }
        let count = records.len();
        world
            .resource_mut::<Trees>()
            .pending
            .push_back((index, records, count));
        world
            .resource_mut::<Trees>()
            .loaded
            .insert(index, Vec::new());
        info!(
            "TREES chunk {}: {count} trees queued (loaded {} chunks)",
            world.resource::<Geology>().pack.nodes[index].name,
            world.resource::<Trees>().loaded.len()
        );
    }
}

fn spawn_records(world: &mut World, node: usize, records: &[TreeRecord]) {
    let Some(dem_height) = world.get_resource::<Geology>().map(|state| {
        let dem = &state.dem;
        records
            .iter()
            .map(|record| dem.height(record.x, record.z) + 0.03)
            .collect::<Vec<_>>()
    }) else {
        return;
    };
    let camera = world
        .query_filtered::<&Transform, With<FlyCamera>>()
        .single(world)
        .ok()
        .copied()
        .map(|transform| transform.translation)
        .unwrap_or(Vec3::ZERO);
    let started = std::time::Instant::now();
    let mut entities = Vec::with_capacity(records.len());
    for (record, y) in records.iter().zip(dem_height) {
        let world_x = record.x + STREAM_WORLD_OFFSET;
        let distance = ((record.x + STREAM_WORLD_OFFSET) - camera.x).hypot(record.z - camera.z);
        let lod = lod_for(distance);
        let mesh = world
            .resource::<TreeAssets>()
            .mesh(record.species, record.tint, lod);
        let material = world.resource::<TreeAssets>().material.clone();
        let entity = world
            .spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::from_xyz(world_x, y, record.z)
                    .with_scale(Vec3::splat(record.height.max(2.0))),
                TreeVisual {
                    species: record.species,
                    tint: record.tint,
                    lod,
                },
                NotShadowCaster,
                Name::new("tree"),
            ))
            .id();
        entities.push(entity);
    }
    let spawned = entities.len();
    {
        let mut trees = world.resource_mut::<Trees>();
        if let Some(entry) = trees.loaded.get_mut(&node) {
            entry.extend(entities);
        }
        trees.active = trees.loaded.values().map(Vec::len).sum();
        info!(
            "TREES spawn: {spawned} in {:.2} ms (active {})",
            started.elapsed().as_secs_f32() * 1000.0,
            trees.active
        );
    }
}

struct TreePose {
    name: &'static str,
    x: f32,
    z: f32,
    eye: f32,
    look: [f32; 3],
    minute: f32,
}

const TREE_POSES: [TreePose; 8] = [
    TreePose {
        name: "trees_street_fremont.png",
        x: 4974.9,
        z: -8366.6,
        eye: 6.0,
        look: [5047.9, 9.0, -8365.9],
        minute: 12.0 * 60.0,
    },
    TreePose {
        name: "trees_street_wallingford.png",
        x: -2662.8,
        z: -4858.7,
        eye: 6.0,
        look: [-2662.8, 9.0, -4938.7],
        minute: 12.0 * 60.0,
    },
    TreePose {
        name: "trees_park_greenlake.png",
        x: 702.0,
        z: -8238.0,
        eye: 32.0,
        look: [780.0, 8.0, -8120.0],
        minute: 12.0 * 60.0,
    },
    TreePose {
        name: "trees_park_discovery.png",
        x: -5169.0,
        z: -5775.0,
        eye: 34.0,
        look: [-5060.0, 8.0, -5660.0],
        minute: 12.0 * 60.0,
    },
    TreePose {
        name: "trees_aerial_ballard.png",
        x: -3658.0,
        z: -6945.0,
        eye: 210.0,
        look: [-4200.0, 25.0, -6300.0],
        minute: 12.0 * 60.0,
    },
    TreePose {
        name: "trees_horizon_rainier.png",
        x: 0.0,
        z: -1500.0,
        eye: 90.0,
        look: [4000.0, 55.0, 8000.0],
        minute: 12.0 * 60.0,
    },
    TreePose {
        name: "trees_dawn_skyline.png",
        x: 0.0,
        z: -1500.0,
        eye: 90.0,
        look: [4000.0, 45.0, 8000.0],
        minute: 5.0 * 60.0 + 40.0,
    },
    TreePose {
        name: "trees_dusk_skyline.png",
        x: 0.0,
        z: -1500.0,
        eye: 90.0,
        look: [4000.0, 45.0, 8000.0],
        minute: 20.0 * 60.0 + 30.0,
    },
];

fn tree_capture(world: &mut World) {
    if std::env::var_os("TREES_CAPTURE").is_none() || !s2_enabled() {
        return;
    }
    let elapsed = world.resource::<Time>().elapsed_secs();
    if elapsed < 5.0 {
        return;
    }
    let (index, entered, since, done) = {
        let state = world.resource::<TreeCaptureState>();
        (state.index, state.entered, state.since, state.done)
    };
    if done {
        return;
    }
    if index >= TREE_POSES.len() {
        world.resource_mut::<TreeCaptureState>().done = true;
        world.write_message(AppExit::Success);
        return;
    }
    let pose = &TREE_POSES[index];
    // Drive the camera every frame: the pilot controller also writes it.
    let ground = world.resource::<Geology>().dem.height(pose.x, pose.z);
    if let Ok(mut camera) = world
        .query_filtered::<&mut Transform, With<FlyCamera>>()
        .single_mut(world)
    {
        camera.translation = Vec3::new(STREAM_WORLD_OFFSET + pose.x, ground + pose.eye, pose.z);
        let look = Vec3::new(
            STREAM_WORLD_OFFSET + pose.look[0],
            ground + pose.look[1],
            pose.look[2],
        );
        let direction = (look - camera.translation).normalize_or_zero();
        camera.rotation = Quat::from_euler(
            EulerRot::YXZ,
            (-direction.x).atan2(-direction.z),
            direction.y.asin(),
            0.0,
        );
    }
    {
        let mut weather = world.resource_mut::<crate::weather::Weather>();
        weather.auto = false;
        weather.target = crate::weather::WeatherKind::Clear;
        weather.cloud_cover = 0.18;
        weather.precipitation = 0.0;
        weather.storm = 0.0;
        weather.snow = 0.0;
    }
    world.resource_mut::<crate::clock::SimClock>().minute_of_day = pose.minute as f64;
    if !entered {
        let mut state = world.resource_mut::<TreeCaptureState>();
        state.entered = true;
        state.since = elapsed;
        return;
    }
    if elapsed - since < 3.0 {
        return;
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("captures")
        .join(pose.name);
    world
        .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
        .observe(bevy::render::view::screenshot::save_to_disk(path));
    info!(
        "TREES capture {} (trees {})",
        pose.name,
        world.resource::<Trees>().active
    );
    let mut state = world.resource_mut::<TreeCaptureState>();
    state.index += 1;
    state.entered = false;
}

#[derive(Resource, Default)]
struct TreeCaptureState {
    index: usize,
    entered: bool,
    since: f32,
    done: bool,
}

fn tree_lod_update(
    time: Res<Time>,
    camera: Single<&Transform, With<FlyCamera>>,
    assets: Res<TreeAssets>,
    mut trees: ResMut<Trees>,
    mut query: Query<(&Transform, &mut Mesh3d, &mut TreeVisual)>,
) {
    if trees.disabled || std::env::var_os("TREES_OFF").is_some() {
        return;
    }
    let now = time.elapsed_secs();
    if now - trees.last_lod < LOD_TICK_SECS {
        return;
    }
    trees.last_lod = now;
    let camera = camera.translation;
    let mut groups: HashSet<(u8, u8, u8)> = HashSet::new();
    let mut active = 0usize;
    for (transform, mut mesh, mut visual) in &mut query {
        let distance = transform.translation.truncate().distance(camera.truncate());
        let lod = lod_for(distance);
        groups.insert((visual.species, visual.tint, lod));
        if lod != visual.lod {
            mesh.0 = assets.mesh(visual.species, visual.tint, lod);
            visual.lod = lod;
        }
        active += 1;
    }
    trees.active = active;
    trees.draw_groups = groups.len();
}
