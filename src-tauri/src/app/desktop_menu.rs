//! 注册 Explorer 桌面菜单，并将静态动词转交给应用主线程。
use tauri::AppHandle;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW};
use windows::Win32::UI::WindowsAndMessaging::{CREATESTRUCTW, CreateWindowExW, DefWindowProcW, DestroyWindow, FindWindowExW, GWLP_USERDATA, GetWindowLongPtrW, HWND_MESSAGE, RegisterClassW, SMTO_ABORTIFHUNG, SendMessageTimeoutW, SetWindowLongPtrW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_NCCREATE, WNDCLASSW};
use windows::core::{HSTRING, Owned, PCWSTR, w};

pub const REGISTRY_PATH: &str = r"Software\Classes\DesktopBackground\shell\DesktopTools"; // 只写当前用户的产品专属键。
const RECEIVER: PCWSTR = w!("DesktopToolsDesktopMenu"); // 当前会话的命令接收窗口。
const MESSAGE: u32 = WM_APP + 0x64; // 仅传递固定数字动词，不接收路径或脚本。
const DISPATCH_MESSAGE: u32 = MESSAGE + 1; // 脱离跨进程 SendMessage 的同步调用栈后再访问 COM。
static OWN_RECEIVER: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0); // 退出时只销毁本进程创建的窗口。
const ACTIONS: [(&str, &str, u32); 5] = [("create-fence", "新建围栏", 0x7804), ("create-folder-portal", "新建文件夹门户", 0x7808), ("edit-layout", "编辑布局", 0x7805), ("refresh", "刷新桌面项目", 0x7806), ("settings", "打开设置", 0x7807)]; // 注册表、启动参数与消息校验共用的动作清单。

/// 区分已经处理的命令与需要启动应用的初始动作。
pub enum Startup
{
    Exit,
    Run(Option<u32>, Owned<windows::Win32::Foundation::HANDLE>),
}

/// 注册一个 UTF-16 字符串值，句柄在写入结束时自动关闭。
fn write_value(path: &str, name: &str, value: &str) -> Result<(), String>
{
    let mut key = HKEY::default(); // 新建或打开当前产品子键。
    unsafe { RegCreateKeyExW(HKEY_CURRENT_USER, &HSTRING::from(path), None, PCWSTR::null(), REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut key, None) }.ok().map_err(|error| error.to_string())?;
    let key = unsafe { Owned::new(key) }; // 所有失败分支均关闭句柄。
    let bytes: Vec<u8> = value.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect(); // 包含字符串终止符。
    unsafe { RegSetValueExW(*key, &HSTRING::from(name), None, REG_SZ, Some(&bytes)) }.ok().map_err(|error| error.to_string())
}

/// 生成固定动作的带引号启动命令，支持安装路径含空格与中文。
fn command_line(executable: &std::path::Path, action: &str) -> Result<String, String>
{
    let executable = executable.to_str().ok_or("程序路径不是有效 Unicode")?; // 注册表命令路径。
    if executable.contains('"') || !ACTIONS.iter().any(|entry| entry.0 == action)
    {
        return Err("桌面菜单启动参数无效".into());
    }
    Ok(format!("\"{executable}\" --desktop-menu {action}"))
}

/// 将产品菜单登记到 Explorer 的桌面背景，不获取或修改任何 HMENU。
pub fn register() -> Result<(), String>
{
    let executable = std::env::current_exe().map_err(|error| error.to_string())?; // 当前安装或开发程序。
    register_actions(REGISTRY_PATH, &executable)?;
    crate::log_info!("桌面菜单：已注册当前用户菜单，路径=HKCU\\{REGISTRY_PATH}");
    Ok(())
}

/// 用当前动作全集替换产品菜单子树，供正式注册和隔离注册表验证共用。
fn register_actions(registry_path: &str, executable: &std::path::Path) -> Result<(), String>
{
    let child_path = format!(r"{registry_path}\shell"); // 产品菜单动作的唯一子树。
    let result = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(&child_path)) }; // 重注册前清除旧序号，避免菜单动作重复。
    if result.0 != 2
    {
        result.ok().map_err(|error| error.to_string())?;
    }
    write_value(registry_path, "MUIVerb", "Desktop Tools")?;
    write_value(registry_path, "SubCommands", "")?;
    write_value(registry_path, "Icon", &format!("\"{}\",0", executable.display()))?;
    for (index, (action, label, _)) in ACTIONS.iter().enumerate()
    {
        let key = format!(r"{registry_path}\shell\{index:02}-{action}"); // 序号保证菜单顺序稳定。
        write_value(&key, "MUIVerb", label)?;
        write_value(&format!(r"{key}\command"), "", &command_line(executable, action)?)?;
    }
    Ok(())
}

