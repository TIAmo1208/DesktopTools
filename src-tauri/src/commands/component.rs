use crate::preferences::SettingsState;
use tauri::{AppHandle, Emitter, State};

use crate::component_runtime::ComponentRuntimeState;
use crate::contracts::{COMPONENT_RUNTIME_SNAPSHOT_CHANGED_EVENT, ComponentCapability, ComponentCapabilityResult, ComponentHostEvent, ComponentInstanceSnapshot, ComponentRuntimeError, ComponentRuntimeSnapshot};
use crate::workspace::WorkspaceState;

/// 同步当前工作区组件实例并返回运行时注册快照。
#[tauri::command]
pub fn get_component_runtime_snapshot(app: AppHandle, state: State<'_, ComponentRuntimeState>, workspace: State<'_, WorkspaceState>, settings: State<'_, SettingsState>) -> Result<ComponentRuntimeSnapshot, ComponentRuntimeError> { execute_get_component_runtime_snapshot(&app, &state, &workspace, &settings) }

/// 两种 transport 共用的命令执行与事件发布。
pub(crate) fn execute_get_component_runtime_snapshot(app: &AppHandle, state: &ComponentRuntimeState, workspace: &WorkspaceState, settings: &SettingsState) -> Result<ComponentRuntimeSnapshot, ComponentRuntimeError>
{
    let snapshot = component_runtime_snapshot(state, workspace, settings)?; // 当前活动布局对应的实例注册结果。
    app.emit(COMPONENT_RUNTIME_SNAPSHOT_CHANGED_EVENT, &snapshot).map_err(|_| ComponentRuntimeError::state_unavailable())?;
    Ok(snapshot)
}

/// 校验实例清单后执行一个已声明并授权的宿主能力。
#[tauri::command]
pub fn invoke_component_capability(settings: State<'_, SettingsState>, state: State<'_, ComponentRuntimeState>, workspace: State<'_, WorkspaceState>, instance_id: String, capability: ComponentCapability) -> Result<ComponentCapabilityResult, ComponentRuntimeError> { execute_invoke_component_capability(&state, &workspace, &settings, instance_id, capability) }

/// 两种 transport 共用的命令执行与事件发布。
pub(crate) fn execute_invoke_component_capability(state: &ComponentRuntimeState, workspace: &WorkspaceState, settings: &SettingsState, instance_id: String, capability: ComponentCapability) -> Result<ComponentCapabilityResult, ComponentRuntimeError>
{
    let instances = merged_instances(state, workspace, settings)?; // 工作区实例 + 允许上岛的合成实例。
    state.invoke_capability(&instances, &instance_id, capability)
}

/// 工作区组件实例与允许上岛的合成实例合并视图；能力调用、loopback 与插件命令共用同一来源。
///
/// 合并只在本次校验内生效，不写布局、不写共享注册集合，也不改变岛实例的临时身份。
pub(crate) fn merged_instances(state: &ComponentRuntimeState, workspace: &WorkspaceState, settings: &SettingsState) -> Result<Vec<ComponentInstanceSnapshot>, ComponentRuntimeError>
{
    let workspace = workspace.snapshot().map_err(|error| ComponentRuntimeError::invalid_instance(error.message))?; // 统一工作区组件实例来源。
    let preferences = settings.preferences().map_err(|_| ComponentRuntimeError::state_unavailable())?; // 岛开关、卡片顺序与停用名单决定可见实例。
    let mut instances = workspace.components;
    instances.extend(state.visible_island_instances(&preferences)); // 仅在本次校验中合并，不共享注册表。
    Ok(instances)
}

/// 岛开关、卡片顺序或插件停用变化后立即释放不再可见实例的通道。
///
/// 与组件快照读取共用同一份合并视图，但不依赖页面是否来读快照。
pub(crate) fn prune_loopback_for_visibility(state: &ComponentRuntimeState, workspace: &WorkspaceState, settings: &SettingsState) -> Result<(), ComponentRuntimeError>
{
    let instances = merged_instances(state, workspace, settings)?;
    let present = instances.iter().map(|component| component.instance_id.clone()).collect::<std::collections::HashSet<_>>();
    state.loopback().prune(&present);
    Ok(())
}

/// 返回 Tauri 与本地 transport 共享的组件运行时快照。
pub(crate) fn component_runtime_snapshot(state: &ComponentRuntimeState, workspace: &WorkspaceState, settings: &SettingsState) -> Result<ComponentRuntimeSnapshot, ComponentRuntimeError>
{
    let workspace = workspace.snapshot().map_err(|error| ComponentRuntimeError::invalid_instance(error.message))?; // 统一工作区的持久组件集合。
    let snapshot = state.snapshot(&workspace.components)?;
    let preferences = settings.preferences().map_err(|_| ComponentRuntimeError::state_unavailable())?; // 岛开关、卡片顺序与停用名单决定保留哪些岛实例通道。
    let mut instances = workspace.components;
    instances.extend(state.visible_island_instances(&preferences)); // 岛实例也可能注册 loopback，不能被裁剪掉。
    let present = instances.iter().map(|component| component.instance_id.clone()).collect::<std::collections::HashSet<_>>(); // 当前布局与岛上仍存在的实例。
    state.loopback().prune(&present); // 实例移除、插件卸载或页面销毁后释放端口并丢弃缓冲。
    Ok(snapshot)
}

/// 返回独立 Windowed transport 最近收到的组件宿主事件。
pub(crate) fn current_component_host_event(state: &ComponentRuntimeState) -> Result<Option<ComponentHostEvent>, ComponentRuntimeError> { state.current_host_event() }
