//! 统一错误类型。

use thiserror::Error;

/// Drift 核心错误
#[derive(Debug, Error)]
pub enum DriftError {
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("数据目录错误: {0}")]
    Dir(String),

    #[error("本地存储错误: {0}")]
    Storage(String),

    #[error("TLS / 证书错误: {0}")]
    Tls(String),

    #[error("协议错误: {0}")]
    Protocol(String),

    #[error("对方设备返回错误({status}): {message}")]
    Remote { status: u16, message: String },

    #[error("设备不在线或地址未知: {0}")]
    UnknownDevice(String),

    #[error("设备未配对: {0}")]
    NotPaired(String),

    #[error("配对会话无效或已过期")]
    PairingSessionInvalid,

    #[error("传输任务不存在")]
    TransferNotFound,

    #[error("参数错误: {0}")]
    Invalid(String),

    #[error("操作已取消")]
    Canceled,

    #[error("序列化错误: {0}")]
    Json(#[from] serde_json::Error),

    #[error("压缩文件夹失败: {0}")]
    Zip(String),

    #[error("{0}")]
    Other(String),
}

impl From<redb::Error> for DriftError {
    fn from(e: redb::Error) -> Self {
        DriftError::Storage(e.to_string())
    }
}

impl From<redb::DatabaseError> for DriftError {
    fn from(e: redb::DatabaseError) -> Self {
        DriftError::Storage(e.to_string())
    }
}

impl From<redb::TransactionError> for DriftError {
    fn from(e: redb::TransactionError) -> Self {
        DriftError::Storage(e.to_string())
    }
}

impl From<redb::CommitError> for DriftError {
    fn from(e: redb::CommitError) -> Self {
        DriftError::Storage(e.to_string())
    }
}

impl From<redb::StorageError> for DriftError {
    fn from(e: redb::StorageError) -> Self {
        DriftError::Storage(e.to_string())
    }
}

impl From<redb::TableError> for DriftError {
    fn from(e: redb::TableError) -> Self {
        DriftError::Storage(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, DriftError>;
