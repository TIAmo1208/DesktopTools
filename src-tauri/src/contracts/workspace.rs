use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::component::ComponentInstanceSnapshot;
use super::desktop_item::ShellOperationResult;

/// 当前正式工作区快照的 Schema 版本。
pub const WORKSPACE_SCHEMA_VERSION: u32 = 1;

/// 当前完整工作区备份文件的 Schema 版本。
pub const WORKSPACE_BACKUP_SCHEMA_VERSION: u32 = 4;

/// 围栏允许的最小宽度，单位为 DIP。
pub const FENCE_MIN_WIDTH: f64 = 164.0;

/// 围栏允许的最小展开高度，单位为 DIP。
pub const FENCE_MIN_HEIGHT: f64 = 154.0;

/// 未归类网格相对工作区左侧的起点。
pub const WORKSPACE_GRID_LEFT: f64 = 0.0;

/// 未归类网格相对工作区顶部的起点。
pub const WORKSPACE_GRID_TOP: f64 = 0.0;

/// 未归类网格相对工作区底部的保留空间。
pub const WORKSPACE_GRID_BOTTOM_MARGIN: f64 = 0.0;

/// 未归类网格相邻列的水平间距。
pub const WORKSPACE_GRID_COLUMN_WIDTH: f64 = 78.0;

/// 未归类网格相邻行的垂直间距。
pub const WORKSPACE_GRID_ROW_HEIGHT: f64 = 98.0;

/// 工作区快照变化事件名称。
pub const WORKSPACE_SNAPSHOT_CHANGED_EVENT: &str = "workspace-snapshot-changed";

/// 工作区中的二维位置。
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePoint
{
    /// 相对工作区左边缘的位置。
    pub left: f64,
    /// 相对工作区上边缘的位置。
    pub top: f64,
}

/// 围栏在单显示器工作区中的边界。
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRect
{
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
pub struct WorkspaceGrid
{
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

/// 一台显示器对应的独立未归类网格配置。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceMonitorGrid
{
    /// Windows 显示设备提供的稳定身份。
    pub monitor_id: String,
    /// 当前显示器 DIP 工作区对应的网格。
    pub grid: WorkspaceGrid,
}

/// 为旧快照和宿主初始化阶段提供最小有效网格。
impl Default for WorkspaceGrid
{
    /// 创建使用默认步进和单格容量的工作区网格。
    fn default() -> Self { Self { left: WORKSPACE_GRID_LEFT, top: WORKSPACE_GRID_TOP, column_width: WORKSPACE_GRID_COLUMN_WIDTH, row_height: WORKSPACE_GRID_ROW_HEIGHT, rows: 1, columns: 1 } }
}

/// 围栏成员的排列方式。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FenceLayoutMode
{
    /// 平铺成员。
    Tile,
    /// 纵向列表排列成员。
    List,
}

/// 围栏名称排序状态，旧数据默认保留手动顺序。
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FenceSortMode
{
    /// 保留手动成员顺序。
    #[default]
    None,
    /// 按名称升序排列。
    NameAscending,
    /// 按名称降序排列。
    NameDescending,
}

impl FenceSortMode
{
    /// 省略默认字段以保持旧备份的规范化校验值。
    fn is_none(&self) -> bool { *self == Self::None }
}

/// 公共标题显示策略。
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TileTitleMode
{
    #[default]
    Fixed, // 固定占用标题轨道。
    Hover, // 悬浮覆盖标题。
}

/// 固定标题下的工具按钮显示偏好。
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TileButtonMode
{
    Fixed, // 始终显示工具。
    #[default]
    Hover, // 标题悬停和键盘访问时显示。
}

/// 全部内容共用的宿主外观状态，独立于组件私有 state。
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TileAppearance
{
    pub title: Option<String>,       // 未设置标题时由内容提供默认名称。
    pub title_mode: TileTitleMode,   // 标题轨道策略。
    pub button_mode: TileButtonMode, // 不受标题切换覆盖的按钮偏好。
    pub collapsed: bool,             // 持久折叠状态。
    pub auto_expand: bool,           // 折叠时是否临时展开。
}

/// 单字段外观命令，禁止陈旧页面覆盖其他偏好。
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "type", content = "value", rename_all = "kebab-case", deny_unknown_fields)]
pub enum TileAppearanceChange
{
    Title(String),              // 修改标题。
    TitleMode(TileTitleMode),   // 修改标题策略。
    ButtonMode(TileButtonMode), // 修改按钮偏好。
    Collapsed(bool),            // 修改持久折叠。
    AutoExpand(bool),           // 修改临时展开策略。
}

