//! 传输引擎：
//! - 出站：每个目标设备一个独立任务（进度、暂停/继续、取消、失败重试互不影响）；
//!   文件按 256KiB 分块上传，断点续传；断线 30 秒内自动重连，超时转"等待重试"；
//! - 入站：会话落盘到下载目录的 .incoming 临时区，完成后改名入库并广播记录。

use std::{
    collections::HashMap,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use axum::http::Method;
use chrono::Utc;
use tokio::{io::{AsyncReadExt, AsyncSeekExt}, sync::mpsc};

use crate::{
    app::Core,
    client::PeerLink,
    error::{DriftError, Result},
    events::Event,
    history::{self, DeliveryStatus, Direction, FileEntry, PeerSummary, Record, RecordId},
    identity::Fingerprint,
    protocol::{self, FileMetaDto, PrepareRequest, PrepareResponse, TextRequest},
    util,
};

// ================= 出站 =================

/// 待发送文件（src 为实际要读的路径；文件夹已压缩为 zip）
#[derive(Clone, Debug)]
pub struct OutgoingFile {
    pub src_path: PathBuf,
    pub name: String,
    pub size: u64,
    pub mime: String,
}

#[derive(Clone)]
pub enum SendPlan {
    Text(String),
    /// temp_files：发送结束后需要清理的临时文件（zip / 剪贴板落盘）
    Files { files: Vec<OutgoingFile>, temp_files: Option<PathBuf> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobCmd {
    Pause,
    Resume,
    Cancel,
    Retry,
}

pub struct TransferManager {
    jobs: Mutex<HashMap<(RecordId, Fingerprint), mpsc::UnboundedSender<JobCmd>>>,
    /// 记录 → 待清理的临时文件（文件夹 zip / 剪贴板落盘文件）
    temp_files: Mutex<HashMap<RecordId, Vec<PathBuf>>>,
}

impl Default for TransferManager {
    fn default() -> Self {
        TransferManager {
            jobs: Mutex::new(HashMap::new()),
            temp_files: Mutex::new(HashMap::new()),
        }
    }
}

impl TransferManager {
    /// 启动一个（record × 目标设备）的发送任务
    pub fn spawn_job(
        &self,
        core: Arc<Core>,
        record_id: RecordId,
        target: PeerSummary,
        addrs: Vec<SocketAddr>,
        plan: SendPlan,
    ) {
        let (tx, rx) = mpsc::unbounded_channel();
        self.jobs.lock().unwrap().insert((record_id, target.fingerprint), tx);
        if let SendPlan::Files { temp_files: Some(z), .. } = &plan {
            self.temp_files
                .lock()
                .unwrap()
                .entry(record_id)
                .or_default()
                .push(z.clone());
        }
        let core2 = core.clone();
        tokio::spawn(async move {
            run_send_job(core2, record_id, target, addrs, plan, rx).await;
        });
    }

    fn send_cmd(&self, record_id: &RecordId, fp: &Fingerprint, cmd: JobCmd) -> Result<()> {
        let jobs = self.jobs.lock().unwrap();
        if let Some(tx) = jobs.get(&(*record_id, *fp)) {
            tx.send(cmd).map_err(|_| DriftError::TransferNotFound)?;
            Ok(())
        } else {
            Err(DriftError::TransferNotFound)
        }
    }

    pub fn pause(&self, record_id: &RecordId, fp: &Fingerprint) -> Result<()> {
        self.send_cmd(record_id, fp, JobCmd::Pause)
    }

    pub fn resume(&self, record_id: &RecordId, fp: &Fingerprint) -> Result<()> {
        self.send_cmd(record_id, fp, JobCmd::Resume)
    }

    pub fn retry(&self, record_id: &RecordId, fp: &Fingerprint) -> Result<()> {
        self.send_cmd(record_id, fp, JobCmd::Retry)
    }

    pub fn cancel(&self, record_id: &RecordId, fp: &Fingerprint) -> Result<()> {
        self.send_cmd(record_id, fp, JobCmd::Cancel)
    }

    /// 取消整条记录的所有目标
    pub fn cancel_record(&self, record_id: &RecordId) -> usize {
        let jobs = self.jobs.lock().unwrap();
        let mut n = 0;
        for ((rid, _), tx) in jobs.iter() {
            if rid == record_id
                && tx.send(JobCmd::Cancel).is_ok()
            {
                n += 1;
            }
        }
        n
    }

    /// 任务退出回收：该记录没有活动任务后清理临时文件
    fn on_job_exit(&self, record_id: &RecordId, fp: &Fingerprint) {
        self.jobs.lock().unwrap().remove(&(*record_id, *fp));
        let no_more = {
            let jobs = self.jobs.lock().unwrap();
            jobs.keys().all(|(rid, _)| rid != record_id)
        };
        if no_more
            && let Some(temps) = self.temp_files.lock().unwrap().remove(record_id)
        {
            for t in temps {
                let _ = std::fs::remove_file(t);
            }
        }
    }
}

/// EMA 速度计
struct SpeedMeter {
    last_sample: Option<(Instant, u64)>,
    ema_bps: f64,
}

impl SpeedMeter {
    fn new() -> Self {
        SpeedMeter { last_sample: None, ema_bps: 0.0 }
    }

    fn push(&mut self, bytes: u64) -> u64 {
        let now = Instant::now();
        if let Some((t, b)) = self.last_sample {
            let dt = now.duration_since(t).as_secs_f64();
            if dt >= 0.25 {
                let bps = (bytes - b) as f64 / dt;
                self.ema_bps = if self.ema_bps == 0.0 { bps } else { 0.3 * bps + 0.7 * self.ema_bps };
                self.last_sample = Some((now, bytes));
            }
        } else {
            self.last_sample = Some((now, bytes));
        }
        self.ema_bps as u64
    }
}

struct JobCtx {
    core: Arc<Core>,
    record_id: RecordId,
    target: PeerSummary,
    /// 最近一次进度（重连等待期间保持显示，不清零）
    last_transferred: u64,
    last_emit: Instant,
}

impl JobCtx {
    /// 更新该目标设备的投递状态（节流推送 RecordUpdated，状态变化立即推）
    fn update(&mut self, status: DeliveryStatus, transferred: u64, speed: u64, error: Option<String>) {
        self.last_transferred = transferred;
        let status_changed = {
            // 读取旧状态判断是否需要立刻广播
            self.core
                .storage
                .get_record(&self.record_id)
                .ok()
                .flatten()
                .and_then(|r| r.deliveries.into_iter().find(|d| d.device.fingerprint == self.target.fingerprint))
                .map(|d| d.status != status)
                .unwrap_or(true)
        };
        let done = status == DeliveryStatus::Delivered;
        let fp = self.target.fingerprint;
        let core = self.core.clone();
        let _ = history::patch_delivery(&core.storage, &self.record_id, &fp, |d| {
            d.status = status;
            d.transferred = transferred;
            d.speed_bps = speed;
            d.error = error.clone();
        });
        let now = Instant::now();
        if status_changed || done || now.duration_since(self.last_emit) >= Duration::from_millis(300) {
            self.last_emit = now;
            if let Ok(Some(r)) = core.storage.get_record(&self.record_id) {
                let summary = r.summary();
                core.emit(Event::RecordUpdated { id: r.id, summary });
            }
        }
    }
}

fn poll_cmd(rx: &mut mpsc::UnboundedReceiver<JobCmd>) -> Option<JobCmd> {
    match rx.try_recv() {
        Ok(cmd) => Some(cmd),
        Err(mpsc::error::TryRecvError::Empty) => None,
        Err(mpsc::error::TryRecvError::Disconnected) => Some(JobCmd::Cancel),
    }
}

async fn run_send_job(
    core: Arc<Core>,
    record_id: RecordId,
    target: PeerSummary,
    addrs: Vec<SocketAddr>,
    plan: SendPlan,
    mut cmd_rx: mpsc::UnboundedReceiver<JobCmd>,
) {
    let link = PeerLink::new(target.fingerprint, addrs, core.identity.tls_material());
    let total = match &plan {
        SendPlan::Text(t) => t.len() as u64,
        SendPlan::Files { files, .. } => files.iter().map(|f| f.size).sum(),
    };
    let mut ctx = JobCtx {
        core: core.clone(),
        record_id,
        target: target.clone(),
        last_transferred: 0,
        last_emit: Instant::now() - Duration::from_secs(1),
    };

    ctx.update(DeliveryStatus::Connecting, 0, 0, None);

    let send_id = record_id.to_string();
    let mut first_err: Option<Instant> = None;
    let mut backoff = Duration::from_secs(1);
    let mut result: Result<()> = Ok(());

    'attempt: loop {
        // 顶部命令处理（暂停在传输开始前到达也生效，不能丢弃）
        match poll_cmd(&mut cmd_rx) {
            Some(JobCmd::Cancel) => {
                result = Err(DriftError::Canceled);
                break 'attempt;
            }
            Some(JobCmd::Pause) => {
                ctx.update(DeliveryStatus::Paused, ctx.last_transferred, 0, None);
                match wait_resume(&mut cmd_rx).await {
                    WaitResult::Resume => {}
                    WaitResult::Cancel => {
                        result = Err(DriftError::Canceled);
                        break 'attempt;
                    }
                }
            }
            _ => {}
        }

        match &plan {
            SendPlan::Text(text) => {
                ctx.update(DeliveryStatus::Connecting, 0, 0, None);
                let body = TextRequest { id: send_id.clone(), text: text.clone() };
                match link.send_json::<protocol::ApiResponse>(Method::POST, protocol::path::SEND_TEXT, &body).await {
                    Ok(_) => {
                        ctx.update(DeliveryStatus::Delivered, total, 0, None);
                        break 'attempt;
                    }
                    Err(e) => {
                        match handle_error(&mut ctx, &mut first_err, &mut backoff, &mut cmd_rx, e).await {
                            ErrorAction::Retry => continue 'attempt,
                            ErrorAction::Canceled => {
                                result = Err(DriftError::Canceled);
                                break 'attempt;
                            }
                        }
                    }
                }
            }
            SendPlan::Files { files, .. } => {
                ctx.update(DeliveryStatus::Connecting, 0, 0, None);
                let metas: Vec<FileMetaDto> = files
                    .iter()
                    .map(|f| FileMetaDto { name: f.name.clone(), size: f.size, mime: Some(f.mime.clone()) })
                    .collect();
                let prepare = PrepareRequest { id: send_id.clone(), sender: core.device_dto(), files: metas };

                let prep: PrepareResponse = match link
                    .send_json(Method::POST, protocol::path::SEND_PREPARE, &prepare)
                    .await
                {
                    Ok(r) => r,
                    Err(e) => {
                        match handle_error(&mut ctx, &mut first_err, &mut backoff, &mut cmd_rx, e).await {
                            ErrorAction::Retry => continue 'attempt,
                            ErrorAction::Canceled => {
                                result = Err(DriftError::Canceled);
                                break 'attempt;
                            }
                        }
                    }
                };
                let session = prep.session;
                let mut offsets: Vec<u64> = prep.files.iter().map(|f| f.offset).collect();

                ctx.update(DeliveryStatus::Transferring, offsets.iter().sum(), 0, None);

                // 逐文件分块上传
                'files: for (i, file) in files.iter().enumerate() {
                    let mut fh = match tokio::fs::File::open(&file.src_path).await {
                        Ok(f) => f,
                        Err(e) => {
                            result = Err(DriftError::Io(e));
                            break 'files;
                        }
                    };
                    if let Err(e) = fh.seek(std::io::SeekFrom::Start(offsets[i])).await {
                        result = Err(DriftError::Io(e));
                        break 'files;
                    }
                    let mut buf = vec![0u8; protocol::CHUNK_SIZE];
                    let mut speed = SpeedMeter::new();

                    loop {
                        // 命令检查（暂停 / 取消）
                        match poll_cmd(&mut cmd_rx) {
                            Some(JobCmd::Cancel) => {
                                result = Err(DriftError::Canceled);
                                break 'files;
                            }
                            Some(JobCmd::Pause) => {
                                ctx.update(DeliveryStatus::Paused, offsets.iter().sum(), 0, None);
                                match wait_resume(&mut cmd_rx).await {
                                    WaitResult::Resume => {
                                        ctx.update(DeliveryStatus::Transferring, offsets.iter().sum(), 0, None);
                                    }
                                    WaitResult::Cancel => {
                                        result = Err(DriftError::Canceled);
                                        break 'files;
                                    }
                                }
                            }
                            _ => {}
                        }

                        let want = ((file.size - offsets[i]) as usize).min(buf.len());
                        if want == 0 {
                            break; // 本文件完成
                        }
                        let n = match fh.read(&mut buf[..want]).await {
                            Ok(0) => {
                                result = Err(DriftError::Protocol("源文件被截断".into()));
                                break 'files;
                            }
                            Ok(n) => n,
                            Err(e) => {
                                result = Err(DriftError::Io(e));
                                break 'files;
                            }
                        };

                        let chunk_path = protocol::path::file_chunk(&session, i);
                        match link.put_chunk(&chunk_path, offsets[i], buf[..n].to_vec()).await {
                            Ok(acked) => {
                                if acked != offsets[i] + n as u64 {
                                    result = Err(DriftError::Protocol("分片确认偏移不一致".into()));
                                    break 'files;
                                }
                                offsets[i] = acked;
                                let done: u64 = offsets.iter().sum();
                                let spd = speed.push(done);
                                ctx.update(DeliveryStatus::Transferring, done, spd, None);
                            }
                            Err(e) => {
                                first_err.get_or_insert(Instant::now());
                                match handle_error(&mut ctx, &mut first_err, &mut backoff, &mut cmd_rx, e).await {
                                    ErrorAction::Retry => {
                                        // 重新 prepare 获取断点（continue 外层重建会话）
                                        continue 'attempt;
                                    }
                                    ErrorAction::Canceled => {
                                        result = Err(DriftError::Canceled);
                                        break 'files;
                                    }
                                }
                            }
                        }
                    }
                }

                match &result {
                    Err(DriftError::Canceled) => {
                        // 尽力通知接收端丢弃
                        let _ = link
                            .request(
                                Method::DELETE,
                                &protocol::path::session(&session),
                                None,
                                "application/json",
                                &[],
                            )
                            .await;
                        ctx.update(DeliveryStatus::Canceled, offsets.iter().sum(), 0, None);
                        break 'attempt;
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        ctx.update(DeliveryStatus::Failed, offsets.iter().sum(), 0, Some(msg));
                        break 'attempt;
                    }
                    Ok(()) => {
                        // 全部文件发完 → finish
                        let fin = protocol::path::session(&session);
                        match link.send_json::<protocol::ApiResponse>(Method::POST, &fin, &serde_json::json!({})).await {
                            Ok(_) => {
                                ctx.update(DeliveryStatus::Delivered, total, 0, None);
                                break 'attempt;
                            }
                            Err(e) => {
                                match handle_error(&mut ctx, &mut first_err, &mut backoff, &mut cmd_rx, e).await {
                                    ErrorAction::Retry => continue 'attempt,
                                    ErrorAction::Canceled => {
                                        result = Err(DriftError::Canceled);
                                        break 'attempt;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // 取消收尾（handle_error 返回 Canceled 的路径）
    if let Err(DriftError::Canceled) = &result {
        ctx.update(DeliveryStatus::Canceled, ctx.last_transferred, 0, None);
    }

    core.transfers.on_job_exit(&record_id, &target.fingerprint);
}

enum ErrorAction {
    Retry,
    Canceled,
}

/// 断线策略：30 秒内退避自动重连；超过转「等待重试」，等用户手动 Retry。
async fn handle_error(
    ctx: &mut JobCtx,
    first_err: &mut Option<Instant>,
    backoff: &mut Duration,
    cmd_rx: &mut mpsc::UnboundedReceiver<JobCmd>,
    e: DriftError,
) -> ErrorAction {
    let started = *first_err.get_or_insert(Instant::now());
    let elapsed = started.elapsed().as_secs();
    if elapsed < protocol::AUTO_RETRY_WINDOW_SECS {
        ctx.update(DeliveryStatus::Connecting, ctx.last_transferred, 0, Some(e.to_string()));
        tokio::time::sleep(*backoff).await;
        *backoff = (*backoff * 2).min(Duration::from_secs(8));
        // 睡眠期间可能收到取消或暂停
        match poll_cmd(cmd_rx) {
            Some(JobCmd::Cancel) => return ErrorAction::Canceled,
            Some(JobCmd::Pause) => {
                ctx.update(DeliveryStatus::Paused, ctx.last_transferred, 0, None);
                match wait_resume(cmd_rx).await {
                    WaitResult::Resume => {}
                    WaitResult::Cancel => return ErrorAction::Canceled,
                }
            }
            _ => {}
        }
        ErrorAction::Retry
    } else {
        ctx.update(DeliveryStatus::WaitingRetry, ctx.last_transferred, 0, Some(e.to_string()));
        loop {
            match cmd_rx.recv().await {
                Some(JobCmd::Retry) | Some(JobCmd::Resume) => {
                    *first_err = None;
                    *backoff = Duration::from_secs(1);
                    return ErrorAction::Retry;
                }
                Some(JobCmd::Cancel) | None => return ErrorAction::Canceled,
                Some(JobCmd::Pause) => {}
            }
        }
    }
}

enum WaitResult {
    Resume,
    Cancel,
}

async fn wait_resume(cmd_rx: &mut mpsc::UnboundedReceiver<JobCmd>) -> WaitResult {
    loop {
        match cmd_rx.recv().await {
            Some(JobCmd::Resume) | Some(JobCmd::Retry) => return WaitResult::Resume,
            Some(JobCmd::Cancel) | None => return WaitResult::Cancel,
            Some(JobCmd::Pause) => {}
        }
    }
}

// ================= 入站 =================

#[derive(Clone, Debug)]
pub struct IncomingFile {
    pub name: String,
    pub size: u64,
    pub mime: String,
    pub tmp_path: PathBuf,
    pub len: u64,
}

#[derive(Clone, Debug)]
pub struct IncomingSession {
    pub id: String,
    pub peer: PeerSummary,
    pub files: Vec<IncomingFile>,
    pub dir: PathBuf,
    pub created: Instant,
    pub last_touch: Instant,
}

#[derive(Default)]
pub struct IncomingManager {
    sessions: Mutex<HashMap<String, IncomingSession>>,
}

impl IncomingManager {
    /// 准备（幂等）：同 ID 再来返回已接收偏移，实现断点续传
    pub fn prepare(
        &self,
        core: &Core,
        id: &str,
        peer: PeerSummary,
        metas: Vec<FileMetaDto>,
    ) -> Result<PrepareResponse> {
        let mut sessions = self.sessions.lock().unwrap();
        if let Some(existing) = sessions.get(id) {
            let files = existing
                .files
                .iter()
                .map(|f| protocol::FileStateDto { offset: f.len })
                .collect();
            return Ok(PrepareResponse { session: existing.id.clone(), files });
        }

        let download_dir = core.settings.read().unwrap().download_dir.clone();
        let dir = download_dir.join(".incoming").join(util::sanitize_filename(id));
        std::fs::create_dir_all(&dir)?;
        let mut files = Vec::with_capacity(metas.len());
        for (i, m) in metas.iter().enumerate() {
            let name = util::sanitize_filename(&m.name);
            let tmp_path = dir.join(format!("{i:04}-{name}"));
            files.push(IncomingFile {
                name,
                size: m.size,
                mime: m.mime.clone().unwrap_or_else(|| "application/octet-stream".into()),
                tmp_path,
                len: 0,
            });
        }
        let session = IncomingSession {
            id: id.to_string(),
            peer,
            files,
            dir,
            created: Instant::now(),
            last_touch: Instant::now(),
        };
        let resp = PrepareResponse {
            session: session.id.clone(),
            files: session
                .files
                .iter()
                .map(|f| protocol::FileStateDto { offset: f.len })
                .collect(),
        };
        sessions.insert(id.to_string(), session);
        Ok(resp)
    }

    /// 追加一个分片；返回确认偏移
    pub fn chunk(&self, id: &str, index: usize, offset: u64, data: &[u8]) -> Result<u64> {
        let mut sessions = self.sessions.lock().unwrap();
        let session = sessions
            .get_mut(id)
            .ok_or_else(|| DriftError::Remote { status: 404, message: "传输会话不存在或已超时".into() })?;
        session.last_touch = Instant::now();
        let file = session
            .files
            .get_mut(index)
            .ok_or_else(|| DriftError::Remote { status: 404, message: "文件序号越界".into() })?;
        if offset != file.len {
            return Err(DriftError::Remote { status: 409, message: "分片偏移与接收进度不一致".into() });
        }
        if file.len + data.len() as u64 > file.size {
            return Err(DriftError::Remote { status: 400, message: "超出声明的文件大小".into() });
        }
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file.tmp_path)?;
        f.write_all(data)?;
        file.len += data.len() as u64;
        Ok(file.len)
    }

    /// 查询某文件已接收偏移（续传对账）
    pub fn state(&self, id: &str, index: usize) -> Result<u64> {
        let sessions = self.sessions.lock().unwrap();
        let session = sessions
            .get(id)
            .ok_or_else(|| DriftError::Remote { status: 404, message: "传输会话不存在".into() })?;
        session
            .files
            .get(index)
            .map(|f| f.len)
            .ok_or_else(|| DriftError::Remote { status: 404, message: "文件序号越界".into() })
    }

    /// 完成：落盘到下载目录、生成接收记录、广播事件
    pub fn finish(&self, core: &Arc<Core>, id: &str) -> Result<Record> {
        let session = {
            let mut sessions = self.sessions.lock().unwrap();
            sessions
                .remove(id)
                .ok_or_else(|| DriftError::Remote { status: 404, message: "传输会话不存在".into() })?
        };

        let download_dir = core.settings.read().unwrap().download_dir.clone();
        std::fs::create_dir_all(&download_dir)?;
        let mut files = Vec::with_capacity(session.files.len());
        for f in &session.files {
            if f.len != f.size {
                // 未收全：拒绝完成
                self.sessions
                    .lock()
                    .unwrap()
                    .insert(id.to_string(), re_touch(session.clone()));
                return Err(DriftError::Remote { status: 409, message: "文件未接收完整".into() });
            }
            let target = util::unique_target_path(&download_dir, &f.name);
            std::fs::rename(&f.tmp_path, &target)?;
            files.push(FileEntry {
                name: target
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| f.name.clone()),
                size: f.size,
                mime: f.mime.clone(),
                local_path: Some(target),
                zipped_from_folder: false,
            });
        }
        util::rm_rf(&session.dir);

        let record = Record {
            id: RecordId::new_v4(),
            direction: Direction::Received,
            timestamp: Utc::now(),
            payload: history::Payload::Files { files },
            deliveries: Vec::new(),
            peer: Some(session.peer.clone()),
            deleted: None,
        };
        core.storage.put_record(&record)?;
        let summary = record.summary();
        core.emit(Event::RecordAdded { id: record.id, summary });
        let _ = core.storage.prune_records(core.settings.read().unwrap().max_records);
        Ok(record)
    }

    /// 丢弃会话（发送方取消）
    pub fn drop_session(&self, id: &str) {
        if let Some(session) = self.sessions.lock().unwrap().remove(id) {
            util::rm_rf(&session.dir);
        }
    }

    /// 空闲回收
    pub fn sweep(&self) {
        let stale: Vec<String> = {
            let sessions = self.sessions.lock().unwrap();
            sessions
                .iter()
                .filter(|(_, s)| s.last_touch.elapsed().as_secs() > protocol::INCOMING_IDLE_SECS)
                .map(|(id, _)| id.clone())
                .collect()
        };
        for id in stale {
            self.drop_session(&id);
        }
    }
}

fn re_touch(mut s: IncomingSession) -> IncomingSession {
    s.last_touch = Instant::now();
    s
}

pub fn spawn_incoming_sweeper(handle: &tokio::runtime::Handle, core: Arc<Core>) {
    handle.spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(60));
        loop {
            ticker.tick().await;
            core.incoming.sweep();
        }
    });
}

/// 接收文本：直接生成记录
pub fn accept_text(core: &Arc<Core>, peer: PeerSummary, text: &str) -> Result<Record> {
    let record = Record {
        id: RecordId::new_v4(),
        direction: Direction::Received,
        timestamp: Utc::now(),
        payload: history::Payload::Text { text: text.to_string() },
        deliveries: Vec::new(),
        peer: Some(peer),
        deleted: None,
    };
    core.storage.put_record(&record)?;
    let summary = record.summary();
    core.emit(Event::RecordAdded { id: record.id, summary });
    let _ = core.storage.prune_records(core.settings.read().unwrap().max_records);
    Ok(record)
}
