# huawei-vr-compat 分析结论与骁龙 855 Plus / 一加 7T / Android 11 适配

> 参考仓库：`EnYaoJingYa/huawei-vr-compat`（HMA-AL00 / EMUI 11 + 华为 VR Glass CV10 兼容层）
> 本文回答两件事：**手柄和姿态到底怎么获取和计算**，以及**如何适配到一加 7T（SM8150 / 骁龙 855 Plus / Android 11）**。

## 一、参考仓库揭示的数据链路（实证）

### 1. 眼镜控制通道：标准 HID，无私有 ABI

`docs/HELMET_HID_PROTOCOL.md` 的结论（反汇编 `libWalleDev.so` / `libvrinfo.so` + 内核镜像对照）：

> "所谓华为私有驱动接口全部是标准 HID `HIDIOCGFEATURE`/`HIDIOCSFEATURE` 透传，没有任何华为私有内核 ABI。"

| 功能 | ioctl | report id | 长度 |
|---|---|---|---|
| 读厂商号 | `HIDIOCGFEATURE(4)` = `0xC0044807` | 1 | 4 |
| 读眼镜版本 | `HIDIOCGFEATURE(62)` | 2 | 62 |
| 读 Halliday 版本 | `HIDIOCGFEATURE(80)` | 2 | 80 |
| 读光学信息 | `HIDIOCGFEATURE(48)` | 9 | 48 |
| 读显示偏移 | `HIDIOCGFEATURE(16)` | 9 | 16 |
| 激活/去激活 | `HIDIOCSFEATURE(5)` = `0xC0054806` | 5 / 6 | 5 |

**关键设计**：靠"协议应答"挑接口——发 `report id=1` 的 GET_FEATURE(4)，谁应答谁就是眼镜控制接口，**不写死 VID/PID**。本工程 `src/hid/hwvr_hid.c` 完全采用此设计（`ga_hid_looks_like_helmet`）。

### 2. 眼镜 IMU / 状态：HID 输入报告 + Android 动态传感器

- 眼镜 IMU 经 `/dev/hwvr1` 的 HID **输入报告**（`read()`）持续推送；EMUI `sensors.default.so` 把它注册成**动态传感器**进 sensor 框架。
- 输入报告格式：`<8 字节头> [buf[8]=类型] [buf[9..]=负载>`，类型：`2=接近 / 3=心跳 / 5=电量 / 6=耳机 / 7=过热`。
- 兼容层实测 **IMU 1000 Hz 无丢帧**，Unity 空间与 3DoF 手柄识别正常。
- CV10 眼镜的 HID 报告描述符里**没有接近(report 4)和电量(report 8)**，原版 `/dev/ttyUSB1` 读法在通用安卓拿不到值 → 改用 Android 自己的 `TYPE_PROXIMITY` + `BatteryManager`（眼镜由手机供电，无独立电池）。

### 3. 手柄：CF20 蓝牙手柄 + 输入事件回调

- 手柄包 `com.huawei.vrhandle`，**蓝牙私有协议 + 广播**（`HUAWEI_VR_SYSTEM_IFACES.md`）。
- 输入经 `VrTouchInterfaceEx.registerInputEventCallback` 进 VR 空间（`InputEvent` → 回调）。
- 手柄是 **3DoF**（只有旋转），近场配对用华为 `nearbysdk`（通用安卓上摘除）。
- 电量/佩戴状态经 `IVRListener.onBatteryChanged` / 佩戴事件（`VrSystemActor` 的 "handle Helmet Up"）。

### 4. 姿态"计算"在哪

参考仓库是**兼容层**：它负责把数据"取到"，姿态解算本身在华为 VR SDK / Unity XR（3DoF）里完成——仓库明确"异步头部跟踪实现留待后续补齐"。所以**姿态计算要用我们自己的实现**，即本工程 `src/pose/pose.c` 的 Mahony 互补滤波。这是对参考仓库的正确补位：它解决了"怎么获取"，我们解决"怎么算"。

## 二、迁移到通用工程的对应实现

| 参考仓库结论 | 本工程实现 |
|---|---|
| 标准 HID feature report 透传 | `src/hid/hwvr_hid.c`：ioctl 常量 + 报文构造/解析 |
| 协议应答挑接口（不写死 VID/PID） | `ga_hid_looks_like_helmet()` |
| HID 输入报告类型 2/3/5/6/7 | `ga_hid_parse_input()` + `ga_status_apply_input()` |
| IMU 1000Hz 动态传感器 | `src/pose`：陀螺积分 + 重力校正，dt 钳制支持高频 |
| 3DoF 手柄 + InputEvent 回调 | `src/controller`：CTRL_DOF_3 + 按键/轴输入 |
| 接近/电量用 Android 等价源 | Java host：TYPE_PROXIMITY + BatteryManager（CV10 无独立电池） |
| 姿态解算在 SDK（仓库未实现） | 本工程 Mahony 自算（补位） |

## 三、一加 7T / SM8150 / 骁龙 855 Plus / Android 11 适配参数

| 项 | 参数 | 依据/处理 |
|---|---|---|
| SOC | SM8150，骁龙 855 Plus | Kryo 485，8 核；arm64-v8a 为主 ABI |
| GPU | Adreno 640 | GLES 3.2，swapchain 用 `GL_RGBA8`（ALVR 硬要求） |
| ABI | arm64-v8a（主）+ armeabi-v7a（兼容） | build.sh 默认 aarch64-linux-android21 |
| Android 版本 | Android 11（API 30） | targetSdk 30；`minSdk` 26；scoped storage 下配置放 app 私有目录 |
| 解码器 | `c2.qti.avc.decoder` 优先，`OMX.qcom.video.decoder.avc` 兼容 | 855 有高通 Codec2 + OMX；**禁软解**（`no_software_decoder=true`） |
| 姿态源 | 手机 IMU（TYPE_ROTATION_VECTOR/GYROSCOPE/ACCELEROMETER） | 855 Plus 有完整 IMU；1000Hz 需 `SENSOR_DELAY_FASTEST` |
| 手柄 | CF20 蓝牙手柄 3DoF + 触摸虚拟手柄 | `CTRL_DOF_3`；蓝牙 InputEvent → 按键/轴 |
| 头显 | 华为 VR Glass CV10（接手机 USB-C） | 3DoF；SBS 3200×1600（各眼 1600×1600） |
| 权限 | 无 `com.huawei.*` | 标准网络/传感器权限；去 VR headtracking required |

### Android 11 特别注意

1. **Scoped Storage**：`active_runtime.json` / `client_config.json` 放 `getFilesDir()`，不要写 `/sdcard` 根。
2. **包可见性**：Android 11 的 `<queries>` 需要声明要交互的包；本工程不查华为包，无需 `queries`。
3. **传感器采样率**：1000Hz 需 `TYPE_GYROSCOPE` 支持 `SENSOR_DELAY_FASTEST`（855 Plus 机型支持），并在 JNI 侧做 dt 钳制（已实现）。
4. **前台服务**：串流是长时任务，建议用前台服务 + 通知保活（`WAKE_LOCK` 已声明）。

## 四、适配后的验证

- C 核心单测 **56/56**（含 HID 协议 14 项、三设备指示灯状态/电量 5 项）
- Rust 客户端单测 **8/8**
- 交付自检 `verify.sh` 全过

真机（一加 7T / Android 11）验证步骤见 `docs/02-构建与运行.md`；需在有 Android SDK/NDK 的环境构建 APK。
