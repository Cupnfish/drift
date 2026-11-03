//! 输入区（drift-main v1.5）：无发送键设计 ——
//! 输入框永远只显示正文和光标，不做 diff 高亮、不加字数徽章；
//! 有内容（文本或附件）设备球体即浮出，清空即收起；差异只属于球体。

use std::rc::Rc;

use nsh_base::input;
use nsh_core::prelude::*;

use crate::{icons as ic, palette as p, state::AppState};

/// 输入区整块（无发送按钮）。
pub fn composer(state: &Rc<AppState>, mobile: bool) -> View {
    let state = state.clone();
    let disabled_now = state.paired.get_untracked().is_empty();
    let disabled_bg = {
        let state = state.clone();
        move || state.paired.get().is_empty()
    };
    let disabled_border = {
        let state = state.clone();
        move || state.paired.get().is_empty()
    };
    let has_content = {
        let state = state.clone();
        move || !state.draft.get().trim().is_empty() || !state.attachments.get().is_empty()
    };
    let drag_state = state.clone();
    let height = signal(px(24.0));
    let radius = if mobile { 20.0 } else { 22.0 };
    view()
        .w(px(if mobile { 350.0 } else { 888.0 }))
        .self_center()
        .rounded(px(radius))
        .bg(move || if disabled_bg() { p::DISABLED_BG } else { p::COMPOSER_BG })
        .border_1()
        .border_color(move || {
            if drag_state.dragging.get() {
                p::DROPZONE
            } else if disabled_border() {
                p::DISABLED_BORDER
            } else if has_content() {
                p::ACCENT
            } else {
                p::COMPOSER_BORDER
            }
        })
        .shadow(if disabled_now {
            Vec::new()
        } else {
            p::shadow_composer_glow()
        })
        .p(px(if mobile { 10.0 } else { 16.0 }))
        .gap(px(8.0))
        .overflow_hidden()
        .child(attachments_strip(&state, mobile))
        .child(clipboard_hint(&state, mobile))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(8.0))
                .child(plus_button(&state, mobile, disabled_now))
                .child(text_input(&state, mobile, &height).flex_1())
                .child(if mobile {
                    mobile_tray_hint(&state).into_element()
                } else {
                    dropzone(&state).into_element()
                }),
        )
        .child(when(
            {
                let dragging = state.dragging.clone();
                move || dragging.get()
            },
            move || {
                view()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .rounded(px(radius))
                    .bg(rgba(0xF7F5FFCC))
                    .items_center()
                    .justify_center()
                    .child(
                        text("松开添加")
                            .text_size(px(14.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(p::DROPZONE_LABEL),
                    )
            },
        ))
}

/// 多行输入：一有内容设备球体即浮出；>3 行自增高（上限 120）。
fn text_input(
    state: &Rc<AppState>,
    mobile: bool,
    height: &Signal<nsh_core::prelude::Points>,
) -> Input {
    let state = state.clone();
    let height = height.clone();
    input()
        .value(state.draft.clone())
        .on_change({
            let state = state.clone();
            move |text: &str| {
                state.update_draft(text.to_string());
            }
        })
        .on_focus({
            let state = state.clone();
            move || {
                if state.draft.get_untracked().is_empty() {
                    state.probe_clipboard();
                }
            }
        })
        .on_paste({
            let state = state.clone();
            move |e: &PasteEvent| {
                if !e.uris.is_empty() {
                    let paths: Vec<_> = e
                        .uris
                        .iter()
                        .filter_map(|u| crate::state::uri_to_path(u))
                        .collect();
                    if !paths.is_empty() {
                        state.add_paths(paths);
                    }
                } else if !e.files.is_empty() {
                    let mut paths = Vec::new();
                    for f in &e.files {
                        let name = drift::util::sanitize_filename(&f.name);
                        let path = std::env::temp_dir().join(format!("drift-paste-{name}"));
                        if std::fs::write(&path, &f.data).is_ok() {
                            paths.push(path);
                        }
                    }
                    if !paths.is_empty() {
                        state.add_paths(paths);
                    }
                }
            }
        })
        .paste_behavior(PasteBehavior::Report)
        .on_content_size_change({
            let height = height.clone();
            move |size: &Size<Points>| {
                height.set(px((size.height.0 + 8.0).clamp(24.0, 120.0)));
            }
        })
        .multiline(true)
        .placeholder(if disabled_hint(&state) {
            if mobile {
                "配对后输入内容即自动弹出发送设备…"
            } else {
                "配对设备后，输入文本或拖入文件即自动弹出发送设备面板"
            }
        } else if mobile {
            "输入或粘贴内容，设备球体会浮出来…"
        } else {
            "粘贴或输入文本，设备球体会浮出来…"
        })
        .h(move || height.get())
        .text_size(px(if mobile { 12.5 } else { 14.0 }))
        .text_color(p::INK)
}

fn disabled_hint(state: &Rc<AppState>) -> bool {
    state.paired.get_untracked().is_empty()
}

/// 移动端输入区右端：托盘图标（拖放提示，点击选图）。板上移动端不带「拖入文件」文字。
fn mobile_tray_hint(state: &Rc<AppState>) -> AnyElement {
    view()
        .w(px(34.0))
        .h(px(34.0))
        .rounded(px(11.0))
        .bg(p::PLUS_KEY)
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|s| s.opacity(0.85))
        .on_press({
            let state = state.clone();
            move || {
                let state = state.clone();
                spawn(async move {
                    let files = pick_images(crate::state::pick_request_multi()).await;
                    let mut paths = Vec::new();
                    for f in files {
                        let name = drift::util::sanitize_filename(&f.name);
                        let path = std::env::temp_dir().join(format!("drift-pick-{name}"));
                        if std::fs::write(&path, f.data.bytes()).is_ok() {
                            paths.push(path);
                        }
                    }
                    if !paths.is_empty() {
                        state.add_paths(paths);
                    }
                });
            }
        })
        .child(ic::icon(ic::TRAY, 16.0, p::MUTED))
        .into_element()
}

