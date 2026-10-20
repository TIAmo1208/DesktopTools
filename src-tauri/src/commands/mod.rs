mod application;
pub mod collect;
mod component;
mod desktop_host;
pub mod desktop_item;
pub mod folder_portal;
pub use desktop_item::get_desktop_item_image;
pub use desktop_item::{cancel_desktop_item_drag_preparation, prepare_desktop_item_drag};
pub mod music;
mod notifications;
pub mod settings;
pub mod weather;
mod workspace;
mod workspace_menu;
pub use music::{execute_music_action, get_current_music_session};

/// 新增格子共用的默认展开尺寸。
const DEFAULT_TILE_WIDTH: f64 = 360.0;

/// 新增格子共用的默认展开高度。
const DEFAULT_TILE_HEIGHT: f64 = 280.0;

/// 天气格子的官方组件包身份。
const WEATHER_COMPONENT_ID: &str = "official.weather";

/// 统一创建一个收集箱格子，设置页、桌面菜单和其他入口共用此逻辑。
pub(crate) fn execute_create_collect_box(app: &tauri::AppHandle, monitor_id: String) -> Result<(), String>
{
    use tauri::Manager;
    let runtime = app.state::<std::sync::Arc<DesktopHostRuntime>>();
    let workspace = app.state::<WorkspaceState>();
    let items = app.state::<DesktopItemState>();
    let bounds = workspace.next_component_bounds(&monitor_id, DEFAULT_TILE_WIDTH, DEFAULT_TILE_HEIGHT).map_err(|error| error.message)?;
    workspace::execute_apply_workspace_command(app, &workspace, &items, &runtime, monitor_id.clone(), WorkspaceCommand::CreateComponent { component_id: "official.collect-box".to_string(), bounds, state: json!({}) }).map_err(|error| error.message)?;
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
    use tauri::Emitter;
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
    let settings = app.state::<crate::app::settings::SettingsState>();
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
        if !app.state::<crate::app::settings::SettingsState>().preferences()?.folder_portal_enabled
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

use std::path::PathBuf;

use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};

use crate::component_runtime::ComponentRuntimeState;
use crate::contracts::{ComponentCapability, DesktopHostError, FrontendHostReadiness, HitRegion, WorkspaceCommand, WorkspaceContextMenuTarget, WorkspaceInteractionCommand};
use crate::host::DesktopHostRuntime;
use crate::shell::DesktopItemState;
use crate::workspace::WorkspaceState;

pub use application::{get_application_snapshot, set_application_status};
pub use component::{get_component_runtime_snapshot, invoke_component_capability};
pub use desktop_host::{begin_desktop_host_input_drag, end_desktop_host_input_drag, exit_desktop_tools, get_desktop_host_snapshot, publish_desktop_host_readiness, record_desktop_host_input_probe, record_desktop_host_renderer_heartbeat, update_desktop_host_hit_map};
pub(crate) use desktop_item::refresh_desktop_item_snapshot;
pub use desktop_item::{delete_desktop_items, get_desktop_item_snapshot, open_desktop_items, paste_desktop_items, rename_desktop_item, set_desktop_item_clipboard, show_workspace_context_menu, start_desktop_item_change_monitor, start_desktop_item_drag};
pub use notifications::{publish_message_notification, sync_message_appearance};

/// 普通 Tauri 页面读取与桌面宿主相同的锁屏及前台快照。
#[tauri::command]
pub(crate) fn get_foreground_snapshot() -> crate::platform::windows::refresh_policy::ForegroundSnapshot { crate::platform::windows::refresh_policy::foreground_snapshot() }
pub(crate) use workspace::reconcile_workspace_environment;
pub use workspace::{apply_workspace_command, apply_workspace_interaction, get_workspace_snapshot, inspect_workspace_backup, list_workspace_auto_snapshots, redo_workspace, restore_workspace_backup, undo_workspace, write_workspace_backup};

