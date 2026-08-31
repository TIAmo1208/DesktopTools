use serde::{Deserialize, Serialize};

/// 当前正式工作区快照的 Schema 版本。
pub const WORKSPACE_SCHEMA_VERSION: u32 = 1;

/// 围栏允许的最小宽度，单位为 DIP。
pub const FENCE_MIN_WIDTH: f64 = 164.0;

/// 围栏允许的最小展开高度，单位为 DIP。
pub const FENCE_MIN_HEIGHT: f64 = 154.0;

/// 未归类网格相对工作区左侧的起点。
pub const WORKSPACE_GRID_LEFT: f64 = 20.0;

/// 未归类网格相对工作区顶部的起点。
pub const WORKSPACE_GRID_TOP: f64 = 60.0;

/// 未归类网格相对工作区底部的保留空间。
pub const WORKSPACE_GRID_BOTTOM_MARGIN: f64 = 40.0;

/// 未归类网格相邻列的水平间距。
pub const WORKSPACE_GRID_COLUMN_WIDTH: f64 = 92.0;

/// 未归类网格相邻行的垂直间距。
pub const WORKSPACE_GRID_ROW_HEIGHT: f64 = 112.0;

/// 工作区快照变化事件名称。
pub const WORKSPACE_SNAPSHOT_CHANGED_EVENT: &str = "workspace-snapshot-changed";

/// 工作区中的二维位置。
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePoint {
    /// 相对工作区左边缘的位置。
    pub left: f64,
    /// 相对工作区上边缘的位置。
    pub top: f64,
}

/// 围栏在单显示器工作区中的边界。
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRect {
    /// 相对工作区左边缘的位置。
    pub left: f64,
    /// 相对工作区上边缘的位置。
    pub top: f64,
    /// 围栏宽度。
    pub width: f64,
    /// 围栏展开高度。
    pub height: f64,
}

/// Rust Workspace 发布给 React 的未归类网格配置。
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceGrid {
    /// 网格左侧起点。
    pub left: f64,
    /// 网格顶部起点。
    pub top: f64,
    /// 相邻列的水平间距。
    pub column_width: f64,
    /// 相邻行的垂直间距。
    pub row_height: f64,
    /// 当前工作区可用行数。
    pub rows: u32,
    /// 当前工作区可用列数。
    pub columns: u32,
}

/// 为旧快照和宿主初始化阶段提供最小有效网格。
impl Default for WorkspaceGrid {
    fn default() -> Self {
        Self {
            left: WORKSPACE_GRID_LEFT,
            top: WORKSPACE_GRID_TOP,
            column_width: WORKSPACE_GRID_COLUMN_WIDTH,
            row_height: WORKSPACE_GRID_ROW_HEIGHT,
            rows: 1,
            columns: 1,
        }
    }
}

/// 围栏成员的排列方式。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FenceLayoutMode {
    /// 平铺成员。
    Tile,
    /// 纵向列表排列成员。
    List,
}

/// 工作区持久化的完整围栏状态。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FenceSnapshot {
    /// 工作区内部稳定围栏标识。
    pub id: String,
    /// 用户可见的围栏标题。
    pub title: String,
    /// 围栏展开状态使用的完整边界。
    pub bounds: WorkspaceRect,
    /// 按视觉顺序保存的成员稳定引用。
    pub members: Vec<String>,
    /// 当前成员布局方式。
    pub layout_mode: FenceLayoutMode,
    /// 围栏内容区滚动位置。
    pub scroll_top: f64,
    /// 围栏当前折叠状态。
    pub collapsed: bool,
    /// 最近一次展开时的内容高度。
    pub last_expanded_height: f64,
    /// 围栏在工作区中的层级顺序。
    pub z_index: u32,
}

/// 未归类桌面项目进入围栏前保留的位置。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnclassifiedItemSnapshot {
    /// Shell 提供的桌面项目稳定引用。
    pub stable_reference: String,
    /// 项目回到未归类区域时恢复的位置。
    pub position: WorkspacePoint,
}

/// Rust 发布给 React 的不可变工作区快照。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSnapshot {
    /// 当前持久化 Schema 版本。
    pub schema_version: u32,
    /// 每次成功布局事务递增的修订号。
    pub revision: u64,
    /// Rust Workspace 拥有的未归类网格配置。
    #[serde(default)]
    pub grid: WorkspaceGrid,
    /// 当前完整围栏集合。
    pub fences: Vec<FenceSnapshot>,
    /// 桌面项目保留的未归类位置。
    pub unclassified_items: Vec<UnclassifiedItemSnapshot>,
}

