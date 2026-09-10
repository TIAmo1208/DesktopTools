use std::sync::Arc;

use tauri::{AppHandle, Emitter, State};

use crate::contracts::{VIRTUAL_DESKTOP_SNAPSHOT_CHANGED_EVENT, VirtualDesktopError, VirtualDesktopSnapshot, WORKSPACE_SNAPSHOT_CHANGED_EVENT};
use crate::host::DesktopHostRuntime;
use crate::shell::DesktopItemState;
use crate::virtual_desktop::VirtualDesktopState;
use crate::workspace::WorkspaceState;

/// 返回当前 Windows 虚拟桌面环境与布局归档摘要。
#[tauri::command]
pub fn get_virtual_desktop_snapshot(state: State<'_, VirtualDesktopState>, workspace: State<'_, WorkspaceState>) -> Result<VirtualDesktopSnapshot, VirtualDesktopError> { state.snapshot(&workspace) }

/// 创建新 Windows 虚拟桌面并建立默认独立布局。
#[tauri::command]
pub fn create_virtual_desktop(app: AppHandle, state: State<'_, VirtualDesktopState>, workspace: State<'_, WorkspaceState>, runtime: State<'_, Arc<DesktopHostRuntime>>, desktop_items: State<'_, DesktopItemState>) -> Result<VirtualDesktopSnapshot, VirtualDesktopError> { execute_create_virtual_desktop(&app, &state, &workspace, &runtime, &desktop_items) }

/// 两种 transport 共用的命令执行与事件发布。
pub(crate) fn execute_create_virtual_desktop(app: &AppHandle, state: &VirtualDesktopState, workspace: &WorkspaceState, runtime: &DesktopHostRuntime, desktop_items: &DesktopItemState) -> Result<VirtualDesktopSnapshot, VirtualDesktopError>
{
    let snapshot = state.create(workspace, runtime, desktop_items)?; // 创建后最终虚拟桌面环境。
    emit_virtual_desktop_state(app, workspace, &snapshot)?;
    Ok(snapshot)
}

/// 删除目标 Windows 虚拟桌面并归档其独立布局。
#[tauri::command]
pub fn delete_virtual_desktop(app: AppHandle, state: State<'_, VirtualDesktopState>, workspace: State<'_, WorkspaceState>, runtime: State<'_, Arc<DesktopHostRuntime>>, desktop_items: State<'_, DesktopItemState>, desktop_id: String, fallback_id: String) -> Result<VirtualDesktopSnapshot, VirtualDesktopError> { execute_delete_virtual_desktop(&app, &state, &workspace, &runtime, &desktop_items, desktop_id, fallback_id) }

/// 两种 transport 共用的命令执行与事件发布。
pub(crate) fn execute_delete_virtual_desktop(app: &AppHandle, state: &VirtualDesktopState, workspace: &WorkspaceState, runtime: &DesktopHostRuntime, desktop_items: &DesktopItemState, desktop_id: String, fallback_id: String) -> Result<VirtualDesktopSnapshot, VirtualDesktopError>
{
    let snapshot = state.remove(&desktop_id, &fallback_id, workspace, runtime, desktop_items)?; // 删除后最终虚拟桌面环境。
    emit_virtual_desktop_state(app, workspace, &snapshot)?;
    Ok(snapshot)
}

/// 创建新 Windows 虚拟桌面并绑定一个删除布局归档。
#[tauri::command]
pub fn restore_virtual_desktop_archive(app: AppHandle, state: State<'_, VirtualDesktopState>, workspace: State<'_, WorkspaceState>, runtime: State<'_, Arc<DesktopHostRuntime>>, desktop_items: State<'_, DesktopItemState>, archive_id: String) -> Result<VirtualDesktopSnapshot, VirtualDesktopError> { execute_restore_virtual_desktop_archive(&app, &state, &workspace, &runtime, &desktop_items, archive_id) }

/// 两种 transport 共用的命令执行与事件发布。
pub(crate) fn execute_restore_virtual_desktop_archive(app: &AppHandle, state: &VirtualDesktopState, workspace: &WorkspaceState, runtime: &DesktopHostRuntime, desktop_items: &DesktopItemState, archive_id: String) -> Result<VirtualDesktopSnapshot, VirtualDesktopError>
{
    let snapshot = state.restore_archive(&archive_id, workspace, runtime, desktop_items)?; // 归档绑定后的最终虚拟桌面环境。
    emit_virtual_desktop_state(app, workspace, &snapshot)?;
    Ok(snapshot)
}

/// 发布虚拟桌面环境与当前活动工作区快照。
fn emit_virtual_desktop_state(app: &AppHandle, workspace: &WorkspaceState, snapshot: &VirtualDesktopSnapshot) -> Result<(), VirtualDesktopError>
{
    app.emit(VIRTUAL_DESKTOP_SNAPSHOT_CHANGED_EVENT, snapshot).map_err(|error| VirtualDesktopError::platform_failed(&error))?;
    let workspace_snapshot = workspace.snapshot().map_err(|error| VirtualDesktopError::workspace_failed(&error))?; // 当前活动虚拟桌面布局。
    app.emit(WORKSPACE_SNAPSHOT_CHANGED_EVENT, workspace_snapshot).map_err(|error| VirtualDesktopError::platform_failed(&error))
}
