//! Custom PBR extension that blends a day texture (on the base StandardMaterial)
//! with a night texture per city block.
//!
//! Block ids are baked into vertex COLOR_0 alpha by the Blender export.  The
//! fragment shader reads the block blend from a uniform array and mixes base
//! colour + emissive accordingly, while keeping Bevy's full PBR lighting, fog,
//! tone mapping and bloom.

use std::collections::HashMap;

#[cfg(not(target_arch = "wasm32"))]
use bevy::camera::visibility::VisibilityRange;
use bevy::{
    gltf::GltfMaterialName,
    image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor},
    pbr::{ExtendedMaterial, MaterialExtension, StandardMaterial},
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderType},
    shader::ShaderRef,
};

use crate::blocks::BlockLights;

pub const MAX_BLOCKS: usize = 1536;

/// Marks a real building mesh.  The viewer only culls these at the fog
/// distance; it never replaces them with merged or box proxy geometry.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildingChunk;

/// One vec4 per block: x = day/night blend, y = twilight progress,
/// z = penumbra salt, w = unused.
#[derive(Clone, Copy, Debug, ShaderType)]
pub struct BlockUniform {
    pub values: [Vec4; MAX_BLOCKS],
}

impl Default for BlockUniform {
    fn default() -> Self {
        Self {
            values: [Vec4::ZERO; MAX_BLOCKS],
        }
    }
}

#[derive(Asset, AsBindGroup, Debug, Clone, TypePath)]
pub struct BlockExtension {
    #[uniform(100)]
    pub blocks: BlockUniform,
    /// Legacy night texture for the always-visible ground plane.
    #[texture(101)]
    #[sampler(102)]
    pub legacy_night: Option<Handle<Image>>,
    /// x = emissive strength, y = per-floor variation, z = ground flag
    #[uniform(103)]
    pub emissive: Vec4,
    /// Upper-floor facade atlases.  The per-building tile index is packed into
    /// COLOR_0 RGB by scripts/assign_upper_facades.py.
    #[texture(104)]
    #[sampler(105)]
    pub facade_day: Option<Handle<Image>>,
    #[texture(106)]
    #[sampler(107)]
    pub facade_night: Option<Handle<Image>>,
    #[texture(108)]
    #[sampler(109)]
    pub roof_day: Option<Handle<Image>>,
    #[texture(110)]
    #[sampler(111)]
    pub roof_night: Option<Handle<Image>>,
    /// x = facade columns, y = facade rows, z = roof columns, w = roof rows
    #[uniform(112)]
    pub atlas_info: Vec4,
    /// x/y = facade UV inset, z = roof UV inset, w = roof UV scale
    #[uniform(113)]
    pub atlas_uv: Vec4,
}

impl Default for BlockExtension {
    fn default() -> Self {
        Self {
            blocks: BlockUniform::default(),
            legacy_night: None,
            emissive: Vec4::new(1.0, 0.0, 0.0, 0.0),
            facade_day: None,
            facade_night: None,
            roof_day: None,
            roof_night: None,
            atlas_info: Vec4::new(10.0, 32.0, 16.0, 10.0),
            atlas_uv: Vec4::new(1.0 / 384.0, 1.0 / 96.0, 1.0 / 256.0, 0.5),
        }
    }
}

impl MaterialExtension for BlockExtension {
    fn fragment_shader() -> ShaderRef {
        "shaders/city_block.wesl".into()
    }
}

pub type CityMaterial = ExtendedMaterial<StandardMaterial, BlockExtension>;

#[derive(Resource, Default)]
pub struct CityMaterialSetup {
    pub done: bool,
    pending_count: usize,
    stable: usize,
}

fn base_material_name(material: &str) -> String {
    let lower = material.to_ascii_lowercase();
    lower.strip_prefix("lod0_").unwrap_or(&lower).to_string()
}

fn night_texture_path(material: &str) -> String {
    let base = base_material_name(material);
    if base.starts_with("ground") {
        return "textures/night/ground/satellite_manhattan_ground_night.jpg".to_string();
    }
    if base.starts_with("roof") {
        return "textures/night/roofs/roof_concrete_gravel_night.jpg".to_string();
    }
    format!("textures/night/floors/{base}_floor_night.jpg")
}

const FACADE_COLS: f32 = 10.0;
const FACADE_ROWS: f32 = 32.0;
const ROOF_COLS: f32 = 16.0;
const ROOF_ROWS: f32 = 10.0;

