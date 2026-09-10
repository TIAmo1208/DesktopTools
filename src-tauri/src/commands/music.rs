use tauri::command;

/// 读取当前 Windows 媒体会话。
#[command]
pub fn get_current_music_session() -> Result<serde_json::Value, String> { crate::platform::windows::music::snapshot() }

/// 执行当前媒体会话的播放控制动作。
#[command]
pub fn execute_music_action(action: String, expected_session: String) -> Result<(), String> { crate::platform::windows::music::execute_action(&action, &expected_session) }
