#!/usr/bin/env bash
# Seattle S3 16-row frame-cost table (S3.4).
#
# Runs the ten S2 poses with buildings streaming, then the four gauntlet
# regimes, the low orbit and the chunk hover, all with S3 building streaming.
# Each run appends/updates its rows in datasets/seattle/s3_perf.json; the
# script then snapshots that file to s3_perf_<label>.json.
#
# Usage: scripts/seattle_s3_table.sh <label>
set -euo pipefail
cd "$(dirname "$0")/.."

LABEL="${1:?usage: scripts/seattle_s3_table.sh <label>}"
BIN="target/release/manhattan-viewer"
LOG_DIR="${TMPDIR:-/tmp}"

if [[ ! -x "$BIN" ]]; then
    echo "missing $BIN; run: cargo build --release" >&2
    exit 1
fi

run() {
    local name="$1"
    shift
    echo "== $name"
    "$@" >"$LOG_DIR/s3_table_${LABEL}_${name}.log" 2>&1
}

# Virtual time clamps at 250 ms per frame, so a heavy tree needs minutes of
# wall clock for the gauntlet to finish its 18.5 virtual seconds.
run poses timeout 600 "$BIN" --seattle-s3
run gauntlet timeout 900 env SEATTLE_S3=1 "$BIN" --seattle-s2-gauntlet
run orbit timeout 600 env SEATTLE_S3=1 "$BIN" --seattle-s2-orbit
run hover timeout 600 env SEATTLE_S3=1 "$BIN" --seattle-s2-hover

SNAP="datasets/seattle/s3_perf_${LABEL}.json"
cp datasets/seattle/s3_perf.json "$SNAP"
python3 - "$SNAP" <<'PY'
import json
import sys

rows = json.load(open(sys.argv[1]))
print(f"{'row':32s} {'avg ms':>8s} {'p99 ms':>8s} {'samples':>8s}")
for row in rows:
    print(
        f"{row['pose']:32s} {row['avg_ms']:8.2f} {row['p99_ms']:8.2f} "
        f"{row.get('samples', 0):8d}"
    )
PY
echo "snapshot: $SNAP"
