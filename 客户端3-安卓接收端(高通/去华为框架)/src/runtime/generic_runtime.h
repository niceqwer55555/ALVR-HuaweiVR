#ifndef GENERIC_ALVR_RUNTIME_H
#define GENERIC_ALVR_RUNTIME_H

/*
 * 通用 OpenXR Runtime 抽象（去华为化）
 * ======================================
 * 参考仓库的做法是写一个名为 libvrsdk.so 的 runtime，靠华为定制 loader
 * (libxr_loader.so 内写死 library_path=libvrsdk.so) 加载。本工程改为：
 *   - 使用 Khronos 官方 openxr-loader（libopenxr_loader.so）
 *   - 通过标准 active_runtime.json 声明本 runtime（名称 libgeneric_alvr_xr.so）
 *   - 不实现任何 XR_HUAWEI_* 扩展，只实现 OpenXR 1.0 核心 + 标准 KHR 扩展
 *
 * 本头文件定义 runtime 与"平台桥"之间的稳定 C ABI：Java host / JNI 通过它
 * 喂 IMU、喂 UDP 姿态、提交 SBS 呈现配置；runtime 内部用 core 模块自算姿态。
 */
#include <stdbool.h>
#include <stdint.h>
#include "pose.h"
#include "controller.h"

#ifdef __cplusplus
extern "C" {
#endif

typedef struct ga_runtime ga_runtime_t;

typedef struct {
    int   view_width;          /* 单眼宽，默认 1600 */
    int   view_height;         /* 单眼高，默认 1600 */
    int   sbs_width;           /* SBS 合成输出宽 = view_width*2 */
    int   sbs_height;          /* SBS 合成输出高 = view_height */
    int   refresh_hz;          /* 60/72/90 */
    float ipd_meters;          /* 瞳距，默认 0.063 */
    bool  position_tracking;   /* true=6DoF，false=3DoF */
} ga_view_config_t;

/* 创建 runtime。内部持有 pose tracker 与 controller hub。 */
ga_runtime_t *ga_runtime_create(const ga_view_config_t *cfg);
void ga_runtime_destroy(ga_runtime_t *r);

/* —— 数据源注入（供 JNI 线程调用）—— */
/* 手机 IMU 喂头部姿态 */
void ga_runtime_feed_head_imu(ga_runtime_t *r, const ga_imu_sample_t *s);
/* 头部 UDP 姿态报文（w x y z [px py pz valid]） */
bool ga_runtime_feed_head_udp(ga_runtime_t *r, const char *text, int len);
/* 手柄 IMU / 6DoF 位姿 / 输入 */
void ga_runtime_feed_controller_imu(ga_runtime_t *r, int hand, const ga_imu_sample_t *s);
bool ga_runtime_feed_controller_udp(ga_runtime_t *r, int hand, const char *text, int len);
void ga_runtime_feed_controller_input(ga_runtime_t *r, int hand,
                                      uint32_t buttons, float ax, float ay,
                                      float trigger, float grip);
void ga_runtime_set_controller_dof(ga_runtime_t *r, int hand, ga_controller_dof_t dof);
void ga_runtime_set_controller_connected(ga_runtime_t *r, int hand, bool connected);

/* 姿态源切换（手机IMU / UDP 注入） */
void ga_runtime_set_head_source(ga_runtime_t *r, ga_pose_source_t src);

/* —— 渲染循环读取（供 OpenXR xrLocateViews / xrEndFrame 调用）—— */
/* 取头部姿态用于视图矩阵；out_quat 为 OpenXR 顺序 {x,y,z,w} */
void ga_runtime_get_head_pose(ga_runtime_t *r, float out_quat_xyzw[4],
                              float out_pos[3], bool *out_pos_valid);
/* 取手柄状态用于 action space / 渲染 */
ga_controller_state_t ga_runtime_get_controller(const ga_runtime_t *r, int hand);
/* 取视图配置 */
ga_view_config_t ga_runtime_view_config(const ga_runtime_t *r);

/* recenter：重置头部与手柄姿态 */
void ga_runtime_recenter(ga_runtime_t *r);

#ifdef __cplusplus
}
#endif
#endif
