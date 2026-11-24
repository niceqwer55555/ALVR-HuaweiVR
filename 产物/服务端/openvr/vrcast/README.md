# vrcast - Windows 桌面投屏到 OpenVR 头显（原型）

`vrcast` 是一个干净室实现的原型：用 Windows Desktop Duplication 捕获桌面，用
D3D11 把桌面渲染成一块虚拟屏幕，再通过 OpenVR compositor 提交到华为 VR Glass /
SteamVR 头显。3DoF/6DoF 姿态直接复用已安装的 SteamVR/Huawei/NOLO 驱动，不重建
SLAM。

> 状态：源码骨架，**未在 Windows 实机完成编译或实机验证**。当前分析工作区没有
> MSVC、CMake、SteamVR 或头显；代码级完善（运行中键盘微调、DDA 故障恢复、
> sRGB 直通、ABI 自检）于 2026-10-26 完成，OpenVR shim 的结构体布局已通过
> 平台无关的编译期断言（见“ABI 自检”一节）。所有驱动/IOCTL 结论来自静态逆向，
> 运行时行为需要在目标机器上按本文的检查清单确认。

## 它能做什么

- 枚举 DXGI 输出，选择任意桌面/虚拟显示器作为捕获源。
- 通过 Desktop Duplication 获取 BGRA 桌面纹理。
- 为左右眼各渲染一张透视校正图像；默认读取当前 OpenVR HMD 的推荐分辨率
  （Huawei 通常为 1600×1600，ALVR 按其虚拟 HMD 上报值），也可用
  `--eye-width/--eye-height` 固定。
- 通过 `IVRSystem::GetProjectionMatrix` 和 `GetEyeToHeadTransform` 读取每眼
  精确投影与 IPD；运行时不可用时回退到 `--fov-deg/--ipd-mm`。
- 通过 `IVRCompositor::WaitGetPoses` 读取 HMD 的 3DoF/6DoF 姿态，把虚拟屏幕
  世界锁定（`world`）或头锁定（`follow`）。
- 可选地通过总线驱动 IOCTL 插入/移除厂商虚拟显示器，并在退出时清理。
- 附带 `vrcast_probe`，用于在没有头显的情况下探测虚拟显示器接口和两套 IOCTL。

## 它不做什么

- 不复制或修改厂商二进制，不使用 `project/build2` 里的地址/goto 伪 C。
- 不实现 6DoF SLAM；姿态来自现有 SteamVR 驱动。
- 不修改 EDID、不进入 NVIDIA direct mode、不写注册表显示配置。
- 不在本程序内实现手机/无线串流的编码、网络和客户端；无线传输复用外部 ALVR
  方案（见“无线串流”一节），`vrcast` 只作为 SteamVR 场景应用提交画面。
- 不处理音频；有线/无线音频由 SteamVR 或 ALVR 负责。
- 不做镜头畸变 shader（由 OpenVR compositor 负责）。
- 不保证与所有 SteamVR 版本或所有厂商驱动版本兼容。

## 数据流

```text
Desktop / vendor virtual display
        │
        ▼
IDXGIOutputDuplication ──► D3D11 BGRA texture
        │
        ▼
D3D11 panel renderer (per eye, HMD-recommended size)
        │
        ▼
OpenVR IVRCompositor::Submit ──► SteamVR compositor ──► HMD (wired)
        ▲                              │
        │                              └─► ALVR SteamVR driver ──► Wi-Fi ──► client (wireless)
        │
IVRCompositor::WaitGetPoses (HMD pose, 3DoF/6DoF)
```

## 依赖

- Windows 10/11 x64。
- Visual Studio 2022，安装“使用 C++ 的桌面开发”和 Windows 10/11 SDK。
- CMake 3.21+。
- SteamVR 正在运行，且目标 HMD 驱动是当前驱动（华为/NOLO 直连，或 ALVR 的
  虚拟 HMD）。
- 可选：OpenVR SDK 的 `headers/openvr.h`。不提供时使用仓库内的最小 ABI shim。
  使用官方头文件时，版本应能与当前 SteamVR 的 `IVRSystem`/`IVRCompositor`
  接口匹配；本工具会请求头文件中声明的接口版本。
