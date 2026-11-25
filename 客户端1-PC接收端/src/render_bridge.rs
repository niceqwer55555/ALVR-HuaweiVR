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
use std::time::{Duration, Instant};

pub const FRAME_PORT: u16 = 9911;
pub const POSE_PORT: u16 = 9912;
/// 手柄6DoF姿态输入端口：slam_bridge.exe → 本进程
/// 数据 = 2手 × [pos x,y,z + quat w,x,y,z] = 14 × f32 = 56 字节（小端）
pub const HAND_POSE_PORT: u16 = 9913;

/// 帧输出器：把解码后的 BGRA 帧通过 TCP 发给本地渲染器
pub struct FrameOutput {
    stream: Option<TcpStream>,
    connected: bool,
    last_attempt: Instant,
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
        Self { stream, connected, last_attempt: Instant::now() }
    }

    pub fn is_connected(&self) -> bool { self.connected }

    /// 发送一帧。返回 false 表示渲染器未连接（帧被丢弃，不阻塞解码）。
    pub fn send_frame(&mut self, bgra: &[u8], w: u32, h: u32, layout: u32) -> bool {
        if self.stream.is_none() {
            // 每 2 秒重试一次；旧实现每帧重试，渲染器未启动时每帧阻塞 100ms
            if self.last_attempt.elapsed() >= Duration::from_secs(2) {
                self.last_attempt = Instant::now();
                if let Ok(s) = TcpStream::connect_timeout(
                    &format!("127.0.0.1:{FRAME_PORT}").parse().unwrap(),
                    Duration::from_millis(100),
                ) {
                    self.stream = Some(s);
                    self.connected = true;
                } else {
                    return false;
                }
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
                    // 头姿来自渲染器 Hmd.dll 的 IMU 融合(g_headRot)，16B quat xyzw。
                    // PCCastScreen 激活后 IMU 有效(yaw随头动)，用它回传服务端渲染。
                    // 过滤单位四元数：IMU 未激活时恒为(0,0,0,1)，不采信以免污染。
                    Ok(n) if n >= 16 => {
                        let q = [
                            f32::from_le_bytes(buf[0..4].try_into().unwrap()),
                            f32::from_le_bytes(buf[4..8].try_into().unwrap()),
                            f32::from_le_bytes(buf[8..12].try_into().unwrap()),
                            f32::from_le_bytes(buf[12..16].try_into().unwrap()),
                        ];
                        let is_identity = q[0].abs() < 1e-6 && q[1].abs() < 1e-6
                            && q[2].abs() < 1e-6 && (q[3] - 1.0).abs() < 1e-6;
                        if !is_identity && q.iter().all(|v| v.is_finite()) {
                            qx_r.store(q[0].to_bits(), Ordering::Relaxed);
                            qy_r.store(q[1].to_bits(), Ordering::Relaxed);
                            qz_r.store(q[2].to_bits(), Ordering::Relaxed);
                            qw_r.store(q[3].to_bits(), Ordering::Relaxed);
                            valid_r.store(1, Ordering::Relaxed);
                        }
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

/// 手柄 6DoF 姿态接收器（来自 slam_bridge.exe，读华为 VR6DOF.dll 的 SLAM 结果）
/// 返回 (左手 quat,左手 pos,右手 quat,右手 pos)，无数据时 None（让 client 回退默认姿态）
pub struct HandPoseInput {
    data: Arc<Vec<AtomicU32>>,   // 14 个 f32 的位模式
    stamp: Arc<AtomicU32>,       // 最后一次收到数据的 unix 秒（低32位）
}

impl HandPoseInput {
    /// 端口占用时的空实现：永远返回 None（client 回退默认姿态）
    pub fn dummy() -> Self {
        Self { data: Arc::new((0..14).map(|_| AtomicU32::new(0)).collect()), stamp: Arc::new(AtomicU32::new(0)) }
    }

    pub fn bind() -> std::io::Result<Self> {
        let sock = UdpSocket::bind(("127.0.0.1", HAND_POSE_PORT))?;
        sock.set_read_timeout(Some(Duration::from_millis(20))).ok();
        let data = Arc::new((0..14).map(|_| AtomicU32::new(0)).collect::<Vec<_>>());
        let stamp = Arc::new(AtomicU32::new(0));
        let (data_r, stamp_r) = (Arc::clone(&data), Arc::clone(&stamp));
        std::thread::spawn(move || {
            let mut buf = [0u8; 64];
            loop {
                match sock.recv(&mut buf) {
                    Ok(n) if n >= 56 => {
                        for i in 0..14 {
                            let b: [u8;4] = buf[i*4..i*4+4].try_into().unwrap();
                            data_r[i].store(u32::from_le_bytes(b), Ordering::Relaxed);
                        }
                        stamp_r.store(
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH).unwrap_or_default()
                                .as_secs() as u32, Ordering::Relaxed);
                    }
                    _ => std::thread::sleep(Duration::from_millis(1)),
                }
            }
        });
        Ok(Self { data, stamp })
    }

    /// 返回 Some((Lq,Lp,Rq,Rp))，数据超过3秒未更新则返回 None
    pub fn read(&self) -> Option<([f32;4],[f32;3],[f32;4],[f32;3])> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as u32;
        let st = self.stamp.load(Ordering::Relaxed);
        if st == 0 || now.wrapping_sub(st) > 3 { return None; }
        let f = |i: usize| f32::from_bits(self.data[i].load(Ordering::Relaxed));
        // 包布局: hand0 = pos(0..3) quat(3..7), hand1 = pos(7..10) quat(10..14)
        let q = |i: usize| [f(i+3), f(i+4), f(i+5), f(i+6)];
        let p = |i: usize| [f(i), f(i+1), f(i+2)];
        // VR6DOF 的 hand0 常为 0（单手在线），hand1 有数据；按位置 x 符号映射到左右手
        let (a_q,a_p,b_q,b_p) = (q(0),p(0),q(7),p(7));
        let a_valid = a_p.iter().any(|v| v.abs() > 1e-4) || a_q[0].abs() > 1e-4;
        let b_valid = b_p.iter().any(|v| v.abs() > 1e-4) || b_q[0].abs() > 1e-4;
        // 默认把有数据的手当左手（x<0），另一只当右手；若只有一只在线，另一只回退 None 由 client 处理
        if a_valid && b_valid {
            // 按 x 坐标排序：x 更负的是左手
            if a_p[0] <= b_p[0] { Some((a_q,a_p,b_q,b_p)) } else { Some((b_q,b_p,a_q,a_p)) }
        } else if a_valid {
            Some((a_q,a_p,a_q,a_p))  // 仅一只手在线：两手共用（可后续校准）
        } else if b_valid {
            Some((b_q,b_p,b_q,b_p))
        } else {
            None
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
        for v in [0.0f32, 0.0, 0.0] { pkt.extend_from_slice(&v.to_le_bytes()); } // 28B SLAM包
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
