//! 核心装配与对外 API。
//!
//! GUI 只需要 `Drift::launch(LaunchConfig)` 拿到 [`DriftHandle`]：
//! - `subscribe()` 订阅事件驱动界面刷新；
//! - `nearby() / paired()` 驱动设备条与配对页；
//! - `send_text() / send_paths()` 发起一次发送（返回记录 ID）；
//! - `pause / resume / retry / cancel` 逐设备控制传输；
//! - `records() / delete_record / restore_record / export_csv` 驱动全局历史；
//! - 异步方法配套 `blocking()` 同步包装，适配同步 GUI 框架。

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::{
        atomic::{AtomicU16, Ordering},
        Arc, Mutex, RwLock,
    },
    time::Duration,
};

use chrono::Utc;
use tokio::{net::TcpListener, runtime::Runtime, sync::broadcast};

use crate::{
    discovery::{self, DiscoveryHandle, PairedDevice, PeerDevice},
    error::{DriftError, Result},
    events::{Event, PairedDeviceView, PairingRequest},
    history::{self, Delivery, DeliveryStatus, Direction, FileEntry, PeerSummary, Record, RecordFilter, RecordId},
    identity::{DeviceInfo, DeviceKind, Fingerprint, Identity},
    pairing,
    protocol::{self, DeviceDto},
    settings::Settings,
    storage::Storage,
    transfer::{self, IncomingManager, OutgoingFile, SendPlan, TransferManager},
    util,
};

/// 启动配置
#[derive(Clone, Debug, Default)]
pub struct LaunchConfig {
    /// 数据目录（配置/身份/数据库）；默认系统配置目录下的 `drift`
    pub data_dir: Option<PathBuf>,
    pub name: Option<String>,
    pub kind: Option<DeviceKind>,
    pub download_dir: Option<PathBuf>,
    /// 服务端口（None = 沿用设置 / 首次随机）
    pub port: Option<u16>,
    /// 是否开启 mDNS 发现（None = 沿用设置）
    pub discovery: Option<bool>,
    /// 监听地址；默认全网卡。测试用 127.0.0.1。
    pub bind: Option<IpAddr>,
}

impl LaunchConfig {
    /// 回环 + 不发现：单进程集成测试 / 本地联调用
    pub fn loopback() -> Self {
        LaunchConfig {
            bind: Some(IpAddr::V4(Ipv4Addr::LOCALHOST)),
            discovery: Some(false),
            ..Default::default()
        }
    }
}

/// 内部共享状态
pub struct Core {
    pub identity: Identity,
    pub storage: Storage,
    pub settings: RwLock<Settings>,
    pub config_dir: PathBuf,
    pub events: broadcast::Sender<Event>,
    pub peers: RwLock<HashMap<Fingerprint, PeerDevice>>,
    pub pairings: Mutex<HashMap<Fingerprint, pairing::PairingSession>>,
    pub incoming: IncomingManager,
    pub transfers: TransferManager,
    pub port: AtomicU16,
    pub discovery_handle: Mutex<Option<DiscoveryHandle>>,
}