- 可选：厂商总线驱动 `virtual_display_device.sys`，仅在使用 `--vdisplay plug`
  时需要。

## 构建

```powershell
cd project\vrcast
cmake --preset vs2022
cmake --build --preset vs2022
```

或者：

```powershell
cmake -S . -B build -G "Visual Studio 17 2022" -A x64
cmake --build build --config Release
```

如果已有 OpenVR SDK：

```powershell
cmake -S . -B build -G "Visual Studio 17 2022" -A x64 `
  -DVRCAST_OPENVR_DIR="C:\path\to\openvr"
cmake --build build --config Release
```

产物：

- `build\...\Release\vrcast.exe`
- `build\...\Release\vrcast_probe.exe`

在 `build\vs2022\Release` 下运行，或把该目录加入 `PATH`。`openvr_api.dll` 需要能被
`vrcast.exe` 找到。程序依次尝试：

1. 当前目录 / `PATH` 中的 `openvr_api.dll`；
2. `%ProgramFiles(x86)%\Steam\steamapps\common\SteamVR\bin\win64\openvr_api.dll`；
3. `%ProgramFiles%\Steam\...`；
4. 注册表 `HKCU\Software\Valve\Steam\SteamPath` 下的 SteamVR 目录；
5. 注册表 `HKLM\SOFTWARE\WOW6432Node\Valve\Steam\InstallPath` 下的 SteamVR 目录；
6. 注册表 `HKLM\SOFTWARE\Valve\Steam\InstallPath` 下的 SteamVR 目录。

## 运行

先看输出列表：

```powershell
.\vrcast.exe --list-outputs
```

只验证 DDA 捕获（不需要头显）：

```powershell
.\vrcast.exe --capture-test 60 --output 0
```

正常投屏（推荐；分辨率/投影自动取当前 HMD）：

```powershell
.\vrcast.exe --output 1 --mode world
```

只有在运行时无法提供每眼参数，或需要覆盖时，才手动指定回退值：

```powershell
.\vrcast.exe --output 1 --eye-width 1600 --eye-height 1600 --ipd-mm 64 --fov-deg 90
```

插入虚拟显示器再投屏（测试机）：

```powershell
.\vrcast.exe --vdisplay plug --vdisplay-code auto --serial 42 --output 1
```

退出时默认会 `unplug` 本次插入的 serial 42；加 `--keep-vdisplay` 可保留。
运行中：

- `R`：把虚拟屏幕重新放到当前视线正前方（重新居中）。
- `[` / `]`：把屏幕移近 / 移远 0.1 m（0.2–10 m；world 模式下会顺带重新居中，
  follow 模式下下一帧自动跟随新距离）。
- `-` / `+`：把屏幕缩小 / 放大 10 %（高度 0.3–5 m，宽度按桌面纵横比自动缩放）。
- `Esc`：退出。

`--mode world`：屏幕固定在空间中，适合 6DoF 走动/转头观察。
`--mode follow`：屏幕跟随头部，类似 HUD；3DoF 头显建议先用这个模式。

## 虚拟显示器 IOCTL

静态逆向结论（详见 `project/WINDOWS_CASTING.md` §6.4）：

| 用途 | 厂商用户态 DLL | 包内总线驱动 |
| --- | --- | --- |
| plug | `0x002A4000` (METHOD_BUFFERED) | `0x002A6EE3` (METHOD_NEITHER) |
| unplug | `0x002A4004` (METHOD_BUFFERED) | `0x002A6EE7` (METHOD_NEITHER) |
| 第三控制 | 未发现 | `0x002A6EEB` (METHOD_NEITHER) |

plug 输入 62 字节：`DWORD 8` + `DWORD SerialNo` + UTF-16
`{HW}\VirtualDisplayDevice`。unplug 输入 16 字节：`DWORD 16` + `DWORD SerialNo`
+ 8 字节零填充。

`--vdisplay-code auto` 先试包内驱动使用的 `0x002A6EE3`，失败再试用户态 DLL
使用的 `0x002A4000`，并打印两边的 `GetLastError`。这正好用于实机确认
“用户态/驱动 IOCTL 不匹配”是否只是安装版本差异。没有 `0x2A4000` 的驱动、也没有
`0x2A6EE3` 的用户态 DLL 时，两者都可能失败；不要据此断言产品本身损坏。

## 3DoF / 6DoF

`vrcast` 不读 IMU、不做 SLAM。它调用：

```cpp
IVRCompositor::WaitGetPoses(renderPoses, ...);
// renderPoses[k_unTrackedDeviceIndex_Hmd].mDeviceToAbsoluteTracking
```

- 3DoF 驱动：姿态只有旋转，平移为 0；屏幕在 `world` 模式下保持在初始位置。
- 6DoF 驱动：姿态含平移；转头/走动时透视变化。
- 首次有效姿态前使用单位矩阵；运行中短暂丢失跟踪时保持最后一次有效头姿态，
  避免画面瞬间跳到世界原点。
- `R` 用当前 HMD 的 yaw/位置重新计算屏幕中心；只取 yaw，保持屏幕竖直。

默认从 `IVRSystem` 读取每眼精确投影矩阵和 eye-to-head 变换，因此 Huawei 的
非对称 FOV、ALVR 虚拟 HMD 的分辨率/IPD 都会自动生效。`--fov-deg`（默认 90°）
和 `--ipd-mm`（默认 64 mm）只在运行时拒绝这些查询时作为回退。若画面比例/位置
仍不对，优先调 `--panel-distance`、`--panel-height`，再检查 HMD 驱动上报值。

## 无线串流（复用 HuaweiVRGlass-ALVR）

无线部分**不需要在 `vrcast` 里重写 NVENC/网络协议**。已有开源方案：

- [LHT02/HuaweiVRGlass-ALVR](https://github.com/LHT02/HuaweiVRGlass-ALVR)
  （基于 ALVR 的 Huawei VR Glass 分支；许可证以该仓库为准）提供 Windows PC
  server、Android client、SteamVR 虚拟 HMD 驱动、视频编码和 Wi-Fi 传输。
- 该分支文档记录的真机参数（仅来自其文档，本仓库未复现）：Glass 外接显示
  3200×1600@70 Hz、OpenXR 推荐交换链 1552×1552/眼、左右眼 95°×95°、IPD 约
  63 mm。`vrcast` 不硬编码这些值，启动时从 `GetRecommendedRenderTargetSize`、
  `GetProjectionMatrix`、`GetEyeToHeadTransform` 读取；ALVR 改分辨率/视锥后
  仍以运行时上报为准。

推荐组合方式（`[推导]`，未在本工作区实测）：

1. 按该仓库说明安装 ALVR server、Android client 和 SteamVR 驱动；先让手机/VR
   Glass 能正常进入 SteamVR。
2. 启动 SteamVR，确认 ALVR 的虚拟 HMD 是当前 HMD（SteamVR 状态窗口能看到
   头显/手柄）。
3. 运行 `vrcast.exe --output N --mode world`。`vrcast` 只负责把桌面渲染成
   虚拟屏幕并 `Submit` 到 SteamVR compositor；ALVR 驱动再把 compositor 的
   画面编码后经 Wi-Fi 发给手机/Glass。
4. 先在 `--capture-test` 确认 DDA；再用 ALVR 的统计页看延迟、丢包和码率。

因此：

- `vrcast` 的“无线”能力来自 ALVR，不是本仓库新增的私有协议。
- `--vdisplay plug` 仍是本地虚拟显示器路径，和 ALVR 无线链路无关；无线时通常
  不需要插虚拟屏，直接抓现有桌面/扩展屏即可。
- 音频、手柄、6DoF 姿态由 ALVR/SteamVR 链路负责；`vrcast` 只消费
  `WaitGetPoses` 和 HMD 上报的投影。
- 不要同时启动两个 HMD 驱动（例如厂商直连驱动和 ALVR 驱动）争用 SteamVR；
  先退出旧运行时，再启动 ALVR。
- ALVR 的具体版本、Huawei VR Glass 固件、手机型号和 Wi-Fi 环境决定实际可用性；
  本仓库未验证。若 ALVR 的 compositor 输出和 `vrcast` 的提交时序冲突，先单独
  跑一个普通 SteamVR 场景应用确认 ALVR 链路正常，再叠加 `vrcast`。

## 安全与兼容性

- `--vdisplay plug` 会改变显示拓扑，可能黑屏/闪烁。只在测试机或可恢复环境使用。
  如果接口打开或 plug IOCTL 失败，程序会直接退出，不会继续捕获其他输出。
- serial 默认 42，避免和厂商 `hwvr_DisplayManager.dll` 的 4/5 冲突。若厂商应用
  已经在投屏，不要用同一个 serial 插入/拔出。
- 如果桌面输出和 HMD 不在同一块 GPU 上，OpenVR `Submit` 可能返回
  `TextureIsOnWrongDevice`。程序启动时会比较 `IVRSystem::GetDXGIOutputInfo`
  报告的适配器和 DDA 输出适配器，不一致会打印警告；优先选择与 HMD 同 GPU 的
  输出（通常 NVIDIA）。
- 当前 DDA 捕获使用输出所在适配器创建 D3D11 设备；没有跨适配器共享纹理逻辑。
- 眼纹理以 `D3D11_RESOURCE_MISC_SHARED` 创建，并在 `Submit` 前 `Flush`。若返回
  `SharedTexturesNotSupported`，确认 D3D11 device 在 `CreateDXGIFactory1` 之后创建
  （当前代码顺序已如此）。
- 色彩：DDA 若返回 `*_SRGB` 表面（`R8G8B8A8_UNORM_SRGB` 等），采样时以对应的
  `UNORM` 视图暴露原始字节，不做 sRGB→线性解码；眼纹理本身是 `UNORM`，因此
  提交给 compositor 的内容与桌面观感一致（字节直通）。这不是物理线性渲染，
  色准追求者可在 shader 中自行做 gamma 处理。
- `openvr_abi.h` 只声明本工具用到的 ABI 子集，并按 `IVRSystem_026` /
  `IVRCompositor_029+` 的槽位排列：`IVRSystem` 的 `ComputeDistortionSet` 占槽 4，
  因此 `GetEyeToHeadTransform` 是槽 5、`GetDXGIOutputInfo` 是槽 8；现代
  `IVRCompositor` 的 `GetSubmitTexture` 占槽 5，`Submit` 是槽 6。`openvr_bridge.cpp`
  优先调用 `VR_InitInternal2` 并通过 `VR_GetGenericInterface` 取得系统接口（用
  shim 时请求 `IVRSystem_026`；用官方头文件时请求该头文件的版本字符串）；
  compositor 也优先请求编译时匹配的版本，避免把旧布局误当成新布局。
  若 SteamVR 大版本再次改变接口，优先用官方 `openvr.h` 重新编译，而不是猜槽位。
- 现代 `openvr_api.dll` 可能不再导出旧的 `VR_InitInternal`；本工具同时尝试
  `VR_InitInternal2` 和旧导出，二者都不可用才报错。旧运行时若没有
  `IVRSystem_026`，程序会继续渲染，但精确投影/眼变换回退到 `--fov-deg`/`--ipd-mm`，
  日志中的 `eyeData=disabled` 表示这一情况。
- 本项目是原型，不是可直接分发的消费级投屏软件；没有安装器、签名、自动更新。

## 实机验证清单

1. `vrcast_probe --decode 0x002A6EE3`，确认解析为 `METHOD_NEITHER`。
2. `vrcast_probe --list`，确认能看到
   `{B85B7C50-6A01-11D2-B841-FF2210915171}` 接口。
3. `vrcast.exe --list-outputs`，确认目标显示器索引。
4. `vrcast.exe --capture-test 60 --output N`，确认 DDA 有帧且尺寸正确。
5. 在测试机运行 `vrcast_probe --plug --serial 42`，看哪套 IOCTL 成功；
   `vrcast_probe --unplug --serial 42 --code <成功的那套>` 清理。
6. 启动 SteamVR，运行 `vrcast.exe --output N --mode world`，确认左右眼图像、
   转头/走动、`R` 重新居中，并用 `[` `]`（距离）、`-` `+`（尺寸）微调屏幕；
   日志中 `eye render size`、`exactProjection`、
   `exactEyeToHead` 应与当前 HMD 相符。`OpenVR initialized` 行应显示
   `compositor=IVRCompositor_029`（或更高兼容版本）和
   `eyeData=IVRSystem_026`；若为 `eyeData=disabled`，说明运行时没有该接口，
   精确眼参数已回退到固定 FOV/IPD。
7. 退出后确认 serial 42 已移除；若程序崩溃，手动执行第 5 步的 unplug。
8. 若要无线：先按“无线串流”一节单独验证 ALVR 链路，再叠加 `vrcast`；记录
   ALVR 版本、Glass 固件、Wi-Fi 频段和延迟。

## 文件

```text
project/vrcast/
  CMakeLists.txt
  CMakePresets.json
  README.md
  src/
    common.h             Windows/D3D 公共头
    log.h/.cpp           日志与 Win32 错误格式化
    vdisplay.h/.cpp      设备接口枚举、IOCTL plug/unplug、会话清理
    probe_main.cpp       vrcast_probe
    capture_dda.h/.cpp   DXGI Desktop Duplication
    renderer.h/.cpp      D3D11 面板渲染、每眼 render target
    openvr_abi.h         最小 OpenVR ABI（无官方 SDK 时使用）
    openvr_bridge.h/.cpp openvr_api.dll 动态加载、IVRSystem 眼参数、WaitGetPoses/Submit
    abi_check.cpp        平台无关 ABI 布局自检（任何 C++20 编译器可跑）
    main.cpp             参数、主循环、精确投影、3DoF/6DoF 面板
