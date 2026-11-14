use crate::platform::windows::island::{self, IslandStateRequest};
use tauri::{AppHandle, Manager};

/// 岛实例与注册投影独立返回，复用现有清单与实例契约，不写工作区。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IslandSnapshot
{
    instances: Vec<crate::contracts::ComponentInstanceSnapshot>, // 按偏好顺序合成的岛实例。
    runtime: crate::contracts::ComponentRuntimeSnapshot,         // 完整清单与岛实例授权信息。
}

/// 同源原生桥与设置窗口共用的只读岛快照。
pub(crate) fn execute_get_island_snapshot(app: &AppHandle) -> Result<IslandSnapshot, String>
{
    let preferences = app.state::<crate::app::settings::SettingsState>().preferences()?; // 与拖动及设置保存共用权威偏好。
    let (instances, runtime) = app.state::<crate::component_runtime::ComponentRuntimeState>().island_snapshot(&preferences.island_card_order).map_err(|error| error.message)?;
    Ok(IslandSnapshot { instances, runtime })
}

/// 快照读取不发布事件，清单变化沿用组件事件、顺序变化沿用偏好事件触发重读。
#[tauri::command]
pub(crate) fn get_island_snapshot(app: AppHandle) -> Result<IslandSnapshot, String> { execute_get_island_snapshot(&app) }

/// 独立协议命令在线程池等待；完整校验与失败隐藏一并投递给 UI 线程，避免非法动画帧留下中间态。
pub(crate) fn execute_set_island_state(app: &AppHandle, request: IslandStateRequest) -> Result<(), String>
{
    let (sender, receiver) = std::sync::mpsc::sync_channel(1); // 等待一次原生几何提交结果。
    app.run_on_main_thread(move || {
        let _ = sender.send(island::set_state(request));
    })
    .map_err(|error| error.to_string())?;
    receiver.recv().map_err(|error| format!("岛状态更新中断：{error}"))?
}

/// 常态或 hover 卡片非交互区域的原生拖动入口。
pub(crate) fn execute_begin_island_drag(app: &AppHandle) -> Result<island::IslandDragResult, String>
{
    let (sender, receiver) = std::sync::mpsc::sync_channel(1); // UI返回原生循环结果及后台保存屏障。
    app.run_on_main_thread(move || {
        let _ = sender.send(island::begin_drag());
    })
    .map_err(|error| error.to_string())?;
    let saved = receiver.recv().map_err(|error| format!("岛拖动中断：{error}"))??;
    Ok(island::wait_for_landing(saved, std::time::Duration::from_secs(2))) // 等待仅发生在命令线程；超时返回 false 与 null，不改变已完成的原生拖动结果。
}

/// 接收状态名和调用方实测的窗口联合盒及卡片区域，不定义视觉尺寸。
#[tauri::command]
pub(crate) async fn set_island_state(app: AppHandle, request: IslandStateRequest) -> Result<(), String> { tauri::async_runtime::spawn_blocking(move || execute_set_island_state(&app, request)).await.map_err(|error| error.to_string())? }

/// 先释放 WebView 鼠标捕获，再交给 Windows 移动循环。
#[tauri::command]
pub(crate) async fn begin_island_drag(app: AppHandle) -> Result<island::IslandDragResult, String> { tauri::async_runtime::spawn_blocking(move || execute_begin_island_drag(&app)).await.map_err(|error| error.to_string())? }
