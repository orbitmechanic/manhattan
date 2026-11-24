//! Camera interior ban + sonic camera.
//!
//! Buildings are solid to the camera: a camera that would end a frame inside a
//! hull is deflected along the hull tangent (glancing, never a hard stop), with
//! the correction scaled by speed so a slow walk eases out and a mach pass
//! glances off. The collision grid covers Manhattan only, so Seattle needs its
//! own test against the live destructibles registry.
//!
//! The sonic camera is the Manhattan motion treatment reinstalled: one shared
//! `sonic_gain` curve drives the wind-rush volume and playback rate (the speed
//! cue), a small FOV pulse and a speed-scaled shake. Staged capture cameras are
//! never shaken — the Camera Ownership Law outranks the effect.

use bevy::prelude::*;

use crate::camera::FlyCamera;
use crate::camera_authority::StagedPose;
use crate::citypack::STREAM_WORLD_OFFSET;
use crate::motion::{MotionState, sonic_gain};

#[derive(Resource, Default)]
pub struct InteriorBanState {
    last_log: f32,
    last_building: u32,
}

pub struct InteriorPlugin;

impl Plugin for InteriorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<InteriorBanState>()
            .init_resource::<InteriorProbeState>()
            .add_systems(
                Update,
                (
                    interior_probe,
                    camera_interior_ban,
                    sonic_camera,
                    sonic_table,
                ),
            );
    }
}

/// INTERIOR_PROBE=1: teleport the camera inside the nearest resident shell so
/// the ban has a deliberate fly-through to log.
#[derive(Resource, Default)]
pub struct InteriorProbeState {
    done: bool,
}

fn interior_probe(world: &mut World) {
    if std::env::var_os("INTERIOR_PROBE").is_none() {
        return;
    }
    if world.resource::<InteriorProbeState>().done || world.resource::<Time>().elapsed_secs() < 6.0
    {
        return;
    }
    world.resource_mut::<InteriorProbeState>().done = true;
    let Some(camera) = world
        .query_filtered::<&Transform, With<FlyCamera>>()
        .single(world)
        .ok()
        .map(|transform| transform.translation)
    else {
        return;
    };
    let target = world
        .resource::<destructibles_bevy::building::Buildings>()
        .all()
        .map(|record| (record.id, record.origin, record.layout.height))
        .min_by(|a, b| a.1.distance(camera).total_cmp(&b.1.distance(camera)));
    let Some((id, origin, height)) = target else {
        return;
    };
    if let Ok(mut transform) = world
        .query_filtered::<&mut Transform, With<FlyCamera>>()
        .single_mut(world)
    {
        transform.translation = origin + Vec3::Y * height * 0.3;
    }
    info!(
        "INTERIOR PROBE: entering b{id} at ({:.0},{:.0}) height {height:.0}",
        origin.x - STREAM_WORLD_OFFSET,
        origin.z
    );
}

