//! Streamed streetlamps and intersection traffic signals.
//!
//! Lamps are generated along building perimeters within the near radius; signal
//! heads are placed at the exported intersections.  Both are despawned as the
//! camera leaves the area, so the city does not retain furniture for the whole
//! map.

use std::collections::HashMap;

use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};
use serde::Deserialize;

use crate::{camera::FlyCamera, environment::Environment, storefront::StorefrontData};

const INTERSECTIONS: &str = include_str!("../assets/models/intersections.json");

#[cfg(target_arch = "wasm32")]
const FURNITURE_RADIUS: f32 = 240.0;
#[cfg(not(target_arch = "wasm32"))]
const FURNITURE_RADIUS: f32 = 330.0;

const LAMP_SPACING: f32 = 24.0;
const LAMP_OFFSET: f32 = 2.8;

#[derive(Deserialize, Clone, Copy)]
struct IntersectionRecord {
    x: f32,
    z: f32,
    yaw: f32,
    half: f32,
}

#[derive(Deserialize)]
struct IntersectionFile {
    intersections: Vec<IntersectionRecord>,
}

#[derive(Resource)]
pub struct Intersections {
    items: Vec<IntersectionRecord>,
}

impl Default for Intersections {
    fn default() -> Self {
        let parsed: IntersectionFile =
            serde_json::from_str(INTERSECTIONS).expect("intersections.json is invalid");
        Self {
            items: parsed.intersections,
        }
    }
}

#[derive(Resource)]
pub struct StreetAssets {
    lamp_pole: Handle<Mesh>,
    lamp_light: Handle<Mesh>,
    signal_pole: Handle<Mesh>,
    signal_lens: Handle<Mesh>,
    pole_material: Handle<StandardMaterial>,
    lamp_material: Handle<StandardMaterial>,
    red_material: Handle<StandardMaterial>,
    amber_material: Handle<StandardMaterial>,
    green_material: Handle<StandardMaterial>,
}

#[derive(Resource, Default)]
pub struct StreetFurniture {
    lamp_buildings: HashMap<usize, Vec<Entity>>,
    signals: HashMap<usize, Vec<Entity>>,
    last_check: f32,
}

fn box_mesh(mut verts: Vec<[f32; 3]>) -> Mesh {
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    let mut all = Vec::new();
    for chunk in verts.chunks(4) {
        if chunk.len() != 4 {
            continue;
        }
        let a = chunk[0];
        let b = chunk[1];
        let c = chunk[2];
        let d = chunk[3];
        let ab = Vec3::from_array(b) - Vec3::from_array(a);
        let ac = Vec3::from_array(c) - Vec3::from_array(a);
        let n = ab.cross(ac).normalize_or_zero();
        let base = all.len() as u32;
        all.extend([a, b, c, d]);
        normals.extend([[n.x, n.y, n.z]; 4]);
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    verts.clear();
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, all)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_indices(Indices::U32(indices))
}

fn push_box(verts: &mut Vec<[f32; 3]>, center: Vec3, half: Vec3) {
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
        verts.extend([
            p[a].to_array(),
            p[b].to_array(),
            p[cc].to_array(),
            p[d].to_array(),
        ]);
    }
}

pub fn setup_street_furniture(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let pole_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.22, 0.23, 0.25),
        metallic: 0.65,
        perceptual_roughness: 0.48,
        ..default()
    });
    let lamp_material = materials.add(StandardMaterial {
        base_color: Color::BLACK,
        emissive: LinearRgba::BLACK,
        perceptual_roughness: 0.35,
        ..default()
    });
    let red_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.25, 0.04, 0.03),
        emissive: LinearRgba::BLACK,
        perceptual_roughness: 0.4,
        ..default()
    });
    let amber_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.22, 0.13, 0.03),
        emissive: LinearRgba::BLACK,
        perceptual_roughness: 0.4,
        ..default()
    });
    let green_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.03, 0.20, 0.09),
        emissive: LinearRgba::BLACK,
        perceptual_roughness: 0.4,
        ..default()
    });

    let mut lamp_verts = Vec::new();
    push_box(
        &mut lamp_verts,
        Vec3::new(0.0, 3.3, 0.0),
        Vec3::new(0.075, 3.3, 0.075),
    );
    push_box(
        &mut lamp_verts,
        Vec3::new(0.0, 6.25, 1.05),
        Vec3::new(0.055, 0.055, 1.05),
    );
    push_box(
        &mut lamp_verts,
        Vec3::new(0.0, 6.12, 2.15),
        Vec3::new(0.28, 0.13, 0.34),
    );
    let lamp_pole = meshes.add(box_mesh(lamp_verts));
    let lamp_light = meshes.add(box_mesh({
        let mut v = Vec::new();
        push_box(
            &mut v,
            Vec3::new(0.0, 5.98, 2.15),
            Vec3::new(0.20, 0.025, 0.25),
        );
        v
    }));

    let mut signal_verts = Vec::new();
    push_box(
        &mut signal_verts,
        Vec3::new(0.0, 2.6, 0.0),
        Vec3::new(0.075, 2.6, 0.075),
    );
    push_box(
        &mut signal_verts,
        Vec3::new(0.0, 4.7, 0.16),
        Vec3::new(0.32, 0.55, 0.22),
    );
    let signal_pole = meshes.add(box_mesh(signal_verts));
    let signal_lens = meshes.add(box_mesh({
        let mut v = Vec::new();
        push_box(&mut v, Vec3::ZERO, Vec3::new(0.16, 0.10, 0.025));
        v
    }));

    commands.insert_resource(StreetAssets {
        lamp_pole,
        lamp_light,
        signal_pole,
        signal_lens,
        pole_material,
        lamp_material,
        red_material,
        amber_material,
        green_material,
    });
}

