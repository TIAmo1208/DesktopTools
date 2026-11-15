mod application;
pub mod audio;
pub mod collect;
mod component;
mod desktop_host;
pub mod desktop_item;
pub mod folder_portal;
pub mod loopback;
pub use desktop_item::get_desktop_item_image;
pub use desktop_item::{cancel_desktop_item_drag_preparation, prepare_desktop_item_drag};
pub(crate) mod island;
pub mod music;
mod notifications;
pub mod plugin_settings;
pub mod settings;
pub use plugin_settings::{apply_plugin_install, apply_plugin_installs, cancel_plugin_install, choose_plugin_directory, create_plugin_component, get_plugin_settings, open_plugin_directory, open_plugin_repository, prepare_plugin_install_from_archive, prepare_plugin_install_from_folder, resolved_plugin_directory, save_plugin_settings, uninstall_plugin};
mod router;
pub mod weather;
mod workspace;
mod workspace_menu;
pub use music::{execute_music_action, get_current_music_session};

/// 程序启动事件是否已外发：首批宿主就绪只发一次。
static STARTED_EVENT_SENT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 新增格子共用的默认展开尺寸。
const DEFAULT_TILE_WIDTH: f64 = 360.0;

/// 新增格子共用的默认展开高度。
const DEFAULT_TILE_HEIGHT: f64 = 280.0;

/// 天气格子的官方组件包身份。
const WEATHER_COMPONENT_ID: &str = "official.weather";

/// 收集箱的官方组件包身份。
const COLLECT_COMPONENT_ID: &str = "official.collect-box";

/// 统一创建一个收集箱格子，设置页、桌面菜单和其他入口共用此逻辑。
pub(crate) fn execute_create_collect_box(app: &tauri::AppHandle, monitor_id: String) -> Result<(), String>
{
    use tauri::Manager;
    let runtime = app.state::<std::sync::Arc<DesktopHostRuntime>>();
    let workspace = app.state::<WorkspaceState>();
    let items = app.state::<DesktopItemState>();
    let bounds = workspace.next_component_bounds(&monitor_id, DEFAULT_TILE_WIDTH, DEFAULT_TILE_HEIGHT).map_err(|error| error.message)?;
    workspace::execute_apply_workspace_command(app, &workspace, &items, &runtime, monitor_id.clone(), WorkspaceCommand::CreateComponent { component_id: COLLECT_COMPONENT_ID.to_string(), bounds, state: json!({}) }).map_err(|error| error.message)?;
    crate::log_info!("收集箱：已创建格子，显示器={monitor_id}");
    Ok(())
}

/// 选择创建入口使用的目标显示器；没有主显示器时退回第一个可用宿主。
pub(crate) fn primary_monitor_id(runtime: &DesktopHostRuntime) -> Result<String, String>
{
    let hosts = runtime.snapshot().map_err(|error| error.message)?;
    hosts.hosts.iter().find(|host| host.monitor.primary).or_else(|| hosts.hosts.first()).map(|host| host.monitor.id.clone()).ok_or("桌面宿主尚未就绪".to_string())
}

/// 设置页从当前主显示器创建收集箱，兼容 Tauri 和独立宿主命令通道。
#[tauri::command]
pub fn create_collect_box(app: tauri::AppHandle) -> Result<(), String>
{
    use tauri::Manager;
    let monitor_id = primary_monitor_id(&app.state::<std::sync::Arc<DesktopHostRuntime>>())?;
    execute_create_collect_box(&app, monitor_id)
}

