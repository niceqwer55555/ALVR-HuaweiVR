# 标准 OpenXR Runtime（替代华为 libxr_loader.so）

## 一、参考仓库的华为加载机制（要替换掉的东西）

参考仓库报告实测：`libxr_loader.so`（华为改过）里写死字符串：
```
"library_path": "libvrsdk.so"
```
loader 会在 App 自己的 `lib/armeabi-v7a/` 里找 `libvrsdk.so` 当 runtime 加载。所以原改造只要塞一个同名自研 so 就能被加载——这是"去华为系统"的技术支点，但**仍依赖华为定制 loader**。

## 二、本工程的通用方案

```
ALVR OpenXR 客户端
   │ xrGetInstanceProcAddr（标准入口）
   ▼
libopenxr_loader.so  ← Khronos 官方 loader（Google Play / 随 App 打包）
   │ 读取 active_runtime.json
   ▼
libgeneric_alvr_xr.so ← 本工程通用 runtime（标准 OpenXR 1.0 ABI）
   │ 平台桥 generic_runtime.c → pose/controller/decoder 核心
   ▼
Android Surface（SBS 双目合成）
```

### active_runtime.json
```json
{
  "file_format_version": "1.0.0",
  "runtime": {
    "library_path": "libgeneric_alvr_xr.so",
    "api_version": "1.0.3",
    "name": "generic_alvr_xr",
    "vendor": "Open Source Community",
    "version": "0.1.0"
  }
}
```
放在 App 的 `assets/`，启动时复制到可访问路径并设 `XR_RUNTIME_JSON`，或随 APK 放到 loader 默认搜索路径。

## 三、runtime 需实现的 OpenXR 入口点（标准 1.0 核心）

| 类别 | 入口点 |
|---|---|
| Instance | `xrCreateInstance`, `xrDestroyInstance`, `xrGetInstanceProperties`, `xrGetInstanceProcAddr`, `xrEnumerateApiLayerProperties`, `xrEnumerateInstanceExtensionProperties` |
| System | `xrGetSystem`, `xrGetSystemProperties`, `xrEnumerateViewConfigurations`, `xrGetViewConfigurationProperties`, `xrEnumerateViewConfigurationViews`, `xrEnumerateEnvironmentBlendModes` |
| Session | `xrCreateSession`, `xrDestroySession`, `xrBeginSession`, `xrEndSession`, `xrEnumerateSwapchainFormats` |
| Graphics | `xrEnumerateSwapchainImages`, `xrCreateSwapchain`, `xrDestroySwapchain`, `xrAcquireSwapchainImage`, `xrWaitSwapchainImage`, `xrReleaseSwapchainImage`（GLES 绑定 `XR_KHR_opengl_es_enable`） |
| Frame | `xrWaitFrame`, `xrBeginFrame`, `xrEndFrame` |
| Space | `xrCreateReferenceSpace`, `xrDestroySpace`, `xrLocateSpace`, `xrGetReferenceSpaceBoundsRect` |
| Path/Action | `xrStringToPath`, `xrPathToString`, `xrCreateActionSet`, `xrCreateAction`, `xrSuggestInteractionProfileBindings`, `xrSyncActions`, `xrGetActionStateBoolean/Vector2f/Pose`, `xrGetCurrentInteractionProfile` |

> 本工程 `src/runtime/generic_runtime.c` 提供**平台桥**（数据源→姿态/手柄状态），这些标准入口点的骨架实现需配合 Khronos OpenXR Headers 完成（build.sh 中 `runtime_openxr.c` 位置），体量与参考仓库 `mini_xr_v3.c` 相当（约 900 行）。平台桥已把姿态/手柄/视图配置准备好，入口点只需按标准 ABI 转发。

## 四、不实现的东西（去华为化）

- 不实现 `XR_HUAWEI_controller_interaction`
- 不依赖华为 EGL context（`HuaweiEglContext`）
- 不用 `OMX.hisi.*` 解码器
- 不读 `/dev/hwvr*`、`/dev/hidraw3` 等华为眼镜节点
- 不 `dlopen("libvrsdk.so")` / `libvrpose.so`

## 五、帧节拍与 SBS 呈现

- `xrWaitFrame` 按 `refresh_hz`（默认 72）驱动
- `xrEndFrame` 解析投影层左右眼 swapchain，用 `glBlitFramebuffer` 合成到 `sbs_width × sbs_height`（默认 3200×1600，各眼 1600×1600）→ `eglSwapBuffers`
- 视图配置由 `ga_runtime_view_config()` 提供，姿态由 `ga_runtime_get_head_pose()`（OpenXR 四元数顺序）提供

详细呈现逻辑可参考仓库 `mini_xr_v3.c` 第 9 章，但本工程用标准 loader + 标准扩展，无华为绑定。