/// 工作区持久化的完整围栏状态。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FenceSnapshot
{
    /// 工作区内部稳定围栏标识。
    pub id: String,
    /// 用户可见的围栏标题。
    pub title: String,
    /// 围栏持久归属的显示器稳定身份。
    #[serde(default)]
    pub monitor_id: String,
    /// 持久显示器断开时使用的临时投射显示器。
    #[serde(default)]
    pub projection_monitor_id: Option<String>,
    /// 临时投射期间使用的可见边界。
    #[serde(default)]
    pub projection_bounds: Option<WorkspaceRect>,
    /// 围栏展开状态使用的完整边界。
    pub bounds: WorkspaceRect,
    /// 按视觉顺序保存的成员稳定引用。
    pub members: Vec<String>,
    /// 当前成员布局方式。
    pub layout_mode: FenceLayoutMode,
    /// 自动排序方式；无排序沿用当前成员顺序。
    #[serde(default, skip_serializing_if = "FenceSortMode::is_none")]
    pub sort_mode: FenceSortMode,
    /// 围栏内容区滚动位置。
    pub scroll_top: f64,
    /// 围栏当前折叠状态。
    pub collapsed: bool,
    /// 折叠时悬停临时展开；旧备份缺省关闭且保持原校验和。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub auto_expand: bool,
    /// 最近一次展开时的内容高度。
    pub last_expanded_height: f64,
    /// 围栏在工作区中的层级顺序。
    pub z_index: u32,
}

/// 未归类桌面项目进入围栏前保留的位置。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnclassifiedItemSnapshot
{
    /// Shell 提供的桌面项目稳定引用。
    pub stable_reference: String,
    /// 项目持久归属的显示器稳定身份。
    #[serde(default)]
    pub monitor_id: String,
    /// 持久显示器断开时使用的临时投射显示器。
    #[serde(default)]
    pub projection_monitor_id: Option<String>,
    /// 临时投射期间使用的可见位置。
    #[serde(default)]
    pub projection_position: Option<WorkspacePoint>,
    /// 项目回到未归类区域时恢复的位置。
    pub position: WorkspacePoint,
    /// 未归类区域中的持久显示层级。
    #[serde(default)]
    pub z_index: u32,
}

/// 一次项目拖动提交的未归类网格位置。
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceItemPlacement
{
    /// Shell 提供的桌面项目稳定引用。
    pub item_id: String,
    /// 项目提交后的网格位置。
    pub position: WorkspacePoint,
}

/// 一次项目拖动提交的围栏滚动位置。
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFenceScroll
{
    /// Workspace 提供的稳定围栏标识。
    pub fence_id: String,
    /// 拖动自动滚动后的内容位置。
    pub scroll_top: f64,
}

/// 当前桌面内容工作区接收键盘命令的语义目标。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "type", rename_all_fields = "camelCase")]
pub enum WorkspaceFocusTarget
{
    /// 当前显示器桌面背景。
    #[serde(rename = "background")]
    Background,
    /// 一个真实桌面项目。
    #[serde(rename = "item")]
    Item
    {
        /// Shell 提供的稳定项目引用。
        item_id: String,
    },
    /// 一个围栏标题。
    #[serde(rename = "fence")]
    Fence
    {
        /// Workspace 提供的稳定围栏标识。
        fence_id: String,
    },
    /// 一个组件表面。
    #[serde(rename = "component")]
    Component
    {
        /// Component Runtime 提供的稳定组件标识。
        component_id: String,
    },
}

