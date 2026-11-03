//! 核心 ↔ UI 的桥：启动 Drift、事件泵、全部界面信号与动作。
//!
//! 线程约定：核心同步方法随处可调；异步方法（send_* / 配对 / unpair）
//! 一律 `spawn(async move { h.x().await })`，绝不在引擎异步上下文用 `blocking()`。

use std::{path::PathBuf, rc::Rc, time::Duration};

use chrono::{DateTime, Local, Utc};
use drift::{
    DeliveryStatus, DeviceInfo, Event, Fingerprint, PairedDeviceView, PairingRequest, PayloadKind,
    Record, RecordFilter, RecordId, Settings,
    discovery::{PeerDevice, PeerSource},
};
use nsh_core::LayoutAnimation;
use nsh_core::prelude::*;
use tokio::sync::broadcast::error::RecvError;

/// drift 的数据类型没有 PartialEq：每次 set 都视为变化。
fn data_signal<T: 'static>(value: T) -> Signal<T> {
    signal_with(value, SignalEquals::Never)
}

/// 让没有 PartialEq 的类型能进 `each`：永远「不相等」，
/// 于是每次列表更新都会把新值写进各行的 ReadSignal（行本身按 key 保留）。
#[derive(Clone, Debug)]
pub struct NeverEq<T>(pub T);

impl<T> PartialEq for NeverEq<T> {
    fn eq(&self, _: &Self) -> bool {
        false
    }
}

/// 输入区附件（已选/已拖入待发送的文件或文件夹）。
#[derive(Clone, Debug, PartialEq)]
pub struct Attachment {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
}

/// 当前输入会话中，某台设备最近一次已发送的内容快照（用于判断内容变更与计算增量）。
#[derive(Clone, Debug, PartialEq)]
pub struct SentSnapshot {
    pub text: String,
    pub files: Vec<PathBuf>,
    /// 该设备在当前输入会话中已发送的次数（1 = 已发首版，2+ = 已增量同步 vN）
    pub version: u32,
}

/// 当前输入内容相对于某台设备上次发送快照的差异描述。
#[derive(Clone, Debug, PartialEq)]
pub enum ContentDelta {
    /// 文本在原末尾追加了新内容（可补发新增部分）
    AppendedText {
        added_chars: usize,
        delta_text: String,
    },
    /// 文本被编辑修改
    ModifiedText {
        total_chars: usize,
    },
    /// 在原有附件基础上新增了文件（可仅增量发送新增文件）
    AddedFiles {
        added_files: Vec<PathBuf>,
    },
    /// 附件或图文组合发生了变更
    ModifiedPayload,
}

impl SentSnapshot {
    /// 比较当前输入区内容与该设备上次发送快照；返回 `None` 表示内容未变（已同步），
    /// 返回 `Some(ContentDelta)` 表示内容已变更，可对该设备发起增量发送。
    pub fn diff(&self, cur_text: &str, cur_files: &[Attachment]) -> Option<ContentDelta> {
        let cur_t = cur_text.trim();
        let snap_t = self.text.trim();
        let cur_paths: Vec<PathBuf> = cur_files.iter().map(|a| a.path.clone()).collect();
        if cur_t == snap_t && cur_paths == self.files {
            return None;
        }
        if !cur_paths.is_empty() && cur_t == snap_t {
            let all_old_kept = self.files.iter().all(|old| cur_paths.contains(old));
            if all_old_kept && cur_paths.len() > self.files.len() {
                let added_files: Vec<PathBuf> = cur_paths
                    .into_iter()
                    .filter(|p| !self.files.contains(p))
                    .collect();
                if !added_files.is_empty() {
                    return Some(ContentDelta::AddedFiles { added_files });
                }
            }
            return Some(ContentDelta::ModifiedPayload);
        }
        if cur_paths.is_empty() && self.files.is_empty() {
            if !snap_t.is_empty() && cur_t.starts_with(snap_t) {
                let raw_tail = &cur_t[snap_t.len()..];
                let clean_tail = raw_tail
                    .trim_start_matches(|c: char| {
                        c.is_whitespace() || matches!(c, '，' | ',' | '。' | '；' | ';' | '、')
                    })
                    .trim();
                let delta_text = if clean_tail.is_empty() {
                    raw_tail.trim().to_string()
                } else {
                    clean_tail.to_string()
                };
                let added_chars = cur_t
                    .chars()
                    .count()
                    .saturating_sub(snap_t.chars().count())
                    .max(1);
                return Some(ContentDelta::AppendedText {
                    added_chars,
                    delta_text,
                });
            }
            return Some(ContentDelta::ModifiedText {
                total_chars: cur_t.chars().count(),
            });
        }
        Some(ContentDelta::ModifiedPayload)
    }
}

