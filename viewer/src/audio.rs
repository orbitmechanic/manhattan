//! Spatial city, wind, traffic, rain and thunder soundscape.
//!
//! Ambient city/wind/rain are stereo loops around the listener; traffic,
//! horns and thunder are spatial one-shots/loops positioned in the world.
//! Bevy's spatial backend is stereo panning rather than HRTF, so distance
//! attenuation is applied explicitly while the engine supplies the panning.

use std::time::Duration;

use bevy::{
    audio::{
        AudioPlayer, AudioSink, AudioSinkPlayback, AudioSource, PlaybackSettings, SpatialAudioSink,
        SpatialListener, Volume,
    },
    prelude::*,
};
use std::f32::consts::TAU;

use crate::{
    camera::FlyCamera,
    motion::MotionState,
    traffic::Vehicle,
    weather::{Weather, update_weather},
};

#[derive(Resource)]
struct AudioHandles {
    ambient: Handle<AudioSource>,
    wind: Handle<AudioSource>,
    rain: Handle<AudioSource>,
    traffic: Handle<AudioSource>,
    thunder: Handle<AudioSource>,
    horns: Vec<Handle<AudioSource>>,
}

#[derive(Resource)]
struct AudioGate {
    enabled: bool,
}

impl Default for AudioGate {
    fn default() -> Self {
        Self {
            enabled: !cfg!(target_arch = "wasm32"),
        }
    }
}

#[derive(Resource)]
struct AudioRng(u32);

impl Default for AudioRng {
    fn default() -> Self {
        Self(0x51A7_3EED)
    }
}

impl AudioRng {
    fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x >> 8) as f32 / 16_777_216.0
    }
}

#[derive(Component)]
struct AmbientAudio;

#[derive(Component)]
struct WindAudio;

#[derive(Component)]
struct RainAudio;

#[derive(Component)]
struct TrafficAudio(usize);

#[derive(Resource)]
struct HornTimer {
    remaining: f32,
}

impl Default for HornTimer {
    fn default() -> Self {
        Self { remaining: 4.0 }
    }
}

struct PendingThunder {
    delay: f32,
    position: Vec3,
    volume: f32,
}

#[derive(Resource, Default)]
struct ThunderQueue {
    pending: Vec<PendingThunder>,
}

fn setup_audio(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    camera: Single<Entity, With<FlyCamera>>,
    mut rng: ResMut<AudioRng>,
) {
    commands.entity(*camera).insert(SpatialListener::new(1.0));

    let handles = AudioHandles {
        ambient: asset_server.load("sounds/ambient_city.ogg"),
        wind: asset_server.load("sounds/wind.ogg"),
        rain: asset_server.load("sounds/rain.ogg"),
        traffic: asset_server.load("sounds/traffic.ogg"),
        thunder: asset_server.load("sounds/thunder.ogg"),
        horns: vec![
            asset_server.load("sounds/horn1.ogg"),
            asset_server.load("sounds/horn2.ogg"),
            asset_server.load("sounds/horn3.ogg"),
        ],
    };
    let paused = !AudioGate::default().enabled;
    commands.spawn((
        AudioPlayer::new(handles.ambient.clone()),
        PlaybackSettings {
            paused,
            ..PlaybackSettings::LOOP.with_volume(Volume::Linear(0.0))
        },
        AmbientAudio,
    ));
    commands.spawn((
        AudioPlayer::new(handles.wind.clone()),
        PlaybackSettings {
            paused,
            ..PlaybackSettings::LOOP.with_volume(Volume::Linear(0.0))
        },
        WindAudio,
    ));
    commands.spawn((
        AudioPlayer::new(handles.rain.clone()),
        PlaybackSettings {
            paused,
            ..PlaybackSettings::LOOP.with_volume(Volume::Linear(0.0))
        },
        RainAudio,
    ));
    for index in 0..4 {
        commands.spawn((
            AudioPlayer::new(handles.traffic.clone()),
            PlaybackSettings {
                paused,
                start_position: Some(Duration::from_secs_f32(rng.next() * 7.0)),
                ..PlaybackSettings::LOOP
                    .with_spatial(true)
                    .with_volume(Volume::Linear(0.0))
            },
            TrafficAudio(index),
            Transform::from_xyz(0.0, 0.0, 0.0),
        ));
    }
    commands.insert_resource(handles);
}

