// slam_bridge.cpp —— 桥接华为 VR6DOF.dll 的 SLAM 手柄姿态，UDP 发到本地客户端
// 原理（逆向确认）：
//   LoadLibrary(VR6DOF.dll) -> GetHwSlamClient() 返回接口对象(静态 data_1801319d0)
//   vtable[0]=Init, vtable[1]=DeInit, vtable[2]=GetTrackingInfo
//   Init 从注册表 HKCU\SOFTWARE\HuaweiPCVRAssistant\Path 找 exe 目录 + 标定 XML
//   GetTrackingInfo 把左右手柄姿态写入模块内全局 base+0xee6038，
//     每手 0x118 字节：+0x08 pos(xyz) f32, +0x14 quat(w,x,y,z) f32
//   发出 UDP 到 127.0.0.1:9913，payload = 2手 × (3f pos + 4f quat) = 56 字节
//   （寄存器 ABI：GetTrackingInfo(this, void* out) 用 __fastcall，this=RCX）
#include <windows.h>
#include <winsock2.h>
#include <cstdio>
#include <cstdint>
#include <cstring>
#include <thread>
#include <atomic>

#pragma comment(lib, "ws2_32.lib")

static std::atomic<bool> g_stop{false};

// 从注册表读华为助手 exe 目录
static bool get_assist_path(char* out, size_t n) {
    HKEY hk;
    if (RegOpenKeyExA(HKEY_CURRENT_USER, "SOFTWARE\\HuaweiPCVRAssistant", 0, KEY_READ, &hk) != ERROR_SUCCESS)
        return false;
    DWORD type = REG_SZ, sz = (DWORD)n;
    bool ok = RegQueryValueExA(hk, "Path", nullptr, &type, (BYTE*)out, &sz) == ERROR_SUCCESS;
    RegCloseKey(hk);
    return ok;
}

