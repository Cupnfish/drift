//! HTTP 服务端：TLS 1.3 + 双向证书（mTLS）。
//!
//! 每个连接：
//! 1. TLS 握手（客户端必须出示证书）；
//! 2. 提取对端证书指纹 → 作为对端身份注入请求上下文；
//! 3. 除 /info 与 /pair/* 外，均要求对端已配对（指纹在信任表中）。
//!
//! 未配对设备无法发送内容、也看不到任何记录 —— 信任即访问控制。

use std::{
    net::SocketAddr,
    sync::Arc,
};

use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Json, Response},
    routing::{get, post, put},
    Router,
};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

use crate::{
    app::Core,
    error::DriftError,
    events::Event,
    history::PeerSummary,
    identity::Fingerprint,
    protocol::{self, ApiResponse, PairRequestBody, PrepareRequest, TextRequest},
    transfer,
};

/// 每连接上下文：对端指纹（mTLS 提取）+ 远端地址
#[derive(Clone)]
pub struct ConnCtx {
    pub core: Arc<Core>,
    pub peer: Option<Fingerprint>,
    pub remote: SocketAddr,
}

fn build_router() -> Router<ConnCtx> {
    Router::new()
        .route(protocol::path::INFO, get(info))
        // 配对（不需要已配对，但需要证书）
        .route(protocol::path::PAIR_REQUEST, post(pair_request))
        .route(protocol::path::PAIR_CONFIRM, post(pair_confirm))
        .route(protocol::path::PAIR_REJECT, post(pair_reject))
        .route(protocol::path::UNPAIR, post(unpair))
        // 收发（需要已配对）
        .route(protocol::path::SEND_TEXT, post(recv_text))
        .route(protocol::path::SEND_PREPARE, post(recv_prepare))
        .route("/api/v1/send/{sid}/file/{idx}/state", get(chunk_state))
        .route("/api/v1/send/{sid}/file/{idx}", put(recv_chunk))
        .route(
            "/api/v1/send/{sid}",
            axum::routing::delete(drop_session).post(finish_session),
        )
        .layer(DefaultBodyLimit::max(8 * 1024 * 1024))
}

fn err_response(status: StatusCode, msg: impl Into<String>) -> Response {
    (
        status,
        Json(ApiResponse { ok: false, error: Some(msg.into()) }),
    )
        .into_response()
}

/// 要求对端已配对，返回其指纹
fn require_paired(ctx: &ConnCtx) -> std::result::Result<Fingerprint, Box<Response>> {
    let fp = ctx
        .peer
        .ok_or_else(|| Box::new(err_response(StatusCode::UNAUTHORIZED, "缺少对端证书")))?;
    if !ctx.core.is_paired(&fp) {
        return Err(Box::new(err_response(StatusCode::FORBIDDEN, "设备未配对，无法接收内容")));
    }
    Ok(fp)
}

fn peer_summary(core: &Core, fp: &Fingerprint) -> PeerSummary {
    let paired = core.storage.get_paired(fp).ok().flatten();
    let peer = core.peers.read().unwrap().get(fp).cloned();
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
            .unwrap_or(crate::identity::DeviceKind::Other),
    }
}

// ---------- handlers ----------

async fn info(State(ctx): State<ConnCtx>) -> Response {
    Json(ctx.core.device_dto()).into_response()
}

async fn pair_request(State(ctx): State<ConnCtx>, body: axum::extract::Json<PairRequestBody>) -> Response {
    let Some(peer_fp) = ctx.peer else {
        return err_response(StatusCode::UNAUTHORIZED, "缺少对端证书");
    };
    // 请求体自报的指纹必须与证书一致，防止顶替他人发起配对
    if body.sender.parse_fingerprint().map(|f| f != peer_fp).unwrap_or(true) {
        return err_response(StatusCode::BAD_REQUEST, "请求体指纹与证书不一致");
    }
    let reach_addr = SocketAddr::new(ctx.remote.ip(), body.sender.port);
    crate::pairing::on_remote_request(&ctx.core, &body.sender, vec![reach_addr]);
    Json(ApiResponse { ok: true, error: None }).into_response()
}

async fn pair_confirm(State(ctx): State<ConnCtx>) -> Response {
    let Some(peer_fp) = ctx.peer else {
        return err_response(StatusCode::UNAUTHORIZED, "缺少对端证书");
    };
    let resp = crate::pairing::on_remote_confirm(&ctx.core, &peer_fp);
    Json(resp).into_response()
}

async fn pair_reject(State(ctx): State<ConnCtx>) -> Response {
    let Some(peer_fp) = ctx.peer else {
        return err_response(StatusCode::UNAUTHORIZED, "缺少对端证书");
    };
    crate::pairing::on_remote_reject(&ctx.core, &peer_fp);
    Json(ApiResponse { ok: true, error: None }).into_response()
}

async fn unpair(State(ctx): State<ConnCtx>) -> Response {
    let Some(peer_fp) = ctx.peer else {
        return err_response(StatusCode::UNAUTHORIZED, "缺少对端证书");
    };
    if ctx.core.storage.remove_paired(&peer_fp).ok().flatten().is_some() {
        ctx.core.emit(Event::DeviceUnpaired { fingerprint: peer_fp });
    }
    Json(ApiResponse { ok: true, error: None }).into_response()
}

