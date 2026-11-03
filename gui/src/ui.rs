//! 共享小部件：节标签、图标按钮、卡片基底等（设计稿高频元素）。

use nsh_core::element::FileAsset;
use nsh_core::prelude::*;

use crate::palette as p;

/// 节标签（「附近设备」「核对配对码」…）：10px / w650 / 0.4 字距。
pub fn section_label(label: nsh_core::prelude::SharedString) -> Text {
    text(label)
        .text_size(px(10.0))
        .font_weight(FontWeight::SEMIBOLD)
        .letter_spacing(px(0.4))
        .text_color(p::MUTED)
}

/// 白卡基底。
pub fn card(radius: f32) -> View {
    view()
        .rounded(px(radius))
        .bg(p::SURFACE)
        .border_1()
        .border_color(p::LINE)
        .shadow(p::shadow_card())
}

/// 无边框图标按钮（工具栏用）：hover 微底、active 按下反馈。
pub fn icon_button(
    asset: FileAsset,
    size: f32,
    color: nsh_core::prelude::Rgba,
    label: &'static str,
    on_press: impl Fn() + 'static,
) -> View {
    view()
        .w(px(36.0))
        .h(px(36.0))
        .items_center()
        .justify_center()
        .rounded(px(10.0))
        .cursor_pointer()
        .label(label)
        .role(AccessibilityRole::Button)
        .hover(|s| s.bg(p::IDLE))
        .active(|s| s.bg(p::LINE_SOFT))
        .on_press(on_press)
        .child(crate::icons::icon(asset, size, color))
}

/// 小型含底图标按钮（卡片内 24×24 控制键：暂停/取消/继续）。
pub fn control_button(
    bg: nsh_core::prelude::Rgba,
    border: nsh_core::prelude::Rgba,
    label: &'static str,
    on_press: impl Fn() + 'static,
    child: impl IntoElement + 'static,
) -> View {
    view()
        .w(px(24.0))
        .h(px(24.0))
        .rounded(px(8.0))
        .items_center()
        .justify_center()
        .bg(bg)
        .border_1()
        .border_color(border)
        .cursor_pointer()
        .label(label)
        .role(AccessibilityRole::Button)
        .hover(|s| s.opacity(0.85))
        .active(|s| s.opacity(0.7))
        .on_press(move || on_press())
        .child(child)
}

/// 暂停键（双竖条）。
pub fn pause_icon() -> nsh_core::element::View {
    view()
        .flex_row()
        .gap(px(2.6))
        .items_center()
        .child(view().w(px(2.4)).h(px(9.0)).rounded(px(1.2)).bg(p::MUTED))
        .child(view().w(px(2.4)).h(px(9.0)).rounded(px(1.2)).bg(p::MUTED))
}

/// 继续键（播放三角，用旋转的三角边框近似）。
pub fn play_icon() -> nsh_core::element::View {
    view()
        .w(px(0.0))
        .h(px(0.0))
        .border_l(px(7.0))
        .border_t(px(4.5))
        .border_b(px(4.5))
        .border_l_color(p::SUCCESS_STRONG)
        .border_t_color(nsh_core::prelude::transparent_white())
        .border_b_color(nsh_core::prelude::transparent_white())
}

/// 取消键（×）。
pub fn close_glyph(size: f32, color: nsh_core::prelude::Rgba) -> nsh_core::element::Image {
    crate::icons::icon(crate::icons::CLOSE, size, color)
}

/// 绿勾徽章（头像右下角的小圆点 + 白勾）。
pub fn check_badge(diameter: f32) -> nsh_core::element::View {
    view()
        .w(px(diameter))
        .h(px(diameter))
        .rounded_full()
        .bg(p::BADGE)
        .items_center()
        .justify_center()
        .child(crate::icons::icon(
            crate::icons::CHECK,
            diameter * 0.62,
            p::WHITE,
        ))
}

/// 琥珀增量徽章（头像右下角的小圆点 + 刷新箭头，表示内容已变更可增量补发）。
pub fn delta_badge(diameter: f32) -> nsh_core::element::View {
    view()
        .w(px(diameter))
        .h(px(diameter))
        .rounded_full()
        .bg(p::DELTA_FILL)
        .items_center()
        .justify_center()
        .child(crate::icons::icon(
            crate::icons::REFRESH,
            diameter * 0.62,
            p::WHITE,
        ))
}

/// 设备头像圆。
pub fn avatar(
    diameter: f32,
    bg: nsh_core::prelude::Rgba,
    kind: &str,
    icon_color: nsh_core::prelude::Rgba,
) -> nsh_core::element::View {
    view()
        .w(px(diameter))
        .h(px(diameter))
        .rounded_full()
        .bg(bg)
        .items_center()
        .justify_center()
        .child(crate::icons::icon(crate::icons::device(kind), diameter * 0.47, icon_color))
}

/// 进度环（设备架发送中芯片）：conic 硬边渐变 + 中心挖空。
/// `progress` 0..=1；`hole` 为挖空色（芯片底色）。
pub fn progress_ring(diameter: f32, progress: f32, hole: nsh_core::prelude::Rgba) -> nsh_core::element::View {
    let clamped = progress.clamp(0.0, 1.0);
    view()
        .w(px(diameter))
        .h(px(diameter))
        .rounded_full()
        .bg_conic(deg(-90.0))
        .gradient_from(p::ACCENT)
        .gradient_from_position(percentage(0.0))
        .gradient_via(p::ACCENT)
        .gradient_via_position(percentage(clamped))
        .gradient_to(p::RING_BASE)
        .gradient_to_position(percentage(clamped + 0.001))
        .items_center()
        .justify_center()
        .child(view()
            .w(px(diameter - 8.0))
            .h(px(diameter - 8.0))
            .rounded_full()
            .bg(hole))
}

/// 品牌渐变块（logo / 主按钮 / 发送键共用）。
pub fn accent_gradient(v: impl Styled + 'static) -> impl Styled {
    v.bg_linear(deg(45.0)).gradient_from(p::ACCENT).gradient_to(p::ACCENT_STRONG)
}
