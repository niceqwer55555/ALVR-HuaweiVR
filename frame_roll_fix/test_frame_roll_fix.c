/* 画面位置修复单测：gcc/mingw 均可编译运行 */
#include "frame_roll_fix.h"
#include <stdio.h>
#include <math.h>

static int pass = 0, fail = 0;
#define CHECK(c, msg) do { if (c) { printf("  [OK]   %s\n", msg); pass++; } \
                           else { printf("  [FAIL] %s\n", msg); fail++; } } while (0)

int main(void)
{
    printf("=== 画面位置修复 · 核心单测 ===\n");

    FrConfig cfg;
    fr_config_default(&cfg);
    CHECK(cfg.enabled == 1, "修复默认启用");
    CHECK(cfg.stage == FR_STAGE_PM, "默认 stage = (+90,-90) CV10");

    /* roll 角 */
    float l, r;
    fr_roll_angles(FR_STAGE_PM, &l, &r);
    CHECK(fabsf(l - 90) < 0.01f && fabsf(r + 90) < 0.01f, "PM: 左+90 右-90");
    fr_roll_angles(FR_STAGE_PP, &l, &r);
    CHECK(l > 0 && r > 0, "PP: 两眼同向+90");

    /* stage 循环与 HwVRGlassLite 一致 */
    CHECK(fr_next_stage(FR_STAGE_PP) == FR_STAGE_MP, "循环 PP→MP");
    CHECK(fr_next_stage(FR_STAGE_MP) == FR_STAGE_PM, "循环 MP→PM");
    CHECK(fr_next_stage(FR_STAGE_PM) == FR_STAGE_MM, "循环 PM→MM");
    CHECK(fr_next_stage(FR_STAGE_MM) == FR_STAGE_PP, "循环 MM→PP 闭环");

    /* 每眼最终角度 + swap */
    CHECK(fabsf(fr_eye_roll(&cfg, 0) - 90) < 0.01f, "左眼最终 +90");
    CHECK(fabsf(fr_eye_roll(&cfg, 1) + 90) < 0.01f, "右眼最终 -90");
    cfg.swap_eyes = 1;
    CHECK(fabsf(fr_eye_roll(&cfg, 0) + 90) < 0.01f, "swap 后左眼取到右眼角 -90");
    cfg.swap_eyes = 0;
    cfg.enabled = 0;
    CHECK(fr_eye_roll(&cfg, 0) == 0.0f, "禁用后旁路=0（普通串流）");
    cfg.enabled = 1;

    /* UV 旋转：+90° 应把左上角(0,0)映到右下(1,1)附近；中心不变 */
    float ou, ov;
    fr_rotate_uv(0.5f, 0.5f, 90.0f, &ou, &ov);
    CHECK(fabsf(ou - 0.5f) < 1e-5f && fabsf(ov - 0.5f) < 1e-5f, "+90 中心点不动");
    fr_rotate_uv(0.0f, 0.5f, 90.0f, &ou, &ov);
    CHECK(fabsf(ou - 0.5f) < 1e-5f && fabsf(ov - 1.0f) < 1e-5f,
          "+90 左边中点→下边中点");
    fr_rotate_uv(0.0f, 0.5f, -90.0f, &ou, &ov);
    CHECK(fabsf(ou - 0.5f) < 1e-5f && fabsf(ov - 0.0f) < 1e-5f,
          "-90 左边中点→上边中点");

    /* SBS 分眼 UV */
    float uv[4];
    fr_eye_uv_rect(0, 0, uv);
    CHECK(uv[0] == 0 && uv[2] == 0.5f, "左眼 SBS UV [0,0.5]");
    fr_eye_uv_rect(1, 0, uv);
    CHECK(uv[0] == 0.5f && uv[2] == 1.0f, "右眼 SBS UV [0.5,1]");

    /* CPU 旋转：4x4 图，+90 后原左上角应出现在右上角 */
    uint8_t src[4*4*4], dst[4*4*4];
    for (int i = 0; i < 4*4*4; ++i) src[i] = 0;
    src[0] = 255; /* 源(0,0) 红 */
    fr_rotate_image_rgba(src, dst, 4, 90.0f);
    /* +90: 源左上→目标右上 (x=3,y=0) */
    CHECK(dst[(0*4+3)*4] == 255, "+90 源左上角转到右上角");

    printf("\nPASS=%d FAIL=%d\n", pass, fail);
    return fail ? 1 : 0;
}
