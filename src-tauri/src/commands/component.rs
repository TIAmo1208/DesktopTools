use crate::app::settings::SettingsState;
use tauri::{AppHandle, Emitter, State};

use crate::component_runtime::ComponentRuntimeState;
use crate::contracts::{COMPONENT_RUNTIME_SNAPSHOT_CHANGED_EVENT, ComponentCapability, ComponentCapabilityResult, ComponentHostEvent, ComponentRuntimeError, ComponentRuntimeSnapshot};
use crate::workspace::WorkspaceState;

/// 同步当前工作区组件实例并返回运行时注册快照。
#[tauri::command]
pub fn get_component_runtime_snapshot(app: AppHandle, state: State<'_, ComponentRuntimeState>, workspace: State<'_, WorkspaceState>) -> Result<ComponentRuntimeSnapshot, ComponentRuntimeError> { execute_get_component_runtime_snapshot(&app, &state, &workspace) }

/// 两种 transport 共用的命令执行与事件发布。
pub(crate) fn execute_get_component_runtime_snapshot(app: &AppHandle, state: &ComponentRuntimeState, workspace: &WorkspaceState) -> Result<ComponentRuntimeSnapshot, ComponentRuntimeError>
{
    let snapshot = component_runtime_snapshot(state, workspace)?; // 当前活动布局对应的实例注册结果。
    app.emit(COMPONENT_RUNTIME_SNAPSHOT_CHANGED_EVENT, &snapshot).map_err(|_| ComponentRuntimeError::state_unavailable())?;
    Ok(snapshot)
}

/// 校验实例清单后执行一个已声明并授权的宿主能力。
#[tauri::command]
pub fn invoke_component_capability(settings: State<'_, SettingsState>, state: State<'_, ComponentRuntimeState>, workspace: State<'_, WorkspaceState>, instance_id: String, capability: ComponentCapability) -> Result<ComponentCapabilityResult, ComponentRuntimeError> { execute_invoke_component_capability(&state, &workspace, &settings, instance_id, capability) }

/// 两种 transport 共用的命令执行与事件发布。
pub(crate) fn execute_invoke_component_capability(state: &ComponentRuntimeState, workspace: &WorkspaceState, settings: &SettingsState, instance_id: String, capability: ComponentCapability) -> Result<ComponentCapabilityResult, ComponentRuntimeError>
{
    let workspace = workspace.snapshot().map_err(|error| ComponentRuntimeError::invalid_instance(error.message))?; // 统一工作区组件实例来源。
    let preferences = settings.preferences().map_err(|_| ComponentRuntimeError::state_unavailable())?; // 当前上岛顺序决定合成实例授权。
    let mut instances = workspace.components;
    instances.extend(state.island_instances(&preferences.island_card_order)); // 仅在本次能力校验中合并，不写布局或共享注册表。
    state.invoke_capability(&instances, &instance_id, capability)
}

/// 返回 Tauri 与本地 transport 共享的组件运行时快照。
pub(crate) fn component_runtime_snapshot(state: &ComponentRuntimeState, workspace: &WorkspaceState) -> Result<ComponentRuntimeSnapshot, ComponentRuntimeError>
{
    let workspace = workspace.snapshot().map_err(|error| ComponentRuntimeError::invalid_instance(error.message))?; // 统一工作区的持久组件集合。
    let snapshot = state.snapshot(&workspace.components)?;
    let present = workspace.components.iter().map(|component| component.instance_id.clone()).collect::<std::collections::HashSet<_>>(); // 当前布局仍存在的实例。
    state.loopback().prune(&present); // 实例移除、插件卸载或页面销毁后释放端口并丢弃缓冲。
    Ok(snapshot)
}

/// 返回独立 Windowed transport 最近收到的组件宿主事件。
pub(crate) fn current_component_host_event(state: &ComponentRuntimeState) -> Result<Option<ComponentHostEvent>, ComponentRuntimeError> { state.current_host_event() }
