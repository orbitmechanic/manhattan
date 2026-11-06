//! Streaming low-poly pedestrians and corner advertising.
//!
//! A catalog of mannequin parts and clothing materials is built once.  Near
//! the camera the city is populated with walkers and prone homeless figures
//! from that catalog; buildings that leave the near radius are despawned
//! again.  Nothing about a distant building's crowd is retained.

use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::{
    asset::RenderAssetUsages,
    image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor},
    math::Affine2,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};

use crate::{
    camera::FlyCamera,
    clock::SimClock,
    storefront::{BuildingMeta, StorefrontData},
};

#[cfg(target_arch = "wasm32")]
const PEOPLE_RADIUS: f32 = 100.0;
#[cfg(not(target_arch = "wasm32"))]
const PEOPLE_RADIUS: f32 = 140.0;

#[cfg(target_arch = "wasm32")]
const MAX_WALKERS: usize = 500;
#[cfg(not(target_arch = "wasm32"))]
const MAX_WALKERS: usize = 1_000;

#[cfg(target_arch = "wasm32")]
const MAX_HOMELESS: usize = 150;
#[cfg(not(target_arch = "wasm32"))]
const MAX_HOMELESS: usize = 300;

const MAX_WALKERS_PER_BUILDING: usize = 100;
const MAX_HOMELESS_PER_BUILDING: usize = 20;
const FT_PER_M: f32 = 3.280_84;

// ---------------------------------------------------------------------------
// catalog
// ---------------------------------------------------------------------------
#[derive(Resource)]
pub struct PeopleCatalog {
    torso_male: Handle<Mesh>,
    torso_female: Handle<Mesh>,
    head: Handle<Mesh>,
    hair_male: Handle<Mesh>,
    hair_female: Handle<Mesh>,
    arm: Handle<Mesh>,
    leg: Handle<Mesh>,
    skirt: Handle<Mesh>,
    hat: Handle<Mesh>,
    bag: Handle<Mesh>,
    prone_body: Handle<Mesh>,
    skins: Vec<Handle<StandardMaterial>>,
    hairs: Vec<Handle<StandardMaterial>>,
    shirts: Vec<Handle<StandardMaterial>>,
    pants: Vec<Handle<StandardMaterial>>,
    skirts: Vec<Handle<StandardMaterial>>,
    jackets: Vec<Handle<StandardMaterial>>,
    hats: Vec<Handle<StandardMaterial>>,
    bags: Vec<Handle<StandardMaterial>>,
}

#[allow(clippy::too_many_arguments)]
fn push_quad_uv(
    verts: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    uvs: &mut Vec<[f32; 2]>,
    indices: &mut Vec<u32>,
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
    d: [f32; 3],
) {
    let ab = Vec3::from_array(b) - Vec3::from_array(a);
    let ac = Vec3::from_array(c) - Vec3::from_array(a);
    let n = ab.cross(ac).normalize_or_zero();
    let base = verts.len() as u32;
    verts.extend([a, b, c, d]);
    normals.extend([[n.x, n.y, n.z]; 4]);
    uvs.extend([[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]);
    indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
}

fn box_mesh(center: Vec3, half: Vec3) -> Mesh {
    let mut verts = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    let p = [
        center + Vec3::new(-half.x, -half.y, -half.z),
        center + Vec3::new(half.x, -half.y, -half.z),
        center + Vec3::new(half.x, half.y, -half.z),
        center + Vec3::new(-half.x, half.y, -half.z),
        center + Vec3::new(-half.x, -half.y, half.z),
        center + Vec3::new(half.x, -half.y, half.z),
        center + Vec3::new(half.x, half.y, half.z),
        center + Vec3::new(-half.x, half.y, half.z),
    ];
    for (a, b, c, d) in [
        (0, 1, 2, 3),
        (5, 4, 7, 6),
        (4, 0, 3, 7),
        (1, 5, 6, 2),
        (3, 2, 6, 7),
        (4, 5, 1, 0),
    ] {
        push_quad_uv(
            &mut verts,
            &mut normals,
            &mut uvs,
            &mut indices,
            p[a].to_array(),
            p[b].to_array(),
            p[c].to_array(),
            p[d].to_array(),
        );
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, verts)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

fn sphere_mesh(radius: f32) -> Mesh {
    let mut verts = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
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
            ring.push(Vec3::new(theta.cos() * rr, y, theta.sin() * rr));
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
            let tris = if r == 0 {
                vec![[a, b, c]]
            } else if r + 1 == rings {
                vec![[a, b, d]]
            } else {
                vec![[a, b, d], [a, d, c]]
            };
            for tri in tris {
                let n = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or_zero();
                let base = verts.len() as u32;
                for p in tri {
                    verts.push(p.to_array());
                    normals.push([n.x, n.y, n.z]);
                }
                indices.extend([base, base + 1, base + 2]);
            }
        }
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, verts)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_indices(Indices::U32(indices))
}

