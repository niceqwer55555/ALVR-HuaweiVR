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
    /// 可选：已知服务端 IP。设置后除了广播发现，还直接向此 IP 单播发现包
    /// （应对手机热点/路由器 AP 隔离导致广播不转发的情况）
    pub server_ip: Option<Ipv4Addr>,
    /// auto / tcp / udp；auto 时按服务端 session 的 connection.stream_protocol 协商
    pub stream_protocol: String,
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
            server_ip: None,
            stream_protocol: "auto".into(),
            // 上游 alvr_common::protocol_id_u64() = hash_string("20")（Rust DefaultHasher/SipHash），
            // 不是字面量 20。发错值服务端会 warn「incompatible client」，华为版虽不拒绝但应如实上报。
            protocol_id_u64: 1_686_174_404_061_257_063,
            target_fps: 70.0,
            ipd_m: 0.063,
            // AlvrFov 是「半角弧度」，服务端会再做 tanf()：
            // 95° 每眼 → 半角 47.5° = 0.829 rad，tan(47.5°)=1.0913（渲染器 ini 用后者）。
            fov: [
                Fov { left: -0.829, right: 0.829, up: 0.829, down: -0.829 },
                Fov { left: -0.829, right: 0.829, up: 0.829, down: -0.829 },
            ],
            stereo_layout: 0,
        }
    }
}

/// StreamConfigPacket.session 是 SessionConfig 的 JSON；只取流通道相关字段。
/// 结构：{"session_settings":{"connection":{"stream_protocol":{"variant":"Tcp"},
/// "stream_port":9944}}}
#[derive(Debug, Clone)]
struct SessionInfo {
    stream_protocol: String,
    stream_port: Option<u16>,
}

impl SessionInfo {
    fn parse(s: &str) -> Self {
        let v: serde_json::Value = serde_json::from_str(s).unwrap_or(serde_json::Value::Null);
        let conn = v.pointer("/session_settings/connection");
        let stream_protocol = conn
            .and_then(|c| c.pointer("/stream_protocol/variant"))
            .and_then(|x| x.as_str())
            .or_else(|| conn.and_then(|c| c.pointer("/stream_protocol")).and_then(|x| x.as_str()))
            .unwrap_or("Tcp")
            .to_string();
        let stream_port = conn
            .and_then(|c| c.get("stream_port"))
            .and_then(|x| x.as_u64())
            .map(|x| x as u16);
        Self { stream_protocol, stream_port }
    }
}

/// 流通道抽象：TCP 面向字节流，UDP 面向分片数据报；上层解析逻辑一致。
enum StreamChannel {
    Tcp(TcpStream),
    // [HVR fix] 用 recv_from/send_to 而非 connect/send：服务端可能从与
    // server_ip:stream_port 不同的源地址发流（多网卡/NAT），connect 会过滤掉
    // 这些包导致永远收不到视频。记录真实对端，回传 tracking 发往同一地址。
    Udp { sock: UdpSocket, peer: Option<SocketAddr>, fallback: SocketAddr },
}

