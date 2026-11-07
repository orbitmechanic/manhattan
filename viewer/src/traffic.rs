//! Streaming vehicle traffic.
//!
//! The road builder exports lane centrelines.  Near the camera the lanes are
//! filled to roughly 85% occupancy from a catalog of 150 low-poly vehicles
//! (30 variants for each of tractor-trailer, cargo van, SUV, sedan and
//! emergency).  Vehicles creep forward unevenly, stop behind each other, and
//! are despawned when their lane leaves the traffic radius.

use std::collections::HashMap;

use bevy::{
    asset::RenderAssetUsages,
    image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor},
    light::NotShadowCaster,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};
use serde::Deserialize;

use crate::{camera::FlyCamera, environment::Environment};

const LANE_DATA: &str = include_str!("../assets/models/traffic_lanes.json");

const TRAFFIC_RADIUS: f32 = 1_000.0;

#[cfg(target_arch = "wasm32")]
const NEAR_TRAFFIC_RADIUS: f32 = 100.0;
#[cfg(not(target_arch = "wasm32"))]
const NEAR_TRAFFIC_RADIUS: f32 = 150.0;

#[cfg(target_arch = "wasm32")]
const MAX_NEAR_VEHICLES: usize = 300;
#[cfg(not(target_arch = "wasm32"))]
const MAX_NEAR_VEHICLES: usize = 600;

#[cfg(target_arch = "wasm32")]
const VEHICLE_LIGHT_RADIUS: f32 = 90.0;
#[cfg(not(target_arch = "wasm32"))]
const VEHICLE_LIGHT_RADIUS: f32 = 130.0;

#[cfg(target_arch = "wasm32")]
const MAX_PROXIES: usize = 2_000;
#[cfg(not(target_arch = "wasm32"))]
const MAX_PROXIES: usize = 3_000;

const OCCUPANCY: f32 = 0.85;
const VEHICLE_GAP: f32 = 0.7;
/// Spawn/despawn are amortised so a camera crossing a streaming boundary does
/// not create a one-frame spike of hundreds of entities.
#[cfg(target_arch = "wasm32")]
const STREAM_VEHICLE_BUDGET: usize = 36;
#[cfg(not(target_arch = "wasm32"))]
const STREAM_VEHICLE_BUDGET: usize = 72;
#[cfg(target_arch = "wasm32")]
const STREAM_PROXY_BUDGET: usize = 48;
#[cfg(not(target_arch = "wasm32"))]
const STREAM_PROXY_BUDGET: usize = 96;
const STREAM_LANE_BUDGET: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum VehicleKind {
    Tractor,
    Van,
    Suv,
    Sedan,
    Emergency,
}

#[derive(Deserialize)]
struct LaneFile {
    lanes: Vec<LaneRecord>,
}

#[derive(Deserialize)]
struct LaneRecord {
    points: Vec<[f32; 2]>,
    width: f32,
    length: f32,
}

pub struct LaneData {
    points: Vec<[f32; 2]>,
    cumulative: Vec<f32>,
    length: f32,
    width: f32,
}

#[derive(Resource)]
pub struct TrafficLanes {
    lanes: Vec<LaneData>,
}

impl Default for TrafficLanes {
    fn default() -> Self {
        let parsed: LaneFile =
            serde_json::from_str(LANE_DATA).expect("traffic_lanes.json is invalid");
        let lanes = parsed
            .lanes
            .into_iter()
            .filter(|lane| lane.points.len() >= 2)
            .map(|lane| {
                let mut cumulative = vec![0.0];
                for i in 0..lane.points.len() {
                    let a = lane.points[i];
                    let b = lane.points[(i + 1) % lane.points.len()];
                    cumulative.push(
                        cumulative.last().copied().unwrap_or(0.0)
                            + ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt(),
                    );
                }
                LaneData {
                    points: lane.points,
                    cumulative,
                    length: lane.length.max(1.0),
                    width: lane.width.max(1.5),
                }
            })
            .collect();
        Self { lanes }
    }
}

fn lane_sample(lane: &LaneData, s: f32) -> ([f32; 2], [f32; 2]) {
    let total = *lane.cumulative.last().unwrap_or(&0.0);
    let s = if total > 1e-6 {
        s.rem_euclid(total)
    } else {
        0.0
    };
    let mut index = 0;
    while index + 1 < lane.cumulative.len() && lane.cumulative[index + 1] <= s {
        index += 1;
    }
    if index + 1 >= lane.cumulative.len() {
        index = lane.points.len().saturating_sub(1);
    }
    let a = lane.points[index];
    let b = lane.points[(index + 1) % lane.points.len()];
    let span = (lane.cumulative[index + 1] - lane.cumulative[index]).max(1e-6);
    let t = ((s - lane.cumulative[index]) / span).clamp(0.0, 1.0);
    (
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t],
        [(b[0] - a[0]) / span, (b[1] - a[1]) / span],
    )
}

// ---------------------------------------------------------------------------
// catalog
// ---------------------------------------------------------------------------
struct VehicleVariant {
    body: Handle<Mesh>,
    lights: Handle<Mesh>,
    length: f32,
    width: f32,
    height: f32,
    body_color: [f32; 4],
    kind: VehicleKind,
}

#[derive(Resource)]
pub struct VehicleCatalog {
    variants: Vec<VehicleVariant>,
    body_material: Handle<StandardMaterial>,
    lights_material: Handle<StandardMaterial>,
}

