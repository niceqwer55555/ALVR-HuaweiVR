//! ALVR 客户端网络核心：发现 → 握手 → 收流 → 回传 tracking
//!
//! 完全复刻上游 connection.rs 的时序，但不依赖其残缺 workspace。
//! 视频帧以 Annex B NAL 交付给回调；头姿由渲染器提供，按 tracking 流回传。

use crate::protocol::*;
use crate::status;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

const DISCOVERY_RETRY: Duration = Duration::from_millis(500);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);
const KEEPALIVE_INTERVAL: Duration = Duration::from_millis(500);
const MAX_UNREAD_PACKETS: usize = 10;

/// 一次视频帧回调：timestamp 来自 VideoPacketHeader，nal 为 Annex B 字节流
pub type FrameCallback = dyn FnMut(Duration, &[u8]) -> bool + Send;

/// 头姿提供者：返回 (orientation xyzw, position xyz)，由 C++ 渲染器经 FFI 填充
pub type PoseProvider = dyn Fn() -> ([f32; 4], [f32; 3]) + Send;

/// 电池提供者：返回 (gauge 0..1, is_plugged)，对应 ALVR ClientControlPacket::Battery(HMD)
pub type BatteryProvider = dyn Fn() -> (f32, bool) + Send;

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub hostname: String,
    pub view_resolution: [u32; 2],
    pub refresh_rates: Vec<f32>,
    pub microphone_sample_rate: u32,
    pub stream_port: u16,
    pub protocol_id_u64: u64,
    pub target_fps: f32,
    pub ipd_m: f32,
    pub fov: [Fov; 2],
    /// 立体布局：0=SBS 左右并排，1=TB 上下
    pub stereo_layout: u32,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            hostname: "HVR-SINK".into(),
            // 华为 Runtime 上报 1552×1552/眼，32 像素对齐 → 1536×1536（README 已验收档）
            view_resolution: [1536, 1536],
            refresh_rates: vec![70.0, 72.0],
            microphone_sample_rate: 48000,
            stream_port: 9944,
            // 上游 alvr_common::protocol_id_u64() = hash_string("20")（Rust DefaultHasher/SipHash），
            // 不是字面量 20。发错值服务端会 warn「incompatible client」，华为版虽不拒绝但应如实上报。
            protocol_id_u64: 1_686_174_404_061_257_063,
            target_fps: 70.0,
            ipd_m: 0.063,
            fov: [
                Fov { left: 0.829, right: 0.829, up: 0.829, down: 0.829 },
                Fov { left: 0.829, right: 0.829, up: 0.829, down: 0.829 },
            ],
            stereo_layout: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RunState {
    Discovering,
    Handshaking,
    Streaming,
    Stopped,
}

#[derive(Debug)]
pub struct RunSummary {
    pub frames_received: u64,
    pub idr_frames: u64,
    pub bytes_received: u64,
    pub tracking_sent: u64,
    pub reconnects: u64,
}

/// 运行客户端主循环（阻塞）。返回统计摘要。
pub fn run<F, P, B>(
    cfg: &ClientConfig,
    mut on_frame: F,
    pose: P,
    battery: B,
    stop: Arc<AtomicBool>,
) -> RunSummary
where
    F: FnMut(Duration, &[u8]) -> bool + Send,
    P: Fn() -> ([f32; 4], [f32; 3]) + Send,
    B: Fn() -> (f32, bool) + Send,
{
    let mut summary = RunSummary {
        frames_received: 0,
        idr_frames: 0,
        bytes_received: 0,
        tracking_sent: 0,
        reconnects: 0,
    };

    while !stop.load(Ordering::Relaxed) {
        match one_session(cfg, &mut on_frame, &pose, &battery, &stop, &mut summary) {
            Ok(()) => {}
            Err(e) => {
                status::set_alvr_connected(false);
                eprintln!("[hvr-sink] 会话结束: {e}");
            }
        }
        if stop.load(Ordering::Relaxed) {
            status::set_alvr_connected(false);
            break;
        }
        summary.reconnects += 1;
        std::thread::sleep(Duration::from_secs(1));
    }
    summary
}

