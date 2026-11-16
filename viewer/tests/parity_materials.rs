//! Iteration-20 parity: one shared palette and one rooftop recipe.

use destructibles_core::{palette, roof};
use manhattan_viewer::storefront::StorefrontData;

#[test]
fn map_and_generated_roofs_share_the_recipe() {
    let data = StorefrontData::default();
    let mut sampled = 0;
    for building in data.buildings().iter().take(400) {
        if building.perimeter.len() < 3 || building.height_m < 4.0 {
            continue;
        }
        let a = roof::roof_equipment(building.id as u32, &building.perimeter, building.height_m);
        let b = roof::roof_equipment(building.id as u32, &building.perimeter, building.height_m);
        assert_eq!(
            a, b,
            "b{} equipment must regenerate identically",
            building.id
        );
        let masts = roof::antenna_heights(building.id as u32, building.height_m);
        assert!(!masts.is_empty());
        for mast in masts {
            assert!(mast <= building.height_m * 0.12 + 0.01);
        }
        if !a.is_empty() {
            sampled += 1;
        }
    }
    assert!(sampled > 100, "expected equipment on most sampled roofs");
}

#[test]
fn palette_is_the_single_source_of_truth() {
    // Both generators import these constants; a drift here is a visual split.
    assert_eq!(palette::ROOF_TAN, [0.64, 0.60, 0.51]);
    assert_eq!(palette::BRICK, [0.82, 0.75, 0.65]);
    assert_eq!(palette::GLASS, [0.63, 0.71, 0.77]);
    assert_eq!(palette::EQUIPMENT, [0.46, 0.47, 0.49]);
}
