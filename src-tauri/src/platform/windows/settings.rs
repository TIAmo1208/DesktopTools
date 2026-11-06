use std::{os::windows::ffi::OsStrExt, path::Path};

/// 使用系统文件夹选择弹窗，不枚举或上传目录内文件。
pub fn choose_config_directory(owner: isize) -> Result<Option<String>, String> { choose_directory(owner, "选择配置目录") }

/// 在调用方 UI 线程显示带有明确用途标题的系统目录选择器。
pub fn choose_directory(owner: isize, title: &str) -> Result<Option<String>, String>
{
    use windows::Win32::{
        Foundation::{ERROR_CANCELLED, HWND},
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree},
        UI::Shell::{FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, SIGDN_FILESYSPATH},
    };
    // Tauri 的窗口主线程已初始化 COM，弹窗和接口都限定在此线程。
    let result = (|| -> windows::core::Result<Option<String>> {
        let dialog: IFileOpenDialog = unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)? }; // 系统原生文件夹选择器。
        unsafe {
            dialog.SetOptions(dialog.GetOptions()? | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM)?;
            dialog.SetTitle(&windows::core::HSTRING::from(title))?;
            if let Err(error) = dialog.Show(Some(HWND(owner as *mut std::ffi::c_void)))
            {
                if error.code() == windows::core::HRESULT::from_win32(ERROR_CANCELLED.0)
                {
                    return Ok(None);
                }
                return Err(error);
            }
            let path = dialog.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?; // Shell 分配的目录文本。
            let value = path.to_string(); // 释放系统内存前转换 UTF-16。
            CoTaskMemFree(Some(path.0.cast()));
            Ok(Some(value?))
        }
    })(); // 将原生错误统一转换为设置页可见说明。
    result.map_err(|error| format!("选择目录失败：{error}"))
}

/// 一次选择多个插件文件夹。取消时返回空列表。
pub fn choose_directories(owner: isize, title: &str) -> Result<Vec<String>, String> { choose_paths(owner, title, true, None).map_err(|error| format!("选择目录失败：{error}")) }

/// 一次选择多个 zip。过滤只是提示，调用方仍要校验内容。取消时返回空列表。
pub fn choose_zip_archives(owner: isize) -> Result<Vec<String>, String>
{
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    use windows::core::w;
    let filter = [COMDLG_FILTERSPEC { pszName: w!("插件压缩包"), pszSpec: w!("*.zip") }]; // 只提示 zip，不选目录。
    choose_paths(owner, "选择插件压缩包", false, Some(&filter)).map_err(|error| format!("选择插件压缩包失败：{error}"))
}

fn choose_paths(owner: isize, title: &str, folders: bool, filter: Option<&[windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC]>) -> windows::core::Result<Vec<String>>
{
    use windows::Win32::{
        Foundation::{ERROR_CANCELLED, HWND},
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree},
        UI::Shell::{FOS_ALLOWMULTISELECT, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, SIGDN_FILESYSPATH},
    };
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?; // 系统原生选择器，允许多选。
        let mut options = dialog.GetOptions()? | FOS_ALLOWMULTISELECT | FOS_FORCEFILESYSTEM;
        options = if folders { options | FOS_PICKFOLDERS } else { options | FOS_FILEMUSTEXIST };
        dialog.SetOptions(options)?;
        if let Some(filter) = filter
        {
            dialog.SetFileTypes(filter)?;
        }
        dialog.SetTitle(&windows::core::HSTRING::from(title))?;
        if let Err(error) = dialog.Show(Some(HWND(owner as *mut std::ffi::c_void)))
        {
            if error.code() == windows::core::HRESULT::from_win32(ERROR_CANCELLED.0)
            {
                return Ok(Vec::new());
            }
            return Err(error);
        }
        let items = dialog.GetResults()?; // 多选结果。
        let mut paths = Vec::new(); // 用户选中的路径。
        for index in 0..items.GetCount()?
        {
            let name = items.GetItemAt(index)?.GetDisplayName(SIGDN_FILESYSPATH)?;
            let value = name.to_string();
            CoTaskMemFree(Some(name.0.cast()));
            paths.push(value?);
        }
        Ok(paths)
    }
}

