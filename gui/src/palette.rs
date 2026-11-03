//! Drift 设计配色（design/src/tokens.json v1.5 的唯一直译）。
//!
//! 设计为纯浅色主题；板内写死但高频复用的色值也收录在第二个区块。

use nsh_core::prelude::Rgba;

// ---- 文字 ----
pub const INK: Rgba = rgb(0x202331); // 主文字 / 标题
pub const INK_SOFT: Rgba = rgb(0x3F4557);
pub const BODY: Rgba = rgb(0x3A4155); // 正文
pub const MUTED: Rgba = rgb(0x5A6276); // 次级
pub const FAINT: Rgba = rgb(0x6A7284); // 辅助 / 说明
pub const DISABLED: Rgba = rgb(0x7C8496);

// ---- 主题色（accent）----
pub const ACCENT: Rgba = rgb(0x6D64F6);
pub const ACCENT_STRONG: Rgba = rgb(0x5046E5);
pub const ACCENT_INK: Rgba = rgb(0x5A50E8);
pub const ACCENT_SOFT: Rgba = rgb(0xEEF0FF);
pub const ACCENT_LINE: Rgba = rgb(0xD6D1FA);
pub const ACCENT_MUTE: Rgba = rgb(0x655BE4);
pub const ACCENT_ICON: Rgba = rgb(0x584DE5);
pub const ACCENT_WASH: Rgba = rgb(0xEDEEFF);
pub const ACCENT_LINE_SOFT: Rgba = rgb(0xC9C4F2);
pub const FILE_INK: Rgba = rgb(0x6459E9); // 文件图标 / 缩略
pub const DROPZONE: Rgba = rgb(0x8578EA); // 拖放虚线（唯一识别描边 ≥3:1）

// ---- 成功（success）----
pub const SUCCESS: Rgba = rgb(0x32866A);
pub const SUCCESS_STRONG: Rgba = rgb(0x2C9A73);
pub const SUCCESS_INK: Rgba = rgb(0x2C7A60);
pub const SUCCESS_SOFT: Rgba = rgb(0xF0F8F5);
pub const SUCCESS_LINE: Rgba = rgb(0xD5EEE2);
pub const SUCCESS_FILL: Rgba = rgb(0x39A27F); // 对勾徽章底
pub const BADGE: Rgba = rgb(0x35A57C); // 绿勾圆点

// ---- 增量 / 待更新（delta：已发设备在输入内容改变后的增量补发态）----
pub const DELTA: Rgba = rgb(0x9A520C);
pub const DELTA_INK: Rgba = rgb(0x7C3F06);
pub const DELTA_SOFT: Rgba = rgb(0xFEF8EC);
pub const DELTA_LINE: Rgba = rgb(0xF3D9B1);
pub const DELTA_FILL: Rgba = rgb(0xB45808);

// ---- 危险（danger）----
pub const DANGER: Rgba = rgb(0xC95863);
pub const DANGER_INK: Rgba = rgb(0x8E3B44);
pub const DANGER_LINE: Rgba = rgb(0xF3D9DB);
pub const DANGER_SOFT: Rgba = rgb(0xFDE8E9);
pub const DANGER_FILL: Rgba = rgb(0xC04F5B);
pub const DANGER_ICON: Rgba = rgb(0xC2636D);
pub const DANGER_WASH: Rgba = rgb(0xFDF2F3);
pub const DANGER_ACTION: Rgba = rgb(0xC96A73); // 记录操作行红色图标

// ---- 线条 / 表面 ----
pub const LINE: Rgba = rgb(0xE8EAF1);
pub const LINE_SOFT: Rgba = rgb(0xEFF0F5);
pub const LINE_STRONG: Rgba = rgb(0xE7E9F0);
pub const BORDER: Rgba = rgb(0xE2E4EC);
pub const BORDER_SOFT: Rgba = rgb(0xEBEDF4);
pub const SURFACE: Rgba = rgb(0xFFFFFF);
pub const CANVAS: Rgba = rgb(0xFBFCFE); // 应用底
pub const SUNKEN: Rgba = rgb(0xF7F8FD);
pub const SUNKEN_ALT: Rgba = rgb(0xF6F8FC);
pub const IDLE: Rgba = rgb(0xF3F4F8); // 闲置设备气泡底
pub const IDLE_ALT: Rgba = rgb(0xF2F3F8);
pub const SHADOW: Rgba = rgb(0x25304A);
pub const BULLET: Rgba = rgb(0xB4B9E8);
pub const FLAG: Rgba = rgb(0xB7BCC9); // 禁用图标
pub const ICON: Rgba = rgb(0x5F6678); // 图标（tokens.json icon）