fn skirt_mesh(top: f32, bottom: f32, r_top: f32, r_bottom: f32) -> Mesh {
    let mut verts = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    let segments = 8usize;
    for i in 0..segments {
        let a0 = i as f32 / segments as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / segments as f32 * std::f32::consts::TAU;
        let p0 = [a0.cos() * r_top, top, a0.sin() * r_top];
        let p1 = [a1.cos() * r_top, top, a1.sin() * r_top];
        let p2 = [a1.cos() * r_bottom, bottom, a1.sin() * r_bottom];
        let p3 = [a0.cos() * r_bottom, bottom, a0.sin() * r_bottom];
        push_quad_uv(
            &mut verts,
            &mut normals,
            &mut uvs,
            &mut indices,
            p0,
            p1,
            p2,
            p3,
        );
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, verts)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

fn hat_mesh(radius: f32, height: f32) -> Mesh {
    let mut verts = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    let segments = 8usize;
    for i in 0..segments {
        let a0 = i as f32 / segments as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / segments as f32 * std::f32::consts::TAU;
        let p0 = [a0.cos() * radius, 0.0, a0.sin() * radius];
        let p1 = [a1.cos() * radius, 0.0, a1.sin() * radius];
        let p2 = [a1.cos() * radius, height, a1.sin() * radius];
        let p3 = [a0.cos() * radius, height, a0.sin() * radius];
        push_quad_uv(
            &mut verts,
            &mut normals,
            &mut uvs,
            &mut indices,
            p0,
            p1,
            p2,
            p3,
        );
    }
    // crown
    let center = [0.0, height, 0.0];
    for i in 0..segments {
        let a0 = i as f32 / segments as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / segments as f32 * std::f32::consts::TAU;
        let p0 = [a0.cos() * radius, height, a0.sin() * radius];
        let p1 = [a1.cos() * radius, height, a1.sin() * radius];
        let base = verts.len() as u32;
        verts.extend([center, p0, p1]);
        normals.extend([[0.0, 1.0, 0.0]; 3]);
        uvs.extend([[0.5, 0.5], [0.0, 0.0], [1.0, 0.0]]);
        indices.extend([base, base + 1, base + 2]);
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, verts)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

fn tile_material(
    materials: &mut Assets<StandardMaterial>,
    atlas: &Handle<Image>,
    index: usize,
) -> Handle<StandardMaterial> {
    let col = (index % 8) as f32;
    let row = (index / 8) as f32;
    let transform = Affine2::from_translation(Vec2::new(col / 8.0, row / 8.0))
        * Affine2::from_scale(Vec2::splat(1.0 / 8.0));
    materials.add(StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(atlas.clone()),
        uv_transform: transform,
        perceptual_roughness: 0.86,
        ..default()
    })
}

fn plain_material(
    materials: &mut Assets<StandardMaterial>,
    color: Color,
) -> Handle<StandardMaterial> {
    materials.add(StandardMaterial {
        base_color: color,
        perceptual_roughness: 0.82,
        ..default()
    })
}

pub fn setup_people_catalog(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let atlas = asset_server
        .load_builder()
        .with_settings(|s: &mut ImageLoaderSettings| {
            s.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                address_mode_u: ImageAddressMode::ClampToEdge,
                address_mode_v: ImageAddressMode::ClampToEdge,
                ..default()
            });
        })
        .load("textures/people/clothing_atlas.jpg".to_string());
    let skins = [
        (0.86, 0.68, 0.53),
        (0.68, 0.50, 0.36),
        (0.48, 0.33, 0.23),
        (0.94, 0.80, 0.67),
        (0.34, 0.24, 0.18),
    ]
    .into_iter()
    .map(|(r, g, b)| plain_material(&mut materials, Color::srgb(r, g, b)))
    .collect();
    let hairs = [
        (0.08, 0.07, 0.06),
        (0.22, 0.14, 0.08),
        (0.48, 0.34, 0.16),
        (0.55, 0.42, 0.30),
        (0.62, 0.62, 0.60),
        (0.30, 0.18, 0.10),
    ]
    .into_iter()
    .map(|(r, g, b)| plain_material(&mut materials, Color::srgb(r, g, b)))
    .collect();
    let shirts = (0..16)
        .map(|i| tile_material(&mut materials, &atlas, i))
        .collect();
    let pants = (16..32)
        .map(|i| tile_material(&mut materials, &atlas, i))
        .collect();
    let skirts = (32..40)
        .map(|i| tile_material(&mut materials, &atlas, i))
        .collect();
    let jackets = (40..48)
        .map(|i| tile_material(&mut materials, &atlas, i))
        .collect();
    let hats = (48..56)
        .map(|i| tile_material(&mut materials, &atlas, i))
        .collect();
    let bags = (56..62)
        .map(|i| tile_material(&mut materials, &atlas, i))
        .collect();

    commands.insert_resource(PeopleCatalog {
        torso_male: meshes.add(box_mesh(Vec3::ZERO, Vec3::new(0.22, 0.32, 0.13))),
        torso_female: meshes.add(box_mesh(Vec3::ZERO, Vec3::new(0.18, 0.31, 0.12))),
        head: meshes.add(sphere_mesh(0.115)),
        hair_male: meshes.add(box_mesh(
            Vec3::new(0.0, 0.03, -0.015),
            Vec3::new(0.125, 0.08, 0.125),
        )),
        hair_female: meshes.add(box_mesh(
            Vec3::new(0.0, -0.07, -0.045),
            Vec3::new(0.135, 0.19, 0.11),
        )),
        arm: meshes.add(box_mesh(
            Vec3::new(0.0, -0.32, 0.0),
            Vec3::new(0.055, 0.32, 0.055),
        )),
        leg: meshes.add(box_mesh(
            Vec3::new(0.0, -0.42, 0.0),
            Vec3::new(0.07, 0.42, 0.07),
        )),
        skirt: meshes.add(skirt_mesh(0.0, -0.35, 0.20, 0.30)),
        hat: meshes.add(hat_mesh(0.135, 0.11)),
        bag: meshes.add(box_mesh(Vec3::ZERO, Vec3::new(0.08, 0.11, 0.045))),
        prone_body: meshes.add(box_mesh(
            Vec3::new(0.0, 0.13, 0.0),
            Vec3::new(0.20, 0.13, 0.75),
        )),
        skins,
        hairs,
        shirts,
        pants,
        skirts,
        jackets,
        hats,
        bags,
    });
    info!("People catalog ready");
}

