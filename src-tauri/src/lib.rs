mod app;
mod collect;
mod commands;
mod component_runtime;
pub mod contracts;
mod host;
mod notifications;
mod persistence;
mod platform;
pub mod recovery;
mod shell;
/// 提供 Rust 后端共享基础工具。
pub mod tools;
mod weather;
mod workspace;

use app::ApplicationState;
use commands::get_desktop_item_image;
use commands::sync_message_appearance;
use commands::{apply_workspace_command, apply_workspace_interaction, begin_desktop_host_input_drag, delete_desktop_items, end_desktop_host_input_drag, execute_music_action, exit_desktop_tools, get_application_snapshot, get_component_runtime_snapshot, get_current_music_session, get_desktop_host_snapshot, get_desktop_item_snapshot, get_workspace_snapshot, inspect_workspace_backup, invoke_component_capability, list_workspace_auto_snapshots, open_desktop_items, paste_desktop_items, publish_desktop_host_readiness, publish_message_notification, record_desktop_host_input_probe, record_desktop_host_renderer_heartbeat, redo_workspace, rename_desktop_item, restore_workspace_backup, set_application_status, set_desktop_item_clipboard, show_workspace_context_menu, start_desktop_item_change_monitor, start_desktop_item_drag, undo_workspace, update_desktop_host_hit_map, write_workspace_backup};
use commands::{cancel_desktop_item_drag_preparation, prepare_desktop_item_drag};
use component_runtime::ComponentRuntimeState;
use host::DesktopHostRuntime;
use notifications::ToastManager;
use shell::DesktopItemState;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use tauri::{Listener, Manager, RunEvent};
use workspace::WorkspaceState;

