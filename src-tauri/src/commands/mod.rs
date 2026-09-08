mod application;
mod component;
mod desktop_host;
mod desktop_item;
pub mod settings;
mod virtual_desktop;
mod workspace;

use std::path::PathBuf;

use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};

use crate::component_runtime::ComponentRuntimeState;
use crate::contracts::{
    ComponentCapability, DesktopHostError, FrontendHostReadiness, HitRegion, ShellOperationResult,
    VIRTUAL_DESKTOP_SNAPSHOT_CHANGED_EVENT, WORKSPACE_SNAPSHOT_CHANGED_EVENT, WorkspaceCommand,
    WorkspaceContextMenuTarget, WorkspaceInteractionCommand,
};
use crate::host::DesktopHostRuntime;
use crate::shell::DesktopItemState;
use crate::virtual_desktop::VirtualDesktopState;
use crate::workspace::WorkspaceState;

pub use application::{get_application_snapshot, set_application_status};
pub use component::{get_component_runtime_snapshot, invoke_component_capability};
pub use desktop_host::{
    begin_desktop_host_input_drag, end_desktop_host_input_drag, exit_desktop_tools,
    get_desktop_host_snapshot, publish_desktop_host_readiness, record_desktop_host_input_probe,
    record_desktop_host_renderer_heartbeat, update_desktop_host_hit_map,
};
pub(crate) use desktop_item::refresh_desktop_item_snapshot;
pub use desktop_item::{
    delete_desktop_items, get_desktop_item_snapshot, open_desktop_items, paste_desktop_items,
    rename_desktop_item, set_desktop_item_clipboard, show_workspace_context_menu,
    start_desktop_item_change_monitor, start_desktop_item_drag,
};
pub use virtual_desktop::{
    create_virtual_desktop, delete_virtual_desktop, get_virtual_desktop_snapshot,
    restore_virtual_desktop_archive,
};
pub(crate) use workspace::reconcile_workspace_environment;
pub use workspace::{
    apply_workspace_command, apply_workspace_interaction, get_workspace_snapshot,
    inspect_workspace_backup, list_workspace_auto_snapshots, redo_workspace,
    restore_workspace_backup, undo_workspace, write_workspace_backup,
};