impl StreamChannel {
    fn set_read_timeout(&self, timeout: Duration) -> std::io::Result<()> {
        match self {
            Self::Tcp(s) => s.set_read_timeout(Some(timeout)),
            Self::Udp { sock, .. } => sock.set_read_timeout(Some(timeout)),
        }
    }

    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Tcp(s) => s.read(buf),
            Self::Udp { sock, peer, .. } => {
                let (n, src) = sock.recv_from(buf)?;
                *peer = Some(src);
                Ok(n)
            }
        }
    }

    fn write_all(&mut self, data: &[u8]) -> std::io::Result<()> {
        match self {
            Self::Tcp(s) => s.write_all(data),
            Self::Udp { sock, peer, fallback } => {
                let dst = peer.unwrap_or(*fallback);
                let n = sock.send_to(data, dst)?;
                if n != data.len() {
                    Err(std::io::Error::new(std::io::ErrorKind::WriteZero, "UDP 分片未完整发送"))
                } else {
                    Ok(())
                }
            }
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
pub fn run<F, P, B, H>(
    cfg: &ClientConfig,
    mut on_frame: F,
    pose: P,
    battery: B,
    hand_pose: H,
    stop: Arc<AtomicBool>,
) -> RunSummary
where
    F: FnMut(Duration, &[u8]) -> bool + Send,
    P: Fn() -> ([f32; 4], [f32; 3]) + Send,
    B: Fn() -> (f32, bool) + Send,
    H: Fn() -> Option<([f32;4], [f32;3], [f32;4], [f32;3])> + Send + Sync,
{
    let mut summary = RunSummary {
        frames_received: 0,
        idr_frames: 0,
        bytes_received: 0,
        tracking_sent: 0,
        reconnects: 0,
    };

    while !stop.load(Ordering::Relaxed) {
        let mut backoff = Duration::from_secs(1);
        match one_session(cfg, &mut on_frame, &pose, &battery, &hand_pose, &stop, &mut summary) {
            Ok(()) => {}
            Err(e) => {
                status::set_alvr_connected(false);
                eprintln!("[hvr-sink] 会话结束: {e}");
                // [HVR fix] 服务端发 Restarting / 主动断开后，它需要时间重新初始化
                // 编码器与流通道。若客户端 1 秒就重连，会撞上服务端尚未就绪的窗口，
                // 表现为双方陷入「连上→立刻断→再连」的反复连接循环。
                // 对这些「服务端侧重启/断开」的错误退避更久，与服务端节奏对齐。
                if e.contains("服务端重启") || e.contains("强迫关闭")
                    || e.contains("远程主机") || e.contains("connection reset")
                    || e.contains("连接被重置") || e.contains("Broken pipe") {
                    backoff = Duration::from_secs(5);
                }
            }
        }
        if stop.load(Ordering::Relaxed) {
            status::set_alvr_connected(false);
            break;
        }
        summary.reconnects += 1;
        std::thread::sleep(backoff);
    }
    summary
}

/// 按上游 encode_video_streaming_capabilities 的规则：
/// 正数 refresh rate 照常放在前面，完整能力 JSON 的每个字节以负 f32 追加。
/// 不追加 JSON 时服务端会按 Null 解码，默认 encoder_av1/10bit/foveated=true，
/// 与实际只支持 H.264 8-bit 的解码器不符。
fn build_capabilities(cfg: &ClientConfig) -> VideoStreamingCapabilitiesLegacy {
    let caps_json = serde_json::json!({
        "default_view_resolution": cfg.view_resolution,
        "supported_refresh_rates": cfg.refresh_rates,
        "microphone_sample_rate": cfg.microphone_sample_rate,
        "supports_foveated_encoding": false,
        "encoder_high_profile": true,
        "encoder_10_bits": false,
        "encoder_av1": false,
        "multimodal_protocol": false,
        "prefer_10bit": false,
        "prefer_full_range": true,
        "preferred_encoding_gamma": 2.2,
        "prefer_hdr": false
    });
    let mut rates = cfg.refresh_rates.clone();
    for b in serde_json::to_string(&caps_json).unwrap_or_default().bytes() {
        rates.push(-(b as f32));
    }
    VideoStreamingCapabilitiesLegacy {
        default_view_resolution: cfg.view_resolution,
        supported_refresh_rates_plus_extra_data: rates,
        microphone_sample_rate: cfg.microphone_sample_rate,
    }
}

fn one_session<F, P, B, H>(
    cfg: &ClientConfig,
    on_frame: &mut F,
    pose: &P,
    battery: &B,
    hand_pose: &H,
    stop: &Arc<AtomicBool>,
    summary: &mut RunSummary,
) -> Result<(), String>
where
    F: FnMut(Duration, &[u8]) -> bool + Send,
    P: Fn() -> ([f32; 4], [f32; 3]) + Send,
    B: Fn() -> (f32, bool) + Send,
    H: Fn() -> Option<([f32;4], [f32;3], [f32;4], [f32;3])> + Send + Sync,
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
        // [HVR fix] 同时向已知服务端 IP 单播发现包。AP 隔离下广播不跨客户端转发，
        // 但服务端→客户端方向的 TCP 是通的，单播能触达服务端触发它回连。
        if let Some(sip) = cfg.server_ip {
            discovery.send_to(&pkt, (sip, CONTROL_PORT)).ok();
        }
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

    // 3) 发送 ConnectionAccepted（含完整能力集 JSON，见 build_capabilities）
    let hello = ClientConnectionResult::ConnectionAccepted {
        client_protocol_id: cfg.protocol_id_u64,
        display_name: cfg.hostname.clone(),
        server_ip,
        streaming_capabilities: Some(build_capabilities(cfg)),
    };
    control
        .write_all(&control_encode(&hello).map_err(|e| format!("hello 序列化失败: {e}"))?)
        .map_err(|e| format!("发送 hello 失败: {e}"))?;

    // 4) 收 StreamConfigPacket。ctl_buf 必须跨调用保留，否则粘包会吞掉 StartStream。
    let mut ctl_buf: Vec<u8> = Vec::with_capacity(4096);
    let config = read_control::<StreamConfigPacket>(&mut control, &mut ctl_buf, HANDSHAKE_TIMEOUT)?;
    let negotiated = NegotiatedConfig::from_json(&config.negotiated)
        .map_err(|e| format!("解析协商配置失败: {e}"))?;
    eprintln!(
        "[hvr-sink] 协商: {}x{} @ {:.1}Hz wired={}",
        negotiated.view_resolution[0],
        negotiated.view_resolution[1],
        negotiated.refresh_rate_hint,
        negotiated.wired
    );

    // 5) 等 StartStream / DecoderConfig（同一个 ctl_buf，保留后续粘包）
    let mut decoder_config: Option<DecoderInitializationConfig> = None;
    let mut started = false;
    for _ in 0..8 {
        let pkt = read_control::<ServerControlPacket>(&mut control, &mut ctl_buf, HANDSHAKE_TIMEOUT)?;
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
        eprintln!(
            "[hvr-sink] 解码器配置: codec={:?} config_buffer={}B",
            c.codec, c.config_buffer.len()
        );
        // [HVR fix] config_buffer 含 SPS/PPS（H.264 解码必需的序列/图像参数集）。
        // 服务端不会在视频 NAL 里重复发 SPS/PPS，必须先把配置帧喂给 ffmpeg，
        // 否则每帧都报 "non-existing PPS 0 referenced" → 解不出任何 BGRA 帧。
        if !c.config_buffer.is_empty() {
            eprintln!("[hvr-sink] 推送 SPS/PPS 配置帧到解码器 ({}B) head={:02X?}",
                c.config_buffer.len(), &c.config_buffer[..c.config_buffer.len().min(32)]);
            on_frame(std::time::Duration::ZERO, &c.config_buffer);
        }
    }

    // 6) 流通道协议/端口以服务端 session 为准（ALVR 默认 UDP，wired 才 TCP）
    let session_info = SessionInfo::parse(&config.session);
    let protocol = if cfg.stream_protocol.eq_ignore_ascii_case("auto") {
        if negotiated.wired {
            "tcp".to_string()
        } else {
            session_info.stream_protocol.clone()
        }
    } else {
        cfg.stream_protocol.clone()
    };
    let stream_port = session_info.stream_port.unwrap_or(cfg.stream_port);
    eprintln!(
        "[hvr-sink] 流通道: {} :{}（session={} wired={}）",
        protocol.to_uppercase(), stream_port, session_info.stream_protocol, negotiated.wired
    );

    let ready = control_encode(&ClientControlPacket::StreamReady)
        .map_err(|e| format!("StreamReady 序列化: {e}"))?;

    // 7) 按协议建流通道，再发 StreamReady
    let mut stream: StreamChannel = if protocol.eq_ignore_ascii_case("udp") {
        let sock = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, stream_port))
            .map_err(|e| format!("绑定 UDP 流端口 {stream_port} 失败: {e}"))?;
        // [HVR fix] 不 connect，避免过滤掉来自服务端其它源地址的流包；
        // fallback 用作尚未收到任何包时 tracking 的回传目标。
        let fallback = SocketAddr::new(server_ip, stream_port);
        control.write_all(&ready).map_err(|e| format!("发送 StreamReady: {e}"))?;
        StreamChannel::Udp { sock, peer: None, fallback }
    } else {
        let stream_listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, stream_port))
            .map_err(|e| format!("绑定 TCP 流端口 {stream_port} 失败: {e}"))?;
        stream_listener.set_nonblocking(true).ok();
        control.write_all(&ready).map_err(|e| format!("发送 StreamReady: {e}"))?;

        // 8) 等 server 连 TCP 流通道
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut accepted: Option<TcpStream> = None;
        while Instant::now() < deadline && !stop.load(Ordering::Relaxed) {
            match stream_listener.accept() {
                Ok((s, addr)) => { eprintln!("[hvr-sink] 流通道已连接 {addr}"); accepted = Some(s); break; }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => return Err(format!("流 accept 失败: {e}")),
            }
        }
        StreamChannel::Tcp(accepted.ok_or("流通道连接超时（TCP）")?)
    };
    stream.set_read_timeout(Duration::from_millis(500)).ok();

    // 9) 发送 ViewsConfig（IPD + FOV），让 server 正确渲染双目
    let views = control_encode(&ClientControlPacket::ViewsConfig(ViewsConfig {
        ipd_m: cfg.ipd_m,
        fov: cfg.fov.clone(),
    })).map_err(|e| format!("ViewsConfig: {e}"))?;
    control.write_all(&views).ok();

    // 10) 后台持续读控制通道：DecoderConfig / Restarting 不能一直堆在 TCP 缓冲里
    // [HVR fix] 服务端只在收到 RequestIdr 时才下发 DecoderConfig(含 SPS/PPS)，且晚于握手。
    // 用共享槽把 config_buffer 从后台控制线程传给主循环，再喂给解码器。
    let config_slot: Arc<std::sync::Mutex<Option<Vec<u8>>>> = Arc::new(std::sync::Mutex::new(None));
    let config_slot_bg = Arc::clone(&config_slot);
    // 主动请求 IDR，触发服务端尽快下发 SPS/PPS
    let req_idr = control_encode(&ClientControlPacket::RequestIdr).unwrap();
    control.write_all(&req_idr).ok();
    let mut ctrl_read = control.try_clone().map_err(|e| format!("克隆控制通道: {e}"))?;
    ctrl_read.set_read_timeout(Some(Duration::from_millis(500))).ok();
    let ctrl_stop = Arc::new(AtomicBool::new(false));
    let ctrl_stop_thread = Arc::clone(&ctrl_stop);
    std::thread::spawn(move || {
        let mut buf = ctl_buf;
        while !ctrl_stop_thread.load(Ordering::Relaxed) {
            match read_control::<ServerControlPacket>(&mut ctrl_read, &mut buf, Duration::from_millis(500)) {
                Ok(ServerControlPacket::DecoderConfig(c)) => {
                    eprintln!(
                        "[hvr-sink] 服务端 DecoderConfig: codec={:?} config_buffer={}B",
                        c.codec, c.config_buffer.len()
                    );
                    if !c.config_buffer.is_empty() {
                        if let Ok(mut slot) = config_slot_bg.lock() { *slot = Some(c.config_buffer); }
                    }
                }
                Ok(ServerControlPacket::Restarting) => {
                    eprintln!("[hvr-sink] 服务端要求重启串流");
                    break;
                }
                Ok(_) => {}
                Err(e) if e == "读取控制包超时" => {}
                Err(_) => break,
            }
        }
    });

    // 11) 主循环：收视频分片重组 + 定时回传 tracking + keepalive
    let result = stream_video_and_track(
        &mut stream,
        &mut control,
        on_frame,
        pose,
        battery,
        &hand_pose,
        stop,
        summary,
        cfg.target_fps,
        config_slot,
    );
    ctrl_stop.store(true, Ordering::Relaxed);
    result
}