fn camera_interior_ban(world: &mut World) {
    if world
        .get_resource::<StagedPose>()
        .is_some_and(|pose| pose.active)
    {
        return;
    }
    let Some(elapsed) = world.get_resource::<Time>().map(|time| time.elapsed_secs()) else {
        return;
    };
    let Some(position) = world
        .query_filtered::<&Transform, With<FlyCamera>>()
        .single(world)
        .ok()
        .map(|transform| transform.translation)
    else {
        return;
    };
    let speed = world
        .get_resource::<MotionState>()
        .map(|motion| motion.speed)
        .unwrap_or(0.0);
    let buildings = world.resource::<destructibles_bevy::building::Buildings>();
    let mut hit: Option<(u32, Vec2, Vec2)> = None;
    for record in buildings.all() {
        // Footprints are centroid-relative; the camera is world-space.
        let local = Vec2::new(position.x - record.origin.x, position.z - record.origin.z);
        let top = record.origin.y + record.layout.height;
        if position.y > top + 0.2 || position.y < record.origin.y - 2.0 {
            continue;
        }
        if !destructibles_core::math::point_in_polygon([local.x, local.y], &record.layout.footprint)
        {
            continue;
        }
        // Nearest hull edge: the deflection normal is the outward edge normal,
        // the tangent runs along the edge.
        let footprint = &record.layout.footprint;
        let mut best = (f32::MAX, Vec2::ZERO, Vec2::ZERO);
        for index in 0..footprint.len() {
            let a = Vec2::from(footprint[index]);
            let b = Vec2::from(footprint[(index + 1) % footprint.len()]);
            let edge = b - a;
            let len = edge.length().max(1.0e-4);
            let t = ((local - a).dot(edge) / (len * len)).clamp(0.0, 1.0);
            let point = a + edge * t;
            let distance = (local - point).length();
            if distance < best.0 {
                let tangent = edge / len;
                let normal = Vec2::new(-tangent.y, tangent.x);
                best = (distance, normal, tangent);
            }
        }
        hit = Some((record.id, best.1, best.2));
        break;
    }
    let Some((building, normal, tangent)) = hit else {
        return;
    };
    // Glancing deflection: push out along the hull normal and carry the
    // tangential component forward, both scaled by speed.
    let glance = (speed / 650.0).clamp(0.15, 1.0);
    let push = 0.7 + 0.8 * glance;
    let slide = 0.5 * glance;
    if let Ok(mut transform) = world
        .query_filtered::<&mut Transform, With<FlyCamera>>()
        .single_mut(world)
    {
        transform.translation.x += normal.x * push + tangent.x * slide;
        transform.translation.z += normal.y * push + tangent.y * slide;
    }
    let mut state = world.resource_mut::<InteriorBanState>();
    if state.last_building != building || elapsed - state.last_log > 1.0 {
        state.last_building = building;
        state.last_log = elapsed;
        info!(
            "CAMERA DEFLECT b{building} speed={speed:.0} resolution=({:.2},{:.2}) glance={glance:.2}",
            normal.x, normal.y
        );
    }
}

fn sonic_camera(
    time: Res<Time>,
    motion: Res<MotionState>,
    staged: Res<StagedPose>,
    mut camera: Query<(&mut Transform, &mut Projection), With<FlyCamera>>,
) {
    let Ok((mut transform, mut projection)) = camera.single_mut() else {
        return;
    };
    let gain = if staged.active {
        0.0
    } else {
        sonic_gain(motion.speed)
    };
    if let Projection::Perspective(perspective) = &mut *projection {
        perspective.fov = (70.0 + 2.0 * gain).to_radians();
    }
    if gain <= 0.01 {
        return;
    }
    // The old 0.15 deg rotational shake at ~6 Hz read as the horizon warping
    // and the mountains moving at cruise. Keep only a whisper of shake.
    let t = time.elapsed_secs();
    let amp = 0.0004 * gain;
    transform.rotate_local_y((t * 37.0).sin() * amp);
    transform.rotate_local_x((t * 53.0).cos() * amp * 0.7);
    transform.translation.y += (t * 61.0).sin() * 0.012 * gain;
}

/// SONIC_TABLE=1: the gain curve at the gauntlet, measured rather than assumed.
fn sonic_table(
    time: Res<Time>,
    motion: Res<MotionState>,
    mut last: Local<f32>,
    mut printed: Local<bool>,
) {
    if std::env::var_os("SONIC_TABLE").is_none() {
        return;
    }
    if !*printed {
        *printed = true;
        // The gain curve at the gauntlet's scripted regimes; the flight camera
        // teleports between samples, so MotionState reads zero velocity.
        for speed in [15.0_f32, 30.0, 100.0, 650.0, 8_000.0] {
            let gain = sonic_gain(speed);
            info!(
                "SONIC CURVE speed={speed:.0} gain={gain:.3} wind_vol={:.3} wind_rate={:.2} fov={:.1} shake_deg={:.3}",
                0.02 + gain * 0.32,
                0.85 + 1.35 * gain,
                70.0 + 5.0 * gain,
                (0.0026 * gain).to_degrees()
            );
        }
    }
    let now = time.elapsed_secs();
    if now - *last < 0.5 {
        return;
    }
    *last = now;
    let gain = sonic_gain(motion.speed);
    info!(
        "SONIC speed={:.0} mach={:.2} gain={gain:.3} wind_vol={:.3} wind_rate={:.2} fov={:.1} shake_deg={:.3}",
        motion.speed,
        motion.mach,
        0.02 + gain * 0.32,
        0.85 + 1.35 * gain,
        70.0 + 2.0 * gain,
        (0.0004 * gain).to_degrees()
    );
}
