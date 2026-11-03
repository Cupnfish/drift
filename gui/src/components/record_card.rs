//! 记录卡三种形态（drift-main-*）：
//! 接收卡（文件/文本）、发送气泡（已完成）、发送中卡片（逐设备行控制）。

use std::rc::Rc;

use drift::{Delivery, DeliveryStatus, Direction, Payload, Record, RecordId};
use nsh_core::prelude::*;

use crate::{icons as ic, motion as mo, palette as p, state::AppState, ui};

/// 记录是否处于「发送中」（有非终态投递）。
fn is_sending(r: &Record) -> bool {
    r.deliveries
        .iter()
        .any(|d| !d.status.is_terminal())
}

/// 一条记录的完整行（含进入动画）。
pub fn record_row(state: &Rc<AppState>, row: ReadSignal<crate::state::NeverEq<TRow>>, mobile: bool) -> AnyElement {
    match row.get_untracked().0 {
        TRow::Day(label) => day_separator(label).into_element(),
        TRow::Rec(record) => record_card(state, record.id, mobile, row).into_element(),
    }
}

/// 时间线行（见 timeline.rs）。
#[derive(Clone, Debug)]
pub enum TRow {
    Day(String),
    Rec(Record),
}

/// 日期分隔："—— 今天 ——"。
pub fn day_separator(label: String) -> View {
    view()
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .py(px(14.0))
        .child(view().flex_1().h(px(1.0)).bg(p::LINE))
        .child(
            view()
                .h(px(24.0))
                .px(px(14.0))
                .items_center()
                .rounded(px(12.0))
                .bg(p::DATE_PILL)
                .child(
                    text(label)
                        .text_size(px(11.0))
                        .text_color(p::MUTED),
                ),
        )
        .child(view().flex_1().h(px(1.0)).bg(p::LINE))
}

fn record_card(
    state: &Rc<AppState>,
    id: RecordId,
    mobile: bool,
    row: ReadSignal<crate::state::NeverEq<TRow>>,
) -> View {
    let state = state.clone();
    let record = {
        let row = row.clone();
        move || {
            let TRow::Rec(r) = row.get().0 else { return None::<Record> };
            Some(r)
        }
    };
    let flash = {
        let state = state.clone();
        move || state.flash_done.get() == Some(id)
    };
    // 完成高亮 320ms 后自动熄灭
    {
        let state = state.clone();
        set_timeout(std::time::Duration::from_millis(600), move || {
            if state.flash_done.get_untracked() == Some(id) {
                state.flash_done.set(None);
            }
        });
    }
    let is_received = record().is_some_and(|r| r.direction == Direction::Received);
    let motion = motion().entering(mo::enter_record());
    let state_dyn = state.clone();

    view()
        .bind_motion(&motion)
        .w_full()
        .style_when(flash.clone(), |s| {
            s.rounded(px(18.0)).bg(p::ACCENT_WASH)
        })
        .child(dynamic(move || {
            let Some(r) = record() else {
                return view().into_element();
            };
            let state = state_dyn.clone();
            if is_received {
                received_card(&state, &r, mobile).into_element()
            } else if is_sending(&r) {
                sending_card(&state, &r, mobile).into_element()
            } else {
                sent_card(&state, &r, mobile, flash()).into_element()
            }
        }))
}

// —— 接收卡 ——

