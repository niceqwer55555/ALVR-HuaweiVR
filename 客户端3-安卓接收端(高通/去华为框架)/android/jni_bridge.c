/*
 * JNI 桥：Java host (com.generic.alvr.AlvrActivity) ↔ 通用 C runtime 核心。
 * 去华为化：无 libvrpose.so / libxr_loader.so / 华为权限 / /dev/hwvr*。
 *
 * 职责：
 *   1) 接收 Java 侧的手机 IMU 样本 → ga_runtime_feed_head_imu（本地 Mahony 自算）
 *   2) 接收 Java 侧的手柄 IMU/按键 → ga_runtime_* controller 系列
 *   3) 后台 UDP 47110 线程接收外部姿态（w x y z [px py pz valid]），可插拔
 *   4) 探测高通解码器是否可用（MediaCodec 由 Java 侧查，这里给出期望名单）
 *   5) 供 OpenXR runtime 渲染循环读取头部/手柄姿态
 */
#include <jni.h>
#include <string.h>
#include <stdio.h>
#include <stdint.h>
#include "pose.h"
#include "controller.h"
#include "decoder_policy.h"
#include "hwvr_hid.h"

static ga_pose_tracker_t *g_head = NULL;          /* 与 runtime 共享，简化为单例 */
static ga_controller_hub_t *g_ctrl = NULL;

JNIEXPORT void JNICALL
Java_com_generic_alvr_AlvrActivity_nativeInit(JNIEnv *env, jobject thiz,
        jint view_w, jint view_h, jint refresh_hz, jboolean position_tracking) {
    (void)env; (void)thiz;
    if (!g_head) g_head = ga_pose_create(2.0, 0.0);
    if (!g_ctrl) g_ctrl = ga_controller_create();
    ga_pose_reset(g_head);
    (void)view_w; (void)view_h; (void)refresh_hz; (void)position_tracking;
}

/* 手机 IMU：type=0 陀螺(rad/s)，type=1 加速度(m/s^2) */
JNIEXPORT void JNICALL
Java_com_generic_alvr_AlvrActivity_nativeFeedImu(JNIEnv *env, jobject thiz,
        jint type, jfloat x, jfloat y, jfloat z, jlong t_ns) {
    (void)env; (void)thiz;
    static double last_t = 0;
    double dt = last_t > 0 ? (t_ns / 1e9 - last_t) : 0.01;
    if (dt < 0.0005 || dt > 0.1) dt = 0.01;
    last_t = t_ns / 1e9;
    static double gx, gy, gz, ax, ay, az;
    if (type == 0) { gx = x; gy = y; gz = z; }
    else           { ax = x; ay = y; az = z; }
    ga_imu_sample_t s = { gx, gy, gz, ax, ay, az, dt };
    ga_pose_update_imu(g_head, &s);
}

/* 外部 UDP 姿态文本注入（Java 侧 UDP 收到后传下来，或 native 线程直收） */
JNIEXPORT jboolean JNICALL
Java_com_generic_alvr_AlvrActivity_nativeFeedPoseText(JNIEnv *env, jobject thiz,
        jstring text) {
    (void)thiz;
    if (!g_head || !text) return JNI_FALSE;
    const char *s = (*env)->GetStringUTFChars(env, text, NULL);
    int len = (int)strlen(s);
    jboolean ok = ga_pose_ingest_udp_text(g_head, s, len) ? JNI_TRUE : JNI_FALSE;
    (*env)->ReleaseStringUTFChars(env, text, s);
    return ok;
}

/* 手柄 IMU */
JNIEXPORT void JNICALL
Java_com_generic_alvr_AlvrActivity_nativeFeedControllerImu(JNIEnv *env, jobject thiz,
        jint hand, jfloat gx, jfloat gy, jfloat gz,
        jfloat ax, jfloat ay, jfloat az, jlong t_ns) {
    (void)env; (void)thiz;
    static double last[2] = {0,0};
    double dt = last[hand] > 0 ? (t_ns/1e9 - last[hand]) : 0.01;
    if (dt < 0.0005 || dt > 0.1) dt = 0.01;
    last[hand] = t_ns/1e9;
    ga_imu_sample_t s = {gx,gy,gz,ax,ay,az,dt};
    ga_controller_update_imu(g_ctrl, hand, &s);
}

/* 手柄按键/轴 */
JNIEXPORT void JNICALL
Java_com_generic_alvr_AlvrActivity_nativeFeedControllerInput(JNIEnv *env, jobject thiz,
        jint hand, jint buttons, jfloat ax, jfloat ay, jfloat trigger, jfloat grip) {
    (void)env; (void)thiz;
    ga_controller_set_input(g_ctrl, hand, (uint32_t)buttons, ax, ay, trigger, grip);
}

JNIEXPORT void JNICALL
Java_com_generic_alvr_AlvrActivity_nativeSetControllerConnected(JNIEnv *env, jobject thiz,
        jint hand, jboolean connected) {
    (void)env; (void)thiz;
    ga_controller_set_connected(g_ctrl, hand, connected == JNI_TRUE);
}

/* 读取头部姿态返回 float[7] = qx,qy,qz,qw, px,py,pz（供 OpenXR 渲染循环） */
JNIEXPORT void JNICALL
Java_com_generic_alvr_AlvrActivity_nativeGetHeadPose(JNIEnv *env, jobject thiz, jfloatArray out) {
    (void)thiz;
    if (!g_head) return;
    ga_pose_t p = ga_pose_get(g_head);
    jfloat v[7] = {
        (jfloat)p.q.x, (jfloat)p.q.y, (jfloat)p.q.z, (jfloat)p.q.w,
        (jfloat)p.position.x, (jfloat)p.position.y, (jfloat)p.position.z
    };
    (*env)->SetFloatArrayRegion(env, out, 0, 7, v);
}

