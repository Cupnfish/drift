//! 设置：持久化为 `settings.json`，GUI 可读写。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    error::{DriftError, Result},
    identity::DeviceKind,
    util,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Settings {
    /// 本机展示名（如 "MacBook Pro"）
    pub device_name: String,
    /// 本机设备类型（GUI 图标用）
    pub device_kind: DeviceKind,
    /// 接收目录（"另存为 / 打开"的默认位置）
    pub download_dir: PathBuf,
    /// 服务端口；0 表示首次随机分配并回写
    pub port: u16,
    /// 是否开启 mDNS 自动发现
    pub discovery: bool,
    /// 历史记录上限（超出裁剪最旧记录）
    pub max_records: usize,
}

impl Settings {
    pub fn load_or_create(config_dir: &Path) -> Result<Settings> {
        std::fs::create_dir_all(config_dir)
            .map_err(|e| DriftError::Dir(format!("创建配置目录失败: {e}")))?;
        let path = config_dir.join("settings.json");
        if path.exists() {
            let raw = std::fs::read_to_string(&path)?;
            let mut s: Settings = serde_json::from_str(&raw)?;
            s.download_dir = absolutize(&s.download_dir);
            return Ok(s);
        }
        let s = Settings::default();
        s.save(config_dir)?;
        Ok(s)
    }

    pub fn save(&self, config_dir: &Path) -> Result<()> {
        let path = config_dir.join("settings.json");
        let raw = serde_json::to_string_pretty(self)?;
        util::atomic_write(&path, raw.as_bytes())
            .map_err(|e| DriftError::Dir(format!("写入设置失败: {e}")))?;
        Ok(())
    }
}

fn absolutize(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        // 相对路径锚定到用户主目录，避免随工作目录漂移
        dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(p)
    }
}

impl Default for Settings {
    fn default() -> Self {
        let download_dir = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Drift");
        Settings {
            device_name: default_device_name(),
            device_kind: DeviceKind::default_for_current_os(),
            download_dir,
            port: 0,
            discovery: true,
            max_records: 5000,
        }
    }
}

pub fn default_device_name() -> String {
    gethostname::gethostname()
        .to_str()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Drift 设备".to_string())
}
