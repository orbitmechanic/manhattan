//! Pedestrian gates: sidewalk band, hill law, and the auditable density math.

use manhattan_viewer::city_rhythm::CityFields;
use manhattan_viewer::geology::Dem;
use manhattan_viewer::pedestrians::{WalkerState, expected_walkers, walker_on_street, walker_pose};

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

fn walker() -> (Dem, WalkerState) {
    use manhattan_viewer::citypack::CityPack;
    let dem = real_dem();
    let pack = CityPack::open(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/datasets/seattle.citypack"),
    )
    .expect("pack");
    let mut best: Option<(f32, WalkerState)> = None;
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
                if grade > best.as_ref().map(|(g, _)| *g).unwrap_or(0.04)
                    && let Some(state) = walker_on_street(street, 7, 1.0, 1.25)
                {
                    best = Some((grade, state));
                }
            }
        }
        if best.as_ref().map(|(g, _)| *g).unwrap_or(0.0) > 0.06 {
            break;
        }
    }
    let (grade, mut state) = best.expect("graded street");
    assert!(grade > 0.03, "grade {grade}");
    state.s = state.length * 0.5;
    (dem, state)
}

#[test]
fn walkers_are_tangent_to_the_grade() {
    let (dem, state) = walker();
    let (position, rotation) = walker_pose(&state, &dem);
    let local_x = position.x - manhattan_viewer::citypack::STREAM_WORLD_OFFSET;
    let ground = dem.height(local_x, position.z);
    assert!(
        (position.y - (ground + 0.9)).abs() < 0.05,
        "foot float: y {} ground {}",
        position.y,
        ground
    );
    let (centre, dir) = state.sample(state.s);
    let sign = state.direction.signum();
    let slope = (dem.height(
        centre[0] + dir[0] * sign * 1.5,
        centre[1] + dir[1] * sign * 1.5,
    ) - dem.height(centre[0], centre[1]))
        / 1.5;
    let expected = slope.abs().atan();
    let measured = (rotation * bevy::prelude::Vec3::Y)
        .y
        .clamp(-1.0, 1.0)
        .acos()
        .abs();
    assert!(
        (measured - expected).abs() < 0.03,
        "pitch {measured} vs {expected}"
    );
}

#[test]
fn walkers_keep_to_the_sidewalk_band() {
    let (dem, state) = walker();
    let (position, _) = walker_pose(&state, &dem);
    let (centre, _) = state.sample(state.s);
    let local_x = position.x - manhattan_viewer::citypack::STREAM_WORLD_OFFSET;
    let offset = ((local_x - centre[0]).powi(2) + (position.z - centre[1]).powi(2)).sqrt();
    assert!((offset - 5.4).abs() < 0.6, "band offset {offset}");
}

#[test]
fn density_math_follows_the_heartbeat() {
    let fields = CityFields::seattle();
    let parks = manhattan_viewer::pedestrians::ParkField::default();
    let seam = [-138.4, -614.3];
    let plaza = [-518.6, -399.4];
    let south = [3000.0, 4000.0];
    assert!(fields.pace(8.5) > fields.pace(3.0));
    assert!(
        expected_walkers(&fields, &parks, 8.5, seam) > expected_walkers(&fields, &parks, 3.0, seam)
    );
    assert!(
        expected_walkers(&fields, &parks, 8.5, plaza)
            > expected_walkers(&fields, &parks, 8.5, south) * 0.5
    );
    assert!(expected_walkers(&fields, &parks, 3.0, south) < 1.0);
    assert!(expected_walkers(&fields, &parks, 8.5, seam) <= 30.0);
}

#[test]
fn walkers_are_deterministic() {
    let street = manhattan_viewer::citypack::PackStreet {
        id: 9,
        kind_idx: 3,
        name_idx: 0,
        points: vec![[0.0, 0.0], [10.0, 0.0], [20.0, 10.0]],
    };
    let a = walker_on_street(&street, 5, 1.0, 1.25).expect("a");
    let b = walker_on_street(&street, 5, 1.0, 1.25).expect("b");
    assert_eq!(a.s, b.s);
    assert_eq!(a.length, b.length);
    let dem = real_dem();
    assert_eq!(walker_pose(&a, &dem).0, walker_pose(&b, &dem).0);
}
