//! Seattle traffic: ambient vehicles on the draped SND ribbons.
//!
//! Density follows the committed CityFields heartbeat (Westlake seam epicentre,
//! 08:30/17:00 peaks, 90/40 commute asymmetry, 5% graveyard) at half the
//! Manhattan-era scale. Vehicles are zero-structure props with shared meshes;
//! they stream per neighborhood and ride the ribbon grade (pitch tangent to the
//! drape). No agents, no signals, no damage.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::render::mesh::{Indices, Mesh, PrimitiveTopology};
use bevy::time::Real;

use crate::camera::FlyCamera;
use crate::city_rhythm::CityFields;
use crate::citypack::{STREAM_WORLD_OFFSET, wanted_neighborhoods};
use crate::clock::SimClock;
use crate::geology::Geology;

const LOAD_M: f32 = 650.0;
const UNLOAD_M: f32 = 800.0;
const MAX_ACTIVE: usize = 300;
const SPAWN_PER_FRAME: usize = 24;
const BASE_PER_CHUNK: f32 = 22.0;
const LANE_OFFSET: f32 = 3.2;

pub const SPECIES_SEDAN: u8 = 0;
pub const SPECIES_TRUCK: u8 = 1;
pub const SPECIES_BUS: u8 = 2;
pub const SPECIES: usize = 3;

#[derive(Clone, Copy, Debug)]
pub struct VehicleState {
    pub points: [([f32; 2], f32); 8],
    pub count: usize,
    pub length: f32,
    pub s: f32,
    pub speed: f32,
    pub direction: f32,
    pub species: u8,
}

impl VehicleState {
    pub fn sample(&self, s: f32) -> ([f32; 2], [f32; 2]) {
        let mut remaining = s.rem_euclid(self.length);
        for index in 0..self.count.saturating_sub(1) {
            let (a, da) = self.points[index];
            let (b, _) = self.points[index + 1];
            let seg = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2))
                .sqrt()
                .max(1.0e-3);
            if remaining <= seg {
                let t = remaining / seg;
                let pos = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                return (pos, [(b[0] - a[0]) / seg, (b[1] - a[1]) / seg]);
            }
            remaining -= seg;
            let _ = da;
        }
        let (a, _) = self.points[0];
        (a, [0.0, 1.0])
    }
}

/// Build a deterministic vehicle state for a street: shared by runtime and test.
pub fn vehicle_on_street(
    street: &crate::citypack::PackStreet,
    seed: u32,
    direction: f32,
    speed: f32,
    species: u8,
) -> Option<VehicleState> {
    if street.points.len() < 2 {
        return None;
    }
    let start = 0usize;
    let mut points = [([0.0_f32, 0.0_f32], 0.0_f32); 8];
    let mut count = 0;
    let mut length = 0.0;
    for index in start..street.points.len().min(start + 8) {
        let point = street.points[index];
        if count > 0 {
            let (prev, _) = points[count - 1];
            length += ((point[0] - prev[0]).powi(2) + (point[1] - prev[1]).powi(2)).sqrt();
        }
        points[count] = (point, length);
        count += 1;
    }
    if count < 2 || length < 1.0 {
        return None;
    }
    Some(VehicleState {
        points,
        count,
        length,
        s: (seed as f32 / 97.0) % length,
        speed,
        direction,
        species,
    })
}

/// Pose on the ribbon: lane offset from the centreline, y from the DEM, pitch
/// tangent to the drape (the hill law).
pub fn vehicle_pose(state: &VehicleState, dem: &crate::geology::Dem) -> (Vec3, Quat) {
    let (pos, dir) = state.sample(state.s);
    let sign = state.direction.signum();
    let nx = -dir[1] * sign;
    let nz = dir[0] * sign;
    let x = pos[0] + nx * LANE_OFFSET;
    let z = pos[1] + nz * LANE_OFFSET;
    let y = dem.height(x, z) + 0.35;
    let dx = dir[0] * sign;
    let dz = dir[1] * sign;
    let ahead_x = pos[0] + dx * 2.0;
    let ahead_z = pos[1] + dz * 2.0;
    let dy = dem.height(ahead_x, ahead_z) - dem.height(pos[0], pos[1]);
    let yaw = dx.atan2(dz);
    let pitch = (dy / 2.0).atan();
    let rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
    (Vec3::new(x + STREAM_WORLD_OFFSET, y, z), rotation)
}

