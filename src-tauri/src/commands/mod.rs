mod application;
mod component;
mod desktop_host;
mod desktop_item;
pub use desktop_item::{cancel_desktop_item_drag_preparation, prepare_desktop_item_drag};
pub mod music;
pub mod settings;
mod virtual_desktop;
mod workspace;
mod workspace_menu;
pub use music::{execute_music_action, get_current_music_session};

/// 执行 Explorer 注册表菜单回传的动作，复用现有布局事务和事件发布。
pub(crate) fn execute_registered_desktop_menu(app: &tauri::AppHandle, selected: u32) -> Result<(), String>
{
    use tauri::{Emitter, Manager};
    if selected == 0x7807
    {
        return crate::app::tray::set_settings_visible(app, true).map_err(|error| error.to_string());
    }
    let runtime = app.state::<std::sync::Arc<DesktopHostRuntime>>(); // 当前逐屏宿主。
    let hosts = runtime.snapshot().map_err(|error| error.message)?; // 当前显示器环境。
    let mut point = windows::Win32::Foundation::POINT::default(); // 执行动作时的物理指针位置。
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut point) }.map_err(|error| error.to_string())?;
    let monitor = hosts.hosts.iter().map(|host| &host.monitor).find(|monitor| point.x >= monitor.x && point.y >= monitor.y && i64::from(point.x) < i64::from(monitor.x) + i64::from(monitor.width) && i64::from(point.y) < i64::from(monitor.y) + i64::from(monitor.height)).or_else(|| hosts.hosts.first().map(|host| &host.monitor)).ok_or("桌面宿主尚未就绪")?; // 跨屏命令按实际指针选择宿主。
    let workspace = app.state::<WorkspaceState>(); // 权威工作区。
    let items = app.state::<DesktopItemState>(); // Shell 项目状态。
    let snapshot = workspace.snapshot().map_err(|error| error.message)?; // 解析动作时的当前布局。
    let action = workspace_menu::choose_workspace_menu(&snapshot, monitor, WorkspaceContextMenuTarget::Background, (f64::from(point.x) - f64::from(monitor.work_x)) / monitor.scale_factor, (f64::from(point.y) - f64::from(monitor.work_y)) / monitor.scale_factor, |_, _, _, _| Ok(Some(selected))).map_err(|error| error.message)?; // 只解释已选择的动作，不获取或弹出菜单。
    match action
    {
        workspace_menu::WorkspaceMenuAction::None =>
        {}
        workspace_menu::WorkspaceMenuAction::Layout(command) =>
        {
            workspace::execute_apply_workspace_command(app, &workspace, &items, &runtime, monitor.id.clone(), command).map_err(|error| error.message)?;
        }
        workspace_menu::WorkspaceMenuAction::Refresh =>
        {
            let snapshot = items.refresh().map_err(|error| error.message)?; // 重新枚举 Shell 事实。
            let next = reconcile_workspace_environment(&workspace, &runtime, &snapshot, true, &[]).map_err(|error| error.message)?; // 沿用原有协调。
            app.emit(crate::contracts::DESKTOP_ITEM_SNAPSHOT_CHANGED_EVENT, &snapshot).map_err(|error| error.to_string())?;
            app.emit(crate::contracts::WORKSPACE_SNAPSHOT_CHANGED_EVENT, &next).map_err(|error| error.to_string())?;
        }
    }
    crate::log_info!("桌面菜单：注册动词执行完成，命令={selected}，显示器={}", monitor.id);
    Ok(())
}

use std::path::PathBuf;

use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};

use crate::component_runtime::ComponentRuntimeState;
use crate::contracts::{ComponentCapability, DesktopHostError, FrontendHostReadiness, HitRegion, WorkspaceCommand, WorkspaceContextMenuTarget, WorkspaceInteractionCommand};
use crate::host::DesktopHostRuntime;
use crate::shell::DesktopItemState;
use crate::virtual_desktop::VirtualDesktopState;
use crate::workspace::WorkspaceState;

