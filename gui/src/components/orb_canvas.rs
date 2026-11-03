//! 设备球体画布（drift-main-desktop / drift-main-mobile v1.5）：
//! - 没有发送键：输入框有内容（文本或附件）球体才浮出，清空即收起；
//! - 状态只画在球体的环上：紫进度环＝传输中（球面标百分比）、橙虚线环＝
//!   内容改过（点按只补发差异）、绿整环＋角标＝已同步、灰部分环＝已暂停、
//!   无环＝待发。差异文案写在那一台球体下方，不搬进输入框；
//! - 单击球体即发送，可连续点几台；传输中的球再点一次暂停，暂停的点按继续；
//! - 「+ 添加设备」是画布上的虚线球，配对完成后新球从加号球附近展开；
//! - 无限画布：拖空白处平移（>4px 才算拖拽，否则仍是点击）、双指 /
//!   Ctrl+滚轮缩放，被窗口裁掉的球体露在边缘，右上小地图标出视口与窗外球。

use std::{collections::HashSet, rc::Rc};

use drift::{DeliveryStatus, Fingerprint, PairedDeviceView, Record};
use nsh_core::prelude::*;

use crate::{
    icons as ic,
    motion as mo,
    palette as p,
    state::{AppState, Attachment, ContentDelta, NeverEq, SentSnapshot},
    ui,
};

// —— 布局常量（按板取值，桌面 / 移动各一套） ——

/// 球体核心直径区间（不含状态环）。
fn core_range(mobile: bool) -> (f32, f32) {
    if mobile { (40.0, 84.0) } else { (64.0, 120.0) }
}

/// 世界盒半宽 / 半高（球体落位以中心为原点；固定尺寸保证落位稳定）。
fn world_half(mobile: bool) -> (f32, f32) {
    if mobile { (260.0, 390.0) } else { (800.0, 500.0) }
}

/// 视口标称尺寸（小地图换算用；实际窗口可变，指示性足够）。
fn nominal_viewport(mobile: bool) -> (f32, f32) {
    if mobile { (370.0, 500.0) } else { (1000.0, 540.0) }
}

/// 球心到状态环外缘的间隙（环直径 = 球心直径 + 2 × gap）。
fn ring_gap(core_d: f32, mobile: bool) -> f32 {
    (core_d * 0.16).clamp(if mobile { 6.0 } else { 9.0 }, if mobile { 15.0 } else { 22.0 })
}

/// 黄金角螺旋落位：第 0 颗在中心偏上，之后按 137.5° 展开并压扁成椭圆贴横向画布。
fn spiral_pos(index: usize, mobile: bool) -> (f32, f32) {
    if index == 0 {
        return (0.0, if mobile { -36.0 } else { -70.0 });
    }
    let base = if mobile { 54.0 } else { 88.0 };
    let step = if mobile { 27.0 } else { 42.0 };
    let r = base + step * (index as f32).sqrt();
    let a = index as f32 * 137.508_f32.to_radians();
    (r * a.cos(), r * a.sin() * 0.74)
}

/// 稳定指纹散列（球体大小 / 浮动相位用，与状态无关 —— 环粗细与球体大小
/// 不随状态变化，避免误读为进度）。
fn fp_hash(fp: &Fingerprint) -> u32 {
    fp.to_hex()
        .bytes()
        .fold(2166136261u32, |h, b| h.wrapping_mul(16777619) ^ b as u32)
}

/// 一台设备在球体上的即时状态（环即状态，颜色必须配文字或勾选）。
#[derive(Clone, Debug, PartialEq)]
enum OrbState {
    /// 待发：无环
    Idle,
    /// 传输中（紫进度环）或已暂停（灰部分环）
    Sending {
        record: drift::RecordId,
        progress: f32,
        paused: bool,
        speed_bps: u64,
    },
    /// 已同步：绿整环 + 角标
    Synced,
    /// 内容改过：橙虚线环，点按只补发差异
    DeltaReady {
        sent_chars: usize,
        delta: ContentDelta,
    },
}

