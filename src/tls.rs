//! TLS 配置与证书锁定：
//! - 客户端：只接受"证书 SHA256 == 期望指纹"的对端（配对或 mDNS 公告的指纹）；
//! - 服务端：要求对端出示证书（mTLS），指纹在 HTTP 层读取，用于身份判定与配对校验。
//!
//! 全部连接为 TLS 1.3 + ed25519 自签名证书。

use std::sync::Arc;

use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{self, ring, WebPkiSupportedAlgorithms},
    pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime},
    server::danger::{ClientCertVerified, ClientCertVerifier},
    CertificateError, DigitallySignedStruct, Error as TlsError, SignatureScheme,
};
use sha2::{Digest, Sha256};

use crate::{error::Result, identity::Fingerprint};

fn provider() -> Arc<crypto::CryptoProvider> {
    Arc::new(ring::default_provider())
}

fn algorithms() -> WebPkiSupportedAlgorithms {
    ring::default_provider().signature_verification_algorithms
}

fn supported_schemes() -> Vec<SignatureScheme> {
    vec![
        SignatureScheme::ED25519,
        SignatureScheme::ECDSA_NISTP256_SHA256,
        SignatureScheme::ECDSA_NISTP384_SHA384,
        SignatureScheme::RSA_PSS_SHA256,
        SignatureScheme::RSA_PKCS1_SHA256,
    ]
}

/// 指纹锁定的服务端证书校验器
#[derive(Debug)]
struct PinVerifier {
    expected: Fingerprint,
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, TlsError> {
        let actual: [u8; 32] = Sha256::digest(end_entity.as_ref()).into();
        if actual == self.expected.0 {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(TlsError::InvalidCertificate(CertificateError::BadEncoding))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, TlsError> {
        crypto::verify_tls12_signature(message, cert, dss, &algorithms())
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, TlsError> {
        crypto::verify_tls13_signature(message, cert, dss, &algorithms())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        supported_schemes()
    }
}

/// 接受任意客户端证书（必须出示）；真实身份由 HTTP 层根据证书指纹判定。
#[derive(Debug)]
struct AnyClientCertVerifier;

impl ClientCertVerifier for AnyClientCertVerifier {
    fn root_hint_subjects(&self) -> &[rustls::DistinguishedName] {
        &[]
    }

    fn verify_client_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> std::result::Result<ClientCertVerified, TlsError> {
        Ok(ClientCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, TlsError> {
        crypto::verify_tls12_signature(message, cert, dss, &algorithms())
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, TlsError> {
        crypto::verify_tls13_signature(message, cert, dss, &algorithms())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        supported_schemes()
    }
}

pub fn server_config(
    cert: &CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
) -> Result<rustls::ServerConfig> {
    let cfg = rustls::ServerConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|e| crate::error::DriftError::Tls(e.to_string()))?
        .with_client_cert_verifier(Arc::new(AnyClientCertVerifier))
        .with_single_cert(vec![cert.clone()], key)
        .map_err(|e| crate::error::DriftError::Tls(e.to_string()))?;
    Ok(cfg)
}

pub fn client_config(
    cert: &CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
    expected: &Fingerprint,
) -> rustls::ClientConfig {
    // unwrap：参数由我们生成，组合不会失败
    rustls::ClientConfig::builder_with_provider(provider())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinVerifier { expected: *expected }))
        .with_client_auth_cert(vec![cert.clone()], key)
        .unwrap()
}

/// 从握手后的 TLS 流中提取对端证书指纹。
pub fn peer_fingerprint<T>(stream: &tokio_rustls::server::TlsStream<T>) -> Option<Fingerprint> {
    let (_, conn) = stream.get_ref();
    let certs = conn.peer_certificates()?;
    let cert = certs.first()?;
    Some(Fingerprint(Sha256::digest(cert.as_ref()).into()))
}
