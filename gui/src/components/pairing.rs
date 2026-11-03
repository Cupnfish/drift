//! 配对面板（drift-pairing.svg）：附近设备 → 同码对照 → 双向确认 → 结果态。
//! 桌面：居中弹层 536 宽；移动：全宽面板。

use std::rc::Rc;

use drift::{PairedDeviceView, discovery::PeerDevice};
use nsh_core::prelude::*;

use crate::{icons as ic, motion as mo, palette as p, state::AppState, ui};

/// 剩余时间 "02:41"。
fn remaining(expires_at: chrono::DateTime<chrono::Utc>) -> String {
    let secs = (expires_at - chrono::Utc::now()).num_seconds().max(0);
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

/// 配对面板（挂在 modal 里）。`mobile` 分流排版。
pub fn pairing_layer(state: &Rc<AppState>, mobile: bool) -> impl IntoElement {
    when(
        {
            let state = state.clone();
            move || state.pairing_open.get()
        },
        {
            let state = state.clone();
            move || modal_shell(&state, mobile)
        },
    )
}

fn modal_shell(state: &Rc<AppState>, mobile: bool) -> AnyElement {
    let state_for_close = state.clone();
    nsh_core::element::modal()
        .on_cancel(move || state_for_close.pairing_open.set(false))
        .child(
            view()
                .flex_1()
                .bg(rgba(0x202331B3))
                .items_center()
                .justify_center()
                .p(px(if mobile { 10.0 } else { 32.0 }))
                .on_press({
                    let state = state.clone();
                    move || state.pairing_open.set(false)
                })
                .child(
                    view()
                        .on_press(|| {})
                        .max_w(px(if mobile { 400.0 } else { 536.0 }))
                        .w_full()
                        .rounded(px(24.0))
                        .bg(p::SURFACE)
                        .shadow(p::shadow_panel())
                        .overflow_hidden()
                        .bind_motion(&motion().entering(mo::enter_panel()))
                        .child(panel_body(state, mobile)),
                ),
        )
        .into_element()
}

fn panel_body(state: &Rc<AppState>, mobile: bool) -> View {
    view()
        .p(px(if mobile { 20.0 } else { 24.0 }))
        .gap(px(14.0))
        // 标题行
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .child(
                    text("配对新设备")
                        .text_size(px(if mobile { 16.0 } else { 19.0 }))
                        .font_weight(FontWeight::BOLD)
                        .text_color(p::INK),
                )
                .child(
                    view()
                        .w(px(28.0))
                        .h(px(28.0))
                        .rounded(px(9.0))
                        .bg(p::IDLE)
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(|s| s.bg(p::LINE_SOFT))
                        .on_press({
                            let state = state.clone();
                            move || state.pairing_open.set(false)
                        })
                        .child(ic::icon(ic::CLOSE, 14.0, p::MUTED)),
                ),
        )
        .child(
            text("点一台附近设备，两端会同时出现同一组号码")
                .text_size(px(11.0))
                .text_color(p::MUTED),
        )
        .child(paired_section(state))
        .child(nearby_section(state))
        .child(code_section(state, mobile))
        .child(privacy_bar())
}

/// 已配对设备（可撤销）。
fn paired_section(state: &Rc<AppState>) -> View {
    let paired = state.paired.clone();
    let state_rows = state.clone();
    view()
        .gap(px(6.0))
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .child(ui::section_label("已配对设备".into()))
                .child(
                    text(move || format!("{} 台", paired.get().len()))
                        .text_size(px(10.0))
                        .text_color(p::FAINT),
                ),
        )
        .child(
            view().gap(px(6.0)).child(each(
                {
                    let paired = state.paired.clone();
                    move || {
                        paired
                            .get()
                            .into_iter()
                            .map(crate::state::NeverEq)
                            .collect::<Vec<_>>()
                    }
                },
                |d: &crate::state::NeverEq<PairedDeviceView>| d.0.fingerprint.to_hex(),
                move |d, _| paired_row(&state_rows, d),
            )),
        )
}

fn paired_row(state: &Rc<AppState>, d: ReadSignal<crate::state::NeverEq<PairedDeviceView>>) -> View {
    let fp = d.get_untracked().0.fingerprint;
    let kind = {
        let d = d.clone();
        move || d.get().0.kind.clone()
    };
    view()
        .h(px(56.0))
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .px(px(12.0))
        .rounded(px(14.0))
        .bg(p::SURFACE)
        .border_1()
        .border_color(p::BORDER_SOFT)
        .child(ui::avatar(36.0, p::IDLE, &kind(), p::MUTED))
        .child(
            view()
                .flex_1()
                .gap(px(2.0))
                .child(
                    text(move || d.get().0.name)
                        .text_size(px(12.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p::INK)
                        .number_of_lines(1),
                )
                .child(
                    text(move || {
                        if d.get().0.online {
                            "在线 · 已建立双向信任".to_string()
                        } else {
                            "离线".to_string()
                        }
                    })
                    .text_size(px(10.0))
                    .text_color(p::MUTED),
                ),
        )
        .child(
            view()
                .w(px(28.0))
                .h(px(28.0))
                .rounded(px(9.0))
                .bg(p::DANGER_WASH)
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.opacity(0.8))
                .on_press({
                    let state = state.clone();
                    move || state.unpair(fp)
                })
                .child(ic::icon(ic::UNLINK, 13.0, p::DANGER_ICON)),
        )
}