fn received_card(state: &Rc<AppState>, r: &Record, mobile: bool) -> View {
    let (title, files, is_text, body) = payload_parts(r);
    let meta = format!(
        "来自 {} · {} · {}",
        r.peer.as_ref().map(|p| p.name.as_str()).unwrap_or("未知设备"),
        crate::state::time_hm(r.timestamp),
        drift::util::human_bytes(r.total_size()),
    );
    let id = r.id;
    view()
        .self_start()
        .max_w(px(if mobile { 350.0 } else { 430.0 }))
        .w_full()
        .rounded(px(18.0))
        .bg(p::SURFACE)
        .border_1()
        .border_color(p::LINE)
        .shadow(if mobile { p::shadow_card_mobile() } else { p::shadow_card() })
        .p(px(14.0))
        .gap(px(10.0))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .child(thumbnail(&files, mobile))
                .child(
                    view()
                        .flex_1()
                        .gap(px(3.0))
                        .child(
                            text(title)
                                .text_size(px(if mobile { 13.0 } else { 14.5 }))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p::INK)
                                .number_of_lines(1),
                        )
                        .child(
                            text(meta)
                                .text_size(px(if mobile { 11.0 } else { 11.5 }))
                                .text_color(p::MUTED)
                                .number_of_lines(1),
                        ),
                )
                .child(more_button(state, r)),
        )
        .child(view().h(px(1.0)).bg(p::LINE_SOFT))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(16.0))
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            view()
                                .w(px(16.0))
                                .h(px(16.0))
                                .rounded_full()
                                .bg(p::CHECK_CIRCLE)
                                .items_center()
                                .justify_center()
                                .child(ic::icon(ic::CHECK, 9.0, p::SUCCESS_STRONG)),
                        )
                        .child(
                            text("已接收")
                                .text_size(px(11.0))
                                .text_color(p::RECEIVED_TEXT),
                        ),
                )
                .child(if is_text {
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(5.0))
                        .cursor_pointer()
                        .hover(|s| s.opacity(0.75))
                        .on_press({
                            let state = state.clone();
                            let body = body.clone();
                            move || state.copy_text(&body)
                        })
                        .child(ic::icon(ic::COPY, 14.0, p::MUTED))
                        .child(text_action_label("复制文本"))
                        .into_element()
                } else {
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(5.0))
                        .cursor_pointer()
                        .hover(|s| s.opacity(0.75))
                        .on_press({
                            let state = state.clone();
                            let id = r.id;
                            move || {
                                if let Some(path) = state.handle.file_path(&id, 0) {
                                    crate::state::open_in_system(path);
                                }
                            }
                        })
                        .child(ic::icon(ic::FOLDER, 14.0, p::MUTED))
                        .child(text_action_label("打开"))
                        .into_element()
                })
                .child(if !is_text && !mobile {
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(5.0))
                        .cursor_pointer()
                        .hover(|s| s.opacity(0.75))
                        .on_press({
                            let state = state.clone();
                            let id = r.id;
                            move || {
                                if let Some(path) = state.handle.file_path(&id, 0) {
                                    if let Some(parent) = path.parent().map(|p| p.to_path_buf()) {
                                        crate::state::open_in_system(parent);
                                    }
                                }
                            }
                        })
                        .child(ic::icon(ic::DOWNLOAD, 14.0, p::MUTED))
                        .child(text_action_label("显示位置"))
                        .into_element()
                } else {
                    view().into_element()
                })
                .child(
                    // 悬停删除（桌面）
                    if mobile {
                        view().into_element()
                    } else {
                        view()
                            .flex_row()
                            .items_center()
                            .gap(px(5.0))
                            .cursor_pointer()
                            .hover(|s| s.opacity(0.75))
                            .on_press({
                                let state = state.clone();
                                move || state.delete_record(id)
                            })
                            .child(ic::icon(ic::TRASH, 14.0, p::DANGER_ACTION))
                            .child(text_action_label("删除"))
                            .into_element()
                    },
                ),
        )
}

fn text_action_label(label: &'static str) -> Text {
    text(label).text_size(px(11.0)).font_weight(FontWeight::MEDIUM).text_color(p::MUTED)
}

// —— 发送气泡（已完成）——

