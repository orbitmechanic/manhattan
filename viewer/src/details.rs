//! Small human-scale street detail: antennas, projecting shop signs and park
//! trees.  Everything here uses the tight 1,000 ft (304.8 m) detail cull.

use std::time::{SystemTime, UNIX_EPOCH};

use bevy::{
    asset::RenderAssetUsages,
    image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor},
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};
use serde::Deserialize;

use crate::{
    detail::{DetailCull, NEAR_DETAIL_RADIUS},
    storefront::StorefrontData,
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

#[derive(Resource, Default)]
pub struct SignMaterials {
    blinking: Vec<Handle<StandardMaterial>>,
    phases: Vec<f32>,
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

fn random_inside(poly: &[[f32; 2]], seed: &mut u32) -> [f32; 2] {
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

fn antenna_mesh() -> Mesh {
    let mut verts = Vec::new();
    let mut normals = Vec::new();
    let mut colors = Vec::new();
    let mut indices = Vec::new();
    let metal = [0.36, 0.37, 0.39, 1.0];
    add_box(
        &mut verts,
        &mut normals,
        &mut colors,
        &mut indices,
        Vec3::new(0.0, 0.5, 0.0),
        Vec3::new(0.022, 0.5, 0.022),
        metal,
    );
    for (y, half) in [(0.70, 0.13), (0.84, 0.09)] {
        add_box(
            &mut verts,
            &mut normals,
            &mut colors,
            &mut indices,
            Vec3::new(0.0, y, 0.0),
            Vec3::new(half, 0.008, 0.008),
            metal,
        );
    }
    add_box(
        &mut verts,
        &mut normals,
        &mut colors,
        &mut indices,
        Vec3::new(0.0, 0.97, 0.0),
        Vec3::new(0.03, 0.03, 0.03),
        metal,
    );
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
) {
    let material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.30, 0.31, 0.33),
        metallic: 0.65,
        perceptual_roughness: 0.42,
        ..default()
    });
    let mesh = meshes.add(antenna_mesh());
    let mut seed = run_seed() ^ 0xA17E_55AA;
    let mut count = 0usize;
    for building in data.buildings() {
        if building.height_m < 8.0 || building.perimeter.len() < 3 {
            continue;
        }
        let chance = if matches!(building.category.as_str(), "executive" | "industrial") {
            0.45
        } else {
            0.30
        };
        if next_rand(&mut seed) > chance {
            continue;
        }
        let [x, z] = random_inside(&building.perimeter, &mut seed);
        let height = building.height_m * 0.08;
        let roof_y = building.height_m;
        let translation = Vec3::new(x, roof_y, z);
        let half = height * 0.18;
        commands.spawn((
            Name::new(format!("Antenna_{:05}", building.id)),
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(translation).with_scale(Vec3::splat(height)),
            DetailCull::new(
                translation - Vec3::new(half, 0.0, half),
                translation + Vec3::new(half, height, half),
                NEAR_DETAIL_RADIUS,
            ),
        ));
        count += 1;
    }
    info!("Antennas: {count}");
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

    let mut seed = run_seed() ^ 0x516E_5EED;
    let mut count = 0usize;
    for building in data.buildings() {
        let cx =
            building.perimeter.iter().map(|p| p[0]).sum::<f32>() / building.perimeter.len() as f32;
        let cz =
            building.perimeter.iter().map(|p| p[1]).sum::<f32>() / building.perimeter.len() as f32;
        let chance = match building.category.as_str() {
            "storefront" => 0.30,
            "industrial" => 0.16,
            _ => continue,
        };
        if building.perimeter.len() < 4 || next_rand(&mut seed) > chance {
            continue;
        }
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
            continue;
        }
        let pick = (next_rand(&mut seed) * candidates.len() as f32) as usize;
        let (edge, length) = candidates.swap_remove(pick.min(candidates.len() - 1));
        let a = Vec2::from(building.perimeter[edge]);
        let b = Vec2::from(building.perimeter[(edge + 1) % building.perimeter.len()]);
        let direction = (b - a) / length;
        let mut outward = Vec2::new(direction.y, -direction.x);
        let midpoint = a + (b - a) * 0.5;
        // Guarantee the blade points away from the building centre even for
        // courtyard/hole loops whose winding is opposite the outer wall.
        if outward.dot(midpoint - Vec2::new(cx, cz)) < 0.0 {
            outward = -outward;
        }
        let t = 0.22 + next_rand(&mut seed) * 0.56;
        let wall = a + (b - a) * t + outward * 0.12;
        let projection = 2.4 + next_rand(&mut seed) * 1.6;
        let height = projection * (0.50 + next_rand(&mut seed) * 0.16);
        let y0 = building.floor_height_m + 0.7;
        let y1 = y0 + height;
        let tip = wall + outward * projection;
        let vertices = [
            [wall.x, y0, wall.y],
            [tip.x, y0, tip.y],
            [tip.x, y1, tip.y],
            [wall.x, y1, wall.y],
        ];
        let normal = Vec3::new(direction.x, 0.0, direction.y);
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
        );
        let tile = (next_rand(&mut seed) * 64.0) as usize % 64;
        let col = (tile % 8) as f32;
        let row = (tile / 8) as f32;
        let u0 = col * SIGN_TILE_UV;
        let v0 = row * SIGN_TILE_UV;
        let u1 = u0 + SIGN_TILE_UV;
        let v1 = v0 + SIGN_TILE_UV;
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vertices.to_vec());
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_NORMAL,
            vec![[normal.x, normal.y, normal.z]; 4],
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_UV_0,
            vec![[u0, v1], [u1, v1], [u1, v0], [u0, v0]],
        );
        mesh.insert_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]));

        let roll = next_rand(&mut seed);
        let material = if roll < 0.22 {
            let phase = (next_rand(&mut seed) * sign_materials.blinking.len() as f32) as usize;
            sign_materials.blinking[phase.min(sign_materials.blinking.len() - 1)].clone()
        } else if roll < 0.58 {
            lit_material.clone()
        } else {
            static_material.clone()
        };
        let min = Vec3::new(wall.x.min(tip.x) - 0.1, y0 - 0.1, wall.y.min(tip.y) - 0.1);
        let max = Vec3::new(wall.x.max(tip.x) + 0.1, y1 + 0.1, wall.y.max(tip.y) + 0.1);
        commands.spawn((
            Name::new(format!("Sign_{:05}", building.id)),
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(material),
            Transform::IDENTITY,
            DetailCull::new(min, max, NEAR_DETAIL_RADIUS),
        ));
        count += 1;
    }
    info!("Signs: {count}");
}

pub fn update_sign_blink(
    time: Res<Time>,
    sign_materials: Res<SignMaterials>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (index, handle) in sign_materials.blinking.iter().enumerate() {
        let phase = sign_materials.phases.get(index).copied().unwrap_or(0.0);
        let on = (time.elapsed_secs() * 1.15 + phase) % 1.0 < 0.64;
        if let Some(mut material) = materials.get_mut(handle) {
            material.emissive = if on {
                LinearRgba::rgb(2.6, 2.6, 2.6)
            } else {
                LinearRgba::BLACK
            };
        }
    }
}