#[derive(Resource, Default)]
pub struct Traffic {
    active: HashMap<usize, Vec<Entity>>,
    proxies: HashMap<usize, Entity>,
    last_check: f32,
}

#[derive(Component)]
pub struct Vehicle {
    lane: usize,
    s: f32,
    length: f32,
    width: f32,
    yaw_offset: f32,
}

/// Marks the emissive headlight/taillight mesh of an individual near vehicle.
/// Proxy lights stay visible at every distance; individual lights are only
/// needed close up, so the LOD system hides them beyond a short radius.
#[derive(Component)]
pub struct VehicleLight;

/// Marks a merged far-lane traffic proxy so high-speed LOD can hide it.
#[derive(Component)]
pub struct TrafficProxy;

fn next_rand(seed: &mut u32) -> f32 {
    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    ((*seed >> 8) as f32) / ((u32::MAX >> 8) as f32)
}

#[allow(clippy::too_many_arguments)]
fn push_quad_color(
    verts: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    colors: &mut Vec<[f32; 4]>,
    indices: &mut Vec<u32>,
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
    d: [f32; 3],
    color: [f32; 4],
) {
    let ab = Vec3::from_array(b) - Vec3::from_array(a);
    let ac = Vec3::from_array(c) - Vec3::from_array(a);
    let n = ab.cross(ac).normalize_or_zero();
    let base = verts.len() as u32;
    verts.extend([a, b, c, d]);
    normals.extend([[n.x, n.y, n.z]; 4]);
    colors.extend([color; 4]);
    indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
}

#[allow(clippy::too_many_arguments)]
fn add_box_color(
    verts: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    colors: &mut Vec<[f32; 4]>,
    indices: &mut Vec<u32>,
    center: Vec3,
    half: Vec3,
    color: [f32; 4],
) {
    let c = center;
    let h = half;
    let p = [
        c + Vec3::new(-h.x, -h.y, -h.z),
        c + Vec3::new(h.x, -h.y, -h.z),
        c + Vec3::new(h.x, h.y, -h.z),
        c + Vec3::new(-h.x, h.y, -h.z),
        c + Vec3::new(-h.x, -h.y, h.z),
        c + Vec3::new(h.x, -h.y, h.z),
        c + Vec3::new(h.x, h.y, h.z),
        c + Vec3::new(-h.x, h.y, h.z),
    ];
    for (a, b, cc, d) in [
        (0, 1, 2, 3),
        (5, 4, 7, 6),
        (4, 0, 3, 7),
        (1, 5, 6, 2),
        (3, 2, 6, 7),
        (4, 5, 1, 0),
    ] {
        push_quad_color(
            verts,
            normals,
            colors,
            indices,
            p[a].to_array(),
            p[b].to_array(),
            p[cc].to_array(),
            p[d].to_array(),
            color,
        );
    }
}

fn finish_mesh(
    verts: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    colors: Option<Vec<[f32; 4]>>,
    uvs: Option<Vec<[f32; 2]>>,
    indices: Vec<u32>,
) -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, verts)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_indices(Indices::U32(indices));
    if let Some(colors) = colors {
        mesh = mesh.with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
    if let Some(uvs) = uvs {
        mesh = mesh.with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    }
    mesh
}

fn civilian_palette() -> [[f32; 4]; 10] {
    [
        [0.44, 0.10, 0.08, 1.0], // muted red
        [0.10, 0.18, 0.38, 1.0], // navy
        [0.10, 0.30, 0.16, 1.0], // forest green
        [0.38, 0.26, 0.10, 1.0], // ochre
        [0.08, 0.30, 0.32, 1.0], // teal
        [0.30, 0.12, 0.28, 1.0], // plum
        [0.45, 0.38, 0.22, 1.0], // sand
        [0.16, 0.17, 0.19, 1.0], // charcoal
        [0.52, 0.38, 0.14, 1.0], // amber
        [0.30, 0.31, 0.34, 1.0], // slate grey
    ]
}

#[allow(clippy::too_many_arguments)]
fn push_quad_uv_tile(
    verts: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    uvs: &mut Vec<[f32; 2]>,
    indices: &mut Vec<u32>,
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
    d: [f32; 3],
    tile: usize,
) {
    let ab = Vec3::from_array(b) - Vec3::from_array(a);
    let ac = Vec3::from_array(c) - Vec3::from_array(a);
    let n = ab.cross(ac).normalize_or_zero();
    let base = verts.len() as u32;
    let col = (tile % 4) as f32;
    let row = (tile / 4) as f32;
    let u0 = col * 0.25;
    let v0 = row * 0.25;
    verts.extend([a, b, c, d]);
    normals.extend([[n.x, n.y, n.z]; 4]);
    uvs.extend([
        [u0, v0 + 0.25],
        [u0 + 0.25, v0 + 0.25],
        [u0 + 0.25, v0],
        [u0, v0],
    ]);
    indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
}

