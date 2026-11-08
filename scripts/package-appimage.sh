#!/usr/bin/env bash
# Package binaries built on the oldest supported distribution, not the packaging host.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
[[ "$(uname -s)" == Linux ]] || { echo 'AppImage packaging requires Linux.' >&2; exit 1; }
arch="$(uname -m)"
case "$arch" in
  x86_64) deploy_sha=c20cd71e3a4e3b80c3483cef793cda3f4e990aca14014d23c544ca3ce1270b4d; node_arch=x64; node_sha=48cc0e21e048790e54c0cf5749c2725005313d652651c40b7a21feb9697aaf75 ;;
  aarch64) deploy_sha=620095110d693282b8ebeb244a95b5e911cf8f65f76c88b4b47d16ae6346fcff; node_arch=arm64; node_sha=eb187837a2230d4047161ac0be008584e16b46f726e9c9490ee35dc6b17d9ba8 ;;
  *) echo "Unsupported AppImage architecture: $arch" >&2; exit 1 ;;
esac
desktop="$(realpath "${NEXODE_DESKTOP_BINARY:-target/release/nexode-desktop}")"
engine="$(realpath "${NEXODE_ENGINE_BINARY:-../nexode-orbit/target/release/nexode}")"
[[ -x "$desktop" && -x "$engine" ]] || { echo 'Build both native executables first.' >&2; exit 1; }
for tool in curl sha256sum readelf file pkg-config; do command -v "$tool" >/dev/null; done
pkg-config --exists gtk+-3.0 librsvg-2.0 gobject-introspection-1.0 ayatana-appindicator3-0.1
cache="$root/.local/appimage-tools/$arch"
mkdir -p "$cache" "$root/dist"

verified_download() {
  local url="$1" destination="$2" expected="$3"
  if [[ ! -f "$destination" ]] || ! printf '%s  %s\n' "$expected" "$destination" | sha256sum --check --status; then
    curl --fail --location --proto '=https' --tlsv1.2 --retry 3 "$url" --output "$destination.part"
    printf '%s  %s\n' "$expected" "$destination.part" | sha256sum --check --status
    mv "$destination.part" "$destination"
  fi
  chmod +x "$destination"
}

# Tagged release, not a mutable `continuous` download. Hashes from official release assets.
deploy="$cache/linuxdeploy-$arch.AppImage"
verified_download "https://github.com/linuxdeploy/linuxdeploy/releases/download/1-alpha-20251107-1/linuxdeploy-$arch.AppImage" "$deploy" "$deploy_sha"
verified_download "https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gtk/7a3fbc31a9e5075073ff8790f26effbac5f84453/linuxdeploy-plugin-gtk.sh" \
  "$cache/linuxdeploy-plugin-gtk.sh" b0f4cbc684a0103a9651f0955b635eaea0096b3a66c0f5a2c2aa337960375171

# Every run starts in a fresh directory. Never delete an existing bundle/source checkout.
stage="$(mktemp -d "$root/.local/appimage-stage.XXXXXXXX")"
appdir="$stage/Nexode_Orbit.AppDir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/share/applications" "$appdir/usr/share/icons/hicolor/256x256/apps" "$appdir/usr/share/doc/nexode-orbit"
install -m755 "$desktop" "$appdir/usr/bin/nexode-desktop"
install -m755 "$engine" "$appdir/usr/bin/nexode"
install -m644 assets/icon.png "$appdir/usr/share/icons/hicolor/256x256/apps/nexode-orbit.png"
install -m644 README.md APPIMAGE.md "$appdir/usr/share/doc/nexode-orbit/"

# The native UI launches this private MCP runtime directly. No global Node/npm install.
node_version=v24.22.2
mcp_revision=237d8d86e9f65906886e8b73d12a2afda2b2e6a4
node_archive="$cache/node-$node_version-linux-$node_arch.tar.xz"
verified_download "https://nodejs.org/dist/$node_version/node-$node_version-linux-$node_arch.tar.xz" "$node_archive" "$node_sha"
tar -xJf "$node_archive" -C "$stage"
node_root="$stage/node-$node_version-linux-$node_arch"
mcp_archive="$cache/nexode-mcp-$mcp_revision.tar.gz"
verified_download "https://codeload.github.com/lucast1574/nexode-mcp/tar.gz/$mcp_revision" "$mcp_archive" \
  8087cd448dbd8603f24abd2e1797de36008ac615bf09b4e3521a050705c36ec4
