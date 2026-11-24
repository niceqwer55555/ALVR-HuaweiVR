//! 单 exe GUI：把核心连接逻辑与眼镜渲染都放进本进程，交付包只需一个 exe。
//!
//! 窗口：启动/停止按钮 + ALVR 连接状态灯 + 日志区。
//! 点【启动串流】后：
//!   1) 起渲染窗口线程（renderer::run_render_window），FrameBuffer 供解码帧写入
//!   2) 起核心线程：client::run 连服务端，decoder 解出的 BGRA 帧直接 publish 到 FrameBuffer
//! 停止：置 stop 标志 + 关闭渲染窗口。

#![cfg(windows)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use windows::core::{w, Result as WinResult};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, EndPaint, FillRect, GetDC, Rectangle, ReleaseDC,
    SetBkMode, SetTextColor, TextOutW, HBRUSH, PAINTSTRUCT, TRANSPARENT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, GetWindowTextLengthW,
    GetWindowLongPtrW, LoadCursorW, PostQuitMessage, PostMessageW, RegisterClassW, SendMessageW,
    SetWindowLongPtrW, ShowWindow, TranslateMessage, CW_USEDEFAULT, ES_AUTOVSCROLL,
    ES_MULTILINE, ES_READONLY, GWLP_USERDATA, IDC_ARROW, MSG, SW_SHOW, WM_COMMAND,
    WM_DESTROY, WM_PAINT, WM_USER, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    WS_VSCROLL, BS_PUSHBUTTON, HMENU, WNDCLASSW, WINDOW_STYLE,
};
use windows::Win32::UI::Controls::{EM_REPLACESEL, EM_SCROLLCARET, EM_SETSEL};

use crate::client::{self, ClientConfig};
use crate::decoder::{DecodeConfig, Decoder};
use crate::protocol::Fov;
use crate::renderer::{self, FrameBuffer};
use crate::status;

const ID_BTN_START: usize = 2003;
const ID_BTN_STOP: usize = 2004;
const ID_LOG: usize = 2002;
const ID_LAMP: usize = 2009;
const WM_LOG: u32 = WM_USER + 1;

