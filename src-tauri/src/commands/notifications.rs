use tauri::{AppHandle, State};

use crate::notifications::{ToastManager, ToastRequest};

/// 将前端消息提交到 Rust 原生 Toast 队列。
#[tauri::command]
pub fn publish_message_notification(app: AppHandle, state: State<'_, std::sync::Arc<ToastManager>>, message: ToastRequest) -> Result<(), String> { state.inner().publish(&app, message) }

/// 复用程序已解析的界面深浅色与动画状态，消息模块不另建系统订阅。
#[tauri::command]
pub fn sync_message_appearance(state: State<'_, std::sync::Arc<ToastManager>>, preferences: crate::contracts::settings::ApplicationPreferences, dark: bool, animations_enabled: bool) { state.sync_appearance(preferences, dark, animations_enabled); }