#[derive(Resource)]
pub struct TrafficAssets {
    meshes: Vec<Vec<Handle<Mesh>>>,
    pub material: Handle<StandardMaterial>,
}

impl TrafficAssets {
    pub fn mesh(&self, species: u8, tint: u8) -> Handle<Mesh> {
        let s = (species as usize).min(self.meshes.len() - 1);
        let t = (tint as usize).min(self.meshes[s].len() - 1);
        self.meshes[s][t].clone()
    }
}

#[derive(Resource, Default)]
pub struct Traffic {
    pub loaded: HashMap<usize, Vec<Entity>>,
    pub states: HashMap<Entity, VehicleState>,
    pending: Vec<(usize, Vec<VehicleState>)>,
    pub active: usize,
    pub enabled: bool,
}

pub struct TrafficPlugin;

impl Plugin for TrafficPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_traffic)
            .add_systems(Update, (traffic_stream, drive_traffic, traffic_capture));
    }
}

fn setup_traffic(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !crate::geology::s2_enabled() {
        return;
    }
    let material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.6,
        ..default()
    });
    let mut all = Vec::new();
    for species in 0..SPECIES {
        let mut tints = Vec::new();
        for tint in 0..3 {
            tints.push(meshes.add(vehicle_mesh(species as u8, tint)));
        }
        all.push(tints);
    }
    commands.insert_resource(TrafficAssets {
        meshes: all,
        material,
    });
    commands.insert_resource(Traffic {
        enabled: std::env::var_os("TRAFFIC_OFF").is_none(),
        ..default()
    });
}