fn orb_state(
    records: &[Record],
    snapshots: &std::collections::HashMap<Fingerprint, SentSnapshot>,
    cur_text: &str,
    cur_files: &[Attachment],
    fp: &Fingerprint,
) -> OrbState {
    for r in records {
        if let Some(d) = r.deliveries.iter().find(|d| d.device.fingerprint == *fp) {
            match d.status {
                DeliveryStatus::Pending
                | DeliveryStatus::Connecting
                | DeliveryStatus::Transferring
                | DeliveryStatus::Paused => {
                    return OrbState::Sending {
                        record: r.id,
                        progress: d.progress() as f32,
                        paused: d.status == DeliveryStatus::Paused,
                        speed_bps: d.speed_bps,
                    };
                }
                DeliveryStatus::Failed
                | DeliveryStatus::WaitingRetry
                | DeliveryStatus::Canceled
                | DeliveryStatus::Delivered => {}
            }
        }
    }

    if let Some(s) = snapshots.get(fp) {
        if let Some(delta) = s.diff(cur_text, cur_files) {
            return OrbState::DeltaReady {
                sent_chars: s.text.trim().chars().count(),
                delta,
            };
        }
        return OrbState::Synced;
    }

    OrbState::Idle
}

/// 球体画布覆盖层：输入区有内容时盖在传输会话之上（会话是背景层，透出）。
pub fn orb_canvas_overlay(state: &Rc<AppState>, mobile: bool) -> AnyElement {
    let state_cond = state.clone();
    let state_body = state.clone();
    when(
        move || {
            !state_cond.draft.get().trim().is_empty()
                || !state_cond.attachments.get().is_empty()
        },
        move || orb_canvas(&state_body, mobile),
    )
    .into_element()
}

/// 画布主体：手势层（平移 / 缩放）＋ 声呐环 ＋ 世界（球体）＋ 提示 ＋ 小地图。
fn orb_canvas(state: &Rc<AppState>, mobile: bool) -> View {
    // 首屏球体集合：这些用「随输入浮出」进场；之后新配对的用「新球展开」。
    state.ensure_orb_order();
    let initial: HashSet<Fingerprint> = state.canvas.order.get_untracked().into_iter().collect();

    let pan_x = state.canvas.pan_x;
    let pan_y = state.canvas.pan_y;
    let zoom = state.canvas.zoom;

    let canvas = view()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        .z_index(25)
        .overflow_hidden()
        // 拖空白处平移：超过 4px 才算拖拽（否则仍是点击发送）
        .claim_touches(TouchClaim::on_pan(PanActivation {
            active_offset_x: Some(px(4.0)),
            active_offset_y: Some(px(4.0)),
            ..PanActivation::default()
        }))
        .pan_values(Some(&pan_x), Some(&pan_y))
        .pan_enabled(true)
        // 双指 / Ctrl+滚轮缩放（平台原生驱动）
        .pinch_value(&zoom)
        .on_pan_end({
            let state = state.clone();
            move |e: &PanEvent| {
                // 松手减速停下（减少动态时直接停在松手位置）
                if !e.cancelled && !reduce_motion() {
                    if e.velocity_x.abs() > 40.0 {
                        pan_x.decay(Some(e.velocity_x), 0.995);
                    }
                    if e.velocity_y.abs() > 40.0 {
                        pan_y.decay(Some(e.velocity_y), 0.995);
                    }
                }
                state.sync_canvas_view();
            }
        })
        .on_pinch_end({
            let state = state.clone();
            move |_e: &GestureEvent| {
                let state = state.clone();
                spawn(async move {
                    let z = zoom.value().await;
                    if !(0.6..=1.6).contains(&z) {
                        zoom.timing(z.clamp(0.6, 1.6), mo::D_CONTROL, mo::ease(mo::EASE_STD));
                    }
                    state.sync_canvas_view();
                });
            }
        })
        .child(sonar_layer(mobile))
        .child(hint(mobile))
        .child(world_layer(state, mobile, &initial))
        .child(minimap(state, mobile));

    canvas
}

/// 声呐环：同一 Wi-Fi 的可达范围，圆心落在输入区下方（固定装饰，不随画布平移）。
fn sonar_layer(mobile: bool) -> View {
    let radii: [(f32, f32); 3] = if mobile {
        [(250.0, 0.45), (360.0, 0.28), (470.0, 0.16)]
    } else {
        [(296.0, 0.50), (424.0, 0.32), (552.0, 0.18)]
    };
    view()
        .absolute()
        .left_0()
        .right_0()
        .bottom_0()
        .children(radii.iter().map(|(r, o)| {
            view()
                .w_full()
                .h(px(0.0))
                .items_center()
                .justify_center()
                .child(
                    view()
                        .w(px(r * 2.0))
                        .h(px(r * 2.0))
                        .rounded_full()
                        .border_1()
                        .border_color(p::ACCENT_LINE)
                        .opacity(*o),
                )
        }))
}

