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

pub const NEEDLE: [ComponentDef; 4] = [
    comp(
        "pedestal",
        [0.0, 0.0],
        0.0,
        [40.0, 40.0],
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
        [9.0, 9.0],
        146.0,
        Shape::Disc,
        MaterialFamily::SteelFrame,
        FacadeStyle::GlassCurtainWall,
        5,
        true,
    ),
    comp(
        "saucer",
        [0.0, 0.0],
        158.0,
        [42.0, 42.0],
        9.0,
        Shape::Disc,
        MaterialFamily::SteelFrame,
        FacadeStyle::GlassCurtainWall,
        5,
        true,
    ),
    comp(
        "crown",
        [0.0, 0.0],
        167.0,
        [8.0, 8.0],
        14.0,
        Shape::Rect,
        MaterialFamily::SteelFrame,
        FacadeStyle::GlassCurtainWall,
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
        [4.0, 4.0],
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
        [-162.1, -642.4],
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
        [-242.6, -738.0],
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
        [-323.1, -833.6],
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
        [-403.6, -929.3],
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
        [-484.1, -1024.9],
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
        [-564.6, -1120.5],
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
        [-645.2, -1216.1],
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
        [-725.7, -1311.7],
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
        [-806.2, -1407.3],
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
        [-886.7, -1503.0],
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
        [-967.2, -1598.6],
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
        [-1047.7, -1694.2],
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
            .add_systems(Update, landmark_gate);
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

fn spawn_landmarks(world: &mut World) {
    let mut instances = Vec::new();
    for (def_index, landmark) in LANDMARKS.iter().enumerate() {
        let mut components = Vec::new();
        for (index, component) in landmark.components.iter().enumerate() {
            let id = LANDMARK_ID_BASE + (def_index as u32) * 100 + index as u32;
            let x = landmark.anchor[0] + component.offset[0];
            let z = landmark.anchor[1] + component.offset[1];
            let ground = world.resource::<Geology>().dem.height(x, z);
            let origin = Vec3::new(x + STREAM_WORLD_OFFSET, ground + component.base_y + 0.03, z);
            spawn_building_world_lod(
                world,
                id,
                component_spec(def_index * 16 + index, component),
                origin,
                BuildingDetail::Shell,
            );
            components.push((id, origin, component.height));
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

fn landmark_gate(world: &mut World) {
    if std::env::var_os("LANDMARK_GATE").is_none() || !s2_enabled() {
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
            info!(
                "LANDMARK {}: tier-up complete, pristine recorded",
                landmark.name
            );
            save(world, 1, elapsed, 0);
        }
        1 => {
            if elapsed - since < 1.0 {
                return;
            }
            set_camera(
                world,
                [-1489.8, 26.0, -2620.3],
                [-1006.1, ground_at(world, -1006.1, -1609.9) + 95.0, -1609.9],
            );
            capture(world, "landmarks_needle_pristine_kerry.png");
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
            capture(world, "landmarks_needle_pristine_elliott.png");
            save(world, 3, elapsed, 0);
        }
        3 => {
            if elapsed - since < 1.0 {
                return;
            }
            set_camera(
                world,
                [-1509.6, 40.0, -1619.9],
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
                capture(world, "landmarks_needle_25.png");
            }
            if act == 4 {
                capture(world, "landmarks_needle_50.png");
            }
            if act == 8 {
                capture(world, "landmarks_needle_90.png");
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
                capture(world, "landmarks_needle_25.png");
            }
            if act == 4 {
                capture(world, "landmarks_needle_50.png");
            }
            if act == 8 {
                capture(world, "landmarks_needle_90.png");
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
            if elapsed - since < 6.0 {
                return;
            }
            save(world, 5, elapsed, act);
        }
        5 => {
            let ids = components
                .iter()
                .map(|(id, _origin, _height, _phase, _signature)| *id)
                .collect::<Vec<_>>();
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
            capture(world, "landmarks_needle_restored.png");
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
struct LandmarkGate {
    spawned: bool,
    stage: u8,
    since: f32,
    act: u8,
    done: bool,
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
    if let Ok(mut camera) = world
        .query_filtered::<&mut Transform, With<FlyCamera>>()
        .single_mut(world)
    {
        camera.translation = Vec3::new(STREAM_WORLD_OFFSET + eye[0], ground + eye[1], eye[2]);
        let look = Vec3::new(STREAM_WORLD_OFFSET + look[0], look[1], look[2]);
        let direction = (look - camera.translation).normalize_or_zero();
        camera.rotation = Quat::from_euler(
            EulerRot::YXZ,
            (-direction.x).atan2(-direction.z),
            direction.y.asin(),
            0.0,
        );
    }
}