/// + 键：桌面选图（pick_images），移动端系统相册。
fn plus_button(state: &Rc<AppState>, mobile: bool, disabled: bool) -> View {
    view()
        .w(px(if mobile { 36.0 } else { 34.0 }))
        .h(px(if mobile { 36.0 } else { 34.0 }))
        .rounded(px(if mobile { 12.0 } else { 11.0 }))
        .bg(if disabled {
            p::LINE_SOFT
        } else if mobile {
            p::IDLE_ALT
        } else {
            p::PLUS_KEY
        })
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(|s| s.opacity(0.85))
        .active(|s| s.opacity(0.7))
        .disabled(disabled)
        .on_press({
            let state = state.clone();
            move || {
                let state = state.clone();
                spawn(async move {
                    let files = pick_images(crate::state::pick_request_multi()).await;
                    let mut paths = Vec::new();
                    for f in files {
                        let name = drift::util::sanitize_filename(&f.name);
                        let path = std::env::temp_dir().join(format!("drift-pick-{name}"));
                        if std::fs::write(&path, f.data.bytes()).is_ok() {
                            paths.push(path);
                        }
                    }
                    if !paths.is_empty() {
                        state.add_paths(paths);
                    }
                });
            }
        })
        .child(ic::icon(
            ic::PLUS,
            18.0,
            if disabled { p::FLAG } else { p::PLUS_ICON },
        ))
}