impl Core {
    pub fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }

    pub fn local_port(&self) -> u16 {
        self.port.load(Ordering::Relaxed)
    }

    pub fn device_dto(&self) -> DeviceDto {
        let s = self.settings.read().unwrap();
        DeviceDto {
            fingerprint: self.identity.fingerprint().to_hex(),
            name: s.device_name.clone(),
            kind: s.device_kind.as_str().to_string(),
            port: self.local_port(),
            version: protocol::PROTOCOL_VERSION,
        }
    }

    pub fn local_info(&self) -> DeviceInfo {
        let s = self.settings.read().unwrap();
        DeviceInfo {
            fingerprint: self.identity.fingerprint(),
            name: s.device_name.clone(),
            kind: s.device_kind,
            port: self.local_port(),
        }
    }

    pub fn is_paired(&self, fp: &Fingerprint) -> bool {
        matches!(self.storage.get_paired(fp), Ok(Some(_)))
    }

    /// 目标设备地址：优先 mDNS 实时地址，退回配对缓存
    pub fn peer_addrs(&self, fp: &Fingerprint) -> Vec<SocketAddr> {
        if let Some(p) = self.peers.read().unwrap().get(fp).filter(|p| !p.addrs.is_empty()) {
            return p.addrs.clone();
        }
        if let Some(p) = self.storage.get_paired(fp).ok().flatten().filter(|p| !p.addrs.is_empty()) {
            return p.addrs;
        }
        Vec::new()
    }

    pub fn peer_summary(&self, fp: &Fingerprint) -> PeerSummary {
        let paired = self.storage.get_paired(fp).ok().flatten();
        let peer = self.peers.read().unwrap().get(fp).cloned();
        PeerSummary {
            fingerprint: *fp,
            name: peer
                .as_ref()
                .map(|p| p.name.clone())
                .or_else(|| paired.as_ref().map(|p| p.name.clone()))
                .unwrap_or_else(|| fp.short()),
            kind: peer
                .as_ref()
                .map(|p| p.kind)
                .or_else(|| paired.as_ref().map(|p| p.kind))
                .unwrap_or(DeviceKind::Other),
        }
    }

    fn apply_settings_patch(&self) {
        let s = self.settings.read().unwrap().clone();
        let _ = s.save(&self.config_dir);
        self.emit(Event::SettingsSaved { settings: s });
    }
}

/// 启动器
pub struct Drift;

impl Drift {
    /// 启动核心：加载/生成身份、打开存储、起 TLS 服务与 mDNS。
    pub fn launch(cfg: LaunchConfig) -> Result<DriftHandle> {
        let config_dir = cfg.data_dir.clone().unwrap_or_else(default_config_dir);
        std::fs::create_dir_all(&config_dir)
            .map_err(|e| DriftError::Dir(format!("创建数据目录失败: {e}")))?;

        let mut settings = Settings::load_or_create(&config_dir)?;
        let mut dirty = false;
        if let Some(n) = &cfg.name {
            settings.device_name = n.clone();
            dirty = true;
        }
        if let Some(k) = cfg.kind {
            settings.device_kind = k;
            dirty = true;
        }
        if let Some(d) = &cfg.download_dir {
            settings.download_dir = d.clone();
            dirty = true;
        }
        if let Some(d) = cfg.discovery {
            settings.discovery = d;
            dirty = true;
        }
        let bind = cfg.bind.unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));

        let storage = Storage::open(&config_dir)?;
        let identity = Identity::load_or_create(&config_dir)?;
        std::fs::create_dir_all(&settings.download_dir).ok();

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("drift-io")
            .enable_all()
            .build()?;

        let (events, _) = broadcast::channel(1024);
        let core = Arc::new(Core {
            identity,
            storage,
            settings: RwLock::new(settings.clone()),
            config_dir: config_dir.clone(),
            events,
            peers: RwLock::new(HashMap::new()),
            pairings: Mutex::new(HashMap::new()),
            incoming: IncomingManager::default(),
            transfers: TransferManager::default(),
            port: AtomicU16::new(0),
            discovery_handle: Mutex::new(None),
        });

        // 绑定端口（设置里 0 = 随机；确定后回写）。
        // 用 std 绑定避免在调用方运行时里 block_on；tokio 化在服务任务内完成。
        let wanted_port = cfg.port.unwrap_or(settings.port);
        let std_listener = std::net::TcpListener::bind(SocketAddr::new(bind, wanted_port))?;
        std_listener.set_nonblocking(true)?;
        let actual_port = std_listener.local_addr()?.port();
        core.port.store(actual_port, Ordering::Relaxed);
        if settings.port != actual_port {
            settings.port = actual_port;
            dirty = true;
        }
        if dirty {
            *core.settings.write().unwrap() = settings.clone();
            let _ = settings.save(&config_dir);
        }

        // TLS 服务
        {
            let core2 = core.clone();
            runtime.spawn(async move {
                let listener = match TcpListener::from_std(std_listener) {
                    Ok(l) => l,
                    Err(e) => {
                        tracing::error!("监听器初始化失败: {e}");
                        return;
                    }
                };
                if let Err(e) = crate::server::serve(core2, listener).await {
                    tracing::error!("服务退出: {e}");
                }
            });
        }

        // mDNS
        if core.settings.read().unwrap().discovery {
            match discovery::spawn(core.clone(), actual_port) {
                Ok(handle) => {
                    *core.discovery_handle.lock().unwrap() = Some(handle);
                    discovery::spawn_offline_sweeper(runtime.handle(), core.clone());
                }
                Err(e) => {
                    core.emit(Event::Error { context: "mDNS 发现启动失败".into(), message: e.to_string() });
                }
            }
        }

        // 定时清扫
        pairing::spawn_sweeper(runtime.handle(), core.clone());
        transfer::spawn_incoming_sweeper(runtime.handle(), core.clone());

        core.emit(Event::ServerStarted { port: actual_port });
        Ok(DriftHandle { core, runtime: Arc::new(Mutex::new(Some(runtime))) })
    }
}

