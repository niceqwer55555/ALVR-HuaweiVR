//! hvr-alvr-win-sink：华为 VR Glass 有线串流接收端（Windows）
//!
//! 拓扑：高配 PC(ALVR server) ──局域网──> 低配 PC(本程序) ──回环──> SBS 渲染器 ──USB-C DP──> VR Glass
//!
//! 用法：
//!   hvr-alvr-win-sink.exe                # 自动发现 PC 串流端
//!   hvr-alvr-win-sink.exe --config cfg.json
//!   hvr-alvr-win-sink.exe --selftest     # 只跑协议自检，不连网络

mod protocol;
mod client;
mod decoder;
mod render_bridge;
mod status;

use client::{run, ClientConfig, RunSummary};
use decoder::{DecodeConfig, Decoder};
use protocol::Fov;
use render_bridge::{FrameOutput, PoseInput};
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn selftest() {
    println!("[hvr-sink] 协议自检");
    let p = protocol::build_discovery_packet("DESKTOP-TEST", 20);
    assert_eq!(&p[0..5], b"ALVR\0");
    assert_eq!(u64::from_le_bytes(p[16..24].try_into().unwrap()), 20);
    let bytes = protocol::control_encode(&protocol::ClientControlPacket::StreamReady).unwrap();
    let (pkt, used): (protocol::ClientControlPacket, usize) =
        protocol::control_decode(&bytes).unwrap().unwrap();
    assert!(matches!(pkt, protocol::ClientControlPacket::StreamReady));
    assert_eq!(used, bytes.len());
    let mut h = [0u8; protocol::SHARD_PREFIX_SIZE];
    h[4..6].copy_from_slice(&protocol::STREAM_VIDEO.to_be_bytes());
    assert_eq!(protocol::ShardHeader::parse(&h).unwrap().stream_id, protocol::STREAM_VIDEO);
    println!("[hvr-sink] 自检通过：发现包/控制帧/分片头格式正确");
}

/// 自动定位 ffmpeg.exe，供 GUI 双击启动时使用（GUI 不会设置 FFMPEG 环境变量）。
/// 优先级：FFMPEG 环境变量 > exe 同目录 > 当前目录 > 常见安装路径 > PATH（ffmpeg.exe）。
fn resolve_ffmpeg() -> String {
    // 1) 环境变量（命令行/脚本显式指定，最高优先）
    if let Ok(p) = std::env::var("FFMPEG") {
        if !p.is_empty() && std::path::Path::new(&p).exists() { return p; }
    }
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    // 2) exe 同目录（交付包自带 ffmpeg.exe 的情形）
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() { candidates.push(dir.join("ffmpeg.exe")); }
    }
    // 3) 当前工作目录（GUI 的工作目录就是 exe 目录）
    candidates.push(std::path::PathBuf::from("ffmpeg.exe"));
    // 4) 常见安装路径（免配置的兜底）
    for p in [
        r"D:\666\ffmpeg\bin\ffmpeg.exe",
        r"C:\ffmpeg\bin\ffmpeg.exe",
    ] {
        candidates.push(std::path::PathBuf::from(p));
    }
    for c in &candidates {
        if c.exists() { return c.to_string_lossy().into_owned(); }
    }
    // 5) 交给 PATH 解析（decoder 报的错会提示用户）
    "ffmpeg.exe".into()
}

fn load_config(path: &str) -> ClientConfig {
    let Ok(text) = fs::read_to_string(path) else {
        return ClientConfig::default();
    };
    let v: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => { eprintln!("[hvr-sink] 配置解析失败: {e}，使用默认值"); return ClientConfig::default(); }
    };
    let mut cfg = ClientConfig::default();
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
        cfg.stereo_layout = match layout.to_lowercase().as_str() {
            "tb" | "topbottom" => 1,
            _ => 0,
        };
    }
    if let Some(fov) = v["fov"].as_array() {
        for i in 0..2.min(fov.len()) {
            if let Some(a) = fov[i].as_array() {
                if a.len() == 4 {
                    cfg.fov[i] = Fov {
                        left: a[0].as_f64().unwrap_or(0.829) as f32,
                        right: a[1].as_f64().unwrap_or(0.829) as f32,
                        up: a[2].as_f64().unwrap_or(0.829) as f32,
                        down: a[3].as_f64().unwrap_or(0.829) as f32,
                    };
                }
            }
        }
    }
    cfg
}

