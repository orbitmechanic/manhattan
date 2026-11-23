//! Seattle pedestrians: walkers on derived sidewalk bands.
//!
//! Density is the committed pedestrian field (ad density × daily pace) from
//! CityFields::seattle(), audited per capture as expected vs actual. Walkers
//! are zero-structure props with one shared mesh and tint variation; they
//! follow draped street polylines on a 2.2 m band beside the lane, pause and
//! resume, and pitch with the grade like the vehicles.

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
const MAX_ACTIVE: usize = 500;
const SPAWN_PER_FRAME: usize = 32;
const BASE_PER_CHUNK: f32 = 26.0;
const BAND: f32 = 2.2;
const HOTSPOTS: [([f32; 2], f32); 2] = [
    ([-518.6, -399.4], 220.0), // Pike Place plaza
    ([-138.4, -614.3], 200.0), // Westlake seam
];

#[derive(Clone, Copy, Debug)]
pub struct WalkerState {
    pub points: [([f32; 2], f32); 8],
    pub count: usize,
    pub length: f32,
    pub s: f32,
    pub speed: f32,
    pub direction: f32,
    pub pause: f32,
    pub tint: u8,
}

impl WalkerState {
    pub fn sample(&self, s: f32) -> ([f32; 2], [f32; 2]) {
        let mut remaining = s.rem_euclid(self.length);
        for index in 0..self.count.saturating_sub(1) {
            let (a, _) = self.points[index];
            let (b, _) = self.points[index + 1];
            let seg = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2))
                .sqrt()
                .max(1.0e-3);
            if remaining <= seg {
                let t = remaining / seg;
                return (
                    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t],
                    [(b[0] - a[0]) / seg, (b[1] - a[1]) / seg],
                );
            }
            remaining -= seg;
        }
        (self.points[0].0, [0.0, 1.0])
    }
}

pub fn walker_on_street(
    street: &crate::citypack::PackStreet,
    seed: u32,
    direction: f32,
    speed: f32,
) -> Option<WalkerState> {
    if street.points.len() < 2 {
        return None;
    }
    let mut points = [([0.0_f32, 0.0_f32], 0.0_f32); 8];
    let mut count = 0;
    let mut length = 0.0;
    for point in street.points.iter().take(8) {
        if count > 0 {
            let (prev, _) = points[count - 1];
            length += ((point[0] - prev[0]).powi(2) + (point[1] - prev[1]).powi(2)).sqrt();
        }
        points[count] = (*point, length);
        count += 1;
    }
    if count < 2 || length < 1.0 {
        return None;
    }
    Some(WalkerState {
        points,
        count,
        length,
        s: (seed as f32 / 89.0) % length,
        speed,
        direction,
        pause: 0.0,
        tint: (seed % 3) as u8,
    })
}

/// Pose on the sidewalk band: offset beside the lane, y from the DEM, pitch
/// with the drape (the hill law applies to feet too).
pub fn walker_pose(state: &WalkerState, dem: &crate::geology::Dem) -> (Vec3, Quat) {
    let (pos, dir) = state.sample(state.s);
    let sign = state.direction.signum();
    let nx = -dir[1] * sign;
    let nz = dir[0] * sign;
    let x = pos[0] + nx * (3.2 + BAND);
    let z = pos[1] + nz * (3.2 + BAND);
    let y = dem.height(x, z) + 0.9;
    let dx = dir[0] * sign;
    let dz = dir[1] * sign;
    let ahead_x = pos[0] + dx * 1.5;
    let ahead_z = pos[1] + dz * 1.5;
    let dy = dem.height(ahead_x, ahead_z) - dem.height(pos[0], pos[1]);
    let yaw = dx.atan2(dz);
    let pitch = (dy / 1.5).atan();
    (
        Vec3::new(x + STREAM_WORLD_OFFSET, y, z),
        Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0),
    )
}

