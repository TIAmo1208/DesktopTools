mod app;
mod commands;
pub mod contracts;
mod host;
mod platform;
pub mod recovery;
mod shell;
/// 提供 Rust 后端共享基础工具。
pub mod tools;

use app::ApplicationState;
use commands::{
    get_application_snapshot, get_desktop_host_snapshot, get_desktop_item_snapshot,
    publish_desktop_host_readiness, record_desktop_host_input_probe, set_application_status,
    update_desktop_host_hit_map,
};
use host::DesktopHostRuntime;
use shell::DesktopItemState;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use tauri::{Manager, RunEvent};

/// 创建并运行 Desktop Tools 正式应用。
pub fn run() {
    if let Err(error) = tools::log::init(tools::log::LogConfig::default()) {
        eprintln!("日志系统初始化失败：{error}");
    }
    crate::log_info!("应用：Desktop Tools 正式应用启动");

    // WebView2 Controller 创建前生效的透明默认背景。
    unsafe {
        std::env::set_var("WEBVIEW2_DEFAULT_BACKGROUND_COLOR", "00000000");
    }

    // 应用启动时创建的专用 Shell STA 状态。
    let desktop_items = DesktopItemState::new().unwrap_or_else(|error| {
        crate::log_fatal!("Shell：专用 STA 运行时启动失败，错误：{}", error.message);
        let _ = tools::log::shutdown();
        panic!("FATAL Shell STA 运行时启动失败: {}", error.message);
    });
    // Tauri 应用构建结果。
    let app = tauri::Builder::default()
        .manage(ApplicationState::default())
        .manage(desktop_items)
        .invoke_handler(tauri::generate_handler![
            get_application_snapshot,
            get_desktop_host_snapshot,
            get_desktop_item_snapshot,
            publish_desktop_host_readiness,
            record_desktop_host_input_probe,
            set_application_status,
            update_desktop_host_hit_map,
        ])
        .setup(|app| {
            // 正式单显示器宿主运行时。
            let desktop_host = DesktopHostRuntime::start(app.handle())?;
            app.manage(Arc::clone(&desktop_host));
            DesktopHostRuntime::start_watchdog(app.handle().clone(), desktop_host);
            Ok(())
        })
        .build(tauri::generate_context!());

    // 已完成构建的正式 Tauri 应用。
    let app = match app {
        Ok(app) => app,
        Err(error) => {
            crate::log_fatal!("应用：正式宿主启动失败，错误：{error}");
            let _ = tools::log::shutdown();
            panic!("FATAL Desktop Tools 正式宿主启动失败: {error}");
        }
    };
    // Tauri 退出事件携带的应用请求码。
    let requested_exit_code = Arc::new(AtomicI32::new(0));
    // 退出回调共享的应用请求码。
    let callback_exit_code = Arc::clone(&requested_exit_code);
    // Windows Tauri 事件循环返回的原生退出码。
    let runtime_exit_code = app.run_return(move |app, event| match event {
        RunEvent::ExitRequested { code, .. } => {
            if let Some(code) = code {
                callback_exit_code.store(code, Ordering::Relaxed);
            }
            if let Some(runtime) = app.try_state::<Arc<DesktopHostRuntime>>() {
                runtime.cleanup();
            }
        }
        RunEvent::Exit => {
            if let Some(runtime) = app.try_state::<Arc<DesktopHostRuntime>>() {
                runtime.cleanup();
            }
        }
        _ => {}
    });

    crate::log_info!("应用：Desktop Tools 正式应用退出");
    let _ = tools::log::shutdown();
    // Tauri Windows 运行时当前以零值结束事件循环，应用请求码保留 watchdog 等失败语义。
    let requested_exit_code = requested_exit_code.load(Ordering::Relaxed);
    std::process::exit(if requested_exit_code == 0 {
        runtime_exit_code
    } else {
        requested_exit_code
    });
}