/// React 可以提交给工作区领域的完整命令集合。
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(
    rename_all = "camelCase",
    tag = "type",
    rename_all_fields = "camelCase"
)]
pub enum WorkspaceCommand {
    /// 创建一个空围栏。
    #[serde(rename = "create-fence")]
    CreateFence {
        /// 用户可见标题。
        title: String,
        /// 初始围栏边界。
        bounds: WorkspaceRect,
    },
    /// 提交围栏移动或缩放后的边界。
    #[serde(rename = "update-fence-bounds")]
    UpdateFenceBounds {
        /// 目标围栏标识。
        fence_id: String,
        /// 提交后的完整边界。
        bounds: WorkspaceRect,
    },
    /// 提交围栏折叠状态。
    #[serde(rename = "set-fence-collapsed")]
    SetFenceCollapsed {
        /// 目标围栏标识。
        fence_id: String,
        /// 下一折叠状态。
        collapsed: bool,
    },
    /// 提交围栏布局方式。
    #[serde(rename = "set-fence-layout")]
    SetFenceLayout {
        /// 目标围栏标识。
        fence_id: String,
        /// 下一布局方式。
        layout_mode: FenceLayoutMode,
    },
    /// 提交围栏内容滚动位置。
    #[serde(rename = "set-fence-scroll")]
    SetFenceScroll {
        /// 目标围栏标识。
        fence_id: String,
        /// 下一滚动位置。
        scroll_top: f64,
    },
    /// 将一个桌面项目加入或移动到围栏。
    #[serde(rename = "move-item-to-fence")]
    MoveItemToFence {
        /// 桌面项目稳定引用。
        item_id: String,
        /// 目标围栏标识。
        fence_id: String,
        /// 手动顺序中的可选后继成员。
        before_item_id: Option<String>,
    },
    /// 将一个桌面项目恢复到未归类区域。
    #[serde(rename = "remove-item-from-fence")]
    RemoveItemFromFence {
        /// 桌面项目稳定引用。
        item_id: String,
    },
    /// 解散围栏并恢复其全部成员来源位置。
    #[serde(rename = "dissolve-fence")]
    DissolveFence {
        /// 目标围栏标识。
        fence_id: String,
    },
}

/// Tauri command 返回的工作区结构化错误。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceError {
    /// 可稳定检索的错误码。
    pub code: String,
    /// 用户可见的中文错误说明。
    pub message: String,
}

/// 创建工作区 command 使用的稳定错误。
impl WorkspaceError {
    /// 创建无效命令参数错误。
    pub fn invalid_command(message: impl Into<String>) -> Self {
        Self {
            code: "invalid-workspace-command".to_string(),
            message: message.into(),
        }
    }

    /// 创建工作区状态锁错误。
    pub fn state_unavailable() -> Self {
        Self {
            code: "workspace-state-unavailable".to_string(),
            message: "工作区状态暂时不可用".to_string(),
        }
    }

    /// 创建工作区快照读取或写入错误。
    pub fn persistence_failed(error: &impl std::fmt::Display) -> Self {
        Self {
            code: "workspace-persistence-failed".to_string(),
            message: format!("工作区快照保存失败: {error}"),
        }
    }

    /// 创建工作区快照恢复错误。
    pub fn restore_failed(error: &impl std::fmt::Display) -> Self {
        Self {
            code: "workspace-restore-failed".to_string(),
            message: format!("工作区快照恢复失败: {error}"),
        }
    }
}

/// 将工作区错误格式化为启动链路可传播的中文说明。
impl std::fmt::Display for WorkspaceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

/// 允许工作区错误进入标准 Rust 错误传播链。
impl std::error::Error for WorkspaceError {}

#[cfg(test)]
mod tests {
    use super::WorkspaceCommand;

    /// 验证前端 camelCase 命令通过公开 Rust 契约反序列化。
    #[test]
    fn workspace_command_decodes_the_public_contract_shape() {
        // React bridge 提交的项目归属命令。
        let command: WorkspaceCommand = serde_json::from_value(serde_json::json!({
            "type": "move-item-to-fence",
            "itemId": "::{project}",
            "fenceId": "fence-1",
            "beforeItemId": null
        }))
        .expect("工作区公开命令应可反序列化");

        assert_eq!(
            command,
            WorkspaceCommand::MoveItemToFence {
                item_id: "::{project}".to_string(),
                fence_id: "fence-1".to_string(),
                before_item_id: None,
            }
        );
    }
}