impl ContentDelta {
    /// 设备卡片副标题文案。
    pub fn summary_label(&self) -> String {
        match self {
            ContentDelta::AppendedText { added_chars, .. } => {
                format!("内容新增 {added_chars} 字 · 点按补发增量")
            }
            ContentDelta::ModifiedText { total_chars } => {
                format!("内容已改（{total_chars} 字）· 点按增量更新")
            }
            ContentDelta::AddedFiles { added_files } => {
                format!("新增 {} 个文件 · 点按仅补发新文件", added_files.len())
            }
            ContentDelta::ModifiedPayload => "内容已变更 · 点按增量同步".to_string(),
        }
    }

    /// 紧凑胶囊文案（顶栏 / 输入区提示）。
    pub fn badge_label(&self) -> String {
        match self {
            ContentDelta::AppendedText { added_chars, .. } => format!("新增 +{added_chars} 字"),
            ContentDelta::ModifiedText { .. } => "文本已改".to_string(),
            ContentDelta::AddedFiles { added_files } => {
                format!("新增 +{} 文件", added_files.len())
            }
            ContentDelta::ModifiedPayload => "内容已改".to_string(),
        }
    }
}

/// 设备球体画布的状态（drift-main-desktop / drift-main-mobile v1.5）。
///
/// 平移与缩放是平台原生手势驱动（拖空白处 / 双指捏合；桌面 Ctrl+滚轮），
/// AnimatedValue 挂在 AppState 上，画布收起再浮出视角不重置。
pub struct CanvasState {
    /// 画布平移（世界坐标 → 视口，拖动跟手）
    pub pan_x: nsh_core::prelude::AnimatedValue,
    pub pan_y: nsh_core::prelude::AnimatedValue,
    /// 画布缩放（1.0 = 原大；捏合 / Ctrl+滚轮驱动）
    pub zoom: nsh_core::prelude::AnimatedValue,
    /// 球体落位顺序：设备首次出现在画布上时追加（黄金角螺旋取位），稳定不变
    pub order: Signal<Vec<Fingerprint>>,
    /// 视口轮询快照 (pan_x, pan_y, zoom)：驱动小地图与「n/m」计数
    pub view: Signal<(f32, f32, f32)>,
}

/// 历史筛选（时间线的 tabs + 设备 + 关键词）。
#[derive(Clone, Debug, PartialEq)]
pub enum FilterKind {
    All,
    File,
    Image,
    Text,
}

impl FilterKind {
    pub fn kinds(&self) -> Option<Vec<PayloadKind>> {
        match self {
            FilterKind::All => None,
            FilterKind::File => Some(vec![PayloadKind::File]),
            FilterKind::Image => Some(vec![PayloadKind::Image]),
            FilterKind::Text => Some(vec![PayloadKind::Text]),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            FilterKind::All => "全部",
            FilterKind::File => "文件",
            FilterKind::Image => "图片",
            FilterKind::Text => "文本",
        }
    }
}

