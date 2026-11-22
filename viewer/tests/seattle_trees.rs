//! Seattle tree dataset gates: exclusion, determinism, LOD and budget bounds.
//!
//! These run against the real generated dataset and the CityPack, so a tree
//! that intersects a building, road ribbon or water cell fails the suite.

use std::collections::HashMap;

use manhattan_viewer::citypack::CityPack;
use manhattan_viewer::geology::{MASK_PATH, REPORT_PATH};
use manhattan_viewer::trees::{
    LOD_MID_M, LOD_NEAR_M, MAX_ACTIVE, SPAWN_PER_FRAME, TREES_PATH, TreeDataset, TreeRecord,
    lod_for,
};

fn asset(relative: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn dataset() -> TreeDataset {
    TreeDataset::open(&asset(TREES_PATH)).expect("tree dataset")
}

fn pack() -> CityPack {
    CityPack::open(asset("assets/datasets/seattle.citypack")).expect("citypack")
}

fn point_in_polygon(point: [f32; 2], polygon: &[[f32; 2]]) -> bool {
    let mut inside = false;
    let mut j = polygon.len() - 1;
    for i in 0..polygon.len() {
        let a = polygon[i];
        let b = polygon[j];
        if (a[1] > point[1]) != (b[1] > point[1]) {
            let t = (point[1] - a[1]) / (b[1] - a[1]);
            let x = a[0] + (b[0] - a[0]) * t;
            if point[0] < x {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

fn segment_distance(point: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let length2 = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if length2 <= 1.0e-9 {
        0.0
    } else {
        (((point[0] - a[0]) * ab[0] + (point[1] - a[1]) * ab[1]) / length2).clamp(0.0, 1.0)
    };
    let dx = point[0] - (a[0] + ab[0] * t);
    let dz = point[1] - (a[1] + ab[1] * t);
    (dx * dx + dz * dz).sqrt()
}

struct Grid {
    rows: usize,
    cols: usize,
    spacing: f32,
    origin: [f32; 2],
    mask: Vec<u8>,
}

impl Grid {
    fn load() -> Self {
        let report: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(asset(REPORT_PATH)).expect("report"))
                .expect("json");
        let grid = &report["grid"];
        let mask = std::fs::read(asset(MASK_PATH)).expect("mask");
        Self {
            rows: grid["rows"].as_u64().unwrap() as usize,
            cols: grid["cols"].as_u64().unwrap() as usize,
            spacing: grid["spacing_m"].as_f64().unwrap() as f32,
            origin: [
                grid["origin_world"][0].as_f64().unwrap() as f32,
                grid["origin_world"][1].as_f64().unwrap() as f32,
            ],
            mask,
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
}

/// Sample every neighborhood that has trees and check the dataset excludes
/// buildings, road ribbons and water cells.
#[test]
fn trees_avoid_buildings_roads_and_water() {
    let dataset = dataset();
    let pack = pack();
    let grid = Grid::load();
    let mut sampled = 0usize;
    let mut building_hits = 0usize;
    let mut road_hits = 0usize;
    let mut water_hits = 0usize;
    let mut checked = 0usize;
    for index in pack.neighborhoods() {
        if dataset.count(index) == 0 {
            continue;
        }
        let node = &pack.nodes[index];
        let Ok(chunk) = pack.read_chunk(index) else {
            continue;
        };
        let buildings = chunk.buildings.clone();
        let streets = chunk.streets.clone();
        let records = dataset.read_chunk(index, node.center);
        // Deterministic stride sample: at most 12 trees per neighborhood.
        let stride = (records.len() / 12).max(1);
        for record in records.iter().step_by(stride) {
            sampled += 1;
            let point = [record.x, record.z];
            for building in &buildings {
                if point_in_polygon(point, &building.perimeter) {
                    building_hits += 1;
                    break;
                }
            }
            let mut nearest = f32::MAX;
            for street in &streets {
                for pair in street.points.windows(2) {
                    nearest = nearest.min(segment_distance(point, pair[0], pair[1]));
                }
            }
            if nearest < 0.5 {
                road_hits += 1;
            }
            if grid.wet(record.x, record.z) {
                water_hits += 1;
            }
            checked += 1;
        }
    }
    assert!(checked > 1000, "only {checked} trees checked");
    assert_eq!(building_hits, 0, "{building_hits} trees inside footprints");
    assert_eq!(road_hits, 0, "{road_hits} trees on road ribbons");
    assert_eq!(water_hits, 0, "{water_hits} trees in water cells");
    println!("trees exclusion: {sampled} sampled, {checked} checked");
}

/// Re-streaming a chunk must reproduce byte-identical records.
#[test]
fn trees_regenerate_identically() {
    let dataset = dataset();
    let pack = pack();
    let mut compared = 0usize;
    for index in pack.neighborhoods() {
        if dataset.count(index) == 0 {
            continue;
        }
        let center = pack.nodes[index].center;
        let first: Vec<TreeRecord> = dataset.read_chunk(index, center);
        let second: Vec<TreeRecord> = dataset.read_chunk(index, center);
        assert_eq!(first, second, "chunk {index} differs on re-read");
        assert_eq!(first.len(), dataset.count(index) as usize);
        compared += first.len();
        if compared > 20_000 {
            break;
        }
    }
    assert!(compared > 10_000, "only {compared} records compared");
}

#[test]
fn trees_header_matches_directory() {
    let dataset = dataset();
    let pack = pack();
    let mut total = 0u64;
    for index in pack.neighborhoods() {
        total += dataset.count(index) as u64;
    }
    assert_eq!(total, dataset.total as u64, "directory sum vs header");
    assert!(
        dataset.total > 50_000,
        "dataset too small: {}",
        dataset.total
    );
}

#[test]
fn trees_lod_bands_are_monotone() {
    assert_eq!(lod_for(0.0), 0);
    assert_eq!(lod_for(LOD_NEAR_M - 1.0), 0);
    assert_eq!(lod_for(LOD_NEAR_M + 1.0), 1);
    assert_eq!(lod_for(LOD_MID_M - 1.0), 1);
    assert_eq!(lod_for(LOD_MID_M + 1.0), 2);
    let mut previous = 0u8;
    for step in 0..1000 {
        let lod = lod_for(step as f32);
        assert!(lod >= previous, "LOD regressed at {step} m");
        previous = lod;
    }
}

#[test]
fn trees_stream_budget_is_bounded() {
    let budget = SPAWN_PER_FRAME;
    assert!((64..=512).contains(&budget));
    let active = MAX_ACTIVE;
    assert!(active >= 4_000);
    // A chunk at the cap drains in a bounded number of frames.
    let frames = MAX_ACTIVE.div_ceil(SPAWN_PER_FRAME);
    assert!(frames <= 64, "drain takes {frames} frames");
}

#[test]
fn trees_species_are_in_range() {
    let dataset = dataset();
    let pack = pack();
    let mut species: HashMap<u8, usize> = HashMap::new();
    for index in pack.neighborhoods() {
        if dataset.count(index) == 0 {
            continue;
        }
        let center = pack.nodes[index].center;
        for record in dataset.read_chunk(index, center) {
            assert!(record.species < 6, "species {}", record.species);
            assert!(record.tint < 3, "tint {}", record.tint);
            assert!(record.height >= 3.0 && record.height <= 50.0);
            *species.entry(record.species).or_default() += 1;
        }
    }
    assert_eq!(species.len(), 6, "all six palette species present");
}
