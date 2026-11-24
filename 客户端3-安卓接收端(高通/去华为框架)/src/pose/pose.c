/*
 * 通用姿态解算实现 —— Mahony 互补滤波 + 双数据源。
 * 从华为眼镜 HID 私有实现中剥离出纯数学部分，改为接收标准 IMU 物理量。
 */
#include "pose.h"

#include <math.h>
#include <string.h>
#include <stdlib.h>

struct ga_pose_tracker {
    double kp, ki;
    ga_quat_t q;
    ga_vec3_t position;
    bool position_valid;
    ga_pose_source_t source;
    double integral[3];      /* Mahony 积分项（陀螺零偏估计） */
};

void ga_quat_mul(const ga_quat_t *a, const ga_quat_t *b, ga_quat_t *out) {
    ga_quat_t r;
    r.w = a->w * b->w - a->x * b->x - a->y * b->y - a->z * b->z;
    r.x = a->w * b->x + a->x * b->w + a->y * b->z - a->z * b->y;
    r.y = a->w * b->y - a->x * b->z + a->y * b->w + a->z * b->x;
    r.z = a->w * b->z + a->x * b->y - a->y * b->x + a->z * b->w;
    *out = r;
}

void ga_quat_normalize(ga_quat_t *q) {
    double n = sqrt(q->w * q->w + q->x * q->x + q->y * q->y + q->z * q->z);
    if (n < 1e-9) {
        q->w = 1; q->x = q->y = q->z = 0;
        return;
    }
    q->w /= n; q->x /= n; q->y /= n; q->z /= n;
}

ga_quat_t ga_quat_from_axis_angle(double x, double y, double z, double angle_rad) {
    ga_quat_t q;
    double n = sqrt(x * x + y * y + z * z);
    if (n < 1e-9) { q.w = 1; q.x = q.y = q.z = 0; return q; }
    double s = sin(angle_rad / 2.0) / n;
    q.w = cos(angle_rad / 2.0);
    q.x = x * s; q.y = y * s; q.z = z * s;
    return q;
}

ga_pose_tracker_t *ga_pose_create(double kp, double ki) {
    ga_pose_tracker_t *t = (ga_pose_tracker_t *)calloc(1, sizeof(*t));
    if (!t) return NULL;
    t->kp = kp;
    t->ki = ki;
    ga_pose_reset(t);
    t->source = POSE_SRC_PHONE_IMU;
    return t;
}

void ga_pose_destroy(ga_pose_tracker_t *t) {
    free(t);
}

void ga_pose_reset(ga_pose_tracker_t *t) {
    if (!t) return;
    t->q.w = 1; t->q.x = t->q.y = t->q.z = 0;
    t->position.x = t->position.y = t->position.z = 0;
    t->position_valid = false;
    t->integral[0] = t->integral[1] = t->integral[2] = 0;
}

void ga_pose_set_source(ga_pose_tracker_t *t, ga_pose_source_t src) {
    if (t) t->source = src;
}

ga_pose_source_t ga_pose_get_source(const ga_pose_tracker_t *t) {
    return t ? t->source : POSE_SRC_PHONE_IMU;
}

/*
 * 从当前四元数旋转的「机体系重力方向」与测得加速度做叉积，得到误差，
 * 用比例+积分项修正角速度。yaw 不可观（重力对偏航无约束），靠陀螺保持。
 */
