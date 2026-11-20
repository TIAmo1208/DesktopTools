//! 命令路由：按领域分组的 transport 路由表；来源白名单、参数校验与序列化仍在边界与领域函数内。

use std::path::PathBuf;

use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};

use crate::component_runtime::ComponentRuntimeState;
use crate::contracts::{ComponentCapability, DesktopHostError, FrontendHostReadiness, HitRegion, WorkspaceCommand, WorkspaceContextMenuTarget, WorkspaceInteractionCommand};
use crate::host::DesktopHostRuntime;
use crate::shell::DesktopItemState;
use crate::workspace::WorkspaceState;

use super::{STARTED_EVENT_SENT, add_weather_city, command_argument, create_collect_box, create_tile_to_desktop, enable_feature, open_settings_window, remove_weather_city, serialize_error, serialize_result};
use super::{audio, collect, component, desktop_host, desktop_item, folder_portal, island, loopback, music, notifications, weather, workspace};

/// island 领域命令。
#[allow(unused_variables)]
pub(crate) fn route_island(command: &str, app: &AppHandle, runtime: &DesktopHostRuntime, arguments: &Value) -> Option<Result<Value, Value>>
{
    match command
    {
        "get_island_snapshot" => Some(island::execute_get_island_snapshot(app).map_err(serialize_error).and_then(serialize_result)),
        "set_island_state" => Some((|| -> Result<Value, Value> {
            let request = command_argument(&arguments, "request", "岛状态参数无效").map_err(serialize_error)?;
            island::execute_set_island_state(app, request).map_err(serialize_error).and_then(serialize_result)
        })()),
        "begin_island_drag" => Some(island::execute_begin_island_drag(app).map_err(serialize_error).and_then(serialize_result)),
        "island_cursor_inside" => Some(island::execute_island_cursor_inside(app).map_err(serialize_error).and_then(serialize_result)),
        // 岛页只写这两项：卡顺序与卡内收起态。整份偏好保存仍只属于设置窗口。
        "save_island_card_collapsed" => Some((|| -> Result<Value, Value> {
            let collapsed: std::collections::BTreeMap<String, bool> = command_argument(&arguments, "islandCardCollapsed", "岛卡收起状态无效").map_err(serialize_error)?;
            let normalized = crate::contracts::settings::normalize_island_card_flags(collapsed); // 与整份保存共用同一条岛卡标识边界。
            app.state::<crate::preferences::SettingsState>()
                .update(|preferences| {
                    preferences.island_card_collapsed = normalized;
                    Ok(())
                })
                .map_err(serialize_error)
                .and_then(serialize_result)
        })()),
        "save_island_card_order" => Some((|| -> Result<Value, Value> {
            let order: Vec<String> = command_argument(&arguments, "islandCardOrder", "岛卡顺序无效").map_err(serialize_error)?;
            let normalized = crate::contracts::settings::normalize_island_card_order(order); // 去重、限定前缀与上限。
            app.state::<crate::preferences::SettingsState>()
                .update(|preferences| {
                    preferences.island_card_order = normalized;
                    Ok(())
                })
                .map_err(serialize_error)
                .and_then(serialize_result)
        })()),
        _ => None,
    }
}

/// notifications 领域命令。
#[allow(unused_variables)]
pub(crate) fn route_notifications(command: &str, app: &AppHandle, runtime: &DesktopHostRuntime, arguments: &Value) -> Option<Result<Value, Value>>
{
    match command
    {
        "get_island_messages" => Some(serialize_result(notifications::execute_get_island_messages(app.state::<std::sync::Arc<crate::notifications::ToastManager>>().inner()))),
        "dismiss_island_message" => Some((|| -> Result<Value, Value> {
            let id: u64 = command_argument(&arguments, "id", "岛消息身份无效").map_err(serialize_error)?; // 队列内消息身份。
            notifications::execute_dismiss_island_message(app, app.state::<std::sync::Arc<crate::notifications::ToastManager>>().inner(), id).map_err(serialize_error).and_then(serialize_result)
        })()),
        "sync_message_appearance" => Some((|| -> Result<Value, Value> {
            let preferences = command_argument(&arguments, "preferences", "消息外观偏好无效").map_err(serialize_error)?; // 页面已解析的当前程序偏好。
            let dark = command_argument(&arguments, "dark", "消息深浅色无效").map_err(serialize_error)?;
            let animations = command_argument(&arguments, "animationsEnabled", "消息动画状态无效").map_err(serialize_error)?;
            app.state::<std::sync::Arc<crate::notifications::ToastManager>>().sync_appearance(preferences, dark, animations); // 页面只补系统深浅与动画，palette 由保存偏好收敛。
            serialize_result(())
        })()),
        "publish_message_notification" => Some((|| -> Result<Value, Value> {
            let message: crate::notifications::ToastRequest = command_argument(&arguments, "message", "消息参数无效").map_err(serialize_error)?;
            let kind = message.r#type.clone(); // 事件里只放类型与来源，不把正文外发给插件。
            let source = message.source.clone();
            let published = app.state::<std::sync::Arc<crate::notifications::ToastManager>>().inner().publish(app, message).map_err(serialize_error);
            if published.is_ok()
            {
                crate::contracts::emit_plugin_event(&app, "toast-shown", serde_json::json!({ "kind": kind, "source": source }));
            }
            published.and_then(serialize_result)
        })()),
        _ => None,
    }
}

/// audio 领域命令。
#[allow(unused_variables)]
pub(crate) fn route_audio(command: &str, app: &AppHandle, runtime: &DesktopHostRuntime, arguments: &Value) -> Option<Result<Value, Value>>
{
    match command
    {
        "list_audio_output_devices" => Some(serialize_result(audio::list_audio_output_devices())),
        "start_audio_spectrum" => Some((|| -> Result<Value, Value> {
            let device: String = command_argument(&arguments, "device", "音频设备参数无效").map_err(serialize_error)?; // 空字符串表示跟随系统默认设备。
            let bars: u8 = command_argument(&arguments, "bars", "频谱段数参数无效").map_err(serialize_error)?;
            let fps: u8 = command_argument(&arguments, "fps", "频谱帧率参数无效").map_err(serialize_error)?;
            audio::start_audio_spectrum(app.clone(), device, bars, fps).map_err(serialize_error).and_then(serialize_result)
        })()),
        "stop_audio_spectrum" => Some((|| -> Result<Value, Value> {
            audio::stop_audio_spectrum();
            serialize_result(())
        })()),
        "get_audio_spectrum_status" => Some(serialize_result(audio::get_audio_spectrum_status())),
        _ => None,
    }
}

