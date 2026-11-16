//! Iteration-20 generation audit over the full Manhattan conversion.

use destructibles_core::math::{distance_to_polygon_edges, point_in_polygon};
use destructibles_core::{BuildingLayout, interior};
use manhattan_viewer::neighborhood::spec_for;
use manhattan_viewer::storefront::StorefrontData;

fn inside(p: [f32; 2], poly: &[[f32; 2]], tol: f32) -> bool {
    point_in_polygon(p, poly) || distance_to_polygon_edges(p, poly) <= tol
}

fn rect_outside(cx: f32, cz: f32, hx: f32, hz: f32, poly: &[[f32; 2]], tol: f32) -> bool {
    [
        [cx - hx, cz - hz],
        [cx + hx, cz - hz],
        [cx + hx, cz + hz],
        [cx - hx, cz + hz],
    ]
    .iter()
    .any(|corner| !inside(*corner, poly, tol))
}

#[test]
fn audit_generation_bounds() {
    let data = StorefrontData::default();
    let mut generated = 0usize;
    let mut failed = 0usize;
    let mut reasons: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut slab_bad = 0usize;
    let mut column_bad = 0usize;
    let mut core_bad = 0usize;
    let mut clutter_bad = 0usize;
    let mut roof_bad = 0usize;
    let mut tall_bad = 0usize;
    let mut shell_bad = 0usize;
    let mut antenna_bad = 0usize;
    let mut examples: Vec<String> = Vec::new();
    for meta in data.buildings() {
        let Ok(spec) = spec_for(meta) else {
            failed += 1;
            if let Err(error) = spec_for(meta) {
                *reasons.entry(error).or_insert(0) += 1;
            }
            continue;
        };
        let Ok(layout) = BuildingLayout::generate(spec.clone()) else {
            failed += 1;
            *reasons.entry("layout".to_string()).or_insert(0) += 1;
            continue;
        };
        generated += 1;
        let poly = &layout.footprint;
        let tol = 0.06;
        let mut bad_types = Vec::new();
        for slab in &layout.slabs {
            if rect_outside(
                slab.center[0],
                slab.center[2],
                slab.half_x,
                slab.half_z,
                poly,
                tol,
            ) {
                if slab_bad == 0 {
                    println!(
                        "FIRST SLAB b{} center {:?} half {}x{} poly {:?}",
                        meta.id, slab.center, slab.half_x, slab.half_z, poly
                    );
                }
                slab_bad += 1;
                bad_types.push("slab");
                break;
            }
        }
        for column in &layout.columns {
            if rect_outside(
                column.center[0],
                column.center[2],
                column.half,
                column.half,
                poly,
                tol,
            ) {
                column_bad += 1;
                bad_types.push("column");
                break;
            }
        }
        if let Some(core) = layout.core
            && rect_outside(
                core.center[0],
                core.center[1],
                core.half_x,
                core.half_z,
                poly,
                tol,
            )
        {
            core_bad += 1;
            bad_types.push("core");
        }
        let plan = interior::plan_interior(&layout.spec);
        if plan.floors.iter().any(|floor| {
            floor.items.iter().any(|item| {
                let (sin, cos) = item.yaw.sin_cos();
                let ex = cos.abs() * item.half[0] + sin.abs() * item.half[2];
                let ez = sin.abs() * item.half[0] + cos.abs() * item.half[2];
                rect_outside(item.center[0], item.center[2], ex, ez, poly, tol)
            })
        }) {
            clutter_bad += 1;
            bad_types.push("clutter");
        }
        for tri in layout.roof_triangles() {
            if tri
                .iter()
                .any(|point| !inside([point[0], point[2]], poly, tol))
            {
                roof_bad += 1;
                bad_types.push("roof");
                break;
            }
        }
        let max_component_top = layout
            .columns
            .iter()
            .map(|column| column.center[1] + column.height * 0.5)
            .fold(0.0f32, f32::max);
        if max_component_top > layout.height + 0.6 {
            tall_bad += 1;
            bad_types.push("tall-column");
        }
        let roof_top = layout
            .roof_triangles()
            .iter()
            .flat_map(|tri| tri.iter())
            .map(|point| point[1])
            .fold(0.0f32, f32::max);
        if roof_top > layout.height + 0.05 {
            tall_bad += 1;
            bad_types.push("tall-roof");
        }
        if let Some(core) = layout.core
            && layout.spec.floor_count as f32 * layout.spec.floor_height > layout.height + 0.6
        {
            let _ = core;
            tall_bad += 1;
            bad_types.push("tall-core");
        }
        let (center, half_x, half_z) = layout.interior_box();
        if rect_outside(center[0], center[1], half_x, half_z, poly, tol) {
            shell_bad += 1;
            bad_types.push("shell");
        }
        let max_mast = destructibles_bevy::night::antenna_heights(meta.id as u32, layout.height)
            .into_iter()
            .fold(0.0f32, f32::max);
        if max_mast > layout.height * 0.12 + 0.01 {
            antenna_bad += 1;
            bad_types.push("antenna");
        }
        if !bad_types.is_empty() && examples.len() < 30 {
            examples.push(format!(
                "b{} {} h{:.1} floors{} {:?}",
                meta.id, meta.category, meta.height_m, spec.floor_count, bad_types
            ));
        }
    }
    println!(
        "AUDIT total={} generated={generated} failed={failed} slab={slab_bad} column={column_bad} core={core_bad} clutter={clutter_bad} roof={roof_bad} tall={tall_bad} shell={shell_bad} antenna={antenna_bad}",
        data.buildings().len()
    );
    for (reason, count) in &reasons {
        println!("  failed {count}x {reason}");
    }
    for example in &examples {
        println!("  {example}");
    }
    assert_eq!(generated + failed, data.buildings().len());
    assert_eq!(
        slab_bad
            + column_bad
            + core_bad
            + clutter_bad
            + roof_bad
            + tall_bad
            + shell_bad
            + antenna_bad,
        0
    );
}