pub use application::{get_application_snapshot, set_application_status};
pub use component::{get_component_runtime_snapshot, invoke_component_capability};
pub use desktop_host::{begin_desktop_host_input_drag, end_desktop_host_input_drag, exit_desktop_tools, get_desktop_host_snapshot, publish_desktop_host_readiness, record_desktop_host_input_probe, record_desktop_host_renderer_heartbeat, update_desktop_host_hit_map};
pub(crate) use desktop_item::refresh_desktop_item_snapshot;
pub use desktop_item::{delete_desktop_items, get_desktop_item_snapshot, open_desktop_items, paste_desktop_items, rename_desktop_item, set_desktop_item_clipboard, show_workspace_context_menu, start_desktop_item_change_monitor, start_desktop_item_drag};
pub use virtual_desktop::{create_virtual_desktop, delete_virtual_desktop, get_virtual_desktop_snapshot, restore_virtual_desktop_archive};
pub(crate) use workspace::reconcile_workspace_environment;
pub use workspace::{apply_workspace_command, apply_workspace_interaction, get_workspace_snapshot, inspect_workspace_backup, list_workspace_auto_snapshots, redo_workspace, restore_workspace_backup, undo_workspace, write_workspace_backup};

/// 路由独立 Windowed WebView2 transport 到正式应用命令边界。
pub(crate) fn dispatch_native_command(app: &AppHandle, runtime: &DesktopHostRuntime, command: &str, arguments: Value) -> Result<Value, Value>
{
    // 当前本地 transport 命令的统一 JSON 结果。
    let result: Result<Value, Value> = match command
    {
        "get_current_music_session" => music::get_current_music_session().map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message))),
        "execute_music_action" =>
        {
            let action: String = command_argument(&arguments, "action", "媒体动作参数无效").map_err(serialize_error)?; // 原生桌面通道与 Tauri 使用相同的动作入口。
            let expected_session: String = command_argument(&arguments, "expectedSession", "媒体会话参数无效").map_err(serialize_error)?; // 用户点击时的会话代次。
            music::execute_music_action(action, expected_session).map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message))).and_then(serialize_result)
        }
        "relay_workspace_drag" =>
        {
            let payload: Value = command_argument(&arguments, "payload", "跨屏拖动参数无效").map_err(serialize_error)?; // 仅允许有界对象消息。
            let maximum_bytes = if payload.get("phase").and_then(Value::as_str) == Some("ole-preview") { 1048576 } else { 65536 }; // 完整命中图比单次移动事件大，仍限制总载荷。
            if !payload.is_object() || payload.to_string().len() > maximum_bytes
            {
                return Err(serialize_error(DesktopHostError::invalid_readiness("跨屏拖动消息无效")));
            }
            app.run_on_main_thread(move || {
                if let Err(error) = crate::platform::windows::relay_workspace_drag(payload)
                {
                    crate::log_warn!("宿主：转发跨屏拖动失败，错误={error}");
                }
            })
            .map_err(|error| serialize_error(DesktopHostError::platform_failed(&error)))?;
            serialize_result(())
        }
        "get_windows_accent_color" => app.state::<crate::platform::windows::settings::WindowsAccentMonitor>().color().map_err(serialize_error).and_then(serialize_result),
        "get_preferences" => app.state::<crate::app::settings::SettingsState>().preferences().map_err(serialize_error).and_then(serialize_result),
        "get_desktop_item_snapshot" => desktop_item::execute_get_desktop_item_snapshot(app, &app.state::<DesktopItemState>(), &app.state::<WorkspaceState>(), runtime).map_err(serialize_error).and_then(serialize_result),
        "get_component_runtime_snapshot" => component::execute_get_component_runtime_snapshot(app, &app.state::<ComponentRuntimeState>(), &app.state::<WorkspaceState>()).map_err(serialize_error).and_then(serialize_result),
        "get_current_component_runtime_snapshot" => component::component_runtime_snapshot(&app.state::<ComponentRuntimeState>(), &app.state::<WorkspaceState>()).map_err(serialize_error).and_then(serialize_result),
        "get_current_component_host_event" => component::current_component_host_event(&app.state::<ComponentRuntimeState>()).map_err(serialize_error).and_then(serialize_result),
        "invoke_component_capability" =>
        {
            let instance_id: String = command_argument(&arguments, "instanceId", "命令参数无效").map_err(serialize_error)?; // 已解码的 instanceId。
            let capability: ComponentCapability = command_argument(&arguments, "capability", "命令参数无效").map_err(serialize_error)?; // 已解码的 capability。
            component::execute_invoke_component_capability(&app.state::<ComponentRuntimeState>(), &app.state::<WorkspaceState>(), instance_id, capability).map_err(serialize_error).and_then(serialize_result)
        }
        "get_current_desktop_item_snapshot" => desktop_item::read_desktop_item_snapshot(&app.state::<DesktopItemState>()).map_err(serialize_error).and_then(serialize_result),
        "open_desktop_items" => command_argument(&arguments, "itemIds", "桌面项目打开参数无效").map_err(serialize_error).and_then(|item_ids: Vec<String>| app.state::<DesktopItemState>().open(&item_ids).map_err(serialize_error)).and_then(serialize_result),
        "rename_desktop_item" =>
        {
            let item_id: String = command_argument(&arguments, "itemId", "命令参数无效").map_err(serialize_error)?; // 已解码的 itemId。
            let new_name: String = command_argument(&arguments, "newName", "命令参数无效").map_err(serialize_error)?; // 已解码的 newName。
            desktop_item::execute_rename_desktop_item(&app.state::<DesktopItemState>(), &app.state::<WorkspaceState>(), item_id, new_name).map_err(serialize_error).and_then(serialize_result)
        }
        "delete_desktop_items" =>
        {
            let item_ids: Vec<String> = command_argument(&arguments, "itemIds", "命令参数无效").map_err(serialize_error)?; // 已解码的 itemIds。
            let permanent: bool = command_argument(&arguments, "permanent", "命令参数无效").map_err(serialize_error)?; // 已解码的 permanent。
            desktop_item::execute_delete_desktop_items(&app.state::<DesktopItemState>(), &app.state::<WorkspaceState>(), item_ids, permanent).map_err(serialize_error).and_then(serialize_result)
        }
        "set_desktop_item_clipboard" =>
        {
            command_argument(&arguments, "itemIds", "桌面项目剪贴板参数无效")
                .map_err(serialize_error)
                .and_then(|item_ids: Vec<String>| {
                    let cut = arguments.get("cut").and_then(Value::as_bool).ok_or_else(|| serialize_error(DesktopHostError::invalid_readiness("缺少 cut")))?; // 是否使用剪切效果。
                    app.state::<DesktopItemState>().set_clipboard(&item_ids, cut).map_err(serialize_error)
                })
                .and_then(serialize_result)
        }
        "paste_desktop_items" => desktop_item::execute_paste_desktop_items(&app.state::<DesktopItemState>(), &app.state::<WorkspaceState>()).map_err(serialize_error).and_then(serialize_result),
        "prepare_desktop_item_drag" => command_argument(&arguments, "itemIds", "桌面项目拖放参数无效").map_err(serialize_error).and_then(|item_ids: Vec<String>| app.state::<DesktopItemState>().prepare_drag(&item_ids).map_err(serialize_error)).and_then(serialize_result),
        "cancel_desktop_item_drag_preparation" => command_argument(&arguments, "token", "拖动凭据无效").map_err(serialize_error).and_then(|token| app.state::<DesktopItemState>().cancel_prepared_drag(token).map_err(serialize_error)).and_then(serialize_result),
        "start_desktop_item_drag" =>
        {
            let token = command_argument(&arguments, "token", "拖动凭据无效").map_err(serialize_error)?; // 单次启动凭据。
            let pointer_offset = command_argument(&arguments, "pointerOffset", "拖动锚点偏移无效").map_err(serialize_error)?; // 源表面 DIP 偏移。
            app.state::<DesktopItemState>().drag(token, pointer_offset).map_err(serialize_error).and_then(serialize_result)
        }
        "show_workspace_context_menu" =>
        {
            let monitor_id: String = command_argument(&arguments, "monitorId", "命令参数无效").map_err(serialize_error)?; // 已解码的 monitorId。
            let target: WorkspaceContextMenuTarget = command_argument(&arguments, "target", "命令参数无效").map_err(serialize_error)?; // 已解码的 target。
            let client_x: f64 = command_argument(&arguments, "clientX", "命令参数无效").map_err(serialize_error)?; // 已解码的 clientX。
            let client_y: f64 = command_argument(&arguments, "clientY", "命令参数无效").map_err(serialize_error)?; // 已解码的 clientY。
            desktop_item::execute_show_workspace_context_menu(app, &app.state::<DesktopItemState>(), &app.state::<WorkspaceState>(), runtime, monitor_id, target, client_x, client_y).map_err(serialize_error).and_then(serialize_result)
        }
        "get_desktop_host_snapshot" => desktop_host::read_desktop_host_snapshot(runtime).map_err(serialize_error).and_then(serialize_result),
        "exit_desktop_tools" =>
        {
            desktop_host::request_exit(app);
            serialize_result(())
        }
        "record_desktop_host_renderer_heartbeat" => command_argument(&arguments, "monitorId", "显示器标识无效").map_err(serialize_error).and_then(|monitor_id: String| desktop_host::record_renderer_heartbeat(runtime, &monitor_id).map_err(serialize_error)).and_then(serialize_result),
        "get_workspace_snapshot" | "get_current_workspace_snapshot" => workspace::read_workspace_snapshot(&app.state::<WorkspaceState>()).map_err(serialize_error).and_then(serialize_result),
        "write_workspace_backup" => command_argument(&arguments, "path", "工作区备份路径无效").map_err(serialize_error).and_then(|path: String| app.state::<WorkspaceState>().write_manual_backup(&PathBuf::from(path)).map_err(serialize_error)).and_then(serialize_result),
        "inspect_workspace_backup" => command_argument(&arguments, "path", "工作区备份路径无效").map_err(serialize_error).and_then(|path: String| app.state::<WorkspaceState>().inspect_backup(&PathBuf::from(path)).map_err(serialize_error)).and_then(serialize_result),
        "list_workspace_auto_snapshots" => app.state::<WorkspaceState>().auto_snapshot_summaries().map_err(serialize_error).and_then(serialize_result),
        "restore_workspace_backup" =>
        {
            let path: String = command_argument(&arguments, "path", "命令参数无效").map_err(serialize_error)?; // 已解码的 path。
            workspace::execute_restore_workspace_backup(app, &app.state::<VirtualDesktopState>(), &app.state::<WorkspaceState>(), &app.state::<DesktopItemState>(), runtime, path).map_err(serialize_error).and_then(serialize_result)
        }
        "undo_workspace" => workspace::apply_workspace_history(app, &app.state::<WorkspaceState>(), &app.state::<DesktopItemState>(), runtime, false).map_err(serialize_error).and_then(serialize_result),
        "redo_workspace" => workspace::apply_workspace_history(app, &app.state::<WorkspaceState>(), &app.state::<DesktopItemState>(), runtime, true).map_err(serialize_error).and_then(serialize_result),
        "get_virtual_desktop_snapshot" => app.state::<VirtualDesktopState>().snapshot(&app.state::<WorkspaceState>()).map_err(serialize_error).and_then(serialize_result),
        "create_virtual_desktop" => virtual_desktop::execute_create_virtual_desktop(app, &app.state::<VirtualDesktopState>(), &app.state::<WorkspaceState>(), runtime, &app.state::<DesktopItemState>()).map_err(serialize_error).and_then(serialize_result),
        "delete_virtual_desktop" =>
        {
            let desktop_id: String = command_argument(&arguments, "desktopId", "命令参数无效").map_err(serialize_error)?; // 已解码的 desktopId。
            let fallback_id: String = command_argument(&arguments, "fallbackId", "命令参数无效").map_err(serialize_error)?; // 已解码的 fallbackId。
            virtual_desktop::execute_delete_virtual_desktop(app, &app.state::<VirtualDesktopState>(), &app.state::<WorkspaceState>(), runtime, &app.state::<DesktopItemState>(), desktop_id, fallback_id).map_err(serialize_error).and_then(serialize_result)
        }
        "restore_virtual_desktop_archive" =>
        {
            let archive_id: String = command_argument(&arguments, "archiveId", "命令参数无效").map_err(serialize_error)?; // 已解码的 archiveId。
            virtual_desktop::execute_restore_virtual_desktop_archive(app, &app.state::<VirtualDesktopState>(), &app.state::<WorkspaceState>(), runtime, &app.state::<DesktopItemState>(), archive_id).map_err(serialize_error).and_then(serialize_result)
        }
        "apply_workspace_command" =>
        {
            let monitor_id: String = command_argument(&arguments, "monitorId", "命令参数无效").map_err(serialize_error)?; // 已解码的 monitorId。
            let command: WorkspaceCommand = command_argument(&arguments, "command", "命令参数无效").map_err(serialize_error)?; // 已解码的 command。
            workspace::execute_apply_workspace_command(app, &app.state::<WorkspaceState>(), &app.state::<DesktopItemState>(), runtime, monitor_id, command).map_err(serialize_error).and_then(serialize_result)
        }
        "apply_workspace_interaction" =>
        {
            let monitor_id: String = command_argument(&arguments, "monitorId", "命令参数无效").map_err(serialize_error)?; // 已解码的 monitorId。
            let command: WorkspaceInteractionCommand = command_argument(&arguments, "command", "命令参数无效").map_err(serialize_error)?; // 已解码的 command。
            workspace::execute_apply_workspace_interaction(app, &app.state::<WorkspaceState>(), runtime, monitor_id, command).map_err(serialize_error).and_then(serialize_result)
        }
        "publish_desktop_host_readiness" =>
        {
            let report: FrontendHostReadiness = command_argument(&arguments, "report", "宿主参数无效").map_err(serialize_error)?; // 已校验参数。
            desktop_host::execute_publish_desktop_host_readiness(app, runtime, report).map_err(serialize_error).and_then(serialize_result)
        }
        "update_desktop_host_hit_map" =>
        {
            let monitor_id: String = command_argument(&arguments, "monitorId", "宿主参数无效").map_err(serialize_error)?; // 已校验参数。
            let hit_regions: Vec<HitRegion> = command_argument(&arguments, "hitRegions", "宿主参数无效").map_err(serialize_error)?; // 已校验参数。
            desktop_host::execute_update_desktop_host_hit_map(app, runtime, monitor_id, hit_regions).map_err(serialize_error).and_then(serialize_result)
        }
        "record_desktop_host_input_probe" =>
        {
            let monitor_id: String = command_argument(&arguments, "monitorId", "宿主参数无效").map_err(serialize_error)?; // 已校验参数。
            let item_id: String = command_argument(&arguments, "itemId", "宿主参数无效").map_err(serialize_error)?; // 已校验参数。
            desktop_host::execute_record_desktop_host_input_probe(app, runtime, monitor_id, item_id).map_err(serialize_error).and_then(serialize_result)
        }
        "begin_desktop_host_input_drag" =>
        {
            let monitor_id: String = command_argument(&arguments, "monitorId", "宿主参数无效").map_err(serialize_error)?; // 已校验参数。
            desktop_host::execute_begin_desktop_host_input_drag(app, runtime, monitor_id).map_err(serialize_error).and_then(serialize_result)
        }
        "end_desktop_host_input_drag" =>
        {
            let monitor_id: String = command_argument(&arguments, "monitorId", "宿主参数无效").map_err(serialize_error)?; // 已校验参数。
            desktop_host::execute_end_desktop_host_input_drag(app, runtime, monitor_id).map_err(serialize_error).and_then(serialize_result)
        }
        _ => Err(serialize_error(DesktopHostError::invalid_readiness(format!("正式宿主命令无效: {command}")))),
    };
    result
}

