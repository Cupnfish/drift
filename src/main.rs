//! drift 命令行演示：无 GUI 跑通「发现 → 配对 → 发文本/文件 → 逐设备控制」全流程。
//!
//! 用法：
//!   drift [--data-dir DIR] [--name NAME] [--no-discovery] [--port PORT]
//!
//! 交互命令：
//!   devices                     列出附近设备
//!   paired                      列出已配对设备
//!   pair <指纹>                 发起配对（对端同时确认）
//!   yes <指纹>                  核对一致，确认配对
//!   no <指纹>                   不一致，作废
//!   send <指纹> <文本>          发送文本
//!   file <指纹> <路径>...      发送文件/文件夹（文件夹自动 zip）
//!   records                     查看全部记录
//!   pause / resume / cancel / retry <记录ID> <指纹>
//!   del <记录ID>                软删除记录
//!   undo <记录ID>               恢复记录
//!   csv <路径>                  导出全部记录 CSV
//!   unpair <指纹>               解除配对
//!   quit

use std::io::{BufRead, Write};

use drift::{
    discovery::PeerDevice,
    DeviceKind, Drift, LaunchConfig, PayloadKind, RecordFilter, TextExportFormat,
};

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn".into()),
        )
        .init();

    let mut cfg = LaunchConfig::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--data-dir" => cfg.data_dir = args.next().map(std::path::PathBuf::from),
            "--name" => cfg.name = args.next(),
            "--no-discovery" => cfg.discovery = Some(false),
            "--port" => cfg.port = args.next().and_then(|p| p.parse().ok()),
            other => {
                eprintln!("未知参数: {other}");
                std::process::exit(2);
            }
        }
    }

    let handle = match Drift::launch(cfg) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("启动失败: {e}");
            std::process::exit(1);
        }
    };
    let local = handle.local_device();
    println!("Drift 已启动：{}（{}）端口 {}", local.name, local.fingerprint, local.port);
    println!("输入 help 查看命令。\n");

    // 事件打印线程
    {
        let mut rx = handle.subscribe();
        std::thread::Builder::new()
            .name("drift-events".into())
            .spawn(move || {
                loop {
                    match rx.blocking_recv() {
                        Ok(event) => println!("  [事件] {}", event.brief()),
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            eprintln!("  [事件] 落后 {n} 条")
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            })
            .ok();
    }

    let stdin = std::io::stdin();
    let mut line = String::new();
    loop {
        print!("drift> ");
        let _ = std::io::stdout().flush();
        line.clear();
        if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let parts: Vec<String> = line.split_whitespace().map(String::from).collect();
        if parts.is_empty() {
            continue;
        }
        let cmd = parts[0].as_str();
        let result = match cmd {
            "help" => {
                print_help();
                Ok(())
            }
            "devices" => {
                for d in handle.nearby() {
                    println!(
                        "  {}  {}（{}） {:?}",
                        d.fingerprint,
                        d.name,
                        d.kind.as_str(),
                        d.addrs
                    );
                }
                Ok(())
            }
            "paired" => {
                for p in handle.paired() {
                    println!("  {}  {} 在线={} 配对于 {}", p.fingerprint, p.name, p.online, p.paired_at);
                }
                Ok(())
            }
            "pair" => arg1(&parts, |fp| handle.blocking().initiate_pairing(&fp)),
            "yes" => arg1(&parts, |fp| handle.blocking().confirm_pairing(&fp)),
            "no" => arg1(&parts, |fp| handle.blocking().reject_pairing(&fp)),
            "send" => send_text(&handle, &parts),
            "file" => send_files(&handle, &parts),
            "records" => {
                for r in handle.records(&RecordFilter::default()).unwrap_or_default() {
                    let devices = r
                        .deliveries
                        .iter()
                        .map(|d| format!("{}:{}", d.device.name, d.status.label()))
                        .collect::<Vec<_>>()
                        .join(" ");
                    println!(
                        "  {} [{}] {} | {} | {}",
                        r.id,
                        r.direction.label(),
                        r.summary().title,
                        match r.payload.kind() {
                            PayloadKind::Text => "文本".to_string(),
                            k => k.label().to_string(),
                        },
                        if devices.is_empty() {
                            r.peer.map(|p| format!("来自 {}", p.name)).unwrap_or_default()
                        } else {
                            devices
                        }
                    );
                }
                Ok(())
            }
            "pause" => with_record_target(&parts, |rid, fp| handle.pause(&rid, &fp).map(|_| ())),
            "resume" => with_record_target(&parts, |rid, fp| handle.resume(&rid, &fp).map(|_| ())),
            "cancel" => with_record_target(&parts, |rid, fp| handle.cancel_target(&rid, &fp).map(|_| ())),
            "retry" => with_record_target(&parts, |rid, fp| handle.retry(&rid, &fp).map(|_| ())),
            "del" => arg_uuid(&parts, |id| handle.delete_record(&id).map(|_| ())),
            "undo" => arg_uuid(&parts, |id| handle.restore_record(&id).map(|_| ())),
            "csv" => {
                let path = parts.get(1).map(std::path::PathBuf::from);
                match path {
                    Some(p) => handle
                        .export_csv(p, &RecordFilter::default())
                        .map(|n| println!("已导出 {n} 条")),
                    None => Err(drift::DriftError::Invalid("用法: csv <路径>".into())),
                }
            }
            "export-text" => {
                let (id, path) = (parts.get(1), parts.get(2));
                match (id, path) {
                    (Some(id), Some(path)) => {
                        let Ok(id) = uuid::Uuid::parse_str(id) else {
                            println!("  错误: 记录 ID 格式错误");
                            continue;
                        };
                        handle
                            .export_text(&id, path.into(), TextExportFormat::Txt)
                            .map(|_| println!("已导出"))
                    }
                    _ => Err(drift::DriftError::Invalid("用法: export-text <记录ID> <路径>".into())),
                }
            }
            "unpair" => arg1(&parts, |fp| handle.blocking().unpair(&fp)),
            "rename" => {
                let name = parts[1..].join(" ");
                handle.set_device_name(&name).map(|_| println!("已改名"))
            }
            "inject" => {
                // inject <指纹> <ip:port> <名称> [kind]
                let (fp, addr, name) = (parts.get(1), parts.get(2), parts.get(3));
                match (fp, addr, name) {
                    (Some(fp), Some(addr), Some(name)) => {
                        let Ok(fp) = drift::Fingerprint::from_hex(fp) else {
                            println!("  错误: 指纹格式错误");
                            continue;
                        };
                        let Ok(addr) = addr.parse::<std::net::SocketAddr>() else {
                            println!("  错误: 地址格式错误");
                            continue;
                        };
                        let kind = parts
                            .get(4)
                            .and_then(|k| DeviceKind::parse(k))
                            .unwrap_or(DeviceKind::Other);
                        handle.add_peer(PeerDevice {
                            fingerprint: fp,
                            name: name.clone(),
                            kind,
                            port: addr.port(),
                            addrs: vec![addr],
                            last_seen: chrono::Utc::now(),
                            source: drift::discovery::PeerSource::Manual,
                        });
                        println!("已注入");
                        Ok(())
                    }
                    _ => Err(drift::DriftError::Invalid(
                        "用法: inject <指纹> <ip:port> <名称> [kind]".into(),
                    )),
                }
            }
            "quit" | "exit" => break,
            other => Err(drift::DriftError::Invalid(format!("未知命令 {other}，help 查看"))),
        };
        if let Err(e) = result {
            println!("  错误: {e}");
        }
    }

    handle.shutdown();
}

fn print_help() {
    println!(
        "devices | paired | pair <fp> | yes <fp> | no <fp> | unpair <fp>\n\
         send <fp> <文本> | file <fp> <路径>...\n\
         records | pause/resume/cancel/retry <记录ID> <fp> | del/undo <记录ID>\n\
         csv <路径> | export-text <记录ID> <路径> | rename <新名字>\n\
         inject <指纹> <ip:port> <名称> [kind] | quit"
    );
}

fn arg1<F>(parts: &[String], f: F) -> drift::Result<()>
where
    F: FnOnce(drift::Fingerprint) -> drift::Result<()>,
{
    let Some(fp) = parts.get(1) else {
        return Err(drift::DriftError::Invalid("缺少指纹参数".into()));
    };
    let fp = drift::Fingerprint::from_hex(fp)?;
    f(fp)
}

fn arg_uuid<F>(parts: &[String], f: F) -> drift::Result<()>
where
    F: FnOnce(uuid::Uuid) -> drift::Result<()>,
{
    let Some(id) = parts.get(1) else {
        return Err(drift::DriftError::Invalid("缺少记录 ID".into()));
    };
    let id = uuid::Uuid::parse_str(id).map_err(|e| drift::DriftError::Invalid(e.to_string()))?;
    f(id)
}

fn with_record_target<F>(parts: &[String], f: F) -> drift::Result<()>
where
    F: FnOnce(uuid::Uuid, drift::Fingerprint) -> drift::Result<()>,
{
    let (Some(id), Some(fp)) = (parts.get(1), parts.get(2)) else {
        return Err(drift::DriftError::Invalid("用法: <命令> <记录ID> <指纹>".into()));
    };
    let id = uuid::Uuid::parse_str(id).map_err(|e| drift::DriftError::Invalid(e.to_string()))?;
    let fp = drift::Fingerprint::from_hex(fp)?;
    f(id, fp)
}

fn send_text(handle: &drift::DriftHandle, parts: &[String]) -> drift::Result<()> {
    let Some(fp) = parts.get(1) else {
        return Err(drift::DriftError::Invalid("用法: send <指纹> <文本>".into()));
    };
    let fp = drift::Fingerprint::from_hex(fp)?;
    let text = parts[2..].join(" ");
    if text.is_empty() {
        return Err(drift::DriftError::Invalid("文本为空".into()));
    }
    let id = handle.blocking().send_text(text, &[fp])?;
    println!("记录 {id} 已创建");
    Ok(())
}

fn send_files(handle: &drift::DriftHandle, parts: &[String]) -> drift::Result<()> {
    let Some(fp) = parts.get(1) else {
        return Err(drift::DriftError::Invalid("用法: file <指纹> <路径>...".into()));
    };
    let fp = drift::Fingerprint::from_hex(fp)?;
    let paths: Vec<std::path::PathBuf> = parts[2..].iter().map(Into::into).collect();
    if paths.is_empty() {
        return Err(drift::DriftError::Invalid("缺少文件路径".into()));
    }
    let id = handle.blocking().send_paths(paths, &[fp])?;
    println!("记录 {id} 已创建");
    Ok(())
}
