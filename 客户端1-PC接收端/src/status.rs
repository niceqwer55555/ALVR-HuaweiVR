//! 设备状态采集与显示：头显（VR Glass）是否插上、手机是否插上、电池
//!
//! 设计依据：
//! - 头显"插上"判定：Windows 用 WMI 查显示器 EDID（HWV0003/RTK2560/HUAWEI/VRGLASS/CV10）
//!   与 USB VID/PID（12D1）；Linux 用 sysfs。来自上游 auto-vr-mode.ps1 的 EDID 判据。
//!   已知头显 PID 白名单 1092/3A09 来自对原厂 HwPCVRAssistant.exe 的逆向
//!   （Check6DOFDevicePlugin / "3a09 12d1"），详见 docs/06-Windows状态获取逆向.md。
//! - 手机"插上"判定：adb devices（上游 alvr/adb 的 pick_device_serial 同源）。
//!   注意：原厂这套 Windows PC VR 组件内没有 adb 逻辑，手机 adb 走的是上游 ALVR 路径。
//! - 电池：原厂用 kernel32!GetSystemPowerStatus 读 BatteryFlag/ACLineStatus（已实证），
//!   本模块 Windows 下优先用该 API，WMI Win32_Battery 作回退；
//!   手机用 `adb shell dumpsys battery`（level/status）；上游 ALVR 用 Battery 包上报 HMD 电池。
//! - 全部通过外部命令/std 读取（电池一处直接链接 kernel32）→ 可交叉编译到 Windows GNU target。

use serde::{Deserialize, Serialize};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// ALVR 串流连接状态（由 client.rs 在握手成功/断开时更新，GUI 指示灯读取）。
/// 使用原子变量，无需加锁，跨线程可见。
static ALVR_CONNECTED: AtomicBool = AtomicBool::new(false);
/// 最近一次握手的毫秒时间戳：用于短保持窗口，避免极短连接导致 GUI 采不到点亮。
static LAST_CONNECT_MS: AtomicU64 = AtomicU64::new(0);
const CONNECT_HOLD_MS: u64 = 4000;

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// 连接成功后调用：点亮 GUI 的「ALVR 串流」指示灯。
pub fn set_alvr_connected(v: bool) {
    if v { LAST_CONNECT_MS.store(now_ms(), Ordering::SeqCst); }
    ALVR_CONNECTED.store(v, Ordering::SeqCst);
}

/// 供 GUI/状态采集读取当前串流连接状态。
/// 真正连接中 → true；断开后 4 秒内仍返回 true（保持窗口，便于 GUI 捕捉"已连接"事件）。
pub fn alvr_connected() -> bool {
    if ALVR_CONNECTED.load(Ordering::SeqCst) { return true; }
    let last = LAST_CONNECT_MS.load(Ordering::SeqCst);
    last != 0 && now_ms().saturating_sub(last) < CONNECT_HOLD_MS
}

/// 头显（VR Glass）状态
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct HeadsetStatus {
    /// 是否插上（EDID 或 USB 命中）
    pub plugged: bool,
    /// 命中依据：edid / usb / none
    pub detected_by: String,
    /// 显示器 EDID 名称（如 DISPLAY\HWV0003\...）
    pub edid: String,
    /// USB VID:PID（如 12D1:3A09）
    pub usb_ids: Vec<String>,
    /// 是否处于 DFU 模式（12D1:1093）
    pub in_dfu: bool,
}

/// 手机状态
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct PhoneStatus {
    pub plugged: bool,          // adb 设备在线
    pub serial: String,
    pub model: String,
    pub battery_pct: Option<u32>,
    pub charging: bool,
}

/// PC（低配机）电池
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct PcBattery {
    pub present: bool,
    pub pct: Option<u32>,
    pub charging: bool,
}

