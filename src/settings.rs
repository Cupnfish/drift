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

// —— Android 路径（dirs 不支持 Android：home/config/download 全返回 None，
//    应用进程的 cwd 是只读的 /，回退 "." 会把目录建到根上 → EROFS） ——

/// 本应用的包名（`/proc/self/cmdline` 首段），如 `dev.drift.app`。
#[cfg(target_os = "android")]
pub fn android_package() -> Option<String> {
    let cmdline = std::fs::read_to_string("/proc/self/cmdline").ok()?;
    let pkg = cmdline.split('\0').next()?.trim();
    (!pkg.is_empty() && !pkg.contains('/')).then(|| pkg.to_string())
}

/// 应用私有目录的父级：user 0 是 `/data/data`，工作资料等其它 userId 是
/// `/data/user/<userId>`（uid = userId × 100000 + appId）。
#[cfg(target_os = "android")]
fn android_data_root() -> Option<String> {
    let uid: u32 = std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find(|l| l.starts_with("Uid:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    Some(if uid >= 1_000_000 {
        format!("/data/user/{}", uid / 100_000)
    } else {
        "/data/data".to_string()
    })
}

/// 应用内部私有目录（`Context.getFilesDir()` 等价物）：配置 / 身份 / 数据库。
#[cfg(target_os = "android")]
pub fn android_files_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(format!(
        "{}/{}",
        android_data_root()?,
        android_package()?
    ))
    .join("files");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// 接收目录默认值：首选应用外部私有存储（`/storage/emulated/0/Android/data/<pkg>/files`，
/// 免存储权限、USB / 文件管理器可见）；建不成（部分 ROM 限制 / 挂载时序）退回内部
/// 私有目录，保证可写。公开的「下载」目录要走 MediaStore，留待后续接入。
#[cfg(target_os = "android")]
fn android_default_download_dir() -> PathBuf {
    if let Some(pkg) = android_package() {
        let ext = PathBuf::from("/storage/emulated/0/Android/data")
            .join(pkg)
            .join("files");
        if std::fs::create_dir_all(&ext).is_ok() {
            return ext;
        }
    }
    android_files_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Drift")
}

impl Default for Settings {
    fn default() -> Self {
        #[cfg(target_os = "android")]
        let download_dir = android_default_download_dir();
        #[cfg(not(target_os = "android"))]
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
