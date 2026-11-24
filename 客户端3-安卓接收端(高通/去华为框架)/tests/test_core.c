/*
 * 核心逻辑单测：姿态解算 / 双模手柄 / 高通解码器策略。
 * 沙箱可运行：gcc -std=c11 -O2 -I src/pose -I src/controller -I src/decoder \
 *   tests/test_core.c src/pose/pose.c src/controller/controller.c \
 *   src/decoder/decoder_policy.c -lm -o test_core && ./test_core
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include "pose.h"
#include "controller.h"
#include "decoder_policy.h"
#include "generic_runtime.h"
#include "hwvr_hid.h"

static int PASS = 0, FAIL = 0;
#define CHECK(cond, msg) do { if (cond) { printf("  [OK] %s\n", msg); PASS++; } \
    else { printf("  [FAIL] %s\n", msg); FAIL++; } } while (0)

/* ---------- 姿态模块 ---------- */
static void test_pose_basics(void) {
    printf("\n[姿态] 四元数基础\n");
    ga_quat_t q = ga_quat_from_axis_angle(0, 1, 0, 3.14159265 / 2);
    /* 绕Y轴90度 = [cos45°, 0, sin45°, 0]，w 与 y 均为 0.7071 */
    CHECK(fabs(q.w - 0.7071) < 1e-3 && fabs(q.y - 0.7071) < 1e-3 &&
          fabs(q.x) < 1e-3 && fabs(q.z) < 1e-3, "绕Y轴90度四元数正确");
    ga_quat_normalize(&q);
    double n = sqrt(q.w*q.w + q.x*q.x + q.y*q.y + q.z*q.z);
    CHECK(fabs(n - 1.0) < 1e-6, "四元数归一化为单位长度");

    ga_quat_t a = ga_quat_from_axis_angle(0,0,1, 3.14159265/2);
    ga_quat_t b = ga_quat_from_axis_angle(0,0,1, 3.14159265/2);
    ga_quat_t c; ga_quat_mul(&a,&b,&c);
    CHECK(fabs(c.w) < 1e-3 && fabs(c.z - 1.0) < 1e-3, "四元数乘法：两次Z轴90度=Z轴180度");
}

static void test_pose_integrate(void) {
    printf("\n[姿态] 陀螺自算积分\n");
    ga_pose_tracker_t *t = ga_pose_create(2.0, 0.0);
    CHECK(t != NULL, "解算器创建成功");
    /* 绕Y轴以 π rad/s 转 0.5s ≈ 90 度；无加速度只做陀螺积分 */
    for (int i = 0; i < 50; ++i) {
        ga_imu_sample_t s = { 0, 3.14159265, 0, 0, 0, 0, 0.01 };
        ga_pose_update_imu(t, &s);
    }
    ga_pose_t p = ga_pose_get(t);
    CHECK(fabs(p.q.y - 0.7071) < 0.05, "陀螺积分90度：q.y≈0.707");
    CHECK(!p.position_valid, "纯IMU: 3DoF 位置无效");
    ga_pose_reset(t);
    p = ga_pose_get(t);
    CHECK(fabs(p.q.w - 1.0) < 1e-9, "reset 回到单位四元数");
    ga_pose_destroy(t);
}

static void test_pose_dual_source(void) {
    printf("\n[姿态] 双数据源切换\n");
    ga_pose_tracker_t *t = ga_pose_create(2.0, 0.0);
    CHECK(ga_pose_get_source(t) == POSE_SRC_PHONE_IMU, "默认手机IMU源");
    /* UDP 注入 8 值报文（带位置，6DoF） */
    const char *msg = "0.7071 0.0 0.7071 0.0 0.1 1.6 -0.2 1";
    CHECK(ga_pose_ingest_udp_text(t, msg, (int)strlen(msg)), "UDP 8值报文解析成功");
    ga_pose_t p = ga_pose_get(t);
    CHECK(fabs(p.q.w - 0.7071) < 1e-3, "UDP 四元数 w 正确");
    CHECK(p.position_valid && fabs(p.position.y - 1.6) < 1e-3, "UDP 6DoF 位置生效");
    /* 切到 UDP 后本地 IMU 不覆盖 */
    ga_pose_set_source(t, POSE_SRC_UDP);
    ga_imu_sample_t s = { 5, 0, 0, 0, 0, 0, 0.01 };
    ga_pose_update_imu(t, &s);
    ga_pose_t p2 = ga_pose_get(t);
    CHECK(fabs(p2.q.w - p.q.w) < 1e-9, "UDP模式: IMU积分被暂停");
    /* 3值报文（仅四元数）→ 位置无效 */
    const char *q3 = "1 0 0 0";
    ga_pose_ingest_udp_text(t, q3, (int)strlen(q3));
    p = ga_pose_get(t);
    CHECK(!p.position_valid, "UDP 4值报文: 位置无效(3DoF)");
    ga_pose_destroy(t);
}

