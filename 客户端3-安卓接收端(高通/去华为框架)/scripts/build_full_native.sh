#!/usr/bin/env bash
# 编译通用包（com.generic.alvr）复用的完整 ALVR arm64 原生客户端。
set -euo pipefail
CLIENT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
ROOT="$(cd "$CLIENT_DIR/../.." && pwd)"
SRC="$ROOT/服务端-ALVR-完整源码"
NDK="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}"
for p in /home/user/ndk/android-ndk-r25c /opt/android-ndk-r25c "$ANDROID_SDK_ROOT/ndk/25.2.9519653"; do
  [ -d "$p" ] && NDK="$p" && break
done
[ -n "$NDK" ] || { echo "缺 Android NDK r25c"; exit 1; }
TC="$NDK/toolchains/llvm/prebuilt/linux-x86_64"
CC="$TC/bin/aarch64-linux-android26-clang"
CXX="$TC/bin/aarch64-linux-android26-clang++"
AR="$TC/bin/llvm-ar"
LOADER="$SRC/deps/android_openxr/arm64-v8a"
mkdir -p "$LOADER"
cp "$ROOT/sdk/hvrsdk-openxr-3.5.0.79/jni/arm64-v8a/libxr_loader.so" "$LOADER/libxr_loader.so"
cp "$LOADER/libxr_loader.so" "$LOADER/libopenxr_loader.so"
cd "$SRC"
env "CC_aarch64_linux_android=$CC" "CXX_aarch64_linux_android=$CXX" "AR_aarch64_linux_android=$AR" \
  "CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$CC" \
  "RUSTFLAGS=-L native=$LOADER" \
  cargo build -p alvr_client_openxr --target aarch64-linux-android --release
"$TC/bin/llvm-strip" --strip-unneeded "$SRC/target/aarch64-linux-android/release/libalvr_client_openxr.so"
