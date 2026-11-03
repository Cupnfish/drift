//! 线路协议：HTTP API 路径、JSON DTO、常量。
//!
//! 所有请求走 TLS 1.3：
//! - 客户端校验服务端证书指纹（mDNS 公告或配对时锁定）；
//! - 服务端要求客户端证书（mTLS），对端身份 = 证书指纹，不再依赖请求体自报。

use serde::{Deserialize, Serialize};

use crate::identity::{DeviceInfo, DeviceKind, Fingerprint};

pub const PROTOCOL_VERSION: u32 = 1;
pub const SERVICE_TYPE: &str = "_drift._tcp.local.";

/// 单个数据块大小（上传分片）
pub const CHUNK_SIZE: usize = 256 * 1024;
/// 配对码有效期（秒）
pub const PAIRING_TTL_SECS: u64 = 180;
/// 断线自动重连窗口（秒）：超过后转为"等待重试"，由用户手动触发
pub const AUTO_RETRY_WINDOW_SECS: u64 = 30;
/// 接收会话空闲回收（秒）
pub const INCOMING_IDLE_SECS: u64 = 600;
/// 文本消息长度上限
pub const MAX_TEXT_LEN: usize = 10 * 1024 * 1024;

pub mod path {
    pub const INFO: &str = "/api/v1/info";
    pub const PAIR_REQUEST: &str = "/api/v1/pair/request";
    pub const PAIR_CONFIRM: &str = "/api/v1/pair/confirm";
    pub const PAIR_REJECT: &str = "/api/v1/pair/reject";
    pub const UNPAIR: &str = "/api/v1/unpair";
    pub const SEND_TEXT: &str = "/api/v1/send/text";
    pub const SEND_PREPARE: &str = "/api/v1/send/prepare";

    pub fn file_state(session_id: &str, index: usize) -> String {
        format!("/api/v1/send/{session_id}/file/{index}/state")
    }

    pub fn file_chunk(session_id: &str, index: usize) -> String {
        format!("/api/v1/send/{session_id}/file/{index}")
    }

    pub fn session(session_id: &str) -> String {
        format!("/api/v1/send/{session_id}")
    }
}

/// 请求头：当前分片在文件中的偏移
pub const HEADER_OFFSET: &str = "x-drift-offset";

// ---------- DTO ----------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceDto {
    pub fingerprint: String,
    pub name: String,
    pub kind: String,
    pub port: u16,
    pub version: u32,
}

impl DeviceDto {
    pub fn from_info(info: &DeviceInfo) -> Self {
        DeviceDto {
            fingerprint: info.fingerprint.to_hex(),
            name: info.name.clone(),
            kind: info.kind.as_str().to_string(),
            port: info.port,
            version: PROTOCOL_VERSION,
        }
    }

    pub fn parse_fingerprint(&self) -> Result<Fingerprint, crate::error::DriftError> {
        Fingerprint::from_hex(&self.fingerprint)
    }

    pub fn parse_kind(&self) -> DeviceKind {
        DeviceKind::parse(&self.kind).unwrap_or(DeviceKind::Other)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileMetaDto {
    pub name: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrepareRequest {
    /// 发送方生成的传输 ID（UUID），接收方据此幂等创建/恢复会话
    pub id: String,
    pub sender: DeviceDto,
    pub files: Vec<FileMetaDto>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileStateDto {
    /// 接收方已收到的字节数（用于断点续传）
    pub offset: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrepareResponse {
    pub session: String,
    pub files: Vec<FileStateDto>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextRequest {
    pub id: String,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChunkAck {
    pub offset: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PairRequestBody {
    pub sender: DeviceDto,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PairConfirmResponse {
    /// 对端是否已同时确认（两端都确认即配对完成）
    pub completed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiResponse {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ApiResponse {
    pub fn err(msg: impl Into<String>) -> Self {
        ApiResponse { ok: false, error: Some(msg.into()) }
    }
}