// ---- 板内高频复用的字面色 ----
pub const TOOLBAR_EDGE: Rgba = rgb(0xE9EBF1); // 工具栏底边
pub const DATE_PILL: Rgba = rgb(0xF1F3F7); // 「今天」胶囊
pub const CHECK_CIRCLE: Rgba = rgb(0xE7F7F0); // 已接收勾圈底
pub const CHECK_GREEN: Rgba = rgb(0x24A674); // 移动端勾
pub const RECEIVED_TEXT: Rgba = rgb(0x3E7F68); // 「已接收」文字
pub const TRACK: Rgba = rgb(0xEBECF3); // 进度轨道
pub const PAUSED_FILL: Rgba = rgb(0xC6C9D6); // 暂停进度填充
pub const PAUSE_BTN: Rgba = rgb(0xF4F5F9); // 暂停键底
pub const RESUME_BTN: Rgba = rgb(0xF1F7F4); // 继续键底
pub const THUMB_BG: Rgba = rgb(0xF0EEFF); // 文件缩略底
pub const MORE_BTN: Rgba = rgb(0xF6F7FB); // 更多键底
pub const SENDING_BG: Rgba = rgb(0xF3F1FF); // 发送中芯片底
pub const SENDING_AVATAR: Rgba = rgb(0xE7E3FF);
pub const RING_BASE: Rgba = rgb(0xCFC9F7); // 进度环底
pub const RING_TRACK: Rgba = rgb(0xEFF0F5); // 球体进度环轨道（drift-main A/F）
pub const ACCENT_CLEAR: Rgba = rgba(0x6D64F600); // 进度环已过弧之后的透明段
pub const FLAG_CLEAR: Rgba = rgba(0xB7BCC900); // 暂停环透明段
pub const SENDING_NAME: Rgba = rgb(0x554BE0); // 发送中设备名
pub const SENDING_NAME_M: Rgba = rgb(0x5549C9); // 移动端
pub const SENDING_CHIP_LINE: Rgba = rgb(0xCEC9F8);
pub const SELECTED_ROW: Rgba = rgb(0xF6F5FF); // 配对选中行
pub const CONFIRM_CIRCLE: Rgba = rgb(0x6157E8); // 配对确认圆
pub const CODE_PILL: Rgba = rgb(0xF1F0FF); // 「对照用，不需要输入」胶囊
pub const CODE_PANEL: Rgba = rgb(0xF7F6FF); // 码面板底
pub const FAIL_ROW: Rgba = rgb(0xFFF7F7); // 失败行底
pub const RETRY_BORDER: Rgba = rgb(0xEDC9CC);
pub const COMPOSER_BG: Rgba = rgb(0xFDFCFF);
pub const COMPOSER_BORDER: Rgba = rgb(0xDCD9E9);
pub const PLUS_KEY: Rgba = rgb(0xF0F1F6); // 加号键底
pub const PLUS_ICON: Rgba = rgb(0x5E6678);
pub const DROPZONE_BG: Rgba = rgb(0xF7F5FF);
pub const DROPZONE_ICON: Rgba = rgb(0x5A4FE2);
pub const DROPZONE_LABEL: Rgba = rgb(0x4E43C8);
pub const DROPZONE_SUB: Rgba = rgb(0x7A739F);
pub const DISABLED_BG: Rgba = rgb(0xFBFBFD); // 禁用输入区
pub const DISABLED_BORDER: Rgba = rgb(0xE6E7EE);
pub const SEND_DISABLED: Rgba = rgb(0xEFEFF4);
pub const IDLE_CHIP_M: Rgba = rgb(0xF7F8FC); // 移动端空闲卡底
pub const IDLE_AVATAR_M: Rgba = rgb(0xEDF0F5);
pub const SCROLL_THUMB: Rgba = rgb(0xD8D6EE); // 移动端滚动条拇指
pub const SCROLL_ARROW: Rgba = rgb(0xF5F6FA); // 设备架滚动箭头底
pub const PALETTE_MISC_A: Rgba = rgb(0xDCDFFC); // 画布装饰圆 1
pub const PALETTE_MISC_B: Rgba = rgb(0xD8ECE8); // 画布装饰圆 2
pub const PATH_DASH: Rgba = rgb(0xD2CCF5); // 空态虚线路径
pub const TRANSIT_CARD: Rgba = rgb(0xDCD8F8); // 在途文件卡描边
pub const TRANSIT_TEXT: Rgba = rgb(0x8B84AE);
pub const CAPSULE_PLUS: Rgba = rgb(0xCDD0DA); // "+9" 折叠胶囊描边
pub const PAIRING_DOT: Rgba = rgb(0xC6C1F4); // 查找中指示点
pub const PAIRING_RING: Rgba = rgb(0xD8D4F8);
pub const SEND_KEY_ICON: Rgba = rgb(0x5E6678); // 与 PLUS_ICON 同源
pub const WHITE: Rgba = rgb(0xFFFFFF);

