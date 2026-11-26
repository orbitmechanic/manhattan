//! Seattle field metadata gates: epicentre, daily pace and commute asymmetry.

use manhattan_viewer::city_rhythm::{CityFields, SEATTLE_EPICENTER, TIMES_SQUARE};

fn seattle_json() -> CityFields {
    let text = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/datasets/seattle_fields.json"),
    )
    .expect("seattle fields");
    CityFields::from_json(&text).expect("parse")
}

#[test]
fn metadata_matches_the_committed_epicenter() {
    let fields = seattle_json();
    assert!((fields.epicenter - SEATTLE_EPICENTER).length() < 0.01);
    assert!((fields.am - 8.5).abs() < 1e-3);
    assert!((fields.pm - 17.0).abs() < 1e-3);
    assert!((fields.toward - 0.90).abs() < 1e-3);
    assert!((fields.away - 0.40).abs() < 1e-3);
}

#[test]
fn epicenter_sits_at_the_westlake_pike_pine_seam() {
    // Pike Place anchor (-518.6, -399.4) is the symbolic heart; the field point
    // is the transit/retail seam within a short walk of it.
    let pike_place = bevy::prelude::Vec2::new(-518.6, -399.4);
    let distance = (SEATTLE_EPICENTER - pike_place).length();
    assert!(distance < 600.0, "epicentre {distance} m from Pike Place");
    assert_ne!(SEATTLE_EPICENTER, TIMES_SQUARE);
}

#[test]
fn daily_pace_peaks_at_eight_thirty_and_five_pm() {
    let fields = seattle_json();
    let am = fields.pace(8.5);
    let pm = fields.pace(17.0);
    let graveyard = fields.pace(3.0);
    assert!(am > 0.90, "AM {am}");
    assert!(pm > 0.90, "PM {pm}");
    assert!((graveyard - 0.05).abs() < 0.02, "graveyard {graveyard}");
    assert!(am > fields.pace(12.0));
}

#[test]
fn lane_occupancy_flips_am_to_pm() {
    let fields = seattle_json();
    let am_toward = fields.occupancy(8.5, true);
    let am_away = fields.occupancy(8.5, false);
    let pm_toward = fields.occupancy(17.0, true);
    let pm_away = fields.occupancy(17.0, false);
    assert!(am_toward > am_away + 0.25, "{am_toward} vs {am_away}");
    assert!(pm_away > pm_toward + 0.25, "{pm_away} vs {pm_toward}");
    assert!(am_toward > 0.80 && am_away < 0.50);
    for toward in [true, false] {
        let night = fields.occupancy(3.0, toward);
        assert!((night - 0.05).abs() < 0.02, "night {toward} {night}");
    }
}

#[test]
fn radial_fields_fall_off_from_the_epicenter() {
    let fields = seattle_json();
    assert!((fields.ad_density(SEATTLE_EPICENTER.x, SEATTLE_EPICENTER.y) - 1.0).abs() < 1e-6);
    let near = fields.ad_density(SEATTLE_EPICENTER.x + 200.0, SEATTLE_EPICENTER.y);
    let far = fields.ad_density(SEATTLE_EPICENTER.x + 1_800.0, SEATTLE_EPICENTER.y);
    assert!(near > far);
    assert_eq!(fields.ad_density(SEATTLE_EPICENTER.x + 3_000.0, 0.0), 0.0);
    let peak = fields.pedestrian_density(SEATTLE_EPICENTER.x, SEATTLE_EPICENTER.y, 17.0);
    let night = fields.pedestrian_density(SEATTLE_EPICENTER.x, SEATTLE_EPICENTER.y, 3.0);
    assert!(peak > night * 10.0);
}

#[test]
fn manhattan_fields_keep_times_square() {
    let fields = CityFields::manhattan();
    assert_eq!(fields.epicenter, TIMES_SQUARE);
    assert!((fields.pace(8.5) - 0.95).abs() < 0.05);
}
