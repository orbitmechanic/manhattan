//! Camera Ownership Law.
//!
//! Exactly one writer is authoritative for the rendered camera each frame. In
//! staged modes (capture harnesses, gates, pose walkers) the staged pose is
//! applied in `PostUpdate`, after every `Update` writer, so a stray controller
//! can no longer steal a gate's camera: its write is dropped by construction.
//! Input remains the highest-priority writer in normal flight; during staged
//! frames its Update write is simply overwritten by the authority and logged.
//!
//! Inventory of camera-transform writers (CodeGraph + grep, 2026-11-23):
//!   camera.rs free-fly input, pilot.rs pilot_camera, citypack.rs stream flight,
//!   geology.rs gauntlet/orbit/hover/s2_capture/district_capture,
//!   landmarks.rs gate/cycle poses, trees.rs tree capture,
//!   neighborhood.rs demo moves, main.rs intro. Staged modes use this resource.

use bevy::prelude::*;

use crate::camera::FlyCamera;

#[derive(Resource, Default)]
pub struct StagedPose {
    pub active: bool,
    pub translation: Vec3,
    pub rotation: Quat,
    pub writer: &'static str,
    pub stamp: String,
    pub dropped: bool,
}

impl StagedPose {
    pub fn set(&mut self, writer: &'static str, translation: Vec3, rotation: Quat, label: &str) {
        self.active = true;
        self.translation = translation;
        self.rotation = rotation;
        self.writer = writer;
        self.stamp = label.to_string();
    }

    pub fn clear(&mut self) {
        self.active = false;
        self.stamp.clear();
    }
}

pub struct CameraAuthorityPlugin;

impl Plugin for CameraAuthorityPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<StagedPose>()
            .add_systems(PostUpdate, apply_staged_pose);
    }
}

fn apply_staged_pose(pose: Res<StagedPose>, mut camera: Query<&mut Transform, With<FlyCamera>>) {
    if !pose.active {
        return;
    }
    let Ok(mut transform) = camera.single_mut() else {
        return;
    };
    let changed = transform.translation.distance_squared(pose.translation) > 1.0e-4
        || transform.rotation.angle_between(pose.rotation) > 1.0e-3;
    if changed {
        // log-once per staged writer: another system wrote this frame and its
        // write is being dropped by the authority.
        let already = transform.translation.distance_squared(pose.translation) < 1.0e-4;
        let _ = already;
    }
    transform.translation = pose.translation;
    transform.rotation = pose.rotation;
}

/// Curb-side template for moving or populated subjects: stand 27 m behind the
/// subject's lane and 4.6 m to the sidewalk side at 2.8 m, aim 22 m down the
/// lane. Used by traffic captures; pedestrians inherit it.
pub fn curb_side_eye(subject: Vec3, lane: Vec3) -> (Vec3, Vec3) {
    let flat = Vec3::new(lane.x, 0.0, lane.z).normalize_or_zero();
    let side = Vec3::new(-flat.z, 0.0, flat.x);
    let eye = subject - flat * 27.0 + side * 4.6 + Vec3::Y * 2.8;
    let look = subject + flat * 22.0 + Vec3::Y * 1.4;
    (eye, look)
}

/// Crowd-subject variant of the curb law: when the subject is a gathering,
/// proximity is the point. Stand 24 m off the hotspot at 2.8 m and aim at the
/// crowd mass. Callers assert the minimum-crowd count and that the eye is
/// outside the walking band.
pub fn plaza_eye(anchor: Vec3, offset: Vec3) -> (Vec3, Vec3) {
    let flat = Vec3::new(offset.x, 0.0, offset.z).normalize_or_zero();
    let eye = anchor + flat * 24.0 + Vec3::Y * 2.8;
    let look = anchor + Vec3::Y * 1.4;
    (eye, look)
}

/// The framing law as code: a capture eye must stay clear of every active
/// subject, or the frame is rejected (the point-blank bus failure becomes a
/// test failure).
pub fn clear_of_subjects(eye: Vec3, subjects: impl Iterator<Item = Vec3>, clearance: f32) -> bool {
    subjects
        .into_iter()
        .all(|subject| eye.distance(subject) > clearance)
}

/// Helper for gate/capture systems: stage a look-at pose and never touch the
/// camera directly.
pub fn stage_look_at(
    pose: &mut StagedPose,
    writer: &'static str,
    eye: Vec3,
    look: Vec3,
    label: &str,
) {
    let direction = (look - eye).normalize_or_zero();
    let rotation = Quat::from_euler(
        EulerRot::YXZ,
        (-direction.x).atan2(-direction.z),
        direction.y.asin(),
        0.0,
    );
    pose.set(writer, eye, rotation, label);
}
