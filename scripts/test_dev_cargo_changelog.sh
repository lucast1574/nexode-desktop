#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Load only the function under test, without starting a build.
eval "$(awk '/^export_git_build_metadata\(\) \{/{copy=1} copy {print} copy && /^\}/{exit}' "$repo_root/scripts/dev_cargo.sh")"
unset NEXODE_BUILD_CHANGELOG_RAW NEXODE_BUILD_GIT_HASH
export_git_build_metadata
expected="$(git -C "$repo_root" log -700 --format='%h|%ct|%D|%s')"
[[ -n "$NEXODE_BUILD_CHANGELOG_RAW" ]]
[[ "$NEXODE_BUILD_CHANGELOG_RAW" == "$expected" ]]
# History is still populated when a caller already supplied the identity.
unset NEXODE_BUILD_CHANGELOG_RAW
NEXODE_BUILD_GIT_HASH=caller-hash
export_git_build_metadata
[[ "$NEXODE_BUILD_CHANGELOG_RAW" == "$expected" ]]
[[ "$NEXODE_BUILD_GIT_HASH" == caller-hash ]]
# Explicit caller history, including empty history, must not be overwritten.
NEXODE_BUILD_CHANGELOG_RAW=caller-history
export_git_build_metadata
[[ "$NEXODE_BUILD_CHANGELOG_RAW" == caller-history ]]
NEXODE_BUILD_CHANGELOG_RAW=''
export_git_build_metadata
[[ -z "$NEXODE_BUILD_CHANGELOG_RAW" ]]
echo 'dev build changelog metadata tests passed'
