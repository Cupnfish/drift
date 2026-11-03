//! 筛选：全部 / 文件 / 图片 / 文本 滑动 pill tabs。

use std::rc::Rc;

use nsh_core::prelude::*;

use crate::{palette as p, state::{AppState, FilterKind}};

const TABS: [FilterKind; 4] = [
    FilterKind::All,
    FilterKind::File,
    FilterKind::Image,
    FilterKind::Text,
];

/// 筛选行（有时间线记录时显示）。
pub fn filter_tabs(state: &Rc<AppState>, mobile: bool) -> View {
    view()
        .max_w(px(888.0))
        .self_center()
        .w_full()
        .px(px(if mobile { 20.0 } else { 24.0 }))
        .py(px(8.0))
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .child(tabs_pill(state))
        .child(view().flex_1())
        .child(device_filter(state))
}

/// 滑动 pill 的分段控件。
fn tabs_pill(state: &Rc<AppState>) -> View {
    let selected = state.filter_kind.clone();
    let index = move || {
        TABS.iter()
            .position(|t| *t == selected.get())
            .unwrap_or(0) as f32
    };
    view()
        .relative()
        .flex_row()
        .p(px(3.0))
        .rounded(px(12.0))
        .bg(p::SUNKEN)
        .child(
            // 滑动 pill
            view()
                .absolute()
                .top(px(3.0))
                .left(px(3.0))
                .w(px(52.0))
                .h(px(26.0))
                .rounded(px(10.0))
                .bg(p::SURFACE)
                .shadow(p::shadow_card())
                .translate_x(move || px(index() * 54.0))
                .transition_transform()
                .duration(crate::motion::D_PANEL)
                .ease_bezier(
                    crate::motion::EASE_STD.0,
                    crate::motion::EASE_STD.1,
                    crate::motion::EASE_STD.2,
                    crate::motion::EASE_STD.3,
                ),
        )
        .children(TABS.iter().map(|tab| {
            let state_tab = state.clone();
            let is_active = {
                let selected = selected.clone();
                let tab = tab.clone();
                move || selected.get() == tab
            };
            view()
                .w(px(52.0))
                .h(px(26.0))
                .items_center()
                .justify_center()
                .cursor_pointer()
                .on_press(move || {
                    state_tab.filter_kind.set(tab.clone());
                    state_tab.refresh_records();
                })
                .child(
                    text(tab.label())
                        .text_size(px(11.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || if is_active() { p::INK } else { p::MUTED }),
                )
        }))
}

/// 设备筛选：循环切换 全部 → 各设备。
fn device_filter(state: &Rc<AppState>) -> View {
    let paired = state.paired.clone();
    let filter = state.filter_device.clone();
    let label = move || {
        let fp = filter.get();
        let paired = paired.get();
        match fp {
            None => "全部设备".to_string(),
            Some(fp) => paired
                .iter()
                .find(|d| d.fingerprint == fp)
                .map(|d| d.name.clone())
                .unwrap_or_else(|| "全部设备".to_string()),
        }
    };
    let state_cycle = state.clone();
    view()
        .h(px(28.0))
        .px(px(12.0))
        .items_center()
        .gap(px(6.0))
        .rounded(px(9.0))
        .bg(p::SUNKEN)
        .border_1()
        .border_color(p::BORDER_SOFT)
        .cursor_pointer()
        .hover(|s| s.bg(p::LINE_SOFT))
        .on_press(move || {
            let paired = state_cycle.paired.get_untracked();
            let current = state_cycle.filter_device.get_untracked();
            let next = match current {
                None => paired.first().map(|d| d.fingerprint),
                Some(fp) => {
                    let pos = paired.iter().position(|d| d.fingerprint == fp);
                    match pos {
                        Some(i) if i + 1 < paired.len() => Some(paired[i + 1].fingerprint),
                        _ => None,
                    }
                }
            };
            state_cycle.filter_device.set(next);
            state_cycle.refresh_records();
        })
        .child(
            text(label)
                .text_size(px(10.5))
                .text_color(p::MUTED),
        )
}