// ---- 渐变 ----
/// 品牌渐变（logo / 主按钮 / 发送键）：45°
pub const GRAD_ACCENT: (Rgba, Rgba) = (ACCENT, ACCENT_STRONG);
/// 桌面发送气泡
pub const GRAD_BUBBLE_DESKTOP: (Rgba, Rgba) = (rgb(0x6B61F3), rgb(0x5147E2));
/// 移动发送气泡
pub const GRAD_BUBBLE_MOBILE: (Rgba, Rgba) = (rgb(0x6B61F3), rgb(0x554BE6));
/// 桌面画布背景（竖直）
pub const GRAD_CANVAS: (Rgba, Rgba) = (rgb(0xF3F5FA), rgb(0xE5EAF2));

// ---- 阴影（BoxShadow 列表）----
use nsh_core::prelude::{BoxShadow, px, rgb, rgba};

/// 卡片：dy2/blur5/#25304A@5.5%
pub fn shadow_card() -> Vec<BoxShadow> {
    vec![BoxShadow {
        offset_y: px(2.0),
        blur: px(5.0),
        color: rgba(0x25304A0E).into(),
        ..BoxShadow::default()
    }]
}

/// 面板（设备架）：dy8/blur14/#2A2F45@10%
pub fn shadow_panel() -> Vec<BoxShadow> {
    vec![BoxShadow {
        offset_y: px(8.0),
        blur: px(14.0),
        color: rgba(0x2A2F451A).into(),
        ..BoxShadow::default()
    }]
}

/// 移动卡片：dy3/blur7/@7%
pub fn shadow_card_mobile() -> Vec<BoxShadow> {
    vec![BoxShadow {
        offset_y: px(3.0),
        blur: px(7.0),
        color: rgba(0x25304A12).into(),
        ..BoxShadow::default()
    }]
}

/// 移动浮层：dy6/blur12/@12%
pub fn shadow_float_mobile() -> Vec<BoxShadow> {
    vec![BoxShadow {
        offset_y: px(6.0),
        blur: px(12.0),
        color: rgba(0x25304A1F).into(),
        ..BoxShadow::default()
    }]
}

/// 输入区 glow：accent@10% + dy3/blur6/@5%
pub fn shadow_composer_glow() -> Vec<BoxShadow> {
    vec![
        BoxShadow {
            offset_y: px(0.0),
            blur: px(10.0),
            color: rgba(0x6D64F61A).into(),
            ..BoxShadow::default()
        },
        BoxShadow {
            offset_y: px(3.0),
            blur: px(6.0),
            color: rgba(0x25304A0D).into(),
            ..BoxShadow::default()
        },
    ]
}

/// 规范卡：dy4/blur10/@6%
pub fn shadow_spec_card() -> Vec<BoxShadow> {
    vec![BoxShadow {
        offset_y: px(4.0),
        blur: px(10.0),
        color: rgba(0x25304A0F).into(),
        ..BoxShadow::default()
    }]
}

/// 设备球体：dy6/blur11/#25304A@11%（drift-main orbShadow）
pub fn shadow_orb() -> Vec<BoxShadow> {
    vec![BoxShadow {
        offset_y: px(6.0),
        blur: px(11.0),
        color: rgba(0x25304A1C).into(),
        ..BoxShadow::default()
    }]
}

/// 传输中球体的 accent 光晕（drift-main A 球 glow）
pub fn shadow_orb_glow() -> Vec<BoxShadow> {
    vec![
        BoxShadow {
            offset_y: px(0.0),
            blur: px(16.0),
            color: rgba(0x6D64F626).into(),
            ..BoxShadow::default()
        },
        BoxShadow {
            offset_y: px(6.0),
            blur: px(11.0),
            color: rgba(0x25304A1C).into(),
            ..BoxShadow::default()
        },
    ]
}
