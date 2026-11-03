//! 横幅：撤销删除条（底部黑条 5s）+ 轻提示（顶部）。

use std::rc::Rc;

use nsh_core::prelude::*;

use crate::{icons as ic, motion as mo, palette as p, state::{AppState, ToastKind}};

/// 覆盖层：toasts（右上）+ 撤销条（底部居中）。
pub fn banners(state: &Rc<AppState>, mobile: bool) -> impl IntoElement {
    view()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        .z_index(60)
        .pointer_events(PointerEvents::None)
        .child(toast_stack(state, mobile))
        .child(undo_bar(state, mobile))
}

fn toast_stack(state: &Rc<AppState>, mobile: bool) -> View {
    view()
        .absolute()
        .top(px(if mobile { 16.0 } else { 20.0 }))
        .right(px(if mobile { 16.0 } else { 20.0 }))
        .flex_col()
        .gap(px(8.0))
        .items_end()
        .pointer_events(PointerEvents::Auto)
        .child(each(
            {
                let toasts = state.toasts.clone();
                move || toasts.get()
            },
            |t: &crate::state::Toast| t.id,
            move |t, _| {
                let kind = t.get_untracked().kind;
                let message = t.get_untracked().message;
                view()
                    .max_w(px(320.0))
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .py(px(9.0))
                    .pl(px(12.0))
                    .pr(px(14.0))
                    .rounded(px(12.0))
                    .bg(p::SURFACE)
                    .border_1()
                    .border_color(match kind {
                        ToastKind::Success => p::SUCCESS_LINE,
                        ToastKind::Error => p::DANGER_LINE,
                        ToastKind::Info => p::BORDER,
                    })
                    .shadow(p::shadow_panel())
                    .bind_motion(&motion().entering(mo::enter_rise(0.0)))
                    .child(match kind {
                        ToastKind::Success => ic::icon(ic::CHECK, 13.0, p::SUCCESS_STRONG).into_element(),
                        ToastKind::Error => ic::icon(ic::WARN, 13.0, p::DANGER).into_element(),
                        ToastKind::Info => ic::icon(ic::SHIELD, 13.0, p::MUTED).into_element(),
                    })
                    .child(
                        text(message)
                            .text_size(px(11.5))
                            .text_color(p::BODY),
                    )
            },
        ))
}

/// 「已删除 · 撤销」黑底条。
fn undo_bar(state: &Rc<AppState>, mobile: bool) -> View {
    view()
        .absolute()
        .bottom(px(if mobile { 96.0 } else { 110.0 }))
        .left_0()
        .right_0()
        .items_center()
        .pointer_events(PointerEvents::Auto)
        .child(when(
            {
                let state = state.clone();
                move || state.undo_pending.get().is_some()
            },
            {
                let state = state.clone();
                move || {
                    view()
                        .h(px(36.0))
                        .flex_row()
                        .items_center()
                        .gap(px(12.0))
                        .pl(px(16.0))
                        .pr(px(8.0))
                        .rounded(px(18.0))
                        .bg(p::INK)
                        .shadow(p::shadow_panel())
                        .bind_motion(&motion().entering(mo::undo_enter()))
                        .child(
                            text("已删除")
                                .text_size(px(11.5))
                                .text_color(rgba(0xFFFFFFCC)),
                        )
                        .child(
                            view()
                                .h(px(24.0))
                                .px(px(10.0))
                                .items_center()
                                .rounded(px(12.0))
                                .bg(rgba(0xFFFFFF26))
                                .cursor_pointer()
                                .hover(|s| s.bg(rgba(0xFFFFFF40)))
                                .on_press({
                                    let state = state.clone();
                                    move || state.undo_delete()
                                })
                                .child(
                                    text("撤销")
                                        .text_size(px(11.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(p::WHITE),
                                ),
                        )
                }
            },
        ))
}
