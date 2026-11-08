#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
[[ ! -f "$HOME/.cargo/env" ]] || source "$HOME/.cargo/env"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
# Both binaries must live together so engine_binary() finds the bundled engine.
mkdir -p "${CARGO_TARGET_DIR:-target}"
export CARGO_TARGET_DIR="$(cd "${CARGO_TARGET_DIR:-target}" && pwd)"
cargo build --locked --release
cargo build --manifest-path nexode-orbit/Cargo.toml --locked --release --no-default-features --bin nexode
