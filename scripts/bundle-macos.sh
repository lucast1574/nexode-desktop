#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
[[ "$(uname -s)" == Darwin ]] || { echo 'Ejecuta este empaquetado en macOS.' >&2; exit 1; }
cargo build --locked --release
cargo build --manifest-path ../nexode-orbit/Cargo.toml --locked --release --no-default-features --bin nexode
bundle="dist/Nexode Orbit.app/Contents"
mkdir -p "$bundle/MacOS" "$bundle/Resources"
cp target/release/nexode-desktop "$bundle/MacOS/"
cp ../nexode-orbit/target/release/nexode "$bundle/MacOS/"
cp assets/icon.icns "$bundle/Resources/Orbit.icns"
cache=".local/macos-bundle"
mkdir -p "$cache" "$bundle/MacOS/mcp"
case "$(uname -m)" in
  arm64) arch=arm64; checksum=1f5ac075be50bed4780ea3be19a007fda95ea4d527d28d76d0b91de47d915115 ;;
  x86_64) arch=x64; checksum=6965578fd823e72c1b5387f5ab84cb25368f886e02eb8779e6f0fce6b91c27b7 ;;
  *) echo 'Arquitectura macOS no soportada' >&2; exit 1 ;;
esac
node_archive="node-v24.22.2-darwin-$arch.tar.xz"
curl --fail --location --proto '=https' "https://nodejs.org/dist/v24.22.2/$node_archive" -o "$cache/$node_archive"
echo "$checksum  $cache/$node_archive" | shasum -a 256 -c -
tar -xJf "$cache/$node_archive" -C "$cache"
export PATH="$PWD/$cache/node-v24.22.2-darwin-$arch/bin:$PATH"
curl --fail --location --proto '=https' https://codeload.github.com/lucast1574/nexode-mcp/tar.gz/v0.1.26 -o "$cache/mcp.tar.gz"
echo "8087cd448dbd8603f24abd2e1797de36008ac615bf09b4e3521a050705c36ec4  $cache/mcp.tar.gz" | shasum -a 256 -c -
tar -xzf "$cache/mcp.tar.gz" -C "$cache"
mcp_source="$cache/nexode-mcp-0.1.26"
npm --prefix "$mcp_source" ci
npm --prefix "$mcp_source" run build
npm --prefix "$mcp_source" prune --omit=dev --ignore-scripts
cp "$cache/node-v24.22.2-darwin-$arch/bin/node" "$bundle/MacOS/mcp/node"
cp "$cache/node-v24.22.2-darwin-$arch/LICENSE" "$bundle/MacOS/mcp/NODE-LICENSE"
cp -R "$mcp_source/dist" "$mcp_source/node_modules" "$bundle/MacOS/mcp/"
cp "$mcp_source/package.json" "$bundle/MacOS/mcp/"
cat > "$bundle/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>nexode-desktop</string>
<key>CFBundleIdentifier</key><string>app.nexode.orbit.desktop</string>
<key>CFBundleName</key><string>Nexode Orbit</string>
<key>CFBundleVersion</key><string>0.2.0</string>
<key>CFBundleShortVersionString</key><string>0.2.0</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleIconFile</key><string>Orbit</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
echo "Creado: dist/Nexode Orbit.app (sin firma ni notarización)"
ditto -c -k --sequesterRsrc --keepParent "dist/Nexode Orbit.app" "dist/Nexode-Orbit-macOS-$arch.zip"
