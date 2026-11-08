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

#[derive(Resource, Default)]
pub struct MotionState {
    pub velocity: Vec3,
    pub speed: f32,
    /// Magnitude of the frame-to-frame velocity change, in m/s².
    pub acceleration: f32,
    /// Acceleration in standard gravities.
    pub accel_g: f32,
    /// 0..1 visual strength for the speed-line overlay.
    pub intensity: f32,
    pub suspended: bool,
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
    camera: Single<&Transform, With<FlyCamera>>,
    mut motion: ResMut<MotionState>,
    mut previous: Local<Option<Vec3>>,
) {
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
    motion.speed = velocity.length();
    motion.acceleration = acceleration.length();
    motion.accel_g = (motion.acceleration / 9.80665).min(50.0);

    if motion.suspended {
        if motion.speed < ACCESSORY_RESUME_SPEED {
            motion.suspended = false;
        }
    } else if motion.speed > ACCESSORY_SUSPEND_SPEED {
        motion.suspended = true;
    }

    motion.intensity = if motion.suspended {
        ((motion.speed - ACCESSORY_RESUME_SPEED)
            / (ACCESSORY_SUSPEND_SPEED - ACCESSORY_RESUME_SPEED))
            .clamp(0.0, 1.0)
    } else {
        0.0
    };
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
        }
        *visibility = if intensity > 0.01 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
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
                    update_motion_state.after(fly_camera_system),
                    apply_accessory_visibility.after(update_motion_state),
                    update_speed_lines.after(update_motion_state),
                ),
            );
    }
}
