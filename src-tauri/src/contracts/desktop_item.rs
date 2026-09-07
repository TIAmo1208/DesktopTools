use serde::{Deserialize, Serialize};

/// 桌面项目当前可解析状态。
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DesktopItemAvailability {
    /// Shell 当前可以解析项目元数据。
    Available,
    /// Shell 当前保留项目引用并等待后续恢复。
    Unavailable,
}

/// Shell 桌面命名空间中的一个项目。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopItem {
    /// Shell 提供的可重新解析稳定引用。
    pub stable_reference: String,
    /// Shell 提供的系统显示名称。
    pub display_name: String,
    /// Shell 系统图像的 PNG data URL。
    pub system_image: Option<String>,
    /// 当前项目元数据可解析状态。
    pub availability: DesktopItemAvailability,
}

/// 发布给前端的不可变桌面项目快照。
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopItemSnapshot {
    /// 每次成功 Shell 枚举递增的快照修订号。
    pub revision: u64,
    /// 当前完整桌面项目集合。
    pub items: Vec<DesktopItem>,
}

/// 用户触发的 Shell 操作类型。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShellOperationKind {
    /// 重命名单个 Shell 项目。
    Rename,
    /// 将一个或多个项目移入回收站。
    Recycle,
    /// 永久删除一个或多个项目。
    PermanentDelete,
    /// 将选择集合复制到系统剪贴板。
    Copy,
    /// 将选择集合剪切到系统剪贴板。
    Cut,
    /// 将系统剪贴板内容粘贴到桌面命名空间。
    Paste,
    /// 将选择集合交给系统 OLE 拖放。
    Drag,
}

/// 一个 Shell 项目的实际操作结果。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellOperationItemResult {
    /// 操作前由 Shell 返回的稳定引用。
    pub source_reference: String,
    /// 成功后由 Shell 返回的新稳定引用。
    pub result_reference: Option<String>,
    /// 当前项目是否由 Shell 确认成功。
    pub succeeded: bool,
    /// Shell 返回的原生 HRESULT。
    pub result_code: i32,
    /// 操作前源项目的父 Shell 稳定引用，仅供补偿日志使用。
    #[serde(skip_serializing)]
    pub(crate) source_parent_reference: Option<String>,
    /// 操作前源项目的编辑名称，仅供补偿日志使用。
    #[serde(skip_serializing)]
    pub(crate) source_name: Option<String>,
    /// 操作后结果项目的父 Shell 稳定引用，仅供补偿日志使用。
    #[serde(skip_serializing)]
    pub(crate) result_parent_reference: Option<String>,
    /// 操作后结果项目的编辑名称，仅供补偿日志使用。
    #[serde(skip_serializing)]
    pub(crate) result_name: Option<String>,
}

/// 用户触发的一次 Shell 操作结果与撤销属性。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellOperationResult {
    /// 当前应用会话内递增的操作标识；纯剪贴板或拖放动作保持零值。
    pub operation_id: u64,
    /// 操作完成并进入统一时间线的 Unix 微秒时间。
    pub completed_at_unix_micros: i64,
    /// 本次用户请求的 Shell 操作类型。
    pub kind: ShellOperationKind,
    /// Shell 是否在完成前取消了至少一个操作。
    pub aborted: bool,
    /// 本次操作是否写入 Shell 撤销记录。
    pub reversible: bool,
    /// 本次操作是否明确不可逆。
    pub permanent: bool,
    /// OLE 拖放或粘贴最终采用的系统效果。
    pub effect: Option<String>,
    /// 每个项目由 Shell 回报的实际结果。
    pub items: Vec<ShellOperationItemResult>,
    /// 用户可见的中文结果说明。
    pub message: String,
}

/// Tauri command 返回的桌面项目结构化错误。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopItemError {
    /// 可稳定检索的错误码。
    pub code: String,
    /// 用户可见的中文错误说明。
    pub message: String,
}

/// 创建桌面项目 command 使用的稳定错误。
impl DesktopItemError {
    /// 创建 Shell 桌面项目枚举错误。
    pub fn shell_enumeration_failed(error: &impl std::fmt::Display) -> Self {
        Self {
            code: "shell-enumeration-failed".to_string(),
            message: format!("桌面项目枚举失败: {error}"),
        }
    }