#[allow(clippy::too_many_arguments)]
fn add_box_uv_tile(
    verts: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    uvs: &mut Vec<[f32; 2]>,
    indices: &mut Vec<u32>,
    center: Vec3,
    half: Vec3,
    tile: usize,
) {
    let c = center;
    let h = half;
    let p = [
        c + Vec3::new(-h.x, -h.y, -h.z),
        c + Vec3::new(h.x, -h.y, -h.z),
        c + Vec3::new(h.x, h.y, -h.z),
        c + Vec3::new(-h.x, h.y, -h.z),
        c + Vec3::new(-h.x, -h.y, h.z),
        c + Vec3::new(h.x, -h.y, h.z),
        c + Vec3::new(h.x, h.y, h.z),
        c + Vec3::new(-h.x, h.y, h.z),
    ];
    for (a, b, cc, d) in [
        (0, 1, 2, 3),
        (5, 4, 7, 6),
        (4, 0, 3, 7),
        (1, 5, 6, 2),
        (3, 2, 6, 7),
        (4, 5, 1, 0),
    ] {
        push_quad_uv_tile(
            verts,
            normals,
            uvs,
            indices,
            p[a].to_array(),
            p[b].to_array(),
            p[cc].to_array(),
            p[d].to_array(),
            tile,
        );
    }
}

