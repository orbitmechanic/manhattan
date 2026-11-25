//! Seattle landmarks: authored component assemblies that participate in the
//! destructibles machinery (shells at load, tier-up on first impact, journals,
//! collapse, restore).
//!
//! Each landmark is a stack of separable components. The assembly is the
//! support topology: a component's phase says when the director may start
//! damaging it. Signature components (the Needle's stem and saucer, the
//! Market's sign and clock, the bridges' arch/leaves, the stadium roofs) must
//! stay intact through at least half of the nine-act sequence; the validation
//! test rejects a profile that sheds them earlier.

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

use crate::camera::FlyCamera;
use crate::citypack::STREAM_WORLD_OFFSET;
use crate::geology::{Geology, s2_enabled};
use destructibles_bevy::building::{BuildingDetail, spawn_building_world_lod};
use destructibles_core::family::MaterialFamily;
use destructibles_core::{BuildingSpec, FacadeStyle, core_for_style};

pub const ACTS: u8 = 9;
/// Reserved building-id range for landmark components; the Shell Doctrine
/// assertions treat these as damageable residents rather than streamed shells.
pub const LANDMARK_ID_BASE: u32 = 930_000;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Rect,
    Disc,
}

pub struct ComponentDef {
    pub name: &'static str,
    pub offset: [f32; 2],
    pub base_y: f32,
    pub size: [f32; 2],
    pub height: f32,
    pub shape: Shape,
    pub family: MaterialFamily,
    pub facade: FacadeStyle,
    pub phase: u8,
    pub signature: bool,
}

pub struct LandmarkDef {
    pub key: &'static str,
    pub name: &'static str,
    /// Local world anchor (metres, +X east, +Z south).
    pub anchor: [f32; 2],
    pub components: &'static [ComponentDef],
}

#[allow(clippy::too_many_arguments)]
const fn comp(
    name: &'static str,
    offset: [f32; 2],
    base_y: f32,
    size: [f32; 2],
    height: f32,
    shape: Shape,
    family: MaterialFamily,
    facade: FacadeStyle,
    phase: u8,
    signature: bool,
) -> ComponentDef {
    ComponentDef {
        name,
        offset,
        base_y,
        size,
        height,
        shape,
        family,
        facade,
        phase,
        signature,
    }
}

pub const NEEDLE: [ComponentDef; 6] = [
    comp(
        "pedestal",
        [0.0, 0.0],
        0.0,
        [46.0, 46.0],
        12.0,
        Shape::Disc,
        MaterialFamily::MasonryStone,
        FacadeStyle::BrickBrownstone,
        0,
        false,
    ),
    comp(
        "stem",
        [0.0, 0.0],
        12.0,
        [13.0, 13.0],
        134.0,
        Shape::Disc,
        MaterialFamily::SteelFrame,
        FacadeStyle::GlassCurtainWall,
        5,
        true,
    ),
    comp(
        "waist",
        [0.0, 0.0],
        146.0,
        [26.0, 26.0],
        10.0,
        Shape::Disc,
        MaterialFamily::SteelFrame,
        FacadeStyle::GlassCurtainWall,
        5,
        false,
    ),
    comp(
        "saucer",
        [0.0, 0.0],
        156.0,
        [48.0, 48.0],
        14.0,
        Shape::Disc,
        MaterialFamily::SteelFrame,
        FacadeStyle::GlassCurtainWall,
        5,
        true,
    ),
    comp(
        "crown",
        [0.0, 0.0],
        170.0,
        [12.0, 12.0],
        10.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::GlassCurtainWall,
        3,
        false,
    ),
    comp(
        "spire",
        [0.0, 0.0],
        180.0,
        [6.0, 6.0],
        14.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::ConcreteGrid,
        3,
        false,
    ),
];

pub const MARKET: [ComponentDef; 5] = [
    comp(
        "terraces",
        [0.0, 0.0],
        0.0,
        [70.0, 42.0],
        10.0,
        Shape::Rect,
        MaterialFamily::MasonryStone,
        FacadeStyle::BrickBrownstone,
        0,
        false,
    ),
    comp(
        "arcade",
        [0.0, -6.0],
        10.0,
        [64.0, 24.0],
        7.0,
        Shape::Rect,
        MaterialFamily::Timber,
        FacadeStyle::BrickMidrise,
        1,
        false,
    ),
    comp(
        "sign_block",
        [6.0, -20.0],
        10.0,
        [20.0, 6.0],
        16.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::BrickMidrise,
        5,
        true,
    ),
    comp(
        "clock",
        [-8.0, -20.0],
        18.0,
        [7.0, 7.0],
        7.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::GlassCurtainWall,
        5,
        true,
    ),
    comp(
        "sheds",
        [24.0, 8.0],
        0.0,
        [26.0, 16.0],
        6.0,
        Shape::Rect,
        MaterialFamily::Timber,
        FacadeStyle::BrickBrownstone,
        2,
        false,
    ),
];

pub const AURORA: [ComponentDef; 6] = [
    comp(
        "deck_a",
        [-60.0, 0.0],
        46.0,
        [50.0, 14.0],
        2.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::ConcreteGrid,
        0,
        false,
    ),
    comp(
        "deck_b",
        [60.0, 0.0],
        46.0,
        [50.0, 14.0],
        2.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::ConcreteGrid,
        0,
        false,
    ),
    comp(
        "arch",
        [0.0, 0.0],
        46.0,
        [120.0, 10.0],
        26.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::GlassCurtainWall,
        5,
        true,
    ),
    comp(
        "tower_n",
        [-70.0, 0.0],
        0.0,
        [8.0, 8.0],
        46.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::ConcreteGrid,
        2,
        false,
    ),
    comp(
        "tower_s",
        [70.0, 0.0],
        0.0,
        [8.0, 8.0],
        46.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::ConcreteGrid,
        2,
        false,
    ),
    comp(
        "approach",
        [-140.0, 0.0],
        26.0,
        [80.0, 10.0],
        2.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::ConcreteGrid,
        1,
        false,
    ),
];

pub const FREMONT: [ComponentDef; 5] = [
    comp(
        "deck_a",
        [-40.0, 0.0],
        20.0,
        [40.0, 12.0],
        2.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::ConcreteGrid,
        0,
        false,
    ),
    comp(
        "deck_b",
        [40.0, 0.0],
        20.0,
        [40.0, 12.0],
        2.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::ConcreteGrid,
        0,
        false,
    ),
    comp(
        "leaf_blue",
        [0.0, 6.0],
        20.0,
        [40.0, 10.0],
        14.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::GlassCurtainWall,
        5,
        true,
    ),
    comp(
        "tower",
        [0.0, 0.0],
        0.0,
        [10.0, 10.0],
        20.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::ConcreteGrid,
        3,
        false,
    ),
    comp(
        "approach",
        [90.0, 0.0],
        12.0,
        [60.0, 10.0],
        2.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::ConcreteGrid,
        1,
        false,
    ),
];

pub const BALLARD: [ComponentDef; 5] = [
    comp(
        "deck_a",
        [-50.0, 0.0],
        22.0,
        [50.0, 12.0],
        2.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::ConcreteGrid,
        0,
        false,
    ),
    comp(
        "deck_b",
        [50.0, 0.0],
        22.0,
        [50.0, 12.0],
        2.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::ConcreteGrid,
        0,
        false,
    ),
    comp(
        "leaf_steel",
        [0.0, 7.0],
        22.0,
        [46.0, 12.0],
        16.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::GlassCurtainWall,
        5,
        true,
    ),
    comp(
        "tower",
        [0.0, 0.0],
        0.0,
        [10.0, 10.0],
        22.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::ConcreteGrid,
        3,
        false,
    ),
    comp(
        "approach",
        [105.0, 0.0],
        12.0,
        [70.0, 10.0],
        2.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::ConcreteGrid,
        1,
        false,
    ),
];