/// Expected walkers for a neighborhood: pedestrian field at its centre ×
/// hotspot boost × base, halved per the performance directive.
pub fn expected_walkers(fields: &CityFields, hour: f32, center: [f32; 2]) -> f32 {
    let density = fields.pedestrian_density(center[0], center[1], hour);
    let boost = HOTSPOTS
        .iter()
        .map(|(anchor, radius)| {
            let d = ((center[0] - anchor[0]).powi(2) + (center[1] - anchor[1]).powi(2)).sqrt();
            if d < *radius { 2.5 } else { 1.0 }
        })
        .fold(1.0_f32, f32::max);
    (density * BASE_PER_CHUNK * boost * 0.5).max(0.0)
}

#[derive(Resource)]
pub struct PedAssets {
    mesh: Handle<Mesh>,
    tinted: Vec<Handle<StandardMaterial>>,
}

#[derive(Resource, Default)]
pub struct Pedestrians {
    pub loaded: HashMap<usize, Vec<Entity>>,
    pub states: HashMap<Entity, WalkerState>,
    pending: Vec<usize>,
    pub active: usize,
    pub expected: f32,
    pub enabled: bool,
}

pub struct PedestriansPlugin;

impl Plugin for PedestriansPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PedCaptureState>()
            .add_systems(Startup, setup_pedestrians)
            .add_systems(Update, (ped_stream, walk_pedestrians, ped_capture));
    }
}

fn tint_material(materials: &mut Assets<StandardMaterial>, tint: u8) -> Handle<StandardMaterial> {
    let color = match tint {
        0 => Color::srgb(0.55, 0.12, 0.10),
        1 => Color::srgb(0.12, 0.20, 0.45),
        _ => Color::srgb(0.20, 0.28, 0.16),
    };
    materials.add(StandardMaterial {
        base_color: color,
        perceptual_roughness: 0.8,
        ..default()
    })
}

fn setup_pedestrians(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !crate::geology::s2_enabled() {
        return;
    }
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    let half = Vec3::new(0.24, 0.85, 0.18);
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::new();
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
        positions.push(corner.to_array());
        normals.push(corner.normalize_or_zero().to_array());
        colors.push([1.0, 1.0, 1.0, 1.0]);
    }
    for face in [
        [0, 1, 2, 0, 2, 3],
        [4, 6, 5, 4, 7, 6],
        [0, 4, 5, 0, 5, 1],
        [3, 2, 6, 3, 6, 7],
        [0, 3, 7, 0, 7, 4],
        [1, 5, 6, 1, 6, 2],
    ] {
        indices.extend(face.iter().map(|i| *i as u32));
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    let mesh = meshes.add(mesh);
    let tinted = (0..3)
        .map(|tint| tint_material(&mut materials, tint))
        .collect();
    commands.insert_resource(PedAssets { mesh, tinted });
    commands.insert_resource(Pedestrians {
        enabled: std::env::var_os("PEDESTRIANS_OFF").is_none(),
        ..default()
    });
}

fn ped_stream(world: &mut World) {
    let Some(peds) = world.get_resource::<Pedestrians>() else {
        return;
    };
    if !crate::geology::s2_enabled() || !peds.enabled {
        return;
    }
    if world.get_resource::<PedAssets>().is_none() {
        return;
    }
    {
        let pending = {
            let mut peds = world.resource_mut::<Pedestrians>();
            peds.pending.drain(..).take(2).collect::<Vec<_>>()
        };
        for node in pending {
            spawn_walkers(world, node);
        }
        let mut peds = world.resource_mut::<Pedestrians>();
        peds.active = peds.loaded.values().map(Vec::len).sum();
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
        let peds = world.resource::<Pedestrians>();
        let wanted = if peds.active + SPAWN_PER_FRAME > MAX_ACTIVE {
            Vec::new()
        } else {
            candidates
                .into_iter()
                .filter(|index| !peds.loaded.contains_key(index))
                .filter(|index| !peds.pending.contains(index))
                .take(1)
                .collect::<Vec<_>>()
        };
        let unload = peds
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
        let mut peds = world.resource_mut::<Pedestrians>();
        for entity in peds.loaded.remove(&index).unwrap_or_default() {
            peds.states.remove(&entity);
        }
        peds.active = peds.loaded.values().map(Vec::len).sum();
    }
    for index in wanted {
        world.resource_mut::<Pedestrians>().pending.push(index);
        world
            .resource_mut::<Pedestrians>()
            .loaded
            .insert(index, Vec::new());
    }
}