fn loop_point(poly: &[[f32; 2]], target: f32) -> ([f32; 2], [f32; 2]) {
    let mut travelled = 0.0;
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        let length = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        if travelled + length >= target && length > 1e-6 {
            let t = ((target - travelled) / length).clamp(0.0, 1.0);
            let point = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
            let tangent = [(b[0] - a[0]) / length, (b[1] - a[1]) / length];
            return (point, tangent);
        }
        travelled += length;
    }
    (poly[0], [0.0, 1.0])
}

fn spawn_lamp(
    commands: &mut Commands,
    assets: &StreetAssets,
    point: [f32; 2],
    outward: [f32; 2],
) -> Entity {
    let yaw = outward[0].atan2(outward[1]);
    let root = commands
        .spawn((
            Name::new("StreetLamp"),
            Mesh3d(assets.lamp_pole.clone()),
            MeshMaterial3d(assets.pole_material.clone()),
            Transform::from_xyz(point[0], 0.06, point[1]).with_rotation(Quat::from_rotation_y(yaw)),
            Visibility::Inherited,
        ))
        .id();
    commands.entity(root).with_children(|parent| {
        parent.spawn((
            Mesh3d(assets.lamp_light.clone()),
            MeshMaterial3d(assets.lamp_material.clone()),
            Transform::IDENTITY,
        ));
    });
    root
}

fn spawn_signal(
    commands: &mut Commands,
    assets: &StreetAssets,
    point: [f32; 2],
    yaw: f32,
) -> Entity {
    let root = commands
        .spawn((
            Name::new("TrafficSignal"),
            Mesh3d(assets.signal_pole.clone()),
            MeshMaterial3d(assets.pole_material.clone()),
            Transform::from_xyz(point[0], 0.06, point[1]).with_rotation(Quat::from_rotation_y(yaw)),
            Visibility::Inherited,
        ))
        .id();
    commands.entity(root).with_children(|parent| {
        for (index, material) in [
            assets.red_material.clone(),
            assets.amber_material.clone(),
            assets.green_material.clone(),
        ]
        .into_iter()
        .enumerate()
        {
            parent.spawn((
                Mesh3d(assets.signal_lens.clone()),
                MeshMaterial3d(material),
                Transform::from_xyz(0.0, 4.98 - index as f32 * 0.27, 0.405),
            ));
        }
    });
    root
}