struct GuiState {
    hwnd: HWND,
    hlog: HWND,
    stop: Arc<AtomicBool>,
    running: bool,
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut GuiState;
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let _dc = BeginPaint(hwnd, &mut ps);
            if !ptr.is_null() {
                let st = &*ptr;
                let mut rc = RECT::default();
                let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rc);
                let dc = ps.hdc;
                let conn = status::alvr_connected();
                let brush = CreateSolidBrush(if conn { windows::Win32::Foundation::COLORREF(0x00_2ED468) } else { windows::Win32::Foundation::COLORREF(0x00_6E6E76) });
                SetTextColor(dc, if conn { windows::Win32::Foundation::COLORREF(0x00_14782D) } else { windows::Win32::Foundation::COLORREF(0x00_5A5A5F) });
                SetBkMode(dc, TRANSPARENT);
                let lamp = 18;
                let x = 16; let y = 14;
                FillRect(dc, &RECT { left: x, top: y, right: x + lamp * 2, bottom: y + lamp * 2 }, HBRUSH(0x00_F5F5F8usize as *mut core::ffi::c_void));
                Rectangle(dc, x, y, x + lamp * 2, y + lamp * 2);
                let old = windows::Win32::Graphics::Gdi::SelectObject(dc, brush);
                windows::Win32::Graphics::Gdi::Ellipse(dc, x + 4, y + 4, x + lamp * 2 - 4, y + lamp * 2 - 4);
                windows::Win32::Graphics::Gdi::SelectObject(dc, old);
                let _ = DeleteBrush(brush);
                TextOutW(dc, x + lamp * 2 + 12, y + 8, &[0x41,0x4C,0x56,0x52, 0x20, 0x4E,0x4A,0x8F,0x54]);
                let text: &[u16] = if conn {
                    &[0x5DF2,0x8FDE,0x63A5,0x20,0x50,0x43,0x20,0x4E32,0x6D41,0x7AEF]
                } else {
                    &[0x7B49,0x5F85,0x20,0x50,0x43,0x20,0x4E32,0x6D41,0x7AEF]
                };
                TextOutW(dc, x + lamp * 2 + 12, y + 28, text);
            }
            EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_LOG => {
            if !ptr.is_null() {
                let st = &*ptr;
                let text = lp.0 as *const u16;
                let len = GetWindowTextLengthW(st.hlog);
                let _ = SendMessageW(st.hlog, EM_SETSEL, WPARAM(len as usize), LPARAM(len as isize));
                let _ = SendMessageW(st.hlog, EM_REPLACESEL, WPARAM(0), LPARAM(text as isize));
                let _ = SendMessageW(st.hlog, EM_SCROLLCARET, WPARAM(0), LPARAM(0));
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = (wp.0 as usize) & 0xFFFF;
            if !ptr.is_null() {
                let st = &mut *ptr;
                if id == ID_BTN_START && !st.running { start_stream(st); }
                else if id == ID_BTN_STOP && st.running { stop_stream(st); }
            }
            LRESULT(0)
        }
        WM_DESTROY => { PostQuitMessage(0); LRESULT(0) }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn DeleteBrush(b: HBRUSH) {
    let _ = unsafe { windows::Win32::Graphics::Gdi::DeleteObject(windows::Win32::Graphics::Gdi::HGDIOBJ(b.0)) };
}

fn log(st: &GuiState, s: &str) {
    let wide: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
    let boxed = wide.into_boxed_slice();
    let ptr = Box::into_raw(boxed) as *mut u16;
    unsafe {
        // 用 PostMessage 异步传递；内存在 WM_LOG 处理后由接收端释放（这里简化为一次性泄漏极小量）
        let _ = PostMessageW(
            st.hwnd, WM_LOG, WPARAM(0), LPARAM(ptr as isize));
    }
}

fn load_config() -> ClientConfig {
    let text = std::fs::read_to_string("sink_config.json").unwrap_or_default();
    let mut cfg = ClientConfig::default();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) {
        if let Some(h) = v["hostname"].as_str() { cfg.hostname = h.into(); }
        if let Some(r) = v["view_resolution"].as_array() {
            if r.len() == 2 {
                cfg.view_resolution = [r[0].as_u64().unwrap_or(1536) as u32, r[1].as_u64().unwrap_or(1536) as u32];
            }
        }
        if let Some(f) = v["target_fps"].as_f64() { cfg.target_fps = f as f32; }
        if let Some(p) = v["ipd_m"].as_f64() { cfg.ipd_m = p as f32; }
        if let Some(port) = v["stream_port"].as_u64() { cfg.stream_port = port as u16; }
        if let Some(layout) = v["stereo_layout"].as_str() {
            cfg.stereo_layout = if layout.to_lowercase().starts_with("tb") { 1 } else { 0 };
        }
    }
    cfg
}

fn start_stream(st: &mut GuiState) {
    st.running = true;
    st.stop.store(false, Ordering::SeqCst);
    log(st, "[系统] 启动串流（含眼镜渲染）…\r\n");

    let frames = FrameBuffer::default();

    // 渲染窗口线程（眼镜屏）
    let rf = frames.clone();
    std::thread::spawn(move || {
        if let Err(e) = renderer::run_render_window(rf) {
            eprintln!("[render] 渲染窗口失败: {e}");
        }
    });

    // 核心连接线程
    let cfg = load_config();
    let stop = Arc::clone(&st.stop);
    let hwnd_raw = st.hwnd.0 as isize;
    std::thread::spawn(move || {
        let mut dc = DecodeConfig::default();
        dc.width = cfg.view_resolution[0];
        dc.height = cfg.view_resolution[1];
        dc.fps = cfg.target_fps;
        dc.layout = cfg.stereo_layout;
        dc.ffmpeg = crate::resolve_ffmpeg();

        let layout = cfg.stereo_layout;
        let fframes = frames.clone();
        let mut dec = match Decoder::start(dc, move |bgra, w, h| {
            fframes.publish(bgra, w, h, layout);
            renderer::invalidate();
        }) {
            Ok(d) => d,
            Err(e) => {
                let msg = format!("[错误] 解码器启动失败: {e}\r\n");
                post_log(HWND(hwnd_raw as *mut core::ffi::c_void), &msg);
                return;
            }
        };
        let mut on_frame = |_t: std::time::Duration, nal: &[u8]| -> bool { dec.push_nal(nal) };
        let pose = || ([0.0f32, 0.0, 0.0, 1.0], [0.0f32, 0.0, 0.0]);
        let battery_mon = status::DeviceStatus::collect();
        let battery = move || battery_mon.hmd_battery();
        let summary = client::run(&cfg, &mut on_frame, pose, battery, stop);
        let msg = format!("[系统] 会话结束: {:?}\r\n", summary);
        post_log(HWND(hwnd_raw as *mut core::ffi::c_void), &msg);
        dec.stop();
    });
}

