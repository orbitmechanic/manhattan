//! Randomised ground-floor facade dressing.
//!
//! `building_perimeters.json` supplies each block's footprint hull, height and
//! category.  `facade_catalog.json` describes four 4096px atlases of 150
//! ground-floor images each.  At startup every building chooses a fixed
//! per-side image, offset and repeat count; the assignment is stored and the
//! quads are only rendered while the building is within the 1 km embellishment
//! radius.
//!
//! The assignment is random between runs (native clock / wasm `Math.random`)
//! and fixed for the lifetime of a run, so revisiting a building always shows
//! the same storefront.

use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::{
    asset::RenderAssetUsages,
    image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor},
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};
use serde::Deserialize;

use crate::detail::DetailCull;

const BUILDING_DATA: &str = include_str!("../assets/models/building_perimeters.json");
const CATALOG_DATA: &str = include_str!("../assets/models/facade_catalog.json");
const TILE_UV: f32 = 256.0 / 4096.0;
/// Keep filtered samples inside a single atlas tile.
const UV_INSET: f32 = 0.35 / 4096.0;

#[derive(Clone, Debug, Deserialize)]
struct CatalogImage {
    id: String,
    col: usize,
    row: usize,
    width_m: f32,
}

#[derive(Clone, Debug, Deserialize)]
struct CategoryCatalog {
    atlas: String,
    images: Vec<CatalogImage>,
}

#[derive(Clone, Debug, Deserialize)]
struct CatalogFile {
    categories: HashMap<String, CategoryCatalog>,
}

#[derive(Clone, Debug, Deserialize)]
struct BuildingMeta {
    id: usize,
    height_m: f32,
    hull: Vec<[f32; 2]>,
    category: String,
}

#[derive(Deserialize)]
struct BuildingFile {
    buildings: Vec<BuildingMeta>,
}

#[derive(Resource)]
pub struct StorefrontData {
    buildings: Vec<BuildingMeta>,
    catalog: CatalogFile,
}

impl Default for StorefrontData {
    fn default() -> Self {
        Self {
            buildings: serde_json::from_str::<BuildingFile>(BUILDING_DATA)
                .expect("building_perimeters.json is invalid")
                .buildings,
            catalog: serde_json::from_str(CATALOG_DATA).expect("facade_catalog.json is invalid"),
        }
    }
}

/// One side of a building: the chosen library image plus how it is placed.
#[derive(Clone, Debug)]
pub struct SideDressing {
    /// Catalog id of the library image (e.g. `storefront_0073`).
    pub image_id: String,
    /// Distance along this side, in metres, where the image sequence starts.
    pub offset_m: f32,
    /// How many times the image repeats along the side.
    pub repeats: f32,
    /// Horizontal scale relative to the catalog image width.
    pub scale: f32,
}

#[derive(Clone, Debug)]
pub struct BuildingDressing {
    pub building_id: usize,
    pub category: String,
    pub sides: Vec<SideDressing>,
}