fn default_config_dir() -> PathBuf {
    // Android：dirs 不支持（None → 回退 cwd，而应用进程的 cwd 是只读的 /）
    #[cfg(target_os = "android")]
    {
        if let Some(dir) = crate::settings::android_files_dir() {
            return dir.join("drift");
        }
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("drift")
}

/// GUI 持有的核心句柄（clone 友好：内部全是 Arc）。
#[derive(Clone)]
pub struct DriftHandle {
    pub(crate) core: Arc<Core>,
    pub(crate) runtime: Arc<Mutex<Option<Runtime>>>,
}

impl DriftHandle {
    // ---------- 基础 ----------

    /// 本机设备信息
    pub fn local_device(&self) -> DeviceInfo {
        self.core.local_info()
    }

    /// 订阅事件（可多次调用）
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.core.events.subscribe()
    }

    pub fn settings(&self) -> Settings {
        self.core.settings.read().unwrap().clone()
    }

    /// 修改设备名（立即生效并重新公告 mDNS）
    pub fn set_device_name(&self, name: &str) -> Result<()> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(DriftError::Invalid("设备名不能为空".into()));
        }
        {
            let mut s = self.core.settings.write().unwrap();
            s.device_name = name.clone();
        }
        self.core.apply_settings_patch();
        if let Some(d) = self.core.discovery_handle.lock().unwrap().as_ref() {
            let kind = self.core.settings.read().unwrap().device_kind;
            let _ = d.reannounce(&name, kind);
        }
        Ok(())
    }

    /// 修改接收目录
    pub fn set_download_dir(&self, dir: PathBuf) -> Result<()> {
        std::fs::create_dir_all(&dir).ok();
        {
            let mut s = self.core.settings.write().unwrap();
            s.download_dir = dir;
        }
        self.core.apply_settings_patch();
        Ok(())
    }

    // ---------- 设备与配对 ----------

    /// 附近设备（mDNS 发现 + 手动注入），按名称排序
    pub fn nearby(&self) -> Vec<PeerDevice> {
        let mut list: Vec<PeerDevice> = self.core.peers.read().unwrap().values().cloned().collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    /// 已配对设备（含在线状态）
    pub fn paired(&self) -> Vec<PairedDeviceView> {
        let peers = self.core.peers.read().unwrap();
        self.core
            .storage
            .paired_list()
            .unwrap_or_default()
            .into_iter()
            .map(|p: PairedDevice| {
                let live = peers.get(&p.fingerprint);
                PairedDeviceView {
                    fingerprint: p.fingerprint,
                    name: live.map(|d| d.name.clone()).unwrap_or(p.name),
                    kind: live.map(|d| d.kind.as_str().to_string()).unwrap_or(p.kind.as_str().to_string()),
                    paired_at: p.paired_at,
                    addrs: p.addrs.iter().map(|a| a.to_string()).collect(),
                    online: live.is_some(),
                    last_seen: p.last_seen.or(live.map(|d| d.last_seen)),
                }
            })
            .collect()
    }

    pub fn is_paired(&self, fp: &Fingerprint) -> bool {
        self.core.is_paired(fp)
    }

    /// 向附近表注入一台设备（测试 / 手动加 IP）
    pub fn add_peer(&self, device: PeerDevice) {
        discovery::inject_peer(&self.core, device);
    }

    /// 发起配对：对端与本端同时显示同一组配对码
    pub async fn initiate_pairing(&self, fp: &Fingerprint) -> Result<()> {
        pairing::initiate(&self.core, fp).await
    }

    /// 「一致，配对」
    pub async fn confirm_pairing(&self, fp: &Fingerprint) -> Result<()> {
        pairing::confirm(&self.core, fp).await
    }

    /// 「不一致」（本次作废并通知对端）
    pub async fn reject_pairing(&self, fp: &Fingerprint) -> Result<()> {
        pairing::reject(&self.core, fp).await
    }

    /// 取消等待（仅本地作废）
    pub fn cancel_pairing(&self, fp: &Fingerprint) {
        pairing::cancel(&self.core, fp);
    }

    /// 当前待确认的配对请求
    pub fn pending_pairings(&self) -> Vec<PairingRequest> {
        pairing::pending(&self.core)
    }

    /// 解除配对：本地移除信任并尽力通知对端
    pub async fn unpair(&self, fp: &Fingerprint) -> Result<()> {
        let addrs = self.core.peer_addrs(fp);
        let removed = self.core.storage.remove_paired(fp)?;
        if removed.is_some() {
            self.core.emit(Event::DeviceUnpaired { fingerprint: *fp });
        }
        if !addrs.is_empty() {
            let link = crate::client::PeerLink::new(*fp, addrs, self.core.identity.tls_material());
            let _ = link
                .send_json::<protocol::ApiResponse>(
                    axum::http::Method::POST,
                    protocol::path::UNPAIR,
                    &serde_json::json!({}),
                )
                .await;
        }
        Ok(())
    }

    // ---------- 发送 ----------

    /// 发送文本（一次发送一条记录，逐设备投递）
    pub async fn send_text(&self, text: String, targets: &[Fingerprint]) -> Result<RecordId> {
        if text.trim().is_empty() {
            return Err(DriftError::Invalid("文本内容为空".into()));
        }
        if text.len() > protocol::MAX_TEXT_LEN {
            return Err(DriftError::Invalid(format!(
                "文本超过上限 {}",
                util::human_bytes(protocol::MAX_TEXT_LEN as u64)
            )));
        }
        let targets = self.validate_targets(targets)?;
        let id = RecordId::new_v4();
        let total = text.len() as u64;
        let deliveries: Vec<Delivery> = targets
            .iter()
            .map(|fp| Delivery {
                device: self.core.peer_summary(fp),
                status: DeliveryStatus::Pending,
                transferred: 0,
                total,
                speed_bps: 0,
                error: None,
            })
            .collect();
        let record = Record {
            id,
            direction: Direction::Sent,
            timestamp: Utc::now(),
            payload: history::Payload::Text { text: text.clone() },
            deliveries,
            peer: None,
            deleted: None,
        };
        self.core.storage.put_record(&record)?;
        let summary = record.summary();
        self.core.emit(Event::RecordAdded { id, summary });

        for fp in targets {
            self.spawn_job(id, fp, SendPlan::Text(text.clone()));
        }
        self.prune();
        Ok(id)
    }

    /// 发送文件/文件夹（文件夹自动压缩为 zip；一次发送一条记录）
    pub async fn send_paths(&self, paths: Vec<PathBuf>, targets: &[Fingerprint]) -> Result<RecordId> {
        if paths.is_empty() {
            return Err(DriftError::Invalid("未选择任何文件".into()));
        }
        if paths.len() > 512 {
            return Err(DriftError::Invalid("一次最多发送 512 个文件".into()));
        }
        let targets = self.validate_targets(targets)?;

        // 收集/压缩（压缩放阻塞线程池）
        let mut entries: Vec<FileEntry> = Vec::new();
        let mut outgoing: Vec<OutgoingFile> = Vec::new();
        let mut temp_files: Option<PathBuf> = None;
        for path in &paths {
            let meta = std::fs::metadata(path)?;
            if meta.is_dir() {
                let zipped = {
                    let dir_name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "folder".into());
                    let tmp = std::env::temp_dir().join(format!(
                        "drift-{}-{dir_name}.zip",
                        Utc::now().timestamp_millis()
                    ));
                    let size = tokio::task::spawn_blocking({
                        let src = path.clone();
                        let tmp = tmp.clone();
                        move || util::zip_dir(&src, &tmp)
                    })
                    .await
                    .map_err(|e| DriftError::Zip(format!("压缩任务失败: {e}")))??;
                    (tmp, size)
                };
                let (tmp, size) = zipped;
                let display_name = format!(
                    "{}.zip",
                    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "folder".into())
                );
                entries.push(FileEntry {
                    name: display_name.clone(),
                    size,
                    mime: "application/zip".into(),
                    local_path: Some(path.clone()),
                    zipped_from_folder: true,
                });
                outgoing.push(OutgoingFile { src_path: tmp.clone(), name: display_name, size, mime: "application/zip".into() });
                temp_files = Some(tmp); // 多个文件夹时全部进 TransferManager 清理列表
            } else {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "file".into());
                let size = meta.len();
                let mime = mime_guess::from_path(path)
                    .first_raw()
                    .unwrap_or("application/octet-stream")
                    .to_string();
                entries.push(FileEntry {
                    name: name.clone(),
                    size,
                    mime: mime.clone(),
                    local_path: Some(path.clone()),
                    zipped_from_folder: false,
                });
                outgoing.push(OutgoingFile { src_path: path.clone(), name, size, mime });
            }
        }
        self.dispatch_files(targets, entries, outgoing, temp_files)
    }

    /// 发送内存字节（剪贴板图片等）：落临时文件后按文件发送，结束后自动清理。
    pub async fn send_blob(
        &self,
        name: &str,
        mime: Option<String>,
        bytes: Vec<u8>,
        targets: &[Fingerprint],
    ) -> Result<RecordId> {
        if bytes.is_empty() {
            return Err(DriftError::Invalid("内容为空".into()));
        }
        let targets = self.validate_targets(targets)?;
        let name = util::sanitize_filename(name);
        let mime = mime.unwrap_or_else(|| {
            mime_guess::from_path(&name)
                .first_raw()
                .unwrap_or("application/octet-stream")
                .to_string()
        });
        let tmp = std::env::temp_dir().join(format!(
            "drift-blob-{}-{name}",
            Utc::now().timestamp_millis()
        ));
        std::fs::write(&tmp, &bytes)?;
        let size = bytes.len() as u64;
        let entry = FileEntry {
            name: name.clone(),
            size,
            mime: mime.clone(),
            local_path: None,
            zipped_from_folder: false,
        };
        let outgoing = OutgoingFile { src_path: tmp.clone(), name, size, mime };
        self.dispatch_files(targets, vec![entry], vec![outgoing], Some(tmp))
    }

    /// 建记录 + 按设备派发文件发送任务
    fn dispatch_files(
        &self,
        targets: Vec<Fingerprint>,
        entries: Vec<FileEntry>,
        outgoing: Vec<OutgoingFile>,
        temp_files: Option<PathBuf>,
    ) -> Result<RecordId> {
        let id = RecordId::new_v4();
        let total: u64 = outgoing.iter().map(|f| f.size).sum();
        let deliveries: Vec<Delivery> = targets
            .iter()
            .map(|fp| Delivery {
                device: self.core.peer_summary(fp),
                status: DeliveryStatus::Pending,
                transferred: 0,
                total,
                speed_bps: 0,
                error: None,
            })
            .collect();
        let record = Record {
            id,
            direction: Direction::Sent,
            timestamp: Utc::now(),
            payload: history::Payload::Files { files: entries },
            deliveries,
            peer: None,
            deleted: None,
        };
        self.core.storage.put_record(&record)?;
        let summary = record.summary();
        self.core.emit(Event::RecordAdded { id, summary });

        let plan = SendPlan::Files { files: outgoing, temp_files };
        for fp in targets {
            self.spawn_job(id, fp, plan.clone());
        }
        self.prune();
        Ok(id)
    }

    fn validate_targets(&self, targets: &[Fingerprint]) -> Result<Vec<Fingerprint>> {
        if targets.is_empty() {
            return Err(DriftError::Invalid("未选择接收设备".into()));
        }
        for fp in targets {
            if !self.core.is_paired(fp) {
                return Err(DriftError::NotPaired(self.core.peer_summary(fp).name));
            }
        }
        Ok(targets.to_vec())
    }

    fn spawn_job(&self, record: RecordId, target: Fingerprint, plan: SendPlan) {
        let addrs = self.core.peer_addrs(&target);
        let summary = self.core.peer_summary(&target);
        self.core
            .transfers
            .spawn_job(self.core.clone(), record, summary, addrs, plan);
    }

    // ---------- 传输控制（逐设备） ----------

    pub fn pause(&self, record: &RecordId, target: &Fingerprint) -> Result<()> {
        self.core.transfers.pause(record, target)
    }

    pub fn resume(&self, record: &RecordId, target: &Fingerprint) -> Result<()> {
        self.core.transfers.resume(record, target)
    }

    /// 手动重试（失败 / 等待重试后）
    pub fn retry(&self, record: &RecordId, target: &Fingerprint) -> Result<()> {
        self.core.transfers.retry(record, target)
    }

    pub fn cancel_target(&self, record: &RecordId, target: &Fingerprint) -> Result<()> {
        self.core.transfers.cancel(record, target)
    }

    /// 取消整条记录的全部目标
    pub fn cancel_record(&self, record: &RecordId) -> usize {
        self.core.transfers.cancel_record(record)
    }

    // ---------- 历史 ----------

    /// 按筛选取记录（新 → 旧）
    pub fn records(&self, filter: &RecordFilter) -> Result<Vec<Record>> {
        history::filter_records(&self.core.storage, filter)
    }

    pub fn record(&self, id: &RecordId) -> Option<Record> {
        self.core.storage.get_record(id).ok().flatten()
    }

    /// 软删除（不删本地文件；可恢复）
    pub fn delete_record(&self, id: &RecordId) -> Result<bool> {
        let ok = history::soft_delete(&self.core.storage, id)?;
        if ok {
            self.core.emit(Event::RecordDeleted { id: *id });
        }
        Ok(ok)
    }

    /// 恢复删除（Ctrl / Cmd + Z）
    pub fn restore_record(&self, id: &RecordId) -> Result<bool> {
        let ok = history::restore(&self.core.storage, id)?;
        if ok {
            self.core.emit(Event::RecordRestored { id: *id });
        }
        Ok(ok)
    }

    /// 导出 CSV（带 BOM）
    pub fn export_csv(&self, path: PathBuf, filter: &RecordFilter) -> Result<usize> {
        history::export_csv(&self.core.storage, &path, filter)
    }

    /// 文本记录导出 .txt / .md
    pub fn export_text(
        &self,
        id: &RecordId,
        path: PathBuf,
        fmt: history::TextExportFormat,
    ) -> Result<()> {
        history::export_text(&self.core.storage, id, &path, fmt)
    }

    /// 记录中第 idx 个文件的本地路径（另存为 / 打开 / 显示位置）
    pub fn file_path(&self, id: &RecordId, idx: usize) -> Option<PathBuf> {
        history::file_path(&self.core.storage, id, idx).ok().flatten()
    }

    fn prune(&self) {
        let max = self.core.settings.read().unwrap().max_records;
        let _ = self.core.storage.prune_records(max);
    }

    // ---------- 同步包装与关闭 ----------

    /// 阻塞包装：给同步 GUI（egui 等）使用。不能在 tokio 运行时线程里调用。
    pub fn blocking(&self) -> BlockingHandle<'_> {
        BlockingHandle { handle: self }
    }

    /// 显式优雅关闭（非阻塞：内部把运行时关停放到独立线程，
    /// 因此既能在同步 GUI 主线程调用，也能在异步上下文调用）
    pub fn shutdown(self) {
        if let Some(d) = self.core.discovery_handle.lock().unwrap().as_ref() {
            d.shutdown();
        }
        let rt = self.runtime.lock().unwrap().take();
        park_and_shutdown(rt, Duration::from_secs(3));
    }
}