/// 手柄（3DoF/6DoF）状态 —— 用于 UI 上的"左右手柄指示灯"
///
/// 数据来源（按优先级）：
/// 1) ALVR tracking 通道上报的手柄连接状态（手柄通常配对在跑 SteamVR 的服务端，
///    其在线/离线性以服务端 tracking 为准，见 controller_status.json 注入通道）；
/// 2) 本机 HID/蓝牙手柄接收器枚举（本地映射手柄时）。
/// 电量多数手柄不向 PC 上报，故为 Option。
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct ControllerStatus {
    /// 是否连接 / 已映射
    #[serde(default)]
    pub connected: bool,
    /// 自由度：3 或 6，0=未知
    #[serde(default)]
    pub dof: u8,
    /// 电量百分比（可选）
    #[serde(default)]
    pub battery_pct: Option<u32>,
    /// 数据来源：alvr / hid / none
    #[serde(default)]
    pub source: String,
    /// 设备显示名
    #[serde(default)]
    pub name: String,
}

/// 汇总状态
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct DeviceStatus {
    pub headset: HeadsetStatus,
    pub phone: PhoneStatus,
    pub pc_battery: PcBattery,
    /// 左手柄（控制器 0）
    pub left_controller: ControllerStatus,
    /// 右手柄（控制器 1）
    pub right_controller: ControllerStatus,
    pub alvr_connected: bool,
    pub updated_at_ms: u64,
}

impl DeviceStatus {
    /// 采集一次（同步，可能耗时约 0.1~1s，调用方应在后台线程调用）
    pub fn collect() -> Self {
        let headset = collect_headset();
        let phone = collect_phone();
        let pc_battery = collect_pc_battery();
        let (left_controller, right_controller) = collect_controllers();
        Self {
            headset,
            phone,
            pc_battery,
            left_controller,
            right_controller,
            alvr_connected: alvr_connected(),
            updated_at_ms: now_ms(),
        }
    }

    /// ALVR Battery 包用的 HMD 电量/充电状态。
    /// VR Glass 本身无电池（靠 PC USB-C 供电），因此用"头显在线 + PC 电池/充电"代表；
    /// 手机电池单独显示，不混入 HMD 通道，避免 Dashboard 误显示。
    pub fn hmd_battery(&self) -> (f32, bool) {
        let gauge = if self.headset.plugged {
            self.pc_battery.pct.unwrap_or(100) as f32 / 100.0
        } else {
            0.0
        };
        (gauge, self.pc_battery.charging || self.headset.plugged)
    }

    /// 渲染成控制台面板（多行文本）
    pub fn render_panel(&self) -> String {
        let mark = |b: bool| if b { "● 已插上" } else { "○ 未插上" };
        let bat = |p: Option<u32>, c: bool| match p {
            Some(v) => format!("{}%{}", v, if c { " (充电中)" } else { "" }),
            None => "未知".into(),
        };
        let usb_text = self.headset.usb_ids.join(", ");
        let ctrl = |c: &ControllerStatus| {
            if c.connected {
                format!("● 已连接  电池:{}  {}DoF [{}]",
                    bat(c.battery_pct, false),
                    if c.dof == 0 { "?" } else { if c.dof == 6 { "6" } else { "3" } },
                    if c.source.is_empty() { "-" } else { &c.source })
            } else {
                "○ 未连接".to_string()
            }
        };
        format!(
            "┌─ 设备状态 ────────────────────────────────\n\
             │ 头显(VR Glass) : {}   [{}]\n\
             │   EDID: {}\n\
             │   USB : {}{}\n\
             │ 左手柄         : {}\n\
             │   名称: {}\n\
             │ 右手柄         : {}\n\
             │   名称: {}\n\
             │ 手机           : {}   {}\n\
             │   型号: {}  电池: {}\n\
             │ PC 电池        : {}\n\
             │ ALVR 串流      : {}\n\
             └──────────────────────────────────────────────",
            mark(self.headset.plugged),
            self.headset.detected_by,
            if self.headset.edid.is_empty() { "-" } else { &self.headset.edid },
            if usb_text.is_empty() { "-" } else { &usb_text },
            if self.headset.in_dfu { "  [DFU模式]" } else { "" },
            ctrl(&self.left_controller),
            if self.left_controller.name.is_empty() { "-" } else { &self.left_controller.name },
            ctrl(&self.right_controller),
            if self.right_controller.name.is_empty() { "-" } else { &self.right_controller.name },
            mark(self.phone.plugged),
            if self.phone.plugged { format!("({})", self.phone.serial) } else { String::new() },
            if self.phone.model.is_empty() { "-" } else { &self.phone.model },
            bat(self.phone.battery_pct, self.phone.charging),
            bat(self.pc_battery.pct, self.pc_battery.charging),
            if self.alvr_connected { "● 已连接" } else { "○ 等待中" },
        )
    }
}