fn sent_card(state: &Rc<AppState>, r: &Record, mobile: bool, flash: bool) -> View {
    let (title, files, is_text, body) = payload_parts(r);
    let time = crate::state::time_hm(r.timestamp);
    let id = r.id;
    let bubble_grad = if mobile { p::GRAD_BUBBLE_MOBILE } else { p::GRAD_BUBBLE_DESKTOP };
    view()
        .self_end()
        .max_w(px(if mobile { 308.0 } else { 456.0 }))
        .w_full()
        .items_end()
        .gap(px(8.0))
        .child(
            view()
                .w_full()
                .rounded(px(if mobile { 19.0 } else { 18.0 }))
                .bg_linear(deg(120.0))
                .gradient_from(bubble_grad.0)
                .gradient_to(bubble_grad.1)
                .shadow(if mobile { p::shadow_card_mobile() } else { p::shadow_card() })
                .p(px(if is_text { 16.0 } else { 14.0 }))
                .gap(px(8.0))
                .child(if is_text {
                    let body_display = body.clone();
                    text(body_display)
                        .text_size(px(if mobile { 13.0 } else { 14.5 }))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p::WHITE)
                        .line_height(px(if mobile { 20.0 } else { 22.0 }))
                        .into_element()
                } else {
                    view()
                        .flex_row()
                        .items_center()
                        .gap(px(12.0))
                        .child(thumbnail(&files, mobile))
                        .child(
                            view()
                                .flex_1()
                                .gap(px(3.0))
                                .child(
                                    text(title)
                                        .text_size(px(if mobile { 13.0 } else { 14.5 }))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(p::INK)
                                        .number_of_lines(1),
                                )
                                .child(
                                    text(format!(
                                        "{} · {}",
                                        drift::util::human_bytes(r.total_size()),
                                        summary_of_deliveries(r)
                                    ))
                                    .text_size(px(if mobile { 11.0 } else { 11.5 }))
                                    .text_color(if files.iter().all(|_| true) && !files.is_empty() {
                                        p::MUTED
                                    } else {
                                        p::MUTED
                                    })
                                    .number_of_lines(1),
                                ),
                        )
                        .into_element()
                }),
        )
        // 接收方胶囊行
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap(px(8.0))
                .justify_end()
                .children(delivery_capsules(r, mobile)),
        )
        // 悬停操作条（桌面）
        .child(if mobile {
            view().into_element()
        } else {
            let body_for_op = body.clone();
            op_bar(state, id, is_text, body_for_op, files.iter().find(|f| f.local_path.is_some()).and_then(|f| f.local_path.clone()))
                .into_element()
        })
        .child(
            text(time.clone())
                .text_size(px(10.5))
                .text_color(p::FAINT),
        )
        .style_when(move || flash, |s| s.opacity(1.0))
}

/// 悬停操作条：复制 / 导出 / 删除。
#[allow(clippy::too_many_arguments)]
fn op_bar(
    state: &Rc<AppState>,
    id: RecordId,
    is_text: bool,
    body: String,
    first_path: Option<std::path::PathBuf>,
) -> View {
    let _ = is_text;
    view()
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .opacity(0.0)
        .hover(|s| s.opacity(1.0))
        .child(
            view()
                .cursor_pointer()
                .hover(|s| s.opacity(0.7))
                .on_press({
                    let state = state.clone();
                    let body = body.clone();
                    move || state.copy_text(&body)
                })
                .child(ic::icon(ic::COPY, 15.0, p::MUTED)),
        )
        .child(
            view()
                .cursor_pointer()
                .hover(|s| s.opacity(0.7))
                .on_press({
                    let state = state.clone();
                    move || state.export_text(id, false)
                })
                .child(ic::icon(ic::DOWNLOAD, 15.0, p::MUTED)),
        )
        .child(
            view()
                .cursor_pointer()
                .hover(|s| s.opacity(0.7))
                .on_press(move || {
                    if let Some(path) = first_path.clone() {
                        crate::state::open_in_system(path);
                    }
                })
                .child(ic::icon(ic::FOLDER, 15.0, p::MUTED)),
        )
        .child(
            view()
                .cursor_pointer()
                .hover(|s| s.opacity(0.7))
                .on_press({
                    let state = state.clone();
                    move || state.delete_record(id)
                })
                .child(ic::icon(ic::TRASH, 15.0, p::DANGER_ACTION)),
        )
}

/// 接收方胶囊（26 高 rx13 白底）：图标 + 状态徽章 + 名字 [+ 时间]。
fn delivery_capsules(r: &Record, mobile: bool) -> Vec<View> {
    let h = if mobile { 24.0 } else { 26.0 };
    let mut sorted: Vec<&Delivery> = r.deliveries.iter().collect();
    sorted.sort_by_key(|d| match d.status {
        DeliveryStatus::Failed | DeliveryStatus::WaitingRetry => 0,
        DeliveryStatus::Connecting | DeliveryStatus::Transferring | DeliveryStatus::Paused => 1,
        _ => 2,
    });
    sorted
        .into_iter()
        .take(3)
        .map(|d| {
            let delivered = matches!(d.status, DeliveryStatus::Delivered);
            let failed = matches!(d.status, DeliveryStatus::Failed | DeliveryStatus::WaitingRetry);
            view()
                .h(px(h))
                .pr(px(10.0))
                .pl(px(5.0))
                .flex_row()
                .items_center()
                .gap(px(5.0))
                .rounded(px(h / 2.0))
                .bg(p::SURFACE)
                .border_1()
                .border_color(p::LINE_STRONG)
                .child(ic::icon(
                    ic::device(&d.device.kind.as_str()),
                    if mobile { 12.0 } else { 14.0 },
                    if delivered { p::SUCCESS } else if failed { p::DANGER } else { p::MUTED },
                ))
                .child(if delivered {
                    ui::check_badge(if mobile { 12.0 } else { 13.0 }).into_element()
                } else if failed {
                    text("失败")
                        .text_size(px(9.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p::DANGER)
                        .into_element()
                } else {
                    view().into_element()
                })
                .child(
                    text(d.device.name.clone())
                        .text_size(px(if mobile { 9.5 } else { 10.5 }))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p::INK)
                        .number_of_lines(1),
                )
        })
        .collect()
}

