//! 配对：自动发现后点选设备 → 两端各自显示同一组 6 位配对码 →
//! 人工核对一致后两端各点一次「一致」→ 建立双向信任。
//!
//! 配对码由双方指纹确定性推导（SAS 模式）：
//! 中间人会导致两端指纹不同 → 配对码不同 → 人工核对必然发现。
//! 不需要输入、不上传、随时撤销。

use std::{
    net::SocketAddr,
    sync::Arc,
    time::Duration,
};

use axum::http::Method;
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

use crate::{
    app::Core,
    client::PeerLink,
    discovery::PairedDevice,
    error::{DriftError, Result},
    events::{Event, PairingRequest},
    identity::{DeviceKind, Fingerprint},
    protocol::{self, PairConfirmResponse, PairRequestBody},
};

/// 一端持有的配对会话
#[derive(Clone, Debug)]
pub struct PairingSession {
    pub peer_fp: Fingerprint,
    pub peer_name: String,
    pub peer_kind: DeviceKind,
    pub peer_addrs: Vec<SocketAddr>,
    pub code: String,
    pub expires_at: DateTime<Utc>,
    pub local_confirmed: bool,
    pub remote_confirmed: bool,
    pub initiated_by_us: bool,
}

impl PairingSession {
    pub fn expired(&self) -> bool {
        Utc::now() > self.expires_at
    }

    pub fn to_request(&self) -> PairingRequest {
        PairingRequest {
            peer: self.peer_fp,
            peer_name: self.peer_name.clone(),
            peer_kind: self.peer_kind.as_str().to_string(),
            code: self.code.clone(),
            expires_at: self.expires_at,
            initiated_by_us: self.initiated_by_us,
        }
    }
}

/// 配对码：两端指纹 + 盐取 SHA256，映射到 6 位数字，格式 "482 916"。
pub fn pairing_code(a: &Fingerprint, b: &Fingerprint) -> String {
    let mut buf = Vec::with_capacity(32 + 64);
    buf.extend_from_slice(b"drift-pairing-v1");
    let (x, y) = if a.as_bytes() < b.as_bytes() { (a, b) } else { (b, a) };
    buf.extend_from_slice(x.as_bytes());
    buf.extend_from_slice(y.as_bytes());
    let h = Sha256::digest(&buf);
    let n = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) % 1_000_000;
    let s = format!("{n:06}");
    format!("{} {}", &s[..3], &s[3..])
}

fn link_for(core: &Core, fp: &Fingerprint, addrs: Vec<SocketAddr>) -> PeerLink {
    PeerLink::new(*fp, addrs, core.identity.tls_material())
}

/// 发起配对（本端点选了附近某台设备）
pub async fn initiate(core: &Arc<Core>, fp: &Fingerprint) -> Result<()> {
    if core.is_paired(fp) {
        return Ok(()); // 已配对，幂等
    }
    let addrs = core.peer_addrs(fp);
    if addrs.is_empty() {
        return Err(DriftError::UnknownDevice(fp.to_hex()));
    }

    let code = pairing_code(&core.identity.fingerprint(), fp);
    let peer = core
        .peers
        .read()
        .unwrap()
        .get(fp)
        .cloned();

    let session = PairingSession {
        peer_fp: *fp,
        peer_name: peer.as_ref().map(|p| p.name.clone()).unwrap_or_else(|| fp.short()),
        peer_kind: peer.as_ref().map(|p| p.kind).unwrap_or(DeviceKind::Other),
        peer_addrs: addrs.clone(),
        code: code.clone(),
        expires_at: Utc::now() + chrono::TimeDelta::seconds(protocol::PAIRING_TTL_SECS as i64),
        local_confirmed: false,
        remote_confirmed: false,
        initiated_by_us: true,
    };

    // 送达配对请求（对端弹「核对配对码」）
    let link = link_for(core, fp, addrs);
    let body = PairRequestBody { sender: core.device_dto() };
    link.send_json::<protocol::ApiResponse>(Method::POST, protocol::path::PAIR_REQUEST, &body)
        .await?;

    let req = session.to_request();
    core.pairings.lock().unwrap().insert(*fp, session);
    core.emit(Event::PairingStarted { request: req });
    Ok(())
}

/// 收到配对请求（本端是被发起方）
pub fn on_remote_request(core: &Arc<Core>, sender: &protocol::DeviceDto, addrs: Vec<SocketAddr>) {
    let Ok(fp) = sender.parse_fingerprint() else {
        return;
    };
    if core.is_paired(&fp) {
        return; // 已配对忽略
    }
    // 合并 mDNS 已知地址
    let mut all = core.peer_addrs(&fp);
    for a in addrs {
        if !all.contains(&a) {
            all.push(a);
        }
    }

    let code = pairing_code(&core.identity.fingerprint(), &fp);
    let session = PairingSession {
        peer_fp: fp,
        peer_name: sender.name.clone(),
        peer_kind: sender.parse_kind(),
        peer_addrs: all,
        code,
        expires_at: Utc::now() + chrono::TimeDelta::seconds(protocol::PAIRING_TTL_SECS as i64),
        local_confirmed: false,
        remote_confirmed: false,
        initiated_by_us: false,
    };
    let req = session.to_request();
    core.pairings.lock().unwrap().insert(fp, session);
    core.emit(Event::PairingIncoming { request: req });
}

