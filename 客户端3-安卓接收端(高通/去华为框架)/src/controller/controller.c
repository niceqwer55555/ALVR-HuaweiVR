/* 通用手柄聚合实现：每只手柄一个姿态解算器 + 输入状态。 */
#include "controller.h"
#include <stdlib.h>
#include <string.h>
#include <stdio.h>

struct ga_controller_hub {
    ga_pose_tracker_t *pose[GA_MAX_CONTROLLERS];
    ga_controller_state_t st[GA_MAX_CONTROLLERS];
};

ga_controller_hub_t *ga_controller_create(void) {
    ga_controller_hub_t *h = (ga_controller_hub_t *)calloc(1, sizeof(*h));
    if (!h) return NULL;
    for (int i = 0; i < GA_MAX_CONTROLLERS; ++i) {
        h->pose[i] = ga_pose_create(2.0, 0.0);
        h->st[i].dof = CTRL_DOF_3;
        h->st[i].connected = false;
        h->st[i].battery_pct = -1;
    }
    return h;
}

void ga_controller_destroy(ga_controller_hub_t *h) {
    if (!h) return;
    for (int i = 0; i < GA_MAX_CONTROLLERS; ++i) ga_pose_destroy(h->pose[i]);
    free(h);
}

static bool valid_hand(int hand) { return hand >= 0 && hand < GA_MAX_CONTROLLERS; }

void ga_controller_set_dof(ga_controller_hub_t *h, int hand, ga_controller_dof_t dof) {
    if (!h || !valid_hand(hand)) return;
    h->st[hand].dof = dof;
    if (dof == CTRL_DOF_3) {
        ga_pose_t p = ga_pose_get(h->pose[hand]);
        p.position.x = p.position.y = p.position.z = 0;
        p.position_valid = false;
        /* 位置 Validity 由 tracker 内部持有，3DoF 下注入文本会被忽略 */
    }
}

void ga_controller_set_connected(ga_controller_hub_t *h, int hand, bool connected) {
    if (!h || !valid_hand(hand)) return;
    h->st[hand].connected = connected;
    if (!connected) {
        memset(&h->st[hand].buttons, 0, sizeof(uint32_t));
        h->st[hand].axis_x = h->st[hand].axis_y = 0;
        h->st[hand].trigger = h->st[hand].grip = 0;
    }
}

void ga_controller_update_imu(ga_controller_hub_t *h, int hand, const ga_imu_sample_t *s) {
    if (!h || !valid_hand(hand)) return;
    ga_pose_update_imu(h->pose[hand], s);
    h->st[hand].pose = ga_pose_get(h->pose[hand]);
    if (h->st[hand].dof == CTRL_DOF_3) h->st[hand].pose.position_valid = false;
}

bool ga_controller_inject_pose_text(ga_controller_hub_t *h, int hand, const char *text, int len) {
    if (!h || !valid_hand(hand)) return false;
    if (h->st[hand].dof == CTRL_DOF_3) {
        /* 3DoF 手柄：只取四元数，忽略位置字段 */
        double q[4] = {0};
        int n = 0;
        const char *p = text, *end = text + len;
        while (p < end && n < 4) {
            while (p < end && (*p == ' ' || *p == '\t' || *p == '\n' || *p == '\r')) ++p;
            if (p >= end) break;
            char *nx = NULL;
            double d = strtod(p, &nx);
            if (nx == p) return false;
            q[n++] = d;
            p = nx;
        }
        if (n < 4) return false;
        ga_quat_t qq = {q[0], q[1], q[2], q[3]};
        ga_quat_normalize(&qq);
        /* 直接设置 tracker 姿态（绕过位置） */
        char buf[128];
        int m = snprintf(buf, sizeof buf, "%.6f %.6f %.6f %.6f", qq.w, qq.x, qq.y, qq.z);
        if (!ga_pose_ingest_udp_text(h->pose[hand], buf, m)) return false;
    } else {
        if (!ga_pose_ingest_udp_text(h->pose[hand], text, len)) return false;
    }
    h->st[hand].pose = ga_pose_get(h->pose[hand]);
    if (h->st[hand].dof == CTRL_DOF_3) h->st[hand].pose.position_valid = false;
    return true;
}

void ga_controller_set_input(ga_controller_hub_t *h, int hand,
                             uint32_t buttons, float axis_x, float axis_y,
                             float trigger, float grip) {
    if (!h || !valid_hand(hand)) return;
    h->st[hand].buttons = buttons;
    h->st[hand].axis_x = axis_x;
    h->st[hand].axis_y = axis_y;
    h->st[hand].trigger = trigger;
    h->st[hand].grip = grip;
}

ga_controller_state_t ga_controller_get(const ga_controller_hub_t *h, int hand) {
    ga_controller_state_t empty;
    memset(&empty, 0, sizeof empty);
    if (!h || !valid_hand(hand)) return empty;
    ga_controller_state_t st = h->st[hand];
    st.pose = ga_pose_get(h->pose[hand]);
    if (st.dof == CTRL_DOF_3) st.pose.position_valid = false;
    return st;
}

void ga_controller_set_battery(ga_controller_hub_t *h, int hand, int battery_pct) {
    if (!h || !valid_hand(hand)) return;
    h->st[hand].battery_pct = battery_pct < 0 ? -1 : (battery_pct > 100 ? 100 : battery_pct);
}

void ga_controller_reset_pose(ga_controller_hub_t *h, int hand) {
    if (!h || !valid_hand(hand)) return;
    ga_pose_reset(h->pose[hand]);
    h->st[hand].pose = ga_pose_get(h->pose[hand]);
}
