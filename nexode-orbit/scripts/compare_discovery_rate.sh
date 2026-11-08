#!/usr/bin/env bash
# Matched before/after comparison for a discover_tools description change.
#
# Runs the same case subset against two binaries with one model, so the only
# variable is the tool description. Usage:
#   scripts/compare_discovery_rate.sh <before-binary> <after-binary> [model]
set -euo pipefail

BEFORE="${1:?before binary}"
AFTER="${2:?after binary}"
MODEL="${3:-gemini-2.5-flash}"
TRIALS="${4:-3}"
cd "$(dirname "$0")/.."

CASES=(
  --case storage-user-uploads
  --case authentication-signin
  --case observability-traces
  --case analytics-product-funnel
  --case code-review-automation
  --case web-search-live-answers
  --case control-sqlite-local
  --case control-regex-debug
)

run() {
  local binary="$1" out="$2"
  echo "=== $out ($binary)"
  NEXODE_BIN="$binary" bash scripts/orbit_dev.sh benchmark-discovery-rate \
    --provider "${PROVIDER:-nexode}" --model "$MODEL" --timeout 150 --trials "$TRIALS" \
    "${CASES[@]}" --output "target/discovery-rate/$out.json" || true
}

run "$BEFORE" before
run "$AFTER" after

bash scripts/orbit_dev.sh compare-discovery-rate target/discovery-rate/before.json target/discovery-rate/after.json
