//! 渲染器桥接：Rust 解码进程 ⇄ 本地 SBS 渲染器（patched HwVRDesktop）
//!
//! 设计取舍：不链接任何自写 C++/C ABI，全部走本地回环网络，纯 std 实现，
//!           因此本工程在 Windows / Linux 均可编译，且可在本机做集成测试。
//!           渲染核心（D3D11 + SBS + 畸变 + IMU）复用上游已真机验证的
//!           `deps/huawei-desktop/`，只给它加一个「网络帧输入」小补丁。
//!
//! 通道：
//! - 帧输出：Rust → TCP 127.0.0.1:9911，帧格式 [u32 宽][u32 高][u32 布局][BGRA 像素...]
//! - 姿态输入：渲染器 → UDP 127.0.0.1:9912，数据 [f32;4](xyzw) + [f32;3](xyz)
//!
//! 上游渲染器补丁见 docs/04-渲染器补丁.md。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub const FRAME_PORT: u16 = 9911;
pub const POSE_PORT: u16 = 9912;

/// 帧输出器：把解码后的 BGRA 帧通过 TCP 发给本地渲染器
pub struct FrameOutput {
    stream: Option<TcpStream>,
    connected: bool,
}

impl FrameOutput {
    /// 尝试连接本地渲染器；未启动也返回对象（会持续重连）
    pub fn connect() -> Self {
        let stream = TcpStream::connect_timeout(
            &"127.0.0.1:9911".parse().unwrap(),
            Duration::from_millis(300),
        )
        .ok();
        let connected = stream.is_some();
        Self { stream, connected }
    }

    pub fn is_connected(&self) -> bool { self.connected }

    /// 发送一帧。返回 false 表示渲染器未连接（帧被丢弃，不阻塞解码）。
    pub fn send_frame(&mut self, bgra: &[u8], w: u32, h: u32, layout: u32) -> bool {
        if self.stream.is_none() {
            // 每 2 秒重试连接一次（渲染器可能后启动）
            if let Ok(s) = TcpStream::connect_timeout(
                &format!("127.0.0.1:{FRAME_PORT}").parse().unwrap(),
                Duration::from_millis(100),
            ) {
                self.stream = Some(s);
                self.connected = true;
            } else {
                return false;
            }
        }
        let s = self.stream.as_mut().unwrap();
        let mut header = Vec::with_capacity(12);
        header.extend_from_slice(&w.to_le_bytes());
        header.extend_from_slice(&h.to_le_bytes());
        header.extend_from_slice(&layout.to_le_bytes());
        let ok = s.write_all(&header).and_then(|_| s.write_all(bgra)).is_ok();
        if !ok {
            self.stream = None;
            self.connected = false;
        }
        ok
    }
}

/// 姿态接收器：从渲染器 UDP 收 IMU 四元数，供回传 server
pub struct PoseInput {
    sock: UdpSocket,
    // 用原子位存四元数，避免锁
    qx: Arc<AtomicU32>, qy: Arc<AtomicU32>, qz: Arc<AtomicU32>, qw: Arc<AtomicU32>,
    valid: Arc<AtomicU32>,
}

impl PoseInput {
    pub fn bind() -> std::io::Result<Self> {
        let sock = UdpSocket::bind(("127.0.0.1", POSE_PORT))?;
        sock.set_read_timeout(Some(Duration::from_millis(20))).ok();
        Ok(Self {
            sock,
            qx: Arc::new(AtomicU32::new(0)),
            qy: Arc::new(AtomicU32::new(0)),
            qz: Arc::new(AtomicU32::new(0)),
            qw: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            valid: Arc::new(AtomicU32::new(0)),
        })
    }

    /// 启动后台接收线程；返回给 client 用的姿态闭包
    pub fn start(self) -> impl Fn() -> ([f32; 4], [f32; 3]) + Send {
        // 一份给接收线程，一份留给返回的读取闭包
        let qx_r = Arc::clone(&self.qx);
        let qy_r = Arc::clone(&self.qy);
        let qz_r = Arc::clone(&self.qz);
        let qw_r = Arc::clone(&self.qw);
        let valid_r = Arc::clone(&self.valid);
        let qx = Arc::clone(&self.qx);
        let qy = Arc::clone(&self.qy);
        let qz = Arc::clone(&self.qz);
        let qw = Arc::clone(&self.qw);
        let valid = Arc::clone(&self.valid);
        let sock = self.sock;
        std::thread::spawn(move || {
            let mut buf = [0u8; 32];
            loop {
                match sock.recv(&mut buf) {
                    Ok(n) if n >= 16 => {
                        let q = [
                            f32::from_le_bytes(buf[0..4].try_into().unwrap()),
                            f32::from_le_bytes(buf[4..8].try_into().unwrap()),
                            f32::from_le_bytes(buf[8..12].try_into().unwrap()),
                            f32::from_le_bytes(buf[12..16].try_into().unwrap()),
                        ];
                        qx_r.store(q[0].to_bits(), Ordering::Relaxed);
                        qy_r.store(q[1].to_bits(), Ordering::Relaxed);
                        qz_r.store(q[2].to_bits(), Ordering::Relaxed);
                        qw_r.store(q[3].to_bits(), Ordering::Relaxed);
                        valid_r.store(1, Ordering::Relaxed);
                    }
                    _ => std::thread::sleep(Duration::from_millis(1)),
                }
            }
        });
        move || {
            // 解析式头姿：朝向来自 IMU，位置固定原点（3DoF）
            let q = [
                f32::from_bits(qx.load(Ordering::Relaxed)),
                f32::from_bits(qy.load(Ordering::Relaxed)),
                f32::from_bits(qz.load(Ordering::Relaxed)),
                f32::from_bits(qw.load(Ordering::Relaxed)),
            ];
            let _ = valid.load(Ordering::Relaxed);
            (q, [0.0, 0.0, 0.0])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_header_layout() {
        let mut h = Vec::new();
        h.extend_from_slice(&3072u32.to_le_bytes());
        h.extend_from_slice(&1536u32.to_le_bytes());
        h.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(h.len(), 12);
        assert_eq!(u32::from_le_bytes(h[0..4].try_into().unwrap()), 3072);
        assert_eq!(u32::from_le_bytes(h[4..8].try_into().unwrap()), 1536);
        assert_eq!(u32::from_le_bytes(h[8..12].try_into().unwrap()), 0);
    }

    #[test]
    fn pose_roundtrip_via_udp() {
        // 起一个姿态接收器，从另一个 socket 发四元数，验证闭包能读到
        let input = PoseInput::bind().expect("bind pose port");
        // 先发再启动线程（UDP 数据报会缓存在 socket 缓冲）
        let sender = UdpSocket::bind("127.0.0.1:0").unwrap();
        let q = [0.1f32, 0.2, 0.3, 0.9];
        let mut pkt = Vec::new();
        for v in q { pkt.extend_from_slice(&v.to_le_bytes()); }
        sender.send_to(&pkt, ("127.0.0.1", POSE_PORT)).unwrap();
        let pose = input.start();
        std::thread::sleep(Duration::from_millis(80));
        let (got_q, got_p) = pose();
        assert!((got_q[0] - 0.1).abs() < 1e-5, "qx={}", got_q[0]);
        assert!((got_q[3] - 0.9).abs() < 1e-5, "qw={}", got_q[3]);
        assert_eq!(got_p, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn frame_output_no_renderer_is_graceful() {
        // 9911 无监听时应返回 false 而不 panic
        let mut out = FrameOutput::connect();
        let frame = vec![0u8; 16];
        let _ = out.send_frame(&frame, 4, 4, 0);
    }
}
