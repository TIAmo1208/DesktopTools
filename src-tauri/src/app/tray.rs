use std::io::Cursor;
use tauri::{
    AppHandle, Manager, WindowEvent, Wry,
    image::Image,
    menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

use crate::{app::settings::SettingsState, contracts::settings::ApplicationTheme};

/// 持有托盘显隐菜单，窗口事件与托盘动作共享同一状态来源。
pub struct TrayState
{
    toggle: MenuItem<Wry>, // 随设置窗口实际显隐变化的菜单项。
    icon: Image<'static>,  // 按需创建设置窗口时复用的图标。
}

/// 读取窗口实际状态，最小化窗口视为等待恢复。
fn settings_visible(app: &AppHandle) -> tauri::Result<bool>
{
    let Some(window) = app.get_webview_window("main")
    else
    {
        return Ok(false);
    }; // 未创建或已销毁时视为隐藏。
    Ok(window.is_visible()? && !window.is_minimized()?)
}

/// 仅更新托盘菜单文案，不修改桌面接管状态。
fn sync_menu(app: &AppHandle) -> tauri::Result<()>
{
    if let Some(state) = app.try_state::<TrayState>()
    {
        state.toggle.set_text(if settings_visible(app)? { "隐藏设置" } else { "显示设置" })?;
    }
    Ok(())
}

/// 在主线程按需创建设置窗口，关闭销毁页面而保持桌面宿主运行。
pub(crate) fn set_settings_visible(app: &AppHandle, visible: bool) -> tauri::Result<()>
{
    if visible
    {
        let window = if let Some(window) = app.get_webview_window("main")
        {
            window
        }
        else
        {
            let mut config = app.config().app.windows.iter().find(|window| window.label == "main").cloned().ok_or(tauri::Error::WindowNotFound)?; // 尺寸、装饰和权限仍以原配置为准。
            config.url = tauri::WebviewUrl::CustomProtocol(crate::platform::windows::settings_page_url()); // 页面与 /plugins/ 同源，不再走 Tauri 默认资产协议。
            let window = tauri::WebviewWindowBuilder::from_config(app, &config)?.build()?; // 仅在打开入口创建页面。
            window.set_icon(app.state::<TrayState>().icon.clone())?;
            if let Ok(preferences) = app.state::<SettingsState>().preferences()
            {
                window.set_theme(match preferences.theme
                {
                    ApplicationTheme::System => None,
                    ApplicationTheme::Light => Some(tauri::Theme::Light),
                    ApplicationTheme::Dark => Some(tauri::Theme::Dark),
                })?;
            }
            let handle = app.clone(); // 窗口销毁后的菜单同步句柄。
            window.on_window_event(move |event| {
                if matches!(event, WindowEvent::Destroyed)
                {
                    if let Some(state) = handle.try_state::<crate::weather::WeatherState>()
                    {
                        state.cancel_all();
                    }
                    if let Some(state) = handle.try_state::<TrayState>()
                    {
                        let _ = state.toggle.set_text("显示设置");
                    }
                    crate::platform::windows::audio::watch_settings(&handle, false); // 设置窗口销毁后释放设备通知监听。
                    crate::log_info!("设置：窗口已销毁，页面资源已释放");
                }
                else if matches!(event, WindowEvent::Resized(_) | WindowEvent::Focused(_))
                    && let Err(error) = sync_menu(&handle)
                {
                    crate::log_warn!("托盘：菜单状态更新失败，错误：{error}");
                }
            });
            crate::log_info!("设置：按需创建窗口");
            window
        }; // 连续打开沿用唯一 main 窗口。
        window.unminimize()?;
        window.show()?;
        window.set_focus()?;
    }
    else
    {
        if let Some(window) = app.get_webview_window("main")
        {
            window.close()?;
        }
    }
    sync_menu(app)?;
    crate::log_debug!("设置：窗口显隐已更新，显示：{visible}");
    Ok(())
}

/// 在应用主线程创建 Windows 原生托盘与设置窗口入口。
pub fn install(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>>
{
    let mut decoder = png::Decoder::new(Cursor::new(include_bytes!("../../../resources/application.png"))); // 应用位图的现有 PNG 解码器。
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?; // PNG 图像信息与读取游标。
    let mut pixels = vec![0; reader.output_buffer_size()]; // 图标解码后的 RGBA 缓冲区。
    let info = reader.next_frame(&mut pixels)?; // 当前图标帧的格式及尺寸。
    if info.color_type != png::ColorType::Rgba
    {
        return Err("应用图标必须为 RGBA 位图".into());
    }
    pixels.truncate(info.buffer_size());
    let icon = Image::new_owned(pixels, info.width, info.height); // 标题栏与托盘共用的应用图像。
    let toggle = MenuItem::with_id(app, "toggle-settings", "显示设置", true, None::<&str>)?; // 设置窗口显隐菜单。
    let about = PredefinedMenuItem::about(app, Some("关于 Desktop Tools"), Some(AboutMetadata { name: Some("Desktop Tools".to_string()), version: Some(env!("CARGO_PKG_VERSION").to_string()), icon: Some(icon.clone()), ..Default::default() }))?; // Windows 原生关于对话框入口。
    let separator = PredefinedMenuItem::separator(app)?; // 常规操作与退出之间的菜单分隔线。
    let exit = MenuItem::with_id(app, "exit", "退出", true, None::<&str>)?; // 触发既有正常退出恢复流程。
    let menu = Menu::with_items(app, &[&toggle, &about, &separator, &exit])?; // 由 Windows 绘制的右键菜单。
    TrayIconBuilder::with_id("desktop-tools")
        .icon(icon.clone())
        .tooltip("Desktop Tools")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if matches!(event, TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. })
            {
                let app = tray.app_handle(); // 托盘所属的应用实例。
                if let Err(error) = settings_visible(app).and_then(|visible| set_settings_visible(app, !visible))
                {
                    crate::log_error!("托盘：切换设置窗口失败，错误：{error}");
                }
            }
        })
        .on_menu_event(|app, event| match event.id.as_ref()
        {
            "toggle-settings" =>
            {
                if let Err(error) = settings_visible(app).and_then(|visible| set_settings_visible(app, !visible))
                {
                    crate::log_error!("托盘：切换设置窗口失败，错误：{error}");
                }
            }
            "exit" =>
            {
                crate::log_info!("托盘：请求正常退出");
                app.exit(0);
            }
            _ =>
            {}
        })
        .build(app)?;
    app.manage(TrayState { toggle, icon });
    crate::log_info!("托盘：图标和原生菜单已创建");
    Ok(())
}
