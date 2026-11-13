use std::{mem::size_of, path::Path};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HANDLE},
        Security::{GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TokenIntegrityLevel},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
        UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MessageBoxW},
    },
    core::PCWSTR,
};

/// 中完整性级别 RID；正式应用启动所需的最低进程完整性。
const MEDIUM_INTEGRITY_RID: u32 = 0x2000;

/// 低完整性级别 RID；无法写入用户日志目录或附着 Explorer 桌面层。
const LOW_INTEGRITY_RID: u32 = 0x1000;

/// 读取当前进程完整性级别；失败时返回中文原因。
fn current_process_integrity() -> Result<u32, String>
{
    // 当前进程的查询访问令牌。
    let mut token = HANDLE::default();
    unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.map_err(|error| format!("读取进程令牌失败：{error}"))?;
    // 完整性标签查询返回的缓冲区长度。
    let mut size = 0u32;
    let _ = unsafe { GetTokenInformation(token, TokenIntegrityLevel, None, 0, &mut size) };
    if size == 0
    {
        let _ = unsafe { CloseHandle(token) };
        return Err("读取进程完整性级别失败：系统未返回缓冲区长度".to_string());
    }
    // 按指针宽度对齐的完整性标签缓冲区，供系统写入 TOKEN_MANDATORY_LABEL。
    let mut buffer = vec![0usize; (size as usize).div_ceil(size_of::<usize>())];
    let result = unsafe { GetTokenInformation(token, TokenIntegrityLevel, Some(buffer.as_mut_ptr().cast()), size, &mut size) };
    let _ = unsafe { CloseHandle(token) };
    result.map_err(|error| format!("读取进程完整性级别失败：{error}"))?;
    // 令牌中的完整性标签结构。
    let label = unsafe { &*(buffer.as_ptr() as *const TOKEN_MANDATORY_LABEL) };
    if label.Label.Sid.0.is_null()
    {
        return Err("读取进程完整性级别失败：完整性 SID 为空".to_string());
    }
    // 完整性 SID 的子权限数量与末位 RID。
    let count = unsafe { *GetSidSubAuthorityCount(label.Label.Sid) };
    if count == 0
    {
        return Err("读取进程完整性级别失败：完整性 SID 缺少子权限".to_string());
    }

    Ok(unsafe { *GetSidSubAuthority(label.Label.Sid, u32::from(count - 1)) })
}

/// 低完整性进程的启动提示；中完整性及以上返回 None。
fn low_integrity_message(level: u32, executable: &Path) -> Option<String>
{
    if level >= MEDIUM_INTEGRITY_RID
    {
        return None;
    }
    // 当前级别名称，用于区分低完整性与不受信任级别。
    let level_name = match level
    {
        LOW_INTEGRITY_RID => "低",
        0 => "不受信任",
        _ => "低于中",
    };
    // 当前可执行文件所在目录，恢复标签时作为 icacls 目标。
    let directory = executable.parent().map(|path| path.display().to_string()).unwrap_or_default();

    Some(format!("Desktop Tools 需要以普通（中）完整性级别运行，当前进程为{level_name}完整性级别（0x{level:04X}）。\n\n低完整性进程无法写入用户日志目录，也无法接管 Explorer 桌面层，因此无法启动。\n\n程序路径：{}\n如果程序位于被标记为低完整性的目录（例如开发者沙盒工作区），请恢复该目录的完整性级别后重试：\nicacls \"{directory}\" /setintegritylevel \"(OI)(CI)Medium\"", executable.display()))
}

/// 显示原生启动失败对话框，让无控制台的发布构建同样能看到原因。
fn show_startup_message(message: &str)
{
    // 消息正文与标题的 UTF-16 缓冲区。
    let text: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
    let title: Vec<u16> = "Desktop Tools 启动失败".encode_utf16().chain(Some(0)).collect();
    let _ = unsafe { MessageBoxW(None, PCWSTR(text.as_ptr()), PCWSTR(title.as_ptr()), MB_OK | MB_ICONERROR | MB_SETFOREGROUND) };
}

/// 启动前检查进程完整性级别；低完整性时显示提示并返回原因，供调用方退出。
pub(crate) fn ensure_startup_integrity() -> Result<(), String>
{
    // 当前进程完整性级别；读取失败不阻断启动，交给后续初始化报出真实错误。
    let level = match current_process_integrity()
    {
        Ok(level) => level,
        Err(error) =>
        {
            if cfg!(debug_assertions)
            {
                eprintln!("启动完整性检查失败：{error}");
            }
            return Ok(());
        }
    };
    // 当前可执行文件路径，用于提示需要恢复标签的目录。
    let executable = std::env::current_exe().unwrap_or_default();
    let Some(message) = low_integrity_message(level, &executable)
    else
    {
        return Ok(());
    };
    show_startup_message(&message);

    Err(message)
}

#[cfg(test)]
#[path = "integrity.test.rs"]
mod tests;
