//! 双节点回环集成测试：跳过 mDNS，走真实 TLS + HTTP 协议栈。
//! 覆盖：配对（一码对照/双向确认）、发文本、发文件（分块）、
//! 逐设备暂停/继续、断点续传、软删除恢复、未配对拒收。

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    time::Duration,
};

use drift::{
    discovery::{PeerDevice, PeerSource},
    Direction, Drift, Event, LaunchConfig, RecordFilter,
};

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("drift-test-{tag}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn launch(tag: &str) -> (drift::DriftHandle, PathBuf) {
    let dir = temp_dir(tag);
    let cfg = LaunchConfig {
        data_dir: Some(dir.join("data")),
        download_dir: Some(dir.join("downloads")),
        name: Some(format!("Drift-{tag}")),
        discovery: Some(false),
        bind: Some(IpAddr::V4(Ipv4Addr::LOCALHOST)),
        ..Default::default()
    };
    let handle = Drift::launch(cfg).expect("启动失败");
    (handle, dir)
}

fn inject_each(other: &drift::DriftHandle, into: &drift::DriftHandle) {
    let info = other.local_device();
    into.add_peer(PeerDevice {
        fingerprint: info.fingerprint,
        name: info.name.clone(),
        kind: info.kind,
        port: info.port,
        addrs: vec![SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), info.port)],
        last_seen: chrono::Utc::now(),
        source: PeerSource::Manual,
    });
}

/// 等待事件满足条件（超时失败）
async fn wait_for<T>(
    rx: &mut tokio::sync::broadcast::Receiver<Event>,
    check: impl Fn(&Event) -> Option<T>,
    timeout: Duration,
    what: &str,
) -> T {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        assert!(!remaining.is_zero(), "等待超时: {what}");
        let event = tokio::time::timeout(remaining, rx.recv())
            .await
            .expect("等待超时")
            .expect("事件通道关闭");
        if let Some(v) = check(&event) {
            return v;
        }
    }
}

fn delivered(record: uuid::Uuid, fp: drift::Fingerprint) -> impl Fn(&Event) -> Option<()> {
    move |e| match e {
        Event::RecordUpdated { id, summary }
            if *id == record && summary.status.starts_with("已送达") =>
        {
            let _ = fp;
            Some(())
        }
        _ => None,
    }
}

