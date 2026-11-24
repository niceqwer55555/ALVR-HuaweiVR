#ifndef GENERIC_ALVR_DECODER_POLICY_H
#define GENERIC_ALVR_DECODER_POLICY_H

/*
 * 高通平台解码器选择策略 —— 完全去掉华为 OMX.hisi.* 依赖。
 *
 * 背景：参考仓库因 platform=HuaweiVrGlass 硬编码 OMX.hisi.video.decoder.avc，
 * 在骁龙设备上不存在该组件，退回软解 c2.android.avc.decoder 后拿不到硬件帧，
 * 导致眼镜纯色。本模块把选择逻辑显式化、可配置化：
 *   - 优先 Codec2 高通组件（Android 10+ 推荐）：c2.qti.avc.decoder
 *   - 兼容旧 OMX 组件                    ：OMX.qcom.video.decoder.avc
 *   - 可选用户覆盖（配置文件/设置下发）
 *   - 明确禁止软解（no_software_fallback）
 *
 * 本模块只负责"给出候选组件名与策略"，实际 MediaCodec 创建由 Java host
 * 或 Rust 客户端调用 MediaCodec API 完成。
 */
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef enum {
    DECODE_CODEC_H264 = 0,
    DECODE_CODEC_H265 = 1,
} ga_codec_t;

typedef struct {
    ga_codec_t codec;
    bool  allow_software;      /* 默认 false —— 用户要求不回退软解 */
    const char *override_name; /* 非空时优先使用（可为 NULL） */
    int   max_buffering_frames;
    bool  render_output_immediately;
    bool  retain_last_frame;
} ga_decoder_config_t;

/* 默认策略：H264 + 高通硬解 + 禁软解。 */
ga_decoder_config_t ga_decoder_default(ga_codec_t codec);

/* 返回按优先级排序的组件名数组指针与数量（静态字符串，无需释放）。
 * codec=H264: ["c2.qti.avc.decoder", "OMX.qcom.video.decoder.avc"]
 * codec=H265: ["c2.qti.hevc.decoder", "OMX.qcom.video.decoder.hevc"] */
const char *const *ga_decoder_candidates(ga_codec_t codec, int *out_count);

/* 判定一个组件名是否为软件解码器（用于实现"禁软解" gate）。 */
bool ga_decoder_is_software(const char *name);

/* 按策略选择第一个可用组件；available 为回调，返回该名称在设备上是否存在。
 * 若全部不可用且 allow_software=false，返回 NULL（调用方应报错而非静默软解）。 */
const char *ga_decoder_pick(const ga_decoder_config_t *cfg,
                            bool (*available)(const char *name, void *ctx),
                            void *ctx);

#ifdef __cplusplus
}
#endif
#endif
