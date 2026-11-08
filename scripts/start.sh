#!/usr/bin/env bash
set -euo pipefail
cd "${1:-$(dirname "$0")/..}"
source "$HOME/.cargo/env"
export CARGO_BUILD_RUSTC_WRAPPER=
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/orbit-desktop-target}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
if [[ -z "${NEXODE_BINARY:-}" ]]; then
  if command -v nexode >/dev/null; then
    export NEXODE_BINARY="$(command -v nexode)"
  elif [[ -x "$HOME/orbit-root-target/debug/nexode" ]]; then
    export NEXODE_BINARY="$HOME/orbit-root-target/debug/nexode"
  else
    echo 'Define NEXODE_BINARY con la ruta del ejecutable de Orbit.' >&2
    exit 1
  fi
fi
export ORBIT_PROJECTS="${ORBIT_PROJECTS:-$(dirname "$PWD")}"
cargo build --locked --manifest-path server/Cargo.toml
exec "$CARGO_TARGET_DIR/debug/nexode-desktop"
