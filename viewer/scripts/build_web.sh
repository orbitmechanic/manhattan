#!/usr/bin/env bash
# Build the Seattle viewer for the web (wasm32 + WebGL2), mirroring the
# Manhattan dock pipeline.
#
#   ./scripts/build_web.sh            # build into viewer/web/dist
#
# Native keeps the photographic BC7 path (default `photo-textures` feature);
# the web build runs --no-default-features so the C++ KTX2 decoder
# (basis-universal-sys, no wasm32 target) is not compiled, and the app uses the
# generated atlas path instead.
#
# NOTE: the runtime asset path is still native-shaped (std::fs for the DEM and
# city pack). Until those reads are fetched over HTTP the produced bundle
# compiles but will not boot in a browser — tracked as seattle-web3.
set -euo pipefail

cd "$(dirname "$0")/.."
OUT=web/dist
PROFILE=wasm-release

cargo build --profile "$PROFILE" --target wasm32-unknown-unknown --no-default-features

WASM="target/wasm32-unknown-unknown/$PROFILE/manhattan_viewer.wasm"
wasm-bindgen --target web --out-dir "$OUT" --out-name seattle "$WASM"
wasm-opt -Oz -o "$OUT/seattle_bg.wasm" "$OUT/seattle_bg.wasm" || true

mkdir -p "$OUT/assets"
cp -r assets/shaders assets/textures assets/sounds "$OUT/assets/" 2>/dev/null || true

echo "web bundle: $(du -sh "$OUT" | cut -f1) in $OUT"
