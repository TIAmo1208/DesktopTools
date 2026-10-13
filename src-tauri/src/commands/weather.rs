use crate::weather::{Command, Snapshot, WeatherState};
use tauri::{Emitter, Manager, WebviewWindow};

/// 两种宿主共用活动登记，限定真实天气实例与其呈现显示器。
#[tauri::command]
pub fn subscribe_weather(app: tauri::AppHandle, instance_id: String, monitor_id: String, city: crate::weather::City) -> Result<u64, String>
{
    let workspace = app.state::<crate::workspace::WorkspaceState>().snapshot().map_err(|error| error.message)?;
    if !workspace.components.iter().any(|value| value.instance_id == instance_id && value.component_id == "official.weather" && value.projection_monitor_id.as_ref().unwrap_or(&value.monitor_id) == &monitor_id)
    {
        return Err("天气实例或显示器无效".into());
    }
    crate::weather::validate_city(&city)?;
    Ok(crate::weather::activity::subscribe(monitor_id, city))
}

/// 旧订阅清理只作用于对应身份。
#[tauri::command]
pub fn unsubscribe_weather(subscription: u64) { crate::weather::activity::unsubscribe(subscription); }

/// 敏感天气用例仅开放给设置窗口，不注册到原生桌面通道。
#[tauri::command]
pub async fn weather_settings(window: WebviewWindow, command: Command) -> Result<Snapshot, String>
{
    if window.label() != "main"
    {
        return Err("天气凭据操作仅适用于设置窗口".into());
    }
    let app = window.app_handle().clone(); // 后台任务只持有应用状态句柄。
    let epoch = app.state::<WeatherState>().window_epoch()?; // 排队期间窗口销毁使命令失效。
    let configuration_changed = matches!(&command, Command::Save { .. } | Command::Clear); // 仅成功提交配置才通知桌面清理旧快照。
    let worker = app.clone(); // 后台任务与事件广播共享同一应用实例。
    let snapshot = tauri::async_runtime::spawn_blocking(move || worker.state::<WeatherState>().execute(command, epoch)).await.map_err(|_| "天气设置任务失败".to_string())??;
    if configuration_changed
    {
        if let Err(error) = app.emit("weather-configuration-changed", ())
        {
            crate::log_warn!("天气配置已保存，但刷新通知失败：{error}");
        }
    }
    Ok(snapshot)
}

/// 获取桌面天气组件使用的最新预报。
#[tauri::command]
pub async fn get_weather_forecast(app: tauri::AppHandle, city: crate::weather::City) -> Result<serde_json::Value, String> { tauri::async_runtime::spawn_blocking(move || app.state::<WeatherState>().forecast(city)).await.map_err(|_| "天气请求失败".to_string())? }

/// 使用已保存天气凭据搜索格子会话中的城市，不持久化 GeoAPI 结果。
#[tauri::command]
pub async fn search_weather_cities(app: tauri::AppHandle, query: String) -> Result<Vec<crate::weather::City>, String> { tauri::async_runtime::spawn_blocking(move || app.state::<WeatherState>().search_saved(query)).await.map_err(|_| "城市搜索失败".to_string())? }

/// 获取已保存的天气城市。
#[tauri::command]
pub async fn get_weather_city(app: tauri::AppHandle) -> Result<Option<crate::weather::City>, String> { tauri::async_runtime::spawn_blocking(move || app.state::<WeatherState>().saved_city()).await.map_err(|_| "天气城市读取失败".to_string())? }

/// 保存当前选中的天气城市。
#[tauri::command]
pub async fn set_weather_city(app: tauri::AppHandle, city: crate::weather::City) -> Result<crate::weather::City, String> { tauri::async_runtime::spawn_blocking(move || app.state::<WeatherState>().save_city(city)).await.map_err(|_| "天气城市保存失败".to_string())? }
