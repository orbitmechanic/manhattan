//! 1 km geometric-embellishment culling.
//!
//! Roads, storefronts and any future facade graphics are components with an
//! AABB.  They are hidden as soon as the camera is more than 1,000 m from the
//! box (3D distance to the closest point), so scene detail does not render
//! outside the embellishment radius.

use bevy::prelude::*;

use crate::camera::FlyCamera;

pub const EMBELLISHMENT_RADIUS: f32 = 1_000.0;
/// Street furniture, signage, trees and antennas use the tighter 1,000 ft
/// (304.8 m) detail radius.
pub const NEAR_DETAIL_RADIUS: f32 = 304.8;

#[derive(Component, Clone, Copy, Debug)]
pub struct DetailCull {
    pub min: Vec3,
    pub max: Vec3,
    pub radius: f32,
}

impl DetailCull {
    pub fn new(min: Vec3, max: Vec3, radius: f32) -> Self {
        Self { min, max, radius }
    }

    pub fn near(min: Vec3, max: Vec3) -> Self {
        Self::new(min, max, NEAR_DETAIL_RADIUS)
    }

    pub fn distance_to(&self, p: Vec3) -> f32 {
        let q = p.clamp(self.min, self.max);
        p.distance(q)
    }
}

pub fn update_detail_culling(
    camera: Single<&Transform, With<FlyCamera>>,
    mut items: Query<(&DetailCull, &mut Visibility)>,
) {
    for (bounds, mut visibility) in &mut items {
        *visibility = if bounds.distance_to(camera.translation) > bounds.radius {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_distance_uses_closest_point() {
        let b = DetailCull::new(
            Vec3::new(-10.0, 0.0, -10.0),
            Vec3::new(10.0, 10.0, 10.0),
            EMBELLISHMENT_RADIUS,
        );
        assert_eq!(b.distance_to(Vec3::ZERO), 0.0);
        assert!((b.distance_to(Vec3::new(20.0, 5.0, 0.0)) - 10.0).abs() < 1e-4);
        assert!(b.distance_to(Vec3::new(20.0, 5.0, 0.0)) < EMBELLISHMENT_RADIUS);
        assert!(b.distance_to(Vec3::new(2_000.0, 5.0, 0.0)) > EMBELLISHMENT_RADIUS);
    }
}