/// 设置页功能格子二级标题行的“添加格子到桌面”：官方格子沿用各自既有创建入口，插件格子沿用组件创建门禁。
/// 清单声明的格子数量上限对所有入口统一生效；返回本次新建的实例身份。
#[tauri::command]
pub fn create_tile_to_desktop(app: tauri::AppHandle, component_id: String) -> Result<Option<String>, String>
{
    use tauri::Manager;
    let workspace = app.state::<WorkspaceState>();
    let monitor_id = primary_monitor_id(&app.state::<std::sync::Arc<DesktopHostRuntime>>())?;
    match component_id.as_str()
    {
        COLLECT_COMPONENT_ID =>
        {
            execute_create_collect_box(&app, monitor_id)?;
            Ok(None)
        }
        WEATHER_COMPONENT_ID => execute_create_weather_tile(&app, None, monitor_id).map(Some),
        _ =>
        {
            let bounds = workspace.next_component_bounds(&monitor_id, DEFAULT_TILE_WIDTH, DEFAULT_TILE_HEIGHT).map_err(|error| error.message)?;
            let runtime = app.state::<std::sync::Arc<DesktopHostRuntime>>();
            let existing = workspace.snapshot().map_err(|error| error.message)?.components.into_iter().filter(|component| component.component_id == component_id).map(|component| component.instance_id).collect::<Vec<_>>(); // 创建前的实例身份。
            let snapshot = workspace::execute_apply_workspace_command(&app, &workspace, &app.state::<DesktopItemState>(), &runtime, monitor_id.clone(), WorkspaceCommand::CreateComponent { component_id: component_id.clone(), bounds, state: json!({}) }).map_err(|error| error.message)?;
            // 本次事务新提交的实例身份。
            let instance_id = snapshot.components.iter().find(|component| component.component_id == component_id && !existing.contains(&component.instance_id)).map(|component| component.instance_id.clone());
            crate::log_info!("设置：已创建格子，组件={component_id}，实例={instance_id:?}，显示器={monitor_id}");
            Ok(instance_id)
        }
    }
}

/// 组件清单声明的桌面格子数量上限；0 或缺省表示不限，官方与插件清单共用同一约束。
/// 与工作区创建命令共用同一实现，所有应用内创建入口都不会绕过。state 决定落进哪个类型桶，官方入口传空对象。
pub(crate) fn enforce_tile_limit(workspace: &WorkspaceState, component_id: &str, state: &serde_json::Value) -> Result<(), String>
{
    let layout = workspace.snapshot().map_err(|error| error.message)?; // 当前所有显示器上的格子。
    workspace.enforce_component_tile_limit(component_id, &workspace.component_tile_types(component_id), state, &layout).map_err(|error| error.message)
}

/// 开启限量为 1 的功能格子：先把开关写成开启并广播，再在桌面没有该格子时补建一个。
/// 先做数量校验再写开关，避免拒绝时留下“开关已开但格子没建”的状态。
/// 只负责“开启”，关闭走普通偏好保存，不删除布局，和官方格子的隐藏语义一致。
#[tauri::command]
pub fn enable_feature(app: tauri::AppHandle, component_id: String) -> Result<(), String>
{
    use tauri::Manager;
    let workspace = app.state::<WorkspaceState>();
    if !workspace.has_component(&component_id)
    {
        enforce_tile_limit(&workspace, &component_id, &serde_json::json!({}))?;
    }
    enable_feature_preferences(&app, &component_id)?;
    if workspace.has_component(&component_id)
    {
        crate::log_info!("设置：功能已开启，桌面已有格子，组件={component_id}");
        return Ok(());
    }
    let monitor_id = primary_monitor_id(&app.state::<std::sync::Arc<DesktopHostRuntime>>())?;
    let bounds = workspace.next_component_bounds(&monitor_id, DEFAULT_TILE_WIDTH, DEFAULT_TILE_HEIGHT).map_err(|error| error.message)?;
    let runtime = app.state::<std::sync::Arc<DesktopHostRuntime>>();
    workspace::execute_apply_workspace_command(&app, &workspace, &app.state::<DesktopItemState>(), &runtime, monitor_id.clone(), WorkspaceCommand::CreateComponent { component_id: component_id.clone(), bounds, state: json!({}) }).map_err(|error| error.message)?;
    crate::log_info!("设置：功能已开启并补齐默认格子，组件={component_id}，显示器={monitor_id}");
    Ok(())
}

/// 把指定功能格子的开关写成开启；没有官方开关的功能格子（例如 Debug 测试，显隐由清单决定）视为已开启。
fn enable_feature_preferences(app: &tauri::AppHandle, component_id: &str) -> Result<(), String>
{
    use tauri::Manager;
    let current = app.state::<crate::preferences::SettingsState>().preferences()?; // 当前偏好，作为写入基底。
    // 指定功能格子已经在开启状态或没有开关时不再写入。
    let mut enabled = current.clone();
    match component_id
    {
        "official.local-clock" => enabled.clock_enabled = true,
        "official.music-control" => enabled.music_enabled = true,
        "official.debug-test" => enabled.debug_enabled = true,
        WEATHER_COMPONENT_ID => enabled.weather_enabled = true,
        _ => return Ok(()),
    }
    if enabled == current
    {
        return Ok(());
    }
    crate::commands::settings::execute_apply_preferences(app, &app.state::<crate::preferences::SettingsState>(), enabled).map(|_| ())
}

