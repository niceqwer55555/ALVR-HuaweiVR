#!/usr/bin/env bash
# ALVR-HuaweiVR 完整交付包自检
set -u
PASS=0; FAIL=0
ok(){ echo "  [OK]   $1"; PASS=$((PASS+1)); }
no(){ echo "  [FAIL] $1"; FAIL=$((FAIL+1)); }
chkfile(){ [ -f "$1" ] && ok "$2" || no "$2 (缺 $1)"; }
grepfile(){ [ -f "$1" ] && grep -q "$2" "$1" && ok "$3" || no "$3"; }

echo "=== 1. 顶层结构 ==="
for f in README.md docs/01-画面位置修复原理.md docs/02-服务端编译复现.md \
         patches/画面位置修复-接入说明.md; do chkfile "$f" "顶层: $f"; done

echo "=== 2. 服务端源码 ==="
grepfile 服务端-ALVR/server-core/src/Cargo.toml 'alvr_server_core' "服务端: server_core Cargo"
chkfile 服务端-ALVR/server-openvr/cpp/platform/win32/CEncoder.cpp "服务端: CEncoder.cpp(投屏)"
chkfile 服务端-ALVR/server-openvr/cpp/platform/win32/DesktopCapture.cpp "服务端: DesktopCapture.cpp(画布blit)"
grepfile 服务端-ALVR/server-openvr/cpp/platform/win32/CEncoder.cpp 'RenderCastFrame' "服务端: RenderCastFrame 投屏入口"
chkfile 服务端-ALVR/本地依赖patch/wgpu-hal-Cargo.toml.patch说明 "服务端: wgpu-hal patch 说明"
[ "$(find 服务端-ALVR/官方编译脚本 -type f 2>/dev/null | wc -l)" -ge 30 ] && ok "服务端: 官方编译脚本≥30" || no "服务端: 官方编译脚本不足"

echo "=== 3. 三类客户端 ==="
# ① PC
grepfile 客户端1-PC接收端/Cargo.toml 'hvr-alvr-win-sink' "客户端①: PC Cargo"
chkfile 客户端1-PC接收端/hvr-alvr-win-sink.exe "客户端①: 源码内 exe"
# ② 依赖华为框架安卓
chkfile "客户端2-安卓接收端(依赖华为框架)/client_openxr/src/huawei_activity.rs" "客户端②: 华为 Activity"
chkfile "客户端2-安卓接收端(依赖华为框架)/client_openxr/src/huawei_runtime.rs" "客户端②: 华为 runtime"
chkfile "客户端2-安卓接收端(依赖华为框架)/client_openxr/android_huawei/AndroidManifest.xml" "客户端②: 华为 Manifest"
grepfile "客户端2-安卓接收端(依赖华为框架)/client_openxr/Cargo.toml" 'client_openxr' "客户端②: Cargo"
# ③ 去华为高通安卓
chkfile "客户端3-安卓接收端(高通/去华为框架)/src/decoder/decoder_policy.c" "客户端③: 高通解码器模块"
chkfile "客户端3-安卓接收端(高通/去华为框架)/src/pose/pose.c" "客户端③: 自算姿态模块"
chkfile "客户端3-安卓接收端(高通/去华为框架)/src/hid/hwvr_hid.c" "客户端③: HID 模块"
grepfile "客户端3-安卓接收端(高通/去华为框架)/src/decoder/decoder_policy.c" 'qti' "客户端③: qti 解码器"
grepfile "客户端3-安卓接收端(高通/去华为框架)/README.md" '去华为' "客户端③: 去华为声明"

echo "=== 4. 画面修复模块 ==="
for f in frame_roll_fix/frame_roll_fix.h frame_roll_fix/frame_roll_fix.c \
         frame_roll_fix/test_frame_roll_fix.c frame_roll_fix/libframe_roll_fix.a; do chkfile "$f" "修复模块: $f"; done
# 单测实跑
T=$(gcc -std=c11 -O2 -Wall -I frame_roll_fix frame_roll_fix/test_frame_roll_fix.c frame_roll_fix/frame_roll_fix.c -lm -o /tmp/fr_verify 2>&1 && /tmp/fr_verify)
echo "$T" | grep -q 'PASS=18 FAIL=0' && ok "修复算法单测 18/18" || no "修复算法单测失败"