/// 当前虚拟桌面的运行期选择、锚点和键盘焦点。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInteractionSnapshot
{
    /// 每次成功交互状态转换递增的修订号。
    pub revision: u64,
    /// 当前跨焦点容器的完整选择集合。
    pub selected_item_ids: Vec<String>,
    /// 当前范围选择使用的项目锚点。
    pub selection_anchor_id: Option<String>,
    /// 当前接收键盘命令的桌面焦点目标。
    pub keyboard_focus: WorkspaceFocusTarget,
}

/// 为应用启动和虚拟桌面切换提供空交互状态。
impl Default for WorkspaceInteractionSnapshot
{
    /// 创建未选择项目且焦点位于背景的交互状态。
    fn default() -> Self { Self { revision: 0, selected_item_ids: Vec::new(), selection_anchor_id: None, keyboard_focus: WorkspaceFocusTarget::Background } }
}

/// Rust 发布给 React 的不可变工作区快照。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSnapshot
{
    /// 按格子稳定身份发布的共享外观，磁盘仅在全局载荷保存。
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub tile_appearances: std::collections::BTreeMap<String, TileAppearance>,
    /// 当前持久化 Schema 版本。
    pub schema_version: u32,
    /// 每次成功布局事务递增的修订号。
    pub revision: u64,
    /// Rust Workspace 拥有的未归类网格配置。
    #[serde(default)]
    pub grid: WorkspaceGrid,
    /// 当前全部活动显示器对应的独立网格。
    #[serde(default)]
    pub monitor_grids: Vec<WorkspaceMonitorGrid>,
    /// 当前完整围栏集合。
    pub fences: Vec<FenceSnapshot>,
    /// 桌面项目保留的未归类位置。
    pub unclassified_items: Vec<UnclassifiedItemSnapshot>,
    /// 当前虚拟桌面持久化的完整组件实例集合。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<ComponentInstanceSnapshot>,
    /// 当前虚拟桌面的运行期交互状态。
    #[serde(default)]
    pub interaction: WorkspaceInteractionSnapshot,
}

/// 一个已校验工作区备份或自动快照的摘要。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceBackupSummary
{
    /// 当前备份文件 Schema 版本。
    pub schema_version: u32,
    /// 备份创建 Unix 时间戳。
    pub created_at_unix: i64,
    /// 当前备份文件路径。
    pub path: String,
    /// 备份包含的活动虚拟桌面布局数量。
    pub layout_count: usize,
    /// 备份包含的删除布局归档数量。
    pub archive_count: usize,
    /// 备份载荷的完整性校验值。
    pub checksum: String,
}

/// 完整工作区备份事务式还原结果。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRestoreResult
{
    /// 还原后当前 Windows 虚拟桌面的权威布局。
    pub snapshot: WorkspaceSnapshot,
    /// 本次从备份恢复的活动布局数量。
    pub restored_layout_count: usize,
    /// 用户可见的中文还原结果说明。
    pub message: String,
}

/// 统一撤销时间线实际处理的历史单元类型。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceHistoryKind
{
    /// 当前虚拟桌面的布局事务。
    Layout,
    /// 所有虚拟桌面共享的 Shell 文件操作。
    Shell,
    /// 当前方向没有可执行历史。
    None,
}

/// 统一撤销时间线的一次命令结果。
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceHistoryResult
{
    /// 命令完成后的当前虚拟桌面布局。
    pub snapshot: WorkspaceSnapshot,
    /// 本轮实际处理的历史单元类型。
    pub kind: WorkspaceHistoryKind,
    /// Shell 补偿时由 Windows 返回的可选实际结果。
    pub shell_operation: Option<ShellOperationResult>,
    /// 用户可见的中文撤销或重做结果说明。
    pub message: String,
}

/// 项目焦点变化时应用的选择规则。
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceSelectionMode
{
    /// 将选择集合收束到目标项目。
    Replace,
    /// 切换目标项目并保留其他已选项目。
    Toggle,
    /// 从当前选择锚点覆盖同一焦点容器的连续范围。
    Range,
    /// 只移动键盘焦点并保留当前选择集合。
    Preserve,
}

