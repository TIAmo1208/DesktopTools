//! 把指定进程的可见主窗口带到前台。
//!
//! `dsh:` 协议只负责把链接交给 DSH，系统不会把已存在的窗口抬到前台；这里按进程名找主窗口，
//! 先恢复最小化，再尝试 `SetForegroundWindow`，被前台锁拒绝时用 `AttachThreadInput` 重试一次。

use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GW_OWNER, GetForegroundWindow, GetWindow, GetWindowTextLengthW, GetWindowThreadProcessId, HWND_TOP, IsIconic, IsWindowVisible, SW_RESTORE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SetForegroundWindow, SetWindowPos, ShowWindow};
use windows::core::{BOOL, PWSTR};

/// 枚举状态：目标进程名与找到的第一个符合条件的主窗口。
struct WindowSearch
{
    /// 目标进程的可执行文件名，比较不区分 ASCII 大小写。
    process_name: String,
    /// 已找到的窗口；找到后不再覆盖。
    window: Option<HWND>,
}

/// 把进程名为 `process_name` 的可见主窗口带到前台。
///
/// 找不到窗口、读取进程名失败或系统仍拒绝前台时返回可读原因；调用方按「协议打开或抬窗任一成功」判定。
pub(crate) fn activate_window_by_process_name(process_name: &str) -> Result<(), String>
{
    // 协议可能刚刚拉起 DSH：最多试三轮（约 300 毫秒），仍失败再报告。
    for attempt in 0..3
    {
        match activate_window_once(process_name)
        {
            Ok(()) => return Ok(()),
            Err(reason) if attempt == 2 => return Err(reason),
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(150)),
        }
    }
    Err("未找到进程的可见主窗口".to_string())
}

/// 单次尝试：枚举一次并激活命中的主窗口。
fn activate_window_once(process_name: &str) -> Result<(), String>
{
    let mut search = WindowSearch { process_name: process_name.to_string(), window: None };
    unsafe {
        let _ = EnumWindows(Some(find_main_window), LPARAM((&mut search as *mut WindowSearch) as isize));
    }
    let window = search.window.ok_or_else(|| format!("未找到进程 {process_name} 的可见主窗口"))?;
    unsafe {
        if IsIconic(window).as_bool()
        {
            let _ = ShowWindow(window, SW_RESTORE); // 最小化窗口先恢复，再要求前台。
        }
        let _ = SetWindowPos(window, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
        if SetForegroundWindow(window).as_bool()
        {
            return Ok(());
        }
        // 前台锁可能拒绝跨线程的 SetForegroundWindow；临时附加到当前前台线程再试一次。
        let foreground_thread = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let current_thread = GetCurrentThreadId();
        let attached = foreground_thread != 0 && foreground_thread != current_thread && AttachThreadInput(current_thread, foreground_thread, true).as_bool();
        let raised = SetForegroundWindow(window).as_bool();
        if attached
        {
            let _ = AttachThreadInput(current_thread, foreground_thread, false);
        }
        if raised || GetForegroundWindow() == window
        {
            Ok(())
        }
        else
        {
            Err("系统拒绝把目标窗口带到前台".to_string())
        }
    }
}

/// 只接受可见、无主且带标题的顶层窗口；进程名按可执行文件名比较。
unsafe extern "system" fn find_main_window(window: HWND, parameter: LPARAM) -> BOOL
{
    let search = unsafe { &mut *(parameter.0 as *mut WindowSearch) };
    let owner = unsafe { GetWindow(window, GW_OWNER) }.unwrap_or_default(); // 无主窗口时返回默认（无效）句柄。
    if search.window.is_some() || !unsafe { IsWindowVisible(window) }.as_bool() || unsafe { GetWindowTextLengthW(window) } <= 0 || !owner.is_invalid()
    {
        return BOOL(1); // 跳过隐藏、无标题与有主窗口，继续枚举。
    }
    let mut process = 0u32;
    unsafe {
        GetWindowThreadProcessId(window, Some(&mut process));
    }
    if process == 0 || !process_matches(process, &search.process_name)
    {
        return BOOL(1);
    }
    search.window = Some(window);
    BOOL(0) // 已找到，停止枚举。
}

/// 进程可执行文件名是否与目标一致；读不到进程名时按不匹配处理。
fn process_matches(process: u32, process_name: &str) -> bool
{
    let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process) })
    else
    {
        return false;
    };
    let mut buffer = [0u16; 512]; // 可执行文件完整路径足够放下的缓冲。
    let mut size = buffer.len() as u32;
    let queried = unsafe { QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut size) };
    let _ = unsafe { CloseHandle(handle) };
    if queried.is_err()
    {
        return false;
    }
    let path = String::from_utf16_lossy(&buffer[..size as usize]);
    path.rsplit('\\').next().is_some_and(|name| name.eq_ignore_ascii_case(process_name))
}
