#!/usr/bin/env bash
# Build a release Hi-Fi.app bundle (macOS) into build/.
# Run from anywhere:  ./scripts/bundle.sh
set -euo pipefail

cd "$(dirname "$0")/.."
cargo build --release --workspace

APP=build/Hi-Fi.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/hifi-app "$APP/Contents/MacOS/Hi-Fi"
cp target/release/hifi "$APP/Contents/MacOS/hifi"
# Cargo's own strip step can fail on some toolchains; strip locally too.
strip -x "$APP/Contents/MacOS/Hi-Fi" "$APP/Contents/MacOS/hifi"
cp Resources/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Hi-Fi</string>
  <key>CFBundleDisplayName</key><string>Hi-Fi</string>
  <key>CFBundleIdentifier</key><string>com.hifi.browser</string>
  <key>CFBundleExecutable</key><string>Hi-Fi</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
</dict>
</plist>
PLIST

codesign --force --deep --sign - "$APP" >/dev/null 2>&1 || true
du -sh "$APP"