/// 附近未配对设备 + 查找指示。
fn nearby_section(state: &Rc<AppState>) -> View {
    let nearby = state.nearby.clone();
    let paired = state.paired.clone();
    let state_rows = state.clone();
    view()
        .gap(px(6.0))
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .child(ui::section_label("附近设备".into()))
                .child(
                    text(move || {
                        let paired: Vec<_> = paired.get().into_iter().map(|d| d.fingerprint).collect();
                        let n = nearby
                            .get()
                            .into_iter()
                            .filter(|d| !paired.contains(&d.fingerprint))
                            .count();
                        format!("{n} 台在线")
                    })
                    .text_size(px(10.0))
                    .text_color(p::FAINT),
                ),
        )
        .child(
            view().gap(px(6.0)).child(each(
                move || {
                    let paired: Vec<_> = paired
                        .get()
                        .into_iter()
                        .map(|d| d.fingerprint)
                        .collect();
                    nearby
                        .get()
                        .into_iter()
                        .filter(|d| !paired.contains(&d.fingerprint))
                        .map(crate::state::NeverEq)
                        .collect::<Vec<_>>()
                },
                |d: &crate::state::NeverEq<PeerDevice>| d.0.fingerprint.to_hex(),
                move |d, _| nearby_row(&state_rows, d),
            )),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(7.0))
                .px(px(4.0))
                .py(px(2.0))
                .child(
                    view()
                        .w(px(9.0))
                        .h(px(9.0))
                        .rounded_full()
                        .border_1()
                        .border_color(p::PAIRING_RING)
                        .items_center()
                        .justify_center()
                        .child(view().w(px(3.5)).h(px(3.5)).rounded_full().bg(p::PAIRING_DOT)),
                )
                .child(
                    text("正在查找更多设备…")
                        .text_size(px(10.0))
                        .text_color(p::FAINT),
                ),
        )
}