/* ---------- 手柄模块 ---------- */
static void test_controller_dual_mode(void) {
    printf("\n[手柄] 3DoF/6DoF 双模\n");
    ga_controller_hub_t *h = ga_controller_create();
    CHECK(h != NULL, "手柄聚合器创建成功");

    /* 左手 3DoF：注入带位置的报文，位置应被忽略 */
    ga_controller_set_dof(h, 0, CTRL_DOF_3);
    ga_controller_set_connected(h, 0, true);
    const char *m6 = "1 0 0 0 9.9 9.9 9.9 1";
    CHECK(ga_controller_inject_pose_text(h, 0, m6, (int)strlen(m6)), "3DoF手柄注入姿态成功");
    ga_controller_state_t L = ga_controller_get(h, 0);
    CHECK(L.connected && !L.pose.position_valid, "3DoF手柄: 已连接且忽略位置");

    /* 右手 6DoF：注入带位置报文，位置生效 */
    ga_controller_set_dof(h, 1, CTRL_DOF_6);
    ga_controller_set_connected(h, 1, true);
    CHECK(ga_controller_inject_pose_text(h, 1, m6, (int)strlen(m6)), "6DoF手柄注入姿态成功");
    ga_controller_state_t R = ga_controller_get(h, 1);
    CHECK(R.pose.position_valid && fabs(R.pose.position.x - 9.9) < 1e-3, "6DoF手柄: 位置生效");

    /* 按键/轴 */
    ga_controller_set_input(h, 0, GA_BTN_TRIGGER | GA_BTN_A, 0.5f, -0.25f, 0.8f, 0.0f);
    L = ga_controller_get(h, 0);
    CHECK((L.buttons & GA_BTN_TRIGGER) && (L.buttons & GA_BTN_A), "按键位掩码正确");
    CHECK(fabs(L.axis_x - 0.5f) < 1e-3 && fabs(L.trigger - 0.8f) < 1e-3, "摇杆/扳机轴正确");

    /* 断开 */
    ga_controller_set_connected(h, 1, false);
    R = ga_controller_get(h, 1);
    CHECK(!R.connected, "手柄断开状态正确");
    ga_controller_destroy(h);
}

/* ---------- 解码器模块 ---------- */
static bool avail_qcom(const char *name, void *ctx) {
    (void)ctx;
    return strcmp(name, "c2.qti.avc.decoder") == 0 ||
           strcmp(name, "OMX.qcom.video.decoder.avc") == 0 ||
           strcmp(name, "c2.android.avc.decoder") == 0;
}
static bool avail_only_sw(const char *name, void *ctx) {
    (void)ctx;
    return strcmp(name, "c2.android.avc.decoder") == 0;
}
static bool avail_none(const char *name, void *ctx) { (void)name; (void)ctx; return false; }

