#!/usr/bin/env bash
# Build native maintenance tooling before entering stripped test environments.
set -euo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
if [[ -n ${ORBIT_DEV_BIN:-} ]]; then
    binary=$ORBIT_DEV_BIN
else
    target_dir=${ORBIT_DEV_TARGET_DIR:-$repo/tools/orbit-dev/target}
    CARGO_BUILD_RUSTC_WRAPPER='' cargo build --quiet --locked \
        --manifest-path "$repo/tools/orbit-dev/Cargo.toml" \
        --target-dir "$target_dir" --bin orbit-dev
    binary=$target_dir/debug/orbit-dev
fi
[[ -x "$binary" ]] || { echo "Native maintenance binary is missing: $binary" >&2; exit 1; }
binary=$(realpath "$binary")
if [[ ${1:-} == --build-only ]]; then
    [[ $# == 1 ]] || { echo "--build-only takes no other arguments" >&2; exit 2; }
    printf '%s\n' "$binary"
else
    exec "$binary" "$@"
fi
