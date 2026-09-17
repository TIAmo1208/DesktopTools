use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, Ordering};

use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, REG_NOTIFY_CHANGE_LAST_SET, RegCloseKey, RegNotifyChangeKeyValue, RegOpenKeyExW};
use windows::Win32::System::Threading::{CreateEventW, INFINITE, SetEvent};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, EVENT_OBJECT_PARENTCHANGE, GetClassNameW, MSG, MsgWaitForMultipleObjects, OBJID_WINDOW, PM_REMOVE, PeekMessageW, QS_ALLINPUT, TranslateMessage, WINEVENT_OUTOFCONTEXT, WM_QUIT};
use windows::core::HSTRING;

const ICON_SETTING_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced"; // “显示桌面图标”所在的 Explorer 用户设置键。
const FALLBACK_INTERVAL_MS: u32 = 30_000; // 仅缺失可靠事件源的项目使用低频故障兜底。
static ENVIRONMENT_EVENT: AtomicIsize = AtomicIsize::new(0); // WinEvent 回调唤醒当前唯一宿主环境协调线程。

/// 从正式宿主窗口过程转发显示或 DPI 变化。
pub(crate) fn signal_environment()
{
    let raw = ENVIRONMENT_EVENT.load(Ordering::Acquire);
    if raw != 0
    {
        let _ = unsafe { SetEvent(HANDLE(raw as *mut c_void)) };
    }
}

/// 宿主环境通知的具体来源。
pub(crate) enum EnvironmentNotification
{
    Stop,
    Environment,
    Visibility,
    Fallback
    {
        environment: bool, // Explorer 窗口 Hook 缺失时才检查。
        visibility: bool,  // 注册表通知缺失时才检查。
    },
}

/// 持有显示、Explorer 窗口和桌面图标设置通知资源。
pub(crate) struct DesktopEnvironmentWatch
{
    stop_event: HANDLE,        // 正常退出时唤醒无限等待。
    environment_event: HANDLE, // 显示或 Explorer 窗口变化事件。
    visibility_event: HANDLE,  // 桌面图标设置注册表变化事件。
    visibility_key: HKEY,      // Explorer Advanced 只读通知键。
    shell_hook: HWINEVENTHOOK, // Explorer 桌面窗口生命周期钩子。
    shell_fallback: bool,      // Hook 注册失败时仅低频检查 Explorer。
    visibility_fallback: bool, // 注册表通知失败时仅低频检查设置。
}

/// Explorer 桌面相关窗口创建、销毁或迁移时唤醒环境协调。
unsafe extern "system" fn handle_shell_window_event(_: HWINEVENTHOOK, event: u32, window: HWND, object: i32, _: i32, _: u32, _: u32)
{
    if object != OBJID_WINDOW.0 || !matches!(event, EVENT_OBJECT_CREATE | EVENT_OBJECT_DESTROY | EVENT_OBJECT_PARENTCHANGE)
    {
        return;
    }
    let mut class = [0_u16; 64]; // Explorer 桌面层窗口类名。
    let length = unsafe { GetClassNameW(window, &mut class) };
    if length == 0
    {
        return;
    }
    let class = String::from_utf16_lossy(&class[..length as usize]);
    if !matches!(class.as_str(), "Progman" | "WorkerW" | "SHELLDLL_DefView" | "SysListView32")
    {
        return;
    }
    signal_environment();
}

/// 等待停止、环境、设置或当前线程的 Windows 消息。
fn wait_for_notification(stop: HANDLE, environment: HANDLE, visibility: HANDLE, timeout: u32) -> Result<Option<EnvironmentNotification>, String>
{
    let handles = [stop, environment, visibility]; // 固定索引与通知类型一一对应。
    let result = unsafe { MsgWaitForMultipleObjects(Some(&handles), false, timeout, QS_ALLINPUT) };
    if result == WAIT_OBJECT_0
    {
        return Ok(Some(EnvironmentNotification::Stop));
    }
    if result.0 == WAIT_OBJECT_0.0 + 1
    {
        return Ok(Some(EnvironmentNotification::Environment));
    }
    if result.0 == WAIT_OBJECT_0.0 + 2
    {
        return Ok(Some(EnvironmentNotification::Visibility));
    }
    if result.0 == WAIT_OBJECT_0.0 + 3
    {
        return Ok(None);
    }
    if result == WAIT_TIMEOUT
    {
        return Ok(Some(EnvironmentNotification::Fallback { environment: false, visibility: false }));
    }
    Err(format!("等待宿主环境通知失败，状态：{}", result.0))
}