static void test_decoder_policy(void) {
    printf("\n[解码器] 高通优先 + 禁软解\n");
    ga_decoder_config_t c = ga_decoder_default(DECODE_CODEC_H264);
    CHECK(!c.allow_software, "默认禁止软解");
    const char *p = ga_decoder_pick(&c, avail_qcom, NULL);
    CHECK(p && strcmp(p, "c2.qti.avc.decoder") == 0, "优先选 Codec2 高通硬解");
    CHECK(ga_decoder_is_software("c2.android.avc.decoder"), "识别软解组件");
    CHECK(!ga_decoder_is_software("c2.qti.avc.decoder"), "高通组件非软解");

    /* 只有软解可用 → 返回 NULL（不静默降级） */
    p = ga_decoder_pick(&c, avail_only_sw, NULL);
    CHECK(p == NULL, "禁软解时仅软解可用→失败(NULL)");
    /* 全不可用 → NULL */
    p = ga_decoder_pick(&c, avail_none, NULL);
    CHECK(p == NULL, "无可用组件→失败(NULL)");

    /* 显式允许软解时才兜底 */
    c.allow_software = true;
    p = ga_decoder_pick(&c, avail_only_sw, NULL);
    CHECK(p && ga_decoder_is_software(p), "显式允许软解时才兜底软解");

    /* 候选链不含任何 hisi */
    int n = 0;
    const char *const *cands = ga_decoder_candidates(DECODE_CODEC_H264, &n);
    bool has_hisi = false;
    for (int i = 0; i < n; ++i) if (strstr(cands[i], "hisi")) has_hisi = true;
    CHECK(!has_hisi, "候选链无华为 hisi 组件");
}

/* ---------- Runtime 桥接模块 ---------- */
static void test_runtime_bridge(void) {
    printf("\n[Runtime] 平台桥接\n");
    ga_runtime_t *r = ga_runtime_create(NULL);
    CHECK(r != NULL, "runtime 创建成功");
    ga_view_config_t c = ga_runtime_view_config(r);
    CHECK(c.sbs_width == 3200 && c.sbs_height == 1600, "默认SBS=3200x1600(双目各1600)");
    CHECK(!c.position_tracking, "默认3DoF");

    /* 头部 UDP 注入 → OpenXR 四元数顺序应为 {x,y,z,w} */
    ga_runtime_feed_head_udp(r, "0.7071 0.0 0.7071 0.0 0.1 1.6 0.2 1", 39);
    float q[4], pos[3]; bool pv = false;
    ga_runtime_get_head_pose(r, q, pos, &pv);
    CHECK(fabs(q[1] - 0.7071) < 1e-3 && fabs(q[3] - 0.7071) < 1e-3,
          "头部姿态转OpenXR顺序 {x,y,z,w}: y=w=0.7071");
    CHECK(pv && fabs(pos[1] - 1.6) < 1e-3, "头部6DoF位置透传");

    /* 手柄桥接 */
    ga_runtime_set_controller_dof(r, 1, CTRL_DOF_6);
    ga_runtime_set_controller_connected(r, 1, true);
    ga_runtime_feed_controller_udp(r, 1, "1 0 0 0 0.5 1.2 -0.3 1", 25);
    ga_runtime_feed_controller_input(r, 1, GA_BTN_GRIP, 0, 0, 0.2f, 0.9f);
    ga_controller_state_t st = ga_runtime_get_controller(r, 1);
    CHECK(st.connected && st.pose.position_valid, "runtime手柄6DoF状态透传");
    CHECK((st.buttons & GA_BTN_GRIP) && fabs(st.grip - 0.9f) < 1e-3, "runtime手柄输入透传");

    ga_runtime_recenter(r);
    ga_runtime_get_head_pose(r, q, pos, &pv);
    CHECK(fabs(q[3] - 1.0f) < 1e-3 && !pv, "recenter复位头部姿态");
    ga_runtime_destroy(r);
}

