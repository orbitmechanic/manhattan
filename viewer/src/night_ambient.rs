//! Ground-up night ambient: a small pool of block-coloured point lights.
//!
//! A standard hemisphere ambient is top-down (sky bright, ground dark), which
//! is wrong for a city at midnight: the street is lit from below and the tower
//! tops fade into the dark.  The pool follows the camera, one light per nearby
//! block, with the block's deterministic halogen/neon hue.  Power-loss
//! correctness lives in destructibles' facade uplights (journaled); these block
//! fills are map presentation and never contradict a dark building.

use bevy::prelude::*;

use crate::{camera::FlyCamera, city_rhythm, environment::Environment, storefront::StorefrontData};

const POOL: usize = 24;
const RANGE: f32 = 105.0;
const BASE_INTENSITY: f32 = 38_000.0;
const UPDATE_SECS: f32 = 0.25;

#[derive(Resource, Default)]
pub struct BlockAmbient {
    blocks: Vec<(usize, Vec2)>,
}

#[derive(Component)]
struct BlockFill;

pub struct NightAmbientPlugin;

impl Plugin for NightAmbientPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (setup_blocks, setup_pool))
            .add_systems(Update, update_block_fills);
    }
}

fn setup_blocks(mut commands: Commands, data: Res<StorefrontData>) {
    let mut sums: std::collections::BTreeMap<usize, (Vec2, u32)> =
        std::collections::BTreeMap::new();
    for building in data.buildings() {
        let entry = sums.entry(building.block).or_insert((Vec2::ZERO, 0));
        entry.0 += Vec2::from_array(building.centroid);
        entry.1 += 1;
    }
    let blocks = sums
        .into_iter()
        .map(|(block, (sum, count))| (block, sum / count.max(1) as f32))
        .collect();
    commands.insert_resource(BlockAmbient { blocks });
}

fn setup_pool(mut commands: Commands) {
    for index in 0..POOL {
        commands.spawn((
            BlockFill,
            PointLight {
                color: Color::srgb(1.0, 0.72, 0.34),
                intensity: 0.0,
                range: RANGE,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_xyz(0.0, 6.0, 0.0),
            Name::new(format!("block ambient {index}")),
        ));
    }
}

fn update_block_fills(
    time: Res<Time>,
    env: Res<Environment>,
    camera: Single<&Transform, With<FlyCamera>>,
    blocks: Res<BlockAmbient>,
    mut last: Local<f32>,
    mut fills: Query<(&BlockFill, &mut PointLight, &mut Transform), Without<FlyCamera>>,
) {
    let now = time.elapsed_secs();
    if now - *last < UPDATE_SECS {
        return;
    }
    *last = now;
    let night = (1.0 - env.daylight).clamp(0.0, 1.0).powf(1.4);
    if night < 0.02 {
        for (_, mut light, _) in &mut fills {
            light.intensity = 0.0;
        }
        return;
    }
    let cam = Vec2::new(camera.translation.x, camera.translation.z);
    let mut nearest: Vec<(f32, usize, Vec2)> = blocks
        .blocks
        .iter()
        .map(|(block, point)| (point.distance_squared(cam), *block, *point))
        .collect();
    nearest.sort_by(|a, b| a.0.total_cmp(&b.0));
    nearest.truncate(POOL);
    for (index, (_, mut light, mut transform)) in fills.iter_mut().enumerate() {
        let Some((_, block, point)) = nearest.get(index) else {
            light.intensity = 0.0;
            continue;
        };
        transform.translation = Vec3::new(point.x, 6.0, point.y);
        let colour = city_rhythm::block_ambient_color(*block);
        // Brighter under the camera, still visible a block or two out.
        let distance = point.distance(cam);
        let falloff = (1.0 - distance / (RANGE * 2.2)).clamp(0.18, 1.0);
        light.color = Color::srgb(colour.x, colour.y, colour.z);
        light.intensity = BASE_INTENSITY * night * falloff;
    }
}
