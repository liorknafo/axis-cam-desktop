#!/bin/bash
set -euo pipefail
project_dir="$(cd "$(dirname "$0")/.." && pwd)"
cd "$project_dir"
# The bundler relocates Homebrew dylibs on Apple Silicon; other layouts are untested.
if [ -z "${AXIS_PACKAGE_ALLOW_UNSUPPORTED:-}" ]; then
    if [ "$(uname -s)" != Darwin ] || [ "$(uname -m)" != arm64 ]; then
        echo "package-macos.sh builds an Apple Silicon (arm64 macOS) bundle; this host is $(uname -s) $(uname -m). Set AXIS_PACKAGE_ALLOW_UNSUPPORTED=1 to try anyway." >&2
        exit 1
    fi
    occt_prefix="${OCCT_PREFIX:-/opt/homebrew/opt/opencascade}"
    case "$occt_prefix" in
        /opt/homebrew/*) ;;
        *)
            echo "OCCT_PREFIX is $occt_prefix, not under /opt/homebrew; the bundler only relocates Homebrew libraries. Set AXIS_PACKAGE_ALLOW_UNSUPPORTED=1 to try anyway." >&2
            exit 1
            ;;
    esac
fi
# build.rs exports AXIS_BUILT_STEP_HELPER; cargo's JSON messages name the helper this build linked.
cargo_messages="$(mktemp -t axis-cam-cargo)"
cargo build --release --locked --message-format=json-render-diagnostics > "$cargo_messages"
build_dir="$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
output_app="${1:-$project_dir/dist/AXIS CAM Studio.app}"
case "$output_app" in
    *.app) ;;
    *) echo "Output path must end in .app (it is replaced): $output_app" >&2; exit 1 ;;
esac
# Stage outside file-provider folders: Finder metadata can race code signing.
stage_dir="$(mktemp -d -t axis-cam-package)"
output_tmp="$output_app.new.$$"
trap 'rm -rf "$stage_dir" "$output_tmp" "$cargo_messages"' EXIT
app_dir="$stage_dir/AXIS CAM Studio.app"
mkdir -p "$app_dir/Contents/MacOS" "$app_dir/Contents/Resources"
cp "$build_dir/release/axis-cam" "$app_dir/Contents/MacOS/axis-cam"
python3 scripts/bundle-step.py "$app_dir" "$cargo_messages"
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
# ditto merges into an existing bundle, which would keep stale files from older builds.
mkdir -p "$(dirname "$output_app")"
rm -rf "$output_tmp"
ditto "$app_dir" "$output_tmp"
rm -rf "$output_app"
mv "$output_tmp" "$output_app"
codesign --verify --deep --strict "$output_app"
printf 'App created: %s\n' "$output_app"
