//! High-speed camera LOD: hide streamed accessories and draw speed lines when
//! the camera is moving too fast for the normal near-field simulation.

use bevy::{
    camera::visibility::NoFrustumCulling,
    mesh::MeshVertexBufferLayoutRef,
    pbr::{MaterialPipeline, MaterialPipelineKey, MaterialPlugin},
    prelude::*,
    reflect::TypePath,
    render::render_resource::{
        AsBindGroup, CompareFunction, RenderPipelineDescriptor, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};

use crate::{
    camera::{FlyCamera, fly_camera_system},
    people::Pedestrian,
    street_furniture::StreetFurnitureRoot,
    traffic::{TrafficProxy, Vehicle},
};

/// Above this actual camera speed, streamed accessories are suspended.
pub const ACCESSORY_SUSPEND_SPEED: f32 = 520.0;
/// The camera must fall below this speed before accessories resume.
pub const ACCESSORY_RESUME_SPEED: f32 = 300.0;
/// Sea-level speed of sound in m/s.
pub const MACH_1_MPS: f32 = 343.0;

/// Sonic camera gain: 0 at a walk, 1 at the 650 m/s flight standard. Shared by
/// the wind-rush mixer, the pitch cue, the FOV pulse and the shake so every
/// effect reads the same curve.
pub fn sonic_gain(speed: f32) -> f32 {
    ((speed - 220.0) / (650.0 - 220.0)).clamp(0.0, 1.0)
}

/// Speed-line level 0..1. Symmetric with the accessory cut: full at the cruise
/// standard, fading out across a narrow band above the resume threshold, zero
/// whenever accessories are back. Never stores state, so it cannot stick.
pub fn speed_line_level(speed: f32, suspended: bool) -> f32 {
    if !suspended {
        return 0.0;
    }
    ((speed - ACCESSORY_RESUME_SPEED) / 140.0).clamp(0.0, 1.0)
}

#[derive(Resource)]
pub struct MotionState {
    pub velocity: Vec3,
    pub speed: f32,
    /// Magnitude of the frame-to-frame velocity change, in m/s².
    pub acceleration: f32,
    /// Acceleration in standard gravities.
    pub accel_g: f32,
    /// Current speed divided by sea-level Mach 1.
    pub mach: f32,
    /// Held subsonic time, sustained supersonic time and boom cooldown: booms
    /// are events with hysteresis, never a per-frame state check.
    pub subsonic_secs: f32,
    pub sustained_supersonic: f32,
    pub boom_cooldown: f32,
    /// Seconds since the last upward Mach 1 crossing (10 when idle).
    pub boom_age: f32,
    /// Increments on each upward Mach 1 crossing for one-shot audio.
    pub boom_serial: u32,
    /// 0..1 visual strength for the speed-line overlay.
    pub intensity: f32,
    pub suspended: bool,
}

impl Default for MotionState {
    fn default() -> Self {
        Self {
            velocity: Vec3::ZERO,
            speed: 0.0,
            acceleration: 0.0,
            accel_g: 0.0,
            mach: 0.0,
            subsonic_secs: 0.0,
            sustained_supersonic: 0.0,
            boom_cooldown: 0.0,
            boom_age: 10.0,
            boom_serial: 0,
            intensity: 0.0,
            suspended: false,
        }
    }
}

pub fn accessories_active(motion: Res<MotionState>) -> bool {
    !motion.suspended
}

#[derive(Asset, AsBindGroup, Debug, Clone, TypePath)]
pub struct SpeedLinesMaterial {
    /// x = time, y = intensity, z/w = screen-space travel direction
    #[uniform(0)]
    pub data: Vec4,
    /// rgb = tint, a = opacity
    #[uniform(1)]
    pub tint: Vec4,
    /// x = seconds since the boom, y = boom strength, z = mach, w unused
    #[uniform(2)]
    pub boom: Vec4,
}

impl Material for SpeedLinesMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/speed_lines.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn enable_prepass() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_compare = Some(CompareFunction::Always);
            depth.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

#[derive(Component)]
pub struct SpeedLines {
    material: Handle<SpeedLinesMaterial>,
}

pub fn update_motion_state(
    time: Res<Time>,
    camera: Query<&Transform, With<FlyCamera>>,
    mut motion: ResMut<MotionState>,
    mut previous: Local<Option<Vec3>>,
) {
    let Ok(camera) = camera.single() else {
        if std::env::var_os("SONIC_TRACE").is_some() {
            info!(
                "MOTION: camera query not single (n={})",
                camera.iter().count()
            );
        }
        return;
    };
    let position = camera.translation;
    let Some(last) = *previous else {
        *previous = Some(position);
        return;
    };
    *previous = Some(position);
    let dt = time.delta_secs().max(1e-4);

    let velocity = (position - last) / dt;
    let acceleration = (velocity - motion.velocity) / dt;
    motion.velocity = velocity;
    // SMOOTHED speed: the render effects (FOV pulse, shake, wind) must not read
    // frame-time spikes from chunk streaming as motion. 120 ms EMA.
    let raw = velocity.length();
    let alpha = 1.0 - (-dt / 0.12).exp();
    motion.speed = if motion.speed <= 0.0 {
        raw
    } else {
        motion.speed + (raw - motion.speed) * alpha
    };
    motion.acceleration = acceleration.length();
    motion.accel_g = (motion.acceleration / 9.80665).min(50.0);

    let mach = motion.speed / MACH_1_MPS;
    // Boom discipline: one event per genuine transition. Require 1 s below
    // Mach 0.95, then 0.25 s sustained above Mach 1, and a 2 s cooldown.
    motion.boom_cooldown = (motion.boom_cooldown - dt).max(0.0);
    if mach < 0.95 {
        motion.subsonic_secs += dt;
        motion.sustained_supersonic = 0.0;
    } else if mach >= 1.0 {
        motion.sustained_supersonic += dt;
    }
    if motion.boom_cooldown <= 0.0
        && motion.subsonic_secs >= 1.0
        && motion.sustained_supersonic >= 0.25
    {
        motion.boom_age = 0.0;
        motion.boom_serial = motion.boom_serial.wrapping_add(1);
        motion.boom_cooldown = 2.0;
        motion.subsonic_secs = 0.0;
    } else {
        motion.boom_age = (motion.boom_age + dt).min(10.0);
    }
    motion.mach = mach;

    if motion.suspended {
        if motion.speed < ACCESSORY_RESUME_SPEED {
            motion.suspended = false;
        }
    } else if motion.speed > ACCESSORY_SUSPEND_SPEED {
        motion.suspended = true;
    }

    // Overlay state is a pure function of the CURRENT smoothed speed: no latch,
    // no asymmetry. It fades out fast below the resume threshold (~0.4 s), so a
    // downshift clears the lines while the hand is still on the keys.
    motion.intensity = speed_line_level(motion.speed, motion.suspended);
}

pub fn apply_accessory_visibility(
    motion: Res<MotionState>,
    mut last: Local<Option<bool>>,
    mut accessories: Query<
        &mut Visibility,
        Or<(
            With<Pedestrian>,
            With<Vehicle>,
            With<TrafficProxy>,
            With<StreetFurnitureRoot>,
        )>,
    >,
) {
    if *last == Some(motion.suspended) {
        return;
    }
    *last = Some(motion.suspended);
    let desired = if motion.suspended {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    for mut visibility in &mut accessories {
        if *visibility != desired {
            *visibility = desired;
        }
    }
}

pub fn setup_speed_lines(
    mut commands: Commands,
    camera: Single<Entity, With<FlyCamera>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SpeedLinesMaterial>>,
) {
    let material = materials.add(SpeedLinesMaterial {
        data: Vec4::ZERO,
        tint: Vec4::new(0.74, 0.82, 1.0, 0.0),
        boom: Vec4::new(10.0, 1.0, 0.0, 0.0),
    });
    commands.entity(*camera).with_children(|parent| {
        parent.spawn((
            Name::new("SpeedLines"),
            NoFrustumCulling,
            Mesh3d(meshes.add(Rectangle::new(8.0, 8.0))),
            MeshMaterial3d(material.clone()),
            Transform::from_xyz(0.0, 0.0, -2.0),
            Visibility::Hidden,
            SpeedLines { material },
        ));
    });
}

pub fn update_speed_lines(
    time: Res<Time>,
    motion: Res<MotionState>,
    camera: Single<&Transform, With<FlyCamera>>,
    mut lines: Query<(&SpeedLines, &mut Visibility)>,
    mut materials: ResMut<Assets<SpeedLinesMaterial>>,
) {
    let intensity = motion.intensity;
    let speed = motion.speed.max(1e-3);
    let local = camera.rotation.inverse() * (motion.velocity / speed);
    let direction = Vec2::new(local.x, local.y);
    for (line, mut visibility) in &mut lines {
        if let Some(mut material) = materials.get_mut(&line.material) {
            material.data = Vec4::new(time.elapsed_secs(), intensity, direction.x, direction.y);
            material.tint.w = 0.45;
            material.boom = Vec4::new(motion.boom_age, 1.0, motion.mach, 0.0);
        }
        *visibility = if intensity > 0.01 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

/// `SEATTLE_DRIVE_SPEED=<m/s>`: hold the stick forward at a fixed speed. The
/// harness cannot press keys, so this is the only way to measure what the
/// player's hand does to the speed-line overlay (MotionState reads real frame
/// displacement, so a teleport would read zero).
fn drive_instrument(time: Res<Time>, mut camera: Query<&mut Transform, With<FlyCamera>>) {
    let Some(speed) = std::env::var("SEATTLE_DRIVE_SPEED")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
    else {
        return;
    };
    let Ok(mut transform) = camera.single_mut() else {
        return;
    };
    let forward = transform.rotation * Vec3::NEG_Z;
    let heading = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
    transform.translation += heading * speed * time.delta_secs();
}

pub struct MotionPlugin;

impl Plugin for MotionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MotionState>()
            .add_plugins(MaterialPlugin::<SpeedLinesMaterial>::default())
            .add_systems(PostStartup, setup_speed_lines)
            .add_systems(
                Update,
                (
                    drive_instrument.after(fly_camera_system),
                    update_motion_state.after(drive_instrument),
                    apply_accessory_visibility.after(update_motion_state),
                    update_speed_lines.after(update_motion_state),
                ),
            );
    }
}

/// Boom state machine: armed after 1 s subsonic, fires after 0.25 s sustained
/// supersonic, then re-arms (2 s cooldown). Extracted so the re-arm path is
/// testable: one-then-never and spam are both defects.
pub fn boom_step(
    subsonic: &mut f32,
    sustained: &mut f32,
    cooldown: &mut f32,
    mach: f32,
    dt: f32,
) -> bool {
    *cooldown = (*cooldown - dt).max(0.0);
    if mach < 0.95 {
        *subsonic += dt;
        *sustained = 0.0;
    } else if mach >= 1.0 {
        *sustained += dt;
    }
    if *cooldown <= 0.0 && *subsonic >= 1.0 && *sustained >= 0.25 {
        *cooldown = 2.0;
        *subsonic = 0.0;
        *sustained = 0.0;
        true
    } else {
        false
    }
}

#[cfg(test)]
mod boom_tests {
    use super::boom_step;

    #[test]
    fn three_genuine_crossings_boom_three_times() {
        let mut subsonic = 0.0;
        let mut sustained = 0.0;
        let mut cooldown = 0.0;
        let mut booms = 0;
        let dt = 0.05f32;
        for _cycle in 0..3 {
            // 1.5 s subsonic
            for _ in 0..30 {
                if boom_step(&mut subsonic, &mut sustained, &mut cooldown, 0.5, dt) {
                    booms += 1;
                }
            }
            // 1.0 s supersonic
            for _ in 0..20 {
                if boom_step(&mut subsonic, &mut sustained, &mut cooldown, 1.4, dt) {
                    booms += 1;
                }
            }
        }
        assert_eq!(booms, 3, "re-arm path broken: {booms} booms");
    }
}

#[cfg(test)]
mod overlay_state {
    use super::*;

    #[test]
    fn overlay_lives_only_inside_the_regime() {
        assert_eq!(speed_line_level(650.0, true), 1.0);
        assert_eq!(speed_line_level(440.0, true), 1.0);
        assert!(speed_line_level(380.0, true) < 0.6);
        assert!(speed_line_level(310.0, true) < 0.1);
        // Below the resume threshold the accessory cut is off, so the lines are
        // off: no latch survives a downshift to hover.
        assert_eq!(speed_line_level(299.0, false), 0.0);
        assert_eq!(speed_line_level(0.0, false), 0.0);
        assert_eq!(speed_line_level(100.0, false), 0.0);
    }

    #[test]
    fn overlay_is_monotone_on_deceleration() {
        // Sweep down from cruise; the level must fall every step and hit zero.
        let mut last = 1.0;
        let mut speed = 650.0;
        while speed >= 0.0 {
            let level = speed_line_level(speed, speed > ACCESSORY_RESUME_SPEED);
            assert!(level <= last + 1e-4, "level rose on decel at {speed}");
            last = level;
            speed -= 10.0;
        }
        assert_eq!(last, 0.0);
    }
}
