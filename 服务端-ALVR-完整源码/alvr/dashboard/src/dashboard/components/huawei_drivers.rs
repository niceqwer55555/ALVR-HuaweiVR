//! Huawei VR Glass hardware driver panel.
//!
//! The glasses are three devices in Windows Device Manager:
//!   * a DisplayPort monitor (RTK2560 / HUAWEI GLASS CV10)
//!   * a USB composite device (VID_12D1&PID_107E) carrying IMU / keys
//!   * (optional) the Huawei virtual display driver
//!
//! This panel reports their state, installs the INF packages shipped next to
//! the dashboard, and opens Device Manager. It is reused by the setup wizard
//! and by the "安装" tab so a fresh machine can be brought up in one place.

use eframe::egui::{Grid, RichText, Ui};
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

#[derive(Default, Clone)]
pub struct HardwareReport {
    pub display_count: u32,
    pub usb_count: u32,
    pub usb_problem: u32,
    pub hid_count: u32,
    pub vdd_count: u32,
    pub problem_count: u32,
    pub problem_devices: Vec<String>,
    pub driver_root: String,
    pub error: Option<String>,
}

impl HardwareReport {
    fn parse(text: &str) -> Self {
        let mut report = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.trim().split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "DISPLAY_COUNT" => report.display_count = value.parse().unwrap_or(0),
                "USB_COUNT" => report.usb_count = value.parse().unwrap_or(0),
                "USB_PROBLEM" => report.usb_problem = value.parse().unwrap_or(0),
                "HID_COUNT" => report.hid_count = value.parse().unwrap_or(0),
                "VDD_COUNT" => report.vdd_count = value.parse().unwrap_or(0),
                "PROBLEM_COUNT" => report.problem_count = value.parse().unwrap_or(0),
                "PROBLEM_DEVICE" => report.problem_devices.push(value.to_owned()),
                "DRIVER_ROOT" => report.driver_root = value.to_owned(),
                "ERROR" => report.error = Some(value.to_owned()),
                _ => {}
            }
        }
        report
    }
}

/// Inline status query. Deliberately outputs ASCII `KEY=VALUE` lines only so
/// the Rust side does not depend on the console code page (GBK vs UTF-8).
#[cfg(windows)]
fn query_status_inline() -> HardwareReport {
    let script = r#"
$ErrorActionPreference = 'SilentlyContinue'
$d = Get-PnpDevice
$disp = @($d | Where-Object { $_.Class -eq 'Monitor' -and ($_.InstanceId -match 'RTK2560' -or $_.InstanceId -match 'HWV' -or $_.InstanceId -match 'CV10') })
$usb  = @($d | Where-Object { $_.InstanceId -match 'VID_12D1&PID_107' })
$hid  = @($d | Where-Object { $_.Class -eq 'HIDClass' -and $_.InstanceId -match 'VID_12D1&PID_107' })
$vdd  = @($d | Where-Object { $_.InstanceId -match 'VDD' -or $_.FriendlyName -match 'Virtual Display' })
$bad  = @($d | Where-Object { $_.Status -ne 'OK' -and ($_.InstanceId -match 'VID_12D1&PID_107' -or $_.InstanceId -match 'RTK2560' -or $_.FriendlyName -match 'VR Glass') })
Write-Output ('DISPLAY_COUNT=' + $disp.Count)
Write-Output ('USB_COUNT=' + $usb.Count)
Write-Output ('USB_PROBLEM=' + @($usb | Where-Object { $_.Status -ne 'OK' }).Count)
Write-Output ('HID_COUNT=' + $hid.Count)
Write-Output ('VDD_COUNT=' + $vdd.Count)
Write-Output ('PROBLEM_COUNT=' + $bad.Count)
foreach ($p in $bad) { Write-Output ('PROBLEM_DEVICE=' + $p.InstanceId + ' | ' + $p.Status) }
Write-Output ('DRIVER_ROOT=')
"#;
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    match std::process::Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        Ok(output) => {
            let text = String::from_utf8_lossy(&output.stdout);
            let mut report = HardwareReport::parse(&text);
            if !output.status.success() && report.usb_count == 0 {
                report.error = Some("PowerShell 查询失败（Get-PnpDevice 不可用？）".into());
            }
            report
        }
        Err(err) => HardwareReport {
            error: Some(format!("无法启动 PowerShell：{err}")),
            ..Default::default()
        },
    }
}

#[cfg(not(windows))]
fn query_status_inline() -> HardwareReport {
    HardwareReport {
        error: Some("硬件驱动安装仅支持 Windows".into()),
        ..Default::default()
    }
}

fn find_script() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.to_path_buf()))
    {
        candidates.push(dir.join("scripts").join("hardware-drivers.ps1"));
        candidates.push(dir.join("hardware-drivers.ps1"));
        candidates.push(
            dir.join("scripts")
                .join("huawei")
                .join("hardware-drivers.ps1"),
        );
        // Dev layout: build/target/release/alvr_dashboard.exe -> repo/scripts/...
        candidates.push(
            dir.join("..")
                .join("..")
                .join("..")
                .join("scripts")
                .join("huawei")
                .join("hardware-drivers.ps1"),
        );
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("scripts").join("huawei").join("hardware-drivers.ps1"));
    }
    candidates.into_iter().find(|path| path.is_file())
}

