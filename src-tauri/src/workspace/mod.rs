use std::path::PathBuf;
use std::sync::Mutex;

use crate::contracts::{
    FENCE_MIN_HEIGHT, FENCE_MIN_WIDTH, FenceLayoutMode, FenceSnapshot, UnclassifiedItemSnapshot,
    WORKSPACE_GRID_BOTTOM_MARGIN, WORKSPACE_GRID_COLUMN_WIDTH, WORKSPACE_GRID_LEFT,
    WORKSPACE_GRID_ROW_HEIGHT, WORKSPACE_GRID_TOP, WORKSPACE_SCHEMA_VERSION, WorkspaceCommand,
    WorkspaceError, WorkspaceGrid, WorkspacePoint, WorkspaceRect, WorkspaceSnapshot,
};
use crate::persistence::{load_json, save_json_atomic};

/// 持有正式桌面内容工作区的权威布局和持久化位置。
pub struct WorkspaceState {
    /// command 共享的当前工作区快照。
    snapshot: Mutex<WorkspaceSnapshot>,
    /// 每次成功布局事务使用的原子 JSON 文件。
    persistence_path: PathBuf,
}

/// 提供工作区 command 使用的领域操作。
impl WorkspaceState {
    /// 从原子 JSON 快照启动工作区，首次启动使用空布局。
    pub fn load(persistence_path: PathBuf) -> Result<Self, WorkspaceError> {
        // 磁盘中恢复或首次创建的工作区快照。
        let snapshot = load_json::<WorkspaceSnapshot>(&persistence_path)
            .map_err(|error| WorkspaceError::restore_failed(&error))?
            .unwrap_or_default();
        snapshot.validate()?;
        crate::log_info!(
            "工作区：快照恢复完成，修订={}，围栏={}，路径={}",
            snapshot.revision,
            snapshot.fences.len(),
            persistence_path.display()
        );

        Ok(Self {
            snapshot: Mutex::new(snapshot),
            persistence_path,
        })
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

    /// 将宿主网格与 Shell 完整项目集合原子协调进 Workspace。
    pub fn reconcile_items(
        &self,
        workspace_width: f64,
        workspace_height: f64,
        item_ids: &[String],
    ) -> Result<WorkspaceSnapshot, WorkspaceError> {
        // 当前正式宿主尺寸对应的网格配置。
        let grid = workspace_grid(workspace_width, workspace_height)?;
        // 等待项目协调的当前工作区快照。
        let mut current = self
            .snapshot
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())?;
        // 保持当前快照可回滚的项目协调副本。
        let mut next = current.clone();
        next.grid = grid;
        next.reconcile_items(item_ids)?;
        if next == *current {
            return Ok(current.clone());
        }

        next.revision += 1;
        next.validate()?;
        save_json_atomic(&self.persistence_path, &next).map_err(|error| {
            crate::log_error!("工作区：桌面项目协调保存失败，错误={error}");
            WorkspaceError::persistence_failed(&error)
        })?;
        *current = next.clone();
        crate::log_info!(
            "工作区：桌面项目位置已协调，修订={}，项目={}",
            next.revision,
            next.unclassified_items.len()
        );
        Ok(next)
    }

    /// 校验并原子提交一个完整布局事务。
    pub fn apply(&self, command: WorkspaceCommand) -> Result<WorkspaceSnapshot, WorkspaceError> {
        // 等待事务提交的当前工作区快照。
        let mut current = self
            .snapshot
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())?;
        // 保持当前快照可回滚的事务副本。
        let mut next = current.clone();
        // 当前命令用于中文日志的稳定名称。
        let command_name = next.apply(command)?;
        if next == *current {
            crate::log_debug!("工作区：布局事务保持当前状态，命令={command_name}");
            return Ok(current.clone());
        }
        next.revision += 1;
        next.validate()?;
        save_json_atomic(&self.persistence_path, &next).map_err(|error| {
            crate::log_error!("工作区：原子快照保存失败，命令={command_name}，错误={error}");
            WorkspaceError::persistence_failed(&error)
        })?;
        *current = next.clone();
        crate::log_info!(
            "工作区：布局事务已提交，命令={command_name}，修订={}，围栏={}",
            next.revision,
            next.fences.len()
        );
        Ok(next)
    }
}