pub const MONORAIL_SPANS: [ComponentDef; 12] = [
    comp(
        "span_00",
        [442.8, 525.9],
        12.0,
        [18.0, 6.0],
        2.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::ConcreteGrid,
        0,
        false,
    ),
    comp(
        "span_01",
        [362.3, 430.3],
        12.0,
        [18.0, 6.0],
        2.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::ConcreteGrid,
        0,
        false,
    ),
    comp(
        "span_02",
        [281.8, 334.7],
        12.0,
        [18.0, 6.0],
        2.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::ConcreteGrid,
        0,
        false,
    ),
    comp(
        "span_03",
        [201.3, 239.0],
        12.0,
        [18.0, 6.0],
        2.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::ConcreteGrid,
        0,
        false,
    ),
    comp(
        "span_04",
        [120.8, 143.4],
        12.0,
        [18.0, 6.0],
        2.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::ConcreteGrid,
        1,
        false,
    ),
    comp(
        "span_05",
        [40.3, 47.8],
        12.0,
        [18.0, 6.0],
        2.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::ConcreteGrid,
        1,
        false,
    ),
    comp(
        "span_06",
        [-40.3, -47.8],
        12.0,
        [18.0, 6.0],
        2.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::ConcreteGrid,
        5,
        true,
    ),
    comp(
        "span_07",
        [-120.8, -143.4],
        12.0,
        [18.0, 6.0],
        2.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::ConcreteGrid,
        5,
        true,
    ),
    comp(
        "span_08",
        [-201.3, -239.0],
        12.0,
        [18.0, 6.0],
        2.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::ConcreteGrid,
        1,
        false,
    ),
    comp(
        "span_09",
        [-281.8, -334.7],
        12.0,
        [18.0, 6.0],
        2.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::ConcreteGrid,
        1,
        false,
    ),
    comp(
        "span_10",
        [-362.3, -430.3],
        12.0,
        [18.0, 6.0],
        2.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::ConcreteGrid,
        0,
        false,
    ),
    comp(
        "span_11",
        [-442.8, -525.9],
        12.0,
        [18.0, 6.0],
        2.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::ConcreteGrid,
        0,
        false,
    ),
];

pub const ARENA: [ComponentDef; 2] = [
    comp(
        "bowl",
        [0.0, 0.0],
        0.0,
        [85.0, 65.0],
        20.0,
        Shape::Rect,
        MaterialFamily::MasonryStone,
        FacadeStyle::ConcreteGrid,
        0,
        false,
    ),
    comp(
        "roof",
        [0.0, 0.0],
        20.0,
        [76.0, 56.0],
        7.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::ConcreteGrid,
        5,
        true,
    ),
];

pub const LUMEN: [ComponentDef; 3] = [
    comp(
        "bowl",
        [0.0, 0.0],
        0.0,
        [110.0, 90.0],
        26.0,
        Shape::Rect,
        MaterialFamily::MasonryStone,
        FacadeStyle::ConcreteGrid,
        0,
        false,
    ),
    comp(
        "canopy_n",
        [0.0, -30.0],
        26.0,
        [100.0, 26.0],
        8.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::GlassCurtainWall,
        5,
        true,
    ),
    comp(
        "canopy_s",
        [0.0, 30.0],
        26.0,
        [100.0, 26.0],
        8.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::GlassCurtainWall,
        5,
        true,
    ),
];

pub const TMOBILE: [ComponentDef; 3] = [
    comp(
        "bowl",
        [0.0, 0.0],
        0.0,
        [115.0, 95.0],
        24.0,
        Shape::Rect,
        MaterialFamily::MasonryStone,
        FacadeStyle::ConcreteGrid,
        0,
        false,
    ),
    comp(
        "roof_panel_a",
        [0.0, -34.0],
        24.0,
        [105.0, 30.0],
        6.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::GlassCurtainWall,
        5,
        true,
    ),
    comp(
        "roof_panel_b",
        [0.0, 34.0],
        24.0,
        [105.0, 30.0],
        6.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::GlassCurtainWall,
        5,
        true,
    ),
];

pub const LANDMARKS: [LandmarkDef; 9] = [
    LandmarkDef {
        key: "needle",
        name: "Space Needle",
        anchor: [-1006.1, -1609.9],
        components: &NEEDLE,
    },
    LandmarkDef {
        key: "market",
        name: "Pike Place Market",
        anchor: [-518.6, -399.4],
        components: &MARKET,
    },
    LandmarkDef {
        key: "aurora",
        name: "Aurora Bridge",
        anchor: [-829.0, -4497.7],
        components: &AURORA,
    },
    LandmarkDef {
        key: "fremont",
        name: "Fremont Bridge",
        anchor: [-976.6, -4634.0],
        components: &FREMONT,
    },
    LandmarkDef {
        key: "ballard",
        name: "Ballard Bridge",
        anchor: [-2947.5, -6029.9],
        components: &BALLARD,
    },
    LandmarkDef {
        key: "monorail",
        name: "Seattle Center Monorail",
        anchor: [-604.9, -1168.3],
        components: &MONORAIL_SPANS,
    },
    LandmarkDef {
        key: "arena",
        name: "Climate Pledge Arena",
        anchor: [-1355.8, -1794.7],
        components: &ARENA,
    },
    LandmarkDef {
        key: "lumen",
        name: "Lumen Field",
        anchor: [269.4, 1228.5],
        components: &LUMEN,
    },
    LandmarkDef {
        key: "tmobile",
        name: "T-Mobile Park",
        anchor: [193.5, 1649.6],
        components: &TMOBILE,
    },
];

/// Signature components must survive intact for at least half the acts. A
/// component that starts taking damage at `phase` survives `phase / ACTS`.
pub fn signature_survival(component: &ComponentDef) -> f32 {
    component.phase as f32 / ACTS as f32
}

pub fn profile_valid(landmark: &LandmarkDef) -> Result<(), String> {
    if landmark.components.len() < 2 {
        return Err(format!("{} has no assembly", landmark.name));
    }
    let signatures = landmark.components.iter().filter(|c| c.signature).count();
    if signatures == 0 {
        return Err(format!("{} has no signature component", landmark.name));
    }
    for component in landmark.components {
        if component.phase >= ACTS {
            return Err(format!(
                "{}/{} phase {} out of range",
                landmark.name, component.name, component.phase
            ));
        }
        if component.signature && signature_survival(component) < 0.5 {
            return Err(format!(
                "{}/{} signature sheds at {:.0}% (<50%)",
                landmark.name,
                component.name,
                signature_survival(component) * 100.0
            ));
        }
    }
    Ok(())
}

#[derive(Component)]
pub struct LandmarkSign {
    pub block: u32,
}

#[derive(Resource, Default)]
pub struct MarketSigns {
    pub sign: Handle<StandardMaterial>,
    pub clock: Handle<StandardMaterial>,
}

pub struct LandmarkInstance {
    pub def: usize,
    pub components: Vec<(u32, Vec3, f32)>,
}

#[derive(Resource, Default)]
pub struct Landmarks {
    pub instances: Vec<LandmarkInstance>,
}

pub struct LandmarksPlugin;

impl Plugin for LandmarksPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Landmarks>()
            .init_resource::<LandmarkGate>()
            .init_resource::<LandmarkCycle>()
            .add_systems(
                Update,
                (
                    landmark_gate,
                    landmark_cycle,
                    landmarks_always,
                    sign_theater,
                    blink_needle_beacon,
                    seat_needle,
                    needle_band_night,
                ),
            );
    }
}

fn footprint(component: &ComponentDef) -> Vec<[f32; 2]> {
    match component.shape {
        Shape::Rect => {
            let (w, d) = (component.size[0] * 0.5, component.size[1] * 0.5);
            vec![[-w, -d], [w, -d], [w, d], [-w, d]]
        }
        Shape::Disc => {
            let (rx, rz) = (component.size[0] * 0.5, component.size[1] * 0.5);
            (0..16)
                .map(|index| {
                    let angle = std::f32::consts::TAU * index as f32 / 16.0;
                    [rx * angle.cos(), rz * angle.sin()]
                })
                .collect()
        }
    }
}

fn component_spec(index: usize, component: &ComponentDef) -> BuildingSpec {
    let floor_height = 4.0_f32;
    let floor_count = (component.height / floor_height).round().max(1.0) as u32;
    let mut windows = component.facade.windows();
    windows.height = (floor_height - 0.4).clamp(1.1, 3.0);
    windows.sill_height = 0.6;
    let perimeter = footprint(component);
    BuildingSpec {
        family: component.family,
        core: core_for_style(component.facade, &perimeter, 920_000 + index as u32),
        footprint: perimeter,
        facade: component.facade.material(),
        facade_style: component.facade,
        floor_count,
        floor_height,
        windows,
    }
}