/// 原生上下文菜单对应的工作区语义目标。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", tag = "type", rename_all_fields = "camelCase")]
pub enum WorkspaceContextMenuTarget
{
    /// 当前选择集合中的真实桌面项目。
    #[serde(rename = "items")]
    Items
    {
        /// 按当前选择顺序提交的稳定项目引用。
        item_ids: Vec<String>,
    },
    /// 当前获得高层焦点的围栏。
    #[serde(rename = "fence")]
    Fence
    {
        /// Workspace 提供的稳定围栏标识。
        fence_id: String,
    },
    /// 当前显示器桌面背景。
    #[serde(rename = "background")]
    Background,
}

/// React 可以提交给 Workspace Domain 的运行期交互命令。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", tag = "type", rename_all_fields = "camelCase")]
pub enum WorkspaceInteractionCommand
{
    /// 将键盘焦点移动到项目并应用选择规则。
    #[serde(rename = "focus-item")]
    FocusItem
    {
        /// Shell 提供的稳定项目引用。
        item_id: String,
        /// 当前指针或键盘修饰键对应的选择规则。
        selection_mode: WorkspaceSelectionMode,
    },
    /// 按前端计算的视觉顺序提交框选结果。
    #[serde(rename = "select-items")]
    SelectItems
    {
        /// 与框选矩形相交的项目稳定引用。
        item_ids: Vec<String>,
    },
    /// 将键盘焦点移动到高层语义表面并清空项目选择。
    #[serde(rename = "focus-surface")]
    FocusSurface
    {
        /// 当前接收键盘命令的高层焦点目标。
        focus: WorkspaceFocusTarget,
    },
    /// 选择当前键盘焦点所属容器的全部项目。
    #[serde(rename = "select-focus-container")]
    SelectFocusContainer,
}

