use serde::Serialize;

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
}

/// 桌面项目快照变化事件名称。
pub const DESKTOP_ITEM_SNAPSHOT_CHANGED_EVENT: &str = "desktop-item-snapshot-changed";

#[cfg(test)]
mod tests {
    use super::{DesktopItem, DesktopItemAvailability, DesktopItemError, DesktopItemSnapshot};

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
}