/// 剪贴板建议。
#[derive(Clone, Debug, Default, PartialEq)]
pub enum ClipHint {
    #[default]
    None,
    Text { chars: usize },
    Image,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Toast {
    pub id: u64,
    pub message: String,
    pub kind: ToastKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

/// 全局界面状态。根组件里创建一次，经 `provide_context` 供给全部子组件。
pub struct AppState {
    pub handle: drift::DriftHandle,
    pub local: DeviceInfo,
    pub settings: Signal<Settings>,

    // —— 数据信号 ——
    pub nearby: Signal<Vec<PeerDevice>>,
    pub paired: Signal<Vec<PairedDeviceView>>,
    pub records: Signal<Vec<Record>>,
    pub pairings: Signal<Vec<PairingRequest>>,

    // —— 界面信号 ——
    /// 配对面板是否打开（手动 / PairingIncoming）
    pub pairing_open: Signal<bool>,
    /// 剪贴板建议（输入区为空时探测）
    pub clip_hint: Signal<ClipHint>,
    /// 输入区
    pub draft: Signal<String>,
    pub attachments: Signal<Vec<Attachment>>,
    /// 当前输入会话中各设备已发送的内容快照（输入清空时重置）
    pub sent_snapshots: Signal<std::collections::HashMap<Fingerprint, SentSnapshot>>,
    /// 设备球体画布（输入有内容时浮出）
    pub canvas: CanvasState,
    /// 文件拖放悬停中（整窗）
    pub dragging: Signal<bool>,
    /// 历史筛选
    pub filter_kind: Signal<FilterKind>,
    pub filter_device: Signal<Option<Fingerprint>>,
    pub filter_query: Signal<String>,
    /// 搜索框展开
    pub search_open: Signal<bool>,
    /// 新记录进入动画：最近一次新增的记录 id
    pub last_added: Signal<Option<RecordId>>,
    /// 刚完成的记录（整卡高亮 320ms，只播一次）
    pub flash_done: Signal<Option<RecordId>>,
    /// 撤销删除：待恢复记录（5s / Ctrl+Z）
    pub undo_pending: Signal<Option<RecordId>>,
    /// 轻提示
    pub toasts: Signal<Vec<Toast>>,
}

impl AppState {
    /// 启动核心并建立事件泵。失败返回错误文案（根组件渲染错误屏）。
    /// 开发联调：`DRIFT_DATA_DIR` / `DRIFT_NAME` 覆盖数据目录与设备名（双实例同机测试）。
    pub fn start() -> Result<Rc<AppState>, String> {
        let mut config = drift::LaunchConfig::default();
        if let Ok(dir) = std::env::var("DRIFT_DATA_DIR") {
            config.data_dir = Some(std::path::PathBuf::from(dir));
        }
        if let Ok(name) = std::env::var("DRIFT_NAME") {
            config.name = Some(name);
        }
        let handle = drift::Drift::launch(config).map_err(|e| e.to_string())?;
        let local = handle.local_device();
        let settings = handle.settings();

        let state = Rc::new(AppState {
            settings: data_signal(settings),
            nearby: data_signal(Vec::new()),
            paired: data_signal(Vec::new()),
            records: data_signal(Vec::new()),
            pairings: data_signal(Vec::new()),
            pairing_open: signal(false),
            clip_hint: signal(ClipHint::None),
            draft: signal(String::new()),
            attachments: signal(Vec::new()),
            sent_snapshots: signal(std::collections::HashMap::new()),
            canvas: CanvasState {
                pan_x: nsh_core::prelude::animated_value(0.0),
                pan_y: nsh_core::prelude::animated_value(0.0),
                zoom: nsh_core::prelude::animated_value(1.0),
                order: signal(Vec::new()),
                view: signal((0.0, 0.0, 1.0)),
            },
            dragging: signal(false),
            filter_kind: signal(FilterKind::All),
            filter_device: signal(None),
            filter_query: signal(String::new()),
            search_open: signal(false),
            last_added: signal(None),
            flash_done: signal(None),
            undo_pending: signal(None),
            toasts: signal(Vec::new()),
            handle,
            local,
        });

        state.refresh_all();
        state.spawn_pump();
        state.spawn_canvas_poll();
        state.dev_peer_exchange();
        // 开发联调：启动时把指定文件加入附件（验证文件发送链路）
        if let Ok(file) = std::env::var("DRIFT_DEV_ATTACH") {
            state.add_paths(vec![PathBuf::from(file)]);
        }
        Ok(state)
    }

    /// 开发联调（同机双实例时 mDNS 不可靠）：DRIFT_PEER_FILE 指定一个交换文件，
    /// 各实例把自己的 DeviceInfo 追加为一行 JSON，并定期注入文件里出现的对端
    /// （等价于 tests/integration.rs 的回环互注）。
    fn dev_peer_exchange(&self) {
        let Ok(path) = std::env::var("DRIFT_PEER_FILE") else { return };
        let path = PathBuf::from(path);
        // 写入自己的信息（幂等：先读出已有行，避免重复追加）
        let local = serde_json::to_string(&self.local).unwrap_or_default();
        let existing = std::fs::read_to_string(&path).unwrap_or_default();
        if !existing.lines().any(|l| l == local) {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                let _ = writeln!(f, "{local}");
            }
        }
        // 定期读取并注入新对端
        let handle = self.handle.clone();
        let local_fp = self.local.fingerprint;
        let seen: std::rc::Rc<std::cell::RefCell<std::collections::HashSet<Fingerprint>>> =
            std::rc::Rc::new(std::cell::RefCell::new(std::collections::HashSet::new()));
        seen.borrow_mut().insert(local_fp);
        set_interval(Duration::from_millis(800), move || {
            let Ok(text) = std::fs::read_to_string(&path) else { return };
            for line in text.lines() {
                let Ok(info) = serde_json::from_str::<DeviceInfo>(line) else { continue };
                if seen.borrow().contains(&info.fingerprint) {
                    continue;
                }
                seen.borrow_mut().insert(info.fingerprint);
                handle.add_peer(PeerDevice {
                    fingerprint: info.fingerprint,
                    name: info.name,
                    kind: info.kind,
                    port: info.port,
                    addrs: vec![std::net::SocketAddr::from((
                        std::net::Ipv4Addr::LOCALHOST,
                        info.port,
                    ))],
                    last_seen: Utc::now(),
                    source: PeerSource::Manual,
                });
            }
        });
    }

    fn current_filter(&self) -> RecordFilter {
        RecordFilter {
            kinds: self.filter_kind.get_untracked().kinds(),
            device: self.filter_device.get_untracked(),
            query: {
                let q = self.filter_query.get_untracked();
                if q.trim().is_empty() { None } else { Some(q) }
            },
            ..Default::default()
        }
    }

    /// 全量刷新（启动 / Lagged 后）。
    pub fn refresh_all(&self) {
        self.nearby.set(self.handle.nearby());
        self.paired.set(self.handle.paired());
        self.ensure_orb_order();
        self.records.set(self.handle.records(&self.current_filter()).unwrap_or_default());
        self.pairings.set(self.handle.pending_pairings());
        self.settings.set(self.handle.settings());
        if !self.pairings.get_untracked().is_empty() {
            self.pairing_open.set(true);
        }
    }

    /// 筛选变化后刷新记录。
    pub fn refresh_records(&self) {
        self.records.set(self.handle.records(&self.current_filter()).unwrap_or_default());
    }

    fn spawn_pump(self: &Rc<Self>) {
        let pump = Pump {
            handle: self.handle.clone(),
            nearby: self.nearby.clone(),
            paired: self.paired.clone(),
            pairings: self.pairings.clone(),
            pairing_open: self.pairing_open.clone(),
            last_added: self.last_added.clone(),
            flash_done: self.flash_done.clone(),
            records: self.records.clone(),
            filter_kind: self.filter_kind.clone(),
            filter_device: self.filter_device.clone(),
            filter_query: self.filter_query.clone(),
            toasts: self.toasts.clone(),
            orb_order: self.canvas.order.clone(),
            completed: std::cell::RefCell::new(std::collections::HashSet::new()),
        };
        let mut rx = self.handle.subscribe();
        spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => pump.apply(event),
                    Err(RecvError::Lagged(_)) => pump.refresh_all(),
                    Err(RecvError::Closed) => break,
                }
            }
        });
    }

    // —— 设备球体画布 ——

    /// 为新出现的已配对设备分配稳定落位（黄金角螺旋）；已落位设备不动。
    /// 事件泵在设备/配对事件后调用；画布挂载时兜底再调一次。
    pub fn ensure_orb_order(&self) {
        self.canvas.order.update(|order| {
            for d in self.paired.get_untracked() {
                if d.online && !order.contains(&d.fingerprint) {
                    order.push(d.fingerprint);
                }
            }
        });
    }

    /// 把当前 pan/zoom 动画值读进 `canvas.view`（小地图 / 可见计数用）。
    /// 动画值只能异步读：丢进 spawn，晚一帧无妨。
    pub fn sync_canvas_view(&self) {
        let pan_x = self.canvas.pan_x.clone();
        let pan_y = self.canvas.pan_y.clone();
        let zoom = self.canvas.zoom.clone();
        let view = self.canvas.view.clone();
        spawn(async move {
            let x = pan_x.value().await;
            let y = pan_y.value().await;
            let z = zoom.value().await;
            view.set((x, y, z));
        });
    }

    /// 画布浮出期间轮询视口（120ms）：原生手势连续驱动动画值，
    /// 组件侧没有逐帧回调，小地图用快照跟随。
    fn spawn_canvas_poll(self: &Rc<Self>) {
        let state = self.clone();
        set_interval(Duration::from_millis(120), move || {
            if state.has_send_content() {
                state.sync_canvas_view();
            }
        });
    }

    // —— 动作：发送与增量同步 ——

    /// 当前输入区是否有可发送内容。
    pub fn has_send_content(&self) -> bool {
        !self.draft.get().trim().is_empty() || !self.attachments.get().is_empty()
    }

    /// 更新输入文本：内容非空时设备球体浮出；内容与附件均清空时收起球体并重置已发快照。
    pub fn update_draft(&self, text: String) {
        let is_empty = text.trim().is_empty();
        self.draft.set(text);
        if is_empty && self.attachments.get_untracked().is_empty() {
            self.sent_snapshots.set(std::collections::HashMap::new());
            self.probe_clipboard();
        }
    }

    /// 清空输入区全部内容，球体收起，回到传输会话。
    pub fn clear_composer(&self) {
        self.draft.set(String::new());
        self.attachments.set(Vec::new());
        self.sent_snapshots.set(std::collections::HashMap::new());
    }

    /// 把输入区当前内容（或变更后的增量内容）发给 `targets`：
    /// - 发送后保留输入区内容，以便继续点选其他设备或编辑内容后增量发送；
    /// - 若目标设备在本次会话中已发送过旧版内容，则按增量策略补发新增文件或新增/更新后的文本，并递推版本号。
    pub fn send_to(&self, targets: &[Fingerprint]) {
        if targets.is_empty() {
            return;
        }
        let cur_attachments = self.attachments.get_untracked();
        let files: Vec<PathBuf> = cur_attachments.iter().map(|a| a.path.clone()).collect();
        let text = self.draft.get_untracked();
        if files.is_empty() && text.trim().is_empty() {
            return;
        }

        let prev_map = self.sent_snapshots.get_untracked();
        let mut jobs: Vec<(Fingerprint, Vec<PathBuf>, String, bool)> = Vec::new();

        self.sent_snapshots.update(|map| {
            for &fp in targets {
                if let Some(prev) = prev_map.get(&fp) {
                    match prev.diff(&text, &cur_attachments) {
                        None => {
                            // 内容未变，无需重复发送
                            continue;
                        }
                        Some(ContentDelta::AddedFiles { added_files }) => {
                            jobs.push((fp, added_files, String::new(), true));
                        }
                        Some(ContentDelta::AppendedText { delta_text, .. })
                            if files.is_empty() && !delta_text.trim().is_empty() =>
                        {
                            jobs.push((fp, Vec::new(), delta_text, true));
                        }
                        Some(_) => {
                            jobs.push((fp, files.clone(), text.clone(), true));
                        }
                    }
                    map.insert(
                        fp,
                        SentSnapshot {
                            text: text.clone(),
                            files: files.clone(),
                            version: prev.version + 1,
                        },
                    );
                } else {
                    jobs.push((fp, files.clone(), text.clone(), false));
                    map.insert(
                        fp,
                        SentSnapshot {
                            text: text.clone(),
                            files: files.clone(),
                            version: 1,
                        },
                    );
                }
            }
        });

        if jobs.is_empty() {
            self.toast("已同步最新内容，修改输入后可增量补发", ToastKind::Info);
            return;
        }

        let handle = self.handle.clone();
        let toasts = self.toasts.clone();
        spawn(async move {
            for (fp, job_files, job_text, is_incremental) in jobs {
                let single = [fp];
                let result = if !job_files.is_empty() {
                    handle.send_paths(job_files, &single).await.map(|_| ())
                } else {
                    handle.send_text(job_text, &single).await.map(|_| ())
                };
                match result {
                    Ok(()) if is_incremental => {
                        push_toast(&toasts, "已发起增量发送".to_string(), ToastKind::Success);
                    }
                    Err(e) => {
                        push_toast(&toasts, e.to_string(), ToastKind::Error);
                    }
                    _ => {}
                }
            }
        });
    }

    /// 读取剪贴板图片并发送（Android 不支持，会静默跳过）。
    pub fn send_clipboard_image(&self, targets: &[Fingerprint]) {
        if targets.is_empty() {
            return;
        }
        let handle = self.handle.clone();
        let toasts = self.toasts.clone();
        let targets = targets.to_vec();
        spawn(async move {
            let Some(img) = clipboard_image().await else {
                return;
            };
            let resp = match fetch(img.url.clone()).await {
                Ok(resp) => resp,
                Err(_) => return,
            };
            let bytes = resp.body;
            if bytes.is_empty() {
                return;
            }
            if let Err(e) =
                handle.send_blob("剪贴板图片.png", Some("image/png".into()), bytes, &targets).await
            {
                push_toast(&toasts, e.to_string(), ToastKind::Error);
            }
        });
    }

    // —— 动作：附件 ——

    /// 加入附件（拖放 / 选择 / 粘贴的文件路径）；输入区有内容，设备球体即浮出。
    pub fn add_paths(&self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        self.attachments.update(|list| {
            for path in paths {
                let (size, is_dir) =
                    std::fs::metadata(&path).map(|m| (m.len(), m.is_dir())).unwrap_or((0, false));
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_string_lossy().into_owned());
                list.push(Attachment { path, name, size, is_dir });
            }
        });
    }

    pub fn remove_attachment(&self, index: usize) {
        self.attachments.update(|list| {
            if index < list.len() {
                list.remove(index);
            }
        });
        if self.attachments.get_untracked().is_empty()
            && self.draft.get_untracked().trim().is_empty()
        {
            self.sent_snapshots.set(std::collections::HashMap::new());
        }
    }

    // —— 动作：配对 ——

    pub fn initiate_pairing(&self, fp: Fingerprint) {
        let handle = self.handle.clone();
        let toasts = self.toasts.clone();
        spawn(async move {
            if let Err(e) = handle.initiate_pairing(&fp).await {
                push_toast(&toasts, e.to_string(), ToastKind::Error);
            }
        });
    }

    pub fn confirm_pairing(&self, fp: Fingerprint) {
        let handle = self.handle.clone();
        let toasts = self.toasts.clone();
        spawn(async move {
            if let Err(e) = handle.confirm_pairing(&fp).await {
                push_toast(&toasts, e.to_string(), ToastKind::Error);
            }
        });
    }

    pub fn reject_pairing(&self, fp: Fingerprint) {
        let handle = self.handle.clone();
        let toasts = self.toasts.clone();
        spawn(async move {
            if let Err(e) = handle.reject_pairing(&fp).await {
                push_toast(&toasts, e.to_string(), ToastKind::Error);
            }
        });
    }

    pub fn cancel_pairing(&self, fp: Fingerprint) {
        self.handle.cancel_pairing(&fp);
        self.pairing_open.set(false);
    }

    pub fn unpair(&self, fp: Fingerprint) {
        let handle = self.handle.clone();
        let toasts = self.toasts.clone();
        spawn(async move {
            if let Err(e) = handle.unpair(&fp).await {
                push_toast(&toasts, e.to_string(), ToastKind::Error);
            }
        });
    }

    // —— 动作：传输控制 ——

    pub fn pause(&self, id: RecordId, fp: Fingerprint) {
        let _ = self.handle.pause(&id, &fp);
        self.refresh_records();
    }

    pub fn resume(&self, id: RecordId, fp: Fingerprint) {
        let _ = self.handle.resume(&id, &fp);
        self.refresh_records();
    }

    pub fn retry(&self, id: RecordId, fp: Fingerprint) {
        let _ = self.handle.retry(&id, &fp);
        self.refresh_records();
    }

    pub fn cancel_target(&self, id: RecordId, fp: Fingerprint) {
        let _ = self.handle.cancel_target(&id, &fp);
        self.refresh_records();
    }

    // —— 动作：历史 ——

    /// 软删除 + 弹撤销条（5s / Ctrl+Z）。
    pub fn delete_record(&self, id: RecordId) {
        if self.handle.delete_record(&id).unwrap_or(false) {
            self.undo_pending.set(Some(id));
            let pending = self.undo_pending.clone();
            set_timeout(Duration::from_secs(5), move || {
                // 到期仍是同一条则收起（已撤销/已顶替则不动）
                if pending.get_untracked() == Some(id) {
                    pending.set(None);
                }
            });
        }
    }

    pub fn undo_delete(&self) {
        if let Some(id) = self.undo_pending.get_untracked() {
            let _ = self.handle.restore_record(&id);
            self.undo_pending.set(None);
        }
    }

    // —— 动作：剪贴板建议 ——

    /// 输入区为空且聚焦时探测剪贴板（has_* 不触发 iOS 粘贴提示）。
    pub fn probe_clipboard(&self) {
        let hint = self.clip_hint.clone();
        spawn(async move {
            if clipboard_has_image().await {
                hint.set(ClipHint::Image);
            } else if clipboard_has_string().await {
                let n = clipboard_string().await.chars().count();
                if n > 0 {
                    hint.set(ClipHint::Text { chars: n });
                }
            }
        });
    }

    // —— 提示 ——

    pub fn toast(&self, message: impl Into<String>, kind: ToastKind) {
        push_toast(&self.toasts, message.into(), kind);
    }

    // —— 导出 / 打开（0.3.0 无系统保存对话框：落接收目录 + toast 路径） ——

    /// 导出全部记录 CSV 到接收目录。
    pub fn export_csv_all(&self) {
        let dir = self.settings.get_untracked().download_dir;
        let path = dir.join("Drift-records.csv");
        match self.handle.export_csv(path.clone(), &RecordFilter::default()) {
            Ok(n) => self.toast(format!("已导出 {n} 条记录 → {}", path.display()), ToastKind::Success),
            Err(e) => self.toast(format!("导出失败：{e}"), ToastKind::Error),
        }
    }

    /// 文本记录导出 .txt / .md。
    pub fn export_text(&self, id: RecordId, md: bool) {
        let Some(record) = self.handle.record(&id) else { return };
        let title: String = record.title().chars().take(24).collect();
        let name = if md { format!("{title}.md") } else { format!("{title}.txt") };
        let dir = self.settings.get_untracked().download_dir;
        let path = dir.join(drift::util::sanitize_filename(&name));
        let fmt = if md { drift::TextExportFormat::Markdown } else { drift::TextExportFormat::Txt };
        match self.handle.export_text(&id, path.clone(), fmt) {
            Ok(()) => self.toast(format!("已导出 → {}", path.display()), ToastKind::Success),
            Err(e) => self.toast(format!("导出失败：{e}"), ToastKind::Error),
        }
    }

    /// 复制文本到剪贴板。
    pub fn copy_text(&self, text: &str) {
        set_clipboard(text.to_string());
        self.toast("已复制", ToastKind::Success);
    }
}

