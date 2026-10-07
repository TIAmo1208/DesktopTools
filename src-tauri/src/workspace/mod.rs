use std::collections::HashMap;
mod drop;
pub(crate) use drop::{DropLayoutChange, DropTarget};
mod appearance;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::app::settings::SettingsState;
use crate::contracts::settings::{ApplicationPreferences, GridPreferences};
use crate::contracts::{ComponentInstanceSnapshot, DesktopHostMonitor, FenceLayoutMode, FenceSnapshot, FenceSortMode, TILE_MIN_HEIGHT, TILE_MIN_WIDTH, UnclassifiedItemSnapshot, WORKSPACE_BACKUP_SCHEMA_VERSION, WORKSPACE_GRID_BOTTOM_MARGIN, WORKSPACE_GRID_LEFT, WORKSPACE_GRID_TOP, WORKSPACE_SCHEMA_VERSION, WorkspaceBackupSummary, WorkspaceCommand, WorkspaceError, WorkspaceFenceScroll, WorkspaceFocusTarget, WorkspaceGrid, WorkspaceInteractionCommand, WorkspaceInteractionSnapshot, WorkspaceItemPlacement, WorkspaceMonitorGrid, WorkspacePoint, WorkspaceRect, WorkspaceRestoreResult, WorkspaceSelectionMode, WorkspaceSnapshot};
use crate::persistence::{load_json, save_json_atomic};

/// 统一工作区持久化格式版本。
const WORKSPACE_STORE_VERSION: u32 = WORKSPACE_BACKUP_SCHEMA_VERSION;

/// 自动快照在布局空闲后的合并等待时间。
const AUTO_SNAPSHOT_IDLE: Duration = Duration::from_secs(2);

/// 统一工作区保留的最近自动快照数量。
const AUTO_SNAPSHOT_LIMIT: usize = 20;

/// 同一进程内自动快照文件名的递增唯一序号。
static NEXT_AUTO_SNAPSHOT_ID: AtomicU64 = AtomicU64::new(1);

/// 一次协调使用的环境事实；采集时间由调用方决定，不宣称跨运行时原子采集。
pub(crate) struct WorkspaceEnvironment<'a>
{
    pub new_item_positions: &'a [(String, String, WorkspacePoint)], // 按准确项目身份读取的 Explorer 新增位置，只作用于首次出现的项目。
    pub transfer_assignments: bool,                                 // 确定的投放新增位置作用于工作区，不重导入 Explorer 布局。
    pub monitors: &'a [DesktopHostMonitor],                         // 当前逐屏物理环境。
    pub items: &'a [crate::contracts::DesktopItem],                 // 同次 Shell 观察的引用与名称。
    pub assignments: &'a [(String, String)],                        // 初次导入可用的 Explorer 归属。
    pub renames: &'a [(String, String)],                            // Shell 确认的精确重命名。
}

/// 持有正式桌面内容工作区的权威布局和持久化位置。
pub struct WorkspaceState
{
    grid_preferences: Mutex<GridPreferences>, // 全部显示器共用的网格生成参数。
    /// Shell 最近发布的显示名称，仅供排序，不写入布局文件。
    item_names: Mutex<HashMap<String, String>>,
    /// command 共享的当前工作区快照。
    snapshot: Mutex<WorkspaceSnapshot>,
    /// 首次 Explorer 布局导入状态，与布局共同保存。
    explorer_layout_imported: Mutex<bool>,
    /// 统一的会话布局撤销与重做栈。
    history: Mutex<WorkspaceHistory>,
    /// 每次成功布局事务使用的原子 JSON 文件。
    persistence_path: PathBuf,
    /// 等待 2 秒空闲合并写入的完整工作区自动快照发送端。
    auto_snapshot_sender: Option<Sender<PersistedWorkspaceStore>>,
    /// 应用退出前刷新待处理快照的后台线程。
    auto_snapshot_worker: Option<JoinHandle<()>>,
    /// 还原前同步快照和自动快照共用的目录。
    auto_snapshot_directory: PathBuf,
}

/// 保存统一工作区的会话布局历史。
#[derive(Clone, Default)]
struct WorkspaceHistory
{
    /// 从最新到最旧保存的可撤销布局。
    undo: Vec<WorkspaceHistoryEntry>,
    /// 从最新到最旧保存的可重做布局。
    redo: Vec<WorkspaceHistoryEntry>,
}

/// 一个按完成时间参与统一时间线的布局历史单元。
#[derive(Clone)]
struct WorkspaceHistoryEntry
{
    /// 当前方向恢复后应成为活动状态的布局。
    layout: WorkspaceSnapshot,
    /// 当前历史单元最近进入该方向的 Unix 微秒时间。
    timeline_at_unix_micros: i64,
}

/// 一次布局撤销或重做的权威快照与部分恢复统计。
pub(crate) struct WorkspaceLayoutHistoryResult
{
    /// 已原子持久化的当前布局。
    pub snapshot: WorkspaceSnapshot,
    /// 仍存在并按历史状态恢复的项目数量。
    pub restored_count: usize,
    /// 历史中已经离开当前工作区的项目数量。
    pub changed_count: usize,
}

/// 统一工作区持久载荷，不保存 Windows 虚拟桌面身份。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedWorkspacePayload
{
    layout: WorkspaceSnapshot,      // 包含成员、外观和组件的唯一布局。
    explorer_layout_imported: bool, // 首次 Explorer 布局导入状态。
}

/// 带版本、创建时间和完整性校验的工作区文件。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PersistedWorkspaceStore
{
    /// 统一工作区存储格式版本。
    store_version: u32,
    /// 文件创建 Unix 时间戳。
    created_at_unix: i64,
    /// 规范化载荷的 FNV-1a 完整性校验值。
    checksum: String,
    /// 完整布局和首次导入状态。
    payload: PersistedWorkspacePayload,
}

/// 提供工作区 command 使用的领域操作。
impl WorkspaceState
{
    /// 测试使用无默认组件的工作区。
    #[cfg(test)]
    pub fn load(persistence_path: PathBuf) -> Result<Self, WorkspaceError> { Self::load_with_components(persistence_path, &[]) }

    /// 恢复唯一工作区；缺失文件使用默认值，旧格式明确报错。
    pub fn load_with_components(persistence_path: PathBuf, default_components: &[ComponentInstanceSnapshot]) -> Result<Self, WorkspaceError>
    {
        let value = load_json::<serde_json::Value>(&persistence_path).map_err(|error| WorkspaceError::restore_failed(&error))?; // 当前保存文件。
        let (mut snapshot, explorer_layout_imported) = match value
        {
            Some(value) =>
            {
                let store = decode_workspace_store(value)?; // 已校验的完整工作区。
                (store.payload.layout, store.payload.explorer_layout_imported)
            }
            None => (default_workspace_snapshot(default_components), false),
        }; // 不查询或分区 Windows 虚拟桌面。
        snapshot.interaction = WorkspaceInteractionSnapshot::default();
        let members = snapshot.fences.iter().flat_map(|fence| fence.members.iter()).collect::<std::collections::HashSet<_>>(); // 围栏身份权威与桌面位置互斥。
        snapshot.unclassified_items.retain(|item| !members.contains(&item.stable_reference)); // 启动时不将隐藏来源记录带入活动布局。
        snapshot.validate()?;
        crate::log_info!("工作区：统一布局恢复完成，修订={}，围栏={}，组件={}，路径={}", snapshot.revision, snapshot.fences.len(), snapshot.components.len(), persistence_path.display());
        let auto_snapshot_directory = persistence_path.with_extension("snapshots"); // 当前工作区快照目录。
        let (auto_snapshot_sender, auto_snapshot_worker) = start_auto_snapshot_worker(auto_snapshot_directory.clone()).map_err(|error| WorkspaceError::persistence_failed(&error))?; // 空闲合并线程。
        Ok(Self { grid_preferences: Mutex::new(GridPreferences::default()), item_names: Mutex::new(HashMap::new()), snapshot: Mutex::new(snapshot), explorer_layout_imported: Mutex::new(explorer_layout_imported), history: Mutex::new(WorkspaceHistory::default()), persistence_path, auto_snapshot_sender: Some(auto_snapshot_sender), auto_snapshot_worker: Some(auto_snapshot_worker), auto_snapshot_directory })
    }

    /// 更新 Shell 权威显示名称供后续布局事务排序。
    #[cfg(test)]
    pub fn update_item_names(&self, items: &[crate::contracts::DesktopItem]) -> Result<(), WorkspaceError>
    {
        let mut names = self.item_names.lock().map_err(|_| WorkspaceError::state_unavailable())?; // 当前运行期名称表。
        *names = items.iter().map(|item| (item.stable_reference.clone(), item.display_name.to_lowercase())).collect();
        Ok(())
    }

    /// 在逐屏宿主启动之前恢复网格生成参数。
    pub fn initialize_grid_preferences(&self, preferences: GridPreferences) -> Result<(), WorkspaceError>
    {
        preferences.validate().map_err(|message| WorkspaceError::invalid_command(&message))?;
        *self.grid_preferences.lock().map_err(|_| WorkspaceError::state_unavailable())? = preferences;
        Ok(())
    }

    /// 所有环境协调入口共用当前网格参数。
    fn monitor_grids(&self, monitors: &[DesktopHostMonitor]) -> Result<Vec<WorkspaceMonitorGrid>, WorkspaceError>
    {
        let preferences = *self.grid_preferences.lock().map_err(|_| WorkspaceError::state_unavailable())?; // 一次协调中的固定参数。
        monitor_grids(monitors, preferences)
    }

    /// 校验工作区网格容量，再保存布局及偏好；偏好写入失败时恢复旧布局文件。
    pub fn save_preferences(&self, settings: &SettingsState, preferences: ApplicationPreferences, monitors: &[DesktopHostMonitor]) -> Result<ApplicationPreferences, String>
    {
        crate::app::settings::validate_preferences(&preferences)?;
        let mut grid_preferences = self.grid_preferences.lock().map_err(|_| "网格状态不可用")?; // 阻止网格配置并发提交。
        if *grid_preferences == preferences.desktop_grid
        {
            return settings.save(preferences);
        }
        let grids = monitor_grids(monitors, preferences.desktop_grid).map_err(|error| error.message)?; // 当前显示器的新网格。
        let mut current = self.snapshot.lock().map_err(|_| "工作区状态不可用")?; // 当前活动布局锁。
        let imported = self.explorer_layout_imported.lock().map_err(|_| "工作区导入状态不可用")?; // 与布局共同保存的首次导入状态。
        let mut next = current.clone(); // 保存失败时不影响可见布局。
        reflow_grid_items(&mut next, &grids).map_err(|error| error.message)?;
        next.monitor_grids = grids.clone();
        next.grid = grids.iter().find(|entry| monitors.iter().any(|monitor| monitor.primary && monitor.id == entry.monitor_id)).or(grids.first()).ok_or("工作区缺少显示器")?.grid;
        next.revision += 1;
        next.validate().map_err(|error| error.message)?;
        let store = save_workspace_store(&self.persistence_path, &next, *imported).map_err(|error| format!("保存网格布局失败：{error}"))?; // 先保存布局，再保存偏好。
        let saved = match settings.save(preferences)
        {
            Ok(saved) => saved,
            Err(error) =>
            {
                if let Err(rollback) = save_workspace_store(&self.persistence_path, &current, *imported)
                {
                    crate::log_error!("设置：网格布局回滚失败，错误：{rollback}");
                    return Err(format!("{error}；布局文件回滚失败：{rollback}"));
                }
                return Err(error);
            }
        }; // 仅两份文件均写入成功后更新运行状态。
        *grid_preferences = saved.desktop_grid;
        *current = next;
        self.schedule_auto_snapshot(store);
        crate::log_info!("设置：桌面网格已应用，宽度：{}，高度：{}，横向间距：{}，纵向间距：{}", saved.desktop_grid.width, saved.desktop_grid.height, saved.desktop_grid.column_gap, saved.desktop_grid.row_gap);
        Ok(saved)
    }

    /// 使用最新 Shell 名称统一排序，返回成员顺序是否变化。
    fn sort_fences(&self, snapshot: &mut WorkspaceSnapshot) -> Result<bool, WorkspaceError>
    {
        let names = self.item_names.lock().map_err(|_| WorkspaceError::state_unavailable())?; // 当前 Shell 名称索引。
        Ok(Self::sort_fences_with_names(snapshot, &names))
    }

    /// 使用本次事务固定的名称事实排序，不重新读取共享名称表。
    fn sort_fences_with_names(snapshot: &mut WorkspaceSnapshot, names: &HashMap<String, String>) -> bool
    {
        let mut changed = false; // 本次排序是否改变权威成员顺序。
        for fence in &mut snapshot.fences
        {
            if fence.sort_mode == FenceSortMode::None
            {
                continue;
            }
            let previous = fence.members.clone(); // 检测真实顺序变化的原始成员集合。
            fence.members.sort_by(|first, second| {
                let order = names.get(first).unwrap_or(first).cmp(names.get(second).unwrap_or(second)); // 同名保留原始顺序。
                if fence.sort_mode == FenceSortMode::NameDescending { order.reverse() } else { order }
            });
            changed |= previous != fence.members;
        }
        changed
    }

    /// 返回当前不可变工作区快照。
    pub fn snapshot(&self) -> Result<WorkspaceSnapshot, WorkspaceError>
    {
        // 当前受锁保护的工作区快照。
        let snapshot = self.snapshot.lock().map_err(|_| WorkspaceError::state_unavailable())?;
        Ok(snapshot.clone())
    }