static void mahony_correct(ga_pose_tracker_t *t, const ga_imu_sample_t *s,
                           double *gx, double *gy, double *gz) {
    double ax = s->ax, ay = s->ay, az = s->az;
    double an = sqrt(ax * ax + ay * ay + az * az);
    if (an < 0.5 || an > 30.0) return; /* 非 1g 环境丢弃（单位 m/s^2，约 9.8） */
    ax /= an; ay /= an; az /= an;

    const ga_quat_t *q = &t->q;
    /* 由四元数算出机体上方向在世界系的表示，再反投影回机体系的期望重力 */
    double vx = 2.0 * (q->x * q->z - q->w * q->y);
    double vy = 2.0 * (q->y * q->z + q->w * q->x);
    double vz = 1.0 - 2.0 * (q->x * q->x + q->y * q->y);

    double ex = ay * vz - az * vy;
    double ey = az * vx - ax * vz;
    double ez = ax * vy - ay * vx;

    if (t->ki > 0.0) {
        t->integral[0] += t->ki * ex * s->dt;
        t->integral[1] += t->ki * ey * s->dt;
        t->integral[2] += t->ki * ez * s->dt;
        *gx += t->integral[0];
        *gy += t->integral[1];
        *gz += t->integral[2];
    }
    *gx += t->kp * ex;
    *gy += t->kp * ey;
    *gz += t->kp * ez;
}

void ga_pose_update_imu(ga_pose_tracker_t *t, const ga_imu_sample_t *s) {
    if (!t || !s) return;
    if (t->source != POSE_SRC_PHONE_IMU) return; /* UDP 模式下暂停本地解算 */

    double dt = s->dt;
    if (dt < 0.0005 || dt > 0.1) dt = 0.01; /* 异常间隔钳制 */

    double gx = s->gx, gy = s->gy, gz = s->gz;
    /* 加速度存在且合理时才做重力校正；纯陀螺场景（acc=0）直接积分 */
    double an = sqrt(s->ax * s->ax + s->ay * s->ay + s->az * s->az);
    if (an >= 0.5 && an <= 30.0) {
        mahony_correct(t, s, &gx, &gy, &gz);
    }

    double wc = sqrt(gx * gx + gy * gy + gz * gz);
    ga_quat_t dq;
    if (wc > 1e-6) {
        /* 修正项并入角速度后再积分：q' = q ⊗ [cos(θ/2), axis·sin(θ/2)] */
        double half = wc * dt / 2.0;
        double sc = sin(half) / wc;
        dq.w = cos(half);
        dq.x = gx * sc; dq.y = gy * sc; dq.z = gz * sc;
    } else {
        dq.w = 1; dq.x = dq.y = dq.z = 0;
    }

    ga_quat_t out;
    ga_quat_mul(&t->q, &dq, &out);
    t->q = out;
    ga_quat_normalize(&t->q);
}

bool ga_pose_ingest_udp_text(ga_pose_tracker_t *t, const char *text, int len) {
    if (!t || !text || len <= 0) return false;
    double v[8] = {0};
    int n = 0;
    /* 手写解析，避免依赖 sscanf 的 locale/分配行为，兼容 "\n" 结尾 */
    const char *p = text;
    const char *end = text + len;
    while (p < end && n < 8) {
        while (p < end && (*p == ' ' || *p == '\t' || *p == '\n' || *p == '\r')) ++p;
        if (p >= end) break;
        char *next = NULL;
        double d = strtod(p, &next);
        if (next == p) return false;
        v[n++] = d;
        p = next;
    }
    if (n < 4) return false;

    ga_quat_t q = {v[0], v[1], v[2], v[3]};
    ga_quat_normalize(&q);
    t->q = q;

    if (n >= 8) {
        t->position.x = v[4];
        t->position.y = v[5];
        t->position.z = v[6];
        t->position_valid = (v[7] != 0.0);
    } else {
        t->position_valid = false;
    }
    return true;
}

ga_pose_t ga_pose_get(const ga_pose_tracker_t *t) {
    ga_pose_t out;
    if (!t) {
        out.q.w = 1; out.q.x = out.q.y = out.q.z = 0;
        out.position.x = out.position.y = out.position.z = 0;
        out.position_valid = out.orientation_valid = false;
        return out;
    }
    out.q = t->q;
    out.position = t->position;
    out.position_valid = t->position_valid;
    out.orientation_valid = true;
    return out;
}
