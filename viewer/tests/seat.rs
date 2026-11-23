//! Datum/seat audit: a footprint must sit on the highest grade beneath it, so
//! steep high-elevation sites do not bury their uphill corners.

use manhattan_viewer::citypack::CityPack;
use manhattan_viewer::geology::{Dem, footprint_seat};

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

#[test]
fn seat_covers_the_highest_grade_under_the_footprint() {
    let dem = real_dem();
    let pack = CityPack::open(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/datasets/seattle.citypack"),
    )
    .expect("pack");
    let mut steep = 0;
    let mut checked = 0;
    for index in pack.neighborhoods() {
        let Ok(chunk) = pack.read_chunk(index) else {
            continue;
        };
        for building in &chunk.buildings {
            let centroid = dem.height(building.centroid[0], building.centroid[1]);
            let mut spread = 0.0_f32;
            for point in &building.perimeter {
                spread = spread.max((dem.height(point[0], point[1]) - centroid).abs());
            }
            if spread < 2.0 {
                continue;
            }
            let seat = footprint_seat(&dem, building.centroid, &building.perimeter);
            assert!(
                seat >= centroid - 1.0e-3,
                "seat {seat} below centroid {centroid}"
            );
            for point in &building.perimeter {
                let grade = dem.height(point[0], point[1]);
                assert!(
                    seat >= grade - 0.05,
                    "b{id} seat {seat} below corner {grade}",
                    id = building.id
                );
            }
            steep += 1;
            checked += 1;
            if steep >= 25 {
                break;
            }
        }
        if steep >= 25 {
            break;
        }
    }
    assert!(checked >= 10, "only {checked} steep footprints checked");
}
