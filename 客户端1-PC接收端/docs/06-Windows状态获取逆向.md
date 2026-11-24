# 华为 VR Windows 组件状态获取机制逆向报告

> 对象：用户经资料库提供的 `work.rar`（标题 "work"，节点 `paCizeu549cOgEahDdfpsH`）
> 目的：搞清华为原厂 Windows 端"头显是否插上 / 电池 / 手柄 / 手机"到底怎么获取，用于校正本项目 `src/status.rs`。
> 结论先行：**原厂不用 adb 读手机，也不靠 WMI `Win32_Battery` 读电池**；它用 `SetupAPI + HID + EDID VID/PID` 判头显，用 `kernel32!GetSystemPowerStatus` 判电池，用命名共享内存 `HwControllerSHARED` 在进程间传手柄状态。本项目当前实现有一处需要修正、两处可以加强。

---

## 1. 附件甄别：为什么一开始读到的是 13 KB HTML

`@资料库` 引用落到 `/root/uploads/1795504492402528860-work.rar` 的**不是文件本体**，而是资料库的 SPA 页面壳：

| 项 | 值 |
|---|---|
| 本地路径头部 | `<!doctype html>` |
| 大小 | 12,339 字节（13 KB） |
| `file` | HTML document, Unicode text, UTF-8 |
| 页面标题 | `work` |
| 页面内 `og:url` | `https://www.workbuddy.cn/space/d/paCizeu549cOgEahDdfpsH` |

按资料库通道查节点：

```text
node-info(paCizeu549cOgEahDdfpsH)
  → kind = drive, ext = rar, title = work   ← 确认为真实文件节点
  → get_download_link → smhwb-gz-002.data-tencentsmh.com/...work_...rar （短期有效）
```

下载后得到**真实文件**：

| 项 | 值 |
|---|---|
| 大小 | 16,383,878 字节（16.4 MB） |
| `file` | RAR archive data, **v5** |
| 压缩内容 | 36 文件 / 37 目录，解压后 208 MB |
| SHA-256（rar 本体） | 见 §7 |

> 注：系统自带 7z 23.01 对这份 RAR5 报 `Unsupported Method`（36 个成员全部失败）。改用 `unrar` 后 `All OK`。这是工具兼容性问题，不是文件损坏。

---

## 2. 包结构：一套完整的 Windows PC VR 驱动/组件

解压后全部是 **native x64 PE**，**没有任何 .NET 程序集**（所以走导出表/导入表/字符串/PDB 路径分析，而不是 IL 反编译）。

按用途分类：

| 类别 | 文件 | 作用 |
|---|---|---|
| 主程序/UI | `HwPCVRAssistant.exe` (5.1 MB)、`FirstUse.exe` (4.3 MB)、`PCCastScreen.exe` (3.7 MB) | 助手 UI、首次引导、投屏；**状态监控逻辑集中在这里** |
| 头显/姿态 | `Hmd.dll` (297 KB)、`Hmd_pccast/Hmd.dll` (280 KB)、`VR6DOF.dll` (3.9 MB)、`arvr_slam.dll` (69 MB)、`HiORBLib.dll` | 头显驱动、IMU、6DoF SLAM、ORB 特征 |
| 手柄状态 | `ControllerStatus.dll` (40 KB) | **导出 `GetShareData`/`SetShareData`，跨进程共享手柄状态** |
| 显示管理 | `hwvr_DisplayManager.dll` (101 KB)、`VirtualDisplayDriver.dll`、`virtual_display_device.sys` | 虚拟显示器热插拔（`DisplayPlugIn`/`DisplayUnplug`） |
| 核心 API | `hwvr_api.dll` (1.1 MB)、`HwPCVR.dll` (756 KB)、`HandlerDLL.dll` (15.7 MB) | `GetIhwvrClient`、D3D11 副屏渲染、IMU 客户端 |
| SteamVR 驱动 | `driver_huaweiVR_steamvr/driver_huaweiVR.dll`、`driver_HuaweiVR_pccast/driver_HuaweiVR.dll` | OpenVR `IServerTrackedDeviceProvider` 实现 |
| USB 驱动 | `hw_cdcacm.sys`、`hw_quusbmdm.sys`、`hw_quusbnet.sys`、`hw_usbdev.sys`、`usbser.sys`、`ew_usbccgpfilter.sys` | 华为高通 USB 设备（CDC-ACM / QUSB diag/mdm/net） |
| 安装/下载 | `devsetup64.exe`、`DriverSetUp.exe`、`DriverUnInstall.exe`、`ParsifalDownload.dll`、`XmlRead.exe` | 驱动安装、固件/配置下载 |
| 三方库 | `opencv_world452.dll`、`Ceres.dll`、`libcrypto/libssl-1_1-x64.dll` | 视觉/优化/TLS |