async fn recv_text(
    State(ctx): State<ConnCtx>,
    axum::extract::Json(body): axum::extract::Json<TextRequest>,
) -> Response {
    let peer_fp = match require_paired(&ctx) {
        Ok(f) => f,
        Err(r) => return *r,
    };
    if body.text.len() > protocol::MAX_TEXT_LEN {
        return err_response(StatusCode::PAYLOAD_TOO_LARGE, "文本过长");
    }
    let summary = peer_summary(&ctx.core, &peer_fp);
    match transfer::accept_text(&ctx.core, summary, &body.text) {
        Ok(_) => Json(ApiResponse { ok: true, error: None }).into_response(),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn recv_prepare(
    State(ctx): State<ConnCtx>,
    axum::extract::Json(body): axum::extract::Json<PrepareRequest>,
) -> Response {
    let peer_fp = match require_paired(&ctx) {
        Ok(f) => f,
        Err(r) => return *r,
    };
    if body.files.is_empty() || body.files.len() > 512 {
        return err_response(StatusCode::BAD_REQUEST, "文件数为 0 或超过 512");
    }
    let summary = peer_summary(&ctx.core, &peer_fp);
    match ctx.core.incoming.prepare(&ctx.core, &body.id, summary, body.files) {
        Ok(resp) => Json(resp).into_response(),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn chunk_state(
    State(ctx): State<ConnCtx>,
    Path((sid, idx)): Path<(String, usize)>,
) -> Response {
    if require_paired(&ctx).is_err() {
        return err_response(StatusCode::FORBIDDEN, "设备未配对");
    }
    match ctx.core.incoming.state(&sid, idx) {
        Ok(offset) => Json(protocol::FileStateDto { offset }).into_response(),
        Err(DriftError::Remote { status, message }) => err_response(
            axum::http::StatusCode::from_u16(status).unwrap_or(StatusCode::NOT_FOUND),
            message,
        ),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn recv_chunk(
    State(ctx): State<ConnCtx>,
    Path((sid, idx)): Path<(String, usize)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if require_paired(&ctx).is_err() {
        return err_response(StatusCode::FORBIDDEN, "设备未配对");
    }
    let Some(offset) = headers
        .get(protocol::HEADER_OFFSET)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
    else {
        return err_response(StatusCode::BAD_REQUEST, "缺少 x-drift-offset 请求头");
    };
    match ctx.core.incoming.chunk(&sid, idx, offset, &body) {
        Ok(len) => Json(protocol::ChunkAck { offset: len }).into_response(),
        Err(DriftError::Remote { status, message }) => err_response(
            axum::http::StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST),
            message,
        ),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn finish_session(State(ctx): State<ConnCtx>, Path(sid): Path<String>) -> Response {
    if require_paired(&ctx).is_err() {
        return err_response(StatusCode::FORBIDDEN, "设备未配对");
    }
    match ctx.core.incoming.finish(&ctx.core, &sid) {
        Ok(_) => Json(ApiResponse { ok: true, error: None }).into_response(),
        Err(DriftError::Remote { status, message }) => err_response(
            axum::http::StatusCode::from_u16(status).unwrap_or(StatusCode::CONFLICT),
            message,
        ),
        Err(e) => err_response(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    }
}

async fn drop_session(State(ctx): State<ConnCtx>, Path(sid): Path<String>) -> Response {
    if require_paired(&ctx).is_err() {
        return err_response(StatusCode::FORBIDDEN, "设备未配对");
    }
    ctx.core.incoming.drop_session(&sid);
    Json(ApiResponse { ok: true, error: None }).into_response()
}

/// 接受循环：TCP → TLS → 注入指纹 → hyper 服务
pub async fn serve(core: Arc<Core>, listener: TcpListener) -> crate::error::Result<()> {
    let acceptor = TlsAcceptor::from(core.identity.server_config());
    let base = build_router();
    loop {
        let (tcp, remote) = match listener.accept().await {
            Ok(x) => x,
            Err(e) => {
                tracing::warn!("accept 失败: {e}");
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                continue;
            }
        };
        let core2 = core.clone();
        let acceptor2 = acceptor.clone();
        let base2 = base.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(core2, acceptor2, base2, tcp, remote).await {
                tracing::debug!("连接处理结束（{remote}）: {e}");
            }
        });
    }
}

async fn handle_connection(
    core: Arc<Core>,
    acceptor: TlsAcceptor,
    base: Router<ConnCtx>,
    tcp: TcpStream,
    remote: SocketAddr,
) -> crate::error::Result<()> {
    let _ = tcp.set_nodelay(true);
    let tls_stream = acceptor
        .accept(tcp)
        .await
        .map_err(|e| DriftError::Tls(format!("TLS 握手失败: {e}")))?;
    let peer = crate::tls::peer_fingerprint(&tls_stream);
    let ctx = ConnCtx { core, peer, remote };
    let router: axum::Router<()> = base.with_state(ctx);
    // 与 axum::serve 相同的桥接方式：tower Service → hyper HttpService
    let service =
        hyper_util::service::TowerToHyperService::new(router.into_service::<hyper::body::Incoming>());
    let io = hyper_util::rt::TokioIo::new(tls_stream);
    hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new())
        .serve_connection_with_upgrades(io, service)
        .await
        .map_err(|e| DriftError::Protocol(format!("HTTP 连接错误: {e}")))?;
    Ok(())
}