/// music 领域命令。
#[allow(unused_variables)]
pub(crate) fn route_music(command: &str, app: &AppHandle, runtime: &DesktopHostRuntime, arguments: &Value) -> Option<Result<Value, Value>>
{
    match command
    {
        "get_music_details" => Some(Ok(music::get_music_details())),
        "get_current_music_session" => Some(music::get_current_music_session().map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message)))),
        "request_music_convergence" => Some((|| -> Result<Value, Value> {
            crate::platform::windows::music::request_convergence();
            serialize_result(())
        })()),
        "execute_music_action" => Some((|| -> Result<Value, Value> {
            let action: String = command_argument(&arguments, "action", "媒体动作参数无效").map_err(serialize_error)?; // 原生桌面通道与 Tauri 使用相同的动作入口。
            let expected_session: String = command_argument(&arguments, "expectedSession", "媒体会话参数无效").map_err(serialize_error)?; // 用户点击时的会话代次。
            music::execute_music_action(action, expected_session).map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message))).and_then(serialize_result)
        })()),
        "seek_music_position" => Some((|| -> Result<Value, Value> {
            let expected_session: String = command_argument(&arguments, "expectedSession", "媒体会话参数无效").map_err(serialize_error)?; // 拖动时的会话代次。
            let position_ms: u64 = command_argument(&arguments, "positionMs", "媒体跳转位置无效").map_err(serialize_error)?; // 目标位置以毫秒传递。
            music::seek_music_position(expected_session, position_ms).map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message))).and_then(serialize_result)
        })()),
        _ => None,
    }
}

/// settings 领域命令。
#[allow(unused_variables)]
pub(crate) fn route_settings(command: &str, app: &AppHandle, runtime: &DesktopHostRuntime, arguments: &Value) -> Option<Result<Value, Value>>
{
    match command
    {
        "get_windows_accent_color" => Some(app.state::<crate::platform::windows::settings::WindowsAccentMonitor>().color().map_err(serialize_error).and_then(serialize_result)),
        "get_preferences" => Some(app.state::<crate::preferences::SettingsState>().preferences().map_err(serialize_error).and_then(serialize_result)),
        _ => None,
    }
}

/// weather 领域命令。
#[allow(unused_variables)]
pub(crate) fn route_weather(command: &str, app: &AppHandle, runtime: &DesktopHostRuntime, arguments: &Value) -> Option<Result<Value, Value>>
{
    match command
    {
        "get_weather_forecast" => Some((|| -> Result<Value, Value> {
            let source: crate::weather::Source = command_argument(&arguments, "source", "天气来源参数无效").map_err(serialize_error)?;
            let city_id: String = command_argument(&arguments, "cityId", "天气城市标识无效").map_err(serialize_error)?;
            app.state::<crate::weather::WeatherState>().forecast_for_city(source, &city_id).map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message))).and_then(serialize_result)
        })()),
        "list_weather_sources" => Some((|| -> Result<Value, Value> {
            // 桌面宿主只读已配置来源列表，不接触草稿会话。
            weather::list_weather_sources(app.clone()).map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message))).and_then(serialize_result)
        })()),
        "get_weather_cities" => Some((|| -> Result<Value, Value> {
            let source: crate::weather::Source = command_argument(&arguments, "source", "天气来源参数无效").map_err(serialize_error)?;
            app.state::<crate::weather::WeatherState>().cities(source).map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message))).and_then(serialize_result)
        })()),
        "add_weather_city" => Some((|| -> Result<Value, Value> {
            let source: crate::weather::Source = command_argument(&arguments, "source", "天气来源参数无效").map_err(serialize_error)?;
            let city: crate::weather::City = command_argument(&arguments, "city", "天气城市参数无效").map_err(serialize_error)?;
            add_weather_city(app, source, &city).map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message))).and_then(serialize_result)
        })()),
        "remove_weather_city" => Some((|| -> Result<Value, Value> {
            let source: crate::weather::Source = command_argument(&arguments, "source", "天气来源参数无效").map_err(serialize_error)?;
            let city_id: String = command_argument(&arguments, "cityId", "天气城市标识无效").map_err(serialize_error)?;
            remove_weather_city(app, source, &city_id).map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message))).and_then(serialize_result)
        })()),
        "validate_weather_city" => Some((|| -> Result<Value, Value> {
            let source: crate::weather::Source = command_argument(&arguments, "source", "天气来源参数无效").map_err(serialize_error)?;
            let city: crate::weather::City = command_argument(&arguments, "city", "天气城市参数无效").map_err(serialize_error)?;
            app.state::<crate::weather::WeatherState>().validate_for_source(source, &city).map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message))).and_then(|()| serialize_result(city))
        })()),
        "create_weather_tile" => Some((|| -> Result<Value, Value> {
            let source: crate::weather::Source = command_argument(&arguments, "source", "天气来源参数无效").map_err(serialize_error)?;
            let city_id: String = command_argument(&arguments, "cityId", "天气城市标识无效").map_err(serialize_error)?;
            let monitor_id: Option<String> = arguments.get("monitorId").cloned().map(serde_json::from_value).transpose().map_err(|error| serialize_error(DesktopHostError::invalid_readiness(error.to_string())))?;
            weather::create_weather_tile(app.clone(), source, city_id, monitor_id).map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message))).and_then(serialize_result)
        })()),
        "subscribe_weather" => Some((|| -> Result<Value, Value> {
            let instance = command_argument(&arguments, "instanceId", "天气实例标识无效").map_err(serialize_error)?;
            let monitor = command_argument(&arguments, "monitorId", "天气显示器标识无效").map_err(serialize_error)?;
            let source: crate::weather::Source = command_argument(&arguments, "source", "天气来源参数无效").map_err(serialize_error)?;
            let city_id: String = command_argument(&arguments, "cityId", "天气城市标识无效").map_err(serialize_error)?;
            weather::subscribe_weather(app.clone(), instance, monitor, source, city_id).map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message))).and_then(serialize_result)
        })()),
        "unsubscribe_weather" => Some((|| -> Result<Value, Value> {
            let subscription = command_argument(&arguments, "subscription", "天气订阅标识无效").map_err(serialize_error)?;
            weather::unsubscribe_weather(subscription);
            serialize_result(())
        })()),
        "search_weather_cities" => Some((|| -> Result<Value, Value> {
            let source: crate::weather::Source = command_argument(&arguments, "source", "天气来源参数无效").map_err(serialize_error)?;
            let query: String = command_argument(&arguments, "query", "城市搜索参数无效").map_err(serialize_error)?;
            app.state::<crate::weather::WeatherState>().search_saved(source, query).map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message))).and_then(serialize_result)
        })()),
        _ => None,
    }
}

