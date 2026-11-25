# 客户端②：依赖华为 VR 框架的安卓接收端

本目录提供可独立编译的 Rust/Manifest 源码与打包脚本，产出 armv7 + arm64 通用 APK。

## 构建

```bash
export ANDROID_NDK_HOME=/path/to/android-ndk-r25c
export ANDROID_BUILD_TOOLS=/path/to/build-tools/34.0.0
export ANDROID_PLATFORM=/path/to/platforms/android-34
./client_openxr/scripts/build_native.sh   # armv7 + arm64
./client_openxr/scripts/build_linux.sh    # 产出 build/HuaweiVRGlass-armv7.apk
```

原生库依赖 `sdk/hvrsdk-openxr-3.5.0.79` 中的 `libxr_loader.so`；Java host 为 `android_huawei/src`。

## 结论

本目录已可独立构建：`openxr 0.18` 兼容层提供华为 bindings，华为 loader 与 Java host 随仓库提供，`build_native.sh` 一次编译 ARMv7 + ARM64。

通用、去华为系统框架的可安装 APK 见 `../客户端3-安卓接收端(高通/去华为框架)`。
