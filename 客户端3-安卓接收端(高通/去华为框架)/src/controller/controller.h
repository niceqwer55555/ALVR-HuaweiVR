#ifndef GENERIC_ALVR_CONTROLLER_H
#define GENERIC_ALVR_CONTROLLER_H

/*
 * 通用手柄聚合 —— 同时支持 3DoF 与 6DoF 手柄，零华为依赖。
 *
 * 数据来源（由 Java host / 桥接层喂入，与本模块解耦）：
 *   - 手机 IMU / 蓝牙手柄 IMU  : 3DoF，仅姿态（ga_pose_update_imu）
 *   - 外部 6DoF 源（ARCore/眼镜OV580/PC SLAM）: 位置通过 UDP 注入
 *   - 按键与摇杆               : 由 Android 输入事件映射成位掩码 + 轴
 *
 * 设计参考：参考仓库 controller.c 的 xr_controller_get_pose(hand,rot,pos)
 * 与 s_ctrl[hand].tracked 状态机，但去掉 libvrpose.so/dlopen 等华为耦合。
 */
#include <stdint.h>
#include <stdbool.h>
#include "pose.h"

#ifdef __cplusplus
extern "C" {
#endif

#define GA_MAX_CONTROLLERS 2
#define GA_BTN_TRIGGER     (1u << 0)
#define GA_BTN_GRIP        (1u << 1)
#define GA_BTN_MENU        (1u << 2)
#define GA_BTN_SYSTEM      (1u << 3)
#define GA_BTN_A           (1u << 4)
#define GA_BTN_B           (1u << 5)

typedef enum {
    CTRL_DOF_3 = 3,   /* 仅旋转，position_valid 恒 false */
    CTRL_DOF_6 = 6,   /* 旋转 + 位置 */
} ga_controller_dof_t;

typedef struct {
    bool      connected;
    ga_controller_dof_t dof;
    ga_pose_t pose;       /* rot + pos；3DoF 时 pos 忽略 */
    uint32_t  buttons;    /* GA_BTN_* 位掩码 */
    float     axis_x;     /* 触摸板/摇杆 X，[-1,1] */
    float     axis_y;     /* 触摸板/摇杆 Y，[-1,1] */
    float     trigger;    /* 扳机 [0,1] */
    float     grip;       /* 握持 [0,1] */
    int       battery_pct; /* 电量百分比，-1=未知（供 UI 指示灯显示） */
} ga_controller_state_t;

typedef struct ga_controller_hub ga_controller_hub_t;

ga_controller_hub_t *ga_controller_create(void);
void ga_controller_destroy(ga_controller_hub_t *h);

/* 设置手柄自由度；3DoF 手柄忽略后续注入的位置。 */
void ga_controller_set_dof(ga_controller_hub_t *h, int hand, ga_controller_dof_t dof);

/* 连接/断开。 */
void ga_controller_set_connected(ga_controller_hub_t *h, int hand, bool connected);

/* 喂手柄 IMU（陀螺+加速度），内部 Mahony 自算姿态。 */
void ga_controller_update_imu(ga_controller_hub_t *h, int hand, const ga_imu_sample_t *s);

/* 注入 6DoF 位姿文本（"w x y z px py pz valid"，来自外部 SLAM/眼镜桥）。 */
bool ga_controller_inject_pose_text(ga_controller_hub_t *h, int hand, const char *text, int len);

/* 更新按键/轴。 */
void ga_controller_set_input(ga_controller_hub_t *h, int hand,
                             uint32_t buttons, float axis_x, float axis_y,
                             float trigger, float grip);

/* 取状态快照。hand: 0=左 1=右。非法 hand 返回 connected=false。 */
ga_controller_state_t ga_controller_get(const ga_controller_hub_t *h, int hand);

/* 设置手柄电量（-1=未知），供 UI 指示灯显示。 */
void ga_controller_set_battery(ga_controller_hub_t *h, int hand, int battery_pct);

/* 重置某只手柄的姿态（recenter）。 */
void ga_controller_reset_pose(ga_controller_hub_t *h, int hand);

#ifdef __cplusplus
}
#endif
#endif