/// React 可以提交给工作区领域的完整命令集合。
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", tag = "type", rename_all_fields = "camelCase")]
pub enum WorkspaceCommand
{
    /// 更新一个格子的共享外观，不进入布局历史。
    #[serde(rename = "set-tile-appearance")]
    SetTileAppearance
    {
        target: WorkspaceFocusTarget, // 围栏或组件的稳定身份。
        change: TileAppearanceChange, // 仅修改用户选择的字段。
    },
    /// 创建一个空围栏。
    #[serde(rename = "create-fence")]
    CreateFence
    {
        /// 用户可见标题。
        title: String,
        /// 初始围栏边界。
        bounds: WorkspaceRect,
    },
    /// 提交围栏移动或缩放后的边界。
    #[serde(rename = "update-fence-bounds")]
    UpdateFenceBounds
    {
        /// 目标围栏标识。
        fence_id: String,
        /// 提交后的完整边界。
        bounds: WorkspaceRect,
    },
    /// 提交围栏折叠状态。
    #[serde(rename = "set-fence-collapsed")]
    SetFenceCollapsed
    {
        /// 目标围栏标识。
        fence_id: String,
        /// 下一折叠状态。
        collapsed: bool,
    },
    /// 修改围栏名称。
    #[serde(rename = "rename-fence")]
    RenameFence
    {
        fence_id: String, // 目标围栏。
        title: String,    // 已编辑的新标题。
    },
    /// 设置折叠时悬停展开策略。
    #[serde(rename = "set-fence-auto-expand")]
    SetFenceAutoExpand
    {
        fence_id: String, // 目标围栏。
        enabled: bool,    // 是否启用临时展开。
    },
    /// 提交围栏布局方式。
    #[serde(rename = "set-fence-layout")]
    SetFenceLayout
    {
        /// 目标围栏标识。
        fence_id: String,
        /// 下一布局方式。
        layout_mode: FenceLayoutMode,
    },
    /// 提交围栏排序状态。
    #[serde(rename = "set-fence-sort")]
    SetFenceSort
    {
        /// 目标围栏标识。
        fence_id: String,
        /// 需要持久保存的排序方式。
        sort_mode: FenceSortMode,
    },
    /// 提交围栏内容滚动位置。
    #[serde(rename = "set-fence-scroll")]
    SetFenceScroll
    {
        /// 目标围栏标识。
        fence_id: String,
        /// 下一滚动位置。
        scroll_top: f64,
    },
    /// 将一个桌面项目加入或移动到围栏。
    #[serde(rename = "move-item-to-fence")]
    MoveItemToFence
    {
        /// 桌面项目稳定引用。
        item_id: String,
        /// 目标围栏标识。
        fence_id: String,
        /// 手动顺序中的可选后继成员。
        before_item_id: Option<String>,
    },
    /// 将一个桌面项目恢复到未归类区域。
    #[serde(rename = "remove-item-from-fence")]
    RemoveItemFromFence
    {
        /// 桌面项目稳定引用。
        item_id: String,
    },
    /// 解散围栏并恢复其全部成员来源位置。
    #[serde(rename = "dissolve-fence")]
    DissolveFence
    {
        /// 目标围栏标识。
        fence_id: String,
    },
    /// 原子提交单项或组拖动产生的归属、顺序、位置和滚动状态。
    #[serde(rename = "commit-item-drag")]
    CommitItemDrag
    {
        /// 当前拖动手势的锚点项目。
        anchor_item_id: String,
        /// 按稳定视觉顺序提交的拖动项目。
        item_ids: Vec<String>,
        /// 未归类网格中的最终项目位置。
        placements: Vec<WorkspaceItemPlacement>,
        /// 可选目标围栏；空值表示未归类区域。
        target_fence_id: Option<String>,
        /// 目标围栏手动顺序中的可选后继成员。
        before_item_id: Option<String>,
        /// 拖动期间产生的围栏滚动位置。
        fence_scrolls: Vec<WorkspaceFenceScroll>,
    },
    /// 提交组件移动后的显示器归属和完整边界。
    #[serde(rename = "update-component-bounds")]
    UpdateComponentBounds
    {
        /// Component Runtime 提供的稳定实例标识。
        instance_id: String,
        /// 组件提交后的完整边界。
        bounds: WorkspaceRect,
    },
    /// 提交组件拥有的版本化 JSON 状态。
    #[serde(rename = "update-component-state")]
    UpdateComponentState
    {
        /// Component Runtime 提供的稳定实例标识。
        instance_id: String,
        /// 组件状态对应的持久化版本。
        state_version: u32,
        /// 组件拥有的 JSON 状态。
        state: Value,
    },
    /// 撤销当前虚拟桌面的最近布局事务。
    #[serde(rename = "undo-layout")]
    UndoLayout,
    /// 重做当前虚拟桌面的最近布局事务。
    #[serde(rename = "redo-layout")]
    RedoLayout,
}

/// Tauri command 返回的工作区结构化错误。
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceError
{
    /// 可稳定检索的错误码。
    pub code: String,
    /// 用户可见的中文错误说明。
    pub message: String,
}

/// 创建工作区 command 使用的稳定错误。
impl WorkspaceError
{
    /// 创建无效命令参数错误。
    pub fn invalid_command(message: impl Into<String>) -> Self { Self { code: "invalid-workspace-command".to_string(), message: message.into() } }

    /// 创建工作区状态锁错误。
    pub fn state_unavailable() -> Self { Self { code: "workspace-state-unavailable".to_string(), message: "工作区状态暂时不可用".to_string() } }

    /// 创建工作区快照读取或写入错误。
    pub fn persistence_failed(error: &impl std::fmt::Display) -> Self { Self { code: "workspace-persistence-failed".to_string(), message: format!("工作区快照保存失败: {error}") } }

    /// 创建工作区快照恢复错误。
    pub fn restore_failed(error: &impl std::fmt::Display) -> Self { Self { code: "workspace-restore-failed".to_string(), message: format!("工作区快照恢复失败: {error}") } }

    /// 创建统一撤销或重做执行错误。
    pub fn history_failed(error: &impl std::fmt::Display) -> Self { Self { code: "workspace-history-failed".to_string(), message: format!("工作区历史操作失败: {error}") } }
}

/// 将工作区错误格式化为启动链路可传播的中文说明。
impl std::fmt::Display for WorkspaceError
{
    /// 输出错误的用户可见说明。
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { formatter.write_str(&self.message) }
}