// ---------------------------------------------------------------------------
// pedestrians
// ---------------------------------------------------------------------------
#[derive(Component)]
pub struct Pedestrian {
    building_id: usize,
    track: Vec<[f32; 2]>,
    cumulative: Vec<f32>,
    total: f32,
    s: f32,
    speed: f32,
    direction: f32,
    offset: f32,
    lateral: f32,
    base_y: f32,
    bob_phase: f32,
    stride: f32,
}

#[derive(Component)]
pub struct Limb {
    phase: f32,
    amp: f32,
    freq: f32,
}

#[derive(Component)]
struct Homeless;

#[derive(Default)]
struct BuildingCrowd {
    walkers: Vec<Entity>,
    homeless: Vec<Entity>,
}

#[derive(Resource, Default)]
pub struct Crowd {
    active: HashMap<usize, BuildingCrowd>,
    last_check: f32,
}

fn perimeter_m(poly: &[[f32; 2]]) -> f32 {
    (0..poly.len())
        .map(|i| {
            let a = poly[i];
            let b = poly[(i + 1) % poly.len()];
            ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()
        })
        .sum()
}

fn offset_track(poly: &[[f32; 2]], offset: f32) -> Vec<[f32; 2]> {
    let n = poly.len();
    let cx = poly.iter().map(|p| p[0]).sum::<f32>() / n as f32;
    let cz = poly.iter().map(|p| p[1]).sum::<f32>() / n as f32;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let p = poly[i];
        let prev = poly[(i + n - 1) % n];
        let next = poly[(i + 1) % n];
        let mut e1 = Vec2::new(p[0] - prev[0], p[1] - prev[1]);
        let mut e2 = Vec2::new(next[0] - p[0], next[1] - p[1]);
        if e1.length_squared() < 1e-8 {
            e1 = e2;
        }
        if e2.length_squared() < 1e-8 {
            e2 = e1;
        }
        e1 = e1.normalize_or_zero();
        e2 = e2.normalize_or_zero();
        let n1 = Vec2::new(e1.y, -e1.x);
        let n2 = Vec2::new(e2.y, -e2.x);
        let mut miter = (n1 + n2).normalize_or_zero();
        if miter.length_squared() < 1e-6 {
            miter = n1;
        }
        let toward = Vec2::new(p[0] - cx, p[1] - cz);
        if miter.dot(toward) < 0.0 {
            miter = -miter;
        }
        let dot = miter.dot(n1).max(0.4);
        let scale = (1.0 / dot).clamp(1.0, 3.0);
        out.push([
            p[0] + miter.x * offset * scale,
            p[1] + miter.y * offset * scale,
        ]);
    }
    out
}

