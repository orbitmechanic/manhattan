//! Readout and perimeter-simplification gates.

use manhattan_viewer::geology::simplify_perimeter;
use manhattan_viewer::minimap::{world_to_lon_lat, world_to_lon_lat_checked};

#[test]
fn known_sites_round_trip_inside_the_dataset() {
    // Space Needle anchor from the landmark table.
    let (lon, lat) = world_to_lon_lat(-1006.1, -1609.9);
    assert!((lat - 47.6205).abs() < 5.0e-4, "lat {lat}");
    assert!((lon + 122.3493).abs() < 5.0e-4, "lon {lon}");
    // Tacoma center.
    let (lon, lat) = world_to_lon_lat(-8453.0, 40415.0);
    assert!((47.0..=47.8).contains(&lat), "tacoma lat {lat}");
    assert!((-122.7..=-122.2).contains(&lon), "tacoma lon {lon}");
}

#[test]
fn complex_perimeters_are_simplified_not_dropped() {
    let ring = (0..100)
        .map(|index| {
            let angle = std::f32::consts::TAU * index as f32 / 100.0;
            [10.0 * angle.cos(), 10.0 * angle.sin()]
        })
        .collect::<Vec<_>>();
    let simplified = simplify_perimeter(&ring);
    assert!(simplified.len() <= 64, "{} vertices", simplified.len());
    assert!(simplified.len() >= 3);
    let small = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]];
    assert_eq!(simplify_perimeter(&small).len(), 3);
}

#[test]
fn far_east_point_round_trips_inside_the_dataset() {
    // 18 km east of the pack origin — the boot-panic site's neighbourhood.
    let (lon, lat) = world_to_lon_lat(18_000.0, -3_000.0);
    assert!((lon + 122.09679379260706).abs() < 1.0e-3, "lon {lon}");
    assert!((lat - 47.63608773871386).abs() < 1.0e-3, "lat {lat}");
}

#[test]
fn world_offset_is_not_part_of_the_fit_space() {
    // The camera's world X carries the +60 km stream offset; the readout must
    // subtract it. -138 local must equal 59862 - 60000.
    let (lon_local, lat_local) = world_to_lon_lat(-138.0, -614.3);
    let (lon_world, lat_world) = world_to_lon_lat(59_862.0 - 60_000.0, -614.3);
    assert!((lon_local - lon_world).abs() < 1.0e-9);
    assert!((lat_local - lat_world).abs() < 1.0e-9);
    assert!((lon_local + 122.33749467260583).abs() < 1.0e-3);
}

#[test]
fn checked_readout_skips_points_outside_the_dataset() {
    // The painter variant must never panic on a record the fit cannot place.
    assert!(world_to_lon_lat_checked(18_000.0, -3_000.0).is_some());
    assert!(world_to_lon_lat_checked(40_000.0, -3_000.0).is_none());
    assert!(world_to_lon_lat_checked(0.0, -80_000.0).is_none());
}

#[test]
#[should_panic(expected = "outside the Seattle dataset bbox")]
fn player_readout_panics_outside_the_dataset() {
    // Camera readout stays strict: a wrong unit boundary must be loud.
    let _ = world_to_lon_lat(40_000.0, -3_000.0);
}
