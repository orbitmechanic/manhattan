//! Readout and perimeter-simplification gates.

use manhattan_viewer::geology::simplify_perimeter;
use manhattan_viewer::minimap::world_to_lon_lat;

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
