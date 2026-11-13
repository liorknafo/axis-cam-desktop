# The Android tools this build uses (LIO-452), all on D: (C: is short of
# space). Source it: `. android/env.sh`
export ANDROID_HOME=D:/android/sdk
export ANDROID_NDK_HOME=D:/android/sdk/ndk/27.2.12479018
export JAVA_HOME="D:/android/jdk-17.0.21+9"
export PATH="$JAVA_HOME/bin:/d/android/gradle-8.14.3/bin:/d/android/sdk/platform-tools:/d/android/sdk/emulator:$PATH"
export CARGO_TARGET_DIR=D:/android/target
# The Pixel Tablet emulator "shop": `emulator -avd shop`
export ANDROID_AVD_HOME=D:/android/avd
# No Open CASCADE on Android: no STEP import on the tablet.
export AXIS_SKIP_STEP=1