echo "=== 5. 编译产物 ==="
chkfile 产物/服务端/alvr_server_core.dll "产物: 服务端 DLL"
chkfile 产物/服务端/libalvr_server_core.dll.a "产物: 服务端导入库"
chkfile 产物/服务端/libalvr_server_core.rlib "产物: 服务端 rlib"
chkfile 产物/客户端-PC/hvr-alvr-win-sink.exe "产物: PC exe"
chkfile "产物/客户端-PC/HVR-VRGlass-接收端.exe" "产物: PC GUI 双击启动器"
chkfile 产物/客户端-PC/device_status.json "产物: PC 设备状态样例"
chkfile 产物/客户端-PC/controller_status.json "产物: PC 手柄状态注入样例"
chkfile 产物/画面修复模块/libframe_roll_fix.a "产物: 修复静态库"
chkfile 产物/怎么运行.md "产物: 运行指南"
chkfile 产物/服务端/openvr/driver_alvr_server.dll "产物: OpenVR 服务端驱动 DLL"
chkfile 产物/服务端/openvr/ALVR\ Dashboard.exe "产物: 服务端 Dashboard GUI"
chkfile 产物/服务端/openvr/openvr_api.dll "产物: OpenVR API DLL"
file 产物/服务端/openvr/ALVR\ Dashboard.exe 2>/dev/null | grep -q 'PE32+' && ok "Dashboard 为 PE32+ GUI" || no "Dashboard 非 PE32+"
chkfile 产物/安卓客户端/GenericALVR-arm64.apk "产物: 安卓接收端 APK"
chkfile 产物/安卓客户端/libs/arm64-v8a/libgeneric_alvr_bridge.so "产物: 安卓 JNI 桥库"
# PE 校验
file 产物/服务端/alvr_server_core.dll 2>/dev/null | grep -q 'PE32+' && ok "服务端 DLL 为 PE32+" || no "服务端 DLL 非 PE32+"
file 产物/客户端-PC/hvr-alvr-win-sink.exe 2>/dev/null | grep -q 'PE32+' && ok "PC exe 为 PE32+" || no "PC exe 非 PE32+"
file "产物/客户端-PC/HVR-VRGlass-接收端.exe" 2>/dev/null | grep -q 'PE32+.*GUI' && ok "PC GUI 为 PE32+ 图形程序" || no "PC GUI 非图形程序"
# 三设备指示灯（头显/左/右手柄）：PC GUI 端
grepfile 客户端1-PC接收端/gui/sink_gui.c 'HvrLedPanelClass' "指示灯: PC GUI 三设备面板"
grepfile 客户端1-PC接收端/gui/sink_gui.c 'left_controller' "指示灯: PC GUI 读左手柄状态"
grepfile 客户端1-PC接收端/gui/sink_gui.c 'right_controller' "指示灯: PC GUI 读右手柄状态"
grepfile 客户端1-PC接收端/src/status.rs 'left_controller' "指示灯: PC 状态模型含左手柄"
grepfile 客户端1-PC接收端/src/status.rs 'right_controller' "指示灯: PC 状态模型含右手柄"
# 三设备指示灯：安卓端
grepfile "客户端3-安卓接收端(高通/去华为框架)/android/java/com/generic/alvr/AlvrActivity.java" 'refreshDeviceLeds' "指示灯: 安卓三设备刷新"
grepfile "客户端3-安卓接收端(高通/去华为框架)/android/jni_bridge.c" 'nativeGetControllerStatus' "指示灯: 安卓手柄状态 JNI"
file 产物/服务端/openvr/driver_alvr_server.dll 2>/dev/null | grep -q 'PE32+' && ok "OpenVR DLL 为 PE32+" || no "OpenVR DLL 非 PE32+"
# ELF/APK 校验
file 产物/安卓客户端/libs/arm64-v8a/libgeneric_alvr_bridge.so 2>/dev/null | grep -q 'ARM aarch64' && ok "JNI 库为 aarch64 ELF" || no "JNI 库架构错误"
file 产物/安卓客户端/GenericALVR-arm64.apk 2>/dev/null | grep -qE 'Android package|Zip archive' && ok "APK 格式有效" || no "APK 格式错误"
unzip -l 产物/安卓客户端/GenericALVR-arm64.apk 2>/dev/null | grep -q 'lib/arm64-v8a/libgeneric_alvr_bridge.so' && ok "APK 含 arm64 native 库" || no "APK 缺 native 库"

echo ""
echo "=========================="
echo "PASS=$PASS FAIL=$FAIL"
[ "$FAIL" -eq 0 ] && echo "✅ 交付包自检全部通过" || echo "⚠ 存在失败项"