fn spawn_walkers(world: &mut World, node_index: usize) {
    let hour = world.resource::<SimClock>().minute_of_day as f32 / 60.0;
    let fields = CityFields::seattle();
    let (chunk_center, streets) = {
        let state = world.resource::<Geology>();
        let node = &state.pack.nodes[node_index];
        let Ok(chunk) = state.pack.read_chunk(node_index) else {
            return;
        };
        (node.center, chunk.streets.clone())
    };
    let expected = expected_walkers(&fields, hour, chunk_center).round() as usize;
    {
        let mut peds = world.resource_mut::<Pedestrians>();
        peds.expected += expected as f32;
    }
    let mut entities = Vec::new();
    for (street_index, street) in streets.iter().enumerate() {
        if entities.len() >= expected {
            break;
        }
        for k in 0..3usize {
            if entities.len() >= expected {
                break;
            }
            let seed = (street.id.wrapping_mul(0x9E37_79B9))
                .wrapping_add(street_index as u32)
                .wrapping_add(k as u32 * 6151)
                .wrapping_add(node_index as u32);
            let direction = if seed & 1 == 0 { 1.0 } else { -1.0 };
            if let Some(state) = walker_on_street(street, seed, direction, 1.25) {
                let (position, rotation) = {
                    let dem = &world.resource::<Geology>().dem;
                    walker_pose(&state, dem)
                };
                let mesh = world.resource::<PedAssets>().mesh.clone();
                let material = world.resource::<PedAssets>().tinted[state.tint as usize].clone();
                let entity = world
                    .spawn((
                        Mesh3d(mesh),
                        MeshMaterial3d(material),
                        Transform::from_translation(position).with_rotation(rotation),
                        NotShadowCaster,
                        Name::new("pedestrian"),
                    ))
                    .id();
                world
                    .resource_mut::<Pedestrians>()
                    .states
                    .insert(entity, state);
                entities.push(entity);
            }
        }
    }
    let mut peds = world.resource_mut::<Pedestrians>();
    if let Some(entry) = peds.loaded.get_mut(&node_index) {
        entry.extend(entities);
    }
    peds.active = peds.loaded.values().map(Vec::len).sum();
}

fn walk_pedestrians(
    time: Res<Time<Real>>,
    dem: Option<Res<Geology>>,
    mut peds: Option<ResMut<Pedestrians>>,
    mut query: Query<&mut Transform>,
) {
    let (Some(dem), Some(peds)) = (dem, peds.as_mut()) else {
        return;
    };
    let dt = time.delta_secs().min(0.25);
    for (entity, state) in peds.states.iter_mut() {
        if state.pause > 0.0 {
            state.pause -= dt;
        } else {
            state.s = (state.s + state.speed * dt * state.direction).rem_euclid(state.length);
            // Occasional pause: deterministic-ish, seeded by position.
            let roll = ((state.s * 13.0).sin() * 43_758.547).fract();
            if roll.abs() < 0.002 {
                state.pause = 1.6;
            }
        }
        if let Ok(mut transform) = query.get_mut(*entity) {
            let (position, rotation) = walker_pose(state, &dem.dem);
            transform.translation = position;
            transform.rotation = rotation;
        }
    }
}

