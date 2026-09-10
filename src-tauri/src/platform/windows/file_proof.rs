use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Security::Cryptography::{BCRYPT_HASH_HANDLE, BCRYPT_SHA256_ALG_HANDLE, BCryptCreateHash, BCryptDestroyHash, BCryptFinishHash, BCryptHashData};
use windows::Win32::Storage::FileSystem::{FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FileIdInfo, GetFileInformationByHandleEx};

/// 文件树的一次完整身份与内容观察，不保存文件副本。
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileProof
{
    pub path: PathBuf,       // 当前可再次观察的真实文件系统路径。
    entries: Vec<FileEntry>, // 根与后代的排序后身份、属性及摘要。
}

/// 单个文件系统对象的补偿安全证据。
#[derive(Clone, Debug, Eq, PartialEq)]
struct FileEntry
{
    relative: PathBuf,                // 根内部的精确相对路径。
    volume: u64,                      // 文件身份所属卷。
    identity: [u8; 16],               // Windows 128 位文件身份。
    attributes: u32,                  // 包含目录、只读等属性，重解析点不进入补偿。
    length: u64,                      // 当前文件长度。
    modified: u64,                    // 文件系统最后写入时间。
    digest: [u8; 32],                 // 常规文件的 SHA-256，目录本身不读内容。
    streams: Vec<(String, [u8; 32])>, // NTFS 命名数据流同样属于不可丢失的文件内容。
}

/// 在所有退出路径释放 CNG 哈希对象。
struct HashHandle(BCRYPT_HASH_HANDLE);

/// 读取实际卷身份，默认拖放效果不能按盘符猜测重定向或目录连接。
pub(crate) fn volume_id(path: &Path) -> Result<u64, String>
{
    let file = OpenOptions::new().access_mode(0x80).share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0).custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0).open(path).map_err(|error| error.to_string())?; // FILE_READ_ATTRIBUTES，不读取文件内容。
    let mut identity = FILE_ID_INFO::default();
    unsafe { GetFileInformationByHandleEx(HANDLE(file.as_raw_handle()), FileIdInfo, (&mut identity as *mut FILE_ID_INFO).cast(), std::mem::size_of::<FILE_ID_INFO>() as u32) }.map_err(|error| error.to_string())?;
    Ok(identity.VolumeSerialNumber)
}

impl Drop for HashHandle
{
    fn drop(&mut self) { let _ = unsafe { BCryptDestroyHash(self.0) }; }
}

impl FileProof
{
    /// 流式记录整个结果树；观察中发生变化、不可读或重解析点均不授权补偿。
    pub(crate) fn capture(path: &Path) -> Result<Self, String>
    {
        let mut entries = Vec::new(); // 当前完整观察。
        let mut pending = vec![path.to_path_buf()]; // 不跟随重解析点的显式目录遍历。
        while let Some(current) = pending.pop()
        {
            let metadata = std::fs::symlink_metadata(&current).map_err(|error| error.to_string())?; // 先检查目录连接和符号链接。
            if metadata.file_attributes() & 0x400 != 0
            {
                return Err("重解析点不能自动补偿".into());
            }
            let mut file = OpenOptions::new().read(true).share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0 | FILE_SHARE_DELETE.0).custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | windows::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT.0).open(&current).map_err(|error| error.to_string())?; // 打开阶段也禁止跟随刚被替换的重解析点。
            let before = file.metadata().map_err(|error| error.to_string())?; // 摘要之前的文件状态。
            if before.file_attributes() & 0x400 != 0
            {
                return Err("观察路径已变为重解析点".into());
            }
            let mut identity = FILE_ID_INFO::default(); // 当前打开对象的稳定文件身份。
            unsafe { GetFileInformationByHandleEx(HANDLE(file.as_raw_handle()), FileIdInfo, (&mut identity as *mut FILE_ID_INFO).cast(), std::mem::size_of::<FILE_ID_INFO>() as u32) }.map_err(|error| error.to_string())?;
            let digest = if before.is_file() { digest_file(&mut file)? } else { [0; 32] }; // 大文件流式处理，不缓存全部内容。
            let streams = digest_streams(&current)?; // 包括目录附加的数据流，不能只核对默认内容。
            let after = file.metadata().map_err(|error| error.to_string())?; // 观察期间的修改使证明失效。
            if before.file_size() != after.file_size() || before.last_write_time() != after.last_write_time()
            {
                return Err("文件在记录补偿依据期间发生变化".into());
            }
            entries.push(FileEntry { relative: current.strip_prefix(path).map_err(|error| error.to_string())?.to_path_buf(), volume: identity.VolumeSerialNumber, identity: identity.FileId.Identifier, attributes: after.file_attributes(), length: after.file_size(), modified: after.last_write_time(), digest, streams });
            if before.is_dir()
            {
                for child in std::fs::read_dir(&current).map_err(|error| error.to_string())?
                {
                    pending.push(child.map_err(|error| error.to_string())?.path());
                }
            }
        }
        entries.sort_by(|first, second| first.relative.cmp(&second.relative));
        Ok(Self { path: path.to_path_buf(), entries })
    }

    /// 补偿前重新观察，任何身份、内容、后代或属性变化都保留当前文件。
    pub(crate) fn unchanged(&self) -> bool { Self::capture(&self.path).is_ok_and(|current| current == *self) }

    /// 子树执行前仍核对原证明，根之外的任何回调都不能借用这份授权。
    pub(crate) fn subtree_unchanged(&self, path: &Path) -> bool
    {
        let Ok(relative) = path.strip_prefix(&self.path)
        else
        {
            return false;
        };
        let entries = self
            .entries
            .iter()
            .filter_map(|entry| {
                let suffix = entry.relative.strip_prefix(relative).ok()?;
                let mut entry = entry.clone();
                entry.relative = suffix.to_path_buf();
                Some(entry)
            })
            .collect::<Vec<_>>();
        !entries.is_empty() && Self::capture(path).is_ok_and(|current| current.entries == entries)
    }

    /// 复制后的卷与文件身份可以变化，完整相对路径和内容必须相同。
    pub(crate) fn same_contents(&self, other: &Self) -> bool { self.entries.len() == other.entries.len() && self.entries.iter().zip(&other.entries).all(|(left, right)| left.relative == right.relative && left.attributes & 0x10 == right.attributes & 0x10 && left.streams == right.streams && (left.attributes & 0x10 != 0 || (left.length == right.length && left.digest == right.digest))) }
}

