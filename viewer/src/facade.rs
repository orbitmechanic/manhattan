//! Per-family shell facades: one shared cladding texture, one window-sash
//! texture and one night-emissive texture per `MaterialFamily`.
//!
//! Textures belong to the shell. A streamed building never allocates a
//! material of its own: it points at its family's handle. The window grid is
//! the layout's own pane geometry; the sash atlas supplies the frame, mullions
//! and glass tint per family, and the emissive atlas (same layout, dark frame)
//! supplies the night glow. Ground-floor panes get the shop variants so the
//! street reads lit after dusk.
//!
//! Provenance: every image is generated here at startup from a deterministic
//! seed and the family palette, so there is no external texture to license.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

use destructibles_bevy::building::{Buildings, Pane};
use destructibles_bevy::night::{SceneTime, WindowLight, night_factor, update_window_lights};
use destructibles_bevy::power::Power;
use destructibles_core::family::MaterialFamily;

const WALL_TEX: u32 = 256;
const SASH_TEX: u32 = 128;

/// One family's share of the facade atlas budget.
#[derive(Clone)]
pub struct FamilySkin {
    pub wall: Handle<StandardMaterial>,
    pub pane: Handle<StandardMaterial>,
    pub lit: Handle<StandardMaterial>,
    pub shop: Handle<StandardMaterial>,
    pub shop_lit: Handle<StandardMaterial>,
}

#[derive(Resource, Clone, Default)]
pub struct FacadeSkins {
    steel: Option<FamilySkin>,
    timber: Option<FamilySkin>,
    masonry: Option<FamilySkin>,
    adobe: Option<FamilySkin>,
    /// Bytes of CPU-side texture data held by the shared atlases.
    pub texture_bytes: usize,
}

impl FacadeSkins {
    pub fn skin(&self, family: MaterialFamily) -> Option<&FamilySkin> {
        match family {
            MaterialFamily::SteelFrame => self.steel.as_ref(),
            MaterialFamily::Timber => self.timber.as_ref(),
            MaterialFamily::MasonryStone => self.masonry.as_ref(),
            MaterialFamily::EarthAdobe => self.adobe.as_ref(),
        }
    }

