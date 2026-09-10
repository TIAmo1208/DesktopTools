use serde::{Deserialize, Serialize};

/// 虚拟桌面环境变化事件名称。
pub const VIRTUAL_DESKTOP_SNAPSHOT_CHANGED_EVENT: &str = "virtual-desktop-snapshot-changed";

/// 前端需要的一个 30 天布局归档摘要。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualDesktopArchiveSummary
{
    /// Desktop Tools 分配的稳定归档标识。
    pub archive_id: String,
    /// 删除前 Windows 虚拟桌面身份。
    pub source_virtual_desktop_id: String,
    /// 归档创建 Unix 时间戳。
    pub archived_at_unix: i64,
    /// 归档自动过期 Unix 时间戳。
    pub expires_at_unix: i64,
}

/// Rust 发布给前端的 Windows 虚拟桌面环境快照。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualDesktopSnapshot
{
    /// 当前适配器使用的 Windows build number。
    pub windows_build: u32,
    /// 当前 Windows 虚拟桌面稳定身份。
    pub current_virtual_desktop_id: String,
    /// 按 Windows 顺序排列的完整活动桌面身份。
    pub virtual_desktop_ids: Vec<String>,
    /// 仍在 30 天保留期内的删除布局归档。
    pub archives: Vec<VirtualDesktopArchiveSummary>,
}

/// 虚拟桌面命令返回的结构化错误。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualDesktopError
{
    /// 可稳定检索的错误码。
    pub code: String,
    /// 用户可见的中文错误说明。
    pub message: String,
}

/// 创建虚拟桌面适配器与协调错误。
impl VirtualDesktopError
{
    /// 创建平台私有兼容层错误。
    pub fn platform_failed(error: &impl std::fmt::Display) -> Self { Self { code: "virtual-desktop-platform-failed".to_string(), message: format!("虚拟桌面适配器失败: {error}") } }

    /// 创建工作区布局协调错误。
    pub fn workspace_failed(error: &impl std::fmt::Display) -> Self { Self { code: "virtual-desktop-workspace-failed".to_string(), message: format!("虚拟桌面布局协调失败: {error}") } }

    /// 创建无效命令参数错误。
    pub fn invalid_command(message: impl Into<String>) -> Self { Self { code: "invalid-virtual-desktop-command".to_string(), message: message.into() } }
}

/// 将虚拟桌面错误写入标准错误传播链。
impl std::fmt::Display for VirtualDesktopError
{
    /// 输出错误的用户可见说明。
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { formatter.write_str(&self.message) }
}

/// 允许虚拟桌面错误进入标准 Rust 错误传播链。
impl std::error::Error for VirtualDesktopError {}
