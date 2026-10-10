use crate::weather::{Command, Snapshot, WeatherState};
use tauri::{Manager, WebviewWindow};

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
    tauri::async_runtime::spawn_blocking(move || app.state::<WeatherState>().execute(command, epoch)).await.map_err(|_| "天气设置任务失败".to_string())?
}