    /// 为新增组件选择当前显示器中不与已有格子重叠的默认边界。
    pub fn next_component_bounds(&self, monitor_id: &str, width: f64, height: f64) -> Result<WorkspaceRect, WorkspaceError>
    {
        let snapshot = self.snapshot()?;
        let overlaps = |candidate: WorkspaceRect| {
            let intersects = |bounds: WorkspaceRect| candidate.left < bounds.left + bounds.width && candidate.left + candidate.width > bounds.left && candidate.top < bounds.top + bounds.height && candidate.top + candidate.height > bounds.top;
            snapshot.components.iter().filter(|component| component.monitor_id == monitor_id).any(|component| intersects(component.bounds)) || snapshot.fences.iter().filter(|fence| fence.monitor_id == monitor_id).any(|fence| intersects(fence.bounds)) || snapshot.folder_portals.iter().filter(|portal| portal.monitor_id == monitor_id).any(|portal| intersects(portal.bounds))
        };
        for row in 0..100
        {
            for column in 0..100
            {
                let candidate = WorkspaceRect { left: 24.0 + f64::from(column) * 24.0, top: 24.0 + f64::from(row) * 24.0, width, height };
                if !overlaps(candidate)
                {
                    return Ok(candidate);
                }
            }
        }
        Err(WorkspaceError::invalid_command("工作区没有可用的组件位置"))
    }

    /// 将当前完整工作区写入用户指定的手动备份文件。
    pub fn write_manual_backup(&self, path: &Path) -> Result<WorkspaceBackupSummary, WorkspaceError>
    {
        validate_backup_path(path)?;
        let current = self.snapshot.lock().map_err(|_| WorkspaceError::state_unavailable())?; // 手动备份中的当前活动布局。
        let imported = self.explorer_layout_imported.lock().map_err(|_| WorkspaceError::state_unavailable())?; // 与布局共同保存的首次导入状态。
        let store = build_workspace_store(&current, *imported).map_err(|error| WorkspaceError::persistence_failed(&error))?; // 等待原子写入的完整备份文件。
        save_json_atomic(path, &store).map_err(|error| {
            crate::log_error!("工作区：手动备份写入失败，路径={}，错误={error}", path.display());
            WorkspaceError::persistence_failed(&error)
        })?;
        crate::log_info!("工作区：手动备份已写入，路径={}", path.display());
        Ok(workspace_backup_summary(path, &store))
    }

    /// 读取并校验当前格式的手动备份或自动快照。
    pub fn inspect_backup(&self, path: &Path) -> Result<WorkspaceBackupSummary, WorkspaceError>
    {
        validate_backup_path(path)?;
        let store = load_workspace_backup(path)?; // 只读临时副本中的已校验当前格式文件。
        crate::log_info!("工作区：备份校验完成，Schema={}，路径={}", store.store_version, path.display());
        Ok(workspace_backup_summary(path, &store))
    }

    /// 返回当前仍保留且通过完整性校验的全部自动快照。
    pub fn auto_snapshot_summaries(&self) -> Result<Vec<WorkspaceBackupSummary>, WorkspaceError>
    {
        let entries = match fs::read_dir(&self.auto_snapshot_directory)
        {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(WorkspaceError::restore_failed(&error)),
        }; // 当前自动快照目录中的文件集合。
        let mut summaries = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("json"))
            .filter_map(|path| match load_workspace_backup(&path)
            {
                Ok(store) => Some(workspace_backup_summary(&path, &store)),
                Err(error) =>
                {
                    crate::log_warn!("工作区：自动快照已损坏并跳过，路径={}，错误={}", path.display(), error.message);
                    None
                }
            })
            .collect::<Vec<_>>(); // 通过当前格式与完整性校验的自动快照摘要。
        summaries.sort_by(|first, second| second.created_at_unix.cmp(&first.created_at_unix).then_with(|| second.path.cmp(&first.path)));
        Ok(summaries)
    }

    /// 保存还原前恢复点，再原子替换统一工作区。
    pub(crate) fn restore_backup(&self, path: &Path, monitors: &[DesktopHostMonitor], item_ids: &[String]) -> Result<WorkspaceRestoreResult, WorkspaceError>
    {
        if monitors.is_empty()
        {
            return Err(WorkspaceError::invalid_command("工作区缺少显示器"));
        }
        let store = load_workspace_backup(path)?; // 通过格式和完整性校验的备份。
        let mut candidate = store.payload.layout; // 保留备份的完整布局。
        let grids = self.monitor_grids(monitors)?; // 当前逐屏网格。
        let primary = monitors.iter().find(|monitor| monitor.primary).unwrap_or(&monitors[0]); // 新增引用进入主显示器。
        let grid = grids.iter().find(|entry| entry.monitor_id == primary.id).expect("主显示器网格已创建").grid; // 主显示器网格。
        let mut restored_ids = item_ids.to_vec(); // 实际项目及备份中暂时不可解析的引用。
        for item in &candidate.unclassified_items
        {
            if !restored_ids.contains(&item.stable_reference)
            {
                restored_ids.push(item.stable_reference.clone());
            }
        }
        candidate.interaction = WorkspaceInteractionSnapshot::default();
        for member in candidate.fences.iter().flat_map(|fence| &fence.members)
        {
            if !restored_ids.contains(member)
            {
                restored_ids.push(member.clone());
            }
        } // 备份成员身份不依赖未归类记录。
        reconcile_layout_snapshot(&mut candidate, &grids, grid, monitors, &restored_ids, &[], &[], &primary.id)?;
        self.sort_fences(&mut candidate)?;
        let mut current = self.snapshot.lock().map_err(|_| WorkspaceError::state_unavailable())?; // 保存前保持现有布局。
        let mut imported = self.explorer_layout_imported.lock().map_err(|_| WorkspaceError::state_unavailable())?; // 共同提交的导入状态。
        let mut history = self.history.lock().map_err(|_| WorkspaceError::state_unavailable())?; // 保存前确认历史可以重置。
        candidate.revision = candidate.revision.max(current.revision).saturating_add(1);
        candidate.validate()?;
        let before = build_workspace_store(&current, *imported).map_err(|error| WorkspaceError::persistence_failed(&error))?; // 还原前恢复点。
        write_auto_snapshot(&self.auto_snapshot_directory, &before).map_err(|error| WorkspaceError::persistence_failed(&error))?;
        let committed = save_workspace_store(&self.persistence_path, &candidate, store.payload.explorer_layout_imported).map_err(|error| WorkspaceError::persistence_failed(&error))?; // 保存失败不更新内存。
        *current = candidate.clone();
        *imported = store.payload.explorer_layout_imported;
        *history = WorkspaceHistory::default();
        self.schedule_auto_snapshot(committed);
        crate::log_info!("工作区：统一备份已还原，路径={}", path.display());
        Ok(WorkspaceRestoreResult { snapshot: candidate, message: "工作区还原完成".to_string() })
    }

    /// 将宿主网格与 Shell 完整项目集合原子协调进 Workspace。
    #[cfg(test)]
    pub fn reconcile_items(&self, workspace_width: f64, workspace_height: f64, item_ids: &[String]) -> Result<WorkspaceSnapshot, WorkspaceError> { self.reconcile_shell_items(workspace_width, workspace_height, item_ids, &[]) }

    /// 将精确重命名事实和最终 Shell 项目集合协调进当前布局。
    #[cfg(test)]
    pub fn reconcile_shell_items(&self, workspace_width: f64, workspace_height: f64, item_ids: &[String], renames: &[(String, String)]) -> Result<WorkspaceSnapshot, WorkspaceError>
    {
        let monitor = DesktopHostMonitor { id: "primary".to_string(), name: "primary".to_string(), primary: true, x: 0, y: 0, width: workspace_width.ceil().max(1.0) as u32, height: workspace_height.ceil().max(1.0) as u32, work_x: 0, work_y: 0, work_width: workspace_width.ceil().max(1.0) as u32, work_height: workspace_height.ceil().max(1.0) as u32, scale_factor: 1.0, dpi: 96 }; // 单显示器兼容调用使用的合成宿主状态。
        self.reconcile_host_monitors(&[monitor], item_ids, &[], renames)
    }

    /// 将全部显示器网格、项目归属、临时投射和 Shell 最终事实原子协调进 Workspace。
    #[cfg(test)]
    pub fn reconcile_host_monitors(&self, monitors: &[DesktopHostMonitor], item_ids: &[String], assignments: &[(String, String)], renames: &[(String, String)]) -> Result<WorkspaceSnapshot, WorkspaceError> { self.reconcile_environment_state(monitors, item_ids, assignments, renames, None, false, &[]) }

    /// 将同次环境观察的名称与布局作为一次协调提交。
    pub(crate) fn reconcile_environment(&self, environment: WorkspaceEnvironment<'_>) -> Result<WorkspaceSnapshot, WorkspaceError>
    {
        let ids = environment.items.iter().map(|item| item.stable_reference.clone()).collect::<Vec<_>>(); // 完整项目引用。
        let names = environment.items.iter().map(|item| (item.stable_reference.clone(), item.display_name.to_lowercase())).collect(); // 本次排序事实。
        self.reconcile_environment_state(environment.monitors, &ids, environment.assignments, environment.renames, Some(names), environment.transfer_assignments, environment.new_item_positions)
    }

    /// 在既有锁顺序下协调布局，持久化成功后才更新名称缓存。
    #[allow(clippy::too_many_arguments)]
    fn reconcile_environment_state(&self, monitors: &[DesktopHostMonitor], item_ids: &[String], assignments: &[(String, String)], renames: &[(String, String)], observed_names: Option<HashMap<String, String>>, transfer_assignments: bool, new_item_positions: &[(String, String, WorkspacePoint)]) -> Result<WorkspaceSnapshot, WorkspaceError>
    {
        if monitors.is_empty()
        {
            return Err(WorkspaceError::invalid_command("工作区缺少显示器"));
        }
        let monitor_grids = self.monitor_grids(monitors)?; // 当前全部显示器对应的独立 DIP 网格。
        let primary_monitor_id = monitors.iter().find(|monitor| monitor.primary).unwrap_or(&monitors[0]).id.clone(); // 临时投射使用的主显示器身份。
        let primary_grid = monitor_grids.iter().find(|entry| entry.monitor_id == primary_monitor_id).expect("主显示器网格已创建").grid; // 默认投放使用的主显示器网格。
        // 等待项目协调的当前活动工作区快照。
        let mut current = self.snapshot.lock().map_err(|_| WorkspaceError::state_unavailable())?;
        let mut imported = self.explorer_layout_imported.lock().map_err(|_| WorkspaceError::state_unavailable())?; // 与布局共同保存的首次导入状态。
        let mut histories = self.history.lock().map_err(|_| WorkspaceError::state_unavailable())?; // 等待同步 Shell 重命名引用的全部布局历史。
        let mut names = self.item_names.lock().map_err(|_| WorkspaceError::state_unavailable())?; // 与布局同时提交的名称表。
        let next_names = observed_names.unwrap_or_else(|| names.clone()); // 测试兼容入口沿用现有名称。
        // 保持当前快照可回滚的项目协调副本。
        let mut next = current.clone();
        let mut next_imported = *imported; // 持久化前不更新首次导入状态。
        let mut next_histories = histories.clone(); // 保存失败时保持会话历史引用不变的候选副本。
        let new_assignments = new_item_positions.iter().map(|(id, monitor, _)| (id.clone(), monitor.clone())).collect::<Vec<_>>(); // 新项目直接在目标屏分配，避免先占用主屏容量。
        let current_assignments = if next_imported && !transfer_assignments { &new_assignments[..] } else { assignments }; // 已有持久归属不重新导入。
        let mut changed = reconcile_layout_snapshot(&mut next, &monitor_grids, primary_grid, monitors, item_ids, current_assignments, renames, &primary_monitor_id)?; // 统一工作区的 Shell 与显示器协调结果。
        for (item_id, monitor_id, desired) in new_item_positions
        {
            if current.contains_item(item_id) || renames.iter().any(|(_, new_id)| new_id == item_id)
            {
                continue;
            }
            let Some(index) = next.unclassified_items.iter().position(|item| &item.stable_reference == item_id)
            else
            {
                continue;
            }; // 不将已归属围栏的身份移动到桌面。
            let mut item = next.unclassified_items.remove(index); // 自身临时槽位不阻挡最终落位。
            item.position = next.nearest_unclassified_position(monitor_id, *desired)?;
            item.monitor_id.clone_from(monitor_id);
            item.projection_monitor_id = None;
            item.projection_position = None;
            crate::log_info!("工作区：新增项目落位，项目={item_id:?}，显示器={monitor_id}，Shell位置=({},{})，网格位置=({},{})", desired.left, desired.top, item.position.left, item.position.top);
            next.unclassified_items.insert(index, item);
            changed = true;
        }
        next.validate()?;
        if Self::sort_fences_with_names(&mut next, &next_names)
        {
            next.revision += 1;
            changed = true;
        }
        if !next_imported
        {
            next_imported = true;
            changed = true;
        }
        for entry in next_histories.undo.iter_mut().chain(next_histories.redo.iter_mut())
        {
            entry.layout.remap_item_references(renames)?;
            entry.layout.validate()?;
        } // 精确重命名同步到同一撤销和重做历史。
        if !changed
        {
            *histories = next_histories;
            *names = next_names;
            return Ok(current.clone());
        }
        let store = save_workspace_store(&self.persistence_path, &next, next_imported).map_err(|error| {
            crate::log_error!("工作区：桌面项目协调保存失败，错误={error}");
            WorkspaceError::persistence_failed(&error)
        })?; // Shell 与显示器协调后的完整持久文件。
        *current = next.clone();
        *imported = next_imported;
        *histories = next_histories;
        *names = next_names;
        self.schedule_auto_snapshot(store);
        crate::log_debug!("工作区：逐屏项目位置已协调，修订={}，项目={}，显示器={}", next.revision, next.unclassified_items.len(), next.monitor_grids.len());
        Ok(next)
    }

    /// 校验并原子提交一个完整布局事务。
    #[cfg(test)]
    pub fn apply(&self, command: WorkspaceCommand) -> Result<WorkspaceSnapshot, WorkspaceError> { self.apply_on_monitor(None, command) }

    /// 在指定逐屏宿主上下文中提交一个完整布局事务。
    pub fn apply_on_monitor(&self, monitor_id: Option<&str>, command: WorkspaceCommand) -> Result<WorkspaceSnapshot, WorkspaceError> { self.apply_on_monitor_at_revision(monitor_id, command, None) }

    /// 原生释放在持有领域锁后校验渲染版本，防止校验与提交之间的并发覆盖。
    pub(crate) fn apply_on_monitor_at_revision(&self, monitor_id: Option<&str>, command: WorkspaceCommand, expected_revision: Option<u64>) -> Result<WorkspaceSnapshot, WorkspaceError>
    {
        if expected_revision.is_some() && matches!(&command, WorkspaceCommand::UndoLayout | WorkspaceCommand::RedoLayout)
        {
            return Err(WorkspaceError::invalid_command("版本化投放不能提交历史命令"));
        }
        if matches!(&command, WorkspaceCommand::UndoLayout)
        {
            return self.restore_layout_history(false).map(|result| result.snapshot);
        }
        if matches!(&command, WorkspaceCommand::RedoLayout)
        {
            return self.restore_layout_history(true).map(|result| result.snapshot);
        }
        // 围栏滚动由拖动事务或当前滚动位置直接提交，不形成独立撤销单元。
        let records_history = !matches!(&command, WorkspaceCommand::SetFolderPortalAppearance { .. } | WorkspaceCommand::SetFenceScroll { .. } | WorkspaceCommand::UpdateComponentState { .. } | WorkspaceCommand::SetTileAppearance { .. } | WorkspaceCommand::RenameFence { .. } | WorkspaceCommand::SetFenceAutoExpand { .. } | WorkspaceCommand::SetFenceCollapsed { .. });
        // 等待事务提交的当前工作区快照。
        let mut current = self.snapshot.lock().map_err(|_| WorkspaceError::state_unavailable())?;
        if expected_revision.is_some_and(|revision| revision != current.revision)
        {
            return Err(WorkspaceError::invalid_command("投放布局版本已经失效"));
        }
        let imported = self.explorer_layout_imported.lock().map_err(|_| WorkspaceError::state_unavailable())?; // 与布局共同保存的首次导入状态。
        // 保持当前快照可回滚的事务副本。
        let mut next = current.clone();
        // 布局事务前的运行期交互状态。
        let interaction = next.interaction.clone();
        // 当前命令用于中文日志的稳定名称。
        let command = match command
        {
            WorkspaceCommand::RenameFence { fence_id, title } => WorkspaceCommand::SetTileAppearance { target: WorkspaceFocusTarget::Fence { fence_id }, change: crate::contracts::TileAppearanceChange::Title(title) },
            WorkspaceCommand::SetFenceAutoExpand { fence_id, enabled } => WorkspaceCommand::SetTileAppearance { target: WorkspaceFocusTarget::Fence { fence_id }, change: crate::contracts::TileAppearanceChange::AutoExpand(enabled) },
            WorkspaceCommand::SetFenceCollapsed { fence_id, collapsed } => WorkspaceCommand::SetTileAppearance { target: WorkspaceFocusTarget::Fence { fence_id }, change: crate::contracts::TileAppearanceChange::Collapsed(collapsed) },
            command => command,
        }; // 所有公共外观入口使用同一校验和共享保存边界。
        let command_name = next.apply(command, monitor_id)?;
        self.sort_fences(&mut next)?;
        if next == *current
        {
            crate::log_debug!("工作区：布局事务保持当前状态，命令={command_name}");
            return Ok(current.clone());
        }
        if next.interaction != interaction
        {
            next.interaction.revision += 1;
        }
        next.revision += 1;
        next.validate()?;
        let mut histories = self.history.lock().map_err(|_| WorkspaceError::state_unavailable())?; // 原子保存前确认会话历史可以同步提交。
        let store = save_workspace_store(&self.persistence_path, &next, *imported).map_err(|error| {
            crate::log_error!("工作区：原子快照保存失败，命令={command_name}，错误={error}");
            WorkspaceError::persistence_failed(&error)
        })?; // 当前布局事务完成后的完整工作区文件。
        // 保存成功后加入统一撤销时间线。
        if records_history
        {
            histories.undo.push(WorkspaceHistoryEntry { layout: layout_snapshot(&current), timeline_at_unix_micros: crate::tools::time::next_timeline_timestamp() });
            histories.redo.clear();
        }
        *current = next.clone();
        self.schedule_auto_snapshot(store);
        crate::log_debug!("工作区：布局事务已提交，命令={command_name}，修订={}，围栏={}", next.revision, next.fences.len());
        Ok(next)
    }

    /// 从当前会话布局历史撤销或重做一个完整事务。
    pub(crate) fn restore_layout_history(&self, redo: bool) -> Result<WorkspaceLayoutHistoryResult, WorkspaceError>
    {
        // 等待历史恢复的当前权威工作区快照。
        let mut current = self.snapshot.lock().map_err(|_| WorkspaceError::state_unavailable())?;
        let imported = self.explorer_layout_imported.lock().map_err(|_| WorkspaceError::state_unavailable())?; // 与布局共同保存的首次导入状态。
        let mut history = self.history.lock().map_err(|_| WorkspaceError::state_unavailable())?; // 当前统一会话历史。
        let target = if redo { history.redo.last().cloned() } else { history.undo.last().cloned() }; // 当前撤销方向对应的最近历史布局。
        let Some(entry) = target
        else
        {
            crate::log_debug!("工作区：布局历史保持当前状态，方向={}", if redo { "重做" } else { "撤销" });
            return Ok(WorkspaceLayoutHistoryResult { snapshot: current.clone(), restored_count: 0, changed_count: 0 });
        };

        let mut target = entry.layout.clone(); // 历史布局仍按当前文件事实过滤。
        target.tile_appearances.clone_from(&current.tile_appearances);
        // 布局历史不回滚组件内容在拖动之后保存的私有状态。
        for component in &mut target.components
        {
            if let Some(active) = current.components.iter().find(|active| active.instance_id == component.instance_id)
            {
                component.state.clone_from(&active.state);
                component.state_version = active.state_version;
            }
        }
        appearance::project(&mut target);
        for portal in &mut target.folder_portals
        {
            if let Some(active) = current.folder_portals.iter().find(|active| active.id == portal.id)
            {
                portal.appearance = active.appearance.clone();
            }
        }
        reflow_grid_items(&mut target, &current.monitor_grids)?;
        let (mut next, restored_count, changed_count) = merge_layout_history_target(target, &current); // 保留外部新增项目并排除已离开工作区的历史引用。
        // 撤销和重做期间需要保留的当前围栏滚动位置。
        let current_scrolls = current.fences.iter().map(|fence| (fence.id.as_str(), fence.scroll_top)).collect::<std::collections::HashMap<_, _>>();
        for fence in &mut next.fences
        {
            if let Some(scroll_top) = current_scrolls.get(fence.id.as_str())
            {
                fence.scroll_top = *scroll_top;
            }
        }
        next.revision = current.revision.saturating_add(1);
        next.interaction = current.interaction.clone();
        self.sort_fences(&mut next)?;
        next.normalize_interaction();
        if next.interaction != current.interaction
        {
            next.interaction.revision = current.interaction.revision.saturating_add(1);
        }
        next.validate()?;
        let store = save_workspace_store(&self.persistence_path, &next, *imported).map_err(|error| {
            crate::log_error!("工作区：布局历史保存失败，错误={error}");
            WorkspaceError::persistence_failed(&error)
        })?; // 当前历史恢复后的完整工作区文件。

        // 历史恢复成功后移动对应撤销单元。
        let current_layout = WorkspaceHistoryEntry { layout: layout_snapshot(&current), timeline_at_unix_micros: crate::tools::time::next_timeline_timestamp() }; // 当前方向完成后进入相反方向的布局历史单元。
        if redo
        {
            history.redo.pop();
            history.undo.push(current_layout);
        }
        else
        {
            history.undo.pop();
            history.redo.push(current_layout);
        }
        *current = next.clone();
        self.schedule_auto_snapshot(store);
        crate::log_debug!("工作区：布局历史已恢复，方向={}，修订={}", if redo { "重做" } else { "撤销" }, next.revision);
        Ok(WorkspaceLayoutHistoryResult { snapshot: next, restored_count, changed_count })
    }

    /// 返回统一布局历史在指定方向的最近完成时间。
    pub(crate) fn latest_layout_history_time(&self, redo: bool) -> Result<Option<i64>, WorkspaceError>
    {
        let history = self.history.lock().map_err(|_| WorkspaceError::state_unavailable())?; // 当前会话唯一历史。
        Ok(if redo { history.redo.last() } else { history.undo.last() }.map(|entry| entry.timeline_at_unix_micros))
    }

    /// 新 Shell 操作提交后清除布局重做分支。
    pub(crate) fn clear_redo_history(&self) -> Result<(), WorkspaceError>
    {
        self.history.lock().map_err(|_| WorkspaceError::state_unavailable())?.redo.clear();
        Ok(())
    }

    /// 将最新完整工作区交给空闲合并线程。
    fn schedule_auto_snapshot(&self, store: PersistedWorkspaceStore)
    {
        if self.auto_snapshot_sender.as_ref().expect("自动快照发送端在工作区生命周期内存在").send(store).is_err()
        {
            crate::log_warn!("工作区：自动快照线程已经停止");
        }
    }

    /// 校验并提交一个不写入布局文件的运行期交互事务。
    #[cfg(test)]
    pub fn apply_interaction(&self, command: WorkspaceInteractionCommand) -> Result<WorkspaceSnapshot, WorkspaceError>
    {
        let monitor_id = self.snapshot()?.monitor_grids.first().map(|entry| entry.monitor_id.clone()).ok_or_else(|| WorkspaceError::invalid_command("工作区显示器尚未初始化"))?;
        self.apply_interaction_on_monitor(&monitor_id, command)
    }

    /// 在指定逐屏宿主上下文中提交一个运行期交互事务。
    pub fn apply_interaction_on_monitor(&self, monitor_id: &str, command: WorkspaceInteractionCommand) -> Result<WorkspaceSnapshot, WorkspaceError>
    {
        // 等待交互事务提交的当前工作区快照。
        let mut current = self.snapshot.lock().map_err(|_| WorkspaceError::state_unavailable())?;
        // 保持当前交互状态可回滚的事务副本。
        let mut next = current.clone();
        next.command_monitor_id(Some(monitor_id))?;
        // 当前交互命令用于中文日志的稳定名称。
        let command_name = next.apply_interaction(command, monitor_id)?;
        if next == *current
        {
            return Ok(current.clone());
        }

        next.interaction.revision += 1;
        next.validate()?;
        *current = next.clone();
        crate::log_debug!("工作区：交互事务已提交，命令={command_name}，交互修订={}，选择数量={}", next.interaction.revision, next.interaction.selected_item_ids.len());
        Ok(next)
    }
}

