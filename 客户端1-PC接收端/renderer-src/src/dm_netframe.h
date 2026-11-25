// dm_netframe.h —— 从 hvr-alvr-win-sink 的 TCP 9911 接收 SBS BGRA 帧
#pragma once
#include <winsock2.h>
#include <windows.h>
#include <d3d11.h>
#include <cstdint>
#include <vector>

class DmNetFrame {
public:
    void Start(ID3D11Device* dev, ID3D11DeviceContext* ctx);
    void Stop();
    bool Acquire();
    ID3D11ShaderResourceView* Srv() const { return srv_; }
    ID3D11Texture2D* Tex() const { return tex_; }
    uint32_t Width() const { return width_; }
    uint32_t Height() const { return height_; }
    uint64_t Frames() const { return frames_; }

private:
    static DWORD WINAPI ThreadEntry(LPVOID p);
    void ThreadMain();

    ID3D11Device* dev_ = nullptr;
    ID3D11DeviceContext* ctx_ = nullptr;
    ID3D11Texture2D* tex_ = nullptr;
    ID3D11ShaderResourceView* srv_ = nullptr;
    uint32_t width_ = 0, height_ = 0;

    HANDLE thread_ = NULL;
    volatile LONG run_ = 0;
    SOCKET listen_ = INVALID_SOCKET;
    SOCKET client_ = INVALID_SOCKET;
    CRITICAL_SECTION cs_;
    bool csInit_ = false;
    std::vector<uint8_t> pending_;
    uint32_t pending_w_ = 0, pending_h_ = 0;
    bool dirty_ = false;
    uint64_t frames_ = 0;
};
