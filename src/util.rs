//! 杂项工具：本机 IP、文件名处理、CSV、可读大小等。

use std::{
    io,
    net::{IpAddr, SocketAddr, UdpSocket},
    path::Path,
};

use crate::error::{DriftError, Result};

/// 取本机局域网 IP（UDP connect 技巧，不会真正发包）。
pub fn local_ip() -> IpAddr {
    let candidates = ["8.8.8.8:80", "8.8.8.8:53", "1.1.1.1:80"];
    for c in candidates {
        if let Ok(sock) = UdpSocket::bind("0.0.0.0:0")
            && sock.connect(c).is_ok()
            && let Ok(addr) = sock.local_addr()
            && let IpAddr::V4(v4) = addr.ip()
            && !v4.is_loopback()
            && !v4.is_link_local()
        {
            return addr.ip();
        }
    }
    IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
}

/// 把一批地址按"IPv4 优先、回环最后"排序。
pub fn sort_addrs(addrs: &mut [SocketAddr]) {
    addrs.sort_by_key(|a| {
        let score = match a.ip() {
            IpAddr::V4(v4) => {
                if v4.is_loopback() {
                    2
                } else if v4.is_link_local() {
                    3
                } else {
                    0
                }
            }
            IpAddr::V6(v6) => {
                if v6.is_loopback() {
                    2
                } else {
                    1
                }
            }
        };
        (score, a.port())
    });
}

/// 清洗文件名：去掉路径分隔符与 Windows 非法字符，限制长度。
pub fn sanitize_filename(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => out.push('_'),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    let trimmed = out.trim().trim_matches('.').to_string();
    let mut s: String = trimmed.chars().take(180).collect();
    if s.is_empty() {
        s.push_str("file");
    }
    s
}

/// 目标路径已存在时自动加 " (n)" 后缀，直到不冲突。
pub fn unique_target_path(dir: &Path, name: &str) -> std::path::PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let stem = Path::new(name).file_stem().map(|s| s.to_string_lossy().into_owned());
    let ext = Path::new(name).extension().map(|s| s.to_string_lossy().into_owned());
    let stem = stem.unwrap_or_else(|| name.to_string());
    for i in 2..u32::MAX {
        let candidate = match &ext {
            Some(e) => dir.join(format!("{stem} ({i}).{e}")),
            None => dir.join(format!("{stem} ({i})")),
        };
        if !candidate.exists() {
            return candidate;
        }
    }
    first
}

/// 人类可读的文件大小（如 "8.4 MB"）。
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut v = bytes as f64;
    let mut unit = 0usize;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if v >= 100.0 {
        format!("{v:.0} {}", UNITS[unit])
    } else if v >= 10.0 {
        format!("{v:.1} {}", UNITS[unit])
    } else {
        format!("{v:.2} {}", UNITS[unit])
    }
}

/// 极简 CSV 字段转义（含逗号/引号/换行时加引号）。
pub fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// 递归压缩目录为 zip，返回 zip 路径与总大小。阻塞函数，请放在 spawn_blocking 里。
pub fn zip_dir(dir: &Path, dest: &Path) -> Result<u64> {
    use std::io::Write;

    use zip::ZipWriter;

    let file = std::fs::File::create(dest)
        .map_err(|e| DriftError::Zip(format!("创建压缩文件失败: {e}")))?;
    let mut writer = ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let base = dir.to_path_buf();
    let mut stack = vec![base.clone()];
    while let Some(cur) = stack.pop() {
        for entry in std::fs::read_dir(&cur)
            .map_err(|e| DriftError::Zip(format!("读取目录失败: {e}")))?
        {
            let entry = entry.map_err(|e| DriftError::Zip(format!("读取目录失败: {e}")))?;
            let path = entry.path();
            let rel = path
                .strip_prefix(&base)
                .map_err(|e| DriftError::Zip(e.to_string()))?;
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            if path.is_dir() {
                if !rel_str.is_empty() {
                    writer
                        .add_directory(&rel_str, options)
                        .map_err(|e| DriftError::Zip(format!("写入 zip 失败: {e}")))?;
                }
                stack.push(path);
            } else {
                writer
                    .start_file(&rel_str, options)
                    .map_err(|e| DriftError::Zip(format!("写入 zip 失败: {e}")))?;
                let mut f = std::fs::File::open(&path)
                    .map_err(|e| DriftError::Zip(format!("打开文件失败: {e}")))?;
                std::io::copy(&mut f, &mut writer)
                    .map_err(|e| DriftError::Zip(format!("写入 zip 失败: {e}")))?;
                let _ = writer.flush();
            }
        }
    }
    writer
        .finish()
        .map_err(|e| DriftError::Zip(format!("收尾 zip 失败: {e}")))?;
    let size = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
    Ok(size)
}

/// 把整个文件读成 String（限制大小，防止误读超大文件）。阻塞函数。
pub fn read_text_limited(path: &Path, limit: u64) -> Result<String> {
    let meta = std::fs::metadata(path)?;
    if meta.len() > limit {
        return Err(DriftError::Invalid(format!(
            "文件过大（超过 {}）",
            human_bytes(limit)
        )));
    }
    let mut s = String::new();
    use std::io::Read;
    std::fs::File::open(path)?.read_to_string(&mut s)?;
    Ok(s)
}

/// 忽略错误的目录删除。
pub fn rm_rf(path: &Path) {
    if path.is_dir() {
        let _ = std::fs::remove_dir_all(path);
    } else if path.exists() {
        let _ = std::fs::remove_file(path);
    }
}

/// 供日志使用：把 io 错误变成字符串。
pub fn ioerr(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// 写文件（原子替换：先写临时文件再 rename）。
pub fn atomic_write(path: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data)?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            // Windows 上 rename 目标已存在时可能失败，退回复制
            match std::fs::copy(&tmp, path) {
                Ok(_) => {
                    let _ = std::fs::remove_file(&tmp);
                    Ok(())
                }
                Err(_) => Err(e),
            }
        }
    }
}
