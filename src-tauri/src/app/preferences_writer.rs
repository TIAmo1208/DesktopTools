use tauri::{AppHandle, Manager};

use crate::contracts::settings::ApplicationPreferences;
use crate::preferences::{PreferencesWriter, SettingsState};

/// 应用层偏好写入口：平台只提交已校验的落点变更。
pub(crate) struct AppPreferencesWriter
{
    pub app: AppHandle, // 存储所在应用句柄。
}

impl PreferencesWriter for AppPreferencesWriter
{
    fn apply_island_landing(&self, edge: &str, offset: f64) -> Result<Option<ApplicationPreferences>, String> { self.app.state::<SettingsState>().apply_island_landing(edge, offset) }
}