fn cumulative(points: &[[f32; 2]]) -> Vec<f32> {
    let mut out = vec![0.0];
    for i in 0..points.len() {
        let a = points[i];
        let b = points[(i + 1) % points.len()];
        out.push(
            out.last().copied().unwrap_or(0.0)
                + ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt(),
        );
    }
    out
}

fn sample_track(points: &[[f32; 2]], cumulative: &[f32], s: f32) -> ([f32; 2], [f32; 2]) {
    let total = *cumulative.last().unwrap_or(&0.0);
    let s = if total > 1e-6 {
        s.rem_euclid(total)
    } else {
        0.0
    };
    let mut index = 0;
    while index + 1 < cumulative.len() && cumulative[index + 1] <= s {
        index += 1;
    }
    if index + 1 >= cumulative.len() {
        index = points.len().saturating_sub(1);
    }
    let a = points[index];
    let b = points[(index + 1) % points.len()];
    let span = (cumulative[index + 1] - cumulative[index]).max(1e-6);
    let t = ((s - cumulative[index]) / span).clamp(0.0, 1.0);
    let tangent = [(b[0] - a[0]) / span, (b[1] - a[1]) / span];
    (
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t],
        tangent,
    )
}

fn pick<T: Clone>(items: &[T], seed: &mut u32) -> T {
    items[(next_rand(seed) * items.len() as f32) as usize % items.len()].clone()
}

fn next_rand(seed: &mut u32) -> f32 {
    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    ((*seed >> 8) as f32) / ((u32::MAX >> 8) as f32)
}

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

