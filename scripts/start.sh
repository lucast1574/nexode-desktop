#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
[[ -f "$HOME/.cargo/env" ]] && source "$HOME/.cargo/env"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
cargo build --locked --release
exec "${CARGO_TARGET_DIR:-target}/release/nexode-desktop"