    /// 创建桌面项目快照锁错误。
    pub fn snapshot_unavailable() -> Self {
        Self {
            code: "desktop-item-snapshot-unavailable".to_string(),
            message: "桌面项目快照暂时不可用".to_string(),
        }
    }

    /// 创建桌面项目事件发布错误。
    pub fn event_publish_failed(error: &impl std::fmt::Display) -> Self {
        Self {
            code: "desktop-item-event-publish-failed".to_string(),
            message: format!("桌面项目事件发布失败: {error}"),
        }
    }

    /// 创建桌面项目与工作区位置协调错误。
    pub fn workspace_reconciliation_failed(error: &impl std::fmt::Display) -> Self {
        Self {
            code: "desktop-item-workspace-reconciliation-failed".to_string(),
            message: format!("桌面项目位置协调失败: {error}"),
        }
    }

    /// 创建 Shell 项目打开或菜单操作错误。
    pub fn shell_operation_failed(error: &impl std::fmt::Display) -> Self {
        Self {
            code: "shell-operation-failed".to_string(),
            message: format!("Shell 项目操作失败: {error}"),
        }
    }

    /// 创建桌面项目选择参数错误。
    pub fn invalid_selection(message: impl Into<String>) -> Self {
        Self {
            code: "invalid-desktop-item-selection".to_string(),
            message: message.into(),
        }
    }
}

/// 桌面项目快照变化事件名称。
pub const DESKTOP_ITEM_SNAPSHOT_CHANGED_EVENT: &str = "desktop-item-snapshot-changed";

#[cfg(test)]
mod tests {
    use super::{
        DesktopItem, DesktopItemAvailability, DesktopItemError, DesktopItemSnapshot,
        ShellOperationItemResult, ShellOperationKind, ShellOperationResult,
    };

    /// 验证桌面项目快照使用公开序列化契约。
    #[test]
    fn desktop_item_snapshot_uses_the_public_contract_shape() {
        // 等待序列化的桌面项目快照。
        let snapshot = DesktopItemSnapshot {
            revision: 3,
            items: vec![DesktopItem {
                stable_reference: r"C:\Users\User\Desktop\Project".to_string(),
                display_name: "Project".to_string(),
                system_image: Some("data:image/png;base64,AA==".to_string()),
                availability: DesktopItemAvailability::Available,
            }],
        };
        // command 与 event 共用的 JSON 载荷。
        let payload = serde_json::to_value(snapshot).expect("桌面项目快照应可序列化");

        assert_eq!(
            payload,
            serde_json::json!({
                "revision": 3,
                "items": [{
                    "stableReference": r"C:\Users\User\Desktop\Project",
                    "displayName": "Project",
                    "systemImage": "data:image/png;base64,AA==",
                    "availability": "available"
                }]
            })
        );
    }

    /// 验证桌面项目错误使用独立公开契约。
    #[test]
    fn desktop_item_error_uses_the_public_contract_shape() {
        // command JSON 错误载荷。
        let payload = serde_json::to_value(DesktopItemError::snapshot_unavailable())
            .expect("桌面项目错误应可序列化");

        assert_eq!(
            payload,
            serde_json::json!({
                "code": "desktop-item-snapshot-unavailable",
                "message": "桌面项目快照暂时不可用"
            })
        );
    }

    /// 验证永久删除结果明确序列化不可逆属性和原生状态。
    #[test]
    fn shell_operation_result_exposes_actual_and_permanent_state() {
        let payload = serde_json::to_value(ShellOperationResult {
            operation_id: 8,
            completed_at_unix_micros: 1_800_000_000_000_000,
            kind: ShellOperationKind::PermanentDelete,
            aborted: false,
            reversible: false,
            permanent: true,
            effect: None,
            items: vec![ShellOperationItemResult {
                source_reference: r"C:\Users\User\Desktop\obsolete.txt".to_string(),
                result_reference: None,
                succeeded: true,
                result_code: 0,
                source_parent_reference: None,
                source_name: None,
                result_parent_reference: None,
                result_name: None,
            }],
            message: "永久删除完成：1 项".to_string(),
        })
        .expect("Shell 操作结果应可序列化"); // command 返回的公开 JSON 载荷。

        assert_eq!(payload["kind"], "permanent-delete");
        assert_eq!(payload["permanent"], true);
        assert_eq!(payload["items"][0]["succeeded"], true);
    }
}