/// 请求显示设置窗口：复用托盘同一显隐入口，不承载凭据或设置语义。
#[tauri::command]
pub fn open_settings_window(app: tauri::AppHandle) -> Result<(), String>
{
    crate::app::tray::set_settings_visible(&app, true).map_err(|error| {
        crate::log_error!("设置：请求显示窗口失败，错误：{error}");
        format!("设置窗口打开失败：{error}")
    })
}

/// 城市从来源列表删除后通知桌面格子重读：绑定该城市的格子立即进入待重选状态，不再等到下次打开菜单。
fn notify_weather_cities_changed(app: &tauri::AppHandle)
{
    if let Err(error) = app.emit("weather-configuration-changed", ())
    {
        crate::log_warn!("天气城市列表已变化，但刷新通知失败：{error}");
    }
}

/// 把城市加入某来源的城市列表；Tauri 与原生通道共用同一条信任边界。
pub(crate) fn add_weather_city(app: &tauri::AppHandle, source: crate::weather::Source, city: &crate::weather::City) -> Result<Vec<crate::weather::City>, String>
{
    use tauri::Manager;
    let state = app.state::<crate::weather::WeatherState>(); // 入列只按该来源规则校验候选对象，不要求城市已在列表中。
    state.save_city(source, city.clone())?;
    state.cities(source)
}

/// 按标识从某来源的城市列表移除一项；Tauri 与原生通道共用同一条信任边界，只删除该项，不改动任何格子的绑定。
pub(crate) fn remove_weather_city(app: &tauri::AppHandle, source: crate::weather::Source, city_id: &str) -> Result<Vec<crate::weather::City>, String>
{
    use tauri::Manager;
    let list = app.state::<crate::weather::WeatherState>().remove_city(source, city_id)?;
    notify_weather_cities_changed(app); // 已展开的格子立即重读列表，不再等到下次打开菜单。
    Ok(list)
}

/// 创建天气格子入口统一应用的功能开关偏好；原本已开启时返回空值，不产生多余写入。
pub(crate) fn weather_enabled_preferences(preferences: &crate::contracts::settings::ApplicationPreferences) -> Option<crate::contracts::settings::ApplicationPreferences>
{
    if preferences.weather_enabled
    {
        return None;
    }
    Some(crate::contracts::settings::ApplicationPreferences { weather_enabled: true, ..preferences.clone() })
}

/// 新格子的初始状态只保存来源与城市两个标识；未绑定入口使用空状态。
pub(crate) fn weather_tile_state(binding: Option<(&crate::weather::Source, &str)>) -> serde_json::Value
{
    match binding
    {
        Some((source, city_id)) => json!({ "weatherSource": source.key(), "weatherCityId": city_id }),
        None => json!({}),
    }
}

/// 本次事务新增的天气实例身份；创建前已存在的身份之外的第一项即新实例。
fn created_weather_instance(existing: &[String], components: &[crate::contracts::ComponentInstanceSnapshot]) -> Option<String> { components.iter().find(|component| component.component_id == WEATHER_COMPONENT_ID && !existing.contains(&component.instance_id)).map(|component| component.instance_id.clone()) }

/// 创建天气格子的统一入口，桌面菜单、来源菜单与 Explorer 菜单共用；不绑定来源与城市时使用空初始状态。
pub(crate) fn execute_create_weather_tile(app: &tauri::AppHandle, binding: Option<(crate::weather::Source, String)>, monitor_id: String) -> Result<String, String>
{
    use tauri::Manager;
    if let Some((source, city_id)) = binding.as_ref()
    {
        app.state::<crate::weather::WeatherState>().validated_city(*source, city_id)?; // 创建前确认城市仍在该来源列表中。
    }
    // 先把功能开关写成开启并广播，使创建动作不以格子可见性作为成功条件。
    let settings = app.state::<crate::preferences::SettingsState>();
    if let Some(preferences) = weather_enabled_preferences(&settings.preferences()?)
    {
        crate::commands::settings::execute_apply_preferences(app, &settings, preferences)?;
        crate::log_info!("天气：创建入口已开启天气功能开关");
    }
    // 新格子的初始状态只保存来源与城市两个标识，未绑定入口保持空状态。
    let state = weather_tile_state(binding.as_ref().map(|(source, city_id)| (source, city_id.as_str())));
    let workspace = app.state::<WorkspaceState>();
    let runtime = app.state::<std::sync::Arc<DesktopHostRuntime>>();
    // 创建前已存在的天气实例身份。
    let existing = workspace.snapshot().map_err(|error| error.message)?.components.into_iter().filter(|component| component.component_id == WEATHER_COMPONENT_ID).map(|component| component.instance_id).collect::<Vec<_>>();
    let bounds = workspace.next_component_bounds(&monitor_id, DEFAULT_TILE_WIDTH, DEFAULT_TILE_HEIGHT).map_err(|error| error.message)?; // 创建前计算的可用位置。
    let snapshot = workspace::execute_apply_workspace_command(app, &workspace, &app.state::<DesktopItemState>(), &runtime, monitor_id.clone(), WorkspaceCommand::CreateComponent { component_id: WEATHER_COMPONENT_ID.to_string(), bounds, state }).map_err(|error| error.message)?;
    // 本次事务新提交的天气实例身份由工作区分配，创建入口沿用它作为返回值。
    let instance_id = created_weather_instance(&existing, &snapshot.components).ok_or("天气格子创建结果缺失")?;
    crate::log_info!("天气：已创建格子，实例={instance_id}，显示器={monitor_id}");
    Ok(instance_id)
}

