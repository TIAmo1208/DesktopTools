use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::app::settings::SettingsState;
use crate::contracts::settings::{ApplicationPreferences, GridPreferences};
use crate::contracts::{
    ComponentInstanceSnapshot, DesktopHostMonitor, FENCE_MIN_HEIGHT, FENCE_MIN_WIDTH,
    FenceLayoutMode, FenceSnapshot, FenceSortMode, UnclassifiedItemSnapshot,
    WORKSPACE_BACKUP_SCHEMA_VERSION, WORKSPACE_GRID_BOTTOM_MARGIN, WORKSPACE_GRID_LEFT,
    WORKSPACE_GRID_TOP, WORKSPACE_SCHEMA_VERSION, WorkspaceBackupSummary, WorkspaceCommand,
    WorkspaceError, WorkspaceFenceScroll, WorkspaceFocusTarget, WorkspaceGrid,
    WorkspaceInteractionCommand, WorkspaceInteractionSnapshot, WorkspaceItemPlacement,
    WorkspaceMonitorGrid, WorkspacePoint, WorkspaceRect, WorkspaceRestoreResult,
    WorkspaceSelectionMode, WorkspaceSnapshot,
};
use crate::persistence::{load_json, save_json_atomic};

/// 多虚拟桌面工作区持久化格式版本。
const WORKSPACE_STORE_VERSION: u32 = WORKSPACE_BACKUP_SCHEMA_VERSION;

/// 自动快照在布局空闲后的合并等待时间。
const AUTO_SNAPSHOT_IDLE: Duration = Duration::from_secs(2);

/// 每个虚拟桌面保留的最近自动快照数量。
const AUTO_SNAPSHOT_LIMIT: usize = 20;

/// 同一进程内自动快照文件名的递增唯一序号。
static NEXT_AUTO_SNAPSHOT_ID: AtomicU64 = AtomicU64::new(1);

/// 已删除虚拟桌面布局归档保留秒数。
const LAYOUT_ARCHIVE_RETENTION_SECONDS: i64 = 30 * 24 * 60 * 60;

/// 持有正式桌面内容工作区的权威布局和持久化位置。
pub struct WorkspaceState {
    grid_preferences: Mutex<GridPreferences>, // 全部虚拟桌面共用的网格生成参数。
    /// Shell 最近发布的显示名称，仅供排序，不写入布局文件。
    item_names: Mutex<HashMap<String, String>>,
    /// command 共享的当前工作区快照。
    snapshot: Mutex<WorkspaceSnapshot>,
    /// 当前活动身份、其他活动布局与 30 天删除归档。
    virtual_desktops: Mutex<WorkspaceVirtualDesktops>,
    /// 每个虚拟桌面独立的会话布局撤销与重做栈。
    history: Mutex<HashMap<String, WorkspaceHistory>>,
    /// 每次成功布局事务使用的原子 JSON 文件。
    persistence_path: PathBuf,
    /// 等待 2 秒空闲合并写入的完整工作区自动快照发送端。
    auto_snapshot_sender: Option<Sender<AutoSnapshotRequest>>,
    /// 应用退出前刷新待处理快照的后台线程。
    auto_snapshot_worker: Option<JoinHandle<()>>,
    /// 还原前同步快照和自动快照共用的目录。
    auto_snapshot_directory: PathBuf,
    /// 新虚拟桌面和旧无组件布局使用的官方默认实例集合。
    default_components: Vec<ComponentInstanceSnapshot>,
}

/// 保存当前虚拟桌面的会话布局历史。
#[derive(Clone, Default)]
struct WorkspaceHistory {
    /// 从最新到最旧保存的可撤销布局。
    undo: Vec<WorkspaceHistoryEntry>,
    /// 从最新到最旧保存的可重做布局。
    redo: Vec<WorkspaceHistoryEntry>,
}

/// 一个按完成时间参与统一时间线的布局历史单元。
#[derive(Clone)]
struct WorkspaceHistoryEntry {
    /// 当前方向恢复后应成为活动状态的布局。
    layout: WorkspaceSnapshot,
    /// 当前历史单元最近进入该方向的 Unix 微秒时间。
    timeline_at_unix_micros: i64,
}

/// 一次布局撤销或重做的权威快照与部分恢复统计。
pub(crate) struct WorkspaceLayoutHistoryResult {
    /// 已原子持久化的当前布局。
    pub snapshot: WorkspaceSnapshot,
    /// 仍存在并按历史状态恢复的项目数量。
    pub restored_count: usize,
    /// 历史中已经离开当前工作区的项目数量。
    pub changed_count: usize,
}

/// 当前活动布局之外的虚拟桌面运行状态。
#[derive(Clone)]
struct WorkspaceVirtualDesktops {
    /// 当前 Windows 虚拟桌面稳定身份。
    current_id: String,
    /// 其他活动虚拟桌面的独立布局。
    layouts: HashMap<String, WorkspaceSnapshot>,
    /// 已删除虚拟桌面的 30 天布局归档。
    archives: Vec<WorkspaceLayoutArchive>,
    /// Explorer 原始图标布局是否已经完成一次性首次导入。
    explorer_layout_imported: bool,
}

/// 持久化文件中的一个活动虚拟桌面布局。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PersistedVirtualDesktopLayout {
    /// Windows 分配的稳定虚拟桌面身份。
    virtual_desktop_id: String,
    /// 当前桌面完整独立布局。
    layout: WorkspaceSnapshot,
}

/// 已删除虚拟桌面的可恢复布局归档。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceLayoutArchive {
    /// Desktop Tools 分配的稳定归档标识。
    pub archive_id: String,
    /// 删除前 Windows 虚拟桌面身份。
    pub source_virtual_desktop_id: String,
    /// 归档创建 Unix 时间戳。
    pub archived_at_unix: i64,
    /// 归档自动过期 Unix 时间戳。
    pub expires_at_unix: i64,
    /// 删除前完整独立布局。
    layout: WorkspaceSnapshot,
}

/// 原子持久化的完整工作区备份载荷。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PersistedWorkspacePayload {
    /// 保存时 Windows 当前虚拟桌面身份。
    current_virtual_desktop_id: String,
    /// 当前全部活动虚拟桌面布局。
    layouts: Vec<PersistedVirtualDesktopLayout>,
    /// 仍在 30 天保留期内的删除归档。
    archives: Vec<WorkspaceLayoutArchive>,
    /// Explorer 原始图标布局是否已经完成一次性首次导入。
    #[serde(default)]
    explorer_layout_imported: bool,
}

/// 带版本、创建时间和完整性校验的工作区文件。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PersistedWorkspaceStore {
    /// 多虚拟桌面存储格式版本。
    store_version: u32,
    /// 文件创建 Unix 时间戳。
    created_at_unix: i64,
    /// 规范化载荷的 FNV-1a 完整性校验值。
    checksum: String,
    /// 当前完整活动布局、归档和首次导入状态。
    payload: PersistedWorkspacePayload,
}

/// Ticket 10 写入的无完整性字段多布局格式。
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LegacyPersistedWorkspaceStoreV2 {
    /// 旧多虚拟桌面存储格式版本。
    store_version: u32,
    /// 保存时 Windows 当前虚拟桌面身份。
    current_virtual_desktop_id: String,
    /// 旧文件中的全部活动虚拟桌面布局。
    layouts: Vec<PersistedVirtualDesktopLayout>,
    /// 旧文件中的删除布局归档。
    archives: Vec<WorkspaceLayoutArchive>,
    /// Explorer 原始图标布局是否已经完成一次性首次导入。
    #[serde(default)]
    explorer_layout_imported: bool,
}

/// 等待空闲窗口写入磁盘的完整工作区快照。
struct AutoSnapshotRequest {
    /// 当前布局事务所属虚拟桌面身份。
    virtual_desktop_id: String,
    /// 已完成完整性校验计算的完整工作区文件。
    store: PersistedWorkspaceStore,
}

/// 提供工作区 command 使用的领域操作。
impl WorkspaceState {
    /// 从原子 JSON 快照启动工作区，首次启动使用空布局。
    #[cfg(test)]
    pub fn load(persistence_path: PathBuf) -> Result<Self, WorkspaceError> {
        Self::load_for_virtual_desktops(persistence_path, "default", &["default".to_string()])
    }

    /// 按当前 Windows 虚拟桌面环境恢复全部独立布局。
    #[cfg(test)]
    pub fn load_for_virtual_desktops(
        persistence_path: PathBuf,
        current_virtual_desktop_id: &str,
        virtual_desktop_ids: &[String],
    ) -> Result<Self, WorkspaceError> {
        Self::load_for_virtual_desktops_with_components(
            persistence_path,
            current_virtual_desktop_id,
            virtual_desktop_ids,
            &[],
        )
    }

    /// 按当前 Windows 虚拟桌面环境和官方默认组件恢复全部独立布局。
    pub fn load_for_virtual_desktops_with_components(
        persistence_path: PathBuf,
        current_virtual_desktop_id: &str,
        virtual_desktop_ids: &[String],
        default_components: &[ComponentInstanceSnapshot],
    ) -> Result<Self, WorkspaceError> {
        validate_id(current_virtual_desktop_id, "当前虚拟桌面标识无效")?;
        let value = load_json::<serde_json::Value>(&persistence_path)
            .map_err(|error| WorkspaceError::restore_failed(&error))?; // 磁盘中等待格式识别的工作区 JSON。
        let (mut snapshot, mut virtual_desktops) = restore_workspace_store(
            value,
            current_virtual_desktop_id,
            virtual_desktop_ids,
            default_components,
        )?; // 已迁移旧布局格式并补齐官方默认组件的多虚拟桌面运行状态。
        snapshot.interaction = WorkspaceInteractionSnapshot::default();
        // 升级已有布局时按稳定身份补齐新增官方格子，不覆盖已有位置、尺寸或状态。
        for layout in std::iter::once(&mut snapshot)
            .chain(virtual_desktops.layouts.values_mut())
            .chain(
                virtual_desktops
                    .archives
                    .iter_mut()
                    .map(|archive| &mut archive.layout),
            )
        {
            for component in default_components {
                if !layout
                    .components
                    .iter()
                    .any(|existing| existing.instance_id == component.instance_id)
                {
                    layout.components.push(component.clone());
                }
            }
        }
        snapshot.validate()?;
        for layout in virtual_desktops.layouts.values_mut() {
            layout.interaction = WorkspaceInteractionSnapshot::default();
            layout.validate()?;
        }
        purge_expired_archives(&mut virtual_desktops.archives, Utc::now().timestamp());
        crate::log_info!(
            "工作区：多虚拟桌面快照恢复完成，当前桌面={}，活动布局={}，归档={}，修订={}，路径={}",
            virtual_desktops.current_id,
            virtual_desktops.layouts.len() + 1,
            virtual_desktops.archives.len(),
            snapshot.revision,
            persistence_path.display()
        );
        let auto_snapshot_directory = persistence_path.with_extension("snapshots"); // 当前工作区文件独占的自动快照目录。
        let (auto_snapshot_sender, auto_snapshot_worker) =
            start_auto_snapshot_worker(auto_snapshot_directory.clone())
                .map_err(|error| WorkspaceError::persistence_failed(&error))?; // 2 秒合并写入线程。

        Ok(Self {
            grid_preferences: Mutex::new(GridPreferences::default()),
            item_names: Mutex::new(HashMap::new()),
            snapshot: Mutex::new(snapshot),
            virtual_desktops: Mutex::new(virtual_desktops),
            history: Mutex::new(HashMap::new()),
            persistence_path,
            auto_snapshot_sender: Some(auto_snapshot_sender),
            auto_snapshot_worker: Some(auto_snapshot_worker),
            auto_snapshot_directory,
            default_components: default_components.to_vec(),
        })
    }

