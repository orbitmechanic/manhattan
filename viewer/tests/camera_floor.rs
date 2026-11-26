//! Camera floor law: land clamps, water dives.

use manhattan_viewer::geology::clamp_camera_floor;

#[test]
fn camera_clamps_to_ground_on_land() {
    assert_eq!(clamp_camera_floor(100.0, false, 40.0), 100.5);
    assert_eq!(clamp_camera_floor(100.0, false, 140.0), 140.0);
}

#[test]
fn camera_may_dive_over_water() {
    assert_eq!(clamp_camera_floor(0.0, true, -30.0), -30.0);
}
