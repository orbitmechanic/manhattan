//! Seattle storefronts: district-grammar props on ground-floor faces.
//!
//! Four grammars (Pike Place awnings/sheds, Pioneer Square colonnades, SODO
//! warehouse doors, residential corner nodes) placed on building faces toward
//! the nearest street, seeded per building, streamed per neighborhood with the
//! tree-round hysteresis. Props only: no journals, no structure graphs.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::render::mesh::{Indices, Mesh, PrimitiveTopology};

use crate::camera::FlyCamera;
use crate::citypack::{Chunk, STREAM_WORLD_OFFSET, wanted_neighborhoods};
use crate::geology::Geology;
use crate::landmarks::LANDMARKS;

const LOAD_M: f32 = 650.0;
const UNLOAD_M: f32 = 800.0;
const SPAWN_PER_FRAME: usize = 48;
const MAX_ACTIVE: usize = 4000;
const MARKET: [f32; 2] = [-518.6, -399.4];

pub const KIND_AWNING: u8 = 0;
pub const KIND_COLONNADE: u8 = 1;
pub const KIND_WAREHOUSE: u8 = 2;
pub const KIND_PLACARD: u8 = 3;
pub const KIND_CRATE: u8 = 4;
pub const KINDS: usize = 5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub x: f32,
    pub z: f32,
    pub yaw: f32,
    pub kind: u8,
    pub tint: u8,
    pub building: u32,
}

fn hash01(mut v: u32) -> f32 {
    v ^= v >> 16;
    v = v.wrapping_mul(0x7FEB_352D);
    v ^= v >> 15;
    v = v.wrapping_mul(0x846C_A68F);
    v ^= v >> 16;
    (v & 0x00FF_FFFF) as f32 / 16_777_216.0
}

/// Grammar for a district. Returns the allowed prop kinds and a per-building
/// probability; `None` means no storefront layer in that district.
pub fn grammar(district: &str) -> Option<(&'static [u8], f32)> {
    match district {
        "Pike Place" => Some((&[KIND_AWNING, KIND_CRATE], 0.85)),
        "Pioneer Square" => Some((&[KIND_COLONNADE], 0.75)),
        "SODO" => Some((&[KIND_WAREHOUSE], 0.55)),
        "Ballard" | "Fremont" | "Wallingford" | "Queen Anne" | "Capitol Hill" | "West Seattle"
        | "First Hill" => Some((&[KIND_PLACARD], 0.12)),
        _ => None,
    }
}

fn nearest_street(chunk: &Chunk, x: f32, z: f32) -> Option<(f32, f32, f32)> {
    let mut best = (f32::MAX, 0.0, 0.0);
    for street in &chunk.streets {
        for pair in street.points.windows(2) {
            let (ax, az) = (pair[0][0], pair[0][1]);
            let (bx, bz) = (pair[1][0], pair[1][1]);
            let (dx, dz) = (bx - ax, bz - az);
            let len2 = dx * dx + dz * dz;
            let t = if len2 <= 1e-6 {
                0.0
            } else {
                (((x - ax) * dx + (z - az) * dz) / len2).clamp(0.0, 1.0)
            };
            let (px, pz) = (ax + dx * t, az + dz * t);
            let d2 = (px - x) * (px - x) + (pz - z) * (pz - z);
            if d2 < best.0 {
                best = (d2, px, pz);
            }
        }
    }
    if best.0 == f32::MAX {
        None
    } else {
        Some((best.0.sqrt(), best.1, best.2))
    }
}