    /// 更新 Shell 权威显示名称供后续布局事务排序。
    pub fn update_item_names(
        &self,
        items: &[crate::contracts::DesktopItem],
    ) -> Result<(), WorkspaceError> {
        let mut names = self
            .item_names
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())?; // 当前运行期名称表。
        *names = items
            .iter()
            .map(|item| {
                (
                    item.stable_reference.clone(),
                    item.display_name.to_lowercase(),
                )
            })
            .collect();
        Ok(())
    }

    /// 在逐屏宿主启动之前恢复网格生成参数。
    pub fn initialize_grid_preferences(
        &self,
        preferences: GridPreferences,
    ) -> Result<(), WorkspaceError> {
        preferences
            .validate()
            .map_err(|message| WorkspaceError::invalid_command(&message))?;
        *self
            .grid_preferences
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())? = preferences;
        Ok(())
    }

    /// 所有环境协调入口共用当前网格参数。
    fn monitor_grids(
        &self,
        monitors: &[DesktopHostMonitor],
    ) -> Result<Vec<WorkspaceMonitorGrid>, WorkspaceError> {
        let preferences = *self
            .grid_preferences
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())?; // 一次协调中的固定参数。
        monitor_grids(monitors, preferences)
    }

    /// 校验全部活动布局的容量，再保存布局及偏好；偏好写入失败时恢复旧布局文件。
    pub fn save_preferences(
        &self,
        settings: &SettingsState,
        preferences: ApplicationPreferences,
        monitors: &[DesktopHostMonitor],
    ) -> Result<ApplicationPreferences, String> {
        preferences.validate()?;
        let mut grid_preferences = self.grid_preferences.lock().map_err(|_| "网格状态不可用")?; // 阻止网格配置并发提交。
        if *grid_preferences == preferences.desktop_grid {
            return settings.save(preferences);
        }
        let grids =
            monitor_grids(monitors, preferences.desktop_grid).map_err(|error| error.message)?; // 当前显示器的新网格。
        let mut current = self.snapshot.lock().map_err(|_| "工作区状态不可用")?; // 当前活动布局锁。
        let mut desktops = self
            .virtual_desktops
            .lock()
            .map_err(|_| "虚拟桌面状态不可用")?; // 其他活动布局锁。
        let mut next = current.clone(); // 保存失败时不影响可见布局。
        let mut next_desktops = desktops.clone(); // 非活动虚拟桌面候选布局。
        for layout in std::iter::once(&mut next).chain(next_desktops.layouts.values_mut()) {
            reflow_grid_items(layout, &grids).map_err(|error| error.message)?;
            layout.monitor_grids = grids.clone();
            layout.grid = grids
                .iter()
                .find(|entry| {
                    monitors
                        .iter()
                        .any(|monitor| monitor.primary && monitor.id == entry.monitor_id)
                })
                .or(grids.first())
                .ok_or("工作区缺少显示器")?
                .grid;
            layout.revision += 1;
            layout.validate().map_err(|error| error.message)?;
        }
        let store = save_workspace_store(&self.persistence_path, &next, &next_desktops)
            .map_err(|error| format!("保存网格布局失败：{error}"))?; // 完整布局先写入磁盘。
        let saved = match settings.save(preferences) {
            Ok(saved) => saved,
            Err(error) => {
                if let Err(rollback) =
                    save_workspace_store(&self.persistence_path, &current, &desktops)
                {
                    crate::log_error!("设置：网格布局回滚失败，错误：{rollback}");
                    return Err(format!("{error}；布局文件回滚失败：{rollback}"));
                }
                return Err(error);
            }
        }; // 仅两份文件均写入成功后更新运行状态。
        *grid_preferences = saved.desktop_grid;
        *current = next;
        *desktops = next_desktops;
        self.schedule_auto_snapshot(&desktops.current_id, store);
        crate::log_info!(
            "设置：桌面网格已应用，宽度：{}，高度：{}，横向间距：{}，纵向间距：{}",
            saved.desktop_grid.width,
            saved.desktop_grid.height,
            saved.desktop_grid.column_gap,
            saved.desktop_grid.row_gap
        );
        Ok(saved)
    }

    /// 使用最新 Shell 名称统一排序，返回成员顺序是否变化。
    fn sort_fences(&self, snapshot: &mut WorkspaceSnapshot) -> Result<bool, WorkspaceError> {
        let names = self
            .item_names
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())?; // 当前 Shell 名称索引。
        let mut changed = false; // 本次排序是否改变权威成员顺序。
        for fence in &mut snapshot.fences {
            if fence.sort_mode == FenceSortMode::None {
                continue;
            }
            let previous = fence.members.clone(); // 检测真实顺序变化的原始成员集合。
            fence.members.sort_by(|first, second| {
                let order = names
                    .get(first)
                    .unwrap_or(first)
                    .cmp(names.get(second).unwrap_or(second)); // 同名保留原始顺序。
                if fence.sort_mode == FenceSortMode::NameDescending {
                    order.reverse()
                } else {
                    order
                }
            });
            changed |= previous != fence.members;
        }
        Ok(changed)
    }

    /// 返回当前不可变工作区快照。
    pub fn snapshot(&self) -> Result<WorkspaceSnapshot, WorkspaceError> {
        // 当前受锁保护的工作区快照。
        let snapshot = self
            .snapshot
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())?;
        Ok(snapshot.clone())
    }

    /// 返回当前 Windows 虚拟桌面稳定身份。
    pub fn current_virtual_desktop_id(&self) -> Result<String, WorkspaceError> {
        self.virtual_desktops
            .lock()
            .map(|state| state.current_id.clone())
            .map_err(|_| WorkspaceError::state_unavailable())
    }

    /// 返回仍在保留期内的删除布局归档。
    pub fn layout_archives(&self) -> Result<Vec<WorkspaceLayoutArchive>, WorkspaceError> {
        self.virtual_desktops
            .lock()
            .map(|state| state.archives.clone())
            .map_err(|_| WorkspaceError::state_unavailable())
    }

    /// 将当前完整工作区写入用户指定的手动备份文件。
    pub fn write_manual_backup(
        &self,
        path: &Path,
    ) -> Result<WorkspaceBackupSummary, WorkspaceError> {
        validate_backup_path(path)?;
        let current = self
            .snapshot
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())?; // 手动备份中的当前活动布局。
        let virtual_desktops = self
            .virtual_desktops
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())?; // 手动备份中的其他活动布局与归档。
        let store = build_workspace_store(&current, &virtual_desktops)
            .map_err(|error| WorkspaceError::persistence_failed(&error))?; // 等待原子写入的完整备份文件。
        save_json_atomic(path, &store).map_err(|error| {
            crate::log_error!(
                "工作区：手动备份写入失败，路径={}，错误={error}",
                path.display()
            );
            WorkspaceError::persistence_failed(&error)
        })?;
        crate::log_info!(
            "工作区：手动备份已写入，布局={}，归档={}，路径={}",
            store.payload.layouts.len(),
            store.payload.archives.len(),
            path.display()
        );
        Ok(workspace_backup_summary(path, &store))
    }

    /// 读取、迁移并校验一个手动备份或自动快照。
    pub fn inspect_backup(&self, path: &Path) -> Result<WorkspaceBackupSummary, WorkspaceError> {
        validate_backup_path(path)?;
        let fallback_id = self.current_virtual_desktop_id()?; // 旧单布局迁移使用的当前 Windows 桌面身份。
        let store = load_workspace_backup(path, &fallback_id)?; // 只读临时副本中的已校验当前格式文件。
        crate::log_info!(
            "工作区：备份校验完成，Schema={}，布局={}，归档={}，路径={}",
            store.store_version,
            store.payload.layouts.len(),
            store.payload.archives.len(),
            path.display()
        );
        Ok(workspace_backup_summary(path, &store))
    }

    /// 返回当前仍保留且通过完整性校验的全部自动快照。
    pub fn auto_snapshot_summaries(&self) -> Result<Vec<WorkspaceBackupSummary>, WorkspaceError> {
        let fallback_id = self.current_virtual_desktop_id()?; // 旧自动快照迁移使用的当前桌面身份。
        let entries = match fs::read_dir(&self.auto_snapshot_directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(WorkspaceError::restore_failed(&error)),
        }; // 当前自动快照目录中的文件集合。
        let mut summaries = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension().and_then(|extension| extension.to_str()) == Some("json")
            })
            .filter_map(|path| match load_workspace_backup(&path, &fallback_id) {
                Ok(store) => Some(workspace_backup_summary(&path, &store)),
                Err(error) => {
                    crate::log_warn!(
                        "工作区：自动快照已损坏并跳过，路径={}，错误={}",
                        path.display(),
                        error.message
                    );
                    None
                }
            })
            .collect::<Vec<_>>(); // 已迁移并通过完整性校验的自动快照摘要。
        summaries.sort_by(|first, second| {
            second
                .created_at_unix
                .cmp(&first.created_at_unix)
                .then_with(|| second.path.cmp(&first.path))
        });
        Ok(summaries)
    }

    /// 返回备份中当前 Windows 环境尚不存在的虚拟桌面布局身份。
    pub(crate) fn missing_backup_virtual_desktop_ids(
        &self,
        path: &Path,
        current_virtual_desktop_id: &str,
        virtual_desktop_ids: &[String],
    ) -> Result<Vec<String>, WorkspaceError> {
        let store = load_workspace_backup(path, current_virtual_desktop_id)?; // 已完成迁移和完整性校验的备份候选。
        let active_ids = virtual_desktop_ids
            .iter()
            .map(String::as_str)
            .chain(std::iter::once(current_virtual_desktop_id))
            .collect::<std::collections::HashSet<_>>(); // 当前 Windows 保留不变的完整虚拟桌面集合。
        Ok(store
            .payload
            .layouts
            .iter()
            .map(|entry| entry.virtual_desktop_id.clone())
            .filter(|desktop_id| !active_ids.contains(desktop_id.as_str()))
            .collect())
    }

    /// 还原前同步快照，并以候选副本原子替换完整工作区。
    pub(crate) fn restore_backup(
        &self,
        path: &Path,
        current_virtual_desktop_id: &str,
        virtual_desktop_ids: &[String],
        desktop_id_mappings: &[(String, String)],
        monitors: &[DesktopHostMonitor],
        item_ids: &[String],
    ) -> Result<WorkspaceRestoreResult, WorkspaceError> {
        validate_id(current_virtual_desktop_id, "当前虚拟桌面标识无效")?;
        if monitors.is_empty() {
            return Err(WorkspaceError::invalid_command("工作区缺少显示器"));
        }
        let store = load_workspace_backup(path, current_virtual_desktop_id)?; // 未修改当前状态的已校验备份候选。
        let mapping = desktop_id_mappings
            .iter()
            .map(|(source, target)| (source.as_str(), target.as_str()))
            .collect::<std::collections::HashMap<_, _>>(); // 备份身份到新建 Windows 桌面身份的稳定映射。
        let mut active_ids = virtual_desktop_ids
            .iter()
            .cloned()
            .collect::<std::collections::HashSet<_>>(); // 还原时 Windows 当前完整桌面集合。
        active_ids.insert(current_virtual_desktop_id.to_string());
        let mut restored_layouts = HashMap::new(); // 等待按当前 Windows 身份提交的备份布局。
        for mut entry in store.payload.layouts {
            if entry.layout.components.is_empty() {
                entry.layout.components = self.default_components.clone();
            }
            let target_id = if active_ids.contains(&entry.virtual_desktop_id) {
                entry.virtual_desktop_id.clone()
            } else {
                mapping
                    .get(entry.virtual_desktop_id.as_str())
                    .filter(|target| active_ids.contains(**target))
                    .map(|target| (*target).to_string())
                    .ok_or_else(|| WorkspaceError::restore_failed(&"备份虚拟桌面映射缺失"))?
            }; // 保留现有身份或使用还原前新建桌面的最终身份。
            if restored_layouts.insert(target_id, entry.layout).is_some() {
                return Err(WorkspaceError::restore_failed(&"备份虚拟桌面映射重复"));
            }
        }
        let restored_layout_count = restored_layouts.len(); // 本次真正来自备份的活动布局数量。
        for desktop_id in &active_ids {
            restored_layouts
                .entry(desktop_id.clone())
                .or_insert_with(|| default_workspace_snapshot(&self.default_components));
        }

        let monitor_grids = self.monitor_grids(monitors)?; // 当前逐屏环境对应的持久网格集合。
        let primary_monitor_id = monitors
            .iter()
            .find(|monitor| monitor.primary)
            .unwrap_or(&monitors[0])
            .id
            .clone(); // 当前主显示器身份。
        let primary_grid = monitor_grids
            .iter()
            .find(|entry| entry.monitor_id == primary_monitor_id)
            .expect("主显示器网格已创建")
            .grid; // 旧布局迁移和新增项目使用的主显示器网格。
        for layout in restored_layouts.values_mut() {
            layout.interaction = WorkspaceInteractionSnapshot::default();
            let mut restored_item_ids = item_ids.to_vec(); // 保持当前 Shell 枚举顺序的可解析项目集合。
            let mut restored_item_set = restored_item_ids
                .iter()
                .cloned()
                .collect::<std::collections::HashSet<_>>(); // 当前已经加入恢复集合的项目引用。
            for item in &layout.unclassified_items {
                if restored_item_set.insert(item.stable_reference.clone()) {
                    restored_item_ids.push(item.stable_reference.clone());
                }
            } // 备份中暂时无法解析的引用按原布局顺序追加并保留。
            reconcile_layout_snapshot(
                layout,
                &monitor_grids,
                primary_grid,
                monitors,
                &restored_item_ids,
                &[],
                &[],
                &primary_monitor_id,
            )?;
            self.sort_fences(layout)?;
        }
        let mut candidate_current = restored_layouts
            .remove(current_virtual_desktop_id)
            .expect("当前虚拟桌面已建立候选布局"); // 还原后仍保持当前 Windows 桌面的活动布局。
        candidate_current.interaction = WorkspaceInteractionSnapshot::default();
        let mut archives = store.payload.archives; // 备份中等待补齐官方默认组件的删除布局归档。
        for archive in &mut archives {
            if archive.layout.components.is_empty() {
                archive.layout.components = self.default_components.clone();
            }
        }
        let mut candidate_virtual_desktops = WorkspaceVirtualDesktops {
            current_id: current_virtual_desktop_id.to_string(),
            layouts: restored_layouts,
            archives,
            explorer_layout_imported: store.payload.explorer_layout_imported,
        }; // 尚未影响当前状态的完整候选工作区。
        purge_expired_archives(
            &mut candidate_virtual_desktops.archives,
            Utc::now().timestamp(),
        );

        let mut current = self
            .snapshot
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())?; // 等待事务式替换的当前活动布局。
        let mut virtual_desktops = self
            .virtual_desktops
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())?; // 等待事务式替换的当前其他布局与归档。
        let mut history = self
            .history
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())?; // 原子保存前确认会话历史可以同步重置。
        let before_store = build_workspace_store(&current, &virtual_desktops)
            .map_err(|error| WorkspaceError::persistence_failed(&error))?; // 还原前完整工作区快照。
        write_auto_snapshot(
            &self.auto_snapshot_directory,
            current_virtual_desktop_id,
            &before_store,
        )
        .map_err(|error| WorkspaceError::persistence_failed(&error))?;
        let committed_store = save_workspace_store(
            &self.persistence_path,
            &candidate_current,
            &candidate_virtual_desktops,
        )
        .map_err(|error| {
            crate::log_error!("工作区：备份事务式还原保存失败，错误={error}");
            WorkspaceError::persistence_failed(&error)
        })?; // 原子写入成功前内存状态保持不变。
        *current = candidate_current.clone();
        *virtual_desktops = candidate_virtual_desktops;
        history.clear();
        self.schedule_auto_snapshot(current_virtual_desktop_id, committed_store);
        crate::log_info!(
            "工作区：完整备份已还原，布局={}，归档={}，当前桌面={}，路径={}",
            restored_layout_count,
            virtual_desktops.archives.len(),
            current_virtual_desktop_id,
            path.display()
        );
        Ok(WorkspaceRestoreResult {
            snapshot: candidate_current,
            restored_layout_count,
            message: format!("工作区还原完成：{restored_layout_count} 个布局"),
        })
    }

    /// 协调 Windows 当前虚拟桌面集合、活动身份、新建默认布局和删除归档。
    pub fn reconcile_virtual_desktops(
        &self,
        current_virtual_desktop_id: &str,
        virtual_desktop_ids: &[String],
        monitors: &[DesktopHostMonitor],
        item_ids: &[String],
    ) -> Result<WorkspaceSnapshot, WorkspaceError> {
        validate_id(current_virtual_desktop_id, "当前虚拟桌面标识无效")?;
        if monitors.is_empty() {
            return Err(WorkspaceError::invalid_command("工作区缺少显示器"));
        }
        let mut active_ids = virtual_desktop_ids
            .iter()
            .cloned()
            .collect::<std::collections::HashSet<_>>(); // Windows 当前完整虚拟桌面身份集合。
        active_ids.insert(current_virtual_desktop_id.to_string());
        let monitor_grids = self.monitor_grids(monitors)?; // 新建默认布局使用相同的网格偏好。
        let primary_monitor_id = monitors
            .iter()
            .find(|monitor| monitor.primary)
            .unwrap_or(&monitors[0])
            .id
            .clone(); // 新布局全部项目进入的当前主显示器身份。
        let primary_grid = monitor_grids
            .iter()
            .find(|entry| entry.monitor_id == primary_monitor_id)
            .expect("主显示器网格已创建")
            .grid; // 新布局兼容网格。
        let mut current = self
            .snapshot
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())?; // 当前活动虚拟桌面布局。
        let mut virtual_desktops = self
            .virtual_desktops
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())?; // 其他活动布局与删除归档。
        let mut next_current = current.clone(); // 保存失败时不影响当前活动布局的候选副本。
        let mut next_virtual_desktops = virtual_desktops.clone(); // 保存失败时不影响其他布局与归档的候选副本。
        let mut histories = self
            .history
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())?; // 原子保存前确认历史集合可以同步协调。
        let previous_current_id = next_virtual_desktops.current_id.clone(); // 切换前当前虚拟桌面身份。
        if previous_current_id != current_virtual_desktop_id {
            next_virtual_desktops
                .layouts
                .insert(previous_current_id.clone(), next_current.clone());
            next_current = next_virtual_desktops
                .layouts
                .remove(current_virtual_desktop_id)
                .unwrap_or_else(|| default_workspace_snapshot(&self.default_components));
            next_virtual_desktops.current_id = current_virtual_desktop_id.to_string();
        }

        let now = Utc::now().timestamp(); // 删除归档创建和过期清理使用的当前时间。
        let removed_ids = next_virtual_desktops
            .layouts
            .keys()
            .filter(|desktop_id| !active_ids.contains(*desktop_id))
            .cloned()
            .collect::<Vec<_>>(); // 本轮 Windows 已删除的非活动桌面身份。
        for desktop_id in removed_ids {
            if let Some(layout) = next_virtual_desktops.layouts.remove(&desktop_id) {
                next_virtual_desktops
                    .archives
                    .push(archive_layout(desktop_id, layout, now));
            }
        }
        purge_expired_archives(&mut next_virtual_desktops.archives, now);

        for desktop_id in &active_ids {
            if desktop_id == current_virtual_desktop_id
                || next_virtual_desktops.layouts.contains_key(desktop_id)
            {
                continue;
            }
            let mut layout = default_workspace_snapshot(&self.default_components); // 新虚拟桌面的官方默认布局。
            reconcile_layout_snapshot(
                &mut layout,
                &monitor_grids,
                primary_grid,
                monitors,
                item_ids,
                &[],
                &[],
                &primary_monitor_id,
            )?;
            next_virtual_desktops
                .layouts
                .insert(desktop_id.clone(), layout);
        }
        reconcile_layout_snapshot(
            &mut next_current,
            &monitor_grids,
            primary_grid,
            monitors,
            item_ids,
            &[],
            &[],
            &primary_monitor_id,
        )?;
        if self.sort_fences(&mut next_current)? {
            next_current.revision += 1;
        }
        for layout in next_virtual_desktops.layouts.values_mut() {
            reconcile_layout_snapshot(
                layout,
                &monitor_grids,
                primary_grid,
                monitors,
                item_ids,
                &[],
                &[],
                &primary_monitor_id,
            )?;
            if self.sort_fences(layout)? {
                layout.revision += 1;
            }
        }
        let store = save_workspace_store(
            &self.persistence_path,
            &next_current,
            &next_virtual_desktops,
        )
        .map_err(|error| {
            crate::log_error!("工作区：虚拟桌面协调保存失败，错误={error}");
            WorkspaceError::persistence_failed(&error)
        })?; // 当前完整虚拟桌面协调后的持久文件。
        *current = next_current.clone();
        *virtual_desktops = next_virtual_desktops;
        histories.retain(|desktop_id, _| active_ids.contains(desktop_id));
        self.schedule_auto_snapshot(current_virtual_desktop_id, store);
        crate::log_debug!(
            "工作区：虚拟桌面已协调，当前={}，活动布局={}，归档={}，切换={}",
            current_virtual_desktop_id,
            virtual_desktops.layouts.len() + 1,
            virtual_desktops.archives.len(),
            u8::from(previous_current_id != current_virtual_desktop_id)
        );
        Ok(next_current)
    }

    /// 将一个删除布局归档绑定到 Windows 新创建的虚拟桌面身份。
    pub fn restore_layout_archive(
        &self,
        archive_id: &str,
        new_virtual_desktop_id: &str,
        monitors: &[DesktopHostMonitor],
        item_ids: &[String],
    ) -> Result<WorkspaceSnapshot, WorkspaceError> {
        validate_id(archive_id, "布局归档标识无效")?;
        validate_id(new_virtual_desktop_id, "新虚拟桌面标识无效")?;
        if monitors.is_empty() {
            return Err(WorkspaceError::invalid_command("工作区缺少显示器"));
        }
        let mut current = self
            .snapshot
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())?; // 当前活动布局。
        let mut virtual_desktops = self
            .virtual_desktops
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())?; // 当前活动布局集合与归档。
        let mut next_current = current.clone(); // 保存失败时保持当前活动布局不变的候选副本。
        let mut next_virtual_desktops = virtual_desktops.clone(); // 保存失败时保持归档和其他布局不变的候选副本。
        let archive_index = next_virtual_desktops
            .archives
            .iter()
            .position(|archive| archive.archive_id == archive_id)
            .ok_or_else(|| WorkspaceError::invalid_command("布局归档不存在"))?; // 当前恢复目标归档索引。
        let mut layout = next_virtual_desktops.archives.remove(archive_index).layout; // 删除归档中恢复的完整布局。
        layout.interaction = WorkspaceInteractionSnapshot::default();
        let monitor_grids = self.monitor_grids(monitors)?; // 恢复目标当前逐屏网格。
        let primary_monitor_id = monitors
            .iter()
            .find(|monitor| monitor.primary)
            .unwrap_or(&monitors[0])
            .id
            .clone(); // 恢复布局当前主显示器身份。
        let primary_grid = monitor_grids
            .iter()
            .find(|entry| entry.monitor_id == primary_monitor_id)
            .expect("主显示器网格已创建")
            .grid; // 恢复布局兼容网格。
        reconcile_layout_snapshot(
            &mut layout,
            &monitor_grids,
            primary_grid,
            monitors,
            item_ids,
            &[],
            &[],
            &primary_monitor_id,
        )?;
        self.sort_fences(&mut layout)?;
        if next_virtual_desktops.current_id == new_virtual_desktop_id {
            next_current = layout;
        } else {
            next_virtual_desktops
                .layouts
                .insert(new_virtual_desktop_id.to_string(), layout);
        }
        let store = save_workspace_store(
            &self.persistence_path,
            &next_current,
            &next_virtual_desktops,
        )
        .map_err(|error| {
            crate::log_error!("工作区：布局归档恢复保存失败，错误={error}");
            WorkspaceError::persistence_failed(&error)
        })?; // 归档绑定后的完整持久文件。
        let current_id = next_virtual_desktops.current_id.clone(); // 自动快照所属的当前虚拟桌面身份。
        *current = next_current.clone();
        *virtual_desktops = next_virtual_desktops;
        self.schedule_auto_snapshot(&current_id, store);
        Ok(next_current)
    }

    /// 将宿主网格与 Shell 完整项目集合原子协调进 Workspace。
    #[cfg(test)]
    pub fn reconcile_items(
        &self,
        workspace_width: f64,
        workspace_height: f64,
        item_ids: &[String],
    ) -> Result<WorkspaceSnapshot, WorkspaceError> {
        self.reconcile_shell_items(workspace_width, workspace_height, item_ids, &[])
    }

    /// 将精确重命名事实和最终 Shell 项目集合协调进当前布局。
    #[cfg(test)]
    pub fn reconcile_shell_items(
        &self,
        workspace_width: f64,
        workspace_height: f64,
        item_ids: &[String],
        renames: &[(String, String)],
    ) -> Result<WorkspaceSnapshot, WorkspaceError> {
        let monitor = DesktopHostMonitor {
            id: "primary".to_string(),
            name: "primary".to_string(),
            primary: true,
            x: 0,
            y: 0,
            width: workspace_width.ceil().max(1.0) as u32,
            height: workspace_height.ceil().max(1.0) as u32,
            work_x: 0,
            work_y: 0,
            work_width: workspace_width.ceil().max(1.0) as u32,
            work_height: workspace_height.ceil().max(1.0) as u32,
            scale_factor: 1.0,
            dpi: 96,
        }; // 单显示器兼容调用使用的合成宿主状态。
        self.reconcile_host_monitors(&[monitor], item_ids, &[], renames)
    }

    /// 将全部显示器网格、项目归属、临时投射和 Shell 最终事实原子协调进 Workspace。
    pub fn reconcile_host_monitors(
        &self,
        monitors: &[DesktopHostMonitor],
        item_ids: &[String],
        assignments: &[(String, String)],
        renames: &[(String, String)],
    ) -> Result<WorkspaceSnapshot, WorkspaceError> {
        if monitors.is_empty() {
            return Err(WorkspaceError::invalid_command("工作区缺少显示器"));
        }
        let monitor_grids = self.monitor_grids(monitors)?; // 当前全部显示器对应的独立 DIP 网格。
        let primary_monitor_id = monitors
            .iter()
            .find(|monitor| monitor.primary)
            .unwrap_or(&monitors[0])
            .id
            .clone(); // 临时投射和旧快照迁移使用的主显示器身份。
        let primary_grid = monitor_grids
            .iter()
            .find(|entry| entry.monitor_id == primary_monitor_id)
            .expect("主显示器网格已创建")
            .grid; // 保留给旧调用方的主显示器兼容网格。
        // 等待项目协调的当前活动工作区快照。
        let mut current = self
            .snapshot
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())?;
        let mut virtual_desktops = self
            .virtual_desktops
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())?; // 当前其他活动虚拟桌面布局与归档。
        let mut histories = self
            .history
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())?; // 等待同步 Shell 重命名引用的全部布局历史。
        // 保持当前快照可回滚的项目协调副本。
        let mut next = current.clone();
        let mut next_virtual_desktops = virtual_desktops.clone(); // 保存失败时保持其他布局和首次导入状态不变的候选副本。
        let mut next_histories = histories.clone(); // 保存失败时保持会话历史引用不变的候选副本。
        let current_assignments = if next_virtual_desktops.explorer_layout_imported {
            &[][..]
        } else {
            assignments
        }; // Explorer 原始位置只导入到首次接管时的当前虚拟桌面。
        let mut changed = reconcile_layout_snapshot(
            &mut next,
            &monitor_grids,
            primary_grid,
            monitors,
            item_ids,
            current_assignments,
            renames,
            &primary_monitor_id,
        )?; // 当前活动虚拟桌面的 Shell 与显示器协调结果。
        if self.sort_fences(&mut next)? {
            next.revision += 1;
            changed = true;
        }
        for layout in next_virtual_desktops.layouts.values_mut() {
            changed |= reconcile_layout_snapshot(
                layout,
                &monitor_grids,
                primary_grid,
                monitors,
                item_ids,
                &[],
                renames,
                &primary_monitor_id,
            )?;
            if self.sort_fences(layout)? {
                layout.revision += 1;
                changed = true;
            }
        }
        if !next_virtual_desktops.explorer_layout_imported {
            next_virtual_desktops.explorer_layout_imported = true;
            changed = true;
        }
        for history in next_histories.values_mut() {
            for entry in history.undo.iter_mut().chain(history.redo.iter_mut()) {
                entry.layout.remap_item_references(renames)?;
                entry.layout.validate()?;
            }
        } // Shell 精确重命名同步到每个虚拟桌面的撤销与重做快照。
        if !changed {
            *histories = next_histories;
            return Ok(current.clone());
        }
        let store = save_workspace_store(&self.persistence_path, &next, &next_virtual_desktops)
            .map_err(|error| {
                crate::log_error!("工作区：桌面项目协调保存失败，错误={error}");
                WorkspaceError::persistence_failed(&error)
            })?; // Shell 与显示器协调后的完整持久文件。
        let current_id = next_virtual_desktops.current_id.clone(); // 自动快照所属的当前虚拟桌面身份。
        *current = next.clone();
        *virtual_desktops = next_virtual_desktops;
        *histories = next_histories;
        self.schedule_auto_snapshot(&current_id, store);
        crate::log_debug!(
            "工作区：全部虚拟桌面逐屏项目位置已协调，活动布局={}，修订={}，项目={}，显示器={}",
            virtual_desktops.layouts.len() + 1,
            next.revision,
            next.unclassified_items.len(),
            next.monitor_grids.len()
        );
        Ok(next)
    }

    /// 校验并原子提交一个完整布局事务。
    #[cfg(test)]
    pub fn apply(&self, command: WorkspaceCommand) -> Result<WorkspaceSnapshot, WorkspaceError> {
        self.apply_on_monitor(None, command)
    }

    /// 在指定逐屏宿主上下文中提交一个完整布局事务。
    pub fn apply_on_monitor(
        &self,
        monitor_id: Option<&str>,
        command: WorkspaceCommand,
    ) -> Result<WorkspaceSnapshot, WorkspaceError> {
        if matches!(&command, WorkspaceCommand::UndoLayout) {
            return self
                .restore_layout_history(false)
                .map(|result| result.snapshot);
        }
        if matches!(&command, WorkspaceCommand::RedoLayout) {
            return self
                .restore_layout_history(true)
                .map(|result| result.snapshot);
        }
        // 围栏滚动由拖动事务或当前滚动位置直接提交，不形成独立撤销单元。
        let records_history = !matches!(
            &command,
            WorkspaceCommand::SetFenceScroll { .. } | WorkspaceCommand::UpdateComponentState { .. }
        );
        // 等待事务提交的当前工作区快照。
        let mut current = self
            .snapshot
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())?;
        let virtual_desktops = self
            .virtual_desktops
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())?; // 当前布局事务所属虚拟桌面和其他持久布局。
        let virtual_desktop_id = virtual_desktops.current_id.clone(); // 当前布局事务所属的虚拟桌面身份。
        // 保持当前快照可回滚的事务副本。
        let mut next = current.clone();
        // 布局事务前的运行期交互状态。
        let interaction = next.interaction.clone();
        // 当前命令用于中文日志的稳定名称。
        let command_name = next.apply(command, monitor_id)?;
        self.sort_fences(&mut next)?;
        if next == *current {
            crate::log_debug!("工作区：布局事务保持当前状态，命令={command_name}");
            return Ok(current.clone());
        }
        if next.interaction != interaction {
            next.interaction.revision += 1;
        }
        next.revision += 1;
        next.validate()?;
        let mut histories = self
            .history
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())?; // 原子保存前确认会话历史可以同步提交。
        let store = save_workspace_store(&self.persistence_path, &next, &virtual_desktops)
            .map_err(|error| {
                crate::log_error!("工作区：原子快照保存失败，命令={command_name}，错误={error}");
                WorkspaceError::persistence_failed(&error)
            })?; // 当前布局事务完成后的完整工作区文件。
        // 保存成功后更新当前虚拟桌面独立的会话布局历史。
        let history = histories.entry(virtual_desktop_id.clone()).or_default(); // 当前虚拟桌面独立的撤销与重做栈。
        if records_history {
            history.undo.push(WorkspaceHistoryEntry {
                layout: layout_snapshot(&current),
                timeline_at_unix_micros: crate::tools::time::next_timeline_timestamp(),
            });
            history.redo.clear();
        }
        *current = next.clone();
        self.schedule_auto_snapshot(&virtual_desktop_id, store);
        crate::log_debug!(
            "工作区：布局事务已提交，命令={command_name}，修订={}，围栏={}",
            next.revision,
            next.fences.len()
        );
        Ok(next)
    }

    /// 从当前会话布局历史撤销或重做一个完整事务。
    pub(crate) fn restore_layout_history(
        &self,
        redo: bool,
    ) -> Result<WorkspaceLayoutHistoryResult, WorkspaceError> {
        // 等待历史恢复的当前权威工作区快照。
        let mut current = self
            .snapshot
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())?;
        let virtual_desktops = self
            .virtual_desktops
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())?; // 当前历史恢复需要一同保存的其他布局。
        let virtual_desktop_id = virtual_desktops.current_id.clone(); // 当前历史恢复所属的虚拟桌面身份。
        let mut histories = self
            .history
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())?; // 当前全部虚拟桌面会话历史。
        let history = histories.entry(virtual_desktop_id.clone()).or_default(); // 当前虚拟桌面独立历史。
        let target = if redo {
            history.redo.last().map(|entry| entry.layout.clone())
        } else {
            history.undo.last().map(|entry| entry.layout.clone())
        }; // 当前撤销方向对应的最近历史布局。
        let Some(target) = target else {
            crate::log_debug!(
                "工作区：布局历史保持当前状态，方向={}",
                if redo { "重做" } else { "撤销" }
            );
            return Ok(WorkspaceLayoutHistoryResult {
                snapshot: current.clone(),
                restored_count: 0,
                changed_count: 0,
            });
        };

        // 使用历史布局和当前选择焦点生成下一份权威快照。
        let mut target = target; // 旧历史先转换到现行步进，再合并 Shell 新项目。
        reflow_grid_items(&mut target, &current.monitor_grids)?;
        let (mut next, restored_count, changed_count) =
            merge_layout_history_target(target, &current); // 保留外部新增项目并排除已离开工作区的历史引用。
        // 撤销和重做期间需要保留的当前围栏滚动位置。
        let current_scrolls = current
            .fences
            .iter()
            .map(|fence| (fence.id.as_str(), fence.scroll_top))
            .collect::<std::collections::HashMap<_, _>>();
        for fence in &mut next.fences {
            if let Some(scroll_top) = current_scrolls.get(fence.id.as_str()) {
                fence.scroll_top = *scroll_top;
            }
        }
        next.revision = current.revision.saturating_add(1);
        next.interaction = current.interaction.clone();
        self.sort_fences(&mut next)?;
        next.normalize_interaction();
        if next.interaction != current.interaction {
            next.interaction.revision = current.interaction.revision.saturating_add(1);
        }
        next.validate()?;
        let store = save_workspace_store(&self.persistence_path, &next, &virtual_desktops)
            .map_err(|error| {
                crate::log_error!("工作区：布局历史保存失败，错误={error}");
                WorkspaceError::persistence_failed(&error)
            })?; // 当前历史恢复后的完整工作区文件。

        // 历史恢复成功后移动对应撤销单元。
        let current_layout = WorkspaceHistoryEntry {
            layout: layout_snapshot(&current),
            timeline_at_unix_micros: crate::tools::time::next_timeline_timestamp(),
        }; // 当前方向完成后进入相反方向的布局历史单元。
        if redo {
            history.redo.pop();
            history.undo.push(current_layout);
        } else {
            history.undo.pop();
            history.redo.push(current_layout);
        }
        *current = next.clone();
        self.schedule_auto_snapshot(&virtual_desktop_id, store);
        crate::log_debug!(
            "工作区：布局历史已恢复，方向={}，修订={}",
            if redo { "重做" } else { "撤销" },
            next.revision
        );
        Ok(WorkspaceLayoutHistoryResult {
            snapshot: next,
            restored_count,
            changed_count,
        })
    }

    /// 返回当前虚拟桌面在指定方向最近的布局历史时间。
    pub(crate) fn latest_layout_history_time(
        &self,
        redo: bool,
    ) -> Result<Option<i64>, WorkspaceError> {
        let virtual_desktop_id = self.current_virtual_desktop_id()?; // 当前统一时间线所属虚拟桌面身份。
        let histories = self
            .history
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())?; // 当前全部虚拟桌面会话历史。
        let Some(history) = histories.get(&virtual_desktop_id) else {
            return Ok(None);
        };
        Ok(if redo {
            history.redo.last()
        } else {
            history.undo.last()
        }
        .map(|entry| entry.timeline_at_unix_micros))
    }

    /// 新 Shell 操作提交后清除当前虚拟桌面的布局重做分支。
    pub(crate) fn clear_redo_history(&self) -> Result<(), WorkspaceError> {
        let virtual_desktop_id = self.current_virtual_desktop_id()?; // 新 Shell 分支所属的当前虚拟桌面身份。
        if let Some(history) = self
            .history
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())?
            .get_mut(&virtual_desktop_id)
        {
            history.redo.clear();
        }
        Ok(())
    }

    /// 将最新完整工作区状态提交给 2 秒空闲合并线程。
    fn schedule_auto_snapshot(&self, virtual_desktop_id: &str, store: PersistedWorkspaceStore) {
        if self
            .auto_snapshot_sender
            .as_ref()
            .expect("自动快照发送端在工作区生命周期内存在")
            .send(AutoSnapshotRequest {
                virtual_desktop_id: virtual_desktop_id.to_string(),
                store,
            })
            .is_err()
        {
            crate::log_warn!("工作区：自动快照线程已经停止");
        }
    }

    /// 校验并提交一个不写入布局文件的运行期交互事务。
    #[cfg(test)]
    pub fn apply_interaction(
        &self,
        command: WorkspaceInteractionCommand,
    ) -> Result<WorkspaceSnapshot, WorkspaceError> {
        let monitor_id = self
            .snapshot()?
            .monitor_grids
            .first()
            .map(|entry| entry.monitor_id.clone())
            .ok_or_else(|| WorkspaceError::invalid_command("工作区显示器尚未初始化"))?;
        self.apply_interaction_on_monitor(&monitor_id, command)
    }

    /// 在指定逐屏宿主上下文中提交一个运行期交互事务。
    pub fn apply_interaction_on_monitor(
        &self,
        monitor_id: &str,
        command: WorkspaceInteractionCommand,
    ) -> Result<WorkspaceSnapshot, WorkspaceError> {
        // 等待交互事务提交的当前工作区快照。
        let mut current = self
            .snapshot
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())?;
        // 保持当前交互状态可回滚的事务副本。
        let mut next = current.clone();
        next.command_monitor_id(Some(monitor_id))?;
        // 当前交互命令用于中文日志的稳定名称。
        let command_name = next.apply_interaction(command, monitor_id)?;
        if next == *current {
            return Ok(current.clone());
        }

        next.interaction.revision += 1;
        next.validate()?;
        *current = next.clone();
        crate::log_debug!(
            "工作区：交互事务已提交，命令={command_name}，交互修订={}，选择数量={}",
            next.interaction.revision,
            next.interaction.selected_item_ids.len()
        );
        Ok(next)
    }
}

