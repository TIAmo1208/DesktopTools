mod app;
mod commands;
pub mod contracts;

use app::ApplicationState;
use commands::{get_application_snapshot, set_application_status};

/// 创建并运行 Desktop Tools 正式应用。
pub fn run() {
    tauri::Builder::default()
        .manage(ApplicationState::default())
        .invoke_handler(tauri::generate_handler![
            get_application_snapshot,
            set_application_status
        ])
        .run(tauri::generate_context!())
        .expect("FATAL Desktop Tools 正式应用运行失败");
}