/// 运行外部命令并返回 stdout（失败返回空串）
fn run_cmd(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

const EDID_KEYWORDS: &[&str] = &["HWV0003", "RTK2560", "HUAWEI", "VRGLASS", "VR GLASS", "AV03", "CV10"];
#[cfg(not(windows))]
const HUAWEI_VID: &str = "12D1";
/// 原厂 HwPCVRAssistant.exe 逆向实证的头显 PID（Check6DOFDevicePlugin / "3a09 12d1"）。
/// 1093 是 DFU 模式（升级器路径），单独处理，不算正常在线。
const HMD_PIDS: &[&str] = &["1092", "3A09"];

#[cfg(windows)]
fn collect_headset() -> HeadsetStatus {
    let mut s = HeadsetStatus::default();

    // 1) WMI 显示器 EDID（与上游 auto-vr-mode.ps1 同源）
    let ps = "Get-CimInstance -Namespace root\\wmi -ClassName WmiMonitorBasicDisplayParams -ErrorAction SilentlyContinue | ForEach-Object { $_.InstanceName }";
    let monitors = run_cmd("powershell", &["-NoProfile", "-Command", ps]);
    for line in monitors.lines() {
        let up = line.trim().to_uppercase();
        if EDID_KEYWORDS.iter().any(|k| up.contains(k)) {
            s.plugged = true;
            s.detected_by = "edid".into();
            s.edid = line.trim().to_string();
            break;
        }
    }

    // 2) USB VID/PID（含 DFU 判定）
    let ps2 = "Get-PnpDevice -Class USB -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -match 'VID_12D1' } | ForEach-Object { $_.InstanceId }";
    let usb = run_cmd("powershell", &["-NoProfile", "-Command", ps2]);
    for line in usb.lines() {
        let up = line.trim().to_uppercase();
        if let Some(pos) = up.find("VID_12D1") {
            let tail = &up[pos..];
            let pid = tail.get(8..12).unwrap_or("????");
            let id = format!("12D1:{}", pid);
            if !s.usb_ids.contains(&id) { s.usb_ids.push(id); }
            if pid == "1093" {
                s.in_dfu = true;
                // DFU 不是正常在线态，不要据此判"头显已插好"
                continue;
            }
            // 匹配原厂已知头显 PID；其它 12D1 设备（如手机）不误判为头显
            if HMD_PIDS.contains(&pid) {
                s.plugged = true;
                if s.detected_by.is_empty() { s.detected_by = "usb".into(); }
            }
        }
    }
    s
}

#[cfg(not(windows))]
fn collect_headset() -> HeadsetStatus {
    // Linux sysfs 回退（与之前 hvr-upgrader-linux/devices.go 同源）
    let mut s = HeadsetStatus::default();
    let Ok(entries) = std::fs::read_dir("/sys/bus/usb/devices") else { return s };
    for e in entries.flatten() {
        let p = e.path();
        let vid = read_trim(&p.join("idVendor")).to_uppercase();
        if vid != HUAWEI_VID { continue; }
        let pid = read_trim(&p.join("idProduct")).to_uppercase();
        let id = format!("{}:{}", vid, pid);
        if !s.usb_ids.contains(&id) { s.usb_ids.push(id); }
        if pid == "1093" { s.in_dfu = true; }
        s.plugged = true;
        if s.detected_by.is_empty() { s.detected_by = "usb".into(); }
    }
    s
}

#[cfg(not(windows))]
fn read_trim(p: &std::path::Path) -> String {
    std::fs::read_to_string(p).map(|s| s.trim().to_string()).unwrap_or_default()
}

fn collect_phone() -> PhoneStatus {
    let mut s = PhoneStatus::default();
    // adb devices：第一行 "List of devices attached"，后续 "serial\tdevice"
    let out = run_cmd("adb", &["devices"]);
    for line in out.lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 && parts[1] == "device" {
            s.plugged = true;
            s.serial = parts[0].to_string();
            break;
        }
    }
    if !s.plugged { return s; }

    // 型号
    s.model = run_cmd("adb", &["-s", &s.serial, "shell", "getprop", "ro.product.model"])
        .trim().to_string();
    // 电池：dumpsys battery 的 level / status（2=充电）
    let bat = run_cmd("adb", &["-s", &s.serial, "shell", "dumpsys", "battery"]);
    for line in bat.lines() {
        let l = line.trim();
        if let Some(v) = l.strip_prefix("level:") {
            s.battery_pct = v.trim().parse().ok();
        } else if let Some(v) = l.strip_prefix("status:") {
            s.charging = v.trim() == "2";
        }
    }
    s
}

