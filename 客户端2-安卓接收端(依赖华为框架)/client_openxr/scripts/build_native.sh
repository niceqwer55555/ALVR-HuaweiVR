#!/usr/bin/env bash
# 编译客户端② ARMv7 原生库；可单独重跑。
set -euo pipefail
CLIENT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
ROOT="$(cd "$CLIENT_DIR/../.." && pwd)"
SRC="$ROOT/服务端-ALVR-完整源码"
cd "$SRC"

NDK="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}"
if [ -z "$NDK" ]; then
  for p in /home/user/ndk/android-ndk-r25c /opt/android-ndk-r25c /opt/android-ndk; do
    [ -d "$p" ] && NDK="$p" && break
  done
fi
[ -n "$NDK" ] || { echo "缺 Android NDK r25c：设置 ANDROID_NDK_HOME"; exit 1; }
TC="$NDK/toolchains/llvm/prebuilt/linux-x86_64"
[ -d "$TC" ] || TC="$NDK/toolchains/llvm/prebuilt/windows-x86_64"
CC="$TC/bin/armv7a-linux-androideabi26-clang"
CXX="$TC/bin/armv7a-linux-androideabi26-clang++"
AR="$TC/bin/llvm-ar"
[ -x "$CC" ] || CC="$TC/bin/armv7a-linux-androideabi24-clang"
[ -x "$CXX" ] || CXX="$TC/bin/armv7a-linux-androideabi24-clang++"

LOADER_DIR="$SRC/deps/android_openxr/armeabi-v7a"
mkdir -p "$LOADER_DIR"
cp "$ROOT/sdk/hvrsdk-openxr-3.5.0.79/jni/armeabi-v7a/libxr_loader.so" "$LOADER_DIR/libxr_loader.so"
cp "$LOADER_DIR/libxr_loader.so" "$LOADER_DIR/libopenxr_loader.so"

export CC="$CC" CXX="$CXX" AR="$AR"
export CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_LINKER="$CC"
export RUSTFLAGS="-L native=$LOADER_DIR"
env "CC_armv7-linux-androideabi=$CC" "CXX_armv7-linux-androideabi=$CXX" "AR_armv7-linux-androideabi=$AR" \
  cargo build -p alvr_client_openxr --target armv7-linux-androideabi --release

# release 产物仍带 debuginfo；不进 APK 符号表，直接裁掉（71MB -> ~6MB）
"$TC/bin/llvm-strip" --strip-unneeded "$SRC/target/armv7-linux-androideabi/release/libalvr_client_openxr.so"

# --- arm64-v8a（现代华为机型 / 通用 arm64 设备）---
LOADER64="$SRC/deps/android_openxr/arm64-v8a"
mkdir -p "$LOADER64"
cp "$ROOT/sdk/hvrsdk-openxr-3.5.0.79/jni/arm64-v8a/libxr_loader.so" "$LOADER64/libxr_loader.so"
cp "$LOADER64/libxr_loader.so" "$LOADER64/libopenxr_loader.so"
CC64="$TC/bin/aarch64-linux-android26-clang"
CXX64="$TC/bin/aarch64-linux-android26-clang++"
AR64="$TC/bin/llvm-ar"
env "CC_aarch64_linux_android=$CC64" "CXX_aarch64_linux_android=$CXX64" "AR_aarch64_linux_android=$AR64" \
  "CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$CC64" \
  "RUSTFLAGS=-L native=$LOADER64" \
  cargo build -p alvr_client_openxr --target aarch64-linux-android --release
"$TC/bin/llvm-strip" --strip-unneeded "$SRC/target/aarch64-linux-android/release/libalvr_client_openxr.so"
