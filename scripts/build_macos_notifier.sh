#!/bin/bash
# Build the Nexode macOS notification helper app bundle.
#
# The bundle is required because notifications posted by bare `osascript` are
# owned by Script Editor: clicking such a banner activates Script Editor rather
# than the terminal running the nexode session. Only an app that owns its own
# notifications can handle the click and focus the right window.
set -euo pipefail

APP_NAME="Nexode Notifier.app"
APP_DIR="${NEXODE_NOTIFIER_APP_DIR:-$HOME/Applications/$APP_NAME}"
BUNDLE_ID="com.nexode.notifier.helper"
SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/assets/macos-notifier"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "build_macos_notifier: macOS only" >&2
  exit 0
fi

rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS" "$APP_DIR/Contents/Resources"

cat > "$APP_DIR/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Nexode</string>
  <key>CFBundleDisplayName</key><string>Nexode</string>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleExecutable</key><string>nexode-notifier</string>
  <key>CFBundleIconFile</key><string>Nexode.icns</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>CFBundleShortVersionString</key><string>1</string>
  <key>LSUIElement</key><true/>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
</dict>
</plist>
PLIST

ICON="$SRC_DIR/../app-icons/Nexode.icns"
[[ -f "$ICON" ]] && cp "$ICON" "$APP_DIR/Contents/Resources/Nexode.icns"

swiftc -O -target "$(uname -m)-apple-macos11" \
  -o "$APP_DIR/Contents/MacOS/nexode-notifier" "$SRC_DIR/main.swift"

# Ad-hoc signing keeps the bundle identity stable so the user's notification
# permission grant survives rebuilds.
codesign --force --sign - "$APP_DIR" >/dev/null

LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
[[ -x "$LSREGISTER" ]] && "$LSREGISTER" -f "$APP_DIR"

echo "built $APP_DIR"