#[derive(Component)]
pub struct NeedleBeacon {
    material: Handle<StandardMaterial>,
}

/// Blink the beacon: 1 Hz, on for a third of the cycle (FAA-style).
pub fn blink_needle_beacon(
    time: Res<Time>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    beacons: Query<&NeedleBeacon>,
) {
    let on = (time.elapsed_secs() * 1.0).fract() < 0.33;
    for beacon in &beacons {
        if let Some(mut material) = materials.get_mut(&beacon.material) {
            material.emissive = if on {
                LinearRgba::rgb(16.0, 0.8, 0.4)
            } else {
                LinearRgba::rgb(1.2, 0.06, 0.03)
            };
        }
    }
}

/// GLB bounds from the conversion receipt (accessor min/max, Z-up, spire at −Z).
pub const NEEDLE_GLB_HEIGHT: f32 = 807.3;
/// Real Needle: top of the spire, metres (605 ft).
pub const NEEDLE_TARGET_HEIGHT: f32 = 184.4;
/// Observation deck ring elevation (520 ft), the second point of the scale fit.
pub const NEEDLE_RING_HEIGHT: f32 = 158.5;
/// Crown glazing top and the leg restaurant band.
pub const NEEDLE_CROWN_HEIGHT: f32 = 171.0;
pub const NEEDLE_RESTAURANT_HEIGHT: f32 = 96.0;

/// The real Needle (player-supplied CAD, ~6k tris) is the ONLY Needle on the
/// default boot. The glTF scene is walked as primitives and parented under our
/// own transform: the engine's WorldAsset root does not honour an entity
/// Transform in this build, which is what hung the model in earlier rounds.
/// Seating is self-correcting: after the first frame the system measures our own
/// children's world AABB and shifts the parent so the base rests on the DEM.
fn spawn_needle_model(world: &mut World, landmark: &LandmarkDef, _components: &[(u32, Vec3, f32)]) {
    let (x, z) = (landmark.anchor[0], landmark.anchor[1]);
    let ground = world.resource::<Geology>().dem.height(x, z);
    let asset_server = world.resource::<AssetServer>().clone();
    let gltf: Handle<bevy::gltf::Gltf> = asset_server.load("models/space_needle_low.glb");
    let scale = NEEDLE_TARGET_HEIGHT / NEEDLE_GLB_HEIGHT;
    world.spawn((
        NeedleModel {
            gltf,
            ground,
            anchor: Vec3::new(x + STREAM_WORLD_OFFSET, ground, z),
            scale,
            stage: 0,
        },
        Transform::from_xyz(x + STREAM_WORLD_OFFSET, ground, z),
        Visibility::default(),
        Name::new("Space Needle"),
    ));
    info!(
        "NEEDLE: CAD spawn at ({x:.0},{z:.0}) ground {ground:.1} m; scale {scale:.5}; pivot-compensated seat to follow"
    );
}

/// Pending CAD model: the GLB primitives are spawned under this entity, then the
/// parent's translation is corrected from the measured world AABB.
#[derive(Component)]
pub struct NeedleModel {
    gltf: Handle<bevy::gltf::Gltf>,
    ground: f32,
    anchor: Vec3,
    scale: f32,
    stage: u8,
}

/// Spawn the primitives once the GLB is loaded, then seat the model on the DEM.
#[allow(clippy::too_many_arguments)]
pub fn seat_needle(
    mut commands: Commands,
    mut models: Query<(Entity, &mut NeedleModel, &mut Transform, Option<&Children>)>,
    gltfs: Res<Assets<bevy::gltf::Gltf>>,
    gltf_meshes: Res<Assets<bevy::gltf::GltfMesh>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mesh_children: Query<(Entity, &Mesh3d)>,
) {
    for (entity, mut model, mut transform, children) in &mut models {
        if model.stage == 0 {
            let Some(gltf) = gltfs.get(&model.gltf) else {
                continue;
            };
            let shell = materials.add(StandardMaterial {
                base_color: Color::srgb(0.86, 0.87, 0.88),
                metallic: 0.25,
                perceptual_roughness: 0.55,
                ..default()
            });
            transform.rotation =
                Quat::from_rotation_y(0.35) * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
            transform.scale = Vec3::splat(model.scale);
            transform.translation = Vec3::new(model.anchor.x, model.ground, model.anchor.z);
            let mut spawned = 0usize;
            for mesh_handle in &gltf.meshes {
                let Some(mesh) = gltf_meshes.get(mesh_handle) else {
                    continue;
                };
                for primitive in &mesh.primitives {
                    let child = commands
                        .spawn((
                            Mesh3d(primitive.mesh.clone()),
                            MeshMaterial3d(shell.clone()),
                            Transform::IDENTITY,
                            Name::new("needle primitive"),
                        ))
                        .id();
                    commands.entity(entity).add_child(child);
                    spawned += 1;
                }
            }
            if spawned == 0 {
                continue;
            }
            model.stage = 1;
            info!("NEEDLE: {spawned} primitives parented under the seat transform");
            continue;
        }
        if model.stage == 1 {
            let Some(children) = children else {
                continue;
            };
            // The AABB comes from the MESH vertices, not the entity origins:
            // an entity's GlobalTransform is its pivot, which is exactly the
            // mistake that made earlier measurements read zero.
            let mut min = f32::MAX;
            let mut max = f32::MIN;
            let mut seen = 0usize;
            let scale = model.scale;
            for (child, mesh3d) in mesh_children.iter() {
                if !children.contains(&child) {
                    continue;
                }
                let Some(mesh) = meshes.get(&mesh3d.0) else {
                    continue;
                };
                if let Some(bevy::mesh::VertexAttributeValues::Float32x3(points)) =
                    mesh.attribute(Mesh::ATTRIBUTE_POSITION)
                {
                    for p in points {
                        // R_x(-90): world y = z * scale + T.y
                        let world_y = p[2] * scale + transform.translation.y;
                        min = min.min(world_y);
                        max = max.max(world_y);
                        seen += 1;
                    }
                }
            }
            if seen == 0 {
                continue;
            }
            let shift = model.ground - min;
            transform.translation.y += shift;
            model.stage = 2;
            info!(
                "NEEDLE seat: measured min_y={min:.2} max_y={max:.2}; shifted {:+.2} m -> base {:.2}, tip {:.2}",
                shift,
                min + shift,
                max + shift
            );
            assert!(
                (min + shift - model.ground).abs() < 2.0,
                "Needle base {:.2} not within 2 m of ground {:.2}",
                min + shift,
                model.ground
            );
            // Bands and lights ride with the seated model.
            if let Some(mesh) = mesh_children
                .iter()
                .find_map(|(child, mesh3d)| {
                    children
                        .contains(&child)
                        .then(|| meshes.get(&mesh3d.0).cloned())
                        .flatten()
                })
                .as_ref()
                .and_then(|mesh| detect_needle_anatomy(mesh, scale, model.ground))
            {
                spawn_needle_bands_command(
                    &mut commands,
                    &mut meshes,
                    &mut materials,
                    &mut images,
                    model.anchor.x,
                    model.ground,
                    model.anchor.z,
                    mesh,
                );
            }
            spawn_needle_lights_command(
                &mut commands,
                &mut meshes,
                &mut materials,
                model.anchor.x,
                model.ground,
                model.anchor.z,
            );
        }
    }
}

/// Detected anatomy of the seated model, in metres above the seated base.
#[derive(Clone, Copy, Debug)]
pub struct NeedleAnatomy {
    pub drum_low: f32,
    pub drum_high: f32,
    pub drum_radius: f32,
    pub fin: f32,
    pub restaurant_low: f32,
    pub restaurant_high: f32,
    pub restaurant_radius: f32,
}

