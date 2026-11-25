/*
 * HVR VR Glass 串流接收端 —— 图形界面入口（双击即用，无需命令行）
 *
 * 设计：本程序是纯 Win32 GUI 前端。用户在窗口里点按钮，后台以「无控制台窗口」
 *       方式启动核心引擎 hvr-alvr-win-sink.exe，并把它的标准输出/错误通过
 *       管道捕获后实时显示在主窗口的日志区。另一路定时读取引擎写出的
 *       device_status.json，把「头显 / 左手柄 / 右手柄 / 手机」的插拔状态与
 *       电量显示在状态区，并用三个自绘指示灯「点亮」（在线绿色，离线灰色）。
 *
 * 编译（ mingw-w64 ）：
 *   x86_64-w64-mingw32-gcc -mwindows -O2 -finput-charset=UTF-8 \
 *     -fexec-charset=UTF-8 gui/sink_gui.c -o "HVR-VRGlass-接收端.exe" -lcomctl32
 */
#include <windows.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <wchar.h>

/* ---------- 控件 ID ---------- */
#define ID_STATEDIT   2001
#define ID_LOGEDIT    2002
#define ID_BTN_START  2003
#define ID_BTN_STOP   2004
#define ID_BTN_STATUS 2005
#define ID_BTN_SELF   2006
#define ID_BTN_CLEAR  2007
#define ID_TIMER      2008
#define ID_LEDPANEL   2009
#define ID_RADIO_VR    2010
#define ID_RADIO_LOCAL 2011
#define WM_APPENDLOG  (WM_APP + 1)

/* ---------- 指示灯数据 ---------- */
typedef struct {
    int  online;      /* 1=点亮 0=灰 */
    int  battery;     /* -1=未知 */
    int  dof;         /* 3/6/0 */
    wchar_t label[24];
    wchar_t detail[64];
} LedItem;

#define LED_COUNT 4
static LedItem g_leds[LED_COUNT];   /* 0=头显 1=左手柄 2=右手柄 3=ALVR 串流 */

/* ---------- 全局 ---------- */
static HWND  g_hwnd = NULL;
static HWND  g_stateEdit, g_logEdit, g_ledPanel;
static HWND  g_btnStart, g_btnStop, g_btnStatus, g_btnSelf, g_btnClear;
static HFONT g_font = NULL;
static PROCESS_INFORMATION g_pi = {0};
static PROCESS_INFORMATION g_renderPi = {0};
static int   g_running = 0;
static wchar_t g_exeDir[MAX_PATH];
static wchar_t g_corePath[MAX_PATH];
static wchar_t g_renderPath[MAX_PATH];
static wchar_t g_statusPath[MAX_PATH];
static wchar_t g_iniPath[MAX_PATH];
static int   g_mode = 0;   /* 0=SteamVR串流(默认) 1=本地桌面投屏(有线) */

/* UTF-8 字节 -> 新分配的宽字符串（调用方 free） */
static wchar_t *utf8_to_wide(const char *s) {
    int n = MultiByteToWideChar(CP_UTF8, 0, s, -1, NULL, 0);
    if (n <= 0) return NULL;
    wchar_t *w = (wchar_t *)malloc(sizeof(wchar_t) * (size_t)n);
    if (!w) return NULL;
    MultiByteToWideChar(CP_UTF8, 0, s, -1, w, n);
    return w;
}

/* 追加一行到日志编辑框 */
static void append_log(LPCWSTR text) {
    if (!g_logEdit || !text) return;
    int len = GetWindowTextLengthW(g_logEdit);
    SendMessageW(g_logEdit, EM_SETSEL, (WPARAM)len, (LPARAM)len);
    SendMessageW(g_logEdit, EM_REPLACESEL, FALSE, (LPARAM)text);
    SendMessageW(g_logEdit, EM_SCROLLCARET, 0, 0);
}

static void log_ascii(const char *s) {
    wchar_t *w = utf8_to_wide(s);
    if (w) { append_log(w); free(w); }
}

