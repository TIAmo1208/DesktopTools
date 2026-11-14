use tauri::{AppHandle, State};

use crate::notifications::{IslandMessage, ToastManager, ToastRequest};

/// 将前端消息提交到 Rust 原生 Toast 队列。
#[tauri::command]
pub fn publish_message_notification(app: AppHandle, state: State<'_, std::sync::Arc<ToastManager>>, message: ToastRequest) -> Result<(), String> { state.inner().publish(&app, message) }

/// 复用程序已解析的界面深浅色与动画状态；Toast palette 由 Rust 保存偏好时更新。
#[tauri::command]
pub fn sync_message_appearance(state: State<'_, std::sync::Arc<ToastManager>>, preferences: crate::contracts::settings::ApplicationPreferences, dark: bool, animations_enabled: bool) { state.sync_appearance(preferences, dark, animations_enabled); }

/// 当前浮岛消息投影：Tauri 命令与原生 transport 共用同一份读取。
pub(crate) fn execute_get_island_messages(state: &std::sync::Arc<ToastManager>) -> Vec<IslandMessage> { state.island_messages() }

/// 关闭一条浮岛消息：两个入口共用原生 Toast 的同一离场路径。
pub(crate) fn execute_dismiss_island_message(app: &AppHandle, state: &std::sync::Arc<ToastManager>, id: u64) -> Result<(), String> { state.dismiss(app, id) }

/// 浮岛页面读取与事件相同的只读投影，用于首次挂载与广播丢失后的补齐。
#[tauri::command]
pub fn get_island_messages(state: State<'_, std::sync::Arc<ToastManager>>) -> Vec<IslandMessage> { execute_get_island_messages(state.inner()) }

/// 浮岛页面关闭一条消息，走原生同一离场路径，不另建第二份队列。
#[tauri::command]
pub fn dismiss_island_message(app: AppHandle, state: State<'_, std::sync::Arc<ToastManager>>, id: u64) -> Result<(), String> { execute_dismiss_island_message(&app, state.inner(), id) }