    pub fn family_count(&self) -> usize {
        [
            self.steel.is_some(),
            self.timber.is_some(),
            self.masonry.is_some(),
            self.adobe.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count()
    }
}

#[derive(Clone, Copy)]
enum Grain {
    Panels,
    Brick,
    Siding,
    Plaster,
}

#[derive(Clone, Copy)]
struct Palette {
    /// Cladding multiplier (sRGB) applied to the grayscale grain texture.
    wall: [f32; 3],
    /// Window frame colour in the sash atlas.
    frame: [f32; 3],
    /// Glass tint at the top and bottom of the pane.
    glass_top: [f32; 3],
    glass_bottom: [f32; 3],
    /// Warm glow colour for the emissive atlas.
    glow: [f32; 3],
    grain: Grain,
    metallic: f32,
    roughness: f32,
}

fn palette(family: MaterialFamily) -> Palette {
    match family {
        MaterialFamily::SteelFrame => Palette {
            wall: [0.42, 0.46, 0.50],
            frame: [0.09, 0.10, 0.11],
            glass_top: [0.34, 0.44, 0.54],
            glass_bottom: [0.16, 0.22, 0.29],
            glow: [1.0, 0.86, 0.62],
            grain: Grain::Panels,
            metallic: 0.30,
            roughness: 0.55,
        },
        MaterialFamily::Timber => Palette {
            wall: [0.46, 0.32, 0.20],
            frame: [0.78, 0.74, 0.66],
            glass_top: [0.30, 0.36, 0.40],
            glass_bottom: [0.12, 0.15, 0.18],
            glow: [1.0, 0.72, 0.40],
            grain: Grain::Siding,
            metallic: 0.0,
            roughness: 0.86,
        },
        MaterialFamily::MasonryStone => Palette {
            wall: [0.50, 0.34, 0.26],
            frame: [0.20, 0.18, 0.17],
            glass_top: [0.26, 0.31, 0.35],
            glass_bottom: [0.10, 0.12, 0.14],
            glow: [1.0, 0.84, 0.58],
            grain: Grain::Brick,
            metallic: 0.0,
            roughness: 0.92,
        },
        MaterialFamily::EarthAdobe => Palette {
            wall: [0.56, 0.44, 0.30],
            frame: [0.34, 0.26, 0.18],
            glass_top: [0.22, 0.25, 0.26],
            glass_bottom: [0.09, 0.10, 0.11],
            glow: [1.0, 0.74, 0.44],
            grain: Grain::Plaster,
            metallic: 0.0,
            roughness: 0.98,
        },
    }
}

fn hash01(mut value: u32) -> f32 {
    value ^= value >> 16;
    value = value.wrapping_mul(0x7FEB_352D);
    value ^= value >> 15;
    value = value.wrapping_mul(0x846C_A68F);
    value ^= value >> 16;
    (value & 0x00FF_FFFF) as f32 / 16_777_216.0
}

fn clamp01(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

fn to_byte(value: f32) -> u8 {
    (clamp01(value) * 255.0).round() as u8
}

/// Grayscale cladding grain. The material's base colour supplies the tint, so
/// one texture serves every district that keys to the family.
fn wall_texture(palette: Palette) -> Image {
    let size = WALL_TEX as usize;
    let mut data = vec![0u8; size * size * 4];
    let debug = std::env::var_os("FACADE_DEBUG").is_some();
    for y in 0..size {
        for x in 0..size {
            let value = match palette.grain {
                Grain::Panels => {
                    // 2 x 2 metal panels with a recessed joint and a rivet.
                    let joint = (x % 128 < 3) || (y % 128 < 3);
                    let rivet =
                        (x % 128 == 12 || x % 128 == 116) && (y % 128 == 12 || y % 128 == 116);
                    let streak = 0.03 * hash01((x as u32).wrapping_mul(131) ^ y as u32);
                    if rivet {
                        0.55
                    } else if joint {
                        0.42
                    } else {
                        0.95 + streak
                    }
                }
                Grain::Brick => {
                    // Running bond: 8 courses, 4 bricks per course. Bold enough
                    // to survive the mip chain at 100-300 m; the mortar is a
                    // 3 px joint, not a hairline.
                    let course = y / 32;
                    let offset = if course % 2 == 0 { 0 } else { 32 };
                    let mortar = (y % 32 < 3) || ((x + offset) % 64 < 3);
                    let brick = hash01(
                        ((x + offset) / 64) as u32 ^ (course as u32).wrapping_mul(0x9E37_79B9),
                    );
                    if mortar { 0.55 } else { 0.86 + brick * 0.14 }
                }
                Grain::Siding => {
                    // 6 horizontal lap boards with a shadow line under each.
                    let board = y / 42;
                    let in_board = y % 42;
                    let grain =
                        0.05 * hash01((x as u32).wrapping_mul(67).wrapping_add(board as u32));
                    if in_board < 5 {
                        0.40
                    } else {
                        0.94 - in_board as f32 * 0.006 + grain
                    }
                }
                Grain::Plaster => {
                    let blob = hash01(
                        ((x / 8) as u32).wrapping_mul(0x85EB_CA6B)
                            ^ ((y / 8) as u32).wrapping_mul(0x2545_F491),
                    );
                    0.86 + blob * 0.12 + 0.03 * hash01(x as u32 ^ (y as u32) << 8)
                }
            };
            let index = (y * size + x) * 4;
            let color = if debug {
                let checker = ((x / 32) + (y / 32)) % 2 == 0;
                if checker {
                    [1.0, 0.0, 1.0]
                } else {
                    [0.0, 0.0, 0.0]
                }
            } else {
                [
                    palette.wall[0] * value,
                    palette.wall[1] * value,
                    palette.wall[2] * value,
                ]
            };
            data[index] = to_byte(color[0]);
            data[index + 1] = to_byte(color[1]);
            data[index + 2] = to_byte(color[2]);
            data[index + 3] = 255;
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: WALL_TEX,
            height: WALL_TEX,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
        address_mode_u: bevy::image::ImageAddressMode::Repeat,
        address_mode_v: bevy::image::ImageAddressMode::Repeat,
        mag_filter: bevy::image::ImageFilterMode::Linear,
        min_filter: bevy::image::ImageFilterMode::Linear,
        mipmap_filter: bevy::image::ImageFilterMode::Linear,
        ..default()
    });
    image
}

/// One window cell: frame, mullions, glass. Each pane quad maps 0..1 onto it.
fn sash_texture(palette: Palette, shop: bool) -> Image {
    let size = SASH_TEX as usize;
    let mut data = vec![0u8; size * size * 4];
    let frame = if shop { 10 } else { 8 };
    for y in 0..size {
        for x in 0..size {
            let border = x < frame || y < frame || x >= size - frame || y >= size - frame;
            // Mullion pattern: one centre mullion plus one transom for timber
            // and masonry houses; a single slim mullion for steel ribbons.
            let mullion = match palette.grain {
                Grain::Panels => x == size / 2 || (!shop && y == size / 3),
                Grain::Siding => x == size / 2 || y == size / 2,
                Grain::Brick => x == size / 2 || y == size / 2,
                Grain::Plaster => x == size / 2,
            };
            let glass_row = y as f32 / size as f32;
            let index = (y * size + x) * 4;
            if border || mullion {
                data[index] = to_byte(palette.frame[0]);
                data[index + 1] = to_byte(palette.frame[1]);
                data[index + 2] = to_byte(palette.frame[2]);
                data[index + 3] = 255;
            } else {
                for channel in 0..3 {
                    let value = palette.glass_top[channel] * (1.0 - glass_row)
                        + palette.glass_bottom[channel] * glass_row;
                    data[index + channel] = to_byte(value);
                }
                data[index + 3] = 255;
            }
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: SASH_TEX,
            height: SASH_TEX,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::linear();
    image
}

/// Emissive mask: the glass cells glow, the frame stays black so the sash
/// silhouette reads at distance.
fn lit_texture(palette: Palette, shop: bool) -> Image {
    let size = SASH_TEX as usize;
    let mut data = vec![0u8; size * size * 4];
    let frame = if shop { 10 } else { 8 };
    for y in 0..size {
        for x in 0..size {
            let border = x < frame || y < frame || x >= size - frame || y >= size - frame;
            let mullion = match palette.grain {
                Grain::Panels => x == size / 2 || (!shop && y == size / 3),
                Grain::Siding => x == size / 2 || y == size / 2,
                Grain::Brick => x == size / 2 || y == size / 2,
                Grain::Plaster => x == size / 2,
            };
            let index = (y * size + x) * 4;
            if border || mullion {
                data[index + 3] = 255;
                continue;
            }
            // Brighter toward the top of the glass, like a ceiling fixture.
            let row = y as f32 / size as f32;
            let value = 0.55 + 0.45 * (1.0 - row);
            for channel in 0..3 {
                data[index + channel] = to_byte(palette.glow[channel] * value);
            }
            data[index + 3] = 255;
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: SASH_TEX,
            height: SASH_TEX,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::linear();
    image
}

fn build_skin(
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
    palette: Palette,
) -> (FamilySkin, usize) {
    let wall = images.add(wall_texture(palette));
    let sash = images.add(sash_texture(palette, false));
    let shop_sash = images.add(sash_texture(palette, true));
    let lit = images.add(lit_texture(palette, false));
    let shop_lit = images.add(lit_texture(palette, true));

    let wall_material = materials.add(StandardMaterial {
        // The grain atlas bakes the family colour, so the shared material is
        // a pure multiplier: one texture, one handle, no per-building tint.
        base_color: Color::WHITE,
        base_color_texture: Some(wall),
        metallic: palette.metallic,
        perceptual_roughness: palette.roughness,
        ..default()
    });
    let pane = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(sash),
        metallic: 0.28,
        perceptual_roughness: 0.16,
        double_sided: true,
        ..default()
    });
    let shop_pane = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(shop_sash),
        metallic: 0.20,
        perceptual_roughness: 0.22,
        double_sided: true,
        ..default()
    });
    let lit_window = materials.add(StandardMaterial {
        base_color: Color::BLACK,
        emissive: LinearRgba::rgb(7.5, 5.2, 2.9),
        emissive_texture: Some(lit),
        unlit: false,
        perceptual_roughness: 0.9,
        ..default()
    });
    let shop_lit_material = materials.add(StandardMaterial {
        base_color: Color::BLACK,
        emissive: LinearRgba::rgb(11.0, 7.4, 4.0),
        emissive_texture: Some(shop_lit),
        unlit: false,
        perceptual_roughness: 0.9,
        ..default()
    });
    let bytes = (WALL_TEX * WALL_TEX * 4) as usize + 4 * (SASH_TEX * SASH_TEX * 4) as usize;
    (
        FamilySkin {
            wall: wall_material,
            pane,
            lit: lit_window,
            shop: shop_pane,
            shop_lit: shop_lit_material,
        },
        bytes,
    )
}

fn setup_facades(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut skins = FacadeSkins::default();
    let mut bytes = 0;
    for family in [
        MaterialFamily::SteelFrame,
        MaterialFamily::Timber,
        MaterialFamily::MasonryStone,
        MaterialFamily::EarthAdobe,
    ] {
        let (skin, family_bytes) = build_skin(&mut images, &mut materials, palette(family));
        bytes += family_bytes;
        match family {
            MaterialFamily::SteelFrame => skins.steel = Some(skin),
            MaterialFamily::Timber => skins.timber = Some(skin),
            MaterialFamily::MasonryStone => skins.masonry = Some(skin),
            MaterialFamily::EarthAdobe => skins.adobe = Some(skin),
        }
    }
    skins.texture_bytes = bytes;
    info!(
        "FACADE skins: {} families, {} shared materials, {:.1} MB CPU textures (mipmaps extra)",
        skins.family_count(),
        5 * skins.family_count(),
        bytes as f64 / (1024.0 * 1024.0)
    );
    commands.insert_resource(skins);
}

/// Point one building's wall and panes at its family's shared handles. Called
/// at shell spawn and again by the guard after a detail tier-up rebuilds the
/// children.
pub fn apply_family_skin(world: &mut World, id: u32, family: MaterialFamily) {
    let Some(skins) = world.get_resource::<FacadeSkins>().cloned() else {
        return;
    };
    let Some(skin) = skins.skin(family).cloned() else {
        return;
    };
    let Some((wall, root, record_floors)) = world.resource::<Buildings>().get(id).map(|record| {
        (
            record.wall_mesh,
            record.entity,
            record.layout.spec.floor_count,
        )
    }) else {
        return;
    };
    // Reuse the crate's existing cool window material for LED-temperature
    // floors: no new material is allocated.
    let cool_lit = world
        .get_resource::<destructibles_bevy::assets::WorldAssets>()
        .map(|assets| assets.window_cool.clone());
    world
        .entity_mut(wall)
        .insert(MeshMaterial3d(skin.wall.clone()));
    grade_wall(world, wall, id, family);
    let Some(children) = world.get::<Children>(root) else {
        return;
    };
    let children = children.iter().collect::<Vec<_>>();
    for child in children {
        if let Some(pane) = world.get::<Pane>(child) {
            let handle = if pane.layout.id.floor == 0 {
                &skin.shop
            } else {
                &skin.pane
            };
            world
                .entity_mut(child)
                .insert(MeshMaterial3d(handle.clone()));
        } else if let Some(window) = world.get::<WindowLight>(child) {
            // Night temperature: a tower mixes warm incandescent and cool LED
            // floor by floor; a 1-3 storey house is one household, one temp.
            let pick_cool = window.floor > 0
                && record_floors > 3
                && window.floor > 1
                && hash01(id.wrapping_mul(0x9E37_79B9) ^ window.floor.wrapping_mul(0x85EB_CA6B))
                    < 0.55;
            let handle = if window.floor == 0 {
                &skin.shop_lit
            } else if pick_cool {
                cool_lit.as_ref().unwrap_or(&skin.lit)
            } else {
                &skin.lit
            };
            world
                .entity_mut(child)
                .insert(MeshMaterial3d(handle.clone()));
        }
    }
}

/// Per-building material grade, baked as a vertex-colour instance attribute on
/// the building's own wall mesh. The shared family atlas and its material are
/// untouched, so the 20-material / ~2 MB budget does not grow. Ranges are
/// hue-constrained per family: timber and brick vary warm, steel stays neutral.
fn grade_wall(world: &mut World, wall: Entity, id: u32, family: MaterialFamily) {
    let Some(handle) = world.get::<Mesh3d>(wall).map(|mesh| mesh.0.clone()) else {
        return;
    };
    // Tint salad: six muted neutral directions, saturation capped.
    let _ = family;
    const MUTED: [[f32; 3]; 6] = [
        [1.06, 0.96, 0.86],
        [1.02, 1.00, 0.95],
        [0.96, 0.96, 0.96],
        [0.92, 0.98, 1.08],
        [1.08, 1.06, 1.02],
        [0.90, 0.86, 0.82],
    ];
    let pick =
        MUTED[(hash01(id.wrapping_mul(0x9E37_79B9)) * MUTED.len() as f32) as usize % MUTED.len()];
    let nudge = 0.78 + 0.44 * hash01(id.rotate_left(7) ^ 0x5EED_1234);
    let tint = [
        (pick[0] * nudge).clamp(0.70, 1.20),
        (pick[1] * nudge).clamp(0.70, 1.20),
        (pick[2] * nudge).clamp(0.70, 1.20),
        1.0,
    ];
    let Some(mut meshes) = world.get_resource_mut::<Assets<Mesh>>() else {
        return;
    };
    let Some(mut mesh) = meshes.get_mut(&handle) else {
        return;
    };
    if mesh.attribute(Mesh::ATTRIBUTE_COLOR).is_some() {
        return;
    }
    let count = mesh.count_vertices();
    if count == 0 {
        return;
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![tint; count]);
}

/// The tier-up rebuild is allowed to respawn children with the stock handles;
/// repair any resident building whose wall handle drifted. Runs twice a second
/// over the resident roots only, so the scan is chunk-sized, not city-sized.
#[derive(Resource, Default)]
pub struct SkinGuard {
    next_at: f32,
}

fn skin_guard(world: &mut World) {
    let now = world.resource::<Time>().elapsed_secs();
    if now < world.resource::<SkinGuard>().next_at {
        return;
    }
    world.resource_mut::<SkinGuard>().next_at = now + 2.0;
    let Some(geology) = world.get_resource::<crate::geology::Geology>() else {
        return;
    };
    let roots = geology
        .building_chunks
        .values()
        .flat_map(|entities| entities.iter().copied())
        .collect::<Vec<_>>();
    if roots.is_empty() {
        return;
    }
    let Some(skins) = world.get_resource::<FacadeSkins>().cloned() else {
        return;
    };
    let mut repaired = 0usize;
    for root in roots {
        let Some((id, family, wall)) = world
            .get::<destructibles_bevy::building::Building>(root)
            .and_then(|building| {
                let record = world.resource::<Buildings>().get(building.id)?;
                Some((record.id, record.family, record.wall_mesh))
            })
        else {
            continue;
        };
        let Some(skin) = skins.skin(family) else {
            continue;
        };
        let current = world
            .get::<MeshMaterial3d<StandardMaterial>>(wall)
            .map(|material| material.0.clone());
        if current.as_ref() != Some(&skin.wall) {
            apply_family_skin(world, id, family);
            repaired += 1;
        }
    }
    if repaired > 0 {
        info!("FACADE skin guard: repaired {repaired} buildings after tier-up");
    }
}

/// Resident height profile for the night density: a three-storey street is
/// not a tower, so it should not light like one.
#[derive(Resource, Default)]
struct NightProfile {
    floors: std::collections::HashMap<u32, u32>,
    at: f32,
}

/// Seattle night density. The stock system lights 30–68 % of panes; what the
/// city needs to be navigable from the air is floor-coherent rows (a dark
/// building with one lit floor reads as a building, a scatter of dots does
/// not) plus a hot ground floor where the shops are.
fn seattle_night_density(
    time: Res<Time>,
    scene: Res<SceneTime>,
    buildings: Res<Buildings>,
    mut profile: ResMut<NightProfile>,
    power: Res<Power>,
    mut windows: Query<(&WindowLight, &mut Visibility)>,
) {
    let night = night_factor(scene.hours);
    if night < 0.25 {
        return;
    }
    let now = time.elapsed_secs();
    let epoch = (now / 24.0) as u32;
    if now - profile.at > 2.0 {
        profile.at = now;
        profile.floors.clear();
        for record in buildings.all() {
            if let Some(floors) = profile.floors.get_mut(&record.id) {
                *floors = (*floors).max(record.layout.spec.floor_count);
            } else {
                profile
                    .floors
                    .insert(record.id, record.layout.spec.floor_count);
            }
        }
    }
    for (window, mut visibility) in &mut windows {
        if power.level(window.building, now) < 0.35 {
            *visibility = Visibility::Hidden;
            continue;
        }
        let floors = profile.floors.get(&window.building).copied().unwrap_or(8);
        let base = if window.glass {
            0.72
        } else if floors <= 3 {
            0.42
        } else {
            0.62
        };
        let floor_lit = hash01(
            window.building
                ^ window.floor.wrapping_mul(0x85EB_CA6B)
                ^ epoch.wrapping_mul(0x27D4_EB2F),
        ) < base * 1.25;
        let pane_lit = hash01(window.seed ^ epoch.wrapping_mul(0x9E37_79B9)) < 0.88;
        let ground = window.floor == 0;
        let lit = if ground {
            // Shops and porches: almost always on until late.
            hash01(window.seed ^ epoch.wrapping_mul(0xC2B2_AE35)) < 0.9
        } else {
            floor_lit && pane_lit
        };
        *visibility = if lit {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

/// Shared storefront prop material: awnings and placards pick up a little of
/// the street's own light after dusk instead of sitting as black cut-outs.
fn storefront_night_glow(
    scene: Res<SceneTime>,
    storefronts: Option<Res<crate::storefronts::StorefrontAssets>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Some(storefronts) = storefronts else {
        return;
    };
    let night = night_factor(scene.hours);
    let Some(mut material) = materials.get_mut(&storefronts.material) else {
        return;
    };
    material.emissive = LinearRgba::rgb(0.34 * night, 0.24 * night, 0.14 * night);
}

/// Per-family proxy facade tile. Strategy (a): **one texture per family**, so
/// repeat is safe in both axes and the UV is the plain physical mapping
/// (u = width / module, v = height / 3.0). No banks, no sub-rect math, no
/// shader work. One window row per tile; a plain 5 % strip at the right edge is
/// the roof/gravel sample.
fn proxy_day_texture(warm: bool) -> Image {
    let size = SASH_TEX as usize;
    let plain_from = (size as f32 * 0.95) as usize;
    let columns = if warm { 4usize } else { 6 };
    let cell_w = size / columns;
    let mut data = vec![0u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let index = (y * size + x) * 4;
            if x >= plain_from {
                let grain = hash01((x as u32).wrapping_mul(97) ^ y as u32) * 0.05;
                data[index] = to_byte(0.40 + grain);
                data[index + 1] = to_byte(0.38 + grain);
                data[index + 2] = to_byte(0.34 + grain);
                data[index + 3] = 255;
                continue;
            }
            let cx = x % cell_w;
            let window = cx > cell_w / 4
                && cx < cell_w * 3 / 4
                && (0.28 * size as f32) < y as f32
                && (y as f32) < 0.74 * size as f32;
            let grain = hash01((x as u32).wrapping_mul(131) ^ y as u32) * 0.08;
            let mut color = if warm {
                [0.55 + grain, 0.40 + grain, 0.27 + grain]
            } else {
                [0.62 + grain, 0.57 + grain, 0.50 + grain]
            };
            if window {
                // DAY: windows are always dark reflective glass.
                color = if warm {
                    [0.10, 0.11, 0.12]
                } else {
                    [0.09, 0.11, 0.13]
                };
            }
            data[index] = to_byte(color[0]);
            data[index + 1] = to_byte(color[1]);
            data[index + 2] = to_byte(color[2]);
            data[index + 3] = 255;
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: SASH_TEX,
            height: SASH_TEX,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = repeat_sampler();
    image
}

fn repeat_sampler() -> ImageSampler {
    ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
        address_mode_u: bevy::image::ImageAddressMode::Repeat,
        address_mode_v: bevy::image::ImageAddressMode::Repeat,
        address_mode_w: bevy::image::ImageAddressMode::Repeat,
        mag_filter: bevy::image::ImageFilterMode::Linear,
        min_filter: bevy::image::ImageFilterMode::Linear,
        mipmap_filter: bevy::image::ImageFilterMode::Linear,
        ..default()
    })
}

/// Night tile: one row, warm scatter for residential and the regular grid for
/// commercial. Repeat-safe because each family owns its own texture.
fn proxy_windows_texture(warm: bool) -> Image {
    let size = SASH_TEX as usize;
    let plain_from = (size as f32 * 0.95) as usize;
    let mut data = vec![0u8; size * size * 4];
    if warm {
        let columns = 9usize;
        let cell_w = size / columns;
        for y in 0..size {
            for x in 0..plain_from {
                let column = x / cell_w;
                let in_cell = x % cell_w;
                let cluster = hash01(column as u32 / 3);
                let roll = hash01((column as u32).wrapping_mul(0x27D4_EB2F) ^ (y as u32 / 11));
                if cluster > 0.30 && roll < 0.62 && in_cell > cell_w / 3 {
                    let temperature = 0.58 + 0.14 * hash01(column as u32);
                    let index = (y * size + x) * 4;
                    data[index] = to_byte(1.0 * temperature);
                    data[index + 1] = to_byte(0.66 * temperature);
                    data[index + 2] = to_byte(0.38 * temperature);
                    data[index + 3] = 255;
                }
            }
        }
    } else {
        let columns = 6usize;
        let cell_w = size / columns;
        for y in 0..size {
            for x in 0..plain_from {
                let column = x / cell_w;
                let in_cell = x % cell_w;
                let on = hash01((column as u32).wrapping_mul(0x9E37_79B9) ^ 0x51ED) < 0.55;
                if on && in_cell > cell_w / 4 && in_cell < cell_w * 3 / 4 {
                    let warm = 0.70 + 0.30 * hash01(column as u32);
                    let index = (y * size + x) * 4;
                    data[index] = to_byte(warm);
                    data[index + 1] = to_byte(warm * 0.86);
                    data[index + 2] = to_byte(warm * 0.62);
                    data[index + 3] = 255;
                }
            }
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: SASH_TEX,
            height: SASH_TEX,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = repeat_sampler();
    image
}

fn proxy_material(
    images: &mut Assets<Image>,
    materials: &mut Assets<StandardMaterial>,
    warm: bool,
) -> Handle<StandardMaterial> {
    let day = images.add(proxy_day_texture(warm));
    let night = images.add(proxy_windows_texture(warm));
    materials.add(StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(day),
        emissive_texture: Some(night),
        emissive: LinearRgba::BLACK,
        perceptual_roughness: 0.92,
        metallic: 0.0,
        ..default()
    })
}

/// Strategy (a): replace the single banked proxy material with one material per
/// family. Two materials, two textures; repeat is safe in both axes and the
/// sampler is Repeat (the old Clamp sampler was smearing edge pixels — that was
/// half of the "windows on one edge and the top floor" defect).
fn install_proxy_windows(
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    geology: Option<ResMut<crate::geology::Geology>>,
) {
    let Some(mut geology) = geology else {
        return;
    };
    let cool = proxy_material(&mut images, &mut materials, false);
    let warm = proxy_material(&mut images, &mut materials, true);
    geology.box_material = cool;
    geology.box_material_warm = warm;
}

fn proxy_night_windows(
    scene: Res<SceneTime>,
    geology: Option<Res<crate::geology::Geology>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Some(geology) = geology else {
        return;
    };
    let night = night_factor(scene.hours);
    for handle in [&geology.box_material, &geology.box_material_warm] {
        if let Some(mut material) = materials.get_mut(handle) {
            material.emissive = LinearRgba::rgb(2.1 * night, 1.5 * night, 0.85 * night);
        }
    }
}

#[derive(Resource, Default)]
struct ScaleOracleState {
    stage: u8,
    since: f32,
    walkers: usize,
    vehicles: usize,
    distance: f32,
}

/// Stage the curb-side template against the sedan nearest `camera`. Returns
/// (vehicle distance, walker count, vehicle count) for the log. Called every
/// frame while the oracle holds, so the subject stays centred as it drives.
fn stage_oracle_pose(world: &mut World, camera: Vec3) -> Option<(f32, usize, usize)> {
    let walkers = world
        .get_resource::<crate::pedestrians::Pedestrians>()
        .map(|peds| peds.states.len())
        .unwrap_or(0);
    let fleet = world
        .get_resource::<crate::traffic_seattle::Traffic>()
        .map(|traffic| {
            traffic
                .states
                .keys()
                .filter_map(|entity| world.get::<Transform>(*entity))
                .map(|transform| (transform.translation, transform.rotation))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let (position, rotation) = *fleet
        .iter()
        .min_by(|left, right| left.0.distance(camera).total_cmp(&right.0.distance(camera)))?;
    let lane = rotation * Vec3::NEG_Z;
    let (eye, look) = crate::camera_authority::curb_side_eye(position, lane);
    {
        let mut staged = world.resource_mut::<crate::camera_authority::StagedPose>();
        crate::camera_authority::stage_look_at(
            &mut staged,
            "scale_oracle",
            eye,
            look,
            "scale_oracle",
        );
    }
    Some((position.distance(camera), walkers, fleet.len()))
}

/// SCALE_ORACLE=1: the curb-side template applied to the sedan nearest the
/// boot camera, so the frame is a vehicle-and-house scale reference. The
/// pedestrian field has no walkers outside its 2.5 km downtown falloff, so the
/// walker half of the gate is reported, not staged.
fn scale_oracle(world: &mut World) {
    if std::env::var_os("SCALE_ORACLE").is_none() {
        return;
    }
    let elapsed = world.resource::<Time>().elapsed_secs();
    let (stage, since) = {
        let state = world.resource::<ScaleOracleState>();
        (state.stage, state.since)
    };
    if stage == 3 {
        return;
    }
    if stage == 0 && elapsed < 10.0 {
        return;
    }
    let Ok(camera) = world
        .query_filtered::<&Transform, With<crate::camera::FlyCamera>>()
        .single(world)
        .map(|transform| transform.translation)
    else {
        return;
    };
    // Walker first: the round closes with a walker in the frame. A sedan must
    // be within 80 m; if none is, the log says so rather than pretending.
    let walkers = world
        .get_resource::<crate::pedestrians::Pedestrians>()
        .map(|peds| {
            peds.states
                .keys()
                .filter_map(|entity| world.get::<Transform>(*entity))
                .map(|transform| (transform.translation, transform.rotation))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let nearest_walker = walkers
        .iter()
        .min_by(|a, b| a.0.distance(camera).total_cmp(&b.0.distance(camera)))
        .copied();
    let vehicles = world
        .get_resource::<crate::traffic_seattle::Traffic>()
        .map(|traffic| {
            traffic
                .states
                .keys()
                .filter_map(|entity| world.get::<Transform>(*entity))
                .map(|transform| transform.translation)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let with_vehicle = walkers
        .iter()
        .filter(|(pos, _)| pos.distance(camera) < 220.0)
        .map(|(pos, rotation)| {
            let vehicle_distance = vehicles
                .iter()
                .map(|vehicle| vehicle.distance(*pos))
                .fold(f32::MAX, f32::min);
            (vehicle_distance, *pos, *rotation)
        })
        .filter(|(vehicle_distance, _, _)| *vehicle_distance < 60.0)
        .min_by(|a, b| a.0.total_cmp(&b.0));
    let chosen = with_vehicle
        .map(|(_, pos, rotation)| (pos, rotation))
        .or_else(|| nearest_walker.filter(|(pos, _)| pos.distance(camera) < 220.0));
    if let Some((target, rotation)) = chosen.filter(|_| stage == 0) {
        let forward = rotation * Vec3::NEG_Z;
        let side = Vec3::new(-forward.z, 0.0, forward.x);
        let eye = target - forward * 8.0 + side * 5.0 + Vec3::Y * 2.3;
        let look = target + Vec3::Y * 1.1;
        {
            let mut staged = world.resource_mut::<crate::camera_authority::StagedPose>();
            crate::camera_authority::stage_look_at(
                &mut staged,
                "scale_oracle",
                eye,
                look,
                "scale_oracle",
            );
        }
        let nearest_vehicle = vehicles
            .iter()
            .map(|vehicle| vehicle.distance(eye))
            .fold(f32::MAX, f32::min);
        if stage == 0 {
            info!(
                "SCALE ORACLE walkers={} vehicles={} nearest_vehicle={nearest_vehicle:.1} m walker_eye={:?}",
                walkers.len(),
                vehicles.len(),
                eye
            );
        }
        let mut state = world.resource_mut::<ScaleOracleState>();
        state.stage = 1;
        state.since = elapsed;
        state.walkers = walkers.len();
        state.vehicles = vehicles.len();
        state.distance = nearest_vehicle;
        return;
    }
    let Some((distance, walkers, vehicles)) = stage_oracle_pose(world, camera) else {
        info!("SCALE ORACLE: no vehicles in the world");
        world.resource_mut::<ScaleOracleState>().stage = 3;
        return;
    };
    if stage == 0 {
        let pose = world.resource::<crate::camera_authority::StagedPose>();
        info!(
            "SCALE ORACLE walkers={walkers} vehicles={vehicles} vehicle_distance={distance:.1} m eye={:?}",
            pose.translation
        );
        let mut state = world.resource_mut::<ScaleOracleState>();
        state.stage = 1;
        state.since = elapsed;
        state.walkers = walkers;
        state.vehicles = vehicles;
        state.distance = distance;
        return;
    }
    let ready = world
        .get_resource::<crate::geology::Geology>()
        .map(|geology| {
            let pose = world.resource::<crate::camera_authority::StagedPose>();
            let center = Vec2::new(
                pose.translation.x - crate::citypack::STREAM_WORLD_OFFSET,
                pose.translation.z,
            );
            geology.pack.neighborhoods().iter().any(|index| {
                let bounds = geology.pack.nodes[*index].bounds;
                center.x >= bounds[0]
                    && center.x <= bounds[2]
                    && center.y >= bounds[1]
                    && center.y <= bounds[3]
                    && geology.building_chunks.contains_key(index)
            })
        })
        .unwrap_or(false);
    if stage == 1 && ready && elapsed - since > 1.5 {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("captures/scale_oracle.png");
        world
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
        let mut state = world.resource_mut::<ScaleOracleState>();
        state.stage = 2;
        state.since = elapsed;
        return;
    }
    if stage == 2 && elapsed - since > 2.0 {
        world.write_message(AppExit::Success);
    }
}

/// SKIN_AUDIT: per-frame skinning histogram, printed every two seconds. The
/// player's ">90 % tan blocks" report needs numbers, not impressions: this
/// separates skinned shells, bare shells and proxy-only entities.
#[derive(Resource, Default)]
pub struct SkinAudit {
    next_at: f32,
}

fn skin_audit(world: &mut World) {
    let now = world.resource::<Time>().elapsed_secs();
    if now < world.resource::<SkinAudit>().next_at {
        return;
    }
    world.resource_mut::<SkinAudit>().next_at = now + 2.0;
    let Some(skins) = world.get_resource::<FacadeSkins>().cloned() else {
        return;
    };
    let Some(geology) = world.get_resource::<crate::geology::Geology>() else {
        return;
    };
    let mut skinned = 0usize;
    let mut bare = 0usize;
    let mut proxies = 0usize;
    let mut shell_buildings = 0usize;
    let mut proxy_buildings = 0usize;
    for (index, entities) in &geology.building_chunks {
        let records = geology.pack.nodes[*index].buildings as usize;
        let mut shells = 0usize;
        for entity in entities {
            if world
                .get::<destructibles_bevy::building::Building>(*entity)
                .is_some()
            {
                shells += 1;
            }
        }
        shell_buildings += shells;
        proxy_buildings += records.saturating_sub(shells);
    }
    for entities in geology.building_chunks.values() {
        for entity in entities {
            let Some(building) = world.get::<destructibles_bevy::building::Building>(*entity)
            else {
                proxies += 1;
                continue;
            };
            let family = world
                .resource::<Buildings>()
                .get(building.id)
                .map(|record| record.family);
            let expected =
                family.and_then(|family| skins.skin(family).map(|skin| skin.wall.clone()));
            let Some(record) = world.resource::<Buildings>().get(building.id) else {
                proxies += 1;
                continue;
            };
            let current = world
                .get::<MeshMaterial3d<StandardMaterial>>(record.wall_mesh)
                .map(|material| material.0.clone());
            match (expected, current) {
                (Some(expected), Some(current)) if expected == current => skinned += 1,
                _ => bare += 1,
            }
        }
    }
    if skinned + bare + proxies > 0 {
        let total = (skinned + bare + proxies) as f32;
        let represented = (shell_buildings + proxy_buildings) as f32;
        info!(
            "SKIN AUDIT t={now:.0} skinned={skinned} bare={bare} proxy_entities={proxies} \
             shell_share={:.1}% shell_buildings={shell_buildings} proxy_buildings={proxy_buildings} proxy_building_share={:.1}%",
            100.0 * skinned as f32 / total,
            if represented > 0.0 {
                100.0 * proxy_buildings as f32 / represented
            } else {
                0.0
            }
        );
    }
}

pub struct FacadePlugin;

impl Plugin for FacadePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SkinGuard>()
            .init_resource::<ScaleOracleState>()
            .init_resource::<NightProfile>()
            .init_resource::<SkinAudit>()
            .add_systems(Startup, setup_facades)
            .add_systems(PostStartup, install_proxy_windows)
            .add_systems(
                Update,
                (
                    seattle_night_density.after(update_window_lights),
                    storefront_night_glow,
                    proxy_night_windows,
                    scale_oracle,
                    skin_audit,
                    skin_guard,
                ),
            );
    }
}