int main() {
    char assist[MAX_PATH];
    if (!get_assist_path(assist, sizeof(assist)) || !assist[0]) {
        // 注册表缺失时探测常见安装路径
        const char* cands[] = {
            "F:////PY////HwPCVR Assistant",
            "C:////Program Files////Huawei////HwPCVR Assistant",
        };
        bool found = false;
        for (auto c : cands) {
            char probe[MAX_PATH];
            snprintf(probe, sizeof(probe), "%s\\steamVRDriver\\huaweiVR\\bin\\win64\\VR6DOF.dll", c);
            FILE* fp = fopen(probe, "rb");
            if (fp) { fclose(fp); strncpy(assist, c, sizeof(assist)-1); assist[sizeof(assist)-1]=0; found=true; break; }
        }
        if (!found) { fprintf(stderr, "[slam_bridge] 找不到 VR6DOF.dll\n"); return 1; }
        fprintf(stderr, "[slam_bridge] 探测到助手目录: %s\n", assist);
    }
    char dllDir[MAX_PATH];
    snprintf(dllDir, sizeof(dllDir), "%s\\steamVRDriver\\huaweiVR\\bin\\win64", assist);
    char dllPath[MAX_PATH];
    snprintf(dllPath, sizeof(dllPath), "%s\\VR6DOF.dll", dllDir);

    SetCurrentDirectoryA(dllDir);
    SetDllDirectoryA(dllDir);
    fprintf(stderr, "[slam_bridge] exe路径=%s\n[slam_bridge] DLL=%s\n", assist, dllPath);

    HMODULE h = LoadLibraryA(dllPath);
    if (!h) {
        fprintf(stderr, "[slam_bridge] LoadLibrary 失败 GetLastError=%lu\n", GetLastError());
        return 2;
    }
    typedef void* (*PFN_GetClient)();
    PFN_GetClient getClient = (PFN_GetClient)GetProcAddress(h, "GetHwSlamClient");
    if (!getClient) {
        fprintf(stderr, "[slam_bridge] GetHwSlamClient 导出缺失\n");
        return 3;
    }
    void* obj = getClient();
    if (!obj) { fprintf(stderr, "[slam_bridge] GetHwSlamClient 返回 NULL\n"); return 4; }
    void** vt = *(void***)obj;

    // vtable slot: 0=Init, 1=DeInit, 2=GetTrackingInfo
    typedef int64_t (__fastcall *PFN_Init)(void* self);
    typedef int64_t (__fastcall *PFN_GTI)(void* self, void* out);
    PFN_Init init = (PFN_Init)vt[0];
    PFN_GTI  gti  = (PFN_GTI)vt[2];

    int64_t ir = init(obj);
    fprintf(stderr, "[slam_bridge] Init ret=%lld\n", (long long)ir);
    if (ir != 0) {
        fprintf(stderr, "[slam_bridge] SLAM Init 失败（检查注册表路径/标定XML/摄像头）\n");
        return 5;
    }

    // 模块基址 = obj - 0x1319d0；姿态区 = base + 0xee6038
    uintptr_t objAddr = (uintptr_t)obj;
    uintptr_t base = objAddr - 0x1319d0;
    uint8_t* poseGlobal = (uint8_t*)(base + 0xee6038);
    fprintf(stderr, "[slam_bridge] base=0x%llx poseGlobal=0x%llx\n",
            (unsigned long long)base, (unsigned long long)poseGlobal);

    // UDP sockets: 9913=手柄姿态, 9912=头显姿态(供sink回传server驱动头姿)
    WSADATA wsa; WSAStartup(MAKEWORD(2,2), &wsa);
    SOCKET sock = socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP);
    sockaddr_in addr; memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET; addr.sin_port = htons(9913);
    addr.sin_addr.s_addr = inet_addr("127.0.0.1");
    SOCKET hsock = socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP);
    sockaddr_in haddr; memset(&haddr, 0, sizeof(haddr));
    haddr.sin_family = AF_INET; haddr.sin_port = htons(9912);
    haddr.sin_addr.s_addr = inet_addr("127.0.0.1");

    const uint8_t* headPose = (const uint8_t*)(base + 0xee6018); // SLAM 头姿 quat(xyzw)+pos

    uint8_t outBuf[0x118 * 2];
    float pkt[14]; // 2手 × (3 pos + 4 quat)
    int sent = 0;
    fprintf(stderr, "[slam_bridge] 开始推送 UDP 9913(手柄 56B) + 9912(头姿 28B)\n");
    while (!g_stop) {
        gti(obj, outBuf);
        for (int hand = 0; hand < 2; ++hand) {
            const uint8_t* seg = poseGlobal + hand * 0x118;
            float px, py, pz, qw, qx, qy, qz;
            memcpy(&px, seg + 0x08, 4); memcpy(&py, seg + 0x0c, 4); memcpy(&pz, seg + 0x10, 4);
            memcpy(&qw, seg + 0x14, 4); memcpy(&qx, seg + 0x18, 4);
            memcpy(&qy, seg + 0x1c, 4); memcpy(&qz, seg + 0x20, 4);
            pkt[hand*7+0]=px; pkt[hand*7+1]=py; pkt[hand*7+2]=pz;
            pkt[hand*7+3]=qw; pkt[hand*7+4]=qx; pkt[hand*7+5]=qy; pkt[hand*7+6]=qz;
        }
        sendto(sock, (const char*)pkt, sizeof(pkt), 0, (sockaddr*)&addr, sizeof(addr));

        // [HVR fix] 头显姿态同样来自 VR6DOF 的 SLAM，发到 9912 供 sink 回传服务端，
        // 解决"头部移动画面不重新渲染"——旧的 9912 数据来自渲染器 IMU(Hmd.dll)，
        // 在 IMU 无效时恒为单位四元数，服务端拿不到头姿就不会重新渲染。
        float headPkt[7];
        memcpy(headPkt, headPose, sizeof(headPkt)); // [x,y,z,w, px,py,pz]
        sendto(hsock, (const char*)headPkt, sizeof(headPkt), 0, (sockaddr*)&haddr, sizeof(haddr));

        if (++sent % 300 == 0) {
            fprintf(stderr, "[slam_bridge] 已推送 %d 包  H0 p=(%.3f,%.3f,%.3f) H1 p=(%.3f,%.3f,%.3f)  HEAD q=(%.3f,%.3f,%.3f,%.3f) p=(%.3f,%.3f,%.3f)\n",
                    sent, pkt[0],pkt[1],pkt[2], pkt[7],pkt[8],pkt[9],
                    headPkt[0],headPkt[1],headPkt[2],headPkt[3], headPkt[4],headPkt[5],headPkt[6]);
            fflush(stderr);
        }
        Sleep(16); // ~60fps
    }
    closesocket(sock); closesocket(hsock); WSACleanup();
    // DeInit 通常不调（可能挂起），直接退出进程让OS回收
    return 0;
}
