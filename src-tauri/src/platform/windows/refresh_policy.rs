use serde::Serialize;
use std::sync::{
    LazyLock, Mutex, OnceLock,
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};
use tauri::{Emitter, Listener, Manager};
use windows::Win32::{
    Foundation::{HWND, LPARAM, RECT, WPARAM},
    Graphics::Dwm::{DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute},
    System::{RemoteDesktop::*, Threading::GetCurrentThreadId},
    UI::{
        Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
        Shell::{QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState},
        WindowsAndMessaging::*,
    },
};

/// 一台显示器被当前前台窗口覆盖的事实，不含用户策略。
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Coverage
{
    pub monitor_id: String, // 稳定显示器身份。
    pub work: bool,         // 完全覆盖工作区。
    pub screen: bool,       // 完全覆盖物理屏幕。
}

/// 内存中的唯一前台快照；Option 的 None 表示未确认。
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ForegroundSnapshot
{
    pub revision: u64,                      // 实际发布次序。
    pub window: Option<String>,             // HWND 的十六进制身份，避免 JS 整数精度损失。
    pub process_id: Option<u32>,            // 窗口所属进程。
    pub external_focus: Option<bool>,       // 外部前台归属。
    pub maximized: Option<bool>,            // 独立最大化属性。
    pub fullscreen: Option<bool>,           // 几何或系统全屏证据的合并结果。
    pub exclusive_fullscreen: Option<bool>, // 系统级独占全屏证据，不归属到 HWND。
    pub locked: Option<bool>,               // 当前会话锁定状态。
    pub coverage: Option<Vec<Coverage>>,    // None 表示覆盖查询失败。
}
static SNAPSHOT: LazyLock<Mutex<ForegroundSnapshot>> = LazyLock::new(Default::default); // 全部消费者共用快照。
static PAUSED_HOSTS: LazyLock<Mutex<std::collections::HashSet<String>>> = LazyLock::new(Default::default); // 页面实际应用状态。
static EVENT_APP: OnceLock<tauri::AppHandle> = OnceLock::new(); // 发布出口。
static EVENT_THREAD: AtomicU32 = AtomicU32::new(0); // 已建立消息队列的线程。
static GENERATION: AtomicU64 = AtomicU64::new(0); // 新事件使在途采样失效。
static SWITCHING: AtomicBool = AtomicBool::new(false); // Alt+Tab 显式切换阶段。
static EVENT_FALLBACK: AtomicBool = AtomicBool::new(true); // Hook 失败时保留页面低频兜底。
const SAMPLE: u32 = WM_APP + 71; // 合并采样通知。

/// 返回是否需要保留页面故障兜底。
pub(crate) fn fallback_enabled() -> bool { EVENT_FALLBACK.load(Ordering::Acquire) }

/// 保存实际应用的单屏暂停状态。
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

/// 后端只过滤已实际暂停屏幕的视觉事件。
pub(crate) fn is_paused(monitor: &str) -> bool { SNAPSHOT.lock().unwrap_or_else(|error| error.into_inner()).locked == Some(true) || PAUSED_HOSTS.lock().unwrap_or_else(|error| error.into_inner()).contains(monitor) }

/// 返回同一份缓存，不按页面重新查询系统。
pub(crate) fn foreground_snapshot() -> ForegroundSnapshot { SNAPSHOT.lock().unwrap_or_else(|error| error.into_inner()).clone() }

/// 唤醒一次采样；正常运行没有永久周期检查。
pub(crate) fn request_sample()
{
    GENERATION.fetch_add(1, Ordering::AcqRel);
    let thread = EVENT_THREAD.load(Ordering::Acquire); // 初始化完成前不发线程消息。
    if thread != 0
    {
        let _ = unsafe { PostThreadMessageW(thread, SAMPLE, WPARAM(0), LPARAM(0)) };
    }
}

/// 发布实际变化，原生 UI 直接消费相同缓存。
fn publish(mut next: ForegroundSnapshot, generation: u64)
{
    let mut current = SNAPSHOT.lock().unwrap_or_else(|error| error.into_inner()); // 原子比较和提交。
    if generation != GENERATION.load(Ordering::Acquire)
    {
        return;
    }
    next.revision = current.revision;
    next.locked = current.locked;
    if *current == next
    {
        return;
    }
    next.revision += 1;
    *current = next.clone();
    drop(current);
    if let Some(app) = EVENT_APP.get()
    {
        let _ = app.emit("desktop-refresh-policy-changed", next);
    }
}

