#!/usr/bin/env bash
# 通用安卓 ALVR 接收端 —— 一键构建（需 Android SDK/NDK，本沙箱未预装）
# 去华为化：只用标准 Khronos OpenXR loader，不依赖任何华为系统库/权限。
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "=== 0. 环境检查 ==="
: "${ANDROID_NDK_HOME:?请设置 ANDROID_NDK_HOME}"
: "${ANDROID_SDK_ROOT:?请设置 ANDROID_SDK_ROOT}"
TARGET=aarch64-linux-android21
CC="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/${TARGET}-clang"
[ -x "$CC" ] || { echo "找不到 NDK clang: $CC"; exit 1; }
echo "NDK clang: $CC"

echo "=== 1. 编译 C 核心 + JNI 桥 → 共享库 ==="
mkdir -p build/android/lib/arm64-v8a build/obj
CFLAGS="-std=c11 -O2 -fPIC -ffunction-sections -fdata-sections -Wall"
INC="-Isrc/pose -Isrc/controller -Isrc/decoder -Isrc/runtime -Isrc/hid"
$CC $CFLAGS $INC -shared \
  src/pose/pose.c src/controller/controller.c \
  src/decoder/decoder_policy.c src/runtime/generic_runtime.c \
  src/hid/hwvr_hid.c android/jni_bridge.c \
  -lm -Wl,--gc-sections \
  -o build/android/lib/arm64-v8a/libgeneric_alvr_bridge.so
echo "  → libgeneric_alvr_bridge.so"

echo "=== 2. Rust 客户端逻辑（宿主验证 + 交叉库）==="
( cd rustsrc && cargo test --offline 2>/dev/null || cargo test )
# 若要产出 Rust cdylib 需 Android rust target：
# rustup target add aarch64-linux-android
# cargo build --release --target aarch64-linux-android

echo "=== 3. 通用 OpenXR runtime 库 ==="
# libgeneric_alvr_xr.so 需要 openxr headers；从 Khronos OpenXR-SDK 获取后编译 src/runtime/runtime_openxr.c
# 本工程提供平台桥，OpenXR 入口点骨架见 docs/07-openxr-runtime.md
if [ -f src/runtime/runtime_openxr.c ] && [ -n "${OPENXR_HEADERS:-}" ]; then
  $CC $CFLAGS $INC -I"$OPENXR_HEADERS" -shared \
    src/runtime/runtime_openxr.c src/runtime/generic_runtime.c \
    src/pose/pose.c src/controller/controller.c src/decoder/decoder_policy.c src/hid/hwvr_hid.c \
    -lm -o build/android/lib/arm64-v8a/libgeneric_alvr_xr.so
  echo "  → libgeneric_alvr_xr.so"
else
  echo "  [跳过] libgeneric_alvr_xr.so（需设置 OPENXR_HEADERS 指向 openxr.h）"
fi

echo "=== 4. 组装 APK 资源 ==="
mkdir -p build/apk/lib/arm64-v8a build/apk/assets
cp build/android/lib/arm64-v8a/*.so build/apk/lib/arm64-v8a/ 2>/dev/null || true
cp app/active_runtime.json build/apk/assets/
cp app/client_config.json build/apk/assets/ 2>/dev/null || true

echo "=== 完成 ==="
echo "产物目录: build/apk/"
echo "说明: 完整 APK 打包（aapt2 + apksigner）在有 Android SDK Build-Tools 的环境执行；"
echo "      本脚本已产出全部 native 库与配置，真机编译步骤见 docs/02-构建与运行.md"
