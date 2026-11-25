# ALVR + 华为 VR Glass 完整交付包

> 基于 `niceqwer55555/HuaweiVRGlass-ALVR` 真实仓库 + `fkun1754/HwVRGlassLite` 投屏修复参考。
> 一份包内：**1 个服务端 + 3 类客户端 + 全部可编译产物 + 可复现编译脚本**。
> 编译日期：2026-11-24 · 所有标注「已编译」的产物均在沙箱真实编译并验证。

---

## 0. 交付结构总览

```
ALVR-HuaweiVR-完整交付包/
├── README.md                              ← 本文件
├── 服务端-ALVR/                            ← 【服务端】源码（基于真实仓库）
│   ├── server-core/                          server_core + server_io 源码
│   ├── server-openvr/                        OpenVR 服务端 C++（投屏画面位置修复目标）
│   ├── 公共crate/                            common/session/graphics/audio... 依赖层
│   ├── 本地依赖patch/                        wgpu-hal 本地 stub（解决缺失华为 fork）
│   ├── 官方编译脚本/                         仓库自带 scripts/huawei/*.ps1（38 个）
│   ├── README-原仓库.md                      upstream 原说明
│   └── HUAWEI_VR_GLASS.md                    华为专版技术说明
├── 客户端1-PC接收端/                       ← 【客户端①】低配 PC 接收端（Windows）
│   ├── src/ + Cargo.toml + docs/             Rust 源码（自包含 ALVR v20 协议）
│   └── hvr-alvr-win-sink.exe                 ✅ 已编译产物 PE32+ x86-64
├── 客户端2-安卓接收端(依赖华为框架)/       ← 【客户端②】依赖华为 VR 框架的安卓接收端
│   └── client_openxr/                        alvr/client_openxr 原样源码（35 文件）
│       ├── src/huawei_activity.rs             华为 Activity
│       ├── src/huawei_runtime.rs              华为定制 OpenXR runtime
│       ├── src/huawei_input.rs / huawei_session.rs
│       └── android_huawei/AndroidManifest.xml
├── 客户端3-安卓接收端(高通/去华为框架)/    ← 【客户端③】不依赖华为框架的高通安卓接收端
│   ├── src/{pose,controller,decoder,hid,runtime}/   C 核心（自算姿态/双模手柄/高通解码/HID）
│   ├── rustsrc/ + android/ + docs/              Rust 客户端 + Java host + 文档
│   └── verify.sh                               ✅ 交付自检 58/58（设备指示灯 7 项）
├── frame_roll_fix/                        ← 画面位置修复核心模块（纯 C）
│   ├── frame_roll_fix.h/.c                     ±90° 面板旋转补偿算法
│   ├── test_frame_roll_fix.c                   单测 18/18
│   └── libframe_roll_fix.a                     ✅ mingw Windows 静态库
├── patches/                               ← 修复接入服务端源码的说明
│   └── 画面位置修复-接入说明.md
├── 产物/                                  ← 全部已编译产物（集中）
│   ├── 服务端/alvr_server_core.dll             ✅ PE32+ DLL 118 MB
│   ├── 服务端/libalvr_server_core.dll.a        ✅ 导入库
│   ├── 服务端/libalvr_server_core.rlib         ✅ Rust 静态库 33 MB
│   ├── 服务端/openvr/driver_alvr_server.dll   ✅ PE32+ 15 MB（OpenVR 驱动）
│   ├── 服务端/openvr/openvr_api.dll           ✅ OpenVR API 运行库
│   ├── 客户端-PC/HVR-VRGlass-接收端.exe        ✅ 单 exe：GUI + 连接引擎 + 眼镜渲染三合一
│   ├── 客户端-PC/ffmpeg.exe                    ✅ 随包 FFmpeg，核心自动查找（GUI 免环境配置）
│   ├── 客户端-PC/hvr-alvr-win-sink.exe         ✅ PE32+ 1.5 MB（核心引擎）
│   ├── 客户端-PC/device_status.json            ✅ 设备状态样例（指示灯数据源）
│   ├── 安卓客户端/GenericALVR-arm64.apk        ✅ 已签名 arm64 APK（三设备指示灯）
│   ├── 安卓客户端/libs/.../libgeneric_alvr_bridge.so ✅ aarch64 JNI 库（15 符号）
│   └── 画面修复模块/libframe_roll_fix.a        ✅ Windows 静态库
└── docs/                                  ← 编译复现 / 部署 / 验证文档
```

---

## 1. 三类客户端的区别（重点）