impl Drop for WorkspaceState {
    /// 关闭发送端并等待后台线程刷新退出前待处理快照。
    fn drop(&mut self) {
        self.auto_snapshot_sender.take();
        if let Some(worker) = self.auto_snapshot_worker.take() {
            let _ = worker.join();
        }
    }
}

/// 启动按虚拟桌面独立合并的自动快照写入线程。
fn start_auto_snapshot_worker(
    directory: PathBuf,
) -> std::io::Result<(Sender<AutoSnapshotRequest>, JoinHandle<()>)> {
    let (sender, receiver) = mpsc::channel::<AutoSnapshotRequest>(); // 布局提交线程到磁盘快照线程的请求通道。
    let worker = thread::Builder::new()
        .name("desktop-tools-workspace-snapshot".to_string())
        .spawn(move || {
            let mut pending = HashMap::<String, (Instant, PersistedWorkspaceStore)>::new(); // 每个虚拟桌面最后一次待合并状态。
            loop {
                let request = if pending.is_empty() {
                    receiver.recv().map_err(|_| RecvTimeoutError::Disconnected)
                } else {
                    let now = Instant::now(); // 当前最早快照截止时间计算基准。
                    let timeout = pending
                        .values()
                        .map(|(deadline, _)| deadline.saturating_duration_since(now))
                        .min()
                        .unwrap_or(AUTO_SNAPSHOT_IDLE); // 下一份快照到期前的精确等待时间。
                    receiver.recv_timeout(timeout)
                };
                match request {
                    Ok(request) => {
                        pending.insert(
                            request.virtual_desktop_id,
                            (Instant::now() + AUTO_SNAPSHOT_IDLE, request.store),
                        );
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => {
                        for (desktop_id, (_, store)) in pending.drain() {
                            if let Err(error) = write_auto_snapshot(&directory, &desktop_id, &store)
                            {
                                crate::log_error!(
                                    "工作区：退出前自动快照写入失败，桌面={}，错误={error}",
                                    desktop_id
                                );
                            }
                        }
                        break;
                    }
                }
                let now = Instant::now(); // 本轮已经到期的虚拟桌面快照判断基准。
                let due = pending
                    .iter()
                    .filter(|(_, (deadline, _))| *deadline <= now)
                    .map(|(desktop_id, _)| desktop_id.clone())
                    .collect::<Vec<_>>(); // 本轮需要落盘的虚拟桌面身份。
                for desktop_id in due {
                    let (_, store) = pending
                        .remove(&desktop_id)
                        .expect("到期自动快照仍在待处理集合");
                    if let Err(error) = write_auto_snapshot(&directory, &desktop_id, &store) {
                        crate::log_error!(
                            "工作区：自动快照写入失败，桌面={}，错误={error}",
                            desktop_id
                        );
                    }
                }
            }
        })?;
    Ok((sender, worker))
}

/// 原子写入一份完整工作区自动快照并保留当前桌面最近 20 份。
fn write_auto_snapshot(
    directory: &Path,
    virtual_desktop_id: &str,
    store: &PersistedWorkspaceStore,
) -> std::io::Result<PathBuf> {
    fs::create_dir_all(directory)?;
    let prefix = format!("{:016x}-", stable_hash(virtual_desktop_id.as_bytes())); // 避免 Windows 身份直接进入文件名的稳定前缀。
    let timestamp = Utc::now().timestamp_micros(); // 自动快照按创建时间排序的 Unix 微秒值。
    let sequence = NEXT_AUTO_SNAPSHOT_ID.fetch_add(1, Ordering::Relaxed); // 同一时钟刻度内避免覆盖的进程内序号。
    let path = directory.join(format!("{prefix}{timestamp}-{sequence}.json")); // 当前完整工作区自动快照路径。
    save_json_atomic(&path, store)?;
    let mut snapshots = fs::read_dir(directory)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".json"))
        })
        .collect::<Vec<_>>(); // 当前虚拟桌面全部自动快照文件。
    snapshots.sort();
    let remove_count = snapshots.len().saturating_sub(AUTO_SNAPSHOT_LIMIT); // 超出最近 20 份配额的旧文件数量。
    for obsolete in snapshots.into_iter().take(remove_count) {
        fs::remove_file(obsolete)?;
    }
    crate::log_debug!(
        "工作区：自动快照已写入，桌面={}，路径={}",
        virtual_desktop_id,
        path.display()
    );
    Ok(path)
}

/// 从文件读取备份并在临时副本中完成迁移和完整性校验。
fn load_workspace_backup(
    path: &Path,
    fallback_virtual_desktop_id: &str,
) -> Result<PersistedWorkspaceStore, WorkspaceError> {
    validate_backup_path(path)?;
    let value = load_json::<serde_json::Value>(path)
        .map_err(|error| {
            crate::log_warn!(
                "工作区：备份读取失败，路径={}，错误={error}",
                path.display()
            );
            WorkspaceError::restore_failed(&error)
        })?
        .ok_or_else(|| {
            crate::log_warn!("工作区：备份文件不存在，路径={}", path.display());
            WorkspaceError::restore_failed(&"工作区备份文件不存在")
        })?; // 等待格式识别的只读 JSON。
    decode_workspace_store(value, fallback_virtual_desktop_id).inspect_err(|error| {
        crate::log_warn!(
            "工作区：备份校验或迁移失败，路径={}，错误={}",
            path.display(),
            error.message
        );
    })
}

/// 生成命令边界使用的已校验备份摘要。
fn workspace_backup_summary(
    path: &Path,
    store: &PersistedWorkspaceStore,
) -> WorkspaceBackupSummary {
    WorkspaceBackupSummary {
        schema_version: store.store_version,
        created_at_unix: store.created_at_unix,
        path: path.to_string_lossy().into_owned(),
        layout_count: store.payload.layouts.len(),
        archive_count: store.payload.archives.len(),
        checksum: store.checksum.clone(),
    }
}

/// 校验用户传入的备份路径可以明确指向 JSON 文件。
fn validate_backup_path(path: &Path) -> Result<(), WorkspaceError> {
    if path.as_os_str().is_empty()
        || path.file_name().is_none()
        || !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
    {
        return Err(WorkspaceError::invalid_command(
            "工作区备份路径必须指向 .json 文件",
        ));
    }
    Ok(())
}

/// 使用固定 FNV-1a 64 位算法生成文件名安全的稳定身份散列。
fn stable_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64; // FNV-1a 64 位偏移基数。
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

/// 从正式宿主显示器状态生成逐屏工作区网格。
fn monitor_grids(
    monitors: &[DesktopHostMonitor],
    preferences: GridPreferences,
) -> Result<Vec<WorkspaceMonitorGrid>, WorkspaceError> {
    monitors
        .iter()
        .map(|monitor| {
            Ok(WorkspaceMonitorGrid {
                monitor_id: monitor.id.clone(),
                grid: workspace_grid(
                    f64::from(monitor.work_width) / monitor.scale_factor,
                    f64::from(monitor.work_height) / monitor.scale_factor,
                    preferences,
                )?,
            })
        })
        .collect()
}

/// 网格步进改变时按原先纵向顺序重排；不触及断开显示器或围栏内的来源记录。
fn reflow_grid_items(
    layout: &mut WorkspaceSnapshot,
    grids: &[WorkspaceMonitorGrid],
) -> Result<(), WorkspaceError> {
    let members = layout
        .fences
        .iter()
        .flat_map(|fence| fence.members.iter().cloned())
        .collect::<std::collections::HashSet<_>>(); // 围栏成员不占桌面格位。
    for entry in grids {
        let previous = layout
            .monitor_grids
            .iter()
            .find(|old| old.monitor_id == entry.monitor_id)
            .map(|old| old.grid); // 此显示器上次使用的步进。
        if previous.is_none_or(|old| {
            old.column_width == entry.grid.column_width && old.row_height == entry.grid.row_height
        }) {
            continue;
        }
        let mut items = layout
            .unclassified_items
            .iter_mut()
            .filter(|item| {
                item.monitor_id == entry.monitor_id && !members.contains(&item.stable_reference)
            })
            .collect::<Vec<_>>(); // 本屏可见桌面项目。
        if items.len() > grid_capacity(entry.grid)? as usize {
            return Err(WorkspaceError::invalid_command(
                "新网格空间不足，请减小格子尺寸或间距",
            ));
        }
        items.sort_by(|a, b| {
            a.position
                .left
                .total_cmp(&b.position.left)
                .then(a.position.top.total_cmp(&b.position.top))
        });
        for (index, item) in items.into_iter().enumerate() {
            item.position = grid_position(entry.grid, index as u32);
            item.projection_position = None;
            item.projection_monitor_id = None;
        }
    }
    Ok(())
}

/// 将一次 Shell 与显示器环境变化协调到单个虚拟桌面布局。
#[allow(clippy::too_many_arguments)]
fn reconcile_layout_snapshot(
    layout: &mut WorkspaceSnapshot,
    monitor_grids: &[WorkspaceMonitorGrid],
    primary_grid: WorkspaceGrid,
    monitors: &[DesktopHostMonitor],
    item_ids: &[String],
    assignments: &[(String, String)],
    renames: &[(String, String)],
    primary_monitor_id: &str,
) -> Result<bool, WorkspaceError> {
    let previous = layout.clone(); // 本次协调前用于判断是否产生状态变化的布局。
    let interaction = layout.interaction.clone(); // 协调前当前桌面运行期选择与焦点。
    reflow_grid_items(layout, monitor_grids)?;
    layout.grid = primary_grid;
    layout.monitor_grids = monitor_grids.to_vec();
    layout.remap_item_references(renames)?;
    layout.reconcile_items_for_monitors(item_ids, assignments, primary_monitor_id)?;
    layout.reconcile_monitor_projections(monitors, primary_monitor_id)?;
    if *layout == previous {
        return Ok(false);
    }
    if layout.interaction != interaction {
        layout.interaction.revision = layout.interaction.revision.saturating_add(1);
    }
    layout.revision = layout.revision.saturating_add(1);
    layout.validate()?;
    Ok(true)
}

/// 恢复多虚拟桌面存储，并兼容迁移旧单布局快照。
fn restore_workspace_store(
    value: Option<serde_json::Value>,
    current_virtual_desktop_id: &str,
    virtual_desktop_ids: &[String],
    default_components: &[ComponentInstanceSnapshot],
) -> Result<(WorkspaceSnapshot, WorkspaceVirtualDesktops), WorkspaceError> {
    let now = Utc::now().timestamp(); // 当前离线删除布局归档使用的 Unix 时间。
    let mut active_ids = virtual_desktop_ids
        .iter()
        .cloned()
        .collect::<std::collections::HashSet<_>>(); // 当前 Windows 仍存在的虚拟桌面身份。
    active_ids.insert(current_virtual_desktop_id.to_string());
    let has_persisted_value = value.is_some(); // 当前是否存在已保存的旧或新格式工作区。
    let mut layouts = HashMap::new(); // 等待按当前 Windows 身份协调的持久布局。
    let mut archives = Vec::new(); // 已恢复或离线删除产生的布局归档。
    let mut explorer_layout_imported = has_persisted_value; // 旧单布局文件已经完成过 Explorer 首次导入。
    if let Some(value) = value {
        let store = decode_workspace_store(value, current_virtual_desktop_id)?; // 已校验并迁移到当前 Schema 的临时文件副本。
        explorer_layout_imported = store.payload.explorer_layout_imported;
        for mut entry in store.payload.layouts {
            if entry.layout.components.is_empty() {
                entry.layout.components = default_components.to_vec();
            }
            if active_ids.contains(&entry.virtual_desktop_id) {
                layouts.insert(entry.virtual_desktop_id, entry.layout);
            } else {
                archives.push(archive_layout(entry.virtual_desktop_id, entry.layout, now));
            }
        }
        archives.extend(store.payload.archives.into_iter().map(|mut archive| {
            if archive.layout.components.is_empty() {
                archive.layout.components = default_components.to_vec();
            }
            archive
        }));
    }
    for desktop_id in &active_ids {
        layouts
            .entry(desktop_id.clone())
            .or_insert_with(|| default_workspace_snapshot(default_components));
    }
    let snapshot = layouts
        .remove(current_virtual_desktop_id)
        .unwrap_or_else(|| default_workspace_snapshot(default_components)); // 当前 Windows 桌面激活的独立布局。
    Ok((
        snapshot,
        WorkspaceVirtualDesktops {
            current_id: current_virtual_desktop_id.to_string(),
            layouts,
            archives,
            explorer_layout_imported,
        },
    ))
}

/// 将当前、Ticket 10 和旧单布局 JSON 迁移到当前完整性格式。
fn decode_workspace_store(
    value: serde_json::Value,
    fallback_virtual_desktop_id: &str,
) -> Result<PersistedWorkspaceStore, WorkspaceError> {
    if let Ok(store) = serde_json::from_value::<PersistedWorkspaceStore>(value.clone()) {
        if store.store_version > WORKSPACE_STORE_VERSION {
            return Err(WorkspaceError::restore_failed(
                &"工作区备份 Schema 版本过新",
            ));
        }
        if store.store_version != WORKSPACE_STORE_VERSION {
            return Err(WorkspaceError::restore_failed(
                &"工作区备份 Schema 迁移失败",
            ));
        }
        validate_workspace_store(&store)?;
        return Ok(store);
    }
    if let Ok(legacy) = serde_json::from_value::<LegacyPersistedWorkspaceStoreV2>(value.clone()) {
        if legacy.store_version > 2 {
            return Err(WorkspaceError::restore_failed(
                &"工作区备份 Schema 版本过新",
            ));
        }
        if legacy.store_version != 2 {
            return Err(WorkspaceError::restore_failed(
                &"工作区备份 Schema 迁移失败",
            ));
        }
        let store = workspace_store_from_payload(PersistedWorkspacePayload {
            current_virtual_desktop_id: legacy.current_virtual_desktop_id,
            layouts: legacy.layouts,
            archives: legacy.archives,
            explorer_layout_imported: legacy.explorer_layout_imported,
        })?; // Ticket 10 格式在临时副本中补齐时间与完整性字段。
        validate_workspace_store(&store)?;
        crate::log_info!("工作区：备份 Schema 已从版本 2 迁移到版本 3");
        return Ok(store);
    }
    let legacy = serde_json::from_value::<WorkspaceSnapshot>(value)
        .map_err(|error| WorkspaceError::restore_failed(&format!("工作区备份迁移失败: {error}")))?; // 最早单布局文件迁移来源。
    let store = workspace_store_from_payload(PersistedWorkspacePayload {
        current_virtual_desktop_id: fallback_virtual_desktop_id.to_string(),
        layouts: vec![PersistedVirtualDesktopLayout {
            virtual_desktop_id: fallback_virtual_desktop_id.to_string(),
            layout: legacy,
        }],
        archives: Vec::new(),
        explorer_layout_imported: true,
    })?;
    validate_workspace_store(&store)?;
    crate::log_info!("工作区：旧单布局快照已迁移到备份 Schema 版本 3");
    Ok(store)
}