/* ---------- 轻量 JSON 取值（device_status.json 为 pretty 格式） ---------- */
static const char *json_value(const char *json, const char *anchor, const char *key) {
    const char *a = anchor ? strstr(json, anchor) : json;
    if (!a) return NULL;
    const char *k = strstr(a, key);
    if (!k) return NULL;
    const char *c = strchr(k, ':');
    if (!c) return NULL;
    const char *p = c + 1;
    while (*p == ' ' || *p == '\n' || *p == '\r' || *p == '\t') ++p;
    return p;
}
static int json_bool(const char *json, const char *anchor, const char *key) {
    const char *p = json_value(json, anchor, key);
    return p && strncmp(p, "true", 4) == 0;
}
static int json_int(const char *json, const char *anchor, const char *key) {
    const char *p = json_value(json, anchor, key);
    if (!p || *p == 'n') return -1;   /* null */
    return atoi(p);
}

/* ---------- 读取 device_status.json 刷新状态区 + 指示灯 ---------- */
static void refresh_status(void) {
    FILE *f = _wfopen(g_statusPath, L"rb");
    if (!f) {
        SetWindowTextW(g_stateEdit,
            L"状态：尚未启动串流。\r\n点击【启动串流】后，这里会显示头显/手机插拔状态与电量，"
            L"上方三个指示灯会实时点亮。");
        memset(g_leds, 0, sizeof(g_leds));
        wcscpy(g_leds[0].label, L"头显 VR Glass");
        wcscpy(g_leds[1].label, L"左手柄");
        wcscpy(g_leds[2].label, L"右手柄");
        wcscpy(g_leds[3].label, L"ALVR 串流");
        wcscpy(g_leds[3].detail, L"未连接");
        if (g_ledPanel) InvalidateRect(g_ledPanel, NULL, TRUE);
        return;
    }
    char buf[8192];
    size_t n = fread(buf, 1, sizeof(buf) - 1, f);
    buf[n] = '\0';
    fclose(f);

    /* 详情文本原样显示 */
    wchar_t *w = utf8_to_wide(buf);
    if (w) { SetWindowTextW(g_stateEdit, w); free(w); }

    /* 头显 */
    memset(&g_leds[0], 0, sizeof(LedItem));
    wcscpy(g_leds[0].label, L"头显 VR Glass");
    g_leds[0].online = json_bool(buf, "\"headset\"", "\"plugged\"");
    wcscpy(g_leds[0].detail, g_leds[0].online ? L"已连接 USB-C" : L"未插上");

    /* 左手柄 */
    memset(&g_leds[1], 0, sizeof(LedItem));
    wcscpy(g_leds[1].label, L"左手柄");
    g_leds[1].online  = json_bool(buf, "\"left_controller\"", "\"connected\"");
    g_leds[1].battery = json_int(buf, "\"left_controller\"", "\"battery_pct\"");
    g_leds[1].dof     = json_int(buf, "\"left_controller\"", "\"dof\"");
    if (g_leds[1].online) {
        int d = g_leds[1].dof > 0 ? g_leds[1].dof : 3;
        if (g_leds[1].battery >= 0)
            swprintf(g_leds[1].detail, 64, L"已连接 · %dDoF · 电量 %d%%", d, g_leds[1].battery);
        else
            swprintf(g_leds[1].detail, 64, L"已连接 · %dDoF · 电量未知", d);
    } else
        wcscpy(g_leds[1].detail, L"未连接");

    /* 右手柄 */
    memset(&g_leds[2], 0, sizeof(LedItem));
    wcscpy(g_leds[2].label, L"右手柄");
    g_leds[2].online  = json_bool(buf, "\"right_controller\"", "\"connected\"");
    g_leds[2].battery = json_int(buf, "\"right_controller\"", "\"battery_pct\"");
    g_leds[2].dof     = json_int(buf, "\"right_controller\"", "\"dof\"");
    if (g_leds[2].online) {
        int d = g_leds[2].dof > 0 ? g_leds[2].dof : 3;
        if (g_leds[2].battery >= 0)
            swprintf(g_leds[2].detail, 64, L"已连接 · %dDoF · 电量 %d%%", d, g_leds[2].battery);
        else
            swprintf(g_leds[2].detail, 64, L"已连接 · %dDoF · 电量未知", d);
    } else
        wcscpy(g_leds[2].detail, L"未连接");

    /* ALVR 串流连接状态（第 4 个灯） */
    memset(&g_leds[3], 0, sizeof(LedItem));
    wcscpy(g_leds[3].label, L"ALVR 串流");
    g_leds[3].online = json_bool(buf, NULL, "\"alvr_connected\"");
    wcscpy(g_leds[3].detail, g_leds[3].online ? L"已连接 PC 串流端" : L"等待 PC 串流端");

    if (g_ledPanel) InvalidateRect(g_ledPanel, NULL, TRUE);
}

