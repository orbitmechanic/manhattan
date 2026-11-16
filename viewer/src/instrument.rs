//! Permanent instrument cluster: FPS, frame-time percentiles, compass.
//!
//! Always visible by default (F3 toggles). The reviewer uses this readout to
//! check every performance claim; percentiles are recomputed once per second
//! from the frames of that second.

use bevy::prelude::*;

use crate::camera::FlyCamera;

#[derive(Resource)]
pub struct Instruments {
    pub samples: Vec<f32>,
    pub window: Vec<f32>,
    pub instant_ms: f32,
    pub avg_5s_ms: f32,
    pub worst_sec_ms: f32,
    pub p50_ms: f32,
    pub p95_ms: f32,
    pub p99_ms: f32,
    pub visible: bool,
    last_sec: f32,
}

impl Default for Instruments {
    fn default() -> Self {
        Self {
            samples: Vec::with_capacity(600),
            window: Vec::with_capacity(240),
            instant_ms: 0.0,
            avg_5s_ms: 0.0,
            worst_sec_ms: 0.0,
            p50_ms: 0.0,
            p95_ms: 0.0,
            p99_ms: 0.0,
            visible: true,
            last_sec: 0.0,
        }
    }
}

impl Instruments {
    pub fn percentile(sorted: &[f32], p: f32) -> f32 {
        if sorted.is_empty() {
            return 0.0;
        }
        let index = ((sorted.len() - 1) as f32 * p).round() as usize;
        sorted[index.min(sorted.len() - 1)]
    }
}

#[derive(Component)]
pub struct InstrumentText;

pub struct InstrumentPlugin;

impl Plugin for InstrumentPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Instruments>()
            .add_systems(Startup, setup_instrument_hud)
            .add_systems(
                Update,
                (tick_instruments, toggle_instruments, draw_instruments),
            );
    }
}

fn setup_instrument_hud(mut commands: Commands) {
    commands.spawn((
        InstrumentText,
        Node {
            position_type: PositionType::Absolute,
            left: px(12),
            top: px(238),
            padding: UiRect::all(px(5)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.01, 0.03, 0.05, 0.55)),
        Text::new("FPS —"),
        TextFont {
            font_size: FontSize::Px(11.0),
            ..default()
        },
        TextColor(Color::srgba(0.75, 1.0, 0.82, 0.98)),
        TextShadow::default(),
        ZIndex(22),
    ));
}

fn tick_instruments(time: Res<Time>, mut instruments: ResMut<Instruments>) {
    let delta = time.delta_secs();
    if delta <= 0.0 {
        return;
    }
    instruments.instant_ms = delta * 1000.0;
    let instant = instruments.instant_ms;
    instruments.samples.push(instant);
    instruments.window.push(instant);
    if instruments.samples.len() > 600 {
        instruments.samples.remove(0);
    }
    let now = time.elapsed_secs();
    if now - instruments.last_sec >= 1.0 {
        let mut sorted = instruments.window.clone();
        sorted.sort_by(f32::total_cmp);
        instruments.p50_ms = Instruments::percentile(&sorted, 0.50);
        instruments.p95_ms = Instruments::percentile(&sorted, 0.95);
        instruments.p99_ms = Instruments::percentile(&sorted, 0.99);
        instruments.worst_sec_ms = sorted.last().copied().unwrap_or(0.0);
        instruments.window.clear();
        instruments.last_sec = now;
        let recent = instruments.samples.len().min(300);
        if recent > 0 {
            instruments.avg_5s_ms = instruments.samples[instruments.samples.len() - recent..]
                .iter()
                .sum::<f32>()
                / recent as f32;
        }
    }
}

fn toggle_instruments(keys: Res<ButtonInput<KeyCode>>, mut instruments: ResMut<Instruments>) {
    if keys.just_pressed(KeyCode::F3) {
        instruments.visible = !instruments.visible;
    }
}

fn draw_instruments(
    instruments: Res<Instruments>,
    tuning: Res<crate::city_tuning::CityTuning>,
    camera: Single<&FlyCamera, With<FlyCamera>>,
    mut text: Query<(&mut Text, &mut Visibility), With<InstrumentText>>,
) {
    for (mut body, mut visibility) in &mut text {
        *visibility = if instruments.visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if !instruments.visible {
            continue;
        }
        let fps = if instruments.instant_ms > 0.01 {
            1000.0 / instruments.instant_ms
        } else {
            0.0
        };
        let avg_fps = if instruments.avg_5s_ms > 0.01 {
            1000.0 / instruments.avg_5s_ms
        } else {
            0.0
        };
        body.0 = format!(
            "FPS {fps:.0}  avg {avg_fps:.0}  worst {:.1} ms\np50 {:.1}  p95 {:.1}  p99 {:.1} ms  hdg {:.0}°  [F3]",
            instruments.worst_sec_ms,
            instruments.p50_ms,
            instruments.p95_ms,
            instruments.p99_ms,
            camera.yaw.to_degrees().rem_euclid(360.0)
        );
        body.0.push_str(&format!(
            "\nTUNE yield {:.1}kg mass {:.0} v {:.0} restore {:.0} grip {:.1} fire {:.2} admit {:.1}ms [F6/F7]",
            tuning.explosive_yield_kg,
            tuning.projectile_mass_kg,
            tuning.projectile_velocity_mps,
            tuning.restoration_rate_m3s,
            tuning.grip_max_g,
            tuning.fire_stage_pace,
            tuning.admission_budget_ms
        ));
    }
}
