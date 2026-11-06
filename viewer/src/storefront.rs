//! Randomised ground-floor facade dressing.
//!
//! `building_perimeters.json` now stores the actual exterior wall loops of each
//! building's first-floor mesh (not a convex hull of a whole block).  The
//! catalog supplies four 4096px atlases of 150 ground-floor images each.
//!
//! Every building is panelised along its own wall loop.  Each panel picks a
//! different library image from the category deck, so an image never repeats
//! around the perimeter of a normal building.  The assignment is stored once
//! at startup and is random between runs, fixed within a run.

use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use bevy::{
    asset::RenderAssetUsages,
    image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor},
    light::NotShadowCaster,
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
/// Leave headroom below the 150-image deck so a building never needs a repeat.
const MAX_PANELS_PER_BUILDING: usize = 140;

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
pub struct BuildingMeta {
    pub id: usize,
    pub block: usize,
    pub height_m: f32,
    pub floor_height_m: f32,
    #[serde(rename = "loop")]
    pub perimeter: Vec<[f32; 2]>,
    pub centroid: [f32; 2],
    pub category: String,
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

impl StorefrontData {
    pub fn buildings(&self) -> &[BuildingMeta] {
        &self.buildings
    }
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

/// One facade panel: a library image plus where and how it sits on the wall.
#[derive(Clone, Debug)]
pub struct PanelDressing {
    /// Catalog id of the library image (e.g. `storefront_0073`).
    pub image_id: String,
    /// Distance along the building perimeter, in metres, where the panel starts.
    pub offset_m: f32,
    /// Panel width along the wall, in metres.
    pub width_m: f32,
    /// Panel width divided by the catalog image's nominal width.
    pub scale: f32,
}

#[derive(Clone, Debug)]
pub struct BuildingDressing {
    pub building_id: usize,
    pub block: usize,
    pub category: String,
    pub height_m: f32,
    pub panels: Vec<PanelDressing>,
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

fn shuffle<T>(items: &mut [T], seed: &mut u32) {
    for i in (1..items.len()).rev() {
        let j = ((next_rand(seed) * (i + 1) as f32) as usize).min(i);
        items.swap(i, j);
    }
}

fn signed_area(poly: &[[f32; 2]]) -> f32 {
    let mut area = 0.0;
    for i in 0..poly.len() {
        let a = poly[i];
        let b = poly[(i + 1) % poly.len()];
        area += a[0] * b[1] - b[0] * a[1];
    }
    area * 0.5
}

fn edge_length(a: [f32; 2], b: [f32; 2]) -> f32 {
    let dx = b[0] - a[0];
    let dz = b[1] - a[1];
    (dx * dx + dz * dz).sqrt()
}

/// One planned panel on one loop edge.
#[derive(Clone, Debug)]
struct PanelPlan {
    edge: usize,
    t0: f32,
    t1: f32,
    image: usize,
    offset_m: f32,
    width_m: f32,
}

/// Panelise a closed wall loop.  Each edge is split into roughly
/// `target_width` panels; if that would exceed the catalog deck, the panel
/// width grows until the whole building fits inside a single unique deck.
/// Returns `None` for degenerate loops.
fn plan_panels(
    perimeter: &[[f32; 2]],
    target_width: f32,
    image_count: usize,
    seed: &mut u32,
) -> Option<Vec<PanelPlan>> {
    if perimeter.len() < 3 || image_count == 0 {
        return None;
    }
    // Keep edges together with lengths so the loop edge index stays valid.
    let edges: Vec<(usize, f32)> = (0..perimeter.len())
        .map(|i| {
            (
                i,
                edge_length(perimeter[i], perimeter[(i + 1) % perimeter.len()]),
            )
        })
        .filter(|(_, length)| *length > 0.01)
        .collect();
    let target = target_width.max(0.5);
    let mut width = target;
    let mut counts: Vec<usize>;
    loop {
        counts = edges
            .iter()
            .map(|(_, length)| ((length / width).round() as usize).max(1))
            .collect();
        let total: usize = counts.iter().sum();
        if total <= MAX_PANELS_PER_BUILDING || width > target * 12.0 {
            break;
        }
        width *= 1.12;
    }

    let mut deck: Vec<usize> = (0..image_count).collect();
    shuffle(&mut deck, seed);
    let mut cursor = 0usize;
    let mut plans = Vec::new();
    let mut travel = 0.0f32;
    for ((edge, length), count) in edges.iter().zip(counts.iter()) {
        for panel in 0..*count {
            if cursor >= deck.len() {
                // Only reachable for a loop with more than 150 edges/panels;
                // reshuffle rather than leaving a gap.
                shuffle(&mut deck, seed);
                cursor = 0;
            }
            let t0 = panel as f32 / *count as f32;
            let t1 = (panel + 1) as f32 / *count as f32;
            plans.push(PanelPlan {
                edge: *edge,
                t0,
                t1,
                image: deck[cursor],
                offset_m: travel + length * t0,
                width_m: length / *count as f32,
            });
            cursor += 1;
        }
        travel += *length;
    }
    Some(plans)
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
    let mut repeat_warnings = 0usize;
    for building in &data.buildings {
        let Some(category) = data.catalog.categories.get(&building.category) else {
            continue;
        };
        if category.images.is_empty() || building.perimeter.len() < 3 {
            continue;
        }
        let Some(material) = material_by_category.get(&building.category) else {
            continue;
        };
        let mut perimeter = building.perimeter.clone();
        if signed_area(&perimeter) < 0.0 {
            perimeter.reverse();
        }
        let target_width = category
            .images
            .first()
            .map(|image| image.width_m)
            .unwrap_or(8.0)
            .max(1.0);
        let mut rng = seed
            ^ (building.id as u32).wrapping_mul(0x9E37_79B9)
            ^ (building.block as u32).wrapping_mul(0x85EB_CA6B);
        let Some(plans) = plan_panels(&perimeter, target_width, category.images.len(), &mut rng)
        else {
            continue;
        };

        let floor_height = building.floor_height_m.clamp(2.2, 4.6);
        let base_y = 0.06;
        let mut verts: Vec<[f32; 3]> = Vec::new();
        let mut normals: Vec<[f32; 3]> = Vec::new();
        let mut uvs: Vec<[f32; 2]> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut panels = Vec::with_capacity(plans.len());
        let mut seen = HashSet::new();
        let mut repeated = false;

        for plan in plans {
            let image = &category.images[plan.image];
            if !seen.insert(plan.image) {
                repeated = true;
            }
            let a = Vec2::from(perimeter[plan.edge]);
            let b = Vec2::from(perimeter[(plan.edge + 1) % perimeter.len()]);
            let edge = b - a;
            let length = edge.length();
            if length < 1e-4 {
                continue;
            }
            let outward = Vec2::new(edge.y, -edge.x) / length;
            let p0 = a + edge * plan.t0 + outward * 0.08;
            let p1 = a + edge * plan.t1 + outward * 0.08;
            let base = verts.len() as u32;
            verts.extend([
                [p0.x, base_y, p0.y],
                [p1.x, base_y, p1.y],
                [p1.x, floor_height, p1.y],
                [p0.x, floor_height, p0.y],
            ]);
            normals.extend([[outward.x, 0.0, outward.y]; 4]);
            let u0 = image.col as f32 * TILE_UV + UV_INSET;
            let v0 = image.row as f32 * TILE_UV + UV_INSET;
            let u1 = u0 + TILE_UV - UV_INSET * 2.0;
            let v1 = v0 + TILE_UV - UV_INSET * 2.0;
            uvs.extend([[u0, v1], [u1, v1], [u1, v0], [u0, v0]]);
            indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
            panels.push(PanelDressing {
                image_id: image.id.clone(),
                offset_m: plan.offset_m,
                width_m: plan.width_m,
                scale: plan.width_m / image.width_m.max(1.0),
            });
        }
        if repeated {
            repeat_warnings += 1;
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
            NotShadowCaster,
            Name::new(format!("Storefront_{:05}", building.id)),
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(material.clone()),
            Transform::IDENTITY,
            Visibility::Inherited,
            DetailCull::new(min, max, crate::detail::EMBELLISHMENT_RADIUS),
        ));
        state.assignments.push(BuildingDressing {
            building_id: building.id,
            block: building.block,
            category: building.category.clone(),
            height_m: building.height_m,
            panels,
        });
    }
    info!(
        "Storefronts: {} buildings, {} categories, {} repeat warnings, seed {:08x}",
        state.assignments.len(),
        material_by_category.len(),
        repeat_warnings,
        seed
    );
}

/// A stable lookup for tooling/tests: every building has at least one panel.
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
    fn every_building_has_a_known_category_and_ccw_loop() {
        let data = StorefrontData::default();
        assert!(data.buildings.len() > 10_000);
        for b in &data.buildings {
            assert!(
                data.catalog.categories.contains_key(&b.category),
                "building {} has category {}",
                b.id,
                b.category
            );
            assert!(b.perimeter.len() >= 3);
            assert!(signed_area(&b.perimeter) > 0.0, "building {}", b.id);
            assert!(b.floor_height_m >= 2.0 && b.floor_height_m <= 4.6);
        }
    }

    #[test]
    fn panels_never_repeat_an_image_on_one_building() {
        let perimeter = vec![[0.0, 0.0], [60.0, 0.0], [60.0, 40.0], [0.0, 40.0]];
        let mut seed = 0x51A7_0001;
        let plans = plan_panels(&perimeter, 8.0, 150, &mut seed).expect("plans");
        assert!(plans.len() >= 20);
        let unique: HashSet<usize> = plans.iter().map(|p| p.image).collect();
        assert_eq!(unique.len(), plans.len());
        assert!(plans.iter().all(|p| p.width_m > 1.0));
    }

    #[test]
    fn huge_loop_grows_panels_to_fit_one_deck() {
        // A 3 km perimeter: adaptive panel width keeps the count at or below
        // the 140-panel budget.
        let perimeter = vec![[0.0, 0.0], [1500.0, 0.0], [1500.0, 10.0], [0.0, 10.0]];
        let mut seed = 7;
        let plans = plan_panels(&perimeter, 8.0, 150, &mut seed).expect("plans");
        assert!(plans.len() <= MAX_PANELS_PER_BUILDING);
        let unique: HashSet<usize> = plans.iter().map(|p| p.image).collect();
        assert_eq!(unique.len(), plans.len());
    }
}
