# Drift 核心

局域网 P2P 文件 / 剪贴板传输核心库（LANDrop 复刻）+ nsh（Native Shell）纯 Rust GUI，桌面（Windows/macOS/Linux）与移动（iOS/Android）同一代码。

> **项目状态：早期原型（Work in Progress）。**
> 目前主要在 Windows 上开发与验证，macOS / Linux / 移动端未经实机测试；
> 不少功能（增量补发、断线重连、配对撤销等）没有充分测试，安全模型（mTLS / 配对码）也未经审计。
> 接口、协议与行为随时可能变化 —— 欢迎尝鲜与提 issue，但请勿用于生产环境或传输敏感数据。

> **Status: early-stage prototype.** Developed and tested on Windows only so far;
> many features lack testing and the security model is unaudited. Expect breaking changes.

## GUI（`gui/`，nsh 0.3 纯 Rust，无 TSX / QuickJS）

```bash
cargo run -p drift-gui                # 本机桌面窗口（Win32/Direct2D）
nsh run                               # 开发迭代（保存即重建重启）
nsh run --target android              # Android 模拟器 / 真机
nsh package --target android          # 出 APK（ios / macos / linux / windows / web 同理）
DRIFT_DATA_DIR=<临时目录> nsh test    # headless 冒烟（App.test.tsx）
```

- 桌面/移动布局按 `is_desktop()` 分流；设计稿 v1.5（`design/`）逐 token 落地（`gui/src/palette.rs`）。
- 主界面是设备球体画布（`gui/src/components/orb_canvas.rs`）：输入有内容球体才浮出、清空即收起，状态画在环上（紫进度 / 橙虚线补发 / 绿已同步 / 灰暂停），点按即发；拖空白平移、双指 / Ctrl+滚轮缩放，加号球就地配对。
- 动效按 `design/drift-motion.svg` 四档时长 / 三条曲线（`gui/src/motion.rs`），transitions.dev 令牌纪律补齐未定义场景；Motion 自动遵守系统「减少动态」。
- 联调：同机双实例用交换文件互注（mDNS 同主机不可靠）——
  `DRIFT_DATA_DIR=A DRIFT_PEER_FILE=peers.jsonl drift-gui.exe` × 2（另见 `DRIFT_NAME` / `DRIFT_DEV_ATTACH`）。

## 功能对照（设计稿 → 核心）

| 设计 | 核心 API |
| --- | --- |
| 同一 Wi-Fi 自动发现 | mDNS `_drift._tcp.local.`，`nearby()` / `DeviceAppeared/Updated/Lost` |
| 配对：一码对照、不输入、双向确认、随时撤销 | `initiate_pairing` → 两端 `PairingIncoming`/`PairingStarted`（同一组 6 位码）→ 各调一次 `confirm_pairing`；`reject_pairing` 作废；`unpair` 撤销 |
| 发送文本 / 文件 / 文件夹（自动 zip） | `send_text` / `send_paths`，一次发送 = 一条记录 |
| 逐设备独立进度、暂停/继续/取消/重试 | `Record.deliveries[]`（进度/速度/ETA）+ `pause/resume/cancel_target/retry` |
| 断网自动重连，30 秒后转等待重试 | 传输引擎内置（`DeliveryStatus::WaitingRetry` 后等用户 `retry`） |
| 配对即自动接收，落入接收目录 | 已配对对端 403 → 200；落盘 `settings.download_dir`（重名自动 ` (2)`） |
| 全局历史、筛选 全部/文件/图片/文本 + 设备 + 关键词 | `records(&RecordFilter)` |
| 删除可撤销（Ctrl/Cmd+Z）、不删本地文件 | `delete_record`（软删）/ `restore_record` |
| 导出全部记录 CSV；文本导出 .txt/.md | `export_csv` / `export_text` |
| 记录内文件的另存为/打开/显示位置 | `file_path(record_id, idx)` |
| 剪贴板粘贴发送 | 文本走 `send_text`；图片等字节流走 `send_blob`（自动落临时文件、发完清理） |
| 未配对设备无法发送内容 | 服务端对所有 send 端点校验信任，403 |

