#!/usr/bin/env bash
# 把 Linux 交叉编译出的 Windows 服务端产物装进可直接分发的 openvr 目录。
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/服务端-ALVR-完整源码"
TARGET="$SRC/target/x86_64-pc-windows-gnu/release"
TEMPLATE="$ROOT/产物/服务端/openvr"
DIST="${DIST_DIR:-$ROOT/dist/windows}"
ZIP="${ZIP_PATH:-$ROOT/dist/ALVR-HuaweiVR-Windows.zip}"

[ -d "$TEMPLATE" ] || { echo "缺 Windows 模板目录：$TEMPLATE"; exit 1; }
for f in alvr_dashboard.exe alvr_launcher.exe alvr_server_core.dll alvr_server_openvr.dll; do
  [ -f "$TARGET/$f" ] || { echo "缺构建产物：$TARGET/$f"; exit 1; }
done

rm -rf "$DIST" && mkdir -p "$DIST"
cp -a "$TEMPLATE/." "$DIST/"
cp "$TARGET/alvr_dashboard.exe" "$DIST/ALVR Dashboard.exe"
cp "$TARGET/alvr_launcher.exe" "$DIST/ALVR-Launcher.exe"
cp "$TARGET/alvr_server_core.dll" "$DIST/alvr_server_core.dll"
cp "$TARGET/alvr_server_openvr.dll" "$DIST/bin/win64/driver_alvr_server.dll"
cp "$TARGET/alvr_server_openvr.dll" "$DIST/driver_alvr_server.dll"

# FFmpeg 运行库
FFMPEG_BIN="${FFMPEG_BIN:-}"
if [ -n "$FFMPEG_BIN" ] && [ -d "$FFMPEG_BIN" ]; then
  # 只带服务端实际链接的 3 个 FFmpeg 运行库，避免把 avfilter/avformat/avdevice 等整包打进去。
  for dll in avcodec-62.dll avutil-60.dll swscale-9.dll; do
    [ -f "$FFMPEG_BIN/$dll" ] && cp -f "$FFMPEG_BIN/$dll" "$DIST/bin/win64/"
  done
fi

# MinGW posix 运行库（std::thread / std::condition_variable 必需）
for dll in libstdc++-6.dll libgcc_s_seh-1.dll libwinpthread-1.dll; do
  found="$(find /usr -name "$dll" 2>/dev/null | head -n 1 || true)"
  if [ -n "$found" ]; then cp -f "$found" "$DIST/bin/win64/"; fi
done

# 去掉符号表，压缩分发包体积（MinGW 的 release 产物默认仍带调试符号）
STRIP="$(command -v x86_64-w64-mingw32-strip || command -v llvm-strip || true)"
if [ -n "$STRIP" ]; then
  for f in "ALVR Dashboard.exe" "ALVR-Launcher.exe" "bin/win64/driver_alvr_server.dll" "driver_alvr_server.dll" "alvr_server_core.dll"; do
    "$STRIP" --strip-unneeded "$DIST/$f" 2>/dev/null || true
  done
fi

# 校验关键 PE 文件
for f in "ALVR Dashboard.exe" "bin/win64/driver_alvr_server.dll" "alvr_server_core.dll"; do
  file "$DIST/$f"
done

mkdir -p "$(dirname "$ZIP")"
( cd "$DIST" && zip -q -r "$ZIP" . )
sha256sum "$ZIP" > "$ZIP.sha256"
ls -lh "$ZIP"