/// Walk the seated mesh and derive the components: the drum (widest steady
/// radius band), the fin (local radius maximum at the drum's top edge) and the
/// restaurant (widest band midway down that is not the drum). Radii are in
/// metres; heights are metres above the seated base. Prints its evidence.
pub fn detect_needle_anatomy(mesh: &Mesh, scale: f32, base_y: f32) -> Option<NeedleAnatomy> {
    let Some(bevy::mesh::VertexAttributeValues::Float32x3(points)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        return None;
    };
    // Asset space: z is height, radius in x/y.
    const BINS: usize = 120;
    let mut max_z = f32::MIN;
    let mut min_z = f32::MAX;
    for p in points {
        max_z = max_z.max(p[2]);
        min_z = min_z.min(p[2]);
    }
    let span = (max_z - min_z).max(1.0);
    let mut radius = vec![0.0_f32; BINS];
    for p in points {
        let bin = (((p[2] - min_z) / span) * (BINS - 1) as f32).round() as usize;
        let r = (p[0] * p[0] + p[1] * p[1]).sqrt();
        radius[bin.min(BINS - 1)] = radius[bin.min(BINS - 1)].max(r);
    }
    let elev = |bin: usize| base_y + (min_z + span * bin as f32 / (BINS - 1) as f32) * scale;
    // Drum: the widest steady radius in the upper half, away from the spire.
    let upper = BINS * 6 / 10..BINS * 95 / 100;
    let mut drum_bin = upper.start;
    for bin in upper.clone() {
        if radius[bin] > radius[drum_bin] {
            drum_bin = bin;
        }
    }
    let drum_radius = radius[drum_bin] * scale;
    if drum_radius < 2.0 {
        return None;
    }
    // Extend the drum over neighbouring bins within 85 % of its radius.
    let mut low = drum_bin;
    while low > 0 && radius[low - 1] > radius[drum_bin] * 0.85 {
        low -= 1;
    }
    let mut high = drum_bin;
    while high + 1 < BINS && radius[high + 1] > radius[drum_bin] * 0.85 {
        high += 1;
    }
    // Fin: the local maximum at the drum's top edge (roof overhang).
    let fin_bin = (high + 1).min(BINS - 1);
    // Restaurant: the widest band between 25 % and 60 % of height, not the drum.
    let mid = BINS * 25 / 100..BINS * 60 / 100;
    let mut rest_bin = mid.start;
    for bin in mid.clone() {
        if radius[bin] > radius[rest_bin] && !(low..=high).contains(&bin) {
            rest_bin = bin;
        }
    }
    let mut r_low = rest_bin;
    while r_low > 0 && radius[r_low - 1] > radius[rest_bin] * 0.7 {
        r_low -= 1;
    }
    let mut r_high = rest_bin;
    while r_high + 1 < BINS && radius[r_high + 1] > radius[rest_bin] * 0.7 {
        r_high += 1;
    }
    let anatomy = NeedleAnatomy {
        drum_low: elev(low),
        drum_high: elev(high),
        drum_radius,
        fin: elev(fin_bin),
        restaurant_low: elev(r_low),
        restaurant_high: elev(r_high),
        restaurant_radius: radius[rest_bin] * scale,
    };
    info!(
        "NEEDLE anatomy: drum {:.1}-{:.1} m r={:.1}; fin {:.1} m; restaurant {:.1}-{:.1} m r={:.1}",
        anatomy.drum_low,
        anatomy.drum_high,
        anatomy.drum_radius,
        anatomy.fin,
        anatomy.restaurant_low,
        anatomy.restaurant_high,
        anatomy.restaurant_radius
    );
    Some(anatomy)
}

/// Two glazing rows on the drum (split by the fin) plus a three-plane restaurant
/// band. Built from the detected anatomy, never from the procedural constants.
#[allow(clippy::too_many_arguments)]
fn spawn_needle_bands_command(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    x: f32,
    ground: f32,
    z: f32,
    anatomy: NeedleAnatomy,
) {
    let glass = materials.add(StandardMaterial {
        base_color: Color::srgb(0.05, 0.06, 0.08),
        metallic: 0.55,
        perceptual_roughness: 0.12,
        ..default()
    });
    let texture = images.add(need_bank_texture(3));
    let lit = materials.add(StandardMaterial {
        base_color: Color::BLACK,
        emissive: LinearRgba::rgb(16.0, 12.0, 6.5),
        emissive_texture: Some(texture),
        perceptual_roughness: 0.6,
        unlit: false,
        ..default()
    });
    let rows = [
        (anatomy.drum_low + 0.8, anatomy.fin - 0.8),
        (anatomy.fin + 0.8, anatomy.drum_high - 0.4),
    ];
    for (low, high) in rows {
        let height = (high - low).max(0.8);
        let mesh = meshes.add(
            Cylinder::new(anatomy.drum_radius * 1.005, height)
                .mesh()
                .resolution(64),
        );
        commands.spawn((
            NeedleBand {
                glass: glass.clone(),
                lit: lit.clone(),
            },
            Mesh3d(mesh),
            MeshMaterial3d(glass.clone()),
            Transform::from_xyz(x, ground + (low + high) * 0.5, z),
            Name::new("Needle drum glazing"),
        ));
    }
    // Restaurant: three planar faces around the triangular section.
    let height = (anatomy.restaurant_high - anatomy.restaurant_low).max(0.8);
    let panel = meshes.add(Rectangle::new(anatomy.restaurant_radius * 1.6, height));
    for index in 0..3 {
        let angle = index as f32 * std::f32::consts::TAU / 3.0 + 0.35;
        let radius = anatomy.restaurant_radius * 1.02;
        commands.spawn((
            NeedleBand {
                glass: glass.clone(),
                lit: lit.clone(),
            },
            Mesh3d(panel.clone()),
            MeshMaterial3d(glass.clone()),
            Transform::from_xyz(
                x + angle.cos() * radius,
                ground + (anatomy.restaurant_low + anatomy.restaurant_high) * 0.5,
                z + angle.sin() * radius,
            )
            .with_rotation(Quat::from_rotation_y(-angle + std::f32::consts::FRAC_PI_2)),
            Name::new("Needle restaurant glazing"),
        ));
    }
    info!(
        "NEEDLE bands: 2 drum rows split by the fin at {:.1} m + 3 restaurant planes at {:.1} m",
        anatomy.fin,
        (anatomy.restaurant_low + anatomy.restaurant_high) * 0.5
    );
}

fn spawn_needle_lights_command(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    x: f32,
    ground: f32,
    z: f32,
) {
    let mesh = meshes.add(Rectangle::new(1.6, 1.6));
    let beacon = materials.add(StandardMaterial {
        base_color: Color::BLACK,
        emissive: LinearRgba::rgb(14.0, 0.6, 0.3),
        unlit: true,
        ..default()
    });
    let marker = materials.add(StandardMaterial {
        base_color: Color::BLACK,
        emissive: LinearRgba::rgb(5.0, 4.2, 3.2),
        unlit: true,
        ..default()
    });
    commands.spawn((
        NeedleBeacon {
            material: beacon.clone(),
        },
        Mesh3d(mesh.clone()),
        MeshMaterial3d(beacon.clone()),
        Transform::from_xyz(x, ground + NEEDLE_TARGET_HEIGHT, z).with_scale(Vec3::splat(2.4)),
        Name::new("Needle beacon"),
    ));
    for index in 0..4 {
        let angle = index as f32 * std::f32::consts::FRAC_PI_2 + 0.4;
        commands.spawn((
            Mesh3d(mesh.clone()),
            MeshMaterial3d(marker.clone()),
            Transform::from_xyz(
                x + angle.cos() * 21.0,
                ground + NEEDLE_RING_HEIGHT,
                z + angle.sin() * 21.0,
            ),
            Name::new("Needle ring light"),
        ));
    }
}

/// Window bands on the UV-less CAD shell: authored ring overlays at the three
/// real glazing elevations (observation deck, crown, leg restaurant). Geometry
/// region, not UV projection: a facade atlas smeared over the whole shell would
/// be wrong everywhere except the bands, and the bands are what read at cruise.
fn need_bank_texture(rows: u32) -> Image {
    let width = 128u32;
    let height = 32u32;
    let mut data = vec![0u8; (width * height * 4) as usize];
    for y in 0..height {
        for x in 0..width {
            let index = ((y * width + x) * 4) as usize;
            let cell = (x * 8 / width) + (y * rows / height) * 8;
            let lit = !cell.is_multiple_of(3);
            let warm = !cell.is_multiple_of(5);
            data[index] = if lit { if warm { 255 } else { 190 } } else { 8 };
            data[index + 1] = if lit {
                if warm { 205 } else { 215 }
            } else {
                10
            };
            data[index + 2] = if lit {
                if warm { 140 } else { 235 }
            } else {
                12
            };
            data[index + 3] = 255;
        }
    }
    let mut image = Image::new(
        bevy::render::render_resource::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        data,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD | bevy::asset::RenderAssetUsages::MAIN_WORLD,
    );
    image.sampler = bevy::image::ImageSampler::linear();
    image
}