pub fn stream_street_furniture(
    mut commands: Commands,
    time: Res<Time>,
    camera: Single<&Transform, With<FlyCamera>>,
    data: Res<StorefrontData>,
    intersections: Res<Intersections>,
    assets: Res<StreetAssets>,
    mut furniture: ResMut<StreetFurniture>,
) {
    let now = time.elapsed_secs();
    if now - furniture.last_check < 0.5 {
        return;
    }
    furniture.last_check = now;
    let cam = camera.translation;

    let mut near_buildings = Vec::new();
    for (id, building) in data.buildings().iter().enumerate() {
        let dx = building.centroid[0] - cam.x;
        let dz = building.centroid[1] - cam.z;
        if (dx * dx + dz * dz).sqrt() <= FURNITURE_RADIUS {
            near_buildings.push(id);
        }
    }
    let near_set: std::collections::HashSet<usize> = near_buildings.iter().copied().collect();
    let far: Vec<usize> = furniture
        .lamp_buildings
        .keys()
        .copied()
        .filter(|id| !near_set.contains(id))
        .collect();
    for id in far {
        if let Some(entities) = furniture.lamp_buildings.remove(&id) {
            for entity in entities {
                commands.entity(entity).despawn();
            }
        }
    }
    for id in near_buildings {
        if furniture.lamp_buildings.contains_key(&id) {
            continue;
        }
        let building = &data.buildings()[id];
        let perimeter: f32 = (0..building.perimeter.len())
            .map(|i| {
                let a = building.perimeter[i];
                let b = building.perimeter[(i + 1) % building.perimeter.len()];
                ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()
            })
            .sum();
        if perimeter < 5.0 {
            continue;
        }
        let mut entities = Vec::new();
        let mut distance = 8.0;
        while distance < perimeter {
            let (point, tangent) = loop_point(&building.perimeter, distance);
            let outward = [tangent[1], -tangent[0]];
            let position = [
                point[0] + outward[0] * LAMP_OFFSET,
                point[1] + outward[1] * LAMP_OFFSET,
            ];
            entities.push(spawn_lamp(&mut commands, &assets, position, outward));
            distance += LAMP_SPACING;
        }
        if !entities.is_empty() {
            furniture.lamp_buildings.insert(id, entities);
        }
    }

    let mut near_signals = Vec::new();
    for (index, signal) in intersections.items.iter().enumerate() {
        let dx = signal.x - cam.x;
        let dz = signal.z - cam.z;
        if (dx * dx + dz * dz).sqrt() <= FURNITURE_RADIUS {
            near_signals.push(index);
        }
    }
    let signal_set: std::collections::HashSet<usize> = near_signals.iter().copied().collect();
    let far_signals: Vec<usize> = furniture
        .signals
        .keys()
        .copied()
        .filter(|id| !signal_set.contains(id))
        .collect();
    for id in far_signals {
        if let Some(entities) = furniture.signals.remove(&id) {
            for entity in entities {
                commands.entity(entity).despawn();
            }
        }
    }
    for index in near_signals {
        if furniture.signals.contains_key(&index) {
            continue;
        }
        let signal = intersections.items[index];
        let yaw = signal.yaw;
        let perp = [yaw.cos(), -yaw.sin()];
        let mut entities = Vec::new();
        for (sign, offset) in [(-1.0, signal.half + 1.8), (1.0, signal.half + 1.8)] {
            let point = [
                signal.x + perp[0] * offset * sign,
                signal.z + perp[1] * offset * sign,
            ];
            entities.push(spawn_signal(
                &mut commands,
                &assets,
                point,
                yaw + if sign > 0.0 {
                    std::f32::consts::PI
                } else {
                    0.0
                },
            ));
        }
        furniture.signals.insert(index, entities);
    }
}

pub fn update_street_lights(
    time: Res<Time>,
    env: Res<Environment>,
    assets: Res<StreetAssets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut last: Local<Option<(bool, u8)>>,
) {
    let night = env.night || env.daylight < 0.35;
    let cycle = time.elapsed_secs() % 14.0;
    let phase = if cycle < 6.0 {
        0
    } else if cycle < 11.0 {
        1
    } else {
        2
    };
    if *last == Some((night, phase)) {
        return;
    }
    *last = Some((night, phase));
    if let Some(mut material) = materials.get_mut(&assets.lamp_material) {
        material.emissive = if night {
            LinearRgba::rgb(4.2, 3.0, 1.35)
        } else {
            LinearRgba::BLACK
        };
    }
    let (red, amber, green) = match phase {
        0 => (true, false, false),
        1 => (false, false, true),
        _ => (false, true, false),
    };
    let active = [
        (assets.red_material.clone(), red, (0.95, 0.06, 0.04)),
        (assets.amber_material.clone(), amber, (0.95, 0.55, 0.05)),
        (assets.green_material.clone(), green, (0.05, 0.85, 0.35)),
    ];
    for (handle, on, color) in active {
        if let Some(mut material) = materials.get_mut(&handle) {
            material.base_color = if on {
                Color::srgb(color.0, color.1, color.2)
            } else {
                Color::srgb(color.0 * 0.12, color.1 * 0.12, color.2 * 0.12)
            };
            material.emissive = if on && night {
                LinearRgba::rgb(color.0 * 3.0, color.1 * 3.0, color.2 * 3.0)
            } else {
                LinearRgba::BLACK
            };
        }
    }
}