/// 导出 CSV（更多菜单入口）。
pub fn export_csv_flow(state: &Rc<AppState>) {
    state.export_csv_all();
}

/// 图片选择请求（多选，上限 20）。
pub fn pick_request_multi() -> nsh_core::picker::PickRequest {
    nsh_core::picker::PickRequest::new(true, 20)
}

/// 尝试用系统默认程序打开（文件或目录）。
pub fn open_in_system(path: std::path::PathBuf) {
    spawn(async move {
        let url = url::Url::from_file_path(&path)
            .map(|u| u.to_string())
            .unwrap_or_default();
        if !url.is_empty() {
            let _ = open_url(url).await;
        }
    });
}

fn push_toast(toasts: &Signal<Vec<Toast>>, message: String, kind: ToastKind) {
    let id = toast_id();
    toasts.update(|list| {
        list.push(Toast { id, message, kind });
        list.truncate(3);
    });
    let toasts = toasts.clone();
    set_timeout(Duration::from_millis(3200), move || {
        toasts.update(|l| l.retain(|t| t.id != id));
    });
}

fn toast_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// 事件泵：从 broadcast 收事件，把变更写回信号。
struct Pump {
    handle: drift::DriftHandle,
    nearby: Signal<Vec<PeerDevice>>,
    paired: Signal<Vec<PairedDeviceView>>,
    pairings: Signal<Vec<PairingRequest>>,
    pairing_open: Signal<bool>,
    last_added: Signal<Option<RecordId>>,
    flash_done: Signal<Option<RecordId>>,
    records: Signal<Vec<Record>>,
    filter_kind: Signal<FilterKind>,
    filter_device: Signal<Option<Fingerprint>>,
    filter_query: Signal<String>,
    toasts: Signal<Vec<Toast>>,
    orb_order: Signal<Vec<Fingerprint>>,
    completed: std::cell::RefCell<std::collections::HashSet<RecordId>>,
}