## 快速开始

```rust
let handle = drift::Drift::launch(drift::LaunchConfig::default())?;

// 事件驱动界面刷新
let mut rx = handle.subscribe();
std::thread::spawn(move || while let Ok(e) = rx.blocking_recv() { /* 重绘 */ });

// 附近设备 → 配对
for d in handle.nearby() { /* 列表 */ }
handle.blocking().initiate_pairing(&fp)?;      // 两端同时显示同一组码
// ... 用户核对一致后，两端各点一次：
handle.blocking().confirm_pairing(&fp)?;

// 发送（返回记录 ID；进度通过 RecordUpdated 事件推送）
let id = handle.blocking().send_text("hello".into(), &[fp])?;
let id = handle.blocking().send_blob("截图.png", Some("image/png".into()), png_bytes, &[fp])?;
let id = handle.blocking().send_paths(vec!["照片.zip".into()], &[fp1, fp2])?;

// 逐设备控制
handle.pause(&id, &fp1)?;
handle.resume(&id, &fp1)?;
handle.retry(&id, &fp1)?;   // 失败/等待重试后
handle.cancel_target(&id, &fp2)?;
```

## 线程模型与调用约定

- 核心自带独立 tokio 运行时（线程名 `drift-io`）；`DriftHandle` 可跨线程 clone。
- 异步方法（`send_text` 等）可在任何运行时里 `.await`；
  同步 GUI 用 `handle.blocking()`（内部 `block_on`，**不要**在异步上下文里调用）。
- `pause / resume / retry / cancel_target / records / delete_record` 等本就是同步方法，随处可调。
- 事件通道容量 1024，慢消费者会收到 `Lagged`（按需全量刷新一次界面状态即可）。
- `shutdown()` 非阻塞，可在同步或异步上下文调用。

## 协议与安全

- 传输：TLS 1.3 + 双向证书（mTLS）。每台设备首次启动生成 ed25519 自签名证书，持久化在数据目录。
- 身份 = 证书 SHA256 指纹（`Fingerprint`）。HTTP 请求不携带自报身份，对端身份一律取自证书。
- 配对即锁定指纹；此后连接校验证书指纹必须匹配（中间人无法通过）。
- 配对码 = 双方指纹 + 盐的 SHA256 → 6 位数字（SAS）：中间人会导致两端码不一致，人工核对必然发现。
- HTTP API（`/api/v1/*`）：`info`、`pair/{request,confirm,reject}`、`unpair`、`send/text`、
  `send/prepare`、`send/{sid}/file/{idx}`（`x-drift-offset` 断点续传）、`finish`、`DELETE` 丢弃会话。
- 数据只存本机（redb 单文件 + JSON 设置）；不经任何服务器。

## 数据目录

`dirs::config_dir()/drift/`（`--data-dir` / `LaunchConfig::data_dir` 可改）：

- `settings.json` 设备名 / 接收目录 / 端口 / 发现开关
- `identity.key` + `identity.crt` 设备身份（删除即"新设备"）
- `drift.db` 信任表 + 全局历史

接收目录默认 `下载/Drift/`；传输中文件在 `.incoming/` 临时区，完成后改名入正式目录。

## CLI 演示

```bash
cargo run -- --name 我的电脑          # 两台机器各跑一个（允许防火墙专用网络）
# A: devices → pair <指纹> → 两端看码 → 两端 yes <对端指纹>
# A: send <指纹> 你好 / file <指纹> D:\照片
# records / pause <记录ID> <指纹> / retry / del / undo / csv out.csv
```

## 开发

```bash
cargo test          # 单元 + 回环集成测试（双节点真实 TLS 收发、暂停续传、配对拒绝）
```