/// 顶部提示（mono faint，居中）。
fn hint(mobile: bool) -> View {
    view()
        .absolute()
        .top(px(if mobile { 14.0 } else { 24.0 }))
        .left_0()
        .right_0()
        .items_center()
        .child(
            text(if mobile {
                "点按球体即发送 · 拖动空白平移画布"
            } else {
                "点按球体即发送 · 可以连续点几个"
            })
            .text_size(px(if mobile { 9.5 } else { 10.5 }))
            .text_color(p::FAINT),
        )
}

/// 世界层：绑定原生手势值（平移 + 缩放），固定尺寸的世界盒居中摆球。
/// 动画值挂在 AppState 上，画布收起再浮出视角不重置。
fn world_layer(state: &Rc<AppState>, mobile: bool, initial: &HashSet<Fingerprint>) -> View {
    let state_rows = state.clone();
    let initial = initial.clone();
    let (hw, hh) = world_half(mobile);
    view()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        .items_center()
        .justify_center()
        .bind_translate_x(&state.canvas.pan_x)
        .bind_translate_y(&state.canvas.pan_y)
        .bind_scale(&state.canvas.zoom)
        .child(
            view()
                .relative()
                .w(px(hw * 2.0))
                .h(px(hh * 2.0))
                .child(plus_orb(state, mobile))
                .child(each(
                    {
                        let paired = state.paired.clone();
                        // 落位分配在事件泵里做（Pump 收到设备/配对事件即 ensure_orb_order），
                        // 这里只按在线设备出球。
                        move || {
                            paired
                                .get()
                                .into_iter()
                                .filter(|d| d.online)
                                .map(NeverEq)
                                .collect::<Vec<_>>()
                        }
                    },
                    |d: &NeverEq<PairedDeviceView>| d.0.fingerprint.to_hex(),
                    move |d, _| device_orb(&state_rows, d, mobile, &initial),
                )),
        )
}

/// 单颗设备球体：定位包装（进场动效）＞ 浮动层 ＞ 按压缩放 ＞ 环 + 球面 + 文字。
fn device_orb(
    state: &Rc<AppState>,
    d: ReadSignal<NeverEq<PairedDeviceView>>,
    mobile: bool,
    initial: &HashSet<Fingerprint>,
) -> View {
    let fp = d.get_untracked().0.fingerprint;
    let index = state
        .canvas
        .order
        .get_untracked()
        .iter()
        .position(|f| *f == fp)
        .unwrap_or(0);
    let (dx, dy) = spiral_pos(index, mobile);
    let hash = fp_hash(&fp);
    let (d_min, d_max) = core_range(mobile);
    let core_d = d_min + (hash % 1000) as f32 / 1000.0 * (d_max - d_min);
    let ring_d = core_d + ring_gap(core_d, mobile) * 2.0;
    let stack_w = (ring_d * 1.2).max(if mobile { 118.0 } else { 150.0 });
    let (hw, hh) = world_half(mobile);

    let name = {
        let d = d.clone();
        move || d.get().0.name
    };
    let kind = {
        let d = d.clone();
        move || d.get().0.kind.clone()
    };
    let records = state.records.clone();
    let snapshots = state.sent_snapshots.clone();
    let draft = state.draft.clone();
    let attachments = state.attachments.clone();
    let st_now = move || {
        orb_state(
            &records.get(),
            &snapshots.get(),
            &draft.get(),
            &attachments.get(),
            &fp,
        )
    };

    // 进场：首屏随输入浮出（180ms · 逐球 40ms 错开），新配对的从加号球位置展开。
    let enter = if initial.contains(&fp) {
        mo::orb_rise(index as f32 * mo::D_STAGGER)
    } else {
        mo::orb_spawn(0.0)
    };
    let press = mo::orb_press_value();
    // 环境浮动循环：7s 缓升缓降 -6px，相位按指纹错开；「减少动态」时不动。
    let bob = (!reduce_motion()).then(|| mo::orb_bob_value((hash % 4000) as f32));

    let mut bob_layer = view();
    if let Some(b) = &bob {
        bob_layer = bob_layer.bind_translate_y(b);
    }

    view()
        .absolute()
        // 以落位为中心摆放整颗球（环 + 文字）；落位以世界盒中心为原点
        .left(px(hw + dx - stack_w / 2.0))
        .top(px(hh + dy - ring_d / 2.0))
        .w(px(stack_w))
        .items_center()
        .bind_motion(&motion().entering(enter))
        .child(
            bob_layer
                .items_center()
                .child(
                    view()
                        .bind_scale(&press)
                        .cursor_pointer()
                        .label("设备球体")
                        .role(AccessibilityRole::Button)
                        .on_press({
                            let state = state.clone();
                            move || on_orb_press(&state, fp)
                        })
                        .child(dynamic(move || {
                            let st = st_now();
                            orb_visual(&st, &kind(), core_d, mobile)
                        })),
                )
                // 球体下方：设备名 + 该台自己的状态 / 差异文案
                .child(
                    view()
                        .gap(px(2.0))
                        .items_center()
                        .mt(px(if mobile { 7.0 } else { 9.0 }))
                        .child(
                            text(name)
                                .text_size(px(if mobile { 11.5 } else { 12.5 }))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p::INK)
                                .number_of_lines(1),
                        )
                        .child(dynamic(move || orb_status_text(&st_now(), mobile))),
                ),
        )
}