fn spawn_walker(
    commands: &mut Commands,
    catalog: &PeopleCatalog,
    building: &BuildingMeta,
    seed: &mut u32,
) -> Option<Entity> {
    let offset = 0.3048 + next_rand(seed) * 3.3528;
    let track = offset_track(&building.perimeter, offset);
    if track.len() < 3 {
        return None;
    }
    let cumulative = cumulative(&track);
    let total = *cumulative.last().unwrap_or(&0.0);
    if total < 2.0 {
        return None;
    }
    let female = next_rand(seed) < 0.5;
    let juvenile = next_rand(seed) < 0.10;
    let scale = if juvenile {
        0.30 + next_rand(seed) * 0.50
    } else {
        0.90 + next_rand(seed) * 0.20
    };
    let direction = if next_rand(seed) < 0.5 { 1.0 } else { -1.0 };
    let speed = 0.85 + next_rand(seed) * 0.65;
    let s = next_rand(seed) * total;
    let (point, tangent) = sample_track(&track, &cumulative, s);
    let yaw = tangent[0].atan2(tangent[1]);
    let root_transform = Transform::from_xyz(point[0], 0.03, point[1])
        .with_rotation(Quat::from_rotation_y(yaw))
        .with_scale(Vec3::splat(scale));

    let shirt = pick(&catalog.shirts, seed);
    let pants = pick(&catalog.pants, seed);
    let jacket = pick(&catalog.jackets, seed);
    let wear_hat = next_rand(seed) < 0.55;
    let hat = pick(&catalog.hats, seed);
    let wear_bag = next_rand(seed) < 0.30;
    let bag = pick(&catalog.bags, seed);
    let wear_skirt = female && next_rand(seed) < 0.40;
    let skirt = pick(&catalog.skirts, seed);
    let skin = pick(&catalog.skins, seed);
    let hair = pick(&catalog.hairs, seed);
    let arms = if next_rand(seed) < 0.5 {
        jacket.clone()
    } else {
        shirt.clone()
    };
    let torso = if female {
        catalog.torso_female.clone()
    } else {
        catalog.torso_male.clone()
    };
    let hair_mesh = if female {
        catalog.hair_female.clone()
    } else {
        catalog.hair_male.clone()
    };
    let shoulder = if female { 0.21 } else { 0.25 };
    let stride = 4.0 + speed * 1.6;

    let entity = commands
        .spawn((
            Name::new(format!("Pedestrian_{}", building.id)),
            root_transform,
            Visibility::Inherited,
            Pedestrian {
                building_id: building.id,
                track,
                cumulative,
                total,
                s,
                speed,
                direction,
                offset,
                lateral: 0.0,
                base_y: 0.03,
                bob_phase: next_rand(seed) * std::f32::consts::TAU,
                stride,
            },
        ))
        .id();

    commands.entity(entity).with_children(|p| {
        p.spawn((
            Mesh3d(torso),
            MeshMaterial3d(shirt),
            Transform::from_xyz(0.0, 1.18, 0.0),
        ));
        p.spawn((
            Mesh3d(catalog.head.clone()),
            MeshMaterial3d(skin),
            Transform::from_xyz(0.0, 1.58, 0.0),
        ));
        p.spawn((
            Mesh3d(hair_mesh),
            MeshMaterial3d(hair),
            Transform::from_xyz(0.0, 1.63, 0.0),
        ));
        if wear_hat {
            p.spawn((
                Mesh3d(catalog.hat.clone()),
                MeshMaterial3d(hat),
                Transform::from_xyz(0.0, 1.73, 0.0),
            ));
        }
        if wear_skirt {
            p.spawn((
                Mesh3d(catalog.skirt.clone()),
                MeshMaterial3d(skirt),
                Transform::from_xyz(0.0, 0.96, 0.0),
            ));
        }
        if wear_bag {
            p.spawn((
                Mesh3d(catalog.bag.clone()),
                MeshMaterial3d(bag),
                Transform::from_xyz(0.23, 1.02, 0.02),
            ));
        }
        p.spawn((
            Mesh3d(catalog.arm.clone()),
            MeshMaterial3d(arms.clone()),
            Transform::from_xyz(-shoulder, 1.35, 0.0),
            Limb {
                phase: std::f32::consts::PI,
                amp: 0.38,
                freq: stride,
            },
        ));
        p.spawn((
            Mesh3d(catalog.arm.clone()),
            MeshMaterial3d(arms),
            Transform::from_xyz(shoulder, 1.35, 0.0),
            Limb {
                phase: 0.0,
                amp: 0.38,
                freq: stride,
            },
        ));
        p.spawn((
            Mesh3d(catalog.leg.clone()),
            MeshMaterial3d(pants.clone()),
            Transform::from_xyz(-0.09, 0.88, 0.0),
            Limb {
                phase: 0.0,
                amp: 0.55,
                freq: stride,
            },
        ));
        p.spawn((
            Mesh3d(catalog.leg.clone()),
            MeshMaterial3d(pants),
            Transform::from_xyz(0.09, 0.88, 0.0),
            Limb {
                phase: std::f32::consts::PI,
                amp: 0.55,
                freq: stride,
            },
        ));
    });
    Some(entity)
}

