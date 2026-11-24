#ifndef GENERIC_ALVR_POSE_H
#define GENERIC_ALVR_POSE_H

/*
 * 通用头部/手柄姿态解算 —— 零华为依赖、零平台依赖的纯 C 实现。
 *
 * 设计来源：对 HuaweiVRGlass-ALVR-Android 仓库 hwvrctl.c 中 Mahony 互补滤波的
 * 通用化重写。原实现绑定华为眼镜 HID report（12D1:107E）与私有偏移；本模块
 * 剥掉所有设备耦合，只接收"角速度 + 加速度"物理量，输出单位四元数。
 *
 * 支持的姿态输入源（运行时可切换，见 pose_source_t）：
 *   - POSE_SRC_PHONE_IMU : Android TYPE_ROTATION_VECTOR / TYPE_GYROSCOPE +
 *                          TYPE_ACCELEROMETER（由 Java host 经 JNI 喂入）
 *   - POSE_SRC_UDP       : localhost UDP 四元数/8 值报文（外部 SLAM/眼镜 HID 桥）
 *
 * 坐标系：世界系 Y 轴向上（与 OpenXR stage 空间一致），四元数按 [w,x,y,z] 存储，
 * 与 OpenXR XrQuaternionf 的 {x,y,z,w} 仅内存序不同，提交时转换。
 */

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct {
    double w, x, y, z;
} ga_quat_t;

typedef struct {
    double x, y, z;
} ga_vec3_t;

typedef enum {
    POSE_SRC_PHONE_IMU = 0,   /* 手机 IMU，本地 Mahony 解算 */
    POSE_SRC_UDP       = 1,   /* 外部 UDP 注入，直接使用收到的四元数 */
} ga_pose_source_t;

typedef struct {
    double gx, gy, gz;        /* 角速度 rad/s（机体系） */
    double ax, ay, az;        /* 加速度 m/s^2（机体系，含重力） */
    double dt;                /* 采样间隔秒 */
} ga_imu_sample_t;

typedef struct {
    ga_quat_t q;              /* 当前姿态 [w,x,y,z] */
    ga_vec3_t position;       /* 位置；3DoF 时恒为 0 */
    bool      position_valid; /* 6DoF 位置有效标志 */
    bool      orientation_valid;
} ga_pose_t;

typedef struct ga_pose_tracker ga_pose_tracker_t;

/* 创建/销毁解算器。比例增益 kp、积分增益 ki 为 Mahony 参数，推荐 2.0 / 0.0（3DoF 位置不可观）。 */
ga_pose_tracker_t *ga_pose_create(double kp, double ki);
void ga_pose_destroy(ga_pose_tracker_t *t);

/* 设置数据源。切到 UDP 后本地 IMU 积分暂停，直到切回 PHONE_IMU。 */
void ga_pose_set_source(ga_pose_tracker_t *t, ga_pose_source_t src);
ga_pose_source_t ga_pose_get_source(const ga_pose_tracker_t *t);

/* —— 手机 IMU 通道 —— */
/* 喂一帧 IMU 样本，内部做「陀螺积分 + 加速度重力校正」Mahony 更新。 */
void ga_pose_update_imu(ga_pose_tracker_t *t, const ga_imu_sample_t *s);

/* —— UDP 注入通道 —— */
/* 解析 "w x y z [px py pz valid]" 文本报文；8 值且 valid=1 时启用 6DoF 位置。 */
bool ga_pose_ingest_udp_text(ga_pose_tracker_t *t, const char *text, int len);

/* 取当前姿态快照（线程安全由调用方在单线程渲染循环内保证；多源并发见 docs）。 */
ga_pose_t ga_pose_get(const ga_pose_tracker_t *t);

/* 重置姿态到单位四元数（用于 recenter）。 */
void ga_pose_reset(ga_pose_tracker_t *t);

/* —— 数学工具（导出供手柄模块与单测复用）—— */
void ga_quat_mul(const ga_quat_t *a, const ga_quat_t *b, ga_quat_t *out);
void ga_quat_normalize(ga_quat_t *q);
ga_quat_t ga_quat_from_axis_angle(double x, double y, double z, double angle_rad);

#ifdef __cplusplus
}
#endif
#endif
