use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;

use windows::Win32::Storage::FileSystem::{MOVEFILE_COPY_ALLOWED, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW};
use windows::core::PCWSTR;

/// 移动单个资源文件，允许跨卷但不覆盖；系统保留源文件时不得报告移动成功。
pub(crate) fn move_resource_file(source: &std::path::Path, target: &std::path::Path) -> io::Result<()>
{
    let source_wide: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect(); // Windows 源文件路径。
    let target_wide: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect(); // Windows 目标文件路径。
    unsafe { MoveFileExW(PCWSTR(source_wide.as_ptr()), PCWSTR(target_wide.as_ptr()), MOVEFILE_COPY_ALLOWED | MOVEFILE_WRITE_THROUGH) }.map_err(io::Error::other)?;
    if source.try_exists()?
    {
        return Err(io::Error::other(format!("移动未完成，源文件仍存在；目标副本保留于 {}", target.display())));
    }
    Ok(())
}

/// 使用 Windows 写穿透替换原子发布同卷临时文件。
pub fn replace_file_atomic(temporary_path: &OsStr, target_path: &OsStr) -> io::Result<()>
{
    // Windows API 接收的零结尾临时文件路径。
    let mut temporary_wide: Vec<u16> = temporary_path.encode_wide().collect();
    temporary_wide.push(0);
    // Windows API 接收的零结尾目标文件路径。
    let mut target_wide: Vec<u16> = target_path.encode_wide().collect();
    target_wide.push(0);

    unsafe { MoveFileExW(PCWSTR(temporary_wide.as_ptr()), PCWSTR(target_wide.as_ptr()), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH).map_err(io::Error::other) }
}
