//! ALVR v20.14.1 线协议：数据结构与帧编解码（自包含重写）
//!
//! 证据来源：HuaweiVRGlass-ALVR-main/alvr/{packets,sockets,client_core}/src/
//! 所有字段、字节序、帧前缀均逐条对照上游源码，保证与 PC 串流端（server）互通。
//!
//! 帧格式速查：
//! - 发现包（UDP 广播 → 255.255.255.255:9943，56 字节）：
//!     [0..8]="ALVR\0..." [16..24]=protocol_id(u64 LE) [24..]=hostname(UTF-8)
//! - 控制通道（TCP 9943，客户端 listen、server 连入）：
//!     每帧 = [u32 payload 长度 BE][bincode(payload)]
//! - 流通道（客户端另起 TCP/UDP listen，server 连入）：
//!     每分片 = [u32 分片长度-4 BE][u16 流ID][u32 包序号][u32 分片总数][u32 分片序号][header(bincode)][data]
//!     长度字段 = 整个分片长度 - 4（与上游 stream_socket.rs 的 packet_length - 4 一致），
//!     因此分片总长 = 4 + 字段值，不是 18 + 字段值。

use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::time::Duration;

/// 上游 alvr_common::protocol_id()：主版本号字符串 "20"（预发布时为 "20-pre"）
/// 对应 protocol_id_u64 = hash_string(protocol_id)。此值由服务端校验，
/// 但华为专版把版本字符串校验放宽（is_version_compatible 恒 true），
/// 实际反序列化失败才拒绝。客户端仍如实上报，保证互通。
pub const ALVR_VERSION_MAJOR: &str = "20";
pub const ALVR_NAME: &str = "ALVR";
pub const CONTROL_PORT: u16 = 9943;
pub const HANDSHAKE_PACKET_SIZE: usize = 56;

pub const STREAM_TRACKING: u16 = 0;
pub const STREAM_HAPTICS: u16 = 1;
pub const STREAM_AUDIO: u16 = 2;
pub const STREAM_VIDEO: u16 = 3;
pub const STREAM_STATISTICS: u16 = 4;

