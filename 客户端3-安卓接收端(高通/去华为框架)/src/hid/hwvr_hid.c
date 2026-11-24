/* VR 眼镜 HID 协议解析（协议应答式，去 VID/PID 写死）。 */
#include "hwvr_hid.h"
#include <string.h>

bool ga_hid_parse_input(const uint8_t *buf, int len, ga_input_report_t *out) {
    if (!buf || !out || len < 9) return false; /* 至少 8 字节头 + 1 字节类型 */
    uint8_t t = buf[8];
    out->raw_type = t;
    switch (t) {
        case GA_INPUT_PROXIMITY: out->type = GA_INPUT_PROXIMITY; break;
        case GA_INPUT_HEARTBEAT: out->type = GA_INPUT_HEARTBEAT; break;
        case GA_INPUT_BATTERY:   out->type = GA_INPUT_BATTERY;   break;
        case GA_INPUT_HEADSET:   out->type = GA_INPUT_HEADSET;   break;
        case GA_INPUT_OVERHEAT:  out->type = GA_INPUT_OVERHEAT;  break;
        default: return false; /* 未知类型不当作有效报告 */
    }
    out->payload = buf + 9;
    out->payload_len = len - 9;
    return true;
}

int ga_hid_build_active(uint8_t *buf, int buf_len, bool activate, uint8_t device_type) {
    if (!buf || buf_len < 5) return -1;
    memset(buf, 0, 5);
    buf[0] = activate ? GA_HID_REPORT_ACTIVE : GA_HID_REPORT_DEACTIVE;
    buf[1] = device_type & 0x7E; /* 与 libWalleDev::ioActiveDev 一致 */
    return 5;
}

bool ga_hid_looks_like_helmet(const uint8_t *buf, int ret, uint8_t expect_report, int min_len) {
    if (!buf || ret <= 0 || ret < min_len) return false;
    return buf[0] == expect_report; /* 首字节是 report id，按协议应答即认定 */
}

void ga_status_apply_input(ga_device_status_t *st, const ga_input_report_t *rep) {
    if (!st || !rep) return;
    switch (rep->type) {
        case GA_INPUT_PROXIMITY:
            st->proximity_valid = true;
            /* 负载首字节：0=near（贴近），非0=far；与 Android TYPE_PROXIMITY 语义对齐 */
            st->proximity_near = rep->payload_len > 0 && rep->payload[0] == 0;
            break;
        case GA_INPUT_BATTERY:
            st->battery_valid = rep->payload_len > 0;
            if (st->battery_valid) {
                int v = rep->payload[0];
                st->battery_percent = v <= 100 ? v : 100;
            }
            break;
        case GA_INPUT_HEADSET:
            st->headset_plugged = rep->payload_len > 0 && rep->payload[0] != 0;
            break;
        case GA_INPUT_OVERHEAT:
            st->overheat = rep->payload_len > 0 && rep->payload[0] != 0;
            break;
        case GA_INPUT_HEARTBEAT:
            st->heartbeat_ok = true;
            break;
        default:
            break;
    }
}