fn load_atlas(asset_server: &AssetServer, path: String, mode: ImageAddressMode) -> Handle<Image> {
    asset_server
        .load_builder()
        .with_settings(move |s: &mut ImageLoaderSettings| {
            s.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                address_mode_u: mode,
                address_mode_v: mode,
                ..default()
            });
        })
        .load(path)
}

fn emissive_strength(material: &str) -> f32 {
    let base = base_material_name(material);
    if base.starts_with("ground") {
        2.8
    } else if base.starts_with("roof") {
        0.7
    } else {
        1.8
    }
}

/// Replace the glTF StandardMaterials with CityMaterials once the scene has
/// finished spawning.
pub fn setup_city_materials(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    standard_materials: Res<Assets<StandardMaterial>>,
    mut city_materials: ResMut<Assets<CityMaterial>>,
    mut setup: ResMut<CityMaterialSetup>,
    query: Query<(Entity, &GltfMaterialName, &MeshMaterial3d<StandardMaterial>)>,
) {
    if setup.done {
        return;
    }
    let primitives = query.iter().count();
    if primitives == 0 {
        return;
    }
    if primitives != setup.pending_count {
        setup.pending_count = primitives;
        setup.stable = 0;
    } else {
        setup.stable += 1;
    }
    let mut has_buildings = false;
    let mut has_ground = false;
    for (_e, name, _m) in &query {
        let base = base_material_name(name);
        has_buildings |= base.starts_with("facade") || base.starts_with("roof");
        has_ground |= base.starts_with("ground");
    }
    if setup.stable < 6 || !has_buildings || !has_ground {
        return;
    }

    let facade_day = load_atlas(
        &asset_server,
        "textures/upper/facade_atlas_day.jpg".to_string(),
        ImageAddressMode::ClampToEdge,
    );
    let facade_night = load_atlas(
        &asset_server,
        "textures/upper/facade_atlas_night.jpg".to_string(),
        ImageAddressMode::ClampToEdge,
    );
    let roof_day = load_atlas(
        &asset_server,
        "textures/upper/roof_atlas_day.jpg".to_string(),
        ImageAddressMode::ClampToEdge,
    );
    let roof_night = load_atlas(
        &asset_server,
        "textures/upper/roof_atlas_night.jpg".to_string(),
        ImageAddressMode::ClampToEdge,
    );

    let mut map: HashMap<String, Handle<CityMaterial>> = HashMap::new();
    for (_entity, name, material) in &query {
        let key = name.to_string();
        if map.contains_key(&key) {
            continue;
        }
        let base = base_material_name(&key);
        if base.starts_with("road") || base.starts_with("sidewalk") || base.starts_with("curb") {
            continue;
        }
        let is_ground = base.starts_with("ground");
        let is_roof = base.starts_with("roof");
        let is_facade = base.starts_with("facade");
        if !(is_ground || is_roof || is_facade) {
            continue;
        }
        let Some(day) = standard_materials.get(&material.0) else {
            continue;
        };
        let mut day = day.clone();
        let legacy_night = if is_ground {
            day.base_color = Color::WHITE;
            Some(load_atlas(
                &asset_server,
                night_texture_path(&key),
                ImageAddressMode::Repeat,
            ))
        } else {
            // Upper floors sample the per-building atlas instead of the
            // original ten repeating floor textures.
            day.base_color = Color::WHITE;
            day.base_color_texture = None;
            day.emissive = LinearRgba::BLACK;
            day.perceptual_roughness = 0.78;
            None
        };
        let strength = emissive_strength(&key);
        let floor_variation = if is_ground || is_roof { 0.0 } else { 1.0 };
        let handle = city_materials.add(CityMaterial {
            base: day,
            extension: BlockExtension {
                blocks: BlockUniform::default(),
                legacy_night,
                emissive: Vec4::new(
                    strength,
                    floor_variation,
                    if is_ground { 1.0 } else { 0.0 },
                    0.0,
                ),
                facade_day: Some(facade_day.clone()),
                facade_night: Some(facade_night.clone()),
                roof_day: Some(roof_day.clone()),
                roof_night: Some(roof_night.clone()),
                atlas_info: Vec4::new(FACADE_COLS, FACADE_ROWS, ROOF_COLS, ROOF_ROWS),
                atlas_uv: Vec4::new(1.0 / 384.0, 1.0 / 96.0, 1.0 / 256.0, 0.5),
            },
        });
        map.insert(key, handle);
    }

    for (entity, name, _) in &query {
        if let Some(handle) = map.get(&**name) {
            commands
                .entity(entity)
                .remove::<MeshMaterial3d<StandardMaterial>>()
                .insert(MeshMaterial3d(handle.clone()));
            if !base_material_name(name).starts_with("ground") {
                // Real geometry only: never swap in a merged region or a
                // larger box proxy.
                commands.entity(entity).insert(BuildingChunk);
                // WebGL2 cannot bind Bevy's VisibilityRange uniform array
                // (the layout advertises a single vec4 while the shader
                // declares 64), so the browser build uses distance visibility
                // culling instead. Both hide real geometry only.
                #[cfg(not(target_arch = "wasm32"))]
                commands.entity(entity).insert(VisibilityRange {
                    start_margin: 0.0..0.0,
                    end_margin: 0.0..25_000.0,
                    use_aabb: false,
                });
            }
        }
    }
    setup.done = true;
    info!(
        "City materials ready: {} materials / {} primitives",
        map.len(),
        primitives
    );
}