/// 校验完整工作区文件的版本、校验值、身份和每份布局。
fn validate_workspace_store(store: &PersistedWorkspaceStore) -> Result<(), WorkspaceError> {
    if store.store_version != WORKSPACE_STORE_VERSION {
        return Err(WorkspaceError::restore_failed(
            &"工作区备份 Schema 版本无效",
        ));
    }
    let checksum =
        workspace_store_checksum(store.store_version, store.created_at_unix, &store.payload)?; // 当前版本、创建时间和载荷重新计算的完整性校验值。
    if checksum != store.checksum {
        return Err(WorkspaceError::restore_failed(&"工作区备份完整性校验失败"));
    }
    validate_id(
        &store.payload.current_virtual_desktop_id,
        "备份当前虚拟桌面标识无效",
    )
    .map_err(|error| WorkspaceError::restore_failed(&error.message))?;
    let mut desktop_ids = std::collections::HashSet::new(); // 备份中已经出现的活动桌面身份。
    for entry in &store.payload.layouts {
        validate_id(&entry.virtual_desktop_id, "备份虚拟桌面标识无效")
            .map_err(|error| WorkspaceError::restore_failed(&error.message))?;
        if !desktop_ids.insert(entry.virtual_desktop_id.as_str()) {
            return Err(WorkspaceError::restore_failed(
                &"工作区备份包含重复虚拟桌面",
            ));
        }
        let mut layout = entry.layout.clone(); // 校验时排除不应持久化的运行期交互状态。
        layout.interaction = WorkspaceInteractionSnapshot::default();
        layout
            .validate()
            .map_err(|error| WorkspaceError::restore_failed(&error.message))?;
    }
    if !desktop_ids.contains(store.payload.current_virtual_desktop_id.as_str()) {
        return Err(WorkspaceError::restore_failed(
            &"工作区备份缺少当前虚拟桌面布局",
        ));
    }
    let mut archive_ids = std::collections::HashSet::new(); // 备份中已经出现的归档标识。
    for archive in &store.payload.archives {
        validate_id(&archive.archive_id, "布局归档标识无效")
            .map_err(|error| WorkspaceError::restore_failed(&error.message))?;
        validate_id(
            &archive.source_virtual_desktop_id,
            "布局归档来源桌面标识无效",
        )
        .map_err(|error| WorkspaceError::restore_failed(&error.message))?;
        if archive.expires_at_unix <= archive.archived_at_unix
            || !archive_ids.insert(archive.archive_id.as_str())
        {
            return Err(WorkspaceError::restore_failed(&"工作区布局归档状态无效"));
        }
        archive
            .layout
            .validate()
            .map_err(|error| WorkspaceError::restore_failed(&error.message))?;
    }
    Ok(())
}

/// 将已删除虚拟桌面布局转换为 30 天可恢复归档。
fn archive_layout(
    source_virtual_desktop_id: String,
    layout: WorkspaceSnapshot,
    archived_at_unix: i64,
) -> WorkspaceLayoutArchive {
    WorkspaceLayoutArchive {
        archive_id: format!("archive-{source_virtual_desktop_id}-{archived_at_unix}"),
        source_virtual_desktop_id,
        archived_at_unix,
        expires_at_unix: archived_at_unix + LAYOUT_ARCHIVE_RETENTION_SECONDS,
        layout: persisted_layout(&layout),
    }
}

/// 移除已经超过 30 天保留期的布局归档。
fn purge_expired_archives(archives: &mut Vec<WorkspaceLayoutArchive>, now_unix: i64) {
    archives.retain(|archive| archive.expires_at_unix > now_unix);
}

/// 保存排除运行期选择与焦点的工作区布局快照。
fn save_workspace_store(
    path: &std::path::Path,
    snapshot: &WorkspaceSnapshot,
    virtual_desktops: &WorkspaceVirtualDesktops,
) -> std::io::Result<PersistedWorkspaceStore> {
    let store = build_workspace_store(snapshot, virtual_desktops)?; // 已排序并计算完整性字段的当前工作区文件。
    save_json_atomic(path, &store)?;
    Ok(store)
}

/// 从当前内存状态生成稳定排序的完整工作区文件。
fn build_workspace_store(
    snapshot: &WorkspaceSnapshot,
    virtual_desktops: &WorkspaceVirtualDesktops,
) -> std::io::Result<PersistedWorkspaceStore> {
    let mut layouts = virtual_desktops
        .layouts
        .iter()
        .map(
            |(virtual_desktop_id, layout)| PersistedVirtualDesktopLayout {
                virtual_desktop_id: virtual_desktop_id.clone(),
                layout: persisted_layout(layout),
            },
        )
        .collect::<Vec<_>>(); // 当前全部非活动虚拟桌面布局。
    layouts.push(PersistedVirtualDesktopLayout {
        virtual_desktop_id: virtual_desktops.current_id.clone(),
        layout: persisted_layout(snapshot),
    });
    layouts.sort_by(|first, second| first.virtual_desktop_id.cmp(&second.virtual_desktop_id));
    workspace_store_from_payload(PersistedWorkspacePayload {
        current_virtual_desktop_id: virtual_desktops.current_id.clone(),
        layouts,
        archives: virtual_desktops.archives.clone(),
        explorer_layout_imported: virtual_desktops.explorer_layout_imported,
    })
    .map_err(|error| std::io::Error::other(error.message))
}

/// 为已经迁移和排序的载荷补齐版本、时间与完整性校验值。
fn workspace_store_from_payload(
    payload: PersistedWorkspacePayload,
) -> Result<PersistedWorkspaceStore, WorkspaceError> {
    let created_at_unix = Utc::now().timestamp(); // 当前工作区文件创建 Unix 时间戳。
    let checksum = workspace_store_checksum(WORKSPACE_STORE_VERSION, created_at_unix, &payload)?; // 排除校验字段自身的规范化文件校验值。
    Ok(PersistedWorkspaceStore {
        store_version: WORKSPACE_STORE_VERSION,
        created_at_unix,
        checksum,
        payload,
    })
}

/// 使用固定 FNV-1a 64 位算法计算版本、时间和 UTF-8 JSON 载荷完整性值。
fn workspace_store_checksum(
    store_version: u32,
    created_at_unix: i64,
    payload: &PersistedWorkspacePayload,
) -> Result<String, WorkspaceError> {
    let bytes = serde_json::to_vec(&(store_version, created_at_unix, payload))
        .map_err(|error| WorkspaceError::restore_failed(&error))?; // serde 字段顺序稳定的规范化文件内容。
    let mut checksum = 0xcbf29ce484222325_u64; // FNV-1a 64 位偏移基数。
    for byte in bytes {
        checksum ^= u64::from(byte);
        checksum = checksum.wrapping_mul(0x100000001b3);
    }
    Ok(format!("fnv1a64:{checksum:016x}"))
}

/// 创建排除运行期选择与焦点的持久布局副本。
fn persisted_layout(snapshot: &WorkspaceSnapshot) -> WorkspaceSnapshot {
    let mut persisted = snapshot.clone(); // 写入磁盘前清理运行期状态的布局副本。
    persisted.interaction = WorkspaceInteractionSnapshot::default();
    persisted
}

/// 创建只包含可恢复布局字段的会话历史快照。
fn layout_snapshot(snapshot: &WorkspaceSnapshot) -> WorkspaceSnapshot {
    // 排除运行期选择与焦点的布局历史副本。
    let mut layout = snapshot.clone();
    layout.interaction = WorkspaceInteractionSnapshot::default();
    layout
}

/// 将历史布局应用到当前项目集合，并保留历史提交后出现的外部项目。
fn merge_layout_history_target(
    mut target: WorkspaceSnapshot,
    current: &WorkspaceSnapshot,
) -> (WorkspaceSnapshot, usize, usize) {
    let current_items = current
        .unclassified_items
        .iter()
        .map(|item| (item.stable_reference.as_str(), item))
        .collect::<std::collections::HashMap<_, _>>(); // 当前仍在工作区中的完整项目集合。
    let target_ids = target
        .unclassified_items
        .iter()
        .map(|item| item.stable_reference.clone())
        .collect::<std::collections::HashSet<_>>(); // 历史事务原本覆盖的项目引用。
    let restored_count = target_ids
        .iter()
        .filter(|item_id| current_items.contains_key(item_id.as_str()))
        .count(); // 当前仍存在并可以恢复历史布局的项目数量。
    let changed_count = target_ids.len().saturating_sub(restored_count); // 已离开当前工作区的历史项目数量。
    target
        .unclassified_items
        .retain(|item| current_items.contains_key(item.stable_reference.as_str()));
    for item in &mut target.unclassified_items {
        if let Some(current_item) = current_items.get(item.stable_reference.as_str()) {
            item.projection_monitor_id
                .clone_from(&current_item.projection_monitor_id);
            item.projection_position = current_item.projection_position;
        }
    }
    for fence in &mut target.fences {
        fence
            .members
            .retain(|item_id| current_items.contains_key(item_id.as_str()));
        if let Some(current_fence) = current.fences.iter().find(|current| current.id == fence.id) {
            fence
                .projection_monitor_id
                .clone_from(&current_fence.projection_monitor_id);
            fence.projection_bounds = current_fence.projection_bounds;
        }
    }
    target.unclassified_items.extend(
        current
            .unclassified_items
            .iter()
            .filter(|item| !target_ids.contains(&item.stable_reference))
            .cloned(),
    ); // 历史提交后由 Shell 新增的项目保持当前布局。
    target.grid = current.grid;
    target.monitor_grids.clone_from(&current.monitor_grids);
    (target, restored_count, changed_count)
}

/// 创建包含当前官方默认组件集合的空工作区布局。
fn default_workspace_snapshot(
    default_components: &[ComponentInstanceSnapshot],
) -> WorkspaceSnapshot {
    WorkspaceSnapshot {
        components: default_components.to_vec(),
        ..WorkspaceSnapshot::default()
    }
}

/// 创建首次启动的空工作区快照。
impl Default for WorkspaceSnapshot {
    fn default() -> Self {
        Self {
            schema_version: WORKSPACE_SCHEMA_VERSION,
            revision: 0,
            grid: WorkspaceGrid::default(),
            monitor_grids: Vec::new(),
            fences: Vec::new(),
            unclassified_items: Vec::new(),
            components: Vec::new(),
            interaction: WorkspaceInteractionSnapshot::default(),
        }
    }
}

/// 实现工作区快照内部的布局事务和完整性校验。
impl WorkspaceSnapshot {
    /// 使用 Shell 精确重命名事实同步布局、选择、锚点和焦点引用。
    fn remap_item_references(
        &mut self,
        renames: &[(String, String)],
    ) -> Result<(), WorkspaceError> {
        for (old_reference, new_reference) in renames {
            validate_id(old_reference, "重命名前项目标识无效")?;
            validate_id(new_reference, "重命名后项目标识无效")?;
            if old_reference == new_reference {
                continue;
            }
            // 当前布局中等待重命名的来源项目索引。
            let Some(item_index) = self
                .unclassified_items
                .iter()
                .position(|item| item.stable_reference == *old_reference)
            else {
                continue;
            };
            if self
                .unclassified_items
                .iter()
                .any(|candidate| candidate.stable_reference == *new_reference)
            {
                return Err(WorkspaceError::invalid_command("重命名后项目标识已经存在"));
            }
            self.unclassified_items[item_index]
                .stable_reference
                .clone_from(new_reference);
            for fence in &mut self.fences {
                for member in &mut fence.members {
                    if member == old_reference {
                        member.clone_from(new_reference);
                    }
                }
            }
            for selected in &mut self.interaction.selected_item_ids {
                if selected == old_reference {
                    selected.clone_from(new_reference);
                }
            }
            if self.interaction.selection_anchor_id.as_ref() == Some(old_reference) {
                self.interaction.selection_anchor_id = Some(new_reference.clone());
            }
            if matches!(
                &self.interaction.keyboard_focus,
                WorkspaceFocusTarget::Item { item_id } if item_id == old_reference
            ) {
                self.interaction.keyboard_focus = WorkspaceFocusTarget::Item {
                    item_id: new_reference.clone(),
                };
            }
        }
        Ok(())
    }

    /// 应用一个已反序列化的运行期交互命令。
    fn apply_interaction(
        &mut self,
        command: WorkspaceInteractionCommand,
        monitor_id: &str,
    ) -> Result<&'static str, WorkspaceError> {
        match command {
            WorkspaceInteractionCommand::FocusItem {
                item_id,
                selection_mode,
            } => {
                validate_id(&item_id, "桌面项目标识无效")?;
                if !self.unclassified_items.iter().any(|item| {
                    item.stable_reference == item_id
                        && self.item_visible_on_monitor(&item_id, monitor_id)
                }) {
                    return Err(WorkspaceError::invalid_command("桌面项目不存在"));
                }
                match selection_mode {
                    WorkspaceSelectionMode::Replace => {
                        self.interaction.selected_item_ids = vec![item_id.clone()];
                        self.interaction.selection_anchor_id = Some(item_id.clone());
                    }
                    WorkspaceSelectionMode::Toggle => {
                        if self.interaction.selected_item_ids.contains(&item_id) {
                            self.interaction
                                .selected_item_ids
                                .retain(|selected| selected != &item_id);
                        } else {
                            self.interaction.selected_item_ids.push(item_id.clone());
                        }
                        self.interaction.selection_anchor_id = Some(item_id.clone());
                    }
                    WorkspaceSelectionMode::Range => {
                        // 当前范围选择开始时的已有锚点。
                        let anchor_id = self.interaction.selection_anchor_id.clone();
                        // 锚点与目标位于同一焦点容器时的连续选择。
                        let range = anchor_id
                            .as_deref()
                            .filter(|anchor| self.same_item_container(anchor, &item_id))
                            .and_then(|anchor| {
                                // 当前目标焦点容器的完整项目顺序。
                                let order = self.item_order(&item_id);
                                // 当前锚点在逻辑顺序中的位置。
                                let anchor_index =
                                    order.iter().position(|candidate| candidate == anchor)?;
                                // 当前目标在逻辑顺序中的位置。
                                let target_index =
                                    order.iter().position(|candidate| candidate == &item_id)?;
                                // 包含锚点与目标的双向连续范围。
                                let start = anchor_index.min(target_index);
                                let end = anchor_index.max(target_index);
                                Some(order[start..=end].to_vec())
                            });
                        if let Some(range) = range {
                            self.interaction.selected_item_ids = range;
                        } else {
                            self.interaction.selected_item_ids = vec![item_id.clone()];
                            self.interaction.selection_anchor_id = Some(item_id.clone());
                        }
                    }
                    WorkspaceSelectionMode::Preserve => {
                        self.interaction.selection_anchor_id = Some(item_id.clone());
                    }
                }
                self.interaction.keyboard_focus = WorkspaceFocusTarget::Item { item_id };
                Ok("选择桌面项目")
            }
            WorkspaceInteractionCommand::SelectItems { item_ids } => {
                // 框选结果中已经出现的项目标识。
                let mut selected = std::collections::HashSet::new();
                // 按前端视觉顺序保留的有效框选结果。
                let mut ordered = Vec::with_capacity(item_ids.len());
                for item_id in item_ids {
                    validate_id(&item_id, "桌面项目标识无效")?;
                    if !self.unclassified_items.iter().any(|item| {
                        item.stable_reference == item_id
                            && self.item_visible_on_monitor(&item_id, monitor_id)
                    }) {
                        return Err(WorkspaceError::invalid_command("桌面项目不存在"));
                    }
                    if selected.insert(item_id.clone()) {
                        ordered.push(item_id);
                    }
                }
                self.interaction.selection_anchor_id = ordered.first().cloned();
                self.interaction.keyboard_focus = ordered
                    .first()
                    .cloned()
                    .map(|item_id| WorkspaceFocusTarget::Item { item_id })
                    .unwrap_or(WorkspaceFocusTarget::Background);
                self.interaction.selected_item_ids = ordered;
                Ok("框选桌面项目")
            }
            WorkspaceInteractionCommand::FocusSurface { focus } => {
                match &focus {
                    WorkspaceFocusTarget::Background => {}
                    WorkspaceFocusTarget::Item { .. } => {
                        return Err(WorkspaceError::invalid_command("项目焦点需要指定选择规则"));
                    }
                    WorkspaceFocusTarget::Fence { fence_id } => {
                        let fence = self.fence(fence_id)?;
                        if (fence
                            .projection_monitor_id
                            .as_deref()
                            .unwrap_or(&fence.monitor_id))
                            != monitor_id
                        {
                            return Err(WorkspaceError::invalid_command("围栏不属于当前显示器"));
                        }
                    }
                    WorkspaceFocusTarget::Component { component_id } => {
                        validate_id(component_id, "组件标识无效")?;
                        let component = self
                            .components
                            .iter()
                            .find(|component| component.instance_id == *component_id)
                            .ok_or_else(|| WorkspaceError::invalid_command("组件实例不存在"))?;
                        if component
                            .projection_monitor_id
                            .as_deref()
                            .unwrap_or(&component.monitor_id)
                            != monitor_id
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
            WorkspaceInteractionCommand::SelectFocusContainer => {
                // 当前容器全选前保持不变的键盘焦点。
                let focus = self.interaction.keyboard_focus.clone();
                // 当前焦点容器按视觉顺序提供的项目集合。
                let selected = match &focus {
                    WorkspaceFocusTarget::Background => {
                        self.unclassified_item_order(Some(monitor_id))
                    }
                    WorkspaceFocusTarget::Item { item_id } => self.item_order(item_id),
                    WorkspaceFocusTarget::Fence { fence_id } => {
                        self.fence(fence_id)?.members.clone()
                    }
                    WorkspaceFocusTarget::Component { .. } => Vec::new(),
                };
                self.interaction.selection_anchor_id = match focus {
                    WorkspaceFocusTarget::Item { item_id } => Some(item_id),
                    _ => selected.first().cloned(),
                };
                self.interaction.selected_item_ids = selected;
                Ok("全选焦点容器")
            }
        }
    }

    /// 判断两个桌面项目是否属于同一焦点容器。
    fn same_item_container(&self, first: &str, second: &str) -> bool {
        let first_fence = self
            .fences
            .iter()
            .find(|fence| fence.members.iter().any(|member| member == first))
            .map(|fence| fence.id.as_str()); // 首个项目所属围栏。
        let second_fence = self
            .fences
            .iter()
            .find(|fence| fence.members.iter().any(|member| member == second))
            .map(|fence| fence.id.as_str()); // 第二个项目所属围栏。
        match (first_fence, second_fence) {
            (Some(first), Some(second)) => first == second,
            (None, None) => {
                self.item_visible_monitor_id(first) == self.item_visible_monitor_id(second)
            }
            _ => false,
        }
    }

    /// 校验命令所属显示器，未指定时使用当前主显示器兼容网格。
    fn command_monitor_id(&self, requested: Option<&str>) -> Result<String, WorkspaceError> {
        let Some(monitor_id) = requested.or_else(|| {
            self.monitor_grids
                .first()
                .map(|entry| entry.monitor_id.as_str())
        }) else {
            return Ok(String::new());
        }; // 当前布局命令使用的持久显示器身份；旧空快照等待首次宿主协调补全。
        if !self
            .monitor_grids
            .iter()
            .any(|entry| entry.monitor_id == monitor_id)
        {
            return Err(WorkspaceError::invalid_command("布局命令显示器不存在"));
        }
        Ok(monitor_id.to_string())
    }

    /// 返回项目当前持久或临时投射后实际可见的显示器身份。
    fn item_visible_monitor_id(&self, item_id: &str) -> Option<&str> {
        if let Some(fence) = self
            .fences
            .iter()
            .find(|fence| fence.members.iter().any(|member| member == item_id))
        {
            return Some(
                fence
                    .projection_monitor_id
                    .as_deref()
                    .unwrap_or(&fence.monitor_id),
            );
        }
        self.unclassified_items
            .iter()
            .find(|item| item.stable_reference == item_id)
            .map(|item| {
                item.projection_monitor_id
                    .as_deref()
                    .unwrap_or(&item.monitor_id)
            })
    }

    /// 判断项目是否可以由指定逐屏页面提交交互。
    fn item_visible_on_monitor(&self, item_id: &str, monitor_id: &str) -> bool {
        self.item_visible_monitor_id(item_id) == Some(monitor_id)
    }

    /// 返回目标项目所在焦点容器的完整视觉逻辑顺序。
    fn item_order(&self, item_id: &str) -> Vec<String> {
        if let Some(fence) = self
            .fences
            .iter()
            .find(|fence| fence.members.iter().any(|member| member == item_id))
        {
            return fence.members.clone();
        }

        let monitor_id = self.item_visible_monitor_id(item_id); // 当前未归类项目可见的逐屏容器。
        self.unclassified_item_order(monitor_id)
    }

    /// 返回未归类区域的完整视觉逻辑顺序。
    fn unclassified_item_order(&self, monitor_id: Option<&str>) -> Vec<String> {
        // 当前全部围栏成员用于排除未归类项目。
        let member_ids = self
            .fences
            .iter()
            .flat_map(|fence| fence.members.iter())
            .collect::<std::collections::HashSet<_>>();
        // 未归类区域按从上到下、从左到右排列的项目。
        let mut items = self
            .unclassified_items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                !member_ids.contains(&item.stable_reference)
                    && monitor_id.is_none_or(|monitor_id| {
                        item.projection_monitor_id
                            .as_deref()
                            .unwrap_or(&item.monitor_id)
                            == monitor_id
                    })
            })
            .collect::<Vec<_>>();
        items.sort_by(|(first_index, first), (second_index, second)| {
            first
                .position
                .top
                .total_cmp(&second.position.top)
                .then_with(|| first.position.left.total_cmp(&second.position.left))
                .then_with(|| first_index.cmp(second_index))
        });
        items
            .into_iter()
            .map(|(_, item)| item.stable_reference.clone())
            .collect()
    }

