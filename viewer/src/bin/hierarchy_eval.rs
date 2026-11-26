//! Offline evaluation-count report for CityPack v2.
//!
//! Compares the flat 1,096-neighborhood scan with the four-level descent at the
//! standard poses, then repeats the count on a synthetic 10x hierarchy to
//! quantify the culling win on a megaregion-sized dataset.

use std::path::PathBuf;

use manhattan_viewer::citypack::{
    CityPack, Evaluations, LEVEL_CITY, LEVEL_COUNTY, LEVEL_NEIGHBORHOOD, PackNode,
    wanted_neighborhoods,
};

const RADIUS: f32 = 2600.0;

fn expand_10x(nodes: &[PackNode]) -> Vec<PackNode> {
    let count = nodes.len();
    let stride = 200_000.0_f32;
    let mut out = Vec::with_capacity(count * 10);
    for copy in 0..10 {
        let base = copy * count;
        let offset = copy as f32 * stride;
        for node in nodes {
            let mut cloned = node.clone();
            cloned.parent = if node.parent < 0 {
                -1
            } else {
                (base + node.parent as usize) as i32
            };
            cloned.first_child = if node.child_count > 0 {
                (base + node.first_child as usize) as u32
            } else {
                0
            };
            cloned.center[0] += offset;
            cloned.bounds[0] += offset;
            cloned.bounds[2] += offset;
            out.push(cloned);
        }
    }
    out
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "assets/datasets/seattle.citypack".to_string());
    let pack = CityPack::open(&path).expect("citypack");
    let flat = pack.neighborhoods().len();
    let expanded = expand_10x(&pack.nodes);
    let flat_10x = expanded
        .iter()
        .filter(|node| node.level == LEVEL_NEIGHBORHOOD)
        .count();
    let counties = pack
        .nodes
        .iter()
        .filter(|node| node.level == LEVEL_COUNTY)
        .count();
    let cities = pack
        .nodes
        .iter()
        .filter(|node| node.level == LEVEL_CITY)
        .count();
    println!(
        "PACK nodes={} counties={} cities={} neighborhoods={} buildings={} streets={}",
        pack.nodes.len(),
        counties,
        cities,
        flat,
        pack.building_count,
        pack.street_count
    );

    let poses = [
        ("downtown", [0.0, 0.0]),
        ("mid-lake", [5_000.0, -200.0]),
        ("bellevue", [10_081.0, -245.0]),
        ("tacoma-corridor", [0.0, 34_000.0]),
        ("far-south", [0.0, 56_000.0]),
    ];
    let mut report = serde_json::json!({
        "pack": path,
        "flat": flat,
        "flat_10x": flat_10x,
        "radius_m": RADIUS,
        "poses": []
    });
    println!(
        "{:<16} {:>6} {:>4} {:>4} {:>5} {:>6} {:>7} {:>8} {:>9} {:>7}",
        "pose", "flat", "co", "ci", "nb", "total", "ratio", "flat10x", "hier10x", "ratio10x"
    );
    for (name, center) in poses {
        let mut evals = Evaluations::default();
        let wanted = wanted_neighborhoods(&pack.nodes, center, RADIUS, &mut evals);
        let mut evals_10x = Evaluations::default();
        let wanted_10x = wanted_neighborhoods(&expanded, center, RADIUS, &mut evals_10x);
        let total = evals.total();
        let total_10x = evals_10x.total();
        let ratio = flat as f32 / total.max(1) as f32;
        let ratio_10x = flat_10x as f32 / total_10x.max(1) as f32;
        println!(
            "{:<16} {:>6} {:>4} {:>4} {:>5} {:>6} {:>7.2} {:>8} {:>9} {:>7.2}",
            name,
            flat,
            evals.counties,
            evals.cities,
            evals.neighborhoods,
            total,
            ratio,
            flat_10x,
            total_10x,
            ratio_10x
        );
        report["poses"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "pose": name,
                "center": center,
                "flat": flat,
                "hierarchical": {
                    "counties": evals.counties,
                    "cities": evals.cities,
                    "neighborhoods": evals.neighborhoods,
                    "total": total,
                },
                "ratio": ratio,
                "loaded_neighborhoods": wanted.len(),
                "flat_10x": flat_10x,
                "hierarchical_10x": {
                    "counties": evals_10x.counties,
                    "cities": evals_10x.cities,
                    "neighborhoods": evals_10x.neighborhoods,
                    "total": total_10x,
                },
                "ratio_10x": ratio_10x,
                "loaded_neighborhoods_10x": wanted_10x.len(),
            }));
    }
    let out = PathBuf::from("datasets/seattle/eval_report.json");
    std::fs::write(&out, serde_json::to_string_pretty(&report).unwrap()).expect("write report");
    println!("wrote {}", out.display());
}
