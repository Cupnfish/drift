//! mDNS 自动发现（`_drift._tcp.local.`）与附近设备表。
//!
//! TXT 记录携带：指纹 / 设备名 / 类型 / 端口 / 协议版本。
//! 附近列表同时是配对入口（「附近设备」面板）与配对缓存刷新来源。

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::Arc,
    time::Duration,
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
}

impl DiscoveryHandle {
    /// 设备名变化后重新注册 TXT
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

/// 启动发现：注册本机服务 + 监听附近设备，更新 Core.peers 并广播事件。
pub fn spawn(core: Arc<Core>, port: u16) -> crate::error::Result<DiscoveryHandle> {
    let daemon = ServiceDaemon::new()
        .map_err(|e| crate::error::DriftError::Other(format!("启动 mDNS 失败: {e}")))?;
    let instance = instance_name(&core.identity.fingerprint());

    let handle = DiscoveryHandle { daemon: daemon.clone(), instance: instance.clone(), port };
    {
        let (name, kind) = {
            let s = core.settings.read().unwrap();
            (s.device_name.clone(), s.device_kind)
        };
        handle.register(&name, kind)?;
    }

    // 监听附近设备
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
        addrs: addrs.clone(),
        last_seen: Utc::now(),
        source: PeerSource::Discovered,
    };

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
        paired.addrs = addrs.clone();
        paired.last_seen = Some(Utc::now());
        let _ = core.storage.add_paired(&paired);
    }

    if appeared {
        core.emit(Event::DeviceAppeared { device: peer });
    } else {
        core.emit(Event::DeviceUpdated { device: peer });
    }
    Ok(())
}

/// 空闲扫描：把超过 TTL 未见的发现设备标记离线（mDNS remove 丢失时的兜底）。
pub fn spawn_offline_sweeper(handle: &tokio::runtime::Handle, core: Arc<Core>) {
    handle.spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(30));
        loop {
            ticker.tick().await;
            let mut lost = Vec::new();
            {
                let peers = core.peers.read().unwrap();
                for (fp, p) in peers.iter() {
                    if p.source == PeerSource::Discovered {
                        let age = Utc::now()
                            .signed_duration_since(p.last_seen)
                            .num_seconds();
                        if age > 120 {
                            lost.push(*fp);
                        }
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
