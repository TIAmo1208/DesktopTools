use crate::platform::windows::island::{self, IslandStateRequest};
use tauri::AppHandle;

/// 独立协议的命令在线程池调用此入口；所有 Win32 操作只投递给 UI 线程。
pub(crate) fn execute_set_island_state(app: &AppHandle, request: IslandStateRequest) -> Result<(), String>
{
    request.validate()?;
    let (sender, receiver) = std::sync::mpsc::sync_channel(1); // 等待一次原生几何提交结果。
    app.run_on_main_thread(move || {
        let _ = sender.send(island::set_state(request));
    })
    .map_err(|error| error.to_string())?;
    receiver.recv().map_err(|error| format!("岛状态更新中断：{error}"))?
}

/// 常态岛体空白区域专用的原生拖动入口。
pub(crate) fn execute_begin_island_drag(app: &AppHandle) -> Result<(), String>
{
    let (sender, receiver) = std::sync::mpsc::sync_channel(1); // 原生移动循环结束后完成命令。
    app.run_on_main_thread(move || {
        let _ = sender.send(island::begin_drag());
    })
    .map_err(|error| error.to_string())?;
    receiver.recv().map_err(|error| format!("岛拖动中断：{error}"))?
}

/// 接收状态名和调用方实测的窗口联合盒及卡片区域，不定义视觉尺寸。
#[tauri::command]
pub(crate) async fn set_island_state(app: AppHandle, request: IslandStateRequest) -> Result<(), String> { tauri::async_runtime::spawn_blocking(move || execute_set_island_state(&app, request)).await.map_err(|error| error.to_string())? }

/// 先释放 WebView 鼠标捕获，再交给 Windows 移动循环。
#[tauri::command]
pub(crate) async fn begin_island_drag(app: AppHandle) -> Result<(), String> { tauri::async_runtime::spawn_blocking(move || execute_begin_island_drag(&app)).await.map_err(|error| error.to_string())? }