#[tokio::test]
async fn end_to_end_pair_text_file_control() {
    let (a, dir_a) = launch("A");
    let (b, dir_b) = launch("B");
    let fp_a = a.local_device().fingerprint;
    let fp_b = b.local_device().fingerprint;
    inject_each(&b, &a);
    inject_each(&a, &b);

    let mut rx_a = a.subscribe();
    let mut rx_b = b.subscribe();

    // ---- 配对 ----
    a.initiate_pairing(&fp_b).await.expect("发起配对");
    let req_b = wait_for(&mut rx_b, |e| match e {
        Event::PairingIncoming { request } => Some(request.clone()),
        _ => None,
    }, Duration::from_secs(10), "B 收到配对请求").await;
    let req_a = wait_for(&mut rx_a, |e| match e {
        Event::PairingStarted { request } => Some(request.clone()),
        _ => None,
    }, Duration::from_secs(10), "A 显示配对码").await;

    // 两端配对码一致（SAS）
    assert_eq!(req_a.code, req_b.code);
    assert_eq!(req_a.code.replace(' ', "").len(), 6);

    // 双向确认
    b.confirm_pairing(&fp_a).await.expect("B 确认");
    a.confirm_pairing(&fp_b).await.expect("A 确认");
    wait_for(&mut rx_a, |e| matches!(e, Event::PairingCompleted { .. }).then_some(()), Duration::from_secs(10), "A 配对完成").await;
    wait_for(&mut rx_b, |e| matches!(e, Event::PairingCompleted { .. }).then_some(()), Duration::from_secs(10), "B 配对完成").await;
    assert!(a.is_paired(&fp_b));
    assert!(b.is_paired(&fp_a));

    // ---- 发文本 ----
    let id = a.send_text("hello from A".to_string(), &[fp_b]).await.expect("发送文本");
    wait_for(&mut rx_a, delivered(id, fp_b), Duration::from_secs(10), "文本送达").await;

    // B 侧应有接收记录
    let recv = b.records(&RecordFilter::default()).unwrap();
    assert_eq!(recv.len(), 1);
    assert_eq!(recv[0].direction, Direction::Received);
    assert!(matches!(&recv[0].payload, drift::Payload::Text { text } if text == "hello from A"));

    // ---- 发送内存字节（剪贴板图片场景）----
    let png_bytes: Vec<u8> = (0..100_000u32).map(|i| (i % 253) as u8).collect();
    let bid = a
        .send_blob("截图.png", Some("image/png".into()), png_bytes.clone(), &[fp_b])
        .await
        .expect("发送字节");
    wait_for(&mut rx_a, delivered(bid, fp_b), Duration::from_secs(10), "字节送达").await;
    let b_all = b.records(&RecordFilter::default()).unwrap();
    assert!(
        b_all.iter().any(|r| matches!(&r.payload, drift::Payload::Files { files } if files[0].name == "截图.png")),
        "B 收到字节记录"
    );

    // ---- 发文件（2.5MB 触发分块）----
    let src = dir_a.join("big.bin");
    let payload: Vec<u8> = (0..2_500_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(&src, &payload).unwrap();
    let fid = a.send_paths(vec![src.clone()], &[fp_b]).await.expect("发送文件");
    wait_for(&mut rx_a, delivered(fid, fp_b), Duration::from_secs(30), "文件送达").await;

    let b_records = b.records(&RecordFilter::default()).unwrap();
    let file_record = b_records.iter().find(|r| r.direction == Direction::Received && matches!(r.payload, drift::Payload::Files { .. })).expect("B 收到文件记录");
    let drift::Payload::Files { files } = &file_record.payload else { unreachable!() };
    assert_eq!(files[0].size, payload.len() as u64);
    let saved = files[0].local_path.as_ref().expect("落盘路径");
    assert_eq!(std::fs::read(saved).unwrap(), payload, "文件内容一致");

    // ---- 逐设备暂停 / 继续（128MB 确保暂停落在传输中） ----
    let big2 = dir_a.join("big2.bin");
    std::fs::write(&big2, vec![7u8; 128 * 1024 * 1024]).unwrap();
    let pid = a.send_paths(vec![big2], &[fp_b]).await.expect("发送第二个文件");
    a.pause(&pid, &fp_b).expect("暂停命令被接受");
    wait_for(&mut rx_a, |e| match e {
        Event::RecordUpdated { id, summary } if *id == pid && summary.status.starts_with("已暂停") => Some(()),
        _ => None,
    }, Duration::from_secs(10), "状态变为已暂停").await;
    // 暂停期间记录里保留进度不清零
    let paused_record = a.record(&pid).expect("记录存在");
    let d = &paused_record.deliveries[0];
    assert_eq!(d.status, drift::DeliveryStatus::Paused);
    assert!(d.transferred < d.total, "暂停时应仍在传输中（{}/{}）", d.transferred, d.total);
    a.resume(&pid, &fp_b).expect("继续命令被接受");
    wait_for(&mut rx_a, delivered(pid, fp_b), Duration::from_secs(120), "暂停后继续并送达").await;

    // ---- 软删除 / 恢复 ----
    let all_a = a.records(&RecordFilter::default()).unwrap();
    let some_id = all_a[0].id;
    a.delete_record(&some_id).unwrap();
    assert!(a.records(&RecordFilter::default()).unwrap().iter().all(|r| r.id != some_id));
    a.restore_record(&some_id).unwrap();
    assert!(a.records(&RecordFilter::default()).unwrap().iter().any(|r| r.id == some_id));

    // ---- CSV 导出 ----
    let csv_path = dir_a.join("records.csv");
    let n = a.export_csv(csv_path.clone(), &RecordFilter::default()).unwrap();
    assert!(n >= 3);
    let csv = std::fs::read_to_string(&csv_path).unwrap();
    assert!(csv.contains("方向"));

    // 清理
    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
    a.shutdown();
    b.shutdown();
}

#[tokio::test]
async fn unpaired_sender_is_rejected() {
    let (a, dir_a) = launch("C");
    let (b, dir_b) = launch("D");
    let fp_b = b.local_device().fingerprint;
    inject_each(&b, &a);

    // 未配对直接发：句柄侧就应拒绝（NotPaired）
    let err = a.send_text("sneak".to_string(), &[fp_b]).await.unwrap_err();
    assert!(matches!(err, drift::DriftError::NotPaired(_)), "实际错误: {err:?}");

    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
    a.shutdown();
    b.shutdown();
}

#[tokio::test]
async fn pairing_code_mismatch_reject_voids_session() {
    let (a, dir_a) = launch("E");
    let (b, dir_b) = launch("F");
    let fp_b = b.local_device().fingerprint;
    inject_each(&b, &a);
    inject_each(&a, &b);

    let mut rx_a = a.subscribe();
    let mut rx_b = b.subscribe();

    a.initiate_pairing(&fp_b).await.unwrap();
    wait_for(&mut rx_b, |e| matches!(e, Event::PairingIncoming { .. }).then_some(()), Duration::from_secs(10), "F 收到请求").await;

    // 一端点「不一致」→ 两端会话作废
    b.reject_pairing(&a.local_device().fingerprint).await.unwrap();
    wait_for(&mut rx_b, |e| matches!(e, Event::PairingRejected { .. }).then_some(()), Duration::from_secs(10), "B 收到作废").await;
    wait_for(&mut rx_a, |e| matches!(e, Event::PairingRejected { .. }).then_some(()), Duration::from_secs(10), "A 收到作废").await;
    assert!(a.pending_pairings().is_empty());
    assert!(!a.is_paired(&fp_b));

    let _ = std::fs::remove_dir_all(&dir_a);
    let _ = std::fs::remove_dir_all(&dir_b);
    a.shutdown();
    b.shutdown();
}
