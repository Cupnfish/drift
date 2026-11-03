//! 在本机原生后端上直接运行 Drift GUI（`cargo run`）。

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(target_os = "linux")]
use nsh_platform_linux as platform;
#[cfg(target_os = "macos")]
use nsh_platform_macos as platform;
#[cfg(target_os = "windows")]
use nsh_platform_windows as platform;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn main() -> impl std::process::Termination {
    env_logger::init();
    let gui = drift_gui::registry();
    let assets = asset_directory(&gui).expect("复制注册资产到可执行文件旁");
    platform::set_ui_sets(vec![nsh_base::registry(), gui]);
    let app_id = "dev.drift.app";
    platform::run_app(
        platform::AppSource::Rust(platform::Root::of::<drift_gui::app::DriftApp>().app_id(app_id)),
        platform::AppOptions {
            title: "Drift".to_owned(),
            app_id: app_id.to_owned(),
            size: (1080, 800),
            appearance: platform::Appearance::Light,
            asset_base: Some(platform::AssetBase::Directory(assets)),
            ..Default::default()
        },
    )
}

/// 注册资产的 URI 布局与开发宿主一致：拷到可执行文件旁的目录供加载。
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn asset_directory(registry: &nsh_core::element::Registry) -> std::io::Result<std::path::PathBuf> {
    let executable = std::env::current_exe()?;
    let base = executable
        .parent()
        .expect("the executable has a directory")
        .join("drift-gui-assets");
    for file in registry.asset_files() {
        let destination = base.join(file.path());
        std::fs::create_dir_all(
            destination.parent().expect("an asset has a parent directory"),
        )?;
        std::fs::copy(file.source, destination)?;
    }
    Ok(base)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("在 macOS / Windows / Linux 上运行本二进制；iOS / Android 请用 `nsh run --target ios|android`。");
