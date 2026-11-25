#!/usr/bin/env bash
# 通用安卓 ALVR 接收端 —— 可复现 APK 打包（arm64-v8a / Android 7+）
# 自动探测 NDK / Build-Tools / android.jar；无完整 JDK 时给出明确错误。
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

find_one() {
  local kind="$1"; shift
  for p in "$@"; do
    if [ -n "$p" ] && [ -e "$p" ]; then echo "$p"; return 0; fi
  done
  return 1
}

NDK="$(find_one ndk \
  "${ANDROID_NDK_HOME:-}" \
  "${ANDROID_NDK_ROOT:-}" \
  /home/user/ndk/android-ndk-r25c \
  /opt/android-ndk-r25c \
  /opt/android-ndk)" || { echo "缺 Android NDK r25c：设置 ANDROID_NDK_HOME"; exit 1; }

BT="$(find_one build-tools \
  "${ANDROID_BUILD_TOOLS:-}" \
  /home/user/bt/android-14 \
  /opt/android-sdk/build-tools/34/android-14 \
  /opt/android-sdk/build-tools/34.0.0 \
  "${ANDROID_SDK_ROOT:-}/build-tools/34.0.0" \
  "${ANDROID_SDK_ROOT:-}/build-tools/34/android-14")" || {
  echo "缺 Android build-tools 34（aapt2/d8/zipalign/apksigner）"; exit 1; }

PLATFORM="$(find_one platform \
  "${ANDROID_PLATFORM:-}" \
  /home/user/platform/android-34 \
  /opt/android-sdk/platforms/android-34/android-34 \
  /opt/android-sdk/platforms/android-34 \
  "${ANDROID_SDK_ROOT:-}/platforms/android-34")" || {
  echo "缺 android-34 platform（android.jar）"; exit 1; }

CC="$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android21-clang"
[ -x "$CC" ] || { echo "缺 NDK clang: $CC"; exit 1; }
[ -f "$BT/aapt2" ] || { echo "缺 aapt2: $BT/aapt2"; exit 1; }
[ -f "$BT/d8" ] || { echo "缺 d8: $BT/d8"; exit 1; }
[ -f "$PLATFORM/android.jar" ] || { echo "缺 android.jar: $PLATFORM/android.jar"; exit 1; }

JAVAC="${JAVAC:-$(command -v javac || true)}"
if [ -z "$JAVAC" ] && [ -x "${JAVA_HOME:-}/bin/javac" ]; then JAVAC="$JAVA_HOME/bin/javac"; fi
if [ -z "$JAVAC" ]; then
  echo "缺 javac。安装 OpenJDK 17 后重试：sudo apt-get install -y openjdk-17-jdk-headless"
  exit 1
fi

echo "NDK: $NDK"
echo "build-tools: $BT"
echo "platform: $PLATFORM"
echo "javac: $JAVAC"

rm -rf build/apkwork && mkdir -p build/apkwork/{lib/arm64-v8a,assets,classes,gen,unsigned,dex_out}
mkdir -p build/android/lib/arm64-v8a

echo "=== 1. 编译 native 核心 + JNI 桥 ==="
CFLAGS="-std=c11 -O2 -fPIC -ffunction-sections -fdata-sections -Wall"
INC="-Isrc/pose -Isrc/controller -Isrc/decoder -Isrc/runtime -Isrc/hid"
"$CC" $CFLAGS $INC -shared \
  src/pose/pose.c src/controller/controller.c \
  src/decoder/decoder_policy.c src/runtime/generic_runtime.c \
  src/hid/hwvr_hid.c android/jni_bridge.c \
  -lm -Wl,--gc-sections \
  -o build/apkwork/lib/arm64-v8a/libgeneric_alvr_bridge.so
cp build/apkwork/lib/arm64-v8a/libgeneric_alvr_bridge.so build/android/lib/arm64-v8a/

# 完整 ALVR 客户端原生库（arm64），没有它 APK 只剩诊断壳
FULL_SO="${ALVR_FULL_SO:-$ROOT/../../服务端-ALVR-完整源码/target/aarch64-linux-android/release/libalvr_client_openxr.so}"
if [ -f "$FULL_SO" ]; then
  cp "$FULL_SO" build/apkwork/lib/arm64-v8a/libalvr_client_openxr.so
  echo "  -> libalvr_client_openxr.so $(du -h "$FULL_SO" | cut -f1)"
else
  echo "  [警告] 未找到 $FULL_SO，将不会打包完整客户端原生库"
fi

# 华为 OpenXR loader（arm64），由 ALVR 原生客户端链接
LOADER="${ALVR_OPENXR_LOADER:-$ROOT/../../sdk/hvrsdk-openxr-3.5.0.79/jni/arm64-v8a/libxr_loader.so}"
if [ -f "$LOADER" ]; then
  cp "$LOADER" build/apkwork/lib/arm64-v8a/libxr_loader.so
  echo "  -> libxr_loader.so"
fi

echo "=== 2. 可选 OpenXR runtime 库 ==="
if [ -f src/runtime/runtime_openxr.c ] && [ -n "${OPENXR_HEADERS:-}" ]; then
  "$CC" $CFLAGS $INC -I"$OPENXR_HEADERS" -shared \
    src/runtime/runtime_openxr.c src/runtime/generic_runtime.c \
    src/pose/pose.c src/controller/controller.c src/decoder/decoder_policy.c src/hid/hwvr_hid.c \
    -lm -o build/apkwork/lib/arm64-v8a/libgeneric_alvr_xr.so
  echo "  -> libgeneric_alvr_xr.so"
else
  echo "  [跳过] libgeneric_alvr_xr.so（无 runtime_openxr.c 或未设置 OPENXR_HEADERS）"
fi

cp app/active_runtime.json build/apkwork/assets/
[ -f app/client_config.json ] && cp app/client_config.json build/apkwork/assets/

echo "=== 3. aapt2 生成基础 APK ==="
"$BT/aapt2" link -o build/apkwork/base.apk \
  -I "$PLATFORM/android.jar" \
  --manifest android/AndroidManifest.xml \
  --java build/apkwork/gen \
  --min-sdk-version 26 --target-sdk-version 34

echo "=== 4. javac 编译 Java host ==="
"$JAVAC" -encoding UTF-8 -source 8 -target 8 \
  -cp "$PLATFORM/android.jar" -d build/apkwork/classes \
  android/java/com/generic/alvr/*.java

echo "=== 5. d8 生成 classes.dex ==="
"$BT/d8" --lib "$PLATFORM/android.jar" --output build/apkwork/dex_out \
  $(find build/apkwork/classes -name "*.class")
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
"$BT/apksigner" verify -v build/GenericALVR-arm64.apk
"$BT/aapt2" dump badging build/GenericALVR-arm64.apk 2>/dev/null | head -8 || true
ls -lh build/GenericALVR-arm64.apk
echo "完成: build/GenericALVR-arm64.apk"