impl Drop for WorkspaceState
{
    /// 关闭发送端并等待后台线程刷新退出前待处理快照。
    /// 释放适配器持有的原生资源。
    fn drop(&mut self)
    {
        self.auto_snapshot_sender.take();
        if let Some(worker) = self.auto_snapshot_worker.take()
        {
            let _ = worker.join();
        }
    }
}

/// 启动统一工作区自动快照线程，空闲或退出时刷新最后一次提交。
fn start_auto_snapshot_worker(directory: PathBuf) -> std::io::Result<(Sender<PersistedWorkspaceStore>, JoinHandle<()>)>
{
    let (sender, receiver) = mpsc::channel::<PersistedWorkspaceStore>(); // 布局提交到磁盘快照的通道。
    let worker = thread::Builder::new().name("desktop-tools-workspace-snapshot".to_string()).spawn(move || {
        let mut pending = None; // 空闲窗口内最后一份完整布局。
        loop
        {
            let request = if pending.is_some()
            {
                receiver.recv_timeout(AUTO_SNAPSHOT_IDLE)
            }
            else
            {
                receiver.recv().map_err(|_| RecvTimeoutError::Disconnected)
            }; // 每次提交重新开始空闲计时。
            match request
            {
                Ok(store) => pending = Some(store),
                Err(error) =>
                {
                    if let Some(store) = pending.take()
                        && let Err(error) = write_auto_snapshot(&directory, &store)
                    {
                        crate::log_error!("工作区：自动快照写入失败，错误={error}");
                    }
                    if error == RecvTimeoutError::Disconnected
                    {
                        break;
                    }
                }
            }
        }
    })?;
    Ok((sender, worker))
}

/// 原子写入一份完整工作区自动快照并保留当前桌面最近 20 份。
fn write_auto_snapshot(directory: &Path, store: &PersistedWorkspaceStore) -> std::io::Result<PathBuf>
{
    fs::create_dir_all(directory)?;
    let prefix = "workspace-"; // 统一快照集合的文件前缀。
    let timestamp = Utc::now().timestamp_micros(); // 自动快照按创建时间排序的 Unix 微秒值。
    let sequence = NEXT_AUTO_SNAPSHOT_ID.fetch_add(1, Ordering::Relaxed); // 同一时钟刻度内避免覆盖的进程内序号。
    let path = directory.join(format!("{prefix}{timestamp}-{sequence}.json")); // 当前完整工作区自动快照路径。
    save_json_atomic(&path, store)?;
    let mut snapshots = fs::read_dir(directory)?.filter_map(Result::ok).map(|entry| entry.path()).filter(|path| path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with(prefix) && name.ends_with(".json"))).collect::<Vec<_>>(); // 统一工作区全部自动快照文件。
    snapshots.sort();
    let remove_count = snapshots.len().saturating_sub(AUTO_SNAPSHOT_LIMIT); // 超出最近 20 份配额的旧文件数量。
    for obsolete in snapshots.into_iter().take(remove_count)
    {
        fs::remove_file(obsolete)?;
    }
    crate::log_debug!("工作区：自动快照已写入，路径={}", path.display());
    Ok(path)
}

