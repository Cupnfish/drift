//! 事件：核心 → GUI 的唯一通知通道（broadcast，GUI 端订阅后驱动重绘）。

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::{
    discovery::PeerDevice,
    history::{RecordId, RecordSummary},
    identity::Fingerprint,
    settings::Settings,
};

/// 配对请求（两端各自看到一份；code 由两端指纹推导，必然一致）
#[derive(Clone, Debug, Serialize)]
pub struct PairingRequest {
    /// 对端指纹
    pub peer: Fingerprint,
    /// 对端展示名
    pub peer_name: String,
    /// 对端设备类型
    pub peer_kind: String,
    /// 六位配对码，展示为 "482 916"
    pub code: String,
    pub expires_at: DateTime<Utc>,
    /// 本端是否发起方
    pub initiated_by_us: bool,
}

/// 已配对设备（信任存储条目）
#[derive(Clone, Debug, Serialize)]
pub struct PairedDeviceView {
    pub fingerprint: Fingerprint,
    pub name: String,
    pub kind: String,
    pub paired_at: DateTime<Utc>,
    pub addrs: Vec<String>,
    pub online: bool,
    pub last_seen: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// 服务启动完成（端口确定）
    ServerStarted { port: u16 },

    DeviceAppeared { device: PeerDevice },
    DeviceUpdated { device: PeerDevice },
    DeviceLost { fingerprint: Fingerprint },

    /// 附近设备发起配对（本端是被发起方，等待用户核对配对码）
    PairingIncoming { request: PairingRequest },
    /// 本端发起配对成功送达对端（本端是发起方，展示同一组配对码）
    PairingStarted { request: PairingRequest },
    PairingCompleted { fingerprint: Fingerprint, name: String },
    /// 任一端点了「不一致」，本次配对作废
    PairingRejected { fingerprint: Fingerprint, by_remote: bool },
    PairingCanceled { fingerprint: Fingerprint },
    PairingExpired { fingerprint: Fingerprint },

    /// 解除配对（本端主动或对端通知）
    DeviceUnpaired { fingerprint: Fingerprint },

    RecordAdded { id: RecordId, summary: RecordSummary },
    /// 传输进度 / 状态变化（含节流后的进度推送）
    RecordUpdated { id: RecordId, summary: RecordSummary },
    RecordDeleted { id: RecordId },
    RecordRestored { id: RecordId },

    SettingsSaved { settings: Settings },

    Error { context: String, message: String },
}

impl Event {
    /// 单行日志描述（CLI / 调试用）
    pub fn brief(&self) -> String {
        match self {
            Event::ServerStarted { port } => format!("服务已启动，端口 {port}"),
            Event::DeviceAppeared { device } => {
                format!("发现设备 {}（{}）", device.name, device.fingerprint.short())
            }
            Event::DeviceUpdated { device } => format!("设备更新 {}", device.name),
            Event::DeviceLost { fingerprint } => format!("设备离线 {}", fingerprint.short()),
            Event::PairingIncoming { request } => {
                format!("配对请求来自 {}，配对码 {}", request.peer_name, request.code)
            }
            Event::PairingStarted { request } => {
                format!("已向 {} 发起配对，配对码 {}", request.peer_name, request.code)
            }
            Event::PairingCompleted { fingerprint, name } => {
                format!("配对完成：{name}（{}）", fingerprint.short())
            }
            Event::PairingRejected { fingerprint, by_remote } => format!(
                "配对不一致已作废 {}{}",
                fingerprint.short(),
                if *by_remote { "（对端操作）" } else { "" }
            ),
            Event::PairingCanceled { fingerprint } => format!("配对已取消 {}", fingerprint.short()),
            Event::PairingExpired { fingerprint } => format!("配对码过期 {}", fingerprint.short()),
            Event::DeviceUnpaired { fingerprint } => format!("已解除配对 {}", fingerprint.short()),
            Event::RecordAdded { id, summary } => {
                format!("新记录 {id}: [{}] {}", summary.status, summary.title)
            }
            Event::RecordUpdated { id, summary } => {
                format!("记录更新 {id}: [{}] {}", summary.status, summary.title)
            }
            Event::RecordDeleted { id } => format!("记录已删除 {id}"),
            Event::RecordRestored { id } => format!("记录已恢复 {id}"),
            Event::SettingsSaved { .. } => "设置已保存".to_string(),
            Event::Error { context, message } => format!("{context}: {message}"),
        }
    }
}
