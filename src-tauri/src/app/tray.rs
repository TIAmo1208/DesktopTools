use std::io::Cursor;
use tauri::{
    AppHandle, Manager, WindowEvent, Wry,
    image::Image,
    menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

use crate::{app::settings::SettingsState, contracts::settings::ApplicationTheme};

/// 持有托盘显隐菜单，窗口事件与托盘动作共享同一状态来源。
pub struct TrayState {
    toggle: MenuItem<Wry>, // 随设置窗口实际显隐变化的菜单项。
}

/// 读取窗口实际状态，最小化窗口视为等待恢复。
fn settings_visible(app: &AppHandle) -> tauri::Result<bool> {
    let window = app
        .get_webview_window("main")
        .ok_or(tauri::Error::WindowNotFound)?; // 唯一设置窗口。
    Ok(window.is_visible()? && !window.is_minimized()?)
}

/// 仅更新托盘菜单文案，不修改桌面接管状态。
fn sync_menu(app: &AppHandle) -> tauri::Result<()> {
    if let Some(state) = app.try_state::<TrayState>() {
        state.toggle.set_text(if settings_visible(app)? {
            "隐藏设置"
        } else {
            "显示设置"
        })?;
    }
    Ok(())
}

/// 显示时恢复最小化并聚焦，隐藏时保持应用和桌面宿主运行。
pub(crate) fn set_settings_visible(app: &AppHandle, visible: bool) -> tauri::Result<()> {
    let window = app
        .get_webview_window("main")
        .ok_or(tauri::Error::WindowNotFound)?; // 本次显隐操作的唯一窗口。
    if visible {
        window.unminimize()?;
        window.show()?;
        window.set_focus()?;
    } else {
        window.hide()?;
    }
    sync_menu(app)?;
    crate::log_debug!("设置：窗口显隐已更新，显示：{visible}");
    Ok(())
}

/// 在应用主线程创建 Windows 原生托盘、菜单和关闭隐藏行为。
pub fn install(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let mut decoder = png::Decoder::new(Cursor::new(include_bytes!(
        "../../../resources/application.png"
    ))); // 应用位图的现有 PNG 解码器。
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?; // PNG 图像信息与读取游标。
    let mut pixels = vec![0; reader.output_buffer_size()]; // 图标解码后的 RGBA 缓冲区。
    let info = reader.next_frame(&mut pixels)?; // 当前图标帧的格式及尺寸。
    if info.color_type != png::ColorType::Rgba {
        return Err("应用图标必须为 RGBA 位图".into());
    }
    pixels.truncate(info.buffer_size());
    let icon = Image::new_owned(pixels, info.width, info.height); // 标题栏与托盘共用的应用图像。
    let toggle = MenuItem::with_id(app, "toggle-settings", "显示设置", true, None::<&str>)?; // 设置窗口显隐菜单。
    let about = PredefinedMenuItem::about(
        app,
        Some("关于 Desktop Tools"),
        Some(AboutMetadata {
            name: Some("Desktop Tools".to_string()),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
            icon: Some(icon.clone()),
            ..Default::default()
        }),
    )?; // Windows 原生关于对话框入口。
    let separator = PredefinedMenuItem::separator(app)?; // 常规操作与退出之间的菜单分隔线。
    let exit = MenuItem::with_id(app, "exit", "退出", true, None::<&str>)?; // 触发既有正常退出恢复流程。
    let menu = Menu::with_items(app, &[&toggle, &about, &separator, &exit])?; // 由 Windows 绘制的右键菜单。
    TrayIconBuilder::with_id("desktop-tools")
        .icon(icon.clone())
        .tooltip("Desktop Tools")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                let app = tray.app_handle(); // 托盘所属的应用实例。
                if let Err(error) =
                    settings_visible(app).and_then(|visible| set_settings_visible(app, !visible))
                {
                    crate::log_error!("托盘：切换设置窗口失败，错误：{error}");
                }
            }
        })
        .on_menu_event(|app, event| match event.id.as_ref() {
            "toggle-settings" => {
                if let Err(error) =
                    settings_visible(app).and_then(|visible| set_settings_visible(app, !visible))
                {
                    crate::log_error!("托盘：切换设置窗口失败，错误：{error}");
                }
            }
            "exit" => {
                crate::log_info!("托盘：请求正常退出");
                app.exit(0);
            }
            _ => {}
        })
        .build(app)?;
    app.manage(TrayState { toggle });
    let window = app
        .get_webview_window("main")
        .ok_or(tauri::Error::WindowNotFound)?; // 已配置且默认隐藏的设置窗口。
    window.set_icon(icon)?;
    let preferences = app.state::<SettingsState>().preferences()?; // 初始化标题栏主题所需的已保存偏好。
    window.set_theme(match preferences.theme {
        ApplicationTheme::System => None,
        ApplicationTheme::Light => Some(tauri::Theme::Light),
        ApplicationTheme::Dark => Some(tauri::Theme::Dark),
    })?;
    let handle = app.clone(); // 窗口事件持有的应用句柄。
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            if let Err(error) = set_settings_visible(&handle, false) {
                crate::log_error!("设置：隐藏窗口失败，错误：{error}");
            }
        } else if matches!(event, WindowEvent::Resized(_) | WindowEvent::Focused(_))
            && let Err(error) = sync_menu(&handle)
        {
            crate::log_warn!("托盘：菜单状态更新失败，错误：{error}");
        }
    });
    crate::log_info!("托盘：图标和原生菜单已创建");
    Ok(())
}
