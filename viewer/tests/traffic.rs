//! Traffic gates: grade tangency, heartbeat asymmetry, species by street class.

use manhattan_viewer::city_rhythm::CityFields;
use manhattan_viewer::geology::Dem;
use manhattan_viewer::traffic_seattle::{
    SPECIES_SEDAN, SPECIES_TRUCK, VehicleState, vehicle_on_street, vehicle_pose,
};

fn real_dem() -> Dem {
    let report: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("datasets/seattle/s2_report.json"),
        )
        .expect("report"),
    )
    .expect("json");
    let grid = &report["grid"];
    let heights = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/terrain/seattle_dem_30m.bin"),
    )
    .expect("dem")
    .chunks(2)
    .filter(|pair| pair.len() == 2)
    .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
    .collect();
    Dem {
        rows: grid["rows"].as_u64().unwrap() as usize,
        cols: grid["cols"].as_u64().unwrap() as usize,
        spacing: grid["spacing_m"].as_f64().unwrap() as f32,
        origin: [
            grid["origin_world"][0].as_f64().unwrap() as f32,
            grid["origin_world"][1].as_f64().unwrap() as f32,
        ],
        heights,
    }
}

fn steep_street() -> (Dem, VehicleState) {
    use manhattan_viewer::citypack::CityPack;
    let dem = real_dem();
    let pack = CityPack::open(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/datasets/seattle.citypack"),
    )
    .expect("pack");
    let mut best: Option<(f32, VehicleState)> = None;
    for index in pack.neighborhoods() {
        let Ok(chunk) = pack.read_chunk(index) else {
            continue;
        };
        for street in &chunk.streets {
            for pair in street.points.windows(2) {
                let dx = pair[1][0] - pair[0][0];
                let dz = pair[1][1] - pair[0][1];
                let len = (dx * dx + dz * dz).sqrt();
                if len < 20.0 {
                    continue;
                }
                let dy = dem.height(pair[1][0], pair[1][1]) - dem.height(pair[0][0], pair[0][1]);
                let grade = (dy / len).abs();
                if grade > best.as_ref().map(|(g, _)| *g).unwrap_or(0.04) {
                    if let Some(state) = vehicle_on_street(street, 11, 1.0, 9.0, SPECIES_SEDAN) {
                        best = Some((grade, state));
                    }
                }
            }
        }
        if best.as_ref().map(|(g, _)| *g).unwrap_or(0.0) > 0.06 {
            break;
        }
    }
    let (grade, mut state) = best.expect("a graded street");
    assert!(grade > 0.03, "grade {grade} too flat for the hill test");
    state.s = state.length * 0.5;
    (dem, state)
}

#[test]
fn vehicles_are_tangent_to_the_grade() {
    let (dem, state) = steep_street();
    let (position, rotation) = vehicle_pose(&state, &dem);
    let local_x = position.x - manhattan_viewer::citypack::STREAM_WORLD_OFFSET;
    let ground = dem.height(local_x, position.z);
    assert!(
        (position.y - (ground + 0.35)).abs() < 0.05,
        "belly float: y {} ground {}",
        position.y,
        ground
    );
    // Pitch magnitude equals the grade along the heading (the hill law:
    // vehicles pitch with the drape; roll is deliberately not modelled).
    let (centre, dir) = state.sample(state.s);
    let sign = state.direction.signum();
    let hx = dir[0] * sign;
    let hz = dir[1] * sign;
    let slope = (dem.height(centre[0] + hx * 2.0, centre[1] + hz * 2.0)
        - dem.height(centre[0], centre[1]))
        / 2.0;
    let expected = slope.abs().atan();
    let up = rotation * bevy::prelude::Vec3::Y;
    let measured = up.y.clamp(-1.0, 1.0).acos().abs();
    assert!(
        (measured - expected).abs() < 0.03,
        "pitch {measured} vs grade {expected} (up {up})"
    );
}

#[test]
fn lane_offset_keeps_right_of_centreline() {
    let (dem, state) = steep_street();
    let (position, _) = vehicle_pose(&state, &dem);
    let (centre, _) = state.sample(state.s);
    let local_x = position.x - manhattan_viewer::citypack::STREAM_WORLD_OFFSET;
    let offset = ((local_x - centre[0]).powi(2) + (position.z - centre[1]).powi(2)).sqrt();
    assert!((offset - 3.2).abs() < 0.6, "lane offset {offset}");
}

#[test]
fn heartbeat_asymmetry_holds_for_seattle_fields() {
    let fields = CityFields::seattle();
    let am_toward = fields.occupancy(8.5, true);
    let am_away = fields.occupancy(8.5, false);
    let pm_toward = fields.occupancy(17.0, true);
    let pm_away = fields.occupancy(17.0, false);
    assert!(am_toward > am_away + 0.25);
    assert!(pm_away > pm_toward + 0.25);
    for toward in [true, false] {
        let night = fields.occupancy(3.0, toward);
        assert!((night - 0.05).abs() < 0.02);
    }
    let toward = bevy::prelude::Vec2::new(-1.0, -1.0);
    assert!(fields.heading_toward(0.0, 0.0, toward));
    assert!(!fields.heading_toward(0.0, 0.0, -toward));
}

#[test]
fn street_class_picks_a_plausible_species() {
    use manhattan_viewer::traffic_seattle::class_species;
    assert_eq!(class_species(1, 0.05), SPECIES_TRUCK);
    assert_eq!(class_species(1, 0.9), SPECIES_SEDAN);
    assert_eq!(class_species(5, 0.05), 2); // bus on a transit corridor
    assert_eq!(class_species(3, 0.9), SPECIES_SEDAN);
}

#[test]
fn vehicle_states_are_deterministic() {
    let street = manhattan_viewer::citypack::PackStreet {
        id: 42,
        kind_idx: 2,
        name_idx: 0,
        points: vec![[0.0, 0.0], [10.0, 0.0], [20.0, 10.0], [30.0, 20.0]],
    };
    let a = vehicle_on_street(&street, 77, 1.0, 12.0, SPECIES_SEDAN).expect("a");
    let b = vehicle_on_street(&street, 77, 1.0, 12.0, SPECIES_SEDAN).expect("b");
    assert_eq!(a.s, b.s);
    assert_eq!(a.length, b.length);
    let dem = real_dem();
    let (pa, _) = vehicle_pose(&a, &dem);
    let (pb, _) = vehicle_pose(&b, &dem);
    assert_eq!(pa, pb);
}
