#!/usr/bin/env bash
# Native isolated startup matrix; accepts <binary> <label> and optional --output.
set -euo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
exec bash "$repo/scripts/orbit_dev.sh" startup-acceptance-matrix "$@" --repo "$repo"