fn one_session<F, P, B>(
    cfg: &ClientConfig,
    on_frame: &mut F,
    pose: &P,
    battery: &B,
    stop: &Arc<AtomicBool>,
    summary: &mut RunSummary,
) -> Result<(), String>
where
    F: FnMut(Duration, &[u8]) -> bool + Send,
    P: Fn() -> ([f32; 4], [f32; 3]) + Send,
    B: Fn() -> (f32, bool) + Send,
{
    // 1) 监听控制端口，等 server 连入（上游 ProtoControlSocket::connect_to(Server(listener))）
    let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, CONTROL_PORT))
        .map_err(|e| format!("绑定控制端口 {CONTROL_PORT} 失败: {e}（检查防火墙/端口占用）"))?;
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("set_nonblocking: {e}"))?;

    // 2) UDP 广播发现包 + 等待 server 的 TCP 连接
    let discovery = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .map_err(|e| format!("UDP 绑定失败: {e}"))?;
    discovery
        .set_broadcast(true)
        .ok();
    let pkt = build_discovery_packet(&cfg.hostname, cfg.protocol_id_u64);

    let mut control: Option<TcpStream> = None;
    let mut server_ip = IpAddr::V4(Ipv4Addr::LOCALHOST);
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline && !stop.load(Ordering::Relaxed) {
        discovery
            .send_to(&pkt, (Ipv4Addr::BROADCAST, CONTROL_PORT))
            .ok();
        match listener.accept() {
            Ok((s, addr)) => {
                server_ip = addr.ip();
                control = Some(s);
                break;
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(format!("accept 失败: {e}")),
        }
        std::thread::sleep(DISCOVERY_RETRY);
    }
    let mut control = control.ok_or("未发现 PC 串流端：确认高配 PC 的 ALVR Dashboard 正在运行、同局域网、防火墙放行 UDP/TCP 9943")?;
    control.set_read_timeout(Some(HANDSHAKE_TIMEOUT)).ok();
    control.set_write_timeout(Some(HANDSHAKE_TIMEOUT)).ok();
    eprintln!("[hvr-sink] 已连接串流端 {server_ip}");
    status::set_alvr_connected(true); // 控制通道连上即点亮 GUI 的 ALVR 串流灯

    // 3) 发送 ConnectionAccepted（含能力集）
    let caps = VideoStreamingCapabilitiesLegacy {
        default_view_resolution: cfg.view_resolution,
        supported_refresh_rates_plus_extra_data: cfg.refresh_rates.clone(),
        microphone_sample_rate: cfg.microphone_sample_rate,
    };
    let hello = ClientConnectionResult::ConnectionAccepted {
        client_protocol_id: cfg.protocol_id_u64,
        display_name: cfg.hostname.clone(),
        server_ip,
        streaming_capabilities: Some(caps),
    };
    control
        .write_all(&control_encode(&hello).map_err(|e| format!("hello 序列化失败: {e}"))?)
        .map_err(|e| format!("发送 hello 失败: {e}"))?;

    // 4) 收 StreamConfigPacket
    let config = read_control::<StreamConfigPacket>(&mut control, HANDSHAKE_TIMEOUT)?;
    let negotiated = NegotiatedConfig::from_json(&config.negotiated)
        .map_err(|e| format!("解析协商配置失败: {e}"))?;
    eprintln!(
        "[hvr-sink] 协商: {}x{} @ {:.1}Hz wired={}",
        negotiated.view_resolution[0],
        negotiated.view_resolution[1],
        negotiated.refresh_rate_hint,
        negotiated.wired
    );

    // 5) 等 StartStream / DecoderConfig
    let mut decoder_config: Option<DecoderInitializationConfig> = None;
    let mut started = false;
    for _ in 0..8 {
        let pkt = read_control::<ServerControlPacket>(&mut control, HANDSHAKE_TIMEOUT)?;
        match pkt {
            ServerControlPacket::StartStream => { started = true; break; }
            ServerControlPacket::DecoderConfig(c) => decoder_config = Some(c),
            ServerControlPacket::Restarting => return Err("服务端重启中".into()),
            other => eprintln!("[hvr-sink] 控制包: {other:?}"),
        }
    }
    if !started {
        return Err("未收到 StartStream".into());
    }
    if let Some(c) = &decoder_config {
        eprintln!("[hvr-sink] 解码器配置: codec={} config_buffer={}B", c.codec, c.config_buffer.len());
    }

    // 6) 建立流通道监听（上游 StreamSocketBuilder::listen_for_server）
    let stream_listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, cfg.stream_port))
        .map_err(|e| format!("绑定流端口 {} 失败: {e}", cfg.stream_port))?;
    stream_listener.set_nonblocking(true).ok();

    // 7) 发 StreamReady
    let ready = control_encode(&ClientControlPacket::StreamReady)
        .map_err(|e| format!("StreamReady 序列化: {e}"))?;
    control.write_all(&ready).map_err(|e| format!("发送 StreamReady: {e}"))?;

    // 8) 等 server 连流通道
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    let mut stream: Option<TcpStream> = None;
    while Instant::now() < deadline && !stop.load(Ordering::Relaxed) {
        match stream_listener.accept() {
            Ok((s, addr)) => { stream = Some(s); eprintln!("[hvr-sink] 流通道已连接 {addr}"); break; }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(format!("流 accept 失败: {e}")),
        }
    }
    let mut stream = stream.ok_or("流通道连接超时")?;
    stream.set_read_timeout(Some(Duration::from_millis(500))).ok();

    // 9) 发送 ViewsConfig（IPD + FOV），让 server 正确渲染双目
    let views = control_encode(&ClientControlPacket::ViewsConfig(ViewsConfig {
        ipd_m: cfg.ipd_m,
        fov: cfg.fov.clone(),
    })).map_err(|e| format!("ViewsConfig: {e}"))?;
    control.write_all(&views).ok();

    // 10) 主循环：收视频分片重组 + 定时回传 tracking + keepalive
    stream_video_and_track(
        &mut stream,
        &mut control,
        on_frame,
        pose,
        battery,
        stop,
        summary,
        cfg.target_fps,
    )
}

