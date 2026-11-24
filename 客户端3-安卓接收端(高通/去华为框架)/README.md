# 通用安卓 ALVR 接收端（去华为化）

一个**完全不依赖华为 VR 框架**的 Android ALVR 接收端工程：手机运行标准 OpenXR 客户端接收高配 PC 的 ALVR 串流，双目 SBS 显示到眼镜/外接屏。基于对 `niceqwer55555/HuaweiVRGlass-ALVR-Android` 与 `EnYaoJingYa/huawei-vr-compat` 的逆向与重构，把其中所有华为私有依赖替换为通用实现，并针对**一加 7T / 骁龙 855 Plus / Android 11** 适配。

## 核心特性

| 需求 | 实现 |
|---|---|
| 不依赖华为 VR 框架 | 标准 Khronos OpenXR loader + 标准 OpenXR 1.0 ABI；无 `libxr_loader.so`/`libvrsdk.so`/`libvrpose.so`/`com.huawei.*` 权限 |
| 高通解码器 | `c2.qti.avc.decoder` → `OMX.qcom.video.decoder.avc` 优先级；**禁止软解**，无硬解则报错不静默降级 |
| 3DoF/6DoF 手柄 | 双模聚合器；每只手柄独立 Mahony 姿态解算；位置有效位自动切换 3DoF/6DoF |
| 自己算姿态 | 陀螺积分 + 加速度重力校正（Mahony 互补滤波），纯 C 数学，零平台依赖 |
| 姿态双通道 | 手机 IMU（TYPE_GYROSCOPE/ACCELEROMETER/ROTATION_VECTOR）+ localhost UDP 47110 外部注入，运行时可切换 |
| 眼镜状态/设备 | 标准 HID feature report（协议应答、不写死 VID/PID）；接近/电量用 `TYPE_PROXIMITY` + `BatteryManager` 等价（CV10 无独立电池） |
| 目标机型 | 一加 7T（SM8150 / 骁龙 855 Plus / Adreno 640 / GLES 3.2 / Android 11） |

## 目录结构

```
hvr-alvr-android-generic/
├── src/
│   ├── pose/         姿态自算（pose.h/c）—— Mahony + 双数据源
│   ├── controller/   手柄聚合（controller.h/c）—— 3DoF/6DoF + 按键
│   ├── decoder/      高通解码器策略（decoder_policy.h/c）—— 禁软解
│   ├── runtime/      通用 OpenXR runtime 平台桥（generic_runtime.h/c）
│   └── hid/          眼镜 HID 协议（hwvr_hid.h/c）—— 协议应答、输入报告解析
├── rustsrc/          客户端配置/协议逻辑 crate（可独立 cargo test）
├── android/          Java host + JNI 桥 + AndroidManifest（去华为权限）
├── app/              active_runtime.json（标准 loader 发现）+ client_config.json
├── tests/            C 核心单测（gcc 可跑）
├── scripts/          build.sh 一键构建
└── docs/             架构/去华为对照/构建/姿态手柄/解码器文档
```

## 验证状态（本沙箱真实执行）

| 项 | 结果 |
|---|---|
| C 核心单测（姿态/手柄/解码器/runtime 桥/HID 协议/三设备指示灯） | `gcc` 编译运行 **56/56 PASS** |
| Rust 客户端逻辑单测 | `cargo test` **8/8 PASS** |
| 去华为化静态检查 | 候选链无任何 `hisi`/华为组件 |
| APK 编译 | 沙箱无 Android SDK/NDK，**未在沙箱产出 APK**；`scripts/build.sh` 已备好，真机编译步骤见文档 |

## 快速开始

```bash
# 1. 沙箱内验证核心逻辑
cd tests && ./run_tests.sh

# 2. 有 Android SDK/NDK 的环境构建
export ANDROID_NDK_HOME=/path/to/ndk
export ANDROID_SDK_ROOT=/path/to/sdk
./scripts/build.sh

# 3. 安装到手机（眼镜插手机 USB-C）
adb install -r build/generic-alvr.apk
```

详解见 [docs/02-构建与运行.md](docs/02-构建与运行.md)。