/// collect 领域命令。
#[allow(unused_variables)]
pub(crate) fn route_collect(command: &str, app: &AppHandle, runtime: &DesktopHostRuntime, arguments: &Value) -> Option<Result<Value, Value>>
{
    match command
    {
        "get_collect_items" => Some((|| -> Result<Value, Value> {
            let preferences = app.state::<crate::preferences::SettingsState>().preferences().map_err(serialize_error)?;
            let path = crate::preferences::resolve_config_directory(&preferences.config_directory).map_err(serialize_error)?;
            crate::collect::list(&path).map_err(|error| serialize_error(DesktopHostError::invalid_readiness(error.to_string()))).and_then(serialize_result)
        })()),
        "read_collect_image" => Some((|| -> Result<Value, Value> {
            let name: String = command_argument(&arguments, "name", "收集项名称无效").map_err(serialize_error)?; // 两个通道使用相同的参数校验。
            let reference: String = command_argument(&arguments, "reference", "图片引用无效").map_err(serialize_error)?; // 编辑器原图引用。
            let pixels: Option<u32> = serde_json::from_value(arguments.get("pixels").cloned().unwrap_or(Value::Null)).map_err(|error| serialize_error(error.to_string()))?; // 列表请求有界缩略图。
            collect::execute_read_collect_image(app, &name, &reference, pixels).map_err(serialize_error).and_then(serialize_result)
        })()),
        "prepare_collect_export" => Some((|| -> Result<Value, Value> {
            let names: Vec<String> = command_argument(&arguments, "names", "收集项选择无效").map_err(serialize_error)?;
            let preferences = app.state::<crate::preferences::SettingsState>().preferences().map_err(serialize_error)?;
            let path = crate::preferences::resolve_config_directory(&preferences.config_directory).map_err(serialize_error)?;
            crate::collect::prepare_export(&path, &names).map_err(serialize_error).and_then(serialize_result)
        })()),
        "receive_collect_items" => Some((|| -> Result<Value, Value> {
            let names: Vec<String> = command_argument(&arguments, "names", "收集项选择无效").map_err(serialize_error)?;
            let target_name: Option<String> = arguments.get("targetName").cloned().map(serde_json::from_value).transpose().map_err(|error| serialize_error(error.to_string()))?;
            let preferences = app.state::<crate::preferences::SettingsState>().preferences().map_err(serialize_error)?;
            let path = crate::preferences::resolve_config_directory(&preferences.config_directory).map_err(serialize_error)?;
            collect::with_collect_editor_state(app, &crate::collect::directory(&path), target_name.as_deref(), |editing| crate::collect::receive_items(&path, target_name.as_deref(), &names, editing)).map_err(serialize_error).and_then(|result| {
                let _ = app.emit("collect-items-changed", ());
                serialize_result(result)
            })
        })()),
        "open_collect_link" => Some((|| -> Result<Value, Value> {
            let url = command_argument(&arguments, "url", "链接地址无效").map_err(serialize_error)?; // 桌面宿主与编辑窗共用打开规则。
            collect::open_collect_link(url).map_err(serialize_error).and_then(serialize_result)
        })()),
        "open_collect_item" => Some((|| -> Result<Value, Value> {
            let name: String = command_argument(&arguments, "name", "收集项名称无效").map_err(serialize_error)?;
            collect::execute_open_collect_item(app, name).map_err(serialize_error).and_then(serialize_result)
        })()),
        "create_collect_item" => Some((|| -> Result<Value, Value> {
            let content: String = command_argument(&arguments, "content", "收集项内容无效").map_err(serialize_error)?;
            let preferences = app.state::<crate::preferences::SettingsState>().preferences().map_err(serialize_error)?;
            let path = crate::preferences::resolve_config_directory(&preferences.config_directory).map_err(serialize_error)?;
            crate::collect::create(&path, &content).map_err(|error| serialize_error(DesktopHostError::invalid_readiness(error.to_string()))).and_then(|item| {
                let _ = app.emit("collect-items-changed", ());
                serialize_result(item)
            })
        })()),
        "save_collect_item" => Some((|| -> Result<Value, Value> {
            let name: String = command_argument(&arguments, "name", "收集项名称无效").map_err(serialize_error)?;
            let content: String = command_argument(&arguments, "content", "收集项内容无效").map_err(serialize_error)?;
            let preferences = app.state::<crate::preferences::SettingsState>().preferences().map_err(serialize_error)?;
            let path = crate::preferences::resolve_config_directory(&preferences.config_directory).map_err(serialize_error)?;
            crate::collect::save(&path, &name, &content).map_err(|error| serialize_error(DesktopHostError::invalid_readiness(error.to_string()))).and_then(|_| {
                let _ = app.emit("collect-items-changed", ());
                serialize_result(())
            })
        })()),
        "rename_collect_item" => Some((|| -> Result<Value, Value> {
            let old_name: String = command_argument(&arguments, "oldName", "原收集项名称无效").map_err(serialize_error)?;
            let new_name: String = command_argument(&arguments, "newName", "新收集项名称无效").map_err(serialize_error)?;
            let draft: Option<String> = serde_json::from_value(arguments.get("draft").cloned().unwrap_or(Value::Null)).map_err(|error| serialize_error(error.to_string()))?; // 可选未保存草稿仍须严格校验类型。
            collect::execute_rename_collect_item(app.clone(), old_name, new_name, draft).map_err(serialize_error).and_then(serialize_result)
        })()),
        "import_collect_resource" => Some((|| -> Result<Value, Value> {
            let name: String = command_argument(&arguments, "name", "收集项名称无效").map_err(serialize_error)?;
            let source: String = command_argument(&arguments, "source", "图片路径无效").map_err(serialize_error)?;
            let mode: String = command_argument(&arguments, "mode", "图片收入方式无效").map_err(serialize_error)?;
            let preferences = app.state::<crate::preferences::SettingsState>().preferences().map_err(serialize_error)?;
            let path = crate::preferences::resolve_config_directory(&preferences.config_directory).map_err(serialize_error)?;
            crate::collect::import_resource(&path, &name, std::path::Path::new(&source), &mode).map_err(|error| serialize_error(DesktopHostError::invalid_readiness(error.to_string()))).and_then(serialize_result)
        })()),
        "write_collect_resource" => Some((|| -> Result<Value, Value> {
            let name: String = command_argument(&arguments, "name", "收集项名称无效").map_err(serialize_error)?;
            let file_name: String = command_argument(&arguments, "fileName", "图片文件名无效").map_err(serialize_error)?;
            let bytes: Vec<u8> = command_argument(&arguments, "bytes", "图片内容无效").map_err(serialize_error)?;
            let preferences = app.state::<crate::preferences::SettingsState>().preferences().map_err(serialize_error)?;
            let path = crate::preferences::resolve_config_directory(&preferences.config_directory).map_err(serialize_error)?;
            crate::collect::write_resource_bytes(&path, &name, &file_name, &bytes).map_err(|error| serialize_error(DesktopHostError::invalid_readiness(error.to_string()))).and_then(serialize_result)
        })()),
        "import_collect_rendered_resource" => Some((|| -> Result<Value, Value> {
            let name: String = command_argument(&arguments, "name", "收集项名称无效").map_err(serialize_error)?;
            let reference: String = command_argument(&arguments, "reference", "图片引用无效").map_err(serialize_error)?;
            let mode: String = command_argument(&arguments, "mode", "图片收入方式无效").map_err(serialize_error)?;
            let preferences = app.state::<crate::preferences::SettingsState>().preferences().map_err(serialize_error)?;
            let path = crate::preferences::resolve_config_directory(&preferences.config_directory).map_err(serialize_error)?;
            crate::collect::import_rendered_resource(&path, &name, &reference, &mode).map_err(|error| serialize_error(DesktopHostError::invalid_readiness(error.to_string()))).and_then(serialize_result)
        })()),
        "set_collect_item_pinned" => Some((|| -> Result<Value, Value> {
            let name: String = command_argument(&arguments, "name", "收集项名称无效").map_err(serialize_error)?;
            let pinned: bool = command_argument(&arguments, "pinned", "置顶状态无效").map_err(serialize_error)?;
            let preferences = app.state::<crate::preferences::SettingsState>().preferences().map_err(serialize_error)?;
            let path = crate::preferences::resolve_config_directory(&preferences.config_directory).map_err(serialize_error)?;
            crate::collect::set_pinned(&path, &name, pinned).map_err(|error| serialize_error(DesktopHostError::invalid_readiness(error.to_string()))).and_then(|_| {
                let _ = app.emit("collect-items-changed", ());
                serialize_result(())
            })
        })()),
        "delete_collect_item" => Some((|| -> Result<Value, Value> {
            let name: String = command_argument(&arguments, "name", "收集项名称无效").map_err(serialize_error)?;
            let preferences = app.state::<crate::preferences::SettingsState>().preferences().map_err(serialize_error)?;
            let path = crate::preferences::resolve_config_directory(&preferences.config_directory).map_err(serialize_error)?;
            crate::collect::delete(&path, &name).map_err(|error| serialize_error(DesktopHostError::invalid_readiness(error.to_string()))).and_then(|_| {
                let _ = app.emit("collect-items-changed", json!({ "deletedName": name })); // 主动删除不生成不可用占位。
                serialize_result(())
            })
        })()),
        "restore_collect_item" => Some((|| -> Result<Value, Value> {
            let preferences = app.state::<crate::preferences::SettingsState>().preferences().map_err(serialize_error)?;
            let path = crate::preferences::resolve_config_directory(&preferences.config_directory).map_err(serialize_error)?;
            crate::collect::restore_latest(&path).map_err(|error| serialize_error(DesktopHostError::invalid_readiness(error.to_string()))).and_then(|item| {
                let _ = app.emit("collect-items-changed", ());
                serialize_result(item)
            })
        })()),
        "open_collect_editor" => Some((|| -> Result<Value, Value> {
            let name: String = command_argument(&arguments, "name", "收集项名称无效").map_err(serialize_error)?;
            collect::execute_open_collect_editor(app.clone(), name).map_err(serialize_error).and_then(serialize_result)
        })()),
        "record_renderer_diagnostic" => Some((|| -> Result<Value, Value> {
            let fields = ["origin", "url", "stage", "message"].map(|key| arguments.get(key).and_then(Value::as_str).unwrap_or("").chars().take(4096).collect::<String>().replace(['\r', '\n'], " ")); // 有界单行诊断，不执行页面内容。
            crate::log_error!("宿主前端：来源={}，页面={}，阶段={}，错误={}", fields[0], fields[1], fields[2], fields[3]);
            // 页面脚本或资源加载失败时标记未就绪宿主，由恢复线程按退避重载，避免接管一直等待该页。
            if matches!(fields[2].as_str(), "资源加载失败" | "脚本异常")
            {
                runtime.report_renderer_failure();
            }
            serialize_result(())
        })()),
        "create_collect_box" => Some(create_collect_box(app.clone()).map_err(serialize_error).and_then(serialize_result)),
        _ => None,
    }
}