/// 环 + 球面。状态只画在环上：颜色必须配文字或勾选，不能只靠颜色区分。
fn orb_visual(st: &OrbState, kind: &str, core_d: f32, mobile: bool) -> View {
    let ring_d = core_d + ring_gap(core_d, mobile) * 2.0;

    let (core_bg, core_border, shadow) = match st {
        OrbState::Sending { paused: false, .. } => (p::ACCENT_WASH, p::LINE, p::shadow_orb_glow()),
        OrbState::Sending { paused: true, .. } => (p::SUNKEN, p::LINE_SOFT, Vec::new()),
        OrbState::DeltaReady { .. } => (p::SURFACE, p::LINE, p::shadow_orb()),
        _ => (p::SURFACE, p::LINE, p::shadow_orb()),
    };

    view()
        .relative()
        .w(px(ring_d))
        .h(px(ring_d))
        .items_center()
        .justify_center()
        // 状态环
        .child(ring(st, ring_d))
        // 球面
        .child(
            view()
                .w(px(core_d))
                .h(px(core_d))
                .rounded_full()
                .bg(core_bg)
                .border(px(1.2))
                .border_color(core_border)
                .shadow(shadow)
                .items_center()
                .justify_center()
                .child(orb_face(st, kind, core_d, mobile)),
        )
        // 已同步角标（绿点白勾，压在球面右下）
        .child(match st {
            OrbState::Synced => view()
                .absolute()
                .right(px(-2.0))
                .bottom(px(-2.0))
                .child(ui::check_badge(if mobile { 14.0 } else { 18.0 }))
                .into_element(),
            _ => view().into_element(),
        })
}

/// 环：待发无环；传输紫进度环（轨道 + 弧）；暂停灰部分环；可补发橙虚线；已同步绿整环。
fn ring(st: &OrbState, ring_d: f32) -> AnyElement {
    match st {
        OrbState::Idle => view().into_element(),
        OrbState::Sending { progress, paused, .. } => {
            let pct = (*progress).clamp(0.0, 1.0);
            let (arc, clear) = if *paused {
                (p::FLAG, p::FLAG_CLEAR)
            } else {
                (p::ACCENT, p::ACCENT_CLEAR)
            };
            view()
                .absolute()
                .top_0()
                .left_0()
                .w(px(ring_d))
                .h(px(ring_d))
                .child(
                    // 轨道（传输中才有；暂停只画灰弧）
                    if *paused {
                        view().into_element()
                    } else {
                        view()
                            .w(px(ring_d))
                            .h(px(ring_d))
                            .rounded_full()
                            .border(px(2.4))
                            .border_color(p::RING_TRACK)
                            .into_element()
                    },
                )
                // 进度弧：conic 硬边渐变，dashoffset 语义，不改圆周长
                .child(
                    view()
                        .absolute()
                        .top_0()
                        .left_0()
                        .w(px(ring_d))
                        .h(px(ring_d))
                        .rounded_full()
                        .bg_conic(deg(-90.0))
                        .gradient_from(arc)
                        .gradient_from_position(percentage(0.0))
                        .gradient_via(arc)
                        .gradient_via_position(percentage(pct))
                        .gradient_to(clear)
                        .gradient_to_position(percentage((pct + 0.002).min(1.0))),
                )
                .into_element()
        }
        OrbState::Synced => view()
            .absolute()
            .top_0()
            .left_0()
            .w(px(ring_d))
            .h(px(ring_d))
            .rounded_full()
            .border(px(1.8))
            .border_color(p::SUCCESS)
            .opacity(0.7)
            .into_element(),
        OrbState::DeltaReady { .. } => view()
            .absolute()
            .top_0()
            .left_0()
            .w(px(ring_d))
            .h(px(ring_d))
            .rounded_full()
            .border(px(1.8))
            .border_dashed()
            .border_color(p::DELTA_LINE)
            .into_element(),
    }
}

