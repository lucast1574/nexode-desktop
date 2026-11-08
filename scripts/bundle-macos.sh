#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
[[ "$(uname -s)" == Darwin ]] || { echo 'Ejecuta este empaquetado en macOS.' >&2; exit 1; }
cargo build --locked --release
bundle="dist/Nexode Orbit.app/Contents"
mkdir -p "$bundle/MacOS" "$bundle/Resources"
cp target/release/nexode-desktop "$bundle/MacOS/"
cp assets/icon.icns "$bundle/Resources/Orbit.icns"
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
