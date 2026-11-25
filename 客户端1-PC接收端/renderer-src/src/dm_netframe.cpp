#include "dm_netframe.h"
#include "dm_log.h"
#include <cstring>

static bool RecvAll(SOCKET s, void* dst, size_t n) {
    uint8_t* p = (uint8_t*)dst;
    while (n) {
        int r = recv(s, (char*)p, (int)n, 0);
        if (r <= 0) return false;
        p += r; n -= (size_t)r;
    }
    return true;
}

static uint32_t Le32(const uint8_t* p) {
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) | ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}

void DmNetFrame::Start(ID3D11Device* dev, ID3D11DeviceContext* ctx) {
    dev_ = dev; ctx_ = ctx;
    if (!csInit_) { InitializeCriticalSection(&cs_); csInit_ = true; }
    run_ = 1;
    WSADATA wsa{};
    if (WSAStartup(MAKEWORD(2, 2), &wsa) != 0) {
        DmLog("网络帧源：WSAStartup 失败");
        run_ = 0; return;
    }
    listen_ = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    if (listen_ == INVALID_SOCKET) {
        DmLog("网络帧源：socket 失败 %d", WSAGetLastError());
        run_ = 0; return;
    }
    BOOL reuse = TRUE;
    setsockopt(listen_, SOL_SOCKET, SO_REUSEADDR, (const char*)&reuse, sizeof(reuse));
    sockaddr_in addr{};
    addr.sin_family = AF_INET;
    addr.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    addr.sin_port = htons(9911);
    if (bind(listen_, (sockaddr*)&addr, sizeof(addr)) != 0) {
        DmLog("网络帧源：bind 127.0.0.1:9911 失败 %d（端口被占用？）", WSAGetLastError());
        closesocket(listen_); listen_ = INVALID_SOCKET; run_ = 0; return;
    }
    if (listen(listen_, 1) != 0) {
        DmLog("网络帧源：listen 失败 %d", WSAGetLastError());
        closesocket(listen_); listen_ = INVALID_SOCKET; run_ = 0; return;
    }
    thread_ = CreateThread(nullptr, 0, &DmNetFrame::ThreadEntry, this, 0, nullptr);
    DmLog("网络帧源：监听 127.0.0.1:9911，等待 PC 接收端");
}

DWORD WINAPI DmNetFrame::ThreadEntry(LPVOID p) {
    ((DmNetFrame*)p)->ThreadMain();
    return 0;
}

void DmNetFrame::ThreadMain() {
    while (run_) {
        SOCKET c = accept(listen_, nullptr, nullptr);
        if (c == INVALID_SOCKET) {
            if (run_) DmLog("网络帧源：accept 失败 %d", WSAGetLastError());
            break;
        }
        DmLog("网络帧源：PC 接收端已连接");
        EnterCriticalSection(&cs_);
        client_ = c;
        LeaveCriticalSection(&cs_);
        while (run_) {
            uint8_t hdr[12];
            if (!RecvAll(c, hdr, sizeof(hdr))) break;
            uint32_t w = Le32(hdr + 0);
            uint32_t h = Le32(hdr + 4);
            uint32_t layout = Le32(hdr + 8);
            (void)layout;
            if (w == 0 || h == 0 || w > 8192 || h > 8192) {
                DmLog("网络帧源：非法帧尺寸 %ux%u", w, h);
                break;
            }
            size_t bytes = (size_t)w * h * 4;
            std::vector<uint8_t> px(bytes);
            if (!RecvAll(c, px.data(), bytes)) break;
            EnterCriticalSection(&cs_);
            pending_.swap(px);
            pending_w_ = w; pending_h_ = h;
            dirty_ = true;
            LeaveCriticalSection(&cs_);
            frames_++;
        }
        EnterCriticalSection(&cs_);
        SOCKET old = client_;
        client_ = INVALID_SOCKET;
        LeaveCriticalSection(&cs_);
        if (old != INVALID_SOCKET) closesocket(old);
        DmLog("网络帧源：PC 接收端断开");
    }
}

bool DmNetFrame::Acquire() {
    if (!dev_ || !ctx_ || !csInit_) return false;
    std::vector<uint8_t> px;
    uint32_t w = 0, h = 0;
    EnterCriticalSection(&cs_);
    if (!dirty_) { LeaveCriticalSection(&cs_); return false; }
    px.swap(pending_);
    w = pending_w_; h = pending_h_;
    dirty_ = false;
    LeaveCriticalSection(&cs_);

    if (px.empty() || w == 0 || h == 0) return false;
    if (!tex_ || w != width_ || h != height_) {
        if (srv_) { srv_->Release(); srv_ = nullptr; }
        if (tex_) { tex_->Release(); tex_ = nullptr; }
        D3D11_TEXTURE2D_DESC desc{};
        desc.Width = w; desc.Height = h;
        desc.MipLevels = 1; desc.ArraySize = 1;
        desc.Format = DXGI_FORMAT_B8G8R8A8_UNORM;
        desc.SampleDesc.Count = 1;
        desc.Usage = D3D11_USAGE_DEFAULT;
        desc.BindFlags = D3D11_BIND_SHADER_RESOURCE;
        if (FAILED(dev_->CreateTexture2D(&desc, nullptr, &tex_))) {
            DmLog("网络帧源：CreateTexture2D %ux%u 失败", w, h);
            return false;
        }
        D3D11_SHADER_RESOURCE_VIEW_DESC sd{};
        sd.Format = desc.Format;
        sd.ViewDimension = D3D11_SRV_DIMENSION_TEXTURE2D;
        sd.Texture2D.MipLevels = 1;
        if (FAILED(dev_->CreateShaderResourceView(tex_, &sd, &srv_))) {
            DmLog("网络帧源：CreateSRV 失败");
            return false;
        }
        width_ = w; height_ = h;
        DmLog("网络帧源：首帧 %ux%u 已就绪", w, h);
    }
    ctx_->UpdateSubresource(tex_, 0, nullptr, px.data(), w * 4, 0);
    return true;
}

void DmNetFrame::Stop() {
    InterlockedExchange(&run_, 0);
    if (listen_ != INVALID_SOCKET) {
        closesocket(listen_);
        listen_ = INVALID_SOCKET;
    }
    SOCKET cli = INVALID_SOCKET;
    if (csInit_) {
        EnterCriticalSection(&cs_);
        cli = client_;
        client_ = INVALID_SOCKET;
        LeaveCriticalSection(&cs_);
    }
    if (cli != INVALID_SOCKET) {
        shutdown(cli, SD_BOTH);
        closesocket(cli);
    }
    if (thread_) {
        WaitForSingleObject(thread_, 2000);
        CloseHandle(thread_);
        thread_ = NULL;
    }
    if (srv_) { srv_->Release(); srv_ = nullptr; }
    if (tex_) { tex_->Release(); tex_ = nullptr; }
    WSACleanup();
}