/// 将真实路径交给系统默认程序打开。
pub fn open_path(path: &Path) -> Result<(), String>
{
    use windows::Win32::{
        Foundation::HWND,
        UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
    };
    let value = windows::core::HSTRING::from(path.as_os_str().to_string_lossy().as_ref());
    let result = unsafe { ShellExecuteW(Some(HWND::default()), windows::core::PCWSTR::null(), windows::core::PCWSTR(value.as_ptr()), windows::core::PCWSTR::null(), windows::core::PCWSTR::null(), SW_SHOWNORMAL) };
    if (result.0 as isize) <= 32
    {
        let code = result.0 as isize; // ShellExecute 返回值不是 GetLastError。
        let reason = match code
        {
            0 | 8 => "系统内存或资源不足",
            2 => "文件不存在或已被删除",
            3 => "路径不存在或无法访问",
            5 => "访问被拒绝",
            11 => "文件格式无效",
            26 => "文件被占用",
            27 | 31 => "未找到可用的默认关联程序",
            28 => "关联程序响应超时",
            29 | 30 => "无法与关联程序通信",
            32 => "关联程序所需的动态库不存在",
            _ => "系统无法打开此目标",
        }; // 使用 Shell 文档定义的错误语义，不把返回码误作 Win32 错误码。
        return Err(format!("打开路径失败：{reason}，路径={}，ShellExecute 返回码={code}", path.display()));
    }
    Ok(())
}
use windows::{
    Foundation::TypedEventHandler,
    UI::ViewManagement::{UIColorType, UISettings},
    Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND},
        System::Registry::{HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_BINARY, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW},
        UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
    },
    core::{PCWSTR, w},
};

/// 持有 Windows 强调色订阅，随应用退出解除系统回调。
pub struct WindowsAccentMonitor
{
    settings: UISettings,                 // Windows 用户界面颜色提供者。
    token: std::sync::Mutex<Option<i64>>, // 可显式解除的订阅凭据。
}

impl WindowsAccentMonitor
{
    /// 先建立系统颜色监听，再由各窗口读取当前颜色，避免启动期间漏更新。
    pub fn install(app: tauri::AppHandle) -> windows::core::Result<Self>
    {
        use tauri::Emitter;
        let settings = UISettings::new()?; // 当前用户的系统颜色来源。
        let token = settings.ColorValuesChanged(&TypedEventHandler::<UISettings, windows::core::IInspectable>::new(move |sender, _| {
            if let Some(sender) = sender.as_ref()
            {
                match sender.GetColorValue(UIColorType::Accent)
                {
                    Ok(color) =>
                    {
                        let accent = format!("#{:02x}{:02x}{:02x}", color.R, color.G, color.B); // 丢弃透明度的 CSS 强调色。
                        crate::log_debug!("设置：Windows 强调色已更新，颜色：{accent}");
                        if let Err(error) = app.emit("windows-accent-color-changed", accent)
                        {
                            crate::log_warn!("设置：Windows 强调色事件发布失败，错误：{error}");
                        }
                    }
                    Err(error) => crate::log_error!("设置：读取 Windows 强调色失败，错误：{error}"),
                }
            }
            Ok(())
        }))?; // 应用生命周期内唯一的系统颜色订阅。
        crate::log_info!("设置：Windows 强调色监听已启动");
        Ok(Self { settings, token: std::sync::Mutex::new(Some(token)) })
    }

    /// 退出时解除回调，保留托管状态以免已借出的引用失效。
    pub fn shutdown(&self)
    {
        let mut token = self.token.lock().unwrap_or_else(|error| error.into_inner()); // 串行化重复退出清理。
        if let Some(value) = *token
        {
            match self.settings.RemoveColorValuesChanged(value)
            {
                Ok(()) => *token = None,
                Err(error) => crate::log_warn!("设置：解除 Windows 强调色监听失败，错误：{error}"),
            }
        }
    }

    /// 返回系统当前强调色，不覆盖已保存的自定义颜色。
    pub fn color(&self) -> Result<String, String>
    {
        let color = self.settings.GetColorValue(UIColorType::Accent).map_err(|error| format!("读取 Windows 强调色失败：{error}"))?; // Windows 当前强调色。
        Ok(format!("#{:02x}{:02x}{:02x}", color.R, color.G, color.B))
    }
}

impl Drop for WindowsAccentMonitor
{
    /// 释放 Windows 事件处理器及其持有的应用句柄。
    fn drop(&mut self) { self.shutdown(); }
}