/// 球面内容：设备图标；传输中图标下方再标百分比（数字直更，不做滚动）。
fn orb_face(st: &OrbState, kind: &str, core_d: f32, mobile: bool) -> AnyElement {
    let icon = ic::icon(ic::device(kind), core_d * 0.26, face_color(st));
    match st {
        OrbState::Sending { progress, paused: false, .. } => {
            let pct = (*progress * 100.0).clamp(0.0, 100.0) as u32;
            view()
                .items_center()
                .child(icon)
                .child(
                    text(format!("{pct}%"))
                        .text_size(px(if mobile { 9.5 } else { 11.5 }))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p::ACCENT_INK)
                        .mt(px(1.0)),
                )
                .into_element()
        }
        _ => icon.into_element(),
    }
}

fn face_color(st: &OrbState) -> nsh_core::prelude::Rgba {
    match st {
        OrbState::Sending { paused: false, .. } => p::ACCENT,
        OrbState::Sending { paused: true, .. } => p::FLAG,
        OrbState::DeltaReady { .. } => p::DELTA,
        _ => p::MUTED,
    }
}

/// 球体下方的状态 / 差异文案（增量差异只挂在这一台球下，不搬进输入框）。
fn orb_status_text(st: &OrbState, mobile: bool) -> AnyElement {
    let (label, ink) = match st {
        OrbState::Idle => ("点按发送".to_string(), false),
        OrbState::Sending { progress, paused, speed_bps, .. } => {
            let pct = (*progress * 100.0).clamp(0.0, 100.0) as u32;
            if *paused {
                (format!("已暂停 {pct}% · 点按继续"), false)
            } else {
                (format!("{}/s · 点按暂停", drift::util::human_bytes(*speed_bps)), false)
            }
        }
        OrbState::Synced => {
            if mobile {
                ("已同步".to_string(), false)
            } else {
                ("已同步 · 刚刚送达".to_string(), false)
            }
        }
        OrbState::DeltaReady { sent_chars, delta } => (delta_label(*sent_chars, delta, mobile), true),
    };
    text(label)
        .text_size(px(if mobile { 9.0 } else { 10.0 }))
        .font_weight(if ink { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
        .text_color(if ink { p::DELTA_INK } else { p::FAINT })
        .number_of_lines(1)
        .into_element()
}

fn delta_label(sent_chars: usize, delta: &ContentDelta, mobile: bool) -> String {
    match delta {
        ContentDelta::AppendedText { added_chars, .. } => {
            if mobile {
                format!("补发 +{added_chars} 字")
            } else {
                format!("曾发 {sent_chars} 字 · 补发 +{added_chars} 字")
            }
        }
        ContentDelta::ModifiedText { .. } => {
            if mobile {
                "内容已改 · 点按更新".to_string()
            } else {
                format!("曾发 {sent_chars} 字 · 内容已改，点按更新")
            }
        }
        ContentDelta::AddedFiles { added_files } => {
            if mobile {
                format!("补发 +{} 文件", added_files.len())
            } else {
                format!("新增 {} 个文件 · 点按仅补发新文件", added_files.len())
            }
        }
        ContentDelta::ModifiedPayload => {
            if mobile {
                "内容已改 · 点按补发".to_string()
            } else {
                format!("曾发 {sent_chars} 字 · 内容已改，点按补发")
            }
        }
    }
}

/// 点按球体：待发 / 可补发 → 立即发送（增量只补差异）；传输中 → 暂停；
/// 已暂停 → 继续；已同步 → 提示改内容后可增量。
fn on_orb_press(state: &Rc<AppState>, fp: Fingerprint) {
    let st = orb_state(
        &state.records.get_untracked(),
        &state.sent_snapshots.get_untracked(),
        &state.draft.get_untracked(),
        &state.attachments.get_untracked(),
        &fp,
    );
    match st {
        OrbState::Sending { record, paused, .. } => {
            if paused {
                state.resume(record, fp);
            } else {
                state.pause(record, fp);
            }
        }
        OrbState::Idle | OrbState::DeltaReady { .. } => {
            state.send_to(&[fp]);
        }
        OrbState::Synced => {
            state.toast("该设备已同步最新内容，修改输入后可增量补发", crate::state::ToastKind::Info);
        }
    }
}

/// 「+ 添加设备」虚线球：点开就地配对，配对成功后新球落在附近。
fn plus_orb(state: &Rc<AppState>, mobile: bool) -> View {
    // 落位在中心右下方（板：G 球位于球群右下）
    let (off_x, off_y) = if mobile { (95.0, 180.0) } else { (210.0, 185.0) };
    let (hw, hh) = world_half(mobile);
    let ring_d = if mobile { 60.0 } else { 80.0 };
    let core_d = if mobile { 48.0 } else { 64.0 };
    let press = mo::orb_press_value();
    let enter = mo::orb_rise(if mobile { 5.0 } else { 6.0 } * mo::D_STAGGER);
    view()
        .absolute()
        .left(px(hw + off_x - ring_d / 2.0))
        .top(px(hh + off_y - ring_d / 2.0))
        .w(px(ring_d))
        .items_center()
        .bind_motion(&motion().entering(enter))
        .child(
            view()
                .bind_scale(&press)
                .cursor_pointer()
                .label("添加设备")
                .role(AccessibilityRole::Button)
                .on_press({
                    let state = state.clone();
                    move || state.pairing_open.set(true)
                })
                .child(
                    view()
                        .relative()
                        .w(px(ring_d))
                        .h(px(ring_d))
                        .items_center()
                        .justify_center()
                        // 虚线外环（唯一识别描边 ≥3:1）
                        .child(
                            view()
                                .absolute()
                                .top_0()
                                .left_0()
                                .w(px(ring_d))
                                .h(px(ring_d))
                                .rounded_full()
                                .border(px(1.4))
                                .border_dashed()
                                .border_color(p::DROPZONE),
                        )
                        .child(
                            view()
                                .w(px(core_d))
                                .h(px(core_d))
                                .rounded_full()
                                .bg(p::SURFACE)
                                .border(px(1.2))
                                .border_color(p::LINE_SOFT)
                                .shadow(p::shadow_orb())
                                .items_center()
                                .justify_center()
                                .child(ic::icon(ic::PLUS, core_d * 0.38, p::ACCENT_ICON)),
                        ),
                ),
        )
        .child(
            view()
                .gap(px(2.0))
                .items_center()
                .mt(px(8.0))
                .child(
                    text("添加设备")
                        .text_size(px(if mobile { 11.5 } else { 12.5 }))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p::INK),
                )
                .child(
                    text(if mobile { "配对后落在这" } else { "新设备配对后落在这里" })
                        .text_size(px(if mobile { 9.0 } else { 10.0 }))
                        .text_color(p::FAINT)
                        .number_of_lines(1),
                ),
        )
}

/// 小地图：当前视口 + 窗外设备位置（点色随状态）；移动端附「可见 / 总数」。
fn minimap(state: &Rc<AppState>, mobile: bool) -> View {
    let (mm_w, mm_h) = if mobile { (56.0, 38.0) } else { (96.0, 58.0) };
    let order = state.canvas.order.clone();
    let view_sig = state.canvas.view.clone();
    let records = state.records.clone();
    let snapshots = state.sent_snapshots.clone();
    let draft = state.draft.clone();
    let attachments = state.attachments.clone();
    let paired = state.paired.clone();

    view()
        .absolute()
        .top(px(if mobile { 12.0 } else { 16.0 }))
        .right(px(if mobile { 12.0 } else { 16.0 }))
        .gap(px(4.0))
        .child(dynamic(move || {
            let (vx, vy, vz) = view_sig.get();
            let fps = order.get();
            let (hw, hh) = world_half(mobile);
            let (vw, vh) = nominal_viewport(mobile);
            // 快照一次性取出，避免逐球克隆
            let (paired_now, records_now, snaps_now, draft_now, atts_now) = (
                paired.get(),
                records.get(),
                snapshots.get(),
                draft.get(),
                attachments.get(),
            );

            let mut visible = 0usize;
            let dots: Vec<(f32, f32, nsh_core::prelude::Rgba)> = fps
                .iter()
                .enumerate()
                .filter_map(|(i, fp)| {
                    let dev = paired_now.iter().find(|d| d.fingerprint == *fp)?;
                    if !dev.online {
                        return None;
                    }
                    let (x, y) = spiral_pos(i, mobile);
                    // 视口内判定（标称尺寸）
                    let sx = vx + vz * x;
                    let sy = vy + vz * y;
                    if sx > 0.0 && sx < vw && sy > 0.0 && sy < vh {
                        visible += 1;
                    }
                    let st = orb_state(&records_now, &snaps_now, &draft_now, &atts_now, fp);
                    let color = match st {
                        OrbState::Sending { paused: false, .. } => p::ACCENT,
                        OrbState::Sending { paused: true, .. } => p::FLAG,
                        OrbState::DeltaReady { .. } => p::DELTA_FILL,
                        OrbState::Synced => p::SUCCESS,
                        OrbState::Idle => p::FAINT,
                    };
                    let nx = ((x + hw) / (hw * 2.0)).clamp(0.04, 0.96);
                    let ny = ((y + hh) / (hh * 2.0)).clamp(0.08, 0.92);
                    Some((nx, ny, color))
                })
                .collect();

            view()
                .gap(px(4.0))
                .child(
                    view()
                        .relative()
                        .w(px(mm_w))
                        .h(px(mm_h))
                        .rounded(px(if mobile { 8.0 } else { 10.0 }))
                        .bg(p::SUNKEN)
                        .border_1()
                        .border_color(p::LINE)
                        // 设备点
                        .children(dots.iter().map(|(nx, ny, c)| {
                            view()
                                .absolute()
                                .left(px(nx * mm_w - 2.2))
                                .top(px(ny * mm_h - 2.2))
                                .w(px(4.4))
                                .h(px(4.4))
                                .rounded_full()
                                .bg(*c)
                        }))
                        // 当前视口框（标称换算，随 pan / zoom 同步）
                        .child(viewport_rect(vx, vy, vz, mm_w, mm_h, hw, hh, vw, vh)),
                )
                .child(if mobile {
                    // 「可见 / 总数」计数（板：2/6；分母 = 在线球数）
                    text(format!("{visible}/{}", dots.len()))
                        .text_size(px(8.5))
                        .text_color(p::FAINT)
                        .into_element()
                } else {
                    text("拖动空白处平移 · 滚轮缩放")
                        .text_size(px(10.0))
                        .text_color(p::FAINT)
                        .into_element()
                })
        }))
}

/// 小地图视口框（accent 描边）。
#[allow(clippy::too_many_arguments)]
fn viewport_rect(
    vx: f32,
    vy: f32,
    vz: f32,
    mm_w: f32,
    mm_h: f32,
    hw: f32,
    hh: f32,
    vw: f32,
    vh: f32,
) -> nsh_core::element::View {
    // 视口在世界坐标中的中心与半宽（世界层变换 = translate(pan) ∘ scale(zoom)）
    let cx = (vw / 2.0 - vx) / vz;
    let cy = (vh / 2.0 - vy) / vz;
    let rw = vw / vz;
    let rh = vh / vz;
    // 世界 → 小地图
    let to_x = |x: f32| ((x + hw) / (hw * 2.0)).clamp(0.0, 1.0) * mm_w;
    let to_y = |y: f32| ((y + hh) / (hh * 2.0)).clamp(0.0, 1.0) * mm_h;
    let left = (to_x(cx - rw / 2.0)).clamp(0.0, (mm_w - 6.0).max(0.0));
    let top = (to_y(cy - rh / 2.0)).clamp(0.0, (mm_h - 5.0).max(0.0));
    let w = (to_x(cx + rw / 2.0) - left).clamp(6.0, (mm_w - left).max(6.0));
    let h = (to_y(cy + rh / 2.0) - top).clamp(5.0, (mm_h - top).max(5.0));
    view()
        .absolute()
        .left(px(left))
        .top(px(top))
        .w(px(w))
        .h(px(h))
        .rounded(px(3.0))
        .border(px(1.2))
        .border_color(p::ACCENT)
}