/// 消息准备显示前只查一次系统证据，再请求事件驱动采样。
pub(crate) fn check_before_notification()
{
    let generation = GENERATION.load(Ordering::Acquire); // 按需读取不能覆盖更新代次。
    let evidence = unsafe { SHQueryUserNotificationState() }.ok().map(|state| state == QUNS_RUNNING_D3D_FULL_SCREEN); // 失败保留未知。
    let mut next = foreground_snapshot(); // 不将系统级证据归给某块屏幕。
    let window = unsafe { GetForegroundWindow() }; // 缓存的窗口已改变时不沿用旧覆盖。
    if next.window.as_deref() != Some(&format!("{:x}", window.0 as usize))
    {
        next = ForegroundSnapshot { locked: next.locked, ..Default::default() };
    }
    next.exclusive_fullscreen = evidence;
    next.fullscreen = combine_fullscreen(next.coverage.as_ref(), evidence);
    if !SWITCHING.load(Ordering::Acquire)
    {
        publish(next, generation);
    }
    request_sample();
}

/// 抑制未知、锁屏和全屏状态，不影响业务消息入队。
pub(crate) fn notifications_suppressed() -> bool
{
    let snapshot = foreground_snapshot(); // 一致读取。
    SWITCHING.load(Ordering::Acquire) || snapshot.locked != Some(false) || snapshot.fullscreen != Some(false)
}

/// 为不依赖真实前台采样的原生通知绘制测试提供确定的可见状态。
#[cfg(test)]
pub(crate) fn assume_notifications_visible_for_test()
{
    if let Ok(mut snapshot) = SNAPSHOT.lock()
    {
        snapshot.locked = Some(false);
        snapshot.fullscreen = Some(false);
        snapshot.exclusive_fullscreen = Some(false);
        snapshot.coverage = Some(Vec::new());
    }
}

/// 合并独立证据；任一确认全屏即可抑制，全部否定才确认非全屏。
fn combine_fullscreen(coverage: Option<&Vec<Coverage>>, exclusive: Option<bool>) -> Option<bool>
{
    if exclusive == Some(true) || coverage.is_some_and(|screens| screens.iter().any(|screen| screen.screen))
    {
        return Some(true);
    }
    if coverage.is_some() && exclusive == Some(false) { Some(false) } else { None }
}

/// 当前会话通知先注册再查询，通知与初始读取在同一窗口线程串行。
pub(crate) fn install_session_notifications(hwnd: HWND) -> Result<(), String>
{
    unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) }.map_err(|error| format!("注册锁屏通知失败：{error}"))?;
    update_session(query_locked());
    Ok(())
}

/// 精确查询会话锁定状态，不读取输入桌面名称或通知适宜状态。
fn query_locked() -> Option<bool>
{
    unsafe {
        let mut buffer = windows::core::PWSTR::null(); // 系统分配的会话结构。
        let mut bytes = 0; // 验证结构长度。
        if WTSQuerySessionInformationW(None, WTS_CURRENT_SESSION, WTSSessionInfoEx, &mut buffer, &mut bytes).is_err()
        {
            return None;
        }
        let result = if !buffer.is_null() && bytes as usize >= size_of::<WTSINFOEXW>()
        {
            let info = &*buffer.0.cast::<WTSINFOEXW>(); // 已验证长度。
            if info.Level == 1
            {
                match info.Data.WTSInfoExLevel1.SessionFlags
                {
                    0 => Some(true),
                    1 => Some(false),
                    _ => None,
                }
            }
            else
            {
                None
            }
        }
        else
        {
            None
        }; // 未知结构不推断已解锁。
        WTSFreeMemory(buffer.0.cast());
        result
    }
}

/// 窗口过程传入 WTS 会话事件；锁屏立即发布，不等待切换收敛。
pub(crate) fn session_changed(event: usize)
{
    match event as u32
    {
        WTS_SESSION_LOCK => update_session(Some(true)),
        WTS_SESSION_UNLOCK => update_session(Some(false)),
        _ => request_sample(),
    }
}

/// 锁屏使旧采样失效；解锁先清除旧窗口事实再重新读取。
fn update_session(locked: Option<bool>)
{
    GENERATION.fetch_add(1, Ordering::AcqRel);
    SWITCHING.store(false, Ordering::Release);
    let mut current = SNAPSHOT.lock().unwrap_or_else(|error| error.into_inner()); // 独立更新锁屏事实。
    if current.locked != locked
    {
        *current = ForegroundSnapshot { revision: current.revision + 1, locked, ..Default::default() };
        let next = current.clone(); // 释放锁后发布。
        drop(current);
        if let Some(app) = EVENT_APP.get()
        {
            let _ = app.emit("desktop-refresh-policy-changed", next);
        }
    }
    request_sample();
}