/// 读取当前用户的应用启动命令；缺失条目表示未启用。
fn autostart_command() -> Result<Option<String>, String>
{
    let mut buffer = vec![0_u16; 32768]; // Windows 注册命令的 UTF-16 缓冲区。
    let mut bytes = (buffer.len() * 2) as u32; // 读取容量与返回的实际字节数。
    let status = unsafe { RegGetValueW(HKEY_CURRENT_USER, w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"), w!("DesktopTools"), RRF_RT_REG_SZ, None, Some(buffer.as_mut_ptr().cast()), Some(&mut bytes)) }; // 原生注册表读取结果。
    if status == ERROR_FILE_NOT_FOUND || status == ERROR_PATH_NOT_FOUND
    {
        return Ok(None);
    }
    status.ok().map_err(|error| format!("读取开机启动失败：{error}"))?;
    let length = (bytes as usize / 2).min(buffer.len()); // 注册表实际返回的 UTF-16 长度。
    Ok(Some(String::from_utf16_lossy(&buffer[..length]).trim_end_matches('\0').to_string()))
}

/// 只读识别当前 Windows 启动审批状态，不覆写用户在系统设置中的禁用决定。
fn autostart_allowed() -> Result<bool, String>
{
    let mut data = [0_u8; 12]; // Windows 启动应用审批记录的状态和时间数据。
    let mut bytes = data.len() as u32; // 当前审批记录的读取容量。
    let status = unsafe { RegGetValueW(HKEY_CURRENT_USER, w!("Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run"), w!("DesktopTools"), RRF_RT_REG_BINARY, None, Some(data.as_mut_ptr().cast()), Some(&mut bytes)) }; // 用户在 Windows 启动应用中设置的审批状态。
    if status == ERROR_FILE_NOT_FOUND || status == ERROR_PATH_NOT_FOUND
    {
        return Ok(true);
    }
    status.ok().map_err(|error| format!("读取 Windows 启动应用状态失败：{error}"))?;
    match (bytes, data[0])
    {
        (12, 2 | 6) => Ok(true),
        (12, 3 | 7) => Ok(false),
        _ => Err("Windows 启动应用状态无法识别，请检查系统启动设置".to_string()),
    }
}

/// 根据实际注册命令与 Windows 启动审批判断是否启用。
pub fn autostart_enabled() -> Result<bool, String>
{
    let executable = std::env::current_exe().map_err(|error| format!("读取程序路径失败：{error}"))?; // 当前正在运行的应用路径。
    Ok(autostart_command()?.is_some_and(|command| command.eq_ignore_ascii_case(&format!("\"{}\"", executable.display()))) && autostart_allowed()?)
}

/// 仅更新本应用的当前用户启动条目，不操作其他应用或请求管理员权限。
pub fn set_autostart(enabled: bool) -> Result<bool, String>
{
    let status = if enabled
    {
        let executable = std::env::current_exe().map_err(|error| format!("读取程序路径失败：{error}"))?; // 注册命令使用的当前应用路径。
        let command: Vec<u16> = format!("\"{}\"", executable.display()).encode_utf16().chain(Some(0)).collect(); // 带引号的完整启动命令。
        unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"), w!("DesktopTools"), REG_SZ.0, Some(command.as_ptr().cast()), (command.len() * 2) as u32) }
    }
    else
    {
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"), w!("DesktopTools")) }
    }; // 当前应用启动条目的写入或删除结果。
    if status != ERROR_FILE_NOT_FOUND && status != ERROR_PATH_NOT_FOUND
    {
        status.ok().map_err(|error| format!("更新开机启动失败：{error}"))?;
    }
    let actual = autostart_enabled()?; // 写入后重新读取的真实注册状态。
    if actual != enabled
    {
        if enabled && !autostart_allowed()?
        {
            return Err("已登记启动项，但 Windows 已禁用 Desktop Tools，请在系统设置的“应用 > 启动”中启用".to_string());
        }
        return Err("开机启动状态与请求不一致".to_string());
    }
    crate::log_info!("设置：开机启动已更新，启用：{actual}");
    Ok(actual)
}

/// 使用 Shell 打开应用确定的日志目录，不接受前端任意路径。
pub fn open_log_directory(path: &Path) -> Result<(), String>
{
    std::fs::create_dir_all(path).map_err(|error| format!("创建日志目录失败：{error}"))?;
    let directory: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect(); // 原生 Shell 使用的日志绝对路径。
    let result = unsafe { ShellExecuteW(None, w!("open"), PCWSTR(directory.as_ptr()), None, None, SW_SHOWNORMAL) }; // Shell 返回的启动结果。
    if result.0 as isize <= 32
    {
        return Err(format!("打开日志目录失败，ShellExecuteW：{}", result.0 as isize));
    }
    Ok(())
}