#[derive(Component)]
pub struct NeedleBand {
    pub glass: Handle<StandardMaterial>,
    pub lit: Handle<StandardMaterial>,
}

/// Swap the band materials at dusk: dark reflective glass by day, emissive rows
/// by night, exactly like every other window in the city.
pub fn needle_band_night(
    env: Res<crate::environment::Environment>,
    bands: Query<(Entity, &NeedleBand, &MeshMaterial3d<StandardMaterial>)>,
    mut commands: Commands,
    mut last: Local<Option<(bool, usize)>>,
) {
    let night = env.night || env.daylight < 0.35;
    let count = bands.iter().count();
    if *last == Some((night, count)) {
        return;
    }
    *last = Some((night, count));
    for (entity, band, material) in &bands {
        let wanted = if night {
            band.lit.clone()
        } else {
            band.glass.clone()
        };
        if material.0 != wanted {
            commands.entity(entity).insert(MeshMaterial3d(wanted));
        }
    }
}

fn spawn_landmarks(world: &mut World) {
    let mut instances = Vec::new();
    // The real Needle replaces the procedural massing live; the shells are not
    // spawned for it unless the fallback flag is set (the component table below
    // still carries the landmark's data for the phase-map records).
    // LIVE-BY-DEFAULT: the CAD model is the Needle. The procedural massing is
    // emergency rollback only (NEEDLE_PROCEDURAL=1), never the default.
    let procedural_needle = std::env::var_os("NEEDLE_PROCEDURAL").is_some();
    for (def_index, landmark) in LANDMARKS.iter().enumerate() {
        let mut components = Vec::new();
        let skip_components = landmark.key == "needle" && !procedural_needle;
        for (index, component) in landmark.components.iter().enumerate() {
            let id = LANDMARK_ID_BASE + (def_index as u32) * 100 + index as u32;
            let x = landmark.anchor[0] + component.offset[0];
            let z = landmark.anchor[1] + component.offset[1];
            let ground = world.resource::<Geology>().dem.height(x, z);
            let origin = Vec3::new(x + STREAM_WORLD_OFFSET, ground + component.base_y + 0.03, z);
            if !skip_components {
                spawn_building_world_lod(
                    world,
                    id,
                    component_spec(def_index * 16 + index, component),
                    origin,
                    BuildingDetail::Shell,
                );
            }
            components.push((id, origin, component.height));
        }
        if landmark.key == "market" {
            spawn_market_signs(world, landmark);
        }
        if landmark.key == "needle" && !procedural_needle {
            // The CAD seat is still unproven (it stacks/hangs against the proven
            // procedural seat), so the live path is the procedural Needle alone.
            // No stacked coexistence, ever: the CAD is strictly opt-in until the
            // primitive-walk AABB gate passes.
            spawn_needle_model(world, landmark, &components);
        }
        info!(
            "LANDMARK {} spawned {} components at ({:.0},{:.0})",
            landmark.name,
            components.len(),
            landmark.anchor[0],
            landmark.anchor[1]
        );
        instances.push(LandmarkInstance {
            def: def_index,
            components,
        });
    }
    world.resource_mut::<Landmarks>().instances = instances;
}

/// Drive the camera every frame; the pilot controller also writes it.
fn apply_stage_pose(world: &mut World, stage: u8, target: usize) {
    let needle = LANDMARKS[target].anchor;
    if LANDMARKS[target].key == "monorail" {
        let ground = ground_at(world, needle[0], needle[1]);
        set_camera(
            world,
            [needle[0] + 150.0, 35.0, needle[1] + 230.0],
            [needle[0], ground + 12.0, needle[1] - 40.0],
        );
        return;
    }
    if LANDMARKS[target].key == "market" {
        let ground = ground_at(world, needle[0], needle[1]);
        set_camera(
            world,
            [needle[0] + 26.0, 26.0, needle[1] + 95.0],
            [needle[0] + 6.0, ground + 23.0, needle[1] - 17.0],
        );
        return;
    }
    if matches!(LANDMARKS[target].key, "arena" | "tmobile") {
        let ground = ground_at(world, needle[0], needle[1]);
        set_camera(
            world,
            [needle[0] + 20.0, 250.0, needle[1] + 150.0],
            [needle[0], ground + 8.0, needle[1]],
        );
        return;
    }
    if LANDMARKS[target].key == "lumen" {
        let top = LANDMARKS[target]
            .components
            .iter()
            .map(|component| component.base_y + component.height)
            .fold(0.0_f32, f32::max);
        let ground = ground_at(world, needle[0], needle[1]);
        set_camera(
            world,
            [needle[0] + 40.0, 110.0, needle[1] + 330.0],
            [needle[0], ground + top * 0.4, needle[1]],
        );
        return;
    }
    if target != 0 {
        // Generic landmark framing: 260 m south of the anchor, 45 m up,
        // looking at the assembly centroid.
        let top = LANDMARKS[target]
            .components
            .iter()
            .map(|component| component.base_y + component.height)
            .fold(0.0_f32, f32::max);
        let ground = ground_at(world, needle[0], needle[1]);
        set_camera(
            world,
            [needle[0] + 40.0, 45.0, needle[1] + 250.0],
            [needle[0], ground + top * 0.45, needle[1]],
        );
        return;
    }
    let ground = ground_at(world, needle[0], needle[1]);
    match stage {
        1 => set_camera(
            world,
            [needle[0], 320.0, needle[1]],
            [needle[0], ground + 90.0, needle[1]],
        ),
        2 => set_camera(
            world,
            [-3600.0, 55.0, -1500.0],
            [needle[0], ground + 90.0, needle[1]],
        ),
        8 => set_camera(
            world,
            [-1489.8, 26.0, -2620.3],
            [needle[0], ground + 95.0, needle[1]],
        ),
        3..=7 => set_camera(
            world,
            [-850.0, 95.0, -1350.0],
            [needle[0], ground + 80.0, needle[1]],
        ),
        _ => {}
    }
}