/// 创建首次启动的空工作区快照。
impl Default for WorkspaceSnapshot {
    fn default() -> Self {
        Self {
            schema_version: WORKSPACE_SCHEMA_VERSION,
            revision: 0,
            grid: WorkspaceGrid::default(),
            fences: Vec::new(),
            unclassified_items: Vec::new(),
        }
    }
}

/// 实现工作区快照内部的布局事务和完整性校验。
impl WorkspaceSnapshot {
    /// 协调 Shell 项目身份并为新项目分配首个空网格位置。
    fn reconcile_items(&mut self, item_ids: &[String]) -> Result<(), WorkspaceError> {
        // 当前 Shell 快照中的稳定项目引用。
        let active_ids = item_ids
            .iter()
            .map(String::as_str)
            .collect::<std::collections::HashSet<_>>();
        // 当前全部围栏继续持有的成员引用。
        let member_ids = self
            .fences
            .iter()
            .flat_map(|fence| fence.members.iter().map(String::as_str))
            .collect::<std::collections::HashSet<_>>();
        self.unclassified_items.retain(|item| {
            active_ids.contains(item.stable_reference.as_str())
                || member_ids.contains(item.stable_reference.as_str())
        });

        // 已由未归类项目或围栏成员来源位置占用的网格位置。
        let mut occupied = self
            .unclassified_items
            .iter()
            .map(|item| item.position)
            .collect::<Vec<_>>();
        // 当前版本化网格的安全容量。
        let capacity = grid_capacity(self.grid)?;
        for item_id in item_ids {
            validate_id(item_id, "桌面项目标识无效")?;
            if self
                .unclassified_items
                .iter()
                .any(|item| item.stable_reference == *item_id)
            {
                continue;
            }

            // 当前新项目获得的首个空网格位置。
            let position = (0..capacity)
                .map(|index| grid_position(self.grid, index))
                .find(|position| !occupied.contains(position))
                .ok_or_else(|| WorkspaceError::invalid_command("未归类网格空间不足"))?;
            self.unclassified_items.push(UnclassifiedItemSnapshot {
                stable_reference: item_id.clone(),
                position,
            });
            occupied.push(position);
        }
        Ok(())
    }

