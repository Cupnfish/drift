//! 空状态（drift-empty-desktop / drift-empty-mobile）：
//! 传输路径插画 + 引导文案 + 主按钮 + 三步说明。

use std::rc::Rc;

use nsh_core::prelude::*;

use crate::{icons as ic, motion as mo, palette as p, state::AppState};

/// 插画：两块设备牌 + 虚线路径 + 在途文件卡。
/// `scale`：桌面 1.0 / 移动 0.74。
fn illustration(scale: f32) -> View {
    let tile = 76.0 * scale;
    let icon = 30.0 * scale;
    let card_w = 116.0 * scale;
    let card_h = 40.0 * scale;
    view()
        .w(px(340.0 * scale))
        .h(px(110.0 * scale))
        .flex_row()
        .items_center()
        // 左牌：这台电脑
        .child(
            view()
                .w(px(tile))
                .h(px(tile))
                .rounded(px(20.0 * scale))
                .bg(p::SURFACE)
                .border_1()
                .border_color(p::TRANSIT_CARD)
                .items_center()
                .justify_center()
                .child(ic::icon(ic::LAPTOP, icon, p::ACCENT)),
        )
        // 中段：虚线路径 + 在途文件卡
        .child(
            view()
                .flex_1()
                .mx(px(12.0 * scale))
                .relative()
                .h(px(tile))
                .child(
                    // 虚线路径：垂直居中的上边框虚线
                    view()
                        .absolute()
                        .left(px(0.0))
                        .right(px(0.0))
                        .top(px(tile / 2.0))
                        .h(px(0.0))
                        .border_t(px(2.0))
                        .border_dashed()
                        .border_color(p::PATH_DASH),
                )
                .child(
                    // 在途文件卡，骑在路径上方
                    view()
                        .absolute()
                        .top(px(tile / 2.0 - card_h - 8.0))
                        .left(px(24.0 * scale))
                        .w(px(card_w))
                        .h(px(card_h))
                        .rounded(px(12.0 * scale))
                        .bg(p::SURFACE)
                        .border_1()
                        .border_color(p::TRANSIT_CARD)
                        .flex_row()
                        .items_center()
                        .gap(px(7.0 * scale))
                        .px(px(7.0 * scale))
                        .child(
                            view()
                                .w(px(22.0 * scale))
                                .h(px(22.0 * scale))
                                .rounded(px(7.0 * scale))
                                .bg(p::THUMB_BG)
                                .items_center()
                                .justify_center()
                                .child(ic::icon(ic::FILE, 14.0 * scale, p::FILE_INK)),
                        )
                        .child(
                            view()
                                .gap(px(1.5))
                                .child(
                                    text("8.4 MB")
                                        .text_size(px(10.5 * scale))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(p::INK),
                                )
                                .child(
                                    text("传输中")
                                        .text_size(px(8.5 * scale))
                                        .text_color(p::TRANSIT_TEXT),
                                ),
                        ),
                )
                .child(
                    // 箭头端点
                    view()
                        .absolute()
                        .right(px(-6.0))
                        .top(px(tile / 2.0 - 5.0))
                        .child(ic::icon(ic::ARROW, 10.0 * scale, p::PATH_DASH)),
                ),
        )
        // 右牌：手机
        .child(
            view()
                .w(px(tile))
                .h(px(tile))
                .rounded(px(20.0 * scale))
                .bg(p::SURFACE)
                .border_1()
                .border_color(p::TRANSIT_CARD)
                .items_center()
                .justify_center()
                .child(ic::icon(ic::PHONE, icon, p::ACCENT)),
        )
}

