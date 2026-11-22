#!/usr/bin/env bash
# Seattle persistence gate (S3.3j).
#
# One command, one pass/fail: damage a CityPack record by printed id, freeze it
# beyond SEATTLE_ARCHIVE_RADIUS_M with the host far plume, thaw in place,
# restore to journal 0, and evict back to a terrain-seated shell.
#
# Asserts (the viewer panics on any failure):
#   - journal > 10 entries before the freeze
#   - journal entries and checksum identical before and after freeze/thaw
#   - journal 0 and checkpoint checksum equal to the pristine baseline
#   - registered false, structures 0, terrain seat true after eviction
#
# Captures land in viewer/captures/s3_cycle_{damaged,plume,return,restore}.png.
#
# Usage: scripts/seattle_persistence.sh [building-id | landmark-key]
#   no args            streamed CityPack record 187064
#   needle|market|...  landmark freeze/thaw/restore cycle (LANDMARK_CYCLE)
set -euo pipefail
cd "$(dirname "$0")/.."

TARGET="${1:-187064}"
BIN="target/release/manhattan-viewer"
LOG="${TMPDIR:-/tmp}/seattle_persistence.log"
case "$TARGET" in
needle | market | aurora | fremont | ballard | monorail | arena | lumen | tmobile)
    MODE=landmark
    ;;
*)
    MODE=building
    ID="$TARGET"
    ;;
esac

if [[ ! -x "$BIN" ]]; then
    echo "missing $BIN; run: cargo build --release" >&2
    exit 1
fi

set +e
if [[ "$MODE" == "landmark" ]]; then
    LANDMARK_CYCLE=1 LANDMARK="$TARGET" TREES_OFF=1 "$BIN" --landmarks >"$LOG" 2>&1
else
    S3_CYCLE=1 S3_ID="$ID" "$BIN" --seattle-s3-restore >"$LOG" 2>&1
fi
status=$?
set -e

if [[ "$MODE" == "landmark" ]]; then
    if grep -q "LANDMARK CYCLE PASS" "$LOG"; then
        grep -a "LANDMARK CYCLE" "$LOG" | sed 's/\x1b\[[0-9;]*m//g'
        echo "PASS: landmark persistence gate ($TARGET)"
        exit 0
    fi
    echo "FAIL: landmark persistence gate ($TARGET, viewer exit $status) — $LOG" >&2
    exit 1
fi

if grep -q "S3 CYCLE thaw: journal continuity PASS" "$LOG" \
    && grep -q "S3 CYCLE PASS: journal 0, checksum_match true" "$LOG"; then
    grep -a "S3 CYCLE" "$LOG" | sed 's/\x1b\[[0-9;]*m//g'
    echo "PASS: seattle persistence gate (id $ID)"
    exit 0
fi

echo "FAIL: seattle persistence gate (id $ID, viewer exit $status) — $LOG" >&2
exit 1