/// desktop_item 领域命令。
#[allow(unused_variables)]
pub(crate) fn route_desktop_item(command: &str, app: &AppHandle, runtime: &DesktopHostRuntime, arguments: &Value) -> Option<Result<Value, Value>>
{
    match command
    {
        "prepare_collect_drag" => Some(command_argument(&arguments, "names", "收集项选择无效").map_err(serialize_error).and_then(|names| desktop_item::execute_prepare_collect_drag(app, names).map_err(serialize_error)).and_then(serialize_result)),
        "get_desktop_item_snapshot" => Some(desktop_item::execute_get_desktop_item_snapshot(app, &app.state::<DesktopItemState>(), &app.state::<WorkspaceState>(), runtime).map_err(serialize_error).and_then(serialize_result)),
        "get_desktop_item_image" => Some((|| -> Result<Value, Value> {
            let item_id = command_argument(&arguments, "itemId", "系统图像项目参数无效").map_err(serialize_error)?; // 当前快照身份。
            let pixels = command_argument(&arguments, "pixels", "系统图像尺寸参数无效").map_err(serialize_error)?; // 后端继续检查范围。
            app.state::<DesktopItemState>().image(item_id, pixels).map_err(serialize_error).and_then(serialize_result)
        })()),
        "get_current_desktop_item_snapshot" => Some(desktop_item::read_desktop_item_snapshot(&app.state::<DesktopItemState>()).map_err(serialize_error).and_then(serialize_result)),
        "open_desktop_items" => Some(command_argument(&arguments, "itemIds", "桌面项目打开参数无效").map_err(serialize_error).and_then(|item_ids: Vec<String>| app.state::<DesktopItemState>().open(&item_ids).map_err(serialize_error)).and_then(serialize_result)),
        "rename_desktop_item" => Some((|| -> Result<Value, Value> {
            let item_id: String = command_argument(&arguments, "itemId", "命令参数无效").map_err(serialize_error)?; // 已解码的 itemId。
            let new_name: String = command_argument(&arguments, "newName", "命令参数无效").map_err(serialize_error)?; // 已解码的 newName。
            desktop_item::execute_rename_desktop_item(&app.state::<DesktopItemState>(), &app.state::<WorkspaceState>(), item_id, new_name).map_err(serialize_error).and_then(serialize_result)
        })()),
        "delete_desktop_items" => Some((|| -> Result<Value, Value> {
            let item_ids: Vec<String> = command_argument(&arguments, "itemIds", "命令参数无效").map_err(serialize_error)?; // 已解码的 itemIds。
            let permanent: bool = command_argument(&arguments, "permanent", "命令参数无效").map_err(serialize_error)?; // 已解码的 permanent。
            desktop_item::execute_delete_desktop_items(&app.state::<DesktopItemState>(), &app.state::<WorkspaceState>(), item_ids, permanent).map_err(serialize_error).and_then(serialize_result)
        })()),
        "set_desktop_item_clipboard" => Some((|| -> Result<Value, Value> {
            command_argument(&arguments, "itemIds", "桌面项目剪贴板参数无效")
                .map_err(serialize_error)
                .and_then(|item_ids: Vec<String>| {
                    let cut = arguments.get("cut").and_then(Value::as_bool).ok_or_else(|| serialize_error(DesktopHostError::invalid_readiness("缺少 cut")))?; // 是否使用剪切效果。
                    app.state::<DesktopItemState>().set_clipboard(&item_ids, cut).map_err(serialize_error)
                })
                .and_then(serialize_result)
        })()),
        "paste_desktop_items" => Some(desktop_item::execute_paste_desktop_items(&app.state::<DesktopItemState>(), &app.state::<WorkspaceState>()).map_err(serialize_error).and_then(serialize_result)),
        "prepare_desktop_item_drag" => Some(command_argument(&arguments, "itemIds", "桌面项目拖放参数无效").map_err(serialize_error).and_then(|item_ids: Vec<String>| app.state::<DesktopItemState>().prepare_drag(&item_ids).map_err(serialize_error)).and_then(serialize_result)),
        "cancel_desktop_item_drag_preparation" => Some(command_argument(&arguments, "token", "拖动凭据无效").map_err(serialize_error).and_then(|token| app.state::<DesktopItemState>().cancel_prepared_drag(token).map_err(serialize_error)).and_then(serialize_result)),
        "start_desktop_item_drag" => Some((|| -> Result<Value, Value> {
            let token = command_argument(&arguments, "token", "拖动凭据无效").map_err(serialize_error)?; // 单次启动凭据。
            let pointer_offset = command_argument(&arguments, "pointerOffset", "拖动锚点偏移无效").map_err(serialize_error)?; // 源表面 DIP 偏移。
            desktop_item::execute_start_desktop_item_drag(app, &app.state::<DesktopItemState>(), token, pointer_offset).map_err(serialize_error).and_then(serialize_result)
        })()),
        "show_workspace_context_menu" => Some((|| -> Result<Value, Value> {
            let monitor_id: String = command_argument(&arguments, "monitorId", "命令参数无效").map_err(serialize_error)?; // 已解码的 monitorId。
            let target: WorkspaceContextMenuTarget = command_argument(&arguments, "target", "命令参数无效").map_err(serialize_error)?; // 已解码的 target。
            let client_x: f64 = command_argument(&arguments, "clientX", "命令参数无效").map_err(serialize_error)?; // 已解码的 clientX。
            let client_y: f64 = command_argument(&arguments, "clientY", "命令参数无效").map_err(serialize_error)?; // 已解码的 clientY。
            desktop_item::execute_show_workspace_context_menu(app, &app.state::<DesktopItemState>(), &app.state::<WorkspaceState>(), runtime, monitor_id, target, client_x, client_y).map_err(serialize_error).and_then(serialize_result)
        })()),
        _ => None,
    }
}