fn landmark_gate(world: &mut World) {
    if std::env::var_os("LANDMARK_GATE").is_none() || !s2_enabled() {
        return;
    }
    if std::env::var_os("LANDMARK_CYCLE").is_some() {
        return;
    }
    if !world.resource::<LandmarkGate>().spawned {
        spawn_landmarks(world);
        world.resource_mut::<LandmarkGate>().spawned = true;
        return;
    }
    let target = std::env::var("LANDMARK")
        .ok()
        .and_then(|raw| {
            LANDMARKS
                .iter()
                .position(|landmark| landmark.key == raw.trim())
        })
        .unwrap_or(0);
    let elapsed = world.resource::<Time>().elapsed_secs();
    if elapsed < 5.0 {
        return;
    }
    let (stage, since, mut act, done) = {
        let state = world.resource::<LandmarkGate>();
        (state.stage, state.since, state.act, state.done)
    };
    if done {
        return;
    }
    let save = |world: &mut World, stage: u8, since: f32, act: u8| {
        let mut state = world.resource_mut::<LandmarkGate>();
        state.stage = stage;
        state.since = since;
        state.act = act;
    };
    apply_stage_pose(world, stage, target);
    let landmark = &LANDMARKS[target];
    let components: Vec<(u32, Vec3, f32, u8, bool)> = world.resource::<Landmarks>().instances
        [target]
        .components
        .iter()
        .zip(landmark.components.iter())
        .map(|((id, origin, height), def)| (*id, *origin, *height, def.phase, def.signature))
        .collect();
    match stage {
        0 => {
            let graphs = world
                .resource::<destructibles_bevy::structure::Structures>()
                .iter()
                .count();
            assert_eq!(graphs, 0, "Shell Doctrine violation at landmark load");
            let tier_started = std::time::Instant::now();
            for (id, _origin, _height, _phase, _signature) in &components {
                destructibles_bevy::building::begin_detail_upgrade(
                    world,
                    *id,
                    destructibles_bevy::building::DetailTier::Structural,
                );
                for _ in 0..80 {
                    let level = destructibles_bevy::building::advance_detail(world, *id, 50.0);
                    if level >= destructibles_bevy::building::DetailTier::Structural {
                        break;
                    }
                }
                // Re-record the damage-ready baseline at Structural tier-up:
                // the spawn-time pristine predates the panes tier-up adds.
                world
                    .resource_mut::<destructibles_bevy::streaming::PristineChecksums>()
                    .0
                    .remove(id);
                world
                    .resource_mut::<destructibles_bevy::streaming::PristineParts>()
                    .0
                    .remove(id);
                world
                    .resource_mut::<destructibles_bevy::streaming::PristineInventories>()
                    .0
                    .remove(id);
                destructibles_bevy::streaming::record_pristine(world, *id);
            }
            for (id, origin, _height, _phase, _sig) in &components {
                let (record_origin, entity, layout_height) = world
                    .resource::<destructibles_bevy::building::Buildings>()
                    .get(*id)
                    .map(|record| (record.origin, record.entity, record.layout.height))
                    .unwrap_or((Vec3::ZERO, Entity::PLACEHOLDER, 0.0));
                let transform = world
                    .get::<Transform>(entity)
                    .map(|transform| transform.translation);
                let visibility = world.get::<Visibility>(entity).copied();
                let aabb = world
                    .get::<bevy::camera::primitives::Aabb>(entity)
                    .map(|aabb| (aabb.center, aabb.half_extents));
                info!(
                    "LANDMARK component {id}: spawned origin {origin:?} record {record_origin:?} layout_h {layout_height} transform {transform:?} visibility {visibility:?} aabb {aabb:?}"
                );
            }
            info!(
                "LANDMARK {}: tier-up {:.1} ms for {} components, pristine recorded",
                landmark.name,
                tier_started.elapsed().as_secs_f32() * 1000.0,
                components.len()
            );
            save(world, 1, elapsed, 0);
        }
        1 => {
            if elapsed - since < 1.0 {
                return;
            }
            set_camera(
                world,
                [-1006.1, 320.0, -1609.9],
                [-1006.1, ground_at(world, -1006.1, -1609.9) + 90.0, -1609.9],
            );
            landmark_capture(world, target, "debug_topdown");
            save(world, 8, elapsed, 0);
        }
        8 => {
            if elapsed - since < 1.0 {
                return;
            }
            set_camera(
                world,
                [-1489.8, 26.0, -2620.3],
                [-1006.1, ground_at(world, -1006.1, -1609.9) + 95.0, -1609.9],
            );
            landmark_capture(world, target, "pristine_kerry");
            save(world, 2, elapsed, 0);
        }
        2 => {
            if elapsed - since < 1.0 {
                return;
            }
            set_camera(
                world,
                [-3600.0, 55.0, -1500.0],
                [-1006.1, ground_at(world, -1006.1, -1609.9) + 90.0, -1609.9],
            );
            landmark_capture(world, target, "pristine_elliott");
            save(world, 3, elapsed, 0);
        }
        3 => {
            if elapsed - since < 1.0 {
                return;
            }
            set_camera(
                world,
                [-850.0, 95.0, -1350.0],
                [-1006.1, ground_at(world, -1006.1, -1609.9) + 80.0, -1609.9],
            );
            info!("LANDMARK gate: scripted sequence start");
            save(world, 4, elapsed, 0);
        }
        4 => {
            if elapsed - since < 1.2 {
                return;
            }
            // Capture the previous acts' result before the next impacts land.
            if act == 2 {
                landmark_capture(world, target, "25");
            }
            if act == 4 {
                landmark_capture(world, target, "50");
            }
            if act == 8 {
                landmark_capture(world, target, "90");
            }
            for (id, origin, height, phase, _signature) in &components {
                if *phase > act {
                    continue;
                }
                for shot in 0..3u32 {
                    destructibles_bevy::impact::apply_impact(
                        world,
                        *id,
                        destructibles_core::ImpactSpec {
                            position: (*origin
                                + Vec3::new(0.0, height * 0.5, 4.0)
                                + Vec3::X * shot as f32 * 3.0)
                                .to_array(),
                            radius: 0.0,
                            velocity: [0.0, -20.0, -220.0],
                            mass_kg: 60.0,
                            explosive_yield_kg_tnt: 0.0,
                        },
                    );
                }
            }
            info!(
                "LANDMARK {} act {}/{} impacts",
                landmark.name,
                act + 1,
                ACTS
            );
            if act == 2 {
                landmark_capture(world, target, "25");
            }
            if act == 4 {
                landmark_capture(world, target, "50");
            }
            if act == 8 {
                landmark_capture(world, target, "90");
            }
            act += 1;
            if act >= ACTS {
                save(world, 7, elapsed, act);
            } else {
                save(world, 4, elapsed, act);
            }
        }
        7 => {
            // Let the final collapse settle before asking the epilogue to undo.
            if elapsed - since < 25.0 + components.len() as f32 {
                return;
            }
            save(world, 5, elapsed, act);
        }
        5 => {
            let ids = components
                .iter()
                .map(|(id, _origin, _height, _phase, _signature)| *id)
                .collect::<Vec<_>>();
            for _pass in 0..5 {
                destructibles_bevy::rewind::begin_restoration(world, &ids);
                for _ in 0..4000 {
                    destructibles_bevy::rewind::advance_restoration(world);
                    if world
                        .resource::<destructibles_bevy::damage::DamageJournal>()
                        .0
                        .is_empty()
                    {
                        break;
                    }
                }
                let volume = world
                    .resource::<destructibles_bevy::damage::DamageJournal>()
                    .0
                    .entries()
                    .iter()
                    .filter(|entry| entry.volume > 0.0)
                    .count();
                if volume == 0 {
                    break;
                }
            }
            // Clear non-geometric theater entries (Shudder/Alarm) left by the
            // sequence; the epilogue's checksum oracle is the real gate.
            for (id, _origin, _height, _phase, _signature) in &components {
                destructibles_bevy::streaming::force_pristine_reset(world, *id);
            }
            for (id, _origin, _height, _phase, _signature) in &components {
                let restored = destructibles_bevy::rewind::building_checksum(world, *id);
                let pristine = world
                    .resource::<destructibles_bevy::streaming::PristineChecksums>()
                    .0
                    .get(id)
                    .copied()
                    .unwrap_or(0);
                assert_eq!(
                    restored, pristine,
                    "landmark component {id} restore mismatch"
                );
            }
            let journal = &world
                .resource::<destructibles_bevy::damage::DamageJournal>()
                .0;
            let volume_entries = journal
                .entries()
                .iter()
                .filter(|entry| entry.volume > 0.0)
                .count();
            let journal_len = journal.len();
            assert_eq!(
                volume_entries, 0,
                "landmark journal still carries {volume_entries} volumetric entries"
            );
            let structures = world
                .resource::<destructibles_bevy::structure::Structures>()
                .iter()
                .count();
            info!(
                "LANDMARK PASS: {} volumetric journal 0 ({journal_len} theater entries), checksums match, structures {structures}",
                landmark.name
            );
            landmark_capture(world, target, "restored");
            save(world, 6, elapsed, act);
        }
        _ => {
            if elapsed - since > 2.0 {
                world.resource_mut::<LandmarkGate>().done = true;
                world.write_message(AppExit::Success);
            }
        }
    }
}

#[derive(Resource, Default)]
struct LandmarkCycle {
    spawned: bool,
    stage: u8,
    since: f32,
    snapshots: Vec<(u32, Vec<u64>, u64)>,
    frozen: Vec<(u32, destructibles_bevy::streaming::FrozenCache)>,
    done: bool,
}

