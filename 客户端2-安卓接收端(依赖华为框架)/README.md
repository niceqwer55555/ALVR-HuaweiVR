# 客户端②：依赖华为 VR 框架的安卓接收端

本目录只保留 `client_openxr` 的 Rust/Manifest 源码片段，**当前不能独立编译 APK**。

## 缺失依赖

1. `../../deps/openxrs-huawei/openxr`、`../../deps/openxrs-huawei/sys`
   - 上游是华为 OpenXR 的 Rust fork，带 `huawei-loader-linked`、`huawei-openxr-1-0-3` 特性；
   - 原仓库已丢失，只剩 README，不能用 crates.io 官方 `openxr` 替代（特性为空，链接行为不等价）。
2. 华为闭源运行库：`libxr_loader.so`、`hvrbridge.jar`、`hvrprompt.aar`
   - 需从 `hvrsdk-openxr-3.5.0.79` 或授权 SDK 中提取。
3. Java host：manifest 中的 `HuaweiVrLauncherActivity`、`HuaweiVrActivity`、`HVRModeActivity` 源码不在本目录。
4. Gradle/`cargo-apk` 打包工程：本目录没有 `build.gradle`。

## 结论

本目录保留为参考源码；不要在当前环境直接 `cargo build`/打包。要恢复完整客户端，需要：

- 找回原始 `deps/openxrs-huawei` fork（或按华为 OpenXR 头文件重建 bindings）；
- 补齐华为 SDK 运行库与 Java host；
- 在带 Android NDK + Gradle 的 Windows/Linux 环境按官方流程构建。

通用、去华为框架的可安装 APK 见 `../客户端3-安卓接收端(高通/去华为框架)`。