/// folder_portal 领域命令。
#[allow(unused_variables)]
pub(crate) fn route_folder_portal(command: &str, app: &AppHandle, runtime: &DesktopHostRuntime, arguments: &Value) -> Option<Result<Value, Value>>
{
    match command
    {
        "watch_folder_portal" => Some((|| -> Result<Value, Value> {
            let root: String = command_argument(&arguments, "rootPath", "门户根路径无效").map_err(serialize_error)?; // 校验绑定根。
            let current: String = command_argument(&arguments, "currentPath", "门户当前目录无效").map_err(serialize_error)?; // 当前导航路径。
            let monitor: String = command_argument(&arguments, "monitorId", "显示器标识无效").map_err(serialize_error)?; // 用于宿主销毁时清理。
            runtime.validate_monitor_id(&monitor).map_err(serialize_error)?;
            let (_, path) = folder_portal::checked_directory(&root, &current).map_err(serialize_error)?; // 复用目录安全边界。
            crate::platform::windows::directory_watch::start(app.clone(), monitor, path).map_err(serialize_error).and_then(serialize_result)
        })()),
        "unwatch_folder_portal" => Some((|| -> Result<Value, Value> {
            let id: u64 = command_argument(&arguments, "subscription", "目录监听标识无效").map_err(serialize_error)?; // 仅取消本应用监听。
            crate::platform::windows::directory_watch::stop(id);
            serialize_result(())
        })()),
        "open_folder_portal_path" => Some((|| -> Result<Value, Value> {
            let root_path = command_argument(&arguments, "rootPath", "门户根路径参数无效").map_err(serialize_error)?;
            let path = command_argument(&arguments, "path", "门户路径参数无效").map_err(serialize_error)?;
            folder_portal::open_folder_portal_path(root_path, path).map_err(serialize_error).and_then(serialize_result)
        })()),
        "delete_folder_portal_items" => Some((|| -> Result<Value, Value> {
            let root_path = command_argument(&arguments, "rootPath", "门户根路径参数无效").map_err(serialize_error)?;
            let current_path = command_argument(&arguments, "currentPath", "门户当前路径参数无效").map_err(serialize_error)?;
            let names = command_argument(&arguments, "names", "门户项目参数无效").map_err(serialize_error)?;
            folder_portal::execute_delete_folder_portal_items(&app.state::<DesktopItemState>(), root_path, current_path, names).map_err(serialize_error).and_then(serialize_result)
        })()),
        "set_folder_portal_clipboard" => Some((|| -> Result<Value, Value> {
            let root_path = command_argument(&arguments, "rootPath", "门户根路径参数无效").map_err(serialize_error)?;
            let current_path = command_argument(&arguments, "currentPath", "门户当前路径参数无效").map_err(serialize_error)?;
            let names = command_argument(&arguments, "names", "门户项目参数无效").map_err(serialize_error)?;
            let cut = command_argument(&arguments, "cut", "剪贴板操作参数无效").map_err(serialize_error)?;
            folder_portal::set_folder_portal_clipboard(app.state::<DesktopItemState>(), root_path, current_path, names, cut).map_err(serialize_error).and_then(serialize_result)
        })()),
        "paste_folder_portal_items" => Some((|| -> Result<Value, Value> {
            let root_path = command_argument(&arguments, "rootPath", "门户根路径参数无效").map_err(serialize_error)?;
            let current_path = command_argument(&arguments, "currentPath", "门户当前路径参数无效").map_err(serialize_error)?;
            folder_portal::paste_folder_portal_items(app.state::<DesktopItemState>(), root_path, current_path).map_err(serialize_error).and_then(serialize_result)
        })()),
        "show_folder_portal_menu" => Some((|| -> Result<Value, Value> {
            let request = command_argument(&arguments, "request", "门户菜单参数无效").map_err(serialize_error)?; // 两种传输使用相同请求结构。
            folder_portal::execute_show_folder_portal_menu(&app.state::<DesktopItemState>(), runtime, request).map_err(serialize_error).and_then(serialize_result)
        })()),
        "rename_folder_portal_item" => Some((|| -> Result<Value, Value> {
            let request = command_argument(&arguments, "request", "门户重命名参数无效").map_err(serialize_error)?;
            folder_portal::execute_rename_folder_portal_item(&app.state::<DesktopItemState>(), request).map_err(serialize_error).and_then(serialize_result)
        })()),
        "get_folder_portal_snapshot" => Some((|| -> Result<Value, Value> {
            let root_path: String = command_argument(&arguments, "rootPath", "门户绑定目录参数无效").map_err(serialize_error)?;
            let current_path: Option<String> = arguments.get("currentPath").cloned().map(serde_json::from_value).transpose().map_err(|_| serialize_error(DesktopHostError::invalid_readiness("门户当前目录参数无效")))?;
            folder_portal::get_folder_portal_snapshot(root_path, current_path).map_err(|error| serialize_error(DesktopHostError::invalid_readiness(error))).and_then(serialize_result)
        })()),
        "prepare_folder_portal_drag" => Some((|| -> Result<Value, Value> {
            let root: String = command_argument(&arguments, "rootPath", "门户根路径无效").map_err(serialize_error)?;
            let current: String = command_argument(&arguments, "currentPath", "门户当前目录无效").map_err(serialize_error)?;
            let names: Vec<String> = command_argument(&arguments, "names", "门户选择无效").map_err(serialize_error)?;
            let references = folder_portal::checked_item_paths(&root, &current, &names).map_err(serialize_error)?;
            app.state::<DesktopItemState>().prepare_portal_drag(references).map_err(serialize_error).and_then(serialize_result)
        })()),
        _ => None,
    }
}

