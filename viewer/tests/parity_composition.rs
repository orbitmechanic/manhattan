//! Iteration-21 composition: baked years, style parity, radial fields.

use manhattan_viewer::city_rhythm;
use manhattan_viewer::neighborhood::{style_for, style_for_parts};
use manhattan_viewer::storefront::StorefrontData;

#[test]
fn year_built_coverage_is_complete_and_reportable() {
    let data = StorefrontData::default();
    assert_eq!(data.buildings().len(), 11_032);
    let mut estimated = 0usize;
    for building in data.buildings() {
        assert!(
            (1850..=2024).contains(&building.year_built),
            "b{} bad year {}",
            building.id,
            building.year_built
        );
        estimated += 1;
    }
    // The shipped dataset has no construction date or stable OSM id, so every
    // record uses the documented heuristic; the bake script records this.
    assert_eq!(estimated, 11_032);
}

#[test]
fn style_parity_is_deterministic_and_uses_the_field() {
    let data = StorefrontData::default();
    for building in data.buildings().iter().take(2_000) {
        let glassy = city_rhythm::is_glassy(
            building.id,
            building.block,
            building.centroid[0],
            building.centroid[1],
            building.year_built,
        );
        let expected = style_for_parts(
            &building.category,
            building.height_m,
            building.year_built,
            glassy,
        );
        assert_eq!(style_for(building), expected);
        assert_eq!(style_for(building), style_for(building));
    }
}

#[test]
fn glass_core_is_high_but_never_certain_and_brick_survives() {
    let data = StorefrontData::default();
    let core = city_rhythm::TIMES_SQUARE;
    let mut near_glassy = 0usize;
    let mut near_brick = 0usize;
    for building in data.buildings() {
        let d = city_rhythm::epicenter_distance(building.centroid[0], building.centroid[1]);
        if d > 350.0 {
            continue;
        }
        if city_rhythm::is_glassy(
            building.id,
            building.block,
            building.centroid[0],
            building.centroid[1],
            building.year_built,
        ) {
            near_glassy += 1;
        } else {
            near_brick += 1;
        }
    }
    assert!(near_glassy > 0, "core should have glass");
    assert!(
        near_brick > 0,
        "holdout brick must survive at the epicentre"
    );
    assert!(city_rhythm::glass_probability(core.x, core.y, 2024) < 0.95);
}

#[test]
fn radial_fields_and_daily_curve_match_the_contract() {
    let core = city_rhythm::TIMES_SQUARE;
    assert_eq!(city_rhythm::ad_density(core.x, core.y), 1.0);
    assert_eq!(city_rhythm::ad_density(core.x, core.y + 4_000.0), 0.0);
    assert!(city_rhythm::daily_pace(3.0) <= 0.06);
    assert!(city_rhythm::daily_pace(8.5) > 0.8);
    assert!(city_rhythm::daily_pace(17.0) > 0.8);
    assert!(city_rhythm::lane_occupancy(8.5, true) > city_rhythm::lane_occupancy(8.5, false));
    assert!(city_rhythm::lane_occupancy(17.0, false) > city_rhythm::lane_occupancy(17.0, true));
    assert!(city_rhythm::cloud_deck_altitude(1.0, 410.0) < 410.0 * 0.9);
    let clean = city_rhythm::moon_tint(0);
    assert!(city_rhythm::moon_tint(40).x > clean.x);
}
