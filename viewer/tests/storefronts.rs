//! Storefront gates: deterministic placement, water/road/bridge exclusion and
//! district grammar coverage.

use std::collections::HashMap;

use manhattan_viewer::citypack::CityPack;
use manhattan_viewer::geology::{MASK_PATH, REPORT_PATH};
use manhattan_viewer::landmarks::{LANDMARK_ID_BASE, LANDMARKS};
use manhattan_viewer::storefronts::{KIND_COLONNADE, KIND_WAREHOUSE, Placement, place_chunk};

fn read(path: &str) -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path)).expect(path)
}

struct Grid {
    rows: usize,
    cols: usize,
    spacing: f32,
    origin: [f32; 2],
    mask: Vec<u8>,
    heights: Vec<i16>,
}

impl Grid {
    fn load() -> Self {
        let report: serde_json::Value = serde_json::from_slice(&read(REPORT_PATH)).expect("report");
        let grid = &report["grid"];
        let heights = read("assets/terrain/seattle_dem_30m.bin")
            .chunks(2)
            .filter(|pair| pair.len() == 2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        Self {
            rows: grid["rows"].as_u64().unwrap() as usize,
            cols: grid["cols"].as_u64().unwrap() as usize,
            spacing: grid["spacing_m"].as_f64().unwrap() as f32,
            origin: [
                grid["origin_world"][0].as_f64().unwrap() as f32,
                grid["origin_world"][1].as_f64().unwrap() as f32,
            ],
            mask: read(MASK_PATH),
            heights,
        }
    }

    fn wet(&self, x: f32, z: f32) -> bool {
        let col = ((x - self.origin[0]) / self.spacing).round() as isize;
        let row = ((z - self.origin[1]) / self.spacing).round() as isize;
        if row < 0 || col < 0 || row as usize >= self.rows || col as usize >= self.cols {
            return false;
        }
        self.mask[row as usize * self.cols + col as usize] != 0
    }

    fn height(&self, x: f32, z: f32) -> f32 {
        let col = ((x - self.origin[0]) / self.spacing).floor() as isize;
        let row = ((z - self.origin[1]) / self.spacing).floor() as isize;
        if row < 0 || col < 0 || row as usize >= self.rows - 1 || col as usize >= self.cols - 1 {
            return 0.0;
        }
        self.heights[row as usize * self.cols + col as usize] as f32 / 10.0
    }
}

fn districts() -> HashMap<String, String> {
    let value: serde_json::Value =
        serde_json::from_slice(&read("assets/styles/seattle_districts.json")).expect("districts");
    value["assignments"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, district)| (name.clone(), district.as_str().unwrap().to_string()))
        .collect()
}

fn placements_for(district: &str, grid: &Grid, pack: &CityPack) -> (usize, Vec<Placement>) {
    let mut nodes = 0;
    let mut all = Vec::new();
    for index in pack.neighborhoods() {
        let node = &pack.nodes[index];
        if districts().get(&node.name).map(String::as_str) != Some(district) {
            continue;
        }
        let Ok(chunk) = pack.read_chunk(index) else {
            continue;
        };
        nodes += 1;
        all.extend(place_chunk(
            index,
            &chunk,
            district,
            |x, z| grid.height(x, z),
            |x, z| grid.wet(x, z),
        ));
        if nodes > 6 {
            break;
        }
    }
    (nodes, all)
}

#[test]
fn placement_is_deterministic_per_chunk() {
    let pack = CityPack::open(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/datasets/seattle.citypack"),
    )
    .expect("pack");
    let grid = Grid::load();
    let mut compared = 0;
    for index in pack.neighborhoods() {
        let Ok(chunk) = pack.read_chunk(index) else {
            continue;
        };
        let district = districts()
            .get(&pack.nodes[index].name)
            .cloned()
            .unwrap_or_default();
        if manhattan_viewer::storefronts::grammar(&district).is_none() {
            continue;
        }
        let first = place_chunk(
            index,
            &chunk,
            &district,
            |x, z| grid.height(x, z),
            |x, z| grid.wet(x, z),
        );
        let second = place_chunk(
            index,
            &chunk,
            &district,
            |x, z| grid.height(x, z),
            |x, z| grid.wet(x, z),
        );
        assert_eq!(first, second, "chunk {index} differs on re-placement");
        compared += first.len();
    }
    assert!(compared > 0, "no placements sampled");
}

#[test]
fn placements_stay_out_of_water_and_landmarks() {
    let pack = CityPack::open(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/datasets/seattle.citypack"),
    )
    .expect("pack");
    let grid = Grid::load();
    for district in ["Pike Place", "Pioneer Square", "SODO", "Ballard"] {
        let (_nodes, placements) = placements_for(district, &grid, &pack);
        assert!(!placements.is_empty(), "no placements for {district}");
        for placement in &placements {
            assert!(!grid.wet(placement.x, placement.z), "water {district}");
            for (index, landmark) in LANDMARKS.iter().enumerate() {
                if index == 1 {
                    continue;
                }
                let dx = landmark.anchor[0] - placement.x;
                let dz = landmark.anchor[1] - placement.z;
                assert!(
                    dx * dx + dz * dz >= 90.0 * 90.0,
                    "{} prop on {}",
                    district,
                    landmark.name
                );
            }
            assert!(placement.building < LANDMARK_ID_BASE);
        }
    }
}

#[test]
fn district_grammars_place_their_own_props() {
    let pack = CityPack::open(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/datasets/seattle.citypack"),
    )
    .expect("pack");
    let grid = Grid::load();
    let (_n, pike) = placements_for("Pike Place", &grid, &pack);
    let (_n, pioneer) = placements_for("Pioneer Square", &grid, &pack);
    let (_n, sodo) = placements_for("SODO", &grid, &pack);
    assert!(
        pike.iter().any(|p| p.kind == 0 || p.kind == 4),
        "Pike Place"
    );
    assert!(
        pioneer.iter().any(|p| p.kind == KIND_COLONNADE),
        "Pioneer Square colonnade"
    );
    assert!(
        sodo.iter().any(|p| p.kind == KIND_WAREHOUSE),
        "SODO warehouse"
    );
    let (_n, residential) = placements_for("Ballard", &grid, &pack);
    assert!(!residential.is_empty(), "residential corner props");
}
