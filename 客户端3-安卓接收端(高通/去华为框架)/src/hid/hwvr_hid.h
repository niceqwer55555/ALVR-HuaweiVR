#ifndef GENERIC_ALVR_HWVR_HID_H
#define GENERIC_ALVR_HWVR_HID_H

/*
 * VR 眼镜 HID 控制/输入通道 —— 通用实现（协议应答，不写死 VID/PID）。
 *
 * 来源：EnYaoJingYa/huawei-vr-compat 的逆向结论：
 *   "所谓华为私有驱动接口全部是标准 HID HIDIOCGFEATURE/HIDIOCSFEATURE 透传，
 *     没有任何华为私有内核 ABI。通用性靠协议应答挑接口，不靠写死 VID/PID。"
 *
 * 关键点：
 *   - 控制通道：HIDIOCGFEATURE / HIDIOCSFEATURE（Linux 标准 hidraw）
 *   - 输入通道：read() 持续收，buf[8] 是数据类型（2 接近 / 3 心跳 / 5 电量 / 6 耳机 / 7 过热）
 *   - 探测方式：发 report id=1 的 GET_FEATURE(4)，谁按协议应答谁就是控制接口
 *
 * 本模块只编码协议常量与报文解析（纯逻辑、可单测）；实际的 open/ioctl/read
 * 由 Android native 层执行，二者通过本模块的解析函数对接。
 */
#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* report id（设备侧约定，换设备时改这一层） */
#define GA_HID_REPORT_VENDOR   1   /* GET_FEATURE(4) -> 厂商应答 */
#define GA_HID_REPORT_VERSION  2   /* GET_FEATURE(62/80) */
#define GA_HID_REPORT_PROX     4   /* GET_FEATURE(1) 接近 */
#define GA_HID_REPORT_BATTERY  8   /* GET_FEATURE(8) 电量 */
#define GA_HID_REPORT_OPTICAL  9   /* GET_FEATURE(48/16) 光学/偏移 */
#define GA_HID_REPORT_ACTIVE   5   /* SET_FEATURE(5) 激活 */
#define GA_HID_REPORT_DEACTIVE 6   /* SET_FEATURE(5) 去激活 */

/* ioctl 常量（Linux hidraw 标准，_IOC(_IOC_READ|_IOC_WRITE,'H',nr,len)） */
#define GA_HIDIOCG_FEATURE(len) (0xC0000000u | ((len) << 16) | 0x4807u)
#define GA_HIDIOCS_FEATURE(len) (0xC0000000u | ((len) << 16) | 0x4806u)

/* 输入报告 buf[8] 的数据类型 */
typedef enum {
    GA_INPUT_PROXIMITY = 2,
    GA_INPUT_HEARTBEAT = 3,
    GA_INPUT_BATTERY   = 5,
    GA_INPUT_HEADSET   = 6,
    GA_INPUT_OVERHEAT  = 7,
} ga_input_type_t;

typedef struct {
    ga_input_type_t type;
    uint8_t raw_type;
    const uint8_t *payload;
    int payload_len;
} ga_input_report_t;

/* 解析一帧 read() 输入报告。格式：<8字节头>[8]=类型[9..]=负载。 */
bool ga_hid_parse_input(const uint8_t *buf, int len, ga_input_report_t *out);

/* 构造 SET_FEATURE 激活/去激活报文（5 字节：report id + device type & 0x7E）。 */
int ga_hid_build_active(uint8_t *buf, int buf_len, bool activate, uint8_t device_type);

/* 判断 GET_FEATURE 应答是否像眼镜控制接口（协议应答探测，不写死 VID/PID）：
 * 条件：ret>0 且 buf[0]==report id 且长度达标。 */
bool ga_hid_looks_like_helmet(const uint8_t *buf, int ret, uint8_t expect_report, int min_len);

/* 设备状态（去华为框架：电量用 BatteryManager、接近用 TYPE_PROXIMITY 等价） */
typedef struct {
    bool  proximity_near;     /* true=贴近（佩戴） */
    bool  proximity_valid;
    int   battery_percent;    /* 0..100；眼镜由手机供电时取手机电量 */
    bool  battery_valid;
    bool  headset_plugged;    /* 耳机插入（buf[8]=6） */
    bool  overheat;           /* 过热（buf[8]=7） */
    bool  heartbeat_ok;       /* 心跳（buf[8]=3）保活 */
} ga_device_status_t;

/* 把解析出的输入报告合并进设备状态。 */
void ga_status_apply_input(ga_device_status_t *st, const ga_input_report_t *rep);

#ifdef __cplusplus
}
#endif
#endif