/// Deterministic placements for one neighborhood chunk. `ground` returns the
/// terrain height, `wet` the project water mask.
pub fn place_chunk(
    node_index: usize,
    chunk: &Chunk,
    district: &str,
    ground: impl Fn(f32, f32) -> f32,
    wet: impl Fn(f32, f32) -> bool,
) -> Vec<Placement> {
    let Some((kinds, probability)) = grammar(district) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for building in &chunk.buildings {
        let key = building
            .id
            .wrapping_mul(0x9E37_79B9)
            .wrapping_add(node_index as u32);
        if hash01(key) > probability {
            continue;
        }
        let (cx, cz) = (building.centroid[0], building.centroid[1]);
        let Some((road_distance, sx, sz)) = nearest_street(chunk, cx, cz) else {
            continue;
        };
        if road_distance > 60.0 {
            continue;
        }
        // Face toward the street; support distance along the normal keeps the
        // prop outside the footprint.
        let (nx, nz) = (sx - cx, sz - cz);
        let norm = (nx * nx + nz * nz).sqrt();
        if norm < 1e-3 {
            continue;
        }
        let (nx, nz) = (nx / norm, nz / norm);
        let mut support: f32 = 0.0;
        for point in &building.perimeter {
            let dx = point[0] - cx;
            let dz = point[1] - cz;
            support = support.max(dx * nx + dz * nz);
        }
        let offset = support + 1.1;
        let x = cx + nx * offset;
        let z = cz + nz * offset;
        if nearest_street(chunk, x, z)
            .map(|(d, _, _)| d)
            .unwrap_or(0.0)
            < 1.6
        {
            continue;
        }
        if wet(x, z) {
            continue;
        }
        // Bridges, stadiums, arena and the Needle keep their feet clear.
        let blocked = LANDMARKS.iter().enumerate().any(|(index, landmark)| {
            if index == 1 {
                return false; // Pike Place hosts its own strip
            }
            let dx = landmark.anchor[0] - x;
            let dz = landmark.anchor[1] - z;
            dx * dx + dz * dz < 90.0 * 90.0
        });
        if blocked {
            continue;
        }
        let kind = kinds[(hash01(key ^ 0x51A7_5EED) * kinds.len() as f32) as usize % kinds.len()];
        let tint = (hash01(key ^ 0x1234_5678) * 3.0) as u8 % 3;
        let yaw = nx.atan2(nz);
        out.push(Placement {
            x,
            z,
            yaw,
            kind,
            tint,
            building: building.id,
        });
        // Pike Place gets clustered crates beside the awnings.
        if district == "Pike Place" && hash01(key ^ 0xABCD_EF01) < 0.45 {
            let side = (hash01(key ^ 0x0F0F_0F0F) - 0.5) * 3.0;
            let (ox, oz) = (-nz * side, nx * side);
            let (crate_x, crate_z) = (x + ox - nx * 0.4, z + oz - nz * 0.4);
            if !wet(crate_x, crate_z) {
                out.push(Placement {
                    x: crate_x,
                    z: crate_z,
                    yaw,
                    kind: KIND_CRATE,
                    tint,
                    building: building.id,
                });
            }
        }
        let _ = ground;
        let _ = MARKET;
    }
    out.sort_by(|a, b| {
        a.building
            .cmp(&b.building)
            .then(a.kind.cmp(&b.kind))
            .then(a.x.total_cmp(&b.x))
    });
    out
}

#[derive(Resource)]
pub struct StorefrontAssets {
    meshes: Vec<Vec<Handle<Mesh>>>,
    pub material: Handle<StandardMaterial>,
}

impl StorefrontAssets {
    pub fn mesh(&self, kind: u8, tint: u8) -> Handle<Mesh> {
        let kind = (kind as usize).min(self.meshes.len() - 1);
        let tint = (tint as usize).min(self.meshes[kind].len() - 1);
        self.meshes[kind][tint].clone()
    }
}

/// Every placement that has been generated, by prop kind, for derived cameras.
#[derive(Resource, Default)]
pub struct PlacementIndex {
    pub by_kind: HashMap<u8, Vec<Placement>>,
}

impl PlacementIndex {
    pub fn remember(&mut self, placements: &[Placement]) {
        for placement in placements {
            let list = self.by_kind.entry(placement.kind).or_default();
            if list.len() < 400 {
                list.push(*placement);
            }
        }
    }
}