fn summary_of_deliveries(r: &Record) -> String {
    let done = r.delivered_count();
    if r.deliveries.len() > 1 {
        format!("已送达 {done} 台")
    } else if done == 1 {
        "已送达".to_string()
    } else {
        r.deliveries
            .first()
            .map(|d| d.status.label().to_string())
            .unwrap_or_default()
    }
}

// —— 发送中卡片 ——

fn sending_card(state: &Rc<AppState>, r: &Record, mobile: bool) -> View {
    let (title, _, _, _) = payload_parts(r);
    let n_files = match &r.payload {
        Payload::Files { files } => files.len(),
        Payload::Text { .. } => 0,
    };
    let header_title = if n_files > 0 {
        format!("正在发送 {n_files} 个文件")
    } else {
        "正在发送文本".to_string()
    };
    // 头部右侧：首个文件名 · 总速度
    let speed: u64 = r.deliveries.iter().map(|d| d.speed_bps).sum();
    let first_name = match &r.payload {
        Payload::Files { files } => files.first().map(|f| f.name.clone()),
        Payload::Text { .. } => Some("文本".to_string()),
    };
    let head_right = format!(
        "{} · {}/s",
        first_name.as_deref().unwrap_or(""),
        drift::util::human_bytes(speed),
    );
    view()
        .self_end()
        .w(px(if mobile { 350.0 } else { 440.0 }))
        .rounded(px(18.0))
        .bg(p::SURFACE)
        .border_1()
        .border_color(rgb(0xE5E6F6))
        .shadow(p::shadow_card())
        .p(px(14.0))
        .gap(px(10.0))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(7.0))
                .child(ic::icon(ic::FILE, 14.0, p::ACCENT_INK))
                .child(
                    text(header_title)
                        .text_size(px(if mobile { 12.0 } else { 12.5 }))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p::INK),
                )
                .child(view().flex_1())
                .child(
                    text(head_right)
                        .text_size(px(if mobile { 9.0 } else { 10.5 }))
                        .text_color(p::MUTED)
                        .number_of_lines(1),
                ),
        )
        .child(view().h(px(1.0)).bg(p::LINE_SOFT))
        .children(r.deliveries.iter().map(|d| delivery_row(state, r.id, d, mobile)))
        .child(
            text(title)
                .text_size(px(9.5))
                .text_color(p::FAINT)
                .number_of_lines(1),
        )
}

