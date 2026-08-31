use serde::Serialize;
use serde::de::DeserializeOwned;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

use crate::platform::windows::replace_file_atomic;

/// 从 UTF-8 JSON 文件恢复完整快照。
pub fn load_json<T: DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    // 文件缺失表示首次启动的空工作区。
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(io::Error::other)
}

/// 使用同目录临时文件和 Windows 原子替换保存 UTF-8 JSON 快照。
pub fn save_json_atomic(path: &Path, value: &impl Serialize) -> io::Result<()> {
    // 工作区快照所在目录。
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "工作区快照路径缺少父目录"))?;
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
mod tests {
    use super::{load_json, save_json_atomic};
    use serde::{Deserialize, Serialize};

    /// 原子 JSON 保存测试使用的最小载荷。
    #[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
    struct Payload {
        /// 当前测试修订号。
        revision: u64,
    }

    /// 验证原子替换可以连续更新同一快照文件。
    #[test]
    fn atomic_json_save_replaces_an_existing_snapshot() {
        // 当前测试独占的快照文件。
        let path = std::env::temp_dir().join(format!(
            "desktop-tools-persistence-{}.json",
            std::process::id()
        ));
        save_json_atomic(&path, &Payload { revision: 1 }).expect("首次快照应可保存");
        save_json_atomic(&path, &Payload { revision: 2 }).expect("现有快照应可原子替换");

        assert_eq!(
            load_json(&path).expect("快照应可读取"),
            Some(Payload { revision: 2 })
        );
        std::fs::remove_file(path).expect("测试快照应可清理");
    }
}