`_report.tsv` 还泄露了构建来源：PDB 路径为 `D:\code\version\2021_105_01\arvr_pccastscreen\...`，说明这套组件属于华为 **arvr_pccastscreen（PC 投屏）** 产品线，与本项目参考的 `HuaweiVRGlass-ALVR` 仓库同源。

---

## 3. 逆向方法与证据强度标注

由于是 stripped native PE，采用三类静态证据交叉验证：

1. **导出表**（`pe_scan.py` 解析 PE Export Directory）：铁证，直接给出函数/接口名。
2. **导入表**（PE Import Directory）：铁证，直接给出调用的 Win32/HID/SetupAPI 函数。
3. **字符串与日志格式串**（ASCII + UTF-16LE）：强证据，日志串往往紧贴实现。
4. **反汇编内部算法**：本报告**未做**（无 Ghidra/radare2，仅有 objdump）。凡涉及"具体算法/阈值"的结论均标注为推断，不作既定事实。

> 证据分级：`[导出/导入=铁证]` `日志字符串=强证据` `推断=待真机验证`

---

## 4. 状态获取机制详解

### 4.1 头显「是否插上」——SetupAPI + HID + EDID VID/PID

**不是**单一 WMI 查询，而是一组专门的监控线程：

| 证据串 | 含义 |
|---|---|
| `DeviceManager::CheckDriverDevicePlugin2` `[导出/日志]` | 判断驱动设备（头显）是否插入 |
| `DeviceManager::Check6DOFDevicePlugin` | 判断 6DoF 设备是否插入 |
| `[%s] IsDevicePlugin %d ret = %d` | 插件式检测：枚举+逐个判断 |
| `DeviceUI::DeviceStateMonitor` / `DeviceUI::HmdStatusMonitor` | 设备/头显状态监控线程 |
| `VRGLASS::UpdateGlassStatus`、`[%s]: vr status %d`、`edidVid` / `edidPid` | 读显示器 EDID 的 VID/PID |
| `HwPCVRAssistant` 导入 `SETUPAPI.dll`：`SetupDiGetClassDevsW`、`SetupDiEnumDeviceInterfaces`、`SetupDiGetDeviceInterfaceDetailW`、`SetupDiDestroyDeviceInfoList` `[铁证]` | 枚举设备接口拿设备路径 |
| `HwPCVRAssistant` / `Hmd.dll` / `PCCastScreen.exe` 导入 `HID.DLL`：`HidD_GetHidGuid`、`HidD_GetAttributes`、`HidD_GetPreparsedData`、`HidP_GetCaps`、`HidD_SetFeature` `[铁证]` | 通过 HID 特征报告识别/控制头显 |
| `USER32`：`EnumDisplayDevicesW`、`EnumDisplayMonitors`、`GetMonitorInfoW`、`QueryDisplayConfig` `[铁证]` | 枚举显示器，配合 EDID 判据 |

**华为 VID/PID 清单（字符串实证）**：

| PID | 出现位置 | 推断用途 |
|---|---|---|
| `VID_12D1&PID_1092` | `HwPCVRAssistant.exe`、`FirstUse.exe` | 头显主 USB 设备 `[强证据]` |
| `VID_12D1&PID_3A09` | `HwPCVRAssistant.exe`（`[%s] 3a09 12d1`）、`ParsifalDownload.dll`、`FirstUse.exe` | 6DoF/定位器设备 `[强证据]` |
| `vid_12d1&pid_107d`, `1050`, `1800`, `1032`, `1030`, `1011`, `1056` | `devsetup64.exe` | 华为通用 USB 设备（驱动匹配） |
| `12D1:1093` | 本项目/升级器已知 | DFU 模式 `[已知事实]` |

