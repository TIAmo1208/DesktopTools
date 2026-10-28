use tauri::{AppHandle, Emitter, Manager, State};

use crate::app::settings::SettingsState;
use crate::component_runtime::ComponentRuntimeState;

/// 读取一个插件的设置；没有文件或内容损坏时返回空，不改写磁盘。
#[tauri::command]
pub fn get_plugin_settings(state: State<'_, SettingsState>, plugin_id: String) -> Result<Option<serde_json::Value>, String> { crate::app::settings::read_plugin_settings(&state.directory()?, &plugin_id) }

/// 原子保存一个插件的设置，并通知读者重新读取。
#[tauri::command]
pub fn save_plugin_settings(app: AppHandle, state: State<'_, SettingsState>, plugin_id: String, value: serde_json::Value) -> Result<(), String>
{
    crate::app::settings::write_plugin_settings(&state.directory()?, &plugin_id, &value)?;
    if let Err(error) = app.emit(crate::contracts::PLUGIN_SETTINGS_CHANGED_EVENT, serde_json::json!({ "pluginId": &plugin_id }))
    {
        crate::log_warn!("插件设置：变更通知失败，插件={plugin_id}，错误：{error}");
    }
    Ok(())
}

/// 在调用窗口执行插件目录中的同名脚本；重复调用由前端自行避免。
#[tauri::command]
pub fn load_plugin_script(window: tauri::WebviewWindow, plugin_id: String) -> Result<(), String>
{
    let source = crate::app::settings::read_plugin_script(window.state::<ComponentRuntimeState>().plugin_directory(), &plugin_id)?; // 与桌面宿主同一插件根内的脚本。
    window.eval(&source).map_err(|error| {
        crate::log_warn!("插件脚本：执行失败，插件={plugin_id}，错误：{error}");
        format!("插件脚本执行失败：{error}")
    })
}

/// 卸载命令返回实际删除的文件名。清理配置时包含设置文件。
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UninstallPluginResult
{
    pub removed: Vec<String>, // 本次删掉的清单、脚本或设置文件名。
}

/// 先校验文件，再移除全部显示器上的实例，最后删文件。工作区失败时文件仍在。
#[tauri::command]
pub fn uninstall_plugin(app: AppHandle, plugin_id: String, remove_settings: bool) -> Result<UninstallPluginResult, String>
{
    let pending = crate::app::settings::plan_plugin_removal(app.state::<ComponentRuntimeState>().plugin_directory(), &app.state::<SettingsState>().directory()?, &plugin_id, remove_settings)?; // 非法路径在改工作区前拒绝。
    let snapshot = app.state::<crate::workspace::WorkspaceState>().remove_uninstalled_plugin(&plugin_id).map_err(|error| error.message)?; // 不走用户移除门禁，也不进撤销历史。
    let removed = crate::app::settings::commit_plugin_removal(pending)?; // 实例已清掉后再删文件，失败时可重入补删。
    super::workspace::publish_workspace_snapshot(&app, &snapshot).map_err(|error| error.message)?;
    Ok(UninstallPluginResult { removed })
}

/// 在资源管理器中选中该插件的清单；沿用日志目录同一 ShellExecuteW，不另起进程封装。
#[tauri::command]
pub fn open_plugin_directory(app: AppHandle, plugin_id: String) -> Result<(), String>
{
    let manifest = crate::app::settings::plugin_manifest_file(app.state::<ComponentRuntimeState>().plugin_directory(), &plugin_id)?; // 已确认位于插件根内的普通文件。
    reveal_plugin_manifest(&manifest)
}

/// 用资源管理器选中清单。open_path 会按文件关联打开，open_log_directory 不能选中文件。
fn reveal_plugin_manifest(path: &std::path::Path) -> Result<(), String>
{
    use windows::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    use windows::core::{PCWSTR, w};
    let parameter: Vec<u16> = format!("/select,\"{}\"", path.display()).encode_utf16().chain(Some(0)).collect(); // 选中清单，而不是打开目录本身。
    let result = unsafe { ShellExecuteW(None, w!("open"), w!("explorer.exe"), PCWSTR(parameter.as_ptr()), None, SW_SHOWNORMAL) }; // 与打开日志目录相同的 Shell 调用。
    if result.0 as isize <= 32
    {
        return Err(format!("打开插件目录失败，ShellExecuteW：{}", result.0 as isize));
    }
    Ok(())
}