/// 执行 Explorer 注册表菜单回传的动作，复用现有布局事务和事件发布。
pub(crate) fn execute_registered_desktop_menu(app: &tauri::AppHandle, selected: u32) -> Result<(), String>
{
    use tauri::{Emitter, Manager};
    if selected == 0x7807
    {
        return crate::app::tray::set_settings_visible(app, true).map_err(|error| error.to_string());
    }
    if selected == 0x7809
    {
        app.exit(0);
        return Ok(());
    }
    let runtime = app.state::<std::sync::Arc<DesktopHostRuntime>>(); // 当前逐屏宿主。
    let hosts = runtime.snapshot().map_err(|error| error.message)?; // 当前显示器环境。
    let mut point = windows::Win32::Foundation::POINT::default(); // 执行动作时的物理指针位置。
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut point) }.map_err(|error| error.to_string())?;
    let monitor = hosts.hosts.iter().map(|host| &host.monitor).find(|monitor| point.x >= monitor.x && point.y >= monitor.y && i64::from(point.x) < i64::from(monitor.x) + i64::from(monitor.width) && i64::from(point.y) < i64::from(monitor.y) + i64::from(monitor.height)).or_else(|| hosts.hosts.first().map(|host| &host.monitor)).ok_or("桌面宿主尚未就绪")?; // 跨屏命令按实际指针选择宿主。
    let workspace = app.state::<WorkspaceState>(); // 权威工作区。
    let items = app.state::<DesktopItemState>(); // Shell 项目状态。
    if selected == 0x780A
    {
        return execute_create_collect_box(app, monitor.id.clone());
    }
    let snapshot = workspace.snapshot().map_err(|error| error.message)?; // 解析动作时的当前布局。
    let action = workspace_menu::choose_workspace_menu(&snapshot, monitor, WorkspaceContextMenuTarget::Background, (f64::from(point.x) - f64::from(monitor.work_x)) / monitor.scale_factor, (f64::from(point.y) - f64::from(monitor.work_y)) / monitor.scale_factor, |_, _, _, _| Ok(Some(selected))).map_err(|error| error.message)?; // 只解释已选择的动作，不获取或弹出菜单。
    if selected == 0x7808
    {
        if !app.state::<crate::preferences::SettingsState>().preferences()?.folder_portal_enabled
        {
            return Err("文件夹门户已关闭，请在功能格子设置中启用".to_string());
        }
        if let Some(path) = folder_portal::create_from_desktop_menu()?
        {
            workspace::execute_apply_workspace_command(app, &workspace, &items, &runtime, monitor.id.clone(), WorkspaceCommand::CreateFolderPortal { root_path: path, bounds: crate::contracts::WorkspaceRect { left: 24.0, top: 24.0, width: 360.0, height: 280.0 } }).map_err(|error| error.message)?;
        }
        return Ok(());
    }
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
        workspace_menu::WorkspaceMenuAction::Weather =>
        {
            // 空白菜单没有来源与城市绑定，创建出的格子进入未选来源状态。
            execute_create_weather_tile(app, None, monitor.id.clone())?;
        }
    }
    crate::log_info!("桌面菜单：注册动词执行完成，命令={selected}，显示器={}", monitor.id);
    Ok(())
}

use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};

use crate::contracts::{DesktopHostError, WorkspaceCommand, WorkspaceContextMenuTarget};
use crate::host::DesktopHostRuntime;
use crate::shell::DesktopItemState;
use crate::workspace::WorkspaceState;