/// 注销会话通知并终止采样消息循环。
pub(crate) fn stop_events(hwnd: HWND)
{
    let _ = unsafe { WTSUnRegisterSessionNotification(hwnd) };
    let thread = EVENT_THREAD.swap(0, Ordering::AcqRel); // 退出后拒绝再次唤醒。
    if thread != 0
    {
        let _ = unsafe { PostThreadMessageW(thread, WM_QUIT, WPARAM(0), LPARAM(0)) };
    }
}

/// 查询失败后最多三次；成功、新事件和锁屏均取消原预算。
fn retry_delay(attempt: usize) -> Option<u32> { [100, 300, 1000].get(attempt).copied() }

/// 监听前台窗口并在唯一线程合并事件、采样和有限复查。
pub(crate) fn start_events(app: &tauri::AppHandle)
{
    if EVENT_APP.set(app.clone()).is_err()
    {
        return;
    }
    let handle = app.clone(); // 原生 UI 更新必须调度回主线程。
    app.listen("desktop-refresh-policy-changed", move |_| {
        let app = handle.clone(); // 主线程回调持有应用。
        let _ = handle.run_on_main_thread(move || {
            if let Some(toast) = app.try_state::<std::sync::Arc<crate::notifications::ToastManager>>()
            {
                toast.refresh_policy_changed();
            }
        });
    });
    let (ready, initialized) = std::sync::mpsc::sync_channel(0); // 确保线程消息队列先就绪。
    std::thread::spawn(move || unsafe {
        let mut message = MSG::default(); // WinEvent 需要持续消息循环。
        let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
        EVENT_THREAD.store(GetCurrentThreadId(), Ordering::Release);
        let events = [EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_DESKTOPSWITCH, EVENT_SYSTEM_SWITCHSTART, EVENT_SYSTEM_SWITCHEND, EVENT_SYSTEM_MOVESIZEEND, EVENT_OBJECT_SHOW, EVENT_OBJECT_HIDE, EVENT_OBJECT_DESTROY, EVENT_OBJECT_LOCATIONCHANGE]; // 有限事件源。
        let mut hooks = Vec::new(); // 线程退出时成对释放。
        let mut failed = false; // 任一 Hook 失败才启用页面兜底。
        for event in events
        {
            let hook = SetWinEventHook(event, event, None, Some(notify), 0, 0, WINEVENT_OUTOFCONTEXT); // 不注入外部进程。
            if hook.0.is_null()
            {
                failed = true;
                crate::log_error!("前台快照：窗口事件注册失败，事件={event}");
            }
            else
            {
                hooks.push(hook);
            }
        }
        EVENT_FALLBACK.store(failed, Ordering::Release);
        let _ = ready.send(());
        request_sample();
        let mut timer = 0; // 只用于事件合并与有限复查。
        let mut attempt = 0; // 当前事件代次已用复查次数。
        while GetMessageW(&mut message, None, 0, 0).0 > 0
        {
            if message.message == SAMPLE
            {
                if timer != 0
                {
                    let _ = KillTimer(None, timer);
                    timer = 0;
                }
                attempt = 0;
                if !SWITCHING.load(Ordering::Acquire) && foreground_snapshot().locked != Some(true)
                {
                    timer = SetTimer(None, 0, 50, None);
                }
            }
            else if message.message == WM_TIMER && message.wParam.0 == timer
            {
                let _ = KillTimer(None, timer);
                timer = 0;
                let generation = GENERATION.load(Ordering::Acquire); // 采样期间任何新事件均取消提交。
                let mut next = sample(); // 同一次系统读取供全部屏幕共享。
                if next.locked.is_none()
                {
                    let locked = query_locked(); // 初始会话查询失败时使用同一有限预算。
                    if locked.is_some()
                    {
                        update_session(locked);
                        continue;
                    }
                }
                if generation != GENERATION.load(Ordering::Acquire) || SWITCHING.load(Ordering::Acquire)
                {
                    continue;
                }
                if next.external_focus.is_some() && next.coverage.is_some() && next.exclusive_fullscreen.is_some() && next.locked.is_some()
                {
                    publish(next, generation);
                }
                else if let Some(delay) = retry_delay(attempt)
                {
                    attempt += 1;
                    timer = SetTimer(None, 0, delay, None);
                }
                else
                {
                    next.window = None;
                    next.process_id = None;
                    next.external_focus = None;
                    next.maximized = None;
                    next.coverage = None;
                    next.fullscreen = combine_fullscreen(None, next.exclusive_fullscreen);
                    publish(next, generation);
                    crate::log_debug!("前台快照：有限复查耗尽，等待新事件");
                }
            }
            else
            {
                DispatchMessageW(&message);
            }
        }
        if timer != 0
        {
            let _ = KillTimer(None, timer);
        }
        for hook in hooks
        {
            let _ = UnhookWinEvent(hook);
        }
    });
    if initialized.recv().is_err()
    {
        crate::log_error!("前台快照：事件线程初始化失败");
    }
}