判定链路（推断，需真机抓日志确认顺序）：
```
SetupDiGetClassDevsW(HID/USB)
  → SetupDiEnumDeviceInterfaces → GetDeviceInterfaceDetailW（设备路径）
  → CreateFileW(设备路径) → HidD_GetAttributes（读 VID/PID）
  → 命中 12D1/1092/3A09 ⇒ 头显在线
并行：EnumDisplayMonitors/EnumDisplayDevicesW → 读 EDID 的 edidVid/edidPid ⇒ 显示器在线
```

### 4.2 电池状态——`GetSystemPowerStatus`，不是 WMI `Win32_Battery`

这是**最值得据以修正当前实现**的发现。

| 证据串 | 含义 |
|---|---|
| `HwPCVRAssistant.exe` 导入 `KERNEL32.dll!GetSystemPowerStatus` `[铁证]` | 系统电源状态 API |
| `%s(%d):ret = %d, BatteryFlag = %d, ACLineStatus = %d.` `[强证据]` | 解析 `SYSTEM_POWER_STATUS.BatteryFlag` 和 `ACLineStatus` |
| `GetNotebookPCPowerStatus` `[强证据]` | 自封装的"笔记本 PC 电量"函数 |
| `nvpowerapi.dll`（`HwPCVRAssistant`/`FirstUse`/`hwvr_api`/`driver_HuaweiVR` 均动态加载）`[强证据]` | NVIDIA 电源管理 API（独显机型） |
| `right_battery` / `left_battery`、`page_battery`、`../battery/dianchi_%03d.jpg` `[强证据]` | **左右手柄电池 + 电池 UI 图片**（dianchi=电池） |
| 驱动侧：`QCDriverPowerManagementEnabled`、`QCDriverPowerNotify`、WMI 电源 GUID `[强证据]` | 高通 USB 驱动的电源管理（非状态读取主路径） |

**关键点**：PC VR 模式下 VR Glass 由 PC 的 USB-C 供电，**头显本身没有可上报的独立电池**。原厂日志里的 `BatteryFlag` 是**PC（笔记本）电池**，`left_battery/right_battery` 是**左右手柄电池**。这与本项目 `hmd_battery()` 的现有设计（头显在线时用 PC 电池代表 HMD）在物理事实上是一致的——只是采集 API 应改为 `GetSystemPowerStatus`。

### 4.3 手柄状态——命名共享内存 `HwControllerSHARED`

`ControllerStatus.dll` 极小（40 KB），导出表 `[铁证]`：

```text
GetShareData
SetShareData
```

内部关键字符串：

```text
HwControllerSHARED                         ← 命名共享内存段名
controller status %d %d %d %d               ← 4 个状态字
HeadConnectState = %d
LeftHandleConnectState = %d
RightHandleConnectState = %d
```

`HwPCVRAssistant.exe` 里：

```text
\steamVRDriver\huaweiVR\bin\win64\ControllerStatus.dll
%s(%d):LoadLibrary ControllerStatus.dll success
%s(%d):GetProcAddress GetShareData success
%s(%d): controller status %d %d %d %d
```

**机制（强证据）**：助手进程运行时 `LoadLibrary` 这个 DLL，`GetProcAddress("GetShareData")` 读取命名共享段 `HwControllerSHARED`，拿到头显/左/右手柄的连接状态；驱动侧（VR6DOF/手柄驱动）通过 `SetShareData` 写入。这是**跨进程无锁状态广播**，不走注册表/WMI。

### 4.4 姿态与 IMU——`Hmd.dll` 导出接口

`Hmd.dll` 导出表 `[铁证]`（14 个）：

```text
createHmdDriver      releaseHmdDriver    getVersionInfo
enableHmdImu         disableHmdImu       getHWVRPose
getHWVR6DofPose      getRawPose          resetPose
leftRudder           rightRudder
setDistortionParams  setDispresiveParams
```

`Hmd_pccast/Hmd.dll` 少 `getHWVR6DofPose`（13 个）——投屏版无 6DoF。这与本项目 `docs/01` 中"3DoF/6DoF 取决于版本"的结论吻合。

### 4.5 显示热插拔与网络

