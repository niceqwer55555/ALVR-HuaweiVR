# HwVRDesktop 网络收帧补丁

本目录是 `HwVRDesktop.exe` 的可编译源码，已包含：

- `src/dm_netframe.{h,cpp}`：TCP `127.0.0.1:9911` 接收 `[u32 w][u32 h][u32 layout][BGRA w*h*4]`，上传 D3D11 纹理。
- `src/main.cpp`：视频源从桌面采集 `g_cap.srv` 切换为 `g_net.Srv()`；新增 UDP `127.0.0.1:9912` 回传 `xyzw` 四元数。
- `desktop_mode.ini`：`ipd=63`、`fov_tan=1.0913`、`[s3d] mode=1`。

编译（MinGW-w64，x64）：

```powershell
x86_64-w64-mingw32-g++ -O2 -s -static -municode -mwindows `
  -D_WIN32_WINNT=0x0A00 -DWINVER=0x0A00 `
  -o HwVRDesktop.exe src/*.cpp -Isrc `
  -ld3d11 -ldxgi -ld3dcompiler -lwindowscodecs -lole32 -luuid -lsetupapi -lhid `
  -lshell32 -ladvapi32 -lcomctl32 -lwbemuuid -loleaut32 -lpdh -lws2_32 `
  -static-libgcc -static-libstdc++
```

运行日志出现 `网络帧源：监听 127.0.0.1:9911` 和 `首帧 3072x1536 已就绪` 即收帧成功。
