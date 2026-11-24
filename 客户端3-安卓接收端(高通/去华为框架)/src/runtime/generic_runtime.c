/*
 * 通用 OpenXR runtime 的平台桥实现。
 * 这里只做"数据源 → core 模块 → OpenXR 可消费姿态"的桥接；真正的 OpenXR
 * 入口点（xrCreateInstance ... xrEndFrame）在 runtime_openxr.c 中实现，
 * 二者通过本结构共享状态。去华为化：无 dlopen/libvrsdk.so/XR_HUAWEI_*。
 */
#include "generic_runtime.h"
#include <stdlib.h>
#include <string.h>

struct ga_runtime {
    ga_pose_tracker_t *head;
    ga_controller_hub_t *ctrl;
    ga_view_config_t cfg;
};

ga_runtime_t *ga_runtime_create(const ga_view_config_t *cfg) {
    ga_runtime_t *r = (ga_runtime_t *)calloc(1, sizeof(*r));
    if (!r) return NULL;
    r->head = ga_pose_create(2.0, 0.0);
    r->ctrl = ga_controller_create();
    if (cfg) {
        r->cfg = *cfg;
    } else {
        r->cfg.view_width = 1600;
        r->cfg.view_height = 1600;
        r->cfg.sbs_width = 3200;
        r->cfg.sbs_height = 1600;
        r->cfg.refresh_hz = 72;
        r->cfg.ipd_meters = 0.063f;
        r->cfg.position_tracking = false;
    }
    /* 保证 SBS 尺寸与单眼一致 */
    r->cfg.sbs_width = r->cfg.view_width * 2;
    r->cfg.sbs_height = r->cfg.view_height;
    return r;
}

void ga_runtime_destroy(ga_runtime_t *r) {
    if (!r) return;
    ga_pose_destroy(r->head);
    ga_controller_destroy(r->ctrl);
    free(r);
}

void ga_runtime_feed_head_imu(ga_runtime_t *r, const ga_imu_sample_t *s) {
    if (r) ga_pose_update_imu(r->head, s);
}
bool ga_runtime_feed_head_udp(ga_runtime_t *r, const char *text, int len) {
    return r ? ga_pose_ingest_udp_text(r->head, text, len) : false;
}
void ga_runtime_feed_controller_imu(ga_runtime_t *r, int hand, const ga_imu_sample_t *s) {
    if (r) ga_controller_update_imu(r->ctrl, hand, s);
}
bool ga_runtime_feed_controller_udp(ga_runtime_t *r, int hand, const char *text, int len) {
    return r ? ga_controller_inject_pose_text(r->ctrl, hand, text, len) : false;
}
void ga_runtime_feed_controller_input(ga_runtime_t *r, int hand,
                                      uint32_t buttons, float ax, float ay,
                                      float trigger, float grip) {
    if (r) ga_controller_set_input(r->ctrl, hand, buttons, ax, ay, trigger, grip);
}
void ga_runtime_set_controller_dof(ga_runtime_t *r, int hand, ga_controller_dof_t dof) {
    if (r) ga_controller_set_dof(r->ctrl, hand, dof);
}
void ga_runtime_set_controller_connected(ga_runtime_t *r, int hand, bool connected) {
    if (r) ga_controller_set_connected(r->ctrl, hand, connected);
}
void ga_runtime_set_head_source(ga_runtime_t *r, ga_pose_source_t src) {
    if (r) ga_pose_set_source(r->head, src);
}

void ga_runtime_get_head_pose(ga_runtime_t *r, float out_quat_xyzw[4],
                              float out_pos[3], bool *out_pos_valid) {
    ga_pose_t p = ga_pose_get(r ? r->head : NULL);
    /* core 存 [w,x,y,z]；OpenXR XrQuaternionf 是 {x,y,z,w} */
    out_quat_xyzw[0] = (float)p.q.x;
    out_quat_xyzw[1] = (float)p.q.y;
    out_quat_xyzw[2] = (float)p.q.z;
    out_quat_xyzw[3] = (float)p.q.w;
    out_pos[0] = (float)p.position.x;
    out_pos[1] = (float)p.position.y;
    out_pos[2] = (float)p.position.z;
    if (out_pos_valid) *out_pos_valid = p.position_valid;
}

ga_controller_state_t ga_runtime_get_controller(const ga_runtime_t *r, int hand) {
    return r ? ga_controller_get(r->ctrl, hand) : ga_controller_get(NULL, hand);
}

ga_view_config_t ga_runtime_view_config(const ga_runtime_t *r) {
    if (!r) {
        ga_view_config_t d = {1600,1600,3200,1600,72,0.063f,false};
        return d;
    }
    return r->cfg;
}

void ga_runtime_recenter(ga_runtime_t *r) {
    if (!r) return;
    ga_pose_reset(r->head);
    ga_controller_reset_pose(r->ctrl, 0);
    ga_controller_reset_pose(r->ctrl, 1);
}
