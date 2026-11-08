//! Visible lightning bolts that use the same strike position as the delayed
//! thunder audio in `audio.rs`.

use std::f32::consts::PI;

use bevy::prelude::*;

use crate::{
    camera::FlyCamera, storefront::StorefrontData, weather::Weather, weather::update_weather,
};

#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct LightningGizmoGroup;

#[derive(Resource, Default)]
pub struct LightningState {
    /// Strike positions waiting for the audio system to schedule thunder.
    pub pending_audio: Vec<Vec3>,
    bolts: Vec<Bolt>,
}

struct Bolt {
    start: f32,
    duration: f32,
    points: Vec<Vec3>,
    branches: Vec<Vec<Vec3>>,
    impact: Vec3,
    seed: f32,
}

#[derive(Resource)]
pub struct LightningRng(u32);

impl Default for LightningRng {
    fn default() -> Self {
        Self(0x4C49_4748)
    }
}

impl LightningRng {
    fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x >> 8) as f32 / 16_777_216.0
    }
}

fn configure_lightning_gizmos(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<LightningGizmoGroup>();
    config.line.width = 5.5;
    config.line.perspective = false;
    config.depth_bias = -0.8;
}

fn make_bolt(impact: Vec3, rng: &mut LightningRng) -> Bolt {
    let top = impact.y + 500.0 + rng.next() * 420.0;
    let mut points = Vec::with_capacity(18);
    for i in 0..=16 {
        let t = i as f32 / 16.0;
        let spread = 18.0 * (1.0 - t) + 3.0;
        points.push(Vec3::new(
            impact.x + (rng.next() - 0.5) * spread,
            top - (top - impact.y) * t,
            impact.z + (rng.next() - 0.5) * spread,
        ));
    }

    let mut branches = Vec::new();
    for _ in 0..2 {
        let start_index = 4 + (rng.next() * 8.0) as usize;
        let origin = points[start_index.min(points.len() - 2)];
        let side = if rng.next() < 0.5 { -1.0 } else { 1.0 };
        let mut branch = vec![origin];
        for step in 1..5 {
            let step = step as f32;
            branch.push(
                origin
                    + Vec3::new(
                        side * (8.0 + rng.next() * 24.0) * step,
                        -14.0 * step,
                        (rng.next() - 0.5) * 24.0 * step,
                    ),
            );
        }
        branches.push(branch);
    }

    Bolt {
        start: 0.0,
        duration: 0.18 + rng.next() * 0.12,
        points,
        branches,
        impact,
        seed: rng.next() * 50.0,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn update_lightning(
    time: Res<Time>,
    weather: Res<Weather>,
    buildings: Res<StorefrontData>,
    camera: Single<(&Transform, &FlyCamera), With<FlyCamera>>,
    mut state: ResMut<LightningState>,
    mut rng: ResMut<LightningRng>,
    mut last_flash: Local<bool>,
    mut gizmos: Gizmos<LightningGizmoGroup>,
) {
    let now = time.elapsed_secs();
    let flash = weather.flash > 0.30;
    let (camera_transform, fly_camera) = camera.into_inner();
    if flash && !*last_flash {
        let forward = Vec2::new(-fly_camera.yaw.sin(), -fly_camera.yaw.cos());
        let base_angle = fly_camera.yaw + PI;
        // Aim at a building roof in the forward view, not at the street.
        let angle = base_angle + (rng.next() - 0.5) * 1.4;
        let distance = 300.0 + rng.next() * 750.0;
        let target = Vec2::new(
            camera_transform.translation.x + angle.sin() * distance,
            camera_transform.translation.z + angle.cos() * distance,
        );

        let mut best: Option<(f32, Vec3)> = None;
        let mut fallback: Option<(f32, Vec3)> = None;
        for building in buildings.buildings() {
            let to = Vec2::new(
                building.centroid[0] - camera_transform.translation.x,
                building.centroid[1] - camera_transform.translation.z,
            );
            let building_distance = to.length();
            if !(150.0..=1_100.0).contains(&building_distance) {
                continue;
            }
            if to.normalize_or_zero().dot(forward) < 0.70 {
                continue;
            }
            let roof_height = building.height_m.max(6.0);
            let roof = Vec3::new(building.centroid[0], roof_height, building.centroid[1]);
            let score = to.distance_squared(target);
            if best.is_none_or(|(best_score, _)| score < best_score) {
                best = Some((score, roof));
            }
            if fallback.is_none_or(|(best_score, _)| building_distance < best_score) {
                fallback = Some((building_distance, roof));
            }
        }

        if let Some((_, impact)) = best.or(fallback) {
            let mut bolt = make_bolt(impact, &mut rng);
            bolt.start = now;
            state.pending_audio.push(impact);
            state.bolts.push(bolt);
        }
    }
    *last_flash = flash;

    state.bolts.retain(|bolt| now - bolt.start < bolt.duration);
    for bolt in &state.bolts {
        let age = now - bolt.start;
        let life = (1.0 - age / bolt.duration).clamp(0.0, 1.0);
        let flicker = if (age * 52.0 + bolt.seed).sin() > -0.2 {
            1.0
        } else {
            0.25
        };
        let alpha = (life * flicker).clamp(0.0, 1.0);
        let color = Color::srgba(0.78, 0.88, 1.0, alpha);
        gizmos.linestrip(bolt.points.iter().copied(), color);
        for branch in &bolt.branches {
            gizmos.linestrip(
                branch.iter().copied(),
                Color::srgba(0.66, 0.78, 1.0, alpha * 0.68),
            );
        }
        let ground = bolt.impact + Vec3::Y * 3.0;
        let flash_color = Color::srgba(0.85, 0.92, 1.0, alpha * 0.8);
        gizmos.line(ground - Vec3::X * 9.0, ground + Vec3::X * 9.0, flash_color);
        gizmos.line(ground - Vec3::Z * 9.0, ground + Vec3::Z * 9.0, flash_color);
    }
}

pub struct LightningPlugin;

impl Plugin for LightningPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LightningState>()
            .init_resource::<LightningRng>()
            .init_gizmo_group::<LightningGizmoGroup>()
            .add_systems(Startup, configure_lightning_gizmos)
            .add_systems(Update, update_lightning.after(update_weather));
    }
}
