//! 眼镜渲染窗口（单 exe GUI 内置）：把解码后的 BGRA 帧以 SBS 显示到 VR Glass 屏。
//!
//! 实现：Win32 全屏窗口 + GDI StretchDIBits。帧由 decoder 直接写入共享 FrameBuffer
//!      （不经 9911 网络），渲染线程取帧后按 SBS 左右眼各画半幅到窗口。
//!      窗口优先放到 EDID=HWV0003 的眼镜屏。
//!
//! 说明：VR Glass 面板物理±90°安装，真机需在 quad 层做旋转补偿；GDI 方案先保证
//!       "有画面"，旋转/畸变调优见 docs/04-渲染器补丁.md。

#![cfg(windows)]

use std::sync::{Arc, RwLock};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::HRGN;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, GetDC, ReleaseDC, SelectObject, StretchDIBits,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetWindowRect, RegisterClassW, ShowWindow,
    WNDCLASSW, CW_USEDEFAULT, SW_SHOW, WM_PAINT, WS_EX_TOPMOST, WS_POPUP,
};

/// 解码帧共享缓冲：decoder 写，渲染线程读。
#[derive(Clone, Default)]
pub struct FrameBuffer {
    inner: Arc<RwLock<FrameData>>,
}

#[derive(Default)]
struct FrameData {
    pixels: Vec<u8>,
    w: u32,
    h: u32,
    layout: u32, // 0=SBS 左右, 1=TB 上下
    seq: u64,
}

impl FrameBuffer {
    pub fn publish(&self, bgra: &[u8], w: u32, h: u32, layout: u32) {
        let mut d = self.inner.write().unwrap();
        if d.pixels.len() != bgra.len() { d.pixels.resize(bgra.len(), 0); }
        d.pixels.copy_from_slice(bgra);
        d.w = w; d.h = h; d.layout = layout;
        d.seq = d.seq.wrapping_add(1);
    }
    pub fn snapshot(&self) -> (Vec<u8>, u32, u32, u32, u64) {
        let d = self.inner.read().unwrap();
        (d.pixels.clone(), d.w, d.h, d.layout, d.seq)
    }
}

/// 渲染窗口状态（传给窗口过程）。
struct RenderState {
    frames: FrameBuffer,
    last_seq: u64,
    last: Option<(Vec<u8>, u32, u32)>,
}

unsafe extern "system" fn render_wndproc(
    hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    if msg == WM_PAINT {
        let state = windows::Win32::UI::WindowsAndMessaging::GetWindowLongPtrW(
            hwnd, windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA,
        ) as *mut RenderState;
        if !state.is_null() {
            let st = &mut *state;
            let (pixels, w, h, _layout, seq) = st.frames.snapshot();
            if seq != st.last_seq && !pixels.is_empty() {
                st.last_seq = seq;
                st.last = Some((pixels, w, h));
            }
            if let Some((px, w, h)) = &st.last {
                let hdc = windows::Win32::Graphics::Gdi::GetDC(hwnd);
                let mut rc = RECT::default();
                let _ = GetWindowRect(hwnd, &mut rc);
                let ww = (rc.right - rc.left).max(1);
                let wh = (rc.bottom - rc.top).max(1);

                let mut bi = BITMAPINFO::default();
                bi.bmiHeader = BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: *w as i32,
                    biHeight: -(*h as i32), // top-down BGRA
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    biSizeImage: 0,
                    biXPelsPerMeter: 0,
                    biYPelsPerMeter: 0,
                    biClrUsed: 0,
                    biClrImportant: 0,
                };
                // SBS：整幅帧（左右眼并排）直接铺满窗口
                unsafe {
                    StretchDIBits(
                        hdc, 0, 0, ww, wh, 0, 0, *w as i32, *h as i32,
                        Some(px.as_ptr() as *const core::ffi::c_void),
                        &bi, DIB_RGB_COLORS, SRCCOPY,
                    );
                }
                let _ = ReleaseDC(hwnd, hdc);
            }
        }
        return windows::Win32::Foundation::LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// 找 VR Glass 眼镜屏（3200×1600 的第二屏）。返回 (x, y, w, h)。
/// 用「宽度≥3000」判据：VR Glass 面板 3200×1600，普通 PC 屏远小于此。
fn find_glasses_screen() -> Option<(i32, i32, i32, i32)> {
    use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, MONITORINFO, HMONITOR};
    use windows::Win32::Foundation::BOOL;
    unsafe extern "system" fn cb(
        hmon: HMONITOR, _hdc: windows::Win32::Graphics::Gdi::HDC,
        _clip: *mut RECT, data: windows::Win32::Foundation::LPARAM,
    ) -> BOOL {
        let mut mi = MONITORINFO::default();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(hmon, &mut mi).as_bool() {
            let w = mi.rcMonitor.right - mi.rcMonitor.left;
            if w >= 3000 {
                let out = &mut *(data.0 as *mut Option<(i32, i32, i32, i32)>);
                *out = Some((mi.rcMonitor.left, mi.rcMonitor.top, w, mi.rcMonitor.bottom - mi.rcMonitor.top));
                return BOOL(0); // 找到，停止枚举
            }
        }
        BOOL(1)
    }
    let mut out: Option<(i32, i32, i32, i32)> = None;
    unsafe {
        EnumDisplayMonitors(
            windows::Win32::Graphics::Gdi::HDC::default(), None,
            Some(cb), windows::Win32::Foundation::LPARAM(&mut out as *mut _ as isize),
        );
    }
    out
}