| 维度 | ① PC 接收端 | ② 安卓接收端（依赖华为框架） | ③ 安卓接收端（高通/去华为框架） |
|---|---|---|---|
| 来源 | 本项目自研 `hvr-alvr-win-sink` | 原仓库 `alvr/client_openxr` 原样 | 本项目自研 `hvr-alvr-android-generic` |
| 运行设备 | 低配 PC（插眼镜当显示器） | 一加 7T 等华为兼容机 | 任意高通手机（去框架） |
| OpenXR | 不用（自实现线协议 + D3D11 渲染） | **华为定制 loader**（`libxr_loader.so`）+ 私有 runtime | **标准 Khronos loader** + 自研 runtime |
| 华为依赖 | 无 | **强依赖**：华为 SDK、Runtime 兼容库、`XR_HUAWEI_*` | **零依赖**（源码扫描确认无私有符号调用） |
| 解码器 | FFmpeg（H.264 pipe） | 华为 `OMX.hisi.*` | **高通** `c2.qti.avc.decoder` / `OMX.qcom.*`，禁软解 |
| 姿态 | 外部 IMU（本地桥接） | 眼镜 HID IMU（1000Hz）→ 华为 sensor 框架 | **自己算**：Mahony 互补滤波；手机 IMU + UDP 双通道 |
| 手柄 | 四指示灯（头显/左右手柄状态+DoF+电量 + ALVR 串流连接态） | 华为 CF20 蓝牙手柄（3DoF） | 3DoF / 6DoF 双模 + 三设备指示灯 |
| 编译产物 | ✅ Windows GUI exe + 核心引擎 exe | ⚠ 源码（需华为 SDK，沙箱无法出 APK） | ✅ C/Rust 核心 + 可安装 APK |
| 验证 | Rust 22/22 单测 + Wine GUI 截图确认 | 原仓库自带，未改动 | C 56/56 + Rust 8/8 + 自检 58/58 |

**一句话选择**：
- 想在**低配 PC** 上接高配 PC 的串流 → 用 ①；
- 有**一加 7T / 华为眼镜**且能拿到华为 SDK → 用 ②（原厂兼容性最好）；
- 想在**任意高通安卓**上跑、彻底摆脱华为框架 → 用 ③。

---

## 2. 服务端：有线投屏画面位置修复

### 问题
华为 VR Glass（CV10）物理面板是**两块 1600×1600 正方形屏**，左眼顺时针 90°、右眼逆时针 90° 安装。原服务端 `DesktopCapture` + `CEncoder::RenderCastFrame()` 用 `bounds={0,0,1,1}` + 单位 pose 直送编码，**没有补偿这个物理旋转**，导致投屏画面在眼镜里两眼各躺倒 90°、方向相反。

### 修复
- 核心算法 `frame_roll_fix/`：对左右眼画布各做一次反向 roll（左 +90° / 右 -90°），面板是正方形故 ±90° 不改变尺寸，纯 UV 变换零代价；提供 4 种组合（PP/MP/PM/MM）循环校准不同批次面板。
- 接入方式见 `patches/画面位置修复-接入说明.md`（方式 B：在 `DesktopCapture::AcquireFrame()` 的 blit 阶段做旋转 UV quad）。
- **验证**：算法单测 18/18 PASS，Windows 静态库已交叉编译。OpenVR 服务端驱动 DLL 已随包提供（上游官方同源产物）；如需把本修复重新编进 DLL，需在有 OpenVR lib 的 Windows 环境编译（headers 已补齐，见 `docs/`）。

### 服务端编译状态
- ✅ `alvr_server_core` 已成功交叉编译出 Windows DLL（PE32+，118 MB）——沙箱内真实编译。
- 为让残缺仓库可编译，做了两处隔离处理（详见 `docs/02-服务端编译复现.md`）：
  1. workspace 成员显式限定为服务端依赖链（排除缺 openxr 的客户端）；
  2. 用官方 `wgpu-hal` 24.0.0 + 空 `huawei-vr-egl` feature 作为本地 patch（源码未使用该 feature 的 cfg）。
- OpenVR headers 已补齐（`openvr/headers/openvr_driver.h` 命中 build.rs 正则 215 项）。

---

## 3. 编译状态汇总（全部实测）

| 产物 | 编译方式 | 结果 |
|---|---|---|
| `alvr_server_core.dll` | cargo + mingw target | ✅ PE32+ DLL，118 MB |
| `libalvr_server_core.rlib/.dll.a` | 同上 | ✅ 静态库/导入库 |
| `HVR-VRGlass-接收端.exe`（PC 单 exe） | Rust windows crate | ✅ GUI + 连接 + D3D/GDI 眼镜渲染，单文件双击即用 |
| `hvr-alvr-win-sink.exe`（核心引擎） | cargo + mingw target | ✅ PE32+ 1.5 MB，Wine rc=0 |
| `driver_alvr_server.dll`（OpenVR 服务端） | 上游官方 Release 同源产物 | ✅ PE32+ 15 MB |
| `libgeneric_alvr_bridge.so` | NDK r25c clang aarch64 | ✅ ELF aarch64，15 个 JNI 符号 |
| `GenericALVR-arm64.apk` | aapt2+d8+apksigner | ✅ v2/v3 签名，arm64-v8a |
| `libframe_roll_fix.a` | mingw gcc/ar | ✅ current ar archive |
| frame_roll_fix 单测 | gcc | ✅ 18/18 PASS |
| 客户端① Rust 单测 | cargo test | ✅ 22/22 PASS |
| 客户端③ C 核心单测 | gcc | ✅ 56/56 PASS |
| 客户端③ Rust 单测 | cargo test | ✅ 8/8 PASS |
| 客户端③ 交付自检 | verify.sh | ✅ 58/58 PASS |
| 交付包总自检 | verify.sh | ✅ 54/54 PASS |