    /// 应用一个已反序列化的工作区命令。
    fn apply(&mut self, command: WorkspaceCommand) -> Result<&'static str, WorkspaceError> {
        match command {
            WorkspaceCommand::CreateFence { title, bounds } => {
                validate_title(&title)?;
                validate_rect(bounds)?;
                // 当前事务分配的稳定围栏标识。
                let id = format!("fence-{}", self.revision + 1);
                // 新围栏使用的最高层级。
                let z_index = self.next_z_index();
                self.fences.push(FenceSnapshot {
                    id,
                    title,
                    bounds,
                    members: Vec::new(),
                    layout_mode: FenceLayoutMode::Tile,
                    scroll_top: 0.0,
                    collapsed: false,
                    last_expanded_height: bounds.height,
                    z_index,
                });
                Ok("创建围栏")
            }
            WorkspaceCommand::UpdateFenceBounds { fence_id, bounds } => {
                validate_rect(bounds)?;
                // 等待移动或缩放提交的围栏。
                let fence = self.fence_mut(&fence_id)?;
                fence.bounds = bounds;
                fence.last_expanded_height = bounds.height;
                Ok("更新围栏边界")
            }
            WorkspaceCommand::SetFenceCollapsed {
                fence_id,
                collapsed,
            } => {
                // 等待折叠状态提交的围栏。
                let fence = self.fence_mut(&fence_id)?;
                fence.collapsed = collapsed;
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
                self.fence(&fence_id)?;
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
                // 目标围栏获得当前最高层级。
                let next_z_index = self.next_z_index();
                // 等待成员顺序提交的目标围栏。
                let fence = self.fence_mut(&fence_id)?;
                // 指定后继成员对应的手动插入位置。
                let insertion_index = before_item_id
                    .as_ref()
                    .and_then(|target| fence.members.iter().position(|member| member == target))
                    .unwrap_or(fence.members.len());
                fence.members.insert(insertion_index, item_id);
                fence.z_index = next_z_index;
                Ok("更新围栏归属")
            }
            WorkspaceCommand::RemoveItemFromFence { item_id } => {
                validate_id(&item_id, "桌面项目标识无效")?;
                for fence in &mut self.fences {
                    fence.members.retain(|member| member != &item_id);
                }
                Ok("移出围栏")
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
                // 围栏内容区左上对应的组锚点网格索引。
                let anchor_column =
                    ((fence.bounds.left + 10.0 - self.grid.left) / self.grid.column_width)
                        .round()
                        .clamp(0.0, f64::from(self.grid.columns - 1)) as u32;
                // 围栏内容区左上对应的组锚点网格行。
                let anchor_row = ((fence.bounds.top + 54.0 - self.grid.top) / self.grid.row_height)
                    .round()
                    .clamp(0.0, f64::from(self.grid.rows - 1))
                    as u32;
                // 当前未归类网格总容量。
                let capacity = grid_capacity(self.grid)?;
                // 下一个成员开始避让的位置。
                let mut next_index = anchor_column * self.grid.rows + anchor_row;
                // 排除当前围栏成员来源格后的已占位置。
                let mut occupied = self
                    .unclassified_items
                    .iter()
                    .filter(|item| !fence.members.contains(&item.stable_reference))
                    .map(|item| item.position)
                    .collect::<Vec<_>>();

                for member in fence.members {
                    // 从当前连续位置开始找到的首个空网格索引。
                    let available_index = (0..capacity)
                        .map(|offset| (next_index + offset) % capacity)
                        .find(|index| {
                            // 当前避让扫描索引对应的网格位置。
                            let candidate = grid_position(self.grid, *index);
                            !occupied.contains(&candidate)
                        })
                        .ok_or_else(|| WorkspaceError::invalid_command("未归类网格空间不足"))?;
                    // 当前成员避让已有项目后的最终网格位置。
                    let position = grid_position(self.grid, available_index);
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
        }
    }

    /// 校验恢复或提交后的完整工作区快照。
    fn validate(&self) -> Result<(), WorkspaceError> {
        if self.schema_version != WORKSPACE_SCHEMA_VERSION {
            return Err(WorkspaceError::invalid_command("工作区 Schema 版本无效"));
        }
        grid_capacity(self.grid)?;
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
        if source_ids.len() != self.unclassified_items.len()
            || member_ids.iter().any(|member| !source_ids.contains(member))
        {
            return Err(WorkspaceError::invalid_command("围栏成员来源位置状态无效"));
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

    /// 分配新的围栏最高层级。
    fn next_z_index(&self) -> u32 {
        self.fences
            .iter()
            .map(|fence| fence.z_index)
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
) -> Result<WorkspaceGrid, WorkspaceError> {
    if !workspace_width.is_finite()
        || !workspace_height.is_finite()
        || workspace_width <= 0.0
        || workspace_height <= 0.0
    {
        return Err(WorkspaceError::invalid_command("工作区尺寸无效"));
    }
    Ok(WorkspaceGrid {
        left: WORKSPACE_GRID_LEFT,
        top: WORKSPACE_GRID_TOP,
        column_width: WORKSPACE_GRID_COLUMN_WIDTH,
        row_height: WORKSPACE_GRID_ROW_HEIGHT,
        rows: ((workspace_height - WORKSPACE_GRID_BOTTOM_MARGIN) / WORKSPACE_GRID_ROW_HEIGHT)
            .floor()
            .max(1.0) as u32,
        columns: ((workspace_width - WORKSPACE_GRID_LEFT) / WORKSPACE_GRID_COLUMN_WIDTH)
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

#[cfg(test)]
mod tests {
    use super::WorkspaceState;
    use crate::contracts::{WorkspaceCommand, WorkspaceRect};
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
        assert_eq!(reconciled.grid.rows, 6);
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
                left: 112.0,
                top: 172.0,
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
                left: 112.0,
                top: 284.0,
            }
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
}