// ---------------------------------------------------------------------------
// 控制通道包（对齐 alvr_packets）
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct VideoStreamingCapabilitiesLegacy {
    pub default_view_resolution: [u32; 2],
    pub supported_refresh_rates_plus_extra_data: Vec<f32>,
    pub microphone_sample_rate: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ClientConnectionResultAccepted {
    pub client_protocol_id: u64,
    pub display_name: String,
    pub server_ip: IpAddr,
    pub streaming_capabilities: Option<VideoStreamingCapabilitiesLegacy>,
}

// 上游 ClientConnectionResult 是 enum：ConnectionAccepted{..} / ClientStandby
// bincode 对 enum 编码为 u32 变体序号（大端）+ 载荷，故手写等价枚举。
#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ClientConnectionResult {
    ConnectionAccepted {
        client_protocol_id: u64,
        display_name: String,
        server_ip: IpAddr,
        streaming_capabilities: Option<VideoStreamingCapabilitiesLegacy>,
    },
    ClientStandby,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct NegotiatedStreamingConfig {
    pub view_resolution: [u32; 2],
    pub refresh_rate_hint: f32,
    pub game_audio_sample_rate: u32,
    pub enable_foveated_encoding: bool,
    pub use_multimodal_protocol: bool,
    pub use_full_range: bool,
    pub encoding_gamma: f32,
    pub enable_hdr: bool,
    pub wired: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StreamConfigPacket {
    pub session: String,
    pub negotiated: String,
}

#[derive(Debug, Clone)]
pub struct NegotiatedConfig {
    pub view_resolution: [u32; 2],
    pub refresh_rate_hint: f32,
    pub game_audio_sample_rate: u32,
    pub enable_foveated_encoding: bool,
    pub use_multimodal_protocol: bool,
    pub use_full_range: bool,
    pub encoding_gamma: f32,
    pub enable_hdr: bool,
    pub wired: bool,
}

impl NegotiatedConfig {
    /// StreamConfigPacket.negotiated 是 NegotiatedStreamingConfig 的 JSON 字符串
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        let n: NegotiatedStreamingConfig = serde_json::from_str(s)?;
        Ok(Self {
            view_resolution: n.view_resolution,
            refresh_rate_hint: n.refresh_rate_hint,
            game_audio_sample_rate: n.game_audio_sample_rate,
            enable_foveated_encoding: n.enable_foveated_encoding,
            use_multimodal_protocol: n.use_multimodal_protocol,
            use_full_range: n.use_full_range,
            encoding_gamma: n.encoding_gamma,
            enable_hdr: n.enable_hdr,
            wired: n.wired,
        })
    }
}

/// 与上游 alvr_session::CodecType 的变体顺序一致；bincode 按变体序号编码，
/// 不是字符串，写成 String 会导致 DecoderConfig 反序列化错位。
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecType {
    H264,
    Hevc,
    AV1,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DecoderInitializationConfig {
    pub codec: CodecType,
    pub config_buffer: Vec<u8>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ServerControlPacket {
    StartStream,
    DecoderConfig(DecoderInitializationConfig),
    Restarting,
    KeepAlive,
    ServerPredictionAverage(Duration),
    Reserved(String),
    ReservedBuffer(Vec<u8>),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ClientControlPacket {
    PlayspaceSync(Option<[f32; 2]>),
    RequestIdr,
    KeepAlive,
    StreamReady,
    ViewsConfig(ViewsConfig),
    Battery(BatteryInfo),
    VideoErrorReport,
    Buttons(Vec<ButtonEntry>),
    ActiveInteractionProfile {
        device_id: u64,
        profile_id: u64,
    },
    Log {
        level: String,
        message: String,
    },
    Reserved(String),
    ReservedBuffer(Vec<u8>),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ViewsConfig {
    pub ipd_m: f32,
    pub fov: [Fov; 2],
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Fov {
    pub left: f32,
    pub right: f32,
    pub up: f32,
    pub down: f32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BatteryInfo {
    pub device_id: u64,
    pub gauge_value: f32,
    pub is_plugged: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ButtonValue {
    Binary(bool),
    Scalar(f32),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ButtonEntry {
    pub path_id: u64,
    pub value: ButtonValue,
}

// ---------------------------------------------------------------------------
// 流通道包
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct VideoPacketHeader {
    pub timestamp: Duration,
    pub is_idr: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct FaceData {
    pub eye_gazes: [Option<Pose>; 2],
    pub fb_face_expression: Option<Vec<f32>>,
    pub htc_eye_expression: Option<Vec<f32>>,
    pub htc_lip_expression: Option<Vec<f32>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Pose {
    pub orientation: [f32; 4], // xyzw
    pub position: [f32; 3],
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct DeviceMotion {
    pub pose: Pose,
    pub linear_velocity: [f32; 3],
    pub angular_velocity: [f32; 3],
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Tracking {
    pub target_timestamp: Duration,
    pub device_motions: Vec<(u64, DeviceMotion)>,
    /// 上游类型是 [Option<[Pose; 26]>; 2]，Pose 为 7 个 f32（四元数+位置）。
    /// 写成 [[f32;4];26] 会在 Some 时改变 bincode 大小。
    pub hand_skeletons: [Option<[Pose; 26]>; 2],
    pub face_data: FaceData,
}

// ---------------------------------------------------------------------------
// 帧编解码工具
// ---------------------------------------------------------------------------

/// 构造 UDP 发现包（56 字节）
pub fn build_discovery_packet(hostname: &str, protocol_id_u64: u64) -> [u8; HANDSHAKE_PACKET_SIZE] {
    let mut p = [0u8; HANDSHAKE_PACKET_SIZE];
    let name = ALVR_NAME.as_bytes();
    p[0..name.len()].copy_from_slice(name);
    p[16..24].copy_from_slice(&protocol_id_u64.to_le_bytes());
    let hb = hostname.as_bytes();
    let n = hb.len().min(HANDSHAKE_PACKET_SIZE - 24);
    p[24..24 + n].copy_from_slice(&hb[..n]);
    p
}

/// 控制通道：发送一帧 = [u32 BE len][bincode payload]
pub fn control_encode<T: Serialize>(packet: &T) -> Result<Vec<u8>, bincode::Error> {
    let payload = bincode::serialize(packet)?;
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(&payload);
    Ok(out)
}

/// 控制通道：从字节缓冲解析一帧，返回 (包, 消耗字节数)
pub fn control_decode<T: DeserializeOwned>(buf: &[u8]) -> Result<Option<(T, usize)>, DecodeError> {
    if buf.len() < 4 {
        return Ok(None);
    }
    let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    if len > 16 * 1024 * 1024 {
        return Err(DecodeError::BadLength(len));
    }
    if buf.len() < 4 + len {
        return Ok(None);
    }
    let packet = bincode::deserialize(&buf[4..4 + len])?;
    Ok(Some((packet, 4 + len)))
}

use serde::de::DeserializeOwned;

/// 流通道分片固定前缀长度：u32 len + u16 stream + u32 idx + u32 shards + u32 shard_idx
pub const SHARD_PREFIX_SIZE: usize = 4 + 2 + 4 + 4 + 4;

#[derive(Debug)]
pub struct ShardHeader {
    /// 线上长度字段，等于「整个分片长度 - 4」，不是前缀之后的字节数
    pub payload_len: usize,
    pub stream_id: u16,
    pub packet_index: u32,
    pub shards_count: usize,
    pub shard_index: usize,
}

impl ShardHeader {
    /// 从分片起始处解析 18 字节前缀
    pub fn parse(buf: &[u8]) -> Result<Self, DecodeError> {
        if buf.len() < SHARD_PREFIX_SIZE {
            return Err(DecodeError::TooShort);
        }
        Ok(Self {
            payload_len: u32::from_be_bytes(buf[0..4].try_into().unwrap()) as usize,
            stream_id: u16::from_be_bytes([buf[4], buf[5]]),
            packet_index: u32::from_be_bytes(buf[6..10].try_into().unwrap()),
            shards_count: u32::from_be_bytes(buf[10..14].try_into().unwrap()) as usize,
            shard_index: u32::from_be_bytes(buf[14..18].try_into().unwrap()) as usize,
        })
    }

    /// 整个分片的字节数（含 18 字节前缀）
    pub fn total_len(&self) -> usize {
        // 上游发送：packet_length - 4 -> 长度字段；因此 packet_length = 4 + 字段
        4 + self.payload_len
    }
}

#[derive(Debug)]
pub enum DecodeError {
    TooShort,
    BadLength(usize),
    Bincode(bincode::Error),
}

impl From<bincode::Error> for DecodeError {
    fn from(e: bincode::Error) -> Self {
        DecodeError::Bincode(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_packet_layout() {
        let p = build_discovery_packet("DESKTOP-TEST", 20);
        assert_eq!(&p[0..5], b"ALVR\0");
        assert_eq!(u64::from_le_bytes(p[16..24].try_into().unwrap()), 20);
        assert_eq!(&p[24..36], b"DESKTOP-TEST");
        assert_eq!(p.len(), 56);
    }

    #[test]
    fn control_roundtrip() {
        let pkt = ClientControlPacket::StreamReady;
        let bytes = control_encode(&pkt).unwrap();
        let (decoded, used): (ClientControlPacket, usize) =
            control_decode(&bytes).unwrap().unwrap();
        assert_eq!(used, bytes.len());
        assert!(matches!(decoded, ClientControlPacket::StreamReady));
    }

    #[test]
    fn control_decode_incomplete() {
        let bytes = [0u8, 0, 0, 10, 1, 2]; // 声明 10 字节，只有 2 字节
        let r: Result<Option<(ClientControlPacket, usize)>, DecodeError> = control_decode(&bytes);
        assert!(matches!(r, Ok(None)));
    }

    #[test]
    fn shard_header_parse() {
        let mut b = [0u8; SHARD_PREFIX_SIZE];
        b[0..4].copy_from_slice(&1200u32.to_be_bytes());
        b[4..6].copy_from_slice(&STREAM_VIDEO.to_be_bytes());
        b[6..10].copy_from_slice(&7u32.to_be_bytes());
        b[10..14].copy_from_slice(&3u32.to_be_bytes());
        b[14..18].copy_from_slice(&1u32.to_be_bytes());
        let h = ShardHeader::parse(&b).unwrap();
        assert_eq!(h.payload_len, 1200);
        assert_eq!(h.total_len(), 1204, "total_len 必须是 4 + 长度字段");
        assert_eq!(h.stream_id, STREAM_VIDEO);
        assert_eq!(h.packet_index, 7);
        assert_eq!(h.shards_count, 3);
        assert_eq!(h.shard_index, 1);
    }

    #[test]
    fn negotiated_config_json() {
        // 上游用 serde_json 序列化 NegotiatedStreamingConfig（外部标记枚举按字符串）
        let js = r#"{"view_resolution":[1536,1536],"refresh_rate_hint":70.0,
            "game_audio_sample_rate":48000,"enable_foveated_encoding":false,
            "use_multimodal_protocol":true,"use_full_range":false,
            "encoding_gamma":2.2,"enable_hdr":false,"wired":true}"#;
        let c = NegotiatedConfig::from_json(js).unwrap();
        assert_eq!(c.view_resolution, [1536, 1536]);
        assert_eq!(c.refresh_rate_hint, 70.0);
        assert!(c.wired);
    }
}
