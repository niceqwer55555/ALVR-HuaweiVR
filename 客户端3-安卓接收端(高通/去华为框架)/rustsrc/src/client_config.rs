//! 通用 OpenXR 客户端配置 —— 完全去华为化。
//!
//! 参考仓库把 platform 写死为 HuaweiVrGlass，进而派生出 hisi 解码器、
//! XR_HUAWEI_controller_interaction 扩展、华为 EGL context。本模块改为：
//! - platform = GenericAndroid（标准 OpenXR + 官方 loader）
//! - 解码器 = 高通硬解（禁软解）
//! - 手柄 = 3DoF/6DoF 通用绑定（标准 OpenXR action 路径）
//! - 姿态 = phone-imu / udp-inject 双源

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PoseSource {
    #[serde(rename = "phone-imu")]
    PhoneImu,
    #[serde(rename = "udp-inject")]
    UdpInject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControllerDof {
    #[serde(rename = "3dof")]
    Dof3,
    #[serde(rename = "6dof")]
    Dof6,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoCodec {
    #[serde(rename = "h264")]
    H264,
    #[serde(rename = "h265")]
    H265,
}

/// 高通解码器候选（与 C 端 decoder_policy 保持一致）。
pub const QCOM_H264_CANDIDATES: &[&str] = &[
    "c2.qti.avc.decoder",
    "OMX.qcom.video.decoder.avc",
];
pub const QCOM_H265_CANDIDATES: &[&str] = &[
    "c2.qti.hevc.decoder",
    "OMX.qcom.video.decoder.hevc",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    /// ALVR server 地址（空则走发现）
    pub server_addr: String,
    /// 视图单眼分辨率
    pub view_resolution: [u32; 2],
    /// 刷新率
    pub refresh_hz: u32,
    /// 瞳距（米）
    pub ipd_meters: f32,
    /// 姿态源
    pub pose_source: PoseSource,
    /// UDP 姿态注入端口（localhost）
    pub pose_udp_port: u16,
    /// 视频编码
    pub codec: VideoCodec,
    /// 禁止软解（用户要求：true）
    pub no_software_decoder: bool,
    /// 解码器组件覆盖（None=按高通候选链）
    pub decoder_override: Option<String>,
    /// 左手柄自由度
    pub left_controller_dof: ControllerDof,
    /// 右手柄自由度
    pub right_controller_dof: ControllerDof,
    /// 是否启用标准 OpenXR loader（true）；false=无效配置
    pub use_standard_loader: bool,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            server_addr: String::new(),
            view_resolution: [1600, 1600],
            refresh_hz: 72,
            ipd_meters: 0.063,
            pose_source: PoseSource::PhoneImu,
            pose_udp_port: 47110,
            codec: VideoCodec::H264,
            no_software_decoder: true,
            decoder_override: None,
            left_controller_dof: ControllerDof::Dof6,
            right_controller_dof: ControllerDof::Dof6,
            use_standard_loader: true,
        }
    }
}

impl ClientConfig {
    /// 去华为化自检：任何华为痕迹都必须被拒绝。
    pub fn assert_no_huawei(&self) -> Result<(), String> {
        if !self.use_standard_loader {
            return Err("必须使用标准 OpenXR loader，不得用华为 libxr_loader.so".into());
        }
        if let Some(ref ov) = self.decoder_override {
            if ov.contains("hisi") {
                return Err(format!("解码器覆盖含华为 hisi 组件: {ov}"));
            }
        }
        Ok(())
    }

    /// 返回当前编码的高通解码器候选链。
    pub fn decoder_candidates(&self) -> &'static [&'static str] {
        match self.codec {
            VideoCodec::H264 => QCOM_H264_CANDIDATES,
            VideoCodec::H265 => QCOM_H265_CANDIDATES,
        }
    }

    /// SBS 合成输出尺寸（宽=单眼*2，高=单眼）。
    pub fn sbs_output_size(&self) -> [u32; 2] {
        [self.view_resolution[0] * 2, self.view_resolution[1]]
    }

    /// 判定组件是否为软件解码器。
    pub fn is_software_decoder(name: &str) -> bool {
        name.starts_with("c2.android.") || name.contains("google") || name.contains("software")
    }

    /// 按候选链选第一个在设备上可用的组件；禁软解时软解直接跳过。
    pub fn pick_decoder<F>(&self, mut available: F) -> Option<&'static str>
    where
        F: FnMut(&str) -> bool,
    {
        if let Some(ref ov) = self.decoder_override {
            // 覆盖名不是 &'static str，单独判断后返回候选链里的静态引用不合适；
            // 这里仅接受覆盖名命中候选链的情形，否则交给调用方直连。
            if available(ov) && (!Self::is_software_decoder(ov) || !self.no_software_decoder) {
                if let Some(&s) = self.decoder_candidates().iter().find(|&&x| x == ov) {
                    return Some(s);
                }
            }
        }
        for &name in self.decoder_candidates() {
            if available(name) {
                return Some(name);
            }
        }
        None // 禁软解且无高通硬解 → 失败，不静默降级
    }
}
