//! mDNS 自动发现（`_drift._tcp.local.`）+ UDP 广播发现（回退通道）与附近设备表。
//!
//! mDNS TXT / 广播宣告携带：指纹 / 设备名 / 类型 / 端口 / 协议版本。
//! 附近列表同时是配对入口（「附近设备」面板）与配对缓存刷新来源。
//!
//! 广播通道的存在原因：Android 应用不持 `MulticastLock` 时 Wi-Fi 固件直接丢弃
//! 组播帧（纯 Rust 无 JNI 拿不到锁），部分桌面防火墙也拦 UDP 5353；定向广播帧
//! 两端都能正常收发，作为并行的发现路径。

use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::{Deserialize, Serialize};

use crate::{
    app::Core,
    events::Event,
    identity::{DeviceKind, Fingerprint},
    protocol::{PROTOCOL_VERSION, SERVICE_TYPE},
    util,
};

/// 广播发现端口（UDP）
pub const BROADCAST_PORT: u16 = 46464;
/// 广播宣告间隔
const BCAST_INTERVAL: Duration = Duration::from_secs(3);
/// 广播来源设备的离线判定（错过 15 个宣告）
const BCAST_TTL_SECS: i64 = 45;

/// 附近设备条目
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PeerDevice {
    pub fingerprint: Fingerprint,
    pub name: String,
    pub kind: DeviceKind,
    pub port: u16,
    pub addrs: Vec<SocketAddr>,
    pub last_seen: DateTime<Utc>,
    /// 发现方式
    pub source: PeerSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerSource {
    /// mDNS 自动发现
    Discovered,
    /// UDP 广播发现（mDNS 不可达环境：Android 无 MulticastLock、防火墙拦 5353 等）
    Broadcast,
    /// 手动/测试注入或配对缓存
    Manual,
}

/// 信任存储中的已配对设备
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PairedDevice {
    pub fingerprint: Fingerprint,
    pub name: String,
    pub kind: DeviceKind,
    pub paired_at: DateTime<Utc>,
    /// 最近已知地址（mDNS 刷新；mDNS 不可用时仍可直连）
    pub addrs: Vec<SocketAddr>,
    pub last_seen: Option<DateTime<Utc>>,
}

/// mDNS 会话句柄（改名时需要重注册）
pub struct DiscoveryHandle {
    daemon: ServiceDaemon,
    instance: String,
    port: u16,
    /// 广播线程停止位
    bcast_stop: Arc<AtomicBool>,
}

impl DiscoveryHandle {
    /// 设备名变化后重新注册 TXT（mDNS 广播宣告读取的是当时的设置快照，改名后自然生效）
    pub fn reannounce(&self, name: &str, kind: DeviceKind) -> crate::error::Result<()> {
        let _ = self.daemon.unregister(&self.instance);
        self.register(name, kind)?;
        Ok(())
    }

    fn register(&self, name: &str, kind: DeviceKind) -> crate::error::Result<()> {
        let info = service_info(&self.instance, name, kind, self.port)?;
        self.daemon
            .register(info)
            .map_err(|e| crate::error::DriftError::Other(format!("注册 mDNS 服务失败: {e}")))?;
        Ok(())
    }

    pub fn shutdown(&self) {
        self.bcast_stop.store(true, Ordering::Relaxed);
        let _ = self.daemon.unregister(&self.instance);
        let _ = self.daemon.shutdown();
    }
}

fn instance_name(fp: &Fingerprint) -> String {
    fp.to_hex()
}

fn service_info(
    instance: &str,
    name: &str,
    kind: DeviceKind,
    port: u16,
) -> crate::error::Result<ServiceInfo> {
    let ip = util::local_ip();
    let host_name = format!("drift-{instance}.local.");
    let props: HashMap<String, String> = HashMap::from([
        ("fp".to_string(), instance.to_string()),
        ("name".to_string(), name.to_string()),
        ("kind".to_string(), kind.as_str().to_string()),
        ("v".to_string(), PROTOCOL_VERSION.to_string()),
    ]);
    ServiceInfo::new(SERVICE_TYPE, instance, &host_name, ip, port, props)
        .map_err(|e| crate::error::DriftError::Other(format!("构造 mDNS 服务信息失败: {e}")))
}

/// 启动发现：注册本机服务 + 监听附近设备（mDNS 与广播两路），更新 Core.peers 并广播事件。
pub fn spawn(core: Arc<Core>, port: u16) -> crate::error::Result<DiscoveryHandle> {
    let daemon = ServiceDaemon::new()
        .map_err(|e| crate::error::DriftError::Other(format!("启动 mDNS 失败: {e}")))?;
    let instance = instance_name(&core.identity.fingerprint());
    let bcast_stop = Arc::new(AtomicBool::new(false));

    let handle = DiscoveryHandle {
        daemon: daemon.clone(),
        instance: instance.clone(),
        port,
        bcast_stop: bcast_stop.clone(),
    };
    {
        let (name, kind) = {
            let s = core.settings.read().unwrap();
            (s.device_name.clone(), s.device_kind)
        };
        handle.register(&name, kind)?;
    }

    // 监听附近设备（mDNS）
    let receiver = daemon
        .browse(SERVICE_TYPE)
        .map_err(|e| crate::error::DriftError::Other(format!("浏览 mDNS 失败: {e}")))?;

    let core2 = core.clone();
    std::thread::Builder::new()
        .name("drift-mdns".into())
        .spawn(move || {
            while let Ok(event) = receiver.recv() {
                match event {
                    ServiceEvent::ServiceResolved(svc) => {
                        if let Err(e) = on_resolved(&core2, &svc) {
                            core2.emit(Event::Error {
                                context: "处理附近设备".into(),
                                message: e.to_string(),
                            });
                        }
                    }
                    ServiceEvent::ServiceRemoved(_ty, fullname) => {
                        // fullname: "<instance>._drift._tcp.local."
                        let instance = fullname.split('.').next().unwrap_or("");
                        if let Ok(fp) = Fingerprint::from_hex(instance) {
                            let removed = core2.peers.write().unwrap().remove(&fp);
                            if removed.is_some() {
                                core2.emit(Event::DeviceLost { fingerprint: fp });
                            }
                        }
                    }
                    _ => {}
                }
            }
        })
        .ok();

    // 广播发现（并行通道，失败不阻断：mDNS 仍可用）
    spawn_broadcast(core, port, bcast_stop);

    Ok(handle)
}

fn on_resolved(core: &Arc<Core>, svc: &mdns_sd::ResolvedService) -> crate::error::Result<()> {
    let Some(fp_hex) = svc.get_property_val_str("fp") else {
        return Ok(());
    };
    let Ok(fp) = Fingerprint::from_hex(fp_hex) else {
        return Ok(());
    };
    // 忽略自己
    if fp == core.identity.fingerprint() {
        return Ok(());
    }
    let name = svc
        .get_property_val_str("name")
        .map(|s| s.to_string())
        .unwrap_or_else(|| fp.short());
    let kind = svc
        .get_property_val_str("kind")
        .and_then(DeviceKind::parse)
        .unwrap_or(DeviceKind::Other);
    let port = svc.port;

    let mut addrs: Vec<SocketAddr> = svc
        .addresses
        .iter()
        .map(|ip| SocketAddr::new(ip.to_ip_addr(), port))
        .collect();
    util::sort_addrs(&mut addrs);
    if addrs.is_empty() {
        return Ok(());
    }

    let peer = PeerDevice {
        fingerprint: fp,
        name,
        kind,
        port,
        addrs,
        last_seen: Utc::now(),
        source: PeerSource::Discovered,
    };
    upsert_peer(core, peer);
    Ok(())
}

// —— UDP 广播发现 ——

/// 广播宣告负载（与 mDNS TXT 同构）
#[derive(Serialize, Deserialize)]
struct Announce {
    fp: String,
    name: String,
    kind: String,
    port: u16,
    v: u32,
}

/// 广播发现线程：每 3s 向各网段定向广播 + 255.255.255.255 宣告一次，
/// 收到陌生设备的宣告后单播回一声加快双向收敛。线程内的收发共用一个 socket，
/// 对端源地址即直连地址（比猜接口地址可靠）。
fn spawn_broadcast(core: Arc<Core>, port: u16, stop: Arc<AtomicBool>) {
    std::thread::Builder::new()
        .name("drift-bcast".into())
        .spawn(move || {
            let Ok(sock) = UdpSocket::bind(("0.0.0.0", BROADCAST_PORT)) else {
                return; // 端口被占（比如同机第二实例）：mDNS 通道仍可用
            };
            let _ = sock.set_broadcast(true);
            // 500ms 醒一次看停止位；宣告节奏独立计算
            let _ = sock.set_read_timeout(Some(Duration::from_millis(500)));
            let mut next_announce = Instant::now();
            loop {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                if Instant::now() >= next_announce {
                    announce_once(&sock, &core, port);
                    next_announce = Instant::now() + BCAST_INTERVAL;
                    continue;
                }
                let mut buf = [0u8; 1024];
                match sock.recv_from(&mut buf) {
                    Ok((n, from)) => {
                        if let Some(reply_to) = on_announce(&core, &buf[..n], from) {
                            send_to(&sock, &core, port, reply_to);
                        }
                    }
                    Err(_) => continue, // 多为读超时：回到循环头检查停止位 / 宣告节奏
                }
            }
        })
        .ok();
}

/// 向所有网段广播一遍宣告
fn announce_once(sock: &UdpSocket, core: &Core, port: u16) {
    let mut targets: Vec<SocketAddr> = vec![SocketAddr::new(Ipv4Addr::BROADCAST.into(), BROADCAST_PORT)];
    if let Ok(ifaces) = if_addrs::get_if_addrs() {
        for iface in ifaces {
            if iface.is_loopback() {
                continue;
            }
            if let if_addrs::IfAddr::V4(v4) = iface.addr
                && let Some(b) = v4.broadcast
            {
                targets.push(SocketAddr::new(b.into(), BROADCAST_PORT));
            }
        }
    }
    for t in targets {
        send_to(sock, core, port, t);
    }
}

fn send_to(sock: &UdpSocket, core: &Core, port: u16, to: SocketAddr) {
    let (name, kind) = {
        let s = core.settings.read().unwrap();
        (s.device_name.clone(), s.device_kind)
    };
    let msg = Announce {
        fp: core.identity.fingerprint().to_hex(),
        name,
        kind: kind.as_str().to_string(),
        port,
        v: PROTOCOL_VERSION,
    };
    if let Ok(raw) = serde_json::to_vec(&msg) {
        let _ = sock.send_to(&raw, to);
    }
}

/// 处理收到的宣告；返回 `Some(对端地址)` 表示对端是刚出现的新面孔（应单播回一声）。
fn on_announce(core: &Arc<Core>, raw: &[u8], from: SocketAddr) -> Option<SocketAddr> {
    let Ok(msg) = serde_json::from_slice::<Announce>(raw) else {
        return None;
    };
    if !version_compatible(msg.v) {
        return None;
    }
    let Ok(fp) = Fingerprint::from_hex(&msg.fp) else {
        return None;
    };
    if fp == core.identity.fingerprint() {
        return None;
    }
    // 广播包的源地址被某些协议栈填 0.0.0.0：无法建立直连，忽略
    if from.ip().is_unspecified() {
        return None;
    }
    let kind = DeviceKind::parse(&msg.kind).unwrap_or(DeviceKind::Other);
    let peer = PeerDevice {
        fingerprint: fp,
        name: if msg.name.is_empty() { fp.short() } else { msg.name },
        kind,
        port: msg.port,
        addrs: vec![SocketAddr::new(from.ip(), msg.port)],
        last_seen: Utc::now(),
        source: PeerSource::Broadcast,
    };
    let was_new = core.peers.read().unwrap().get(&fp).is_none();
    upsert_peer(core, peer);
    was_new.then_some(SocketAddr::new(from.ip(), BROADCAST_PORT))
}

/// 附近表 upsert + 事件广播 + 已配对设备地址缓存刷新（mDNS / 广播共用）。
fn upsert_peer(core: &Arc<Core>, peer: PeerDevice) {
    let fp = peer.fingerprint;
    let addrs = peer.addrs.clone();
    let appeared = core
        .peers
        .write()
        .unwrap()
        .insert(fp, peer.clone())
        .is_none();

    // 同步刷新信任表缓存（设备改名 / 地址变化也能跟上）
    if let Ok(Some(mut paired)) = core.storage.get_paired(&fp) {
        paired.name = peer.name.clone();
        paired.kind = peer.kind;
        paired.addrs = addrs;
        paired.last_seen = Some(Utc::now());
        let _ = core.storage.add_paired(&paired);
    }

    if appeared {
        core.emit(Event::DeviceAppeared { device: peer });
    } else {
        core.emit(Event::DeviceUpdated { device: peer });
    }
}

/// 空闲扫描：把超过 TTL 未见的发现设备标记离线（mDNS remove 丢失 / 广播停止的兜底）。
pub fn spawn_offline_sweeper(handle: &tokio::runtime::Handle, core: Arc<Core>) {
    handle.spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(10));
        loop {
            ticker.tick().await;
            let mut lost = Vec::new();
            {
                let peers = core.peers.read().unwrap();
                for (fp, p) in peers.iter() {
                    let ttl = match p.source {
                        PeerSource::Discovered => 120,
                        PeerSource::Broadcast => BCAST_TTL_SECS,
                        PeerSource::Manual => continue,
                    };
                    let age = Utc::now()
                        .signed_duration_since(p.last_seen)
                        .num_seconds();
                    if age > ttl {
                        lost.push(*fp);
                    }
                }
            }
            if !lost.is_empty() {
                let mut peers = core.peers.write().unwrap();
                for fp in lost {
                    peers.remove(&fp);
                    core.emit(Event::DeviceLost { fingerprint: fp });
                }
            }
        }
    });
}

/// 把手动注入的设备塞进附近表（测试 / 未来手动添加 IP 用）
pub fn inject_peer(core: &Arc<Core>, peer: PeerDevice) {
    let fp = peer.fingerprint;
    let appeared = core
        .peers
        .write()
        .unwrap()
        .insert(fp, peer.clone())
        .is_none();
    if appeared {
        core.emit(Event::DeviceAppeared { device: peer });
    } else {
        core.emit(Event::DeviceUpdated { device: peer });
    }
}

/// 协议版本兼容检查（预留：版本不一致仅提示不阻断）
pub fn version_compatible(remote: u32) -> bool {
    remote <= PROTOCOL_VERSION + 1 || remote + 1 >= PROTOCOL_VERSION
}
