//! 出站 HTTP 客户端：每台目标设备一条可复用的 TLS(hyper http1) 连接。
//!
//! 安全模型：连接前已知对端指纹（mDNS 公告 / 配对锁定），
//! TLS 握手时校验证书指纹必须匹配；本端同时出示自己的证书（mTLS），
//! 对端凭证书指纹识别我们，不需要在请求体里自报身份。

use std::{
    net::SocketAddr,
    sync::{Arc, RwLock},
    time::Duration,
};

use axum::http::{header, Method, Request, StatusCode};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::client::conn::http1::SendRequest;
use serde::{de::DeserializeOwned, Serialize};
use tokio::net::TcpStream;

use crate::{
    error::{DriftError, Result},
    identity::{Fingerprint, TlsMaterial},
    util,
};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

pub struct PeerLink {
    fingerprint: Fingerprint,
    addrs: RwLock<Vec<SocketAddr>>,
    tls: TlsMaterial,
    conn: tokio::sync::Mutex<Option<SendRequest<Full<Bytes>>>>,
}

impl PeerLink {
    pub fn new(fingerprint: Fingerprint, addrs: Vec<SocketAddr>, tls: TlsMaterial) -> Self {
        PeerLink {
            fingerprint,
            addrs: RwLock::new(addrs),
            tls,
            conn: tokio::sync::Mutex::new(None),
        }
    }

    pub fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }

    pub fn update_addrs(&self, addrs: Vec<SocketAddr>) {
        *self.addrs.write().unwrap() = addrs;
    }

    /// 发请求（自动重连一次）；返回 (状态码, 响应体)
    pub async fn request(
        &self,
        method: Method,
        path: &str,
        body: Option<Vec<u8>>,
        content_type: &str,
        extra_headers: &[(String, String)],
    ) -> Result<(u16, Vec<u8>)> {
        let mut last_err: Option<DriftError> = None;
        for attempt in 0..2 {
            let mut guard = self.conn.lock().await;
            if guard.is_none() {
                match self.connect().await {
                    Ok(sr) => *guard = Some(sr),
                    Err(e) => {
                        last_err = Some(e);
                        continue;
                    }
                }
            }
            let mut sr = guard.take().expect("连接刚建立");

            let body = Full::new(Bytes::from(body.clone().unwrap_or_default()));
            let mut builder = Request::builder()
                .method(method.clone())
                .uri(path)
                .header(header::CONTENT_TYPE, content_type);
            for (k, v) in extra_headers {
                builder = builder.header(k.as_str(), v.as_str());
            }
            let req = builder
                .body(body)
                .map_err(|e| DriftError::Protocol(format!("构造请求失败: {e}")))?;

            let outcome = tokio::time::timeout(REQUEST_TIMEOUT, async {
                let resp = sr.send_request(req).await?;
                let status = resp.status();
                let bytes = resp.into_body().collect().await?.to_bytes();
                Ok::<_, hyper::Error>((status, bytes.to_vec()))
            })
            .await;

            match outcome {
                Ok(Ok((status, bytes))) => {
                    *guard = Some(sr);
                    let code = status.as_u16();
                    if code >= 400 {
                        return Err(remote_error(code, &bytes));
                    }
                    return Ok((code, bytes));
                }
                Ok(Err(e)) => {
                    last_err = Some(DriftError::Protocol(format!("请求失败: {e}")));
                }
                Err(_) => {
                    last_err = Some(DriftError::Protocol("请求超时".into()));
                }
            }
            guard.take(); // 连接已废
            if attempt == 0 {
                continue;
            }
        }
        Err(last_err.unwrap_or_else(|| DriftError::Protocol("连接失败".into())))
    }

    async fn connect(&self) -> Result<SendRequest<Full<Bytes>>> {
        let addrs = self.addrs.read().unwrap().clone();
        let mut last_err: Option<DriftError> = None;
        for addr in &addrs {
            let tcp = match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr)).await {
                Ok(Ok(tcp)) => tcp,
                Ok(Err(e)) => {
                    last_err = Some(DriftError::Io(e));
                    continue;
                }
                Err(_) => {
                    last_err = Some(DriftError::Protocol("TCP 连接超时".into()));
                    continue;
                }
            };
            let _ = tcp.set_nodelay(true);

            let connector = tokio_rustls::TlsConnector::from(Arc::new(
                crate::tls::client_config(&self.tls.cert, self.tls.key.clone_key(), &self.fingerprint),
            ));
            let name = rustls::pki_types::ServerName::IpAddress(addr.ip().into());
            match connector.connect(name, tcp).await {
                Ok(tls_stream) => {
                    let (sr, conn) =
                        hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(tls_stream))
                            .await
                            .map_err(|e| DriftError::Protocol(format!("HTTP 握手失败: {e}")))?;
                    // 后台驱动连接
                    tokio::spawn(async move {
                        let _ = conn.await;
                    });
                    return Ok(sr);
                }
                Err(e) => {
                    last_err = Some(DriftError::Tls(format!("TLS 握手失败: {e}")));
                }
            }
        }
        Err(last_err.unwrap_or_else(|| {
            DriftError::UnknownDevice(format!("无可达地址：{}", self.fingerprint))
        }))
    }

    // ---------- JSON 便捷方法 ----------

    pub async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let (code, body) = self.request(Method::GET, path, None, "application/json", &[]).await?;
        parse_json(code, &body)
    }

    pub async fn send_json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        payload: impl Serialize,
    ) -> Result<T> {
        let bytes = serde_json::to_vec(&payload)?;
        let (code, body) = self
            .request(method, path, Some(bytes), "application/json", &[])
            .await?;
        parse_json(code, &body)
    }

    /// 上传一个数据块（断点续传：offset 为当前文件偏移）
    pub async fn put_chunk(
        &self,
        path: &str,
        offset: u64,
        chunk: Vec<u8>,
    ) -> Result<u64> {
        let headers = vec![(crate::protocol::HEADER_OFFSET.to_string(), offset.to_string())];
        let (code, body) = self
            .request(Method::PUT, path, Some(chunk), "application/octet-stream", &headers)
            .await?;
        let ack: crate::protocol::ChunkAck = parse_json(code, &body)?;
        Ok(ack.offset)
    }
}

fn parse_json<T: DeserializeOwned>(code: u16, body: &[u8]) -> Result<T> {
    if code >= 400 {
        return Err(remote_error(code, body));
    }
    serde_json::from_slice(body).map_err(|e| DriftError::Protocol(format!("解析响应失败: {e}: {}", util::ioerr(String::from_utf8_lossy(body)))))
}

fn remote_error(code: u16, body: &[u8]) -> DriftError {
    let message = serde_json::from_slice::<crate::protocol::ApiResponse>(body)
        .ok()
        .and_then(|r| r.error)
        .unwrap_or_else(|| {
            StatusCode::from_u16(code)
                .map(|s| s.canonical_reason().unwrap_or("未知错误").to_string())
                .unwrap_or_else(|_| "未知错误".into())
        });
    DriftError::Remote { status: code, message }
}