fn ped_capture(world: &mut World) {
    if std::env::var_os("PEDESTRIAN_CAPTURE").is_none() || !crate::geology::s2_enabled() {
        return;
    }
    let elapsed = world.resource::<Time>().elapsed_secs();
    if elapsed < 5.0 {
        return;
    }
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        static FIRST: AtomicBool = AtomicBool::new(false);
        if !FIRST.swap(true, Ordering::Relaxed) {
            info!(
                "PEDESTRIANS capture: first frame, active {}",
                world.resource::<Pedestrians>().active
            );
        }
    }
    let (index, entered, since, hour_state, done) = {
        let state = world.resource::<PedCaptureState>();
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
    const POSES: [(&str, f32, &str); 4] = [
        ("pedestrians_am_westlake", 8.5, "am"),
        ("pedestrians_graveyard", 3.0, "grave"),
        ("pedestrians_pike_place", 8.5, "plaza"),
        ("pedestrians_capitol_ascent", 8.5, "ascent"),
    ];
    if index >= POSES.len() {
        world.resource_mut::<PedCaptureState>().done = true;
        world.write_message(AppExit::Success);
        return;
    }
    let (name, hour, mode) = POSES[index];
    {
        let view = world.resource::<crate::camera_authority::StagedPose>();
        let direction = if view.active {
            (view.rotation * Vec3::NEG_Z).normalize_or_zero()
        } else {
            Vec3::new(-0.45, -0.75, -0.35).normalize()
        };
        let rotation = Quat::from_rotation_arc(Vec3::NEG_Z, direction);
        let mut lights = world.query::<(&mut DirectionalLight, &mut Transform)>();
        for (mut light, mut transform) in lights.iter_mut(world) {
            light.illuminance = 26_000.0;
            light.color = Color::srgb(1.0, 0.97, 0.92);
            transform.rotation = rotation;
        }
    }
    if !entered || hour_state != hour {
        if hour_state != hour {
            let entities = {
                let peds = world.resource::<Pedestrians>();
                peds.loaded.values().flatten().copied().collect::<Vec<_>>()
            };
            for entity in entities {
                let _ = world.despawn(entity);
            }
            let mut peds = world.resource_mut::<Pedestrians>();
            peds.loaded.clear();
            peds.states.clear();
            peds.pending.clear();
            peds.active = 0;
            peds.expected = 0.0;
        }
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
        if index == 1 && state_has_am(world) {
            let (eye, look) = state_am(world);
            let mut staged = world.resource_mut::<crate::camera_authority::StagedPose>();
            crate::camera_authority::stage_look_at(
                &mut staged,
                "pedestrian_capture",
                eye,
                look,
                name,
            );
        } else {
            // Curb-side template; plaza frames aim at the Pike Place anchor.
            if mode == "plaza" {
                let (ground, look_ground) = {
                    let geology = world.resource::<Geology>();
                    (
                        geology.dem.height(-470.0, -330.0),
                        geology.dem.height(-518.6, -399.4),
                    )
                };
                let mut staged = world.resource_mut::<crate::camera_authority::StagedPose>();
                crate::camera_authority::stage_look_at(
                    &mut staged,
                    "pedestrian_capture",
                    Vec3::new(STREAM_WORLD_OFFSET - 470.0, ground + 6.0, -330.0),
                    Vec3::new(STREAM_WORLD_OFFSET - 518.6, look_ground + 3.0, -399.4),
                    name,
                );
            } else if !curb_pose(world, name) {
                let (ground, look_ground) = {
                    let geology = world.resource::<Geology>();
                    (
                        geology.dem.height(-138.0, 80.0),
                        geology.dem.height(-138.0, -614.3),
                    )
                };
                let mut staged = world.resource_mut::<crate::camera_authority::StagedPose>();
                crate::camera_authority::stage_look_at(
                    &mut staged,
                    "pedestrian_capture",
                    Vec3::new(STREAM_WORLD_OFFSET - 138.0, ground + 6.0, 80.0),
                    Vec3::new(STREAM_WORLD_OFFSET - 138.0, look_ground + 3.0, -614.3),
                    name,
                );
            }
        }
        let mut state = world.resource_mut::<PedCaptureState>();
        state.entered = true;
        state.hour = hour;
        state.since = elapsed;
        return;
    }
    if elapsed - since < 4.0 {
        if (index != 1 || !state_has_am(world)) && POSES[index].2 != "plaza" {
            curb_pose(world, name);
        }
        return;
    }
    if POSES[index].2 == "plaza" {
        // Minimum-crowd law: the plaza frame must show a crowd.
        let pose = world.resource::<crate::camera_authority::StagedPose>();
        let eye = pose.translation;
        let forward = pose.rotation * Vec3::NEG_Z;
        let anchor = Vec3::new(STREAM_WORLD_OFFSET - 518.6, eye.y - 1.6, -399.4);
        let crowd = {
            let peds = world.resource::<Pedestrians>();
            peds.states
                .keys()
                .filter_map(|entity| world.get::<Transform>(*entity))
                .filter(|transform| {
                    let to = transform.translation - eye;
                    to.length() < 60.0 && to.normalize_or_zero().dot(forward) > 0.25
                })
                .count()
        };
        let nearest = {
            let peds = world.resource::<Pedestrians>();
            peds.states
                .keys()
                .filter_map(|entity| world.get::<Transform>(*entity))
                .map(|transform| transform.translation.distance(eye))
                .fold(f32::MAX, f32::min)
        };
        if elapsed - since < 9.0 && (crowd < 15 || nearest < 4.0) {
            return;
        }
        info!(
            "PEDESTRIANS plaza crowd: {crowd} in view, nearest {nearest:.1} m from anchor {:?}",
            anchor
        );
        assert!(crowd >= 15, "plaza frame has only {crowd} walkers in view");
        assert!(
            nearest >= 4.0,
            "plaza eye stands inside the crowd ({nearest} m)"
        );
    }
    let peds = world.resource::<Pedestrians>();
    let expected = peds.expected;
    let actual = peds.active;
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("captures")
        .join(format!("{name}.png"));
    world
        .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
        .observe(bevy::render::view::screenshot::save_to_disk(path));
    info!(
        "PEDESTRIANS capture {name} hour {hour}: expected {expected:.0}, actual {actual} (cap 500)"
    );
    let am = if index == 0 {
        let pose = world.resource::<crate::camera_authority::StagedPose>();
        Some((
            pose.translation,
            pose.translation + (pose.rotation * Vec3::NEG_Z) * 20.0,
        ))
    } else {
        None
    };
    let mut state = world.resource_mut::<PedCaptureState>();
    if let Some((eye, look)) = am {
        state.am_eye = eye;
        state.am_look = look;
        state.has_am = true;
    }
    state.index += 1;
    state.entered = false;
}

