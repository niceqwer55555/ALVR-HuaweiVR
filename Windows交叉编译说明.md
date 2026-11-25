# Windows x64 交叉编译说明

本仓库的 Windows 服务端与 PC 接收端已在 Linux 上通过 MinGW-w64 交叉编译，产物为 PE32+。

## 工具链

- Rust 1.82.0，target `x86_64-pc-windows-gnu`
- MinGW-w64 GCC 10 **posix 线程版**（必须用 posix；win32 版 libstdc++ 缺 `std::thread` / `std::condition_variable`）
- FFmpeg 8.1 win64 gpl-shared
- DirectXMath 头文件
- libclang（bindgen）

```bash
export CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc-posix
export CXX_x86_64_pc_windows_gnu=x86_64-w64-mingw32-g++-posix
export AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc-posix
export LIBCLANG_PATH=/lib/x86_64-linux-gnu
```

## 关键修复

1. `alvr/server_openvr/build.rs` 按**目标 OS**选择 FFmpeg 路径，不再按构建宿主 Linux 选择；所有 FFmpeg/x264/vulkan 分支改为运行时 `platform_name` 判断。
2. `/permissive-` 只在 MSVC 下添加；MinGW 不再报错。
3. 增加 `WINVER/_WIN32_WINNT/_USE_MATH_DEFINES`、DirectXMath 与 MinGW 大小写兼容头文件。
4. GNU 编译器补 `-pthread`、`-D_GLIBCXX_HAS_GTHREADS`，并强制包含 `alvr_prelude.h`，解决 `std::mutex/std::thread`。
5. `VideoEncoderAMF.cpp` 的 `L#expr` 改为 `#expr`。
6. `VideoEncoderSW.cpp` 的 `FF_PROFILE_H264_*` 改为 FFmpeg 8 的 `AV_PROFILE_H264_*`。

## 构建命令

```bash
cargo build --release -p alvr_dashboard --target x86_64-pc-windows-gnu
cargo build --release -p alvr_launcher --target x86_64-pc-windows-gnu
cargo build --release -p alvr_server_core --target x86_64-pc-windows-gnu
cargo build --release -p alvr_server_openvr --features gpl --target x86_64-pc-windows-gnu
```

`alvr_server_openvr` 的 `crate-type` 是 `cdylib`，产物是 `driver_alvr_server.dll`，由 SteamVR 的 `vrserver.exe` 加载；服务端可见 exe 是 `ALVR Dashboard.exe`。