tar -xzf "$mcp_archive" -C "$stage"
mcp_source="$stage/nexode-mcp-$mcp_revision"
(
  cd "$mcp_source"
  export PATH="$node_root/bin:$PATH"
  # Dependency lifecycle scripts are unnecessary for this TypeScript MCP.
  npm ci --ignore-scripts --no-audit --no-fund
  npm run build
  npm prune --omit=dev --ignore-scripts --no-audit --no-fund
)
mkdir -p "$appdir/usr/bin/mcp"
install -m755 "$node_root/bin/node" "$appdir/usr/bin/mcp/node"
install -m644 "$node_root/LICENSE" "$appdir/usr/bin/mcp/NODE-LICENSE"
cp -a "$mcp_source/dist" "$mcp_source/node_modules" "$appdir/usr/bin/mcp/"
install -m644 "$mcp_source/package.json" "$mcp_source/package-lock.json" "$mcp_source/README.md" "$appdir/usr/bin/mcp/"
printf 'Nexode MCP v0.1.26\nSource: https://github.com/lucast1574/nexode-mcp/tree/%s\nNode.js %s\n' "$mcp_revision" "$node_version" > "$appdir/usr/bin/mcp/PROVENANCE.txt"
cat > "$appdir/usr/share/applications/app.nexode.orbit.desktop" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=Nexode Orbit
Comment=Native AI workspace for the Nexode platform
Exec=nexode-desktop
Icon=nexode-orbit
Terminal=false
Categories=Development;Utility;
StartupWMClass=app.nexode.orbit.desktop
DESKTOP

# libayatana is loaded with dlopen, so ELF dependency scanning cannot discover it.
ayatana="$(pkg-config --variable=libdir ayatana-appindicator3-0.1)/libayatana-appindicator3.so.1"
[[ -f "$ayatana" ]] || { echo 'Ayatana indicator library not found.' >&2; exit 1; }
export APPIMAGE_EXTRACT_AND_RUN=1 DEPLOY_GTK_VERSION=3
export PATH="$cache:$PATH"
"$deploy" --appdir "$appdir" \
  --executable "$appdir/usr/bin/nexode-desktop" --executable "$appdir/usr/bin/nexode" \
  --deploy-deps-only "$appdir/usr/bin/mcp/node" \
  --library "$ayatana" --plugin gtk \
  --desktop-file "$appdir/usr/share/applications/app.nexode.orbit.desktop" \
  --icon-file "$appdir/usr/share/icons/hicolor/256x256/apps/nexode-orbit.png"

# Fail closed if any bundled ELF requires newer glibc than the documented baseline.
# Graphics drivers and glibc itself must remain supplied by the host OS.
max_glibc="${APPIMAGE_MAX_GLIBC:-2.35}"
required="$(find "$appdir" -type f -print0 | while IFS= read -r -d '' item; do
  readelf --version-info "$item" 2>/dev/null | sed -nE 's/.*Name: GLIBC_([0-9]+\.[0-9]+).*/\1/p' || true
done | sort -Vu | tail -n1)"
[[ -n "$required" ]] || { echo 'Could not determine the ELF glibc requirement.' >&2; exit 1; }
newest="$(printf '%s\n%s\n' "$required" "$max_glibc" | sort -V | tail -n1)"
[[ "$newest" == "$max_glibc" ]] || { echo "Bundle needs glibc $required; build on Ubuntu 22.04 or older (maximum $max_glibc)." >&2; exit 1; }

version="$(sed -nE 's/^version = "([^"]+)"/\1/p' Cargo.toml | head -n1)"
export VERSION="$version" ARCH="$arch"
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct 2>/dev/null || date +%s)}"
export OUTPUT="$root/dist/Nexode-Orbit-$version-$arch.AppImage"
"$deploy" --appdir "$appdir" --output appimage
chmod +x "$OUTPUT"
printf 'UI SHA256: %s\nEngine SHA256: %s\nRequired glibc: %s\nArchitecture: %s\nlinuxdeploy: %s\nGTK plugin: %s\nNode: %s\nMCP revision: %s\n' \
  "$(sha256sum "$desktop" | cut -d' ' -f1)" "$(sha256sum "$engine" | cut -d' ' -f1)" "$required" "$arch" \
  '1-alpha-20251107-1' '7a3fbc31a9e5075073ff8790f26effbac5f84453' "$node_version" "$mcp_revision" > "$OUTPUT.build-info.txt"
(cd "$root/dist" && sha256sum "$(basename "$OUTPUT")" > "$(basename "$OUTPUT").sha256")
echo "AppImage: $OUTPUT (glibc >= $required; AppDir retained at $appdir)"