/// SYSTEM_POWER_STATUS —— 原厂 HwPCVRAssistant.exe 用的就是这个 API
/// （逆向实证：导入 KERNEL32!GetSystemPowerStatus，日志 BatteryFlag/ACLineStatus）。
#[cfg(windows)]
#[repr(C)]
struct SystemPowerStatus {
    ac_line_status: u8,
    battery_flag: u8,
    battery_life_percent: u8,
    battery_life_time: u32,
    battery_full_life_time: u32,
    battery_voltage: u32,
    battery_capacity: u32,
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn GetSystemPowerStatus(ps: *mut SystemPowerStatus) -> i32;
}

#[cfg(windows)]
fn collect_pc_battery() -> PcBattery {
    const NO_BATTERY: u8 = 0x80;
    const CHARGING: u8 = 0x08;
    let mut b = PcBattery::default();

    // 主路径：GetSystemPowerStatus（与原厂一致，且不启动 PowerShell）
    let mut ps = SystemPowerStatus {
        ac_line_status: 255,
        battery_flag: 255,
        battery_life_percent: 255,
        battery_life_time: u32::MAX,
        battery_full_life_time: u32::MAX,
        battery_voltage: u32::MAX,
        battery_capacity: u32::MAX,
    };
    let ok = unsafe { GetSystemPowerStatus(&mut ps) != 0 };
    if ok && ps.battery_flag != 255 && (ps.battery_flag & NO_BATTERY) == 0 {
        b.present = true;
        if ps.battery_life_percent != 255 {
            b.pct = Some(ps.battery_life_percent as u32);
        }
        b.charging = ps.ac_line_status == 1 || (ps.battery_flag & CHARGING) != 0;
        return b;
    }

    // 回退：Win32_Battery WMI（GetSystemPowerStatus 不可用/台式机无电池时）
    let ps_wmi = "Get-CimInstance Win32_Battery -ErrorAction SilentlyContinue | ForEach-Object { \"$($_.EstimatedChargeRemaining);$($_.BatteryStatus)\" }";
    let out = run_cmd("powershell", &["-NoProfile", "-Command", ps_wmi]);
    if let Some(line) = out.lines().find(|l| l.contains(';')) {
        let parts: Vec<&str> = line.trim().split(';').collect();
        if let Ok(pct) = parts[0].parse::<u32>() {
            b.present = true;
            b.pct = Some(pct);
            let st = parts.get(1).and_then(|x| x.parse::<u32>().ok()).unwrap_or(0);
            b.charging = matches!(st, 2 | 6 | 7 | 8 | 9);
        }
    }
    b
}