fn vehicle_and_lights(
    kind: VehicleKind,
    index: usize,
    seed: &mut u32,
) -> (Mesh, Mesh, f32, f32, f32, [f32; 4]) {
    let palette = civilian_palette();
    let mut color = palette[(next_rand(seed) * palette.len() as f32) as usize % palette.len()];
    let (mut length, mut width, mut height) = match kind {
        VehicleKind::Tractor => (16.0, 2.5, 3.7),
        VehicleKind::Van => (5.4, 2.0, 2.4),
        VehicleKind::Suv => (4.9, 1.95, 1.85),
        VehicleKind::Sedan => (4.6, 1.8, 1.5),
        VehicleKind::Emergency => {
            if index.is_multiple_of(3) {
                (6.0, 2.2, 2.8)
            } else if index % 3 == 1 {
                (9.2, 2.5, 3.4)
            } else {
                (4.8, 1.9, 1.6)
            }
        }
    };
    length *= 0.92 + next_rand(seed) * 0.16;
    width *= 0.94 + next_rand(seed) * 0.12;
    height *= 0.94 + next_rand(seed) * 0.12;
    if kind == VehicleKind::Emergency {
        color = if index.is_multiple_of(3) {
            [0.88, 0.88, 0.86, 1.0]
        } else if index % 3 == 1 {
            [0.62, 0.07, 0.05, 1.0]
        } else {
            [0.08, 0.10, 0.14, 1.0]
        };
    }
    let window = [0.06, 0.08, 0.10, 1.0];
    let wheel = [0.04, 0.04, 0.05, 1.0];
    let mut v = Vec::new();
    let mut n = Vec::new();
    let mut c = Vec::new();
    let mut i = Vec::new();

    let half_w = width * 0.5;
    let chassis_h = height * 0.34;
    if kind == VehicleKind::Tractor {
        let cab_len = 2.2;
        add_box_color(
            &mut v,
            &mut n,
            &mut c,
            &mut i,
            Vec3::new(0.0, height * 0.42, length * 0.5 - cab_len * 0.5),
            Vec3::new(half_w, height * 0.42, cab_len * 0.5),
            color,
        );
        add_box_color(
            &mut v,
            &mut n,
            &mut c,
            &mut i,
            Vec3::new(0.0, height * 0.42, -length * 0.12),
            Vec3::new(half_w, height * 0.42, length * 0.33),
            [0.55, 0.54, 0.50, 1.0],
        );
        add_box_color(
            &mut v,
            &mut n,
            &mut c,
            &mut i,
            Vec3::new(0.0, height * 0.68, length * 0.5 - cab_len * 0.5),
            Vec3::new(half_w * 0.92, height * 0.16, cab_len * 0.35),
            window,
        );
        for z in [-length * 0.34, -length * 0.18, length * 0.5 - cab_len * 0.2] {
            for x in [-half_w + 0.12, half_w - 0.12] {
                add_box_color(
                    &mut v,
                    &mut n,
                    &mut c,
                    &mut i,
                    Vec3::new(x, 0.30, z),
                    Vec3::new(0.12, 0.30, 0.34),
                    wheel,
                );
            }
        }
    } else {
        add_box_color(
            &mut v,
            &mut n,
            &mut c,
            &mut i,
            Vec3::new(0.0, chassis_h, 0.0),
            Vec3::new(half_w, chassis_h, length * 0.5),
            color,
        );
        let cabin_z = if kind == VehicleKind::Van {
            length * 0.22
        } else {
            -length * 0.08
        };
        let cabin_h = height * 0.30;
        add_box_color(
            &mut v,
            &mut n,
            &mut c,
            &mut i,
            Vec3::new(0.0, chassis_h + cabin_h, cabin_z),
            Vec3::new(half_w * 0.90, cabin_h, length * 0.28),
            color,
        );
        add_box_color(
            &mut v,
            &mut n,
            &mut c,
            &mut i,
            Vec3::new(0.0, chassis_h + cabin_h * 0.72, cabin_z),
            Vec3::new(half_w * 0.92, cabin_h * 0.55, length * 0.285),
            window,
        );
        for z in [-length * 0.30, length * 0.30] {
            for x in [-half_w + 0.12, half_w - 0.12] {
                add_box_color(
                    &mut v,
                    &mut n,
                    &mut c,
                    &mut i,
                    Vec3::new(x, 0.28, z),
                    Vec3::new(0.12, 0.28, 0.32),
                    wheel,
                );
            }
        }
        if kind == VehicleKind::Emergency {
            if index.is_multiple_of(3) {
                // ambulance red stripe
                add_box_color(
                    &mut v,
                    &mut n,
                    &mut c,
                    &mut i,
                    Vec3::new(0.0, chassis_h * 0.95, 0.0),
                    Vec3::new(half_w + 0.01, chassis_h * 0.13, length * 0.5),
                    [0.68, 0.08, 0.07, 1.0],
                );
            } else if index % 3 == 1 {
                // fire-truck ladder
                add_box_color(
                    &mut v,
                    &mut n,
                    &mut c,
                    &mut i,
                    Vec3::new(0.0, height * 0.92, -length * 0.05),
                    Vec3::new(half_w * 0.55, 0.10, length * 0.36),
                    [0.55, 0.55, 0.52, 1.0],
                );
            } else {
                // police white doors
                for x in [-half_w + 0.01, half_w - 0.01] {
                    add_box_color(
                        &mut v,
                        &mut n,
                        &mut c,
                        &mut i,
                        Vec3::new(x, chassis_h, 0.0),
                        Vec3::new(0.015, chassis_h * 0.7, length * 0.22),
                        [0.88, 0.88, 0.86, 1.0],
                    );
                }
            }
        }
    }

    // lights mesh (shared warm-white emissive material)
    let mut lv = Vec::new();
    let mut ln = Vec::new();
    let mut lu = Vec::new();
    let mut li = Vec::new();
    for x in [-half_w * 0.65, half_w * 0.65] {
        add_box_uv_tile(
            &mut lv,
            &mut ln,
            &mut lu,
            &mut li,
            Vec3::new(x, chassis_h * 0.9, length * 0.5 + 0.02),
            Vec3::new(0.14, 0.08, 0.035),
            1,
        );
    }
    let cabin_light_z = if kind == VehicleKind::Tractor {
        length * 0.5 - 1.1
    } else if kind == VehicleKind::Van {
        length * 0.22
    } else {
        -length * 0.08
    };
    for x in [-half_w * 0.91, half_w * 0.91] {
        add_box_uv_tile(
            &mut lv,
            &mut ln,
            &mut lu,
            &mut li,
            Vec3::new(x, chassis_h + height * 0.24, cabin_light_z),
            Vec3::new(0.016, height * 0.11, length * 0.17),
            0,
        );
    }
    // Red taillights, amber side markers and (for emergency) the roof bar.
    for x in [-half_w * 0.65, half_w * 0.65] {
        add_box_uv_tile(
            &mut lv,
            &mut ln,
            &mut lu,
            &mut li,
            Vec3::new(x, chassis_h * 0.82, -length * 0.5 - 0.02),
            Vec3::new(0.16, 0.09, 0.045),
            2,
        );
    }
    for x in [-half_w - 0.01, half_w + 0.01] {
        for z in [-length * 0.28, length * 0.28] {
            add_box_uv_tile(
                &mut lv,
                &mut ln,
                &mut lu,
                &mut li,
                Vec3::new(x, chassis_h * 0.8, z),
                Vec3::new(0.025, 0.055, 0.08),
                3,
            );
        }
    }
    add_box_uv_tile(
        &mut lv,
        &mut ln,
        &mut lu,
        &mut li,
        Vec3::new(0.0, height + 0.02, cabin_light_z),
        Vec3::new(half_w * 0.30, 0.025, length * 0.15),
        0,
    );
    add_box_uv_tile(
        &mut lv,
        &mut ln,
        &mut lu,
        &mut li,
        Vec3::new(0.0, height + 0.02, -length * 0.35),
        Vec3::new(half_w * 0.45, 0.02, 0.055),
        3,
    );
    add_box_uv_tile(
        &mut lv,
        &mut ln,
        &mut lu,
        &mut li,
        Vec3::new(0.0, height * 0.72, length * 0.50),
        Vec3::new(half_w * 0.55, 0.02, 0.05),
        1,
    );
    if kind == VehicleKind::Emergency {
        add_box_uv_tile(
            &mut lv,
            &mut ln,
            &mut lu,
            &mut li,
            Vec3::new(0.0, height * 0.98, 0.0),
            Vec3::new(half_w * 0.45, 0.06, 0.08),
            4,
        );
    }

    (
        finish_mesh(v, n, Some(c), None, i),
        finish_mesh(lv, ln, None, Some(lu), li),
        length,
        width,
        height,
        color,
    )
}