/// 只读当前格式备份并完成结构与完整性校验。
fn load_workspace_backup(path: &Path) -> Result<PersistedWorkspaceStore, WorkspaceError>
{
    validate_backup_path(path)?;
    let value = load_json::<serde_json::Value>(path)
        .map_err(|error| {
            crate::log_warn!("工作区：备份读取失败，路径={}，错误={error}", path.display());
            WorkspaceError::restore_failed(&error)
        })?
        .ok_or_else(|| {
            crate::log_warn!("工作区：备份文件不存在，路径={}", path.display());
            WorkspaceError::restore_failed(&"工作区备份文件不存在")
        })?; // 等待格式识别的只读 JSON。
    decode_workspace_store(value).inspect_err(|error| {
        crate::log_warn!("工作区：备份校验失败，路径={}，错误={}", path.display(), error.message);
    })
}

/// 生成命令边界使用的已校验备份摘要。
fn workspace_backup_summary(path: &Path, store: &PersistedWorkspaceStore) -> WorkspaceBackupSummary { WorkspaceBackupSummary { schema_version: store.store_version, created_at_unix: store.created_at_unix, path: path.to_string_lossy().into_owned(), checksum: store.checksum.clone() } }

/// 校验用户传入的备份路径可以明确指向 JSON 文件。
fn validate_backup_path(path: &Path) -> Result<(), WorkspaceError>
{
    if path.as_os_str().is_empty() || path.file_name().is_none() || !path.extension().and_then(|extension| extension.to_str()).is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
    {
        return Err(WorkspaceError::invalid_command("工作区备份路径必须指向 .json 文件"));
    }
    Ok(())
}

/// 从正式宿主显示器状态生成逐屏工作区网格。
fn monitor_grids(monitors: &[DesktopHostMonitor], preferences: GridPreferences) -> Result<Vec<WorkspaceMonitorGrid>, WorkspaceError> { monitors.iter().map(|monitor| Ok(WorkspaceMonitorGrid { monitor_id: monitor.id.clone(), grid: workspace_grid(f64::from(monitor.work_width) / monitor.scale_factor, f64::from(monitor.work_height) / monitor.scale_factor, preferences)? })).collect() }

/// 网格步进改变时按原先纵向顺序重排；不触及断开显示器或围栏内的来源记录。
fn reflow_grid_items(layout: &mut WorkspaceSnapshot, grids: &[WorkspaceMonitorGrid]) -> Result<(), WorkspaceError>
{
    let members = layout.fences.iter().flat_map(|fence| fence.members.iter().cloned()).collect::<std::collections::HashSet<_>>(); // 围栏成员不占桌面格位。
    for entry in grids
    {
        let previous = layout.monitor_grids.iter().find(|old| old.monitor_id == entry.monitor_id).map(|old| old.grid); // 此显示器上次使用的步进。
        if previous.is_none_or(|old| old.column_width == entry.grid.column_width && old.row_height == entry.grid.row_height)
        {
            continue;
        }
        let mut items = layout.unclassified_items.iter_mut().filter(|item| item.monitor_id == entry.monitor_id && !members.contains(&item.stable_reference)).collect::<Vec<_>>(); // 本屏可见桌面项目。
        if items.len() > grid_capacity(entry.grid)? as usize
        {
            return Err(WorkspaceError::invalid_command("新网格空间不足，请减小格子尺寸或间距"));
        }
        items.sort_by(|a, b| a.position.left.total_cmp(&b.position.left).then(a.position.top.total_cmp(&b.position.top)));
        for (index, item) in items.into_iter().enumerate()
        {
            item.position = grid_position(entry.grid, index as u32);
            item.projection_position = None;
            item.projection_monitor_id = None;
        }
    }
    Ok(())
}

/// 将一次 Shell 与显示器环境变化协调到统一工作区布局。
#[allow(clippy::too_many_arguments)]
fn reconcile_layout_snapshot(layout: &mut WorkspaceSnapshot, monitor_grids: &[WorkspaceMonitorGrid], primary_grid: WorkspaceGrid, monitors: &[DesktopHostMonitor], item_ids: &[String], assignments: &[(String, String)], renames: &[(String, String)], primary_monitor_id: &str) -> Result<bool, WorkspaceError>
{
    let previous = layout.clone(); // 本次协调前用于判断是否产生状态变化的布局。
    let interaction = layout.interaction.clone(); // 协调前当前桌面运行期选择与焦点。
    reflow_grid_items(layout, monitor_grids)?;
    layout.grid = primary_grid;
    layout.monitor_grids = monitor_grids.to_vec();
    layout.remap_item_references(renames)?;
    layout.reconcile_items_for_monitors(item_ids, assignments, primary_monitor_id)?;
    layout.reconcile_monitor_projections(monitors, primary_monitor_id)?;
    if *layout == previous
    {
        return Ok(false);
    }
    if layout.interaction != interaction
    {
        layout.interaction.revision = layout.interaction.revision.saturating_add(1);
    }
    layout.revision = layout.revision.saturating_add(1);
    layout.validate()?;
    Ok(true)
}

/// 只读取当前统一格式，不迁移旧布局。
fn decode_workspace_store(value: serde_json::Value) -> Result<PersistedWorkspaceStore, WorkspaceError>
{
    if value.get("storeVersion").and_then(serde_json::Value::as_u64) != Some(u64::from(WORKSPACE_STORE_VERSION))
    {
        return Err(WorkspaceError::restore_failed(&"工作区格式不受支持，仅接受当前统一工作区格式"));
    }
    let store: PersistedWorkspaceStore = serde_json::from_value(value).map_err(|error| WorkspaceError::restore_failed(&error))?; // 当前结构必须完整。
    validate_workspace_store(&store)?;
    Ok(store)
}

/// 校验当前版本、完整性以及唯一布局中的身份和外观。
fn validate_workspace_store(store: &PersistedWorkspaceStore) -> Result<(), WorkspaceError>
{
    if store.store_version != WORKSPACE_STORE_VERSION
    {
        return Err(WorkspaceError::restore_failed(&"工作区备份 Schema 版本无效"));
    }
    if workspace_store_checksum(store.store_version, store.created_at_unix, &store.payload)? != store.checksum
    {
        return Err(WorkspaceError::restore_failed(&"工作区备份完整性校验失败"));
    }
    for (key, appearance) in &store.payload.layout.tile_appearances
    {
        validate_id(key, "格子外观身份无效")?;
        if !key.starts_with("fence:") && !key.starts_with("component:")
        {
            return Err(WorkspaceError::invalid_command("格子外观身份类型无效"));
        }
        if let Some(title) = &appearance.title
        {
            validate_title(title)?;
        }
    }
    store.payload.layout.validate().map_err(|error| WorkspaceError::restore_failed(&error.message))
}

/// 保存排除运行期选择与焦点的统一布局。
fn save_workspace_store(path: &Path, snapshot: &WorkspaceSnapshot, explorer_layout_imported: bool) -> std::io::Result<PersistedWorkspaceStore>
{
    let store = build_workspace_store(snapshot, explorer_layout_imported)?; // 完整性字段计算后原子落盘。
    save_json_atomic(path, &store)?;
    Ok(store)
}

/// 构造不含系统桌面身份的唯一工作区文件。
fn build_workspace_store(snapshot: &WorkspaceSnapshot, explorer_layout_imported: bool) -> std::io::Result<PersistedWorkspaceStore> { workspace_store_from_payload(PersistedWorkspacePayload { layout: persisted_layout(snapshot), explorer_layout_imported }).map_err(|error| std::io::Error::other(error.message)) }

/// 为当前格式载荷补齐版本、时间与完整性校验值。
fn workspace_store_from_payload(payload: PersistedWorkspacePayload) -> Result<PersistedWorkspaceStore, WorkspaceError>
{
    let created_at_unix = Utc::now().timestamp(); // 当前工作区文件创建 Unix 时间戳。
    let checksum = workspace_store_checksum(WORKSPACE_STORE_VERSION, created_at_unix, &payload)?; // 排除校验字段自身的规范化文件校验值。
    Ok(PersistedWorkspaceStore { store_version: WORKSPACE_STORE_VERSION, created_at_unix, checksum, payload })
}

/// 使用固定 FNV-1a 64 位算法计算版本、时间和 UTF-8 JSON 载荷完整性值。
fn workspace_store_checksum(store_version: u32, created_at_unix: i64, payload: &PersistedWorkspacePayload) -> Result<String, WorkspaceError>
{
    let bytes = serde_json::to_vec(&(store_version, created_at_unix, payload)).map_err(|error| WorkspaceError::restore_failed(&error))?; // serde 字段顺序稳定的规范化文件内容。
    let mut checksum = 0xcbf29ce484222325_u64; // FNV-1a 64 位偏移基数。
    for byte in bytes
    {
        checksum ^= u64::from(byte);
        checksum = checksum.wrapping_mul(0x100000001b3);
    }
    Ok(format!("fnv1a64:{checksum:016x}"))
}

/// 创建排除运行期选择与焦点的持久布局副本。
fn persisted_layout(snapshot: &WorkspaceSnapshot) -> WorkspaceSnapshot
{
    let mut persisted = snapshot.clone(); // 写入磁盘前清理运行期状态的布局副本。
    persisted.interaction = WorkspaceInteractionSnapshot::default();
    persisted
}

/// 创建只包含可恢复布局字段的会话历史快照。
fn layout_snapshot(snapshot: &WorkspaceSnapshot) -> WorkspaceSnapshot
{
    // 排除运行期选择与焦点的布局历史副本。
    let mut layout = snapshot.clone();
    layout.interaction = WorkspaceInteractionSnapshot::default();
    layout
}

/// 将历史布局应用到当前项目集合，并保留历史提交后出现的外部项目。
fn merge_layout_history_target(mut target: WorkspaceSnapshot, current: &WorkspaceSnapshot) -> (WorkspaceSnapshot, usize, usize)
{
    let current_items = current.unclassified_items.iter().map(|item| (item.stable_reference.as_str(), item)).collect::<std::collections::HashMap<_, _>>(); // 当前仍在工作区中的完整项目集合。
    let current_ids = current_items.keys().copied().chain(current.fences.iter().flat_map(|fence| fence.members.iter().map(String::as_str))).collect::<std::collections::HashSet<_>>(); // 成员身份独立于桌面位置。
    let target_ids = target.unclassified_items.iter().map(|item| item.stable_reference.clone()).chain(target.fences.iter().flat_map(|fence| fence.members.iter().cloned())).collect::<std::collections::HashSet<_>>(); // 历史事务原本覆盖的项目引用。
    let restored_count = target_ids.iter().filter(|item_id| current_ids.contains(item_id.as_str())).count(); // 当前仍存在并可以恢复历史布局的项目数量。
    let changed_count = target_ids.len().saturating_sub(restored_count); // 已离开当前工作区的历史项目数量。
    target.unclassified_items.retain(|item| current_ids.contains(item.stable_reference.as_str()));
    for item in &mut target.unclassified_items
    {
        if let Some(current_item) = current_items.get(item.stable_reference.as_str())
        {
            item.projection_monitor_id.clone_from(&current_item.projection_monitor_id);
            item.projection_position = current_item.projection_position;
        }
    }
    for fence in &mut target.fences
    {
        fence.members.retain(|item_id| current_ids.contains(item_id.as_str()));
        if let Some(current_fence) = current.fences.iter().find(|current| current.id == fence.id)
        {
            fence.projection_monitor_id.clone_from(&current_fence.projection_monitor_id);
            fence.projection_bounds = current_fence.projection_bounds;
        }
    }
    target.grid = current.grid;
    target.monitor_grids.clone_from(&current.monitor_grids);
    for mut item in current.unclassified_items.iter().filter(|item| !target_ids.contains(&item.stable_reference)).cloned()
    {
        if target.unclassified_items.iter().any(|restored| restored.monitor_id == item.monitor_id && restored.position == item.position)
            && let Ok(position) = target.nearest_unclassified_position(&item.monitor_id, item.position)
        {
            item.position = position;
        } // 撤销恢复原位；同期新增项目仍保留，只对实际冲突寻找最近空格。
        target.unclassified_items.push(item);
    }
    (target, restored_count, changed_count)
}

/// 创建包含当前官方默认组件集合的空工作区布局。
fn default_workspace_snapshot(default_components: &[ComponentInstanceSnapshot]) -> WorkspaceSnapshot { WorkspaceSnapshot { components: default_components.to_vec(), ..WorkspaceSnapshot::default() } }

/// 创建首次启动的空工作区快照。
impl Default for WorkspaceSnapshot
{
    /// 创建当前版本的空工作区布局。
    fn default() -> Self { Self { folder_portals: Vec::new(), tile_appearances: Default::default(), schema_version: WORKSPACE_SCHEMA_VERSION, revision: 0, grid: WorkspaceGrid::default(), monitor_grids: Vec::new(), fences: Vec::new(), unclassified_items: Vec::new(), components: Vec::new(), interaction: WorkspaceInteractionSnapshot::default() } }
}

/// 实现工作区快照内部的布局事务和完整性校验。
impl WorkspaceSnapshot
{
    /// 桌面项目身份来自未归类记录或围栏成员，不依赖历史位置。
    pub(crate) fn contains_item(&self, item_id: &str) -> bool { self.unclassified_items.iter().any(|item| item.stable_reference == item_id) || self.fences.iter().any(|fence| fence.members.iter().any(|member| member == item_id)) }

