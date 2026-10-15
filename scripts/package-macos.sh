#!/bin/bash
set -euo pipefail
project_dir="$(cd "$(dirname "$0")/.." && pwd)"
cd "$project_dir"
cargo build --release --locked
build_dir="$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
output_app="${1:-$project_dir/dist/AXIS CAM Studio.app}"
# Stage outside file-provider folders: Finder metadata can race code signing.
stage_dir="$(mktemp -d -t axis-cam-package)"
trap 'rm -rf "$stage_dir"' EXIT
app_dir="$stage_dir/AXIS CAM Studio.app"
mkdir -p "$app_dir/Contents/MacOS" "$app_dir/Contents/Resources"
cp "$build_dir/release/axis-cam" "$app_dir/Contents/MacOS/axis-cam"
python3 scripts/bundle-step.py "$app_dir" "$build_dir"
cp assets/Info.plist "$app_dir/Contents/Info.plist"
cp assets/AppIcon.icns "$app_dir/Contents/Resources/AppIcon.icns"
mkdir -p "$app_dir/Contents/Resources/Licenses"
cp licenses/*.txt "$app_dir/Contents/Resources/Licenses/"
cp licenses/NOTICES.md "$app_dir/Contents/Resources/ThirdParty/NOTICES.md"
# Finder metadata in synced Documents folders can prevent local code signing.
# Remove only signing-incompatible metadata, preserving other attributes.
xattr -dr com.apple.FinderInfo "$app_dir" 2>/dev/null || true
xattr -dr com.apple.ResourceFork "$app_dir" 2>/dev/null || true
codesign --force --deep --sign - "$app_dir"
# File-provider/Finder metadata can be attached again during signing.
xattr -dr com.apple.FinderInfo "$app_dir" 2>/dev/null || true
xattr -dr com.apple.ResourceFork "$app_dir" 2>/dev/null || true
codesign --verify --deep --strict "$app_dir"
if [ -n "${AXIS_PACKAGE_ZIP:-}" ]; then
    ditto -c -k --sequesterRsrc --keepParent "$app_dir" "$AXIS_PACKAGE_ZIP"
fi
ditto "$app_dir" "$output_app"
printf 'App created: %s\n' "$output_app"