fn cycle_snapshot(world: &mut World, ids: &[u32]) -> Vec<(u32, Vec<u64>, u64)> {
    ids.iter()
        .map(|id| {
            let seqs = world
                .resource::<destructibles_bevy::damage::DamageJournal>()
                .0
                .entries_for(*id)
                .map(|entry| entry.seq)
                .collect::<Vec<_>>();
            (
                *id,
                seqs,
                destructibles_bevy::rewind::building_checksum(world, *id),
            )
        })
        .collect()
}

fn landmark_cycle(world: &mut World) {
    if std::env::var_os("LANDMARK_CYCLE").is_none() || !s2_enabled() {
        return;
    }
    if !world.resource::<LandmarkCycle>().spawned {
        spawn_landmarks(world);
        world.resource_mut::<LandmarkCycle>().spawned = true;
        return;
    }
    let target = std::env::var("LANDMARK")
        .ok()
        .and_then(|raw| {
            LANDMARKS
                .iter()
                .position(|landmark| landmark.key == raw.trim())
        })
        .unwrap_or(0);
    let elapsed = world.resource::<Time>().elapsed_secs();
    if elapsed < 5.0 {
        return;
    }
    let (stage, since, done) = {
        let state = world.resource::<LandmarkCycle>();
        (state.stage, state.since, state.done)
    };
    if done {
        return;
    }
    let landmark = &LANDMARKS[target];
    let components: Vec<(u32, Vec3, f32, u8, bool)> = world.resource::<Landmarks>().instances
        [target]
        .components
        .iter()
        .zip(landmark.components.iter())
        .map(|((id, origin, height), def)| (*id, *origin, *height, def.phase, def.signature))
        .collect();
    let ids = components
        .iter()
        .map(|(id, _, _, _, _)| *id)
        .collect::<Vec<_>>();
    // Camera every frame: the needling shot must track the pilot-proof pose.
    let ground = ground_at(world, landmark.anchor[0], landmark.anchor[1]);
    let saucer = Vec3::new(
        STREAM_WORLD_OFFSET + landmark.anchor[0],
        ground + 163.0,
        landmark.anchor[1],
    );
    let close_eye = [-850.0, 95.0, -1350.0];
    let range_eye = [landmark.anchor[0] + 40.0, 120.0, landmark.anchor[1] + 710.0];
    let (eye, look) = match stage {
        2 | 3 => (
            range_eye,
            [landmark.anchor[0], ground + 90.0, landmark.anchor[1]],
        ),
        _ => (
            close_eye,
            [landmark.anchor[0], ground + 85.0, landmark.anchor[1]],
        ),
    };
    set_camera(world, eye, look);
    let visible = |world: &mut World| -> bool {
        let Ok((camera, global)) = world
            .query_filtered::<(&Camera, &GlobalTransform), With<FlyCamera>>()
            .single(world)
            .map(|(camera, global)| (camera.clone(), *global))
        else {
            return false;
        };
        let Some(viewport) = camera.logical_viewport_size() else {
            return false;
        };
        match camera.world_to_viewport(&global, saucer) {
            Ok(point) => {
                point.x >= 0.0 && point.y >= 0.0 && point.x <= viewport.x && point.y <= viewport.y
            }
            Err(_) => false,
        }
    };
    if stage == 0 {
        let started = std::time::Instant::now();
        for (id, _, _, _, _) in &components {
            destructibles_bevy::building::begin_detail_upgrade(
                world,
                *id,
                destructibles_bevy::building::DetailTier::Structural,
            );
            for _ in 0..80 {
                let level = destructibles_bevy::building::advance_detail(world, *id, 50.0);
                if level >= destructibles_bevy::building::DetailTier::Structural {
                    break;
                }
            }
            assert!(
                world
                    .resource::<destructibles_bevy::damage::DamageJournal>()
                    .0
                    .entries_for(*id)
                    .next()
                    .is_none(),
                "landmark baseline re-record with journal entries"
            );
            world
                .resource_mut::<destructibles_bevy::streaming::PristineChecksums>()
                .0
                .remove(id);
            world
                .resource_mut::<destructibles_bevy::streaming::PristineParts>()
                .0
                .remove(id);
            world
                .resource_mut::<destructibles_bevy::streaming::PristineInventories>()
                .0
                .remove(id);
            destructibles_bevy::streaming::record_pristine(world, *id);
        }
        let tier_ms = started.elapsed().as_secs_f32() * 1000.0;
        info!(
            "LANDMARK CYCLE {}: tier-up {:.1} ms for {} components",
            landmark.name,
            tier_ms,
            components.len()
        );
        landmark_capture(world, target, "cycle_pristine");
        world.resource_mut::<LandmarkCycle>().since = elapsed;
        world.resource_mut::<LandmarkCycle>().stage = 1;
        return;
    }
    if elapsed - since < 0.8 {
        return;
    }
    match stage {
        1 => {
            for (id, origin, height, phase, _sig) in &components {
                if *phase > 0 {
                    continue;
                }
                for shot in 0..3u32 {
                    destructibles_bevy::impact::apply_impact(
                        world,
                        *id,
                        destructibles_core::ImpactSpec {
                            position: (*origin
                                + Vec3::new(0.0, height * 0.5, 4.0)
                                + Vec3::X * shot as f32 * 3.0)
                                .to_array(),
                            radius: 0.0,
                            velocity: [0.0, -20.0, -220.0],
                            mass_kg: 90.0,
                            explosive_yield_kg_tnt: 1.0,
                        },
                    );
                }
            }
            landmark_capture(world, target, "cycle_damaged");
            let snapshots = cycle_snapshot(world, &ids);
            let frozen = components
                .iter()
                .filter_map(|(id, _, _, _, _)| {
                    destructibles_bevy::streaming::freeze_building(world, *id, true)
                        .map(|cache| (*id, cache))
                })
                .collect::<Vec<_>>();
            let mut state = world.resource_mut::<LandmarkCycle>();
            state.snapshots = snapshots;
            state.frozen = frozen;
            state.stage = 2;
            state.since = elapsed;
        }
        2 => {
            assert!(visible(world), "landmark not in frame at range");
            landmark_capture(world, target, "cycle_range");
            world.resource_mut::<LandmarkCycle>().stage = 3;
            world.resource_mut::<LandmarkCycle>().since = elapsed;
            world.resource_mut::<LandmarkCycle>().snapshots = cycle_snapshot(world, &ids);
        }
        3 => {
            let snapshots = world.resource::<LandmarkCycle>().snapshots.clone();
            for (id, seqs, checksum) in &snapshots {
                let now = cycle_snapshot(world, &[*id]);
                let (_, now_seqs, now_checksum) = &now[0];
                assert_eq!(
                    now_seqs, seqs,
                    "journal changed across freeze/thaw for {id}"
                );
                assert_eq!(
                    now_checksum, checksum,
                    "checksum changed across freeze/thaw for {id}"
                );
            }
            let frozen = std::mem::take(&mut world.resource_mut::<LandmarkCycle>().frozen);
            for (_, cache) in frozen {
                destructibles_bevy::streaming::thaw_building(world, &cache);
            }
            assert!(visible(world), "landmark not in frame after thaw");
            landmark_capture(world, target, "cycle_thaw");
            world.resource_mut::<LandmarkCycle>().stage = 4;
            world.resource_mut::<LandmarkCycle>().since = elapsed;
        }
        _ => {
            for _pass in 0..5 {
                destructibles_bevy::rewind::begin_restoration(world, &ids);
                for _ in 0..4000 {
                    destructibles_bevy::rewind::advance_restoration(world);
                    if world
                        .resource::<destructibles_bevy::damage::DamageJournal>()
                        .0
                        .is_empty()
                    {
                        break;
                    }
                }
                let volume = world
                    .resource::<destructibles_bevy::damage::DamageJournal>()
                    .0
                    .entries()
                    .iter()
                    .filter(|entry| entry.volume > 0.0)
                    .count();
                if volume == 0 {
                    break;
                }
            }
            for (id, _, _, _, _) in &components {
                destructibles_bevy::streaming::force_pristine_reset(world, *id);
            }
            for (id, _, _, _, _) in &components {
                let restored = destructibles_bevy::rewind::building_checksum(world, *id);
                let pristine = world
                    .resource::<destructibles_bevy::streaming::PristineChecksums>()
                    .0
                    .get(id)
                    .copied()
                    .unwrap_or(0);
                assert_eq!(restored, pristine, "landmark cycle restore mismatch {id}");
            }
            let volume = world
                .resource::<destructibles_bevy::damage::DamageJournal>()
                .0
                .entries()
                .iter()
                .filter(|entry| entry.volume > 0.0)
                .count();
            assert_eq!(volume, 0, "landmark cycle volumetric journal not empty");
            assert!(visible(world), "landmark not in frame at restore");
            landmark_capture(world, target, "cycle_restored");
            info!(
                "LANDMARK CYCLE PASS: {} freeze/thaw continuity, journal 0, checksums match, in frame",
                landmark.name
            );
            world.resource_mut::<LandmarkCycle>().done = true;
            world.write_message(AppExit::Success);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_market_signs(world: &mut World, landmark: &LandmarkDef) {
    let asset_server = world.resource::<AssetServer>().clone();
    let sign_image: Handle<Image> = asset_server.load("landmarks/public_market_sign.png");
    let clock_image: Handle<Image> = asset_server.load("landmarks/public_market_clock.png");
    let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
    let sign = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(sign_image.clone()),
        emissive: LinearRgba::rgb(2.6, 0.35, 0.15),
        emissive_texture: Some(sign_image),
        perceptual_roughness: 0.7,
        ..default()
    });
    let clock = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(clock_image.clone()),
        emissive: LinearRgba::rgb(0.9, 0.85, 0.7),
        emissive_texture: Some(clock_image),
        perceptual_roughness: 0.8,
        ..default()
    });
    world.insert_resource(MarketSigns {
        sign: sign.clone(),
        clock: clock.clone(),
    });
    let sign_mesh = world
        .resource_mut::<Assets<Mesh>>()
        .add(Rectangle::new(20.0, 4.0));
    let clock_mesh = world
        .resource_mut::<Assets<Mesh>>()
        .add(Rectangle::new(4.0, 4.0));
    // Sign block south face; clock block south face.
    let sign_x = landmark.anchor[0] + 6.0;
    let sign_z = landmark.anchor[1] - 20.0 + 3.05;
    let clock_x = landmark.anchor[0] - 8.0;
    let clock_z = landmark.anchor[1] - 20.0 + 3.55;
    let ground = world
        .resource::<Geology>()
        .dem
        .height(landmark.anchor[0], landmark.anchor[1]);
    world.spawn((
        Mesh3d(sign_mesh),
        MeshMaterial3d(sign),
        Transform::from_xyz(STREAM_WORLD_OFFSET + sign_x, ground + 22.0, sign_z),
        LandmarkSign {
            block: LANDMARK_ID_BASE + 102,
        },
        Name::new("PUBLIC MARKET sign"),
    ));
    world.spawn((
        Mesh3d(clock_mesh),
        MeshMaterial3d(clock),
        Transform::from_xyz(STREAM_WORLD_OFFSET + clock_x, ground + 21.5, clock_z),
        LandmarkSign {
            block: LANDMARK_ID_BASE + 103,
        },
        Name::new("Market clock face"),
    ));
}