/// 处理消息队列并返回下一项具体环境通知。
impl DesktopEnvironmentWatch
{
    /// 建立可用事件源；单项注册失败时仅为该项启用低频故障兜底。
    pub(crate) fn new(stop_event: HANDLE) -> Result<Self, String>
    {
        let environment_event = unsafe { CreateEventW(None, false, false, None) }.map_err(|error| format!("创建宿主环境通知事件失败：{error}"))?;
        let visibility_event = match unsafe { CreateEventW(None, false, false, None) }
        {
            Ok(event) => event,
            Err(error) =>
            {
                let _ = unsafe { CloseHandle(environment_event) };
                return Err(format!("创建桌面图标设置通知事件失败：{error}"));
            }
        };
        ENVIRONMENT_EVENT.store(environment_event.0 as isize, Ordering::Release);
        let shell_hook = unsafe { SetWinEventHook(EVENT_OBJECT_CREATE, EVENT_OBJECT_PARENTCHANGE, None, Some(handle_shell_window_event), 0, 0, WINEVENT_OUTOFCONTEXT) };
        let shell_fallback = shell_hook.0.is_null(); // 注册失败只退回 Explorer 低频检查。
        if shell_fallback
        {
            crate::log_warn!("宿主：Explorer 窗口事件注册失败，仅为该项启用 30 秒故障兜底");
        }
        let mut visibility_key = HKEY::default(); // Explorer 用户设置通知键。
        let mut visibility_fallback = false; // 注册表通知是否需要低频替代。
        if let Err(error) = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(ICON_SETTING_KEY), None, KEY_NOTIFY, &mut visibility_key) }.ok()
        {
            visibility_fallback = true;
            crate::log_warn!("宿主：桌面图标设置通知键打开失败，仅为该项启用 30 秒故障兜底，错误：{error}");
        }
        let mut watch = Self { stop_event, environment_event, visibility_event, visibility_key, shell_hook, shell_fallback, visibility_fallback };
        if !watch.visibility_fallback
            && let Err(error) = watch.rearm_visibility()
        {
            watch.visibility_fallback = true;
            crate::log_warn!("宿主：桌面图标设置通知注册失败，仅为该项启用 30 秒故障兜底，错误：{error}");
        }
        Ok(watch)
    }

    /// 无限等待下一项真实通知，Windows 消息在返回环境事件前先完整分发。
    pub(crate) fn wait(&mut self) -> Result<EnvironmentNotification, String>
    {
        loop
        {
            let timeout = if self.shell_fallback || self.visibility_fallback { FALLBACK_INTERVAL_MS } else { INFINITE };
            if let Some(notification) = wait_for_notification(self.stop_event, self.environment_event, self.visibility_event, timeout)?
            {
                if matches!(notification, EnvironmentNotification::Visibility)
                    && let Err(error) = self.rearm_visibility()
                {
                    self.visibility_fallback = true;
                    crate::log_warn!("宿主：桌面图标设置通知重新注册失败，仅为该项启用 30 秒故障兜底，错误：{error}");
                }
                if matches!(notification, EnvironmentNotification::Fallback { .. })
                {
                    return Ok(EnvironmentNotification::Fallback { environment: self.shell_fallback, visibility: self.visibility_fallback });
                }
                return Ok(notification);
            }
            let mut message = MSG::default(); // 本轮全部显示、DPI 和 WinEvent 回调消息。
            while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool()
            {
                if message.message == WM_QUIT
                {
                    return Ok(EnvironmentNotification::Stop);
                }
                unsafe {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        }
    }

    /// 注册一次异步设置变化通知，收到事件后必须重新注册。
    fn rearm_visibility(&mut self) -> Result<(), String> { unsafe { RegNotifyChangeKeyValue(self.visibility_key, false, REG_NOTIFY_CHANGE_LAST_SET, Some(self.visibility_event), true) }.ok().map_err(|error| format!("注册桌面图标设置变化通知失败：{error}")) }
}

/// 释放当前环境通知线程独占的全部 Windows 资源。
impl Drop for DesktopEnvironmentWatch
{
    fn drop(&mut self)
    {
        ENVIRONMENT_EVENT.store(0, Ordering::Release);
        if !self.shell_hook.0.is_null()
        {
            let _ = unsafe { UnhookWinEvent(self.shell_hook) };
        }
        if !self.visibility_key.is_invalid()
        {
            let _ = unsafe { RegCloseKey(self.visibility_key) };
        }
        for event in [self.visibility_event, self.environment_event, self.stop_event]
        {
            let _ = unsafe { CloseHandle(event) };
        }
    }
}

#[cfg(test)]
#[path = "environment_watch.test.rs"]
mod tests;
