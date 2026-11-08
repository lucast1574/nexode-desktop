#!/usr/bin/env bash
# Populate the platform npm packages from nexode release tarballs.
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage: $0 <release-assets-directory>" >&2
  exit 2
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
assets="$(cd "$1" && pwd)"

prepare() {
  local package="$1" archive="$2" archived_binary="$3" installed_binary="$4"
  local package_dir="$repo_root/sdk/npm/$package"
  rm -rf "$package_dir/bin"
  mkdir -p "$package_dir/bin"
  tar -xzf "$assets/$archive" -C "$package_dir/bin"
  mv "$package_dir/bin/$archived_binary" "$package_dir/bin/$installed_binary"
  chmod +x "$package_dir/bin/$installed_binary"
}

prepare linux-x64 nexode-linux-x86_64.tar.gz nexode-linux-x86_64 nexode
prepare linux-arm64 nexode-linux-aarch64.tar.gz nexode-linux-aarch64 nexode
prepare darwin-x64 nexode-macos-x86_64.tar.gz nexode-macos-x86_64 nexode
prepare darwin-arm64 nexode-macos-aarch64.tar.gz nexode-macos-aarch64 nexode
prepare win32-x64 nexode-windows-x86_64.tar.gz nexode-windows-x86_64.exe nexode.exe
prepare win32-arm64 nexode-windows-aarch64.tar.gz nexode-windows-aarch64.exe nexode.exe
