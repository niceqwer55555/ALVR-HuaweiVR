#!/usr/bin/env bash
# 客户端②（依赖华为框架）Linux/GitHub Actions 打包脚本
# 产物：build/HuaweiVRGlass-armv7.apk
set -euo pipefail

CLIENT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
ROOT="$(cd "$CLIENT_DIR/../.." && pwd)"
cd "$CLIENT_DIR"

need() { command -v "$1" >/dev/null 2>&1 || { echo "缺少命令：$1"; exit 1; }; }
need unzip
need javac
need keytool
need zip

NDK="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}"
if [ -z "$NDK" ]; then
  for p in /home/user/ndk/android-ndk-r25c /opt/android-ndk-r25c /opt/android-ndk; do
    [ -d "$p" ] && NDK="$p" && break
  done
fi
[ -n "$NDK" ] || { echo "缺 Android NDK r25c：设置 ANDROID_NDK_HOME"; exit 1; }
TC="$NDK/toolchains/llvm/prebuilt/linux-x86_64"
[ -d "$TC" ] || TC="$NDK/toolchains/llvm/prebuilt/windows-x86_64"

SDK="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-}}"
if [ -z "$SDK" ]; then
  for p in /opt/android-sdk /home/user/android-sdk /home/user; do
    [ -d "$p/build-tools" ] && SDK="$p" && break
  done
fi
[ -n "$SDK" ] || { echo "缺 Android SDK：设置 ANDROID_SDK_ROOT"; exit 1; }
BT="${ANDROID_BUILD_TOOLS:-}"
if [ -z "$BT" ]; then
  for p in "$SDK/build-tools/34.0.0" /home/user/bt/android-14 /opt/android-sdk/build-tools/34.0.0; do
    [ -d "$p" ] && BT="$p" && break
  done
fi
PLATFORM="${ANDROID_PLATFORM:-$SDK/platforms/android-34}"
[ -f "$PLATFORM/android.jar" ] || PLATFORM=/home/user/platform/android-34
[ -f "$PLATFORM/android.jar" ] || PLATFORM="$SDK/platforms/android-33"
[ -f "$PLATFORM/android.jar" ] || { echo "缺 android-34/android.jar"; exit 1; }
AAPT2="$BT/aapt2"; D8="$BT/d8"; ZIPALIGN="$BT/zipalign"; APKSIGNER="$BT/apksigner"
for f in "$AAPT2" "$D8" "$ZIPALIGN" "$APKSIGNER"; do [ -x "$f" ] || { echo "缺 $f"; exit 1; }; done

SDK_HW="$ROOT/sdk/hvrsdk-openxr-3.5.0.79"
SDK_VR="$ROOT/sdk/vrsdk-3.5.0.68"
[ -f "$SDK_HW/jni/armeabi-v7a/libxr_loader.so" ] || { echo "缺华为 OpenXR loader"; exit 1; }
[ -f "$SDK_VR/Plugins/Android/hvrprompt.aar" ] || { echo "缺 hvrprompt.aar"; exit 1; }

WORK="$CLIENT_DIR/build/huawei-pack"
rm -rf "$WORK" && mkdir -p "$WORK"/{gen,classes,dex,res,assets,lib/armeabi-v7a,libs}
cp -r android_huawei/assets/. "$WORK/assets/"
cp "$SDK_HW/jni/armeabi-v7a/libxr_loader.so" "$WORK/lib/armeabi-v7a/"
cp "$SDK_HW/libs/hvrbridge.jar" "$WORK/libs/hvrbridge.jar"
cp "$SDK_VR/Plugins/Android/hvrbridge.jar" "$WORK/libs/hvrbridge-vrsdk.jar"

# 提取两个 AAR 的 classes.jar 与资源（目录分开，避免同名覆盖）
rm -rf "$WORK/prompt" "$WORK/version"
mkdir -p "$WORK/prompt" "$WORK/version"
( cd "$WORK/prompt" && unzip -q "$SDK_VR/Plugins/Android/hvrprompt.aar" )
( cd "$WORK/version" && unzip -q "$SDK_VR/Plugins/Android/hvrversioncheck.aar" )
for d in prompt version; do
  [ -f "$WORK/$d/classes.jar" ] && unzip -q -o "$WORK/$d/classes.jar" -d "$WORK/classes"
  [ -d "$WORK/$d/res" ] && cp -r "$WORK/$d/res/." "$WORK/res/"
