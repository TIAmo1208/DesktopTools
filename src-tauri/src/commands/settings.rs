use tauri::{AppHandle, Emitter, Manager, State};

/// 在设置窗口所属线程显示原生目录选择器，取消时不改变当前目录。
#[tauri::command]
pub async fn choose_config_directory(window: tauri::WebviewWindow) -> Result<Option<String>, String>
{
    if window.label() != "main"
    {
        return Err("目录选择仅适用于设置窗口".to_string());
    }
    let owner = window.hwnd().map_err(|error| error.to_string())?.0 as isize; // 设置窗口原生父句柄。
    let (sender, receiver) = std::sync::mpsc::sync_channel(1); // 主线程弹窗返回通道。
    window
        .run_on_main_thread(move || {
            let _ = sender.send(settings::choose_config_directory(owner));
        })
        .map_err(|error| error.to_string())?;
    tauri::async_runtime::spawn_blocking(move || receiver.recv().map_err(|error| error.to_string())).await.map_err(|error| error.to_string())??
}

use crate::{
    app::settings::SettingsState,
    contracts::settings::{ApplicationPreferences, ApplicationTheme, SettingsSnapshot},
    platform::windows::settings,
};

/// 为所有窗口读取当前 Windows 强调色。
#[tauri::command]
pub fn get_windows_accent_color(state: State<'_, settings::WindowsAccentMonitor>) -> Result<String, String> { state.color() }

/// 返回已保存的偏好与 Windows 实际启动状态。
#[tauri::command]
pub fn get_settings(state: State<'_, SettingsState>) -> Result<SettingsSnapshot, String> { Ok(SettingsSnapshot { preferences: state.preferences()?, autostart: settings::autostart_enabled()?, version: env!("CARGO_PKG_VERSION").to_string(), debug_build: cfg!(debug_assertions) }) }

/// 仅控制调用命令的设置窗口，桌面宿主不可使用该入口。
#[tauri::command]
pub fn control_settings_window(window: tauri::WebviewWindow, action: String) -> Result<(), String>
{
    if window.label() != "main"
    {
        return Err("窗口控制仅适用于设置窗口".to_string());
    }
    let result = match action.as_str()
    {
        "drag" => window.start_dragging(),
        "minimize" => window.minimize(),
        "maximize" =>
        {
            if window.is_maximized().map_err(|error| error.to_string())?
            {
                window.unmaximize()
            }
            else
            {
                window.maximize()
            }
        }
        "close" => window.close(),
        _ => return Err("未知窗口操作".to_string()),
    }; // 由 Tauri 原生窗口接口完成操作。
    result.map_err(|error| format!("设置窗口操作失败：{error}"))
}

/// 为设置窗口与逐屏原生宿主返回相同偏好。
#[tauri::command]
pub fn get_preferences(state: State<'_, SettingsState>) -> Result<ApplicationPreferences, String> { state.preferences() }

/// 保存偏好并通知所有窗口；事件失败时保留已成功保存的真实结果。
#[tauri::command]
pub fn save_preferences(app: AppHandle, state: State<'_, SettingsState>, preferences: ApplicationPreferences) -> Result<ApplicationPreferences, String>
{
    let monitors = app.state::<std::sync::Arc<crate::host::DesktopHostRuntime>>().snapshot().map_err(|error| error.message)?.hosts.into_iter().map(|host| host.monitor).collect::<Vec<_>>(); // 原生宿主的当前工作区尺寸。
    let workspace = app.state::<crate::workspace::WorkspaceState>(); // 全部布局共同使用的权威状态。
    let saved = workspace.save_preferences(&state, preferences, &monitors).inspect_err(|error| crate::log_error!("设置：{error}"))?; // 原子保存后的权威偏好。
    if let Some(window) = app.get_webview_window("main")
    {
        let theme = match saved.theme
        {
            ApplicationTheme::System => None,
            ApplicationTheme::Light => Some(tauri::Theme::Light),
            ApplicationTheme::Dark => Some(tauri::Theme::Dark),
        }; // 原生窗口标题栏主题。
        if let Err(error) = window.set_theme(theme)
        {
            crate::log_warn!("设置：标题栏主题更新失败，错误：{error}");
        }
    }
    if let Err(error) = app.emit("preferences-changed", &saved)
    {
        crate::log_warn!("设置：偏好事件发布失败，错误：{error}");
    }
    if let Ok(snapshot) = workspace.snapshot()
        && let Err(error) = app.emit(crate::contracts::WORKSPACE_SNAPSHOT_CHANGED_EVENT, snapshot)
    {
        crate::log_warn!("设置：网格快照事件发布失败，错误：{error}");
    }
    Ok(saved)
}

/// 更新并复核当前 Windows 用户的开机启动条目。
#[tauri::command]
pub fn set_autostart(enabled: bool) -> Result<bool, String> { settings::set_autostart(enabled).inspect_err(|error| crate::log_error!("设置：{error}")) }

/// 打开与当前日志初始化配置一致的目录。
#[tauri::command]
pub fn open_logs() -> Result<(), String>
{
    let directory = crate::tools::log::default_directory()?; // 与启动日志共用当前用户固定目录。
    settings::open_log_directory(&directory).inspect_err(|error| crate::log_error!("设置：{error}"))
}
