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
    // The web page fetches every pack file with progress BEFORE it calls init(),
    // and parks the bytes on `window.__seattleAssets`. The binary therefore
    // carries no pack data: the browser caches the pack separately from the
    // wasm, and Pages gzips both.
    use wasm_bindgen::JsCast;
    let window = web_sys::window().expect("window");
    let assets = js_sys::Reflect::get(&window, &wasm_bindgen::JsValue::from_str("__seattleAssets"))
        .unwrap_or(wasm_bindgen::JsValue::UNDEFINED);
    let entry = js_sys::Reflect::get(&assets, &wasm_bindgen::JsValue::from_str(relative))
        .unwrap_or(wasm_bindgen::JsValue::UNDEFINED);
    let array = entry
        .dyn_into::<js_sys::Uint8Array>()
        .unwrap_or_else(|_| panic!("web asset not preloaded: {relative}"));
    array.to_vec()
}