/// 枚举并摘要所有命名数据流，任何不完整观察都不能授权补偿。
fn digest_streams(path: &Path) -> Result<Vec<(String, [u8; 32])>, String>
{
    use windows::Win32::Storage::FileSystem::{FindClose, FindFirstStreamW, FindNextStreamW, FindStreamInfoStandard, WIN32_FIND_STREAM_DATA};
    let mut data = WIN32_FIND_STREAM_DATA::default(); // Windows 返回的当前数据流描述。
    let handle = match unsafe { FindFirstStreamW(&windows::core::HSTRING::from(path.as_os_str()), FindStreamInfoStandard, (&mut data as *mut WIN32_FIND_STREAM_DATA).cast(), None) }
    {
        Ok(handle) => handle,
        Err(error) if error.code() == windows::core::HRESULT::from_win32(windows::Win32::Foundation::ERROR_HANDLE_EOF.0) => return Ok(Vec::new()),
        Err(error) => return Err(error.to_string()),
    };
    let result = (|| {
        let mut streams = Vec::new(); // 排序后与下一次观察逐项比较。
        loop
        {
            let end = data.cStreamName.iter().position(|value| *value == 0).unwrap_or(data.cStreamName.len());
            let name = String::from_utf16(&data.cStreamName[..end]).map_err(|error| error.to_string())?;
            if name != "::$DATA"
            {
                let mut stream_path = path.as_os_str().to_os_string();
                stream_path.push(&name);
                let mut stream = File::open(stream_path).map_err(|error| error.to_string())?;
                streams.push((name, digest_file(&mut stream)?));
            }
            match unsafe { FindNextStreamW(handle, (&mut data as *mut WIN32_FIND_STREAM_DATA).cast()) }
            {
                Ok(()) => (),
                Err(error) if error.code() == windows::core::HRESULT::from_win32(windows::Win32::Foundation::ERROR_HANDLE_EOF.0) => break,
                Err(error) => return Err(error.to_string()),
            }
        }
        streams.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(streams)
    })();
    let _ = unsafe { FindClose(handle) }; // 任意读取失败也关闭枚举句柄。
    result
}

/// 使用 Windows CNG 对文件生成流式 SHA-256。
fn digest_file(file: &mut File) -> Result<[u8; 32], String>
{
    let mut handle = BCRYPT_HASH_HANDLE::default(); // CNG 分配并由 RAII 释放的哈希对象。
    unsafe { BCryptCreateHash(BCRYPT_SHA256_ALG_HANDLE, &mut handle, None, None, 0) }.ok().map_err(|error| error.to_string())?;
    let hash = HashHandle(handle); // 后续任一读取错误仍释放对象。
    let mut buffer = [0_u8; 65536]; // 固定内存的流式读取缓冲区。
    loop
    {
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?; // 当前摘要分块。
        if count == 0
        {
            break;
        }
        unsafe { BCryptHashData(hash.0, &buffer[..count], 0) }.ok().map_err(|error| error.to_string())?;
    }
    let mut digest = [0_u8; 32]; // SHA-256 固定长度结果。
    unsafe { BCryptFinishHash(hash.0, &mut digest, 0) }.ok().map_err(|error| error.to_string())?;
    Ok(digest)
}

#[cfg(test)]
mod tests
{
    use super::*;

    /// 同长度内容、附加流、新增后代和同名替换均使旧补偿授权失效。
    #[test]
    fn proof_detects_content_stream_tree_and_identity_changes()
    {
        let root = std::env::temp_dir().join(format!("desktop-tools-proof-{}-{}", std::process::id(), crate::tools::time::next_timeline_timestamp())); // 独占测试目录。
        std::fs::create_dir(&root).unwrap();
        let path = root.join("item.txt");
        std::fs::write(&path, b"before").unwrap();
        let initial = FileProof::capture(&root).unwrap();
        assert!(initial.unchanged());
        assert!(initial.subtree_unchanged(&path));
        std::fs::write(&path, b"after!").unwrap();
        assert!(!initial.unchanged());
        let modified = FileProof::capture(&root).unwrap();
        let mut stream = path.as_os_str().to_os_string();
        stream.push(":proof-test:$DATA");
        std::fs::write(stream, b"named stream").unwrap();
        assert!(!modified.unchanged());
        let with_stream = FileProof::capture(&root).unwrap();
        std::fs::write(root.join("new.txt"), b"concurrent").unwrap();
        assert!(!with_stream.unchanged());
        let before_replace = FileProof::capture(&path).unwrap();
        std::fs::rename(&path, root.join("old.txt")).unwrap();
        std::fs::write(&path, b"after!").unwrap();
        assert!(!before_replace.unchanged());
        std::fs::remove_dir_all(root).unwrap();
    }
}
