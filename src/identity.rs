//! 设备身份：指纹、设备类型、自签名 ed25519 证书的生成与持久化。
//!
//! 指纹 = SHA256(整张 DER 证书)。配对即锁定指纹；TLS 校验也用它。

use std::{
    fmt,
    path::Path,
    sync::{Arc, Mutex},
};

use rcgen::{CertificateParams, KeyPair, PKCS_ED25519};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

use crate::{
    error::{DriftError, Result},
    tls,
};

/// 设备指纹（32 字节，十六进制展示）
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Fingerprint(pub [u8; 32]);

impl Fingerprint {
    pub fn from_bytes(b: [u8; 32]) -> Self {
        Fingerprint(b)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(s: &str) -> Result<Self> {
        let v = hex::decode(s.trim())
            .map_err(|e| DriftError::Invalid(format!("指纹格式错误: {e}")))?;
        if v.len() != 32 {
            return Err(DriftError::Invalid("指纹长度必须为 32 字节".into()));
        }
        let mut b = [0u8; 32];
        b.copy_from_slice(&v);
        Ok(Fingerprint(b))
    }

    /// 展示用短指纹（前 8 位）
    pub fn short(&self) -> String {
        self.to_hex()[..8].to_string()
    }
}

impl fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint({})", self.to_hex())
    }
}

impl Serialize for Fingerprint {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Fingerprint {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Fingerprint::from_hex(&s).map_err(serde::de::Error::custom)
    }
}

/// 设备类型（用于 GUI 图标）
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceKind {
    Desktop,
    Laptop,
    Phone,
    Tablet,
    Watch,
    Tv,
    Other,
}

impl DeviceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            DeviceKind::Desktop => "desktop",
            DeviceKind::Laptop => "laptop",
            DeviceKind::Phone => "phone",
            DeviceKind::Tablet => "tablet",
            DeviceKind::Watch => "watch",
            DeviceKind::Tv => "tv",
            DeviceKind::Other => "other",
        }
    }
}

impl std::str::FromStr for DeviceKind {
    type Err = crate::error::DriftError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "desktop" => Ok(DeviceKind::Desktop),
            "laptop" => Ok(DeviceKind::Laptop),
            "phone" => Ok(DeviceKind::Phone),
            "tablet" => Ok(DeviceKind::Tablet),
            "watch" => Ok(DeviceKind::Watch),
            "tv" => Ok(DeviceKind::Tv),
            "other" => Ok(DeviceKind::Other),
            _ => Err(crate::error::DriftError::Invalid(format!("未知设备类型: {s}"))),
        }
    }
}

impl DeviceKind {
    pub fn parse(s: &str) -> Option<Self> {
        s.parse().ok()
    }

    pub fn default_for_current_os() -> Self {
        if cfg!(any(target_os = "android", target_os = "ios")) {
            DeviceKind::Phone
        } else if cfg!(target_os = "macos") {
            DeviceKind::Laptop
        } else {
            DeviceKind::Desktop
        }
    }
}

/// 本机 / 对端设备的基础信息
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub fingerprint: Fingerprint,
    pub name: String,
    pub kind: DeviceKind,
    pub port: u16,
}

/// TLS 材料：证书 + 私钥（从 Identity 派生，供客户端连接使用）
pub struct TlsMaterial {
    pub cert: CertificateDer<'static>,
    pub key: PrivateKeyDer<'static>,
}

impl Clone for TlsMaterial {
    fn clone(&self) -> Self {
        TlsMaterial {
            cert: self.cert.clone(),
            key: self.key.clone_key(),
        }
    }
}

/// 设备身份：自签名 ed25519 证书，持久化在配置目录。
pub struct Identity {
    cert: CertificateDer<'static>,
    key: PrivateKeyDer<'static>,
    fingerprint: Fingerprint,
    server_config: rustls::ServerConfig,
    client_configs: Mutex<std::collections::HashMap<Fingerprint, Arc<rustls::ClientConfig>>>,
}

impl Identity {
    /// 加载或首次生成身份。文件为 `identity.key`（PKCS#8 DER）与 `identity.crt`（DER）。
    pub fn load_or_create(config_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(config_dir)
            .map_err(|e| DriftError::Dir(format!("创建配置目录失败: {e}")))?;
        let key_path = config_dir.join("identity.key");
        let cert_path = config_dir.join("identity.crt");

        let (cert_der, key_der, fingerprint) =
            if key_path.exists() && cert_path.exists() {
                let key_der = std::fs::read(&key_path)?;
                let cert_der = std::fs::read(&cert_path)?;
                let fingerprint = fingerprint_of(&cert_der)?;
                (cert_der, key_der, fingerprint)
            } else {
                let (cert_der, key_der, fingerprint) = generate()?;
                crate::util::atomic_write(&key_path, &key_der)?;
                crate::util::atomic_write(&cert_path, &cert_der)?;
                (cert_der, key_der, fingerprint)
            };

        let cert = CertificateDer::from(cert_der);
        let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der));
        let server_config = tls::server_config(&cert, key.clone_key())?;
        Ok(Identity {
            cert,
            key,
            fingerprint,
            server_config,
            client_configs: Mutex::new(std::collections::HashMap::new()),
        })
    }

    pub fn fingerprint(&self) -> Fingerprint {
        self.fingerprint
    }

    pub fn tls_material(&self) -> TlsMaterial {
        TlsMaterial {
            cert: self.cert.clone(),
            key: self.key.clone_key(),
        }
    }

    pub fn server_config(&self) -> Arc<rustls::ServerConfig> {
        Arc::new(self.server_config.clone())
    }

    /// 指向特定设备（按指纹锁定）的 TLS 客户端配置，带缓存。
    pub fn client_config_for(&self, expected: &Fingerprint) -> Arc<rustls::ClientConfig> {
        if let Some(cfg) = self.client_configs.lock().unwrap().get(expected) {
            return cfg.clone();
        }
        let cfg = Arc::new(tls::client_config(&self.cert, self.key.clone_key(), expected));
        self.client_configs
            .lock()
            .unwrap()
            .insert(*expected, cfg.clone());
        cfg
    }
}

fn fingerprint_of(cert_der: &[u8]) -> Result<Fingerprint> {
    Ok(Fingerprint(Sha256::digest(cert_der).into()))
}

fn generate() -> Result<(Vec<u8>, Vec<u8>, Fingerprint)> {
    let key = KeyPair::generate_for(&PKCS_ED25519)
        .map_err(|e| DriftError::Tls(format!("生成密钥失败: {e}")))?;
    let mut params = CertificateParams::new(vec!["drift.device.local".to_string()])
        .map_err(|e| DriftError::Tls(format!("证书参数错误: {e}")))?;
    params.distinguished_name.push(
        rcgen::DnType::CommonName,
        format!("Drift-{}", hex::encode(&key.public_key_raw()[..8])),
    );
    let cert = params
        .self_signed(&key)
        .map_err(|e| DriftError::Tls(format!("签发证书失败: {e}")))?;
    let cert_der = cert.der().to_vec();
    let key_der = key.serialize_der();
    let fingerprint = fingerprint_of(&cert_der)?;
    Ok((cert_der, key_der, fingerprint))
}