fn main() {
    tracing_subscriber::fmt().with_target(false).init();

    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--selftest") {
        selftest();
        return;
    }
    if args.iter().any(|a| a == "--status") {
        let s = status::DeviceStatus::collect();
        println!("{}", s.render_panel());
        println!("\nJSON: {}", serde_json::to_string(&s).unwrap());
        return;
    }
    let cfg_path = args.iter().position(|a| a == "--config")
        .and_then(|i| args.get(i + 1))
        .map(|s| s.as_str())
        .unwrap_or("sink_config.json");
    let cfg = load_config(cfg_path);

    let stop = Arc::new(AtomicBool::new(false));
    {
        let s = Arc::clone(&stop);
        ctrlc_handler(move || { s.store(true, Ordering::Relaxed); });
    }

    // 设备状态监视器：头显/手机是否插上 + 电池，后台定期采集
    let status_mon = status::StatusMonitor::start(std::time::Duration::from_secs(1));
    {
        let mon = status_mon.clone();
        let stop_panel = Arc::clone(&stop);
        std::thread::spawn(move || loop {
            if stop_panel.load(Ordering::Relaxed) { break; }
            let mut s = mon.snapshot();
            // 连接状态必须实时读原子标志：StatusMonitor 的快照最多滞后 1 秒，
            // 而会话可能在瞬间建立又断开，用缓存会永远采不到 true。
            s.alvr_connected = status::alvr_connected();
            // 写 JSON 文件，供渲染器/外部 UI 读取
            if let Ok(j) = serde_json::to_string_pretty(&s) {
                let _ = fs::write("device_status.json", j);
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        });
    }
    // 打印一次初始状态
    eprintln!("{}", status_mon.snapshot().render_panel());

    // 姿态输入：等本地 SBS 渲染器通过 UDP 发来 IMU 四元数
    let pose = match PoseInput::bind() {
        Ok(p) => p.start(),
        Err(e) => {
            eprintln!("[hvr-sink] 姿态端口绑定失败: {e}（检查 9912 端口占用）");
            return;
        }
    };

    // 解码配置：分辨率取协商值，立体布局 SBS/TB 决定合成帧尺寸
    let mut dec_cfg = DecodeConfig::default();
    dec_cfg.width = cfg.view_resolution[0];
    dec_cfg.height = cfg.view_resolution[1];
    dec_cfg.fps = cfg.target_fps;
    dec_cfg.layout = cfg.stereo_layout;
    dec_cfg.ffmpeg = resolve_ffmpeg();
    eprintln!("[hvr-sink] ffmpeg: {}", dec_cfg.ffmpeg);
    let layout = cfg.stereo_layout;

    // 帧输出：解码后的 BGRA 帧经 TCP 回环发给本地 SBS 渲染器
    let mut frame_out = FrameOutput::connect();
    if !frame_out.is_connected() {
        eprintln!("[hvr-sink] 提示：本地 SBS 渲染器未在 127.0.0.1:{} 监听（可稍后启动，会持续重连）", render_bridge::FRAME_PORT);
    }

    // 解码器：NAL(Annex B) → FFmpeg → BGRA → 帧输出
    let mut dec = match Decoder::start(dec_cfg, move |bgra, w, h| {
        if !frame_out.send_frame(bgra, w, h, layout) {
            // 渲染器未连接，静默丢弃（低延迟优先，不积压）
        }
    }) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("[hvr-sink] 解码器启动失败: {e}");
            eprintln!("[hvr-sink] 请安装 FFmpeg 并放到同目录，或用 FFMPEG 环境变量指定路径");
            return;
        }
    };

    let mut on_frame = |_ts: std::time::Duration, nal: &[u8]| -> bool {
        dec.push_nal(nal)
    };

    // 电池回传闭包：把 HMD 电池/插电状态发给高配 PC 的 Dashboard
    let battery_mon = status_mon.clone();
    let battery = move || battery_mon.snapshot().hmd_battery();

    eprintln!("[hvr-sink] 启动，等待 PC 串流端（同局域网，ALVR Dashboard 运行中）…");
    let summary: RunSummary = run(&cfg, &mut on_frame, pose, battery, stop);
    eprintln!("[hvr-sink] 结束统计: {:?}", summary);
    dec.stop();
}

fn ctrlc_handler<F: FnOnce() + Send + 'static>(_f: F) {
    // 简化：Ctrl+C 由默认 SIGINT 终止；正式版可接 SetConsoleCtrlHandler。
}