/// 三步引导（桌面：三列；移动：凹陷卡内三行）。
fn steps(mobile: bool) -> View {
    let rows: [(&'static str, &'static str, &'static str); 3] = if mobile {
        [
            ("01", "配对后，输入文本或添加文件即弹出设备层", ""),
            ("02", "无发送按钮，点设备即发，改内容可增量补发", ""),
            ("03", "所有设备的记录排在同一条时间线上", ""),
        ]
    } else {
        [
            ("01", "配对设备", "同一 Wi-Fi 自动发现"),
            ("02", "输入即弹出", "有内容自动弹出设备层"),
            ("03", "点设备即发", "无需发送键，改内容可增量补发"),
        ]
    };
    if mobile {
        view()
            .w(px(350.0))
            .rounded(px(18.0))
            .bg(p::SUNKEN)
            .py(px(14.0))
            .gap(px(11.0))
            .children(rows.iter().map(|(no, title, _)| {
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(12.0))
                    .px(px(18.0))
                    .child(
                        text(*no)
                            .text_size(px(9.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(p::ACCENT)
                            .letter_spacing(px(0.8)),
                    )
                    .child(text(*title).text_size(px(11.0)).text_color(p::INK))
            }))
    } else {
        view()
            .flex_row()
            .gap(px(64.0))
            .children(rows.iter().map(|(no, title, desc)| {
                view()
                    .gap(px(5.0))
                    .items_center()
                    .child(
                        text(*no)
                            .text_size(px(10.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(p::ACCENT)
                            .letter_spacing(px(0.8)),
                    )
                    .child(
                        text(*title)
                            .text_size(px(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p::INK),
                    )
                    .child(text(*desc).text_size(px(10.0)).text_color(p::FAINT))
            }))
    }
}

/// 空状态主区。`mobile` 分流文案与排版。
pub fn empty_state(state: &Rc<AppState>, mobile: bool) -> View {
    let scale = if mobile { 0.74 } else { 1.0 };
    let title = if mobile { "先配对一台设备" } else { "把第一台设备连起来" };
    let subtitle = if mobile {
        "在电脑上打开 Drift，会自动发现这台设备，\n核对配对码即可互传。传输不经过服务器。"
    } else {
        "同一 Wi-Fi 下 Drift 会自动发现附近设备，\n配对时两端会显示同一组号码，核对一致即可。"
    };
    let btn_label = if mobile { "查找附近设备" } else { "配对新设备" };

    let art = motion().entering(mo::enter_rise(0.0));
    let copy = motion().entering(mo::enter_rise(40.0));
    let button = motion().entering(mo::enter_rise(80.0));
    let extra = motion().entering(mo::enter_rise(120.0));

    let open_pairing = {
        let state = state.clone();
        move || state.pairing_open.set(true)
    };

    view()
        .flex_1()
        .items_center()
        .justify_center()
        .gap(px(if mobile { 14.0 } else { 18.0 }))
        .py(px(16.0))
        .child(view().bind_motion(&art).child(illustration(scale)))
        .child(
            view()
                .bind_motion(&copy)
                .gap(px(8.0))
                .items_center()
                .child(
                    text(title)
                        .text_size(px(if mobile { 21.0 } else { 24.0 }))
                        .font_weight(FontWeight::BOLD)
                        .text_color(p::INK),
                )
                .child(
                    text(subtitle)
                        .text_size(px(if mobile { 12.0 } else { 13.0 }))
                        .text_color(p::MUTED)
                        .text_center(),
                ),
        )
        .child(view().bind_motion(&button).child(primary_button(btn_label, open_pairing)))
        .child(if mobile {
            view()
                .bind_motion(&extra)
                .gap(px(12.0))
                .child(
                    text("查看本机配对码")
                        .text_size(px(11.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p::MUTED),
                )
                .child(steps(true))
                .child(
                    text("配对后可在设备列表里随时撤销")
                        .text_size(px(9.5))
                        .text_color(p::FAINT),
                )
                .into_element()
        } else {
            view()
                .bind_motion(&extra)
                .gap(px(18.0))
                .child(steps(false))
                .child(
                    text("配对之后，收发记录会按时间出现在这里")
                        .text_size(px(10.5))
                        .text_color(p::FAINT),
                )
                .into_element()
        })
}

/// 主按钮（品牌渐变 + 雷达图标）。
pub fn primary_button(label: &'static str, on_press: impl Fn() + 'static) -> View {
    let (h, r, fs) = if label.chars().count() > 4 {
        (46.0, 15.0, 14.0)
    } else {
        (42.0, 14.0, 12.5)
    };
    view()
        .h(px(h))
        .px(px(22.0))
        .flex_row()
        .items_center()
        .justify_center()
        .gap(px(7.0))
        .rounded(px(r))
        .bg_linear(deg(45.0))
        .gradient_from(p::ACCENT)
        .gradient_to(p::ACCENT_STRONG)
        .cursor_pointer()
        .hover(|s| s.opacity(0.92))
        .active(|s| s.opacity(0.8))
        .on_press(move || on_press())
        .child(ic::icon(ic::RADAR, 16.0, p::WHITE))
        .child(
            text(label)
                .text_size(px(fs))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p::WHITE),
        )
}