#[allow(unused_variables, unused_mut)]
fn unlock_audio(
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    mut gate: ResMut<AudioGate>,
    mut ambient: Query<
        &mut AudioSink,
        (With<AmbientAudio>, Without<WindAudio>, Without<RainAudio>),
    >,
    mut wind: Query<&mut AudioSink, (With<WindAudio>, Without<AmbientAudio>, Without<RainAudio>)>,
    mut rain: Query<&mut AudioSink, (With<RainAudio>, Without<AmbientAudio>, Without<WindAudio>)>,
    mut traffic: Query<&mut SpatialAudioSink, With<TrafficAudio>>,
) {
    if !gate.enabled {
        if keys.get_just_pressed().next().is_none() && !buttons.just_pressed(MouseButton::Left) {
            return;
        }
        gate.enabled = true;
    }
    // Native audio starts immediately; only the browser needs a gesture, and
    // the sink is created asynchronously after the asset loads, so keep
    // nudging paused loop sinks until they are running.
    #[cfg(not(target_arch = "wasm32"))]
    return;
    #[cfg(target_arch = "wasm32")]
    {
        for sink in &mut ambient {
            sink.play();
        }
        for sink in &mut wind {
            sink.play();
        }
        for sink in &mut rain {
            sink.play();
        }
        for sink in &mut traffic {
            sink.play();
        }
    }
}

fn approach(current: f32, target: f32, dt: f32, tau: f32) -> f32 {
    current + (target - current) * (1.0 - (-dt / tau).exp())
}

fn update_audio_levels(
    time: Res<Time>,
    motion: Res<MotionState>,
    weather: Res<Weather>,
    mut ambient: Query<
        &mut AudioSink,
        (With<AmbientAudio>, Without<WindAudio>, Without<RainAudio>),
    >,
    mut wind: Query<&mut AudioSink, (With<WindAudio>, Without<AmbientAudio>, Without<RainAudio>)>,
    mut rain: Query<&mut AudioSink, (With<RainAudio>, Without<AmbientAudio>, Without<WindAudio>)>,
    mut levels: Local<(f32, f32, f32)>,
) {
    let dt = time.delta_secs().clamp(0.0, 0.1);
    let target_ambient = 0.16;
    let target_wind = 0.02 + (motion.speed / 900.0).clamp(0.0, 1.0) * 0.30;
    let target_rain =
        weather.precipitation.clamp(0.0, 1.0) * 0.30 * (1.0 - 0.55 * weather.snow.clamp(0.0, 1.0));
    levels.0 = approach(levels.0, target_ambient, dt, 0.35);
    levels.1 = approach(levels.1, target_wind, dt, 0.20);
    levels.2 = approach(levels.2, target_rain, dt, 0.35);
    for mut sink in &mut ambient {
        sink.set_volume(Volume::Linear(levels.0));
    }
    for mut sink in &mut wind {
        sink.set_volume(Volume::Linear(levels.1));
    }
    for mut sink in &mut rain {
        sink.set_volume(Volume::Linear(levels.2));
    }
}

fn update_traffic_audio(
    time: Res<Time>,
    motion: Res<MotionState>,
    camera: Single<&Transform, With<FlyCamera>>,
    vehicles: Query<&GlobalTransform, With<Vehicle>>,
    mut loops: Query<(&TrafficAudio, &mut Transform, &mut SpatialAudioSink), Without<FlyCamera>>,
    mut last: Local<f32>,
) {
    let now = time.elapsed_secs();
    if now - *last < 0.15 {
        return;
    }
    *last = now;
    let camera_position = camera.translation;
    let mut near: Vec<(f32, Vec3)> = vehicles
        .iter()
        .map(|transform| {
            let position = transform.translation();
            (position.distance_squared(camera_position), position)
        })
        .filter(|(distance_sq, _)| *distance_sq < 170.0 * 170.0)
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    near.truncate(4);

    for (slot, mut transform, mut sink) in &mut loops {
        if let Some((distance_sq, position)) = near.get(slot.0) {
            transform.translation = *position;
            let distance = distance_sq.sqrt();
            let volume = if motion.suspended {
                0.0
            } else {
                0.20 * (1.0 - distance / 170.0).clamp(0.0, 1.0)
            };
            sink.set_volume(Volume::Linear(volume));
        } else {
            sink.set_volume(Volume::Linear(0.0));
        }
    }
}

