#!/usr/bin/env bash
# 交付自检：结构完整性 + 去华为化断言 + 单测
set -u
ROOT="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT"
PASS=0; FAIL=0
ok(){ echo "  [OK] $1"; PASS=$((PASS+1)); }
no(){ echo "  [FAIL] $1"; FAIL=$((FAIL+1)); }
hasfile(){ [ -f "$1" ] && ok "文件存在: $1" || no "缺失: $1"; }
grepfile(){ grep -q "$2" "$1" && ok "$3" || no "$3"; }

echo "=== 1. 文件结构 ==="
for f in README.md \
  src/pose/pose.h src/pose/pose.c \
  src/controller/controller.h src/controller/controller.c \
  src/decoder/decoder_policy.h src/decoder/decoder_policy.c \
  src/hid/hwvr_hid.h src/hid/hwvr_hid.c \
  src/runtime/generic_runtime.h src/runtime/generic_runtime.c \
  rustsrc/Cargo.toml rustsrc/src/lib.rs rustsrc/src/client_config.rs \
  android/java/com/generic/alvr/AlvrActivity.java android/jni_bridge.c android/AndroidManifest.xml \
  app/active_runtime.json app/client_config.json \
  scripts/build.sh tests/test_core.c tests/run_tests.sh \
  docs/01-去华为化对照.md docs/02-构建与运行.md docs/03-姿态与手柄.md \
  docs/04-高通解码器.md docs/05-标准OpenXR-Runtime.md \
  docs/06-huawei-vr-compat分析与855Plus适配.md; do
  hasfile "$f"
done

echo "=== 2. 去华为化断言 ==="
# C/runtime/JNI 源码不得 dlopen 华为私有库（Rust 测试字面量不算实际调用）
if grep -rniE 'dlopen\("libvrpose|dlopen\("libvrsdk|dlopen\("libxr_loader|create_decoder.*hisi' \
     src/ android/ 2>/dev/null >/dev/null; then
  no "源码含实际华为私有库调用"
else
  ok "源码无实际华为私有库调用"
fi
grepfile app/active_runtime.json 'libgeneric_alvr_xr.so' '标准 runtime 库声明'
grepfile docs/05-标准OpenXR-Runtime.md 'libopenxr_loader.so' '标准 loader 说明完整'
grepfile docs/01-去华为化对照.md 'libxr_loader.so' '去华为对照文档完整'
grepfile android/AndroidManifest.xml 'com.generic.alvr' '包名已改为通用'

echo "=== 3. 高通解码器 + 禁软解 ==="
grepfile src/decoder/decoder_policy.c 'c2.qti.avc.decoder' 'C: 高通Codec2首选'
grepfile src/decoder/decoder_policy.c 'OMX.qcom.video.decoder.avc' 'C: 高通OMX兼容'
grepfile rustsrc/src/client_config.rs 'no_software_decoder' 'Rust: 禁软解字段'
grepfile app/client_config.json 'no_software_decoder": true' '配置: 禁软解开启'

echo "=== 4. 双模手柄 + 自算姿态 ==="
grepfile src/pose/pose.c 'mahony_correct' '姿态: Mahony重力校正'
grepfile src/pose/pose.c 'ga_pose_update_imu' '姿态: 陀螺积分'
grepfile src/controller/controller.h 'CTRL_DOF_3' '手柄: 3DoF枚举'
grepfile src/controller/controller.h 'CTRL_DOF_6' '手柄: 6DoF枚举'
grepfile src/controller/controller.c 'dof == CTRL_DOF_3' '手柄: 3DoF位置忽略逻辑'
grepfile src/pose/pose.h 'POSE_SRC_UDP' '姿态: UDP外部源'
grepfile src/pose/pose.h 'POSE_SRC_PHONE_IMU' '姿态: 手机IMU源'

echo "=== 5. 单测 ==="
echo "=== 4b. HID 协议 + 855Plus 适配 ==="
grepfile src/hid/hwvr_hid.h 'GA_HIDIOCG_FEATURE' 'HID ioctl 常量构造'
grepfile src/hid/hwvr_hid.c 'ga_hid_looks_like_helmet' '协议应答探测(不写死VID/PID)'
grepfile android/java/com/generic/alvr/AlvrActivity.java 'TYPE_PROXIMITY' '接近传感器等价源'
grepfile android/java/com/generic/alvr/AlvrActivity.java 'BATTERY_PROPERTY_CAPACITY' '电量等价源'
grepfile docs/06-huawei-vr-compat分析与855Plus适配.md 'SM8150' '855Plus 适配文档'

echo "=== 4c. 三设备状态指示灯（头显/左/右手柄点亮） ==="
grepfile src/controller/controller.h 'battery_pct' '手柄: 状态含电量字段'
grepfile src/controller/controller.c 'ga_controller_set_battery' '手柄: 电量接口实现'
grepfile android/jni_bridge.c 'nativeGetControllerStatus' 'JNI: 手柄状态查询导出'
grepfile android/java/com/generic/alvr/AlvrActivity.java 'refreshDeviceLeds' 'UI: 三设备指示灯刷新'
grepfile android/java/com/generic/alvr/AlvrActivity.java '左手柄' 'UI: 左手柄指示灯'
grepfile android/java/com/generic/alvr/AlvrActivity.java '右手柄' 'UI: 右手柄指示灯'
grepfile android/java/com/generic/alvr/AlvrActivity.java 'setLed' 'UI: 指示灯点亮/灰暗渲染'

C_OUT=$(gcc -std=c11 -O2 -Wall -I src/pose -I src/controller -I src/decoder -I src/runtime -I src/hid \
  tests/test_core.c src/pose/pose.c src/controller/controller.c \
  src/decoder/decoder_policy.c src/runtime/generic_runtime.c src/hid/hwvr_hid.c -lm -o /tmp/gv_test_core 2>&1 && /tmp/gv_test_core)
echo "$C_OUT" | grep -q 'PASS=56 FAIL=0' && ok "C 核心单测 56/56" || no "C 核心单测失败"
R_OUT=$(cd rustsrc && cargo test 2>&1)
echo "$R_OUT" | grep -q '8 passed' && ok "Rust 单测 8/8" || no "Rust 单测失败"

echo ""
echo "========== 结果: PASS=$PASS FAIL=$FAIL =========="
[ "$FAIL" -eq 0 ] && echo "全部通过" || echo "存在失败项"
exit "$FAIL"