/// 在启动 Shell 与 guardian 之前处理安装、移除和已有实例命令。
pub fn prepare_startup() -> Result<Startup, String>
{
    let arguments: Vec<String> = std::env::args().skip(1).collect(); // 固定命令入口。
    let action = match arguments.as_slice()
    {
        [] => None,
        [flag] if flag == "--register-desktop-menu" =>
        {
            register()?;
            return Ok(Startup::Exit);
        }
        [flag] if flag == "--unregister-desktop-menu" =>
        {
            let result = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(REGISTRY_PATH)) }; // 只移除产品专属键。
            if result.0 != 2
            {
                result.ok().map_err(|error| error.to_string())?;
            }
            return Ok(Startup::Exit);
        }
        [flag, action] if flag == "--desktop-menu" => Some(ACTIONS.iter().find(|entry| entry.0 == action).ok_or("未知桌面菜单动作")?.2),
        _ => return Err("未知启动参数".into()),
    }; // 没有运行实例时保留的初始动作。
    let mutex = unsafe { windows::Win32::System::Threading::CreateMutexW(None, false, w!("Local\\DesktopTools.DesktopMenu.Instance")) }.map_err(|error| error.to_string())?; // 防止启动过程中连续点击产生两个宿主。
    let exists = unsafe { windows::Win32::Foundation::GetLastError() } == windows::Win32::Foundation::ERROR_ALREADY_EXISTS; // 紧接创建读取结果。
    let mutex = unsafe { Owned::new(mutex) }; // 主实例持有到退出，辅助进程返回时释放。
    if !exists
    {
        return Ok(Startup::Run(action, mutex));
    }
    let started = std::time::Instant::now(); // 等待主实例完成启动的上限。
    let receiver = loop
    {
        let hwnd = unsafe { FindWindowExW(Some(HWND_MESSAGE), None, RECEIVER, PCWSTR::null()) }.unwrap_or_default(); // 当前会话的接收窗口。
        if !hwnd.is_invalid()
        {
            break hwnd;
        }
        if started.elapsed().as_secs() >= 10
        {
            return Err("主程序正在启动，但桌面菜单接收窗口尚未就绪".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let mut acknowledged = 0; // 主实例确认接收后辅助进程才成功退出。
    let sent = unsafe { SendMessageTimeoutW(receiver, MESSAGE, WPARAM(action.unwrap_or(0x7807) as usize), LPARAM(0), SMTO_ABORTIFHUNG, 2000, Some(&mut acknowledged)) };
    if sent.0 == 0 || acknowledged != 1
    {
        return Err("运行中的程序未接收桌面菜单命令".into());
    }
    Ok(Startup::Exit)
}

/// 将经过校验的数字动词排入主线程，不在窗口过程内执行文件操作。
unsafe extern "system" fn receiver_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT
{
    if message == WM_NCCREATE
    {
        let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) }; // 创建方持有的应用句柄。
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize) };
    }
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut AppHandle; // 生命周期与接收窗口一致。
    if (message == MESSAGE || message == DISPATCH_MESSAGE) && !pointer.is_null() && ACTIONS.iter().any(|entry| entry.2 as usize == wparam.0)
    {
        if message == MESSAGE
        {
            return LRESULT(isize::from(unsafe { windows::Win32::UI::WindowsAndMessaging::PostMessageW(Some(hwnd), DISPATCH_MESSAGE, wparam, LPARAM(0)) }.is_ok()));
        }
        let app = unsafe { &*pointer }; // 当前已回到主线程消息队列，不处于跨进程同步回调。
        if let Err(error) = crate::commands::execute_registered_desktop_menu(app, wparam.0 as u32)
        {
            crate::log_error!("桌面菜单：命令执行失败，错误：{error}");
        }
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

/// 在主线程创建消息接收窗口，随后更新注册表中的程序路径。
pub fn install(app: &AppHandle, initial: Option<u32>) -> Result<(), String>
{
    let module = unsafe { GetModuleHandleW(None) }.map_err(|error| error.to_string())?; // 当前程序模块。
    let class = WNDCLASSW { lpfnWndProc: Some(receiver_proc), hInstance: HINSTANCE(module.0), lpszClassName: RECEIVER, ..Default::default() }; // 无可见界面的命令窗口。
    if unsafe { RegisterClassW(&class) } == 0
    {
        return Err("注册桌面菜单接收窗口失败".into());
    }
    let pointer = Box::into_raw(Box::new(app.clone())); // 窗口销毁时释放。
    let created = unsafe { CreateWindowExW(WINDOW_EX_STYLE::default(), RECEIVER, w!(""), WINDOW_STYLE::default(), 0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(HINSTANCE(module.0)), Some(pointer.cast())) };
    let hwnd = match created
    {
        Ok(hwnd) => hwnd,
        Err(error) =>
        {
            drop(unsafe { Box::from_raw(pointer) });
            return Err(error.to_string());
        }
    }; // 创建失败不遗留应用引用。
    OWN_RECEIVER.store(hwnd.0 as isize, std::sync::atomic::Ordering::Relaxed);
    if let Err(error) = register()
    {
        shutdown();
        return Err(error);
    }
    if let Some(action) = initial
    {
        unsafe { windows::Win32::UI::WindowsAndMessaging::PostMessageW(Some(hwnd), MESSAGE, WPARAM(action as usize), LPARAM(0)) }.map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// 主线程退出时销毁接收窗口并释放应用引用，注册表保留供下次启动。
pub fn shutdown()
{
    let hwnd = HWND(OWN_RECEIVER.swap(0, std::sync::atomic::Ordering::Relaxed) as *mut std::ffi::c_void); // 当前实例的接收窗口。
    if !hwnd.is_invalid()
    {
        let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut AppHandle; // 创建时分配的引用。
        if unsafe { DestroyWindow(hwnd) }.is_ok() && !pointer.is_null()
        {
            drop(unsafe { Box::from_raw(pointer) });
        }
    }
}

#[cfg(test)]
#[path = "desktop_menu.test.rs"]
mod tests;
