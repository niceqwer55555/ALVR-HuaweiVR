#!/usr/bin/env bash
# 交付自检：结构完整性 + 关键内容断言 + Rust 单元测试
set -u
ROOT="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT"
PASS=0; FAIL=0
ok(){ echo "  [OK] $1"; PASS=$((PASS+1)); }
no(){ echo "  [FAIL] $1"; FAIL=$((FAIL+1)); }
hasfile(){ [ -f "$1" ] && ok "文件存在: $1" || no "缺失: $1"; }
grepfile(){ grep -q "$2" "$1" && ok "$3" || no "$3"; }

echo "=== 1. 文件结构 ==="
for f in Cargo.toml README.md sink_config.json \
  src/main.rs src/protocol.rs src/client.rs src/decoder.rs src/render_bridge.rs src/status.rs \
  hvr-alvr-win-sink.exe .cargo/config.toml \
  docs/01-架构与可行性.md docs/02-构建与运行.md docs/03-接线与调优.md docs/04-渲染器补丁.md \
  docs/05-设备状态显示.md docs/06-Windows状态获取逆向.md docs/topology.svg scripts/send_test_frame.py; do
  hasfile "$f"
done

echo "=== 2. 协议关键常量/字节序 ==="
grepfile src/protocol.rs 'CONTROL_PORT: u16 = 9943' '控制端口 9943'
grepfile src/protocol.rs 'SHARD_PREFIX_SIZE: usize = 4 + 2 + 4 + 4 + 4' '分片前缀 18 字节'
grepfile src/protocol.rs 'to_le_bytes' 'protocol_id 小端'
grepfile src/client.rs 'build_discovery_packet' '发现包构造被调用'
grepfile src/client.rs 'ClientControlPacket::StreamReady' 'StreamReady 握手'
grepfile src/client.rs 'RequestIdr' 'IDR 请求逻辑'
grepfile src/client.rs 'STREAM_TRACKING' 'tracking 回传'

echo "=== 3. 解码与渲染桥接 ==="
grepfile src/decoder.rs 'rawvideo' 'FFmpeg rawvideo 输出'
grepfile src/decoder.rs 'bgra' 'BGRA 像素格式'
grepfile src/decoder.rs 'h264' 'H.264 解码输入'
grepfile src/render_bridge.rs 'FRAME_PORT: u16 = 9911' '帧通道 9911'
grepfile src/render_bridge.rs 'POSE_PORT: u16 = 9912' '姿态通道 9912'
grepfile docs/04-渲染器补丁.md 'g_cap.srv' '渲染器视频源替换点'
grepfile docs/04-渲染器补丁.md 'sbsMode' 'SBS 分眼依据'

echo "=== 4. 事实准确性 ==="
grepfile docs/01-架构与可行性.md '3200' '眼镜分辨率事实'
grepfile docs/01-架构与可行性.md 'OpenXR fork' '缺失 fork 事实'
grepfile docs/01-架构与可行性.md '互斥' 'USB-C 互斥事实'
grepfile docs/01-架构与可行性.md 'Annex B' 'NAL 格式事实'

echo "=== 4b. 设备状态与交叉编译 ==="
grepfile src/status.rs 'WmiMonitorBasicDisplayParams' '头显 EDID 检测'
grepfile src/status.rs 'dumpsys battery' '手机电池检测'
grepfile src/status.rs 'adb' '手机在线检测'
grepfile src/client.rs 'Battery(BatteryInfo' '电池回传 ALVR 包'
grepfile docs/05-设备状态显示.md 'device_status.json' '状态快照文件'
echo "=== 4c. 原厂逆向落地（work.rar） ==="
grepfile docs/06-Windows状态获取逆向.md 'GetSystemPowerStatus' '逆向报告: 电池 API 实证'
grepfile docs/06-Windows状态获取逆向.md 'HwControllerSHARED' '逆向报告: 手柄共享内存'
grepfile docs/06-Windows状态获取逆向.md '3A09' '逆向报告: 头显 PID 白名单'
grepfile src/status.rs 'GetSystemPowerStatus' '实现: 电池改系统 API'
grepfile src/status.rs 'HMD_PIDS' '实现: 头显 PID 白名单'
# exe 必须是 PE32+
if file hvr-alvr-win-sink.exe 2>/dev/null | grep -q 'PE32+.*x86-64'; then
  ok '交叉编译 exe 是 PE32+ x86-64'
else
  no 'exe 不是 PE32+ x86-64'
fi

echo "=== 5. Rust 单元测试 ==="
if command -v cargo >/dev/null 2>&1; then
  OUT=$(cargo test 2>&1)
  echo "$OUT" | grep -q 'test result: ok' && ok 'cargo test 通过' || no 'cargo test 失败'
  N=$(echo "$OUT" | grep -oE '[0-9]+ passed' | head -1)
  echo "    测试统计: $N"
else
  echo "  [SKIP] 无 cargo，跳过单元测试"
fi

echo ""
echo "========== 结果: PASS=$PASS FAIL=$FAIL =========="
[ "$FAIL" -eq 0 ] && echo "全部通过" || echo "存在失败项"
exit "$FAIL"