fn spawn_homeless(
    commands: &mut Commands,
    catalog: &PeopleCatalog,
    building: &BuildingMeta,
    seed: &mut u32,
) -> Option<Entity> {
    let offset = 0.30 + next_rand(seed) * 0.65;
    let track = offset_track(&building.perimeter, offset);
    let cumulative = cumulative(&track);
    let total = *cumulative.last().unwrap_or(&0.0);
    if total < 2.0 {
        return None;
    }
    let s = next_rand(seed) * total;
    let (point, tangent) = sample_track(&track, &cumulative, s);
    let yaw = tangent[0].atan2(tangent[1]);
    let scale = 0.75 + next_rand(seed) * 0.30;
    let body = pick(&catalog.shirts, seed);
    let skin = pick(&catalog.skins, seed);
    let hair = pick(&catalog.hairs, seed);
    let entity = commands
        .spawn((
            Name::new(format!("Homeless_{}", building.id)),
            Transform::from_xyz(point[0], 0.02, point[1])
                .with_rotation(Quat::from_rotation_y(yaw))
                .with_scale(Vec3::splat(scale)),
            Visibility::Inherited,
            Homeless,
        ))
        .id();
    commands.entity(entity).with_children(|p| {
        p.spawn((
            Mesh3d(catalog.prone_body.clone()),
            MeshMaterial3d(body),
            Transform::IDENTITY,
        ));
        p.spawn((
            Mesh3d(catalog.head.clone()),
            MeshMaterial3d(skin),
            Transform::from_xyz(0.0, 0.22, 0.80),
        ));
        p.spawn((
            Mesh3d(catalog.hair_male.clone()),
            MeshMaterial3d(hair),
            Transform::from_xyz(0.0, 0.27, 0.82),
        ));
    });
    Some(entity)
}

pub fn stream_people(
    mut commands: Commands,
    time: Res<Time>,
    clock: Res<SimClock>,
    camera: Single<&Transform, With<FlyCamera>>,
    data: Res<StorefrontData>,
    catalog: Res<PeopleCatalog>,
    mut crowd: ResMut<Crowd>,
) {
    let now = time.elapsed_secs();
    if now - crowd.last_check < 0.5 {
        return;
    }
    crowd.last_check = now;

    let hour = clock.minute_of_day.rem_euclid(1440.0) / 60.0;
    let morning = (-((hour - 8.0) / 1.2).powi(2)).exp();
    let evening = (-((hour - 17.5) / 1.2).powi(2)).exp();
    let surge = morning.max(evening).clamp(0.0, 1.0);
    // 1 per 5 ft nominal, 1 per 1 ft at rush hour.
    let per_ft = 0.2f32 + 0.8f32 * surge as f32;

    let cam = camera.translation;
    let mut near: Vec<(f32, usize)> = data
        .buildings()
        .iter()
        .enumerate()
        .filter_map(|(id, building)| {
            let dx = building.centroid[0] - cam.x;
            let dz = building.centroid[1] - cam.z;
            let distance = (dx * dx + dz * dz).sqrt();
            (distance <= PEOPLE_RADIUS).then_some((distance, id))
        })
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    let near_ids: HashSet<usize> = near.iter().map(|(_, id)| *id).collect();

    let far: Vec<usize> = crowd
        .active
        .keys()
        .copied()
        .filter(|id| !near_ids.contains(id))
        .collect();
    for id in far {
        if let Some(active) = crowd.active.remove(&id) {
            for entity in active.walkers.into_iter().chain(active.homeless) {
                commands.entity(entity).despawn();
            }
        }
    }

    let mut walker_count: usize = crowd.active.values().map(|c| c.walkers.len()).sum();
    let mut homeless_count: usize = crowd.active.values().map(|c| c.homeless.len()).sum();
    let mut seed = run_seed() ^ (now * 1000.0) as u32;

    for (_, id) in near {
        let building = &data.buildings()[id];
        let perimeter_ft = perimeter_m(&building.perimeter) * FT_PER_M;
        let target_walkers =
            ((perimeter_ft * per_ft).round() as usize).min(MAX_WALKERS_PER_BUILDING);
        let target_homeless = (perimeter_ft / 20.0).round() as usize;
        let target_homeless = target_homeless.min(MAX_HOMELESS_PER_BUILDING);
        let entry = crowd.active.entry(id).or_default();

        while entry.walkers.len() > target_walkers {
            if let Some(entity) = entry.walkers.pop() {
                commands.entity(entity).despawn();
                walker_count = walker_count.saturating_sub(1);
            }
        }
        while entry.homeless.len() > target_homeless {
            if let Some(entity) = entry.homeless.pop() {
                commands.entity(entity).despawn();
                homeless_count = homeless_count.saturating_sub(1);
            }
        }
        while entry.walkers.len() < target_walkers && walker_count < MAX_WALKERS {
            if let Some(entity) = spawn_walker(&mut commands, &catalog, building, &mut seed) {
                entry.walkers.push(entity);
                walker_count += 1;
            } else {
                break;
            }
        }
        while entry.homeless.len() < target_homeless && homeless_count < MAX_HOMELESS {
            if let Some(entity) = spawn_homeless(&mut commands, &catalog, building, &mut seed) {
                entry.homeless.push(entity);
                homeless_count += 1;
            } else {
                break;
            }
        }
    }
}