/// component 领域命令。
#[allow(unused_variables)]
pub(crate) fn route_component(command: &str, app: &AppHandle, runtime: &DesktopHostRuntime, arguments: &Value) -> Option<Result<Value, Value>>
{
    match command
    {
        "subscribe_component_time" => Some((|| -> Result<Value, Value> {
            let instance: String = command_argument(&arguments, "instanceId", "组件实例标识无效").map_err(serialize_error)?; // 需要校验的真实组件实例。
            let monitor: String = command_argument(&arguments, "monitorId", "显示器标识无效").map_err(serialize_error)?; // 当前呈现宿主。
            runtime.validate_monitor_id(&monitor).map_err(serialize_error)?;
            component::execute_invoke_component_capability(&app.state::<ComponentRuntimeState>(), &app.state::<WorkspaceState>(), &app.state::<crate::preferences::SettingsState>(), instance, crate::contracts::ComponentCapability::ReadHostTime).map_err(serialize_error)?;
            serialize_result(ComponentRuntimeState::subscribe_time(monitor))
        })()),
        "unsubscribe_component_time" => Some((|| -> Result<Value, Value> {
            let id: u64 = command_argument(&arguments, "subscription", "时间订阅标识无效").map_err(serialize_error)?; // 独立生命周期身份。
            ComponentRuntimeState::unsubscribe_time(id);
            serialize_result(())
        })()),
        "get_component_runtime_snapshot" => Some(component::execute_get_component_runtime_snapshot(app, &app.state::<ComponentRuntimeState>(), &app.state::<WorkspaceState>()).map_err(serialize_error).and_then(serialize_result)),
        "get_current_component_runtime_snapshot" => Some(component::component_runtime_snapshot(&app.state::<ComponentRuntimeState>(), &app.state::<WorkspaceState>()).map_err(serialize_error).and_then(serialize_result)),
        "get_current_component_host_event" => Some(component::current_component_host_event(&app.state::<ComponentRuntimeState>()).map_err(serialize_error).and_then(serialize_result)),
        "invoke_component_capability" => Some((|| -> Result<Value, Value> {
            let instance_id: String = command_argument(&arguments, "instanceId", "命令参数无效").map_err(serialize_error)?; // 已解码的 instanceId。
            let capability: ComponentCapability = command_argument(&arguments, "capability", "命令参数无效").map_err(serialize_error)?; // 已解码的 capability。
            component::execute_invoke_component_capability(&app.state::<ComponentRuntimeState>(), &app.state::<WorkspaceState>(), &app.state::<crate::preferences::SettingsState>(), instance_id, capability).map_err(serialize_error).and_then(serialize_result)
        })()),
        _ => None,
    }
}

/// popup_layer 领域命令。
#[allow(unused_variables)]
pub(crate) fn route_popup_layer(command: &str, app: &AppHandle, runtime: &DesktopHostRuntime, arguments: &Value) -> Option<Result<Value, Value>>
{
    match command
    {
        "open_popup_layer" => Some((|| -> Result<Value, Value> {
            let request = command_argument(&arguments, "request", "弹层请求参数无效").map_err(serialize_error)?; // 宿主页面与弹层页面共用同一请求结构。
            let read_preferences = crate::preferences::reader(app); // 弹层主题读取经应用层注入。
            crate::platform::windows::popup_layer::execute_open(app, request, &read_preferences).map_err(serialize_error).and_then(serialize_result)
        })()),
        "update_popup_layer_payload" => Some((|| -> Result<Value, Value> {
            let request_id: u64 = command_argument(&arguments, "requestId", "弹层请求身份无效").map_err(serialize_error)?;
            let payload = arguments.get("payload").cloned().unwrap_or(serde_json::Value::Null); // 载荷结构由弹层页面按种类校验。
            crate::platform::windows::popup_layer::execute_update_payload(app, request_id, payload).map_err(serialize_error).and_then(serialize_result)
        })()),
        "dismiss_popup_layer" => Some((|| -> Result<Value, Value> {
            let request_id: Option<u64> = arguments.get("requestId").cloned().map(serde_json::from_value).transpose().map_err(|error| serialize_error(error.to_string()))?; // 省略时结束当前请求。
            crate::platform::windows::popup_layer::execute_dismiss(app, request_id).map_err(serialize_error).and_then(serialize_result)
        })()),
        _ => None,
    }
}

