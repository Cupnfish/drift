//! 根组件与窗口装配：桌面 / 移动布局分流、标题栏、快捷键、文件拖放。

use std::rc::Rc;

use nsh_core::prelude::*;

use crate::{
    components::{banners, composer, empty, filters, orb_canvas, pairing, timeline, toolbar},
    palette as p,
    state::AppState,
};

/// 应用根组件（nsh.json "root"）。
#[component]
#[allow(refining_impl_trait)]
pub fn DriftApp() -> AnyElement {
    // 核心只随根组件启动一次
    let state = match AppState::start() {
        Ok(state) => state,
        Err(e) => return launch_error(e).into_element(),
    };

    // 桌面端用系统原生标题栏（Windows caption / macOS 交通灯 / GTK 按钮在内容区外，
    // 不占布局）；移动端平台无桌面窗口，此调用无操作
    nsh_core::element::desktop::set_title_bar(
        nsh_core::desktop::TitleBarStyle::Default,
        None,
    );

    // 快捷键：Ctrl/Cmd+Z 撤销删除；Esc 取消拖放态
    {
        let state = state.clone();
        on_key("mod+z", move |_key| state.undo_delete());
    }
    {
        let state = state.clone();
        on_key("escape", move |_key| state.dragging.set(false));
    }

    let mobile = !is_desktop();
    let root = view()
        .flex_1()
        .bg(p::CANVAS)
        // 整窗文件拖放：任意位置落进输入区
        .on_file_drag_enter({
            let state = state.clone();
            move || state.dragging.set(true)
        })
        .on_file_drag_leave({
            let state = state.clone();
            move || state.dragging.set(false)
        })
        .on_file_drop({
            let state = state.clone();
            move |e: &FileDropEvent| {
                let paths: Vec<_> = e.uris.iter().filter_map(|u| crate::state::uri_to_path(u)).collect();
                if paths.is_empty() {
                    state.toast("无法读取拖入的文件路径", crate::state::ToastKind::Error);
                } else {
                    state.add_paths(paths);
                }
                state.dragging.set(false);
            }
        });

    if mobile {
        root.child(mobile_layout(&state)).into_element()
    } else {
        root.child(desktop_layout(&state)).into_element()
    }
}

/// 桌面布局：工具栏 /（空态 或 筛选+时间线 + 内容触发的设备球体画布）/ 输入区 / 覆盖层。
fn desktop_layout(state: &Rc<AppState>) -> impl IntoElement {
    let has_any = {
        let state = state.clone();
        move || !state.records.get().is_empty() || !state.paired.get().is_empty()
    };
    view()
        .flex_1()
        .child(toolbar::toolbar_desktop(&state))
        .child(
            view()
                .flex_1()
                .relative()
                .justify_end()
                .child(when(
                    has_any.clone(),
                    {
                        let state = state.clone();
                        move || {
                            view()
                                .flex_1()
                                .child(filters::filter_tabs(&state, false))
                                .child(timeline::timeline(&state, false))
                        }
                    },
                ))
                .child(when(
                    {
                        let has_any = has_any.clone();
                        move || !has_any()
                    },
                    {
                        let state = state.clone();
                        move || {
                            scroll().flex_1().child(view().flex_1().child(empty::empty_state(&state, false)))
                        }
                    },
                ))
                .child(orb_canvas::orb_canvas_overlay(&state, false)),
        )
        .child(
            view()
                .px(px(24.0))
                .pb(px(24.0))
                .child(composer::composer(&state, false)),
        )
        .child(banners::banners(&state, false))
        .child(pairing::pairing_layer(&state, false))
}

/// 移动布局：安全区 + 紧凑页头 +（时间线 + 内容触发的设备球体画布）+ 输入区 + 页脚。
fn mobile_layout(state: &Rc<AppState>) -> impl IntoElement {
    let has_any = {
        let state = state.clone();
        move || !state.records.get().is_empty() || !state.paired.get().is_empty()
    };
    let inset = safe_area();
    view()
        .flex_1()
        .pt(px(inset.top.0))
        .pl(px(inset.left.0))
        .pr(px(inset.right.0))
        .child(toolbar::toolbar_mobile(state))
        .child(
            view()
                .flex_1()
                .relative()
                .child(when(
                    has_any.clone(),
                    {
                        let state = state.clone();
                        move || {
                            view()
                                .flex_1()
                                .child(filters::filter_tabs(&state, true))
                                .child(timeline::timeline(&state, true))
                        }
                    },
                ))
                .child(when(
                    {
                        let has_any = has_any.clone();
                        move || !has_any()
                    },
                    {
                        let state = state.clone();
                        move || {
                            scroll().flex_1().child(
                                view()
                                    .flex_1()
                                    .items_center()
                                    .child(empty::empty_state(&state, true)),
                            )
                        }
                    },
                ))
                .child(orb_canvas::orb_canvas_overlay(&state, true)),
        )
        .child(view().h(px(10.0)))
        .child(view().items_center().child(composer::composer(&state, true)))
        .child(
            view()
                .h(px(34.0))
                .items_center()
                .justify_center()
                .pb(px(inset.bottom.0))
                .child(
                    text("清空输入即收起球体，回到会话")
                        .text_size(px(10.0))
                        .text_color(p::FAINT),
                ),
        )
        .child(banners::banners(&state, true))
        .child(pairing::pairing_layer(&state, true))
}

/// 启动失败屏（端口占用 / 数据目录损坏等）。
fn launch_error(message: String) -> impl IntoElement {
    view()
        .flex_1()
        .bg(p::CANVAS)
        .items_center()
        .justify_center()
        .child(
            view()
                .gap(px(10.0))
                .items_center()
                .p(px(32.0))
                .rounded(px(18.0))
                .bg(p::DANGER_SOFT)
                .border_1()
                .border_color(p::DANGER_LINE)
                .child(
                    text("核心启动失败")
                        .text_size(px(16.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(p::DANGER_INK),
                )
                .child(
                    text(message)
                        .text_size(px(12.0))
                        .text_color(p::MUTED),
                ),
        )
}
