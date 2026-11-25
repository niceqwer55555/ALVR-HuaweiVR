# 通用安卓 ALVR 接收端（去华为化）

一个**完全不依赖华为 VR 框架**的 Android 平台桥 / 诊断壳：负责手机 IMU 姿态、外部姿态注入、高通解码器枚举、设备与手柄状态显示，并预留 SurfaceView 给后续 OpenXR/解码渲染。

> 边界说明：本工程**不是**完整 ALVR 串流客户端。ALVR 控制协议、H.264 解码和双目 SBS 渲染需要 `alvr_client_core` + OpenXR runtime，当前仓库未包含这部分源码；APK 可安装、可验证 JNI/传感器/解码器，但不会直接出串流画面。

## 核心特性

| 需求 | 实现 |
|---|---|
| 不依赖华为 VR 框架 | 无 `libxr_loader.so`/`libvrsdk.so`/`libvrpose.so`/`com.huawei.*` 权限；平台桥只依赖 Android 标准 API |
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
| APK 编译 | **已产出** `build/GenericALVR-arm64.apk`（29 KB，arm64-v8a，v2/v3 签名）；aapt2 显示 `native-code: arm64-v8a`、`launchable-activity: com.generic.alvr.AlvrActivity` |

## 快速开始

```bash
# 1. 沙箱内验证核心逻辑
cd tests && ./run_tests.sh

# 2. 构建 APK（脚本自动探测 NDK / build-tools / android.jar）
./scripts/build_apk.sh
# 产物：build/GenericALVR-arm64.apk

# 3. 安装到手机（眼镜插手机 USB-C）
adb install -r build/GenericALVR-arm64.apk
```

详解见 [docs/02-构建与运行.md](docs/02-构建与运行.md)。
