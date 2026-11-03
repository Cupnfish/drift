//! # Drift 核心
//!
//! 局域网 P2P 文件 / 剪贴板传输的核心库（无 GUI）。
//!
//! 功能对齐设计稿：
//! - 自动发现（mDNS `_drift._tcp.local.`）
//! - 配对：一码对照 + 双向确认 + 随时撤销
//! - 发送：文本 / 文件 / 文件夹（自动 zip）；一次发送一条记录，逐设备进度、暂停/继续/取消/重试
//! - 接收：配对即自动接收，落入接收目录
//! - 全局历史：筛选（全部/文件/图片/文本）、搜索、软删除恢复、CSV / txt / md 导出
//!
//! 安全：自签名 ed25519 证书 + TLS 1.3 mTLS；配对锁定证书指纹；
//! 配对码由双方指纹推导（SAS），中间人必然导致两端码不一致。
//!
//! GUI 集成入口：[`Drift::launch`] → [`DriftHandle`]。

pub mod app;
pub mod client;
pub mod discovery;
pub mod error;
pub mod events;
pub mod history;
pub mod identity;
pub mod pairing;
pub mod protocol;
pub mod server;
pub mod settings;
pub mod storage;
pub mod tls;
pub mod transfer;
pub mod util;

pub use app::{Drift, DriftHandle, LaunchConfig};
pub use error::{DriftError, Result};
pub use events::{Event, PairedDeviceView, PairingRequest};
pub use history::{
    Delivery, DeliveryStatus, Direction, FileEntry, Payload, PayloadKind, PeerSummary, Record,
    RecordFilter, RecordId, RecordSummary, TextExportFormat,
};
pub use identity::{DeviceKind, DeviceInfo, Fingerprint};
pub use settings::Settings;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_code_symmetric_and_six_digits() {
        let a = Fingerprint::from_bytes([1u8; 32]);
        let b = Fingerprint::from_bytes([2u8; 32]);
        let code_a = pairing::pairing_code(&a, &b);
        let code_b = pairing::pairing_code(&b, &a);
        assert_eq!(code_a, code_b);
        assert_eq!(code_a.len(), 7); // "482 916"
        assert!(code_a.chars().filter(|c| *c == ' ').count() == 1);
        // 相同设备对生成相同码
        assert_eq!(code_a, pairing::pairing_code(&a, &b));
        // 不同设备对码不同（概率上必然）
        let c = Fingerprint::from_bytes([3u8; 32]);
        assert_ne!(code_a, pairing::pairing_code(&a, &c));
    }

    #[test]
    fn fingerprint_hex_roundtrip() {
        let fp = Fingerprint::from_bytes([7u8; 32]);
        let hex = fp.to_hex();
        assert_eq!(hex.len(), 64);
        assert_eq!(Fingerprint::from_hex(&hex).unwrap(), fp);
        assert!(Fingerprint::from_hex("xyz").is_err());
    }

    #[test]
    fn sanitize_filename_strips_paths() {
        assert_eq!(util::sanitize_filename("a/b\\c:d*e?f\"g<h>i|j"), "a_b_c_d_e_f_g_h_i_j");
        assert_eq!(util::sanitize_filename("..."), "file");
        assert_eq!(util::sanitize_filename("  名字.txt "), "名字.txt");
    }
}
