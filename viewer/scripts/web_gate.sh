#!/usr/bin/env bash
# Web build gate: fail the build if a std-time symbol is imported by the wasm,
# or the bundle is missing its core files. Cloned from the Seattle precedent for
# the destructibles deck.
#
#   ./scripts/web_gate.sh <path-to.wasm>
#
# std::time::Instant::now() panics on wasm32-unknown-unknown ("time not
# implemented on this platform"); clock_compat exists to keep that symbol out of
# the module. This gate makes a regression fail here, not in a player's browser.
set -euo pipefail

WASM=${1:?usage: web_gate.sh <module.wasm>}
fail=0

if [ ! -f "$WASM" ]; then
  echo "GATE: missing wasm: $WASM"
  exit 1
fi

# 1. std time imports. The wasm import section names them as
#    `__wbindgen`-free rust symbols; scan the raw bytes for the canonical names.
if strings -a "$WASM" | grep -qE 'std[0-9a-f]*::(time::Instant::now|sys::time)'; then
  echo "GATE: std::time symbols present — route them through clock_compat"
  strings -a "$WASM" | grep -E 'std[0-9a-f]*::(time::Instant::now|sys::time)' | head -5
  fail=1
fi

# 2. no KTX2/UASTC requests from the web bundle
if strings -a "$WASM" | grep -q '\.ktx2'; then
  echo "GATE: KTX2 asset paths referenced — the web build has no basis decoder"
  strings -a "$WASM" | grep '\.ktx2' | head -5
  fail=1
fi

if [ "$fail" -ne 0 ]; then
  echo "GATE: FAILED"
  exit 1
fi

echo "GATE: ok ($(du -h "$WASM" | cut -f1) wasm, no std-time or ktx2 symbols)"