/// 本端用户点了「一致，配对」
pub async fn confirm(core: &Arc<Core>, fp: &Fingerprint) -> Result<()> {
    {
        let mut sessions = core.pairings.lock().unwrap();
        let Some(session) = sessions.get_mut(fp) else {
            return Err(DriftError::PairingSessionInvalid);
        };
        if session.expired() {
            sessions.remove(fp);
            drop(sessions);
            core.emit(Event::PairingExpired { fingerprint: *fp });
            return Err(DriftError::PairingSessionInvalid);
        }
        session.local_confirmed = true;
    }

    // 通知对端；若对端已确认，响应会带 completed=true
    let addrs = current_session_addrs(core, fp);
    if addrs.is_empty() {
        return Err(DriftError::UnknownDevice(fp.to_hex()));
    }
    let link = link_for(core, fp, addrs);
    let resp: PairConfirmResponse = link
        .send_json(Method::POST, protocol::path::PAIR_CONFIRM, &serde_json::json!({}))
        .await?;

    if resp.completed {
        let data = core.pairings.lock().unwrap().remove(fp).map(session_info);
        finalize_trust(core, fp, data);
    }
    Ok(())
}

/// 对端发来「一致」确认（HTTP 处理器调用）
pub fn on_remote_confirm(core: &Arc<Core>, fp: &Fingerprint) -> PairConfirmResponse {
    let mut data: Option<(String, DeviceKind, Vec<SocketAddr>)> = None;
    let completed = {
        let mut sessions = core.pairings.lock().unwrap();
        let Some(session) = sessions.get_mut(fp) else {
            return PairConfirmResponse { completed: false };
        };
        if session.expired() {
            sessions.remove(fp);
            drop(sessions);
            core.emit(Event::PairingExpired { fingerprint: *fp });
            return PairConfirmResponse { completed: false };
        }
        session.remote_confirmed = true;
        let completed = session.remote_confirmed && session.local_confirmed;
        if completed {
            data = sessions.remove(fp).map(session_info);
        }
        completed
    };
    if completed {
        finalize_trust(core, fp, data);
    }
    PairConfirmResponse { completed }
}

/// 本端点了「不一致」或主动取消
pub async fn reject(core: &Arc<Core>, fp: &Fingerprint) -> Result<()> {
    // 先取地址再删除会话
    let addrs = current_session_addrs(core, fp);
    let existed = core.pairings.lock().unwrap().remove(fp).is_some();
    if existed {
        core.emit(Event::PairingRejected { fingerprint: *fp, by_remote: false });
    }
    // 通知对端作废（尽力而为）
    if !addrs.is_empty() {
        let link = link_for(core, fp, addrs);
        let _ = link
            .send_json::<protocol::ApiResponse>(
                Method::POST,
                protocol::path::PAIR_REJECT,
                &serde_json::json!({}),
            )
            .await;
    }
    Ok(())
}

/// 对端点了「不一致」
pub fn on_remote_reject(core: &Arc<Core>, fp: &Fingerprint) {
    let existed = core.pairings.lock().unwrap().remove(fp).is_some();
    if existed {
        core.emit(Event::PairingRejected { fingerprint: *fp, by_remote: true });
    }
}

/// 取消本端发起的等待（不发 reject，对端超时自动作废）
pub fn cancel(core: &Arc<Core>, fp: &Fingerprint) {
    let existed = core.pairings.lock().unwrap().remove(fp).is_some();
    if existed {
        core.emit(Event::PairingCanceled { fingerprint: *fp });
    }
}

/// 会话中缓存的对端信息
fn session_info(s: PairingSession) -> (String, DeviceKind, Vec<SocketAddr>) {
    (s.peer_name, s.peer_kind, s.peer_addrs)
}

/// 双端确认后落地信任。名字/类型/地址优先级：mDNS 实时 > 配对会话缓存 > 短指纹。
fn finalize_trust(
    core: &Arc<Core>,
    fp: &Fingerprint,
    session: Option<(String, DeviceKind, Vec<SocketAddr>)>,
) {
    let peer = core.peers.read().unwrap().get(fp).cloned();
    let (fb_name, fb_kind, fb_addrs) =
        session.unwrap_or_else(|| (fp.short(), DeviceKind::Other, Vec::new()));

    let mut addrs: Vec<SocketAddr> = peer.as_ref().map(|p| p.addrs.clone()).unwrap_or_default();
    if addrs.is_empty() {
        addrs = fb_addrs;
    }
    let (name, kind) = match &peer {
        Some(p) => (p.name.clone(), p.kind),
        None => (fb_name, fb_kind),
    };
    let paired = PairedDevice {
        fingerprint: *fp,
        name: name.clone(),
        kind,
        paired_at: Utc::now(),
        addrs,
        last_seen: peer.as_ref().map(|p| p.last_seen),
    };
    let _ = core.storage.add_paired(&paired);
    core.emit(Event::PairingCompleted { fingerprint: *fp, name });
}

fn current_session_addrs(core: &Core, fp: &Fingerprint) -> Vec<SocketAddr> {
    core.pairings
        .lock()
        .unwrap()
        .get(fp)
        .map(|s| s.peer_addrs.clone())
        .unwrap_or_default()
}

/// 过期清扫（两端各自跑；需传入 drift 运行时句柄）
pub fn spawn_sweeper(handle: &tokio::runtime::Handle, core: Arc<Core>) {
    handle.spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(10));
        loop {
            ticker.tick().await;
            let expired: Vec<Fingerprint> = {
                let mut sessions = core.pairings.lock().unwrap();
                let expired: Vec<Fingerprint> = sessions
                    .iter()
                    .filter(|(_, s)| s.expired())
                    .map(|(fp, _)| *fp)
                    .collect();
                for fp in &expired {
                    sessions.remove(fp);
                }
                expired
            };
            for fp in expired {
                core.emit(Event::PairingExpired { fingerprint: fp });
            }
        }
    });
}

/// 测试与工具：当前会话快照
pub fn pending(core: &Core) -> Vec<PairingRequest> {
    core.pairings
        .lock()
        .unwrap()
        .values()
        .map(|s| s.to_request())
        .collect()
}