    /// 使用 Shell 精确重命名事实同步布局、选择、锚点和焦点引用。
    fn remap_item_references(&mut self, renames: &[(String, String)]) -> Result<(), WorkspaceError>
    {
        for (old_reference, new_reference) in renames
        {
            validate_id(old_reference, "重命名前项目标识无效")?;
            validate_id(new_reference, "重命名后项目标识无效")?;
            if old_reference == new_reference
            {
                continue;
            }
            // 当前布局中等待重命名的来源项目索引。
            if !self.contains_item(old_reference)
            {
                continue;
            }
            if self.contains_item(new_reference)
            {
                return Err(WorkspaceError::invalid_command("重命名后项目标识已经存在"));
            }
            if let Some(item) = self.unclassified_items.iter_mut().find(|item| item.stable_reference == *old_reference)
            {
                item.stable_reference.clone_from(new_reference);
            }
            for fence in &mut self.fences
            {
                for member in &mut fence.members
                {
                    if member == old_reference
                    {
                        member.clone_from(new_reference);
                    }
                }
            }
            for selected in &mut self.interaction.selected_item_ids
            {
                if selected == old_reference
                {
                    selected.clone_from(new_reference);
                }
            }
            if self.interaction.selection_anchor_id.as_ref() == Some(old_reference)
            {
                self.interaction.selection_anchor_id = Some(new_reference.clone());
            }
            if matches!(
                &self.interaction.keyboard_focus,
                WorkspaceFocusTarget::Item { item_id } if item_id == old_reference
            )
            {
                self.interaction.keyboard_focus = WorkspaceFocusTarget::Item { item_id: new_reference.clone() };
            }
        }
        Ok(())
    }

    /// 应用一个已反序列化的运行期交互命令。
    fn apply_interaction(&mut self, command: WorkspaceInteractionCommand, monitor_id: &str) -> Result<&'static str, WorkspaceError>
    {
        match command
        {
            WorkspaceInteractionCommand::FocusItem { item_id, selection_mode } =>
            {
                validate_id(&item_id, "桌面项目标识无效")?;
                if !self.contains_item(&item_id) || !self.item_visible_on_monitor(&item_id, monitor_id)
                {
                    return Err(WorkspaceError::invalid_command("桌面项目不存在"));
                }
                match selection_mode
                {
                    WorkspaceSelectionMode::Replace =>
                    {
                        self.interaction.selected_item_ids = vec![item_id.clone()];
                        self.interaction.selection_anchor_id = Some(item_id.clone());
                    }
                    WorkspaceSelectionMode::Toggle =>
                    {
                        if self.interaction.selected_item_ids.contains(&item_id)
                        {
                            self.interaction.selected_item_ids.retain(|selected| selected != &item_id);
                        }
                        else
                        {
                            self.interaction.selected_item_ids.push(item_id.clone());
                        }
                        self.interaction.selection_anchor_id = Some(item_id.clone());
                    }
                    WorkspaceSelectionMode::Range =>
                    {
                        // 当前范围选择开始时的已有锚点。
                        let anchor_id = self.interaction.selection_anchor_id.clone();
                        // 锚点与目标位于同一焦点容器时的连续选择。
                        let range = anchor_id.as_deref().filter(|anchor| self.same_item_container(anchor, &item_id)).and_then(|anchor| {
                            // 当前目标焦点容器的完整项目顺序。
                            let order = self.item_order(&item_id);
                            // 当前锚点在逻辑顺序中的位置。
                            let anchor_index = order.iter().position(|candidate| candidate == anchor)?;
                            // 当前目标在逻辑顺序中的位置。
                            let target_index = order.iter().position(|candidate| candidate == &item_id)?;
                            // 包含锚点与目标的双向连续范围。
                            let start = anchor_index.min(target_index);
                            let end = anchor_index.max(target_index);
                            Some(order[start..=end].to_vec())
                        });
                        if let Some(range) = range
                        {
                            self.interaction.selected_item_ids = range;
                        }
                        else
                        {
                            self.interaction.selected_item_ids = vec![item_id.clone()];
                            self.interaction.selection_anchor_id = Some(item_id.clone());
                        }
                    }
                    WorkspaceSelectionMode::Preserve =>
                    {
                        self.interaction.selection_anchor_id = Some(item_id.clone());
                    }
                }
                self.interaction.keyboard_focus = WorkspaceFocusTarget::Item { item_id };
                Ok("选择桌面项目")
            }
            WorkspaceInteractionCommand::SelectItems { item_ids } =>
            {
                // 框选结果中已经出现的项目标识。
                let mut selected = std::collections::HashSet::new();
                // 按前端视觉顺序保留的有效框选结果。
                let mut ordered = Vec::with_capacity(item_ids.len());
                for item_id in item_ids
                {
                    validate_id(&item_id, "桌面项目标识无效")?;
                    if !self.contains_item(&item_id) || !self.item_visible_on_monitor(&item_id, monitor_id)
                    {
                        return Err(WorkspaceError::invalid_command("桌面项目不存在"));
                    }
                    if selected.insert(item_id.clone())
                    {
                        ordered.push(item_id);
                    }
                }
                self.interaction.selection_anchor_id = ordered.first().cloned();
                self.interaction.keyboard_focus = ordered.first().cloned().map(|item_id| WorkspaceFocusTarget::Item { item_id }).unwrap_or(WorkspaceFocusTarget::Background);
                self.interaction.selected_item_ids = ordered;
                Ok("框选桌面项目")
            }
            WorkspaceInteractionCommand::FocusSurface { focus } =>
            {
                match &focus
                {
                    WorkspaceFocusTarget::Background =>
                    {}
                    WorkspaceFocusTarget::Item { .. } =>
                    {
                        return Err(WorkspaceError::invalid_command("项目焦点需要指定选择规则"));
                    }
                    WorkspaceFocusTarget::Fence { fence_id } =>
                    {
                        let fence = self.fence(fence_id)?;
                        if (fence.projection_monitor_id.as_deref().unwrap_or(&fence.monitor_id)) != monitor_id
                        {
                            return Err(WorkspaceError::invalid_command("围栏不属于当前显示器"));
                        }
                    }
                    WorkspaceFocusTarget::Component { component_id } =>
                    {
                        validate_id(component_id, "组件标识无效")?;
                        let component = self.components.iter().find(|component| component.instance_id == *component_id).ok_or_else(|| WorkspaceError::invalid_command("组件实例不存在"))?;
                        if component.projection_monitor_id.as_deref().unwrap_or(&component.monitor_id) != monitor_id
                        {
                            return Err(WorkspaceError::invalid_command("组件不属于当前显示器"));
                        }
                    }
                }
                self.interaction.selected_item_ids.clear();
                self.interaction.selection_anchor_id = None;
                self.interaction.keyboard_focus = focus;
                Ok("更新高层焦点")
            }
            WorkspaceInteractionCommand::SelectFocusContainer =>
            {
                // 当前容器全选前保持不变的键盘焦点。
                let focus = self.interaction.keyboard_focus.clone();
                // 当前焦点容器按视觉顺序提供的项目集合。
                let selected = match &focus
                {
                    WorkspaceFocusTarget::Background => self.unclassified_item_order(Some(monitor_id)),
                    WorkspaceFocusTarget::Item { item_id } => self.item_order(item_id),
                    WorkspaceFocusTarget::Fence { fence_id } => self.fence(fence_id)?.members.clone(),
                    WorkspaceFocusTarget::Component { .. } => Vec::new(),
                };
                self.interaction.selection_anchor_id = match focus
                {
                    WorkspaceFocusTarget::Item { item_id } => Some(item_id),
                    _ => selected.first().cloned(),
                };
                self.interaction.selected_item_ids = selected;
                Ok("全选焦点容器")
            }
        }
    }

    /// 判断两个桌面项目是否属于同一焦点容器。
    fn same_item_container(&self, first: &str, second: &str) -> bool
    {
        let first_fence = self.fences.iter().find(|fence| fence.members.iter().any(|member| member == first)).map(|fence| fence.id.as_str()); // 首个项目所属围栏。
        let second_fence = self.fences.iter().find(|fence| fence.members.iter().any(|member| member == second)).map(|fence| fence.id.as_str()); // 第二个项目所属围栏。
        match (first_fence, second_fence)
        {
            (Some(first), Some(second)) => first == second,
            (None, None) => self.item_visible_monitor_id(first) == self.item_visible_monitor_id(second),
            _ => false,
        }
    }

    /// 校验命令所属显示器，未指定时使用当前主显示器兼容网格。
    fn command_monitor_id(&self, requested: Option<&str>) -> Result<String, WorkspaceError>
    {
        let Some(monitor_id) = requested.or_else(|| self.monitor_grids.first().map(|entry| entry.monitor_id.as_str()))
        else
        {
            return Ok(String::new());
        }; // 当前布局命令使用的持久显示器身份；空快照等待首次宿主协调补全。
        if !self.monitor_grids.iter().any(|entry| entry.monitor_id == monitor_id)
        {
            return Err(WorkspaceError::invalid_command("布局命令显示器不存在"));
        }
        Ok(monitor_id.to_string())
    }

    /// 返回项目当前持久或临时投射后实际可见的显示器身份。
    fn item_visible_monitor_id(&self, item_id: &str) -> Option<&str>
    {
        if let Some(fence) = self.fences.iter().find(|fence| fence.members.iter().any(|member| member == item_id))
        {
            return Some(fence.projection_monitor_id.as_deref().unwrap_or(&fence.monitor_id));
        }
        self.unclassified_items.iter().find(|item| item.stable_reference == item_id).map(|item| item.projection_monitor_id.as_deref().unwrap_or(&item.monitor_id))
    }

    /// 判断项目是否可以由指定逐屏页面提交交互。
    fn item_visible_on_monitor(&self, item_id: &str, monitor_id: &str) -> bool { self.item_visible_monitor_id(item_id) == Some(monitor_id) }

    /// 返回目标项目所在焦点容器的完整视觉逻辑顺序。
    fn item_order(&self, item_id: &str) -> Vec<String>
    {
        if let Some(fence) = self.fences.iter().find(|fence| fence.members.iter().any(|member| member == item_id))
        {
            return fence.members.clone();
        }

        let monitor_id = self.item_visible_monitor_id(item_id); // 当前未归类项目可见的逐屏容器。
        self.unclassified_item_order(monitor_id)
    }

    /// 返回未归类区域的完整视觉逻辑顺序。
    fn unclassified_item_order(&self, monitor_id: Option<&str>) -> Vec<String>
    {
        // 当前全部围栏成员用于排除未归类项目。
        let member_ids = self.fences.iter().flat_map(|fence| fence.members.iter()).collect::<std::collections::HashSet<_>>();
        // 未归类区域按从上到下、从左到右排列的项目。
        let mut items = self.unclassified_items.iter().enumerate().filter(|(_, item)| !member_ids.contains(&item.stable_reference) && monitor_id.is_none_or(|monitor_id| item.projection_monitor_id.as_deref().unwrap_or(&item.monitor_id) == monitor_id)).collect::<Vec<_>>();
        items.sort_by(|(first_index, first), (second_index, second)| first.position.top.total_cmp(&second.position.top).then_with(|| first.position.left.total_cmp(&second.position.left)).then_with(|| first_index.cmp(second_index)));
        items.into_iter().map(|(_, item)| item.stable_reference.clone()).collect()
    }

    /// 协调 Shell 项目身份、持久显示器归属并为新项目分配首个空网格位置。
    fn reconcile_items_for_monitors(&mut self, item_ids: &[String], assignments: &[(String, String)], primary_monitor_id: &str) -> Result<(), WorkspaceError>
    {
        // 当前 Shell 快照中的稳定项目引用。
        let active_ids = item_ids.iter().map(String::as_str).collect::<std::collections::HashSet<_>>();
        // 当前全部围栏继续持有的成员引用。
        let member_ids = self.fences.iter().flat_map(|fence| fence.members.iter().cloned()).collect::<std::collections::HashSet<_>>();
        let active_monitor_ids = self.monitor_grids.iter().map(|entry| entry.monitor_id.as_str()).collect::<std::collections::HashSet<_>>(); // 当前可以直接呈现持久布局的显示器身份。
        let assignments = assignments.iter().map(|(item_id, monitor_id)| (item_id.as_str(), monitor_id.as_str())).collect::<std::collections::HashMap<_, _>>(); // Explorer 位置提供的项目初始显示器归属。
        for fence in &mut self.fences
        {
            // 完整 Shell 枚举确认离开的引用退出活动归属，历史快照仍由撤销层保留。
            fence.members.retain(|item_id| {
                let retained = active_ids.contains(item_id.as_str()); // 暂时无法解析但仍被枚举的引用继续保留。
                if !retained
                {
                    crate::log_info!("工作区：清理已离开桌面的围栏成员，item_id={item_id:?}，fence_id={:?}", fence.id);
                }
                retained
            });
            if fence.monitor_id.is_empty()
            {
                fence.monitor_id = primary_monitor_id.to_string();
            }
        }
        for item in &mut self.unclassified_items
        {
            if item.monitor_id.is_empty()
            {
                item.monitor_id = assignments.get(item.stable_reference.as_str()).filter(|monitor_id| active_monitor_ids.contains(**monitor_id)).copied().unwrap_or(primary_monitor_id).to_string();
            }
        }
        self.unclassified_items.retain(|item| active_ids.contains(item.stable_reference.as_str()) && !member_ids.contains(&item.stable_reference));

        let mut occupied = self.monitor_grids.iter().map(|entry| (entry.monitor_id.clone(), self.unclassified_items.iter().filter(|item| item.monitor_id == entry.monitor_id).map(|item| item.position).collect::<Vec<_>>())).collect::<std::collections::HashMap<_, _>>(); // 每台显示器已占用的持久网格位置。
        for item_id in item_ids
        {
            validate_id(item_id, "桌面项目标识无效")?;
            if self.contains_item(item_id)
            {
                continue;
            }

            let monitor_id = assignments.get(item_id.as_str()).filter(|monitor_id| active_monitor_ids.contains(**monitor_id)).copied().unwrap_or(primary_monitor_id); // 新项目按 Explorer 位置或主屏回退获得的持久归属。
            let grid = self.monitor_grids.iter().find(|entry| entry.monitor_id == monitor_id).ok_or_else(|| WorkspaceError::invalid_command("项目显示器网格不存在"))?.grid; // 当前新项目所属显示器的网格。
            let capacity = grid_capacity(grid)?; // 当前显示器网格的安全容量。
            let monitor_occupied = occupied.get_mut(monitor_id).expect("活动显示器已创建占用集合"); // 当前显示器已经占用的位置。
            // 没有确定 Shell 坐标时，从上到下、从左到右填补当前空格。
            let position = (0..capacity).map(|index| grid_position(grid, index)).find(|position| !monitor_occupied.contains(position)).ok_or_else(|| WorkspaceError::invalid_command("未归类网格空间不足"))?;
            // 当前新项目获得的最高显示层级。
            let z_index = self.next_z_index();
            self.unclassified_items.push(UnclassifiedItemSnapshot { stable_reference: item_id.clone(), monitor_id: monitor_id.to_string(), projection_monitor_id: None, projection_position: None, position, z_index });
            monitor_occupied.push(position);
        }
        self.normalize_interaction();
        Ok(())
    }