/// 路由独立 Windowed WebView2 transport 到正式应用命令边界。
pub(crate) fn dispatch_native_command(app: &AppHandle, runtime: &DesktopHostRuntime, command: &str, arguments: Value) -> Result<Value, Value>
{
    use tauri::Emitter;
    // 当前本地 transport 命令的统一 JSON 结果。
    let result: Result<Value, Value> = match command
    {
        "get_current_music_session" => music::get_current_music_session().map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message))),
        "request_music_convergence" =>
        {
            crate::platform::windows::music::request_convergence();
            serialize_result(())
        }
        "execute_music_action" =>
        {
            let action: String = command_argument(&arguments, "action", "媒体动作参数无效").map_err(serialize_error)?; // 原生桌面通道与 Tauri 使用相同的动作入口。
            let expected_session: String = command_argument(&arguments, "expectedSession", "媒体会话参数无效").map_err(serialize_error)?; // 用户点击时的会话代次。
            music::execute_music_action(action, expected_session).map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message))).and_then(serialize_result)
        }
        "relay_workspace_drag" =>
        {
            let payload: Value = command_argument(&arguments, "payload", "跨屏拖动参数无效").map_err(serialize_error)?; // 仅允许有界对象消息。
            let maximum_bytes = crate::platform::windows::drag_payload_limit(&payload); // 完整格子与几何确认在两层使用同一预算。
            if !payload.is_object() || payload.to_string().len() > maximum_bytes
            {
                return Err(serialize_error(DesktopHostError::invalid_readiness("跨屏拖动消息无效")));
            }
            if payload.get("phase").and_then(Value::as_str) == Some("ole-preview")
            {
                // 渲染确认只更新受锁保护的几何数据，不排到正在运行 DoDragDrop 的 UI 事件队列后面。
                return crate::platform::windows::relay_workspace_drag(payload).map_err(|error| serialize_error(DesktopHostError::invalid_readiness(error))).and_then(serialize_result);
            }
            app.run_on_main_thread(move || {
                if let Err(error) = crate::platform::windows::relay_workspace_drag(payload)
                {
                    crate::log_warn!("宿主：转发跨屏拖动失败，错误={error}");
                }
            })
            .map_err(|error| serialize_error(DesktopHostError::platform_failed(&error)))?;
            serialize_result(())
        }
        "get_windows_accent_color" => app.state::<crate::platform::windows::settings::WindowsAccentMonitor>().color().map_err(serialize_error).and_then(serialize_result),
        "get_preferences" => app.state::<crate::app::settings::SettingsState>().preferences().map_err(serialize_error).and_then(serialize_result),
        "get_weather_forecast" =>
        {
            let source: crate::weather::Source = command_argument(&arguments, "source", "天气来源参数无效").map_err(serialize_error)?;
            let city_id: String = command_argument(&arguments, "cityId", "天气城市标识无效").map_err(serialize_error)?;
            app.state::<crate::weather::WeatherState>().forecast_for_city(source, &city_id).map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message))).and_then(serialize_result)
        }
        "list_weather_sources" =>
        {
            // 桌面宿主只读已配置来源列表，不接触草稿会话。
            weather::list_weather_sources(app.clone()).map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message))).and_then(serialize_result)
        }
        "get_weather_cities" =>
        {
            let source: crate::weather::Source = command_argument(&arguments, "source", "天气来源参数无效").map_err(serialize_error)?;
            app.state::<crate::weather::WeatherState>().cities(source).map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message))).and_then(serialize_result)
        }
        "add_weather_city" =>
        {
            let source: crate::weather::Source = command_argument(&arguments, "source", "天气来源参数无效").map_err(serialize_error)?;
            let city: crate::weather::City = command_argument(&arguments, "city", "天气城市参数无效").map_err(serialize_error)?;
            add_weather_city(app, source, &city).map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message))).and_then(serialize_result)
        }
        "remove_weather_city" =>
        {
            let source: crate::weather::Source = command_argument(&arguments, "source", "天气来源参数无效").map_err(serialize_error)?;
            let city_id: String = command_argument(&arguments, "cityId", "天气城市标识无效").map_err(serialize_error)?;
            remove_weather_city(app, source, &city_id).map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message))).and_then(serialize_result)
        }
        "validate_weather_city" =>
        {
            let source: crate::weather::Source = command_argument(&arguments, "source", "天气来源参数无效").map_err(serialize_error)?;
            let city: crate::weather::City = command_argument(&arguments, "city", "天气城市参数无效").map_err(serialize_error)?;
            app.state::<crate::weather::WeatherState>().validate_for_source(source, &city).map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message))).and_then(|()| serialize_result(city))
        }
        "create_weather_tile" =>
        {
            let source: crate::weather::Source = command_argument(&arguments, "source", "天气来源参数无效").map_err(serialize_error)?;
            let city_id: String = command_argument(&arguments, "cityId", "天气城市标识无效").map_err(serialize_error)?;
            let monitor_id: Option<String> = arguments.get("monitorId").cloned().map(serde_json::from_value).transpose().map_err(|error| serialize_error(DesktopHostError::invalid_readiness(error.to_string())))?;
            weather::create_weather_tile(app.clone(), source, city_id, monitor_id).map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message))).and_then(serialize_result)
        }
        "subscribe_weather" =>
        {
            let instance = command_argument(&arguments, "instanceId", "天气实例标识无效").map_err(serialize_error)?;
            let monitor = command_argument(&arguments, "monitorId", "天气显示器标识无效").map_err(serialize_error)?;
            let source: crate::weather::Source = command_argument(&arguments, "source", "天气来源参数无效").map_err(serialize_error)?;
            let city_id: String = command_argument(&arguments, "cityId", "天气城市标识无效").map_err(serialize_error)?;
            weather::subscribe_weather(app.clone(), instance, monitor, source, city_id).map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message))).and_then(serialize_result)
        }
        "unsubscribe_weather" =>
        {
            let subscription = command_argument(&arguments, "subscription", "天气订阅标识无效").map_err(serialize_error)?;
            weather::unsubscribe_weather(subscription);
            serialize_result(())
        }
        "search_weather_cities" =>
        {
            let source: crate::weather::Source = command_argument(&arguments, "source", "天气来源参数无效").map_err(serialize_error)?;
            let query: String = command_argument(&arguments, "query", "城市搜索参数无效").map_err(serialize_error)?;
            app.state::<crate::weather::WeatherState>().search_saved(source, query).map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message))).and_then(serialize_result)
        }
        "get_collect_items" =>
        {
            let preferences = app.state::<crate::app::settings::SettingsState>().preferences().map_err(serialize_error)?;
            let path = crate::app::settings::resolve_config_directory(&preferences.config_directory).map_err(serialize_error)?;
            crate::collect::list(&path).map_err(|error| serialize_error(DesktopHostError::invalid_readiness(error.to_string()))).and_then(serialize_result)
        }
        "read_collect_image" =>
        {
            let name: String = command_argument(&arguments, "name", "收集项名称无效").map_err(serialize_error)?; // 两个通道使用相同的参数校验。
            let reference: String = command_argument(&arguments, "reference", "图片引用无效").map_err(serialize_error)?; // 编辑器原图引用。
            let pixels: Option<u32> = serde_json::from_value(arguments.get("pixels").cloned().unwrap_or(Value::Null)).map_err(|error| serialize_error(error.to_string()))?; // 列表请求有界缩略图。
            collect::execute_read_collect_image(app, &name, &reference, pixels).map_err(serialize_error).and_then(serialize_result)
        }
        "prepare_collect_drag" => command_argument(&arguments, "names", "收集项选择无效").map_err(serialize_error).and_then(|names| desktop_item::execute_prepare_collect_drag(app, names).map_err(serialize_error)).and_then(serialize_result),
        "prepare_collect_export" =>
        {
            let names: Vec<String> = command_argument(&arguments, "names", "收集项选择无效").map_err(serialize_error)?;
            let preferences = app.state::<crate::app::settings::SettingsState>().preferences().map_err(serialize_error)?;
            let path = crate::app::settings::resolve_config_directory(&preferences.config_directory).map_err(serialize_error)?;
            crate::collect::prepare_export(&path, &names).map_err(serialize_error).and_then(serialize_result)
        }
        "receive_collect_items" =>
        {
            let names: Vec<String> = command_argument(&arguments, "names", "收集项选择无效").map_err(serialize_error)?;
            let target_name: Option<String> = arguments.get("targetName").cloned().map(serde_json::from_value).transpose().map_err(|error| serialize_error(error.to_string()))?;
            let preferences = app.state::<crate::app::settings::SettingsState>().preferences().map_err(serialize_error)?;
            let path = crate::app::settings::resolve_config_directory(&preferences.config_directory).map_err(serialize_error)?;
            collect::with_collect_editor_state(app, &crate::collect::directory(&path), target_name.as_deref(), |editing| crate::collect::receive_items(&path, target_name.as_deref(), &names, editing)).map_err(serialize_error).and_then(|result| {
                let _ = app.emit("collect-items-changed", ());
                serialize_result(result)
            })
        }
        "open_collect_link" =>
        {
            let url = command_argument(&arguments, "url", "链接地址无效").map_err(serialize_error)?; // 桌面宿主与编辑窗共用打开规则。
            collect::open_collect_link(url).map_err(serialize_error).and_then(serialize_result)
        }
        "open_collect_item" =>
        {
            let name: String = command_argument(&arguments, "name", "收集项名称无效").map_err(serialize_error)?;
            collect::execute_open_collect_item(app, name).map_err(serialize_error).and_then(serialize_result)
        }
        "create_collect_item" =>
        {
            let content: String = command_argument(&arguments, "content", "收集项内容无效").map_err(serialize_error)?;
            let preferences = app.state::<crate::app::settings::SettingsState>().preferences().map_err(serialize_error)?;
            let path = crate::app::settings::resolve_config_directory(&preferences.config_directory).map_err(serialize_error)?;
            crate::collect::create(&path, &content).map_err(|error| serialize_error(DesktopHostError::invalid_readiness(error.to_string()))).and_then(|item| {
                let _ = app.emit("collect-items-changed", ());
                serialize_result(item)
            })
        }
        "save_collect_item" =>
        {
            let name: String = command_argument(&arguments, "name", "收集项名称无效").map_err(serialize_error)?;
            let content: String = command_argument(&arguments, "content", "收集项内容无效").map_err(serialize_error)?;
            let preferences = app.state::<crate::app::settings::SettingsState>().preferences().map_err(serialize_error)?;
            let path = crate::app::settings::resolve_config_directory(&preferences.config_directory).map_err(serialize_error)?;
            crate::collect::save(&path, &name, &content).map_err(|error| serialize_error(DesktopHostError::invalid_readiness(error.to_string()))).and_then(|_| {
                let _ = app.emit("collect-items-changed", ());
                serialize_result(())
            })
        }
        "rename_collect_item" =>
        {
            let old_name: String = command_argument(&arguments, "oldName", "原收集项名称无效").map_err(serialize_error)?;
            let new_name: String = command_argument(&arguments, "newName", "新收集项名称无效").map_err(serialize_error)?;
            let draft: Option<String> = serde_json::from_value(arguments.get("draft").cloned().unwrap_or(Value::Null)).map_err(|error| serialize_error(error.to_string()))?; // 可选未保存草稿仍须严格校验类型。
            collect::execute_rename_collect_item(app.clone(), old_name, new_name, draft).map_err(serialize_error).and_then(serialize_result)
        }
        "import_collect_resource" =>
        {
            let name: String = command_argument(&arguments, "name", "收集项名称无效").map_err(serialize_error)?;
            let source: String = command_argument(&arguments, "source", "图片路径无效").map_err(serialize_error)?;
            let mode: String = command_argument(&arguments, "mode", "图片收入方式无效").map_err(serialize_error)?;
            let preferences = app.state::<crate::app::settings::SettingsState>().preferences().map_err(serialize_error)?;
            let path = crate::app::settings::resolve_config_directory(&preferences.config_directory).map_err(serialize_error)?;
            crate::collect::import_resource(&path, &name, std::path::Path::new(&source), &mode).map_err(|error| serialize_error(DesktopHostError::invalid_readiness(error.to_string()))).and_then(serialize_result)
        }
        "write_collect_resource" =>
        {
            let name: String = command_argument(&arguments, "name", "收集项名称无效").map_err(serialize_error)?;
            let file_name: String = command_argument(&arguments, "fileName", "图片文件名无效").map_err(serialize_error)?;
            let bytes: Vec<u8> = command_argument(&arguments, "bytes", "图片内容无效").map_err(serialize_error)?;
            let preferences = app.state::<crate::app::settings::SettingsState>().preferences().map_err(serialize_error)?;
            let path = crate::app::settings::resolve_config_directory(&preferences.config_directory).map_err(serialize_error)?;
            crate::collect::write_resource_bytes(&path, &name, &file_name, &bytes).map_err(|error| serialize_error(DesktopHostError::invalid_readiness(error.to_string()))).and_then(serialize_result)
        }
        "import_collect_rendered_resource" =>
        {
            let name: String = command_argument(&arguments, "name", "收集项名称无效").map_err(serialize_error)?;
            let reference: String = command_argument(&arguments, "reference", "图片引用无效").map_err(serialize_error)?;
            let mode: String = command_argument(&arguments, "mode", "图片收入方式无效").map_err(serialize_error)?;
            let preferences = app.state::<crate::app::settings::SettingsState>().preferences().map_err(serialize_error)?;
            let path = crate::app::settings::resolve_config_directory(&preferences.config_directory).map_err(serialize_error)?;
            crate::collect::import_rendered_resource(&path, &name, &reference, &mode).map_err(|error| serialize_error(DesktopHostError::invalid_readiness(error.to_string()))).and_then(serialize_result)
        }
        "set_collect_item_pinned" =>
        {
            let name: String = command_argument(&arguments, "name", "收集项名称无效").map_err(serialize_error)?;
            let pinned: bool = command_argument(&arguments, "pinned", "置顶状态无效").map_err(serialize_error)?;
            let preferences = app.state::<crate::app::settings::SettingsState>().preferences().map_err(serialize_error)?;
            let path = crate::app::settings::resolve_config_directory(&preferences.config_directory).map_err(serialize_error)?;
            crate::collect::set_pinned(&path, &name, pinned).map_err(|error| serialize_error(DesktopHostError::invalid_readiness(error.to_string()))).and_then(|_| {
                let _ = app.emit("collect-items-changed", ());
                serialize_result(())
            })
        }
        "delete_collect_item" =>
        {
            let name: String = command_argument(&arguments, "name", "收集项名称无效").map_err(serialize_error)?;
            let preferences = app.state::<crate::app::settings::SettingsState>().preferences().map_err(serialize_error)?;
            let path = crate::app::settings::resolve_config_directory(&preferences.config_directory).map_err(serialize_error)?;
            crate::collect::delete(&path, &name).map_err(|error| serialize_error(DesktopHostError::invalid_readiness(error.to_string()))).and_then(|_| {
                let _ = app.emit("collect-items-changed", json!({ "deletedName": name })); // 主动删除不生成不可用占位。
                serialize_result(())
            })
        }
        "restore_collect_item" =>
        {
            let preferences = app.state::<crate::app::settings::SettingsState>().preferences().map_err(serialize_error)?;
            let path = crate::app::settings::resolve_config_directory(&preferences.config_directory).map_err(serialize_error)?;
            crate::collect::restore_latest(&path).map_err(|error| serialize_error(DesktopHostError::invalid_readiness(error.to_string()))).and_then(|item| {
                let _ = app.emit("collect-items-changed", ());
                serialize_result(item)
            })
        }
        "open_collect_editor" =>
        {
            let name: String = command_argument(&arguments, "name", "收集项名称无效").map_err(serialize_error)?;
            collect::execute_open_collect_editor(app.clone(), name).map_err(serialize_error).and_then(serialize_result)
        }
        "watch_folder_portal" =>
        {
            let root: String = command_argument(&arguments, "rootPath", "门户根路径无效").map_err(serialize_error)?; // 校验绑定根。
            let current: String = command_argument(&arguments, "currentPath", "门户当前目录无效").map_err(serialize_error)?; // 当前导航路径。
            let monitor: String = command_argument(&arguments, "monitorId", "显示器标识无效").map_err(serialize_error)?; // 用于宿主销毁时清理。
            runtime.validate_monitor_id(&monitor).map_err(serialize_error)?;
            let (_, path) = folder_portal::checked_directory(&root, &current).map_err(serialize_error)?; // 复用目录安全边界。
            crate::platform::windows::directory_watch::start(app.clone(), monitor, path).map_err(serialize_error).and_then(serialize_result)
        }
        "unwatch_folder_portal" =>
        {
            let id: u64 = command_argument(&arguments, "subscription", "目录监听标识无效").map_err(serialize_error)?; // 仅取消本应用监听。
            crate::platform::windows::directory_watch::stop(id);
            serialize_result(())
        }
        "subscribe_component_time" =>
        {
            let instance: String = command_argument(&arguments, "instanceId", "组件实例标识无效").map_err(serialize_error)?; // 需要校验的真实组件实例。
            let monitor: String = command_argument(&arguments, "monitorId", "显示器标识无效").map_err(serialize_error)?; // 当前呈现宿主。
            runtime.validate_monitor_id(&monitor).map_err(serialize_error)?;
            component::execute_invoke_component_capability(&app.state::<ComponentRuntimeState>(), &app.state::<WorkspaceState>(), instance, crate::contracts::ComponentCapability::ReadHostTime).map_err(serialize_error)?;
            serialize_result(ComponentRuntimeState::subscribe_time(monitor))
        }
        "unsubscribe_component_time" =>
        {
            let id: u64 = command_argument(&arguments, "subscription", "时间订阅标识无效").map_err(serialize_error)?; // 独立生命周期身份。
            ComponentRuntimeState::unsubscribe_time(id);
            serialize_result(())
        }
        "get_desktop_refresh_policy" =>
        {
            let monitor_id: String = command_argument(&arguments, "monitorId", "显示器标识无效").map_err(serialize_error)?; // 当前页面的显示器身份。
            let snapshot = runtime.snapshot().map_err(serialize_error)?; // 暂停只适用于已完成接管的宿主。
            runtime.validate_monitor_id(&monitor_id).map_err(serialize_error)?;
            let preferences = app.state::<crate::app::settings::SettingsState>().preferences().map_err(serialize_error)?; // 每次读取已保存的开关。
            let foreground = crate::platform::windows::refresh_policy::foreground_snapshot(); // 页面与原生消息共享同一前台事实。
            serialize_result(serde_json::json!({ "snapshot": foreground, "committed": snapshot.committed, "pauseOnFocus": preferences.pause_on_focus, "pauseOnFullscreen": preferences.pause_on_fullscreen }))
        }
        "refresh_foreground_snapshot" =>
        {
            crate::platform::windows::refresh_policy::request_sample();
            serialize_result(())
        }
        "record_renderer_diagnostic" =>
        {
            let fields = ["origin", "url", "stage", "message"].map(|key| arguments.get(key).and_then(Value::as_str).unwrap_or("").chars().take(4096).collect::<String>().replace(['\r', '\n'], " ")); // 有界单行诊断，不执行页面内容。
            crate::log_error!("宿主前端：来源={}，页面={}，阶段={}，错误={}", fields[0], fields[1], fields[2], fields[3]);
            serialize_result(())
        }
        "record_desktop_refresh_state" =>
        {
            let monitor_id: String = command_argument(&arguments, "monitorId", "显示器标识无效").map_err(serialize_error)?; // 已校验显示器身份。
            let paused: bool = command_argument(&arguments, "paused", "暂停状态无效").map_err(serialize_error)?; // 页面实际应用的暂停状态。
            runtime.validate_monitor_id(&monitor_id).map_err(serialize_error)?;
            crate::platform::windows::refresh_policy::set_paused(&monitor_id, paused);
            ComponentRuntimeState::wake_time_source();
            let _ = app.emit("desktop-refresh-policy-changed", ());
            crate::log_info!("桌面刷新：显示器={}，状态={}", monitor_id, if paused { "已暂停" } else { "已恢复" });
            serialize_result(())
        }
        "sync_message_appearance" =>
        {
            let preferences = command_argument(&arguments, "preferences", "消息外观偏好无效").map_err(serialize_error)?; // 页面已解析的当前程序偏好。
            let dark = command_argument(&arguments, "dark", "消息深浅色无效").map_err(serialize_error)?;
            let animations = command_argument(&arguments, "animationsEnabled", "消息动画状态无效").map_err(serialize_error)?;
            app.state::<std::sync::Arc<crate::notifications::ToastManager>>().sync_appearance(preferences, dark, animations);
            serialize_result(())
        }
        "publish_message_notification" =>
        {
            let message: crate::notifications::ToastRequest = command_argument(&arguments, "message", "消息参数无效").map_err(serialize_error)?;
            app.state::<std::sync::Arc<crate::notifications::ToastManager>>().inner().publish(app, message).map_err(serialize_error).and_then(serialize_result)
        }
        "open_folder_portal_path" =>
        {
            let root_path = command_argument(&arguments, "rootPath", "门户根路径参数无效").map_err(serialize_error)?;
            let path = command_argument(&arguments, "path", "门户路径参数无效").map_err(serialize_error)?;
            folder_portal::open_folder_portal_path(root_path, path).map_err(serialize_error).and_then(serialize_result)
        }
        "delete_folder_portal_items" =>
        {
            let root_path = command_argument(&arguments, "rootPath", "门户根路径参数无效").map_err(serialize_error)?;
            let current_path = command_argument(&arguments, "currentPath", "门户当前路径参数无效").map_err(serialize_error)?;
            let names = command_argument(&arguments, "names", "门户项目参数无效").map_err(serialize_error)?;
            folder_portal::execute_delete_folder_portal_items(&app.state::<DesktopItemState>(), root_path, current_path, names).map_err(serialize_error).and_then(serialize_result)
        }
        "set_folder_portal_clipboard" =>
        {
            let root_path = command_argument(&arguments, "rootPath", "门户根路径参数无效").map_err(serialize_error)?;
            let current_path = command_argument(&arguments, "currentPath", "门户当前路径参数无效").map_err(serialize_error)?;
            let names = command_argument(&arguments, "names", "门户项目参数无效").map_err(serialize_error)?;
            let cut = command_argument(&arguments, "cut", "剪贴板操作参数无效").map_err(serialize_error)?;
            folder_portal::set_folder_portal_clipboard(app.state::<DesktopItemState>(), root_path, current_path, names, cut).map_err(serialize_error).and_then(serialize_result)
        }
        "paste_folder_portal_items" =>
        {
            let root_path = command_argument(&arguments, "rootPath", "门户根路径参数无效").map_err(serialize_error)?;
            let current_path = command_argument(&arguments, "currentPath", "门户当前路径参数无效").map_err(serialize_error)?;
            folder_portal::paste_folder_portal_items(app.state::<DesktopItemState>(), root_path, current_path).map_err(serialize_error).and_then(serialize_result)
        }
        "show_folder_portal_menu" =>
        {
            let request = command_argument(&arguments, "request", "门户菜单参数无效").map_err(serialize_error)?; // 两种传输使用相同请求结构。
            folder_portal::execute_show_folder_portal_menu(&app.state::<DesktopItemState>(), runtime, request).map_err(serialize_error).and_then(serialize_result)
        }
        "rename_folder_portal_item" =>
        {
            let request = command_argument(&arguments, "request", "门户重命名参数无效").map_err(serialize_error)?;
            folder_portal::execute_rename_folder_portal_item(&app.state::<DesktopItemState>(), request).map_err(serialize_error).and_then(serialize_result)
        }
        "get_desktop_item_snapshot" => desktop_item::execute_get_desktop_item_snapshot(app, &app.state::<DesktopItemState>(), &app.state::<WorkspaceState>(), runtime).map_err(serialize_error).and_then(serialize_result),
        "get_desktop_item_image" =>
        {
            let item_id = command_argument(&arguments, "itemId", "系统图像项目参数无效").map_err(serialize_error)?; // 当前快照身份。
            let pixels = command_argument(&arguments, "pixels", "系统图像尺寸参数无效").map_err(serialize_error)?; // 后端继续检查范围。
            app.state::<DesktopItemState>().image(item_id, pixels).map_err(serialize_error).and_then(serialize_result)
        }
        "get_folder_portal_snapshot" =>
        {
            let root_path: String = command_argument(&arguments, "rootPath", "门户绑定目录参数无效").map_err(serialize_error)?;
            let current_path: Option<String> = arguments.get("currentPath").cloned().map(serde_json::from_value).transpose().map_err(|_| serialize_error(DesktopHostError::invalid_readiness("门户当前目录参数无效")))?;
            folder_portal::get_folder_portal_snapshot(root_path, current_path).map_err(|error| serialize_error(DesktopHostError::invalid_readiness(error))).and_then(serialize_result)
        }
        "get_component_runtime_snapshot" => component::execute_get_component_runtime_snapshot(app, &app.state::<ComponentRuntimeState>(), &app.state::<WorkspaceState>()).map_err(serialize_error).and_then(serialize_result),
        "get_current_component_runtime_snapshot" => component::component_runtime_snapshot(&app.state::<ComponentRuntimeState>(), &app.state::<WorkspaceState>()).map_err(serialize_error).and_then(serialize_result),
        "get_current_component_host_event" => component::current_component_host_event(&app.state::<ComponentRuntimeState>()).map_err(serialize_error).and_then(serialize_result),
        "invoke_component_capability" =>
        {
            let instance_id: String = command_argument(&arguments, "instanceId", "命令参数无效").map_err(serialize_error)?; // 已解码的 instanceId。
            let capability: ComponentCapability = command_argument(&arguments, "capability", "命令参数无效").map_err(serialize_error)?; // 已解码的 capability。
            component::execute_invoke_component_capability(&app.state::<ComponentRuntimeState>(), &app.state::<WorkspaceState>(), instance_id, capability).map_err(serialize_error).and_then(serialize_result)
        }
        "get_current_desktop_item_snapshot" => desktop_item::read_desktop_item_snapshot(&app.state::<DesktopItemState>()).map_err(serialize_error).and_then(serialize_result),
        "open_desktop_items" => command_argument(&arguments, "itemIds", "桌面项目打开参数无效").map_err(serialize_error).and_then(|item_ids: Vec<String>| app.state::<DesktopItemState>().open(&item_ids).map_err(serialize_error)).and_then(serialize_result),
        "rename_desktop_item" =>
        {
            let item_id: String = command_argument(&arguments, "itemId", "命令参数无效").map_err(serialize_error)?; // 已解码的 itemId。
            let new_name: String = command_argument(&arguments, "newName", "命令参数无效").map_err(serialize_error)?; // 已解码的 newName。
            desktop_item::execute_rename_desktop_item(&app.state::<DesktopItemState>(), &app.state::<WorkspaceState>(), item_id, new_name).map_err(serialize_error).and_then(serialize_result)
        }
        "delete_desktop_items" =>
        {
            let item_ids: Vec<String> = command_argument(&arguments, "itemIds", "命令参数无效").map_err(serialize_error)?; // 已解码的 itemIds。
            let permanent: bool = command_argument(&arguments, "permanent", "命令参数无效").map_err(serialize_error)?; // 已解码的 permanent。
            desktop_item::execute_delete_desktop_items(&app.state::<DesktopItemState>(), &app.state::<WorkspaceState>(), item_ids, permanent).map_err(serialize_error).and_then(serialize_result)
        }
        "set_desktop_item_clipboard" =>
        {
            command_argument(&arguments, "itemIds", "桌面项目剪贴板参数无效")
                .map_err(serialize_error)
                .and_then(|item_ids: Vec<String>| {
                    let cut = arguments.get("cut").and_then(Value::as_bool).ok_or_else(|| serialize_error(DesktopHostError::invalid_readiness("缺少 cut")))?; // 是否使用剪切效果。
                    app.state::<DesktopItemState>().set_clipboard(&item_ids, cut).map_err(serialize_error)
                })
                .and_then(serialize_result)
        }
        "paste_desktop_items" => desktop_item::execute_paste_desktop_items(&app.state::<DesktopItemState>(), &app.state::<WorkspaceState>()).map_err(serialize_error).and_then(serialize_result),
        "prepare_desktop_item_drag" => command_argument(&arguments, "itemIds", "桌面项目拖放参数无效").map_err(serialize_error).and_then(|item_ids: Vec<String>| app.state::<DesktopItemState>().prepare_drag(&item_ids).map_err(serialize_error)).and_then(serialize_result),
        "prepare_folder_portal_drag" =>
        {
            let root: String = command_argument(&arguments, "rootPath", "门户根路径无效").map_err(serialize_error)?;
            let current: String = command_argument(&arguments, "currentPath", "门户当前目录无效").map_err(serialize_error)?;
            let names: Vec<String> = command_argument(&arguments, "names", "门户选择无效").map_err(serialize_error)?;
            let references = folder_portal::checked_item_paths(&root, &current, &names).map_err(serialize_error)?;
            app.state::<DesktopItemState>().prepare_portal_drag(references).map_err(serialize_error).and_then(serialize_result)
        }
        "cancel_desktop_item_drag_preparation" => command_argument(&arguments, "token", "拖动凭据无效").map_err(serialize_error).and_then(|token| app.state::<DesktopItemState>().cancel_prepared_drag(token).map_err(serialize_error)).and_then(serialize_result),
        "start_desktop_item_drag" =>
        {
            let token = command_argument(&arguments, "token", "拖动凭据无效").map_err(serialize_error)?; // 单次启动凭据。
            let pointer_offset = command_argument(&arguments, "pointerOffset", "拖动锚点偏移无效").map_err(serialize_error)?; // 源表面 DIP 偏移。
            desktop_item::execute_start_desktop_item_drag(app, &app.state::<DesktopItemState>(), token, pointer_offset).map_err(serialize_error).and_then(serialize_result)
        }
        "show_workspace_context_menu" =>
        {
            let monitor_id: String = command_argument(&arguments, "monitorId", "命令参数无效").map_err(serialize_error)?; // 已解码的 monitorId。
            let target: WorkspaceContextMenuTarget = command_argument(&arguments, "target", "命令参数无效").map_err(serialize_error)?; // 已解码的 target。
            let client_x: f64 = command_argument(&arguments, "clientX", "命令参数无效").map_err(serialize_error)?; // 已解码的 clientX。
            let client_y: f64 = command_argument(&arguments, "clientY", "命令参数无效").map_err(serialize_error)?; // 已解码的 clientY。
            desktop_item::execute_show_workspace_context_menu(app, &app.state::<DesktopItemState>(), &app.state::<WorkspaceState>(), runtime, monitor_id, target, client_x, client_y).map_err(serialize_error).and_then(serialize_result)
        }
        "get_desktop_host_snapshot" => desktop_host::read_desktop_host_snapshot(runtime).map_err(serialize_error).and_then(serialize_result),
        "exit_desktop_tools" =>
        {
            desktop_host::request_exit(app);
            serialize_result(())
        }
        "record_desktop_host_renderer_heartbeat" => command_argument(&arguments, "monitorId", "显示器标识无效").map_err(serialize_error).and_then(|monitor_id: String| desktop_host::record_renderer_heartbeat(runtime, &monitor_id).map_err(serialize_error)).and_then(serialize_result),
        "get_workspace_snapshot" | "get_current_workspace_snapshot" => workspace::read_workspace_snapshot(&app.state::<WorkspaceState>()).map_err(serialize_error).and_then(serialize_result),
        "write_workspace_backup" => command_argument(&arguments, "path", "工作区备份路径无效").map_err(serialize_error).and_then(|path: String| app.state::<WorkspaceState>().write_manual_backup(&PathBuf::from(path)).map_err(serialize_error)).and_then(serialize_result),
        "inspect_workspace_backup" => command_argument(&arguments, "path", "工作区备份路径无效").map_err(serialize_error).and_then(|path: String| app.state::<WorkspaceState>().inspect_backup(&PathBuf::from(path)).map_err(serialize_error)).and_then(serialize_result),
        "list_workspace_auto_snapshots" => app.state::<WorkspaceState>().auto_snapshot_summaries().map_err(serialize_error).and_then(serialize_result),
        "restore_workspace_backup" =>
        {
            let path: String = command_argument(&arguments, "path", "命令参数无效").map_err(serialize_error)?; // 已解码的 path。
            workspace::execute_restore_workspace_backup(app, &app.state::<WorkspaceState>(), &app.state::<DesktopItemState>(), runtime, path).map_err(serialize_error).and_then(serialize_result)
        }
        "undo_workspace" => workspace::apply_workspace_history(app, &app.state::<WorkspaceState>(), &app.state::<DesktopItemState>(), runtime, false).map_err(serialize_error).and_then(serialize_result),
        "redo_workspace" => workspace::apply_workspace_history(app, &app.state::<WorkspaceState>(), &app.state::<DesktopItemState>(), runtime, true).map_err(serialize_error).and_then(serialize_result),
        "apply_workspace_command" =>
        {
            let monitor_id: String = command_argument(&arguments, "monitorId", "命令参数无效").map_err(serialize_error)?; // 已解码的 monitorId。
            let command: WorkspaceCommand = command_argument(&arguments, "command", "命令参数无效").map_err(serialize_error)?; // 已解码的 command。
            workspace::execute_apply_workspace_command(app, &app.state::<WorkspaceState>(), &app.state::<DesktopItemState>(), runtime, monitor_id, command).map_err(serialize_error).and_then(serialize_result)
        }
        "create_collect_box" => create_collect_box(app.clone()).map_err(serialize_error).and_then(serialize_result),
        "open_settings_window" => open_settings_window(app.clone()).map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message))).and_then(serialize_result), // 天气格子在原生宿主中请求显示设置窗口。
        "apply_workspace_interaction" =>
        {
            let monitor_id: String = command_argument(&arguments, "monitorId", "命令参数无效").map_err(serialize_error)?; // 已解码的 monitorId。
            let command: WorkspaceInteractionCommand = command_argument(&arguments, "command", "命令参数无效").map_err(serialize_error)?; // 已解码的 command。
            workspace::execute_apply_workspace_interaction(app, &app.state::<WorkspaceState>(), runtime, monitor_id, command).map_err(serialize_error).and_then(serialize_result)
        }
        "publish_desktop_host_readiness" =>
        {
            let report: FrontendHostReadiness = command_argument(&arguments, "report", "宿主参数无效").map_err(serialize_error)?; // 已校验参数。
            desktop_host::execute_publish_desktop_host_readiness(app, runtime, report).map_err(serialize_error).and_then(serialize_result)
        }
        "update_desktop_host_hit_map" =>
        {
            let monitor_id: String = command_argument(&arguments, "monitorId", "宿主参数无效").map_err(serialize_error)?; // 已校验参数。
            let hit_regions: Vec<HitRegion> = command_argument(&arguments, "hitRegions", "宿主参数无效").map_err(serialize_error)?; // 已校验参数。
            desktop_host::execute_update_desktop_host_hit_map(app, runtime, monitor_id, hit_regions).map_err(serialize_error).and_then(serialize_result)
        }
        "record_desktop_host_input_probe" =>
        {
            let monitor_id: String = command_argument(&arguments, "monitorId", "宿主参数无效").map_err(serialize_error)?; // 已校验参数。
            let item_id: String = command_argument(&arguments, "itemId", "宿主参数无效").map_err(serialize_error)?; // 已校验参数。
            desktop_host::execute_record_desktop_host_input_probe(app, runtime, monitor_id, item_id).map_err(serialize_error).and_then(serialize_result)
        }
        "begin_desktop_host_input_drag" =>
        {
            let monitor_id: String = command_argument(&arguments, "monitorId", "宿主参数无效").map_err(serialize_error)?; // 已校验参数。
            desktop_host::execute_begin_desktop_host_input_drag(app, runtime, monitor_id).map_err(serialize_error).and_then(serialize_result)
        }
        "end_desktop_host_input_drag" =>
        {
            let monitor_id: String = command_argument(&arguments, "monitorId", "宿主参数无效").map_err(serialize_error)?; // 已校验参数。
            desktop_host::execute_end_desktop_host_input_drag(app, runtime, monitor_id).map_err(serialize_error).and_then(serialize_result)
        }
        _ => Err(serialize_error(DesktopHostError::invalid_readiness(format!("正式宿主命令无效: {command}")))),
    };
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