fn vehicle_mesh(species: u8, tint: u8) -> Mesh {
    let tint_scale = [1.0, 0.85, 1.12][tint as usize % 3];
    let (length, width, height) = match species {
        SPECIES_TRUCK => (7.0, 2.4, 3.0),
        SPECIES_BUS => (11.0, 2.6, 3.2),
        _ => (4.6, 1.9, 1.5),
    };
    let color = if species == SPECIES_BUS {
        [0.75, 0.35 * tint_scale, 0.10, 1.0]
    } else {
        [0.25 * tint_scale, 0.30, 0.55, 1.0]
    };
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::new();
    let base = 0u32;
    let half = Vec3::new(width * 0.5, height * 0.5, length * 0.5);
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
        positions.push((corner + Vec3::Y * 0.35).to_array());
        normals.push(corner.normalize_or_zero().to_array());
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

pub fn class_species(kind: u8, roll: f32) -> u8 {
    match kind {
        1 | 2 => {
            if roll < 0.18 {
                SPECIES_TRUCK
            } else {
                SPECIES_SEDAN
            }
        }
        5 | 6 => {
            if roll < 0.35 {
                SPECIES_BUS
            } else {
                SPECIES_SEDAN
            }
        }
        _ => SPECIES_SEDAN,
    }
}

fn traffic_stream(world: &mut World) {
    let Some(traffic) = world.get_resource::<Traffic>() else {
        return;
    };
    if !crate::geology::s2_enabled() || !traffic.enabled {
        return;
    }
    if world.get_resource::<TrafficAssets>().is_none() {
        return;
    }
    {
        let mut budget = SPAWN_PER_FRAME;
        while budget > 0 {
            let take = {
                let mut traffic = world.resource_mut::<Traffic>();
                let Some((_, batch)) = traffic.pending.first_mut() else {
                    break;
                };
                budget.min(batch.len())
            };
            let node = world
                .resource::<Traffic>()
                .pending
                .first()
                .map(|(node, _)| *node)
                .unwrap_or(usize::MAX);
            let mut batch = Vec::with_capacity(take);
            {
                let mut traffic = world.resource_mut::<Traffic>();
                if let Some((_, states)) = traffic.pending.first_mut() {
                    batch.extend(states.drain(..take));
                }
            }
            spawn_vehicles(world, node, &batch);
            let empty = world
                .resource::<Traffic>()
                .pending
                .first()
                .map(|(_, states)| states.is_empty())
                .unwrap_or(true);
            if empty {
                world
                    .resource_mut::<Traffic>()
                    .pending
                    .retain(|(_, states)| !states.is_empty());
            }
            budget -= take;
        }
        let mut traffic = world.resource_mut::<Traffic>();
        traffic.active = traffic.loaded.values().map(Vec::len).sum();
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
        let traffic = world.resource::<Traffic>();
        let wanted = if traffic.active + SPAWN_PER_FRAME > MAX_ACTIVE {
            Vec::new()
        } else {
            candidates
                .into_iter()
                .filter(|index| !traffic.loaded.contains_key(index))
                .filter(|index| !traffic.pending.iter().any(|(node, _)| node == index))
                .take(1)
                .collect::<Vec<_>>()
        };
        let unload = traffic
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
        let mut traffic = world.resource_mut::<Traffic>();
        for entity in traffic.loaded.remove(&index).unwrap_or_default() {
            traffic.states.remove(&entity);
        }
        traffic.active = traffic.loaded.values().map(Vec::len).sum();
    }
    for index in wanted {
        let hour = world.resource::<SimClock>().minute_of_day as f32 / 60.0;
        let fields = CityFields::seattle();
        let pace = fields.pace(hour);
        let blocks = {
            let state = world.resource::<Geology>();
            let node = &state.pack.nodes[index];
            let Ok(chunk) = state.pack.read_chunk(index) else {
                continue;
            };
            let mut blocks = Vec::new();
            for (street_index, street) in chunk.streets.iter().enumerate() {
                let toward = {
                    let mid = street.points[street.points.len() / 2];
                    let heading = Vec2::new(
                        street.points[street.points.len() - 1][0] - street.points[0][0],
                        street.points[street.points.len() - 1][1] - street.points[0][1],
                    );
                    fields.heading_toward(mid[0], mid[1], heading)
                };
                let occupancy = fields.occupancy(hour, toward);
                let share = (BASE_PER_CHUNK * pace * occupancy * 0.5).round() as usize;
                for k in 0..share.min(4) {
                    let seed = (street.id.wrapping_mul(0x9E37_79B9))
                        .wrapping_add(street_index as u32)
                        .wrapping_add(k as u32 * 7919);
                    let roll = (seed % 1000) as f32 / 1000.0;
                    let species = class_species(street.kind_idx, roll);
                    let direction = if seed & 1 == 0 { 1.0 } else { -1.0 };
                    let speed = match street.kind_idx {
                        1 | 2 => 13.0,
                        5 | 6 => 9.0,
                        _ => 7.0,
                    };
                    if let Some(state) = vehicle_on_street(street, seed, direction, speed, species)
                    {
                        blocks.push(state);
                    }
                }
                if blocks.len() >= 24 {
                    break;
                }
            }
            let _ = node;
            blocks
        };
        if !blocks.is_empty() {
            world
                .resource_mut::<Traffic>()
                .pending
                .push((index, blocks));
        }
        world
            .resource_mut::<Traffic>()
            .loaded
            .insert(index, Vec::new());
    }
}

fn spawn_vehicles(world: &mut World, node: usize, states: &[VehicleState]) {
    let poses = {
        let dem = &world.resource::<Geology>().dem;
        states
            .iter()
            .map(|state| vehicle_pose(state, dem))
            .collect::<Vec<_>>()
    };
    let mut entities = Vec::with_capacity(states.len());
    for (state, (position, rotation)) in states.iter().zip(poses) {
        let mesh = world
            .resource::<TrafficAssets>()
            .mesh(state.species, ((state.length as u32) % 3) as u8);
        let material = world.resource::<TrafficAssets>().material.clone();
        let entity = world
            .spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::from_translation(position)
                    .with_rotation(rotation)
                    .with_scale(Vec3::splat(1.0)),
                NotShadowCaster,
                Name::new("vehicle"),
            ))
            .id();
        world
            .resource_mut::<Traffic>()
            .states
            .insert(entity, *state);
        entities.push(entity);
    }
    let mut traffic = world.resource_mut::<Traffic>();
    if let Some(entry) = traffic.loaded.get_mut(&node) {
        entry.extend(entities);
    }
    traffic.active = traffic.loaded.values().map(Vec::len).sum();
}