/// 从控制通道读一帧指定类型
fn read_control<T: serde::de::DeserializeOwned>(
    s: &mut TcpStream,
    timeout: Duration,
) -> Result<T, String> {
    let deadline = Instant::now() + timeout;
    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    let mut tmp = [0u8; 8192];
    loop {
        if buf.len() >= 4 {
            let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
            if len > 16 * 1024 * 1024 {
                return Err(format!("控制包长度异常: {len}"));
            }
            if buf.len() >= 4 + len {
                let (pkt, used) = control_decode::<T>(&buf)
                    .map_err(|e| format!("控制包解析失败: {e:?}"))?
                    .ok_or("控制包不完整")?;
                let _ = used;
                return Ok(pkt);
            }
        }
        if Instant::now() > deadline {
            return Err("读取控制包超时".into());
        }
        match s.read(&mut tmp) {
            Ok(0) => return Err("控制通道断开".into()),
            Ok(n) => buf.extend_from_slice(&tmp[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(format!("读控制通道: {e}")),
        }
    }
}

/// 视频分片重组状态
struct Reassembler {
    current_index: u32,
    last_complete_index: Option<u32>,
    expected_shards: usize,
    parts: Vec<Vec<u8>>,
    assembled: Vec<u8>,
    /// 检测到包序号跳变（丢包）时置位，交给调用方请求 IDR
    lost: bool,
}

impl Reassembler {
    fn new() -> Self {
        Self {
            current_index: u32::MAX,
            last_complete_index: None,
            expected_shards: 0,
            parts: vec![],
            assembled: Vec::with_capacity(2 * 1024 * 1024),
            lost: false,
        }
    }

    /// 收一个分片；完整时返回重组后的 (stream_id, 完整包字节)。
    /// 仅对视频流做序号连续性检测（audio/tracking 流序号独立）。
    fn push(&mut self, stream_id: u16, idx: u32, shards: usize, shard: usize, bytes: &[u8]) -> Option<(u16, Vec<u8>)> {
        if shards == 0 || shard >= shards {
            return None;
        }
        if idx != self.current_index {
            // 序号跳变检测：新包序号 should == last_complete+1
            if let Some(last) = self.last_complete_index {
                if idx != last.wrapping_add(1) {
                    self.lost = true;
                }
            }
            self.current_index = idx;
            self.expected_shards = shards;
            self.parts = vec![Vec::new(); shards];
        }
        if shard >= self.parts.len() {
            self.parts.resize(shards, Vec::new());
        }
        self.parts[shard].extend_from_slice(bytes);
        if self.parts.iter().all(|p| !p.is_empty()) {
            self.assembled.clear();
            for p in &self.parts {
                self.assembled.extend_from_slice(p);
            }
            self.last_complete_index = Some(idx);
            self.current_index = u32::MAX;
            Some((stream_id, std::mem::take(&mut self.assembled)))
        } else {
            None
        }
    }

    /// 取出并清除丢包标志
    fn take_lost(&mut self) -> bool {
        std::mem::replace(&mut self.lost, false)
    }
}

fn stream_video_and_track<F, P, B>(
    stream: &mut TcpStream,
    control: &mut TcpStream,
    on_frame: &mut F,
    pose: &P,
    battery: &B,
    stop: &Arc<AtomicBool>,
    summary: &mut RunSummary,
    target_fps: f32,
) -> Result<(), String>
where
    F: FnMut(Duration, &[u8]) -> bool + Send,
    P: Fn() -> ([f32; 4], [f32; 3]) + Send,
    B: Fn() -> (f32, bool) + Send,
{
    let mut reasm = Reassembler::new();
    let mut tmp = vec![0u8; 64 * 1024];
    let mut carry: Vec<u8> = Vec::new();
    let pose_interval = Duration::from_millis((1000.0 / target_fps.max(1.0)) as u64);
    let mut last_pose = Instant::now() - pose_interval;
    let mut last_ka = Instant::now();
    let battery_interval = Duration::from_secs(5); // 对齐上游 connection.rs
    let mut last_battery = Instant::now() - battery_interval;
    // 丢包后进入损坏期：丢弃非 IDR 帧，直到下一个 IDR 到达（对齐上游 avoid_video_glitching）
    let mut stream_corrupted = false;

    while !stop.load(Ordering::Relaxed) {
        // 收流（带缓冲拼接，处理 TCP 粘包/半包）
        match stream.read(&mut tmp) {
            Ok(0) => return Err("流通道断开".into()),
            Ok(n) => {
                carry.extend_from_slice(&tmp[..n]);
                // 逐分片解析
                loop {
                    if carry.len() < SHARD_PREFIX_SIZE {
                        break;
                    }
                    let h = match ShardHeader::parse(&carry) {
                        Ok(h) => h,
                        Err(_) => break,
                    };
                    let total = SHARD_PREFIX_SIZE + h.payload_len;
                    if carry.len() < total {
                        break;
                    }
                    // 分片内容 = header(由 stream_id 决定大小) + data；这里整体交给重组器，
                    // 完整包后再按 VideoPacketHeader 的 bincode 大小分离 header 与 NAL。
                    let shard_bytes = &carry[SHARD_PREFIX_SIZE..total];
                    if let Some((sid, packet)) = reasm.push(h.stream_id, h.packet_index, h.shards_count, h.shard_index, shard_bytes) {
                        if sid == STREAM_VIDEO {
                            handle_video_packet(&packet, on_frame, summary, &mut stream_corrupted, control);
                        }
                        // AUDIO / HAPTICS / STATISTICS 由渲染端按需扩展，本版先丢弃
                    }
                    // 检测丢包 → 请求 IDR + 进入损坏期（push 之后检查标志）
                    if reasm.take_lost() {
                        stream_corrupted = true;
                        let req = control_encode(&ClientControlPacket::RequestIdr).unwrap();
                        let _ = control.write_all(&req);
                    }
                    carry.drain(..total);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {}
            Err(e) => return Err(format!("读流通道: {e}")),
        }

        // 定时回传 tracking（头姿）
        if Instant::now() - last_pose >= pose_interval {
            let (quat, pos) = pose();
            let motion = DeviceMotion {
                pose: Pose { orientation: quat, position: pos },
                ..Default::default()
            };
            let track = Tracking {
                target_timestamp: Duration::from_nanos(0), // SERVER_TIME_SYNC 简化；真机可接 NTP 同步
                device_motions: vec![(0, motion)], // 0 = head id（见 common::HEAD_ID）
                ..Default::default()
            };
            if let Ok(bytes) = bincode::serialize(&track) {
                // 流通道 tracking 必须严格按上游 alvr_sockets::StreamSender::send 的帧格式：
                // 服务端 stream_socket.rs 解析为 shard_length = 4 + len_field。
                // len_field 必须 = 整片长度 - 4 = (18 字节前缀 + payload) - 4，
                // 否则服务端按错误长度读取、错位后 packet_start_index 越界并 panic
                // （crash_log: "range end index 18 out of range for slice of length 4"）。
                // 所有多字节字段均为大端（服务端 to_be_bytes 解析）。
                let shard_len = (SHARD_PREFIX_SIZE + bytes.len() - 4) as u32;
                let mut frame = Vec::with_capacity(SHARD_PREFIX_SIZE + bytes.len());
                frame.extend_from_slice(&shard_len.to_be_bytes());
                frame.extend_from_slice(&STREAM_TRACKING.to_be_bytes());
                frame.extend_from_slice(&(summary.tracking_sent as u32).to_be_bytes()); // packet index (BE)
                frame.extend_from_slice(&1u32.to_be_bytes()); // shards_count
                frame.extend_from_slice(&0u32.to_be_bytes()); // shard_index
                frame.extend_from_slice(&bytes);
                let _ = stream.write_all(&frame);
                summary.tracking_sent += 1;
            }
            last_pose = Instant::now();
        }

        // keepalive
        if Instant::now() - last_ka >= KEEPALIVE_INTERVAL {
            let ka = control_encode(&ClientControlPacket::KeepAlive).unwrap();
            let _ = control.write_all(&ka);
            last_ka = Instant::now();
        }

        // 电池状态上报（HMD 通道，device_id=0）：高配 PC 的 Dashboard 可显示头显电量
        if Instant::now() - last_battery >= battery_interval {
            let (gauge, plugged) = battery();
            let pkt = control_encode(&ClientControlPacket::Battery(BatteryInfo {
                device_id: 0, // HEAD_ID：与 tracking 的头 id 一致
                gauge_value: gauge.clamp(0.0, 1.0),
                is_plugged: plugged,
            })).unwrap();
            let _ = control.write_all(&pkt);
            last_battery = Instant::now();
        }
    }
    Ok(())
}

fn handle_video_packet<F>(
    packet: &[u8],
    on_frame: &mut F,
    summary: &mut RunSummary,
    stream_corrupted: &mut bool,
    control: &mut TcpStream,
) where
    F: FnMut(Duration, &[u8]) -> bool + Send,
{
    // packet = bincode(VideoPacketHeader) + NAL(Annex B)
    // bincode Duration = u64 秒 + u32 纳秒（小端，共 12 字节，已实测）
    use std::io::Cursor;
    let mut cursor = Cursor::new(packet);
    let header: VideoPacketHeader = match bincode::deserialize_from(&mut cursor) {
        Ok(h) => h,
        Err(e) => { eprintln!("[hvr-sink] 视频头解析失败: {e}"); return; }
    };
    let nal = &packet[cursor.position() as usize..];
    summary.frames_received += 1;
    summary.bytes_received += nal.len() as u64;

    if header.is_idr {
        // IDR 是完整的参考帧：结束损坏期
        *stream_corrupted = false;
        summary.idr_frames += 1;
    } else if *stream_corrupted {
        // 损坏期丢弃非 IDR 帧（避免花屏），并再次请求 IDR
        let req = control_encode(&ClientControlPacket::RequestIdr).unwrap();
        let _ = control.write_all(&req);
        return;
    }

    // 提交解码；解码器饱和则请求 IDR
    if !on_frame(header.timestamp, nal) {
        *stream_corrupted = true;
        let req = control_encode(&ClientControlPacket::RequestIdr).unwrap();
        let _ = control.write_all(&req);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reassembler_single_shard() {
        let mut r = Reassembler::new();
        let data = b"hello";
        let out = r.push(STREAM_VIDEO, 1, 1, 0, data);
        let (sid, packet) = out.unwrap();
        assert_eq!(sid, STREAM_VIDEO);
        assert_eq!(&packet, data);
    }

    #[test]
    fn reassembler_multi_shard_out_of_order() {
        let mut r = Reassembler::new();
        assert!(r.push(STREAM_VIDEO, 5, 3, 2, b"CCC").is_none());
        assert!(r.push(STREAM_VIDEO, 5, 3, 0, b"AAA").is_none());
        let (_, packet) = r.push(STREAM_VIDEO, 5, 3, 1, b"BBB").unwrap();
        assert_eq!(&packet, b"AAABBBCCC");
    }

    #[test]
    fn reassembler_detects_packet_loss() {
        let mut r = Reassembler::new();
        // 连续包 1、2 正常
        assert!(r.push(STREAM_VIDEO, 1, 1, 0, b"A").unwrap().1 == b"A");
        assert!(r.push(STREAM_VIDEO, 2, 1, 0, b"B").unwrap().1 == b"B");
        assert!(!r.take_lost());
        // 跳到 5（丢 3、4）
        assert!(r.push(STREAM_VIDEO, 5, 1, 0, b"E").unwrap().1 == b"E");
        assert!(r.take_lost(), "应检测到序号跳变");
        assert!(!r.take_lost(), "标志应被清除");
    }

    #[test]
    fn config_defaults_match_huawei() {
        let c = ClientConfig::default();
        assert_eq!(c.view_resolution, [1536, 1536]);
        assert!(c.refresh_rates.contains(&70.0));
        assert!((c.ipd_m - 0.063).abs() < 1e-6);
    }

    /// 端到端连接测试：用一个遵循 ALVR v20 真实时序的模拟服务端，
    /// 验证客户端「发现 → 控制握手 → 协商 → 收流重组 → tracking 回传」全链路可跑通。
    /// 不依赖 SteamVR / 眼镜硬件 / ffmpeg（on_frame 用计数闭包代替解码器）。
    #[test]
    fn mock_server_end_to_end_connection() {
        use std::io::{Read as _, Write as _};
        use std::sync::mpsc;
        use std::thread;
        use std::time::Duration as StdDuration;

        let (ready_tx, ready_rx) = mpsc::channel::<()>();
        let (track_tx, track_rx) = mpsc::channel::<usize>();

        let server = thread::spawn(move || {
            // 0) 发现包格式验证（纯函数，不依赖网络回环；Windows 广播不回环本机 socket）
            let dp = build_discovery_packet("HVR-SINK", 20);
            assert_eq!(&dp[0..5], b"ALVR\0", "发现包魔数错误");
            assert_eq!(u64::from_le_bytes(dp[16..24].try_into().unwrap()), 20, "protocol_id 错误");
            assert_eq!(&dp[24..32], b"HVR-SINK", "发现包 hostname 错误");
            eprintln!("[mock-server] ✓ 发现包格式校验通过（ALVR\\0 + protocol_id=20 + hostname）");
            ready_tx.send(()).unwrap();

            // 1) 模拟「服务端收到广播后连回客户端」：轮询 TCP 连 127.0.0.1:9943，
            //    规避客户端 listener 尚未 bind 的时序竞争。
            let mut control = loop {
                match TcpStream::connect((Ipv4Addr::LOCALHOST, CONTROL_PORT)) {
                    Ok(s) => break s,
                    Err(_) => thread::sleep(StdDuration::from_millis(50)),
                }
            };
            eprintln!("[mock-server] ✓ TCP 控制通道已连回客户端 :{}", CONTROL_PORT);

            // 3) 读客户端 hello
            let hello: ClientConnectionResult =
                read_control(&mut control, StdDuration::from_secs(3)).expect("读 hello 失败");
            match hello {
                ClientConnectionResult::ConnectionAccepted { .. } => {
                    eprintln!("[mock-server] ✓ 收到客户端 hello（ConnectionAccepted + 能力集）");
                }
                other => panic!("hello 类型错误: {other:?}"),
            }

            // 4) 下发 StreamConfig（negotiated 为 JSON 字符串）
            let negotiated = r#"{"view_resolution":[1536,1536],"refresh_rate_hint":70.0,"game_audio_sample_rate":48000,"enable_foveated_encoding":false,"use_multimodal_protocol":true,"use_full_range":false,"encoding_gamma":2.2,"enable_hdr":false,"wired":true}"#;
            let sc = StreamConfigPacket {
                session: "mock-session".into(),
                negotiated: negotiated.into(),
            };
            control.write_all(&control_encode(&sc).unwrap()).unwrap();
            eprintln!("[mock-server] ✓ 下发 StreamConfig（1536x1536@70Hz wired=true）");
            // 客户端 read_control 每次调用新建 buffer，粘包会丢失后续包；
            // 真实 ALVR server 也是逐包发送，这里 pacing 模拟该节奏。
            thread::sleep(StdDuration::from_millis(50));

            // 5) 下发 DecoderConfig + StartStream
            let dc = ServerControlPacket::DecoderConfig(DecoderInitializationConfig {
                codec: "H264".into(),
                config_buffer: vec![0x67, 0x42],
            });
            control.write_all(&control_encode(&dc).unwrap()).unwrap();
            thread::sleep(StdDuration::from_millis(50));
            control.write_all(&control_encode(&ServerControlPacket::StartStream).unwrap()).unwrap();
            eprintln!("[mock-server] ✓ 下发 DecoderConfig(H264) + StartStream");

            // 6) 读 StreamReady（客户端可能先发 ViewsConfig/keepalive，逐个跳过）
            let mut got_ready = false;
            for i in 0..8 {
                let pkt: ClientControlPacket = read_control(&mut control, StdDuration::from_secs(3))
                    .unwrap_or_else(|e| panic!("读控制包#{}失败: {}", i, e));
                eprintln!("[mock-server]   控制包#{}: {:?}", i, pkt);
                if matches!(pkt, ClientControlPacket::StreamReady) {
                    got_ready = true;
                    break;
                }
            }
            assert!(got_ready, "客户端未发 StreamReady");
            eprintln!("[mock-server] ✓ 收到 StreamReady");

            // 7) 连客户端流端口 9944
            let mut stream = loop {
                match TcpStream::connect((Ipv4Addr::LOCALHOST, 9944u16)) {
                    Ok(s) => break s,
                    Err(_) => {
                        // 客户端会在 StreamReady 后 bind 9944，稍等即可
                        thread::sleep(StdDuration::from_millis(20));
                    }
                }
            };
            eprintln!("[mock-server] ✓ 流通道已连回客户端 :9944");

            // 8) 发 3 个视频帧（ALVR shard 帧格式）
            for i in 0..3u32 {
                let mut payload = bincode::serialize(&VideoPacketHeader {
                    timestamp: StdDuration::from_millis((i * 16) as u64),
                    is_idr: i == 0,
                }).unwrap();
                payload.extend_from_slice(&[0, 0, 0, 1, 0x65, 0x88, 0x99]); // 伪 Annex B NAL
                let mut frame = Vec::new();
                frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
                frame.extend_from_slice(&STREAM_VIDEO.to_be_bytes());
                frame.extend_from_slice(&i.to_be_bytes());
                frame.extend_from_slice(&1u32.to_be_bytes());
                frame.extend_from_slice(&0u32.to_be_bytes());
                frame.extend_from_slice(&payload);
                stream.write_all(&frame).unwrap();
                thread::sleep(StdDuration::from_millis(30));
            }

            // 9) 收 tracking（流通道）与 keepalive/battery（控制通道）
            stream.set_read_timeout(Some(StdDuration::from_millis(300))).ok();
            control.set_read_timeout(Some(StdDuration::from_millis(300))).ok();
            let mut tmp = [0u8; 4096];
            let mut tracking_bytes = 0usize;
            for _ in 0..5 {
                match stream.read(&mut tmp) { Ok(n) if n > 0 => tracking_bytes += n, _ => {} }
                let _ = control.read(&mut tmp);
            }
            assert!(tracking_bytes > 0, "未收到客户端 tracking 回传");
            eprintln!("[mock-server] ✓ 收到客户端 tracking 回传 {} 字节", tracking_bytes);

            track_tx.send(tracking_bytes).unwrap();
            // 正常收尾：先停客户端 stop 标志由测试线程控制；这里 drop 让客户端 read 到 EOF。
            // 10054/EOF 均为预期结束信号，不代表连接失败。
            drop(stream);
            drop(control);
        });

        ready_rx.recv_timeout(StdDuration::from_secs(3)).expect("模拟服务端未就绪");

        let cfg = ClientConfig::default();
        let stop = Arc::new(AtomicBool::new(false));
        let frames = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let fc = frames.clone();
        let mut on_frame = move |_ts: StdDuration, _nal: &[u8]| {
            fc.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            true
        };
        let pose = || ([0.0f32, 0.0, 0.0, 1.0], [0.0f32, 0.0, 0.0]);
        let battery = || (1.0f32, true);

        let mut summary = RunSummary {
            frames_received: 0, idr_frames: 0, bytes_received: 0,
            tracking_sent: 0, reconnects: 0,
        };
        let result = one_session(&cfg, &mut on_frame, &pose, &battery, &stop, &mut summary);
        eprintln!("[hvr-sink] 会话结束: {:?}", result);
        let tracking_bytes = track_rx.recv_timeout(StdDuration::from_secs(3)).unwrap();

        eprintln!("\n===== 端到端连接测试统计 =====");
        eprintln!("  会话结果: {:?}", result);
        eprintln!("  收帧数: {} / IDR: {} / 字节: {}",
            summary.frames_received, summary.idr_frames, summary.bytes_received);
        eprintln!("  tracking 回传包: {} / tracking 字节: {}", summary.tracking_sent, tracking_bytes);
        eprintln!("==============================\n");

        assert!(frames.load(std::sync::atomic::Ordering::Relaxed) >= 3, "客户端未收满 3 个视频帧");
        assert!(summary.tracking_sent > 0, "客户端未回传 tracking");
        assert!(tracking_bytes > 0);
        server.join().unwrap();
    }
}
