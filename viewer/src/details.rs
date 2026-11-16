//! Small human-scale street detail: antennas, projecting shop signs and park
//! trees.  Everything here uses the tight 1,000 ft (304.8 m) detail cull.

use std::time::{SystemTime, UNIX_EPOCH};

use bevy::{
    asset::RenderAssetUsages,
    image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor},
    light::NotShadowCaster,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};
use serde::Deserialize;

use destructibles_core::{palette, roof};

use crate::{
    detail::{DetailCull, NEAR_DETAIL_RADIUS},
    storefront::{BuildingMeta, StorefrontData},
};

const TREE_DATA: &str = include_str!("../assets/models/trees.json");
const SIGN_GRID: f32 = 8.0;
const SIGN_TILE_UV: f32 = 1.0 / SIGN_GRID;

fn run_seed() -> u32 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u32)
            .unwrap_or(0x51A7_0001)
    }
    #[cfg(target_arch = "wasm32")]
    {
        (js_sys::Math::random() * u32::MAX as f64) as u32
    }
}

fn next_rand(seed: &mut u32) -> f32 {
    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    ((*seed >> 8) as f32) / ((u32::MAX >> 8) as f32)
}

#[derive(Clone, Copy, Deserialize)]
pub struct TreeSpec {
    x: f32,
    z: f32,
    scale: f32,
    variant: usize,
    rotation: f32,
}

#[derive(Deserialize)]
struct TreeFile {
    trees: Vec<TreeSpec>,
}

#[derive(Resource)]
pub struct TreeData {
    trees: Vec<TreeSpec>,
}

impl Default for TreeData {
    fn default() -> Self {
        Self {
            trees: serde_json::from_str::<TreeFile>(TREE_DATA)
                .expect("trees.json is invalid")
                .trees,
        }
    }
}

/// One advertisement plate, captured in world space at generation so the live
/// destructible can re-project the identical quad onto its own root.
#[derive(Clone, Debug)]
pub struct AdQuad {
    pub corners: [[f32; 3]; 4],
    pub normal: [f32; 3],
    pub uv: [[f32; 2]; 4],
    pub material: usize,
}

/// Plain-map rooftop furniture entities, hidden while a live destructible
/// occupies the footprint (the generated building carries its own recipe).
#[derive(Resource, Default)]
pub struct RoofFurniture {
    pub entities: std::collections::HashMap<usize, Vec<Entity>>,
}

#[derive(Resource, Default)]
pub struct SignMaterials {
    pub materials: Vec<Handle<StandardMaterial>>,
    blinking: Vec<Handle<StandardMaterial>>,
    phases: Vec<f32>,
    /// Ad plates per building (second floor and up), world space.
    pub plates: std::collections::HashMap<usize, Vec<AdQuad>>,
    /// Plain map sign meshes per building, hidden while a live destructible
    /// occupies the same footprint.
    pub entities: std::collections::HashMap<usize, Vec<Entity>>,
}

const SQFT_PER_M2: f32 = 10.7639;
const MAX_ANTENNAS_PER_BUILDING: usize = 64;

fn polygon_area(poly: &[[f32; 2]]) -> f32 {
    let mut area = 0.0;
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        area += a[0] * b[1] - b[0] * a[1];
    }
    area.abs() * 0.5
}

/// One antenna per floor per 10,000 sq ft of floor area, capped so a
/// supertall roof does not become a solid thicket.
fn antenna_count(building: &BuildingMeta) -> usize {
    let area = polygon_area(&building.perimeter);
    let floors = (building.height_m / building.floor_height_m.max(1.0))
        .round()
        .max(1.0);
    let count = (area * SQFT_PER_M2 * floors / 10_000.0).round() as usize;
    count.clamp(1, MAX_ANTENNAS_PER_BUILDING)
}

struct SignBucket {
    verts: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
    min: Vec3,
    max: Vec3,
}