done

# 应用图标（清单引用 @mipmap/ic_launcher）
mkdir -p "$WORK/res/mipmap-xxxhdpi"
cp android_huawei/assets/vr_icon_foreground.png "$WORK/res/mipmap-xxxhdpi/ic_launcher.png"

# 编译资源（AAR 资源 + 本应用图标）
"$AAPT2" compile --dir "$WORK/res" -o "$WORK/res.zip"
"$AAPT2" link -o "$WORK/base.apk" \
  -I "$PLATFORM/android.jar" \
  --manifest android_huawei/AndroidManifest.xml \
  --java "$WORK/gen" \
  -R "$WORK/res.zip" \
  --extra-packages com.huawei.vrlab \
  --extra-packages com.huawei.hvr \
  -A "$WORK/assets" \
  --auto-add-overlay \
  --min-sdk-version 26 --target-sdk-version 28

# 编译 Java（含 AAPT2 生成的 R）
mapfile -t JAVA_SRC < <(find android_huawei/src "$WORK/gen" -name '*.java')
# 不用 -bootclasspath：Android android.jar 不含 LambdaMetafactory，JDK 平台类需参与编译；D8 负责脱糖。
javac -encoding UTF-8 -source 8 -target 8 \
  -cp "$PLATFORM/android.jar:$WORK/libs/*:$WORK/classes" -d "$WORK/classes" "${JAVA_SRC[@]}"

# d8
mapfile -t CLASSES < <(find "$WORK/classes" -name '*.class')
"$D8" --lib "$PLATFORM/android.jar" --min-api 26 --output "$WORK/dex" "${CLASSES[@]}"
cp "$WORK/dex/classes.dex" "$WORK/classes.dex"

# 原生库
NATIVE_SO="$ROOT/服务端-ALVR-完整源码/target/armv7-linux-androideabi/release/libalvr_client_openxr.so"
if [ ! -f "$NATIVE_SO" ]; then
  echo "缺 $NATIVE_SO；请先运行 build_native.sh"
  exit 1
fi
cp "$NATIVE_SO" "$WORK/lib/armeabi-v7a/libalvr_client_openxr.so"
NATIVE_SO64="$ROOT/服务端-ALVR-完整源码/target/aarch64-linux-android/release/libalvr_client_openxr.so"
if [ -f "$NATIVE_SO64" ]; then
  mkdir -p "$WORK/lib/arm64-v8a"
  cp "$NATIVE_SO64" "$WORK/lib/arm64-v8a/libalvr_client_openxr.so"
  cp "$SDK_HW/jni/arm64-v8a/libxr_loader.so" "$WORK/lib/arm64-v8a/"
fi

# 合并 APK
cp "$WORK/base.apk" "$WORK/final.apk"
( cd "$WORK" && zip -q -r final.apk classes.dex lib assets )
"$ZIPALIGN" -f -p 4 "$WORK/final.apk" "$WORK/aligned.apk"

KEYSTORE="$CLIENT_DIR/build/alvr_client_android/debug.keystore"
if [ ! -f "$KEYSTORE" ]; then
  mkdir -p "$(dirname "$KEYSTORE")"
  keytool -genkeypair -v -keystore "$KEYSTORE" -alias alvrclient \
    -keyalg RSA -keysize 2048 -storepass alvrclient -keypass alvrclient \
    -dname "CN=ALVR Huawei VR, OU=Dev, O=ALVR, L=NA, ST=NA, C=CN" -validity 10000
fi
"$APKSIGNER" sign --ks "$KEYSTORE" --ks-pass pass:alvrclient --key-pass pass:alvrclient \
  --out "$CLIENT_DIR/build/HuaweiVRGlass-armv7.apk" "$WORK/aligned.apk"
"$APKSIGNER" verify "$CLIENT_DIR/build/HuaweiVRGlass-armv7.apk"
"$AAPT2" dump badging "$CLIENT_DIR/build/HuaweiVRGlass-armv7.apk" | head -8 || true
ls -lh "$CLIENT_DIR/build/HuaweiVRGlass-armv7.apk"
