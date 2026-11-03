//! 全局历史：收发记录模型、筛选（全部/文件/图片/文本 + 设备 + 关键词 + 时间）、
//! 软删除/恢复（Ctrl+Z）、CSV 导出、文本导出。
//!
//! 设计约定：一次发送只产生一条记录；记录内按设备列出各自状态。

use std::{
    collections::HashSet,
    io::Write,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    error::{DriftError, Result},
    identity::{DeviceKind, Fingerprint},
    storage::Storage,
    util,
};

pub type RecordId = uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Sent,
    Received,
}

impl Direction {
    pub fn label(&self) -> &'static str {
        match self {
            Direction::Sent => "发送",
            Direction::Received => "接收",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub size: u64,
    pub mime: String,
    /// 本地路径：发送方=源文件；接收方=落盘位置（另存为/打开/显示位置用）
    #[serde(default)]
    pub local_path: Option<PathBuf>,
    /// 是否为"文件夹压缩为 zip"
    #[serde(default)]
    pub zipped_from_folder: bool,
}

impl FileEntry {
    pub fn is_image(&self) -> bool {
        self.mime.starts_with("image/")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Payload {
    Text { text: String },
    Files { files: Vec<FileEntry> },
}

impl Payload {
    pub fn kind(&self) -> PayloadKind {
        match self {
            Payload::Text { .. } => PayloadKind::Text,
            Payload::Files { files } => {
                if files.iter().any(|f| f.is_image()) {
                    PayloadKind::Image
                } else {
                    PayloadKind::File
                }
            }
        }
    }

    pub fn total_size(&self) -> u64 {
        match self {
            Payload::Text { text } => text.len() as u64,
            Payload::Files { files } => files.iter().map(|f| f.size).sum(),
        }
    }
}

/// 筛选用内容类型（图片也是文件：Image 记录同时命中 File 与 Image 筛选）
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadKind {
    Text,
    File,
    Image,
}

impl PayloadKind {
    pub fn label(&self) -> &'static str {
        match self {
            PayloadKind::Text => "文本",
            PayloadKind::File => "文件",
            PayloadKind::Image => "图片",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PeerSummary {
    pub fingerprint: Fingerprint,
    pub name: String,
    pub kind: DeviceKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    /// 排队中
    Pending,
    /// 连接中（含自动重连）
    Connecting,
    /// 传输中
    Transferring,
    /// 已暂停
    Paused,
    /// 已送达
    Delivered,
    /// 失败（自动重连窗口内仍在重试时不会进入该状态）
    Failed,
    /// 等待重试（自动重连超时，等用户手动点重试）
    WaitingRetry,
    /// 已取消
    Canceled,
}

impl DeliveryStatus {
    pub fn label(&self) -> &'static str {
        match self {
            DeliveryStatus::Pending => "排队中",
            DeliveryStatus::Connecting => "连接中",
            DeliveryStatus::Transferring => "发送中",
            DeliveryStatus::Paused => "已暂停",
            DeliveryStatus::Delivered => "已送达",
            DeliveryStatus::Failed => "失败",
            DeliveryStatus::WaitingRetry => "等待重试",
            DeliveryStatus::Canceled => "已取消",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            DeliveryStatus::Delivered | DeliveryStatus::Failed | DeliveryStatus::Canceled | DeliveryStatus::WaitingRetry
        )
    }
}

/// 一次发送中，单个目标设备的投递状态
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Delivery {
    pub device: PeerSummary,
    pub status: DeliveryStatus,
    pub transferred: u64,
    pub total: u64,
    /// 实时速度（字节/秒，EMA 平滑）
    pub speed_bps: u64,
    #[serde(default)]
    pub error: Option<String>,
}

impl Delivery {
    pub fn progress(&self) -> f64 {
        if self.total == 0 {
            1.0
        } else {
            (self.transferred as f64 / self.total as f64).clamp(0.0, 1.0)
        }
    }

    pub fn eta_secs(&self) -> Option<u64> {
        if self.speed_bps == 0 || self.transferred >= self.total {
            return None;
        }
        Some((self.total - self.transferred) / self.speed_bps.max(1))
    }
}

/// 一条历史记录
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub id: RecordId,
    pub direction: Direction,
    pub timestamp: DateTime<Utc>,
    pub payload: Payload,
    /// 发送方向：每个目标设备一条状态；接收方向：空
    #[serde(default)]
    pub deliveries: Vec<Delivery>,
    /// 接收方向：来源设备；发送方向为 None
    #[serde(default)]
    pub peer: Option<PeerSummary>,
    /// 软删除时间（Ctrl+Z 恢复用）
    #[serde(default)]
    pub deleted: Option<DateTime<Utc>>,
}

impl Record {
    pub fn total_size(&self) -> u64 {
        self.payload.total_size()
    }

    pub fn delivered_count(&self) -> usize {
        self.deliveries
            .iter()
            .filter(|d| d.status == DeliveryStatus::Delivered)
            .count()
    }

    pub fn title(&self) -> String {
        match &self.payload {
            Payload::Text { text } => {
                let first = text.trim();
                if first.chars().count() > 40 {
                    let s: String = first.chars().take(40).collect();
                    format!("{s}…")
                } else {
                    first.to_string()
                }
            }
            Payload::Files { files } => {
                if files.len() == 1 {
                    files[0].name.clone()
                } else {
                    let first = files.first().map(|f| f.name.clone()).unwrap_or_default();
                    format!("{first} 等 {} 个文件", files.len())
                }
            }
        }
    }

    pub fn status_line(&self) -> String {
        match self.direction {
            Direction::Sent => {
                if self.deliveries.len() == 1 {
                    let d = &self.deliveries[0];
                    let mut s = match d.status {
                        DeliveryStatus::Transferring => {
                            format!("发送中 {}%", (d.progress() * 100.0) as u32)
                        }
                        other => other.label().to_string(),
                    };
                    if let Some(err) = &d.error {
                        s = format!("{s}：{err}");
                    }
                    s
                } else {
                    format!(
                        "已送达 {} 台 · {} 台未完成",
                        self.delivered_count(),
                        self.deliveries.len() - self.delivered_count()
                    )
                }
            }
            Direction::Received => "已接收".to_string(),
        }
    }

    pub fn summary(&self) -> RecordSummary {
        RecordSummary {
            id: self.id,
            direction: self.direction,
            kind: self.payload.kind(),
            title: self.title(),
            size: self.total_size(),
            timestamp: self.timestamp,
            status: self.status_line(),
        }
    }
}

/// 事件里随行推送的记录摘要
#[derive(Clone, Debug, Serialize)]
pub struct RecordSummary {
    pub id: RecordId,
    pub direction: Direction,
    pub kind: PayloadKind,
    pub title: String,
    pub size: u64,
    pub timestamp: DateTime<Utc>,
    pub status: String,
}

// ---------- 筛选 ----------

#[derive(Clone, Debug, Default)]
pub struct RecordFilter {
    pub direction: Option<Direction>,
    pub kinds: Option<Vec<PayloadKind>>,
    pub device: Option<Fingerprint>,
    pub query: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub include_deleted: bool,
}

impl RecordFilter {
    pub fn matches(&self, r: &Record) -> bool {
        if !self.include_deleted && r.deleted.is_some() {
            return false;
        }
        if self.direction.is_some_and(|d| r.direction != d) {
            return false;
        }
        if let Some(kinds) = &self.kinds {
            let kind = r.payload.kind();
            let hit = kinds.iter().any(|k| match k {
                // 图片同时属于"文件"
                PayloadKind::File => matches!(kind, PayloadKind::File | PayloadKind::Image),
                other => *other == kind,
            });
            if !hit {
                return false;
            }
        }
        if let Some(fp) = &self.device {
            let hit = r.peer.as_ref().is_some_and(|p| &p.fingerprint == fp)
                || r.deliveries.iter().any(|d| &d.device.fingerprint == fp);
            if !hit {
                return false;
            }
        }
        if let Some(q) = &self.query {
            let q = q.to_lowercase();
            let hit = match &r.payload {
                Payload::Text { text } => text.to_lowercase().contains(&q),
                Payload::Files { files } => files
                    .iter()
                    .any(|f| f.name.to_lowercase().contains(&q)),
            };
            if !hit {
                return false;
            }
        }
        if self.since.is_some_and(|since| r.timestamp < since) {
            return false;
        }
        if self.until.is_some_and(|until| r.timestamp > until) {
            return false;
        }
        true
    }
}

/// 按筛选取记录，新 → 旧
pub fn filter_records(storage: &Storage, filter: &RecordFilter) -> Result<Vec<Record>> {
    let all = storage.all_records_desc()?;
    Ok(all.into_iter().filter(|r| filter.matches(r)).collect())
}

/// 修改某条记录中某个设备的投递状态（无该设备则忽略）
pub fn patch_delivery<F>(storage: &Storage, id: &RecordId, fp: &Fingerprint, mut patch: F) -> Result<bool>
where
    F: FnMut(&mut Delivery),
{
    let Some(mut record) = storage.get_record(id)? else {
        return Ok(false);
    };
    let mut changed = false;
    for d in record.deliveries.iter_mut() {
        if &d.device.fingerprint == fp {
            patch(d);
            changed = true;
        }
    }
    if changed {
        storage.update_record(&record)?;
    }
    Ok(changed)
}

// ---------- 软删除 / 恢复 ----------

/// 软删除（不删本地文件）
pub fn soft_delete(storage: &Storage, id: &RecordId) -> Result<bool> {
    set_deleted(storage, id, Some(Utc::now()))
}

/// 恢复（Ctrl / Cmd + Z）
pub fn restore(storage: &Storage, id: &RecordId) -> Result<bool> {
    set_deleted(storage, id, None)
}

fn set_deleted(
    storage: &Storage,
    id: &RecordId,
    deleted: Option<DateTime<Utc>>,
) -> Result<bool> {
    let Some(mut record) = storage.get_record(id)? else {
        return Ok(false);
    };
    if record.deleted == deleted {
        return Ok(true);
    }
    record.deleted = deleted;
    storage.update_record(&record)?;
    Ok(true)
}

// ---------- 导出 ----------

/// 导出全部（按筛选）记录为 CSV。带 BOM，Excel 可直接打开中文。
pub fn export_csv(storage: &Storage, path: &Path, filter: &RecordFilter) -> Result<usize> {
    let records = filter_records(storage, filter)?;
    let mut w = std::fs::File::create(path)?;
    write!(w, "\u{FEFF}时间,方向,类型,内容,大小(字节),大小,设备,状态\r\n")
        ?;
    for r in &records {
        let devices = match r.direction {
            Direction::Sent => r
                .deliveries
                .iter()
                .map(|d| format!("{}：{}", d.device.name, d.status.label()))
                .collect::<Vec<_>>()
                .join("；"),
            Direction::Received => r
                .peer
                .as_ref()
                .map(|p| p.name.clone())
                .unwrap_or_else(|| "未知设备".into()),
        };
        let kind = match &r.payload {
            Payload::Text { .. } => "文本",
            Payload::Files { files } => {
                if files.iter().any(|f| f.is_image()) {
                    "图片"
                } else {
                    "文件"
                }
            }
        };
        writeln!(
            w,
            "{},{},{},{},{},{},{},{}\r\n",
            util::csv_escape(&r.timestamp.to_rfc3339()),
            r.direction.label(),
            kind,
            util::csv_escape(&r.title()),
            r.total_size(),
            util::human_bytes(r.total_size()),
            util::csv_escape(&devices),
            util::csv_escape(&r.status_line()),
        )?;
    }
    Ok(records.len())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextExportFormat {
    Txt,
    Markdown,
}

/// 把文本记录导出为 .txt / .md
pub fn export_text(
    storage: &Storage,
    id: &RecordId,
    path: &Path,
    fmt: TextExportFormat,
) -> Result<()> {
    let Some(record) = storage.get_record(id)? else {
        return Err(DriftError::Invalid("记录不存在".into()));
    };
    let Payload::Text { text } = &record.payload else {
        return Err(DriftError::Invalid("仅文本记录可导出为 .txt / .md".into()));
    };
    let content = match fmt {
        TextExportFormat::Txt => text.clone(),
        // md：代码块包裹，保留原始内容
        TextExportFormat::Markdown => format!("```\n{text}\n```\n"),
    };
    std::fs::write(path, content)?;
    Ok(())
}

/// 记录中第 idx 个文件的本地路径（发送=源文件；接收=落盘文件）
pub fn file_path(storage: &Storage, id: &RecordId, idx: usize) -> Result<Option<PathBuf>> {
    let Some(record) = storage.get_record(id)? else {
        return Ok(None);
    };
    match &record.payload {
        Payload::Files { files } => Ok(files.get(idx).and_then(|f| f.local_path.clone())),
        Payload::Text { .. } => Ok(None),
    }
}

/// 记录的可发送设备指纹集合（用于校验目标）
pub fn record_targets(r: &Record) -> HashSet<Fingerprint> {
    r.deliveries.iter().map(|d| d.device.fingerprint).collect()
}