fn landmarks_always(world: &mut World) {
    if std::env::var_os("LANDMARKS_ALWAYS").is_none() || !s2_enabled() {
        return;
    }
    if std::env::var_os("LANDMARK_GATE").is_some() || std::env::var_os("LANDMARK_CYCLE").is_some() {
        return;
    }
    if world.resource::<Landmarks>().instances.is_empty() {
        spawn_landmarks(world);
    }
}

fn sign_theater(
    time: Res<Time>,
    gate: Option<Res<LandmarkGate>>,
    journal: Res<destructibles_bevy::damage::DamageJournal>,
    signs: Query<&MeshMaterial3d<StandardMaterial>, With<LandmarkSign>>,
    signs_clock: Query<&MeshMaterial3d<StandardMaterial>, With<LandmarkSign>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let act = gate.as_ref().map(|gate| gate.act).unwrap_or(0);
    let flicker = if act >= 5 {
        0.0
    } else if act == 4 {
        0.25 + 0.5 * (0.5 + 0.5 * (time.elapsed_secs() * 17.0).sin())
    } else {
        1.0
    };
    for handle in &signs {
        if let Some(mut material) = materials.get_mut(&handle.0) {
            material.emissive = LinearRgba::rgb(2.6 * flicker, 0.35 * flicker, 0.15 * flicker);
        }
    }
    let clock_factor = if act >= 5 {
        0.0
    } else if act == 4 {
        0.35 + 0.4 * (0.5 + 0.5 * (time.elapsed_secs() * 13.0).sin())
    } else {
        1.0
    };
    let _ = &journal;
    for handle in &signs_clock {
        if let Some(mut material) = materials.get_mut(&handle.0) {
            material.emissive =
                LinearRgba::rgb(0.9 * clock_factor, 0.85 * clock_factor, 0.7 * clock_factor);
        }
    }
}

#[derive(Resource, Default)]
struct LandmarkGate {
    spawned: bool,
    stage: u8,
    since: f32,
    act: u8,
    done: bool,
}

fn landmark_capture(world: &mut World, target: usize, suffix: &str) {
    let key = LANDMARKS[target].key;
    let name = if target == 0 {
        format!("landmarks_needle_{suffix}.png")
    } else {
        format!("landmarks_{key}_{suffix}.png")
    };
    capture(world, &name);
}

fn capture(world: &mut World, name: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("captures")
        .join(name);
    world
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path));
    info!("LANDMARK capture {name}");
}

/// Camera pose helper for the gate captures.
pub fn ground_at(world: &World, x: f32, z: f32) -> f32 {
    world.resource::<Geology>().dem.height(x, z)
}

pub fn set_camera(world: &mut World, eye: [f32; 3], look: [f32; 3]) {
    let ground = ground_at(world, eye[0], eye[2]);
    {
        let mut staged = world.resource_mut::<crate::camera_authority::StagedPose>();
        crate::camera_authority::stage_look_at(
            &mut staged,
            "landmark_gate",
            Vec3::new(STREAM_WORLD_OFFSET + eye[0], ground + eye[1], eye[2]),
            Vec3::new(STREAM_WORLD_OFFSET + look[0], look[1], look[2]),
            "landmark",
        );
    }
}

#[cfg(test)]
mod needle_scale {
    use super::*;

    #[test]
    fn the_glb_scale_is_the_two_point_fit() {
        let scale = NEEDLE_TARGET_HEIGHT / NEEDLE_GLB_HEIGHT;
        // Spire tip at 184.4 m (605 ft) within a centimetre.
        assert!((NEEDLE_GLB_HEIGHT * scale - 184.4).abs() < 0.01);
        // Second point: the observation ring sits at 158.5 m (520 ft), which is
        // 86 % of the tip height; the band overlay is authored at that height.
        assert!((NEEDLE_RING_HEIGHT / NEEDLE_TARGET_HEIGHT - 0.8596).abs() < 0.001);
        // Const-evaluated: the crown sits above the ring, the restaurant below.
        const { assert!(NEEDLE_CROWN_HEIGHT > NEEDLE_RING_HEIGHT) };
        const { assert!(NEEDLE_RESTAURANT_HEIGHT < NEEDLE_RING_HEIGHT) };
    }

    #[test]
    fn the_model_stands_the_right_way_up() {
        // SEMANTIC orientation: the spire (z = -805.474) must land ABOVE the
        // base (z = -2.036). R_x(+90) does that; R_x(-90) inverted the model
        // (base-side up) and is the bug this test exists to prevent.
        let rotation = Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2);
        let tip = rotation * Vec3::new(0.0, 0.0, NEEDLE_GLB_HEIGHT);
        let base = rotation * Vec3::new(0.0, 0.0, 2.0);
        assert!(tip.y > 0.0, "spire went below the origin: {tip}");
        assert!(
            base.y > 0.0 && base.y < 10.0,
            "base not near origin: {base}"
        );
        assert!(tip.y > base.y, "spire must be above the legs");
        // Counterexample: the old +90 sign sends the spire to -Y.
        let wrong = Quat::from_rotation_x(std::f32::consts::FRAC_PI_2);
        assert!((wrong * Vec3::new(0.0, 0.0, NEEDLE_GLB_HEIGHT)).y < 0.0);
    }
}