#[cfg(not(windows))]
fn collect_pc_battery() -> PcBattery {
    let mut b = PcBattery::default();
    if let Ok(s) = std::fs::read_to_string("/sys/class/power_supply/BAT0/capacity") {
        b.pct = s.trim().parse().ok();
        b.present = b.pct.is_some();
    }
    if let Ok(s) = std::fs::read_to_string("/sys/class/power_supply/BAT0/status") {
        b.charging = s.trim() == "Charging" || s.trim() == "Full";
    }
    b
}

/// 手柄注入文件（可选）：ALVR tracking 解析线程或外部工具写入，格式：
/// {"left":{"connected":true,"dof":6,"battery_pct":80,"name":"Index 左手"},
///  "right":{"connected":true,"dof":6,"battery_pct":75,"name":"Index 右手"}}
#[derive(Serialize, Deserialize, Default)]
struct ControllerInjectFile {
    left: Option<ControllerStatus>,
    right: Option<ControllerStatus>,
}

/// 采集左右手柄：优先读注入文件（服务端 tracking 下发），否则枚举本机 HID 手柄。
fn collect_controllers() -> (ControllerStatus, ControllerStatus) {
    // 1) 外部注入（ALVR tracking / 手动）—— 最贴近真实手柄在线态
    let inject_path = std::env::current_dir()
        .unwrap_or_default()
        .join("controller_status.json");
    if let Ok(txt) = std::fs::read_to_string(&inject_path) {
        if let Ok(inj) = serde_json::from_str::<ControllerInjectFile>(&txt) {
            let mut l = inj.left.unwrap_or_default();
            let mut r = inj.right.unwrap_or_default();
            if l.source.is_empty() && l.connected { l.source = "alvr".into(); }
            if r.source.is_empty() && r.connected { r.source = "alvr".into(); }
            return (l, r);
        }
    }

    // 2) 本机 HID/蓝牙手柄枚举
    let names = enumerate_hid_controllers();
    let mk = |name: Option<String>, hand: &str| {
        let mut c = ControllerStatus::default();
        if let Some(n) = name {
            c.connected = true;
            c.name = n.clone();
            c.dof = if n.to_uppercase().contains("QUEST") { 6 } else { 3 };
            c.source = "hid".into();
        } else {
            c.name = if hand == "left" { "左手柄".into() } else { "右手柄".into() };
            c.source = "none".into();
        }
        c
    };
    (mk(names.get(0).cloned(), "left"), mk(names.get(1).cloned(), "right"))
}