#[derive(Resource, Default)]
pub struct StorefrontState {
    pub assignments: Vec<BuildingDressing>,
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

fn next_rand(seed: &mut u32) -> f32 {
    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    ((*seed >> 8) as f32) / ((u32::MAX >> 8) as f32)
}

fn hull_is_ccw(hull: &[[f32; 2]]) -> bool {
    let mut area = 0.0;
    for i in 0..hull.len() {
        let a = hull[i];
        let b = hull[(i + 1) % hull.len()];
        area += a[0] * b[1] - b[0] * a[1];
    }
    area > 0.0
}

pub fn spawn_storefronts(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    data: Res<StorefrontData>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut state: ResMut<StorefrontState>,
) {
    let mut material_by_category: HashMap<String, Handle<StandardMaterial>> = HashMap::new();
    for (category, catalog) in &data.catalog.categories {
        let path = format!("textures/facades/{}", catalog.atlas);
        let texture = asset_server
            .load_builder()
            .with_settings(|s: &mut ImageLoaderSettings| {
                s.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                    address_mode_u: ImageAddressMode::ClampToEdge,
                    address_mode_v: ImageAddressMode::ClampToEdge,
                    ..default()
                });
            })
            .load(path);
        material_by_category.insert(
            category.clone(),
            materials.add(StandardMaterial {
                base_color: Color::WHITE,
                base_color_texture: Some(texture),
                unlit: true,
                double_sided: true,
                cull_mode: None,
                perceptual_roughness: 0.88,
                ..default()
            }),
        );
    }

    let seed = run_seed();
    for building in &data.buildings {
        let Some(category) = data.catalog.categories.get(&building.category) else {
            continue;
        };
        if category.images.is_empty() || building.hull.len() < 3 {
            continue;
        }
        let Some(material) = material_by_category.get(&building.category) else {
            continue;
        };
        let mut hull = building.hull.clone();
        if !hull_is_ccw(&hull) {
            hull.reverse();
        }
        let mut verts: Vec<[f32; 3]> = Vec::new();
        let mut normals: Vec<[f32; 3]> = Vec::new();
        let mut uvs: Vec<[f32; 2]> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut sides = Vec::new();
        let mut rng = seed ^ (building.id as u32).wrapping_mul(0x9E37_79B9);
        let base_y = 0.06;
        let top_y = building.height_m.clamp(2.4, 3.6);
        for i in 0..hull.len() {
            let a = Vec2::from(hull[i]);
            let b = Vec2::from(hull[(i + 1) % hull.len()]);
            let edge = b - a;
            let len = edge.length();
            if len < 0.4 {
                continue;
            }
            let n = Vec2::new(edge.y, -edge.x) / len;
            let image_index = (next_rand(&mut rng) * category.images.len() as f32) as usize
                % category.images.len();
            let image = &category.images[image_index];
            let image_w = image.width_m.max(1.0);
            let repeats = (len / image_w).max(1.0 / image_w);
            let whole = repeats.floor().max(1.0) as usize;
            let last = (repeats - repeats.floor()).clamp(0.02, 1.0);
            let phase = next_rand(&mut rng);
            let offset_m = phase * image_w;
            let u0 = image.col as f32 * TILE_UV + UV_INSET;
            let v0 = image.row as f32 * TILE_UV + UV_INSET;
            let u_full = TILE_UV - UV_INSET * 2.0;
            let v_full = TILE_UV - UV_INSET * 2.0;
            let u1 = u0 + u_full;
            let v1 = v0 + v_full;
            let a = a + n * 0.08;
            let b = b + n * 0.08;
            let mut emit = |t0: f32, t1: f32, u_start: f32, u_end: f32| {
                let p0 = a.lerp(b, t0);
                let p1 = a.lerp(b, t1);
                let base = verts.len() as u32;
                verts.extend([
                    [p0.x, base_y, p0.y],
                    [p1.x, base_y, p1.y],
                    [p1.x, top_y, p1.y],
                    [p0.x, top_y, p0.y],
                ]);
                normals.extend([[n.x, 0.0, n.y]; 4]);
                uvs.extend([[u_start, v1], [u_end, v1], [u_end, v0], [u_start, v0]]);
                indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
            };
            let mut t = 0.0;
            for _ in 0..whole {
                let t1 = t + (1.0 / repeats).min(1.0 - t);
                emit(t, t1, u0, u1);
                t = t1;
            }
            if last > 0.02 && t < 0.999 {
                emit(t, 1.0, u0, u0 + u_full * last);
            }
            sides.push(SideDressing {
                image_id: image.id.clone(),
                offset_m,
                repeats,
                scale: (len / repeats) / image_w,
            });
        }
        if indices.is_empty() {
            continue;
        }
        let mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, verts.clone())
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(indices));
        let min = verts.iter().fold(Vec3::splat(f32::INFINITY), |m, v| {
            m.min(Vec3::from_array(*v))
        });
        let max = verts.iter().fold(Vec3::splat(f32::NEG_INFINITY), |m, v| {
            m.max(Vec3::from_array(*v))
        });
        commands.spawn((
            Name::new(format!("Storefront_{:04}", building.id)),
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(material.clone()),
            Transform::IDENTITY,
            Visibility::Inherited,
            DetailCull { min, max },
        ));
        state.assignments.push(BuildingDressing {
            building_id: building.id,
            category: building.category.clone(),
            sides,
        });
    }
    info!(
        "Storefronts: {} buildings, {} categories, seed {:08x}",
        state.assignments.len(),
        material_by_category.len(),
        seed
    );
}

/// A stable lookup for tooling/tests: every building has at least one side.
pub fn assignment_for(state: &StorefrontState, building_id: usize) -> Option<&BuildingDressing> {
    state
        .assignments
        .iter()
        .find(|a| a.building_id == building_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_four_categories_of_150() {
        let data = StorefrontData::default();
        assert_eq!(data.catalog.categories.len(), 4);
        for category in ["storefront", "executive", "residential", "industrial"] {
            let c = data
                .catalog
                .categories
                .get(category)
                .unwrap_or_else(|| panic!("missing {category}"));
            assert_eq!(c.images.len(), 150, "{category}");
            assert!(c.images.iter().all(|i| i.col < 16 && i.row < 16));
        }
    }

    #[test]
    fn every_building_has_a_known_category() {
        let data = StorefrontData::default();
        assert_eq!(data.buildings.len(), 1_223);
        for b in &data.buildings {
            assert!(
                data.catalog.categories.contains_key(&b.category),
                "building {} has category {}",
                b.id,
                b.category
            );
            assert!(b.hull.len() >= 3);
        }
    }

    #[test]
    fn hull_orientation_is_detected() {
        let ccw = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let mut cw = ccw;
        cw.reverse();
        assert!(hull_is_ccw(&ccw));
        assert!(!hull_is_ccw(&cw));
    }
}