/// loopback 领域命令。
#[allow(unused_variables)]
pub(crate) fn route_loopback(command: &str, app: &AppHandle, runtime: &DesktopHostRuntime, arguments: &Value) -> Option<Result<Value, Value>>
{
    match command
    {
        "register_loopback_channel" => Some((|| -> Result<Value, Value> {
            let instance_id: String = command_argument(&arguments, "instanceId", "通道实例标识无效").map_err(serialize_error)?;
            let channel_id: String = command_argument(&arguments, "channelId", "通道标识无效").map_err(serialize_error)?;
            let mode: crate::contracts::LoopbackChannelMode = command_argument(&arguments, "mode", "通道方向无效").map_err(serialize_error)?;
            let port: Option<u16> = arguments.get("port").cloned().map(serde_json::from_value).transpose().map_err(|_| serialize_error(DesktopHostError::invalid_readiness("通道端口参数无效")))?;
            let base_url: Option<String> = arguments.get("baseUrl").cloned().map(serde_json::from_value).transpose().map_err(|_| serialize_error(DesktopHostError::invalid_readiness("通道基址参数无效")))?;
            let credential_ref: Option<String> = arguments.get("credentialRef").cloned().map(serde_json::from_value).transpose().map_err(|_| serialize_error(DesktopHostError::invalid_readiness("通道凭据参数无效")))?;
            let header_name: Option<String> = arguments.get("headerName").cloned().map(serde_json::from_value).transpose().map_err(|_| serialize_error(DesktopHostError::invalid_readiness("通道头名参数无效")))?;
            let request = crate::contracts::RegisterLoopbackChannel { instance_id, channel_id, mode, port, base_url, credential_ref, header_name };
            loopback::execute_register_loopback_channel(app, &app.state::<ComponentRuntimeState>(), &app.state::<WorkspaceState>(), request).map_err(serialize_error).and_then(serialize_result)
        })()),
        "read_loopback_channel" => Some((|| -> Result<Value, Value> {
            let instance_id: String = command_argument(&arguments, "instanceId", "通道实例标识无效").map_err(serialize_error)?;
            let channel_id: String = command_argument(&arguments, "channelId", "通道标识无效").map_err(serialize_error)?;
            let after_sequence: Option<u64> = arguments.get("afterSequence").cloned().map(serde_json::from_value).transpose().map_err(|_| serialize_error(DesktopHostError::invalid_readiness("通道序号参数无效")))?;
            loopback::execute_read_loopback_channel(&app.state::<ComponentRuntimeState>(), &app.state::<WorkspaceState>(), instance_id, channel_id, after_sequence).map_err(serialize_error).and_then(serialize_result)
        })()),
        "request_loopback" => Some((|| -> Result<Value, Value> {
            let instance_id: String = command_argument(&arguments, "instanceId", "通道实例标识无效").map_err(serialize_error)?;
            let channel_id: String = command_argument(&arguments, "channelId", "通道标识无效").map_err(serialize_error)?;
            let method: String = command_argument(&arguments, "method", "请求方法无效").map_err(serialize_error)?;
            let path: String = command_argument(&arguments, "path", "请求路径无效").map_err(serialize_error)?;
            let body: Option<String> = arguments.get("body").cloned().filter(|value| !value.is_null()).map(serde_json::from_value).transpose().map_err(|_| serialize_error(DesktopHostError::invalid_readiness("请求体参数无效")))?;
            loopback::execute_request_loopback(&app.state::<ComponentRuntimeState>(), &app.state::<WorkspaceState>(), instance_id, channel_id, method, path, body).map_err(serialize_error).and_then(serialize_result)
        })()),
        "set_loopback_credential" => Some((|| -> Result<Value, Value> {
            let target: String = command_argument(&arguments, "target", "凭据目标无效").map_err(serialize_error)?;
            let secret: String = command_argument(&arguments, "secret", "共享密钥无效").map_err(serialize_error)?;
            loopback::execute_set_loopback_credential(target, secret).map_err(serialize_error).and_then(serialize_result)
        })()),
        "clear_loopback_credential" => Some((|| -> Result<Value, Value> {
            let target: String = command_argument(&arguments, "target", "凭据目标无效").map_err(serialize_error)?;
            loopback::execute_clear_loopback_credential(target).map_err(serialize_error).and_then(serialize_result)
        })()),
        _ => None,
    }
}

/// desktop_host 领域命令。
#[allow(unused_variables)]
pub(crate) fn route_desktop_host(command: &str, app: &AppHandle, runtime: &DesktopHostRuntime, arguments: &Value) -> Option<Result<Value, Value>>
{
    match command
    {
        "get_desktop_refresh_policy" => Some((|| -> Result<Value, Value> {
            let monitor_id: String = command_argument(&arguments, "monitorId", "显示器标识无效").map_err(serialize_error)?; // 当前页面的显示器身份。
            let snapshot = runtime.snapshot().map_err(serialize_error)?; // 暂停只适用于已完成接管的宿主。
            runtime.validate_monitor_id(&monitor_id).map_err(serialize_error)?;
            let preferences = app.state::<crate::preferences::SettingsState>().preferences().map_err(serialize_error)?; // 每次读取已保存的开关。
            let foreground = crate::platform::windows::refresh_policy::foreground_snapshot(); // 页面与原生消息共享同一前台事实。
            serialize_result(serde_json::json!({ "snapshot": foreground, "committed": snapshot.committed, "pauseOnFocus": preferences.pause_on_focus, "pauseOnFullscreen": preferences.pause_on_fullscreen }))
        })()),
        "refresh_foreground_snapshot" => Some((|| -> Result<Value, Value> {
            crate::platform::windows::refresh_policy::request_sample();
            serialize_result(())
        })()),
        "record_desktop_refresh_state" => Some((|| -> Result<Value, Value> {
            let monitor_id: String = command_argument(&arguments, "monitorId", "显示器标识无效").map_err(serialize_error)?; // 已校验显示器身份。
            let paused: bool = command_argument(&arguments, "paused", "暂停状态无效").map_err(serialize_error)?; // 页面实际应用的暂停状态。
            runtime.validate_monitor_id(&monitor_id).map_err(serialize_error)?;
            crate::platform::windows::refresh_policy::set_paused(&monitor_id, paused);
            ComponentRuntimeState::wake_time_source();
            let _ = app.emit("desktop-refresh-policy-changed", ());
            crate::log_debug!("桌面刷新：显示器={}，状态={}", monitor_id, if paused { "已暂停" } else { "已恢复" });
            serialize_result(())
        })()),
        "get_desktop_host_snapshot" => Some(desktop_host::read_desktop_host_snapshot(runtime).map_err(serialize_error).and_then(serialize_result)),
        "exit_desktop_tools" => Some((|| -> Result<Value, Value> {
            desktop_host::request_exit(app);
            serialize_result(())
        })()),
        "record_desktop_host_renderer_heartbeat" => Some(command_argument(&arguments, "monitorId", "显示器标识无效").map_err(serialize_error).and_then(|monitor_id: String| desktop_host::record_renderer_heartbeat(runtime, &monitor_id).map_err(serialize_error)).and_then(serialize_result)),
        "publish_desktop_host_readiness" => Some((|| -> Result<Value, Value> {
            let report: FrontendHostReadiness = command_argument(&arguments, "report", "宿主参数无效").map_err(serialize_error)?; // 已校验参数。
            let ready = desktop_host::execute_publish_desktop_host_readiness(app, runtime, report).map_err(serialize_error);
            if ready.is_ok() && !STARTED_EVENT_SENT.swap(true, std::sync::atomic::Ordering::AcqRel)
            {
                crate::contracts::emit_plugin_event(&app, "app-started", serde_json::json!({})); // 首批宿主就绪即视为程序进入可用状态，只发一次。
            }
            ready.and_then(serialize_result)
        })()),
        "update_desktop_host_hit_map" => Some((|| -> Result<Value, Value> {
            let monitor_id: String = command_argument(&arguments, "monitorId", "宿主参数无效").map_err(serialize_error)?; // 已校验参数。
            let hit_regions: Vec<HitRegion> = command_argument(&arguments, "hitRegions", "宿主参数无效").map_err(serialize_error)?; // 已校验参数。
            desktop_host::execute_update_desktop_host_hit_map(app, runtime, monitor_id, hit_regions).map_err(serialize_error).and_then(serialize_result)
        })()),
        "record_desktop_host_input_probe" => Some((|| -> Result<Value, Value> {
            let monitor_id: String = command_argument(&arguments, "monitorId", "宿主参数无效").map_err(serialize_error)?; // 已校验参数。
            let item_id: String = command_argument(&arguments, "itemId", "宿主参数无效").map_err(serialize_error)?; // 已校验参数。
            desktop_host::execute_record_desktop_host_input_probe(app, runtime, monitor_id, item_id).map_err(serialize_error).and_then(serialize_result)
        })()),
        "begin_desktop_host_input_drag" => Some((|| -> Result<Value, Value> {
            let monitor_id: String = command_argument(&arguments, "monitorId", "宿主参数无效").map_err(serialize_error)?; // 已校验参数。
            desktop_host::execute_begin_desktop_host_input_drag(app, runtime, monitor_id).map_err(serialize_error).and_then(serialize_result)
        })()),
        "end_desktop_host_input_drag" => Some((|| -> Result<Value, Value> {
            let monitor_id: String = command_argument(&arguments, "monitorId", "宿主参数无效").map_err(serialize_error)?; // 已校验参数。
            desktop_host::execute_end_desktop_host_input_drag(app, runtime, monitor_id).map_err(serialize_error).and_then(serialize_result)
        })()),
        _ => None,
    }
}

