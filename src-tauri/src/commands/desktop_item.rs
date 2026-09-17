use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, State};

use crate::contracts::{DESKTOP_ITEM_SNAPSHOT_CHANGED_EVENT, DesktopItemError, DesktopItemSnapshot, ShellOperationResult, WORKSPACE_SNAPSHOT_CHANGED_EVENT, WorkspaceContextMenuTarget, WorkspaceSnapshot};
use crate::host::DesktopHostRuntime;
use crate::platform::windows::NativeMenuTarget;
use crate::platform::windows::ShellChangeEvent;
use crate::shell::DesktopItemState;
use crate::workspace::WorkspaceState;

use super::workspace::{reconcile_workspace_environment, reconcile_workspace_items};

/// 枚举真实 Shell 桌面项目并发布新的不可变快照。
#[tauri::command]
pub async fn get_desktop_item_snapshot(app: AppHandle, state: State<'_, DesktopItemState>, workspace: State<'_, WorkspaceState>, runtime: State<'_, Arc<DesktopHostRuntime>>) -> Result<DesktopItemSnapshot, DesktopItemError> { execute_get_desktop_item_snapshot(&app, &state, &workspace, &runtime) }

/// 两种 transport 共用的命令执行与事件发布。
pub(crate) fn execute_get_desktop_item_snapshot(app: &AppHandle, state: &DesktopItemState, workspace: &WorkspaceState, runtime: &DesktopHostRuntime) -> Result<DesktopItemSnapshot, DesktopItemError>
{
    // 已发布到内存状态的桌面项目快照。
    let snapshot = refresh_desktop_item_snapshot(state)?;
    app.emit(DESKTOP_ITEM_SNAPSHOT_CHANGED_EVENT, &snapshot).map_err(|error| {
        crate::log_error!("Shell：桌面项目快照事件发布失败，错误：{error}");
        DesktopItemError::event_publish_failed(&error)
    })?;
    // 已按正式宿主网格协调的工作区快照。
    let workspace_snapshot = reconcile_workspace_items(workspace, runtime, &snapshot).map_err(|error| DesktopItemError::workspace_reconciliation_failed(&error))?;
    app.emit(WORKSPACE_SNAPSHOT_CHANGED_EVENT, &workspace_snapshot).map_err(|error| DesktopItemError::event_publish_failed(&error))?;

    Ok(snapshot)
}

/// 使用 Shell 默认动词打开一个或多个桌面项目。
#[tauri::command]
pub fn open_desktop_items(state: State<'_, DesktopItemState>, item_ids: Vec<String>) -> Result<(), DesktopItemError> { state.open(&item_ids) }

/// 使用 IFileOperation 重命名单个桌面项目。
#[tauri::command]
pub fn rename_desktop_item(state: State<'_, DesktopItemState>, workspace: State<'_, WorkspaceState>, item_id: String, new_name: String) -> Result<ShellOperationResult, DesktopItemError> { execute_rename_desktop_item(&state, &workspace, item_id, new_name) }

/// 两种 transport 共用的命令执行与事件发布。
pub(crate) fn execute_rename_desktop_item(state: &DesktopItemState, workspace: &WorkspaceState, item_id: String, new_name: String) -> Result<ShellOperationResult, DesktopItemError>
{
    let result = state.rename(&item_id, new_name)?; // 已记录统一时间线的实际 Shell 结果。
    clear_layout_redo_for_shell_operation(workspace, &result)?;
    Ok(result)
}

/// 将完整选择集合移入回收站或永久删除。
#[tauri::command]
pub fn delete_desktop_items(state: State<'_, DesktopItemState>, workspace: State<'_, WorkspaceState>, item_ids: Vec<String>, permanent: bool) -> Result<ShellOperationResult, DesktopItemError> { execute_delete_desktop_items(&state, &workspace, item_ids, permanent) }

/// 两种 transport 共用的命令执行与事件发布。
pub(crate) fn execute_delete_desktop_items(state: &DesktopItemState, workspace: &WorkspaceState, item_ids: Vec<String>, permanent: bool) -> Result<ShellOperationResult, DesktopItemError>
{
    let result = state.delete(&item_ids, permanent)?; // 已记录统一时间线的实际 Shell 结果。
    clear_layout_redo_for_shell_operation(workspace, &result)?;
    Ok(result)
}

/// 将完整选择集合复制或剪切到系统 Shell 剪贴板。
#[tauri::command]
pub fn set_desktop_item_clipboard(state: State<'_, DesktopItemState>, item_ids: Vec<String>, cut: bool) -> Result<ShellOperationResult, DesktopItemError> { state.set_clipboard(&item_ids, cut) }

/// 将系统 Shell 剪贴板内容粘贴到桌面命名空间。
#[tauri::command]
pub fn paste_desktop_items(state: State<'_, DesktopItemState>, workspace: State<'_, WorkspaceState>) -> Result<ShellOperationResult, DesktopItemError> { execute_paste_desktop_items(&state, &workspace) }

/// 两种 transport 共用的命令执行与事件发布。
pub(crate) fn execute_paste_desktop_items(state: &DesktopItemState, workspace: &WorkspaceState) -> Result<ShellOperationResult, DesktopItemError>
{
    let result = state.paste()?; // 已记录统一时间线的实际 Shell 粘贴结果。
    clear_layout_redo_for_shell_operation(workspace, &result)?;
    Ok(result)
}

/// 使用完整选择集合启动系统 OLE 拖放。
#[tauri::command]
pub async fn start_desktop_item_drag(app: AppHandle, state: State<'_, DesktopItemState>, token: u64, pointer_offset: crate::contracts::WorkspacePoint) -> Result<ShellOperationResult, DesktopItemError> { execute_start_desktop_item_drag(&app, &state, token, pointer_offset) }

/// 两个 transport 均在宿主 UI STA 运行 OLE，准备数据通过 COM 引用封送。
pub(crate) fn execute_start_desktop_item_drag(app: &AppHandle, state: &DesktopItemState, token: u64, pointer_offset: crate::contracts::WorkspacePoint) -> Result<ShellOperationResult, DesktopItemError>
{
    let prepared = state.take_prepared_drag(token)?; // 准备凭据已消费，Shell STA 不等待 UI 拖放。
    let (reply, response) = std::sync::mpsc::channel(); // transport 工作线程等待原生结果，不阻塞 UI 消息循环。
    app.run_on_main_thread(move || {
        let result = prepared.data.resolve().map_err(|error| format!("解析源拖放数据失败：{error}")).and_then(|data| crate::platform::windows::start_desktop_item_drag(data, &prepared.references, pointer_offset, prepared.portal_source)); // 仅在当前 UI apartment 使用代理。
        let _ = reply.send(result);
    })
    .map_err(|error| DesktopItemError::shell_operation_failed(&error))?;
    let mut result = response.recv().map_err(|error| DesktopItemError::shell_operation_failed(&error))?.map_err(|error| DesktopItemError::shell_operation_failed(&error))?; // 不推测文件成功或取消结果。
    result.completed_at_unix_micros = crate::tools::time::next_timeline_timestamp();
    crate::log_info!("拖放：宿主 OLE 交接结束，凭据={token}，效果={}，取消={}", result.effect.as_deref().unwrap_or("无"), u8::from(result.aborted));
    Ok(result)
}

/// 准备完整选择集合，返回空值表示沿用内部布局拖动。
#[tauri::command]
pub async fn prepare_desktop_item_drag(state: State<'_, DesktopItemState>, item_ids: Vec<String>) -> Result<Option<crate::contracts::ShellDragPreparation>, DesktopItemError> { state.prepare_drag(&item_ids) }

/// 释放点击或取消留下的准备数据。
#[tauri::command]
pub fn cancel_desktop_item_drag_preparation(state: State<'_, DesktopItemState>, token: u64) -> Result<(), DesktopItemError> { state.cancel_prepared_drag(token) }

/// 在正确语义目标显示原生菜单并提交选中的产品命令。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn show_workspace_context_menu(app: AppHandle, state: State<'_, DesktopItemState>, workspace: State<'_, WorkspaceState>, runtime: State<'_, Arc<DesktopHostRuntime>>, monitor_id: String, target: WorkspaceContextMenuTarget, client_x: f64, client_y: f64) -> Result<WorkspaceSnapshot, DesktopItemError> { execute_show_workspace_context_menu(&app, &state, &workspace, &runtime, monitor_id, target, client_x, client_y) }

/// 两种 transport 共用的命令执行与事件发布。
#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_show_workspace_context_menu(app: &AppHandle, state: &DesktopItemState, workspace: &WorkspaceState, runtime: &DesktopHostRuntime, monitor_id: String, target: WorkspaceContextMenuTarget, client_x: f64, client_y: f64) -> Result<WorkspaceSnapshot, DesktopItemError>
{
    // 已应用 Shell 或产品菜单动作的权威工作区快照。
    let snapshot = show_workspace_context_menu_state(state, workspace, runtime, &monitor_id, target, client_x, client_y)?;
    app.emit(WORKSPACE_SNAPSHOT_CHANGED_EVENT, &snapshot).map_err(|error| DesktopItemError::event_publish_failed(&error))?;
    Ok(snapshot)
}

/// 为 Tauri 与本地 transport 共享原生菜单领域协调。
pub(crate) fn show_workspace_context_menu_state(state: &DesktopItemState, workspace: &WorkspaceState, runtime: &DesktopHostRuntime, monitor_id: &str, target: WorkspaceContextMenuTarget, client_x: f64, client_y: f64) -> Result<WorkspaceSnapshot, DesktopItemError>
{
    crate::log_info!("菜单：收到请求，显示器={monitor_id}，目标={target:?}，位置=({client_x},{client_y})");
    let host_snapshot = runtime.snapshot().map_err(|error| DesktopItemError::shell_operation_failed(&error.message))?; // 当前宿主环境。
    let monitor = host_snapshot.hosts.iter().find(|host| host.monitor.id == monitor_id).map(|host| &host.monitor).ok_or_else(|| DesktopItemError::shell_operation_failed(&"当前显示器宿主不存在"))?; // 目标显示器。
    let snapshot = workspace.snapshot().map_err(|error| DesktopItemError::shell_operation_failed(&error))?; // 菜单打开前的事实。
    let action = super::workspace_menu::choose_workspace_menu(&snapshot, monitor, target, client_x, client_y, |target, x, y, commands| {
        let target = match target
        {
            NativeMenuTarget::Items(ids) => NativeMenuTarget::Items(state.selection_references(&ids)?),
            target => target,
        }; // 进入 STA 前校验 Shell 引用。
        state.show_menu(target, x, y, commands)
    })?; // 已解释的产品选择。
    let command = match action
    {
        super::workspace_menu::WorkspaceMenuAction::None => return Ok(snapshot),
        super::workspace_menu::WorkspaceMenuAction::Refresh =>
        {
            let item_snapshot = state.refresh()?; // 刷新后的 Shell 事实。
            return reconcile_workspace_environment(workspace, runtime, &item_snapshot, true, &[]).map_err(|error| DesktopItemError::workspace_reconciliation_failed(&error));
        }
        super::workspace_menu::WorkspaceMenuAction::Layout(command) => command,
    }; // 进入现有事务路径的命令。
    let before_revision = snapshot.revision; // 判断产品菜单动作是否创建新统一时间线分支的修订号。
    let next = workspace.apply_on_monitor(Some(monitor_id), command).map_err(|error| DesktopItemError::shell_operation_failed(&error))?;
    if next.revision != before_revision
    {
        state.clear_redo_history()?;
    }
    Ok(next)
}

/// 枚举 transport 共享的桌面项目快照。
pub(crate) fn refresh_desktop_item_snapshot(state: &DesktopItemState) -> Result<DesktopItemSnapshot, DesktopItemError> { state.refresh() }

/// 返回本地事件轮询使用的当前桌面项目快照。
pub(crate) fn read_desktop_item_snapshot(state: &DesktopItemState) -> Result<DesktopItemSnapshot, DesktopItemError> { state.snapshot() }

/// 启动 250 ms 合并窗口和拖动结束门槛后的 Shell 变化协调。
pub fn start_desktop_item_change_monitor(app: AppHandle, changes: Receiver<ShellChangeEvent>)
{
    thread::spawn(move || {
        while let Ok(first_change) = changes.recv()
        {
            // 当前合并窗口内收集的精确重命名事实。
            let mut renames = Vec::new();
            collect_shell_change(first_change, &mut renames);
            loop
            {
                match changes.recv_timeout(Duration::from_millis(250))
                {
                    Ok(change) => collect_shell_change(change, &mut renames),
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            // 拖动期间到达的变化等待布局事务完成或取消。
            while app.state::<Arc<DesktopHostRuntime>>().is_input_drag_active()
            {
                thread::sleep(Duration::from_millis(25));
                for change in changes.try_iter()
                {
                    collect_shell_change(change, &mut renames);
                }
            }

            // Shell 最终事实重新枚举后的桌面项目快照。
            let item_snapshot = match app.state::<DesktopItemState>().refresh()
            {
                Ok(snapshot) => snapshot,
                Err(error) =>
                {
                    crate::log_error!("Shell：变化后桌面项目协调失败，错误：{}", error.message);
                    continue;
                }
            };
            if let Err(error) = app.state::<DesktopItemState>().reconcile_operation_history(&item_snapshot, &renames)
            {
                crate::log_error!("Shell：操作历史对账失败，错误：{}", error.message);
            }
            // 精确重命名和最终枚举共同产生的权威工作区快照。
            let workspace_snapshot = match reconcile_workspace_environment(&app.state::<WorkspaceState>(), &app.state::<Arc<DesktopHostRuntime>>(), &item_snapshot, true, &renames)
            {
                Ok(snapshot) => snapshot,
                Err(error) =>
                {
                    crate::log_error!("Shell：变化协调更新工作区失败，错误：{}", error.message);
                    continue;
                }
            };
            let _ = app.emit(DESKTOP_ITEM_SNAPSHOT_CHANGED_EVENT, &item_snapshot);
            let _ = app.emit(WORKSPACE_SNAPSHOT_CHANGED_EVENT, &workspace_snapshot);
            crate::log_debug!("Shell：桌面变化已协调，重命名={}，项目={}，布局修订={}", renames.len(), item_snapshot.items.len(), workspace_snapshot.revision);
        }
    });
}

/// 新 Shell 文件操作提交后清除当前布局重做分支。
pub(crate) fn clear_layout_redo_for_shell_operation(workspace: &WorkspaceState, result: &ShellOperationResult) -> Result<(), DesktopItemError>
{
    if result.operation_id == 0
    {
        return Ok(());
    }
    workspace.clear_redo_history().map_err(|error| DesktopItemError::shell_operation_failed(&error))
}

/// 将一个 Shell 变化事件归并到精确重命名事实集合。
fn collect_shell_change(change: ShellChangeEvent, renames: &mut Vec<(String, String)>)
{
    if let ShellChangeEvent::Renamed { old_reference, new_reference } = change
    {
        renames.push((old_reference, new_reference));
    }
}