impl Default for SignBucket {
    fn default() -> Self {
        Self {
            verts: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
            min: Vec3::splat(f32::INFINITY),
            max: Vec3::splat(f32::NEG_INFINITY),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn add_sign_quad(
    bucket: &mut SignBucket,
    plates: &mut Vec<AdQuad>,
    material: usize,
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
    d: [f32; 3],
    uv: [[f32; 2]; 4],
) {
    let ab = Vec3::from_array(b) - Vec3::from_array(a);
    let ac = Vec3::from_array(c) - Vec3::from_array(a);
    let n = ab.cross(ac).normalize_or_zero();
    let base = bucket.verts.len() as u32;
    bucket.verts.extend([a, b, c, d]);
    bucket.normals.extend([[n.x, n.y, n.z]; 4]);
    bucket.uvs.extend(uv);
    bucket
        .indices
        .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    for p in [a, b, c, d] {
        let v = Vec3::from_array(p);
        bucket.min = bucket.min.min(v);
        bucket.max = bucket.max.max(v);
    }
    plates.push(AdQuad {
        corners: [a, b, c, d],
        normal: [n.x, n.y, n.z],
        uv,
        material,
    });
}

fn point_in_poly(x: f32, z: f32, poly: &[[f32; 2]]) -> bool {
    let mut inside = false;
    let count = poly.len();
    let mut j = count - 1;
    for i in 0..count {
        let (xi, zi) = (poly[i][0], poly[i][1]);
        let (xj, zj) = (poly[j][0], poly[j][1]);
        if ((zi > z) != (zj > z)) && (x < (xj - xi) * (z - zi) / (zj - zi) + xi) {
            inside = !inside;
        }
        j = i;
    }
    inside
}

// ---------------------------------------------------------------------------
// mesh helpers (flat-shaded boxes / cones / spheres)
// ---------------------------------------------------------------------------
#[allow(clippy::too_many_arguments)]
fn push_quad(
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
    let normal = ab.cross(ac).normalize_or_zero();
    let base = verts.len() as u32;
    verts.extend([a, b, c, d]);
    normals.extend([[normal.x, normal.y, normal.z]; 4]);
    colors.extend([color; 4]);
    indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
}

fn add_box(
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
        (0, 1, 2, 3), // -z
        (5, 4, 7, 6), // +z
        (4, 0, 3, 7), // -x
        (1, 5, 6, 2), // +x
        (3, 2, 6, 7), // +y
        (4, 5, 1, 0), // -y
    ] {
        push_quad(
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

#[allow(clippy::too_many_arguments)]
fn add_cone(
    verts: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    colors: &mut Vec<[f32; 4]>,
    indices: &mut Vec<u32>,
    base_y: f32,
    radius: f32,
    height: f32,
    color: [f32; 4],
) {
    let apex = [0.0, base_y + height, 0.0];
    let segments = 8usize;
    for i in 0..segments {
        let a0 = i as f32 / segments as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / segments as f32 * std::f32::consts::TAU;
        let p0 = [a0.cos() * radius, base_y, a0.sin() * radius];
        let p1 = [a1.cos() * radius, base_y, a1.sin() * radius];
        let base = verts.len() as u32;
        let ab = Vec3::from_array(p0) - Vec3::from_array(apex);
        let ac = Vec3::from_array(p1) - Vec3::from_array(apex);
        let normal = ab.cross(ac).normalize_or_zero();
        verts.extend([apex, p0, p1]);
        normals.extend([[normal.x, normal.y, normal.z]; 3]);
        colors.extend([color; 3]);
        indices.extend([base, base + 1, base + 2]);
    }
}

fn add_sphere(
    verts: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    colors: &mut Vec<[f32; 4]>,
    indices: &mut Vec<u32>,
    center: Vec3,
    radius: f32,
    color: [f32; 4],
) {
    let rings = 4usize;
    let segments = 8usize;
    let mut points = Vec::new();
    for r in 0..=rings {
        let phi = std::f32::consts::PI * r as f32 / rings as f32;
        let y = phi.cos() * radius;
        let rr = phi.sin() * radius;
        let mut ring = Vec::new();
        for s in 0..segments {
            let theta = s as f32 / segments as f32 * std::f32::consts::TAU;
            ring.push(center + Vec3::new(theta.cos() * rr, y, theta.sin() * rr));
        }
        points.push(ring);
    }
    for r in 0..rings {
        for s in 0..segments {
            let s1 = (s + 1) % segments;
            let a = points[r][s];
            let b = points[r][s1];
            let c = points[r + 1][s];
            let d = points[r + 1][s1];
            if r == 0 {
                push_quad(
                    verts,
                    normals,
                    colors,
                    indices,
                    a.to_array(),
                    b.to_array(),
                    c.to_array(),
                    c.to_array(),
                    color,
                );
            } else if r + 1 == rings {
                push_quad(
                    verts,
                    normals,
                    colors,
                    indices,
                    a.to_array(),
                    b.to_array(),
                    d.to_array(),
                    d.to_array(),
                    color,
                );
            } else {
                push_quad(
                    verts,
                    normals,
                    colors,
                    indices,
                    a.to_array(),
                    b.to_array(),
                    d.to_array(),
                    c.to_array(),
                    color,
                );
            }
        }
    }
}

fn tree_mesh(variant: usize) -> Mesh {
    let mut verts = Vec::new();
    let mut normals = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::new();
    let trunk = [0.24, 0.16, 0.10, 1.0];
    let leaf = match variant {
        1 => [0.12, 0.30, 0.14, 1.0],
        2 => [0.16, 0.38, 0.18, 1.0],
        _ => [0.20, 0.42, 0.20, 1.0],
    };
    match variant {
        1 => {
            add_box(
                &mut verts,
                &mut normals,
                &mut colors,
                &mut indices,
                Vec3::new(0.0, 1.1, 0.0),
                Vec3::new(0.16, 1.1, 0.16),
                trunk,
            );
            add_cone(
                &mut verts,
                &mut normals,
                &mut colors,
                &mut indices,
                1.4,
                2.0,
                4.4,
                leaf,
            );
        }
        2 => {
            add_box(
                &mut verts,
                &mut normals,
                &mut colors,
                &mut indices,
                Vec3::new(0.0, 0.5, 0.0),
                Vec3::new(0.14, 0.5, 0.14),
                trunk,
            );
            add_sphere(
                &mut verts,
                &mut normals,
                &mut colors,
                &mut indices,
                Vec3::new(0.0, 1.7, 0.0),
                1.25,
                leaf,
            );
        }
        _ => {
            add_box(
                &mut verts,
                &mut normals,
                &mut colors,
                &mut indices,
                Vec3::new(0.0, 1.5, 0.0),
                Vec3::new(0.18, 1.5, 0.18),
                trunk,
            );
            add_sphere(
                &mut verts,
                &mut normals,
                &mut colors,
                &mut indices,
                Vec3::new(0.0, 4.0, 0.0),
                2.1,
                leaf,
            );
        }
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, verts)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_indices(Indices::U32(indices))
}

// ---------------------------------------------------------------------------
// startup systems
// ---------------------------------------------------------------------------
pub fn spawn_trees(
    mut commands: Commands,
    data: Res<TreeData>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.92,
        ..default()
    });
    let variants = [tree_mesh(0), tree_mesh(1), tree_mesh(2)].map(|mesh| meshes.add(mesh));
    for (index, tree) in data.trees.iter().enumerate() {
        let mesh = variants[tree.variant.min(2)].clone();
        let scale = tree.scale.max(0.5);
        let translation = Vec3::new(tree.x, 0.03, tree.z);
        let half = 3.0 * scale;
        commands.spawn((
            NotShadowCaster,
            Name::new(format!("Tree_{index:04}")),
            Mesh3d(mesh),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(translation)
                .with_rotation(Quat::from_rotation_y(tree.rotation))
                .with_scale(Vec3::splat(scale)),
            DetailCull::new(
                translation - Vec3::new(half, 0.0, half),
                translation + Vec3::new(half, 7.0 * scale, half),
                NEAR_DETAIL_RADIUS,
            ),
        ));
    }
    info!("Trees: {}", data.trees.len());
}

pub fn spawn_antennas(
    mut commands: Commands,
    data: Res<StorefrontData>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut furniture: ResMut<RoofFurniture>,
) {
    furniture.entities.clear();
    let equipment = {
        let p = palette::EQUIPMENT;
        materials.add(StandardMaterial {
            base_color: Color::srgb(p[0], p[1], p[2]),
            perceptual_roughness: 0.78,
            metallic: 0.15,
            ..default()
        })
    };
    let steel = {
        let p = palette::ANTENNA;
        materials.add(StandardMaterial {
            base_color: Color::srgb(p[0], p[1], p[2]),
            metallic: 0.7,
            perceptual_roughness: 0.42,
            ..default()
        })
    };
    let mut total_boxes = 0usize;
    let mut total_antennas = 0usize;
    for building in data.buildings() {
        if building.perimeter.len() < 3 || building.height_m < 4.0 {
            continue;
        }
        let mut spawned = Vec::new();
        let boxes =
            roof::roof_equipment(building.id as u32, &building.perimeter, building.height_m);
        if !boxes.is_empty() {
            let mut verts = Vec::new();
            let mut normals = Vec::new();
            let mut uvs = Vec::new();
            let mut indices = Vec::new();
            for unit in &boxes {
                push_box(
                    &mut verts,
                    &mut normals,
                    &mut uvs,
                    &mut indices,
                    Vec3::new(
                        unit.center[0],
                        building.height_m + unit.size[1] * 0.5,
                        unit.center[1],
                    ),
                    Vec3::new(unit.size[0] * 0.5, unit.size[1] * 0.5, unit.size[2] * 0.5),
                    Quat::from_rotation_y(unit.yaw),
                );
            }
            let (min, max) = mesh_bounds(&verts);
            spawned.push(
                commands
                    .spawn((
                        NotShadowCaster,
                        Name::new(format!("RoofEquipment_{:05}", building.id)),
                        Mesh3d(meshes.add(simple_mesh(verts, normals, uvs, indices))),
                        MeshMaterial3d(equipment.clone()),
                        Transform::IDENTITY,
                        DetailCull::new(min, max, NEAR_DETAIL_RADIUS),
                    ))
                    .id(),
            );
            total_boxes += boxes.len();
        }
        let masts = roof::antenna_heights(building.id as u32, building.height_m);
        if !masts.is_empty() {
            let mut seed = building.id as u32 ^ 0xA17E_55AA;
            let mut verts = Vec::new();
            let mut normals = Vec::new();
            let mut uvs = Vec::new();
            let mut indices = Vec::new();
            for mast in masts {
                let x = building.perimeter[0][0];
                let z = building.perimeter[0][1];
                let _ = (x, z, seed);
                let point = deterministic_inside(&building.perimeter, &mut seed);
                let base = building.height_m;
                push_box(
                    &mut verts,
                    &mut normals,
                    &mut uvs,
                    &mut indices,
                    Vec3::new(point[0], base + mast * 0.5, point[1]),
                    Vec3::new(0.09, mast * 0.5, 0.09),
                    Quat::IDENTITY,
                );
                total_antennas += 1;
            }
            let (min, max) = mesh_bounds(&verts);
            spawned.push(
                commands
                    .spawn((
                        NotShadowCaster,
                        Name::new(format!("RoofAntennas_{:05}", building.id)),
                        Mesh3d(meshes.add(simple_mesh(verts, normals, uvs, indices))),
                        MeshMaterial3d(steel.clone()),
                        Transform::IDENTITY,
                        DetailCull::new(min, max, NEAR_DETAIL_RADIUS),
                    ))
                    .id(),
            );
        }
        if !spawned.is_empty() {
            furniture.entities.insert(building.id, spawned);
        }
    }
    info!(
        "Roof furniture: {total_boxes} equipment boxes, {total_antennas} masts on {} buildings",
        furniture.entities.len()
    );
}

fn deterministic_inside(poly: &[[f32; 2]], seed: &mut u32) -> [f32; 2] {
    let min_x = poly.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min);
    let max_x = poly.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max);
    let min_z = poly.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min);
    let max_z = poly.iter().map(|p| p[1]).fold(f32::NEG_INFINITY, f32::max);
    for _ in 0..48 {
        let x = min_x + next_rand(seed) * (max_x - min_x);
        let z = min_z + next_rand(seed) * (max_z - min_z);
        if point_in_poly(x, z, poly) {
            return [x, z];
        }
    }
    [(min_x + max_x) * 0.5, (min_z + max_z) * 0.5]
}

