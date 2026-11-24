#!/usr/bin/env bash
# 通用安卓 ALVR 接收端 —— 完整 APK 打包（arm64-v8a / Android 11+）
# 依赖：Android NDK r25c + build-tools 34 + platform android-34（本机 /opt）
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

NDK=/opt/android-ndk-r25c
SDK=/opt/android-sdk
BT="$SDK/build-tools/34/android-14"
PLATFORM="$SDK/platforms/android-34/android-34"
CC="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android21-clang"
JAVA_HOME_BIN="$(dirname "$(readlink -f "$(command -v javac)")")"

echo "=== 0. 环境 ==="
[ -x "$CC" ] || { echo "缺 NDK clang: $CC"; exit 1; }
[ -f "$BT/aapt2" ] || { echo "缺 aapt2: $BT/aapt2"; exit 1; }
[ -f "$PLATFORM/android.jar" ] || { echo "缺 android.jar: $PLATFORM/android.jar"; exit 1; }
echo "NDK: $NDK"; echo "build-tools: $BT"; echo "platform: $PLATFORM"

rm -rf build/apkwork && mkdir -p build/apkwork/{lib/arm64-v8a,assets,classes,gen,unsigned}
mkdir -p build/android/lib/arm64-v8a

echo "=== 1. 编译 native 核心 + JNI 桥 ==="
CFLAGS="-std=c11 -O2 -fPIC -ffunction-sections -fdata-sections -Wall"
INC="-Isrc/pose -Isrc/controller -Isrc/decoder -Isrc/runtime -Isrc/hid"
$CC $CFLAGS $INC -shared \
  src/pose/pose.c src/controller/controller.c \
  src/decoder/decoder_policy.c src/runtime/generic_runtime.c \
  src/hid/hwvr_hid.c android/jni_bridge.c \
  -lm -Wl,--gc-sections \
  -o build/apkwork/lib/arm64-v8a/libgeneric_alvr_bridge.so

echo "=== 2. OpenXR runtime 库（headers 可用时）==="
if [ -f src/runtime/runtime_openxr.c ] && [ -d /opt/openxr/include ]; then
  $CC $CFLAGS $INC -I/opt/openxr/include -shared \
    src/runtime/runtime_openxr.c src/runtime/generic_runtime.c \
    src/pose/pose.c src/controller/controller.c src/decoder/decoder_policy.c src/hid/hwvr_hid.c \
    -lm -o build/apkwork/lib/arm64-v8a/libgeneric_alvr_xr.so
  echo "  -> libgeneric_alvr_xr.so"
else
  echo "  [跳过] 无 runtime_openxr.c 源码（平台桥已编入 bridge 库）"
fi

cp app/active_runtime.json build/apkwork/assets/
[ -f app/client_config.json ] && cp app/client_config.json build/apkwork/assets/

echo "=== 3. aapt2 生成基础 APK（manifest，无资源）==="
"$BT/aapt2" link -o build/apkwork/base.apk \
  -I "$PLATFORM/android.jar" \
  --manifest android/AndroidManifest.xml \
  --java build/apkwork/gen \
  --min-sdk-version 24 --target-sdk-version 34

echo "=== 4. javac 编译 Java host ==="
mkdir -p build/apkwork/gen_src
javac -encoding UTF-8 -cp "$PLATFORM/android.jar" -d build/apkwork/classes \
  android/java/com/generic/alvr/*.java

echo "=== 5. d8 生成 classes.dex ==="
mkdir -p build/apkwork/dex_out
"$BT/d8" --output build/apkwork/dex_out $(find build/apkwork/classes -name "*.class")
cp build/apkwork/dex_out/classes.dex build/apkwork/

echo "=== 6. 合并资源 + native 库 + assets + dex ==="
cp build/apkwork/base.apk build/apkwork/final.apk
( cd build/apkwork && zip -q -r final.apk classes.dex lib assets )

echo "=== 7. 对齐 + 签名 ==="
"$BT/zipalign" -f -p 4 build/apkwork/final.apk build/apkwork/aligned.apk
if [ ! -f build/debug.keystore ]; then
  keytool -genkeypair -v -keystore build/debug.keystore \
    -alias androiddebugkey -keyalg RSA -keysize 2048 -storepass android -keypass android \
    -dname "CN=Generic ALVR, OU=Dev, O=Generic, L=NA, ST=NA, C=CN" \
    -validity 10000 2>/dev/null
fi
"$BT/apksigner" sign --ks build/debug.keystore \
  --ks-pass pass:android --key-pass pass:android \
  --out build/GenericALVR-arm64.apk build/apkwork/aligned.apk

echo "=== 8. 验证 ==="
"$BT/apksigner" verify -v build/GenericALVR-arm64.apk || true
"$BT/aapt2" dump badging build/GenericALVR-arm64.apk 2>/dev/null | head -5 || true
ls -lh build/GenericALVR-arm64.apk
echo "完成: build/GenericALVR-arm64.apk"