impl Pump {
    fn apply(&self, event: Event) {
        match event {
            Event::ServerStarted { .. } => {}
            Event::DeviceAppeared { .. } | Event::DeviceUpdated { .. } | Event::DeviceLost { .. } => {
                self.nearby.set(self.handle.nearby());
                self.paired.set(self.handle.paired());
                self.ensure_orb_order();
            }
            Event::PairingIncoming { .. } | Event::PairingStarted { .. } => {
                self.pairings.set(self.handle.pending_pairings());
                self.pairing_open.set(true);
            }
            Event::PairingCompleted { name, .. } => {
                self.pairings.set(self.handle.pending_pairings());
                self.pairing_open.set(false);
                self.paired.set(self.handle.paired());
                self.ensure_orb_order();
                push_toast(&self.toasts, format!("已与 {name} 建立双向信任"), ToastKind::Success);
            }
            Event::PairingRejected { by_remote, .. } => {
                self.pairings.set(self.handle.pending_pairings());
                if !by_remote {
                    // 本端拒绝：立即收起；对端拒绝：保留面板展示「重新发起」态
                    self.pairing_open.set(false);
                }
            }
            Event::PairingCanceled { .. } | Event::PairingExpired { .. } => {
                self.pairings.set(self.handle.pending_pairings());
                if self.pairings.get_untracked().is_empty() {
                    self.pairing_open.set(false);
                }
            }
            Event::DeviceUnpaired { .. } => {
                self.paired.set(self.handle.paired());
            }
            Event::RecordAdded { id, .. } => {
                self.last_added.set(Some(id));
                self.refresh_records();
            }
            Event::RecordUpdated { id, .. } => {
                if let Some(record) = self.handle.record(&id) {
                    let now_complete = !record.deliveries.is_empty()
                        && record.deliveries.iter().all(|d| {
                            matches!(
                                d.status,
                                DeliveryStatus::Delivered | DeliveryStatus::Canceled
                            )
                        })
                        && record.deliveries.iter().any(|d| d.status == DeliveryStatus::Delivered);
                    if now_complete && self.completed.borrow_mut().insert(id) {
                        self.flash_done.set(Some(id));
                    }
                }
                self.refresh_records();
            }
            Event::RecordDeleted { .. } | Event::RecordRestored { .. } => {
                // 删除折叠/恢复上移：动效规范场景 6（240ms · 标准）
                animate_layout(LayoutAnimation::EASE_IN_EASE_OUT);
                self.refresh_records();
            }
            Event::SettingsSaved { .. } => {}
            Event::Error { context, message } => {
                push_toast(&self.toasts, format!("{context}: {message}"), ToastKind::Error);
            }
        }
    }

