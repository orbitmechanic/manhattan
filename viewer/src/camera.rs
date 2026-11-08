//! Fly-through camera with a hard floor at the city ground line and swept
//! collision against the building height grid.

use bevy::{
    input::{
        gamepad::{Gamepad, GamepadAxis, GamepadButton},
        mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll},
    },
    prelude::*,
};

use crate::collision::{CAMERA_RADIUS, CollisionGrid};

/// The model's ground plane sits at y = 0 (after Blender's Z-up -> glTF Y-up).
pub const GROUND_Y: f32 = 0.0;
/// Keep the near plane above the pavement even when looking down.
pub const MIN_CAMERA_HEIGHT: f32 = 3.0;
pub const MAX_PITCH: f32 = 1.553_343; // 89 degrees

#[derive(Component, Clone, Debug)]
pub struct FlyCamera {
    pub yaw: f32,
    pub pitch: f32,
    /// Base fly speed in world units per second.
    pub speed: f32,
}

impl Default for FlyCamera {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.0,
            speed: 650.0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MoveState {
    pub forward: f32,
    pub right: f32,
    pub up: f32,
    pub boost: f32,
    /// 0..1 analog scale; keyboard uses 1.0, a gamepad stick uses its radius.
    pub analog: f32,
}

impl Default for MoveState {
    fn default() -> Self {
        Self {
            forward: 0.0,
            right: 0.0,
            up: 0.0,
            boost: 1.0,
            analog: 1.0,
        }
    }
}

/// Unit view direction for a yaw/pitch pair (yaw 0 looks down -Z).
pub fn forward_from_yaw_pitch(yaw: f32, pitch: f32) -> Vec3 {
    Vec3::new(
        -yaw.sin() * pitch.cos(),
        pitch.sin(),
        -yaw.cos() * pitch.cos(),
    )
}

/// Clamp the camera to the ground line. Returns true when a clamp happened.
pub fn clamp_above_ground(transform: &mut Transform) -> bool {
    let floor = GROUND_Y + MIN_CAMERA_HEIGHT;
    if transform.translation.y < floor {
        transform.translation.y = floor;
        true
    } else {
        false
    }
}

fn movement_delta(camera: &FlyCamera, state: MoveState, dt: f32) -> Vec3 {
    let dt = dt.clamp(0.0, 0.1);
    let forward = forward_from_yaw_pitch(camera.yaw, camera.pitch);
    let flat_forward = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
    let right = Vec3::new(-flat_forward.z, 0.0, flat_forward.x);

    let mut direction = flat_forward * state.forward + right * state.right + Vec3::Y * state.up;
    let length = direction.length();
    if length > 1e-4 {
        direction /= length;
    }
    let speed = camera.speed * state.boost * state.analog;
    direction * speed * dt
}

fn apply_view(transform: &mut Transform, camera: &FlyCamera) {
    transform.rotation = Quat::from_rotation_y(camera.yaw) * Quat::from_rotation_x(camera.pitch);
}

pub fn step_camera(transform: &mut Transform, camera: &mut FlyCamera, state: MoveState, dt: f32) {
    transform.translation += movement_delta(camera, state, dt);
    apply_view(transform, camera);
    clamp_above_ground(transform);
}

/// Like [`step_camera`], but sweeps the movement against the building height
/// grid and slides along walls instead of penetrating them.
pub fn step_camera_colliding(
    transform: &mut Transform,
    camera: &mut FlyCamera,
    state: MoveState,
    dt: f32,
    grid: &CollisionGrid,
) {
    let delta = movement_delta(camera, state, dt);
    transform.translation = grid.resolve(transform.translation, delta, CAMERA_RADIUS);
    apply_view(transform, camera);
    clamp_above_ground(transform);
}

fn stick_deadzone(value: f32) -> f32 {
    const DEADZONE: f32 = 0.15;
    if value.abs() <= DEADZONE {
        0.0
    } else {
        value.signum() * ((value.abs() - DEADZONE) / (1.0 - DEADZONE))
    }
}

#[allow(clippy::too_many_arguments)]
pub fn fly_camera_system(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    mouse_motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    gamepads: Query<&Gamepad>,
    grid: Option<Res<CollisionGrid>>,
    mut query: Query<(&mut Transform, &mut FlyCamera)>,
) {
    let dt = time.delta_secs();
    for (mut transform, mut camera) in &mut query {
        // Right mouse button (or middle button) drags the view.
        if buttons.pressed(MouseButton::Right) || buttons.pressed(MouseButton::Middle) {
            let delta = mouse_motion.delta;
            camera.yaw -= delta.x * 0.0026;
            camera.pitch = (camera.pitch - delta.y * 0.0026).clamp(-MAX_PITCH, MAX_PITCH);
        }

        // Keyboard look, handy without a mouse.
        let look_speed = 1.6 * dt;
        if keys.pressed(KeyCode::ArrowLeft) {
            camera.yaw += look_speed;
        }
        if keys.pressed(KeyCode::ArrowRight) {
            camera.yaw -= look_speed;
        }
        if keys.pressed(KeyCode::ArrowUp) {
            camera.pitch = (camera.pitch + look_speed).clamp(-MAX_PITCH, MAX_PITCH);
        }
        if keys.pressed(KeyCode::ArrowDown) {
            camera.pitch = (camera.pitch - look_speed).clamp(-MAX_PITCH, MAX_PITCH);
        }

        // Wheel changes the base speed exponentially.
        if scroll.delta.y.abs() > 0.01 {
            camera.speed = (camera.speed * (1.0 + scroll.delta.y * 0.12)).clamp(20.0, 8000.0);
        }

        let mut state = MoveState::default();
        if keys.pressed(KeyCode::KeyW) {
            state.forward += 1.0;
        }
        if keys.pressed(KeyCode::KeyS) {
            state.forward -= 1.0;
        }
        if keys.pressed(KeyCode::KeyD) {
            state.right += 1.0;
        }
        if keys.pressed(KeyCode::KeyA) {
            state.right -= 1.0;
        }
        if keys.pressed(KeyCode::KeyE) {
            state.up += 1.0;
        }
        if keys.pressed(KeyCode::KeyQ) || keys.pressed(KeyCode::ControlLeft) {
            state.up -= 1.0;
        }
        state.boost = if keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight) {
            5.0
        } else if keys.pressed(KeyCode::AltLeft) {
            0.2
        } else {
            1.0
        };

        if let Some(pad) = gamepads.iter().next() {
            let left_x = stick_deadzone(pad.get(GamepadAxis::LeftStickX).unwrap_or(0.0));
            let left_y = stick_deadzone(pad.get(GamepadAxis::LeftStickY).unwrap_or(0.0));
            let right_x = stick_deadzone(pad.get(GamepadAxis::RightStickX).unwrap_or(0.0));
            let right_y = stick_deadzone(pad.get(GamepadAxis::RightStickY).unwrap_or(0.0));
            let stick_magnitude = (left_x * left_x + left_y * left_y).sqrt().min(1.0);

            // Left stick moves; the Y axis is negative when pushed up.
            state.forward -= left_y;
            state.right += left_x;
            let keyboard_move = keys.pressed(KeyCode::KeyW)
                || keys.pressed(KeyCode::KeyS)
                || keys.pressed(KeyCode::KeyA)
                || keys.pressed(KeyCode::KeyD)
                || keys.pressed(KeyCode::KeyQ)
                || keys.pressed(KeyCode::KeyE);
            if stick_magnitude > 0.01 && !keyboard_move {
                state.analog = stick_magnitude;
            }
            let left_trigger = pad
                .get(GamepadButton::LeftTrigger2)
                .unwrap_or(0.0)
                .clamp(0.0, 1.0);
            let right_trigger = pad
                .get(GamepadButton::RightTrigger2)
                .unwrap_or(0.0)
                .clamp(0.0, 1.0);
            state.up -= left_trigger;
            if pad.pressed(GamepadButton::South) {
                state.up += 1.0;
            }
            if right_trigger > 0.05 {
                state.boost = state.boost.max(1.0 + 4.0 * right_trigger);
            }

            let look = 2.0 * dt;
            camera.yaw -= right_x * look;
            camera.pitch = (camera.pitch - right_y * look).clamp(-MAX_PITCH, MAX_PITCH);

            let speed_step = 1.0 + 0.9 * dt;
            if pad.pressed(GamepadButton::DPadUp) {
                camera.speed = (camera.speed * speed_step).clamp(20.0, 8_000.0);
            }
            if pad.pressed(GamepadButton::DPadDown) {
                camera.speed = (camera.speed / speed_step).clamp(20.0, 8_000.0);
            }
        }

        if let Some(grid) = &grid {
            step_camera_colliding(&mut transform, &mut camera, state, dt, grid);
        } else {
            step_camera(&mut transform, &mut camera, state, dt);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analog_input_scales_speed_linearly() {
        let camera = FlyCamera::default();
        let full = movement_delta(
            &camera,
            MoveState {
                forward: 1.0,
                ..default()
            },
            0.1,
        );
        let half = movement_delta(
            &camera,
            MoveState {
                forward: 1.0,
                analog: 0.5,
                ..default()
            },
            0.1,
        );
        assert!((half.length() - full.length() * 0.5).abs() < 1e-3);
    }

    #[test]
    fn gamepad_deadzone_is_centred() {
        assert_eq!(stick_deadzone(0.0), 0.0);
        assert_eq!(stick_deadzone(0.1), 0.0);
        assert!(stick_deadzone(0.8) > 0.7);
        assert!(stick_deadzone(-0.8) < -0.7);
    }

    #[test]
    fn forward_matches_yaw_convention() {
        let f = forward_from_yaw_pitch(0.0, 0.0);
        assert!((f - Vec3::NEG_Z).length() < 1e-5);
        let f = forward_from_yaw_pitch(std::f32::consts::FRAC_PI_2, 0.0);
        assert!((f - Vec3::NEG_X).length() < 1e-5);
        let f = forward_from_yaw_pitch(0.0, std::f32::consts::FRAC_PI_4);
        assert!(f.y > 0.7);
    }

    #[test]
    fn camera_is_clamped_above_ground() {
        let mut t = Transform::from_xyz(0.0, -25.0, 0.0);
        assert!(clamp_above_ground(&mut t));
        assert!((t.translation.y - (GROUND_Y + MIN_CAMERA_HEIGHT)).abs() < 1e-5);
        t.translation.y = 100.0;
        assert!(!clamp_above_ground(&mut t));
        assert_eq!(t.translation.y, 100.0);
    }

    #[test]
    fn moving_forward_changes_position_and_never_dips_below_ground() {
        let mut t = Transform::default();
        let mut cam = FlyCamera {
            yaw: 0.0,
            pitch: -1.5, // looking almost straight down
            speed: 650.0,
        };
        let start = t.translation;
        step_camera(
            &mut t,
            &mut cam,
            MoveState {
                forward: 1.0,
                ..default()
            },
            0.1,
        );
        assert!(t.translation.distance(start) > 50.0);
        assert!(t.translation.y >= GROUND_Y + MIN_CAMERA_HEIGHT - 1e-4);
    }

    #[test]
    fn vertical_descent_is_clamped() {
        let mut t = Transform::from_xyz(0.0, 50.0, 0.0);
        let mut cam = FlyCamera::default();
        for _ in 0..100 {
            step_camera(
                &mut t,
                &mut cam,
                MoveState {
                    up: -1.0,
                    ..default()
                },
                0.1,
            );
        }
        assert!((t.translation.y - (GROUND_Y + MIN_CAMERA_HEIGHT)).abs() < 1e-4);
    }
}

#[cfg(test)]
mod app_tests {
    use super::*;

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(ButtonInput::<KeyCode>::default());
        app.insert_resource(ButtonInput::<MouseButton>::default());
        app.insert_resource(AccumulatedMouseMotion { delta: Vec2::ZERO });
        app.insert_resource(AccumulatedMouseScroll::default());
        app.add_systems(Update, fly_camera_system);
        app
    }

    #[test]
    fn wasd_moves_the_camera_in_the_app() {
        let mut app = test_app();
        let entity = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, 100.0, 0.0),
                FlyCamera {
                    yaw: 0.0,
                    pitch: 0.0,
                    speed: 650.0,
                },
            ))
            .id();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyW);
        for _ in 0..20 {
            app.update();
        }
        let t = app.world().get::<Transform>(entity).unwrap();
        assert!(
            t.translation.z < 99.0,
            "expected forward (-Z) movement, got {:?}",
            t.translation
        );
    }

    #[test]
    fn actual_system_clamps_camera_below_ground() {
        let mut app = test_app();
        let entity = app
            .world_mut()
            .spawn((Transform::from_xyz(0.0, -100.0, 0.0), FlyCamera::default()))
            .id();
        app.update();
        let t = app.world().get::<Transform>(entity).unwrap();
        assert!(
            t.translation.y >= GROUND_Y + MIN_CAMERA_HEIGHT - 1e-4,
            "camera y {}",
            t.translation.y
        );
    }

    #[test]
    fn right_mouse_drag_rotates_the_camera() {
        let mut app = test_app();
        let entity = app
            .world_mut()
            .spawn((Transform::default(), FlyCamera::default()))
            .id();
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Right);
        app.world_mut()
            .resource_mut::<AccumulatedMouseMotion>()
            .delta = Vec2::new(120.0, 0.0);
        app.update();
        let cam = app.world().get::<FlyCamera>(entity).unwrap();
        assert!(cam.yaw.abs() > 0.01, "yaw did not change: {}", cam.yaw);
    }
}