/* ---------- 指示灯面板自绘 ---------- */
static void draw_led(HDC dc, int x, int y, const LedItem *it) {
    /* 卡片底 */
    RECT card = { x, y, x + 232, y + 92 };
    HBRUSH bg = CreateSolidBrush(RGB(245, 245, 248));
    FillRect(dc, &card, bg);
    DeleteObject(bg);
    Rectangle(dc, card.left, card.top, card.right, card.bottom);

    /* 圆灯：在线=绿色带光晕「点亮」，离线=灰色 */
    int cx = x + 34, cy = y + 34, r = 15;
    HBRUSH halo = CreateSolidBrush(it->online ? RGB(150, 235, 170) : RGB(190, 190, 195));
    HBRUSH core = CreateSolidBrush(it->online ? RGB(30, 190, 70) : RGB(110, 110, 118));
    HBRUSH old1 = (HBRUSH)SelectObject(dc, halo);
    Ellipse(dc, cx - r - 4, cy - r - 4, cx + r + 4, cy + r + 4);
    SelectObject(dc, core);
    Ellipse(dc, cx - r, cy - r, cx + r, cy + r);
    SelectObject(dc, old1);
    DeleteObject(halo); DeleteObject(core);
    if (it->online) {
        /* 高光点增强点亮感 */
        HBRUSH hi = CreateSolidBrush(RGB(220, 255, 228));
        HBRUSH old2 = (HBRUSH)SelectObject(dc, hi);
        Ellipse(dc, cx - 6, cy - 7, cx - 1, cy - 2);
        SelectObject(dc, old2);
        DeleteObject(hi);
    }

    SetTextColor(dc, it->online ? RGB(20, 120, 45) : RGB(90, 90, 95));
    TextOutW(dc, x + 62, y + 16, it->label, (int)wcslen(it->label));
    SetTextColor(dc, RGB(40, 40, 45));
    TextOutW(dc, x + 62, y + 44, it->detail, (int)wcslen(it->detail));
}

static LRESULT CALLBACK LedPanelProc(HWND h, UINT msg, WPARAM wp, LPARAM lp) {
    if (msg == WM_PAINT) {
        PAINTSTRUCT ps;
        HDC dc = BeginPaint(h, &ps);
        RECT rc; GetClientRect(h, &rc);
        HBRUSH back = CreateSolidBrush(RGB(235, 237, 242));
        FillRect(dc, &rc, back);
        DeleteObject(back);
        if (g_font) SelectObject(dc, g_font);
        SetBkMode(dc, TRANSPARENT);
        for (int i = 0; i < LED_COUNT; ++i) draw_led(dc, 8 + i * 240, 6, &g_leds[i]);
        EndPaint(h, &ps);
        return 0;
    }
    return DefWindowProcW(h, msg, wp, lp);
}

/* ---------- 管道读取线程：把子进程输出转发到主窗口 ---------- */
typedef struct { HWND hwnd; HANDLE hPipe; } PipeCtx;

static DWORD WINAPI pipe_thread(LPVOID param) {
    PipeCtx *ctx = (PipeCtx *)param;
    char buf[2048];
    DWORD n;
    while (ReadFile(ctx->hPipe, buf, sizeof(buf) - 1, &n, NULL) && n > 0) {
        buf[n] = '\0';
        wchar_t *wch = utf8_to_wide(buf);
        if (wch) {
            PostMessage(ctx->hwnd, WM_APPENDLOG, 0, (LPARAM)wch);
            /* wch 在主窗口 WM_APPENDLOG 处理里 free */
        }
    }
    CloseHandle(ctx->hPipe);
    free(ctx);
    return 0;
}