#[derive(Resource, Default)]
pub struct Storefronts {
    pub loaded: HashMap<usize, Vec<Entity>>,
    pending: Vec<(usize, Vec<Placement>)>,
    pub active: usize,
    pub enabled: bool,
}

pub struct StorefrontsPlugin;

impl Plugin for StorefrontsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlacementIndex>()
            .init_resource::<StorefrontCaptureState>()
            .add_systems(Startup, setup_storefronts)
            .add_systems(Update, (storefront_stream, storefront_capture));
    }
}

fn asset(relative: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn setup_storefronts(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !crate::geology::s2_enabled() {
        return;
    }
    let material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.85,
        cull_mode: None,
        ..default()
    });
    let mut all = Vec::new();
    for kind in 0..KINDS {
        let mut tints = Vec::new();
        for tint in 0..3 {
            tints.push(meshes.add(prop_mesh(kind as u8, tint)));
        }
        all.push(tints);
    }
    commands.insert_resource(StorefrontAssets {
        meshes: all,
        material,
    });
    commands.insert_resource(Storefronts {
        enabled: std::env::var_os("STOREFRONTS_OFF").is_none(),
        ..default()
    });
    let _ = asset("");
}

fn prop_mesh(kind: u8, tint: u8) -> Mesh {
    let tint_scale = [1.0, 0.85, 1.15][tint as usize % 3];
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::new();
    let mut boxx = |center: Vec3, half: Vec3, color: [f32; 4]| {
        let base = positions.len() as u32;
        let corners = [
            Vec3::new(-half.x, -half.y, -half.z),
            Vec3::new(half.x, -half.y, -half.z),
            Vec3::new(half.x, half.y, -half.z),
            Vec3::new(-half.x, half.y, -half.z),
            Vec3::new(-half.x, -half.y, half.z),
            Vec3::new(half.x, -half.y, half.z),
            Vec3::new(half.x, half.y, half.z),
            Vec3::new(-half.x, half.y, half.z),
        ];
        for corner in corners {
            positions.push((center + corner).to_array());
            normals.push((center + corner).normalize_or_zero().to_array());
            colors.push(color);
        }
        for face in [
            [0, 1, 2, 0, 2, 3],
            [4, 6, 5, 4, 7, 6],
            [0, 4, 5, 0, 5, 1],
            [3, 2, 6, 3, 6, 7],
            [0, 3, 7, 0, 7, 4],
            [1, 5, 6, 1, 6, 2],
        ] {
            indices.extend(face.iter().map(|i| base + *i as u32));
        }
    };
    match kind {
        KIND_AWNING => {
            boxx(
                Vec3::new(0.0, 3.0, 0.0),
                Vec3::new(2.2, 0.12, 1.1),
                [0.55 * tint_scale, 0.12, 0.10, 1.0],
            );
        }
        KIND_COLONNADE => {
            for pier in [-1.8, 0.0, 1.8] {
                boxx(
                    Vec3::new(pier, 2.0, 0.0),
                    Vec3::new(0.32, 2.0, 0.32),
                    [0.32 * tint_scale, 0.30, 0.28, 1.0],
                );
            }
            boxx(
                Vec3::new(0.0, 4.1, 0.0),
                Vec3::new(2.4, 0.28, 0.45),
                [0.30 * tint_scale, 0.28, 0.26, 1.0],
            );
        }
        KIND_WAREHOUSE => {
            boxx(
                Vec3::new(0.0, 2.0, 0.0),
                Vec3::new(2.3, 2.0, 0.10),
                [0.20, 0.22 * tint_scale, 0.24, 1.0],
            );
        }
        KIND_PLACARD => {
            boxx(
                Vec3::new(0.0, 1.3, 0.0),
                Vec3::new(0.08, 1.3, 0.08),
                [0.18, 0.16, 0.15, 1.0],
            );
            boxx(
                Vec3::new(0.0, 2.7, 0.0),
                Vec3::new(0.9, 0.6, 0.08),
                [0.20, 0.45 * tint_scale, 0.30, 1.0],
            );
        }
        _ => {
            boxx(
                Vec3::new(0.0, 0.42, 0.0),
                Vec3::new(0.42, 0.42, 0.42),
                [0.45 * tint_scale, 0.30, 0.16, 1.0],
            );
        }
    }
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

fn storefront_stream(world: &mut World) {
    let Some(storefronts) = world.get_resource::<Storefronts>() else {
        return;
    };
    if !crate::geology::s2_enabled() || !storefronts.enabled {
        return;
    }
    if world.get_resource::<StorefrontAssets>().is_none() {
        return;
    }
    // Spawn queued placements first (props are cheap; keep the budget tight).
    {
        let mut budget = SPAWN_PER_FRAME;
        while budget > 0 {
            let take = {
                let mut storefronts = world.resource_mut::<Storefronts>();
                let Some((_, placements)) = storefronts.pending.first_mut() else {
                    break;
                };
                budget.min(placements.len())
            };
            let node = world
                .resource::<Storefronts>()
                .pending
                .first()
                .map(|(node, _)| *node)
                .unwrap_or(usize::MAX);
            let mut batch = Vec::with_capacity(take);
            {
                let mut storefronts = world.resource_mut::<Storefronts>();
                if let Some((_, placements)) = storefronts.pending.first_mut() {
                    batch.extend(placements.drain(..take));
                }
            }
            spawn_placements(world, node, &batch);
            let empty = world
                .resource::<Storefronts>()
                .pending
                .first()
                .map(|(_, placements)| placements.is_empty())
                .unwrap_or(true);
            if empty {
                let mut storefronts = world.resource_mut::<Storefronts>();
                storefronts
                    .pending
                    .retain(|(_, placements)| !placements.is_empty());
            }
            budget -= take;
        }
        let mut storefronts = world.resource_mut::<Storefronts>();
        storefronts.active = storefronts.loaded.values().map(Vec::len).sum();
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
        let candidates = wanted_neighborhoods(
            &state.pack.nodes,
            center,
            LOAD_M,
            &mut crate::citypack::Evaluations::default(),
        );
        let storefronts = world.resource::<Storefronts>();
        let wanted = if storefronts.active + SPAWN_PER_FRAME > MAX_ACTIVE {
            Vec::new()
        } else {
            candidates
                .into_iter()
                .filter(|index| !storefronts.loaded.contains_key(index))
                .filter(|index| !storefronts.pending.iter().any(|(node, _)| node == index))
                .take(1)
                .collect::<Vec<_>>()
        };
        let unload = storefronts
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
        for entity in entities {
            let _ = world.despawn(entity);
        }
        let mut storefronts = world.resource_mut::<Storefronts>();
        storefronts.loaded.remove(&index);
        storefronts.active = storefronts.loaded.values().map(Vec::len).sum();
    }
    for index in wanted {
        let (placements, count) = {
            let state = world.resource::<Geology>();
            let node = &state.pack.nodes[index];
            let district = state
                .districts
                .get(&node.name)
                .map(String::as_str)
                .unwrap_or("");
            let Ok(chunk) = state.pack.read_chunk(index) else {
                continue;
            };
            let dem = &state.dem;
            let mask = &state.mask;
            let wet = |x: f32, z: f32| {
                let col = ((x - dem.origin[0]) / dem.spacing).round() as isize;
                let row = ((z - dem.origin[1]) / dem.spacing).round() as isize;
                if row < 0 || col < 0 || row as usize >= dem.rows || col as usize >= dem.cols {
                    return false;
                }
                mask[row as usize * dem.cols + col as usize] != 0
            };
            let placements = place_chunk(index, &chunk, district, |x, z| dem.height(x, z), wet);
            let count = placements.len();
            (placements, count)
        };
        if count == 0 {
            world
                .resource_mut::<Storefronts>()
                .loaded
                .insert(index, Vec::new());
            continue;
        }
        world.resource_mut::<PlacementIndex>().remember(&placements);
        world
            .resource_mut::<Storefronts>()
            .pending
            .push((index, placements));
        world
            .resource_mut::<Storefronts>()
            .loaded
            .insert(index, Vec::new());
        info!(
            "STOREFRONTS {}: {count} props queued",
            world.resource::<Geology>().pack.nodes[index].name
        );
    }
}

#[derive(Resource, Default)]
struct StorefrontCaptureState {
    index: usize,
    entered: bool,
    since: f32,
    done: bool,
    chosen: Vec<Placement>,
}

const CAPTURE_KINDS: [(u8, usize, &str); 4] = [
    (KIND_AWNING, 1, "storefronts_pike_place"),
    (KIND_COLONNADE, 1, "storefronts_pioneer_square"),
    (KIND_WAREHOUSE, 0, "storefronts_sodo"),
    (KIND_PLACARD, 1, "storefronts_residential"),
];

/// Derived camera: pick a real placed prop, stand 18 m along its outward face
/// normal at pedestrian eye height, aim at its mid-height with a little street.
fn storefront_capture(world: &mut World) {
    if std::env::var_os("STOREFRONT_CAPTURE").is_none() || !crate::geology::s2_enabled() {
        return;
    }
    let elapsed = world.resource::<Time>().elapsed_secs();
    if elapsed < 5.0 {
        return;
    }
    let (index, entered, since, done) = {
        let state = world.resource::<StorefrontCaptureState>();
        (state.index, state.entered, state.since, state.done)
    };
    if done {
        return;
    }
    if index >= CAPTURE_KINDS.len() {
        world.resource_mut::<StorefrontCaptureState>().done = true;
        world.write_message(AppExit::Success);
        return;
    }
    {
        let mut clock = world.resource_mut::<crate::clock::SimClock>();
        clock.minute_of_day = 12.0 * 60.0;
        clock.playing = false;
        let mut weather = world.resource_mut::<crate::weather::Weather>();
        weather.auto = false;
        weather.target = crate::weather::WeatherKind::Clear;
        weather.cloud_cover = 0.18;
        weather.precipitation = 0.0;
        weather.storm = 0.0;
        weather.snow = 0.0;
    }
    let (kind, instance, name) = CAPTURE_KINDS[index];
    // Lighting override (documented workaround: capture-mode sun path renders
    // night; the storefront gate is the subject, the sun path is queued).
    {
        let mut lights = world.query::<(&mut DirectionalLight, &mut Transform)>();
        let rotation =
            Quat::from_rotation_arc(Vec3::NEG_Z, Vec3::new(-0.45, -0.75, -0.35).normalize());
        for (mut light, mut transform) in lights.iter_mut(world) {
            light.illuminance = 22_000.0;
            light.color = Color::srgb(1.0, 0.96, 0.9);
            transform.rotation = rotation;
        }
    }
    if !entered {
        let chosen = world
            .resource::<PlacementIndex>()
            .by_kind
            .get(&kind)
            .and_then(|list| {
                list.get(instance.min(list.len().saturating_sub(1)))
                    .copied()
            });
        let Some(placement) = chosen else {
            // Nothing streamed for this kind yet: park the camera in that
            // district so its chunks stream, then wait.
            let (wx, wz) = match kind {
                KIND_AWNING => (-440.0, -400.0),
                KIND_COLONNADE => (250.0, 650.0),
                KIND_WAREHOUSE => (400.0, 3600.0),
                _ => (-3060.0, -6060.0),
            };
            let ground = world
                .resource::<crate::geology::Geology>()
                .dem
                .height(wx, wz);
            let mut staged = world.resource_mut::<crate::camera_authority::StagedPose>();
            crate::camera_authority::stage_look_at(
                &mut staged,
                "storefront_capture",
                Vec3::new(crate::citypack::STREAM_WORLD_OFFSET + wx, ground + 6.0, wz),
                Vec3::new(
                    crate::citypack::STREAM_WORLD_OFFSET + wx + 80.0,
                    ground + 8.0,
                    wz - 60.0,
                ),
                "warming",
            );
            return;
        };
        let ground = world
            .resource::<crate::geology::Geology>()
            .dem
            .height(placement.x, placement.z);
        let nx = placement.yaw.sin();
        let nz = placement.yaw.cos();
        // 45 degrees along the facade: prop in the near third, host facade
        // and district context behind it.
        let (s45, c45) = (
            std::f32::consts::FRAC_1_SQRT_2,
            std::f32::consts::FRAC_1_SQRT_2,
        );
        let ex = nx * c45 - nz * s45;
        let ez = nx * s45 + nz * c45;
        let eye = Vec3::new(
            crate::citypack::STREAM_WORLD_OFFSET + placement.x + ex * 9.0,
            ground + 2.6,
            placement.z + ez * 9.0,
        );
        let look = Vec3::new(
            crate::citypack::STREAM_WORLD_OFFSET + placement.x - nx * 2.0,
            ground + 4.2,
            placement.z - nz * 2.0,
        );
        {
            let mut staged = world.resource_mut::<crate::camera_authority::StagedPose>();
            crate::camera_authority::stage_look_at(
                &mut staged,
                "storefront_capture",
                eye,
                look,
                name,
            );
        }
        {
            let mut clock = world.resource_mut::<crate::clock::SimClock>();
            clock.minute_of_day = 12.0 * 60.0;
            let mut weather = world.resource_mut::<crate::weather::Weather>();
            weather.auto = false;
            weather.target = crate::weather::WeatherKind::Clear;
            weather.cloud_cover = 0.18;
            weather.precipitation = 0.0;
            weather.storm = 0.0;
            weather.snow = 0.0;
        }
        let mut state = world.resource_mut::<StorefrontCaptureState>();
        state.chosen = vec![placement];
        state.entered = true;
        state.since = elapsed;
        return;
    }
    if elapsed - since < 1.4 {
        return;
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("captures")
        .join(format!("{name}.png"));
    world
        .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
        .observe(bevy::render::view::screenshot::save_to_disk(path));
    info!(
        "STOREFRONT capture {name} from kind {} at ({:.0},{:.0})",
        kind,
        world.resource::<StorefrontCaptureState>().chosen[0].x,
        world.resource::<StorefrontCaptureState>().chosen[0].z
    );
    let mut state = world.resource_mut::<StorefrontCaptureState>();
    state.index += 1;
    state.entered = false;
}

fn spawn_placements(world: &mut World, node: usize, placements: &[Placement]) {
    let ground = {
        let dem = &world.resource::<Geology>().dem;
        placements
            .iter()
            .map(|placement| dem.height(placement.x, placement.z))
            .collect::<Vec<_>>()
    };
    let mut entities = Vec::with_capacity(placements.len());
    for (placement, y) in placements.iter().zip(ground) {
        let mesh = world
            .resource::<StorefrontAssets>()
            .mesh(placement.kind, placement.tint);
        let material = world.resource::<StorefrontAssets>().material.clone();
        let entity = world
            .spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::from_xyz(placement.x + STREAM_WORLD_OFFSET, y + 0.03, placement.z)
                    .with_rotation(Quat::from_rotation_y(placement.yaw)),
                NotShadowCaster,
                Name::new("storefront"),
            ))
            .id();
        entities.push(entity);
    }
    let mut storefronts = world.resource_mut::<Storefronts>();
    if let Some(entry) = storefronts.loaded.get_mut(&node) {
        entry.extend(entities);
    }
    storefronts.active = storefronts.loaded.values().map(Vec::len).sum();
}