fn simple_mesh(
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
) -> Mesh {
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

fn mesh_bounds(verts: &[[f32; 3]]) -> (Vec3, Vec3) {
    let min = verts.iter().fold(Vec3::splat(f32::INFINITY), |m, v| {
        m.min(Vec3::from_array(*v))
    });
    let max = verts.iter().fold(Vec3::splat(f32::NEG_INFINITY), |m, v| {
        m.max(Vec3::from_array(*v))
    });
    (min, max)
}

#[allow(clippy::too_many_arguments)]
fn push_box(
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    uvs: &mut Vec<[f32; 2]>,
    indices: &mut Vec<u32>,
    centre: Vec3,
    half: Vec3,
    rotation: Quat,
) {
    let corners = [
        Vec3::new(-half.x, -half.y, half.z),
        Vec3::new(half.x, -half.y, half.z),
        Vec3::new(half.x, half.y, half.z),
        Vec3::new(-half.x, half.y, half.z),
        Vec3::new(-half.x, -half.y, -half.z),
        Vec3::new(half.x, -half.y, -half.z),
        Vec3::new(half.x, half.y, -half.z),
        Vec3::new(-half.x, half.y, -half.z),
    ];
    let normal = [
        [0.0, 0.0, 1.0],
        [0.0, 0.0, -1.0],
        [1.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, -1.0, 0.0],
    ];
    let faces = [
        [0, 1, 2, 3],
        [5, 4, 7, 6],
        [1, 5, 6, 2],
        [4, 0, 3, 7],
        [3, 2, 6, 7],
        [4, 5, 1, 0],
    ];
    for (face, n) in faces.iter().zip(normal) {
        let base = positions.len() as u32;
        let rotated = rotation * Vec3::from_array(n);
        for index in face {
            positions.push((rotation * corners[*index] + centre).to_array());
            normals.push(rotated.to_array());
            uvs.push([0.0, 0.0]);
        }
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
}

pub fn spawn_signs(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    data: Res<StorefrontData>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut sign_materials: ResMut<SignMaterials>,
) {
    let texture = asset_server
        .load_builder()
        .with_settings(|s: &mut ImageLoaderSettings| {
            s.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                address_mode_u: ImageAddressMode::ClampToEdge,
                address_mode_v: ImageAddressMode::ClampToEdge,
                ..default()
            });
        })
        .load("textures/signage/signage_atlas.jpg".to_string());
    let static_material = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(texture.clone()),
        unlit: true,
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    let lit_material = materials.add(StandardMaterial {
        base_color: Color::BLACK,
        base_color_texture: Some(texture.clone()),
        emissive: LinearRgba::rgb(2.2, 2.2, 2.2),
        emissive_texture: Some(texture.clone()),
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    sign_materials.blinking.clear();
    sign_materials.phases.clear();
    sign_materials.plates.clear();
    sign_materials.entities.clear();
    for phase in 0..6 {
        sign_materials.phases.push(phase as f32 / 6.0);
        sign_materials
            .blinking
            .push(materials.add(StandardMaterial {
                base_color: Color::BLACK,
                base_color_texture: Some(texture.clone()),
                emissive: LinearRgba::rgb(2.4, 2.4, 2.4),
                emissive_texture: Some(texture.clone()),
                double_sided: true,
                cull_mode: None,
                ..default()
            }));
    }
    sign_materials.materials.clear();
    sign_materials.materials.push(static_material);
    sign_materials.materials.push(lit_material);
    let blinking_handles = sign_materials.blinking.clone();
    sign_materials.materials.extend(blinking_handles);
    let material_handles = sign_materials.materials.clone();

    let mut seed = run_seed() ^ 0x516E_5EED;
    let mut total = 0usize;
    let mut buildings = 0usize;
    for building in data.buildings() {
        if building.perimeter.len() < 4 {
            continue;
        }
        let antenna_count = antenna_count(building);
        let density = crate::city_rhythm::ad_density(building.centroid[0], building.centroid[1]);
        if density < 0.02 {
            continue;
        }
        let sign_count = ((antenna_count * 4) as f32 * density).round() as usize;
        let cx =
            building.perimeter.iter().map(|p| p[0]).sum::<f32>() / building.perimeter.len() as f32;
        let cz =
            building.perimeter.iter().map(|p| p[1]).sum::<f32>() / building.perimeter.len() as f32;
        let mut buckets: Vec<SignBucket> = (0..material_handles.len())
            .map(|_| SignBucket::default())
            .collect();
        let mut plates: Vec<AdQuad> = Vec::new();
        let mut placed = 0usize;

        for _ in 0..sign_count {
            let mut candidates: Vec<(usize, f32)> = building
                .perimeter
                .iter()
                .enumerate()
                .map(|(i, a)| {
                    let b = building.perimeter[(i + 1) % building.perimeter.len()];
                    (i, ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt())
                })
                .filter(|(_, length)| *length > 4.0)
                .collect();
            if candidates.is_empty() {
                break;
            }
            let pick = (next_rand(&mut seed) * candidates.len() as f32) as usize;
            let (edge, length) = candidates.swap_remove(pick.min(candidates.len() - 1));
            let a = Vec2::from(building.perimeter[edge]);
            let b = Vec2::from(building.perimeter[(edge + 1) % building.perimeter.len()]);
            let direction = (b - a) / length;
            let mut outward = Vec2::new(direction.y, -direction.x);
            let midpoint = a + (b - a) * 0.5;
            if outward.dot(midpoint - Vec2::new(cx, cz)) < 0.0 {
                outward = -outward;
            }
            let y0 = building.floor_height_m + 0.8;
            if building.height_m < y0 + 0.5 {
                continue;
            }
            let kind = next_rand(&mut seed);
            let tile = (next_rand(&mut seed) * 64.0) as usize % 64;
            let col = (tile % 8) as f32;
            let row = (tile / 8) as f32;
            let u0 = col * SIGN_TILE_UV;
            let v0 = row * SIGN_TILE_UV;
            let u1 = u0 + SIGN_TILE_UV;
            let v1 = v0 + SIGN_TILE_UV;
            let uv = [[u0, v1], [u1, v1], [u1, v0], [u0, v0]];

            let roll = next_rand(&mut seed);
            let bucket = if roll < 0.25 {
                1
            } else if roll < 0.50 {
                0
            } else {
                let span = material_handles.len().saturating_sub(2).max(1);
                2 + ((next_rand(&mut seed) * span as f32) as usize).min(span - 1)
            };

            if kind < 0.12 {
                // Vertical banner running up a building corner.
                let count = building.perimeter.len();
                let corner = (next_rand(&mut seed) * count as f32) as usize % count;
                let p = Vec2::from(building.perimeter[corner]);
                let prev = Vec2::from(building.perimeter[(corner + count - 1) % count]);
                let next = Vec2::from(building.perimeter[(corner + 1) % count]);
                let e1 = (p - prev).normalize_or_zero();
                let e2 = (next - p).normalize_or_zero();
                let n1 = Vec2::new(e1.y, -e1.x);
                let n2 = Vec2::new(e2.y, -e2.x);
                let mut miter = (n1 + n2).normalize_or_zero();
                if miter.dot(p - Vec2::new(cx, cz)) < 0.0 {
                    miter = -miter;
                }
                let tangent = Vec2::new(miter.y, -miter.x);
                let width = 1.0 + next_rand(&mut seed) * 0.9;
                let height =
                    (building.height_m * (0.10 + next_rand(&mut seed) * 0.20)).clamp(3.0, 16.0);
                let y1 = (y0 + height).min(building.height_m - 0.2);
                if y1 <= y0 + 1.2 {
                    continue;
                }
                let base = p + miter * 0.16;
                let w = tangent * (width * 0.5);
                add_sign_quad(
                    &mut buckets[bucket],
                    &mut plates,
                    bucket,
                    [base.x - w.x, y0, base.y - w.y],
                    [base.x + w.x, y0, base.y + w.y],
                    [base.x + w.x, y1, base.y + w.y],
                    [base.x - w.x, y1, base.y - w.y],
                    uv,
                );
            } else if kind < 0.62 {
                let t = 0.20 + next_rand(&mut seed) * 0.60;
                let wall = a + (b - a) * t + outward * 0.12;
                let projection = 2.4 + next_rand(&mut seed) * 1.6;
                let height = projection * (0.50 + next_rand(&mut seed) * 0.16);
                let tip = wall + outward * projection;
                add_sign_quad(
                    &mut buckets[bucket],
                    &mut plates,
                    bucket,
                    [wall.x, y0, wall.y],
                    [tip.x, y0, tip.y],
                    [tip.x, y0 + height, tip.y],
                    [wall.x, y0 + height, wall.y],
                    uv,
                );
            } else {
                let width = (2.0 + next_rand(&mut seed) * 2.0).min(length * 0.8);
                let height = 0.9 + next_rand(&mut seed) * 1.1;
                let half = width / (2.0 * length);
                let t = (0.18 + next_rand(&mut seed) * 0.64).clamp(half + 0.02, 0.98 - half);
                let t0 = (t - half).max(0.02);
                let t1 = (t + half).min(0.98);
                let p0 = a + (b - a) * t0 + outward * 0.14;
                let p1 = a + (b - a) * t1 + outward * 0.14;
                add_sign_quad(
                    &mut buckets[bucket],
                    &mut plates,
                    bucket,
                    [p0.x, y0, p0.y],
                    [p1.x, y0, p1.y],
                    [p1.x, y0 + height, p1.y],
                    [p0.x, y0 + height, p0.y],
                    uv,
                );
            }
            placed += 1;
        }

        if placed == 0 {
            continue;
        }
        let mut spawned: Vec<Entity> = Vec::new();
        for (index, bucket) in buckets.into_iter().enumerate() {
            if bucket.indices.is_empty() {
                continue;
            }
            let mesh = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
            )
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, bucket.verts)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, bucket.normals)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, bucket.uvs)
            .with_inserted_indices(Indices::U32(bucket.indices));
            spawned.push(
                commands
                    .spawn((
                        NotShadowCaster,
                        Name::new(format!("Signs_{:05}_{index}", building.id)),
                        Mesh3d(meshes.add(mesh)),
                        MeshMaterial3d(material_handles[index].clone()),
                        Transform::IDENTITY,
                        DetailCull::new(bucket.min, bucket.max, NEAR_DETAIL_RADIUS),
                    ))
                    .id(),
            );
        }
        sign_materials.entities.insert(building.id, spawned);
        sign_materials.plates.insert(building.id, plates);
        total += placed;
        buildings += 1;
    }
    info!("Signs: {total} on {buildings} buildings");
}

pub fn update_sign_blink(
    time: Res<Time>,
    sign_materials: Res<SignMaterials>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut last: Local<Option<Vec<bool>>>,
) {
    let mut states = Vec::with_capacity(sign_materials.blinking.len());
    for index in 0..sign_materials.blinking.len() {
        let phase = sign_materials.phases.get(index).copied().unwrap_or(0.0);
        states.push((time.elapsed_secs() * 1.15 + phase) % 1.0 < 0.64);
    }
    if last.as_ref() == Some(&states) {
        return;
    }
    *last = Some(states.clone());
    for (index, handle) in sign_materials.blinking.iter().enumerate() {
        let on = states[index];
        if let Some(mut material) = materials.get_mut(handle) {
            material.emissive = if on {
                LinearRgba::rgb(2.6, 2.6, 2.6)
            } else {
                LinearRgba::BLACK
            };
        }
    }
}