/// 枚举本机手柄类 HID 设备名（Windows 用 PnP，其它平台返回空）。
fn enumerate_hid_controllers() -> Vec<String> {
    #[cfg(windows)]
    {
        // 匹配常见手柄关键字；同时避开把头显/键盘/鼠标误判成手柄
        let ps = "Get-PnpDevice -Class HIDClass -ErrorAction SilentlyContinue | \
                  Where-Object { $_.FriendlyName -match 'controller|手柄|joy|index|vive|oculus|quest|wand|touch' } | \
                  ForEach-Object { $_.FriendlyName }";
        let out = run_cmd("powershell", &["-NoProfile", "-Command", ps]);
        let mut v: Vec<String> = out.lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        v.truncate(2);
        v
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// 后台状态监视器：定期采集，提供最新快照
#[derive(Clone)]
pub struct StatusMonitor {
    latest: std::sync::Arc<std::sync::RwLock<DeviceStatus>>,
}

impl StatusMonitor {
    pub fn start(interval: Duration) -> Self {
        let latest = std::sync::Arc::new(std::sync::RwLock::new(DeviceStatus::collect()));
        let l = std::sync::Arc::clone(&latest);
        std::thread::spawn(move || loop {
            std::thread::sleep(interval);
            *l.write().unwrap() = DeviceStatus::collect();
        });
        Self { latest }
    }

    pub fn snapshot(&self) -> DeviceStatus {
        self.latest.read().unwrap().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edid_keywords_match_glasses() {
        let s = "DISPLAY\\HWV0003\\5&abcde&0&UID0";
        let up = s.to_uppercase();
        assert!(EDID_KEYWORDS.iter().any(|k| up.contains(k)));
        let s2 = "DISPLAY\\DELL123\\5&abc";
        let up2 = s2.to_uppercase();
        assert!(!EDID_KEYWORDS.iter().any(|k| up2.contains(k)));
    }

    #[test]
    fn parse_dumpsys_battery() {
        let txt = "  level: 73\n  status: 2\n  scale: 100\n";
        let mut pct = None;
        let mut charging = false;
        for line in txt.lines() {
            let l = line.trim();
            if let Some(v) = l.strip_prefix("level:") { pct = v.trim().parse().ok(); }
            else if let Some(v) = l.strip_prefix("status:") { charging = v.trim() == "2"; }
        }
        assert_eq!(pct, Some(73));
        assert!(charging);
    }

    #[test]
    fn panel_renders_both_devices() {
        let mut s = DeviceStatus::default();
        s.headset.plugged = true;
        s.headset.detected_by = "edid".into();
        s.phone.plugged = true;
        s.phone.model = "LYA-AL00".into();
        s.phone.battery_pct = Some(50);
        s.pc_battery.pct = Some(80);
        let panel = s.render_panel();
        assert!(panel.contains("已插上"));
        assert!(panel.contains("LYA-AL00"));
        assert!(panel.contains("50%"));
        assert!(panel.contains("80%"));
    }

    #[test]
    fn known_hmd_pids_from_reverse_engineering() {
        // 原厂 HwPCVRAssistant.exe 实证：1092 / 3A09 为头显，1093 为 DFU
        assert!(HMD_PIDS.contains(&"1092"));
        assert!(HMD_PIDS.contains(&"3A09"));
        assert!(!HMD_PIDS.contains(&"1093")); // DFU 不算在线
        assert!(!HMD_PIDS.contains(&"107D")); // 通用 USB 设备，非头显
    }

    #[test]
    fn controller_status_defaults_offline() {
        let c = ControllerStatus::default();
        assert!(!c.connected);
        assert_eq!(c.dof, 0);
        assert!(c.battery_pct.is_none());
    }

    #[test]
    fn controller_inject_json_parses() {
        let txt = r#"{"left":{"connected":true,"dof":6,"battery_pct":80,"name":"IndexL"},"right":{"connected":false}}"#;
        let inj: ControllerInjectFile = serde_json::from_str(txt).unwrap();
        let l = inj.left.unwrap();
        assert!(l.connected);
        assert_eq!(l.dof, 6);
        assert_eq!(l.battery_pct, Some(80));
        assert!(inj.right.unwrap().connected == false);
    }

    #[test]
    fn panel_renders_controllers_lit_and_offline() {
        let mut s = DeviceStatus::default();
        s.left_controller.connected = true;
        s.left_controller.dof = 6;
        s.left_controller.battery_pct = Some(80);
        s.left_controller.name = "IndexL".into();
        // right 保持默认（离线）
        let panel = s.render_panel();
        assert!(panel.contains("左手柄"));
        assert!(panel.contains("右手柄"));
        assert!(panel.contains("已连接"));
        assert!(panel.contains("未连接"));
        assert!(panel.contains("80%"));
        assert!(panel.contains("6DoF"));
    }

    #[test]
    fn hmd_battery_uses_pc_when_headset_plugged() {
        let mut s = DeviceStatus::default();
        s.pc_battery.pct = Some(60);
        s.pc_battery.charging = true;
        // 头显未插 → 0
        assert_eq!(s.hmd_battery().0, 0.0);
        // 头显插上 → 用 PC 电池
        s.headset.plugged = true;
        let (g, plugged) = s.hmd_battery();
        assert!((g - 0.6).abs() < 1e-6);
        assert!(plugged);
    }
}
