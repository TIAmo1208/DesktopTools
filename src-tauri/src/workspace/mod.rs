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
use crate::contracts::{ComponentInstanceSnapshot, DesktopHostMonitor, FENCE_MIN_HEIGHT, FENCE_MIN_WIDTH, FenceLayoutMode, FenceSnapshot, FenceSortMode, UnclassifiedItemSnapshot, WORKSPACE_BACKUP_SCHEMA_VERSION, WORKSPACE_GRID_BOTTOM_MARGIN, WORKSPACE_GRID_LEFT, WORKSPACE_GRID_TOP, WORKSPACE_SCHEMA_VERSION, WorkspaceBackupSummary, WorkspaceCommand, WorkspaceError, WorkspaceFenceScroll, WorkspaceFocusTarget, WorkspaceGrid, WorkspaceInteractionCommand, WorkspaceInteractionSnapshot, WorkspaceItemPlacement, WorkspaceMonitorGrid, WorkspacePoint, WorkspaceRect, WorkspaceRestoreResult, WorkspaceSelectionMode, WorkspaceSnapshot};
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
    pub transfer_assignments: bool,                 // 确定的投放新增位置作用于工作区，不重导入 Explorer 布局。
    pub monitors: &'a [DesktopHostMonitor],         // 当前逐屏物理环境。
    pub items: &'a [crate::contracts::DesktopItem], // 同次 Shell 观察的引用与名称。
    pub assignments: &'a [(String, String)],        // 初次导入可用的 Explorer 归属。
    pub renames: &'a [(String, String)],            // Shell 确认的精确重命名。
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
    pub fn reconcile_host_monitors(&self, monitors: &[DesktopHostMonitor], item_ids: &[String], assignments: &[(String, String)], renames: &[(String, String)]) -> Result<WorkspaceSnapshot, WorkspaceError> { self.reconcile_environment_state(monitors, item_ids, assignments, renames, None, false) }

    /// 将同次环境观察的名称与布局作为一次协调提交。
    pub(crate) fn reconcile_environment(&self, environment: WorkspaceEnvironment<'_>) -> Result<WorkspaceSnapshot, WorkspaceError>
    {
        let ids = environment.items.iter().map(|item| item.stable_reference.clone()).collect::<Vec<_>>(); // 完整项目引用。
        let names = environment.items.iter().map(|item| (item.stable_reference.clone(), item.display_name.to_lowercase())).collect(); // 本次排序事实。
        self.reconcile_environment_state(environment.monitors, &ids, environment.assignments, environment.renames, Some(names), environment.transfer_assignments)
    }

    /// 在既有锁顺序下协调布局，持久化成功后才更新名称缓存。
    fn reconcile_environment_state(&self, monitors: &[DesktopHostMonitor], item_ids: &[String], assignments: &[(String, String)], renames: &[(String, String)], observed_names: Option<HashMap<String, String>>, transfer_assignments: bool) -> Result<WorkspaceSnapshot, WorkspaceError>
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
        let current_assignments = if next_imported && !transfer_assignments { &[][..] } else { assignments }; // 投放新增位置与首次 Explorer 导入分开。
        let mut changed = reconcile_layout_snapshot(&mut next, &monitor_grids, primary_grid, monitors, item_ids, current_assignments, renames, &primary_monitor_id)?; // 统一工作区的 Shell 与显示器协调结果。
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
    let target_ids = target.unclassified_items.iter().map(|item| item.stable_reference.clone()).collect::<std::collections::HashSet<_>>(); // 历史事务原本覆盖的项目引用。
    let restored_count = target_ids.iter().filter(|item_id| current_items.contains_key(item_id.as_str())).count(); // 当前仍存在并可以恢复历史布局的项目数量。
    let changed_count = target_ids.len().saturating_sub(restored_count); // 已离开当前工作区的历史项目数量。
    target.unclassified_items.retain(|item| current_items.contains_key(item.stable_reference.as_str()));
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
        fence.members.retain(|item_id| current_items.contains_key(item_id.as_str()));
        if let Some(current_fence) = current.fences.iter().find(|current| current.id == fence.id)
        {
            fence.projection_monitor_id.clone_from(&current_fence.projection_monitor_id);
            fence.projection_bounds = current_fence.projection_bounds;
        }
    }
    target.unclassified_items.extend(current.unclassified_items.iter().filter(|item| !target_ids.contains(&item.stable_reference)).cloned()); // 历史提交后由 Shell 新增的项目保持当前布局。
    target.grid = current.grid;
    target.monitor_grids.clone_from(&current.monitor_grids);
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
            let Some(item_index) = self.unclassified_items.iter().position(|item| item.stable_reference == *old_reference)
            else
            {
                continue;
            };
            if self.unclassified_items.iter().any(|candidate| candidate.stable_reference == *new_reference)
            {
                return Err(WorkspaceError::invalid_command("重命名后项目标识已经存在"));
            }
            self.unclassified_items[item_index].stable_reference.clone_from(new_reference);
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
                if !self.unclassified_items.iter().any(|item| item.stable_reference == item_id && self.item_visible_on_monitor(&item_id, monitor_id))
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
                    if !self.unclassified_items.iter().any(|item| item.stable_reference == item_id && self.item_visible_on_monitor(&item_id, monitor_id))
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
        self.unclassified_items.retain(|item| active_ids.contains(item.stable_reference.as_str()) || member_ids.contains(item.stable_reference.as_str()));

        let mut occupied = self.monitor_grids.iter().map(|entry| (entry.monitor_id.clone(), self.unclassified_items.iter().filter(|item| item.monitor_id == entry.monitor_id).map(|item| item.position).collect::<Vec<_>>())).collect::<std::collections::HashMap<_, _>>(); // 每台显示器已占用的持久网格位置。
        for item_id in item_ids
        {
            validate_id(item_id, "桌面项目标识无效")?;
            if self.unclassified_items.iter().any(|item| item.stable_reference == *item_id)
            {
                continue;
            }

            let monitor_id = assignments.get(item_id.as_str()).filter(|monitor_id| active_monitor_ids.contains(**monitor_id)).copied().unwrap_or(primary_monitor_id); // 新项目按 Explorer 位置或主屏回退获得的持久归属。
            let grid = self.monitor_grids.iter().find(|entry| entry.monitor_id == monitor_id).ok_or_else(|| WorkspaceError::invalid_command("项目显示器网格不存在"))?.grid; // 当前新项目所属显示器的网格。
            let capacity = grid_capacity(grid)?; // 当前显示器网格的安全容量。
            let monitor_occupied = occupied.get_mut(monitor_id).expect("活动显示器已创建占用集合"); // 当前显示器已经占用的位置。
            // 新增项目逐行从左上扫描，保留已有项目和拖动网格的顺序语义。
            let position = (0..capacity).map(|index| WorkspacePoint { left: grid.left + f64::from(index % grid.columns) * grid.column_width, top: grid.top + f64::from(index / grid.columns) * grid.row_height }).find(|position| !monitor_occupied.contains(position)).ok_or_else(|| WorkspaceError::invalid_command("未归类网格空间不足"))?;
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
        let viewport_width = f64::from(primary_monitor.work_width) / primary_monitor.scale_factor; // 主屏 CSS 工作区宽度。
        let viewport_height = f64::from(primary_monitor.work_height) / primary_monitor.scale_factor; // 主屏 CSS 工作区高度。
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
            component.projection_bounds = Some(project_component_rect(component.bounds, viewport_width, viewport_height));
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
                validate_component_rect(bounds)?;
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
            WorkspaceCommand::UpdateFolderPortalBounds { portal_id, bounds } =>
            {
                validate_component_rect(bounds)?;
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
                validate_rect(bounds)?;
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
            WorkspaceCommand::UpdateFenceBounds { fence_id, bounds } =>
            {
                validate_rect(bounds)?;
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
                let target_monitor_id = self.fence(&fence_id)?.monitor_id.clone(); // 目标围栏持久归属的显示器身份。
                if !self.unclassified_items.iter().any(|item| item.stable_reference == item_id)
                {
                    return Err(WorkspaceError::invalid_command("桌面项目缺少未归类来源位置"));
                }
                if before_item_id.as_deref() == Some(item_id.as_str())
                {
                    return Ok("保持围栏成员顺序");
                }
                for fence in &mut self.fences
                {
                    fence.members.retain(|member| member != &item_id);
                }
                if let Some(item) = self.unclassified_items.iter_mut().find(|item| item.stable_reference == item_id)
                {
                    item.monitor_id = target_monitor_id;
                    item.projection_monitor_id = None;
                    item.projection_position = None;
                }
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
                for fence in &mut self.fences
                {
                    fence.members.retain(|member| member != &item_id);
                }
                if let Some(item) = self.unclassified_items.iter_mut().find(|item| item.stable_reference == item_id)
                {
                    item.monitor_id = target_monitor_id;
                    item.projection_monitor_id = None;
                    item.projection_position = None;
                }
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
                    if !dragged_ids.insert(item_id.clone()) || !self.unclassified_items.iter().any(|item| item.stable_reference == *item_id)
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
                    let target_monitor_id = self.fence(&fence_id)?.monitor_id.clone(); // 目标围栏持久归属的显示器身份。
                    for item in self.unclassified_items.iter_mut().filter(|item| dragged_ids.contains(&item.stable_reference))
                    {
                        item.monitor_id.clone_from(&target_monitor_id);
                        item.projection_monitor_id = None;
                        item.projection_position = None;
                    }
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
                    self.repair_fence_member_source_collisions(&target_monitor_id, &std::collections::HashSet::new())?;
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
                        let item = self.unclassified_items.iter_mut().find(|item| item.stable_reference == item_id).ok_or_else(|| WorkspaceError::invalid_command("网格位置包含未知项目"))?;
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
                    self.repair_fence_member_source_collisions(&target_monitor_id, &dragged_ids)?;
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
                    let source = self.unclassified_items.iter_mut().find(|item| item.stable_reference == member).ok_or_else(|| WorkspaceError::invalid_command("围栏成员来源位置状态无效"))?;
                    source.position = position;
                    occupied.push(position);
                    next_index = (available_index + 1) % capacity;
                }
                Ok("解散围栏")
            }
            WorkspaceCommand::UpdateComponentBounds { instance_id, bounds } =>
            {
                validate_component_rect(bounds)?;
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
        let item_ids = self.unclassified_items.iter().map(|item| item.stable_reference.as_str()).collect::<std::collections::HashSet<_>>();
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
            if !selected_ids.insert(item_id) || !self.unclassified_items.iter().any(|item| &item.stable_reference == item_id)
            {
                return Err(WorkspaceError::invalid_command("选择集合包含未知项目"));
            }
        }
        if let Some(anchor_id) = &self.interaction.selection_anchor_id
        {
            validate_id(anchor_id, "选择锚点标识无效")?;
            if !self.unclassified_items.iter().any(|item| &item.stable_reference == anchor_id)
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
            validate_rect(fence.bounds)?;
            if !fence.scroll_top.is_finite() || fence.scroll_top < 0.0 || !fence.last_expanded_height.is_finite() || fence.last_expanded_height < FENCE_MIN_HEIGHT || !fence_ids.insert(&fence.id)
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
        // 当前快照中已经出现的来源位置项目标识。
        let source_ids: std::collections::HashSet<_> = self.unclassified_items.iter().map(|item| &item.stable_reference).collect();
        // 当前快照中已经占用的未归类来源位置。
        let source_positions = self.unclassified_items.iter().map(|item| (item.monitor_id.as_str(), item.position.left.to_bits(), item.position.top.to_bits())).collect::<std::collections::HashSet<_>>();
        if source_ids.len() != self.unclassified_items.len() || source_positions.len() != self.unclassified_items.len() || member_ids.iter().any(|member| !source_ids.contains(member))
        {
            return Err(WorkspaceError::invalid_command("围栏成员来源位置状态无效"));
        }
        let mut portal_ids = std::collections::HashSet::new(); // 路径可以重复，实例身份必须唯一。
        for portal in &self.folder_portals
        {
            validate_id(&portal.id, "门户身份无效")?;
            validate_component_rect(portal.bounds)?;
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
            validate_component_rect(component.bounds)?;
            if !component_ids.insert(component.instance_id.as_str()) || component.state_version == 0 || serde_json::to_vec(&component.state).map_err(|error| WorkspaceError::invalid_command(format!("组件状态无效: {error}")))?.len() > 64 * 1024
            {
                return Err(WorkspaceError::invalid_command("组件持久化状态无效"));
            }
            if let Some(projection_monitor_id) = &component.projection_monitor_id
            {
                validate_id(projection_monitor_id, "组件投射显示器标识无效")?;
                validate_component_rect(component.projection_bounds.ok_or_else(|| WorkspaceError::invalid_command("组件投射边界缺失"))?)?;
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
                if !source_ids.contains(item_id)
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

    /// 为围栏成员重新分配被新网格位置占用的来源槽位。
    fn repair_fence_member_source_collisions(&mut self, monitor_id: &str, dragged_ids: &std::collections::HashSet<String>) -> Result<(), WorkspaceError>
    {
        let projection = self.fences.iter().find(|fence| fence.monitor_id == monitor_id && fence.projection_monitor_id.is_some()).and_then(|fence| fence.projection_monitor_id.as_deref()); // 离线屏围栏使用当前可见投影的容量。
        let grid = self.monitor_grids.iter().find(|entry| entry.monitor_id == monitor_id).or_else(|| self.monitor_grids.iter().find(|entry| Some(entry.monitor_id.as_str()) == projection)).map(|entry| entry.grid).unwrap_or(self.grid);
        let capacity = grid_capacity(grid)?;
        let fence_members = self.fences.iter().flat_map(|fence| fence.members.iter()).collect::<std::collections::HashSet<_>>();
        let mut occupied = std::collections::HashSet::new();
        for item in &self.unclassified_items
        {
            if item.monitor_id == monitor_id && (dragged_ids.contains(&item.stable_reference) || !fence_members.contains(&item.stable_reference))
            {
                occupied.insert((item.position.left.to_bits(), item.position.top.to_bits()));
            }
        }
        for item in &mut self.unclassified_items
        {
            if item.monitor_id != monitor_id || dragged_ids.contains(&item.stable_reference) || !fence_members.contains(&item.stable_reference)
            {
                continue;
            }
            let key = (item.position.left.to_bits(), item.position.top.to_bits());
            if occupied.insert(key)
            {
                continue;
            }
            let Some(position) = (0..capacity).map(|index| grid_position(grid, index)).find(|candidate| occupied.insert((candidate.left.to_bits(), candidate.top.to_bits())))
            else
            {
                return Err(WorkspaceError::invalid_command("未归类网格空间不足"));
            };
            item.position = position;
        }
        Ok(())
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

/// 校验围栏边界与原型一致的最小尺寸。
fn validate_rect(bounds: WorkspaceRect) -> Result<(), WorkspaceError>
{
    if !bounds.left.is_finite() || !bounds.top.is_finite() || !bounds.width.is_finite() || !bounds.height.is_finite() || bounds.width < FENCE_MIN_WIDTH || bounds.height < FENCE_MIN_HEIGHT
    {
        return Err(WorkspaceError::invalid_command("围栏边界无效"));
    }
    Ok(())
}

/// 校验组件表面的自由位置和最小可交互尺寸。
fn validate_component_rect(bounds: WorkspaceRect) -> Result<(), WorkspaceError>
{
    if !bounds.left.is_finite() || !bounds.top.is_finite() || !bounds.width.is_finite() || !bounds.height.is_finite() || bounds.width < 80.0 || bounds.height < 48.0
    {
        return Err(WorkspaceError::invalid_command("组件边界无效"));
    }
    Ok(())
}

/// 将断开显示器上的围栏临时投射到目标工作区可见范围内。
fn project_rect(bounds: WorkspaceRect, viewport_width: f64, viewport_height: f64) -> WorkspaceRect
{
    let width = bounds.width.min(viewport_width).max(FENCE_MIN_WIDTH.min(viewport_width)); // 临时投射后的可见宽度。
    let height = bounds.height.min(viewport_height).max(FENCE_MIN_HEIGHT.min(viewport_height)); // 临时投射后的可见高度。
    WorkspaceRect { left: bounds.left.clamp(0.0, (viewport_width - width).max(0.0)), top: bounds.top.clamp(0.0, (viewport_height - height).max(0.0)), width, height }
}

/// 将断开显示器上的组件投射到主屏可见范围内。
fn project_component_rect(bounds: WorkspaceRect, viewport_width: f64, viewport_height: f64) -> WorkspaceRect
{
    let width = bounds.width.min(viewport_width).max(80.0_f64.min(viewport_width)); // 临时投射后的组件可见宽度。
    let height = bounds.height.min(viewport_height).max(48.0_f64.min(viewport_height)); // 临时投射后的组件可见高度。
    WorkspaceRect { left: bounds.left.clamp(0.0, (viewport_width - width).max(0.0)), top: bounds.top.clamp(0.0, (viewport_height - height).max(0.0)), width, height }
}

#[cfg(test)]
mod tests
{
    use super::{AUTO_SNAPSHOT_LIMIT, PersistedWorkspacePayload, WorkspaceState, decode_workspace_store, merge_layout_history_target, workspace_store_from_payload, write_auto_snapshot};
    use crate::contracts::{ComponentInstanceSnapshot, DesktopHostMonitor, UnclassifiedItemSnapshot, WorkspaceCommand, WorkspaceFocusTarget, WorkspaceInteractionCommand, WorkspaceItemPlacement, WorkspacePoint, WorkspaceRect, WorkspaceSelectionMode, WorkspaceSnapshot};
    use std::time::{SystemTime, UNIX_EPOCH};

    /// 相同路径的独立身份、几何、移除和重启恢复通过真实持久化路径验证。
    #[test]
    fn folder_portals_persist_independent_duplicate_paths()
    {
        let path = snapshot_path("folder-portals"); // 独立测试布局文件。
        let root_path = std::env::temp_dir().to_string_lossy().into_owned(); // 已存在且不写入内容的目录。
        let bounds = WorkspaceRect { left: 24.0, top: 24.0, width: 360.0, height: 280.0 }; // 初始几何。
        let state = WorkspaceState::load(path.clone()).unwrap(); // 使用真实原子保存。
        state.apply(WorkspaceCommand::CreateFolderPortal { root_path: root_path.clone(), bounds }).unwrap();
        let created = state.apply(WorkspaceCommand::CreateFolderPortal { root_path, bounds }).unwrap(); // 同路径始终追加。
        assert_eq!(created.folder_portals.len(), 2);
        assert_ne!(created.folder_portals[0].id, created.folder_portals[1].id);
        let moved = WorkspaceRect { left: 100.0, ..bounds }; // 只修改第一个实例。
        state.apply(WorkspaceCommand::UpdateFolderPortalBounds { portal_id: created.folder_portals[0].id.clone(), bounds: moved }).unwrap();
        state.apply(WorkspaceCommand::SetFolderPortalAppearance { portal_id: created.folder_portals[0].id.clone(), change: crate::contracts::TileAppearanceChange::Collapsed(true) }).unwrap(); // 外观不占据布局撤销步骤。
        let undone = state.apply(WorkspaceCommand::UndoLayout).unwrap(); // 撤销移动，保留当前外观。
        assert_eq!(undone.folder_portals[0].bounds, bounds);
        assert!(undone.folder_portals[0].appearance.collapsed);
        state.apply(WorkspaceCommand::RedoLayout).unwrap();
        drop(state);
        let restored = WorkspaceState::load(path.clone()).unwrap(); // 重启恢复权威工作区。
        let snapshot = restored.snapshot().unwrap(); // 断言路径和几何均独立。
        assert_eq!(snapshot.folder_portals[0].bounds, moved);
        assert!(snapshot.folder_portals[0].appearance.collapsed);
        assert_eq!(snapshot.folder_portals[1].bounds, bounds);
        let removed = restored.apply(WorkspaceCommand::RemoveFolderPortal { portal_id: snapshot.folder_portals[0].id.clone() }).unwrap();
        assert_eq!(removed.folder_portals.len(), 1);
        assert!(std::path::Path::new(&removed.folder_portals[0].root_path).is_dir());
    }

    /// 创建当前测试独占的工作区快照路径。
    fn snapshot_path(name: &str) -> std::path::PathBuf
    {
        // 避免并发测试共享文件的纳秒时间标识。
        let unique = SystemTime::now().duration_since(UNIX_EPOCH).expect("系统时间应晚于 Unix epoch").as_nanos();
        std::env::temp_dir().join(format!("desktop-tools-{name}-{}-{unique}.json", std::process::id()))
    }

    /// 创建当前测试使用的工作区状态。
    fn workspace_state(path: std::path::PathBuf) -> WorkspaceState { WorkspaceState::load(path).expect("测试工作区应可启动") }

    /// 环境重复观察不改变统一布局、选择或撤销，保存和还原只包含同一布局。
    #[test]
    fn unified_workspace_preserves_layout_interaction_and_history()
    {
        let path = snapshot_path("unified-workspace"); // 独占持久文件。
        let backup = snapshot_path("unified-backup"); // 独占手动备份。
        let mut state = WorkspaceState::load(path.clone()).unwrap(); // 唯一工作区不接收系统桌面身份。
        let monitors = [monitor("DISPLAY1", true, 0, 1.0)]; // 固定物理显示器。
        let items = ["one".to_string(), "two".to_string()]; // 固定 Shell 事实。
        state.reconcile_host_monitors(&monitors, &items, &[], &[]).unwrap();
        let bounds = WorkspaceRect { left: 40.0, top: 40.0, width: 320.0, height: 240.0 }; // 初始围栏位置。
        let created = state.apply(WorkspaceCommand::CreateFence { title: "统一".into(), bounds }).unwrap(); // 创建唯一围栏。
        let fence_id = created.fences[0].id.clone(); // 后续事务使用稳定身份。
        state.apply(WorkspaceCommand::MoveItemToFence { item_id: "one".into(), fence_id: fence_id.clone(), before_item_id: None }).unwrap();
        let moved_bounds = WorkspaceRect { left: 500.0, ..bounds }; // 成员操作后的空间事务。
        state.apply(WorkspaceCommand::UpdateFenceBounds { fence_id: fence_id.clone(), bounds: moved_bounds }).unwrap();
        state.apply_interaction_on_monitor("DISPLAY1", WorkspaceInteractionCommand::FocusItem { item_id: "two".into(), selection_mode: WorkspaceSelectionMode::Replace }).unwrap();
        let before = state.snapshot().unwrap(); // 选择、位置、成员共同保留。
        let time = state.latest_layout_history_time(false).unwrap(); // 环境重复观察不产生历史。
        for _ in 0..3
        {
            assert_eq!(state.reconcile_host_monitors(&monitors, &items, &[], &[]).unwrap(), before);
        }
        assert_eq!(state.latest_layout_history_time(false).unwrap(), time);
        let undone = state.apply(WorkspaceCommand::UndoLayout).unwrap(); // 空间先于成员撤销。
        assert_eq!(undone.fences[0].bounds, bounds);
        assert_eq!(undone.fences[0].members, ["one"]);
        assert_eq!(undone.interaction.selected_item_ids, ["two"]);
        assert!(state.apply(WorkspaceCommand::UndoLayout).unwrap().fences[0].members.is_empty());
        state.apply(WorkspaceCommand::RedoLayout).unwrap();
        let restored = state.apply(WorkspaceCommand::RedoLayout).unwrap(); // 同一时间线重做恢复布局。
        assert_eq!(restored.fences, before.fences);
        state.write_manual_backup(&backup).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap(); // 磁盘唯一布局。
        assert_eq!(value["storeVersion"], 5);
        assert_eq!(value["payload"]["layout"]["fences"][0]["members"], serde_json::json!(["one"]));
        assert!(value["payload"].get("layouts").is_none());
        assert!(value["payload"].get("currentVirtualDesktopId").is_none());
        state.apply(WorkspaceCommand::UpdateFenceBounds { fence_id, bounds }).unwrap();
        let modified = state.snapshot().unwrap(); // 注入保存失败后应完整保留。
        let history_time = state.latest_layout_history_time(false).unwrap(); // 失败不能清空历史。
        state.persistence_path = path.join("blocked.json");
        assert!(state.restore_backup(&backup, &monitors, &items).is_err());
        assert_eq!(state.snapshot().unwrap(), modified);
        assert_eq!(state.latest_layout_history_time(false).unwrap(), history_time);
        state.persistence_path = path.clone();
        assert_eq!(state.restore_backup(&backup, &monitors, &items).unwrap().snapshot.fences, before.fences);
        assert!(state.latest_layout_history_time(false).unwrap().is_none());
        drop(state);
        let reopened = WorkspaceState::load(path.clone()).unwrap(); // 重启不依赖系统桌面身份。
        assert_eq!(reopened.snapshot().unwrap().fences, before.fences);
        assert!(reopened.snapshot().unwrap().interaction.selected_item_ids.is_empty());
        drop(reopened);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(backup).unwrap();
    }

    /// 验证环境观察失败时名称、布局和磁盘保持原状态，重试可整体提交。
    #[test]
    fn environment_observation_rolls_back_names_with_layout()
    {
        let path = snapshot_path("environment-transaction"); // 测试独占持久文件。
        let mut state = workspace_state(path.clone()); // 可注入保存失败的工作区。
        let monitors = [monitor("DISPLAY1", true, 0, 1.0)]; // 当前宿主事实。
        let items = vec![crate::contracts::DesktopItem { stable_reference: "a".into(), display_name: "Before".into(), system_image: None, availability: crate::contracts::DesktopItemAvailability::Available }]; // 首次 Shell 观察。
        state.reconcile_environment(super::WorkspaceEnvironment { transfer_assignments: false, monitors: &monitors, items: &items, assignments: &[], renames: &[] }).unwrap();
        let before = state.snapshot().unwrap(); // 原始权威布局。
        let disk = std::fs::read(&path).unwrap(); // 已保存的原始快照。
        let mut next_items = items.clone(); // Shell 确认的重命名结果。
        next_items[0].stable_reference = "b".into();
        next_items[0].display_name = "After".into();
        let renames = [("a".into(), "b".into())]; // 精确身份变化。
        state.persistence_path = path.join("blocked.json");
        assert!(state.reconcile_environment(super::WorkspaceEnvironment { transfer_assignments: false, monitors: &monitors, items: &next_items, assignments: &[], renames: &renames }).is_err());
        assert_eq!(state.snapshot().unwrap(), before);
        assert_eq!(state.item_names.lock().unwrap().get("a").unwrap(), "before");
        assert_eq!(std::fs::read(&path).unwrap(), disk);
        state.persistence_path = path.clone();
        let result = state.reconcile_environment(super::WorkspaceEnvironment { transfer_assignments: false, monitors: &monitors, items: &next_items, assignments: &[], renames: &renames }).unwrap(); // 同一观察重试成功。
        assert_eq!(result.unclassified_items[0].stable_reference, "b");
        assert_eq!(state.item_names.lock().unwrap().get("b").unwrap(), "after");
        drop(state);
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir_all(path.with_extension("snapshots")).unwrap();
    }

    /// 共享标题和悬停策略持久化，撤销空间移动不得恢复旧外观。
    #[test]
    fn fence_title_and_hover_policy_persist_and_undo()
    {
        let path = snapshot_path("fence-options"); // 独占配置。
        let state = workspace_state(path.clone()); // 测试工作区。
        let snapshot = state.apply(WorkspaceCommand::CreateFence { title: "旧标题".into(), bounds: WorkspaceRect { left: 10.0, top: 10.0, width: 320.0, height: 240.0 } }).unwrap(); // 初始围栏。
        let id = snapshot.fences[0].id.clone(); // 稳定围栏身份。
        state.apply(WorkspaceCommand::UpdateFenceBounds { fence_id: id.clone(), bounds: WorkspaceRect { left: 100.0, ..snapshot.fences[0].bounds } }).unwrap(); // 建立独立空间历史。
        let rename = serde_json::from_value(serde_json::json!({"type":"rename-fence", "fenceId":id, "title":"新标题"})).unwrap(); // 前端实际命令格式。
        state.apply(rename).unwrap();
        let enable = serde_json::from_value(serde_json::json!({"type":"set-fence-auto-expand", "fenceId":id, "enabled":true})).unwrap(); // 前端开关命令。
        state.apply(enable).unwrap();
        assert!(state.snapshot().unwrap().fences[0].auto_expand);
        assert_eq!(workspace_state(path.clone()).snapshot().unwrap().fences[0].title, "新标题");
        state.apply(WorkspaceCommand::UndoLayout).unwrap();
        assert!(state.snapshot().unwrap().fences[0].auto_expand);
        assert_eq!(state.snapshot().unwrap().fences[0].bounds, snapshot.fences[0].bounds);
        assert_eq!(state.snapshot().unwrap().fences[0].title, "新标题");
        let empty = state.apply(WorkspaceCommand::RenameFence { fence_id: id.clone(), title: "  ".into() }).unwrap(); // 空白标题规范化为空标题。
        assert_eq!(empty.fences[0].title, "");
        assert!(state.apply(WorkspaceCommand::RenameFence { fence_id: id, title: "\0".into() }).is_err());
        drop(state);
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir_all(path.with_extension("snapshots")).unwrap();
    }

    /// 新增项目逐行填补空位，保留已有布局位置。
    #[test]
    fn new_desktop_items_fill_rows_and_reuse_holes()
    {
        let path = snapshot_path("new-items-row-order"); // 测试独占布局文件。
        let state = workspace_state(path.clone()); // 当前权威工作区。
        let first = state.reconcile_items(400.0, 400.0, &["a".into(), "b".into(), "c".into()]).unwrap(); // 首次枚举。
        let positions = first.unclassified_items.iter().map(|item| item.position).collect::<Vec<_>>(); // 初始逐行位置。
        assert_eq!(positions[0].top, positions[1].top);
        assert_eq!(positions[1].left - positions[0].left, first.grid.column_width);
        state.reconcile_items(400.0, 400.0, &["a".into(), "c".into()]).unwrap();
        let next = state.reconcile_items(400.0, 400.0, &["a".into(), "c".into(), "d".into()]).unwrap(); // 补充新项目。
        assert_eq!(next.unclassified_items.iter().find(|item| item.stable_reference == "d").unwrap().position, positions[1]);
        assert_eq!(next.unclassified_items.iter().find(|item| item.stable_reference == "c").unwrap().position, positions[2]);
        drop(state);
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir_all(path.with_extension("snapshots")).unwrap();
    }

    /// 独立网格设置覆盖运行、重启及容量不足不提交的完整路径。
    #[test]
    fn grid_preferences_reflow_and_reject_insufficient_capacity()
    {
        let path = snapshot_path("grid-preferences"); // 测试独占布局文件。
        let settings_path = snapshot_path("grid-settings"); // 测试独占偏好文件。
        let state = workspace_state(path.clone()); // 原子持久化工作区。
        let settings = crate::app::settings::SettingsState::load(settings_path.clone()).unwrap(); // 首次启动配置。
        let monitors = [monitor("DISPLAY1", true, 0, 1.5)]; // 混合 DPI 计算中的 150% 显示器。
        let items = (0..30).map(|index| format!("item-{index}")).collect::<Vec<_>>(); // 新大网格不能全部容纳的项目数。
        state.reconcile_host_monitors(&monitors, &items, &[], &[]).unwrap();
        let mut preferences = settings.preferences().unwrap(); // 等待修改的配置。
        preferences.desktop_grid.width = 100;
        state.save_preferences(&settings, preferences.clone(), &monitors).unwrap();
        let snapshot = state.snapshot().unwrap(); // 已应用的权威布局。
        assert_eq!(snapshot.grid.column_width, 110.0);
        assert_eq!(snapshot.grid.row_height, 98.0);
        assert_eq!(snapshot.unclassified_items[0].position.left, 0.0);
        assert_eq!(settings.preferences().unwrap().fence_grid, crate::contracts::settings::ApplicationPreferences::default().fence_grid);
        preferences.desktop_grid = crate::contracts::settings::GridPreferences { width: 160, height: 180, column_gap: 48, row_gap: 48 };
        assert!(state.save_preferences(&settings, preferences, &monitors).is_err());
        assert_eq!(state.snapshot().unwrap(), snapshot);
        assert_eq!(settings.preferences().unwrap().desktop_grid.width, 100);
        let blocked_path = snapshot_path("blocked-grid-settings"); // 写入时才变成目录的配置目标。
        let blocked_settings = crate::app::settings::SettingsState::load(blocked_path.clone()).unwrap(); // 可读取但随后无法替换目标的配置状态。
        std::fs::create_dir(&blocked_path).unwrap();
        let mut rejected = settings.preferences().unwrap(); // 会触发重排但最终保存失败的配置。
        rejected.desktop_grid.width = 110;
        assert!(state.save_preferences(&blocked_settings, rejected, &monitors).is_err());
        assert_eq!(state.snapshot().unwrap(), snapshot);
        std::fs::remove_dir(blocked_path).unwrap();
        drop(state);
        let restored = workspace_state(path.clone()); // 重启恢复旧格位与新步进。
        restored.initialize_grid_preferences(settings.preferences().unwrap().desktop_grid).unwrap();
        let next = restored.reconcile_host_monitors(&monitors, &items, &[], &[]).unwrap(); // 环境变化不回退默认网格。
        assert_eq!(next.grid.column_width, 110.0);
        assert_eq!(next.unclassified_items, snapshot.unclassified_items);
        drop(restored);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(settings_path).unwrap();
    }

    /// 创建逐屏工作区测试使用的显示器物理状态。
    fn monitor(id: &str, primary: bool, x: i32, scale_factor: f64) -> DesktopHostMonitor { DesktopHostMonitor { id: id.to_string(), name: id.to_string(), primary, x, y: 0, width: 1920, height: 1080, work_x: x, work_y: 0, work_width: 1920, work_height: 1040, scale_factor, dpi: (96.0 * scale_factor).round() as u32 } }

    /// 验证官方组件边界、状态和显示器归属可恢复，并在原显示器缺失时仅临时投射。
    #[test]
    fn component_layout_persists_and_projects_without_losing_ownership()
    {
        let path = snapshot_path("component-layout"); // 当前测试使用的持久工作区文件。
        let snapshot_directory = path.with_extension("snapshots"); // 当前测试可能生成的自动快照目录。
        let default_component = ComponentInstanceSnapshot { instance_id: "official.local-clock.default".to_string(), component_id: "official.local-clock".to_string(), monitor_id: String::new(), projection_monitor_id: None, projection_bounds: None, bounds: WorkspaceRect { left: 0.0, top: 0.0, width: 220.0, height: 116.0 }, state_version: 1, state: serde_json::json!({"format": "24-hour"}) }; // 组件运行时注册的官方默认实例。
        let state = WorkspaceState::load_with_components(path.clone(), std::slice::from_ref(&default_component)).expect("包含官方组件的工作区应可启动");
        state.reconcile_host_monitors(&[monitor("DISPLAY1", true, 0, 1.0), monitor("DISPLAY2", false, 1920, 1.25)], &[], &[], &[]).expect("官方组件应可进入主显示器默认布局");
        state.apply_on_monitor(Some("DISPLAY2"), WorkspaceCommand::UpdateComponentBounds { instance_id: default_component.instance_id.clone(), bounds: WorkspaceRect { left: 140.0, top: 96.0, width: 240.0, height: 128.0 } }).expect("组件应可迁移到第二显示器");
        state.apply_on_monitor(Some("DISPLAY2"), WorkspaceCommand::UpdateComponentState { instance_id: default_component.instance_id.clone(), state_version: 1, state: serde_json::json!({"format": "12-hour"}) }).expect("组件版本化状态应可保存");
        let undone = state.apply(WorkspaceCommand::UndoLayout).unwrap(); // 只撤销空间移动。
        assert_eq!(undone.components[0].monitor_id, "DISPLAY1");
        assert_eq!(undone.components[0].state["format"], "12-hour");
        let redone = state.apply(WorkspaceCommand::RedoLayout).unwrap(); // 同一历史恢复位置，不覆盖私有状态。
        assert_eq!(redone.components[0].monitor_id, "DISPLAY2");
        assert_eq!(redone.components[0].state["format"], "12-hour");
        drop(state);

        let restored = WorkspaceState::load_with_components(path.clone(), std::slice::from_ref(&default_component)).expect("组件布局应可从工作区文件恢复");
        let restored_snapshot = restored.snapshot().expect("恢复后的组件快照应可读取");
        assert_eq!(restored_snapshot.components[0].monitor_id, "DISPLAY2");
        assert_eq!(restored_snapshot.components[0].bounds.width, 240.0);
        assert_eq!(restored_snapshot.components[0].state["format"], "12-hour");

        let projected = restored.reconcile_host_monitors(&[monitor("DISPLAY1", true, 0, 1.0)], &[], &[], &[]).expect("缺失显示器上的组件应可临时投射");
        assert_eq!(projected.components[0].monitor_id, "DISPLAY2");
        assert_eq!(projected.components[0].projection_monitor_id.as_deref(), Some("DISPLAY1"));
        assert!(projected.components[0].projection_bounds.is_some());
        drop(restored);

        std::fs::remove_file(path).expect("组件测试快照应可清理");
        if snapshot_directory.exists()
        {
            std::fs::remove_dir_all(snapshot_directory).expect("组件测试自动快照目录应可清理");
        }
    }

    /// 验证排序事务覆盖归属、拖动、重命名、历史和持久化边界。
    #[test]
    fn fence_sorting_is_persisted_and_applies_to_every_membership_path()
    {
        use crate::contracts::{DesktopItem, DesktopItemAvailability, FenceSortMode};
        let path = snapshot_path("fence-sorting"); // 本测试独占的持久化文件。
        let state = workspace_state(path.clone()); // 使用真实布局事务和原子持久化的工作区。
        let mut items = [("a", "Zulu"), ("b", "alpha"), ("c", "Bravo"), ("d", "ALPHA")].map(|(id, name)| DesktopItem { stable_reference: id.to_string(), display_name: name.to_string(), system_image: None, availability: DesktopItemAvailability::Available }); // 显示名称与引用顺序故意不同，并包含忽略大小写的同名项目。
        let ids = items.iter().map(|item| item.stable_reference.clone()).collect::<Vec<_>>(); // 当前完整 Shell 引用集合。
        state.update_item_names(&items).unwrap();
        state.reconcile_items(1280.0, 720.0, &ids).unwrap();
        let fence_id = state.apply(WorkspaceCommand::CreateFence { title: "排序验收".to_string(), bounds: WorkspaceRect { left: 100.0, top: 80.0, width: 320.0, height: 240.0 } }).unwrap().fences[0].id.clone(); // 本测试目标围栏。
        for id in ["a", "b"]
        {
            state.apply(WorkspaceCommand::MoveItemToFence { item_id: id.to_string(), fence_id: fence_id.clone(), before_item_id: None }).unwrap();
        }
        let ascending = state.apply(WorkspaceCommand::SetFenceSort { fence_id: fence_id.clone(), sort_mode: FenceSortMode::NameAscending }).unwrap(); // 首次名称排序结果。
        assert_eq!(ascending.fences[0].members, ["b", "a"]);
        let grouped = state.apply(WorkspaceCommand::CommitItemDrag { anchor_item_id: "c".to_string(), item_ids: vec!["c".to_string(), "d".to_string()], placements: vec![], target_fence_id: Some(fence_id.clone()), before_item_id: Some("stale-target".to_string()), fence_scrolls: vec![] }).unwrap(); // 排序模式忽略手动插入位置，包括旧前端快照中的失效目标。
        assert_eq!(grouped.fences[0].members, ["b", "d", "c", "a"]);
        assert_eq!(state.apply(WorkspaceCommand::UndoLayout).unwrap().fences[0].members, ["b", "a"]);
        assert_eq!(state.apply(WorkspaceCommand::RedoLayout).unwrap().fences[0].members, ["b", "d", "c", "a"]);
        let descending = state.apply(WorkspaceCommand::SetFenceSort { fence_id: fence_id.clone(), sort_mode: FenceSortMode::NameDescending }).unwrap(); // 再次名称排序结果。
        assert_eq!(descending.fences[0].members, ["a", "c", "b", "d"]);
        let disabled = state.apply(WorkspaceCommand::SetFenceSort { fence_id: fence_id.clone(), sort_mode: FenceSortMode::None }).unwrap(); // 关闭自动排序保留已有排列。
        assert_eq!(disabled.fences[0].members, descending.fences[0].members);
        let manual = state.apply(WorkspaceCommand::MoveItemToFence { item_id: "d".to_string(), fence_id: fence_id.clone(), before_item_id: Some("a".to_string()) }).unwrap(); // 手动插入继续生效。
        assert_eq!(manual.fences[0].members, ["d", "a", "c", "b"]);
        state.apply(WorkspaceCommand::SetFenceSort { fence_id: fence_id.clone(), sort_mode: FenceSortMode::NameAscending }).unwrap();
        items[0].display_name = "Aardvark".to_string();
        state.update_item_names(&items).unwrap();
        let renamed = state.reconcile_items(1280.0, 720.0, &ids).unwrap(); // 名称变化但引用不变也应重新排序。
        assert_eq!(renamed.fences[0].members, ["a", "d", "b", "c"]);
        drop(state);
        let restored = workspace_state(path.clone()); // 验证重启后保留排序状态及成员顺序。
        assert_eq!(restored.snapshot().unwrap().fences, renamed.fences);
        drop(restored);
        std::fs::remove_file(path).unwrap();
    }

    /// 验证围栏成员随唯一布局保存，并通过当前格式完整性校验。
    #[test]
    fn current_fence_store_round_trips()
    {
        let path = snapshot_path("shared-fence-store"); // 本测试独占的当前格式备份。
        let state = workspace_state(path.clone()); // 创建默认无排序围栏的工作区。
        state.apply(WorkspaceCommand::CreateFence { title: "旧围栏".to_string(), bounds: WorkspaceRect { left: 0.0, top: 0.0, width: 320.0, height: 240.0 } }).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap(); // 当前规范化备份载荷。
        assert!(!serde_json::to_string(&value).unwrap().contains("sortMode"));
        assert!(value["payload"].get("sharedFences").is_none());
        assert_eq!(value["payload"]["layout"]["fences"][0]["members"], serde_json::json!([]));
        let store = decode_workspace_store(value).unwrap(); // 当前格式校验并读取完整成员。
        assert_eq!(store.payload.layout.fences[0].sort_mode, crate::contracts::FenceSortMode::None);
        drop(state);
        std::fs::remove_file(path).unwrap();
    }

    /// 验证创建围栏命令持久化完整初始状态并可在重启后恢复。
    #[test]
    fn create_fence_command_persists_and_restores_the_snapshot()
    {
        // 当前测试使用的持久化文件。
        let path = snapshot_path("create-fence");
        // 模拟首次应用启动的工作区状态。
        let state = workspace_state(path.clone());
        // 创建围栏命令返回的权威快照。
        let snapshot = state.apply(WorkspaceCommand::CreateFence { title: "项目资料".to_string(), bounds: WorkspaceRect { left: 120.0, top: 80.0, width: 360.0, height: 280.0 } }).expect("有效围栏应可创建");

        assert_eq!(snapshot.revision, 1);
        assert_eq!(snapshot.fences[0].id, "fence-1");
        assert_eq!(snapshot.fences[0].last_expanded_height, 280.0);
        drop(state);

        // 模拟应用重启后从磁盘恢复的工作区状态。
        let restored = WorkspaceState::load(path.clone()).expect("已保存工作区应可恢复");
        assert_eq!(restored.snapshot().expect("恢复快照应可读取"), snapshot);

        std::fs::remove_file(path).expect("测试快照应可清理");
    }

    /// 验证单项选择通过公开快照发布且应用重启后回到空运行期状态。
    #[test]
    fn item_selection_is_runtime_state()
    {
        // 当前测试使用的持久化文件。
        let path = snapshot_path("item-selection");
        // 已协调一个真实桌面项目身份的工作区状态。
        let state = workspace_state(path.clone());
        state.reconcile_items(1280.0, 720.0, &["::{project}".to_string()]).expect("Shell 项目应可协调到工作区");

        // 单击项目后发布的权威交互快照。
        let snapshot = state.apply_interaction(WorkspaceInteractionCommand::FocusItem { item_id: "::{project}".to_string(), selection_mode: WorkspaceSelectionMode::Replace }).expect("项目应可成为唯一选择和键盘焦点");
        assert_eq!(snapshot.interaction.revision, 1);
        assert_eq!(snapshot.interaction.selected_item_ids, ["::{project}"]);
        assert_eq!(snapshot.interaction.selection_anchor_id.as_deref(), Some("::{project}"));

        drop(state);
        // 重启后只恢复布局，选择与焦点使用新的运行期初始状态。
        let restored = WorkspaceState::load(path.clone()).expect("工作区布局应可恢复");
        assert_eq!(restored.snapshot().expect("恢复快照应可读取").interaction.revision, 0);
        assert!(restored.snapshot().expect("恢复快照应可读取").interaction.selected_item_ids.is_empty());

        std::fs::remove_file(path).expect("测试快照应可清理");
    }

    /// 验证切换、范围和保留选择模式遵循焦点容器逻辑顺序。
    #[test]
    fn selection_modes_follow_the_focus_container()
    {
        // 当前测试使用的持久化文件。
        let path = snapshot_path("selection-modes");
        // 已协调四个桌面项目的工作区状态。
        let state = workspace_state(path.clone());
        state.reconcile_items(1280.0, 720.0, &["a".to_string(), "b".to_string(), "c".to_string(), "d".to_string()]).expect("Shell 项目应可协调到工作区");
        // 当前测试用于范围选择的围栏标识。
        let fence_id = state.apply(WorkspaceCommand::CreateFence { title: "范围".to_string(), bounds: WorkspaceRect { left: 100.0, top: 80.0, width: 320.0, height: 240.0 } }).expect("围栏应可创建").fences[0].id.clone();
        for item_id in ["b", "c", "d"]
        {
            state.apply(WorkspaceCommand::MoveItemToFence { item_id: item_id.to_string(), fence_id: fence_id.clone(), before_item_id: None }).expect("项目应可加入范围选择围栏");
        }

        state.apply_interaction(WorkspaceInteractionCommand::FocusItem { item_id: "b".to_string(), selection_mode: WorkspaceSelectionMode::Replace }).expect("范围锚点应可建立");
        // 同一围栏中的 Shift 范围选择结果。
        let range = state.apply_interaction(WorkspaceInteractionCommand::FocusItem { item_id: "d".to_string(), selection_mode: WorkspaceSelectionMode::Range }).expect("同一焦点容器应可执行范围选择");
        assert_eq!(range.interaction.selected_item_ids, ["b", "c", "d"]);
        assert_eq!(range.interaction.selection_anchor_id.as_deref(), Some("b"));

        // Ctrl 切换目标后的选择集合。
        let toggled = state.apply_interaction(WorkspaceInteractionCommand::FocusItem { item_id: "c".to_string(), selection_mode: WorkspaceSelectionMode::Toggle }).expect("焦点项目应可切换选择状态");
        assert_eq!(toggled.interaction.selected_item_ids, ["b", "d"]);
        assert_eq!(toggled.interaction.selection_anchor_id.as_deref(), Some("c"));

        // Ctrl+方向键只移动焦点并保留现有选择。
        let preserved = state.apply_interaction(WorkspaceInteractionCommand::FocusItem { item_id: "d".to_string(), selection_mode: WorkspaceSelectionMode::Preserve }).expect("项目焦点应可独立移动");
        assert_eq!(preserved.interaction.selected_item_ids, ["b", "d"]);

        // 项目焦点上的 Ctrl+A 保留当前项目作为选择锚点。
        let selected_container = state.apply_interaction(WorkspaceInteractionCommand::SelectFocusContainer).expect("当前项目焦点容器应可全选");
        assert_eq!(selected_container.interaction.selected_item_ids, ["b", "c", "d"]);
        assert_eq!(selected_container.interaction.selection_anchor_id.as_deref(), Some("d"));

        // 跨焦点容器的 Shift 选择收束为目标单项。
        let reset = state.apply_interaction(WorkspaceInteractionCommand::FocusItem { item_id: "a".to_string(), selection_mode: WorkspaceSelectionMode::Range }).expect("跨容器范围选择应建立新锚点");
        assert_eq!(reset.interaction.selected_item_ids, ["a"]);
        assert_eq!(reset.interaction.selection_anchor_id.as_deref(), Some("a"));

        std::fs::remove_file(path).expect("测试快照应可清理");
    }

    /// 验证框选、高层焦点、容器全选和围栏折叠协调公开交互状态。
    #[test]
    fn surface_focus_and_fence_collapse_coordinate_selection()
    {
        // 当前测试使用的持久化文件。
        let path = snapshot_path("surface-focus");
        // 已协调三个桌面项目的工作区状态。
        let state = workspace_state(path.clone());
        state.reconcile_items(1280.0, 720.0, &["a".to_string(), "b".to_string(), "c".to_string()]).expect("Shell 项目应可协调到工作区");
        // 当前测试使用的围栏标识。
        let fence_id = state.apply(WorkspaceCommand::CreateFence { title: "焦点".to_string(), bounds: WorkspaceRect { left: 100.0, top: 80.0, width: 320.0, height: 240.0 } }).expect("围栏应可创建").fences[0].id.clone();
        for item_id in ["b", "c"]
        {
            state.apply(WorkspaceCommand::MoveItemToFence { item_id: item_id.to_string(), fence_id: fence_id.clone(), before_item_id: None }).expect("项目应可加入围栏");
        }

        // 框选按视觉顺序发布的选择、锚点和焦点。
        let marquee = state.apply_interaction(WorkspaceInteractionCommand::SelectItems { item_ids: vec!["a".to_string(), "c".to_string()] }).expect("框选结果应可提交");
        assert_eq!(marquee.interaction.selected_item_ids, ["a", "c"]);
        assert_eq!(marquee.interaction.selection_anchor_id.as_deref(), Some("a"));

        // 围栏标题焦点清空项目选择。
        let fence_focus = state.apply_interaction(WorkspaceInteractionCommand::FocusSurface { focus: WorkspaceFocusTarget::Fence { fence_id: fence_id.clone() } }).expect("围栏标题应可获得高层焦点");
        assert!(fence_focus.interaction.selected_item_ids.is_empty());
        // 围栏标题上的 Ctrl+A 选择全部成员。
        let selected_fence = state.apply_interaction(WorkspaceInteractionCommand::SelectFocusContainer).expect("围栏焦点容器应可全选");
        assert_eq!(selected_fence.interaction.selected_item_ids, ["b", "c"]);

        state.apply_interaction(WorkspaceInteractionCommand::FocusItem { item_id: "b".to_string(), selection_mode: WorkspaceSelectionMode::Replace }).expect("围栏成员应可获得焦点");
        // 折叠围栏后的权威交互快照。
        let collapsed = state.apply(WorkspaceCommand::SetFenceCollapsed { fence_id: fence_id.clone(), collapsed: true }).expect("围栏应可折叠");
        assert!(collapsed.interaction.selected_item_ids.is_empty());
        assert!(collapsed.interaction.selection_anchor_id.is_none());
        assert_eq!(collapsed.interaction.keyboard_focus, WorkspaceFocusTarget::Fence { fence_id: fence_id.clone() });

        // 桌面背景焦点上的 Ctrl+A 只选择未归类区域。
        state.apply_interaction(WorkspaceInteractionCommand::FocusSurface { focus: WorkspaceFocusTarget::Background }).expect("桌面背景应可获得焦点");
        let selected_background = state.apply_interaction(WorkspaceInteractionCommand::SelectFocusContainer).expect("未归类焦点容器应可全选");
        assert_eq!(selected_background.interaction.selected_item_ids, ["a"]);

        std::fs::remove_file(path).expect("测试快照应可清理");
    }

    /// 验证项目归属保留来源位置并在移出或解散后恢复未归类状态。
    #[test]
    fn membership_commands_preserve_the_unclassified_source_position()
    {
        // 当前测试使用的持久化文件。
        let path = snapshot_path("membership");
        // 模拟当前应用会话的工作区状态。
        let state = workspace_state(path.clone());
        // 宿主网格与 Shell 项目形成的单次原子协调事务。
        let reconciled = state.reconcile_items(1280.0, 720.0, &["::{occupied}".to_string(), "::{project}".to_string(), "::{document}".to_string()]).expect("Shell 项目应可协调到未归类网格");
        assert_eq!(reconciled.revision, 1);
        assert_eq!(reconciled.grid.rows, 7);
        assert_eq!(reconciled.unclassified_items.len(), 3);
        // 新围栏的权威标识。
        let fence_id = state.apply(WorkspaceCommand::CreateFence { title: "资料".to_string(), bounds: WorkspaceRect { left: 100.0, top: 80.0, width: 320.0, height: 240.0 } }).expect("围栏应可创建").fences[0].id.clone();
        // 桌面项目进入围栏前的来源位置。
        let source_position = state.snapshot().expect("协调快照应可读取").unclassified_items.iter().find(|item| item.stable_reference == "::{project}").expect("项目应拥有未归类位置").position;

        // 首次加入围栏后的权威快照。
        let snapshot = state.apply(WorkspaceCommand::MoveItemToFence { item_id: "::{project}".to_string(), fence_id: fence_id.clone(), before_item_id: None }).expect("桌面项目应可加入围栏");
        assert_eq!(snapshot.fences[0].members, ["::{project}"]);
        assert_eq!(snapshot.unclassified_items.iter().find(|item| item.stable_reference == "::{project}").expect("围栏成员应保留来源位置").position, source_position);
        // 项目释放到自身时保持当前成员顺序和修订。
        let unchanged = state.apply(WorkspaceCommand::MoveItemToFence { item_id: "::{project}".to_string(), fence_id: fence_id.clone(), before_item_id: Some("::{project}".to_string()) }).expect("释放到自身应保持当前归属");
        assert_eq!(unchanged, snapshot);

        // 项目移出围栏后的权威快照。
        let snapshot = state.apply(WorkspaceCommand::RemoveItemFromFence { item_id: "::{project}".to_string() }).expect("桌面项目应可移出围栏");
        assert!(snapshot.fences[0].members.is_empty());
        assert_eq!(snapshot.unclassified_items.iter().find(|item| item.stable_reference == "::{project}").expect("移出项目应恢复来源位置").position, source_position);

        state.apply(WorkspaceCommand::MoveItemToFence { item_id: "::{project}".to_string(), fence_id: fence_id.clone(), before_item_id: None }).expect("桌面项目应可再次加入围栏");
        state.apply(WorkspaceCommand::MoveItemToFence { item_id: "::{document}".to_string(), fence_id: fence_id.clone(), before_item_id: None }).expect("第二个桌面项目应可加入围栏");
        // 围栏解散并避让已有未归类项目后的权威快照。
        let snapshot = state.apply(WorkspaceCommand::DissolveFence { fence_id }).expect("围栏应可解散");
        assert!(snapshot.fences.is_empty());
        assert_eq!(snapshot.unclassified_items.iter().find(|item| item.stable_reference == "::{project}").expect("解散后项目应回到未归类区域").position, crate::contracts::WorkspacePoint { left: 78.0, top: 98.0 });
        assert_eq!(snapshot.unclassified_items.iter().find(|item| item.stable_reference == "::{document}").expect("解散后第二个项目应回到未归类区域").position, crate::contracts::WorkspacePoint { left: 78.0, top: 196.0 });

        std::fs::remove_file(path).expect("测试快照应可清理");
    }

    /// 验证未归类项目可以占用围栏成员释放出的视觉空网格。
    #[test]
    fn free_drop_repairs_a_fence_member_source_collision()
    {
        let path = snapshot_path("fence-source-collision");
        let state = workspace_state(path.clone());
        state.reconcile_items(1280.0, 720.0, &["a".to_string(), "b".to_string()]).expect("Shell 项目应可协调到工作区");
        let fence_id = state.apply(WorkspaceCommand::CreateFence { title: "资料".to_string(), bounds: WorkspaceRect { left: 100.0, top: 80.0, width: 320.0, height: 240.0 } }).expect("围栏应可创建").fences[0].id.clone();
        let source_position = state.snapshot().expect("快照应可读取").unclassified_items.iter().find(|item| item.stable_reference == "a").expect("围栏成员应拥有来源位置").position;
        state.apply(WorkspaceCommand::MoveItemToFence { item_id: "a".to_string(), fence_id, before_item_id: None }).expect("项目应可进入围栏");
        let snapshot = state.apply(WorkspaceCommand::CommitItemDrag { anchor_item_id: "b".to_string(), item_ids: vec!["b".to_string()], placements: vec![WorkspaceItemPlacement { item_id: "b".to_string(), position: source_position }], target_fence_id: None, before_item_id: None, fence_scrolls: Vec::new() }).expect("空白网格应可放置");
        let positions = snapshot.unclassified_items.iter().map(|item| (item.position.left.to_bits(), item.position.top.to_bits())).collect::<std::collections::HashSet<_>>();
        assert_eq!(positions.len(), snapshot.unclassified_items.len());
        assert_eq!(snapshot.unclassified_items.iter().find(|item| item.stable_reference == "b").expect("项目 b 应存在").position, source_position);
        std::fs::remove_file(path).expect("测试快照应可清理");
    }

    /// 验证围栏几何、折叠、布局和滚动状态形成完整持久化事务。
    #[test]
    fn fence_state_commands_persist_the_complete_visual_state()
    {
        // 当前测试使用的持久化文件。
        let path = snapshot_path("fence-state");
        // 模拟当前应用会话的工作区状态。
        let state = workspace_state(path.clone());
        // 新围栏的权威标识。
        let fence_id = state.apply(WorkspaceCommand::CreateFence { title: "任务".to_string(), bounds: WorkspaceRect { left: 40.0, top: 40.0, width: 320.0, height: 240.0 } }).expect("围栏应可创建").fences[0].id.clone();
        // 移动和缩放后提交的围栏边界。
        let bounds = WorkspaceRect { left: 180.0, top: 120.0, width: 420.0, height: 300.0 };
        state.apply(WorkspaceCommand::UpdateFenceBounds { fence_id: fence_id.clone(), bounds }).expect("围栏边界应可更新");
        state.apply(WorkspaceCommand::SetFenceLayout { fence_id: fence_id.clone(), layout_mode: crate::contracts::FenceLayoutMode::List }).expect("围栏布局应可更新");
        state.apply(WorkspaceCommand::SetFenceScroll { fence_id: fence_id.clone(), scroll_top: 72.0 }).expect("围栏滚动应可更新");
        // 围栏全部视觉状态提交后的权威快照。
        let snapshot = state.apply(WorkspaceCommand::SetFenceCollapsed { fence_id, collapsed: true }).expect("围栏应可折叠");

        assert_eq!(snapshot.fences[0].bounds, bounds);
        assert_eq!(snapshot.fences[0].last_expanded_height, 300.0);
        assert_eq!(snapshot.fences[0].layout_mode, crate::contracts::FenceLayoutMode::List);
        assert_eq!(snapshot.fences[0].scroll_top, 72.0);
        assert!(snapshot.fences[0].collapsed);

        std::fs::remove_file(path).expect("测试快照应可清理");
    }

    /// 验证组拖动形成一个可完整撤销和重做的布局事务。
    #[test]
    fn group_drag_is_one_undoable_layout_transaction()
    {
        // 当前测试使用的持久化文件。
        let path = snapshot_path("group-drag-history");
        // 已协调三个项目的当前应用会话工作区。
        let state = workspace_state(path.clone());
        state.reconcile_items(1280.0, 720.0, &["a".to_string(), "b".to_string(), "c".to_string()]).expect("Shell 项目应可协调到工作区");
        // 当前组拖动使用的目标围栏标识。
        let fence_id = state.apply(WorkspaceCommand::CreateFence { title: "拖动".to_string(), bounds: WorkspaceRect { left: 120.0, top: 80.0, width: 320.0, height: 240.0 } }).expect("目标围栏应可创建").fences[0].id.clone();
        state.apply_interaction(WorkspaceInteractionCommand::SelectItems { item_ids: vec!["a".to_string(), "b".to_string()] }).expect("组拖动选择应可建立");

        // 两个项目原子进入围栏后的权威快照。
        let committed = state.apply(WorkspaceCommand::CommitItemDrag { anchor_item_id: "a".to_string(), item_ids: vec!["a".to_string(), "b".to_string()], placements: Vec::new(), target_fence_id: Some(fence_id.clone()), before_item_id: None, fence_scrolls: Vec::new() }).expect("组拖动应可原子提交");
        assert_eq!(committed.fences[0].members, ["a", "b"]);
        assert_eq!(committed.interaction.selected_item_ids, ["a", "b"]);
        let stale = state.apply_on_monitor_at_revision(None, WorkspaceCommand::CommitItemDrag { anchor_item_id: "c".into(), item_ids: vec!["c".into()], placements: Vec::new(), target_fence_id: Some(fence_id.clone()), before_item_id: None, fence_scrolls: Vec::new() }, Some(committed.revision - 1)); // 校验后发生其他布局事务时，旧几何不能再提交。
        assert!(stale.is_err());
        assert_eq!(state.snapshot().unwrap(), committed);

        // 撤销组拖动后的完整布局。
        let undone = state.apply(WorkspaceCommand::UndoLayout).expect("组拖动应可撤销");
        assert!(undone.fences[0].members.is_empty());
        assert_eq!(undone.interaction.selected_item_ids, ["a", "b"]);

        // 重做组拖动后的完整布局。
        let redone = state.apply(WorkspaceCommand::RedoLayout).expect("组拖动应可重做");
        assert_eq!(redone.fences[0].members, ["a", "b"]);
        assert_eq!(redone.interaction.keyboard_focus, WorkspaceFocusTarget::Item { item_id: "a".to_string() });

        let appended = state.apply(WorkspaceCommand::CommitItemDrag { anchor_item_id: "c".into(), item_ids: vec!["c".into()], placements: Vec::new(), target_fence_id: Some(fence_id), before_item_id: Some("removed-successor".into()), fence_scrolls: Vec::new() }).expect("释放后后继消失应追加而不丢掉整组"); // 并发后继失效仍按实际提交顺序追加。
        assert_eq!(appended.fences[0].members, ["a", "b", "c"]);
        assert_eq!(state.apply(WorkspaceCommand::UndoLayout).unwrap().fences[0].members, ["a", "b"]);

        std::fs::remove_file(path).expect("测试快照应可清理");
    }

    /// 验证 Shell 精确重命名同步布局成员、选择、锚点和键盘焦点。
    #[test]
    fn shell_rename_remaps_all_runtime_item_references()
    {
        let path = snapshot_path("shell-rename"); // 当前测试使用的持久化文件。
        let state = workspace_state(path.clone()); // 模拟当前应用会话的工作区状态。
        state.reconcile_items(1280.0, 720.0, &["old".to_string()]).expect("原项目应可协调到工作区");
        let fence_id = state.apply(WorkspaceCommand::CreateFence { title: "重命名".to_string(), bounds: WorkspaceRect { left: 100.0, top: 80.0, width: 320.0, height: 240.0 } }).expect("围栏应可创建").fences[0].id.clone(); // 当前项目所属围栏标识。
        state.apply(WorkspaceCommand::MoveItemToFence { item_id: "old".to_string(), fence_id, before_item_id: None }).expect("原项目应可加入围栏");
        state.apply_interaction(WorkspaceInteractionCommand::FocusItem { item_id: "old".to_string(), selection_mode: WorkspaceSelectionMode::Replace }).expect("原项目应可建立选择和焦点");

        let renamed = state.reconcile_shell_items(1280.0, 720.0, &["new".to_string()], &[("old".to_string(), "new".to_string())]).expect("Shell 重命名事实应可协调"); // 精确重命名和最终枚举共同产生的快照。
        assert_eq!(renamed.fences[0].members, ["new"]);
        assert_eq!(renamed.interaction.selected_item_ids, ["new"]);
        assert_eq!(renamed.interaction.selection_anchor_id.as_deref(), Some("new"));
        assert_eq!(renamed.interaction.keyboard_focus, WorkspaceFocusTarget::Item { item_id: "new".to_string() });

        std::fs::remove_file(path).expect("测试快照应可清理");
    }

    /// 验证重命名补偿后的精确引用仍能撤销先前的布局移动。
    #[test]
    fn rename_compensation_preserves_prior_layout_undo()
    {
        let path = snapshot_path("rename-layout-undo"); // 当前测试布局。
        let state = workspace_state(path.clone()); // 同一会话的历史集合。
        let initial = state.reconcile_items(1280.0, 720.0, &["old".into()]).unwrap(); // 原始来源位置。
        state.apply(WorkspaceCommand::CommitItemDrag { anchor_item_id: "old".into(), item_ids: vec!["old".into()], placements: vec![WorkspaceItemPlacement { item_id: "old".into(), position: WorkspacePoint { left: 300.0, top: 200.0 } }], target_fence_id: None, before_item_id: None, fence_scrolls: vec![] }).unwrap();
        state.reconcile_shell_items(1280.0, 720.0, &["new".into()], &[("old".into(), "new".into())]).unwrap();
        state.reconcile_shell_items(1280.0, 720.0, &["old".into()], &[("new".into(), "old".into())]).unwrap();
        let restored = state.restore_layout_history(false).unwrap(); // 撤销重命名后再撤销布局。
        assert_eq!(restored.snapshot.unclassified_items[0].position, initial.unclassified_items[0].position);
        assert_eq!(restored.snapshot.unclassified_items[0].stable_reference, "old");
        drop(state);
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir_all(path.with_extension("snapshots")).unwrap();
    }

    /// 验证 Explorer 初始归属、断开投射、重连恢复和 DPI 变化保持持久显示器。
    #[test]
    fn monitor_reconciliation_preserves_persistent_assignment()
    {
        let path = snapshot_path("monitor-reconciliation"); // 当前测试使用的持久化文件。
        let state = workspace_state(path.clone()); // 模拟首次逐屏启动的工作区状态。
        let primary = monitor("DISPLAY1", true, 0, 1.0); // 当前主显示器。
        let secondary = monitor("DISPLAY2", false, 1920, 1.5); // 当前混合 DPI 次显示器。
        let initial = state.reconcile_host_monitors(&[primary.clone(), secondary.clone()], &["primary-item".to_string(), "secondary-item".to_string()], &[("secondary-item".to_string(), "DISPLAY2".to_string())], &[]).expect("Explorer 项目归属应可首次导入");
        let secondary_position = initial.unclassified_items.iter().find(|item| item.stable_reference == "secondary-item").expect("次屏项目应存在").position; // 次屏项目持久 DIP 位置。
        assert_eq!(initial.unclassified_items[1].monitor_id, "DISPLAY2");

        let projected = state.reconcile_host_monitors(std::slice::from_ref(&primary), &["primary-item".to_string(), "secondary-item".to_string()], &[], &[]).expect("显示器断开后项目应可临时投射");
        let projected_item = projected.unclassified_items.iter().find(|item| item.stable_reference == "secondary-item").expect("投射项目应存在");
        assert_eq!(projected_item.monitor_id, "DISPLAY2");
        assert_eq!(projected_item.projection_monitor_id.as_deref(), Some("DISPLAY1"));
        assert_eq!(projected_item.position, secondary_position);

        let mut scaled_secondary = secondary; // DPI-only 重建后的次显示器。
        scaled_secondary.scale_factor = 2.0;
        scaled_secondary.dpi = 192;
        let restored = state.reconcile_host_monitors(&[primary, scaled_secondary], &["primary-item".to_string(), "secondary-item".to_string()], &[], &[]).expect("显示器重连和 DPI 变化应可协调");
        let restored_item = restored.unclassified_items.iter().find(|item| item.stable_reference == "secondary-item").expect("恢复项目应存在");
        assert_eq!(restored_item.monitor_id, "DISPLAY2");
        assert_eq!(restored_item.projection_monitor_id, None);
        assert_eq!(restored_item.position, secondary_position);

        std::fs::remove_file(path).expect("测试快照应可清理");
    }

    /// 验证跨屏围栏和项目拖动在一个布局事务中改写目标显示器归属。
    #[test]
    fn cross_monitor_layout_commands_use_the_target_monitor()
    {
        let path = snapshot_path("cross-monitor-command"); // 当前测试使用的持久化文件。
        let state = workspace_state(path.clone()); // 当前逐屏布局状态。
        let monitors = [monitor("DISPLAY1", true, 0, 1.0), monitor("DISPLAY2", false, 1920, 1.5)]; // 当前双屏混合 DPI 环境。
        state.reconcile_host_monitors(&monitors, &["item".to_string()], &[], &[]).expect("项目应可初始化");
        let fence = state.apply_on_monitor(Some("DISPLAY2"), WorkspaceCommand::CreateFence { title: "次屏围栏".to_string(), bounds: WorkspaceRect { left: 80.0, top: 80.0, width: 320.0, height: 240.0 } }).expect("围栏应在目标显示器创建");
        assert_eq!(fence.fences[0].monitor_id, "DISPLAY2");

        let moved = state.apply_on_monitor(Some("DISPLAY2"), WorkspaceCommand::CommitItemDrag { anchor_item_id: "item".to_string(), item_ids: vec!["item".to_string()], placements: vec![WorkspaceItemPlacement { item_id: "item".to_string(), position: WorkspacePoint { left: 112.0, top: 172.0 } }], target_fence_id: None, before_item_id: None, fence_scrolls: Vec::new() }).expect("项目应可跨屏提交");
        assert_eq!(moved.unclassified_items[0].monitor_id, "DISPLAY2");
        assert_eq!(moved.unclassified_items[0].position.left, 112.0);

        std::fs::remove_file(path).expect("测试快照应可清理");
    }

    /// 验证当前备份格式拒绝被修改但未更新校验值的载荷。
    #[test]
    fn current_backup_rejects_tampered_payload()
    {
        let store = workspace_store_from_payload(PersistedWorkspacePayload {
            layout: WorkspaceSnapshot::default(), // 唯一布局。
            explorer_layout_imported: true,
        })
        .expect("当前备份应可创建"); // 已计算完整性校验值的原始备份。
        let mut value = serde_json::to_value(store).expect("备份应可序列化"); // 等待模拟损坏的 JSON 载荷。
        value["payload"]["layout"]["revision"] = serde_json::json!(9);

        let error = decode_workspace_store(value).expect_err("校验值未同步的备份应被拒绝");

        assert_eq!(error.code, "workspace-restore-failed");
        assert!(error.message.contains("完整性校验失败"));
    }

    /// 验证旧多布局与单布局格式均被拒绝，不执行兼容读取。
    #[test]
    fn legacy_workspace_formats_are_rejected()
    {
        for version in 1..5
        {
            let value = serde_json::json!({"storeVersion": version}); // 所有旧版本直接拒绝。
            assert!(decode_workspace_store(value).is_err());
        }
        assert!(decode_workspace_store(serde_json::to_value(WorkspaceSnapshot::default()).unwrap()).is_err());
    }

    /// 验证统一工作区的自动快照只保留最近 20 份。
    #[test]
    fn automatic_snapshots_keep_the_latest_twenty()
    {
        let directory = snapshot_path("auto-snapshot-limit").with_extension("snapshots"); // 当前测试独占的自动快照目录。
        let store = workspace_store_from_payload(PersistedWorkspacePayload {
            layout: WorkspaceSnapshot::default(), // 唯一布局。
            explorer_layout_imported: true,
        })
        .expect("自动快照载荷应可创建");
        for _ in 0..AUTO_SNAPSHOT_LIMIT + 3
        {
            write_auto_snapshot(&directory, &store).expect("自动快照应可写入");
        }

        let count = std::fs::read_dir(&directory).expect("自动快照目录应可读取").count();
        assert_eq!(count, AUTO_SNAPSHOT_LIMIT);

        std::fs::remove_dir_all(directory).expect("自动快照目录应可清理");
    }

    /// 验证损坏备份在还原前被拒绝且当前工作区保持完整。
    #[test]
    fn corrupted_restore_preserves_the_current_workspace()
    {
        let path = snapshot_path("restore-rollback"); // 当前测试使用的权威工作区文件。
        let backup_path = snapshot_path("restore-rollback-backup"); // 当前测试使用的手动备份文件。
        let state = workspace_state(path.clone());
        state.reconcile_items(1280.0, 720.0, &["project".to_string()]).expect("当前工作区项目应可初始化");
        state.write_manual_backup(&backup_path).expect("手动备份应可写入");
        let before = state.snapshot().expect("还原前工作区应可读取"); // 损坏还原必须保持的当前状态。
        let mut value: serde_json::Value = serde_json::from_slice(&std::fs::read(&backup_path).expect("手动备份应可读取")).expect("手动备份应为 JSON");
        value["payload"]["layout"]["revision"] = serde_json::json!(77);
        std::fs::write(&backup_path, serde_json::to_vec_pretty(&value).expect("损坏备份应可序列化")).expect("损坏备份应可写入");

        let result = state.restore_backup(&backup_path, &[monitor("DISPLAY1", true, 0, 1.0)], &["project".to_string()]);

        assert_eq!(result.expect_err("损坏备份应被拒绝").code, "workspace-restore-failed");
        assert_eq!(state.snapshot().expect("失败后工作区应可读取"), before);

        drop(state);
        std::fs::remove_file(path).expect("测试工作区应可清理");
        std::fs::remove_file(backup_path).expect("测试备份应可清理");
    }

    /// 验证布局历史跳过已离开项目并保留历史提交后新增的项目。
    #[test]
    fn layout_history_merge_reports_partial_restore_and_keeps_external_items()
    {
        let item = |stable_reference: &str, left: f64| UnclassifiedItemSnapshot { stable_reference: stable_reference.to_string(), monitor_id: "DISPLAY1".to_string(), projection_monitor_id: None, projection_position: None, position: WorkspacePoint { left, top: 60.0 }, z_index: left as u32 }; // 当前测试构造稳定项目位置的最小工厂。
        let target = WorkspaceSnapshot { unclassified_items: vec![item("existing", 20.0), item("removed", 112.0)], ..WorkspaceSnapshot::default() }; // 历史事务原本覆盖的布局。
        let current = WorkspaceSnapshot { unclassified_items: vec![item("existing", 204.0), item("created", 296.0)], ..WorkspaceSnapshot::default() }; // 外部删除和新增后的当前布局。

        let (merged, restored_count, changed_count) = merge_layout_history_target(target, &current);

        assert_eq!(restored_count, 1);
        assert_eq!(changed_count, 1);
        assert!(merged.unclassified_items.iter().any(|item| item.stable_reference == "created"));
        assert!(merged.unclassified_items.iter().all(|item| item.stable_reference != "removed"));
    }
}
