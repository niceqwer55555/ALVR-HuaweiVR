# 华为 VR SDK 编译输入

本目录只保留 GitHub Actions / Linux 打包所需的华为 SDK 文件，原始 SDK 压缩包未入库。

- `hvrsdk-openxr-3.5.0.79/`
  - `jni/armeabi-v7a/libxr_loader.so`、`jni/arm64-v8a/libxr_loader.so`
  - `include/openxr/openxr.h`、`openxr_hw.h`
  - `libs/hvrbridge.jar`（`com.huawei.hvr.LibUpdateClient`）
- `vrsdk-3.5.0.68/Plugins/Android/`
  - `hvrprompt.aar`（官方 2D 连接提示页，含 `com.huawei.vrlab.HVRModeActivity`）
  - `hvrversioncheck.aar`
  - `hvrbridge.jar`

这些文件来自用户提供的华为 VR SDK 包，仅用于构建本仓库的客户端②。重新分发前请确认华为 SDK 的授权条款；如不需要客户端②，可删除本目录并移除 `android-huawei` 工作流任务。