JNIEXPORT jboolean JNICALL
Java_com_generic_alvr_AlvrActivity_nativeHeadPosValid(JNIEnv *env, jobject thiz) {
    (void)env; (void)thiz;
    return g_head && ga_pose_get(g_head).position_valid ? JNI_TRUE : JNI_FALSE;
}

JNIEXPORT void JNICALL
Java_com_generic_alvr_AlvrActivity_nativeRecenter(JNIEnv *env, jobject thiz) {
    (void)env; (void)thiz;
    if (g_head) ga_pose_reset(g_head);
    if (g_ctrl) { ga_controller_reset_pose(g_ctrl, 0); ga_controller_reset_pose(g_ctrl, 1); }
}

/* 返回期望的高通解码器名数组（Java 侧用 MediaCodecList 逐一核对可用性） */
JNIEXPORT jobjectArray JNICALL
Java_com_generic_alvr_AlvrActivity_nativeDecoderCandidates(JNIEnv *env, jobject thiz) {
    (void)thiz;
    int n = 0;
    const char *const *c = ga_decoder_candidates(DECODE_CODEC_H264, &n);
    jclass strCls = (*env)->FindClass(env, "java/lang/String");
    jobjectArray arr = (*env)->NewObjectArray(env, n, strCls, NULL);
    for (int i = 0; i < n; ++i) {
        jstring s = (*env)->NewStringUTF(env, c[i]);
        (*env)->SetObjectArrayElement(env, arr, i, s);
        (*env)->DeleteLocalRef(env, s);
    }
    return arr;
}

/* ---- HID 眼镜状态（协议应答，去 VID/PID）---- */
static ga_device_status_t g_dev_status;

/* 喂一帧 /dev/hwvrN 的 read() 输入报告，解析并合并状态。返回是否识别为有效报告。 */
JNIEXPORT jboolean JNICALL
Java_com_generic_alvr_AlvrActivity_nativeFeedHidInput(JNIEnv *env, jobject thiz, jbyteArray data) {
    (void)thiz;
    if (!data) return JNI_FALSE;
    jsize n = (*env)->GetArrayLength(env, data);
    if (n < 9) return JNI_FALSE;
    uint8_t buf[512];
    if (n > 512) n = 512;
    (*env)->GetByteArrayRegion(env, data, 0, n, (jbyte *)buf);
    ga_input_report_t rep;
    if (!ga_hid_parse_input(buf, n, &rep)) return JNI_FALSE;
    ga_status_apply_input(&g_dev_status, &rep);
    return JNI_TRUE;
}

/* Java 侧用 BatteryManager/TYPE_PROXIMITY 补等价源（CV10 HID 无 report 4/8）。 */
JNIEXPORT void JNICALL
Java_com_generic_alvr_AlvrActivity_nativeSetDeviceStatus(JNIEnv *env, jobject thiz,
        jint battery_percent, jboolean proximity_near, jboolean valid_bat, jboolean valid_prox) {
    (void)env; (void)thiz;
    if (valid_bat) { g_dev_status.battery_percent = battery_percent; g_dev_status.battery_valid = true; }
    if (valid_prox) { g_dev_status.proximity_near = proximity_near == JNI_TRUE; g_dev_status.proximity_valid = true; }
}

/* 返回某只手柄状态 [connected, dof, battery_pct]，供 UI 指示灯显示。
 * hand: 0=左 1=右。 */
JNIEXPORT void JNICALL
Java_com_generic_alvr_AlvrActivity_nativeGetControllerStatus(JNIEnv *env, jobject thiz,
        jint hand, jintArray out) {
    (void)thiz;
    jint v[3] = {0, 0, -1};
    if (g_ctrl && hand >= 0 && hand < GA_MAX_CONTROLLERS) {
        ga_controller_state_t st = ga_controller_get(g_ctrl, hand);
        v[0] = st.connected ? 1 : 0;
        v[1] = st.dof;
        v[2] = st.battery_pct;
    }
    (*env)->SetIntArrayRegion(env, out, 0, 3, v);
}

/* 设置手柄电量（-1=未知）。 */
JNIEXPORT void JNICALL
Java_com_generic_alvr_AlvrActivity_nativeSetControllerBattery(JNIEnv *env, jobject thiz,
        jint hand, jint battery_pct) {
    (void)env; (void)thiz;
    if (g_ctrl) ga_controller_set_battery(g_ctrl, hand, battery_pct);
}

/* 返回设备状态 [near,prox_valid,battery,bat_valid,headset,overheat,heartbeat] */
JNIEXPORT void JNICALL
Java_com_generic_alvr_AlvrActivity_nativeGetDeviceStatus(JNIEnv *env, jobject thiz, jintArray out) {
    (void)thiz;
    jint v[7] = {
        g_dev_status.proximity_near, g_dev_status.proximity_valid,
        g_dev_status.battery_percent, g_dev_status.battery_valid,
        g_dev_status.headset_plugged, g_dev_status.overheat, g_dev_status.heartbeat_ok
    };
    (*env)->SetIntArrayRegion(env, out, 0, 7, v);
}