/// 从本地 transport 请求中读取并反序列化一个命名参数。
fn command_argument<T: serde::de::DeserializeOwned>(arguments: &Value, name: &str, invalid_message: &str) -> Result<T, DesktopHostError>
{
    // 等待类型校验的命名参数值。
    let value = arguments.get(name).cloned().ok_or_else(|| DesktopHostError::invalid_readiness(format!("缺少 {name}")))?;
    serde_json::from_value(value).map_err(|error| DesktopHostError::invalid_readiness(format!("{invalid_message}: {error}")))
}

/// 将类型化命令结果转换为本地 transport JSON。
fn serialize_result<T: Serialize>(value: T) -> Result<Value, Value> { serde_json::to_value(value).map_err(|_| serialize_error(DesktopHostError::state_unavailable())) }

/// 将类型化命令错误转换为本地 transport JSON。
fn serialize_error(error: impl Serialize) -> Value { serde_json::to_value(error).unwrap_or_else(|_| json!({ "code": "desktop-host-error", "message": "桌面宿主命令失败" })) }

#[cfg(test)]
mod tests
{
    use super::command_argument;
    use crate::contracts::FrontendHostReadiness;
    use serde_json::json;

    /// 缺失和类型错误参数不得进入业务执行，错误保留公开编码。
    #[test]
    fn native_arguments_reject_invalid_mutations()
    {
        for value in [json!({}), json!({"permanent": "true"}), json!({"permanent": null})]
        {
            let error = command_argument::<bool>(&value, "permanent", "命令参数无效").unwrap_err(); // 入口校验失败。
            let encoded = super::serialize_error(error); // transport 实际返回的错误对象。
            assert!(encoded["code"].is_string());
            assert!(encoded["message"].as_str().is_some_and(|message| !message.is_empty()));
        }
        assert!(!command_argument::<bool>(&json!({"permanent": false}), "permanent", "命令参数无效").unwrap());
        assert!(command_argument::<crate::contracts::WorkspaceCommand>(&json!({"command": {"type": "unknown"}}), "command", "命令参数无效").is_err());
        assert_eq!(super::serialize_result(()).unwrap(), serde_json::Value::Null);
    }

    /// 验证本地 transport 与 Tauri command 使用相同的类型化准备证据。
    #[test]
    fn native_transport_decodes_typed_readiness_arguments()
    {
        // 完整且字段类型有效的页面准备请求。
        let arguments = json!({
            "report": {
                "monitorId": "\\\\.\\DISPLAY1",
                "viewportWidth": 1920.0,
                "viewportHeight": 1040.0,
                "devicePixelRatio": 1.0,
                "totalSystemImages": 1,
                "loadedSystemImages": 1,
                "hitRegions": [{
                    "id": "desktop-item",
                    "kind": "desktop-item",
                    "left": 20.0,
                    "top": 20.0,
                    "width": 92.0,
                    "height": 135.0,
                    "priority": 1
                }]
            }
        });
        // 解析后的类型化页面准备证据。
        let report: FrontendHostReadiness = command_argument(&arguments, "report", "页面准备参数无效").unwrap();

        assert_eq!(report.hit_regions.len(), 1);
        assert!(command_argument::<FrontendHostReadiness>(&json!({}), "report", "页面准备参数无效").is_err());
    }
}