| 证据 | 含义 |
|---|---|
| `hwvr_DisplayManager.dll` 导出 `DisplayPlugIn` / `DisplayUnplug` `[铁证]` | 虚拟显示器插入/拔出 |
| 导入 `CM_Get_Device_Interface_ListW`、`DeviceIoControl`、`CreateFileW` `[铁证]` | 通过设备接口 + IOCTL 控制虚拟显示驱动 |
| `PCCastScreen.exe`：`[%s] system detected a new device`、`virtualDisplayFlag=%d gpuType=%d 1 NVIDIA,2 HD Graphics` `[强证据]` | 新设备检测 + GPU 类型分支 |
| `HwPCVRAssistant` 导入 `wlanapi.dll`（`WlanEnumInterfaces`/`WlanGetAvailableNetworkList`）、`SensApi!IsNetworkAlive` `[铁证]` | Wi-Fi / 网络在线状态 |
| `[%s]: locator status %d`、`label_LocatorConnect` `[强证据]` | 定位器（基站）连接状态 |

### 4.6 手机状态——本包内**没有** adb 逻辑（重要澄清）

对全部 36 个组件扫描 `adb`、`dumpsys`、`android`、`phone`、`getprop` 等关键词：**零命中**。

原因：这套是 **PC VR（眼镜插 PC 当显示器）** 方案，手机串流路径不在其中。本项目 `status.rs` 中的 `adb devices` + `dumpsys battery` 是**上游 ALVR 客户端**的做法，来源不同，不是华为这套 Windows 组件的行为。结论：**保留 adb 逻辑没问题，但不要把它归为"华为原厂方式"**。

---

## 5. 与当前 `src/status.rs` 的对比与修正建议

| 状态项 | 本项目当前实现 | 原厂实际方式（本报告） | 处置 |
|---|---|---|---|
| 头显是否插上 | WMI `WmiMonitorBasicDisplayParams` EDID 关键字 + PowerShell `Get-PnpDevice` 查 `VID_12D1` | `SetupAPI` + `HID`（`HidD_GetAttributes`）+ EDID `edidVid/edidPid` | **加强**：补 PID 白名单 `1092/3A09`，EDID 用 VID/PID 而非仅关键字 |
| PC 电池 | `Get-CimInstance Win32_Battery` | `kernel32!GetSystemPowerStatus` → `BatteryFlag`/`ACLineStatus` | **修正**：改用系统 API，WMI 作回退 |
| 手柄状态 | 无 | 命名共享内存 `HwControllerSHARED` + `GetShareData` | **新增**（可选）：接入左右手柄连接/电量 |
| 手机状态 | `adb devices` + `dumpsys battery` | 本包内不存在 | **保留**：来源为上游 ALVR，标注清楚 |
| DFU | `12D1:1093` | 升级器路径已知 | 正确，保留 |
| HMD 电池回传 | 头显在线时用 PC 电池代表 | 物理事实一致（头显无独立电池） | 正确，保留 |

### 5.1 修正 PC 电池：用 `GetSystemPowerStatus`（可直接落地，无需新 crate）

Windows GNU target 下可直接声明系统 API：

```rust
#[cfg(windows)]
#[repr(C)]
struct SystemPowerStatus {
    ac_line_status: u8,      // 0= Offline(电池), 1= Online(插电), 255= 未知
    battery_flag: u8,        // bit0=高,bit1=低,bit2=临界,bit3=充电,bit4=无电池, 255=未知
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
    let mut ps = SystemPowerStatus {
        ac_line_status: 255, battery_flag: 255, battery_life_percent: 255,
        battery_life_time: u32::MAX, battery_full_life_time: u32::MAX,
        battery_voltage: u32::MAX, battery_capacity: u32::MAX,
    };
    let mut b = PcBattery::default();
    unsafe {
        if GetSystemPowerStatus(&mut ps) == 0 { return b; }
    }
    const NO_BATTERY: u8 = 0x80;
    const CHARGING:   u8 = 0x08;
    if ps.battery_flag != 255 && (ps.battery_flag & NO_BATTERY) == 0 {
        b.present = true;
        if ps.battery_life_percent != 255 {
            b.pct = Some(ps.battery_life_percent as u32);
        }
        b.charging = ps.ac_line_status == 1 || (ps.battery_flag & CHARGING) != 0;
    }
    b
}
```

对照原厂日志 `BatteryFlag = %d, ACLineStatus = %d` —— 这就是同一组字段。

### 5.2 加强头显：补齐 PID 白名单

把 `collect_headset()` 的 USB 判据从"任意 `VID_12D1`"收窄/标注为已知 PID：

```rust
const HMD_PIDS: &[&str] = &["1092", "3A09"]; // 原厂 Check6DOFDevicePlugin / 3a09 12d1
```