/// 路由独立 Windowed WebView2 transport 到正式应用命令边界。
pub(crate) fn dispatch_native_command(
    app: &AppHandle,
    runtime: &DesktopHostRuntime,
    command: &str,
    arguments: Value,
) -> Result<Value, Value> {
    // 当前本地 transport 命令的统一 JSON 结果。
    let result: Result<Value, Value> = match command {
        "get_windows_accent_color" => app
            .state::<crate::platform::windows::settings::WindowsAccentMonitor>()
            .color()
            .map_err(serialize_error)
            .and_then(serialize_result),
        "get_preferences" => app
            .state::<crate::app::settings::SettingsState>()
            .preferences()
            .map_err(serialize_error)
            .and_then(serialize_result),
        "get_desktop_item_snapshot" => {
            desktop_item::refresh_desktop_item_snapshot(&app.state::<DesktopItemState>())
                .map_err(serialize_error)
                .and_then(|snapshot| {
                    workspace::reconcile_workspace_items(
                        &app.state::<WorkspaceState>(),
                        runtime,
                        &snapshot,
                    )
                    .map_err(serialize_error)?;
                    serialize_result(snapshot)
                })
        }
        "get_component_runtime_snapshot" => component::component_runtime_snapshot(
            &app.state::<ComponentRuntimeState>(),
            &app.state::<WorkspaceState>(),
        )
        .map_err(serialize_error)
        .and_then(serialize_result),
        "get_current_component_host_event" => {
            component::current_component_host_event(&app.state::<ComponentRuntimeState>())
                .map_err(serialize_error)
                .and_then(serialize_result)
        }
        "invoke_component_capability" => {
            command_argument(&arguments, "instanceId", "组件实例标识无效")
                .map_err(serialize_error)
                .and_then(|instance_id: String| {
                    let capability: ComponentCapability =
                        command_argument(&arguments, "capability", "组件能力参数无效")
                            .map_err(serialize_error)?; // 当前实例请求的声明能力。
                    let workspace = app
                        .state::<WorkspaceState>()
                        .snapshot()
                        .map_err(serialize_error)?; // 当前活动虚拟桌面的组件实例集合。
                    app.state::<ComponentRuntimeState>()
                        .invoke_capability(&workspace.components, &instance_id, capability)
                        .map_err(serialize_error)
                })
                .and_then(serialize_result)
        }
        "get_current_desktop_item_snapshot" => {
            desktop_item::read_desktop_item_snapshot(&app.state::<DesktopItemState>())
                .map_err(serialize_error)
                .and_then(serialize_result)
        }
        "open_desktop_items" => command_argument(&arguments, "itemIds", "桌面项目打开参数无效")
            .map_err(serialize_error)
            .and_then(|item_ids: Vec<String>| {
                app.state::<DesktopItemState>()
                    .open(&item_ids)
                    .map_err(serialize_error)
            })
            .and_then(serialize_result),
        "rename_desktop_item" => command_argument(&arguments, "itemId", "桌面项目重命名参数无效")
            .map_err(serialize_error)
            .and_then(|item_id: String| {
                let new_name: String =
                    command_argument(&arguments, "newName", "桌面项目新名称无效")
                        .map_err(serialize_error)?; // 已通过 transport 契约校验的新名称。
                app.state::<DesktopItemState>()
                    .rename(&item_id, new_name)
                    .map_err(serialize_error)
                    .and_then(|result| {
                        desktop_item::clear_layout_redo_for_shell_operation(
                            &app.state::<WorkspaceState>(),
                            &result,
                        )
                        .map_err(serialize_error)?;
                        Ok(result)
                    })
            })
            .and_then(serialize_result),
        "delete_desktop_items" => command_argument(&arguments, "itemIds", "桌面项目删除参数无效")
            .map_err(serialize_error)
            .and_then(|item_ids: Vec<String>| {
                let permanent = arguments
                    .get("permanent")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| {
                        serialize_error(DesktopHostError::invalid_readiness("缺少 permanent"))
                    })?; // 是否明确永久删除。
                app.state::<DesktopItemState>()
                    .delete(&item_ids, permanent)
                    .map_err(serialize_error)
                    .and_then(|result| {
                        desktop_item::clear_layout_redo_for_shell_operation(
                            &app.state::<WorkspaceState>(),
                            &result,
                        )
                        .map_err(serialize_error)?;
                        Ok(result)
                    })
            })
            .and_then(serialize_result),
        "set_desktop_item_clipboard" => {
            command_argument(&arguments, "itemIds", "桌面项目剪贴板参数无效")
                .map_err(serialize_error)
                .and_then(|item_ids: Vec<String>| {
                    let cut = arguments
                        .get("cut")
                        .and_then(Value::as_bool)
                        .ok_or_else(|| {
                            serialize_error(DesktopHostError::invalid_readiness("缺少 cut"))
                        })?; // 是否使用剪切效果。
                    app.state::<DesktopItemState>()
                        .set_clipboard(&item_ids, cut)
                        .map_err(serialize_error)
                })
                .and_then(serialize_result)
        }
        "paste_desktop_items" => app
            .state::<DesktopItemState>()
            .paste()
            .map_err(serialize_error)
            .and_then(|result| {
                desktop_item::clear_layout_redo_for_shell_operation(
                    &app.state::<WorkspaceState>(),
                    &result,
                )
                .map_err(serialize_error)?;
                Ok(result)
            })
            .and_then(serialize_result::<ShellOperationResult>),
        "start_desktop_item_drag" => {
            command_argument(&arguments, "itemIds", "桌面项目拖放参数无效")
                .map_err(serialize_error)
                .and_then(|item_ids: Vec<String>| {
                    app.state::<DesktopItemState>()
                        .drag(&item_ids)
                        .map_err(serialize_error)
                })
                .and_then(serialize_result)
        }
        "show_workspace_context_menu" => {
            command_argument(&arguments, "monitorId", "显示器标识无效")
                .map_err(serialize_error)
                .and_then(|monitor_id: String| {
                    let target: WorkspaceContextMenuTarget =
                        command_argument(&arguments, "target", "工作区菜单目标无效")
                            .map_err(serialize_error)?; // 当前逐屏页面请求的语义菜单目标。
                    let client_x = arguments
                        .get("clientX")
                        .and_then(Value::as_f64)
                        .ok_or_else(|| {
                            serialize_error(DesktopHostError::invalid_readiness("缺少 clientX"))
                        })?;
                    let client_y = arguments
                        .get("clientY")
                        .and_then(Value::as_f64)
                        .ok_or_else(|| {
                            serialize_error(DesktopHostError::invalid_readiness("缺少 clientY"))
                        })?;
                    desktop_item::show_workspace_context_menu_state(
                        &app.state::<DesktopItemState>(),
                        &app.state::<WorkspaceState>(),
                        runtime,
                        &monitor_id,
                        target,
                        client_x,
                        client_y,
                    )
                    .map_err(serialize_error)
                })
                .and_then(serialize_result)
        }
        "get_desktop_host_snapshot" => desktop_host::read_desktop_host_snapshot(runtime)
            .map_err(serialize_error)
            .and_then(serialize_result),
        "exit_desktop_tools" => {
            desktop_host::request_exit(app);
            serialize_result(())
        }
        "record_desktop_host_renderer_heartbeat" => {
            command_argument(&arguments, "monitorId", "显示器标识无效")
                .map_err(serialize_error)
                .and_then(|monitor_id: String| {
                    desktop_host::record_renderer_heartbeat(runtime, &monitor_id)
                        .map_err(serialize_error)
                })
                .and_then(serialize_result)
        }
        "get_workspace_snapshot" | "get_current_workspace_snapshot" => {
            workspace::read_workspace_snapshot(&app.state::<WorkspaceState>())
                .map_err(serialize_error)
                .and_then(serialize_result)
        }
        "write_workspace_backup" => command_argument(&arguments, "path", "工作区备份路径无效")
            .map_err(serialize_error)
            .and_then(|path: String| {
                app.state::<WorkspaceState>()
                    .write_manual_backup(&PathBuf::from(path))
                    .map_err(serialize_error)
            })
            .and_then(serialize_result),
        "inspect_workspace_backup" => command_argument(&arguments, "path", "工作区备份路径无效")
            .map_err(serialize_error)
            .and_then(|path: String| {
                app.state::<WorkspaceState>()
                    .inspect_backup(&PathBuf::from(path))
                    .map_err(serialize_error)
            })
            .and_then(serialize_result),
        "list_workspace_auto_snapshots" => app
            .state::<WorkspaceState>()
            .auto_snapshot_summaries()
            .map_err(serialize_error)
            .and_then(serialize_result),
        "restore_workspace_backup" => command_argument(&arguments, "path", "工作区备份路径无效")
            .map_err(serialize_error)
            .and_then(|path: String| {
                app.state::<VirtualDesktopState>()
                    .restore_workspace_backup(
                        &PathBuf::from(path),
                        &app.state::<WorkspaceState>(),
                        runtime,
                        &app.state::<DesktopItemState>(),
                    )
                    .map_err(serialize_error)
            })
            .and_then(|result| {
                app.state::<DesktopItemState>()
                    .clear_operation_history()
                    .map_err(serialize_error)?;
                if let Ok(snapshot) = app
                    .state::<VirtualDesktopState>()
                    .snapshot(&app.state::<WorkspaceState>())
                {
                    let _ = app.emit(VIRTUAL_DESKTOP_SNAPSHOT_CHANGED_EVENT, snapshot);
                }
                let _ = app.emit(WORKSPACE_SNAPSHOT_CHANGED_EVENT, &result.snapshot);
                serialize_result(result)
            }),
        "undo_workspace" => workspace::apply_workspace_history(
            app,
            &app.state::<WorkspaceState>(),
            &app.state::<DesktopItemState>(),
            runtime,
            false,
        )
        .map_err(serialize_error)
        .and_then(serialize_result),
        "redo_workspace" => workspace::apply_workspace_history(
            app,
            &app.state::<WorkspaceState>(),
            &app.state::<DesktopItemState>(),
            runtime,
            true,
        )
        .map_err(serialize_error)
        .and_then(serialize_result),
        "get_virtual_desktop_snapshot" => app
            .state::<VirtualDesktopState>()
            .snapshot(&app.state::<WorkspaceState>())
            .map_err(serialize_error)
            .and_then(serialize_result),
        "create_virtual_desktop" => app
            .state::<VirtualDesktopState>()
            .create(
                &app.state::<WorkspaceState>(),
                runtime,
                &app.state::<DesktopItemState>(),
            )
            .map_err(serialize_error)
            .and_then(|snapshot| {
                let _ = app.emit(VIRTUAL_DESKTOP_SNAPSHOT_CHANGED_EVENT, &snapshot);
                if let Ok(workspace) = app.state::<WorkspaceState>().snapshot() {
                    let _ = app.emit(WORKSPACE_SNAPSHOT_CHANGED_EVENT, workspace);
                }
                serialize_result(snapshot)
            }),
        "delete_virtual_desktop" => {
            command_argument(&arguments, "desktopId", "删除虚拟桌面标识无效")
                .map_err(serialize_error)
                .and_then(|desktop_id: String| {
                    let fallback_id: String =
                        command_argument(&arguments, "fallbackId", "回退虚拟桌面标识无效")
                            .map_err(serialize_error)?; // 删除后承接窗口的回退桌面身份。
                    app.state::<VirtualDesktopState>()
                        .remove(
                            &desktop_id,
                            &fallback_id,
                            &app.state::<WorkspaceState>(),
                            runtime,
                            &app.state::<DesktopItemState>(),
                        )
                        .map_err(serialize_error)
                })
                .and_then(|snapshot| {
                    let _ = app.emit(VIRTUAL_DESKTOP_SNAPSHOT_CHANGED_EVENT, &snapshot);
                    if let Ok(workspace) = app.state::<WorkspaceState>().snapshot() {
                        let _ = app.emit(WORKSPACE_SNAPSHOT_CHANGED_EVENT, workspace);
                    }
                    serialize_result(snapshot)
                })
        }
        "restore_virtual_desktop_archive" => {
            command_argument(&arguments, "archiveId", "布局归档标识无效")
                .map_err(serialize_error)
                .and_then(|archive_id: String| {
                    app.state::<VirtualDesktopState>()
                        .restore_archive(
                            &archive_id,
                            &app.state::<WorkspaceState>(),
                            runtime,
                            &app.state::<DesktopItemState>(),
                        )
                        .map_err(serialize_error)
                })
                .and_then(|snapshot| {
                    let _ = app.emit(VIRTUAL_DESKTOP_SNAPSHOT_CHANGED_EVENT, &snapshot);
                    if let Ok(workspace) = app.state::<WorkspaceState>().snapshot() {
                        let _ = app.emit(WORKSPACE_SNAPSHOT_CHANGED_EVENT, workspace);
                    }
                    serialize_result(snapshot)
                })
        }
        "apply_workspace_command" => command_argument(&arguments, "command", "工作区命令参数无效")
            .map_err(serialize_error)
            .and_then(|command: WorkspaceCommand| {
                let monitor_id: String =
                    command_argument(&arguments, "monitorId", "显示器标识无效")
                        .map_err(serialize_error)?; // 当前布局事务所属显示器身份。
                let before_revision = app
                    .state::<WorkspaceState>()
                    .snapshot()
                    .map_err(serialize_error)?
                    .revision; // 判断本地布局命令是否创建新统一时间线分支。
                let history_command = matches!(
                    &command,
                    WorkspaceCommand::UndoLayout | WorkspaceCommand::RedoLayout
                );
                let snapshot = workspace::apply_workspace_state_command(
                    &app.state::<WorkspaceState>(),
                    runtime,
                    &monitor_id,
                    command,
                )
                .map_err(serialize_error)?;
                if !history_command && snapshot.revision != before_revision {
                    app.state::<DesktopItemState>()
                        .clear_redo_history()
                        .map_err(serialize_error)?;
                }
                Ok(snapshot)
            })
            .and_then(serialize_result),
        "apply_workspace_interaction" => {
            command_argument(&arguments, "command", "工作区交互参数无效")
                .map_err(serialize_error)
                .and_then(|command: WorkspaceInteractionCommand| {
                    let monitor_id: String =
                        command_argument(&arguments, "monitorId", "显示器标识无效")
                            .map_err(serialize_error)?; // 当前交互事务所属显示器身份。
                    workspace::apply_workspace_state_interaction(
                        &app.state::<WorkspaceState>(),
                        runtime,
                        &monitor_id,
                        command,
                    )
                    .map_err(serialize_error)
                })
                .and_then(serialize_result)
        }
        "publish_desktop_host_readiness" => {
            command_argument(&arguments, "report", "页面准备参数无效")
                .and_then(|report: FrontendHostReadiness| {
                    desktop_host::publish_readiness(runtime, report)
                })
                .map_err(serialize_error)
                .and_then(serialize_result)
        }
        "update_desktop_host_hit_map" => {
            command_argument(&arguments, "monitorId", "显示器标识无效")
                .and_then(|monitor_id: String| {
                    let regions: Vec<HitRegion> =
                        command_argument(&arguments, "hitRegions", "命中图参数无效")?; // 当前逐屏页面发布的完整命中图。
                    desktop_host::replace_hit_map(runtime, &monitor_id, regions)
                })
                .map_err(serialize_error)
                .and_then(serialize_result)
        }
        "record_desktop_host_input_probe" => {
            command_argument(&arguments, "monitorId", "显示器标识无效")
                .and_then(|monitor_id: String| {
                    let item_id: String =
                        command_argument(&arguments, "itemId", "桌面项目标识无效")?; // 当前输入探针命中的项目引用。
                    desktop_host::record_input_probe(runtime, &monitor_id, &item_id)
                })
                .map_err(serialize_error)
                .and_then(serialize_result)
        }
        "begin_desktop_host_input_drag" => {
            command_argument(&arguments, "monitorId", "显示器标识无效")
                .and_then(|monitor_id: String| desktop_host::begin_input_drag(runtime, &monitor_id))
                .map_err(serialize_error)
                .and_then(serialize_result)
        }
        "end_desktop_host_input_drag" => {
            command_argument(&arguments, "monitorId", "显示器标识无效")
                .and_then(|monitor_id: String| desktop_host::end_input_drag(runtime, &monitor_id))
                .map_err(serialize_error)
                .and_then(serialize_result)
        }
        _ => Err(serialize_error(DesktopHostError::invalid_readiness(
            format!("正式宿主命令无效: {command}"),
        ))),
    };
    result
}

