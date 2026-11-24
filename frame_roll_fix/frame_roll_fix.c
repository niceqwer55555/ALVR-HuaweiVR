/*
 * frame_roll_fix.c — 华为 VR Glass 服务端投屏画面位置修复核心实现
 * 纯 C99。证据与原理见 frame_roll_fix.h 头部。
 */
#include "frame_roll_fix.h"

#include <math.h>
#include <string.h>

#ifndef M_PI
#define M_PI 3.14159265358979323846
#endif

void fr_config_default(FrConfig *cfg)
{
    if (!cfg) return;
    cfg->stage = FR_STAGE_PM;   /* (+90,-90) CV10 常见默认 */
    cfg->swap_eyes = 0;
    cfg->enabled = 1;
    cfg->roll_override[0] = 0.0f;
    cfg->roll_override[1] = 0.0f;
}

void fr_roll_angles(FrRollStage stage, float *left, float *right)
{
    float l = 90.0f, r = -90.0f;
    switch (stage) {
    case FR_STAGE_PP: l =  90.0f; r =  90.0f; break;
    case FR_STAGE_MP: l = -90.0f; r =  90.0f; break;
    case FR_STAGE_PM: l =  90.0f; r = -90.0f; break;
    case FR_STAGE_MM: l = -90.0f; r = -90.0f; break;
    }
    if (left)  *left  = l;
    if (right) *right = r;
}

FrRollStage fr_next_stage(FrRollStage stage)
{
    /* 与 HwVRGlassLite DM_A_EYE_ROLL_CYCLE 完全一致的循环顺序 */
    return (FrRollStage)(((int)stage + 1) & 3);
}

float fr_eye_roll(const FrConfig *cfg, int eye_index)
{
    if (!cfg || !cfg->enabled) return 0.0f;
    int e = eye_index ? 1 : 0;
    if (cfg->swap_eyes) e = 1 - e;
    float l = 0.0f, r = 0.0f;
    fr_roll_angles(cfg->stage, &l, &r);
    float angle = e ? r : l;
    if (cfg->roll_override[e] != 0.0f) angle = cfg->roll_override[e];
    return angle;
}

void fr_rotate_uv(float u, float v, float angle, float *ou, float *ov)
{
    /* 以纹理中心 (0.5,0.5) 为轴旋转。
       目标像素 (u,v) 采源图时应取反方向旋转后的坐标：
         a = -angle
         ou = 0.5 + (u-0.5)*cos a - (v-0.5)*sin a
         ov = 0.5 + (u-0.5)*sin a + (v-0.5)*cos a
       对 ±90°，cos≈0，sin=±1，退化为轴交换，无插值误差。 */
    float a = -angle * (float)(M_PI / 180.0);
    float c = cosf(a), s = sinf(a);
    float du = u - 0.5f, dv = v - 0.5f;
    /* 抑制浮点残差：把接近 0 的 cos 归零，接近 ±1 的 sin 归一 */
    if (fabsf(c) < 1e-6f) c = 0.0f;
    if (fabsf(s - 1.0f) < 1e-6f) s = 1.0f;
    if (fabsf(s + 1.0f) < 1e-6f) s = -1.0f;
    if (fabsf(s) < 1e-6f) s = 0.0f;
    float no = 0.5f + du * c - dv * s;
    float nv = 0.5f + du * s + dv * c;
    if (ou) *ou = no;
    if (ov) *ov = nv;
}

void fr_eye_uv_rect(int eye_index, int swap_eyes, float *uv)
{
    if (!uv) return;
    int e = eye_index ? 1 : 0;
    if (swap_eyes) e = 1 - e;
    uv[0] = 0.5f * (float)e;
    uv[1] = 0.0f;
    uv[2] = 0.5f * (float)(e + 1);
    uv[3] = 1.0f;
}

void fr_rotate_image_rgba(const uint8_t *src, uint8_t *dst, uint32_t n, float angle)
{
    /* CPU 参考：dst(x,y) = src(R(-angle) 映射点)。正方形、RGBA、row-major。 */
    for (uint32_t y = 0; y < n; ++y) {
        for (uint32_t x = 0; x < n; ++x) {
            float u = (float)(x + 0.5f) / (float)n;
            float v = (float)(y + 0.5f) / (float)n;
            float ou, ov;
            fr_rotate_uv(u, v, angle, &ou, &ov);
            int sx = (int)(ou * (float)n);
            int sy = (int)(ov * (float)n);
            if (sx < 0) sx = 0;
            if (sy < 0) sy = 0;
            if (sx >= (int)n) sx = (int)n - 1;
            if (sy >= (int)n) sy = (int)n - 1;
            uint32_t d = (y * n + x) * 4;
            uint32_t s = (uint32_t)(sy * (int)n + sx) * 4;
            dst[d + 0] = src[s + 0];
            dst[d + 1] = src[s + 1];
            dst[d + 2] = src[s + 2];
            dst[d + 3] = src[s + 3];
        }
    }
}
