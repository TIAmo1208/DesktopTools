use crate::contracts::{HitRegion, ShellOperationResult, WorkspaceFenceScroll, WorkspacePoint, WorkspaceSnapshot};
use crate::workspace::DropTarget;

/// 原生层已经校验会话与版本的投放输入，不携带 COM 对象。
pub(crate) struct DropInput
{
    pub monitor_id: String,                 // 目标显示器。
    pub pointer: WorkspacePoint,            // 已确认的 DIP 指针。
    pub regions: Vec<HitRegion>,            // 已渲染且裁剪后的命中区域。
    pub scrolls: Vec<WorkspaceFenceScroll>, // 本次临时滚动结果。
}

/// 预检后固定的外部投放计划，文件执行期间不重新解释目标。
pub(crate) struct ExternalDrop
{
    pub(crate) target: DropTarget,          // 共享归属与原视图空间意图。
    pub(crate) snapshot: WorkspaceSnapshot, // 预检时的布局，用于隔离同期新增项目。
    pub(crate) sources: Vec<String>,        // 原始顺序。
    pub(crate) existing: Vec<String>,       // 只改变布局的已知项目。
    pub transfers: Vec<String>,             // 平台需要执行真实传输的完整来源。
    pub portal_root: Option<String>,        // 门户目标绑定根目录。
}

/// 浮岛投放端口：平台只负责 OLE 会话与格式提取，岛卡状态与收集收入在应用层。
pub(crate) trait IslandDropPort: Send + Sync + 'static
{
    fn collect_accepting(&self) -> bool;
    fn receive_collect(&self, content: crate::collect::IncomingContent, mode: String, collect_names: Option<Vec<String>>) -> Result<crate::collect::ReceiveResult, String>;
    fn report_error(&self, error: String);
}

/// 拖放用例端口：平台只提交原生事实，布局事务与文件操作在应用层。
pub(crate) trait DesktopDropPort: Send + Sync + 'static
{
    fn snapshot(&self) -> Result<WorkspaceSnapshot, String>;
    fn content_target(&self, snapshot: &WorkspaceSnapshot, region: &HitRegion, monitor_id: &str) -> Result<Option<String>, String>;
    fn commit_internal(&self, snapshot: WorkspaceSnapshot, input: DropInput, item_ids: Vec<String>, pointer_offset: &WorkspacePoint, free: bool, source_positions: &[WorkspacePoint]) -> Result<bool, String>;
    fn prepare_external(&self, snapshot: WorkspaceSnapshot, input: DropInput, sources: Vec<String>, copy_all: bool) -> Result<Option<ExternalDrop>, String>;
    fn complete_external(&self, plan: ExternalDrop, completed: ShellOperationResult) -> (bool, bool, Option<ShellOperationResult>);
    fn receive_content(&self, region: HitRegion, monitor_id: String, target_name: Option<String>, content: crate::collect::IncomingContent, mode: String, collect_names: Option<Vec<String>>) -> Result<crate::collect::ReceiveResult, String>;
    fn report_content_error(&self, error: String);
}