    /// 为断开的持久显示器建立主屏临时投射，重连后恢复原归属与位置。
    fn reconcile_monitor_projections(&mut self, monitors: &[DesktopHostMonitor], primary_monitor_id: &str) -> Result<(), WorkspaceError>
    {
        let active_monitor_ids = monitors.iter().map(|monitor| monitor.id.as_str()).collect::<std::collections::HashSet<_>>(); // 当前仍连接的显示器身份。
        let primary_monitor = monitors.iter().find(|monitor| monitor.id == primary_monitor_id).ok_or_else(|| WorkspaceError::invalid_command("主显示器不存在"))?; // 临时投射目标显示器。
        let viewport_width = f64::from(primary_monitor.work_width) / primary_monitor.scale_factor; // 主屏逻辑工作区宽度。
        let viewport_height = f64::from(primary_monitor.work_height) / primary_monitor.scale_factor; // 主屏逻辑工作区高度。
        if !primary_monitor.scale_factor.is_finite() || primary_monitor.scale_factor <= 0.0 || !viewport_width.is_finite() || !viewport_height.is_finite() || viewport_width <= 0.0 || viewport_height <= 0.0
        {
            return Err(WorkspaceError::invalid_command("主显示器工作区尺寸无效"));
        }
        let primary_grid = self.monitor_grids.iter().find(|entry| entry.monitor_id == primary_monitor_id).expect("主显示器网格已创建").grid; // 临时投射项目使用的主屏网格。
        let capacity = grid_capacity(primary_grid)?; // 主屏临时投射可使用的网格容量。
        let mut occupied = self
            .unclassified_items
            .iter()
            .filter_map(|item| {
                if item.monitor_id == primary_monitor_id
                {
                    Some(item.position)
                }
                else if item.projection_monitor_id.as_deref() == Some(primary_monitor_id)
                {
                    item.projection_position
                }
                else
                {
                    None
                }
            })
            .collect::<Vec<_>>(); // 主屏已有持久项目与临时投射项目占用的位置。
        for item in &mut self.unclassified_items
        {
            if active_monitor_ids.contains(item.monitor_id.as_str())
            {
                item.projection_monitor_id = None;
                item.projection_position = None;
                continue;
            }
            let position = (0..capacity).map(|index| grid_position(primary_grid, index)).find(|position| !occupied.contains(position)).unwrap_or_else(|| grid_position(primary_grid, capacity.saturating_sub(1))); // 主屏满载时允许临时重叠但不改变持久位置。
            item.projection_monitor_id = Some(primary_monitor_id.to_string());
            item.projection_position = Some(position);
            occupied.push(position);
        }
        for fence in &mut self.fences
        {
            if active_monitor_ids.contains(fence.monitor_id.as_str())
            {
                fence.projection_monitor_id = None;
                fence.projection_bounds = None;
                continue;
            }
            fence.projection_monitor_id = Some(primary_monitor_id.to_string());
            fence.projection_bounds = Some(project_rect(fence.bounds, viewport_width, viewport_height));
        }
        for component in &mut self.components
        {
            let was_unassigned = component.monitor_id.is_empty(); // 官方默认实例首次进入逐屏布局的标记。
            if was_unassigned
            {
                component.monitor_id = primary_monitor_id.to_string();
                component.bounds.left = (viewport_width - component.bounds.width - 28.0).max(0.0);
                component.bounds.top = if component.component_id == "official.debug-test"
                {
                    16.0
                }
                else
                {
                    (viewport_height - component.bounds.height - 28.0).max(0.0)
                };
            }
            if active_monitor_ids.contains(component.monitor_id.as_str())
            {
                component.projection_monitor_id = None;
                component.projection_bounds = None;
                continue;
            }
            component.projection_monitor_id = Some(primary_monitor_id.to_string());
            component.projection_bounds = Some(project_rect(component.bounds, viewport_width, viewport_height));
        }
        Ok(())
    }