    fn refresh_all(&self) {
        self.nearby.set(self.handle.nearby());
        self.paired.set(self.handle.paired());
        self.ensure_orb_order();
        self.refresh_records();
        self.pairings.set(self.handle.pending_pairings());
    }

    /// 新在线设备上画布即分配稳定落位（黄金角螺旋追加，已落位不动）。
    fn ensure_orb_order(&self) {
        self.orb_order.update(|order| {
            for d in self.handle.paired() {
                if d.online && !order.contains(&d.fingerprint) {
                    order.push(d.fingerprint);
                }
            }
        });
    }

    fn refresh_records(&self) {
        let filter = RecordFilter {
            kinds: self.filter_kind.get_untracked().kinds(),
            device: self.filter_device.get_untracked(),
            query: {
                let q = self.filter_query.get_untracked();
                if q.trim().is_empty() { None } else { Some(q) }
            },
            ..Default::default()
        };
        self.records.set(self.handle.records(&filter).unwrap_or_default());
    }
}

// —— 时间展示 ——

/// 记录时间 "14:22"。
pub fn time_hm(ts: DateTime<Utc>) -> String {
    to_local(ts).format("%H:%M").to_string()
}

/// 日期分组标签：今天 / 昨天 / M月d日 / yyyy年M月d日。
pub fn day_label(ts: DateTime<Utc>) -> String {
    let date = to_local(ts).date_naive();
    let today = Local::now().date_naive();
    if date == today {
        "今天".to_string()
    } else if date == today - chrono::Duration::days(1) {
        "昨天".to_string()
    } else if date.format("%Y").to_string() == today.format("%Y").to_string() {
        date.format("%-m月%-d日").to_string()
    } else {
        date.format("%Y年%-m月%-d日").to_string()
    }
}