/// 桌面内联拖放目标（点击 = 选图；拖入即进入附件，设备球体保持浮出）。
fn dropzone(state: &Rc<AppState>) -> View {
    view()
        .w(px(190.0))
        .h(px(44.0))
        .rounded(px(14.0))
        .bg(p::DROPZONE_BG)
        .border_1()
        .border_dashed()
        .border_color(p::DROPZONE)
        // 常态虚线呼吸（1.6s 循环；reduce-motion 时静止 —— 动效规范场景 3）
        .style_when(|| !reduce_motion(), |s| s.animate_pulse())
        .flex_row()
        .items_center()
        .gap(px(8.0))
        .px(px(12.0))
        .cursor_pointer()
        .hover(|s| s.opacity(0.85))
        .on_press({
            let state = state.clone();
            move || {
                let state = state.clone();
                spawn(async move {
                    let files = pick_images(crate::state::pick_request_multi()).await;
                    let mut paths = Vec::new();
                    for f in files {
                        let name = drift::util::sanitize_filename(&f.name);
                        let path = std::env::temp_dir().join(format!("drift-pick-{name}"));
                        if std::fs::write(&path, f.data.bytes()).is_ok() {
                            paths.push(path);
                        }
                    }
                    if !paths.is_empty() {
                        state.add_paths(paths);
                    }
                });
            }
        })
        .child(ic::icon(ic::TRAY, 18.0, p::DROPZONE_ICON))
        .child(
            view()
                .gap(px(1.0))
                .child(
                    text("拖入文件")
                        .text_size(px(11.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(p::DROPZONE_LABEL),
                )
                .child(
                    text("或点击选择")
                        .text_size(px(9.5))
                        .text_color(p::DROPZONE_SUB),
                ),
        )
}

/// 附件条：文件芯片，点 × 移除。
fn attachments_strip(state: &Rc<AppState>, _mobile: bool) -> AnyElement {
    let atts = state.attachments.clone();
    when(
        {
            let atts = atts.clone();
            move || !atts.get().is_empty()
        },
        {
            let state = state.clone();
            move || {
                view()
                    .flex_row()
                    .flex_wrap()
                    .gap(px(6.0))
                    .child(each(
                        {
                            let atts = state.attachments.clone();
                            move || atts.get()
                        },
                        |a: &crate::state::Attachment| format!("{}:{}", a.name, a.size),
                        {
                            let state = state.clone();
                            move |a, _| {
                                let a = a.get_untracked();
                                let index = state
                                    .attachments
                                    .get_untracked()
                                    .iter()
                                    .position(|x| x.name == a.name && x.size == a.size)
                                    .unwrap_or(0);
                                let state_remove = state.clone();
                                view()
                                    .h(px(26.0))
                                    .pl(px(9.0))
                                    .pr(px(6.0))
                                    .flex_row()
                                    .items_center()
                                    .gap(px(6.0))
                                    .rounded(px(9.0))
                                    .bg(p::ACCENT_SOFT)
                                    .border_1()
                                    .border_color(p::ACCENT_LINE)
                                    .child(ic::icon(
                                        if a.is_dir { ic::FOLDER } else { ic::FILE },
                                        12.0,
                                        p::FILE_INK,
                                    ))
                                    .child(
                                        text(a.name)
                                            .text_size(px(10.0))
                                            .text_color(p::INK)
                                            .number_of_lines(1),
                                    )
                                    .child(
                                        text(drift::util::human_bytes(a.size))
                                            .text_size(px(9.0))
                                            .text_color(p::MUTED),
                                    )
                                    .child(
                                        view()
                                            .cursor_pointer()
                                            .hover(|s| s.opacity(0.6))
                                            .on_press(move || state_remove.remove_attachment(index))
                                            .child(ic::icon(ic::CLOSE, 11.0, p::MUTED)),
                                    )
                            }
                        },
                    ))
            }
        },
    )
    .into_element()
}

/// 剪贴板建议条：文本 N 字 [粘贴] / 图片 [粘贴]。
fn clipboard_hint(state: &Rc<AppState>, _mobile: bool) -> AnyElement {
    let hint = state.clip_hint.clone();
    let draft = state.draft.clone();
    when(
        {
            let hint = hint.clone();
            let draft = draft.clone();
            move || {
                !matches!(hint.get(), crate::state::ClipHint::None)
                    && draft.get().trim().is_empty()
            }
        },
        {
            let state = state.clone();
            move || {
                let hint = state.clip_hint.get_untracked();
                let (label, action) = match hint {
                    crate::state::ClipHint::Text { chars } => {
                        (format!("剪贴板 · 文本 {chars} 字"), "粘贴")
                    }
                    crate::state::ClipHint::Image => ("剪贴板 · 图片".to_string(), "粘贴"),
                    crate::state::ClipHint::None => (String::new(), ""),
                };
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .h(px(26.0))
                    .pl(px(9.0))
                    .pr(px(5.0))
                    .rounded(px(9.0))
                    .bg(p::SUNKEN)
                    .border_1()
                    .border_color(p::BORDER_SOFT)
                    .self_start()
                    .child(text(label).text_size(px(9.5)).text_color(p::MUTED))
                    .child(
                        view()
                            .h(px(18.0))
                            .px(px(8.0))
                            .items_center()
                            .rounded(px(6.0))
                            .bg(p::ACCENT_SOFT)
                            .cursor_pointer()
                            .hover(|s| s.opacity(0.8))
                            .on_press({
                                let state = state.clone();
                                let is_image = matches!(
                                    state.clip_hint.get_untracked(),
                                    crate::state::ClipHint::Image
                                );
                                move || {
                                    if is_image {
                                        let state = state.clone();
                                        spawn(async move {
                                            let Some(img) = clipboard_image().await else {
                                                return;
                                            };
                                            let Ok(resp) = fetch(img.url.clone()).await else {
                                                return;
                                            };
                                            if resp.body.is_empty() {
                                                return;
                                            }
                                            let path = std::env::temp_dir().join(format!(
                                                "drift-clip-{}.png",
                                                chrono::Utc::now().timestamp_millis()
                                            ));
                                            if std::fs::write(&path, &resp.body).is_ok() {
                                                state.add_paths(vec![path]);
                                            }
                                        });
                                    } else {
                                        let state = state.clone();
                                        spawn(async move {
                                            let text = clipboard_string().await;
                                            if !text.is_empty() {
                                                state.update_draft(text);
                                            }
                                        });
                                    }
                                }
                            })
                            .child(
                                text(action)
                                    .text_size(px(9.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(p::ACCENT_INK),
                            ),
                    )
            }
        },
    )
    .into_element()
}
