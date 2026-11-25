// dm_capture.h —— DXGI Desktop Duplication 屏幕采集
#pragma once
#include <windows.h>
#include <cstdint>
#include <d3d11.h>
#include <dxgi1_2.h>
#include <string>
#include <vector>

struct DmMonitorInfo {
    std::wstring device;      // \\.\DISPLAY2
    std::wstring name;        // 友好名
    RECT rect;
    bool primary;
    uint32_t w, h;
};

std::vector<DmMonitorInfo> DmEnumMonitors();

struct DmCapture {
    ID3D11Device* dev = NULL;
    ID3D11DeviceContext* ctx = NULL;

    IDXGIOutputDuplication* dup = NULL;
    ID3D11Texture2D* tex = NULL;
    ID3D11ShaderResourceView* srv = NULL;
    uint32_t w = 0, h = 0;
    int index = -1;
    RECT rect = {0, 0, 0, 0};
    bool started = false;
    double fps = 0;
    uint64_t frames = 0;
    int lastErr = 0;
    bool texRotated = false;      // DDA 纹理相对逻辑显示器旋转了 90°（宽高互换）

    // 光标
    ID3D11ShaderResourceView* curSrv = NULL;
    uint32_t curW = 0, curH = 0, curHotX = 0, curHotY = 0;
    HCURSOR curHandle = NULL;
    bool curVisible = false;
    float curU = 0, curV = 0;      // 0..1 相对屏幕（纹理空间）
    float curAspect = 1.0f;        // 宽/高
    bool  curSwapped = false;      // 纹理相对逻辑显示器旋转了90°（宽高互换）

    bool Start(ID3D11Device* d, ID3D11DeviceContext* c, int monitorIndex, std::string* err);
    void Stop();
    bool Acquire();                 // 有新帧则更新 tex，返回是否更新
    void UpdateCursor(ID3D11Device* d);
    // 逻辑桌面的宽高比：用显示器矩形而不是纹理尺寸。旋转过的 DDA 纹理
    // 宽高互换，若用纹理算 aspect，桌面会被压成竖版并贴歪。
    float logicalAspect() const {
        LONG rw = rect.right - rect.left, rh = rect.bottom - rect.top;
        return (rw > 0 && rh > 0) ? (float)rw / (float)rh : 1.0f;
    }
};