/// 回调只记录切换阶段并唤醒，避免系统回调内重复查询。
unsafe extern "system" fn notify(_: HWINEVENTHOOK, event: u32, window: HWND, object: i32, _: i32, _: u32, _: u32)
{
    if event >= EVENT_OBJECT_SHOW && (object != 0 || (window != unsafe { GetForegroundWindow() } && foreground_snapshot().window.as_deref() != Some(&format!("{:x}", window.0 as usize))))
    {
        return;
    }
    if event == EVENT_SYSTEM_SWITCHSTART
    {
        SWITCHING.store(true, Ordering::Release);
    }
    if event == EVENT_SYSTEM_SWITCHEND
    {
        SWITCHING.store(false, Ordering::Release);
    }
    request_sample();
}

/// 一次读取前台窗口所有事实；前后台身份变化时拒绝整次结果。
fn sample() -> ForegroundSnapshot
{
    let mut next = ForegroundSnapshot { locked: foreground_snapshot().locked, ..Default::default() }; // 不覆盖独立会话事实。
    unsafe {
        next.exclusive_fullscreen = SHQueryUserNotificationState().ok().map(|state| state == QUNS_RUNNING_D3D_FULL_SCREEN);
        let window = GetForegroundWindow(); // 采样前身份。
        if window.0.is_null()
        {
            return next;
        }
        next.window = Some(format!("{:x}", window.0 as usize));
        let mut process = 0; // 窗口进程身份。
        GetWindowThreadProcessId(window, Some(&mut process));
        if process == 0
        {
            return next;
        }
        next.process_id = Some(process);
        let mut class = [0_u16; 256]; // 系统桌面与瞬态 Shell 类。
        let length = GetClassNameW(window, &mut class); // 有效类名长度。
        if length == 0
        {
            return next;
        }
        let class = String::from_utf16_lossy(&class[..length as usize]); // 排除已有系统窗口规则。
        let excluded = process == std::process::id() || matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "#32768" | "XamlExplorerHostIslandWindow" | "Windows.UI.Core.CoreWindow") || !IsWindowVisible(window).as_bool() || IsIconic(window).as_bool(); // 不把自身和桌面算作外部应用。
        let mut cloaked = 0_u32; // DWM 隐藏事实。
        if !excluded && DwmGetWindowAttribute(window, DWMWA_CLOAKED, (&mut cloaked as *mut u32).cast(), size_of::<u32>() as u32).is_err()
        {
            return next;
        }
        next.external_focus = Some(!excluded && cloaked == 0);
        next.maximized = Some(next.external_focus == Some(true) && IsZoomed(window).as_bool());
        if next.external_focus == Some(false) || GetWindowLongPtrW(window, GWL_EXSTYLE) as u32 & (WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0) != 0
        {
            next.coverage = Some(Vec::new());
        }
        else
        {
            let mut bounds = RECT::default(); // 不含阴影的物理边界。
            if DwmGetWindowAttribute(window, DWMWA_EXTENDED_FRAME_BOUNDS, (&mut bounds as *mut RECT).cast(), size_of::<RECT>() as u32).is_ok()
            {
                next.coverage = super::enumerate_desktop_monitors().ok().map(|monitors| monitors.iter().map(|monitor| Coverage { monitor_id: monitor.id.clone(), work: covers(bounds, monitor.work_x, monitor.work_y, monitor.work_width, monitor.work_height), screen: covers(bounds, monitor.x, monitor.y, monitor.width, monitor.height) }).collect());
            }
        }
        if GetForegroundWindow() != window
        {
            return ForegroundSnapshot { locked: next.locked, ..Default::default() };
        }
        next.fullscreen = combine_fullscreen(next.coverage.as_ref(), next.exclusive_fullscreen);
    }
    next
}

/// 宽整数支持负坐标与跨屏窗口。
fn covers(bounds: RECT, x: i32, y: i32, width: u32, height: u32) -> bool { bounds.left <= x && bounds.top <= y && i64::from(bounds.right) >= i64::from(x) + i64::from(width) && i64::from(bounds.bottom) >= i64::from(y) + i64::from(height) }

#[cfg(test)]
#[path = "refresh_policy.test.rs"]
mod tests;