/// 创建并运行 Desktop Tools 正式应用。
pub fn run()
{
    if let Err(error) = tools::log::init(tools::log::LogConfig::default())
        && cfg!(debug_assertions)
    {
        eprintln!("日志系统初始化失败：{error}");
    }
    let (initial_menu_action, _instance_guard) = match app::desktop_menu::prepare_startup()
    {
        Ok(app::desktop_menu::Startup::Exit) => return,
        Ok(app::desktop_menu::Startup::Run(action, guard)) => (action, guard),
        Err(error) =>
        {
            crate::log_error!("桌面菜单：启动命令失败，错误：{error}");
            std::process::exit(1);
        }
    }; // 菜单辅助进程在启动 Shell 和 guardian 前退出。
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
    let component_runtime = ComponentRuntimeState::new(); // 已隔离加载失败的官方组件注册表。
    let default_components = component_runtime.default_instances(); // 首次创建工作区使用的官方默认组件集合。
    // Tauri 应用构建结果。
    let app = tauri::Builder::default()
        .manage(ApplicationState::default())
        .manage(desktop_items)
        .manage(component_runtime)
        .invoke_handler(tauri::generate_handler![
            commands::get_foreground_snapshot,
            commands::settings::get_settings,
            commands::weather::weather_settings,
            commands::weather::subscribe_weather,
            commands::weather::unsubscribe_weather,
            commands::weather::get_weather_forecast,
            commands::weather::search_weather_cities,
            commands::weather::get_weather_city,
            commands::weather::set_weather_city,
            commands::settings::control_settings_window,
            commands::settings::get_preferences,
            commands::settings::get_windows_accent_color,
            commands::settings::save_preferences,
            commands::settings::set_autostart,
            commands::settings::open_logs,
            commands::settings::choose_config_directory,
            commands::settings::choose_folder_portal_directory,
            get_application_snapshot,
            get_desktop_host_snapshot,
            get_desktop_item_snapshot,
            get_desktop_item_image,
            commands::folder_portal::get_folder_portal_snapshot,
            commands::folder_portal::open_folder_portal_path,
            commands::folder_portal::delete_folder_portal_items,
            commands::folder_portal::set_folder_portal_clipboard,
            commands::folder_portal::paste_folder_portal_items,
            commands::folder_portal::show_folder_portal_menu,
            commands::folder_portal::rename_folder_portal_item,
            commands::folder_portal::prepare_folder_portal_drag,
            commands::collect::get_collect_items,
            commands::collect::prepare_collect_export,
            commands::collect::receive_collect_items,
            commands::collect::read_collect_image,
            commands::collect::open_collect_link,
            commands::collect::open_collect_item,
            commands::collect::open_collect_editor,
            commands::collect::create_collect_item,
            commands::collect::save_collect_item,
            commands::collect::cleanup_collect_resources,
            commands::collect::import_collect_resource,
            commands::collect::import_collect_rendered_resource,
            commands::collect::write_collect_resource,
            commands::collect::rename_collect_item,
            commands::collect::set_collect_item_pinned,
            commands::collect::delete_collect_item,
            commands::collect::restore_collect_item,
            commands::create_collect_box,
            get_component_runtime_snapshot,
            invoke_component_capability,
            get_current_music_session,
            execute_music_action,
            open_desktop_items,
            rename_desktop_item,
            delete_desktop_items,
            set_desktop_item_clipboard,
            paste_desktop_items,
            start_desktop_item_drag,
            prepare_desktop_item_drag,
            commands::desktop_item::prepare_collect_drag,
            cancel_desktop_item_drag_preparation,
            show_workspace_context_menu,
            get_workspace_snapshot,
            write_workspace_backup,
            inspect_workspace_backup,
            list_workspace_auto_snapshots,
            restore_workspace_backup,
            undo_workspace,
            redo_workspace,
            apply_workspace_command,
            apply_workspace_interaction,
            begin_desktop_host_input_drag,
            end_desktop_host_input_drag,
            exit_desktop_tools,
            publish_desktop_host_readiness,
            record_desktop_host_input_probe,
            record_desktop_host_renderer_heartbeat,
            set_application_status,
            publish_message_notification,
            sync_message_appearance,
            update_desktop_host_hit_map,
        ])
        .setup(move |app| {
            let config_directory = crate::app::settings::prepare_config_directory(&app.path().app_data_dir()?)?; // 在后台写入线程启动前完成配置迁移。
            let settings_path = config_directory.join("settings.json"); // 当前用户的正式设置文件。
            app.manage(crate::app::settings::SettingsState::load(settings_path)?);
            app.manage(crate::weather::WeatherState::new(&config_directory));
            std::fs::create_dir_all(config_directory.join("collect"))?;
            let _ = crate::platform::windows::directory_watch::start(app.handle().clone(), "collect".to_string(), config_directory.join("collect"));
            app.manage(crate::platform::windows::settings::WindowsAccentMonitor::install(app.handle().clone())?);
            app.manage(ToastManager::install(app.state::<crate::app::settings::SettingsState>().preferences()?)?);
            crate::app::tray::install(app.handle())?;
            // 当前 Windows 用户独占的正式工作区快照路径。
            let workspace_path = config_directory.join("workspace-layout.json");
            // 启动前完成恢复的权威工作区状态。
            let workspace = WorkspaceState::load_with_components(workspace_path, &default_components)?;
            workspace.initialize_grid_preferences(app.state::<crate::app::settings::SettingsState>().preferences()?.desktop_grid)?;
            app.manage(workspace);

            // 正式逐屏宿主运行时。
            crate::platform::windows::install_host_events(app.handle());
            let session_handle = app.handle().clone(); // 锁屏时隐藏按需设置窗口，解锁后不强制打开。
            app.listen("desktop-refresh-policy-changed", move |event| {
                let locked = serde_json::from_str::<serde_json::Value>(event.payload()).ok().and_then(|value| value.get("locked").and_then(serde_json::Value::as_bool)).unwrap_or(false);
                if locked && let Some(window) = session_handle.get_webview_window("main")
                {
                    let _ = window.hide();
                }
            });
            let desktop_host = DesktopHostRuntime::start(app.handle())?;
            app.manage(Arc::clone(&desktop_host));
            // Shell 变化通知由唯一协调线程消费。
            let changes = app.state::<DesktopItemState>().take_change_receiver().map_err(|error| error.message)?;
            start_desktop_item_change_monitor(app.handle().clone(), changes);
            DesktopHostRuntime::start_environment_watchdog(app.handle().clone(), Arc::clone(&app.state::<Arc<DesktopHostRuntime>>()));
            DesktopHostRuntime::start_renderer_watchdog(app.handle().clone(), Arc::clone(&app.state::<Arc<DesktopHostRuntime>>()));
            ComponentRuntimeState::start_host_events(app.handle().clone());
            crate::weather::activity::start(app.handle().clone());
            crate::platform::windows::music::start_monitor(app.handle().clone());
            app::desktop_menu::install(app.handle(), initial_menu_action)?;
            Ok(())
        })
        .build(tauri::generate_context!());

    // 已完成构建的正式 Tauri 应用。
    let app = match app
    {
        Ok(app) => app,
        Err(error) =>
        {
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
    let runtime_exit_code = app.run_return(move |app, event| match event
    {
        RunEvent::ExitRequested { code, api, .. } =>
        {
            if code.is_none()
            {
                api.prevent_exit();
                return;
            } // 最后一个设置窗口关闭不退出托盘应用。
            if let Some(code) = code
            {
                callback_exit_code.store(code, Ordering::Relaxed);
            }
            if let Some(runtime) = app.try_state::<Arc<DesktopHostRuntime>>()
            {
                runtime.cleanup();
            }
            if let Some(toast) = app.try_state::<Arc<ToastManager>>()
            {
                toast.inner().cleanup();
            }
        }
        RunEvent::Exit =>
        {
            app.state::<DesktopItemState>().images.shutdown(); // 退出时停止队列并回收独立图像 STA。
            app::desktop_menu::shutdown();
            if let Some(monitor) = app.try_state::<crate::platform::windows::settings::WindowsAccentMonitor>()
            {
                monitor.shutdown();
            }
            if let Some(runtime) = app.try_state::<Arc<DesktopHostRuntime>>()
            {
                runtime.cleanup();
            }
            if let Some(toast) = app.try_state::<Arc<ToastManager>>()
            {
                toast.inner().cleanup();
            }
        }
        _ =>
        {}
    });

    crate::log_info!("应用：Desktop Tools 正式应用退出");
    let _ = tools::log::shutdown();
    // Tauri Windows 运行时当前以零值结束事件循环，应用请求码保留 watchdog 等失败语义。
    let requested_exit_code = requested_exit_code.load(Ordering::Relaxed);
    std::process::exit(if requested_exit_code == 0 { runtime_exit_code } else { requested_exit_code });
}
