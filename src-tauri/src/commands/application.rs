use tauri::{AppHandle, Emitter, State};

use crate::app::ApplicationState;
use crate::contracts::{APPLICATION_SNAPSHOT_CHANGED_EVENT, ApplicationError, ApplicationSnapshot};

/// 返回正式应用当前权威快照。
#[tauri::command]
pub fn get_application_snapshot(state: State<'_, ApplicationState>) -> Result<ApplicationSnapshot, ApplicationError>
{
    // 返回给前端的当前应用快照。
    let snapshot = state.snapshot()?;
    crate::log_debug!("应用：读取状态快照，修订：{}", snapshot.revision);
    Ok(snapshot)
}

/// 校验并提交应用状态，然后发布新的权威快照。
#[tauri::command]
pub fn set_application_status(app: AppHandle, state: State<'_, ApplicationState>, status: String) -> Result<ApplicationSnapshot, ApplicationError>
{
    // 已提交的权威应用快照。
    let snapshot = state.set_status(&status)?;
    app.emit(APPLICATION_SNAPSHOT_CHANGED_EVENT, &snapshot).map_err(|error| {
        crate::log_error!("应用：状态事件发布失败，错误：{error}");
        ApplicationError::event_publish_failed(&error)
    })?;
    crate::log_debug!("应用：状态已更新，状态：{:?}，修订：{}", snapshot.status, snapshot.revision);

    Ok(snapshot)
}
