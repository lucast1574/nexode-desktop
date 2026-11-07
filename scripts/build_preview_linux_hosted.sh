#!/usr/bin/env bash
# Hosted compilation only. Packaging and native/baseline runtime acceptance belong
# to the caller. Requires a reviewed client with managed glibc217/ARM64 support.
# Published alpha.19 predates that support, despite sharing the version label
# with newer source builds. Unsupported clients must fail, never build locally.
# Usage: NEXODE_BUILD_SEMVER=v1.0.0-preview.N bash scripts/build_preview_linux_hosted.sh TARGET
set -euo pipefail

fail() { printf 'error: %s\n' "$*" >&2; exit 1; }
[[ $# == 1 ]] || fail 'usage: build_preview_linux_hosted.sh TARGET'
target=$1
case "$target" in
  x86_64-unknown-linux-gnu) toolchain=1.99.0 ;;
  aarch64-unknown-linux-gnu) toolchain=nightly-2026-10-08 ;;
  *) fail "unsupported hosted preview target: $target" ;;
esac
[[ $(uname -s) == Linux ]] || fail 'hosted Linux preview compilation requires a Linux caller'
[[ -n ${NEXODE_BUILD_SEMVER:-} ]] || fail 'NEXODE_BUILD_SEMVER must identify the preview version'
# Global flags can override target-only TLS or bring a development linker into
# a cross build. Refuse rather than silently dropping caller requirements.
[[ -z ${RUSTFLAGS:-} && -z ${CARGO_ENCODED_RUSTFLAGS:-} ]] || fail 'global Rust flags are unsupported, use target-specific configuration'
[[ -z ${RUSTC_BOOTSTRAP:-} ]] || fail 'RUSTC_BOOTSTRAP is not a substitute for the pinned managed compiler'
[[ -z ${CARGO_TARGET_DIR:-} && -z ${CARGO_BUILD_TARGET_DIR:-} ]] || fail 'custom target directories are unsupported by preview artifact discovery'
[[ -z ${JUSTRUST_DISABLE:-} || ${JUSTRUST_DISABLE} == 0 ]] || fail 'JUSTRUST_DISABLE conflicts with required hosted compilation'
for tool in git justrust readelf; do
  command -v "$tool" >/dev/null || fail "missing required command: $tool"
done
# Match the build-metadata parser's supported preview identity before paying for
# compilation. Invalid overrides otherwise fall back to the package version.
repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"
orbit_dev_bin=$(bash "$repo_root/scripts/orbit_dev.sh" --build-only)
"$orbit_dev_bin" validate-preview-version "$NEXODE_BUILD_SEMVER"

# .git is not mirrored to the tenant host. Snapshot real source metadata before
# dispatch, including dirty state, rather than embedding unknown or stale values.
NEXODE_BUILD_GIT_HASH=$(git rev-parse --short HEAD)
NEXODE_BUILD_GIT_DATE=$(git log -1 --format=%ci)
NEXODE_BUILD_GIT_TAG=$(git describe --tags --always)
NEXODE_BUILD_CHANGELOG_RAW=$(git log -700 --format='%h|%ct|%D|%s')
export NEXODE_BUILD_GIT_HASH NEXODE_BUILD_GIT_DATE NEXODE_BUILD_GIT_TAG NEXODE_BUILD_CHANGELOG_RAW
NEXODE_BUILD_GIT_DIRTY=false
git_status=$(git status --porcelain)
[[ -z $git_status ]] || NEXODE_BUILD_GIT_DIRTY=true
export NEXODE_BUILD_GIT_DIRTY
export NEXODE_RELEASE_BUILD=1 NEXODE_CI_BUILD=1
export CARGO_BUILD_RUSTC_WRAPPER=''
export CARGO_PROFILE_RELEASE_OPT_LEVEL=3 CARGO_INCREMENTAL=0
export JUSTRUST_REMOTE_REQUIRED=1 JUSTRUST_TOOLCHAIN_REQUIRED=1
export JUSTRUST_TOOLCHAIN=$toolchain
# An empty encoded global flag also overrides target flags in Cargo.
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS RUSTUP_TOOLCHAIN
if [[ $target == aarch64-unknown-linux-gnu ]]; then
  # Pinned justrust 5c8c6e653 runs real_cargo() metadata on the caller before
  # hosted dispatch. JUSTRUST_TOOLCHAIN selects the managed remote compiler,
  # not this rustup shim. On the native ARM CI caller, metadata probes the
  # host with ARM target flags, so stable rejects the target-only -Z TLS flag.
  # Install only the caller's minimal nightly, not target std
  # or rust-src, and scope selection to this script rather than rustup default.
  command -v rustup >/dev/null || fail 'ARM hosted preview metadata requires rustup'
  rustup toolchain install nightly-2026-10-08 --profile minimal --no-self-update
  export RUSTUP_TOOLCHAIN=nightly-2026-10-08
fi
# Select hosted explicitly, never a legacy account-owned AWS backend. Credentials
# come from NEXODE_API_KEY (CI) or the existing tenant login. No key provisioning,
# allowance changes, or billing exemptions occur here.
justrust remote use hosted
args=(--remote --locked --release --target "$target" -p nexode --bin nexode --features linux-compat-vendored-openssl)
if [[ $target == x86_64-unknown-linux-gnu ]]; then
  args=(--remote --locked --release --target "$target.2.17" -p nexode --bin nexode --features linux-compat-vendored-openssl)
  justrust zigbuild "${args[@]}"
else
  # Managed ARM setup validates/resolves Cargo target flags and injects target-
  # only emulated TLS. Pass it explicitly too, with matching nightly rust-src.
  export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS="${CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS:+${CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS} }-Ztls-model=emulated"
  justrust build "${args[@]}" -Zbuild-std=std,panic_abort
fi
artifact="target/$target/release/nexode"
[[ -x $artifact ]] || fail "hosted build did not return an executable: $artifact"

# The client validates downloaded artifacts too. Independently gate the exact
# expected file here, never glob a warm host's other release outputs. Do not run
# cross artifacts or confuse ELF/TLS inspection with native runtime acceptance.
"$orbit_dev_bin" verify-hosted-linux-abi "$artifact" "$target"