fn drive_traffic(
    time: Res<Time<Real>>,
    dem: Option<Res<Geology>>,
    mut traffic: Option<ResMut<Traffic>>,
    mut query: Query<&mut Transform>,
) {
    let (Some(dem), Some(traffic)) = (dem, traffic.as_mut()) else {
        return;
    };
    let dt = time.delta_secs().min(0.25);
    for (entity, state) in traffic.states.iter_mut() {
        state.s = (state.s + state.speed * dt * state.direction).rem_euclid(state.length);
        if let Ok(mut transform) = query.get_mut(*entity) {
            let (position, rotation) = vehicle_pose(state, &dem.dem);
            transform.translation = position;
            transform.rotation = rotation;
        }
    }
}

fn traffic_capture(world: &mut World) {
    if std::env::var_os("TRAFFIC_CAPTURE").is_none() || !crate::geology::s2_enabled() {
        return;
    }
    let elapsed = world.resource::<Time>().elapsed_secs();
    if elapsed < 5.0 {
        return;
    }
    let (index, entered, since, hour_state, done) = {
        let state = world.resource::<TrafficCaptureState>();
        (
            state.index,
            state.entered,
            state.since,
            state.hour,
            state.done,
        )
    };
    if done {
        return;
    }
    const POSES: [(&str, f32, [f32; 2], [f32; 2]); 5] = [
        ("traffic_am_westlake", 8.5, [-138.0, 80.0], [-138.0, -614.3]),
        (
            "traffic_midday_ballard",
            13.0,
            [-3060.0, -6060.0],
            [-2980.0, -6140.0],
        ),
        (
            "traffic_pm_westlake",
            17.0,
            [-138.0, 80.0],
            [-138.0, -614.3],
        ),
        ("traffic_graveyard", 3.0, [-138.0, 80.0], [-138.0, -614.3]),
        (
            "traffic_capitol_ascent",
            8.5,
            [250.0, 650.0],
            [250.0, 500.0],
        ),
    ];
    if index >= POSES.len() {
        world.resource_mut::<TrafficCaptureState>().done = true;
        world.write_message(AppExit::Success);
        return;
    }
    let (name, hour, eye, look) = POSES[index];
    if !entered || hour_state != hour {
        if hour_state != hour {
            // Reset the fleet so density reflects this moment.
            let entities = {
                let traffic = world.resource::<Traffic>();
                traffic
                    .loaded
                    .values()
                    .flatten()
                    .copied()
                    .collect::<Vec<_>>()
            };
            for entity in entities {
                let _ = world.despawn(entity);
            }
            let mut traffic = world.resource_mut::<Traffic>();
            traffic.loaded.clear();
            traffic.states.clear();
            traffic.pending.clear();
            traffic.active = 0;
        }
        let (ground, look_ground) = {
            let geology = world.resource::<Geology>();
            (
                geology.dem.height(eye[0], eye[1]),
                geology.dem.height(look[0], look[1]),
            )
        };
        {
            let mut clock = world.resource_mut::<SimClock>();
            clock.minute_of_day = hour as f64 * 60.0;
            clock.playing = false;
            let mut weather = world.resource_mut::<crate::weather::Weather>();
            weather.auto = false;
            weather.target = crate::weather::WeatherKind::Clear;
            weather.cloud_cover = 0.18;
            weather.precipitation = 0.0;
            weather.storm = 0.0;
            weather.snow = 0.0;
        }
        {
            let mut staged = world.resource_mut::<crate::camera_authority::StagedPose>();
            crate::camera_authority::stage_look_at(
                &mut staged,
                "traffic_capture",
                Vec3::new(STREAM_WORLD_OFFSET + eye[0], ground + 6.0, eye[1]),
                Vec3::new(STREAM_WORLD_OFFSET + look[0], look_ground + 4.0, look[1]),
                name,
            );
        }
        let mut state = world.resource_mut::<TrafficCaptureState>();
        state.entered = true;
        state.hour = hour;
        state.since = elapsed;
        return;
    }
    if elapsed - since < 4.0 {
        return;
    }
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("captures")
        .join(format!("{name}.png"));
    world
        .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
        .observe(bevy::render::view::screenshot::save_to_disk(path));
    info!(
        "TRAFFIC capture {name} hour {hour} vehicles {}",
        world.resource::<Traffic>().active
    );
    let mut state = world.resource_mut::<TrafficCaptureState>();
    state.index += 1;
    state.entered = false;
}

#[derive(Resource, Default)]
struct TrafficCaptureState {
    index: usize,
    entered: bool,
    since: f32,
    hour: f32,
    done: bool,
}
