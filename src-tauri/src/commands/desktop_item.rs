use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};

use crate::contracts::{
    DESKTOP_ITEM_SNAPSHOT_CHANGED_EVENT, DesktopItemError, DesktopItemSnapshot,
    WORKSPACE_SNAPSHOT_CHANGED_EVENT,
};
use crate::host::DesktopHostRuntime;
use crate::shell::DesktopItemState;
use crate::workspace::WorkspaceState;

use super::workspace::reconcile_workspace_items;

/// 枚举真实 Shell 桌面项目并发布新的不可变快照。
#[tauri::command]
pub fn get_desktop_item_snapshot(
    app: AppHandle,
    state: State<'_, DesktopItemState>,
    workspace: State<'_, WorkspaceState>,
    runtime: State<'_, Arc<DesktopHostRuntime>>,
) -> Result<DesktopItemSnapshot, DesktopItemError> {
    // 已发布到内存状态的桌面项目快照。
    let snapshot = refresh_desktop_item_snapshot(&state)?;
    app.emit(DESKTOP_ITEM_SNAPSHOT_CHANGED_EVENT, &snapshot)
        .map_err(|error| {
            crate::log_error!("Shell：桌面项目快照事件发布失败，错误：{error}");
            DesktopItemError::event_publish_failed(&error)
        })?;
    // 已按正式宿主网格协调的工作区快照。
    let workspace_snapshot = reconcile_workspace_items(&workspace, &runtime, &snapshot)
        .map_err(|error| DesktopItemError::workspace_reconciliation_failed(&error))?;
    app.emit(WORKSPACE_SNAPSHOT_CHANGED_EVENT, &workspace_snapshot)
        .map_err(|error| DesktopItemError::event_publish_failed(&error))?;

    Ok(snapshot)
}

/// 枚举 transport 共享的桌面项目快照。
pub(crate) fn refresh_desktop_item_snapshot(
    state: &DesktopItemState,
) -> Result<DesktopItemSnapshot, DesktopItemError> {
    state.refresh()
}

/// 返回本地事件轮询使用的当前桌面项目快照。
pub(crate) fn read_desktop_item_snapshot(
    state: &DesktopItemState,
) -> Result<DesktopItemSnapshot, DesktopItemError> {
    state.snapshot()
}
