use std::path::PathBuf;
use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};

use crate::contracts::{DESKTOP_ITEM_SNAPSHOT_CHANGED_EVENT, DesktopItemSnapshot, WORKSPACE_SNAPSHOT_CHANGED_EVENT, WorkspaceBackupSummary, WorkspaceCommand, WorkspaceError, WorkspaceHistoryKind, WorkspaceHistoryResult, WorkspaceInteractionCommand, WorkspaceRestoreResult, WorkspaceSnapshot};
use crate::host::DesktopHostRuntime;
use crate::platform::windows::desktop_item_monitor_assignments;
use crate::shell::DesktopItemState;
use crate::workspace::WorkspaceState;

/// 返回当前不可变工作区快照。
#[tauri::command]
pub fn get_workspace_snapshot(state: State<'_, WorkspaceState>) -> Result<WorkspaceSnapshot, WorkspaceError> { read_workspace_snapshot(&state) }

/// 校验并提交一个工作区命令，然后发布新的不可变快照。
#[tauri::command]
pub fn apply_workspace_command(app: AppHandle, state: State<'_, WorkspaceState>, desktop_items: State<'_, DesktopItemState>, runtime: State<'_, std::sync::Arc<DesktopHostRuntime>>, monitor_id: String, command: WorkspaceCommand) -> Result<WorkspaceSnapshot, WorkspaceError> { execute_apply_workspace_command(&app, &state, &desktop_items, &runtime, monitor_id, command) }

/// 两种 transport 共用的命令执行与事件发布。
pub(crate) fn execute_apply_workspace_command(app: &AppHandle, state: &WorkspaceState, desktop_items: &DesktopItemState, runtime: &DesktopHostRuntime, monitor_id: String, command: WorkspaceCommand) -> Result<WorkspaceSnapshot, WorkspaceError>
{
    let before_revision = state.snapshot()?.revision; // 判断布局命令是否真正创建新统一时间线分支的修订号。
    let history_command = matches!(&command, WorkspaceCommand::SetFolderPortalAppearance { .. } | WorkspaceCommand::UndoLayout | WorkspaceCommand::RedoLayout | WorkspaceCommand::SetTileAppearance { .. } | WorkspaceCommand::RenameFence { .. } | WorkspaceCommand::SetFenceAutoExpand { .. } | WorkspaceCommand::SetFenceCollapsed { .. }); // 外观和历史命令不创建新的 Shell 重做分支。
    // 已原子持久化的下一工作区快照。
    let snapshot = apply_workspace_state_command(state, runtime, &monitor_id, command)?;
    if !history_command && snapshot.revision != before_revision
    {
        desktop_items.clear_redo_history().map_err(|error| WorkspaceError::history_failed(&error.message))?;
    }
    app.emit(WORKSPACE_SNAPSHOT_CHANGED_EVENT, &snapshot).map_err(|error| {
        crate::log_error!("工作区：快照事件发布失败，错误={error}");
        WorkspaceError::state_unavailable()
    })?;
    Ok(snapshot)
}

/// 按完成时间撤销统一时间线中的最近布局或 Shell 操作。
#[tauri::command]
pub async fn undo_workspace(app: AppHandle, workspace: State<'_, WorkspaceState>, desktop_items: State<'_, DesktopItemState>, runtime: State<'_, Arc<DesktopHostRuntime>>) -> Result<WorkspaceHistoryResult, WorkspaceError> { apply_workspace_history(&app, &workspace, &desktop_items, &runtime, false) }

/// 按最近撤销时间重做统一时间线中的布局或 Shell 操作。
#[tauri::command]
pub async fn redo_workspace(app: AppHandle, workspace: State<'_, WorkspaceState>, desktop_items: State<'_, DesktopItemState>, runtime: State<'_, Arc<DesktopHostRuntime>>) -> Result<WorkspaceHistoryResult, WorkspaceError> { apply_workspace_history(&app, &workspace, &desktop_items, &runtime, true) }

/// 将当前完整工作区原子写入手动备份文件。
#[tauri::command]
pub fn write_workspace_backup(workspace: State<'_, WorkspaceState>, path: String) -> Result<WorkspaceBackupSummary, WorkspaceError> { workspace.write_manual_backup(&PathBuf::from(path)) }

/// 只读校验当前格式的手动备份或自动快照。
#[tauri::command]
pub fn inspect_workspace_backup(workspace: State<'_, WorkspaceState>, path: String) -> Result<WorkspaceBackupSummary, WorkspaceError> { workspace.inspect_backup(&PathBuf::from(path)) }

/// 返回当前仍保留且通过完整性校验的全部自动快照。
#[tauri::command]
pub fn list_workspace_auto_snapshots(workspace: State<'_, WorkspaceState>) -> Result<Vec<WorkspaceBackupSummary>, WorkspaceError> { workspace.auto_snapshot_summaries() }

/// 事务式还原统一工作区备份。
#[tauri::command]
pub fn restore_workspace_backup(app: AppHandle, workspace: State<'_, WorkspaceState>, desktop_items: State<'_, DesktopItemState>, runtime: State<'_, Arc<DesktopHostRuntime>>, path: String) -> Result<WorkspaceRestoreResult, WorkspaceError> { execute_restore_workspace_backup(&app, &workspace, &desktop_items, &runtime, path) }

/// 两种 transport 共用的命令执行与事件发布。
pub(crate) fn execute_restore_workspace_backup(app: &AppHandle, workspace: &WorkspaceState, desktop_items: &DesktopItemState, runtime: &DesktopHostRuntime, path: String) -> Result<WorkspaceRestoreResult, WorkspaceError>
{
    let monitors = runtime.snapshot().map_err(|error| WorkspaceError::restore_failed(&error.message))?.hosts.into_iter().map(|host| host.monitor).collect::<Vec<_>>(); // 当前逐屏环境。
    let item_ids = desktop_items.snapshot().map_err(|error| WorkspaceError::restore_failed(&error.message))?.items.into_iter().map(|item| item.stable_reference).collect::<Vec<_>>(); // 当前可解析的 Shell 引用。
    let result = workspace.restore_backup(&PathBuf::from(path), &monitors, &item_ids)?; // 工作区原子替换完成。
    desktop_items.clear_operation_history().map_err(|error| WorkspaceError::history_failed(&error.message))?;
    app.emit(WORKSPACE_SNAPSHOT_CHANGED_EVENT, &result.snapshot).map_err(|error| WorkspaceError::restore_failed(&error))?;
    Ok(result)
}

/// 在统一布局历史与共享 Shell 历史之间执行最近有效单元。
pub(crate) fn apply_workspace_history(app: &AppHandle, workspace: &WorkspaceState, desktop_items: &DesktopItemState, runtime: &DesktopHostRuntime, redo: bool) -> Result<WorkspaceHistoryResult, WorkspaceError>
{
    let layout_time = workspace.latest_layout_history_time(redo)?; // 当前方向最近布局单元时间。
    let shell_time = desktop_items.latest_shell_history_time(redo).map_err(|error| WorkspaceError::history_failed(&error.message))?; // 当前方向最近 Shell 单元时间。
    let use_shell = shell_time.is_some() && (layout_time.is_none() || shell_time.unwrap_or_default() >= layout_time.unwrap_or_default()); // 时间相同时优先处理共享 Shell 事实。
    if use_shell
    {
        let shell_operation = desktop_items.restore_shell_history(redo).map_err(|error| WorkspaceError::history_failed(&error.message))?;
        if let Some(mut shell_operation) = shell_operation
        {
            let operation_id = shell_operation.operation_id; // 无论后续哪一步失败都释放历史占用。
            let outcome = (|| {
                let renames = if shell_operation.kind == crate::contracts::ShellOperationKind::Rename
                {
                    shell_operation.items.iter().filter(|item| item.succeeded).filter_map(|item| item.result_reference.as_ref().filter(|reference| *reference != &item.source_reference).map(|reference| (item.source_reference.clone(), reference.clone()))).collect::<Vec<_>>()
                }
                else
                {
                    Vec::new()
                }; // 仅使用已确认的重命名事实，不把复制或回收站还原误认为同一引用迁移。
                let item_snapshot = desktop_items.refresh().map_err(|error| WorkspaceError::history_failed(&error.message))?; // 补偿后重新枚举的 Shell 最终事实。
                desktop_items.reconcile_operation_history(&item_snapshot, &renames).map_err(|error| WorkspaceError::history_failed(&error.message))?;
                reconcile_workspace_environment(workspace, runtime, &item_snapshot, false, &renames)?; // 先补齐真实文件事实，再恢复联合归属。
                let (restored, changed) = desktop_items.restore_drop_history_layout(workspace, &item_snapshot, operation_id, redo, shell_operation.aborted).map_err(|error| WorkspaceError::history_failed(&error.message))?;
                if restored > 0
                {
                    shell_operation.message.push_str(&format!("；布局恢复 {restored} 项"));
                }
                if changed > 0
                {
                    shell_operation.message.push_str(&format!("；{changed} 项归属已变化，保留当前布局"));
                }
                let snapshot = workspace.snapshot()?; // 联合布局已经原子保存。
                app.emit(DESKTOP_ITEM_SNAPSHOT_CHANGED_EVENT, &item_snapshot).map_err(|error| WorkspaceError::history_failed(&error))?;
                app.emit(WORKSPACE_SNAPSHOT_CHANGED_EVENT, &snapshot).map_err(|error| WorkspaceError::history_failed(&error))?;
                Ok(WorkspaceHistoryResult { snapshot, kind: WorkspaceHistoryKind::Shell, message: shell_operation.message.clone(), shell_operation: Some(shell_operation) })
            })();
            desktop_items.finish_shell_history(operation_id);
            return outcome;
        }
    }
    if layout_time.is_some()
    {
        let result = workspace.restore_layout_history(redo)?; // 已原子持久化的布局历史恢复结果。
        let snapshot = result.snapshot;
        app.emit(WORKSPACE_SNAPSHOT_CHANGED_EVENT, &snapshot).map_err(|error| WorkspaceError::history_failed(&error))?;
        return Ok(WorkspaceHistoryResult {
            snapshot,
            kind: WorkspaceHistoryKind::Layout,
            shell_operation: None,
            message: if result.changed_count > 0
            {
                if redo
                {
                    format!("已重做 {} 项，{} 项状态已变化", result.restored_count, result.changed_count)
                }
                else
                {
                    format!("已恢复 {} 项，{} 项状态已变化", result.restored_count, result.changed_count)
                }
            }
            else if redo
            {
                "布局已重做".to_string()
            }
            else
            {
                "布局已撤销".to_string()
            },
        });
    }
    let snapshot = workspace.snapshot()?; // 当前方向没有可执行历史时保持不变的布局。
    Ok(WorkspaceHistoryResult { snapshot, kind: WorkspaceHistoryKind::None, shell_operation: None, message: if redo { "没有可重做的操作".to_string() } else { "没有可撤销的操作".to_string() } })
}

/// 校验并提交一个运行期交互命令，然后发布新的不可变快照。
#[tauri::command]
pub fn apply_workspace_interaction(app: AppHandle, state: State<'_, WorkspaceState>, runtime: State<'_, std::sync::Arc<DesktopHostRuntime>>, monitor_id: String, command: WorkspaceInteractionCommand) -> Result<WorkspaceSnapshot, WorkspaceError> { execute_apply_workspace_interaction(&app, &state, &runtime, monitor_id, command) }

/// 两种 transport 共用的命令执行与事件发布。
pub(crate) fn execute_apply_workspace_interaction(app: &AppHandle, state: &WorkspaceState, runtime: &DesktopHostRuntime, monitor_id: String, command: WorkspaceInteractionCommand) -> Result<WorkspaceSnapshot, WorkspaceError>
{
    // 已提交到内存的下一工作区快照。
    let snapshot = apply_workspace_state_interaction(state, runtime, &monitor_id, command)?;
    app.emit(WORKSPACE_SNAPSHOT_CHANGED_EVENT, &snapshot).map_err(|error| {
        crate::log_error!("工作区：交互快照事件发布失败，错误={error}");
        WorkspaceError::state_unavailable()
    })?;
    Ok(snapshot)
}

/// 返回 Tauri 与本地 transport 共享的当前工作区快照。
pub(crate) fn read_workspace_snapshot(state: &WorkspaceState) -> Result<WorkspaceSnapshot, WorkspaceError> { state.snapshot() }

/// 应用 Tauri 与本地 transport 共享的工作区命令。
pub(crate) fn apply_workspace_state_command(state: &WorkspaceState, runtime: &DesktopHostRuntime, monitor_id: &str, command: WorkspaceCommand) -> Result<WorkspaceSnapshot, WorkspaceError>
{
    runtime.validate_monitor_id(monitor_id).map_err(|error| {
        crate::log_error!("工作区：命令显示器校验失败，错误={}", error.message);
        WorkspaceError::invalid_command("布局命令显示器不存在")
    })?;
    state.apply_on_monitor(Some(monitor_id), command)
}

/// 应用 Tauri 与本地 transport 共享的运行期交互命令。
pub(crate) fn apply_workspace_state_interaction(state: &WorkspaceState, runtime: &DesktopHostRuntime, monitor_id: &str, command: WorkspaceInteractionCommand) -> Result<WorkspaceSnapshot, WorkspaceError>
{
    runtime.validate_monitor_id(monitor_id).map_err(|error| {
        crate::log_error!("工作区：交互显示器校验失败，错误={}", error.message);
        WorkspaceError::invalid_command("交互命令显示器不存在")
    })?;
    state.apply_interaction_on_monitor(monitor_id, command)
}

/// 使用正式宿主尺寸和 Shell 身份协调完整未归类位置。
pub(crate) fn reconcile_workspace_items(state: &WorkspaceState, runtime: &DesktopHostRuntime, desktop_items: &DesktopItemSnapshot) -> Result<WorkspaceSnapshot, WorkspaceError> { reconcile_workspace_environment(state, runtime, desktop_items, true, &[]) }

/// 使用正式宿主环境、Shell 项目归属和精确重命名协调完整逐屏工作区。
pub(crate) fn reconcile_workspace_environment(state: &WorkspaceState, runtime: &DesktopHostRuntime, desktop_items: &DesktopItemSnapshot, read_assignments: bool, renames: &[(String, String)]) -> Result<WorkspaceSnapshot, WorkspaceError>
{
    // 当前正式宿主提供的全部显示器工作区。
    let host_snapshot = runtime.snapshot().map_err(|error| {
        crate::log_error!("工作区：读取宿主尺寸失败，错误={}", error.message);
        WorkspaceError::state_unavailable()
    })?;
    let monitors = host_snapshot.hosts.into_iter().map(|host| host.monitor).collect::<Vec<_>>(); // 当前逐屏宿主的显示器物理状态。
    let assignments = if read_assignments
    {
        desktop_item_monitor_assignments(
            &desktop_items.items,
            &monitors,
            runtime.shell_state().map_err(|error| {
                crate::log_error!("工作区：读取 Explorer 宿主失败，错误={}", error.message);
                WorkspaceError::state_unavailable()
            })?,
        )
    }
    else
    {
        Vec::new()
    }; // 初始导入、拓扑或 Explorer 变化时使用的原始项目归属。
    state.reconcile_environment(crate::workspace::WorkspaceEnvironment { transfer_assignments: false, monitors: &monitors, items: &desktop_items.items, assignments: &assignments, renames })
}
