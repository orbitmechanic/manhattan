//! Pilot neighborhood generator: writes a committed bundle from the real map
//! records using the same spec/layout code the runtime generator uses.

use std::collections::BTreeMap;
use std::fs;

use destructibles_core::{BuildingLayout, FacadeStyle};
use manhattan_viewer::neighborhood::spec_for;
use manhattan_viewer::storefront::BuildingMeta;
use serde::Serialize;

const CENTER: [f32; 2] = [1711.638, -1300.0];
const HALF: f32 = 420.0;
const MAX_BUILDINGS: usize = 140;

#[derive(Serialize)]
struct Bundle {
    city_id: String,
    region: Region,
    records: Vec<Record>,
    audits: Audits,
    view: Option<View>,
}

#[derive(Serialize)]
struct View {
    street_pos: [f32; 3],
    street_look: [f32; 3],
    aerial_pos: [f32; 3],
    aerial_look: [f32; 3],
}

#[derive(Serialize)]
struct Region {
    center: [f32; 2],
    half_extent_m: f32,
    ground_elevation_m: f32,
}

#[derive(Serialize)]
struct Record {
    id: usize,
    origin: [f32; 3],
    height_m: f32,
    style: String,
    footprint: Vec<[f32; 2]>,
    floor_height_m: f32,
    floor_count: u32,
    panes: usize,
    cells: usize,
    storefronts: usize,
    ads: usize,
    fallback_box: Option<[f32; 3]>,
}

#[derive(Serialize, Default)]
struct Audits {
    selected: usize,
    generated: usize,
    rejected: usize,
    fallback_boxes: usize,
    envelope_offenders: usize,
    row_offenders: usize,
    batches: BTreeMap<String, usize>,
    triangles_estimate: usize,
    storefronts: usize,
    ads: usize,
}

fn layout_envelope_ok(layout: &BuildingLayout) -> bool {
    let poly = &layout.footprint;
    let inside = |x: f32, z: f32| {
        destructibles_core::math::point_in_polygon([x, z], poly)
            || destructibles_core::math::distance_to_polygon_edges([x, z], poly) <= 0.06
    };
    layout.slabs.iter().all(|slab| {
        [
            [slab.center[0] - slab.half_x, slab.center[2] - slab.half_z],
            [slab.center[0] + slab.half_x, slab.center[2] - slab.half_z],
            [slab.center[0] + slab.half_x, slab.center[2] + slab.half_z],
            [slab.center[0] - slab.half_x, slab.center[2] + slab.half_z],
        ]
        .iter()
        .all(|corner| inside(corner[0], corner[1]))
    }) && layout.columns.iter().all(|column| {
        inside(
            column.center[0] - column.half,
            column.center[2] - column.half,
        ) && inside(
            column.center[0] + column.half,
            column.center[2] + column.half,
        )
    })
}