/// 在独立线程里关停运行时（Runtime 的 Drop/阻塞关闭不允许出现在异步上下文）
fn park_and_shutdown(rt: Option<Runtime>, timeout: Duration) {
    if let Some(rt) = rt {
        let _ = std::thread::Builder::new()
            .name("drift-rt-shutdown".into())
            .spawn(move || rt.shutdown_timeout(timeout));
    }
}

/// 同步方法包装（内部 `Runtime::block_on`）
pub struct BlockingHandle<'a> {
    handle: &'a DriftHandle,
}

impl BlockingHandle<'_> {
    pub fn send_text(&self, text: String, targets: &[Fingerprint]) -> Result<RecordId> {
        self.with_rt(|rt| rt.block_on(self.handle.send_text(text, targets)))
    }

    pub fn send_paths(&self, paths: Vec<PathBuf>, targets: &[Fingerprint]) -> Result<RecordId> {
        self.with_rt(|rt| rt.block_on(self.handle.send_paths(paths, targets)))
    }

    pub fn initiate_pairing(&self, fp: &Fingerprint) -> Result<()> {
        self.with_rt(|rt| rt.block_on(self.handle.initiate_pairing(fp)))
    }

    pub fn confirm_pairing(&self, fp: &Fingerprint) -> Result<()> {
        self.with_rt(|rt| rt.block_on(self.handle.confirm_pairing(fp)))
    }

    pub fn reject_pairing(&self, fp: &Fingerprint) -> Result<()> {
        self.with_rt(|rt| rt.block_on(self.handle.reject_pairing(fp)))
    }

    pub fn unpair(&self, fp: &Fingerprint) -> Result<()> {
        self.with_rt(|rt| rt.block_on(self.handle.unpair(fp)))
    }

    fn with_rt<T>(&self, f: impl FnOnce(&Runtime) -> T) -> T {
        let guard = self.handle.runtime.lock().unwrap();
        let rt = guard.as_ref().expect("drift 运行时已关闭");
        f(rt)
    }
}

impl Drop for DriftHandle {
    fn drop(&mut self) {
        // 未显式 shutdown 时的兜底（clone 的句柄不触发：Arc 引用计数）
        if Arc::strong_count(&self.core) == 1 {
            if let Some(d) = self.core.discovery_handle.lock().unwrap().as_ref() {
                d.shutdown();
            }
            let rt = self.runtime.lock().unwrap().take();
            park_and_shutdown(rt, Duration::from_secs(1));
        }
    }
}
