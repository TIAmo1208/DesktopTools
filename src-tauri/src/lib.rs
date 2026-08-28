mod app;
mod commands;
pub mod contracts;
mod platform;
mod shell;
/// 提供 Rust 后端共享基础工具。
pub mod tools;

use app::ApplicationState;
use commands::{get_application_snapshot, get_desktop_item_snapshot, set_application_status};
use shell::DesktopItemState;

/// 创建并运行 Desktop Tools 正式应用。
pub fn run() {
    if let Err(error) = tools::log::init(tools::log::LogConfig::default()) {
        eprintln!("日志系统初始化失败：{error}");
    }
    crate::log_info!("应用：Desktop Tools 正式应用启动");

    // 应用启动时创建的专用 Shell STA 状态。
    let desktop_items = DesktopItemState::new().unwrap_or_else(|error| {
        crate::log_fatal!("Shell：专用 STA 运行时启动失败，错误：{}", error.message);
        let _ = tools::log::shutdown();
        panic!("FATAL Shell STA 运行时启动失败: {}", error.message);
    });
    // Tauri 应用运行结果。
    let result = tauri::Builder::default()
        .manage(ApplicationState::default())
        .manage(desktop_items)
        .invoke_handler(tauri::generate_handler![
            get_application_snapshot,
            get_desktop_item_snapshot,
            set_application_status
        ])
        .run(tauri::generate_context!());

    match &result {
        Ok(()) => crate::log_info!("应用：Desktop Tools 正式应用退出"),
        Err(error) => crate::log_fatal!("应用：Desktop Tools 正式应用运行失败，错误：{error}"),
    }
    let _ = tools::log::shutdown();
    result.expect("FATAL Desktop Tools 正式应用运行失败");
}
