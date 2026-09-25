#!/bin/bash
# bundle.sh — build Hi-Fi.app from the SwiftPM binaries.
# Usage: scripts/bundle.sh [release]
set -euo pipefail
cd "$(dirname "$0")/.."

CONFIG=debug
[ "${1:-}" = "release" ] && CONFIG=release

swift build -c "$CONFIG" --product HiFiApp
swift build -c "$CONFIG" --product hifi

BIN_DIR=".build/$CONFIG"
APP="build/Hi-Fi.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp "$BIN_DIR/HiFiApp" "$APP/Contents/MacOS/HiFiApp"
cp "$BIN_DIR/hifi" "$APP/Contents/MacOS/hifi"

cat > "$APP/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Hi-Fi</string>
  <key>CFBundleDisplayName</key><string>Hi-Fi</string>
  <key>CFBundleIdentifier</key><string>com.hifi.browser</string>
  <key>CFBundleExecutable</key><string>HiFiApp</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticTermination</key><false/>
  <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
</dict>
</plist>
PLIST

if [ -f Resources/AppIcon.icns ]; then
  cp Resources/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
fi

# Solar icon set (Linear, CC BY 4.0 — 480 Design), same set as Cosmos.
if [ -d Resources/icons ]; then
  mkdir -p "$APP/Contents/Resources/icons"
  cp Resources/icons/*.svg "$APP/Contents/Resources/icons/"
fi

# ad-hoc sign so Gatekeeper lets it launch locally
codesign --force --deep --sign - "$APP" 2>/dev/null || true

echo "built: $APP"
echo "run:   open $APP"