#[cfg(windows)]
fn launch_install(script: &PathBuf) -> Result<(), String> {
    let path = script.to_string_lossy().replace('\'', "''");
    let command = format!(
        "Start-Process -FilePath 'powershell.exe' -Verb RunAs -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-File','{path}','-Install')"
    );
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    std::process::Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &command])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|err| format!("无法请求管理员权限：{err}"))
}

#[cfg(not(windows))]
fn launch_install(_script: &PathBuf) -> Result<(), String> {
    Err("硬件驱动安装仅支持 Windows".into())
}

pub struct HuaweiDriverPanel {
    report: Option<HardwareReport>,
    receiver: Option<Receiver<HardwareReport>>,
    last_refresh: Option<Instant>,
    auto_requested: bool,
    message: Option<(bool, String)>,
}

impl HuaweiDriverPanel {
    pub fn new() -> Self {
        Self {
            report: None,
            receiver: None,
            last_refresh: None,
            auto_requested: false,
            message: None,
        }
    }

    fn request_refresh(&mut self) {
        if self.receiver.is_some() {
            return;
        }
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(query_status_inline());
        });
        self.receiver = Some(receiver);
        self.last_refresh = Some(Instant::now());
    }

    fn poll(&mut self) {
        let received = self
            .receiver
            .as_ref()
            .and_then(|receiver| receiver.try_recv().ok());
        if let Some(report) = received {
            self.report = Some(report);
            self.receiver = None;
        }
        if self.receiver.is_none() {
            let due = self
                .last_refresh
                .map(|instant| instant.elapsed() >= Duration::from_secs(4))
                .unwrap_or(true);
            if due {
                self.request_refresh();
            }
        }
    }

    fn install(&mut self) {
        match find_script() {
            Some(script) => match launch_install(&script) {
                Ok(()) => {
                    self.message = Some((
                        true,
                        "已弹出管理员授权窗口，完成 UAC 后驱动会自动安装；装完点“重新检测”。"
                            .to_owned(),
                    ));
                }
                Err(err) => self.message = Some((false, err)),
            },
            None => {
                self.message = Some((
                    false,
                    "找不到 hardware-drivers.ps1（应与 dashboard 一起打包到 scripts\\ 目录）"
                        .to_owned(),
                ));
            }
        }
    }

    fn open_device_manager(&mut self) {
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("mmc").arg("devmgmt.msc").spawn();
        }
    }

    pub fn ui(&mut self, ui: &mut Ui) {
        if !self.auto_requested {
            self.auto_requested = true;
            self.request_refresh();
        }
        self.poll();

        ui.label(
            RichText::new(
                "先插好眼镜 USB-C。这里安装的是设备管理器里的华为硬件驱动（显示器 / USB 复合设备 / HID / 虚拟显示器），\
                 不包含 SteamVR 驱动注册；SteamVR 驱动请用上面的“注册/移除”按钮。",
            )
            .size(11.0)
            .weak(),
        );
        ui.add_space(4.0);

        let (display, usb, hid, vdd, problem) = match &self.report {
            Some(report) => (
                report.display_count > 0,
                report.usb_count > 0 && report.usb_problem == 0,
                report.hid_count > 0,
                report.vdd_count > 0,
                report.problem_count == 0,
            ),
            None => (false, false, false, false, false),
        };

        Grid::new(0).num_columns(3).show(ui, |ui| {
            for (ok, name, detail) in [
                (display, "眼镜显示器（RTK2560 / HUAWEI GLASS）", "DisplayPort 视频链路"),
                (usb, "USB 复合设备 VID_12D1&PID_107E", "IMU / 按键 / 控制通道"),
                (hid, "HID 设备（6DoF / IMU）", "没有 6DoF 套件时可为空"),
                (vdd, "虚拟显示器驱动", "部分固件才需要"),
                (problem, "无异常（黄色感叹号）设备", "异常设备数应为 0"),
            ] {
                let (color, text) = if ok {
                    (ui.visuals().hyperlink_color, "正常")
                } else {
                    (ui.visuals().error_fg_color, "缺失 / 异常")
                };
                ui.colored_label(color, text);
                ui.label(name);
                ui.label(RichText::new(detail).weak().size(10.0));
                ui.end_row();
            }
        });

        if let Some(report) = &self.report {
            if !report.driver_root.is_empty() {
                ui.label(RichText::new(format!("驱动包目录：{}", report.driver_root)).weak().size(10.0));
            }
            if let Some(error) = &report.error {
                ui.colored_label(ui.visuals().error_fg_color, error.as_str());
            }
            if !report.problem_devices.is_empty() {
                ui.label(RichText::new("异常设备（设备管理器里带感叹号）：").weak().size(10.0));
                for device in report.problem_devices.iter().take(5) {
                    ui.label(RichText::new(format!("  {device}")).weak().size(10.0));
                }
            }
        } else if self.receiver.is_some() {
            ui.label(RichText::new("正在检测设备…").weak());
        }

        ui.horizontal(|ui| {
            if ui.button("重新检测").clicked() {
                self.last_refresh = None;
            }
            if ui.button("安装 / 修复硬件驱动（管理员）").clicked() {
                self.install();
            }
            if ui.button("打开设备管理器").clicked() {
                self.open_device_manager();
            }
        });

        if let Some((ok, message)) = &self.message {
            ui.colored_label(
                if *ok {
                    ui.visuals().hyperlink_color
                } else {
                    ui.visuals().error_fg_color
                },
                message.as_str(),
            );
        }
    }
}

impl Default for HuaweiDriverPanel {
    fn default() -> Self {
        Self::new()
    }
}
