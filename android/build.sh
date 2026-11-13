#!/usr/bin/env bash
# Builds the shop tablet APK (LIO-452).
#   android/build.sh                 the tablet (arm64), release
#   android/build.sh debug x86_64    the emulator
# The APK ends up in android/app/build/outputs/apk/<profile>/.
set -euo pipefail
cd "$(dirname "$0")"
. ./env.sh

profile=${1:-release}
abi=${2:-arm64-v8a}
cargo_profile=()
[ "$profile" = release ] && cargo_profile=(--release)

rm -rf app/src/main/jniLibs
# --link-libcxx-shared: clipper2 is C++, and its runtime must ship in the APK.
cargo ndk --platform 26 -t "$abi" --link-libcxx-shared -o app/src/main/jniLibs build --lib -j 2 "${cargo_profile[@]}"

task=assembleRelease
[ "$profile" = debug ] && task=assembleDebug
gradle --quiet "$task"
ls app/build/outputs/apk/"$profile"/*.apk
