//! Asset bytes: filesystem on native, embedded pack on web.
//!
//! The browser has no filesystem, and the viewer's data (DEM, mask, water,
//! city pack, trees) is read once at boot. Rather than build a streaming fetch
//! layer for a walking-pace demo, the web build EMBEDS the demo pack in the
//! wasm binary: one download, no fetch ordering, no CORS, no half-fetched city.
//! Native keeps the exact same call sites reading the same files from disk.

/// Read a repository-relative asset (e.g. `assets/datasets/seattle.citypack`).
pub fn bytes(relative: &str) -> Vec<u8> {
    #[cfg(target_arch = "wasm32")]
    {
        embedded(relative)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
        std::fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
    }
}

/// Read a UTF-8 asset (JSON, WESL, styles).
pub fn string(relative: &str) -> String {
    String::from_utf8(bytes(relative)).unwrap_or_else(|_| panic!("{relative} is not utf-8"))
}

#[cfg(target_arch = "wasm32")]
fn embedded(relative: &str) -> Vec<u8> {
    // The web pack: downtown demo footprint. Every byte here is downloaded
    // before the first frame, so the list is the budget.
    match relative {
        "assets/datasets/seattle.citypack" => {
            include_bytes!("../assets/datasets/seattle.citypack").to_vec()
        }
        "assets/datasets/seattle_trees.bin" => {
            include_bytes!("../assets/datasets/seattle_trees.bin").to_vec()
        }
        "assets/terrain/seattle_dem_30m.bin" => {
            include_bytes!("../assets/terrain/seattle_dem_30m.bin").to_vec()
        }
        "assets/terrain/seattle_water_mask.bin" => {
            include_bytes!("../assets/terrain/seattle_water_mask.bin").to_vec()
        }
        "assets/terrain/seattle_water.bin" => {
            include_bytes!("../assets/terrain/seattle_water.bin").to_vec()
        }
        "datasets/seattle/s2_report.json" => {
            include_bytes!("../datasets/seattle/s2_report.json").to_vec()
        }
        "assets/styles/seattle_pack.json" => {
            include_bytes!("../assets/styles/seattle_pack.json").to_vec()
        }
        "assets/styles/seattle_districts.json" => {
            include_bytes!("../assets/styles/seattle_districts.json").to_vec()
        }
        "assets/styles/seattle_governance.json" => {
            include_bytes!("../assets/styles/seattle_governance.json").to_vec()
        }
        "assets/horizon/seattle_profiles.bin" => {
            include_bytes!("../assets/horizon/seattle_profiles.bin").to_vec()
        }
        other => panic!("asset not in the web pack: {other}"),
    }
}