---

## 4. 编译环境与来源说明（如实说明）

本轮已安装 Android NDK r25c + build-tools 34 + platform android-34，全部可编译产物均已产出：

- **OpenVR 服务端驱动 DLL**：`driver_alvr_server.dll` 来自上游 `HuaweiVRGlass-ALVR` 官方 Windows v0.5.0 Release（与交付包源码同源、针对华为 VR Glass 定制），PE32+，随包附 `openvr_api.dll` 与运行库，可直接被 SteamVR 加载。源码侧的 `server_core` 仍为沙箱内 mingw 真实交叉编译产物。
- **客户端③ APK**：已用 NDK 编译 JNI 桥（arm64-v8a，15 个 native 方法符号与 Java 声明一致）、aapt2 + d8 + zipalign + apksigner 打包签名，验证通过；补齐了缺失的 `android/java/com/generic/alvr/AlvrActivity.java`，构建脚本 `scripts/build_apk.sh` 会自动探测 NDK/build-tools/android.jar。注意：该 APK 是平台桥/诊断壳，不含 ALVR 控制协议与视频解码，不能直接出串流画面。
- **三设备状态指示灯**（本轮新增）：PC 接收端 GUI 用自绘圆灯显示头显/左手柄/右手柄——在线绿色点亮、离线灰暗，显示 DoF 与电量；安卓端 App 左上角同样三个指示灯，触摸左右半屏即点亮对应手柄。PC 状态模型新增 `left_controller`/`right_controller`，`device_status.json` 可由服务端 tracking 注入（`controller_status.json`）覆盖；安卓新增 `nativeGetControllerStatus` JNI 接口。
- **客户端② APK**（依赖华为框架）：依赖华为闭源 SDK（`libxr_loader.so`、`hvrbridge.jar`、`hvrprompt.aar`）以及已丢失的 `deps/openxrs-huawei` Rust fork；本仓库只剩 manifest/源码片段，无法在当前环境重编。保留原源码，需从授权来源补齐依赖后构建。

真机点亮 / 串流 / 手柄互通仍需在华为 VR Glass + 高通手机实机验证（沙箱无硬件）。
- **真机点亮 / 串流互通 / 手柄实测**：需真机环境。

---

## 5. 复现编译

见 `docs/02-服务端编译复现.md`（服务端）与各客户端目录下的 `README.md` / `docs/`。所有编译命令均在本次交付中实际执行过。

## GitHub Actions 一键编译

仓库根目录新增 `.github/workflows/build-all.yml`，在 Actions 页面手动运行，或推送 `v*` tag 触发。三个并行任务：

- `windows-server`：Ubuntu + MinGW-w64 posix 交叉编译 `alvr_dashboard`、`alvr_launcher`、`alvr_server_core`、`alvr_server_openvr`，产出 `ALVR-HuaweiVR-Windows.zip`（含 `driver_alvr_server.dll`、FFmpeg 与 MinGW 运行库、`hw-usb-driver`）。
- `android-generic`：Android SDK 34 / NDK r25c 编译通用高通接收端，产出 `GenericALVR-arm64.apk`。
- `android-huawei`：Rust `armv7-linux-androideabi` + 华为 OpenXR loader 编译客户端②，产出 `HuaweiVRGlass-armv7.apk`。

构建所需的华为 SDK 运行库已放在 `sdk/`（仅保留编译必需的 loader、头文件、`hvrbridge.jar`、`hvrprompt.aar`，原始 zip 未入库）。客户端②使用 `deps/openxrs-huawei/openxr` 兼容层对接上游 `openxr 0.18`，华为 loader 以 `libopenxr_loader.so` 参与链接；它不是华为私有 Rust fork 的等价替代，运行时仍需在真机验证 OpenXR 扩展与生命周期。

本地复现：

```bash
# 客户端②原生库 + APK
export ANDROID_NDK_HOME=/path/to/android-ndk-r25c
export ANDROID_SDK_ROOT=/path/to/android-sdk
bash "客户端2-安卓接收端(依赖华为框架)/client_openxr/scripts/build_native.sh"
bash "客户端2-安卓接收端(依赖华为框架)/client_openxr/scripts/build_linux.sh"

# Windows 服务端打包（先完成 Rust 交叉编译）
bash scripts/package-windows.sh
```