/* ---------- 启动一个核心进程（可选参数），输出捕获到日志区 ---------- */
static int run_core(LPCWSTR args, int track_handle) {
    SECURITY_ATTRIBUTES sa;
    memset(&sa, 0, sizeof(sa));
    sa.nLength = sizeof(sa);
    sa.bInheritHandle = TRUE;

    HANDLE rd = NULL, wr = NULL;
    if (!CreatePipe(&rd, &wr, &sa, 0)) {
        log_ascii("[错误] 创建输出管道失败\r\n");
        return 0;
    }
    SetHandleInformation(rd, HANDLE_FLAG_INHERIT, 0);

    HANDLE hnul = CreateFileW(L"NUL", GENERIC_READ, FILE_SHARE_READ,
                              NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);

    STARTUPINFOW si;
    memset(&si, 0, sizeof(si));
    si.cb = sizeof(si);
    si.dwFlags = STARTF_USESTDHANDLES;
    si.hStdOutput = wr;
    si.hStdError  = wr;
    si.hStdInput  = hnul;

    PROCESS_INFORMATION pi;
    memset(&pi, 0, sizeof(pi));

    wchar_t cmdline[1024];
    swprintf(cmdline, 1024, L"\"%ls\" %ls", g_corePath, args ? args : L"");

    BOOL ok = CreateProcessW(g_corePath, cmdline, NULL, NULL, TRUE,
                             CREATE_NO_WINDOW, NULL, g_exeDir, &si, &pi);
    if (hnul != INVALID_HANDLE_VALUE) CloseHandle(hnul);
    CloseHandle(wr);
    if (!ok) {
        CloseHandle(rd);
        char msg[512];
        snprintf(msg, sizeof(msg), "[错误] 启动核心引擎失败，GetLastError=%lu\r\n",
                 (unsigned long)GetLastError());
        log_ascii(msg);
        return 0;
    }

    PipeCtx *ctx = (PipeCtx *)malloc(sizeof(PipeCtx));
    ctx->hwnd = g_hwnd;
    ctx->hPipe = rd;
    HANDLE th = CreateThread(NULL, 0, pipe_thread, ctx, 0, NULL);
    if (th) CloseHandle(th);

    if (track_handle) {
        g_pi = pi;
        g_running = 1;
    } else {
        CloseHandle(pi.hThread);
        CloseHandle(pi.hProcess);
    }
    return 1;
}

/* ---------- 按当前模式写 desktop_mode.ini（帧源 + SteamVR 抢占开关） ---------- */
static void apply_mode_config(void) {
    /* frame_source: 0=网络串流(SteamVR) 1=本地桌面投屏(有线)
       kill_steamvr: 串流=0(保留SteamVR产帧) 有线投屏=1(独占眼镜面板) */
    WritePrivateProfileStringW(L"general", L"frame_source",
        g_mode == 1 ? L"1" : L"0", g_iniPath);
    WritePrivateProfileStringW(L"general", L"kill_steamvr",
        g_mode == 1 ? L"1" : L"0", g_iniPath);
    append_log(g_mode == 1
        ? L"[系统] 模式=本地桌面投屏（有线）：frame_source=1, kill_steamvr=1\r\n"
        : L"[系统] 模式=SteamVR 串流（无线）：frame_source=0, kill_steamvr=0\r\n");
}

/* ---------- 确保 SteamVR 不抢面板（有线投屏模式） ---------- */
static void stop_steamvr_if_local(void) {
    if (g_mode != 1) return;
    /* 用隐藏窗口方式结束 SteamVR，避免 GUI 程序闪控制台 */
    ShellExecuteW(NULL, L"open", L"taskkill.exe", L"/F /IM vrserver.exe", NULL, SW_HIDE);
    ShellExecuteW(NULL, L"open", L"taskkill.exe", L"/F /IM vrmonitor.exe", NULL, SW_HIDE);
    append_log(L"[系统] 已结束 SteamVR（本地桌面投屏独占眼镜面板）\r\n");
}