/// 从控制通道读一帧指定类型
fn read_control<T: serde::de::DeserializeOwned>(
    s: &mut TcpStream,
    carry: &mut Vec<u8>,
    timeout: Duration,
) -> Result<T, String> {
    let deadline = Instant::now() + timeout;
    let mut tmp = [0u8; 8192];
    loop {
        if carry.len() >= 4 {
            let len = u32::from_be_bytes([carry[0], carry[1], carry[2], carry[3]]) as usize;
            if len > 16 * 1024 * 1024 {
                return Err(format!("控制包长度异常: {len}"));
            }
            if carry.len() >= 4 + len {
                let (pkt, used) = control_decode::<T>(&carry)
                    .map_err(|e| format!("控制包解析失败: {e:?}"))?
                    .ok_or("控制包不完整")?;
                // 关键：只消费当前包，保留同一次 read 里粘着的后续包
                carry.drain(..used);
                return Ok(pkt);
            }
        }
        if Instant::now() > deadline {
            return Err("读取控制包超时".into());
        }
        match s.read(&mut tmp) {
            Ok(0) => return Err("控制通道断开".into()),
            Ok(n) => carry.extend_from_slice(&tmp[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(format!("读控制通道: {e}")),
        }
    }
}

/// 单个流的分片重组状态。ALVR 的音频/视频/统计分片可以在同一 socket 里交错，
/// 必须按 stream_id 分桶，不能用一个全局 current_index。
struct StreamReassembly {
    current_index: u32,
    last_complete_index: Option<u32>,
    parts: Vec<Vec<u8>>,
    assembled: Vec<u8>,
    lost: bool,
}

impl Default for StreamReassembly {
    fn default() -> Self {
        Self {
            current_index: u32::MAX,
            last_complete_index: None,
            parts: vec![],
            assembled: Vec::with_capacity(2 * 1024 * 1024),
            lost: false,
        }
    }
}

impl StreamReassembly {
    fn push(&mut self, idx: u32, shards: usize, shard: usize, bytes: &[u8]) -> Option<Vec<u8>> {
        if shards == 0 || shard >= shards {
            return None;
        }
        if idx != self.current_index {
            if let Some(last) = self.last_complete_index {
                if idx != last.wrapping_add(1) {
                    self.lost = true;
                }
            }
            self.current_index = idx;
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
            Some(std::mem::take(&mut self.assembled))
        } else {
            None
        }
    }
}

/// 按 stream_id 分桶的重组器
struct Reassembler {
    streams: std::collections::HashMap<u16, StreamReassembly>,
}

impl Reassembler {
    fn new() -> Self {
        Self { streams: std::collections::HashMap::new() }
    }

    /// 收一个分片；完整时返回重组后的 (stream_id, 完整包字节)。
    fn push(&mut self, stream_id: u16, idx: u32, shards: usize, shard: usize, bytes: &[u8]) -> Option<(u16, Vec<u8>)> {
        let state = self.streams.entry(stream_id).or_default();
        state.push(idx, shards, shard, bytes).map(|p| (stream_id, p))
    }

    /// 取出并清除所有流的丢包标志；只应对视频流触发 IDR。
    fn take_lost(&mut self, stream_id: u16) -> bool {
        self.streams
            .get_mut(&stream_id)
            .map(|s| std::mem::replace(&mut s.lost, false))
            .unwrap_or(false)
    }
}

fn stream_video_and_track<F, P, B, H>(
    stream: &mut StreamChannel,
    control: &mut TcpStream,
    on_frame: &mut F,
    pose: &P,
    battery: &B,
    hand_pose: &H,
    stop: &Arc<AtomicBool>,
    summary: &mut RunSummary,
    target_fps: f32,
    config_slot: Arc<std::sync::Mutex<Option<Vec<u8>>>>,
) -> Result<(), String>
where
    F: FnMut(Duration, &[u8]) -> bool + Send,
    P: Fn() -> ([f32; 4], [f32; 3]) + Send,
    B: Fn() -> (f32, bool) + Send,
    H: Fn() -> Option<([f32;4], [f32;3], [f32;4], [f32;3])> + Send + Sync,
{
    let mut reasm = Reassembler::new();
    let session_start = Instant::now();
    let mut tracking_index: u32 = 0;
    let mut tmp = vec![0u8; 64 * 1024];
    let mut carry: Vec<u8> = Vec::new();
    let pose_interval = Duration::from_millis((1000.0 / target_fps.max(1.0)) as u64);
    let mut last_pose = Instant::now() - pose_interval;
    let mut last_ka = Instant::now();
    let battery_interval = Duration::from_secs(5); // 对齐上游 connection.rs
    let mut last_battery = Instant::now() - battery_interval;
    // 丢包后进入损坏期：丢弃非 IDR 帧，直到下一个 IDR 到达（对齐上游 avoid_video_glitching）
    let mut stream_corrupted = false;

    // [HVR fix] 视频流超时检测：若服务端连着控制通道却迟迟不发视频（会话假死），
    // 旧逻辑会在 stream.read 超时的 WouldBlock 分支永远空转，客户端再也不会重新发现。
    // 这里记录最后一个视频包时间，超过阈值主动结束会话触发重连。
    // 给 15 秒启动宽限期：服务端初始化编码器/等 SteamVR 首帧可能要几秒。
    const VIDEO_STALL_TIMEOUT: Duration = Duration::from_secs(15);
    let mut last_video = Instant::now();
    let mut warned_stall = false;

    let mut config_pushed = false;
    while !stop.load(Ordering::Relaxed) {
        // 会话建立超过宽限期后，若一直没有任何视频包则判定假死，重连
        if session_start.elapsed() > VIDEO_STALL_TIMEOUT
            && summary.frames_received == 0
            && Instant::now() - last_video > VIDEO_STALL_TIMEOUT {
            return Err(format!("视频流超时 {}s 无数据，主动重连", VIDEO_STALL_TIMEOUT.as_secs()));
        }
        if !warned_stall && session_start.elapsed() > VIDEO_STALL_TIMEOUT / 2
            && summary.frames_received == 0 {
            eprintln!("[hvr-sink] 警告：流通道已建但 {}s 无视频包，服务端可能未出帧",
                (VIDEO_STALL_TIMEOUT / 2).as_secs());
            warned_stall = true;
        }
        // [HVR fix] 后台控制线程收到 DecoderConfig(SPS/PPS) 后，推给解码器一次
        if !config_pushed {
            if let Ok(mut slot) = config_slot.lock() {
                if let Some(cfgbuf) = slot.take() {
                    eprintln!("[hvr-sink] 推送服务端 SPS/PPS 到解码器 ({}B)", cfgbuf.len());
                    on_frame(std::time::Duration::ZERO, &cfgbuf);
                    config_pushed = true;
                }
            }
        }
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
                    let total = h.total_len();
                    if carry.len() < total {
                        break;
                    }
                    // 分片内容 = header(由 stream_id 决定大小) + data；这里整体交给重组器，
                    // 完整包后再按 VideoPacketHeader 的 bincode 大小分离 header 与 NAL。
                    let shard_bytes = &carry[SHARD_PREFIX_SIZE..total];
                    if let Some((sid, packet)) = reasm.push(h.stream_id, h.packet_index, h.shards_count, h.shard_index, shard_bytes) {
                        if sid == STREAM_VIDEO {
                            last_video = Instant::now();
                            handle_video_packet(&packet, on_frame, summary, &mut stream_corrupted, control);
                        }
                        // AUDIO / HAPTICS / STATISTICS 由渲染端按需扩展，本版先丢弃
                    }
                    // 检测丢包 → 请求 IDR + 进入损坏期（push 之后检查标志）
                    if reasm.take_lost(STREAM_VIDEO) {
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
            let head_motion = DeviceMotion {
                pose: Pose { orientation: quat, position: pos },
                ..Default::default()
            };
            // [HVR] 手柄姿态：优先用 slam_bridge 从华为 VR6DOF.dll 读到的真实 6DoF；
            // 无数据（桥接未启动/未收敛）时回退到"头显前方握持位"默认姿态，
            // 位置长度 > 0.05m 阈值让服务端判定为 6DoF。
            let (left_pose, right_pose) = if let Some((lq, lp, rq, rp)) = hand_pose() {
                (Pose { orientation: lq, position: lp }, Pose { orientation: rq, position: rp })
            } else {
                let fallback = |lateral: f32| {
                    let off = [lateral, -0.30f32, -0.35f32];
                    let rotate = |v: [f32;3]| {
                        let (x,y,z,w) = (quat[0],quat[1],quat[2],quat[3]);
                        let ix =  w*v[0] + y*v[2] - z*v[1];
                        let iy =  w*v[1] + z*v[0] - x*v[2];
                        let iz =  w*v[2] + x*v[1] - y*v[0];
                        let iw = -x*v[0] - y*v[1] - z*v[2];
                        [ ix*w + iw*(-x) + iy*(-z) - iz*(-y),
                          iy*w + iw*(-y) + iz*(-x) - ix*(-z),
                          iz*w + iw*(-z) + ix*(-y) - iy*(-x) ]
                    };
                    let p = rotate(off);
                    Pose { orientation: [1.0,0.0,0.0,0.0], position: [pos[0]+p[0], pos[1]+p[1], pos[2]+p[2]] }
                };
                (fallback(-0.22), fallback(0.22))
            };
            let left_motion = DeviceMotion { pose: left_pose, ..Default::default() };
            let right_motion = DeviceMotion { pose: right_pose, ..Default::default() };
            let track = Tracking {
                // 用会话开始以来的单调时间，服务端才能做姿态预测；固定 0 会让预测失效
                target_timestamp: session_start.elapsed(),
                device_motions: vec![
                    (HEAD_ID, head_motion),
                    (HAND_LEFT_ID, left_motion),
                    (HAND_RIGHT_ID, right_motion),
                ],
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
                frame.extend_from_slice(&tracking_index.to_be_bytes()); // packet index (BE)，每会话从 0 开始
                frame.extend_from_slice(&1u32.to_be_bytes()); // shards_count
                frame.extend_from_slice(&0u32.to_be_bytes()); // shard_index
                frame.extend_from_slice(&bytes);
                let _ = stream.write_all(&frame);
                tracking_index = tracking_index.wrapping_add(1);
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
                device_id: HEAD_ID, // 与 tracking 的头 id 一致（官方客户端也用 *HEAD_ID）
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
    if summary.frames_received == 1 || summary.frames_received % 300 == 0 {
        eprintln!("[hvr-sink] 视频包 #{} IDR={} nal={}B 累计={}B",
            summary.frames_received, header.is_idr, nal.len(), summary.bytes_received);
    }

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
    fn reassembler_keeps_streams_independent() {
        let mut r = Reassembler::new();
        // 视频包 7 的第一片
        assert!(r.push(STREAM_VIDEO, 7, 2, 0, b"VID-A").is_none());
        // 音频包 9 插进来，不能清掉视频进度
        assert!(r.push(STREAM_AUDIO, 9, 1, 0, b"AUDIO").is_some());
        // 视频包 7 的第二片，应能完整拼出
        let (sid, packet) = r.push(STREAM_VIDEO, 7, 2, 1, b"VID-B").expect("视频应完整");
        assert_eq!(sid, STREAM_VIDEO);
        assert_eq!(&packet, b"VID-AVID-B");
    }

    #[test]
    fn reassembler_detects_packet_loss() {
        let mut r = Reassembler::new();
        // 连续包 1、2 正常
        assert!(r.push(STREAM_VIDEO, 1, 1, 0, b"A").unwrap().1 == b"A");
        assert!(r.push(STREAM_VIDEO, 2, 1, 0, b"B").unwrap().1 == b"B");
        assert!(!r.take_lost(STREAM_VIDEO));
        // 跳到 5（丢 3、4）
        assert!(r.push(STREAM_VIDEO, 5, 1, 0, b"E").unwrap().1 == b"E");
        assert!(r.take_lost(STREAM_VIDEO), "应检测到序号跳变");
        assert!(!r.take_lost(STREAM_VIDEO), "标志应被清除");
    }

    #[test]
    fn config_defaults_match_huawei() {
        let c = ClientConfig::default();
        assert_eq!(c.view_resolution, [1536, 1536]);
        assert!(c.refresh_rates.contains(&70.0));
        assert!((c.ipd_m - 0.063).abs() < 1e-6);
        assert!((c.fov[0].left + 0.829).abs() < 1e-4);
        assert!((c.fov[0].right - 0.829).abs() < 1e-4);
        assert!((c.fov[1].down + 0.829).abs() < 1e-4);
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

            // 3) 读客户端 hello（mock 侧也保留 buffer，避免粘包）
            let mut mock_buf: Vec<u8> = Vec::new();
            let hello: ClientConnectionResult =
                read_control(&mut control, &mut mock_buf, StdDuration::from_secs(3)).expect("读 hello 失败");
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
            // 关键回归：三个控制包一次性写入，模拟真实 TCP 粘包；
            // 旧 read_control 只返回第一包并丢掉剩余，会卡在「未收到 StartStream」。
            let dc = ServerControlPacket::DecoderConfig(DecoderInitializationConfig {
                codec: CodecType::H264,
                config_buffer: vec![0x67, 0x42],
            });
            let mut burst = control_encode(&sc).unwrap();
            burst.extend_from_slice(&control_encode(&dc).unwrap());
            burst.extend_from_slice(&control_encode(&ServerControlPacket::StartStream).unwrap());
            control.write_all(&burst).unwrap();
            eprintln!("[mock-server] ✓ 一次性下发 StreamConfig + DecoderConfig(H264) + StartStream");

            // 6) 读 StreamReady（客户端可能先发 ViewsConfig/keepalive，逐个跳过）
            let mut got_ready = false;
            for i in 0..8 {
                let pkt: ClientControlPacket = read_control(&mut control, &mut mock_buf, StdDuration::from_secs(3))
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
                // 真实 ALVR server：长度字段 = 整片长度 - 4 = 18 + payload - 4
                frame.extend_from_slice(&((SHARD_PREFIX_SIZE + payload.len() - 4) as u32).to_be_bytes());
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
        let no_hand = || None;

        let mut summary = RunSummary {
            frames_received: 0, idr_frames: 0, bytes_received: 0,
            tracking_sent: 0, reconnects: 0,
        };
        let result = one_session(&cfg, &mut on_frame, &pose, &battery, &no_hand, &stop, &mut summary);
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
