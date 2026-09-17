use serde::Serialize;
use serde::de::DeserializeOwned;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

use crate::platform::windows::replace_file_atomic;

/// 从 UTF-8 JSON 文件恢复完整快照。
pub fn load_json<T: DeserializeOwned>(path: &Path) -> io::Result<Option<T>>
{
    // 文件缺失表示首次启动的空工作区。
    let bytes = match fs::read(path)
    {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    serde_json::from_slice(&bytes).map(Some).map_err(io::Error::other)
}

/// 使用同目录临时文件和 Windows 原子替换保存 UTF-8 JSON 快照。
pub fn save_json_atomic(path: &Path, value: &impl Serialize) -> io::Result<()>
{
    // 工作区快照所在目录。
    let parent = path.parent().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "工作区快照路径缺少父目录"))?;
    fs::create_dir_all(parent)?;

    // 与目标文件位于同一文件系统的临时快照路径。
    let temporary_path = path.with_extension("json.tmp");
    // 等待写入和刷盘的 JSON 内容。
    let bytes = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    // 原子替换前持有的临时文件。
    let mut file = File::create(&temporary_path)?;
    file.write_all(&bytes)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);

    replace_file_atomic(temporary_path.as_os_str(), path.as_os_str())?;
    Ok(())
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