EDID 侧在现有关键字基础上，若能从 `WmiMonitorID` 取 `ManufacturerCode`/`ProductCode`，优先按 `12D1` VID 与 `1092/3A09` PID 判定，比字符串关键字更稳。

### 5.3 新增手柄共享内存（`GetShareData`，可选增强）

`ControllerStatus.dll` 的 `GetShareData` 是 `extern "C"` 无参导出，返回指向共享结构的指针（推断，`HwControllerSHARED` 段）。若要把手柄状态接入状态面板：

1. 运行时 `LoadLibraryW(\steamVRDriver\huaweiVR\bin\win64\ControllerStatus.dll)`（路径来自字符串实证）；
2. `GetProcAddress(h, "GetShareData")`；
3. 按返回结构读 `HeadConnectState` / `LeftHandleConnectState` / `RightHandleConnectState` / `left_battery` / `right_battery`；
4. 结构布局需真机/进一步反汇编确认——**不要猜字段偏移上线**。

---

## 6. 可信度与未验证项

- **已确证**：文件类型与来源、包结构、全部导出/导入表、VID/PID 清单、电池 API、共享内存段名、监控线程名、PDB 源码路径。
- **强证据但未逐指令验证**：各监控线程的调用顺序、EDID 读取的具体 WMI 类、6DoF 设备判据细节。
- **推断/待真机验证**：`GetShareData` 返回结构布局；`GetNotebookPCPowerStatus` 的确切算法（是否融合 NVIDIA API）；手柄电量的具体字节位置。
- **本沙箱无法验证**：真机插拔行为、SteamVR 联动、WiFi/定位器状态。

---

## 7. 附录

### 7.1 关键导出表（铁证汇总）

| 模块 | 导出函数 |
|---|---|
| `ControllerStatus.dll` | `GetShareData`, `SetShareData` |
| `Hmd.dll` | `createHmdDriver`, `releaseHmdDriver`, `getVersionInfo`, `enableHmdImu`, `disableHmdImu`, `getHWVRPose`, `getRawPose`, `getHWVR6DofPose`, `resetPose`, `leftRudder`, `rightRudder`, `setDistortionParams`, `setDispresiveParams` |
| `hwvr_DisplayManager.dll` | `DisplayPlugIn`, `DisplayUnplug` |
| `hwvr_api.dll` | `GetIhwvrClient` |
| `HwPCVR.dll` | `HwPCVR` |
| `HiORBLib.dll` | `ExtractFeature`, `GetExtractFeatureBufferSize` |

### 7.2 关键导入 API（铁证汇总）

| 用途 | DLL / API |
|---|---|
| 设备枚举 | `SETUPAPI!SetupDiGetClassDevsW/SetupDiEnumDeviceInterfaces/SetupDiGetDeviceInterfaceDetailW` |
| HID 通信 | `HID!HidD_GetHidGuid/GetAttributes/GetPreparsedData/SetFeature`, `HidP_GetCaps` |
| 电池 | `KERNEL32!GetSystemPowerStatus` |
| 显示器 | `USER32!EnumDisplayDevicesW/EnumDisplayMonitors/GetMonitorInfoW/QueryDisplayConfig/SetDisplayConfig` |
| 虚拟显示 | `CM_Get_Device_Interface_ListW`, `KERNEL32!DeviceIoControl/CreateFileW` |
| 网络 | `wlanapi!WlanEnumInterfaces/WlanGetAvailableNetworkList`, `SensApi!IsNetworkAlive` |
| 电源管理（驱动） | `QCDriverPowerManagementEnabled`, `QCDriverPowerNotify`, NVIDIA `nvpowerapi.dll` |

### 7.3 复现命令

```bash
# 1) 从资料库取真实文件（本地 uploads 里的只是 HTML 壳）
python3 /root/.codebuddy/skills/library/space_api.py space.workspace.node-info --node-id paCizeu549cOgEahDdfpsH
python3 /root/.codebuddy/skills/library/drive/get_download_link.py --node-id paCizeu549cOgEahDdfpsH
# 2) 下载后用 unrar（7z 23.01 不支持该 RAR5 方法）
unrar x -o+ work_real.rar work_extract/
# 3) 导出/导入/字符串分析脚本见本报告工作目录：pe_scan.py, str_scan.py
```

---

*报告生成于对 `work.rar`（真实本体 16.4 MB，36 个 native x64 PE）的静态逆向；所有"铁证"级结论均来自 PE 导出表/导入表，可复现。*
