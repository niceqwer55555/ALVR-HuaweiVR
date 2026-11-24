//! generic_alvr_client —— 去华为化的通用安卓 ALVR OpenXR 接收端（客户端逻辑层）。
//!
//! 真正的 OpenXR/网络/解码运行时在 Android 工程内用 Rust+FDI 实现；
//! 本 crate 收敛可独立验证的：配置模型、解码器策略、去华为化自检。

pub mod client_config;

pub use client_config::{
    ClientConfig, ControllerDof, PoseSource, VideoCodec,
    QCOM_H264_CANDIDATES, QCOM_H265_CANDIDATES,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_de_huawei() {
        let c = ClientConfig::default();
        assert!(c.assert_no_huawei().is_ok());
        assert!(c.use_standard_loader);
        assert!(c.no_software_decoder);
    }

    #[test]
    fn rejects_huawei_decoder_override() {
        let mut c = ClientConfig::default();
        c.decoder_override = Some("OMX.hisi.video.decoder.avc".into());
        assert!(c.assert_no_huawei().is_err());
    }

    #[test]
    fn huawei_loader_flag_rejected() {
        let mut c = ClientConfig::default();
        c.use_standard_loader = false;
        assert!(c.assert_no_huawei().is_err());
    }

    #[test]
    fn qcom_candidates_no_hisi() {
        for &n in QCOM_H264_CANDIDATES.iter().chain(QCOM_H265_CANDIDATES.iter()) {
            assert!(!n.contains("hisi"), "候选含华为组件: {n}");
            assert!(n.contains("qti") || n.contains("qcom"), "非高通组件: {n}");
        }
    }

    #[test]
    fn software_decoder_detection() {
        assert!(ClientConfig::is_software_decoder("c2.android.avc.decoder"));
        assert!(!ClientConfig::is_software_decoder("c2.qti.avc.decoder"));
    }

    #[test]
    fn pick_decoder_prefers_qcom_and_never_falls_back_to_sw() {
        let c = ClientConfig::default();
        // 只有软解可用 → None（禁软解，不静默降级）
        let pick = c.pick_decoder(|n| n.contains("android"));
        assert_eq!(pick, None);
        // 高通 C2 可用 → 首选
        let pick = c.pick_decoder(|n| n == "c2.qti.avc.decoder" || n.contains("android"));
        assert_eq!(pick, Some("c2.qti.avc.decoder"));
    }

    #[test]
    fn sbs_output_is_double_width() {
        let c = ClientConfig::default();
        assert_eq!(c.sbs_output_size(), [3200, 1600]);
    }

    #[test]
    fn default_both_controllers_6dof() {
        let c = ClientConfig::default();
        assert_eq!(c.left_controller_dof, ControllerDof::Dof6);
        assert_eq!(c.right_controller_dof, ControllerDof::Dof6);
    }
}
