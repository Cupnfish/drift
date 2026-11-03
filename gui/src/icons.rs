//! 设计图标集（由 design/src/icons.svg 经 tools/split_icons.py 生成）。
//!
//! 所有图标 24×24 描边画法；`icon()` 以 tint 重着色。
//! `cursor` 例外：自带白描边的实心指针（空态插画用），不要上 tint。

use nsh_core::element::{FileAsset, Image};
use nsh_core::prelude::{Rgba, Styled, asset, image, px};

pub const ARROW: FileAsset = asset!("icons/arrow.svg");
pub const CHECK: FileAsset = asset!("icons/check.svg");
pub const CLOCK: FileAsset = asset!("icons/clock.svg");
pub const CLOSE: FileAsset = asset!("icons/close.svg");
pub const COPY: FileAsset = asset!("icons/copy.svg");
pub const CURSOR: FileAsset = asset!("icons/cursor.svg");
pub const DOWNLOAD: FileAsset = asset!("icons/download.svg");
pub const EYE: FileAsset = asset!("icons/eye.svg");
pub const FILE: FileAsset = asset!("icons/file.svg");
pub const FOLDER: FileAsset = asset!("icons/folder.svg");
pub const LAPTOP: FileAsset = asset!("icons/laptop.svg");
pub const LINK: FileAsset = asset!("icons/link.svg");
pub const MONITOR: FileAsset = asset!("icons/monitor.svg");
pub const MORE: FileAsset = asset!("icons/more.svg");
pub const PHONE: FileAsset = asset!("icons/phone.svg");
pub const PLUS: FileAsset = asset!("icons/plus.svg");
pub const RADAR: FileAsset = asset!("icons/radar.svg");
pub const REFRESH: FileAsset = asset!("icons/refresh.svg");
pub const SEARCH: FileAsset = asset!("icons/search.svg");
pub const SEND: FileAsset = asset!("icons/send.svg");
pub const SHIELD: FileAsset = asset!("icons/shield.svg");
pub const TABLET: FileAsset = asset!("icons/tablet.svg");
pub const TRASH: FileAsset = asset!("icons/trash.svg");
pub const TRAY: FileAsset = asset!("icons/tray.svg");
pub const UNLINK: FileAsset = asset!("icons/unlink.svg");
pub const WARN: FileAsset = asset!("icons/warn.svg");

/// tint 着色的图标。
pub fn icon(asset: FileAsset, size: f32, color: Rgba) -> Image {
    image(asset.uri).w(px(size)).h(px(size)).tint_color(color)
}

/// 保持原色的图标（如 cursor）。
pub fn icon_plain(asset: FileAsset, size: f32) -> Image {
    image(asset.uri).w(px(size)).h(px(size))
}

/// 设备类型 → 图标。
pub fn device(kind: &str) -> FileAsset {
    match kind {
        "desktop" => MONITOR,
        "laptop" => LAPTOP,
        "phone" => PHONE,
        "tablet" => TABLET,
        "watch" => CLOCK,
        "tv" => MONITOR,
        _ => MONITOR,
    }
}