/// 允许工作区错误进入标准 Rust 错误传播链。
impl std::error::Error for WorkspaceError {}

#[cfg(test)]
mod tests
{
    use super::{WorkspaceCommand, WorkspaceContextMenuTarget, WorkspaceFocusTarget, WorkspaceInteractionCommand};

    /// 验证排序与滚动 transport 名称独立，并拒绝非法排序状态。
    #[test]
    fn sorting_command_decodes_without_shadowing_scroll()
    {
        for mode in ["none", "name-ascending", "name-descending"]
        {
            let command: WorkspaceCommand = serde_json::from_value(serde_json::json!({ "type": "set-fence-sort", "fenceId": "fence-1", "sortMode": mode })).unwrap(); // 模拟 React 的真实命令载荷。
            assert!(matches!(command, WorkspaceCommand::SetFenceSort { .. }));
        }
        let scroll: WorkspaceCommand = serde_json::from_value(serde_json::json!({ "type": "set-fence-scroll", "fenceId": "fence-1", "scrollTop": 12 })).unwrap(); // 原有滚动命令必须继续可用。
        assert!(matches!(scroll, WorkspaceCommand::SetFenceScroll { scroll_top: 12.0, .. }));
        assert!(serde_json::from_value::<WorkspaceCommand>(serde_json::json!({ "type": "set-fence-sort", "fenceId": "fence-1", "sortMode": "unknown" })).is_err());
    }

    /// 验证前端多选项目菜单目标通过公开 Rust 契约反序列化。
    #[test]
    fn workspace_context_menu_decodes_selected_items()
    {
        let target: WorkspaceContextMenuTarget = serde_json::from_value(serde_json::json!({
            "type": "items",
            "itemIds": ["a", "b"]
        }))
        .expect("工作区项目菜单目标应可反序列化");

        assert_eq!(target, WorkspaceContextMenuTarget::Items { item_ids: vec!["a".to_string(), "b".to_string()] });
    }

    /// 验证前端组拖动命令通过公开 Rust 契约反序列化。
    #[test]
    fn workspace_command_decodes_group_drag_transaction()
    {
        let command: WorkspaceCommand = serde_json::from_value(serde_json::json!({
            "type": "commit-item-drag",
            "anchorItemId": "a",
            "itemIds": ["a", "b"],
            "placements": [],
            "targetFenceId": "fence-1",
            "beforeItemId": null,
            "fenceScrolls": [{"fenceId": "fence-1", "scrollTop": 24}]
        }))
        .expect("工作区组拖动命令应可反序列化");

        assert!(matches!(
            command,
            WorkspaceCommand::CommitItemDrag {
                anchor_item_id,
                item_ids,
                target_fence_id,
                ..
            } if anchor_item_id == "a" && item_ids == ["a", "b"] && target_fence_id.as_deref() == Some("fence-1")
        ));
    }

    /// 验证前端 camelCase 交互命令通过公开 Rust 契约反序列化。
    #[test]
    fn workspace_interaction_decodes_camel_case_focus_surface()
    {
        let command: WorkspaceInteractionCommand = serde_json::from_value(serde_json::json!({
            "type": "focus-surface",
            "focus": {"type": "fence", "fenceId": "fence-1"}
        }))
        .expect("工作区公开交互命令应可反序列化");

        assert_eq!(command, WorkspaceInteractionCommand::FocusSurface { focus: WorkspaceFocusTarget::Fence { fence_id: "fence-1".to_string() } });
    }

    /// 验证前端 camelCase 命令通过公开 Rust 契约反序列化。
    #[test]
    fn workspace_command_decodes_the_public_contract_shape()
    {
        // React bridge 提交的项目归属命令。
        let command: WorkspaceCommand = serde_json::from_value(serde_json::json!({
            "type": "move-item-to-fence",
            "itemId": "::{project}",
            "fenceId": "fence-1",
            "beforeItemId": null
        }))
        .expect("工作区公开命令应可反序列化");

        assert_eq!(command, WorkspaceCommand::MoveItemToFence { item_id: "::{project}".to_string(), fence_id: "fence-1".to_string(), before_item_id: None });
    }
}