/* ---------- 启动眼镜渲染器（D3D11 SBS，监听 9911 收解码帧） ---------- */
static void start_renderer(void) {
    if (g_renderPi.hProcess) return; /* 已启动 */
    if (GetFileAttributesW(g_renderPath) == INVALID_FILE_ATTRIBUTES) {
        append_log(L"[警告] 未找到 renderer\\HwVRDesktop.exe，眼镜将无画面\r\n");
        return;
    }
    wchar_t cmd[1024];
    swprintf(cmd, 1024, L"\"%ls\"", g_renderPath);
    wchar_t rdir[MAX_PATH];
    swprintf(rdir, MAX_PATH, L"%lsrenderer", g_exeDir);
    STARTUPINFOW si; memset(&si, 0, sizeof(si)); si.cb = sizeof(si);
    PROCESS_INFORMATION pi; memset(&pi, 0, sizeof(pi));
    if (CreateProcessW(g_renderPath, cmd, NULL, NULL, FALSE,
                       CREATE_NO_WINDOW, NULL, rdir, &si, &pi)) {
        g_renderPi = pi;
        append_log(L"[系统] 眼镜渲染器已启动（3200x1600 SBS，监听 9911）\r\n");
    } else {
        append_log(L"[错误] 眼镜渲染器启动失败\r\n");
    }
}

/* ---------- 停止核心引擎 ---------- */
static void stop_core(void) {
    if (g_renderPi.hProcess) {
        TerminateProcess(g_renderPi.hProcess, 0);
        CloseHandle(g_renderPi.hThread);
        CloseHandle(g_renderPi.hProcess);
        memset(&g_renderPi, 0, sizeof(g_renderPi));
        append_log(L"[系统] 已停止眼镜渲染器\r\n");
    }
    if (g_running && g_pi.hProcess) {
        TerminateProcess(g_pi.hProcess, 0);
        CloseHandle(g_pi.hThread);
        CloseHandle(g_pi.hProcess);
        memset(&g_pi, 0, sizeof(g_pi));
        g_running = 0;
        append_log(L"[系统] 已停止串流引擎\r\n");
    }
}

/* ---------- 创建字体（中文显示） ---------- */
static HFONT make_font(int height) {
    return CreateFontW(height, 0, 0, 0, FW_NORMAL, 0, 0, 0,
                       DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
                       DEFAULT_QUALITY, DEFAULT_PITCH | FF_SWISS,
                       L"Microsoft YaHei");
}