    /// 协调 Shell 项目身份、持久显示器归属并为新项目分配首个空网格位置。
    fn reconcile_items_for_monitors(
        &mut self,
        item_ids: &[String],
        assignments: &[(String, String)],
        primary_monitor_id: &str,
    ) -> Result<(), WorkspaceError> {
        // 当前 Shell 快照中的稳定项目引用。
        let active_ids = item_ids
            .iter()
            .map(String::as_str)
            .collect::<std::collections::HashSet<_>>();
        // 当前全部围栏继续持有的成员引用。
        let member_ids = self
            .fences
            .iter()
            .flat_map(|fence| fence.members.iter().cloned())
            .collect::<std::collections::HashSet<_>>();
        let active_monitor_ids = self
            .monitor_grids
            .iter()
            .map(|entry| entry.monitor_id.as_str())
            .collect::<std::collections::HashSet<_>>(); // 当前可以直接呈现持久布局的显示器身份。
        let assignments = assignments
            .iter()
            .map(|(item_id, monitor_id)| (item_id.as_str(), monitor_id.as_str()))
            .collect::<std::collections::HashMap<_, _>>(); // Explorer 位置提供的项目初始显示器归属。
        for fence in &mut self.fences {
            if fence.monitor_id.is_empty() {
                fence.monitor_id = primary_monitor_id.to_string();
            }
        }
        for item in &mut self.unclassified_items {
            if item.monitor_id.is_empty() {
                item.monitor_id = assignments
                    .get(item.stable_reference.as_str())
                    .filter(|monitor_id| active_monitor_ids.contains(**monitor_id))
                    .copied()
                    .unwrap_or(primary_monitor_id)
                    .to_string();
            }
        }
        self.unclassified_items.retain(|item| {
            active_ids.contains(item.stable_reference.as_str())
                || member_ids.contains(item.stable_reference.as_str())
        });

        let mut occupied = self
            .monitor_grids
            .iter()
            .map(|entry| {
                (
                    entry.monitor_id.clone(),
                    self.unclassified_items
                        .iter()
                        .filter(|item| item.monitor_id == entry.monitor_id)
                        .map(|item| item.position)
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<std::collections::HashMap<_, _>>(); // 每台显示器已占用的持久网格位置。
        for item_id in item_ids {
            validate_id(item_id, "桌面项目标识无效")?;
            if self
                .unclassified_items
                .iter()
                .any(|item| item.stable_reference == *item_id)
            {
                continue;
            }

            let monitor_id = assignments
                .get(item_id.as_str())
                .filter(|monitor_id| active_monitor_ids.contains(**monitor_id))
                .copied()
                .unwrap_or(primary_monitor_id); // 新项目按 Explorer 位置或主屏回退获得的持久归属。
            let grid = self
                .monitor_grids
                .iter()
                .find(|entry| entry.monitor_id == monitor_id)
                .ok_or_else(|| WorkspaceError::invalid_command("项目显示器网格不存在"))?
                .grid; // 当前新项目所属显示器的网格。
            let capacity = grid_capacity(grid)?; // 当前显示器网格的安全容量。
            let monitor_occupied = occupied
                .get_mut(monitor_id)
                .expect("活动显示器已创建占用集合"); // 当前显示器已经占用的位置。
            // 当前新项目获得的首个空网格位置。
            let position = (0..capacity)
                .map(|index| grid_position(grid, index))
                .find(|position| !monitor_occupied.contains(position))
                .ok_or_else(|| WorkspaceError::invalid_command("未归类网格空间不足"))?;
            // 当前新项目获得的最高显示层级。
            let z_index = self.next_z_index();
            self.unclassified_items.push(UnclassifiedItemSnapshot {
                stable_reference: item_id.clone(),
                monitor_id: monitor_id.to_string(),
                projection_monitor_id: None,
                projection_position: None,
                position,
                z_index,
            });
            monitor_occupied.push(position);
        }
        self.normalize_interaction();
        Ok(())
    }

    /// 为断开的持久显示器建立主屏临时投射，重连后恢复原归属与位置。
    fn reconcile_monitor_projections(
        &mut self,
        monitors: &[DesktopHostMonitor],
        primary_monitor_id: &str,
    ) -> Result<(), WorkspaceError> {
        let active_monitor_ids = monitors
            .iter()
            .map(|monitor| monitor.id.as_str())
            .collect::<std::collections::HashSet<_>>(); // 当前仍连接的显示器身份。
        let primary_monitor = monitors
            .iter()
            .find(|monitor| monitor.id == primary_monitor_id)
            .ok_or_else(|| WorkspaceError::invalid_command("主显示器不存在"))?; // 临时投射目标显示器。
        let primary_grid = self
            .monitor_grids
            .iter()
            .find(|entry| entry.monitor_id == primary_monitor_id)
            .expect("主显示器网格已创建")
            .grid; // 临时投射项目使用的主屏网格。
        let capacity = grid_capacity(primary_grid)?; // 主屏临时投射可使用的网格容量。
        let mut occupied = self
            .unclassified_items
            .iter()
            .filter_map(|item| {
                if item.monitor_id == primary_monitor_id {
                    Some(item.position)
                } else if item.projection_monitor_id.as_deref() == Some(primary_monitor_id) {
                    item.projection_position
                } else {
                    None
                }
            })
            .collect::<Vec<_>>(); // 主屏已有持久项目与临时投射项目占用的位置。
        for item in &mut self.unclassified_items {
            if active_monitor_ids.contains(item.monitor_id.as_str()) {
                item.projection_monitor_id = None;
                item.projection_position = None;
                continue;
            }
            let position = (0..capacity)
                .map(|index| grid_position(primary_grid, index))
                .find(|position| !occupied.contains(position))
                .unwrap_or_else(|| grid_position(primary_grid, capacity.saturating_sub(1))); // 主屏满载时允许临时重叠但不改变持久位置。
            item.projection_monitor_id = Some(primary_monitor_id.to_string());
            item.projection_position = Some(position);
            occupied.push(position);
        }
        let viewport_width = f64::from(primary_monitor.work_width) / primary_monitor.scale_factor; // 主屏 CSS 工作区宽度。
        let viewport_height = f64::from(primary_monitor.work_height) / primary_monitor.scale_factor; // 主屏 CSS 工作区高度。
        for fence in &mut self.fences {
            if active_monitor_ids.contains(fence.monitor_id.as_str()) {
                fence.projection_monitor_id = None;
                fence.projection_bounds = None;
                continue;
            }
            fence.projection_monitor_id = Some(primary_monitor_id.to_string());
            fence.projection_bounds =
                Some(project_rect(fence.bounds, viewport_width, viewport_height));
        }
        for component in &mut self.components {
            let was_unassigned = component.monitor_id.is_empty(); // 官方默认实例首次进入逐屏布局的标记。
            if was_unassigned {
                component.monitor_id = primary_monitor_id.to_string();
                component.bounds.left = (viewport_width - component.bounds.width - 28.0).max(0.0);
                component.bounds.top = if component.component_id == "official.debug-test" {
                    16.0
                } else {
                    (viewport_height - component.bounds.height - 28.0).max(0.0)
                };
            }
            if active_monitor_ids.contains(component.monitor_id.as_str()) {
                component.projection_monitor_id = None;
                component.projection_bounds = None;
                continue;
            }
            component.projection_monitor_id = Some(primary_monitor_id.to_string());
            component.projection_bounds = Some(project_component_rect(
                component.bounds,
                viewport_width,
                viewport_height,
            ));
        }
        Ok(())
    }

    /// 应用一个已反序列化的工作区命令。
    fn apply(
        &mut self,
        command: WorkspaceCommand,
        monitor_id: Option<&str>,
    ) -> Result<&'static str, WorkspaceError> {
        match command {
            WorkspaceCommand::CreateFence { title, bounds } => {
                validate_title(&title)?;
                validate_rect(bounds)?;
                // 当前事务分配的稳定围栏标识。
                let id = format!("fence-{}", self.revision + 1);
                // 新围栏使用的最高层级。
                let z_index = self.next_z_index();
                let monitor_id = self.command_monitor_id(monitor_id)?; // 当前页面所属的持久显示器身份。
                self.fences.push(FenceSnapshot {
                    id,
                    title,
                    monitor_id,
                    projection_monitor_id: None,
                    projection_bounds: None,
                    bounds,
                    members: Vec::new(),
                    layout_mode: FenceLayoutMode::Tile,
                    sort_mode: FenceSortMode::None,
                    scroll_top: 0.0,
                    collapsed: false,
                    last_expanded_height: bounds.height,
                    z_index,
                });
                Ok("创建围栏")
            }
            WorkspaceCommand::UpdateFenceBounds { fence_id, bounds } => {
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
            WorkspaceCommand::SetFenceCollapsed {
                fence_id,
                collapsed,
            } => {
                // 折叠时需要协调选择与焦点的围栏成员。
                let members = self.fence(&fence_id)?.members.clone();
                self.fence_mut(&fence_id)?.collapsed = collapsed;
                if collapsed {
                    self.interaction
                        .selected_item_ids
                        .retain(|item_id| !members.contains(item_id));
                    if self
                        .interaction
                        .selection_anchor_id
                        .as_ref()
                        .is_some_and(|item_id| members.contains(item_id))
                    {
                        self.interaction.selection_anchor_id = None;
                    }
                    if matches!(
                        &self.interaction.keyboard_focus,
                        WorkspaceFocusTarget::Item { item_id } if members.contains(item_id)
                    ) {
                        self.interaction.keyboard_focus = WorkspaceFocusTarget::Fence {
                            fence_id: fence_id.clone(),
                        };
                    }
                }
                Ok("更新围栏折叠状态")
            }
            WorkspaceCommand::SetFenceLayout {
                fence_id,
                layout_mode,
            } => {
                // 等待布局方式提交的围栏。
                let fence = self.fence_mut(&fence_id)?;
                fence.layout_mode = layout_mode;
                Ok("更新围栏布局")
            }
            WorkspaceCommand::SetFenceSort {
                fence_id,
                sort_mode,
            } => {
                self.fence_mut(&fence_id)?.sort_mode = sort_mode;
                Ok("更新围栏排序")
            }
            WorkspaceCommand::SetFenceScroll {
                fence_id,
                scroll_top,
            } => {
                if !scroll_top.is_finite() || scroll_top < 0.0 {
                    return Err(WorkspaceError::invalid_command("围栏滚动位置无效"));
                }
                // 等待滚动位置提交的围栏。
                let fence = self.fence_mut(&fence_id)?;
                fence.scroll_top = scroll_top;
                Ok("更新围栏滚动位置")
            }
            WorkspaceCommand::MoveItemToFence {
                item_id,
                fence_id,
                before_item_id,
            } => {
                validate_id(&item_id, "桌面项目标识无效")?;
                let target_monitor_id = self.fence(&fence_id)?.monitor_id.clone(); // 目标围栏持久归属的显示器身份。
                if !self
                    .unclassified_items
                    .iter()
                    .any(|item| item.stable_reference == item_id)
                {
                    return Err(WorkspaceError::invalid_command(
                        "桌面项目缺少未归类来源位置",
                    ));
                }
                if before_item_id.as_deref() == Some(item_id.as_str()) {
                    return Ok("保持围栏成员顺序");
                }
                for fence in &mut self.fences {
                    fence.members.retain(|member| member != &item_id);
                }
                if let Some(item) = self
                    .unclassified_items
                    .iter_mut()
                    .find(|item| item.stable_reference == item_id)
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
                let insertion_index = before_item_id
                    .as_ref()
                    .filter(|_| fence.sort_mode == FenceSortMode::None)
                    .and_then(|target| fence.members.iter().position(|member| member == target))
                    .unwrap_or(fence.members.len());
                fence.members.insert(insertion_index, item_id);
                fence.z_index = next_z_index;
                Ok("更新围栏归属")
            }
            WorkspaceCommand::RemoveItemFromFence { item_id } => {
                validate_id(&item_id, "桌面项目标识无效")?;
                let target_monitor_id = self.command_monitor_id(monitor_id)?; // 项目移出围栏后的显示器归属。
                for fence in &mut self.fences {
                    fence.members.retain(|member| member != &item_id);
                }
                if let Some(item) = self
                    .unclassified_items
                    .iter_mut()
                    .find(|item| item.stable_reference == item_id)
                {
                    item.monitor_id = target_monitor_id;
                    item.projection_monitor_id = None;
                    item.projection_position = None;
                }
                Ok("移出围栏")
            }
            WorkspaceCommand::CommitItemDrag {
                anchor_item_id,
                item_ids,
                placements,
                target_fence_id,
                before_item_id,
                fence_scrolls,
            } => {
                validate_id(&anchor_item_id, "拖动锚点标识无效")?;
                if item_ids.is_empty() || !item_ids.contains(&anchor_item_id) {
                    return Err(WorkspaceError::invalid_command("拖动项目集合无效"));
                }
                // 当前拖动集合中已经出现的项目标识。
                let mut dragged_ids = std::collections::HashSet::new();
                for item_id in &item_ids {
                    validate_id(item_id, "拖动项目标识无效")?;
                    if !dragged_ids.insert(item_id.clone())
                        || !self
                            .unclassified_items
                            .iter()
                            .any(|item| item.stable_reference == *item_id)
                    {
                        return Err(WorkspaceError::invalid_command("拖动项目集合包含未知项目"));
                    }
                }

                // 拖动自动滚动提交中已经出现的围栏标识。
                let mut scrolled_fences = std::collections::HashSet::new();
                for WorkspaceFenceScroll {
                    fence_id,
                    scroll_top,
                } in fence_scrolls
                {
                    if !scroll_top.is_finite()
                        || scroll_top < 0.0
                        || !scrolled_fences.insert(fence_id.clone())
                    {
                        return Err(WorkspaceError::invalid_command("拖动围栏滚动位置无效"));
                    }
                    self.fence_mut(&fence_id)?.scroll_top = scroll_top;
                }

                for fence in &mut self.fences {
                    fence.members.retain(|member| !dragged_ids.contains(member));
                }
                if let Some(fence_id) = target_fence_id {
                    if !placements.is_empty() {
                        return Err(WorkspaceError::invalid_command("围栏投放包含未归类位置"));
                    }
                    let target_monitor_id = self.fence(&fence_id)?.monitor_id.clone(); // 目标围栏持久归属的显示器身份。
                    for item in self
                        .unclassified_items
                        .iter_mut()
                        .filter(|item| dragged_ids.contains(&item.stable_reference))
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
                    let insertion_index = match before_item_id
                        .filter(|_| fence.sort_mode == FenceSortMode::None)
                    {
                        Some(before_item_id) => fence
                            .members
                            .iter()
                            .position(|member| member == &before_item_id)
                            .ok_or_else(|| WorkspaceError::invalid_command("围栏插入目标不存在"))?,
                        None => fence.members.len(),
                    };
                    fence
                        .members
                        .splice(insertion_index..insertion_index, item_ids.iter().cloned());
                    fence.z_index = next_z_index;
                } else {
                    if before_item_id.is_some() {
                        return Err(WorkspaceError::invalid_command(
                            "未归类投放包含围栏插入目标",
                        ));
                    }
                    let target_monitor_id = self.command_monitor_id(monitor_id)?; // 未归类投放所属的目标显示器身份。
                    // 当前提交中已经出现的网格项目标识。
                    let mut placed_ids = std::collections::HashSet::new();
                    for WorkspaceItemPlacement { item_id, position } in placements {
                        validate_id(&item_id, "网格项目标识无效")?;
                        validate_point(position)?;
                        if !placed_ids.insert(item_id.clone()) {
                            return Err(WorkspaceError::invalid_command("网格位置包含重复项目"));
                        }
                        let item = self
                            .unclassified_items
                            .iter_mut()
                            .find(|item| item.stable_reference == item_id)
                            .ok_or_else(|| {
                                WorkspaceError::invalid_command("网格位置包含未知项目")
                            })?;
                        item.position = position;
                        item.monitor_id.clone_from(&target_monitor_id);
                        item.projection_monitor_id = None;
                        item.projection_position = None;
                    }
                    if item_ids.iter().any(|item_id| !placed_ids.contains(item_id)) {
                        return Err(WorkspaceError::invalid_command("拖动项目缺少最终网格位置"));
                    }
                    // 拖动项目按稳定视觉顺序获得连续最高层级。
                    let first_z_index = self.next_z_index();
                    for (offset, item_id) in item_ids.iter().enumerate() {
                        let item = self
                            .unclassified_items
                            .iter_mut()
                            .find(|item| item.stable_reference == *item_id)
                            .expect("拖动项目已完成存在性校验");
                        item.z_index = first_z_index.saturating_add(offset as u32);
                    }
                    self.repair_fence_member_source_collisions(&target_monitor_id, &dragged_ids)?;
                }

                self.interaction.selected_item_ids = item_ids;
                self.interaction.selection_anchor_id = Some(anchor_item_id.clone());
                self.interaction.keyboard_focus = WorkspaceFocusTarget::Item {
                    item_id: anchor_item_id,
                };
                Ok("提交项目拖动")
            }
            WorkspaceCommand::DissolveFence { fence_id } => {
                // 当前事务移除并释放成员的围栏。
                let fence_index = self
                    .fences
                    .iter()
                    .position(|fence| fence.id == fence_id)
                    .ok_or_else(|| WorkspaceError::invalid_command("围栏不存在"))?;
                // 保留当前视觉成员顺序和内容区锚点的待解散围栏。
                let fence = self.fences.remove(fence_index);
                let grid = self
                    .monitor_grids
                    .iter()
                    .find(|entry| entry.monitor_id == fence.monitor_id)
                    .map(|entry| entry.grid)
                    .unwrap_or(self.grid); // 围栏持久显示器对应的未归类网格。
                for item in self
                    .unclassified_items
                    .iter_mut()
                    .filter(|item| fence.members.contains(&item.stable_reference))
                {
                    item.monitor_id.clone_from(&fence.monitor_id);
                    item.projection_monitor_id = None;
                    item.projection_position = None;
                }
                if self.interaction.keyboard_focus
                    == (WorkspaceFocusTarget::Fence {
                        fence_id: fence.id.clone(),
                    })
                {
                    self.interaction.keyboard_focus = WorkspaceFocusTarget::Background;
                }
                // 围栏内容区左上对应的组锚点网格索引。
                let anchor_column = ((fence.bounds.left + 10.0 - grid.left) / grid.column_width)
                    .round()
                    .clamp(0.0, f64::from(grid.columns - 1))
                    as u32;
                // 围栏内容区左上对应的组锚点网格行。
                let anchor_row = ((fence.bounds.top + 54.0 - grid.top) / grid.row_height)
                    .round()
                    .clamp(0.0, f64::from(grid.rows - 1)) as u32;
                // 当前未归类网格总容量。
                let capacity = grid_capacity(grid)?;
                // 下一个成员开始避让的位置。
                let mut next_index = anchor_column * grid.rows + anchor_row;
                // 排除当前围栏成员来源格后的已占位置。
                let mut occupied = self
                    .unclassified_items
                    .iter()
                    .filter(|item| {
                        item.monitor_id == fence.monitor_id
                            && !fence.members.contains(&item.stable_reference)
                    })
                    .map(|item| item.position)
                    .collect::<Vec<_>>();

                for member in fence.members {
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
                    let source = self
                        .unclassified_items
                        .iter_mut()
                        .find(|item| item.stable_reference == member)
                        .ok_or_else(|| {
                            WorkspaceError::invalid_command("围栏成员来源位置状态无效")
                        })?;
                    source.position = position;
                    occupied.push(position);
                    next_index = (available_index + 1) % capacity;
                }
                Ok("解散围栏")
            }
            WorkspaceCommand::UpdateComponentBounds {
                instance_id,
                bounds,
            } => {
                validate_component_rect(bounds)?;
                let monitor_id = self.command_monitor_id(monitor_id)?; // 组件移动提交后的持久显示器身份。
                let component = self.component_mut(&instance_id)?; // 等待提交位置和尺寸的组件实例。
                component.monitor_id = monitor_id;
                component.projection_monitor_id = None;
                component.projection_bounds = None;
                component.bounds = bounds;
                Ok("更新组件边界")
            }
            WorkspaceCommand::UpdateComponentState {
                instance_id,
                state_version,
                state,
            } => {
                if state_version == 0 {
                    return Err(WorkspaceError::invalid_command("组件状态版本无效"));
                }
                let state_size = serde_json::to_vec(&state)
                    .map_err(|error| {
                        WorkspaceError::invalid_command(format!("组件状态无效: {error}"))
                    })?
                    .len(); // 组件持久状态的 UTF-8 JSON 字节数。
                if state_size > 64 * 1024 {
                    return Err(WorkspaceError::invalid_command("组件状态超过 64 KiB 限制"));
                }
                let component = self.component_mut(&instance_id)?; // 等待提交版本化状态的组件实例。
                component.state_version = state_version;
                component.state = state;
                Ok("更新组件状态")
            }
            WorkspaceCommand::UndoLayout | WorkspaceCommand::RedoLayout => {
                unreachable!("布局历史命令由 WorkspaceState 处理")
            }
        }
    }

    /// 清理布局变化后已经失效的选择、锚点和键盘焦点。
    fn normalize_interaction(&mut self) {
        // 当前布局仍保存的全部项目稳定引用。
        let item_ids = self
            .unclassified_items
            .iter()
            .map(|item| item.stable_reference.as_str())
            .collect::<std::collections::HashSet<_>>();
        // 当前布局仍保存的全部围栏稳定标识。
        let fence_ids = self
            .fences
            .iter()
            .map(|fence| fence.id.as_str())
            .collect::<std::collections::HashSet<_>>();
        let component_ids = self
            .components
            .iter()
            .map(|component| component.instance_id.as_str())
            .collect::<std::collections::HashSet<_>>(); // 当前布局仍保存的全部组件实例标识。
        self.interaction
            .selected_item_ids
            .retain(|item_id| item_ids.contains(item_id.as_str()));
        if self
            .interaction
            .selection_anchor_id
            .as_deref()
            .is_some_and(|item_id| !item_ids.contains(item_id))
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
        ) {
            self.interaction.keyboard_focus = WorkspaceFocusTarget::Background;
        }
    }

    /// 校验恢复或提交后的完整工作区快照。
    fn validate(&self) -> Result<(), WorkspaceError> {
        if self.schema_version != WORKSPACE_SCHEMA_VERSION {
            return Err(WorkspaceError::invalid_command("工作区 Schema 版本无效"));
        }
        // 当前选择集合中已经出现的项目标识。
        let mut selected_ids = std::collections::HashSet::new();
        for item_id in &self.interaction.selected_item_ids {
            validate_id(item_id, "选择项目标识无效")?;
            if !selected_ids.insert(item_id)
                || !self
                    .unclassified_items
                    .iter()
                    .any(|item| &item.stable_reference == item_id)
            {
                return Err(WorkspaceError::invalid_command("选择集合包含未知项目"));
            }
        }
        if let Some(anchor_id) = &self.interaction.selection_anchor_id {
            validate_id(anchor_id, "选择锚点标识无效")?;
            if !self
                .unclassified_items
                .iter()
                .any(|item| &item.stable_reference == anchor_id)
            {
                return Err(WorkspaceError::invalid_command("选择锚点包含未知项目"));
            }
        }
        grid_capacity(self.grid)?;
        let mut monitor_ids = std::collections::HashSet::new(); // 当前逐屏网格中已经出现的显示器身份。
        for entry in &self.monitor_grids {
            validate_id(&entry.monitor_id, "显示器标识无效")?;
            grid_capacity(entry.grid)?;
            if !monitor_ids.insert(entry.monitor_id.as_str()) {
                return Err(WorkspaceError::invalid_command("显示器网格重复"));
            }
        }
        // 当前快照中已经出现的围栏标识。
        let mut fence_ids = std::collections::HashSet::new();
        // 当前快照中已经归属围栏的桌面项目。
        let mut member_ids = std::collections::HashSet::new();
        for fence in &self.fences {
            validate_id(&fence.id, "围栏标识无效")?;
            validate_title(&fence.title)?;
            validate_rect(fence.bounds)?;
            if !fence.scroll_top.is_finite()
                || fence.scroll_top < 0.0
                || !fence.last_expanded_height.is_finite()
                || fence.last_expanded_height < FENCE_MIN_HEIGHT
                || !fence_ids.insert(&fence.id)
            {
                return Err(WorkspaceError::invalid_command("围栏持久化状态无效"));
            }
            for member in &fence.members {
                validate_id(member, "围栏成员标识无效")?;
                if !member_ids.insert(member) {
                    return Err(WorkspaceError::invalid_command("桌面项目存在重复围栏归属"));
                }
            }
        }
        for item in &self.unclassified_items {
            validate_id(&item.stable_reference, "桌面项目标识无效")?;
            validate_point(item.position)?;
        }
        // 当前快照中已经出现的来源位置项目标识。
        let source_ids: std::collections::HashSet<_> = self
            .unclassified_items
            .iter()
            .map(|item| &item.stable_reference)
            .collect();
        // 当前快照中已经占用的未归类来源位置。
        let source_positions = self
            .unclassified_items
            .iter()
            .map(|item| {
                (
                    item.monitor_id.as_str(),
                    item.position.left.to_bits(),
                    item.position.top.to_bits(),
                )
            })
            .collect::<std::collections::HashSet<_>>();
        if source_ids.len() != self.unclassified_items.len()
            || source_positions.len() != self.unclassified_items.len()
            || member_ids.iter().any(|member| !source_ids.contains(member))
        {
            return Err(WorkspaceError::invalid_command("围栏成员来源位置状态无效"));
        }
        let mut component_ids = std::collections::HashSet::new(); // 当前快照中已经出现的组件实例标识。
        for component in &self.components {
            validate_id(&component.instance_id, "组件实例标识无效")?;
            validate_id(&component.component_id, "组件清单标识无效")?;
            validate_component_rect(component.bounds)?;
            if !component_ids.insert(component.instance_id.as_str())
                || component.state_version == 0
                || serde_json::to_vec(&component.state)
                    .map_err(|error| {
                        WorkspaceError::invalid_command(format!("组件状态无效: {error}"))
                    })?
                    .len()
                    > 64 * 1024
            {
                return Err(WorkspaceError::invalid_command("组件持久化状态无效"));
            }
            if let Some(projection_monitor_id) = &component.projection_monitor_id {
                validate_id(projection_monitor_id, "组件投射显示器标识无效")?;
                validate_component_rect(
                    component
                        .projection_bounds
                        .ok_or_else(|| WorkspaceError::invalid_command("组件投射边界缺失"))?,
                )?;
            } else if component.projection_bounds.is_some() {
                return Err(WorkspaceError::invalid_command("组件投射状态无效"));
            }
        }
        match &self.interaction.keyboard_focus {
            WorkspaceFocusTarget::Background => {}
            WorkspaceFocusTarget::Item { item_id } => {
                if !source_ids.contains(item_id) {
                    return Err(WorkspaceError::invalid_command("项目焦点包含未知项目"));
                }
            }
            WorkspaceFocusTarget::Fence { fence_id } => {
                if !fence_ids.contains(fence_id) {
                    return Err(WorkspaceError::invalid_command("围栏焦点包含未知围栏"));
                }
            }
            WorkspaceFocusTarget::Component { component_id } => {
                validate_id(component_id, "组件焦点标识无效")?;
                if !component_ids.contains(component_id.as_str()) {
                    return Err(WorkspaceError::invalid_command("组件焦点包含未知实例"));
                }
            }
        }
        Ok(())
    }

    /// 返回指定围栏的只读引用。
    fn fence(&self, fence_id: &str) -> Result<&FenceSnapshot, WorkspaceError> {
        validate_id(fence_id, "围栏标识无效")?;
        self.fences
            .iter()
            .find(|fence| fence.id == fence_id)
            .ok_or_else(|| WorkspaceError::invalid_command("围栏不存在"))
    }

    /// 返回指定围栏的可写引用。
    fn fence_mut(&mut self, fence_id: &str) -> Result<&mut FenceSnapshot, WorkspaceError> {
        validate_id(fence_id, "围栏标识无效")?;
        self.fences
            .iter_mut()
            .find(|fence| fence.id == fence_id)
            .ok_or_else(|| WorkspaceError::invalid_command("围栏不存在"))
    }

    /// 返回指定组件实例的可写引用。
    fn component_mut(
        &mut self,
        instance_id: &str,
    ) -> Result<&mut ComponentInstanceSnapshot, WorkspaceError> {
        validate_id(instance_id, "组件实例标识无效")?;
        self.components
            .iter_mut()
            .find(|component| component.instance_id == instance_id)
            .ok_or_else(|| WorkspaceError::invalid_command("组件实例不存在"))
    }

    /// 为围栏成员重新分配被新网格位置占用的来源槽位。
    fn repair_fence_member_source_collisions(
        &mut self,
        monitor_id: &str,
        dragged_ids: &std::collections::HashSet<String>,
    ) -> Result<(), WorkspaceError> {
        let Some(grid) = self
            .monitor_grids
            .iter()
            .find(|entry| entry.monitor_id == monitor_id)
            .map(|entry| entry.grid)
        else {
            return Ok(());
        };
        let capacity = grid_capacity(grid)?;
        let fence_members = self
            .fences
            .iter()
            .flat_map(|fence| fence.members.iter())
            .collect::<std::collections::HashSet<_>>();
        let mut occupied = std::collections::HashSet::new();
        for item in &self.unclassified_items {
            if item.monitor_id == monitor_id
                && (dragged_ids.contains(&item.stable_reference)
                    || !fence_members.contains(&item.stable_reference))
            {
                occupied.insert((item.position.left.to_bits(), item.position.top.to_bits()));
            }
        }
        for item in &mut self.unclassified_items {
            if item.monitor_id != monitor_id
                || dragged_ids.contains(&item.stable_reference)
                || !fence_members.contains(&item.stable_reference)
            {
                continue;
            }
            let key = (item.position.left.to_bits(), item.position.top.to_bits());
            if occupied.insert(key) {
                continue;
            }
            let Some(position) =
                (0..capacity)
                    .map(|index| grid_position(grid, index))
                    .find(|candidate| {
                        occupied.insert((candidate.left.to_bits(), candidate.top.to_bits()))
                    })
            else {
                return Err(WorkspaceError::invalid_command("未归类网格空间不足"));
            };
            item.position = position;
        }
        Ok(())
    }

    /// 分配新的围栏最高层级。
    fn next_z_index(&self) -> u32 {
        self.fences
            .iter()
            .map(|fence| fence.z_index)
            .chain(self.unclassified_items.iter().map(|item| item.z_index))
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }
}

/// 校验用户可见围栏标题。
fn validate_title(title: &str) -> Result<(), WorkspaceError> {
    if title.trim().is_empty() {
        return Err(WorkspaceError::invalid_command("围栏标题无效"));
    }
    Ok(())
}

/// 校验工作区对象标识。
fn validate_id(id: &str, message: &str) -> Result<(), WorkspaceError> {
    if id.trim().is_empty() {
        return Err(WorkspaceError::invalid_command(message));
    }
    Ok(())
}

/// 校验未归类项目位置。
fn validate_point(point: WorkspacePoint) -> Result<(), WorkspaceError> {
    if !point.left.is_finite() || !point.top.is_finite() {
        return Err(WorkspaceError::invalid_command("桌面项目来源位置无效"));
    }
    Ok(())
}

/// 从正式宿主工作区尺寸生成版本化未归类网格配置。
fn workspace_grid(
    workspace_width: f64,
    workspace_height: f64,
    preferences: GridPreferences,
) -> Result<WorkspaceGrid, WorkspaceError> {
    if !workspace_width.is_finite()
        || !workspace_height.is_finite()
        || workspace_width <= 0.0
        || workspace_height <= 0.0
    {
        return Err(WorkspaceError::invalid_command("工作区尺寸无效"));
    }
    let column_width = f64::from(preferences.width + preferences.column_gap); // 格子宽度加横向空白。
    let row_height = f64::from(preferences.height + preferences.row_gap); // 格子高度加纵向空白。
    Ok(WorkspaceGrid {
        left: WORKSPACE_GRID_LEFT,
        top: WORKSPACE_GRID_TOP,
        column_width,
        row_height,
        rows: ((workspace_height - WORKSPACE_GRID_BOTTOM_MARGIN) / row_height)
            .floor()
            .max(1.0) as u32,
        columns: ((workspace_width - WORKSPACE_GRID_LEFT) / column_width)
            .floor()
            .max(1.0) as u32,
    })
}

/// 校验网格配置并返回受控容量。
fn grid_capacity(grid: WorkspaceGrid) -> Result<u32, WorkspaceError> {
    if !grid.left.is_finite()
        || !grid.top.is_finite()
        || !grid.column_width.is_finite()
        || grid.column_width <= 0.0
        || !grid.row_height.is_finite()
        || grid.row_height <= 0.0
        || grid.rows == 0
        || grid.columns == 0
    {
        return Err(WorkspaceError::invalid_command("未归类网格配置无效"));
    }
    grid.rows
        .checked_mul(grid.columns)
        .filter(|capacity| *capacity <= 1_000_000)
        .ok_or_else(|| WorkspaceError::invalid_command("未归类网格容量无效"))
}

/// 将列优先线性索引转换为未归类网格位置。
fn grid_position(grid: WorkspaceGrid, index: u32) -> WorkspacePoint {
    WorkspacePoint {
        left: grid.left + f64::from(index / grid.rows) * grid.column_width,
        top: grid.top + f64::from(index % grid.rows) * grid.row_height,
    }
}

/// 校验围栏边界与原型一致的最小尺寸。
fn validate_rect(bounds: WorkspaceRect) -> Result<(), WorkspaceError> {
    if !bounds.left.is_finite()
        || !bounds.top.is_finite()
        || !bounds.width.is_finite()
        || !bounds.height.is_finite()
        || bounds.width < FENCE_MIN_WIDTH
        || bounds.height < FENCE_MIN_HEIGHT
    {
        return Err(WorkspaceError::invalid_command("围栏边界无效"));
    }
    Ok(())
}

/// 校验组件表面的自由位置和最小可交互尺寸。
fn validate_component_rect(bounds: WorkspaceRect) -> Result<(), WorkspaceError> {
    if !bounds.left.is_finite()
        || !bounds.top.is_finite()
        || !bounds.width.is_finite()
        || !bounds.height.is_finite()
        || bounds.width < 80.0
        || bounds.height < 48.0
    {
        return Err(WorkspaceError::invalid_command("组件边界无效"));
    }
    Ok(())
}

/// 将断开显示器上的围栏临时投射到目标工作区可见范围内。
fn project_rect(bounds: WorkspaceRect, viewport_width: f64, viewport_height: f64) -> WorkspaceRect {
    let width = bounds
        .width
        .min(viewport_width)
        .max(FENCE_MIN_WIDTH.min(viewport_width)); // 临时投射后的可见宽度。
    let height = bounds
        .height
        .min(viewport_height)
        .max(FENCE_MIN_HEIGHT.min(viewport_height)); // 临时投射后的可见高度。
    WorkspaceRect {
        left: bounds.left.clamp(0.0, (viewport_width - width).max(0.0)),
        top: bounds.top.clamp(0.0, (viewport_height - height).max(0.0)),
        width,
        height,
    }
}

/// 将断开显示器上的组件投射到主屏可见范围内。
fn project_component_rect(
    bounds: WorkspaceRect,
    viewport_width: f64,
    viewport_height: f64,
) -> WorkspaceRect {
    let width = bounds
        .width
        .min(viewport_width)
        .max(80.0_f64.min(viewport_width)); // 临时投射后的组件可见宽度。
    let height = bounds
        .height
        .min(viewport_height)
        .max(48.0_f64.min(viewport_height)); // 临时投射后的组件可见高度。
    WorkspaceRect {
        left: bounds.left.clamp(0.0, (viewport_width - width).max(0.0)),
        top: bounds.top.clamp(0.0, (viewport_height - height).max(0.0)),
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AUTO_SNAPSHOT_LIMIT, LAYOUT_ARCHIVE_RETENTION_SECONDS, PersistedVirtualDesktopLayout,
        PersistedWorkspacePayload, WorkspaceLayoutArchive, WorkspaceState, decode_workspace_store,
        merge_layout_history_target, purge_expired_archives, workspace_store_from_payload,
        write_auto_snapshot,
    };
    use crate::contracts::{
        ComponentInstanceSnapshot, DesktopHostMonitor, UnclassifiedItemSnapshot, WorkspaceCommand,
        WorkspaceFocusTarget, WorkspaceInteractionCommand, WorkspaceItemPlacement, WorkspacePoint,
        WorkspaceRect, WorkspaceSelectionMode, WorkspaceSnapshot,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    /// 创建当前测试独占的工作区快照路径。
    fn snapshot_path(name: &str) -> std::path::PathBuf {
        // 避免并发测试共享文件的纳秒时间标识。
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("系统时间应晚于 Unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "desktop-tools-{name}-{}-{unique}.json",
            std::process::id()
        ))
    }

    /// 创建当前测试使用的工作区状态。
    fn workspace_state(path: std::path::PathBuf) -> WorkspaceState {
        WorkspaceState::load(path).expect("测试工作区应可启动")
    }

    /// 独立网格设置覆盖运行、重启及容量不足不提交的完整路径。
    #[test]
    fn grid_preferences_reflow_and_reject_insufficient_capacity() {
        let path = snapshot_path("grid-preferences"); // 测试独占布局文件。
        let settings_path = snapshot_path("grid-settings"); // 测试独占偏好文件。
        let state = workspace_state(path.clone()); // 原子持久化工作区。
        let settings = crate::app::settings::SettingsState::load(settings_path.clone()).unwrap(); // 首次启动配置。
        let monitors = [monitor("DISPLAY1", true, 0, 1.5)]; // 混合 DPI 计算中的 150% 显示器。
        let items = (0..30)
            .map(|index| format!("item-{index}"))
            .collect::<Vec<_>>(); // 新大网格不能全部容纳的项目数。
        state
            .reconcile_host_monitors(&monitors, &items, &[], &[])
            .unwrap();
        let mut preferences = settings.preferences().unwrap(); // 等待修改的配置。
        preferences.desktop_grid.width = 100;
        state
            .save_preferences(&settings, preferences.clone(), &monitors)
            .unwrap();
        let snapshot = state.snapshot().unwrap(); // 已应用的权威布局。
        assert_eq!(snapshot.grid.column_width, 110.0);
        assert_eq!(snapshot.grid.row_height, 98.0);
        assert_eq!(snapshot.unclassified_items[0].position.left, 0.0);
        assert_eq!(
            settings.preferences().unwrap().fence_grid,
            crate::contracts::settings::ApplicationPreferences::default().fence_grid
        );
        preferences.desktop_grid = crate::contracts::settings::GridPreferences {
            width: 160,
            height: 180,
            column_gap: 48,
            row_gap: 48,
        };
        assert!(
            state
                .save_preferences(&settings, preferences, &monitors)
                .is_err()
        );
        assert_eq!(state.snapshot().unwrap(), snapshot);
        assert_eq!(settings.preferences().unwrap().desktop_grid.width, 100);
        let blocked_path = snapshot_path("blocked-grid-settings"); // 写入时才变成目录的配置目标。
        let blocked_settings =
            crate::app::settings::SettingsState::load(blocked_path.clone()).unwrap(); // 可读取但随后无法替换目标的配置状态。
        std::fs::create_dir(&blocked_path).unwrap();
        let mut rejected = settings.preferences().unwrap(); // 会触发重排但最终保存失败的配置。
        rejected.desktop_grid.width = 110;
        assert!(
            state
                .save_preferences(&blocked_settings, rejected, &monitors)
                .is_err()
        );
        assert_eq!(state.snapshot().unwrap(), snapshot);
        std::fs::remove_dir(blocked_path).unwrap();
        drop(state);
        let restored = workspace_state(path.clone()); // 重启恢复旧格位与新步进。
        restored
            .initialize_grid_preferences(settings.preferences().unwrap().desktop_grid)
            .unwrap();
        let next = restored
            .reconcile_host_monitors(&monitors, &items, &[], &[])
            .unwrap(); // 环境变化不回退默认网格。
        assert_eq!(next.grid.column_width, 110.0);
        assert_eq!(next.unclassified_items, snapshot.unclassified_items);
        drop(restored);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(settings_path).unwrap();
    }

    /// 创建逐屏工作区测试使用的显示器物理状态。
    fn monitor(id: &str, primary: bool, x: i32, scale_factor: f64) -> DesktopHostMonitor {
        DesktopHostMonitor {
            id: id.to_string(),
            name: id.to_string(),
            primary,
            x,
            y: 0,
            width: 1920,
            height: 1080,
            work_x: x,
            work_y: 0,
            work_width: 1920,
            work_height: 1040,
            scale_factor,
            dpi: (96.0 * scale_factor).round() as u32,
        }
    }

    /// 验证官方组件边界、状态和显示器归属可恢复，并在原显示器缺失时仅临时投射。
    #[test]
    fn component_layout_persists_and_projects_without_losing_ownership() {
        let path = snapshot_path("component-layout"); // 当前测试使用的持久工作区文件。
        let snapshot_directory = path.with_extension("snapshots"); // 当前测试可能生成的自动快照目录。
        let default_component = ComponentInstanceSnapshot {
            instance_id: "official.local-clock.default".to_string(),
            component_id: "official.local-clock".to_string(),
            monitor_id: String::new(),
            projection_monitor_id: None,
            projection_bounds: None,
            bounds: WorkspaceRect {
                left: 0.0,
                top: 0.0,
                width: 220.0,
                height: 116.0,
            },
            state_version: 1,
            state: serde_json::json!({"format": "24-hour"}),
        }; // 组件运行时注册的官方默认实例。
        let desktops = ["desktop-1".to_string()]; // 当前测试唯一的虚拟桌面身份。
        let state = WorkspaceState::load_for_virtual_desktops_with_components(
            path.clone(),
            "desktop-1",
            &desktops,
            std::slice::from_ref(&default_component),
        )
        .expect("包含官方组件的工作区应可启动");
        state
            .reconcile_host_monitors(
                &[
                    monitor("DISPLAY1", true, 0, 1.0),
                    monitor("DISPLAY2", false, 1920, 1.25),
                ],
                &[],
                &[],
                &[],
            )
            .expect("官方组件应可进入主显示器默认布局");
        state
            .apply_on_monitor(
                Some("DISPLAY2"),
                WorkspaceCommand::UpdateComponentBounds {
                    instance_id: default_component.instance_id.clone(),
                    bounds: WorkspaceRect {
                        left: 140.0,
                        top: 96.0,
                        width: 240.0,
                        height: 128.0,
                    },
                },
            )
            .expect("组件应可迁移到第二显示器");
        state
            .apply_on_monitor(
                Some("DISPLAY2"),
                WorkspaceCommand::UpdateComponentState {
                    instance_id: default_component.instance_id.clone(),
                    state_version: 1,
                    state: serde_json::json!({"format": "12-hour"}),
                },
            )
            .expect("组件版本化状态应可保存");
        drop(state);

        let restored = WorkspaceState::load_for_virtual_desktops_with_components(
            path.clone(),
            "desktop-1",
            &desktops,
            std::slice::from_ref(&default_component),
        )
        .expect("组件布局应可从工作区文件恢复");
        let restored_snapshot = restored.snapshot().expect("恢复后的组件快照应可读取");
        assert_eq!(restored_snapshot.components[0].monitor_id, "DISPLAY2");
        assert_eq!(restored_snapshot.components[0].bounds.width, 240.0);
        assert_eq!(restored_snapshot.components[0].state["format"], "12-hour");

        let projected = restored
            .reconcile_host_monitors(&[monitor("DISPLAY1", true, 0, 1.0)], &[], &[], &[])
            .expect("缺失显示器上的组件应可临时投射");
        assert_eq!(projected.components[0].monitor_id, "DISPLAY2");
        assert_eq!(
            projected.components[0].projection_monitor_id.as_deref(),
            Some("DISPLAY1")
        );
        assert!(projected.components[0].projection_bounds.is_some());
        drop(restored);

        std::fs::remove_file(path).expect("组件测试快照应可清理");
        if snapshot_directory.exists() {
            std::fs::remove_dir_all(snapshot_directory).expect("组件测试自动快照目录应可清理");
        }
    }

    /// 验证排序事务覆盖归属、拖动、重命名、历史和持久化边界。
    #[test]
    fn fence_sorting_is_persisted_and_applies_to_every_membership_path() {
        use crate::contracts::{DesktopItem, DesktopItemAvailability, FenceSortMode};
        let path = snapshot_path("fence-sorting"); // 本测试独占的持久化文件。
        let state = workspace_state(path.clone()); // 使用真实布局事务和原子持久化的工作区。
        let mut items = [
            ("a", "Zulu"),
            ("b", "alpha"),
            ("c", "Bravo"),
            ("d", "ALPHA"),
        ]
        .map(|(id, name)| DesktopItem {
            stable_reference: id.to_string(),
            display_name: name.to_string(),
            system_image: None,
            availability: DesktopItemAvailability::Available,
        }); // 显示名称与引用顺序故意不同，并包含忽略大小写的同名项目。
        let ids = items
            .iter()
            .map(|item| item.stable_reference.clone())
            .collect::<Vec<_>>(); // 当前完整 Shell 引用集合。
        state.update_item_names(&items).unwrap();
        state.reconcile_items(1280.0, 720.0, &ids).unwrap();
        let fence_id = state
            .apply(WorkspaceCommand::CreateFence {
                title: "排序验收".to_string(),
                bounds: WorkspaceRect {
                    left: 100.0,
                    top: 80.0,
                    width: 320.0,
                    height: 240.0,
                },
            })
            .unwrap()
            .fences[0]
            .id
            .clone(); // 本测试目标围栏。
        for id in ["a", "b"] {
            state
                .apply(WorkspaceCommand::MoveItemToFence {
                    item_id: id.to_string(),
                    fence_id: fence_id.clone(),
                    before_item_id: None,
                })
                .unwrap();
        }
        let ascending = state
            .apply(WorkspaceCommand::SetFenceSort {
                fence_id: fence_id.clone(),
                sort_mode: FenceSortMode::NameAscending,
            })
            .unwrap(); // 首次名称排序结果。
        assert_eq!(ascending.fences[0].members, ["b", "a"]);
        let grouped = state
            .apply(WorkspaceCommand::CommitItemDrag {
                anchor_item_id: "c".to_string(),
                item_ids: vec!["c".to_string(), "d".to_string()],
                placements: vec![],
                target_fence_id: Some(fence_id.clone()),
                before_item_id: Some("stale-target".to_string()),
                fence_scrolls: vec![],
            })
            .unwrap(); // 排序模式忽略手动插入位置，包括旧前端快照中的失效目标。
        assert_eq!(grouped.fences[0].members, ["b", "d", "c", "a"]);
        assert_eq!(
            state.apply(WorkspaceCommand::UndoLayout).unwrap().fences[0].members,
            ["b", "a"]
        );
        assert_eq!(
            state.apply(WorkspaceCommand::RedoLayout).unwrap().fences[0].members,
            ["b", "d", "c", "a"]
        );
        let descending = state
            .apply(WorkspaceCommand::SetFenceSort {
                fence_id: fence_id.clone(),
                sort_mode: FenceSortMode::NameDescending,
            })
            .unwrap(); // 再次名称排序结果。
        assert_eq!(descending.fences[0].members, ["a", "c", "b", "d"]);
        let disabled = state
            .apply(WorkspaceCommand::SetFenceSort {
                fence_id: fence_id.clone(),
                sort_mode: FenceSortMode::None,
            })
            .unwrap(); // 关闭自动排序保留已有排列。
        assert_eq!(disabled.fences[0].members, descending.fences[0].members);
        let manual = state
            .apply(WorkspaceCommand::MoveItemToFence {
                item_id: "d".to_string(),
                fence_id: fence_id.clone(),
                before_item_id: Some("a".to_string()),
            })
            .unwrap(); // 手动插入继续生效。
        assert_eq!(manual.fences[0].members, ["d", "a", "c", "b"]);
        state
            .apply(WorkspaceCommand::SetFenceSort {
                fence_id: fence_id.clone(),
                sort_mode: FenceSortMode::NameAscending,
            })
            .unwrap();
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

    /// 验证默认排序字段省略后旧备份仍然通过规范化校验。
    #[test]
    fn legacy_fence_sorting_keeps_backup_checksum_compatible() {
        let path = snapshot_path("legacy-fence-sort"); // 本测试独占的旧格式备份。
        let state = workspace_state(path.clone()); // 创建默认无排序围栏的工作区。
        state
            .apply(WorkspaceCommand::CreateFence {
                title: "旧围栏".to_string(),
                bounds: WorkspaceRect {
                    left: 0.0,
                    top: 0.0,
                    width: 320.0,
                    height: 240.0,
                },
            })
            .unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap(); // 当前规范化备份载荷。
        assert!(!serde_json::to_string(&value).unwrap().contains("sortMode"));
        let store = decode_workspace_store(value, "desktop-a").unwrap(); // 模拟缺少新增字段的旧备份校验入口。
        assert_eq!(
            store.payload.layouts[0].layout.fences[0].sort_mode,
            crate::contracts::FenceSortMode::None
        );
        drop(state);
        std::fs::remove_file(path).unwrap();
    }

    /// 验证创建围栏命令持久化完整初始状态并可在重启后恢复。
    #[test]
    fn create_fence_command_persists_and_restores_the_snapshot() {
        // 当前测试使用的持久化文件。
        let path = snapshot_path("create-fence");
        // 模拟首次应用启动的工作区状态。
        let state = workspace_state(path.clone());
        // 创建围栏命令返回的权威快照。
        let snapshot = state
            .apply(WorkspaceCommand::CreateFence {
                title: "项目资料".to_string(),
                bounds: WorkspaceRect {
                    left: 120.0,
                    top: 80.0,
                    width: 360.0,
                    height: 280.0,
                },
            })
            .expect("有效围栏应可创建");

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
    fn item_selection_is_runtime_state() {
        // 当前测试使用的持久化文件。
        let path = snapshot_path("item-selection");
        // 已协调一个真实桌面项目身份的工作区状态。
        let state = workspace_state(path.clone());
        state
            .reconcile_items(1280.0, 720.0, &["::{project}".to_string()])
            .expect("Shell 项目应可协调到工作区");

        // 单击项目后发布的权威交互快照。
        let snapshot = state
            .apply_interaction(WorkspaceInteractionCommand::FocusItem {
                item_id: "::{project}".to_string(),
                selection_mode: WorkspaceSelectionMode::Replace,
            })
            .expect("项目应可成为唯一选择和键盘焦点");
        assert_eq!(snapshot.interaction.revision, 1);
        assert_eq!(snapshot.interaction.selected_item_ids, ["::{project}"]);
        assert_eq!(
            snapshot.interaction.selection_anchor_id.as_deref(),
            Some("::{project}")
        );

        drop(state);
        // 重启后只恢复布局，选择与焦点使用新的运行期初始状态。
        let restored = WorkspaceState::load(path.clone()).expect("工作区布局应可恢复");
        assert_eq!(
            restored
                .snapshot()
                .expect("恢复快照应可读取")
                .interaction
                .revision,
            0
        );
        assert!(
            restored
                .snapshot()
                .expect("恢复快照应可读取")
                .interaction
                .selected_item_ids
                .is_empty()
        );

        std::fs::remove_file(path).expect("测试快照应可清理");
    }

    /// 验证切换、范围和保留选择模式遵循焦点容器逻辑顺序。
    #[test]
    fn selection_modes_follow_the_focus_container() {
        // 当前测试使用的持久化文件。
        let path = snapshot_path("selection-modes");
        // 已协调四个桌面项目的工作区状态。
        let state = workspace_state(path.clone());
        state
            .reconcile_items(
                1280.0,
                720.0,
                &[
                    "a".to_string(),
                    "b".to_string(),
                    "c".to_string(),
                    "d".to_string(),
                ],
            )
            .expect("Shell 项目应可协调到工作区");
        // 当前测试用于范围选择的围栏标识。
        let fence_id = state
            .apply(WorkspaceCommand::CreateFence {
                title: "范围".to_string(),
                bounds: WorkspaceRect {
                    left: 100.0,
                    top: 80.0,
                    width: 320.0,
                    height: 240.0,
                },
            })
            .expect("围栏应可创建")
            .fences[0]
            .id
            .clone();
        for item_id in ["b", "c", "d"] {
            state
                .apply(WorkspaceCommand::MoveItemToFence {
                    item_id: item_id.to_string(),
                    fence_id: fence_id.clone(),
                    before_item_id: None,
                })
                .expect("项目应可加入范围选择围栏");
        }

        state
            .apply_interaction(WorkspaceInteractionCommand::FocusItem {
                item_id: "b".to_string(),
                selection_mode: WorkspaceSelectionMode::Replace,
            })
            .expect("范围锚点应可建立");
        // 同一围栏中的 Shift 范围选择结果。
        let range = state
            .apply_interaction(WorkspaceInteractionCommand::FocusItem {
                item_id: "d".to_string(),
                selection_mode: WorkspaceSelectionMode::Range,
            })
            .expect("同一焦点容器应可执行范围选择");
        assert_eq!(range.interaction.selected_item_ids, ["b", "c", "d"]);
        assert_eq!(range.interaction.selection_anchor_id.as_deref(), Some("b"));

        // Ctrl 切换目标后的选择集合。
        let toggled = state
            .apply_interaction(WorkspaceInteractionCommand::FocusItem {
                item_id: "c".to_string(),
                selection_mode: WorkspaceSelectionMode::Toggle,
            })
            .expect("焦点项目应可切换选择状态");
        assert_eq!(toggled.interaction.selected_item_ids, ["b", "d"]);
        assert_eq!(
            toggled.interaction.selection_anchor_id.as_deref(),
            Some("c")
        );

        // Ctrl+方向键只移动焦点并保留现有选择。
        let preserved = state
            .apply_interaction(WorkspaceInteractionCommand::FocusItem {
                item_id: "d".to_string(),
                selection_mode: WorkspaceSelectionMode::Preserve,
            })
            .expect("项目焦点应可独立移动");
        assert_eq!(preserved.interaction.selected_item_ids, ["b", "d"]);

        // 项目焦点上的 Ctrl+A 保留当前项目作为选择锚点。
        let selected_container = state
            .apply_interaction(WorkspaceInteractionCommand::SelectFocusContainer)
            .expect("当前项目焦点容器应可全选");
        assert_eq!(
            selected_container.interaction.selected_item_ids,
            ["b", "c", "d"]
        );
        assert_eq!(
            selected_container
                .interaction
                .selection_anchor_id
                .as_deref(),
            Some("d")
        );

        // 跨焦点容器的 Shift 选择收束为目标单项。
        let reset = state
            .apply_interaction(WorkspaceInteractionCommand::FocusItem {
                item_id: "a".to_string(),
                selection_mode: WorkspaceSelectionMode::Range,
            })
            .expect("跨容器范围选择应建立新锚点");
        assert_eq!(reset.interaction.selected_item_ids, ["a"]);
        assert_eq!(reset.interaction.selection_anchor_id.as_deref(), Some("a"));

        std::fs::remove_file(path).expect("测试快照应可清理");
    }

    /// 验证框选、高层焦点、容器全选和围栏折叠协调公开交互状态。
    #[test]
    fn surface_focus_and_fence_collapse_coordinate_selection() {
        // 当前测试使用的持久化文件。
        let path = snapshot_path("surface-focus");
        // 已协调三个桌面项目的工作区状态。
        let state = workspace_state(path.clone());
        state
            .reconcile_items(
                1280.0,
                720.0,
                &["a".to_string(), "b".to_string(), "c".to_string()],
            )
            .expect("Shell 项目应可协调到工作区");
        // 当前测试使用的围栏标识。
        let fence_id = state
            .apply(WorkspaceCommand::CreateFence {
                title: "焦点".to_string(),
                bounds: WorkspaceRect {
                    left: 100.0,
                    top: 80.0,
                    width: 320.0,
                    height: 240.0,
                },
            })
            .expect("围栏应可创建")
            .fences[0]
            .id
            .clone();
        for item_id in ["b", "c"] {
            state
                .apply(WorkspaceCommand::MoveItemToFence {
                    item_id: item_id.to_string(),
                    fence_id: fence_id.clone(),
                    before_item_id: None,
                })
                .expect("项目应可加入围栏");
        }

        // 框选按视觉顺序发布的选择、锚点和焦点。
        let marquee = state
            .apply_interaction(WorkspaceInteractionCommand::SelectItems {
                item_ids: vec!["a".to_string(), "c".to_string()],
            })
            .expect("框选结果应可提交");
        assert_eq!(marquee.interaction.selected_item_ids, ["a", "c"]);
        assert_eq!(
            marquee.interaction.selection_anchor_id.as_deref(),
            Some("a")
        );

        // 围栏标题焦点清空项目选择。
        let fence_focus = state
            .apply_interaction(WorkspaceInteractionCommand::FocusSurface {
                focus: WorkspaceFocusTarget::Fence {
                    fence_id: fence_id.clone(),
                },
            })
            .expect("围栏标题应可获得高层焦点");
        assert!(fence_focus.interaction.selected_item_ids.is_empty());
        // 围栏标题上的 Ctrl+A 选择全部成员。
        let selected_fence = state
            .apply_interaction(WorkspaceInteractionCommand::SelectFocusContainer)
            .expect("围栏焦点容器应可全选");
        assert_eq!(selected_fence.interaction.selected_item_ids, ["b", "c"]);

        state
            .apply_interaction(WorkspaceInteractionCommand::FocusItem {
                item_id: "b".to_string(),
                selection_mode: WorkspaceSelectionMode::Replace,
            })
            .expect("围栏成员应可获得焦点");
        // 折叠围栏后的权威交互快照。
        let collapsed = state
            .apply(WorkspaceCommand::SetFenceCollapsed {
                fence_id: fence_id.clone(),
                collapsed: true,
            })
            .expect("围栏应可折叠");
        assert!(collapsed.interaction.selected_item_ids.is_empty());
        assert!(collapsed.interaction.selection_anchor_id.is_none());
        assert_eq!(
            collapsed.interaction.keyboard_focus,
            WorkspaceFocusTarget::Fence {
                fence_id: fence_id.clone()
            }
        );

        // 桌面背景焦点上的 Ctrl+A 只选择未归类区域。
        state
            .apply_interaction(WorkspaceInteractionCommand::FocusSurface {
                focus: WorkspaceFocusTarget::Background,
            })
            .expect("桌面背景应可获得焦点");
        let selected_background = state
            .apply_interaction(WorkspaceInteractionCommand::SelectFocusContainer)
            .expect("未归类焦点容器应可全选");
        assert_eq!(selected_background.interaction.selected_item_ids, ["a"]);

        std::fs::remove_file(path).expect("测试快照应可清理");
    }

    /// 验证项目归属保留来源位置并在移出或解散后恢复未归类状态。
    #[test]
    fn membership_commands_preserve_the_unclassified_source_position() {
        // 当前测试使用的持久化文件。
        let path = snapshot_path("membership");
        // 模拟当前应用会话的工作区状态。
        let state = workspace_state(path.clone());
        // 宿主网格与 Shell 项目形成的单次原子协调事务。
        let reconciled = state
            .reconcile_items(
                1280.0,
                720.0,
                &[
                    "::{occupied}".to_string(),
                    "::{project}".to_string(),
                    "::{document}".to_string(),
                ],
            )
            .expect("Shell 项目应可协调到未归类网格");
        assert_eq!(reconciled.revision, 1);
        assert_eq!(reconciled.grid.rows, 7);
        assert_eq!(reconciled.unclassified_items.len(), 3);
        // 新围栏的权威标识。
        let fence_id = state
            .apply(WorkspaceCommand::CreateFence {
                title: "资料".to_string(),
                bounds: WorkspaceRect {
                    left: 100.0,
                    top: 80.0,
                    width: 320.0,
                    height: 240.0,
                },
            })
            .expect("围栏应可创建")
            .fences[0]
            .id
            .clone();
        // 桌面项目进入围栏前的来源位置。
        let source_position = state
            .snapshot()
            .expect("协调快照应可读取")
            .unclassified_items
            .iter()
            .find(|item| item.stable_reference == "::{project}")
            .expect("项目应拥有未归类位置")
            .position;

        // 首次加入围栏后的权威快照。
        let snapshot = state
            .apply(WorkspaceCommand::MoveItemToFence {
                item_id: "::{project}".to_string(),
                fence_id: fence_id.clone(),
                before_item_id: None,
            })
            .expect("桌面项目应可加入围栏");
        assert_eq!(snapshot.fences[0].members, ["::{project}"]);
        assert_eq!(
            snapshot
                .unclassified_items
                .iter()
                .find(|item| item.stable_reference == "::{project}")
                .expect("围栏成员应保留来源位置")
                .position,
            source_position
        );
        // 项目释放到自身时保持当前成员顺序和修订。
        let unchanged = state
            .apply(WorkspaceCommand::MoveItemToFence {
                item_id: "::{project}".to_string(),
                fence_id: fence_id.clone(),
                before_item_id: Some("::{project}".to_string()),
            })
            .expect("释放到自身应保持当前归属");
        assert_eq!(unchanged, snapshot);

        // 项目移出围栏后的权威快照。
        let snapshot = state
            .apply(WorkspaceCommand::RemoveItemFromFence {
                item_id: "::{project}".to_string(),
            })
            .expect("桌面项目应可移出围栏");
        assert!(snapshot.fences[0].members.is_empty());
        assert_eq!(
            snapshot
                .unclassified_items
                .iter()
                .find(|item| item.stable_reference == "::{project}")
                .expect("移出项目应恢复来源位置")
                .position,
            source_position
        );

        state
            .apply(WorkspaceCommand::MoveItemToFence {
                item_id: "::{project}".to_string(),
                fence_id: fence_id.clone(),
                before_item_id: None,
            })
            .expect("桌面项目应可再次加入围栏");
        state
            .apply(WorkspaceCommand::MoveItemToFence {
                item_id: "::{document}".to_string(),
                fence_id: fence_id.clone(),
                before_item_id: None,
            })
            .expect("第二个桌面项目应可加入围栏");
        // 围栏解散并避让已有未归类项目后的权威快照。
        let snapshot = state
            .apply(WorkspaceCommand::DissolveFence { fence_id })
            .expect("围栏应可解散");
        assert!(snapshot.fences.is_empty());
        assert_eq!(
            snapshot
                .unclassified_items
                .iter()
                .find(|item| item.stable_reference == "::{project}")
                .expect("解散后项目应回到未归类区域")
                .position,
            crate::contracts::WorkspacePoint {
                left: 78.0,
                top: 98.0,
            }
        );
        assert_eq!(
            snapshot
                .unclassified_items
                .iter()
                .find(|item| item.stable_reference == "::{document}")
                .expect("解散后第二个项目应回到未归类区域")
                .position,
            crate::contracts::WorkspacePoint {
                left: 78.0,
                top: 196.0,
            }
        );

        std::fs::remove_file(path).expect("测试快照应可清理");
    }

    /// 验证未归类项目可以占用围栏成员释放出的视觉空网格。
    #[test]
    fn free_drop_repairs_a_fence_member_source_collision() {
        let path = snapshot_path("fence-source-collision");
        let state = workspace_state(path.clone());
        state
            .reconcile_items(1280.0, 720.0, &["a".to_string(), "b".to_string()])
            .expect("Shell 项目应可协调到工作区");
        let fence_id = state
            .apply(WorkspaceCommand::CreateFence {
                title: "资料".to_string(),
                bounds: WorkspaceRect {
                    left: 100.0,
                    top: 80.0,
                    width: 320.0,
                    height: 240.0,
                },
            })
            .expect("围栏应可创建")
            .fences[0]
            .id
            .clone();
        let source_position = state
            .snapshot()
            .expect("快照应可读取")
            .unclassified_items
            .iter()
            .find(|item| item.stable_reference == "a")
            .expect("围栏成员应拥有来源位置")
            .position;
        state
            .apply(WorkspaceCommand::MoveItemToFence {
                item_id: "a".to_string(),
                fence_id,
                before_item_id: None,
            })
            .expect("项目应可进入围栏");
        let snapshot = state
            .apply(WorkspaceCommand::CommitItemDrag {
                anchor_item_id: "b".to_string(),
                item_ids: vec!["b".to_string()],
                placements: vec![WorkspaceItemPlacement {
                    item_id: "b".to_string(),
                    position: source_position,
                }],
                target_fence_id: None,
                before_item_id: None,
                fence_scrolls: Vec::new(),
            })
            .expect("空白网格应可放置");
        let positions = snapshot
            .unclassified_items
            .iter()
            .map(|item| (item.position.left.to_bits(), item.position.top.to_bits()))
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(positions.len(), snapshot.unclassified_items.len());
        assert_eq!(
            snapshot
                .unclassified_items
                .iter()
                .find(|item| item.stable_reference == "b")
                .expect("项目 b 应存在")
                .position,
            source_position
        );
        std::fs::remove_file(path).expect("测试快照应可清理");
    }

    /// 验证围栏几何、折叠、布局和滚动状态形成完整持久化事务。
    #[test]
    fn fence_state_commands_persist_the_complete_visual_state() {
        // 当前测试使用的持久化文件。
        let path = snapshot_path("fence-state");
        // 模拟当前应用会话的工作区状态。
        let state = workspace_state(path.clone());
        // 新围栏的权威标识。
        let fence_id = state
            .apply(WorkspaceCommand::CreateFence {
                title: "任务".to_string(),
                bounds: WorkspaceRect {
                    left: 40.0,
                    top: 40.0,
                    width: 320.0,
                    height: 240.0,
                },
            })
            .expect("围栏应可创建")
            .fences[0]
            .id
            .clone();
        // 移动和缩放后提交的围栏边界。
        let bounds = WorkspaceRect {
            left: 180.0,
            top: 120.0,
            width: 420.0,
            height: 300.0,
        };
        state
            .apply(WorkspaceCommand::UpdateFenceBounds {
                fence_id: fence_id.clone(),
                bounds,
            })
            .expect("围栏边界应可更新");
        state
            .apply(WorkspaceCommand::SetFenceLayout {
                fence_id: fence_id.clone(),
                layout_mode: crate::contracts::FenceLayoutMode::List,
            })
            .expect("围栏布局应可更新");
        state
            .apply(WorkspaceCommand::SetFenceScroll {
                fence_id: fence_id.clone(),
                scroll_top: 72.0,
            })
            .expect("围栏滚动应可更新");
        // 围栏全部视觉状态提交后的权威快照。
        let snapshot = state
            .apply(WorkspaceCommand::SetFenceCollapsed {
                fence_id,
                collapsed: true,
            })
            .expect("围栏应可折叠");

        assert_eq!(snapshot.fences[0].bounds, bounds);
        assert_eq!(snapshot.fences[0].last_expanded_height, 300.0);
        assert_eq!(
            snapshot.fences[0].layout_mode,
            crate::contracts::FenceLayoutMode::List
        );
        assert_eq!(snapshot.fences[0].scroll_top, 72.0);
        assert!(snapshot.fences[0].collapsed);

        std::fs::remove_file(path).expect("测试快照应可清理");
    }

    /// 验证组拖动形成一个可完整撤销和重做的布局事务。
    #[test]
    fn group_drag_is_one_undoable_layout_transaction() {
        // 当前测试使用的持久化文件。
        let path = snapshot_path("group-drag-history");
        // 已协调三个项目的当前应用会话工作区。
        let state = workspace_state(path.clone());
        state
            .reconcile_items(
                1280.0,
                720.0,
                &["a".to_string(), "b".to_string(), "c".to_string()],
            )
            .expect("Shell 项目应可协调到工作区");
        // 当前组拖动使用的目标围栏标识。
        let fence_id = state
            .apply(WorkspaceCommand::CreateFence {
                title: "拖动".to_string(),
                bounds: WorkspaceRect {
                    left: 120.0,
                    top: 80.0,
                    width: 320.0,
                    height: 240.0,
                },
            })
            .expect("目标围栏应可创建")
            .fences[0]
            .id
            .clone();
        state
            .apply_interaction(WorkspaceInteractionCommand::SelectItems {
                item_ids: vec!["a".to_string(), "b".to_string()],
            })
            .expect("组拖动选择应可建立");

        // 两个项目原子进入围栏后的权威快照。
        let committed = state
            .apply(WorkspaceCommand::CommitItemDrag {
                anchor_item_id: "a".to_string(),
                item_ids: vec!["a".to_string(), "b".to_string()],
                placements: Vec::new(),
                target_fence_id: Some(fence_id.clone()),
                before_item_id: None,
                fence_scrolls: Vec::new(),
            })
            .expect("组拖动应可原子提交");
        assert_eq!(committed.fences[0].members, ["a", "b"]);
        assert_eq!(committed.interaction.selected_item_ids, ["a", "b"]);

        // 撤销组拖动后的完整布局。
        let undone = state
            .apply(WorkspaceCommand::UndoLayout)
            .expect("组拖动应可撤销");
        assert!(undone.fences[0].members.is_empty());
        assert_eq!(undone.interaction.selected_item_ids, ["a", "b"]);

        // 重做组拖动后的完整布局。
        let redone = state
            .apply(WorkspaceCommand::RedoLayout)
            .expect("组拖动应可重做");
        assert_eq!(redone.fences[0].members, ["a", "b"]);
        assert_eq!(
            redone.interaction.keyboard_focus,
            WorkspaceFocusTarget::Item {
                item_id: "a".to_string()
            }
        );

        std::fs::remove_file(path).expect("测试快照应可清理");
    }

    /// 验证 Shell 精确重命名同步布局成员、选择、锚点和键盘焦点。
    #[test]
    fn shell_rename_remaps_all_runtime_item_references() {
        let path = snapshot_path("shell-rename"); // 当前测试使用的持久化文件。
        let state = workspace_state(path.clone()); // 模拟当前应用会话的工作区状态。
        state
            .reconcile_items(1280.0, 720.0, &["old".to_string()])
            .expect("原项目应可协调到工作区");
        let fence_id = state
            .apply(WorkspaceCommand::CreateFence {
                title: "重命名".to_string(),
                bounds: WorkspaceRect {
                    left: 100.0,
                    top: 80.0,
                    width: 320.0,
                    height: 240.0,
                },
            })
            .expect("围栏应可创建")
            .fences[0]
            .id
            .clone(); // 当前项目所属围栏标识。
        state
            .apply(WorkspaceCommand::MoveItemToFence {
                item_id: "old".to_string(),
                fence_id,
                before_item_id: None,
            })
            .expect("原项目应可加入围栏");
        state
            .apply_interaction(WorkspaceInteractionCommand::FocusItem {
                item_id: "old".to_string(),
                selection_mode: WorkspaceSelectionMode::Replace,
            })
            .expect("原项目应可建立选择和焦点");

        let renamed = state
            .reconcile_shell_items(
                1280.0,
                720.0,
                &["new".to_string()],
                &[("old".to_string(), "new".to_string())],
            )
            .expect("Shell 重命名事实应可协调"); // 精确重命名和最终枚举共同产生的快照。
        assert_eq!(renamed.fences[0].members, ["new"]);
        assert_eq!(renamed.interaction.selected_item_ids, ["new"]);
        assert_eq!(
            renamed.interaction.selection_anchor_id.as_deref(),
            Some("new")
        );
        assert_eq!(
            renamed.interaction.keyboard_focus,
            WorkspaceFocusTarget::Item {
                item_id: "new".to_string()
            }
        );

        std::fs::remove_file(path).expect("测试快照应可清理");
    }

    /// 验证 Explorer 初始归属、断开投射、重连恢复和 DPI 变化保持持久显示器。
    #[test]
    fn monitor_reconciliation_preserves_persistent_assignment() {
        let path = snapshot_path("monitor-reconciliation"); // 当前测试使用的持久化文件。
        let state = workspace_state(path.clone()); // 模拟首次逐屏启动的工作区状态。
        let primary = monitor("DISPLAY1", true, 0, 1.0); // 当前主显示器。
        let secondary = monitor("DISPLAY2", false, 1920, 1.5); // 当前混合 DPI 次显示器。
        let initial = state
            .reconcile_host_monitors(
                &[primary.clone(), secondary.clone()],
                &["primary-item".to_string(), "secondary-item".to_string()],
                &[("secondary-item".to_string(), "DISPLAY2".to_string())],
                &[],
            )
            .expect("Explorer 项目归属应可首次导入");
        let secondary_position = initial
            .unclassified_items
            .iter()
            .find(|item| item.stable_reference == "secondary-item")
            .expect("次屏项目应存在")
            .position; // 次屏项目持久 DIP 位置。
        assert_eq!(initial.unclassified_items[1].monitor_id, "DISPLAY2");

        let projected = state
            .reconcile_host_monitors(
                std::slice::from_ref(&primary),
                &["primary-item".to_string(), "secondary-item".to_string()],
                &[],
                &[],
            )
            .expect("显示器断开后项目应可临时投射");
        let projected_item = projected
            .unclassified_items
            .iter()
            .find(|item| item.stable_reference == "secondary-item")
            .expect("投射项目应存在");
        assert_eq!(projected_item.monitor_id, "DISPLAY2");
        assert_eq!(
            projected_item.projection_monitor_id.as_deref(),
            Some("DISPLAY1")
        );
        assert_eq!(projected_item.position, secondary_position);

        let mut scaled_secondary = secondary; // DPI-only 重建后的次显示器。
        scaled_secondary.scale_factor = 2.0;
        scaled_secondary.dpi = 192;
        let restored = state
            .reconcile_host_monitors(
                &[primary, scaled_secondary],
                &["primary-item".to_string(), "secondary-item".to_string()],
                &[],
                &[],
            )
            .expect("显示器重连和 DPI 变化应可协调");
        let restored_item = restored
            .unclassified_items
            .iter()
            .find(|item| item.stable_reference == "secondary-item")
            .expect("恢复项目应存在");
        assert_eq!(restored_item.monitor_id, "DISPLAY2");
        assert_eq!(restored_item.projection_monitor_id, None);
        assert_eq!(restored_item.position, secondary_position);

        std::fs::remove_file(path).expect("测试快照应可清理");
    }

    /// 验证跨屏围栏和项目拖动在一个布局事务中改写目标显示器归属。
    #[test]
    fn cross_monitor_layout_commands_use_the_target_monitor() {
        let path = snapshot_path("cross-monitor-command"); // 当前测试使用的持久化文件。
        let state = workspace_state(path.clone()); // 当前逐屏布局状态。
        let monitors = [
            monitor("DISPLAY1", true, 0, 1.0),
            monitor("DISPLAY2", false, 1920, 1.5),
        ]; // 当前双屏混合 DPI 环境。
        state
            .reconcile_host_monitors(&monitors, &["item".to_string()], &[], &[])
            .expect("项目应可初始化");
        let fence = state
            .apply_on_monitor(
                Some("DISPLAY2"),
                WorkspaceCommand::CreateFence {
                    title: "次屏围栏".to_string(),
                    bounds: WorkspaceRect {
                        left: 80.0,
                        top: 80.0,
                        width: 320.0,
                        height: 240.0,
                    },
                },
            )
            .expect("围栏应在目标显示器创建");
        assert_eq!(fence.fences[0].monitor_id, "DISPLAY2");

        let moved = state
            .apply_on_monitor(
                Some("DISPLAY2"),
                WorkspaceCommand::CommitItemDrag {
                    anchor_item_id: "item".to_string(),
                    item_ids: vec!["item".to_string()],
                    placements: vec![WorkspaceItemPlacement {
                        item_id: "item".to_string(),
                        position: WorkspacePoint {
                            left: 112.0,
                            top: 172.0,
                        },
                    }],
                    target_fence_id: None,
                    before_item_id: None,
                    fence_scrolls: Vec::new(),
                },
            )
            .expect("项目应可跨屏提交");
        assert_eq!(moved.unclassified_items[0].monitor_id, "DISPLAY2");
        assert_eq!(moved.unclassified_items[0].position.left, 112.0);

        std::fs::remove_file(path).expect("测试快照应可清理");
    }

    /// 验证虚拟桌面布局、选择和焦点隔离，删除后归档并可绑定新身份恢复。
    #[test]
    fn virtual_desktop_layouts_are_isolated_archived_and_restored() {
        let path = snapshot_path("virtual-desktop-layouts"); // 当前测试使用的多布局持久化文件。
        let desktop_ids = ["desktop-a".to_string(), "desktop-b".to_string()]; // Windows 初始活动虚拟桌面身份。
        let state =
            WorkspaceState::load_for_virtual_desktops(path.clone(), "desktop-a", &desktop_ids)
                .expect("多虚拟桌面工作区应可启动");
        let monitors = [monitor("DISPLAY1", true, 0, 1.0)]; // 当前测试使用的单显示器环境。
        let item_ids = ["first".to_string(), "second".to_string()]; // 每个桌面共享的完整 Shell 项目集合。
        state
            .reconcile_virtual_desktops("desktop-a", &desktop_ids, &monitors, &item_ids)
            .expect("初始虚拟桌面集合应可协调");
        state
            .apply_on_monitor(
                Some("DISPLAY1"),
                WorkspaceCommand::CreateFence {
                    title: "桌面 A".to_string(),
                    bounds: WorkspaceRect {
                        left: 80.0,
                        top: 80.0,
                        width: 320.0,
                        height: 240.0,
                    },
                },
            )
            .expect("桌面 A 围栏应可创建");
        state
            .apply_interaction_on_monitor(
                "DISPLAY1",
                WorkspaceInteractionCommand::FocusItem {
                    item_id: "first".to_string(),
                    selection_mode: WorkspaceSelectionMode::Replace,
                },
            )
            .expect("桌面 A 选择应可建立");

        let desktop_b = state
            .reconcile_virtual_desktops("desktop-b", &desktop_ids, &monitors, &item_ids)
            .expect("切换到桌面 B 应激活独立布局");
        assert!(desktop_b.fences.is_empty());
        assert!(desktop_b.interaction.selected_item_ids.is_empty());
        state
            .apply_on_monitor(
                Some("DISPLAY1"),
                WorkspaceCommand::CreateFence {
                    title: "桌面 B".to_string(),
                    bounds: WorkspaceRect {
                        left: 160.0,
                        top: 120.0,
                        width: 360.0,
                        height: 260.0,
                    },
                },
            )
            .expect("桌面 B 围栏应可创建");
        state
            .apply_interaction_on_monitor(
                "DISPLAY1",
                WorkspaceInteractionCommand::FocusItem {
                    item_id: "second".to_string(),
                    selection_mode: WorkspaceSelectionMode::Replace,
                },
            )
            .expect("桌面 B 选择应可建立");

        let desktop_a = state
            .reconcile_virtual_desktops("desktop-a", &desktop_ids, &monitors, &item_ids)
            .expect("切回桌面 A 应恢复独立状态");
        assert_eq!(desktop_a.fences[0].title, "桌面 A");
        assert_eq!(desktop_a.interaction.selected_item_ids, ["first"]);

        state
            .reconcile_virtual_desktops(
                "desktop-a",
                &["desktop-a".to_string()],
                &monitors,
                &item_ids,
            )
            .expect("删除桌面 B 后布局应进入归档");
        let archive = state
            .layout_archives()
            .expect("布局归档应可读取")
            .into_iter()
            .find(|archive| archive.source_virtual_desktop_id == "desktop-b")
            .expect("桌面 B 布局应存在归档");
        state
            .restore_layout_archive(&archive.archive_id, "desktop-c", &monitors, &item_ids)
            .expect("布局归档应可绑定新桌面身份");
        let desktop_c = state
            .reconcile_virtual_desktops(
                "desktop-c",
                &["desktop-a".to_string(), "desktop-c".to_string()],
                &monitors,
                &item_ids,
            )
            .expect("新桌面身份应激活恢复布局");
        assert_eq!(desktop_c.fences[0].title, "桌面 B");
        assert!(desktop_c.interaction.selected_item_ids.is_empty());

        std::fs::remove_file(path).expect("测试快照应可清理");
    }

    /// 验证每个虚拟桌面拥有独立撤销历史。
    #[test]
    fn virtual_desktop_layout_history_is_independent() {
        let path = snapshot_path("virtual-desktop-history"); // 当前测试使用的多布局历史文件。
        let desktop_ids = ["desktop-a".to_string(), "desktop-b".to_string()]; // 当前活动虚拟桌面身份。
        let state =
            WorkspaceState::load_for_virtual_desktops(path.clone(), "desktop-a", &desktop_ids)
                .expect("多布局历史工作区应可启动");
        let monitors = [monitor("DISPLAY1", true, 0, 1.0)]; // 当前测试显示器。
        state
            .reconcile_virtual_desktops("desktop-a", &desktop_ids, &monitors, &[])
            .expect("虚拟桌面集合应可初始化");
        state
            .apply_on_monitor(
                Some("DISPLAY1"),
                WorkspaceCommand::CreateFence {
                    title: "A".to_string(),
                    bounds: WorkspaceRect {
                        left: 80.0,
                        top: 80.0,
                        width: 320.0,
                        height: 240.0,
                    },
                },
            )
            .expect("桌面 A 事务应可提交");
        state
            .reconcile_virtual_desktops("desktop-b", &desktop_ids, &monitors, &[])
            .expect("应可切换到桌面 B");
        state
            .apply_on_monitor(
                Some("DISPLAY1"),
                WorkspaceCommand::CreateFence {
                    title: "B".to_string(),
                    bounds: WorkspaceRect {
                        left: 120.0,
                        top: 100.0,
                        width: 320.0,
                        height: 240.0,
                    },
                },
            )
            .expect("桌面 B 事务应可提交");
        let undone_b = state
            .apply_on_monitor(Some("DISPLAY1"), WorkspaceCommand::UndoLayout)
            .expect("桌面 B 应撤销自身事务");
        assert!(undone_b.fences.is_empty());

        state
            .reconcile_virtual_desktops("desktop-a", &desktop_ids, &monitors, &[])
            .expect("应可切回桌面 A");
        assert_eq!(
            state.snapshot().expect("桌面 A 布局应可读取").fences[0].title,
            "A"
        );
        let undone_a = state
            .apply_on_monitor(Some("DISPLAY1"), WorkspaceCommand::UndoLayout)
            .expect("桌面 A 应撤销自身事务");
        assert!(undone_a.fences.is_empty());

        std::fs::remove_file(path).expect("测试快照应可清理");
    }

    /// 验证删除布局归档只保留 30 天窗口内的记录。
    #[test]
    fn deleted_layout_archives_expire_after_thirty_days() {
        let now = 1_000_000; // 当前测试使用的固定 Unix 时间。
        let mut archives = vec![
            WorkspaceLayoutArchive {
                archive_id: "expired".to_string(),
                source_virtual_desktop_id: "desktop-a".to_string(),
                archived_at_unix: now - LAYOUT_ARCHIVE_RETENTION_SECONDS - 1,
                expires_at_unix: now - 1,
                layout: Default::default(),
            },
            WorkspaceLayoutArchive {
                archive_id: "active".to_string(),
                source_virtual_desktop_id: "desktop-b".to_string(),
                archived_at_unix: now,
                expires_at_unix: now + LAYOUT_ARCHIVE_RETENTION_SECONDS,
                layout: Default::default(),
            },
        ]; // 当前包含过期和有效记录的布局归档集合。

        purge_expired_archives(&mut archives, now);
        assert_eq!(archives.len(), 1);
        assert_eq!(archives[0].archive_id, "active");
    }

    /// 验证当前备份格式拒绝被修改但未更新校验值的载荷。
    #[test]
    fn current_backup_rejects_tampered_payload() {
        let store = workspace_store_from_payload(PersistedWorkspacePayload {
            current_virtual_desktop_id: "desktop-a".to_string(),
            layouts: vec![PersistedVirtualDesktopLayout {
                virtual_desktop_id: "desktop-a".to_string(),
                layout: Default::default(),
            }],
            archives: Vec::new(),
            explorer_layout_imported: true,
        })
        .expect("当前备份应可创建"); // 已计算完整性校验值的原始备份。
        let mut value = serde_json::to_value(store).expect("备份应可序列化"); // 等待模拟损坏的 JSON 载荷。
        value["payload"]["layouts"][0]["layout"]["revision"] = serde_json::json!(9);

        let error =
            decode_workspace_store(value, "desktop-a").expect_err("校验值未同步的备份应被拒绝");

        assert_eq!(error.code, "workspace-restore-failed");
        assert!(error.message.contains("完整性校验失败"));
    }

    /// 验证 Ticket 10 的版本 2 多布局文件在临时副本中迁移到当前格式。
    #[test]
    fn version_two_workspace_store_migrates_to_current_schema() {
        let value = serde_json::json!({
            "storeVersion": 2,
            "currentVirtualDesktopId": "desktop-a",
            "layouts": [{
                "virtualDesktopId": "desktop-a",
                "layout": WorkspaceSnapshot::default()
            }],
            "archives": [],
            "explorerLayoutImported": true
        }); // Ticket 10 实际写入的旧多布局结构。

        let migrated = decode_workspace_store(value, "desktop-a").expect("版本 2 工作区应可迁移");

        assert_eq!(migrated.store_version, 3);
        assert!(migrated.checksum.starts_with("fnv1a64:"));
        assert_eq!(migrated.payload.layouts.len(), 1);
    }

    /// 验证每个虚拟桌面的自动快照只保留最近 20 份。
    #[test]
    fn automatic_snapshots_keep_the_latest_twenty_per_virtual_desktop() {
        let directory = snapshot_path("auto-snapshot-limit").with_extension("snapshots"); // 当前测试独占的自动快照目录。
        let store = workspace_store_from_payload(PersistedWorkspacePayload {
            current_virtual_desktop_id: "desktop-a".to_string(),
            layouts: vec![PersistedVirtualDesktopLayout {
                virtual_desktop_id: "desktop-a".to_string(),
                layout: Default::default(),
            }],
            archives: Vec::new(),
            explorer_layout_imported: true,
        })
        .expect("自动快照载荷应可创建");
        for _ in 0..AUTO_SNAPSHOT_LIMIT + 3 {
            write_auto_snapshot(&directory, "desktop-a", &store).expect("自动快照应可写入");
        }

        let count = std::fs::read_dir(&directory)
            .expect("自动快照目录应可读取")
            .count();
        assert_eq!(count, AUTO_SNAPSHOT_LIMIT);

        std::fs::remove_dir_all(directory).expect("自动快照目录应可清理");
    }

    /// 验证损坏备份在还原前被拒绝且当前工作区保持完整。
    #[test]
    fn corrupted_restore_preserves_the_current_workspace() {
        let path = snapshot_path("restore-rollback"); // 当前测试使用的权威工作区文件。
        let backup_path = snapshot_path("restore-rollback-backup"); // 当前测试使用的手动备份文件。
        let state = workspace_state(path.clone());
        state
            .reconcile_items(1280.0, 720.0, &["project".to_string()])
            .expect("当前工作区项目应可初始化");
        state
            .write_manual_backup(&backup_path)
            .expect("手动备份应可写入");
        let before = state.snapshot().expect("还原前工作区应可读取"); // 损坏还原必须保持的当前状态。
        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&backup_path).expect("手动备份应可读取"))
                .expect("手动备份应为 JSON");
        value["payload"]["layouts"][0]["layout"]["revision"] = serde_json::json!(77);
        std::fs::write(
            &backup_path,
            serde_json::to_vec_pretty(&value).expect("损坏备份应可序列化"),
        )
        .expect("损坏备份应可写入");

        let result = state.restore_backup(
            &backup_path,
            "default",
            &["default".to_string()],
            &[],
            &[monitor("DISPLAY1", true, 0, 1.0)],
            &["project".to_string()],
        );

        assert_eq!(
            result.expect_err("损坏备份应被拒绝").code,
            "workspace-restore-failed"
        );
        assert_eq!(state.snapshot().expect("失败后工作区应可读取"), before);

        drop(state);
        std::fs::remove_file(path).expect("测试工作区应可清理");
        std::fs::remove_file(backup_path).expect("测试备份应可清理");
    }

    /// 验证布局历史跳过已离开项目并保留历史提交后新增的项目。
    #[test]
    fn layout_history_merge_reports_partial_restore_and_keeps_external_items() {
        let item = |stable_reference: &str, left: f64| UnclassifiedItemSnapshot {
            stable_reference: stable_reference.to_string(),
            monitor_id: "DISPLAY1".to_string(),
            projection_monitor_id: None,
            projection_position: None,
            position: WorkspacePoint { left, top: 60.0 },
            z_index: left as u32,
        }; // 当前测试构造稳定项目位置的最小工厂。
        let target = WorkspaceSnapshot {
            unclassified_items: vec![item("existing", 20.0), item("removed", 112.0)],
            ..WorkspaceSnapshot::default()
        }; // 历史事务原本覆盖的布局。
        let current = WorkspaceSnapshot {
            unclassified_items: vec![item("existing", 204.0), item("created", 296.0)],
            ..WorkspaceSnapshot::default()
        }; // 外部删除和新增后的当前布局。

        let (merged, restored_count, changed_count) = merge_layout_history_target(target, &current);

        assert_eq!(restored_count, 1);
        assert_eq!(changed_count, 1);
        assert!(
            merged
                .unclassified_items
                .iter()
                .any(|item| item.stable_reference == "created")
        );
        assert!(
            merged
                .unclassified_items
                .iter()
                .all(|item| item.stable_reference != "removed")
        );
    }
}