/// workspace 领域命令。
#[allow(unused_variables)]
pub(crate) fn route_workspace(command: &str, app: &AppHandle, runtime: &DesktopHostRuntime, arguments: &Value) -> Option<Result<Value, Value>>
{
    match command
    {
        "relay_workspace_drag" => Some((|| -> Result<Value, Value> {
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
        })()),
        "get_workspace_snapshot" | "get_current_workspace_snapshot" => Some(workspace::read_workspace_snapshot(&app.state::<WorkspaceState>()).map_err(serialize_error).and_then(serialize_result)),
        "write_workspace_backup" => Some(command_argument(&arguments, "path", "工作区备份路径无效").map_err(serialize_error).and_then(|path: String| app.state::<WorkspaceState>().write_manual_backup(&PathBuf::from(path)).map_err(serialize_error)).and_then(serialize_result)),
        "inspect_workspace_backup" => Some(command_argument(&arguments, "path", "工作区备份路径无效").map_err(serialize_error).and_then(|path: String| app.state::<WorkspaceState>().inspect_backup(&PathBuf::from(path)).map_err(serialize_error)).and_then(serialize_result)),
        "list_workspace_auto_snapshots" => Some(app.state::<WorkspaceState>().auto_snapshot_summaries().map_err(serialize_error).and_then(serialize_result)),
        "restore_workspace_backup" => Some((|| -> Result<Value, Value> {
            let path: String = command_argument(&arguments, "path", "命令参数无效").map_err(serialize_error)?; // 已解码的 path。
            workspace::execute_restore_workspace_backup(app, &app.state::<WorkspaceState>(), &app.state::<DesktopItemState>(), runtime, path).map_err(serialize_error).and_then(serialize_result)
        })()),
        "undo_workspace" => Some(workspace::apply_workspace_history(app, &app.state::<WorkspaceState>(), &app.state::<DesktopItemState>(), runtime, false).map_err(serialize_error).and_then(serialize_result)),
        "redo_workspace" => Some(workspace::apply_workspace_history(app, &app.state::<WorkspaceState>(), &app.state::<DesktopItemState>(), runtime, true).map_err(serialize_error).and_then(serialize_result)),
        "apply_workspace_command" => Some((|| -> Result<Value, Value> {
            let monitor_id: String = command_argument(&arguments, "monitorId", "命令参数无效").map_err(serialize_error)?; // 已解码的 monitorId。
            let command: WorkspaceCommand = command_argument(&arguments, "command", "命令参数无效").map_err(serialize_error)?; // 已解码的 command。
            workspace::execute_apply_workspace_command(app, &app.state::<WorkspaceState>(), &app.state::<DesktopItemState>(), runtime, monitor_id, command).map_err(serialize_error).and_then(serialize_result)
        })()),
        "apply_workspace_interaction" => Some((|| -> Result<Value, Value> {
            let monitor_id: String = command_argument(&arguments, "monitorId", "命令参数无效").map_err(serialize_error)?; // 已解码的 monitorId。
            let command: WorkspaceInteractionCommand = command_argument(&arguments, "command", "命令参数无效").map_err(serialize_error)?; // 已解码的 command。
            workspace::execute_apply_workspace_interaction(app, &app.state::<WorkspaceState>(), runtime, monitor_id, command).map_err(serialize_error).and_then(serialize_result)
        })()),
        _ => None,
    }
}

/// plugin_settings 领域命令。
#[allow(unused_variables)]
pub(crate) fn route_plugin_settings(command: &str, app: &AppHandle, runtime: &DesktopHostRuntime, arguments: &Value) -> Option<Result<Value, Value>>
{
    match command
    {
        "get_plugin_settings" => Some((|| -> Result<Value, Value> {
            let plugin_id: String = command_argument(&arguments, "pluginId", "插件标识无效").map_err(serialize_error)?; // 逐屏载荷用 camelCase。
            let directory = app.state::<crate::preferences::SettingsState>().directory().map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message)))?; // 与设置窗口同一份配置目录。
            let manifests = app.state::<crate::component_runtime::ComponentRuntimeState>().snapshot(&[]).map_err(|error| serialize_error(DesktopHostError::invalid_readiness(error.message)))?.manifests; // 作者取自当前清单。
            let author = crate::preferences::plugin_settings_author(&manifests, &plugin_id).map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message)))?;
            crate::preferences::read_plugin_settings(&directory, &author, &plugin_id).map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message))).and_then(serialize_result)
        })()),
        _ => None,
    }
}

/// tiles 领域命令。
#[allow(unused_variables)]
pub(crate) fn route_tiles(command: &str, app: &AppHandle, runtime: &DesktopHostRuntime, arguments: &Value) -> Option<Result<Value, Value>>
{
    match command
    {
        "create_tile_to_desktop" => Some((|| -> Result<Value, Value> {
            let component_id: String = command_argument(&arguments, "componentId", "命令参数无效").map_err(serialize_error)?; // 已解码的 componentId。
            create_tile_to_desktop(app.clone(), component_id).map_err(serialize_error).and_then(serialize_result)
        })()),
        "enable_feature" => Some((|| -> Result<Value, Value> {
            let component_id: String = command_argument(&arguments, "componentId", "命令参数无效").map_err(serialize_error)?; // 已解码的 componentId。
            enable_feature(app.clone(), component_id).map_err(serialize_error).and_then(serialize_result)
        })()),
        _ => None,
    }
}

/// app 领域命令。
#[allow(unused_variables)]
pub(crate) fn route_app(command: &str, app: &AppHandle, runtime: &DesktopHostRuntime, arguments: &Value) -> Option<Result<Value, Value>>
{
    match command
    {
        "open_settings_window" => Some(open_settings_window(app.clone()).map_err(|message| serialize_error(DesktopHostError::invalid_readiness(message))).and_then(serialize_result)), // 天气格子在原生宿主中请求显示设置窗口。
        _ => None,
    }
}