/* ---------- 窗口过程 ---------- */
static LRESULT CALLBACK WndProc(HWND hwnd, UINT msg, WPARAM wParam, LPARAM lParam) {
    switch (msg) {
    case WM_CREATE: {
        g_hwnd = hwnd;
        g_font = make_font(16);

        /* 设备指示灯面板标题 */
        CreateWindowExW(0, L"Static", L"设备指示灯（在线点亮 / 离线灰暗，每 1 秒刷新）",
                        WS_CHILD | WS_VISIBLE | SS_LEFT, 12, 10, 500, 22,
                        hwnd, NULL, NULL, NULL);
        g_ledPanel = CreateWindowExW(WS_EX_CLIENTEDGE, L"HvrLedPanelClass", L"",
                        WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN, 12, 34, 960, 104,
                        hwnd, (HMENU)ID_LEDPANEL, NULL, NULL);

        /* 状态区标题 + 文本 */
        CreateWindowExW(0, L"Static", L"设备详细状态",
                        WS_CHILD | WS_VISIBLE | SS_LEFT, 12, 146, 400, 22,
                        hwnd, NULL, NULL, NULL);
        g_stateEdit = CreateWindowExW(WS_EX_CLIENTEDGE, L"Edit", L"",
                        WS_CHILD | WS_VISIBLE | ES_MULTILINE | ES_READONLY |
                        ES_AUTOVSCROLL | WS_VSCROLL, 12, 170, 960, 132,
                        hwnd, (HMENU)ID_STATEDIT, NULL, NULL);

        /* 日志区标题 + 文本 */
        CreateWindowExW(0, L"Static", L"运行日志",
                        WS_CHILD | WS_VISIBLE | SS_LEFT, 12, 310, 400, 22,
                        hwnd, NULL, NULL, NULL);
        g_logEdit = CreateWindowExW(WS_EX_CLIENTEDGE, L"Edit", L"",
                        WS_CHILD | WS_VISIBLE | ES_MULTILINE | ES_READONLY |
                        ES_AUTOVSCROLL | WS_VSCROLL | ES_WANTRETURN, 12, 334, 960, 210,
                        hwnd, (HMENU)ID_LOGEDIT, NULL, NULL);

        /* 按钮行 */
        g_btnStart  = CreateWindowExW(0, L"Button", L"启动串流",
                        WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON, 12, 554, 130, 34,
                        hwnd, (HMENU)ID_BTN_START, NULL, NULL);
        g_btnStop   = CreateWindowExW(0, L"Button", L"停止串流",
                        WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON, 150, 554, 130, 34,
                        hwnd, (HMENU)ID_BTN_STOP, NULL, NULL);
        g_btnStatus = CreateWindowExW(0, L"Button", L"刷新状态",
                        WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON, 288, 554, 130, 34,
                        hwnd, (HMENU)ID_BTN_STATUS, NULL, NULL);
        g_btnSelf   = CreateWindowExW(0, L"Button", L"协议自检",
                        WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON, 426, 554, 130, 34,
                        hwnd, (HMENU)ID_BTN_SELF, NULL, NULL);
        g_btnClear  = CreateWindowExW(0, L"Button", L"清空日志",
                        WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON, 610, 554, 130, 34,
                        hwnd, (HMENU)ID_BTN_CLEAR, NULL, NULL);

        /* 模式选择：默认 SteamVR 串流 */
        CreateWindowExW(0, L"Static", L"显示模式：",
                        WS_CHILD | WS_VISIBLE | SS_LEFT, 12, 600, 90, 24,
                        hwnd, NULL, NULL, NULL);
        HWND radioVR = CreateWindowExW(0, L"Button", L"SteamVR 串流（无线，默认）",
                        WS_CHILD | WS_VISIBLE | BS_AUTORADIOBUTTON | WS_GROUP | WS_TABSTOP,
                        100, 598, 240, 26, hwnd, (HMENU)ID_RADIO_VR, NULL, NULL);
        HWND radioLocal = CreateWindowExW(0, L"Button", L"本地桌面投屏（有线）",
                        WS_CHILD | WS_VISIBLE | BS_AUTORADIOBUTTON | WS_TABSTOP,
                        348, 598, 220, 26, hwnd, (HMENU)ID_RADIO_LOCAL, NULL, NULL);
        SendMessageW(radioVR, BM_SETCHECK, BST_CHECKED, 0);
        SendMessageW(radioLocal, BM_SETCHECK, BST_UNCHECKED, 0);
        SendMessageW(radioVR, WM_SETFONT, (WPARAM)g_font, TRUE);
        SendMessageW(radioLocal, WM_SETFONT, (WPARAM)g_font, TRUE);

        HWND ctrls[] = {g_stateEdit, g_logEdit, g_btnStart, g_btnStop,
                        g_btnStatus, g_btnSelf, g_btnClear};
        for (int i = 0; i < 7; ++i)
            SendMessageW(ctrls[i], WM_SETFONT, (WPARAM)g_font, TRUE);

        GetModuleFileNameW(NULL, g_exeDir, MAX_PATH);
        wchar_t *slash = wcsrchr(g_exeDir, L'\\');
        if (slash) *(slash + 1) = L'\0';
        swprintf(g_corePath, MAX_PATH, L"%lshvr-alvr-win-sink.exe", g_exeDir);
        swprintf(g_renderPath, MAX_PATH, L"%lsrenderer\\HwVRDesktop.exe", g_exeDir);
        swprintf(g_statusPath, MAX_PATH, L"%lsdevice_status.json", g_exeDir);
        swprintf(g_iniPath, MAX_PATH, L"%lsrenderer\\desktop_mode.ini", g_exeDir);

        append_log(L"[系统] HVR VR Glass 串流接收端已就绪\r\n");
        append_log(L"[系统] 上方三个指示灯实时显示头显 / 左手柄 / 右手柄状态\r\n");
        append_log(L"[提示] 请确保眼镜已插入本机 USB-C，且与高配电脑在同一局域网\r\n");

        SetTimer(hwnd, ID_TIMER, 1000, NULL);
        refresh_status();
        return 0;
    }

    case WM_APPENDLOG:
        append_log((LPCWSTR)lParam);
        free((void *)lParam);
        return 0;

    case WM_TIMER:
        if (wParam == ID_TIMER) refresh_status();
        return 0;

    case WM_COMMAND:
        switch (LOWORD(wParam)) {
        case ID_RADIO_VR:
            g_mode = 0;
            append_log(L"[系统] 已选择：SteamVR 串流（无线）\r\n");
            return 0;
        case ID_RADIO_LOCAL:
            g_mode = 1;
            append_log(L"[系统] 已选择：本地桌面投屏（有线）\r\n");
            return 0;
        case ID_BTN_START:
            if (g_running) {
                append_log(L"[提示] 串流引擎已在运行\r\n");
            } else {
                apply_mode_config();
                if (g_mode == 1) {
                    /* 有线投屏：杀 SteamVR 独占面板，只起渲染器（它抓本地桌面），不启核心 */
                    stop_steamvr_if_local();
                    append_log(L"[系统] 正在启动本地桌面投屏（无需核心引擎）...\r\n");
                    start_renderer();
                } else {
                    /* SteamVR 串流：先起渲染器，再起连接核心 */
                    append_log(L"[系统] 正在启动 SteamVR 串流引擎...\r\n");
                    start_renderer();
                    run_core(L"--config sink_config.json", 1);
                }
            }
            break;
        case ID_BTN_STOP:
            stop_core();
            break;
        case ID_BTN_STATUS:
            refresh_status();
            append_log(L"[系统] 状态已刷新（读取 device_status.json）\r\n");
            break;
        case ID_BTN_SELF:
            append_log(L"[系统] 运行协议自检...\r\n");
            run_core(L"--selftest", 0);
            break;
        case ID_BTN_CLEAR:
            SetWindowTextW(g_logEdit, L"");
            break;
        }
        return 0;

    case WM_DESTROY:
        stop_core();
        if (g_font) DeleteObject(g_font);
        KillTimer(hwnd, ID_TIMER);
        PostQuitMessage(0);
        return 0;
    }
    return DefWindowProcW(hwnd, msg, wParam, lParam);
}

