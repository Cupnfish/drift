//! 工具栏（桌面 72px / 移动紧凑页头）+ 搜索展开 + 更多菜单。

use std::rc::Rc;

use nsh_base::input;
use nsh_core::prelude::*;

use crate::{icons as ic, palette as p, state::AppState, ui};

/// 桌面工具栏（原生标题栏下方）。
pub fn toolbar_desktop(state: &Rc<AppState>) -> View {
    let state_close = state.clone();
    view()
        .h(px(72.0))
        .flex_row()
        .items_center()
        .px(px(24.0))
        .bg(p::SURFACE)
        .border_b_1()
        .border_b_color(p::TOOLBAR_EDGE)
        // 品牌
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(12.0))
                .child(
                    view()
                        .w(px(34.0))
                        .h(px(34.0))
                        .rounded(px(11.0))
                        .bg_linear(deg(45.0))
                        .gradient_from(p::ACCENT)
                        .gradient_to(p::ACCENT_STRONG)
                        .items_center()
                        .justify_center()
                        .child(ic::icon(ic::SEND, 19.0, p::WHITE)),
                )
                .child(
                    text("Drift")
                        .text_size(px(21.0))
                        .font_weight(FontWeight::BOLD)
                        .letter_spacing(px(-0.8))
                        .text_color(p::INK),
                ),
        )
        .child(view().flex_1())
        .child(online_count(state, 10.5))
        .child(folder_button(state))
        .child(search_area(state, true))
        .child(more_button(&state_close))
}

/// 移动页头。
pub fn toolbar_mobile(state: &Rc<AppState>) -> View {
    let state_close = state.clone();
    view()
        .h(px(56.0))
        .flex_row()
        .items_center()
        .px(px(20.0))
        .bg(p::SURFACE)
        .border_b_1()
        .border_b_color(p::LINE_SOFT)
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(10.0))
                .child(
                    view()
                        .w(px(32.0))
                        .h(px(32.0))
                        .rounded(px(10.0))
                        .bg_linear(deg(45.0))
                        .gradient_from(p::ACCENT)
                        .gradient_to(p::ACCENT_STRONG)
                        .items_center()
                        .justify_center()
                        .child(ic::icon(ic::SEND, 18.0, p::WHITE)),
                )
                .child(
                    text("Drift")
                        .text_size(px(22.0))
                        .font_weight(FontWeight::BOLD)
                        .letter_spacing(px(-0.8))
                        .text_color(p::INK),
                ),
        )
        .child(view().flex_1())
        .child(online_count(state, 10.0))
        .child(search_area(state, false))
        .child(more_button(&state_close))
}

/// 「N 台在线」+ accent 圆点（板：mono faint，圆点紧贴文字右侧）。
fn online_count(state: &Rc<AppState>, size: f32) -> View {
    let paired = state.paired.clone();
    view()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .mr(px(8.0))
        .child(
            text(move || {
                let n = paired.get().iter().filter(|d| d.online).count();
                format!("{n} 台在线")
            })
            .text_size(px(size))
            .text_color(p::FAINT),
        )
        .child(view().w(px(6.4)).h(px(6.4)).rounded_full().bg(p::ACCENT))
}

/// 接收目录按钮（仅桌面；打开下载目录）。
fn folder_button(state: &Rc<AppState>) -> View {
    let state = state.clone();
    ui::icon_button(ic::FOLDER, 18.0, p::ICON, "接收目录", move || {
        let dir = state.settings.get_untracked().download_dir;
        crate::state::open_in_system(dir);
    })
}

/// 搜索：图标按钮 ↔ 内联输入框。
fn search_area(state: &Rc<AppState>, desktop: bool) -> View {
    let open = state.search_open.clone();
    let query = state.filter_query.clone();
    view().flex_row().items_center().gap(px(4.0)).child(when(
        {
            let open = open.clone();
            move || open.get()
        },
        {
            let state = state.clone();
            move || {
                let filter = state.filter_query.clone();
                view()
                    .w(px(if desktop { 220.0 } else { 160.0 }))
                    .h(px(34.0))
                    .px(px(12.0))
                    .rounded(px(10.0))
                    .bg(p::SUNKEN)
                    .border_1()
                    .border_color(p::BORDER_SOFT)
                    .child(
                        input()
                            .value(query.clone())
                            .on_change({
                                let state = state.clone();
                                move |text: &str| {
                                    filter.set(text.to_string());
                                    state.refresh_records();
                                }
                            })
                            .placeholder("搜索记录…")
                            .flex_1()
                            .text_size(px(12.5)),
                    )
            }
        },
    ))
    .child(when(
        {
            let open = open.clone();
            move || !open.get()
        },
        {
            let state = state.clone();
            move || {
                ui::icon_button(ic::SEARCH, 18.0, p::ICON, "搜索", {
                    let state = state.clone();
                    move || state.search_open.set(true)
                })
            }
        },
    ))
}

/// 更多菜单按钮（下拉）。
fn more_button(state: &Rc<AppState>) -> View {
    let open = signal(false);
    view()
        .child(
            ui::icon_button(ic::MORE, 18.0, p::ICON, "更多", {
                let open = open.clone();
                move || open.update(|o| *o = !*o)
            }),
        )
        .child(when(
            {
                let open = open.clone();
                move || open.get()
            },
            {
                let state = state.clone();
                let open = open.clone();
                move || more_dropdown(&state, open)
            },
        ))
}

fn more_dropdown(state: &Rc<AppState>, open: Signal<bool>) -> View {
    view()
        .absolute()
        .top(px(64.0))
        .right(px(16.0))
        .w(px(190.0))
        .rounded(px(14.0))
        .bg(p::SURFACE)
        .border_1()
        .border_color(p::BORDER)
        .shadow(p::shadow_panel())
        .overflow_hidden()
        .z_index(40)
        .bind_motion(&motion().entering(crate::motion::enter_panel()))
        .child(
            view().gap(px(2.0)).p(px(6.0)).child(
                view()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .on_press({
                        let open = open.clone();
                        move || open.set(false)
                    }),
            ),
        )
        .child(
            view()
                .gap(px(2.0))
                .p(px(6.0))
                .z_index(1)
                .children(vec![
                    dropdown_item("本机名称与配对管理", {
                        let state = state.clone();
                        let open = open.clone();
                        move || {
                            open.set(false);
                            state.pairing_open.set(true);
                        }
                    }),
                    dropdown_item("打开接收目录", {
                        let state = state.clone();
                        let open = open.clone();
                        move || {
                            open.set(false);
                            let dir = state.settings.get_untracked().download_dir;
                            crate::state::open_in_system(dir);
                        }
                    }),
                    dropdown_item("导出全部记录 CSV", {
                        let state = state.clone();
                        let open = open.clone();
                        move || {
                            open.set(false);
                            crate::state::export_csv_flow(&state);
                        }
                    }),
                ]),
        )
}

fn dropdown_item(label: &'static str, on_press: impl Fn() + 'static) -> View {
    view()
        .h(px(32.0))
        .px(px(10.0))
        .rounded(px(9.0))
        .flex_row()
        .items_center()
        .cursor_pointer()
        .hover(|s| s.bg(p::SUNKEN))
        .active(|s| s.opacity(0.8))
        .on_press(move || on_press())
        .child(
            text(label)
                .text_size(px(12.0))
                .text_color(p::BODY),
        )
}
