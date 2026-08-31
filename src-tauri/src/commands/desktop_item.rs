use tauri::{AppHandle, Emitter, State};

use crate::contracts::{
    DESKTOP_ITEM_SNAPSHOT_CHANGED_EVENT, DesktopItemError, DesktopItemSnapshot,
};
use crate::shell::DesktopItemState;

/// 枚举真实 Shell 桌面项目并发布新的不可变快照。
#[tauri::command]
pub fn get_desktop_item_snapshot(
    app: AppHandle,
    state: State<'_, DesktopItemState>,
) -> Result<DesktopItemSnapshot, DesktopItemError> {
    // 已发布到内存状态的桌面项目快照。
    let snapshot = refresh_desktop_item_snapshot(&state)?;
    app.emit(DESKTOP_ITEM_SNAPSHOT_CHANGED_EVENT, &snapshot)
        .map_err(|error| {
            crate::log_error!("Shell：桌面项目快照事件发布失败，错误：{error}");
            DesktopItemError::event_publish_failed(&error)
        })?;

    Ok(snapshot)
}

/// 枚举 transport 共享的桌面项目快照。
pub(crate) fn refresh_desktop_item_snapshot(
    state: &DesktopItemState,
) -> Result<DesktopItemSnapshot, DesktopItemError> {
    state.refresh()
}

/// 返回本地事件轮询使用的当前桌面项目快照。
pub(crate) fn read_desktop_item_snapshot(
    state: &DesktopItemState,
) -> Result<DesktopItemSnapshot, DesktopItemError> {
    state.snapshot()
}
