//! Drift GUI：UI 全部用 Rust 编写（无 TSX / QuickJS）。
//!
//! `nsh.json` 的 `"rust": "gui"` 指向本 crate，`"root": "app::DriftApp"`。

// 组件宏生成的 trait 方法返回 impl IntoElement；根组件具体化为 AnyElement 属预期。
#![allow(refining_impl_trait)]

pub mod app;
pub mod components;
pub mod icons;
pub mod motion;
pub mod palette;
pub mod state;
pub mod ui;

/// 本 crate 的 UI 集（nsh.json "rust"）：`nsh run` / `nsh package` 挂载根组件。
#[nsh_core::prelude::native_module("drift-gui/native")]
fn native(registry: &mut nsh_core::prelude::Registry) {
    registry.register::<app::DriftApp>();
    registry.assets(nsh_core::prelude::assets!());
}