fn state_has_am(world: &World) -> bool {
    world.resource::<PedCaptureState>().has_am
}

fn state_am(world: &World) -> (Vec3, Vec3) {
    let state = world.resource::<PedCaptureState>();
    (state.am_eye, state.am_look)
}

fn curb_pose(world: &mut World, label: &str) -> bool {
    let fleet = {
        let peds = world.resource::<Pedestrians>();
        peds.states
            .keys()
            .filter_map(|entity| world.get::<Transform>(*entity))
            .map(|transform| (transform.translation, transform.rotation))
            .collect::<Vec<_>>()
    };
    for (position, rotation) in &fleet {
        let lane = *rotation * Vec3::NEG_Z;
        let (eye, look) = crate::camera_authority::curb_side_eye(*position, lane);
        if crate::camera_authority::clear_of_subjects(
            eye,
            fleet.iter().map(|(position, _)| *position),
            5.0,
        ) {
            let mut staged = world.resource_mut::<crate::camera_authority::StagedPose>();
            crate::camera_authority::stage_look_at(
                &mut staged,
                "pedestrian_capture",
                eye,
                look,
                label,
            );
            return true;
        }
    }
    false
}

#[derive(Resource, Default)]
struct PedCaptureState {
    index: usize,
    entered: bool,
    since: f32,
    hour: f32,
    done: bool,
    am_eye: Vec3,
    am_look: Vec3,
    has_am: bool,
}