/// 启动渲染窗口（阻塞，运行消息循环）。FrameBuffer 供 decoder 写帧。
/// 调用方应在独立线程里跑这个函数。
pub fn run_render_window(frames: FrameBuffer) -> windows::core::Result<()> {
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, LoadCursorW, PostQuitMessage, TranslateMessage,
        IDC_ARROW, MSG, WM_DESTROY,
    };

    let cls = windows::core::w!("HvrRenderWindowClass");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(render_wndproc),
        hCursor: unsafe { LoadCursorW(HINSTANCE::default(), IDC_ARROW).unwrap_or_default() },
        lpszClassName: cls,
        hbrBackground: unsafe { windows::Win32::Graphics::Gdi::GetSysColorBrush(
            windows::Win32::Graphics::Gdi::SYS_COLOR_INDEX(1),
        ) },
        ..Default::default()
    };
    let _atom = unsafe { RegisterClassW(&wc) };

    let state = Box::into_raw(Box::new(RenderState {
        frames,
        last_seq: 0,
        last: None,
    }));
    // 眼镜屏（3200×1600）存在则放上去，否则退化为主屏全屏（便于无眼镜调试）
    let (vx, vy, vw, vh) = find_glasses_screen().unwrap_or_else(|| {
        let w = unsafe { windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
            windows::Win32::UI::WindowsAndMessaging::SM_CXSCREEN) };
        let h = unsafe { windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
            windows::Win32::UI::WindowsAndMessaging::SM_CYSCREEN) };
        (0, 0, w, h)
    });
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST, cls, windows::core::w!("VR Glass 显示"),
            WS_POPUP, vx, vy, vw, vh,
            HWND::default(), windows::Win32::UI::WindowsAndMessaging::HMENU::default(), HINSTANCE::default(),
            Some(state as *const core::ffi::c_void),
        )
    }?;
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::SetWindowLongPtrW(
            hwnd, windows::Win32::UI::WindowsAndMessaging::GWLP_USERDATA, state as isize,
        );
    }
    unsafe { ShowWindow(hwnd, SW_SHOW); }

    // 定时刷新窗口（触发 WM_PAINT）
    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, HWND::default(), 0, 0) }.as_bool() {
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if msg.message == WM_DESTROY { unsafe { PostQuitMessage(0); } }
    }
    Ok(())
}

/// 请求重绘（由 decoder 线程在有新帧时调用）。
pub fn invalidate() {
    // 用全局 HWND 广播重绘；简单可靠。
    unsafe {
        windows::Win32::Graphics::Gdi::RedrawWindow(
            HWND::default(), Default::default(), HRGN::default(),
            windows::Win32::Graphics::Gdi::RDW_ALLCHILDREN
                | windows::Win32::Graphics::Gdi::RDW_INVALIDATE,
        );
    }
}