fn nearby_row(state: &Rc<AppState>, d: ReadSignal<crate::state::NeverEq<PeerDevice>>) -> View {
    let fp = d.get_untracked().0.fingerprint;
    let kind = {
        let d = d.clone();
        move || d.get().0.kind.as_str().to_string()
    };
    let name = {
        let d = d.clone();
        move || d.get().0.name
    };
    let is_target = {
        let state = state.clone();
        move || state.pairings.get().first().is_some_and(|r| r.peer == fp)
    };
    view()
        .h(px(64.0))
        .flex_row()
        .items_center()
        .gap(px(12.0))
        .px(px(12.0))
        .rounded(px(16.0))
        .cursor_pointer()
        .style_when(is_target.clone(), |s| {
            s.bg(p::SELECTED_ROW).border_1().border_color(p::ACCENT_LINE)
        })
        .style_when(
            {
                let is_target = is_target.clone();
                move || !is_target()
            },
            |s| s.bg(p::SUNKEN_ALT).border_1().border_color(p::BORDER_SOFT),
        )
        .hover(|s| s.opacity(0.92))
        .active(|s| s.opacity(0.85))
        .on_press({
            let state = state.clone();
            move || state.initiate_pairing(fp)
        })
        .child(ui::avatar(40.0, p::ACCENT_SOFT, &kind(), p::ACCENT_ICON))
        .child(
            view()
                .flex_1()
                .gap(px(2.0))
                .child(
                    text(name)
                        .text_size(px(12.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p::INK)
                        .number_of_lines(1),
                )
                .child(
                    text("在线 · 同一 Wi-Fi")
                        .text_size(px(10.0))
                        .text_color(p::MUTED),
                ),
        )
        .child(when(is_target, || {
            view()
                .w(px(22.0))
                .h(px(22.0))
                .rounded_full()
                .bg(p::CONFIRM_CIRCLE)
                .items_center()
                .justify_center()
                .child(ic::icon(ic::CHECK, 12.0, p::WHITE))
        }))
}

/// 码区：节头常显；六位码 + 倒计时 + 双按钮随活动配对出现，否则占位。
fn code_section(state: &Rc<AppState>, mobile: bool) -> impl IntoElement {
    let state = state.clone();
    let request = {
        let state = state.clone();
        move || state.pairings.get().into_iter().next()
    };
    // 每秒一跳，驱动倒计时文本重算
    let tick = signal(0u32);
    set_interval(std::time::Duration::from_secs(1), {
        let tick = tick.clone();
        move || tick.update(|t| *t = t.wrapping_add(1))
    });

    let countdown_active = {
        let request = request.clone();
        let tick = tick.clone();
        move || {
            tick.get();
            request().map(|r| remaining(r.expires_at)).unwrap_or_default()
        }
    };

    view()
        .gap(px(10.0))
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_between()
                .child(ui::section_label("核对配对码".into()))
                .child(
                    text(move || match countdown_active() {
                        left if left.is_empty() => "3 分钟内有效".to_string(),
                        left => format!("有效期剩余 {left}"),
                    })
                    .text_size(px(9.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p::ACCENT_MUTE),
                ),
        )
        .child(view().gap(px(10.0)).child(dynamic(move || {
            let Some(req) = request() else {
                return placeholder_code_panel();
            };
            let fp = req.peer;
            let code: String = req.code.chars().filter(|c| c.is_ascii_digit()).collect();
            let grouped = if code.len() >= 4 {
                format!("{} {}", &code[..3], &code[3..])
            } else {
                code
            };
        view()
            .gap(px(10.0))
            .child(
                view()
                    .h(px(if mobile { 72.0 } else { 60.0 }))
                    .items_center()
                    .justify_center()
                    .rounded(px(16.0))
                    .bg(p::CODE_PANEL)
                    .child(
                        text(grouped)
                            .text_size(px(if mobile { 34.0 } else { 30.0 }))
                            .font_weight(FontWeight::SEMIBOLD)
                            .letter_spacing(px(6.0))
                            .font_family(nsh_core::element::tokens::FONT_MONO)
                            .text_color(p::INK),
                    ),
            )
            .child(
                text(format!(
                    "{} 上应显示同一组数字，不一致就点「不一致」",
                    req.peer_name
                ))
                .text_size(px(10.5))
                .text_color(p::MUTED),
            )
            .child(if mobile {
                view()
                    .items_center()
                    .child(
                        view()
                            .h(px(24.0))
                            .px(px(12.0))
                            .items_center()
                            .rounded(px(12.0))
                            .bg(p::CODE_PILL)
                            .child(
                                text("对照用，不需要输入")
                                    .text_size(px(10.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(p::CONFIRM_CIRCLE),
                            ),
                    )
                    .into_element()
            } else {
                view().h(px(0.0)).into_element()
            })
            .child(
                view()
                    .flex_row()
                    .gap(px(10.0))
                    .child({
                        let state = state.clone();
                        view()
                            .flex_1()
                            .h(px(if mobile { 48.0 } else { 42.0 }))
                            .items_center()
                            .justify_center()
                            .gap(px(7.0))
                            .rounded(px(14.0))
                            .bg_linear(deg(45.0))
                            .gradient_from(p::ACCENT)
                            .gradient_to(p::ACCENT_STRONG)
                            .cursor_pointer()
                            .hover(|s| s.opacity(0.92))
                            .active(|s| s.opacity(0.8))
                            .on_press({
                                let state = state.clone();
                                move || state.confirm_pairing(fp)
                            })
                            .child(ic::icon(ic::CHECK, 15.0, p::WHITE))
                            .child(
                                text(if mobile { "一致，确认配对" } else { "一致，配对" })
                                    .text_size(px(if mobile { 14.0 } else { 12.5 }))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(p::WHITE),
                            )
                    })
                    .child({
                        let state = state.clone();
                        view()
                            .flex_1()
                            .h(px(if mobile { 48.0 } else { 42.0 }))
                            .items_center()
                            .justify_center()
                            .rounded(px(14.0))
                            .bg(p::SURFACE)
                            .border_1()
                            .border_color(rgb(0xE1E3EC))
                            .cursor_pointer()
                            .hover(|s| s.bg(p::SUNKEN))
                            .active(|s| s.opacity(0.85))
                            .on_press({
                                let state = state.clone();
                                move || state.reject_pairing(fp)
                            })
                            .child(
                                text("不一致")
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(p::MUTED),
                            )
                    }),
            )
            .into_element()
        })))
}

/// 无进行中配对时的占位。
fn placeholder_code_panel() -> AnyElement {
    view()
        .h(px(72.0))
        .items_center()
        .justify_center()
        .rounded(px(16.0))
        .bg(p::SUNKEN)
        .child(
            text("从上面选一台设备，两端会同时出现同一组号码")
                .text_size(px(10.5))
                .text_color(p::FAINT),
        )
        .into_element()
}

/// 隐私条。
fn privacy_bar() -> View {
    view()
        .h(px(34.0))
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(12.0))
        .rounded(px(11.0))
        .bg(p::SUNKEN)
        .child(ic::icon(ic::SHIELD, 13.0, p::MUTED))
        .child(
            text("号码只用于两端对照，不会上传 · 配对后可随时撤销")
                .text_size(px(9.5))
                .text_color(p::FAINT),
        )
}