    /// 应用一个已反序列化的工作区命令。
    fn apply(&mut self, command: WorkspaceCommand, monitor_id: Option<&str>) -> Result<&'static str, WorkspaceError>
    {
        match command
        {
            WorkspaceCommand::SetFolderPortalAppearance { portal_id, change } =>
            {
                use crate::contracts::TileAppearanceChange;
                let portal = self.folder_portals.iter_mut().find(|portal| portal.id == portal_id).ok_or_else(|| WorkspaceError::invalid_command("门户不存在"))?; // 仅更新指定实例外观。
                match change
                {
                    TileAppearanceChange::Title(_) => return Err(WorkspaceError::invalid_command("门户标题由绑定目录决定")),
                    TileAppearanceChange::TitleMode(mode) => portal.appearance.title_mode = mode,
                    TileAppearanceChange::ButtonMode(mode) =>
                    {
                        if portal.appearance.title_mode == crate::contracts::TileTitleMode::Hover
                        {
                            return Err(WorkspaceError::invalid_command("悬浮标题下不能修改按钮偏好"));
                        }
                        portal.appearance.button_mode = mode;
                    }
                    TileAppearanceChange::Collapsed(value) => portal.appearance.collapsed = value,
                    TileAppearanceChange::AutoExpand(value) => portal.appearance.auto_expand = value,
                }
                Ok("更新门户外观")
            }
            WorkspaceCommand::CreateFolderPortal { root_path, bounds } =>
            {
                validate_rect(bounds, false)?;
                if !Path::new(&root_path).is_absolute() || !Path::new(&root_path).is_dir()
                {
                    return Err(WorkspaceError::invalid_command("绑定路径必须是可访问的绝对目录"));
                }
                let mut sequence = self.revision.saturating_add(1); // 只检测身份碰撞，不检测路径重复。
                while self.folder_portals.iter().any(|portal| portal.id == format!("portal-{sequence}"))
                {
                    sequence = sequence.checked_add(1).ok_or_else(|| WorkspaceError::invalid_command("门户身份已耗尽"))?;
                }
                self.folder_portals.push(crate::contracts::FolderPortalInstance { id: format!("portal-{sequence}"), root_path, monitor_id: monitor_id.unwrap_or_default().to_string(), bounds, z_index: self.folder_portals.len() as u32, appearance: Default::default() });
                Ok("创建文件夹门户")
            }
            WorkspaceCommand::RemapFolderPortal { portal_id, root_path } =>
            {
                if !Path::new(&root_path).is_absolute() || !Path::new(&root_path).is_dir()
                {
                    return Err(WorkspaceError::invalid_command("绑定路径必须是可访问的绝对目录"));
                }
                let portal = self.folder_portals.iter_mut().find(|portal| portal.id == portal_id).ok_or_else(|| WorkspaceError::invalid_command("门户不存在"))?;
                portal.root_path = root_path;
                Ok("重新映射文件夹门户")
            }
            WorkspaceCommand::UpdateFolderPortalBounds { portal_id, bounds } =>
            {
                validate_rect(bounds, false)?;
                let portal = self.folder_portals.iter_mut().find(|portal| portal.id == portal_id).ok_or_else(|| WorkspaceError::invalid_command("门户不存在"))?; // 按实例身份提交。
                portal.bounds = bounds;
                if let Some(monitor_id) = monitor_id
                {
                    portal.monitor_id = monitor_id.to_string();
                }
                Ok("更新门户布局")
            }
            WorkspaceCommand::RemoveFolderPortal { portal_id } =>
            {
                self.folder_portals.retain(|portal| portal.id != portal_id);
                Ok("移除文件夹门户")
            }
            WorkspaceCommand::SetTileAppearance { target, change } =>
            {
                appearance::apply(self, target, change)?;
                Ok("更新格子外观")
            }
            WorkspaceCommand::CreateFence { title, bounds } =>
            {
                validate_title(&title)?;
                validate_rect(bounds, false)?;
                // 当前事务分配的稳定围栏标识。
                let mut sequence = self.revision.saturating_add(1); // 全局投影中查找未占用身份，避免桌面修订号重名。
                let id = loop
                {
                    let candidate = format!("fence-{sequence}"); // 当前候选全局围栏身份。
                    if !self.fences.iter().any(|fence| fence.id == candidate) && !self.tile_appearances.contains_key(&format!("fence:{candidate}"))
                    {
                        break candidate;
                    }
                    sequence = sequence.checked_add(1).ok_or_else(|| WorkspaceError::invalid_command("围栏身份已耗尽"))?;
                };
                // 新围栏使用的最高层级。
                let z_index = self.next_z_index();
                let monitor_id = self.command_monitor_id(monitor_id)?; // 当前页面所属的持久显示器身份。
                self.fences.push(FenceSnapshot { id, title, monitor_id, projection_monitor_id: None, projection_bounds: None, bounds, members: Vec::new(), layout_mode: FenceLayoutMode::Tile, sort_mode: FenceSortMode::None, auto_expand: false, scroll_top: 0.0, collapsed: false, last_expanded_height: bounds.height, z_index });
                Ok("创建围栏")
            }
            WorkspaceCommand::CreateFenceWithItems { title, bounds, item_ids } =>
            {
                let anchor_item_id = match &self.interaction.keyboard_focus
                {
                    WorkspaceFocusTarget::Item { item_id } if item_ids.contains(item_id) => item_id.clone(),
                    _ => item_ids.first().cloned().ok_or_else(|| WorkspaceError::invalid_command("新建围栏的项目选择为空"))?,
                }; // 保留仍在完整选择中的项目焦点。
                self.apply(WorkspaceCommand::CreateFence { title, bounds }, monitor_id)?;
                let fence_id = self.fences.last().expect("创建命令已追加新围栏").id.clone(); // 同一事务副本中的新身份。
                self.apply(WorkspaceCommand::CommitItemDrag { anchor_item_id, item_ids, placements: Vec::new(), target_fence_id: Some(fence_id), before_item_id: None, fence_scrolls: Vec::new() }, monitor_id)?;
                Ok("新建围栏并移入")
            }
            WorkspaceCommand::UpdateFenceBounds { fence_id, bounds } =>
            {
                validate_rect(bounds, false)?;
                let target_monitor_id = self.command_monitor_id(monitor_id)?; // 围栏中心最终所在的显示器身份。
                // 等待移动或缩放提交的围栏。
                let fence = self.fence_mut(&fence_id)?;
                fence.monitor_id = target_monitor_id;
                fence.projection_monitor_id = None;
                fence.projection_bounds = None;
                fence.bounds = bounds;
                fence.last_expanded_height = bounds.height;
                Ok("更新围栏边界")
            }
            WorkspaceCommand::RenameFence { fence_id, title } =>
            {
                let title = title.trim(); // 去除输入两端空白。
                validate_title(title)?;
                self.fence_mut(&fence_id)?.title = title.to_string();
                Ok("重命名围栏")
            }
            WorkspaceCommand::SetFenceAutoExpand { fence_id, enabled } =>
            {
                self.fence_mut(&fence_id)?.auto_expand = enabled;
                Ok("更新围栏悬停展开策略")
            }
            WorkspaceCommand::SetFenceCollapsed { fence_id, collapsed } =>
            {
                // 折叠时需要协调选择与焦点的围栏成员。
                let members = self.fence(&fence_id)?.members.clone();
                self.fence_mut(&fence_id)?.collapsed = collapsed;
                if collapsed
                {
                    self.interaction.selected_item_ids.retain(|item_id| !members.contains(item_id));
                    if self.interaction.selection_anchor_id.as_ref().is_some_and(|item_id| members.contains(item_id))
                    {
                        self.interaction.selection_anchor_id = None;
                    }
                    if matches!(
                        &self.interaction.keyboard_focus,
                        WorkspaceFocusTarget::Item { item_id } if members.contains(item_id)
                    )
                    {
                        self.interaction.keyboard_focus = WorkspaceFocusTarget::Fence { fence_id: fence_id.clone() };
                    }
                }
                Ok("更新围栏折叠状态")
            }
            WorkspaceCommand::SetFenceLayout { fence_id, layout_mode } =>
            {
                // 等待布局方式提交的围栏。
                let fence = self.fence_mut(&fence_id)?;
                fence.layout_mode = layout_mode;
                Ok("更新围栏布局")
            }
            WorkspaceCommand::SetFenceSort { fence_id, sort_mode } =>
            {
                self.fence_mut(&fence_id)?.sort_mode = sort_mode;
                Ok("更新围栏排序")
            }
            WorkspaceCommand::SetFenceScroll { fence_id, scroll_top } =>
            {
                if !scroll_top.is_finite() || scroll_top < 0.0
                {
                    return Err(WorkspaceError::invalid_command("围栏滚动位置无效"));
                }
                // 等待滚动位置提交的围栏。
                let fence = self.fence_mut(&fence_id)?;
                fence.scroll_top = scroll_top;
                Ok("更新围栏滚动位置")
            }
            WorkspaceCommand::MoveItemToFence { item_id, fence_id, before_item_id } =>
            {
                validate_id(&item_id, "桌面项目标识无效")?;
                self.fence(&fence_id)?; // 修改归属前确认目标存在。
                if !self.contains_item(&item_id)
                {
                    return Err(WorkspaceError::invalid_command("桌面项目不存在"));
                }
                if before_item_id.as_deref() == Some(item_id.as_str())
                {
                    return Ok("保持围栏成员顺序");
                }
                for fence in &mut self.fences
                {
                    fence.members.retain(|member| member != &item_id);
                }
                self.unclassified_items.retain(|item| item.stable_reference != item_id);
                // 目标围栏获得当前最高层级。
                let next_z_index = self.next_z_index();
                // 等待成员顺序提交的目标围栏。
                let fence = self.fence_mut(&fence_id)?;
                // 指定后继成员对应的手动插入位置。
                let insertion_index = before_item_id.as_ref().filter(|_| fence.sort_mode == FenceSortMode::None).and_then(|target| fence.members.iter().position(|member| member == target)).unwrap_or(fence.members.len());
                fence.members.insert(insertion_index, item_id);
                fence.z_index = next_z_index;
                Ok("更新围栏归属")
            }
            WorkspaceCommand::RemoveItemFromFence { item_id } =>
            {
                validate_id(&item_id, "桌面项目标识无效")?;
                let target_monitor_id = self.command_monitor_id(monitor_id)?; // 项目移出围栏后的显示器归属。
                if !self.contains_item(&item_id)
                {
                    return Err(WorkspaceError::invalid_command("桌面项目不存在"));
                }
                let Some(fence) = self.fences.iter().find(|fence| fence.members.contains(&item_id))
                else
                {
                    return Ok("保持未归类位置");
                }; // 已在桌面的项目不重复移动。
                let bounds = fence.projection_bounds.unwrap_or(fence.bounds); // 普通移出使用当前可见围栏位置。
                let desired = WorkspacePoint { left: bounds.left, top: bounds.top };
                let position = self.nearest_unclassified_position(&target_monitor_id, desired)?; // 当前空格不受历史槽位影响。
                for fence in &mut self.fences
                {
                    fence.members.retain(|member| member != &item_id);
                }
                self.unclassified_items.retain(|item| item.stable_reference != item_id);
                self.unclassified_items.push(UnclassifiedItemSnapshot { stable_reference: item_id, monitor_id: target_monitor_id, projection_monitor_id: None, projection_position: None, position, z_index: self.next_z_index() });
                Ok("移出围栏")
            }
            WorkspaceCommand::CommitItemDrag { anchor_item_id, item_ids, placements, target_fence_id, before_item_id, fence_scrolls } =>
            {
                validate_id(&anchor_item_id, "拖动锚点标识无效")?;
                if item_ids.is_empty() || !item_ids.contains(&anchor_item_id)
                {
                    return Err(WorkspaceError::invalid_command("拖动项目集合无效"));
                }
                // 当前拖动集合中已经出现的项目标识。
                let mut dragged_ids = std::collections::HashSet::new();
                for item_id in &item_ids
                {
                    validate_id(item_id, "拖动项目标识无效")?;
                    if !dragged_ids.insert(item_id.clone()) || !self.contains_item(item_id)
                    {
                        return Err(WorkspaceError::invalid_command("拖动项目集合包含未知项目"));
                    }
                }

                // 拖动自动滚动提交中已经出现的围栏标识。
                let mut scrolled_fences = std::collections::HashSet::new();
                for WorkspaceFenceScroll { fence_id, scroll_top } in fence_scrolls
                {
                    if !scroll_top.is_finite() || scroll_top < 0.0 || !scrolled_fences.insert(fence_id.clone())
                    {
                        return Err(WorkspaceError::invalid_command("拖动围栏滚动位置无效"));
                    }
                    self.fence_mut(&fence_id)?.scroll_top = scroll_top;
                }

                for fence in &mut self.fences
                {
                    fence.members.retain(|member| !dragged_ids.contains(member));
                }
                if let Some(fence_id) = target_fence_id
                {
                    if !placements.is_empty()
                    {
                        return Err(WorkspaceError::invalid_command("围栏投放包含未归类位置"));
                    }
                    self.unclassified_items.retain(|item| !dragged_ids.contains(&item.stable_reference));
                    // 目标围栏在本次拖动后获得的最高层级。
                    let next_z_index = self.next_z_index();
                    // 等待组成员插入的目标围栏。
                    let fence = self.fence_mut(&fence_id)?;
                    // 可选后继成员在移除拖动集合后的插入位置。
                    let insertion_index = match before_item_id.filter(|_| fence.sort_mode == FenceSortMode::None)
                    {
                        Some(before_item_id) => fence.members.iter().position(|member| member == &before_item_id).unwrap_or(fence.members.len()),
                        None => fence.members.len(),
                    };
                    fence.members.splice(insertion_index..insertion_index, item_ids.iter().cloned());
                    fence.z_index = next_z_index;
                }
                else
                {
                    if before_item_id.is_some()
                    {
                        return Err(WorkspaceError::invalid_command("未归类投放包含围栏插入目标"));
                    }
                    let target_monitor_id = self.command_monitor_id(monitor_id)?; // 未归类投放所属的目标显示器身份。
                    // 当前提交中已经出现的网格项目标识。
                    let mut placed_ids = std::collections::HashSet::new();
                    for WorkspaceItemPlacement { item_id, position } in placements
                    {
                        validate_id(&item_id, "网格项目标识无效")?;
                        validate_point(position)?;
                        if !placed_ids.insert(item_id.clone())
                        {
                            return Err(WorkspaceError::invalid_command("网格位置包含重复项目"));
                        }
                        if !self.unclassified_items.iter().any(|item| item.stable_reference == item_id)
                        {
                            if !dragged_ids.contains(&item_id)
                            {
                                return Err(WorkspaceError::invalid_command("网格位置包含未知项目"));
                            }
                            self.unclassified_items.push(UnclassifiedItemSnapshot { stable_reference: item_id.clone(), monitor_id: target_monitor_id.clone(), projection_monitor_id: None, projection_position: None, position, z_index: 0 });
                        }
                        let item = self.unclassified_items.iter_mut().find(|item| item.stable_reference == item_id).expect("已校验或创建当前落位记录");
                        item.position = position;
                        item.monitor_id.clone_from(&target_monitor_id);
                        item.projection_monitor_id = None;
                        item.projection_position = None;
                    }
                    if item_ids.iter().any(|item_id| !placed_ids.contains(item_id))
                    {
                        return Err(WorkspaceError::invalid_command("拖动项目缺少最终网格位置"));
                    }
                    // 拖动项目按稳定视觉顺序获得连续最高层级。
                    let first_z_index = self.next_z_index();
                    for (offset, item_id) in item_ids.iter().enumerate()
                    {
                        let item = self.unclassified_items.iter_mut().find(|item| item.stable_reference == *item_id).expect("拖动项目已完成存在性校验");
                        item.z_index = first_z_index.saturating_add(offset as u32);
                    }
                }

                self.interaction.selected_item_ids = item_ids;
                self.interaction.selection_anchor_id = Some(anchor_item_id.clone());
                self.interaction.keyboard_focus = WorkspaceFocusTarget::Item { item_id: anchor_item_id };
                Ok("提交项目拖动")
            }
            WorkspaceCommand::DissolveFence { fence_id } =>
            {
                // 当前事务移除并释放成员的围栏。
                let fence_index = self.fences.iter().position(|fence| fence.id == fence_id).ok_or_else(|| WorkspaceError::invalid_command("围栏不存在"))?;
                // 保留当前视觉成员顺序和内容区锚点的待解散围栏。
                let fence = self.fences.remove(fence_index);
                let grid = self.monitor_grids.iter().find(|entry| entry.monitor_id == fence.monitor_id).map(|entry| entry.grid).unwrap_or(self.grid); // 围栏持久显示器对应的未归类网格。
                for item in self.unclassified_items.iter_mut().filter(|item| fence.members.contains(&item.stable_reference))
                {
                    item.monitor_id.clone_from(&fence.monitor_id);
                    item.projection_monitor_id = None;
                    item.projection_position = None;
                }
                if self.interaction.keyboard_focus == (WorkspaceFocusTarget::Fence { fence_id: fence.id.clone() })
                {
                    self.interaction.keyboard_focus = WorkspaceFocusTarget::Background;
                }
                // 围栏内容区左上对应的组锚点网格索引。
                let anchor_column = ((fence.bounds.left + 10.0 - grid.left) / grid.column_width).round().clamp(0.0, f64::from(grid.columns - 1)) as u32;
                // 围栏内容区左上对应的组锚点网格行。
                let anchor_row = ((fence.bounds.top + 54.0 - grid.top) / grid.row_height).round().clamp(0.0, f64::from(grid.rows - 1)) as u32;
                // 当前未归类网格总容量。
                let capacity = grid_capacity(grid)?;
                // 下一个成员开始避让的位置。
                let mut next_index = anchor_column * grid.rows + anchor_row;
                // 排除当前围栏成员来源格后的已占位置。
                let mut occupied = self.unclassified_items.iter().filter(|item| item.monitor_id == fence.monitor_id && !fence.members.contains(&item.stable_reference)).map(|item| item.position).collect::<Vec<_>>();

                for member in fence.members
                {
                    // 从当前连续位置开始找到的首个空网格索引。
                    let available_index = (0..capacity)
                        .map(|offset| (next_index + offset) % capacity)
                        .find(|index| {
                            // 当前避让扫描索引对应的网格位置。
                            let candidate = grid_position(grid, *index);
                            !occupied.contains(&candidate)
                        })
                        .ok_or_else(|| WorkspaceError::invalid_command("未归类网格空间不足"))?;
                    // 当前成员避让已有项目后的最终网格位置。
                    let position = grid_position(grid, available_index);
                    // 当前成员对应的持久化未归类位置。
                    self.unclassified_items.retain(|item| item.stable_reference != member);
                    self.unclassified_items.push(UnclassifiedItemSnapshot { stable_reference: member, monitor_id: fence.monitor_id.clone(), projection_monitor_id: None, projection_position: None, position, z_index: self.next_z_index() });
                    occupied.push(position);
                    next_index = (available_index + 1) % capacity;
                }
                Ok("解散围栏")
            }
            WorkspaceCommand::CreateComponent { component_id, bounds } =>
            {
                validate_rect(bounds, false)?;
                if component_id != "official.collect-box"
                {
                    return Err(WorkspaceError::invalid_command("不支持创建该组件"));
                }
                let monitor_id = self.command_monitor_id(monitor_id)?;
                let mut suffix = self.components.len() + 1;
                let instance_id = loop
                {
                    let candidate = format!("{component_id}.{suffix}");
                    if !self.components.iter().any(|component| component.instance_id == candidate) && !self.tile_appearances.contains_key(&format!("component:{candidate}"))
                    {
                        break candidate;
                    }
                    suffix += 1;
                };
                self.components.push(ComponentInstanceSnapshot { instance_id, component_id, monitor_id, projection_monitor_id: None, projection_bounds: None, bounds, state_version: 1, state: serde_json::json!({}) });
                Ok("创建组件")
            }
            WorkspaceCommand::RemoveComponent { instance_id } =>
            {
                if self.component_mut(&instance_id)?.component_id != "official.collect-box"
                {
                    return Err(WorkspaceError::invalid_command("仅支持移除收集箱格子"));
                }
                // 外观独立于布局历史保留，并占用旧身份，避免新格子继承旧偏好。
                self.tile_appearances.entry(format!("component:{instance_id}")).or_default();
                self.components.retain(|component| component.instance_id != instance_id);
                self.normalize_interaction();
                Ok("移除收集箱格子")
            }
            WorkspaceCommand::UpdateComponentBounds { instance_id, bounds } =>
            {
                validate_rect(bounds, false)?;
                let monitor_id = self.command_monitor_id(monitor_id)?; // 组件移动提交后的持久显示器身份。
                let component = self.component_mut(&instance_id)?; // 等待提交位置和尺寸的组件实例。
                component.monitor_id = monitor_id;
                component.projection_monitor_id = None;
                component.projection_bounds = None;
                component.bounds = bounds;
                Ok("更新组件边界")
            }
            WorkspaceCommand::UpdateComponentState { instance_id, state_version, state } =>
            {
                if state_version == 0
                {
                    return Err(WorkspaceError::invalid_command("组件状态版本无效"));
                }
                let state_size = serde_json::to_vec(&state).map_err(|error| WorkspaceError::invalid_command(format!("组件状态无效: {error}")))?.len(); // 组件持久状态的 UTF-8 JSON 字节数。
                if state_size > 64 * 1024
                {
                    return Err(WorkspaceError::invalid_command("组件状态超过 64 KiB 限制"));
                }
                let component = self.component_mut(&instance_id)?; // 等待提交版本化状态的组件实例。
                component.state_version = state_version;
                component.state = state;
                Ok("更新组件状态")
            }
            WorkspaceCommand::UndoLayout | WorkspaceCommand::RedoLayout =>
            {
                unreachable!("布局历史命令由 WorkspaceState 处理")
            }
        }
    }

