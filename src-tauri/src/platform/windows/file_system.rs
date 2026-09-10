use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;

use windows::Win32::Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW};
use windows::core::PCWSTR;

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