/// 单台接收方行：名称 + 进度 + 百分比 + 暂停/继续 + 取消 / 失败态 + 重试。
fn delivery_row(state: &Rc<AppState>, record: RecordId, d: &Delivery, mobile: bool) -> View {
    let fp = d.device.fingerprint;
    let pct = (d.progress() * 100.0).round() as u32;
    let track_w = if mobile { 156.0 } else { 190.0 };
    match d.status {
        DeliveryStatus::Failed | DeliveryStatus::WaitingRetry => {
            let label = if matches!(d.status, DeliveryStatus::WaitingRetry) {
                "等待重试".to_string()
            } else {
                "失败".to_string()
            };
            view()
                .w_full()
                .h(px(44.0))
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .px(px(10.0))
                .rounded(px(12.0))
                .bg(p::FAIL_ROW)
                .border_1()
                .border_color(p::DANGER_LINE)
                .child(ic::icon(ic::WARN, 13.0, p::DANGER))
                .child(
                    view()
                        .flex_1()
                        .gap(px(1.0))
                        .child(
                            text(format!("{} · {label}", d.device.name))
                                .text_size(px(10.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p::DANGER_INK)
                                .number_of_lines(1),
                        )
                        .child(
                            text(d.error.clone().unwrap_or_default())
                                .text_size(px(9.0))
                                .text_color(p::DANGER)
                                .number_of_lines(1),
                        ),
                )
                .child(
                    view()
                        .h(px(22.0))
                        .px(px(10.0))
                        .items_center()
                        .rounded(px(8.0))
                        .bg(p::SURFACE)
                        .border_1()
                        .border_color(p::RETRY_BORDER)
                        .cursor_pointer()
                        .hover(|s| s.opacity(0.8))
                        .on_press({
                            let state = state.clone();
                            move || state.retry(record, fp)
                        })
                        .child(
                            text("重试")
                                .text_size(px(10.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p::DANGER_FILL),
                        ),
                )
        }
        _ => {
            let paused = matches!(d.status, DeliveryStatus::Paused);
            view()
                .w_full()
                .h(px(36.0))
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .child(
                    text(d.device.name.clone())
                        .text_size(px(11.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p::BODY)
                        .number_of_lines(1),
                )
                .child(if paused {
                    text("已暂停")
                        .text_size(px(10.0))
                        .text_color(p::MUTED)
                        .into_element()
                } else {
                    view().into_element()
                })
                .child(view().flex_1())
                // 进度条
                .child(
                    view()
                        .w(px(track_w))
                        .h(px(4.0))
                        .rounded(px(2.0))
                        .bg(p::TRACK)
                        .overflow_hidden()
                        .child(
                            view()
                                .w(percentage(d.progress() as f32))
                                .h_full()
                                .rounded(px(2.0))
                                .bg(if paused { p::PAUSED_FILL } else { p::ACCENT }),
                        ),
                )
                .child(
                    text(format!(
                        "{pct}%{}",
                        if paused { "" } else { "" }
                    ))
                    .text_size(px(if mobile { 9.0 } else { 10.5 }))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(if paused { p::FAINT } else { p::ACCENT_MUTE })
                    .w(px(if mobile { 30.0 } else { 36.0 })),
                )
                // 暂停 / 继续
                .child(ui::control_button(
                    if paused { p::RESUME_BTN } else { p::PAUSE_BTN },
                    if paused { p::SUCCESS_LINE } else { p::BORDER_SOFT },
                    if paused { "继续传输" } else { "暂停传输" },
                    {
                        let state = state.clone();
                        move || {
                            if paused {
                                state.resume(record, fp);
                            } else {
                                state.pause(record, fp);
                            }
                        }
                    },
                    if paused { ui::play_icon() } else { ui::pause_icon() },
                ))
                // 取消
                .child(ui::control_button(
                    p::DANGER_WASH,
                    p::DANGER_LINE,
                    "取消传输",
                    {
                        let state = state.clone();
                        move || state.cancel_target(record, fp)
                    },
                    ui::close_glyph(11.0, p::DANGER_ICON),
                ))
        }
    }
}

// —— 公共小件 ——

/// 文件缩略（首文件；图片用真图）。
fn thumbnail(files: &[drift::FileEntry], mobile: bool) -> View {
    let size = if mobile { 42.0 } else { 44.0 };
    let first = files.first();
    let inner = match first {
        Some(f) if f.is_image() && f.local_path.is_some() => {
            let path = f.local_path.clone().unwrap();
            image(path.to_string_lossy().into_owned())
                .w(px(size))
                .h(px(size))
                .rounded(px(if mobile { 13.0 } else { 14.0 }))
                .into_element()
        }
        _ => ic::icon(ic::FILE, if mobile { 20.0 } else { 22.0 }, p::FILE_INK).into_element(),
    };
    view()
        .w(px(size))
        .h(px(size))
        .rounded(px(if mobile { 13.0 } else { 14.0 }))
        .bg(p::THUMB_BG)
        .overflow_hidden()
        .items_center()
        .justify_center()
        .child(inner)
}

/// 更多按钮（28×28 rx9）。
fn more_button(state: &Rc<AppState>, r: &Record) -> View {
    let id = r.id;
    let is_text = matches!(r.payload, Payload::Text { .. });
    let body = match &r.payload {
        Payload::Text { text } => text.clone(),
        Payload::Files { .. } => String::new(),
    };
    let open = signal(false);
    view()
        .child(
            view()
                .w(px(28.0))
                .h(px(28.0))
                .rounded(px(9.0))
                .bg(p::MORE_BTN)
                .border_1()
                .border_color(p::BORDER_SOFT)
                .items_center()
                .justify_center()
                .cursor_pointer()
                .label("记录操作")
                .role(AccessibilityRole::Button)
                .hover(|s| s.bg(p::LINE_SOFT))
                .on_press({
                    let open = open.clone();
                    move || open.update(|o| *o = !*o)
                })
                .child(ic::icon(ic::MORE, 13.0, p::MUTED)),
        )
        .child(when(
            {
                let open = open.clone();
                move || open.get()
            },
            {
                let state = state.clone();
                let open = open.clone();
                let body = body.clone();
                move || record_menu(&state, open.clone(), id, is_text, body.clone())
            },
        ))
}

fn record_menu(state: &Rc<AppState>, open: Signal<bool>, id: RecordId, is_text: bool, body: String) -> View {
    let mut items: Vec<View> = Vec::new();
    if is_text {
        let state1 = state.clone();
        let body1 = body.clone();
        items.push(menu_item("复制文本", move || {
            state1.copy_text(&body1);
        }));
        let state2 = state.clone();
        items.push(menu_item("导出 .txt", move || {
            state2.export_text(id, false);
        }));
        let state3 = state.clone();
        items.push(menu_item("导出 .md", move || {
            state3.export_text(id, true);
        }));
    } else {
        let state1 = state.clone();
        items.push(menu_item("打开", move || {
            if let Some(path) = state1.handle.file_path(&id, 0) {
                crate::state::open_in_system(path);
            }
        }));
        let state2 = state.clone();
        items.push(menu_item("显示位置", move || {
            if let Some(path) = state2.handle.file_path(&id, 0)
                && let Some(parent) = path.parent().map(|p| p.to_path_buf())
            {
                crate::state::open_in_system(parent);
            }
        }));
    }
    let state4 = state.clone();
    items.push(menu_item_danger("删除记录（可撤销）", move || {
        state4.delete_record(id);
    }));
    view()
        .absolute()
        .top(px(36.0))
        .right(px(0.0))
        .w(px(170.0))
        .rounded(px(12.0))
        .bg(p::SURFACE)
        .border_1()
        .border_color(p::BORDER)
        .shadow(p::shadow_panel())
        .p(px(5.0))
        .gap(px(1.0))
        .z_index(30)
        .bind_motion(&motion().entering(mo::enter_panel()))
        .child(view().absolute().top_0().left_0().right_0().bottom_0().z_index(0).on_press({
            let open = open.clone();
            move || open.set(false)
        }))
        .child(view().relative().gap(px(1.0)).z_index(1).children(items))
}

pub(crate) fn menu_item(label: &'static str, on_press: impl Fn() + 'static) -> View {
    view()
        .h(px(30.0))
        .px(px(10.0))
        .rounded(px(8.0))
        .flex_row()
        .items_center()
        .cursor_pointer()
        .hover(|s| s.bg(p::SUNKEN))
        .active(|s| s.opacity(0.8))
        .on_press(move || on_press())
        .child(text(label).text_size(px(11.5)).text_color(p::BODY))
}

fn menu_item_danger(label: &'static str, on_press: impl Fn() + 'static) -> View {
    view()
        .h(px(30.0))
        .px(px(10.0))
        .rounded(px(8.0))
        .flex_row()
        .items_center()
        .cursor_pointer()
        .hover(|s| s.bg(p::DANGER_SOFT))
        .active(|s| s.opacity(0.8))
        .on_press(move || on_press())
        .child(text(label).text_size(px(11.5)).text_color(p::DANGER_FILL))
}

/// 拆 payload：标题 / 文件列表 / 是否文本 / 文本内容。
fn payload_parts(r: &Record) -> (String, Vec<drift::FileEntry>, bool, String) {
    match &r.payload {
        Payload::Text { text } => (r.title(), Vec::new(), true, text.clone()),
        Payload::Files { files } => (r.title(), files.clone(), false, String::new()),
    }
}