/* ---------- 入口 ---------- */
int WINAPI WinMain(HINSTANCE hInst, HINSTANCE hPrev, LPSTR lpCmdLine, int nShow) {
    (void)hPrev; (void)lpCmdLine;

    WNDCLASSW wc;
    memset(&wc, 0, sizeof(wc));
    wc.lpfnWndProc = WndProc;
    wc.hInstance = hInst;
    wc.lpszClassName = L"HvrSinkGuiClass";
    wc.hbrBackground = (HBRUSH)(COLOR_WINDOW + 1);
    wc.hCursor = LoadCursorW(NULL, MAKEINTRESOURCEW(IDC_ARROW));
    if (!RegisterClassW(&wc)) {
        MessageBoxW(NULL, L"窗口类注册失败", L"错误", MB_ICONERROR);
        return 1;
    }

    /* 指示灯面板自定义类 */
    WNDCLASSW lc;
    memset(&lc, 0, sizeof(lc));
    lc.lpfnWndProc = LedPanelProc;
    lc.hInstance = hInst;
    lc.lpszClassName = L"HvrLedPanelClass";
    lc.hbrBackground = (HBRUSH)(COLOR_BTNFACE + 1);
    lc.hCursor = LoadCursorW(NULL, MAKEINTRESOURCEW(IDC_ARROW));
    RegisterClassW(&lc);

    HWND hwnd = CreateWindowExW(0, L"HvrSinkGuiClass",
                    L"HVR VR Glass 串流接收端 —— 低配电脑显示端",
                    WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                    CW_USEDEFAULT, CW_USEDEFAULT, 1000, 700,
                    NULL, NULL, hInst, NULL);
    if (!hwnd) {
        MessageBoxW(NULL, L"窗口创建失败", L"错误", MB_ICONERROR);
        return 1;
    }

    ShowWindow(hwnd, nShow);
    UpdateWindow(hwnd);

    MSG msg;
    while (GetMessageW(&msg, NULL, 0, 0)) {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    return (int)msg.wParam;
}