fn post_log(hwnd: HWND, s: &str) {
    let mut wide: Vec<u16> = s.encode_utf16().chain(std::iter::once(0)).collect();
    let ptr = wide.as_mut_ptr();
    let _ = unsafe { PostMessageW(
        hwnd, WM_LOG, WPARAM(0), LPARAM(ptr as isize)) };
    std::mem::forget(wide);
}

fn stop_stream(st: &mut GuiState) {
    st.stop.store(true, Ordering::SeqCst);
    st.running = false;
    log(st, "[系统] 已停止串流\r\n");
}

/// 运行 GUI（阻塞消息循环）。autostart=true 时窗口出现后立即开始串流（用于无人值守测试）。
pub fn run_gui_with_autostart(autostart: bool) -> WinResult<()> {
    let cls = w!("HvrSinkGuiClass");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        hCursor: unsafe { LoadCursorW(HINSTANCE::default(), IDC_ARROW).unwrap_or_default() },
        lpszClassName: cls,
        hbrBackground: HBRUSH(6usize as *mut core::ffi::c_void),
        ..Default::default()
    };
    let _ = unsafe { RegisterClassW(&wc) };

    let state = Box::new(GuiState {
        hwnd: HWND::default(), hlog: HWND::default(),
        stop: Arc::new(AtomicBool::new(false)), running: false,
    });
    let sp = Box::into_raw(state);
    let hwnd = unsafe {
        CreateWindowExW(
            Default::default(), cls, w!("HVR VR Glass 接收端（单 exe）"),
            WINDOW_STYLE(WS_OVERLAPPEDWINDOW.0) | WINDOW_STYLE(WS_VISIBLE.0),
            CW_USEDEFAULT, CW_USEDEFAULT, 720, 520,
            HWND::default(), HMENU::default(), HINSTANCE::default(),
            Some(sp as *const core::ffi::c_void),
        )
    }?;
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, sp as isize);
        (*sp).hwnd = hwnd;
    }

    let hlog = unsafe {
        CreateWindowExW(
            Default::default(), w!("Edit"), w!(""),
            WINDOW_STYLE(WS_CHILD.0) | WINDOW_STYLE(WS_VISIBLE.0) | WINDOW_STYLE(WS_VSCROLL.0) | WINDOW_STYLE(ES_MULTILINE as u32) | WINDOW_STYLE(ES_READONLY as u32) | WINDOW_STYLE(ES_AUTOVSCROLL as u32),
            16, 70, 672, 340, hwnd, HMENU(ID_LOG as usize as *mut core::ffi::c_void), HINSTANCE::default(), None,
        )
    }?;
    let _btn_start = unsafe {
        CreateWindowExW(Default::default(), w!("Button"), w!("启动串流（含画面）"),
            WINDOW_STYLE(WS_CHILD.0) | WINDOW_STYLE(WS_VISIBLE.0) | WINDOW_STYLE(BS_PUSHBUTTON as u32), 16, 430, 200, 36,
            hwnd, HMENU(ID_BTN_START as usize as *mut core::ffi::c_void), HINSTANCE::default(), None)
    }?;
    let _btn_stop = unsafe {
        CreateWindowExW(Default::default(), w!("Button"), w!("停止串流"),
            WINDOW_STYLE(WS_CHILD.0) | WINDOW_STYLE(WS_VISIBLE.0) | WINDOW_STYLE(BS_PUSHBUTTON as u32), 240, 430, 160, 36,
            hwnd, HMENU(ID_BTN_STOP as usize as *mut core::ffi::c_void), HINSTANCE::default(), None)
    }?;
    unsafe { (*sp).hlog = hlog; }

    // 状态文件写入线程：供外部工具/GUI 读取 device_status.json（含 alvr_connected）
    std::thread::spawn(|| loop {
        let mut s = status::DeviceStatus::collect();
        s.alvr_connected = status::alvr_connected();
        if let Ok(j) = serde_json::to_string_pretty(&s) {
            let _ = std::fs::write("device_status.json", j);
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    });

    unsafe { ShowWindow(hwnd, SW_SHOW); }
    if autostart { unsafe { start_stream(&mut *sp); } }

    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, HWND::default(), 0, 0) }.as_bool() {
        unsafe { TranslateMessage(&msg); DispatchMessageW(&msg); }
    }
    Ok(())
}