pub use application::{get_application_snapshot, set_application_status};
pub use component::{get_component_runtime_snapshot, invoke_component_capability};
pub use desktop_host::{begin_desktop_host_input_drag, end_desktop_host_input_drag, exit_desktop_tools, get_desktop_host_snapshot, publish_desktop_host_readiness, record_desktop_host_input_probe, record_desktop_host_renderer_heartbeat, update_desktop_host_hit_map};
pub(crate) use desktop_item::refresh_desktop_item_snapshot;
pub use desktop_item::{delete_desktop_items, get_desktop_item_snapshot, open_desktop_items, paste_desktop_items, rename_desktop_item, set_desktop_item_clipboard, show_workspace_context_menu, start_desktop_item_change_monitor, start_desktop_item_drag};
pub use notifications::{dismiss_island_message, get_island_messages, publish_message_notification, sync_message_appearance};

/// 普通 Tauri 页面读取与桌面宿主相同的锁屏及前台快照。
#[tauri::command]
pub(crate) fn get_foreground_snapshot() -> crate::platform::windows::refresh_policy::ForegroundSnapshot { crate::platform::windows::refresh_policy::foreground_snapshot() }
pub(crate) use workspace::reconcile_workspace_environment;
pub use workspace::{apply_workspace_command, apply_workspace_interaction, get_workspace_snapshot, inspect_workspace_backup, list_workspace_auto_snapshots, redo_workspace, restore_workspace_backup, undo_workspace, write_workspace_backup};

/// 原生命令的协议来源；同一套命令面按来源收窄，不把宿主能力暴露给其他窗口。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeCommandSource
{
    /// 桌面宿主页面（逐屏宿主与独立岛）。
    DesktopHost,
    /// 设置窗口页面。
    Settings,
}

/// 判断命令是否属于桌面层浮层命令；这些命令只服务宿主页面。
pub(crate) fn is_popup_layer_command(command: &str) -> bool { matches!(command, "open_popup_layer" | "dismiss_popup_layer" | "update_popup_layer_payload") }

/// 判断协议来源能否调用浮层命令；设置窗口只保留只读与设置命令。
pub(crate) fn popup_layer_command_allowed(source: NativeCommandSource) -> bool { source == NativeCommandSource::DesktopHost }

/// 路由独立 Windowed WebView2 transport 到正式应用命令边界。
pub(crate) fn dispatch_native_command(app: &AppHandle, runtime: &DesktopHostRuntime, source: NativeCommandSource, command: &str, arguments: Value) -> Result<Value, Value>
{
    // 置顶弹层只服务桌面层浮层：设置窗口与弹层页面自身都不得替换、关闭或改写宿主请求。
    if is_popup_layer_command(command) && !popup_layer_command_allowed(source)
    {
        return Err(serialize_error(DesktopHostError::invalid_readiness("弹层命令不接受设置窗口调用")));
    }
    // 当前本地 transport 命令的统一 JSON 结果。
    let result: Result<Value, Value> = router::route_island(command, app, runtime, &arguments)
        .or_else(|| router::route_notifications(command, app, runtime, &arguments))
        .or_else(|| router::route_audio(command, app, runtime, &arguments))
        .or_else(|| router::route_music(command, app, runtime, &arguments))
        .or_else(|| router::route_settings(command, app, runtime, &arguments))
        .or_else(|| router::route_weather(command, app, runtime, &arguments))
        .or_else(|| router::route_collect(command, app, runtime, &arguments))
        .or_else(|| router::route_desktop_item(command, app, runtime, &arguments))
        .or_else(|| router::route_folder_portal(command, app, runtime, &arguments))
        .or_else(|| router::route_component(command, app, runtime, &arguments))
        .or_else(|| router::route_popup_layer(command, app, runtime, &arguments))
        .or_else(|| router::route_loopback(command, app, runtime, &arguments))
        .or_else(|| router::route_desktop_host(command, app, runtime, &arguments))
        .or_else(|| router::route_workspace(command, app, runtime, &arguments))
        .or_else(|| router::route_plugin_settings(command, app, runtime, &arguments))
        .or_else(|| router::route_tiles(command, app, runtime, &arguments))
        .or_else(|| router::route_app(command, app, runtime, &arguments))
        .unwrap_or_else(|| Err(serialize_error(DesktopHostError::invalid_readiness(format!("正式宿主命令无效: {command}")))));
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
#[path = "mod.test.rs"]
mod tests;