```

## ABI 自检

`src/abi_check.cpp` 只包含 `openvr_abi.h`（不依赖 Windows 头），因此可以在
**没有 Windows SDK 的机器**（包括本分析工作区 / Linux CI）上编译运行，验证
shim 依赖的结构体尺寸与成员偏移：

```bash
g++ -std=c++20 -I src src/abi_check.cpp -o abi_check && ./abi_check
```

它把 `openvr_abi.h` 的 static_assert 落实为可执行检查，并把
`IVRSystem_026` / `IVRCompositor_029` 的 vtable 槽位表（含
`ComputeDistortionSet` 槽 4、`GetSubmitTexture` 槽 5）与
[ValveSoftware/openvr](https://github.com/ValveSoftware/openvr) 的
`headers/openvr.h`（master）逐项核对后写进注释。虚函数槽位无法编译期断言，
若 SteamVR 运行时接口再次演进，以官方 `openvr.h` 编译并同步更新此表。
CMake 已把 `abi_check` 注册为测试（`ctest`），与 `ioctl_decode` 一起运行。

## 参考

- `project/WINDOWS_CASTING.md`：Windows 投屏全链路（电路、驱动、物理层、应用层）。
- `project/DRIVERS.md`：驱动包清单与 IOCTL/EDID 摘要。
- `project/decomp/asm/virtual_display_device.asm`：总线驱动 IOCTL 分发。
- `project/decomp/asm/hwvr_DisplayManager.asm`：用户态 plug/unplug 客户端。
- [LHT02/HuaweiVRGlass-ALVR](https://github.com/LHT02/HuaweiVRGlass-ALVR)：外部
  ALVR 分支，提供 Huawei VR Glass 无线串流；不是本仓库的逆向产物。
- [fkun1754/HwVRGlassLite](https://github.com/fkun1754/HwVRGlassLite)：CV10 的
  Windows 管理客户端，不是无线串流实现；本次仅用其 `openvr.h` /
  `VR_InitInternal2` 调用序列核对 OpenVR ABI。