    /// 清理布局变化后已经失效的选择、锚点和键盘焦点。
    fn normalize_interaction(&mut self)
    {
        // 当前布局仍保存的全部项目稳定引用。
        let item_ids = self.unclassified_items.iter().map(|item| item.stable_reference.as_str()).chain(self.fences.iter().flat_map(|fence| fence.members.iter().map(String::as_str))).collect::<std::collections::HashSet<_>>();
        // 当前布局仍保存的全部围栏稳定标识。
        let fence_ids = self.fences.iter().map(|fence| fence.id.as_str()).collect::<std::collections::HashSet<_>>();
        let component_ids = self.components.iter().map(|component| component.instance_id.as_str()).collect::<std::collections::HashSet<_>>(); // 当前布局仍保存的全部组件实例标识。
        self.interaction.selected_item_ids.retain(|item_id| item_ids.contains(item_id.as_str()));
        if self.interaction.selection_anchor_id.as_deref().is_some_and(|item_id| !item_ids.contains(item_id))
        {
            self.interaction.selection_anchor_id = None;
        }
        if matches!(
            &self.interaction.keyboard_focus,
            WorkspaceFocusTarget::Item { item_id } if !item_ids.contains(item_id.as_str())
        ) || matches!(
            &self.interaction.keyboard_focus,
            WorkspaceFocusTarget::Fence { fence_id } if !fence_ids.contains(fence_id.as_str())
        ) || matches!(
            &self.interaction.keyboard_focus,
            WorkspaceFocusTarget::Component { component_id } if !component_ids.contains(component_id.as_str())
        )
        {
            self.interaction.keyboard_focus = WorkspaceFocusTarget::Background;
        }
    }

    /// 校验恢复或提交后的完整工作区快照。
    fn validate(&self) -> Result<(), WorkspaceError>
    {
        if self.schema_version != WORKSPACE_SCHEMA_VERSION
        {
            return Err(WorkspaceError::invalid_command("工作区 Schema 版本无效"));
        }
        // 当前选择集合中已经出现的项目标识。
        let mut selected_ids = std::collections::HashSet::new();
        for item_id in &self.interaction.selected_item_ids
        {
            validate_id(item_id, "选择项目标识无效")?;
            if !selected_ids.insert(item_id) || !self.contains_item(item_id)
            {
                return Err(WorkspaceError::invalid_command("选择集合包含未知项目"));
            }
        }
        if let Some(anchor_id) = &self.interaction.selection_anchor_id
        {
            validate_id(anchor_id, "选择锚点标识无效")?;
            if !self.contains_item(anchor_id)
            {
                return Err(WorkspaceError::invalid_command("选择锚点包含未知项目"));
            }
        }
        grid_capacity(self.grid)?;
        let mut monitor_ids = std::collections::HashSet::new(); // 当前逐屏网格中已经出现的显示器身份。
        for entry in &self.monitor_grids
        {
            validate_id(&entry.monitor_id, "显示器标识无效")?;
            grid_capacity(entry.grid)?;
            if !monitor_ids.insert(entry.monitor_id.as_str())
            {
                return Err(WorkspaceError::invalid_command("显示器网格重复"));
            }
        }
        // 当前快照中已经出现的围栏标识。
        let mut fence_ids = std::collections::HashSet::new();
        // 当前快照中已经归属围栏的桌面项目。
        let mut member_ids = std::collections::HashSet::new();
        for fence in &self.fences
        {
            validate_id(&fence.id, "围栏标识无效")?;
            validate_title(&fence.title)?;
            validate_rect(fence.bounds, false)?;
            if let Some(projection_monitor_id) = &fence.projection_monitor_id
            {
                validate_id(projection_monitor_id, "围栏投射显示器标识无效")?;
                validate_rect(fence.projection_bounds.ok_or_else(|| WorkspaceError::invalid_command("围栏投射边界缺失"))?, true)?;
            }
            else if fence.projection_bounds.is_some()
            {
                return Err(WorkspaceError::invalid_command("围栏投射状态无效"));
            }
            if !fence.scroll_top.is_finite() || fence.scroll_top < 0.0 || !fence.last_expanded_height.is_finite() || fence.last_expanded_height < TILE_MIN_HEIGHT || !fence_ids.insert(&fence.id)
            {
                return Err(WorkspaceError::invalid_command("围栏持久化状态无效"));
            }
            for member in &fence.members
            {
                validate_id(member, "围栏成员标识无效")?;
                if !member_ids.insert(member)
                {
                    return Err(WorkspaceError::invalid_command("桌面项目存在重复围栏归属"));
                }
            }
        }
        for item in &self.unclassified_items
        {
            validate_id(&item.stable_reference, "桌面项目标识无效")?;
            validate_point(item.position)?;
        }
        // 当前未归类项目身份必须唯一。
        let source_ids: std::collections::HashSet<_> = self.unclassified_items.iter().map(|item| &item.stable_reference).collect();
        // 当前桌面位置不能重复，围栏成员不参与此集合。
        let source_positions = self.unclassified_items.iter().map(|item| (item.monitor_id.as_str(), item.position.left.to_bits(), item.position.top.to_bits())).collect::<std::collections::HashSet<_>>();
        if source_ids.len() != self.unclassified_items.len() || source_positions.len() != self.unclassified_items.len()
        {
            return Err(WorkspaceError::invalid_command("未归类项目身份或位置重复"));
        }
        let mut portal_ids = std::collections::HashSet::new(); // 路径可以重复，实例身份必须唯一。
        for portal in &self.folder_portals
        {
            validate_id(&portal.id, "门户身份无效")?;
            validate_rect(portal.bounds, false)?;
            if !portal_ids.insert(&portal.id) || !Path::new(&portal.root_path).is_absolute()
            {
                return Err(WorkspaceError::invalid_command("门户绑定或身份无效"));
            }
        }
        let mut component_ids = std::collections::HashSet::new(); // 当前快照中已经出现的组件实例标识。
        for component in &self.components
        {
            validate_id(&component.instance_id, "组件实例标识无效")?;
            validate_id(&component.component_id, "组件清单标识无效")?;
            validate_rect(component.bounds, false)?;
            if !component_ids.insert(component.instance_id.as_str()) || component.state_version == 0 || serde_json::to_vec(&component.state).map_err(|error| WorkspaceError::invalid_command(format!("组件状态无效: {error}")))?.len() > 64 * 1024
            {
                return Err(WorkspaceError::invalid_command("组件持久化状态无效"));
            }
            if let Some(projection_monitor_id) = &component.projection_monitor_id
            {
                validate_id(projection_monitor_id, "组件投射显示器标识无效")?;
                validate_rect(component.projection_bounds.ok_or_else(|| WorkspaceError::invalid_command("组件投射边界缺失"))?, true)?;
            }
            else if component.projection_bounds.is_some()
            {
                return Err(WorkspaceError::invalid_command("组件投射状态无效"));
            }
        }
        match &self.interaction.keyboard_focus
        {
            WorkspaceFocusTarget::Background =>
            {}
            WorkspaceFocusTarget::Item { item_id } =>
            {
                if !self.contains_item(item_id)
                {
                    return Err(WorkspaceError::invalid_command("项目焦点包含未知项目"));
                }
            }
            WorkspaceFocusTarget::Fence { fence_id } =>
            {
                if !fence_ids.contains(fence_id)
                {
                    return Err(WorkspaceError::invalid_command("围栏焦点包含未知围栏"));
                }
            }
            WorkspaceFocusTarget::Component { component_id } =>
            {
                validate_id(component_id, "组件焦点标识无效")?;
                if !component_ids.contains(component_id.as_str())
                {
                    return Err(WorkspaceError::invalid_command("组件焦点包含未知实例"));
                }
            }
        }
        Ok(())
    }

    /// 返回指定围栏的只读引用。
    fn fence(&self, fence_id: &str) -> Result<&FenceSnapshot, WorkspaceError>
    {
        validate_id(fence_id, "围栏标识无效")?;
        self.fences.iter().find(|fence| fence.id == fence_id).ok_or_else(|| WorkspaceError::invalid_command("围栏不存在"))
    }

    /// 返回指定围栏的可写引用。
    fn fence_mut(&mut self, fence_id: &str) -> Result<&mut FenceSnapshot, WorkspaceError>
    {
        validate_id(fence_id, "围栏标识无效")?;
        self.fences.iter_mut().find(|fence| fence.id == fence_id).ok_or_else(|| WorkspaceError::invalid_command("围栏不存在"))
    }

    /// 返回指定组件实例的可写引用。
    fn component_mut(&mut self, instance_id: &str) -> Result<&mut ComponentInstanceSnapshot, WorkspaceError>
    {
        validate_id(instance_id, "组件实例标识无效")?;
        self.components.iter_mut().find(|component| component.instance_id == instance_id).ok_or_else(|| WorkspaceError::invalid_command("组件实例不存在"))
    }

    /// 只按当前可见桌面位置寻找最近空格，距离相同采用列优先顺序。
    fn nearest_unclassified_position(&self, monitor_id: &str, desired: WorkspacePoint) -> Result<WorkspacePoint, WorkspaceError>
    {
        validate_point(desired)?;
        let grid = self.monitor_grids.iter().find(|entry| entry.monitor_id == monitor_id).map(|entry| entry.grid).ok_or_else(|| WorkspaceError::invalid_command("项目显示器网格不存在"))?; // 目标显示器网格。
        let occupied = self.unclassified_items.iter().filter(|item| item.projection_monitor_id.as_deref().unwrap_or(&item.monitor_id) == monitor_id).map(|item| item.projection_position.unwrap_or(item.position)).collect::<Vec<_>>(); // 仅当前桌面上的项目占位。
        (0..grid_capacity(grid)?).map(|index| grid_position(grid, index)).filter(|point| !occupied.contains(point)).min_by(|a, b| ((a.left - desired.left).powi(2) + (a.top - desired.top).powi(2)).total_cmp(&((b.left - desired.left).powi(2) + (b.top - desired.top).powi(2)))).ok_or_else(|| WorkspaceError::invalid_command("未归类网格空间不足"))
    }

    /// 分配新的围栏最高层级。
    fn next_z_index(&self) -> u32 { self.fences.iter().map(|fence| fence.z_index).chain(self.unclassified_items.iter().map(|item| item.z_index)).max().unwrap_or(0).saturating_add(1) }
}

/// 校验用户可见围栏标题。
fn validate_title(title: &str) -> Result<(), WorkspaceError>
{
    if title.contains('\0')
    {
        return Err(WorkspaceError::invalid_command("围栏标题无效"));
    }
    Ok(())
}

/// 校验工作区对象标识。
fn validate_id(id: &str, message: &str) -> Result<(), WorkspaceError>
{
    if id.trim().is_empty()
    {
        return Err(WorkspaceError::invalid_command(message));
    }
    Ok(())
}

/// 校验未归类项目位置。
fn validate_point(point: WorkspacePoint) -> Result<(), WorkspaceError>
{
    if !point.left.is_finite() || !point.top.is_finite()
    {
        return Err(WorkspaceError::invalid_command("桌面项目来源位置无效"));
    }
    Ok(())
}

/// 从正式宿主工作区尺寸生成版本化未归类网格配置。
fn workspace_grid(workspace_width: f64, workspace_height: f64, preferences: GridPreferences) -> Result<WorkspaceGrid, WorkspaceError>
{
    if !workspace_width.is_finite() || !workspace_height.is_finite() || workspace_width <= 0.0 || workspace_height <= 0.0
    {
        return Err(WorkspaceError::invalid_command("工作区尺寸无效"));
    }
    let column_width = f64::from(preferences.width + preferences.column_gap); // 格子宽度加横向空白。
    let row_height = f64::from(preferences.height + preferences.row_gap); // 格子高度加纵向空白。
    Ok(WorkspaceGrid { left: WORKSPACE_GRID_LEFT, top: WORKSPACE_GRID_TOP, column_width, row_height, rows: ((workspace_height - WORKSPACE_GRID_BOTTOM_MARGIN) / row_height).floor().max(1.0) as u32, columns: ((workspace_width - WORKSPACE_GRID_LEFT) / column_width).floor().max(1.0) as u32 })
}

/// 校验网格配置并返回受控容量。
fn grid_capacity(grid: WorkspaceGrid) -> Result<u32, WorkspaceError>
{
    if !grid.left.is_finite() || !grid.top.is_finite() || !grid.column_width.is_finite() || grid.column_width <= 0.0 || !grid.row_height.is_finite() || grid.row_height <= 0.0 || grid.rows == 0 || grid.columns == 0
    {
        return Err(WorkspaceError::invalid_command("未归类网格配置无效"));
    }
    grid.rows.checked_mul(grid.columns).filter(|capacity| *capacity <= 1_000_000).ok_or_else(|| WorkspaceError::invalid_command("未归类网格容量无效"))
}

/// 将列优先线性索引转换为未归类网格位置。
fn grid_position(grid: WorkspaceGrid, index: u32) -> WorkspacePoint { WorkspacePoint { left: grid.left + f64::from(index / grid.rows) * grid.column_width, top: grid.top + f64::from(index % grid.rows) * grid.row_height } }

/// 持久展开边界遵守公共底线，临时投射仅要求有限正尺寸。
fn validate_rect(bounds: WorkspaceRect, projection: bool) -> Result<(), WorkspaceError>
{
    if !bounds.left.is_finite() || !bounds.top.is_finite() || !bounds.width.is_finite() || !bounds.height.is_finite() || bounds.width <= 0.0 || bounds.height <= 0.0 || (!projection && (bounds.width < TILE_MIN_WIDTH || bounds.height < TILE_MIN_HEIGHT))
    {
        return Err(WorkspaceError::invalid_command("格子边界无效"));
    }
    Ok(())
}

/// 所有格子共用临时投射，较小的可用视口优先于持久尺寸底线。
fn project_rect(bounds: WorkspaceRect, viewport_width: f64, viewport_height: f64) -> WorkspaceRect
{
    let width = bounds.width.min(viewport_width).max(TILE_MIN_WIDTH.min(viewport_width)); // 临时投射后的可见宽度。
    let height = bounds.height.min(viewport_height).max(TILE_MIN_HEIGHT.min(viewport_height)); // 临时投射后的可见高度。
    WorkspaceRect { left: bounds.left.clamp(0.0, (viewport_width - width).max(0.0)), top: bounds.top.clamp(0.0, (viewport_height - height).max(0.0)), width, height }
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
