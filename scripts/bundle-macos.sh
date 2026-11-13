#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")/.."
if [[ "$(uname -s)" != Darwin ]]; then
    echo "This script creates a macOS app bundle and must run on macOS." >&2
    exit 1
fi

profile=release
case "${1:-}" in
    --debug) profile=debug; cargo build --locked ;;
    "") cargo build --release --locked ;;
    *) echo "Usage: scripts/bundle-macos.sh [--debug]" >&2; exit 1 ;;
esac

app_path="$PWD/dist/Find Dev Data.app"
mkdir -p "$app_path/Contents/MacOS" "$app_path/Contents/Resources" target/AppIcon.iconset
cp "target/$profile/find-dev-data" "$app_path/Contents/MacOS/find-dev-data"
cp resources/Info.plist "$app_path/Contents/Info.plist"

# Draw the icon with AppKit. Keep Swift's compiler cache inside the project.
swift -module-cache-path "$PWD/target/swift-module-cache" scripts/make-icon.swift "$PWD/target/AppIcon.iconset" "$app_path/Contents/Resources/AppIcon.icns"
codesign --force --sign - "$app_path"
echo "Built: $app_path"
echo "Launch: open \"$app_path\""