/* ---------- HID 协议模块（协议应答，去 VID/PID） ---------- */
static void test_hid_protocol(void) {
    printf("\n[HID] 眼镜控制/输入协议\n");
    CHECK(GA_HIDIOCG_FEATURE(4) == 0xC0044807, "HIDIOCGFEATURE(4)=0xC0044807");
    CHECK(GA_HIDIOCS_FEATURE(5) == 0xC0054806, "HIDIOCSFEATURE(5)=0xC0054806");

    /* 协议应答探测：report id=1，长度4 → 认定眼镜控制接口（不看 VID/PID） */
    uint8_t vendor_resp[4] = {1, 0x12, 0xD1, 0x00};
    CHECK(ga_hid_looks_like_helmet(vendor_resp, 4, 1, 4), "协议应答: report1 认定眼镜接口");
    CHECK(!ga_hid_looks_like_helmet(vendor_resp, 2, 1, 4), "协议应答: 长度不足拒绝");
    uint8_t wrong[4] = {9, 0, 0, 0};
    CHECK(!ga_hid_looks_like_helmet(wrong, 4, 1, 4), "协议应答: report id 不符拒绝");

    /* 激活报文：report id=5, buf[1]=device_type&0x7E */
    uint8_t act[5];
    CHECK(ga_hid_build_active(act, 5, true, 0x91) == 5, "激活报文长度5");
    CHECK(act[0] == 5 && act[1] == 0x10, "激活报文: id=5, type&0x7E=0x10");
    ga_hid_build_active(act, 5, false, 0x91);
    CHECK(act[0] == 6, "去激活报文: id=6");

    /* 输入报告：buf[8]=类型 */
    uint8_t prox[10] = {0,0,0,0,0,0,0,0, 2, 0};  /* 接近, near */
    ga_input_report_t rep;
    CHECK(ga_hid_parse_input(prox, 10, &rep) && rep.type == GA_INPUT_PROXIMITY, "输入报告: 类型2=接近");
    ga_device_status_t st = {0};
    ga_status_apply_input(&st, &rep);
    CHECK(st.proximity_valid && st.proximity_near, "接近状态: near(佩戴)");

    uint8_t bat[10] = {0,0,0,0,0,0,0,0, 5, 73};  /* 电量 73% */
    CHECK(ga_hid_parse_input(bat, 10, &rep) && rep.type == GA_INPUT_BATTERY, "输入报告: 类型5=电量");
    ga_status_apply_input(&st, &rep);
    CHECK(st.battery_valid && st.battery_percent == 73, "电量状态: 73%");

    uint8_t hb[9] = {0,0,0,0,0,0,0,0, 3};  /* 心跳 */
    ga_hid_parse_input(hb, 9, &rep);
    ga_status_apply_input(&st, &rep);
    CHECK(st.heartbeat_ok, "心跳保活状态");
    CHECK(!ga_hid_parse_input(prox, 8, &rep), "输入报告: 长度<9拒绝");
}

static void test_controller_status_battery(void) {
    ga_controller_hub_t *h = ga_controller_create();
    CHECK(h != NULL, "手柄状态: hub 创建");
    ga_controller_state_t s0 = ga_controller_get(h, 0);
    CHECK(!s0.connected && s0.battery_pct == -1, "手柄状态: 初始离线且电量未知");

    /* 连接左手柄：6DoF + 电量 85% */
    ga_controller_set_connected(h, 0, true);
    ga_controller_set_dof(h, 0, CTRL_DOF_6);
    ga_controller_set_battery(h, 0, 85);
    ga_controller_state_t l = ga_controller_get(h, 0);
    CHECK(l.connected && l.dof == CTRL_DOF_6 && l.battery_pct == 85,
          "手柄状态: 左手柄 已连接/6DoF/85%");

    /* 右手柄：3DoF + 电量钳制到 100 */
    ga_controller_set_connected(h, 1, true);
    ga_controller_set_battery(h, 1, 150);
    ga_controller_state_t r = ga_controller_get(h, 1);
    CHECK(r.connected && r.dof == CTRL_DOF_3 && r.battery_pct == 100,
          "手柄状态: 右手柄 电量钳制100");

    /* 断开后电量保留但 connected=false（UI 应显示灰暗） */
    ga_controller_set_connected(h, 0, false);
    l = ga_controller_get(h, 0);
    CHECK(!l.connected, "手柄状态: 断开后 UI 应灰暗");
    ga_controller_destroy(h);
}

int main(void) {
    printf("=== 通用安卓 ALVR 接收端 · 核心逻辑单测 ===\n");
    test_pose_basics();
    test_pose_integrate();
    test_pose_dual_source();
    test_controller_dual_mode();
    test_controller_status_battery();
    test_decoder_policy();
    test_runtime_bridge();
    test_hid_protocol();
    printf("\n========== 结果: PASS=%d FAIL=%d ==========\n", PASS, FAIL);
    return FAIL == 0 ? 0 : 1;
}