pub fn setup_vehicle_catalog(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let night_atlas = asset_server
        .load_builder()
        .with_settings(|s: &mut ImageLoaderSettings| {
            s.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                address_mode_u: ImageAddressMode::ClampToEdge,
                address_mode_v: ImageAddressMode::ClampToEdge,
                ..default()
            });
        })
        .load("textures/vehicles/vehicle_night_atlas.png".to_string());
    let body_material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        metallic: 0.15,
        perceptual_roughness: 0.55,
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    let lights_material = materials.add(StandardMaterial {
        base_color: Color::BLACK,
        emissive: LinearRgba::BLACK,
        emissive_texture: Some(night_atlas),
        perceptual_roughness: 0.4,
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    let mut seed = 0x7E31_C1E5u32;
    let mut variants = Vec::with_capacity(150);
    for kind in [
        VehicleKind::Tractor,
        VehicleKind::Van,
        VehicleKind::Suv,
        VehicleKind::Sedan,
        VehicleKind::Emergency,
    ] {
        for index in 0..30 {
            let (body, lights, length, width, height, body_color) =
                vehicle_and_lights(kind, index, &mut seed);
            variants.push(VehicleVariant {
                body: meshes.add(body),
                lights: meshes.add(lights),
                length,
                width,
                height,
                body_color,
                kind,
            });
        }
    }
    commands.insert_resource(VehicleCatalog {
        variants,
        body_material,
        lights_material,
    });
    info!("Vehicle catalog: 150 variants");
}

fn choose_variant(
    catalog: &VehicleCatalog,
    kind: VehicleKind,
    lane_width: f32,
    seed: &mut u32,
) -> Option<usize> {
    let candidates: Vec<usize> = catalog
        .variants
        .iter()
        .enumerate()
        .filter(|(_, variant)| variant.kind == kind && variant.width + 0.35 <= lane_width)
        .map(|(index, _)| index)
        .collect();
    if candidates.is_empty() {
        return None;
    }
    Some(candidates[(next_rand(seed) * candidates.len() as f32) as usize % candidates.len()])
}

fn choose_kind(seed: &mut u32) -> VehicleKind {
    let roll = next_rand(seed);
    if roll < 0.10 {
        VehicleKind::Tractor
    } else if roll < 0.30 {
        VehicleKind::Van
    } else if roll < 0.70 {
        VehicleKind::Suv
    } else if roll < 0.90 {
        VehicleKind::Sedan
    } else {
        VehicleKind::Emergency
    }
}

fn spawn_vehicle(
    commands: &mut Commands,
    catalog: &VehicleCatalog,
    lane: &LaneData,
    lane_index: usize,
    s: f32,
    seed: &mut u32,
) -> Option<Entity> {
    let mut variant_index = None;
    for _ in 0..12 {
        let kind = choose_kind(seed);
        if let Some(index) = choose_variant(catalog, kind, lane.width, seed) {
            variant_index = Some(index);
            break;
        }
    }
    let variant_index = variant_index?;
    let variant = &catalog.variants[variant_index];
    let (point, tangent) = lane_sample(lane, s);
    let yaw_offset = (next_rand(seed) - 0.5) * 0.06;
    let yaw = tangent[0].atan2(tangent[1]) + yaw_offset;
    let entity = commands
        .spawn((
            NotShadowCaster,
            Name::new("Vehicle"),
            Mesh3d(variant.body.clone()),
            MeshMaterial3d(catalog.body_material.clone()),
            Transform::from_xyz(point[0], 0.07, point[1]).with_rotation(Quat::from_rotation_y(yaw)),
            Visibility::Inherited,
            Vehicle {
                lane: lane_index,
                s,
                length: variant.length,
                width: variant.width,
                yaw_offset,
            },
        ))
        .id();
    commands.entity(entity).with_children(|parent| {
        parent.spawn((
            NotShadowCaster,
            VehicleLight,
            Mesh3d(variant.lights.clone()),
            MeshMaterial3d(catalog.lights_material.clone()),
            Transform::IDENTITY,
        ));
    });
    Some(entity)
}

fn vehicle_aabb(x: f32, z: f32, yaw: f32, length: f32, width: f32) -> (f32, f32, f32, f32) {
    let (sin, cos) = yaw.sin_cos();
    let ex = (length * 0.5 * sin).abs() + (width * 0.5 * cos).abs();
    let ez = (length * 0.5 * cos).abs() + (width * 0.5 * sin).abs();
    (x - ex, x + ex, z - ez, z + ez)
}

#[allow(clippy::too_many_arguments)]
fn add_box_yaw(
    verts: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    colors: &mut Vec<[f32; 4]>,
    indices: &mut Vec<u32>,
    center: Vec3,
    half: Vec3,
    yaw: f32,
    color: [f32; 4],
) {
    let (sin, cos) = yaw.sin_cos();
    let rotate = |p: Vec3| {
        Vec3::new(
            center.x + p.x * cos + p.z * sin,
            center.y + p.y,
            center.z - p.x * sin + p.z * cos,
        )
    };
    let p = [
        rotate(Vec3::new(-half.x, -half.y, -half.z)),
        rotate(Vec3::new(half.x, -half.y, -half.z)),
        rotate(Vec3::new(half.x, half.y, -half.z)),
        rotate(Vec3::new(-half.x, half.y, -half.z)),
        rotate(Vec3::new(-half.x, -half.y, half.z)),
        rotate(Vec3::new(half.x, -half.y, half.z)),
        rotate(Vec3::new(half.x, half.y, half.z)),
        rotate(Vec3::new(-half.x, half.y, half.z)),
    ];
    for (a, b, c, d) in [
        (0, 1, 2, 3),
        (5, 4, 7, 6),
        (4, 0, 3, 7),
        (1, 5, 6, 2),
        (3, 2, 6, 7),
        (4, 5, 1, 0),
    ] {
        push_quad_color(
            verts,
            normals,
            colors,
            indices,
            p[a].to_array(),
            p[b].to_array(),
            p[c].to_array(),
            p[d].to_array(),
            color,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn add_box_yaw_uv(
    verts: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    uvs: &mut Vec<[f32; 2]>,
    indices: &mut Vec<u32>,
    center: Vec3,
    half: Vec3,
    yaw: f32,
    tile: usize,
) {
    let (sin, cos) = yaw.sin_cos();
    let rotate = |p: Vec3| {
        Vec3::new(
            center.x + p.x * cos + p.z * sin,
            center.y + p.y,
            center.z - p.x * sin + p.z * cos,
        )
    };
    let p = [
        rotate(Vec3::new(-half.x, -half.y, -half.z)),
        rotate(Vec3::new(half.x, -half.y, -half.z)),
        rotate(Vec3::new(half.x, half.y, -half.z)),
        rotate(Vec3::new(-half.x, half.y, -half.z)),
        rotate(Vec3::new(-half.x, -half.y, half.z)),
        rotate(Vec3::new(half.x, -half.y, half.z)),
        rotate(Vec3::new(half.x, half.y, half.z)),
        rotate(Vec3::new(-half.x, half.y, half.z)),
    ];
    for (a, b, c, d) in [
        (0, 1, 2, 3),
        (5, 4, 7, 6),
        (4, 0, 3, 7),
        (1, 5, 6, 2),
        (3, 2, 6, 7),
        (4, 5, 1, 0),
    ] {
        push_quad_uv_tile(
            verts,
            normals,
            uvs,
            indices,
            p[a].to_array(),
            p[b].to_array(),
            p[c].to_array(),
            p[d].to_array(),
            tile,
        );
    }
}

fn proxy_mesh(lane: &LaneData, catalog: &VehicleCatalog, seed: &mut u32) -> Option<(Mesh, Mesh)> {
    let mut bv = Vec::new();
    let mut bn = Vec::new();
    let mut bc = Vec::new();
    let mut bi = Vec::new();
    let mut lv = Vec::new();
    let mut ln = Vec::new();
    let mut lu = Vec::new();
    let mut li = Vec::new();
    let mut cursor = 1.2;
    let limit = lane.length * OCCUPANCY;
    let window = [0.07, 0.08, 0.10, 1.0];
    while cursor < limit {
        let mut variant = None;
        for _ in 0..10 {
            let kind = choose_kind(seed);
            if let Some(index) = choose_variant(catalog, kind, lane.width, seed) {
                variant = Some(index);
                break;
            }
        }
        let Some(index) = variant else {
            break;
        };
        let v = &catalog.variants[index];
        let length = v.length;
        if cursor + length > limit {
            break;
        }
        let center_s = cursor + length * 0.5;
        let (point, tangent) = lane_sample(lane, center_s);
        let yaw = tangent[0].atan2(tangent[1]);
        let center = Vec3::new(point[0], 0.07 + v.height * 0.5, point[1]);
        add_box_yaw(
            &mut bv,
            &mut bn,
            &mut bc,
            &mut bi,
            center,
            Vec3::new(v.width * 0.5, v.height * 0.5, length * 0.5),
            yaw,
            v.body_color,
        );
        add_box_yaw(
            &mut bv,
            &mut bn,
            &mut bc,
            &mut bi,
            Vec3::new(point[0], 0.07 + v.height * 0.72, point[1]),
            Vec3::new(v.width * 0.42, v.height * 0.18, length * 0.22),
            yaw,
            window,
        );
        // Headlight and taillight boxes; enough to read as traffic at 0.25-1 km.
        let forward = Vec3::new(tangent[0], 0.0, tangent[1]);
        let front =
            Vec3::new(point[0], 0.07 + v.height * 0.42, point[1]) + forward * (length * 0.5);
        add_box_yaw_uv(
            &mut lv,
            &mut ln,
            &mut lu,
            &mut li,
            front,
            Vec3::new(v.width * 0.22, 0.08, 0.06),
            yaw,
            1,
        );
        let rear = Vec3::new(point[0], 0.07 + v.height * 0.42, point[1]) - forward * (length * 0.5);
        add_box_yaw_uv(
            &mut lv,
            &mut ln,
            &mut lu,
            &mut li,
            rear,
            Vec3::new(v.width * 0.18, 0.06, 0.05),
            yaw,
            2,
        );
        add_box_yaw_uv(
            &mut lv,
            &mut ln,
            &mut lu,
            &mut li,
            Vec3::new(point[0], 0.07 + v.height + 0.02, point[1]),
            Vec3::new(v.width * 0.30, 0.025, length * 0.14),
            yaw,
            0,
        );
        cursor += length + VEHICLE_GAP;
    }
    if bi.is_empty() {
        return None;
    }
    Some((
        finish_mesh(bv, bn, Some(bc), None, bi),
        finish_mesh(lv, ln, None, Some(lu), li),
    ))
}

fn spawn_proxy(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    catalog: &VehicleCatalog,
    lane: &LaneData,
    lane_index: usize,
    seed: &mut u32,
) -> Option<Entity> {
    let (body, lights) = proxy_mesh(lane, catalog, seed)?;
    let entity = commands
        .spawn((
            NotShadowCaster,
            TrafficProxy,
            Name::new(format!("TrafficProxy_{lane_index}")),
            Mesh3d(meshes.add(body)),
            MeshMaterial3d(catalog.body_material.clone()),
            Transform::IDENTITY,
            Visibility::Inherited,
        ))
        .id();
    commands.entity(entity).with_children(|parent| {
        parent.spawn((
            NotShadowCaster,
            Mesh3d(meshes.add(lights)),
            MeshMaterial3d(catalog.lights_material.clone()),
            Transform::IDENTITY,
        ));
    });
    Some(entity)
}

pub fn stream_traffic(
    mut commands: Commands,
    time: Res<Time>,
    camera: Single<&Transform, With<FlyCamera>>,
    lanes: Res<TrafficLanes>,
    catalog: Res<VehicleCatalog>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut traffic: ResMut<Traffic>,
) {
    let now = time.elapsed_secs();
    if now - traffic.last_check < 0.5 {
        return;
    }
    traffic.last_check = now;
    let cam = camera.translation;
    let mut near: Vec<(f32, usize)> = lanes
        .lanes
        .iter()
        .enumerate()
        .filter_map(|(index, lane)| {
            let (point, _) = lane_sample(lane, lane.length * 0.5);
            let dx = point[0] - cam.x;
            let dz = point[1] - cam.z;
            let distance = (dx * dx + dz * dz).sqrt();
            (distance <= TRAFFIC_RADIUS).then_some((distance, index))
        })
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    let near_ids: std::collections::HashSet<usize> = near
        .iter()
        .filter(|(distance, _)| *distance <= NEAR_TRAFFIC_RADIUS)
        .map(|(_, id)| *id)
        .collect();
    let within_ids: std::collections::HashSet<usize> = near.iter().map(|(_, id)| *id).collect();

    let far_active: Vec<usize> = traffic
        .active
        .keys()
        .copied()
        .filter(|id| !near_ids.contains(id))
        .collect();
    for id in far_active.into_iter().take(STREAM_LANE_BUDGET) {
        if let Some(entities) = traffic.active.remove(&id) {
            for entity in entities {
                commands.entity(entity).despawn();
            }
        }
    }
    let far_proxies: Vec<usize> = traffic
        .proxies
        .keys()
        .copied()
        .filter(|id| !within_ids.contains(id) || near_ids.contains(id))
        .collect();
    for id in far_proxies.into_iter().take(STREAM_LANE_BUDGET) {
        if let Some(entity) = traffic.proxies.remove(&id) {
            commands.entity(entity).despawn();
        }
    }

    let initial_fill = traffic.active.is_empty() && traffic.proxies.is_empty();
    let mut count: usize = traffic.active.values().map(|list| list.len()).sum();
    let mut vehicle_budget = if initial_fill {
        MAX_NEAR_VEHICLES
    } else {
        STREAM_VEHICLE_BUDGET
    };
    let mut proxy_budget = if initial_fill {
        MAX_PROXIES
    } else {
        STREAM_PROXY_BUDGET
    };
    let mut seed = (now * 1000.0) as u32 ^ 0x7A3F_5EED;
    for (distance, lane_id) in near {
        if distance <= NEAR_TRAFFIC_RADIUS {
            if vehicle_budget == 0 {
                continue;
            }
            if let Some(entity) = traffic.proxies.remove(&lane_id) {
                commands.entity(entity).despawn();
            }
            if traffic.active.contains_key(&lane_id) {
                continue;
            }
            let lane = &lanes.lanes[lane_id];
            let mut entities = Vec::new();
            let mut cursor = 1.2;
            let limit = lane.length * OCCUPANCY;
            while cursor < limit && count < MAX_NEAR_VEHICLES && vehicle_budget > 0 {
                let mut variant = None;
                for _ in 0..12 {
                    let kind = choose_kind(&mut seed);
                    if let Some(index) = choose_variant(&catalog, kind, lane.width, &mut seed) {
                        variant = Some(index);
                        break;
                    }
                }
                let Some(variant_index) = variant else {
                    break;
                };
                let length = catalog.variants[variant_index].length;
                if cursor + length > limit {
                    break;
                }
                let center = cursor + length * 0.5;
                if let Some(entity) =
                    spawn_vehicle(&mut commands, &catalog, lane, lane_id, center, &mut seed)
                {
                    entities.push(entity);
                    count += 1;
                    vehicle_budget -= 1;
                    cursor += length + VEHICLE_GAP;
                } else {
                    break;
                }
            }
            if !entities.is_empty() {
                traffic.active.insert(lane_id, entities);
            }
        } else if !traffic.proxies.contains_key(&lane_id)
            && traffic.proxies.len() < MAX_PROXIES
            && proxy_budget > 0
            && let Some(entity) = spawn_proxy(
                &mut commands,
                &mut meshes,
                &catalog,
                &lanes.lanes[lane_id],
                lane_id,
                &mut seed,
            )
        {
            traffic.proxies.insert(lane_id, entity);
            proxy_budget -= 1;
        }
    }
}

pub fn update_traffic(
    time: Res<Time>,
    lanes: Res<TrafficLanes>,
    mut query: Query<(Entity, &mut Vehicle, &mut Transform)>,
) {
    let now = time.elapsed_secs();
    let states: Vec<(Entity, usize, f32, f32, f32, f32)> = query
        .iter_mut()
        .map(|(entity, vehicle, transform)| {
            (
                entity,
                vehicle.lane,
                vehicle.s,
                vehicle.length,
                vehicle.width,
                transform.rotation.to_euler(EulerRot::YXZ).0,
            )
        })
        .collect();
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for (index, state) in states.iter().enumerate() {
        groups.entry(state.1).or_default().push(index);
    }

    // Current AABBs for cross-lane collision checks.
    let mut grid: HashMap<(i32, i32), Vec<(usize, f32, f32, f32, f32)>> = HashMap::new();
    for (index, state) in states.iter().enumerate() {
        let lane = &lanes.lanes[state.1];
        let (point, _) = lane_sample(lane, state.2);
        let (minx, maxx, minz, maxz) = vehicle_aabb(point[0], point[1], state.5, state.3, state.4);
        let gx0 = (minx / 6.0).floor() as i32;
        let gx1 = (maxx / 6.0).floor() as i32;
        let gz0 = (minz / 6.0).floor() as i32;
        let gz1 = (maxz / 6.0).floor() as i32;
        for gx in gx0..=gx1 {
            for gz in gz0..=gz1 {
                grid.entry((gx, gz))
                    .or_default()
                    .push((index, minx, maxx, minz, maxz));
            }
        }
    }

    let mut wanted: HashMap<Entity, (f32, f32, f32)> = HashMap::new();
    for members in groups.values() {
        let mut ordered = members.clone();
        ordered.sort_by(|a, b| states[*b].2.total_cmp(&states[*a].2));
        for position in 0..ordered.len() {
            let index = ordered[position];
            let (entity, lane_index, s, length, width, yaw) = states[index];
            let lane = &lanes.lanes[lane_index];
            let front = lane.length - length * 0.5 - s;
            let gap = if position == 0 {
                front
            } else {
                let ahead = states[ordered[position - 1]];
                ahead.2 - ahead.3 * 0.5 - (s + length * 0.5)
            };
            let mut move_by = 0.0;
            if gap > 1.4 && now >= 0.0 {
                move_by = (gap - 1.2).min(0.18);
            }
            // Random stop/go: many vehicles simply sit for a fraction of a second.
            let tick = (entity.to_bits() as f32 * 0.37 + now * 0.83).sin() * 0.5 + 0.5;
            if tick < 0.45 {
                move_by = 0.0;
            }
            if move_by > 0.0 {
                let target_s = (s + move_by).min(lane.length - length * 0.5);
                let (point, _tangent) = lane_sample(lane, target_s);
                let (minx, maxx, minz, maxz) = vehicle_aabb(point[0], point[1], yaw, length, width);
                let mut blocked = false;
                let gx0 = (minx / 6.0).floor() as i32;
                let gx1 = (maxx / 6.0).floor() as i32;
                let gz0 = (minz / 6.0).floor() as i32;
                let gz1 = (maxz / 6.0).floor() as i32;
                'cells: for gx in gx0..=gx1 {
                    for gz in gz0..=gz1 {
                        for (other, ominx, omaxx, ominz, omaxz) in
                            grid.get(&(gx, gz)).into_iter().flatten()
                        {
                            if *other == index {
                                continue;
                            }
                            if minx < *omaxx && maxx > *ominx && minz < *omaxz && maxz > *ominz {
                                blocked = true;
                                break 'cells;
                            }
                        }
                    }
                }
                if !blocked {
                    wanted.insert(entity, (target_s, point[0], point[1]));
                }
            }
        }
    }

    for (entity, mut vehicle, mut transform) in &mut query {
        if let Some((s, x, z)) = wanted.get(&entity) {
            vehicle.s = *s;
            transform.translation.x = *x;
            transform.translation.z = *z;
            let lane = &lanes.lanes[vehicle.lane];
            let (_, tangent) = lane_sample(lane, *s);
            let yaw = tangent[0].atan2(tangent[1]) + vehicle.yaw_offset;
            transform.rotation = Quat::from_rotation_y(yaw);
        }
    }
}

pub fn update_vehicle_light_lod(
    time: Res<Time>,
    camera: Single<&Transform, With<FlyCamera>>,
    mut lights: Query<(&GlobalTransform, &mut Visibility), With<VehicleLight>>,
    mut last: Local<f32>,
) {
    let now = time.elapsed_secs();
    if now - *last < 0.2 {
        return;
    }
    *last = now;
    let radius_sq = VEHICLE_LIGHT_RADIUS * VEHICLE_LIGHT_RADIUS;
    let camera_position = camera.translation;
    for (transform, mut visibility) in &mut lights {
        let desired = if transform.translation().distance_squared(camera_position) <= radius_sq {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != desired {
            *visibility = desired;
        }
    }
}

pub fn update_vehicle_lights(
    env: Res<Environment>,
    catalog: Res<VehicleCatalog>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut last: Local<Option<bool>>,
) {
    let on = env.night || env.daylight < 0.35;
    if *last == Some(on) {
        return;
    }
    *last = Some(on);
    if let Some(mut material) = materials.get_mut(&catalog.lights_material) {
        material.emissive = if on {
            LinearRgba::rgb(2.2, 2.2, 2.2)
        } else {
            LinearRgba::BLACK
        };
    }
    if let Some(mut material) = materials.get_mut(&catalog.body_material) {
        // No uniform emissive tint: it washed every vehicle to the same grey.
        // At night the body is unlit so its vertex-coloured paint remains
        // readable and keeps each vehicle's own colour; by day it returns to
        // normal PBR lighting.
        material.emissive = LinearRgba::BLACK;
        material.unlit = on;
        material.base_color = if on {
            Color::srgb(0.72, 0.72, 0.72)
        } else {
            Color::WHITE
        };
    }
}