/// Copy the current block blends into every CityMaterial's uniform array.
pub fn update_city_materials(
    blocks: Res<BlockLights>,
    mut materials: ResMut<Assets<CityMaterial>>,
) {
    if blocks.states.is_empty() {
        return;
    }
    let mut uniform = BlockUniform::default();
    for (i, state) in blocks.states.iter().take(MAX_BLOCKS).enumerate() {
        // x = block day/night blend, y = twilight progress,
        // z = penumbra salt (re-randomises the per-floor rates each dusk/dawn)
        uniform.values[i] = Vec4::new(state.blend, blocks.progress, blocks.penumbra_id as f32, 0.0);
    }
    for (_, material) in materials.iter_mut() {
        material.extension.blocks = uniform;
    }
}

pub fn setup_road_materials(
    mut done: Local<bool>,
    query: Query<(&GltfMaterialName, &MeshMaterial3d<StandardMaterial>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if *done {
        return;
    }
    for (name, handle) in &query {
        let lower = name.to_string().to_ascii_lowercase();
        if !(lower.contains("road") || lower.contains("sidewalk") || lower.contains("curb")) {
            continue;
        }
        if let Some(mut material) = materials.get_mut(&handle.0) {
            material.base_color = if lower.contains("road") {
                Color::srgb(0.10, 0.10, 0.11)
            } else {
                Color::srgb(0.78, 0.78, 0.75)
            };
            // The road mesh is intentionally unlit so the flat pavement stays
            // readable at grazing angles; update_road_colors tints it with the
            // time of day.
            material.unlit = true;
            material.depth_bias = 50.0;
            material.cull_mode = None;
        }
        *done = true;
    }
}

pub fn update_road_colors(
    env: Res<crate::environment::Environment>,
    query: Query<(&GltfMaterialName, &MeshMaterial3d<StandardMaterial>)>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let day = 0.12 + 0.88 * env.daylight;
    for (name, handle) in &query {
        let lower = name.to_string().to_ascii_lowercase();
        if !(lower.contains("road") || lower.contains("sidewalk") || lower.contains("curb")) {
            continue;
        }
        if let Some(mut material) = materials.get_mut(&handle.0) {
            let base = if lower.contains("road") {
                Vec3::new(0.10, 0.10, 0.11)
            } else {
                Vec3::new(0.78, 0.78, 0.75)
            };
            material.base_color = Color::srgb(base.x * day, base.y * day, base.z * day);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lod0_prefix_is_stripped() {
        assert_eq!(
            base_material_name("LOD0_Facade_Glass_Blue_0056"),
            "facade_glass_blue_0056"
        );
        assert_eq!(
            base_material_name("LOD0_Roof_Concrete_Gravel"),
            "roof_concrete_gravel"
        );
        assert_eq!(
            base_material_name("Ground_Satellite_Manhattan"),
            "ground_satellite_manhattan"
        );
    }

    #[test]
    fn night_paths_use_one_floor_crops() {
        assert!(
            night_texture_path("LOD0_Facade_Glass_Blue_0056")
                .ends_with("floors/facade_glass_blue_0056_floor_night.jpg")
        );
        assert!(
            night_texture_path("LOD0_Roof_Concrete_Gravel")
                .ends_with("roofs/roof_concrete_gravel_night.jpg")
        );
        assert!(
            night_texture_path("Ground_Satellite_Manhattan")
                .ends_with("ground/satellite_manhattan_ground_night.jpg")
        );
    }

    #[test]
    fn emissive_strengths_are_per_surface() {
        assert_eq!(emissive_strength("LOD0_Facade_Glass_Blue_0056"), 1.8);
        assert_eq!(emissive_strength("LOD0_Roof_Concrete_Gravel"), 0.7);
        assert_eq!(emissive_strength("Ground_Satellite_Manhattan"), 2.8);
    }
}
