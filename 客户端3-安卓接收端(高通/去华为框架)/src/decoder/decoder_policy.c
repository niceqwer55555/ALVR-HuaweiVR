/* 高通解码器策略实现：去华为化、禁软解、优先级回退。 */
#include "decoder_policy.h"
#include <string.h>

static const char *const H264_CANDIDATES[] = {
    "c2.qti.avc.decoder",         /* Android 10+ Codec2，首选 */
    "OMX.qcom.video.decoder.avc", /* 旧 OMX 兼容 */
};
static const char *const H265_CANDIDATES[] = {
    "c2.qti.hevc.decoder",
    "OMX.qcom.video.decoder.hevc",
};
/* 允许的软解兜底 —— 仅在 allow_software=true 时才会进入候选链末尾 */
static const char *const H264_SOFTWARE[] = { "c2.android.avc.decoder" };
static const char *const H265_SOFTWARE[] = { "c2.android.hevc.decoder" };

ga_decoder_config_t ga_decoder_default(ga_codec_t codec) {
    ga_decoder_config_t c;
    c.codec = codec;
    c.allow_software = false;       /* 用户明确要求：不回退软解 */
    c.override_name = NULL;
    c.max_buffering_frames = 3;     /* 硬件输出池小，与参考仓库 hisi 分支一致 */
    c.render_output_immediately = true;
    c.retain_last_frame = true;
    return c;
}

const char *const *ga_decoder_candidates(ga_codec_t codec, int *out_count) {
    if (out_count) {
        *out_count = codec == DECODE_CODEC_H265 ? 2 : 2;
    }
    return codec == DECODE_CODEC_H265 ? H265_CANDIDATES : H264_CANDIDATES;
}

bool ga_decoder_is_software(const char *name) {
    if (!name) return false;
    /* 已知软解组件特征：android 通用 C2 / google / software */
    if (strstr(name, "c2.android.") == name) return true;
    if (strstr(name, "google") != NULL) return true;
    if (strstr(name, "software") != NULL) return true;
    return false;
}

const char *ga_decoder_pick(const ga_decoder_config_t *cfg,
                            bool (*available)(const char *name, void *ctx),
                            void *ctx) {
    if (!cfg || !available) return NULL;

    /* 1) 用户覆盖优先（但软解覆盖仍受 allow_software gate 约束） */
    if (cfg->override_name && available(cfg->override_name, ctx)) {
        if (!ga_decoder_is_software(cfg->override_name) || cfg->allow_software) {
            return cfg->override_name;
        }
    }

    /* 2) 高通硬解候选链 */
    int n = 0;
    const char *const *cands = ga_decoder_candidates(cfg->codec, &n);
    for (int i = 0; i < n; ++i) {
        if (available(cands[i], ctx)) return cands[i];
    }

    /* 3) 软解兜底 —— 默认关闭；用户要求禁软解，命中即返回 NULL */
    if (cfg->allow_software) {
        const char *const *sw = cfg->codec == DECODE_CODEC_H265 ? H265_SOFTWARE : H264_SOFTWARE;
        for (int i = 0; i < 1; ++i) { /* 软解兜底数组各含 1 个组件 */
            if (available(sw[i], ctx)) return sw[i];
        }
    }
    return NULL; /* 无可用硬解且禁软解 → 失败，调用方报错 */
}