/// 从本地 transport 请求中读取并反序列化一个命名参数。
fn command_argument<T: serde::de::DeserializeOwned>(
    arguments: &Value,
    name: &str,
    invalid_message: &str,
) -> Result<T, DesktopHostError> {
    // 等待类型校验的命名参数值。
    let value = arguments
        .get(name)
        .cloned()
        .ok_or_else(|| DesktopHostError::invalid_readiness(format!("缺少 {name}")))?;
    serde_json::from_value(value)
        .map_err(|error| DesktopHostError::invalid_readiness(format!("{invalid_message}: {error}")))
}

/// 将类型化命令结果转换为本地 transport JSON。
fn serialize_result<T: Serialize>(value: T) -> Result<Value, Value> {
    serde_json::to_value(value).map_err(|_| serialize_error(DesktopHostError::state_unavailable()))
}

/// 将类型化命令错误转换为本地 transport JSON。
fn serialize_error(error: impl Serialize) -> Value {
    serde_json::to_value(error)
        .unwrap_or_else(|_| json!({ "code": "desktop-host-error", "message": "桌面宿主命令失败" }))
}

#[cfg(test)]
mod tests {
    use super::command_argument;
    use crate::contracts::FrontendHostReadiness;
    use serde_json::json;

    /// 验证本地 transport 与 Tauri command 使用相同的类型化准备证据。
    #[test]
    fn native_transport_decodes_typed_readiness_arguments() {
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
        let report: FrontendHostReadiness =
            command_argument(&arguments, "report", "页面准备参数无效").unwrap();

        assert_eq!(report.hit_regions.len(), 1);
        assert!(
            command_argument::<FrontendHostReadiness>(&json!({}), "report", "页面准备参数无效")
                .is_err()
        );
    }
}
