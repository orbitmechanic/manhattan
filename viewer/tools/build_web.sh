#!/usr/bin/env bash
# Build the browser version of the Manhattan viewer.
#
# Run from anywhere; output lands in viewer/web/dist and is ready to copy to
# the GitHub Pages docs/ folder.
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$(pwd)"
DIST="$ROOT/web/dist"
WASM="$ROOT/target/wasm32-unknown-unknown/wasm-release/manhattan-viewer.wasm"

echo "== building wasm =="
cargo build --profile wasm-release --target wasm32-unknown-unknown --bin manhattan-viewer

echo "== bindgen =="
rm -rf "$DIST"
mkdir -p "$DIST"
wasm-bindgen \
  --target web \
  --no-typescript \
  --out-name manhattan \
  --out-dir "$DIST" \
  "$WASM"

echo "== wasm-opt =="
wasm-opt -Oz --output "$DIST/manhattan_bg.opt.wasm" "$DIST/manhattan_bg.wasm"
mv "$DIST/manhattan_bg.opt.wasm" "$DIST/manhattan_bg.wasm"

echo "== stage html/assets =="
cp "$ROOT/web/index.html" "$ROOT/web/style.css" "$DIST/"
mkdir -p "$DIST/assets/models" "$DIST/assets/textures/night" \
  "$DIST/assets/textures/facades" "$DIST/assets/shaders"
cp "$ROOT/assets/models/manhattan_city.glb" "$DIST/assets/models/"
cp "$ROOT/assets/models/manhattan_ground.glb" "$DIST/assets/models/"
cp -R "$ROOT/assets/models/roadway_chunks" "$DIST/assets/models/"
cp "$ROOT/assets/textures/fog_bank.png" "$DIST/assets/textures/"
cp -R "$ROOT/assets/textures/facades/." "$DIST/assets/textures/facades/"
cp -R "$ROOT/assets/textures/night/." "$DIST/assets/textures/night/"
cp "$ROOT/assets/shaders/city_block.wesl" "$DIST/assets/shaders/"
touch "$DIST/.nojekyll"

echo "== done =="
du -sh "$DIST"
ls -lh "$DIST"/*.wasm "$DIST/assets/models"/*.glb
echo "roadway chunks: $(ls "$DIST/assets/models/roadway_chunks"/*.glb | wc -l)"