fn update_horns(
    time: Res<Time>,
    gate: Res<AudioGate>,
    handles: Option<Res<AudioHandles>>,
    camera: Single<&Transform, With<FlyCamera>>,
    mut timer: ResMut<HornTimer>,
    mut rng: ResMut<AudioRng>,
    mut commands: Commands,
) {
    if !gate.enabled {
        return;
    }
    let Some(handles) = handles else {
        return;
    };
    timer.remaining -= time.delta_secs();
    if timer.remaining > 0.0 {
        return;
    }
    let distance = 90.0 + rng.next() * 520.0;
    let angle = rng.next() * TAU;
    let position = camera.translation
        + Vec3::new(
            angle.cos() * distance,
            8.0 + rng.next() * 35.0,
            angle.sin() * distance,
        );
    let volume = 0.11 * (1.0 - distance / 700.0).clamp(0.05, 1.0);
    let horn_index = ((rng.next() * 3.0) as usize).min(2);
    let horn = handles.horns[horn_index].clone();
    debug!("distant horn {horn_index} at {distance:.0} m, volume {volume:.2}");
    commands.spawn((
        AudioPlayer::new(horn),
        PlaybackSettings::DESPAWN
            .with_spatial(true)
            .with_volume(Volume::Linear(volume)),
        Transform::from_translation(position),
    ));
    timer.remaining = 4.0 + rng.next() * 16.0;
}

#[allow(clippy::too_many_arguments)]
fn update_thunder(
    time: Res<Time>,
    gate: Res<AudioGate>,
    handles: Option<Res<AudioHandles>>,
    weather: Res<Weather>,
    camera: Single<&Transform, With<FlyCamera>>,
    mut queue: ResMut<ThunderQueue>,
    mut rng: ResMut<AudioRng>,
    mut commands: Commands,
    mut flash_active: Local<bool>,
) {
    let active = weather.flash > 0.30;
    if gate.enabled && active && !*flash_active {
        let distance = 450.0 + rng.next() * 1_650.0;
        let angle = rng.next() * TAU;
        let position = camera.translation
            + Vec3::new(
                angle.cos() * distance,
                90.0 + rng.next() * 260.0,
                angle.sin() * distance,
            );
        let volume = 0.48 * (1.0 - distance / 2_300.0).clamp(0.08, 1.0);
        queue.pending.push(PendingThunder {
            delay: 0.35 + distance / 343.0 + rng.next() * 0.5,
            position,
            volume,
        });
    }
    *flash_active = active;

    let dt = time.delta_secs();
    for pending in &mut queue.pending {
        pending.delay -= dt;
    }
    let Some(handles) = handles else {
        return;
    };
    let mut index = 0;
    while index < queue.pending.len() {
        if queue.pending[index].delay <= 0.0 {
            let bolt = queue.pending.remove(index);
            debug!(
                "thunder at {:.0} m, volume {:.2}",
                bolt.position.length(),
                bolt.volume
            );
            commands.spawn((
                AudioPlayer::new(handles.thunder.clone()),
                PlaybackSettings::DESPAWN
                    .with_spatial(true)
                    .with_volume(Volume::Linear(bolt.volume)),
                Transform::from_translation(bolt.position),
            ));
        } else {
            index += 1;
        }
    }
}

pub struct CityAudioPlugin;

impl Plugin for CityAudioPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AudioGate>()
            .init_resource::<AudioRng>()
            .init_resource::<HornTimer>()
            .init_resource::<ThunderQueue>()
            .add_systems(PostStartup, setup_audio)
            .add_systems(
                Update,
                (
                    unlock_audio,
                    update_audio_levels,
                    update_traffic_audio,
                    update_horns,
                    update_thunder,
                )
                    .after(update_weather),
            );
    }
}
