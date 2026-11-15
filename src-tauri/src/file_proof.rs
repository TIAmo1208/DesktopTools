use std::path::PathBuf;

/// 文件树的一次完整身份与内容观察，不保存文件副本。
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct FileProof
{
    pub path: PathBuf,                  // 当前可再次观察的真实文件系统路径。
    pub(crate) entries: Vec<FileEntry>, // 根与后代的排序后身份、属性及摘要。
}

/// 单个文件系统对象的补偿安全证据。
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct FileEntry
{
    pub(crate) relative: PathBuf,                // 根内部的精确相对路径。
    pub(crate) volume: u64,                      // 文件身份所属卷。
    pub(crate) identity: [u8; 16],               // Windows 128 位文件身份。
    pub(crate) attributes: u32,                  // 包含目录、只读等属性，重解析点不进入补偿。
    pub(crate) length: u64,                      // 当前文件长度。
    pub(crate) modified: u64,                    // 文件系统最后写入时间。
    pub(crate) digest: [u8; 32],                 // 常规文件的 SHA-256，目录本身不读内容。
    pub(crate) streams: Vec<(String, [u8; 32])>, // NTFS 命名数据流同样属于不可丢失的文件内容。
}
