use tauri::{AppHandle, Emitter, State};

use crate::component_runtime::ComponentRuntimeState;
use crate::contracts::{
    COMPONENT_RUNTIME_SNAPSHOT_CHANGED_EVENT, ComponentCapability, ComponentCapabilityResult,
    ComponentHostEvent, ComponentRuntimeError, ComponentRuntimeSnapshot,
};
use crate::workspace::WorkspaceState;

/// 同步当前工作区组件实例并返回运行时注册快照。
#[tauri::command]
pub fn get_component_runtime_snapshot(
    app: AppHandle,
    state: State<'_, ComponentRuntimeState>,
    workspace: State<'_, WorkspaceState>,
) -> Result<ComponentRuntimeSnapshot, ComponentRuntimeError> {
    let snapshot = component_runtime_snapshot(&state, &workspace)?; // 当前活动布局对应的实例注册结果。
    app.emit(COMPONENT_RUNTIME_SNAPSHOT_CHANGED_EVENT, &snapshot)
        .map_err(|_| ComponentRuntimeError::state_unavailable())?;
    Ok(snapshot)
}

/// 校验实例清单后执行一个已声明并授权的宿主能力。
#[tauri::command]
pub fn invoke_component_capability(
    state: State<'_, ComponentRuntimeState>,
    workspace: State<'_, WorkspaceState>,
    instance_id: String,
    capability: ComponentCapability,
) -> Result<ComponentCapabilityResult, ComponentRuntimeError> {
    let workspace = workspace
        .snapshot()
        .map_err(|error| ComponentRuntimeError::invalid_instance(error.message))?; // 当前活动虚拟桌面组件实例来源。
    state.invoke_capability(&workspace.components, &instance_id, capability)
}

/// 返回 Tauri 与本地 transport 共享的组件运行时快照。
pub(crate) fn component_runtime_snapshot(
    state: &ComponentRuntimeState,
    workspace: &WorkspaceState,
) -> Result<ComponentRuntimeSnapshot, ComponentRuntimeError> {
    let workspace = workspace
        .snapshot()
        .map_err(|error| ComponentRuntimeError::invalid_instance(error.message))?; // 当前活动虚拟桌面的持久组件集合。
    state.snapshot(&workspace.components)
}

/// 返回独立 Windowed transport 最近收到的组件宿主事件。
pub(crate) fn current_component_host_event(
    state: &ComponentRuntimeState,
) -> Result<Option<ComponentHostEvent>, ComponentRuntimeError> {
    state.current_host_event()
}
