/*
 * frame_roll_fix.h — 华为 VR Glass 服务端「有线投屏画面位置」修复核心
 *
 * 问题根因（实证来源：fkun1754/HwVRGlassLite desktop-mode/src/main.cpp:778）:
 *   华为 VR Glass(CV10) 物理面板是【两块 1600x1600 正方形】，左眼那块顺时针
 *   转了 90°(CW)、右眼逆时针 90°(CCW) 安装（排线方向所致）。
 *   ALVR 服务端 DesktopCapture 把桌面贴到两块方形画布、RenderCastFrame 用
 *   bounds={0,0,1,1} + 单位 pose 直送编码，完全没有补偿这个物理旋转，
 *   结果用户在眼镜里看到的投屏画面【两眼各躺倒 90°且方向相反】。
 *
 * 修复：送显/送编码前，对左右眼画布各做一次反向 roll（默认左 +90°/右 -90°）。
 *   因为单眼画布是正方形，±90° 旋转不改变尺寸，纯 UV 变换即可，代价为零。
 *   HwVRGlassLite 提供 4 种组合循环校准不同批次面板（stage 0..3）。
 *
 * 本模块是纯 C、无平台/无华为库依赖：
 *   - 计算每眼的 roll 角与旋转后的 UV 映射（供 D3D11 像素着色器采样）
 *   - 提供 CPU 参考实现用于单测验证（gcc 可编译运行）
 *   - 服务端 CEncoder.cpp 接入方式见 patches/README.md
 */
#ifndef FRAME_ROLL_FIX_H
#define FRAME_ROLL_FIX_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define FR_EYE_PANEL 1600u   /* 单眼正方形面板边长 */
#define FR_FULL_W    3200u   /* 合眼整屏宽 */
#define FR_FULL_H    1600u   /* 合眼整屏高 */

/* HwVRGlassLite 的 4 种 roll 组合（main.cpp:248-259），顺序一致 */
typedef enum {
    FR_STAGE_PP = 0, /* (+90, +90) */
    FR_STAGE_MP = 1, /* (-90, +90) */
    FR_STAGE_PM = 2, /* (+90, -90) ← CV10 常见默认 */
    FR_STAGE_MM = 3  /* (-90, -90) */
} FrRollStage;

typedef struct {
    FrRollStage stage;
    int         swap_eyes;      /* 左右眼互换 */
    int         enabled;        /* 0 = 旁路（普通串流不需要） */
    float       roll_override[2]; /* 非 0 覆盖 stage 默认，精细微调 */
} FrConfig;

/* 默认：启用，CV10 常见 (+90,-90) */
void fr_config_default(FrConfig *cfg);

/* 取 stage 的左右补偿角（度） */
void fr_roll_angles(FrRollStage stage, float *left, float *right);

/* stage 循环（用于面板批次校准） */
FrRollStage fr_next_stage(FrRollStage stage);

/* 取某只眼最终 roll 角（含 override 与 swap） */
float fr_eye_roll(const FrConfig *cfg, int eye_index);

/*
 * 计算旋转后的 UV 映射：给定源 UV (u,v)，返回采样坐标 (ou,ov)。
 * 用于把"躺倒"的画面转正：目标像素 (u,v) 去源图 (ou,ov) 取色。
 * angle 为 fr_eye_roll 返回值（度，应为 ±90 的倍数）。
 */
void fr_rotate_uv(float u, float v, float angle, float *ou, float *ov);

/* SBS 分眼：某只眼在 3200x1600 合眼画面里的 UV 子矩形 [u0,v0,u1,v1] */
void fr_eye_uv_rect(int eye_index, int swap_eyes, float *uv);

/* CPU 参考实现：把正方形 RGBA 图按 angle 旋转（row-major，w=h=n），用于单测 */
void fr_rotate_image_rgba(
    const uint8_t *src, uint8_t *dst, uint32_t n, float angle);

#ifdef __cplusplus
}
#endif
#endif /* FRAME_ROLL_FIX_H */
