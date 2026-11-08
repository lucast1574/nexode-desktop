#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
bash scripts/build.sh
exec "${CARGO_TARGET_DIR:-target}/release/nexode-desktop"