fn main() {
    // CityDataset v1.1 is the only input. Manhattan is just the adapter.
    let dataset = std::env::var("PILOT_DATASET")
        .unwrap_or_else(|_| "assets/datasets/manhattan_v11.json".to_string());
    let text = fs::read_to_string(&dataset).expect("dataset");
    let root: serde_json::Value = serde_json::from_str(&text).expect("dataset json");
    let mut center = CENTER;
    if let Ok(raw) = std::env::var("PILOT_CENTER") {
        if let Some((x, z)) = raw.split_once(',') {
            center = [
                x.trim().parse().unwrap_or(CENTER[0]),
                z.trim().parse().unwrap_or(CENTER[1]),
            ];
        }
    } else if root["city_id"].as_str() != Some("manhattan")
        && let Some(ep) = root["fields"]["epicenter"].as_array()
    {
        center = [
            ep[0].as_f64().unwrap() as f32,
            ep[1].as_f64().unwrap() as f32,
        ];
    }
    let mut metas: Vec<BuildingMeta> = Vec::new();
    for b in root["buildings"].as_array().expect("buildings") {
        metas.push(BuildingMeta {
            id: b["id"].as_u64().unwrap() as usize,
            block: b["block"].as_u64().unwrap() as usize,
            height_m: b["height_m"].as_f64().unwrap() as f32,
            floor_height_m: b["floor_height_m"].as_f64().unwrap() as f32,
            perimeter: serde_json::from_value(b["perimeter"].clone()).unwrap(),
            centroid: serde_json::from_value(b["centroid"].clone()).unwrap(),
            category: b["category"].as_str().unwrap().to_string(),
            year_built: b["year_built"].as_u64().unwrap_or(0) as u32,
        });
    }
    let mut candidates: Vec<_> = metas
        .iter()
        .map(|meta| {
            let d = ((meta.centroid[0] - center[0]).powi(2)
                + (meta.centroid[1] - center[1]).powi(2))
            .sqrt();
            (d, meta)
        })
        .filter(|(d, _)| *d <= HALF)
        .collect();
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
    candidates.truncate(MAX_BUILDINGS);

    let city_id = root["city_id"].as_str().unwrap_or("unknown").to_string();
    let epicenter = root["fields"]["epicenter"]
        .as_array()
        .map(|p| [p[0].as_f64().unwrap() as f32, p[1].as_f64().unwrap() as f32])
        .unwrap_or(CENTER);
    let bases: std::collections::HashMap<usize, f32> = root["buildings"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|b| {
                    Some((
                        b["id"].as_u64()? as usize,
                        b["base_elevation_m"].as_f64().unwrap_or(0.0) as f32,
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut audits = Audits::default();
    let mut records = Vec::new();
    for (_, meta) in &candidates {
        audits.selected += 1;
        let base = bases.get(&meta.id).copied().unwrap_or(0.0);
        let origin = [meta.centroid[0], base, meta.centroid[1]];
        match spec_for(meta) {
            Ok(spec) => match BuildingLayout::generate(spec.clone()) {
                Ok(layout) => {
                    audits.generated += 1;
                    if !layout_envelope_ok(&layout) {
                        audits.envelope_offenders += 1;
                    }
                    let expected = (meta.height_m / meta.floor_height_m.clamp(2.4, 6.0))
                        .round()
                        .max(1.0) as u32;
                    if layout.spec.floor_count != expected {
                        audits.row_offenders += 1;
                    }
                    let style = layout.spec.facade_style;
                    *audits.batches.entry(style.label().to_string()).or_insert(0) += 1;
                    audits.triangles_estimate += layout.panes.len() * 2
                        + layout.slabs.len() * 32
                        + layout.columns.len() * 12;
                    let d = ((meta.centroid[0] - epicenter[0]).powi(2)
                        + (meta.centroid[1] - epicenter[1]).powi(2))
                    .sqrt();
                    let density = if d <= 250.0 {
                        1.0
                    } else {
                        ((2500.0 - d) / 2250.0).clamp(0.0, 1.0).powf(1.35)
                    };
                    let storefronts = if density > 0.05 {
                        (2.0 + density * 4.0).round() as usize
                    } else {
                        0
                    };
                    let ads = (density * 4.0).round() as usize;
                    audits.storefronts += storefronts;
                    audits.ads += ads;
                    records.push(Record {
                        id: meta.id,
                        origin,
                        height_m: meta.height_m,
                        style: style.label().to_string(),
                        footprint: layout.spec.footprint.clone(),
                        floor_height_m: layout.spec.floor_height,
                        floor_count: layout.spec.floor_count,
                        panes: layout.panes.len(),
                        cells: layout.cells.len(),
                        storefronts,
                        ads,
                        fallback_box: None,
                    });
                }
                Err(_) => rejected(meta, base, &mut audits, &mut records),
            },
            Err(_) => rejected(meta, base, &mut audits, &mut records),
        }
    }

    let glass = records
        .iter()
        .filter(|record| record.style == "glass curtain wall")
        .count() as f32;
    let fraction = if records.is_empty() {
        0.0
    } else {
        glass / records.len() as f32
    };
    println!(
        "PILOT MIX: glass {:.2}, records {}",
        fraction,
        records.len()
    );
    if root["city_id"].as_str() == Some("manhattan") {
        assert!(
            (0.40..=0.60).contains(&fraction),
            "mixed-region gate: glass fraction {fraction:.2} outside 0.40..=0.60"
        );
    }
    let ground = records
        .iter()
        .map(|record| record.origin[1])
        .fold(f32::INFINITY, f32::min);
    let ground = if ground.is_finite() { ground } else { 0.0 };
    let view = (city_id != "manhattan").then(|| plan_view(ground, &records));
    let bundle = Bundle {
        city_id,
        region: Region {
            center,
            half_extent_m: HALF,
            ground_elevation_m: ground,
        },
        records,
        audits,
        view,
    };
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/pilot");
    fs::create_dir_all(&dir).expect("pilot dir");
    let out_name =
        std::env::var("PILOT_BUNDLE_OUT").unwrap_or_else(|_| "pilot_bundle.json".to_string());
    let path = dir.join(out_name);
    fs::write(&path, serde_json::to_string_pretty(&bundle).unwrap()).expect("write bundle");
    println!(
        "PILOT: selected={} generated={} rejected={} failures={} envelope={} rows={} batches={:?} triangles≈{} -> {}",
        bundle.audits.selected,
        bundle.audits.generated,
        bundle.audits.rejected,
        bundle.audits.fallback_boxes,
        bundle.audits.envelope_offenders,
        bundle.audits.row_offenders,
        bundle.audits.batches,
        bundle.audits.triangles_estimate,
        path.display()
    );
    println!(
        "PILOT PLACEMENTS: storefronts={} ads={} (region radial field)",
        bundle.audits.storefronts, bundle.audits.ads
    );
}

fn plan_view(ground: f32, records: &[Record]) -> View {
    let n = records.len().max(1) as f32;
    let target = [
        records.iter().map(|record| record.origin[0]).sum::<f32>() / n,
        records.iter().map(|record| record.origin[2]).sum::<f32>() / n,
    ];
    let look_y = records
        .iter()
        .map(|record| record.origin[1])
        .fold(f32::NEG_INFINITY, f32::max)
        .min(ground + 40.0);
    let blocked = |x: f32, z: f32| {
        records.iter().any(|record| {
            if record.footprint.is_empty() {
                return false;
            }
            if destructibles_core::math::point_in_polygon([x, z], &record.footprint) {
                return true;
            }
            if destructibles_core::math::distance_to_polygon_edges([x, z], &record.footprint) < 3.0
            {
                return true;
            }
            // Plinth bbox is a little larger than the footprint.
            let minx = record
                .footprint
                .iter()
                .map(|p| p[0])
                .fold(f32::INFINITY, f32::min)
                - 1.0;
            let maxx = record
                .footprint
                .iter()
                .map(|p| p[0])
                .fold(f32::NEG_INFINITY, f32::max)
                + 1.0;
            let minz = record
                .footprint
                .iter()
                .map(|p| p[1])
                .fold(f32::INFINITY, f32::min)
                - 1.0;
            let maxz = record
                .footprint
                .iter()
                .map(|p| p[1])
                .fold(f32::NEG_INFINITY, f32::max)
                + 1.0;
            x > minx && x < maxx && z > minz && z < maxz
        })
    };
    let corridor_clear = |x: f32, z: f32| {
        (0..=32).all(|step| {
            let t = step as f32 / 32.0;
            let px = x + (target[0] - x) * t;
            let pz = z + (target[1] - z) * t;
            !blocked(px, pz)
        })
    };
    let mut street = [target[0], ground + 8.0, target[1] + HALF * 0.95];
    'outer: for radius_step in 0..6 {
        for step in 0..96 {
            let angle = std::f32::consts::PI * (0.10 + 0.80 * step as f32 / 95.0);
            let radius = HALF * (0.42 + 0.06 * radius_step as f32);
            let x = target[0] + radius * angle.sin();
            let z = target[1] + radius * angle.cos();
            if corridor_clear(x, z) {
                street = [x, ground + 8.0, z];
                break 'outer;
            }
        }
    }
    View {
        street_pos: street,
        street_look: [target[0], look_y, target[1]],
        aerial_pos: [target[0], ground + 170.0, target[1] + HALF * 0.6],
        aerial_look: [target[0], ground + 25.0, target[1]],
    }
}

fn rejected(
    meta: &manhattan_viewer::storefront::BuildingMeta,
    base: f32,
    audits: &mut Audits,
    records: &mut Vec<Record>,
) {
    audits.rejected += 1;
    audits.fallback_boxes += 1;
    let mut min = [f32::INFINITY; 2];
    let mut max = [f32::NEG_INFINITY; 2];
    for p in &meta.perimeter {
        min[0] = min[0].min(p[0]);
        min[1] = min[1].min(p[1]);
        max[0] = max[0].max(p[0]);
        max[1] = max[1].max(p[1]);
    }
    records.push(Record {
        id: meta.id,
        origin: [meta.centroid[0], base, meta.centroid[1]],
        height_m: meta.height_m,
        style: FacadeStyle::BrickMidrise.label().to_string(),
        footprint: vec![],
        floor_height_m: 3.2,
        floor_count: 1,
        panes: 0,
        cells: 0,
        storefronts: 0,
        ads: 0,
        fallback_box: Some([
            (max[0] - min[0]).max(2.0),
            meta.height_m.max(2.0),
            (max[1] - min[1]).max(2.0),
        ]),
    });
}
