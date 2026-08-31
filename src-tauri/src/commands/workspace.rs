use tauri::{AppHandle, Emitter, State};

use crate::contracts::{
    DesktopItemSnapshot, WORKSPACE_SNAPSHOT_CHANGED_EVENT, WorkspaceCommand, WorkspaceError,
    WorkspaceSnapshot,
};
use crate::host::DesktopHostRuntime;
use crate::workspace::WorkspaceState;

/// 返回当前不可变工作区快照。
#[tauri::command]
pub fn get_workspace_snapshot(
    state: State<'_, WorkspaceState>,
) -> Result<WorkspaceSnapshot, WorkspaceError> {
    read_workspace_snapshot(&state)
}

/// 校验并提交一个工作区命令，然后发布新的不可变快照。
#[tauri::command]
pub fn apply_workspace_command(
    app: AppHandle,
    state: State<'_, WorkspaceState>,
    command: WorkspaceCommand,
) -> Result<WorkspaceSnapshot, WorkspaceError> {
    // 已原子持久化的下一工作区快照。
    let snapshot = apply_workspace_state_command(&state, command)?;
    app.emit(WORKSPACE_SNAPSHOT_CHANGED_EVENT, &snapshot)
        .map_err(|error| {
            crate::log_error!("工作区：快照事件发布失败，错误={error}");
            WorkspaceError::state_unavailable()
        })?;
    Ok(snapshot)
}

/// 返回 Tauri 与本地 transport 共享的当前工作区快照。
pub(crate) fn read_workspace_snapshot(
    state: &WorkspaceState,
) -> Result<WorkspaceSnapshot, WorkspaceError> {
    state.snapshot()
}

/// 应用 Tauri 与本地 transport 共享的工作区命令。
pub(crate) fn apply_workspace_state_command(
    state: &WorkspaceState,
    command: WorkspaceCommand,
) -> Result<WorkspaceSnapshot, WorkspaceError> {
    state.apply(command)
}

/// 使用正式宿主尺寸和 Shell 身份协调完整未归类位置。
pub(crate) fn reconcile_workspace_items(
    state: &WorkspaceState,
    runtime: &DesktopHostRuntime,
    desktop_items: &DesktopItemSnapshot,
) -> Result<WorkspaceSnapshot, WorkspaceError> {
    // 当前正式宿主提供的单显示器工作区。
    let monitor = runtime
        .snapshot()
        .map_err(|error| {
            crate::log_error!("工作区：读取宿主尺寸失败，错误={}", error.message);
            WorkspaceError::state_unavailable()
        })?
        .monitor;
    // Shell 当前完整项目集合的稳定引用。
    let item_ids = desktop_items
        .items
        .iter()
        .map(|item| item.stable_reference.clone())
        .collect::<Vec<_>>();
    state.reconcile_items(
        f64::from(monitor.work_width) / monitor.scale_factor,
        f64::from(monitor.work_height) / monitor.scale_factor,
        &item_ids,
    )
}