fn to_local(ts: DateTime<Utc>) -> DateTime<Local> {
    use chrono::TimeZone;
    Local.timestamp_opt(ts.timestamp(), 0).single().unwrap_or_else(Local::now)
}

/// 拖放 uri（file: URL）→ 本地路径。解析失败返回 None（按文档警告不要手剥前缀）。
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    url::Url::parse(uri).ok()?.to_file_path().ok()
}

/// 供联调：回环注入对端（与 tests/integration.rs 的 inject_each 同型）。
pub fn inject_loopback_peer(state: &AppState, peer: &DeviceInfo) {
    state.handle.add_peer(PeerDevice {
        fingerprint: peer.fingerprint,
        name: peer.name.clone(),
        kind: peer.kind,
        port: peer.port,
        addrs: vec![std::net::SocketAddr::from((
            std::net::Ipv4Addr::LOCALHOST,
            peer.port,
        ))],
        last_seen: Utc::now(),
        source: PeerSource::Manual,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_diff_detects_unchanged_and_incremental_edits() {
        let snap = SentSnapshot {
            text: "这版稿子周五前要定".into(),
            files: Vec::new(),
            version: 1,
        };
        assert_eq!(snap.diff("这版稿子周五前要定", &[]), None);

        // 末尾追加文本 -> AppendedText
        let d1 = snap
            .diff("这版稿子周五前要定，工厂那边我下午再确认。", &[])
            .expect("should detect appended text");
        assert_eq!(
            d1,
            ContentDelta::AppendedText {
                added_chars: 12,
                delta_text: "工厂那边我下午再确认。".into(),
            }
        );
        assert_eq!(d1.badge_label(), "新增 +12 字");

        // 修改原文本 -> ModifiedText
        let d2 = snap
            .diff("下周三前定稿即可", &[])
            .expect("should detect modified text");
        assert_eq!(d2, ContentDelta::ModifiedText { total_chars: 8 });
    }

    #[test]
    fn snapshot_diff_detects_incremental_added_files() {
        let p1 = PathBuf::from("a.pdf");
        let p2 = PathBuf::from("b.png");
        let snap = SentSnapshot {
            text: String::new(),
            files: vec![p1.clone()],
            version: 1,
        };
        let atts = vec![
            Attachment {
                path: p1,
                name: "a.pdf".into(),
                size: 100,
                is_dir: false,
            },
            Attachment {
                path: p2.clone(),
                name: "b.png".into(),
                size: 200,
                is_dir: false,
            },
        ];
        assert_eq!(
            snap.diff("", &atts),
            Some(ContentDelta::AddedFiles {
                added_files: vec![p2],
            })
        );
    }
}