pub fn update_pedestrians(
    time: Res<Time>,
    mut query: Query<(Entity, &mut Pedestrian, &mut Transform)>,
) {
    let dt = time.delta_secs().min(0.1);
    let states: Vec<(Entity, usize, f32, f32, f32, f32)> = query
        .iter()
        .map(|(entity, p, _)| (entity, p.building_id, p.s, p.offset, p.direction, p.total))
        .collect();
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for (index, state) in states.iter().enumerate() {
        groups.entry(state.1).or_default().push(index);
    }
    let mut factors = vec![1.0f32; states.len()];
    let mut laterals = vec![0.0f32; states.len()];
    for members in groups.values() {
        for (a, &i) in members.iter().enumerate() {
            for (b, &j) in members.iter().enumerate() {
                if a == b {
                    continue;
                }
                let same_direction = (states[i].4 > 0.0) == (states[j].4 > 0.0);
                let ds =
                    ((states[j].2 - states[i].2) * states[i].4).rem_euclid(states[i].5.max(1e-3));
                let lane = (states[i].3 - states[j].3).abs();
                if ds > 0.04 && ds < 1.6 && lane < 0.9 && same_direction {
                    factors[i] = 0.0;
                }
                if ds > 0.04 && ds < 2.0 && lane < 0.9 && !same_direction {
                    laterals[i] = 0.35;
                    laterals[j] = -0.35;
                }
            }
        }
    }
    let index_by_entity: HashMap<Entity, usize> = states
        .iter()
        .enumerate()
        .map(|(index, state)| (state.0, index))
        .collect();
    let now = time.elapsed_secs();
    for (entity, mut pedestrian, mut transform) in &mut query {
        let Some(index) = index_by_entity.get(&entity).copied() else {
            continue;
        };
        let factor = factors[index];
        let target_lateral = laterals[index];
        pedestrian.lateral += (target_lateral - pedestrian.lateral) * (dt * 4.0).min(1.0);
        pedestrian.s = (pedestrian.s + pedestrian.direction * pedestrian.speed * factor * dt)
            .rem_euclid(pedestrian.total.max(1e-3));
        let (point, tangent) =
            sample_track(&pedestrian.track, &pedestrian.cumulative, pedestrian.s);
        let right = [tangent[1], -tangent[0]];
        let x = point[0] + right[0] * pedestrian.lateral;
        let z = point[1] + right[1] * pedestrian.lateral;
        let forward = [
            tangent[0] * pedestrian.direction,
            tangent[1] * pedestrian.direction,
        ];
        let yaw = forward[0].atan2(forward[1]);
        let bob = if factor > 0.1 {
            (now * pedestrian.stride * 2.0 + pedestrian.bob_phase).sin() * 0.022
        } else {
            0.0
        };
        transform.translation = Vec3::new(x, pedestrian.base_y + bob, z);
        transform.rotation = Quat::from_rotation_y(yaw);
    }
}

pub fn update_limbs(time: Res<Time>, mut query: Query<(&Limb, &mut Transform)>) {
    let now = time.elapsed_secs();
    for (limb, mut transform) in &mut query {
        let angle = (now * limb.freq + limb.phase).sin() * limb.amp;
        transform.rotation = Quat::from_rotation_x(angle);
    }
}
