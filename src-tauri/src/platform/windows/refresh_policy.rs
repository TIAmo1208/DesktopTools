use crate::contracts::DesktopHostMonitor;

static PAUSED_HOSTS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<String>>> = std::sync::LazyLock::new(Default::default); // 页面实际暂停状态，不替代健康心跳。

/// 保存页面已应用的暂停状态，新建或重载页面时清除旧值。
pub(crate) fn set_paused(monitor: &str, paused: bool)
{
    let mut hosts = PAUSED_HOSTS.lock().unwrap_or_else(|error| error.into_inner()); // 当前暂停集合。
    if paused
    {
        hosts.insert(monitor.to_string());
    }
    else
    {
        hosts.remove(monitor);
    }
}

/// 判断是否需要停止向该屏交付视觉事件。
pub(crate) fn is_paused(monitor: &str) -> bool { PAUSED_HOSTS.lock().unwrap_or_else(|error| error.into_inner()).contains(monitor) }

/// 前台钩子线程持有事件出口，回调只发布重新检查通知。
static EVENT_APP: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();
static EVENT_FALLBACK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true); // 全部 Hook 成功前保守保留低频检查。

/// 返回页面是否需要保留 5 秒故障兜底。
pub(crate) fn fallback_enabled() -> bool { EVENT_FALLBACK.load(std::sync::atomic::Ordering::Acquire) }

/// 用 Windows 前台及窗口边界事件唤醒页面，失败时页面仍有低频检查。
pub(crate) fn start_events(app: &tauri::AppHandle)
{
    use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
    use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE, EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_SHOW, EVENT_SYSTEM_DESKTOPSWITCH, EVENT_SYSTEM_FOREGROUND, GetMessageW, MSG, WINEVENT_OUTOFCONTEXT};
    if EVENT_APP.set(app.clone()).is_err()
    {
        return;
    }
    let (ready, initialized) = std::sync::mpsc::sync_channel(0); // 宿主创建前确定是否需要页面低频兜底。
    unsafe extern "system" fn notify(_: HWINEVENTHOOK, event: u32, window: windows::Win32::Foundation::HWND, object: i32, _: i32, _: u32, _: u32)
    {
        use tauri::Emitter;
        if !matches!(event, EVENT_SYSTEM_FOREGROUND | EVENT_SYSTEM_DESKTOPSWITCH) && (object != 0 || window != unsafe { GetForegroundWindow() })
        {
            return;
        }
        if let Some(app) = EVENT_APP.get()
        {
            let _ = app.emit("desktop-refresh-policy-changed", ());
        }
    }
    std::thread::spawn(move || unsafe {
        let mut hooks = Vec::new(); // 线程生命周期内保持钩子句柄。
        let events = [EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_DESKTOPSWITCH, EVENT_OBJECT_SHOW, EVENT_OBJECT_HIDE, EVENT_OBJECT_DESTROY, EVENT_OBJECT_LOCATIONCHANGE]; // 前台、解锁/桌面切换和窗口可见边界事件。
        for event in events
        {
            let hook = SetWinEventHook(event, event, None, Some(notify), 0, 0, WINEVENT_OUTOFCONTEXT); // 不注入其他进程。
            if hook.0.is_null()
            {
                crate::log_warn!("桌面刷新：窗口事件钩子注册失败，事件={event}，保留低频检查");
            }
            else
            {
                hooks.push(hook);
            }
        }
        let fallback = hooks.len() != events.len(); // 任一 Hook 失败才保留页面低频兜底。
        EVENT_FALLBACK.store(fallback, std::sync::atomic::Ordering::Release);
        let _ = ready.send(());
        let mut message = MSG::default(); // WinEvent 回调要求安装线程持续泵消息。
        while GetMessageW(&mut message, None, 0, 0).0 > 0
        {
            DispatchMessageW(&message);
        }
        for hook in hooks
        {
            let _ = UnhookWinEvent(hook);
        }
    });
    if initialized.recv().is_err()
    {
        EVENT_FALLBACK.store(true, std::sync::atomic::Ordering::Release);
    }
}
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::UI::WindowsAndMessaging::{GWL_EXSTYLE, GetClassNameW, GetForegroundWindow, GetWindowLongPtrW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, IsZoomed, WS_EX_LAYERED, WS_EX_TRANSPARENT};

/// 读取当前前台窗口事实；无法确认覆盖或窗口归属时保持桌面刷新。
pub(crate) fn should_pause(monitor: &DesktopHostMonitor, on_fullscreen: bool, on_focus: bool) -> bool
{
    if !on_fullscreen && !on_focus
    {
        return false;
    }
    unsafe {
        let window = GetForegroundWindow(); // 当前前台顶层窗口。
        if window.0.is_null() || !IsWindowVisible(window).as_bool() || IsIconic(window).as_bool()
        {
            return false;
        }
        let mut process_id = 0; // 排除本程序设置和宿主。
        GetWindowThreadProcessId(window, Some(&mut process_id));
        if process_id == 0 || process_id == std::process::id()
        {
            return false;
        }
        let mut class = [0_u16; 256]; // 系统桌面与瞬态 Shell 窗口类。
        let length = GetClassNameW(window, &mut class); // 实际类名长度。
        if length == 0
        {
            return false;
        }
        let class = String::from_utf16_lossy(&class[..length as usize]); // 可比较的 Windows 类名。
        if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "#32768" | "XamlExplorerHostIslandWindow" | "Windows.UI.Core.CoreWindow")
        {
            return false;
        }
        let mut cloaked = 0_u32; // 虚拟桌面或系统隐藏状态。
        if DwmGetWindowAttribute(window, DWMWA_CLOAKED, (&mut cloaked as *mut u32).cast(), size_of::<u32>() as u32).is_err() || cloaked != 0
        {
            return false;
        }
        if on_focus
        {
            return true;
        }
        let style = GetWindowLongPtrW(window, GWL_EXSTYLE) as u32; // 保守排除透明与分层窗口。
        if style & (WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0) != 0
        {
            return false;
        }
        let mut bounds = RECT::default(); // 不含阴影的可见窗口边界。
        if DwmGetWindowAttribute(window, DWMWA_EXTENDED_FRAME_BOUNDS, (&mut bounds as *mut RECT).cast(), size_of::<RECT>() as u32).is_err()
        {
            return false;
        }
        let covers_work = covers(bounds, monitor.work_x, monitor.work_y, monitor.work_width, monitor.work_height); // 最大化窗口覆盖工作区。
        let covers_screen = covers(bounds, monitor.x, monitor.y, monitor.width, monitor.height); // 无边框全屏覆盖物理屏幕。
        (IsZoomed(window).as_bool() && covers_work) || covers_screen
    }
}

/// 用宽整数比较逐屏覆盖，支持负坐标与跨屏窗口。
fn covers(bounds: RECT, x: i32, y: i32, width: u32, height: u32) -> bool { bounds.left <= x && bounds.top <= y && i64::from(bounds.right) >= i64::from(x) + i64::from(width) && i64::from(bounds.bottom) >= i64::from(y) + i64::from(height) }

#[cfg(test)]
#[path = "refresh_policy.test.rs"]
mod tests;
