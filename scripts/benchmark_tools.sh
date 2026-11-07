#!/usr/bin/env bash
# Native tool benchmark; use additional --fresh --binary PATH for an isolated daemon.
set -euo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
iterations=${1:-5}
if [[ $# -gt 0 ]]; then shift; fi
if [[ $# == 0 ]]; then
    set -- --socket "${NEXODE_DEBUG_SOCK:-/run/user/$(id -u)/nexode-debug.sock}"
fi
exec bash "$repo/scripts/orbit_dev.sh" benchmark-tools "$iterations" \
    --repo "$repo" --csv "${RESULTS_FILE:-/tmp/nexode_tool_benchmark_$(date +%Y%m%d_%H%M%S).csv}" "$@"
