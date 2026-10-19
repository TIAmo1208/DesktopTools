use crate::weather::{City, Command, Snapshot, Source, SourceStatus, WeatherState};
use tauri::{Emitter, Manager, WebviewWindow};

/// 两种宿主共用活动登记，限定真实天气实例与其呈现显示器。
#[tauri::command]
pub fn subscribe_weather(app: tauri::AppHandle, instance_id: String, monitor_id: String, source: Source, city_id: String) -> Result<u64, String>
{
    let workspace = app.state::<crate::workspace::WorkspaceState>().snapshot().map_err(|error| error.message)?;
    if !workspace.components.iter().any(|value| value.instance_id == instance_id && value.component_id == "official.weather" && value.projection_monitor_id.as_ref().unwrap_or(&value.monitor_id) == &monitor_id)
    {
        return Err("天气实例或显示器无效".into());
    }
    let city = app.state::<WeatherState>().validated_city(source, &city_id)?; // 格子只提交两个标识，城市对象由该来源列表解析。
    Ok(crate::weather::activity::subscribe(source, monitor_id, city))
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
    let configuration_changed = matches!(&command, Command::Save { .. } | Command::Remove { .. }); // 仅成功提交配置才通知桌面清理旧快照。
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

/// 读取已配置来源列表；桌面格子菜单与设置页共用同一份权威结果。
#[tauri::command]
pub fn list_weather_sources(app: tauri::AppHandle) -> Result<Vec<SourceStatus>, String>
{
    let state = app.state::<WeatherState>();
    Ok(state.execute(Command::Status, state.window_epoch()?)?.sources)
}

/// 读取某来源已添加的城市列表。
#[tauri::command]
pub fn get_weather_cities(app: tauri::AppHandle, source: Source) -> Result<Vec<City>, String> { app.state::<WeatherState>().cities(source) }

/// 使用已保存天气凭据搜索某来源的城市，不持久化 GeoAPI 结果。
#[tauri::command]
pub async fn search_weather_cities(app: tauri::AppHandle, source: Source, query: String) -> Result<Vec<City>, String> { tauri::async_runtime::spawn_blocking(move || app.state::<WeatherState>().search_saved(source, query)).await.map_err(|_| "城市搜索失败".to_string())? }

/// 按来源验证候选城市：候选来自搜索结果，入列前即可校验该来源规则，详情链接规则由该来源决定。
#[tauri::command]
pub fn validate_weather_city(source: Source, city: City) -> Result<City, String>
{
    crate::weather::validate_city_for_source(source, &city)?; // 校验只依赖来源规则与候选对象，不读取运行期状态。
    Ok(city)
}

/// 把候选城市加入某来源的城市列表；桌面与设置页的“添加城市”共用该入口，信任边界由 Rust 校验。
#[tauri::command]
pub fn add_weather_city(app: tauri::AppHandle, source: Source, city: City) -> Result<Vec<City>, String> { crate::commands::add_weather_city(&app, source, &city) }

/// 按标识从某来源的城市列表移除一项并返回更新后的完整列表；正在使用该城市的格子保留自己的绑定。
#[tauri::command]
pub fn remove_weather_city(app: tauri::AppHandle, source: Source, city_id: String) -> Result<Vec<City>, String> { crate::commands::remove_weather_city(&app, source, &city_id) }

/// 获取格子绑定的来源与城市标识对应的最新预报；城市对象由 Rust 从该来源列表解析。
#[tauri::command]
pub async fn get_weather_forecast(app: tauri::AppHandle, source: Source, city_id: String) -> Result<serde_json::Value, String> { tauri::async_runtime::spawn_blocking(move || app.state::<WeatherState>().forecast_for_city(source, &city_id)).await.map_err(|_| "天气请求失败".to_string())? }

/// 创建绑定来源与城市的天气格子；来源菜单与 Explorer 菜单共用同一创建入口。
#[tauri::command]
pub fn create_weather_tile(app: tauri::AppHandle, source: Source, city_id: String, monitor_id: Option<String>) -> Result<String, String>
{
    let runtime = app.state::<std::sync::Arc<crate::host::DesktopHostRuntime>>(); // 未指定显示器时使用主显示器。
    let monitor = match monitor_id
    {
        Some(monitor_id) => monitor_id,
        None => crate::commands::primary_monitor_id(&runtime)?,
    };
    crate::commands::execute_create_weather_tile(&app, Some((source, city_id)), monitor)
}
