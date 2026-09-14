use std::collections::{HashMap, HashSet};

use super::{WorkspaceError, WorkspaceItemPlacement, WorkspacePoint, WorkspaceSnapshot, grid_capacity, grid_position};

/// 释放时固定的共享目标和源视图空间意图，文件完成后不重新命中。
#[derive(Clone, Debug)]
pub(crate) struct DropTarget
{
    pub monitor_id: String,                        // 原空间目标显示器。
    pub revision: u64,                             // 开始传输前必须一致的布局版本。
    pub fence_id: Option<String>,                  // 全局稳定围栏身份；None 表示未归类。
    pub portal_id: Option<String>,                 // 文件夹门户实例身份；与围栏和未归类互斥。
    pub portal_path: Option<String>,               // 当前门户目录，供真实投放锁定目标。
    pub before_item_id: Option<String>,            // 手动顺序的稳定后继。
    pub position: WorkspacePoint,                  // 已扣除锚点偏移的释放位置。
    pub free: bool,                                // 内部 Alt 自由定位。
    pub insertion: Option<(String, String)>,       // 原生几何确认的桌面插入目标与方向。
    pub scrolls: Vec<super::WorkspaceFenceScroll>, // 原视图最终滚动位置。
}

/// 一次外部投放的布局部分，供同一 Shell 历史单元按成功项目恢复。
#[derive(Clone, Debug)]
pub(crate) struct DropLayoutChange
{
    pub before: WorkspaceSnapshot,    // 实际提交之前的状态，不回滚等待期间其他操作。
    pub after: WorkspaceSnapshot,     // 本次已持久化的状态。
    pub layout_only_ids: Vec<String>, // 没有执行文件传输的已有项目。
    pub item_ids: Vec<String>,        // 实际完成归属提交的准确结果身份。
}

impl super::WorkspaceState
{
    /// 按准确文件完成映射恢复共享归属，只恢复未被后续操作改变的位置。
    pub(crate) fn restore_drop_layout(&self, change: &DropLayoutChange, mappings: &[(String, String, String, bool)], files: &crate::contracts::DesktopItemSnapshot) -> Result<Vec<String>, WorkspaceError>
    {
        let mut current = self.snapshot.lock().map_err(|_| WorkspaceError::state_unavailable())?; // 当前工作区权威。
        let imported = self.explorer_layout_imported.lock().map_err(|_| WorkspaceError::state_unavailable())?; // 与布局共同保存的导入状态。
        let mut candidate = current.clone(); // 只恢复当前统一布局。
        let original_positions = candidate.unclassified_items.clone(); // 空间冲突时保留后续操作结果。
        let active = files.items.iter().map(|item| item.stable_reference.as_str()).collect::<HashSet<_>>(); // 只使用本次明确枚举的实际身份。
        let mut restored = Vec::new(); // 归属检查成功的原历史身份。
        let mut retired = HashSet::new(); // 文件已离开桌面的准确旧引用，不清扫其他新增文件。
        for (original, previous, actual, redo) in mappings
        {
            let (desired, expected) = if *redo { (&change.after, &change.before) } else { (&change.before, &change.after) };
            let expected_owner = expected.fences.iter().find(|fence| fence.members.contains(original)).map(|fence| fence.id.as_str());
            let desired_fence = desired.fences.iter().find(|fence| fence.members.contains(original));
            let current_owner = candidate.fences.iter().find(|fence| fence.members.contains(previous) || fence.members.contains(actual)).map(|fence| fence.id.clone());
            let file_change = previous != actual; // 布局项目的映射始终使用同一身份。
            if file_change && !active.contains(previous.as_str())
            {
                retired.insert(previous.clone());
            }
            if !active.contains(actual.as_str())
            {
                if file_change
                {
                    restored.push(original.clone());
                }
                continue; // 已移动到桌面外或回收站，只清理确切旧引用，不添加回收站成员。
            }
            if current_owner.as_deref() != expected_owner && !(file_change && current_owner.is_none())
            {
                continue;
            } // 其他操作已改归属时保留现状。
            if let Some(fence) = desired_fence
                && !candidate.fences.iter().any(|current| current.id == fence.id)
            {
                continue;
            } // 不复活已经删除的围栏。
            for fence in &mut candidate.fences
            {
                fence.members.retain(|id| id != actual && (id != previous || active.contains(previous.as_str())));
            }
            if let Some(fence) = desired_fence
            {
                let index = fence.members.iter().position(|id| id == original).unwrap_or(fence.members.len());
                let target = candidate.fence_mut(&fence.id)?;
                let successor = fence.members.iter().skip(index + 1).find_map(|id| target.members.iter().position(|current| current == id)).unwrap_or(target.members.len()); // 后继不存在时追加，保留其他并发成员。
                target.members.insert(successor, actual.clone());
            }
            if let Some(item) = candidate.unclassified_items.iter_mut().find(|item| &item.stable_reference == actual)
                && let Some(wanted) = desired.unclassified_items.iter().find(|item| &item.stable_reference == original)
                && let Some(applied) = expected.unclassified_items.iter().find(|item| &item.stable_reference == original)
            {
                if file_change || (item.position == applied.position && item.monitor_id == applied.monitor_id)
                {
                    item.position = wanted.position;
                    item.monitor_id.clone_from(&wanted.monitor_id);
                    item.projection_position = None;
                    item.projection_monitor_id = None;
                }
            }
            restored.push(original.clone());
        }
        let clean = |snapshot: &mut WorkspaceSnapshot| {
            for fence in &mut snapshot.fences
            {
                fence.members.retain(|id| !retired.contains(id));
            }
            snapshot.unclassified_items.retain(|item| !retired.contains(&item.stable_reference));
            snapshot.normalize_interaction();
        }; // 只清理成功移出工作区的准确引用。
        clean(&mut candidate);
        if !restored.is_empty()
        {
            let redo = mappings.first().is_some_and(|mapping| mapping.3); // 一轮历史只恢复一个方向。
            let (desired, expected) = if redo { (&change.after, &change.before) } else { (&change.before, &change.after) };
            for item in &mut candidate.unclassified_items
            {
                if change.item_ids.contains(&item.stable_reference)
                {
                    continue;
                }
                if let Some(wanted) = desired.unclassified_items.iter().find(|old| old.stable_reference == item.stable_reference)
                    && let Some(applied) = expected.unclassified_items.iter().find(|old| old.stable_reference == item.stable_reference)
                    && item.position == applied.position
                    && item.monitor_id == applied.monitor_id
                {
                    item.position = wanted.position;
                    item.monitor_id.clone_from(&wanted.monitor_id);
                } // 只恢复这次插入实际挤开的项目，不覆盖后续独立移动。
            }
            loop
            {
                let conflicts = candidate.unclassified_items.iter().enumerate().filter(|(index, item)| candidate.unclassified_items.iter().enumerate().any(|(other, candidate)| other != *index && candidate.monitor_id == item.monitor_id && candidate.position == item.position)).filter_map(|(_, item)| original_positions.iter().find(|old| old.stable_reference == item.stable_reference && (old.position != item.position || old.monitor_id != item.monitor_id)).cloned()).collect::<Vec<_>>();
                if conflicts.is_empty()
                {
                    break;
                }
                for original in conflicts
                {
                    if let Some(item) = candidate.unclassified_items.iter_mut().find(|item| item.stable_reference == original.stable_reference)
                    {
                        item.position = original.position;
                        item.monitor_id = original.monitor_id;
                    }
                } // 部分文件未撤销时不能把其他项目恢复进仍被占用的位置。
            }
        }
        self.sort_fences(&mut candidate)?;
        candidate.revision = current.revision.max(candidate.revision).saturating_add(1);
        candidate.validate()?;
        let store = super::save_workspace_store(&self.persistence_path, &candidate, *imported).map_err(|error| WorkspaceError::persistence_failed(&error))?; // 原子保存后才发布状态。
        *current = candidate;
        self.schedule_auto_snapshot(store);
        Ok(restored)
    }

    /// 在磁盘传输前模拟完整成功集合，容量不足不启动文件操作。
    pub(crate) fn preflight_drop(&self, target: &DropTarget, existing: &[String], new_count: usize) -> Result<(), WorkspaceError>
    {
        let current = self.snapshot.lock().map_err(|_| WorkspaceError::state_unavailable())?;
        if current.revision != target.revision
        {
            return Err(WorkspaceError::invalid_command("投放视图已经失效"));
        }
        let mut candidate = current.clone(); // 仅模拟，不写入占位数据。
        let mut ids = existing.to_vec(); // 成功时要一起提交的完整集合。
        let monitor_id = target.fence_id.as_deref().map(|id| candidate.fence(id).map(|fence| fence.monitor_id.clone())).transpose()?.unwrap_or_else(|| target.monitor_id.clone()); // 围栏来源位置使用围栏持久显示器。
        let grid = candidate.monitor_grids.iter().find(|entry| entry.monitor_id == monitor_id).or_else(|| candidate.monitor_grids.iter().find(|entry| entry.monitor_id == target.monitor_id)).map(|entry| entry.grid).ok_or_else(|| WorkspaceError::invalid_command("投放显示器不存在"))?;
        let mut occupied = candidate.unclassified_items.iter().filter(|item| item.monitor_id == monitor_id).map(|item| (item.position.left.to_bits(), item.position.top.to_bits())).collect::<HashSet<_>>();
        for index in 0..new_count
        {
            let id = format!("__drop-reservation-{}-{index}", target.revision); // 只存在于此函数的模拟身份。
            if candidate.unclassified_items.iter().any(|item| item.stable_reference == id)
            {
                return Err(WorkspaceError::invalid_command("投放模拟身份冲突"));
            }
            let position = (0..grid_capacity(grid)?).map(|slot| grid_position(grid, slot)).find(|point| occupied.insert((point.left.to_bits(), point.top.to_bits()))).ok_or_else(|| WorkspaceError::invalid_command("投放目标空间不足"))?;
            candidate.unclassified_items.push(super::UnclassifiedItemSnapshot { stable_reference: id.clone(), monitor_id: monitor_id.clone(), projection_monitor_id: None, projection_position: None, position, z_index: 0 });
            ids.push(id);
        }
        candidate.apply_drop(target, &ids)?;
        Ok(())
    }

    /// 文件事实协调后一次保存准确归属，由 Shell 联合历史记录本次操作。
    pub(crate) fn commit_external_drop(&self, target: &DropTarget, item_ids: &[String], layout_only_ids: &[String]) -> Result<(WorkspaceSnapshot, DropLayoutChange), WorkspaceError>
    {
        let mut current = self.snapshot.lock().map_err(|_| WorkspaceError::state_unavailable())?; // 文件传输期间的后续操作必须保留。
        let imported = self.explorer_layout_imported.lock().map_err(|_| WorkspaceError::state_unavailable())?; // 共同保存的导入状态。
        let before = current.clone(); // 实际提交前状态，用于部分补偿。
        let mut candidate = before.clone(); // 保存前不改变内存。
        candidate.apply_drop(target, item_ids)?;
        self.sort_fences(&mut candidate)?;
        candidate.revision = current.revision.saturating_add(1);
        candidate.validate()?;
        let mut history = self.history.lock().map_err(|_| WorkspaceError::state_unavailable())?; // 保存前确认重做分支可清理。
        let store = super::save_workspace_store(&self.persistence_path, &candidate, *imported).map_err(|error| WorkspaceError::persistence_failed(&error))?; // 持久化后再提交联合历史结果。
        history.redo.clear();
        *current = candidate.clone();
        self.schedule_auto_snapshot(store);
        Ok((candidate.clone(), DropLayoutChange { before, after: candidate, layout_only_ids: layout_only_ids.to_vec(), item_ids: item_ids.to_vec() }))
    }
}

impl WorkspaceSnapshot
{
    /// 对单份空间投影应用投放意图，预检与实际提交使用同一规则。
    fn apply_drop(&mut self, target: &DropTarget, ids: &[String]) -> Result<(), WorkspaceError>
    {
        if ids.is_empty()
        {
            return Err(WorkspaceError::invalid_command("投放集合为空"));
        }
        let placements = if target.fence_id.is_some()
        {
            Vec::new()
        }
        else
        {
            self.grid_drop_placements_at(ids, &target.monitor_id, target.position, target.free, target.insertion.as_ref(), None)?
        };
        self.apply(super::WorkspaceCommand::CommitItemDrag { anchor_item_id: ids[0].clone(), item_ids: ids.to_vec(), placements, target_fence_id: target.fence_id.clone(), before_item_id: target.before_item_id.clone(), fence_scrolls: target.scrolls.clone() }, Some(&target.monitor_id))?;
        self.validate()
    }

    /// 沿用前端 resolveGridDrop 的最近槽位与组内几何规则，原生释放不等待页面答复。
    #[cfg(test)]
    pub(crate) fn grid_drop_placements(&self, item_ids: &[String], monitor_id: &str, desired: WorkspacePoint, free: bool) -> Result<Vec<WorkspaceItemPlacement>, WorkspaceError> { self.grid_drop_placements_at(item_ids, monitor_id, desired, free, None, None) }

    /// 组吸附与四向传播共用来源槽位，所有被挤开的项目在同一领域事务提交。
    pub(crate) fn grid_drop_placements_at(&self, item_ids: &[String], monitor_id: &str, desired: WorkspacePoint, free: bool, insertion: Option<&(String, String)>, source_positions: Option<&[WorkspacePoint]>) -> Result<Vec<WorkspaceItemPlacement>, WorkspaceError>
    {
        super::validate_point(desired)?;
        if item_ids.is_empty() || item_ids.iter().collect::<HashSet<_>>().len() != item_ids.len()
        {
            return Err(WorkspaceError::invalid_command("拖动项目集合无效"));
        }
        let members = self.fences.iter().flat_map(|fence| fence.members.iter()).collect::<HashSet<_>>(); // 围栏来源占位不阻挡桌面落点，提交时由领域修复占位。
        let positions = self.unclassified_items.iter().filter(|item| item_ids.contains(&item.stable_reference) || (!members.contains(&item.stable_reference) && item.projection_monitor_id.as_deref().unwrap_or(&item.monitor_id) == monitor_id)).map(|item| (&item.stable_reference, item.projection_position.unwrap_or(item.position))).collect::<Vec<_>>(); // 与前端目标屏投影相同的稳定输入顺序。
        let sources = item_ids.iter().map(|id| positions.iter().find(|(candidate, _)| *candidate == id).map(|(_, point)| *point).ok_or_else(|| WorkspaceError::invalid_command("拖动项目不存在"))).collect::<Result<Vec<_>, _>>()?; // 完整选择必须可解析。
        let sources = if let Some(rendered) = source_positions
        {
            if rendered.len() != item_ids.len()
            {
                return Err(WorkspaceError::invalid_command("拖动来源几何不完整"));
            }
            for point in rendered
            {
                if !point.left.is_finite() || !point.top.is_finite()
                {
                    return Err(WorkspaceError::invalid_command("拖动来源几何无效"));
                }
            }
            rendered.to_vec()
        }
        else
        {
            sources
        }; // 内部来源优先使用拖动开始时的真实排列，外部投放保留领域规则。
        if free
        {
            let left = sources.iter().map(|position| position.left - sources[0].left).fold(0.0, f64::min); // 组内最左偏移。
            let top = sources.iter().map(|position| position.top - sources[0].top).fold(0.0, f64::min); // 组内最上偏移。
            return Ok(item_ids.iter().zip(sources.iter()).map(|(id, position)| WorkspaceItemPlacement { item_id: id.clone(), position: WorkspacePoint { left: desired.left.max(-left) + position.left - sources[0].left, top: desired.top.max(-top) + position.top - sources[0].top } }).collect());
        }
        let grid = self.monitor_grids.iter().find(|entry| entry.monitor_id == monitor_id).map(|entry| entry.grid).unwrap_or(self.grid); // 当前目标显示器的有效网格。
        let capacity = grid_capacity(grid)?; // 复用领域网格上限校验。
        // 按原型相同的顺序为离网项目分配最近的未占槽位。
        let assign = |exclude_dragged: bool| {
            let mut used = HashSet::new(); // 已分配的列优先网格槽位。
            let mut slots = HashMap::new(); // 项目对应的稳定槽位。
            for (id, position) in &positions
            {
                if exclude_dragged && item_ids.contains(id)
                {
                    continue;
                }
                let nearest = (0..capacity).filter(|index| !used.contains(index)).min_by(|left, right| {
                    let left = grid_position(grid, *left); // 左候选的 DIP。
                    let right = grid_position(grid, *right); // 右候选的 DIP。
                    ((left.left - position.left).powi(2) + (left.top - position.top).powi(2)).total_cmp(&((right.left - position.left).powi(2) + (right.top - position.top).powi(2)))
                }); // 等距离时沿用较小槽位。
                if let Some(index) = nearest
                {
                    used.insert(index);
                    slots.insert(*id, index);
                }
            }
            slots
        };
        let occupied = assign(true).into_values().collect::<HashSet<_>>(); // 移除拖动集合后的占用。
        if capacity as usize - occupied.len() < item_ids.len()
        {
            return Err(WorkspaceError::invalid_command("目标区域空间不足"));
        }
        let assigned = assign(false); // 来源组的规范网格几何。
        if let Some((target, direction)) = insertion
            && !item_ids.contains(target)
        {
            let vertical = matches!(direction.as_str(), "up" | "down"); // 与原型 resolveGridInsertion 相同的传播轴。
            let before = matches!(direction.as_str(), "left" | "up");
            if !matches!(direction.as_str(), "left" | "right" | "up" | "down")
            {
                return Err(WorkspaceError::invalid_command("桌面插入方向无效"));
            }
            let others = assign(true);
            if let Some(target_slot) = others.get(target)
            {
                let index = *target_slot as usize + usize::from(!before);
                let mut slots = vec![None; capacity as usize]; // 移除拖动组后的有界占用。
                for (id, slot) in others
                {
                    slots[slot as usize] = Some(id);
                }
                let gap = if before { index.checked_sub(item_ids.len()) } else { Some(index) };
                let open_gap = gap.filter(|start| start + item_ids.len() <= slots.len() && (0..item_ids.len()).all(|offset| slots[start + offset].is_none()) && item_ids.iter().all(|id| assigned.get(id).is_none_or(|slot| (*slot as usize) < *start || *slot as usize >= start + item_ids.len())));
                if let Some(start) = open_gap
                {
                    for (offset, id) in item_ids.iter().enumerate()
                    {
                        slots[start + offset] = Some(id);
                    }
                    return Ok(item_ids
                        .iter()
                        .enumerate()
                        .map(|(offset, id)| {
                            let slot = (start + offset) as u32;
                            WorkspaceItemPlacement {
                                item_id: id.clone(),
                                position: if vertical
                                {
                                    grid_position(grid, slot)
                                }
                                else
                                {
                                    WorkspacePoint { left: grid.left + f64::from(slot % grid.columns) * grid.column_width, top: grid.top + f64::from(slot / grid.columns) * grid.row_height }
                                },
                            }
                        })
                        .collect());
                }
                let mut pending = std::collections::VecDeque::from(item_ids.iter().collect::<Vec<_>>()); // 插入组及被占项目依次传播。
                for slot in slots.iter_mut().skip(index)
                {
                    let Some(id) = pending.pop_front()
                    else
                    {
                        break;
                    };
                    if let Some(displaced) = slot.replace(id)
                    {
                        pending.push_back(displaced);
                    }
                }
                if !pending.is_empty()
                {
                    return Err(WorkspaceError::invalid_command("插入方向剩余空间不足"));
                }
                return Ok(slots
                    .into_iter()
                    .enumerate()
                    .filter_map(|(index, id)| {
                        id.map(|id| {
                            let slot = index as u32;
                            WorkspaceItemPlacement {
                                item_id: id.clone(),
                                position: if vertical
                                {
                                    grid_position(grid, slot)
                                }
                                else
                                {
                                    WorkspacePoint { left: grid.left + f64::from(slot % grid.columns) * grid.column_width, top: grid.top + f64::from(slot / grid.columns) * grid.row_height }
                                },
                            }
                        })
                    })
                    .collect());
            } // 等待文件时目标被移走，按当前空位落位，不猜测新插入身份。
        }
        let source_slot = |index: usize| assigned.get(&item_ids[index]).map(|slot| ((slot / grid.rows) as i64, (slot % grid.rows) as i64)).unwrap_or((((sources[index].left - grid.left) / grid.column_width).round() as i64, ((sources[index].top - grid.top) / grid.row_height).round() as i64)); // 源超出目标容量时保留其相对网格偏移。
        let anchor = source_slot(0); // 第一个项目是拖动锚点。
        let offsets = (0..item_ids.len())
            .map(|index| {
                if source_positions.is_some()
                {
                    return (((sources[index].left - sources[0].left) / grid.column_width).round() as i64, ((sources[index].top - sources[0].top) / grid.row_height).round() as i64);
                }
                let slot = source_slot(index); // 没有渲染来源时沿用规范网格。
                (slot.0 - anchor.0, slot.1 - anchor.1)
            })
            .collect::<Vec<_>>(); // 组内相对行列。
        let mut candidates = (0..capacity).collect::<Vec<_>>(); // 最近释放点优先，距离相同保持列优先顺序。
        candidates.sort_by(|left, right| {
            let first = grid_position(grid, *left); // 首个候选。
            let second = grid_position(grid, *right); // 第二个候选。
            ((first.left - desired.left).powi(2) + (first.top - desired.top).powi(2)).total_cmp(&((second.left - desired.left).powi(2) + (second.top - desired.top).powi(2))).then(left.cmp(right))
        });
        for candidate in &candidates
        {
            let slots = offsets
                .iter()
                .map(|(column, row)| {
                    let column = i64::from(candidate / grid.rows) + column; // 候选目标列。
                    let row = i64::from(candidate % grid.rows) + row; // 候选目标行。
                    (column >= 0 && column < i64::from(grid.columns) && row >= 0 && row < i64::from(grid.rows)).then_some((column * i64::from(grid.rows) + row) as u32)
                })
                .collect::<Option<Vec<_>>>(); // 任意成员越界则尝试下一个完整组锚点。
            if let Some(slots) = slots.filter(|slots| slots.iter().collect::<HashSet<_>>().len() == slots.len() && slots.iter().all(|slot| !occupied.contains(slot)))
            {
                return Ok(item_ids.iter().zip(slots).map(|(id, slot)| WorkspaceItemPlacement { item_id: id.clone(), position: grid_position(grid, slot) }).collect());
            }
        }
        let slots = (0..capacity).map(|offset| (candidates[0] + offset) % capacity).filter(|slot| !occupied.contains(slot)).take(item_ids.len()); // 组形状无法保留时按原型循环填充空槽。
        Ok(item_ids.iter().zip(slots).map(|(id, slot)| WorkspaceItemPlacement { item_id: id.clone(), position: grid_position(grid, slot) }).collect())
    }
}

#[cfg(test)]
mod tests
{
    use super::*;

    /// 原生内部投放按围栏可见排列落位，不恢复分散的旧桌面坐标。
    #[test]
    fn internal_drop_uses_rendered_group_geometry()
    {
        let mut snapshot = WorkspaceSnapshot::default(); // 最小目标工作区。
        snapshot.grid = super::super::WorkspaceGrid { left: 0.0, top: 0.0, column_width: 80.0, row_height: 100.0, rows: 10, columns: 10 };
        snapshot.unclassified_items = [("a", 0.0, 0.0), ("b", 640.0, 800.0)].into_iter().map(|(id, left, top)| super::super::UnclassifiedItemSnapshot { stable_reference: id.into(), monitor_id: "screen".into(), projection_monitor_id: None, projection_position: None, position: WorkspacePoint { left, top }, z_index: 0 }).collect();
        let ids = ["a".into(), "b".into()]; // 原子拖动集合。
        let rendered = [WorkspacePoint { left: -10.0, top: -10.0 }, WorkspacePoint { left: 64.0, top: -10.0 }]; // 原生 Enter 缓存的围栏排列。
        let result = snapshot.grid_drop_placements_at(&ids, "screen", WorkspacePoint { left: 160.0, top: 300.0 }, false, None, Some(&rendered)).unwrap(); // OLE 正式落位入口。
        assert_eq!(result[0].position, WorkspacePoint { left: 160.0, top: 300.0 });
        assert_eq!(result[1].position, WorkspacePoint { left: 240.0, top: 300.0 });
        assert!(snapshot.grid_drop_placements_at(&ids, "screen", WorkspacePoint { left: 0.0, top: 0.0 }, false, None, Some(&rendered[..1])).is_err());
    }

    /// 混合结果只消费指定身份；释放后文件传输仍使用固定目标，联合恢复不吞掉并发项目。
    #[test]
    fn released_drop_keeps_shared_target_and_restores_only_confirmed_items()
    {
        use crate::contracts::{DesktopHostMonitor, DesktopItem, DesktopItemAvailability, DesktopItemSnapshot, WorkspaceCommand, WorkspaceRect};
        {
            let root = std::env::temp_dir().join(format!("desktop-tools-drop-{}-{}", std::process::id(), crate::tools::time::next_timeline_timestamp())); // 所有写入仅位于本例独占目录。
            std::fs::create_dir(&root).unwrap();
            let path = root.join("workspace.json");
            let monitors = [DesktopHostMonitor { id: "screen".into(), name: "screen".into(), primary: true, x: 0, y: 0, width: 1200, height: 800, work_x: 0, work_y: 0, work_width: 1200, work_height: 800, scale_factor: 1.0, dpi: 96 }];
            let mut state = super::super::WorkspaceState::load(path.clone()).unwrap();
            state.reconcile_host_monitors(&monitors, &["existing".into()], &[], &[]).unwrap();
            let created = state.apply(WorkspaceCommand::CreateFence { title: " ".into(), bounds: WorkspaceRect { left: 40.0, top: 40.0, width: 320.0, height: 240.0 } }).unwrap();
            let fence = created.fences[0].id.clone();
            let target = DropTarget { monitor_id: "screen".into(), revision: created.revision, fence_id: Some(fence.clone()), portal_id: None, portal_path: None, before_item_id: None, position: WorkspacePoint { left: 0.0, top: 0.0 }, free: false, insertion: None, scrolls: Vec::new() };
            let disk = std::fs::read(&path).unwrap();
            state.preflight_drop(&target, &["existing".into()], 1).unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), disk);
            assert!(state.preflight_drop(&target, &["existing".into()], 1000).is_err());
            let ids = ["existing".into(), "new".into(), "concurrent".into()]; // 新文件成功和其他程序并发新增使用不同准确身份。
            state.reconcile_host_monitors(&monitors, &ids, &[], &[]).unwrap();
            assert!(state.preflight_drop(&target, &["existing".into()], 1).is_err()); // 释放前布局版本变化必须拒绝。
            let (committed, change) = state.commit_external_drop(&target, &["existing".into(), "new".into()], &["existing".into()]).unwrap(); // 释放后完成可以使用固定共享目标。
            assert_eq!(committed.fences[0].members, ["existing", "new"]);
            let files = DesktopItemSnapshot { revision: 1, items: ["existing", "concurrent"].into_iter().map(|id| DesktopItem { stable_reference: id.into(), display_name: id.into(), system_image: None, availability: DesktopItemAvailability::Available }).collect() };
            let mappings = [("existing".into(), "existing".into(), "existing".into(), false), ("new".into(), "new".into(), "recycle-new".into(), false)];
            state.persistence_path = path.join("blocked.json");
            assert!(state.restore_drop_layout(&change, &mappings, &files).is_err());
            assert_eq!(state.snapshot().unwrap(), committed); // 保存失败不能改变归属，也不要求重做已经完成的文件补偿。
            state.persistence_path = path.clone();
            assert_eq!(state.restore_drop_layout(&change, &mappings, &files).unwrap().len(), 2);
            let undone = state.snapshot().unwrap();
            assert!(undone.fences[0].members.is_empty());
            assert!(undone.unclassified_items.iter().any(|item| item.stable_reference == "concurrent"));
            assert!(!undone.unclassified_items.iter().any(|item| item.stable_reference == "new"));
            drop(state);
            std::fs::remove_dir_all(root).unwrap();
        }
    }

    /// 与前端同一 L 形组实例核对原生落位，并验证自由定位和输入拒绝。
    #[test]
    fn native_grid_drop_preserves_group_geometry()
    {
        let mut snapshot = WorkspaceSnapshot::default(); // 测试只使用布局，不访问文件系统。
        snapshot.grid = super::super::WorkspaceGrid { left: 0.0, top: 0.0, column_width: 80.0, row_height: 100.0, rows: 3, columns: 3 };
        snapshot.unclassified_items = [("anchor", 0.0, 0.0), ("right", 80.0, 0.0), ("down", 0.0, 100.0)].into_iter().map(|(id, left, top)| super::super::UnclassifiedItemSnapshot { stable_reference: id.into(), monitor_id: "screen".into(), projection_monitor_id: None, projection_position: None, position: WorkspacePoint { left, top }, z_index: 0 }).collect();
        let ids = vec!["anchor".into(), "right".into(), "down".into()]; // 前端回归使用的相同锚点顺序。
        let desired = WorkspacePoint { left: 80.0, top: 100.0 }; // 目标屏释放点。
        let result = snapshot.grid_drop_placements(&ids, "screen", desired, false).unwrap(); // 应与 resolveGridDrop 保持一致。
        assert_eq!(result.iter().map(|item| item.position).collect::<Vec<_>>(), [WorkspacePoint { left: 80.0, top: 100.0 }, WorkspacePoint { left: 160.0, top: 100.0 }, WorkspacePoint { left: 80.0, top: 200.0 }]);
        let free = snapshot.grid_drop_placements(&ids, "screen", WorkspacePoint { left: 13.0, top: 27.0 }, true).unwrap(); // Alt 不吸附但保留组内相对位置。
        assert_eq!(free[1].position, WorkspacePoint { left: 93.0, top: 27.0 });
        assert!(snapshot.grid_drop_placements(&[], "screen", desired, false).is_err());
        assert!(snapshot.grid_drop_placements(&["anchor".into(), "anchor".into()], "screen", desired, false).is_err());
        assert!(snapshot.grid_drop_placements(&["missing".into()], "screen", desired, false).is_err());
        assert!(snapshot.grid_drop_placements(&ids, "screen", WorkspacePoint { left: f64::NAN, top: 0.0 }, false).is_err());
        snapshot.grid.columns = 1;
        snapshot.grid.rows = 2;
        assert!(snapshot.grid_drop_placements(&ids, "screen", desired, false).is_err());
    }

    /// 离网持久位置按稳定槽位占用，落位不能改变其他项目。
    #[test]
    fn native_grid_drop_keeps_unrelated_positions()
    {
        let mut snapshot = WorkspaceSnapshot::default(); // 对应前端离网来源回归。
        snapshot.grid = super::super::WorkspaceGrid { left: 0.0, top: 0.0, column_width: 78.0, row_height: 98.0, rows: 7, columns: 2 };
        snapshot.unclassified_items = [("dragged", 0.0, 0.0), ("later", 0.0, 196.0), ("legacy", 20.0, 396.0)].into_iter().map(|(id, left, top)| super::super::UnclassifiedItemSnapshot { stable_reference: id.into(), monitor_id: "screen".into(), projection_monitor_id: None, projection_position: None, position: WorkspacePoint { left, top }, z_index: 0 }).collect();
        let result = snapshot.grid_drop_placements(&["dragged".into()], "screen", WorkspacePoint { left: 0.0, top: 98.0 }, false).unwrap(); // 只返回实际拖动项目的提交位置。
        assert_eq!(result, [WorkspaceItemPlacement { item_id: "dragged".into(), position: WorkspacePoint { left: 0.0, top: 98.0 } }]);
        assert_eq!(snapshot.unclassified_items[1].position.top, 196.0);
    }

    /// 使用前端同一传播实例核对原生组插入，且 Alt 不执行挤位。
    #[test]
    fn native_insertion_matches_frontend_group_propagation()
    {
        let mut snapshot = WorkspaceSnapshot::default();
        snapshot.grid = super::super::WorkspaceGrid { left: 0.0, top: 0.0, column_width: 80.0, row_height: 100.0, rows: 3, columns: 2 };
        snapshot.unclassified_items = [("a", 0.0, 0.0), ("b", 0.0, 100.0), ("target", 0.0, 200.0), ("tail", 80.0, 0.0)].into_iter().map(|(id, left, top)| super::super::UnclassifiedItemSnapshot { stable_reference: id.into(), monitor_id: "screen".into(), projection_monitor_id: None, projection_position: None, position: WorkspacePoint { left, top }, z_index: 0 }).collect();
        let ids = ["a".into(), "b".into()];
        let insertion = ("target".into(), "up".into());
        let desired = WorkspacePoint { left: 0.0, top: 200.0 };
        let positions = snapshot.grid_drop_placements_at(&ids, "screen", desired, false, Some(&insertion), None).unwrap();
        assert_eq!(positions.iter().find(|item| item.item_id == "a").unwrap().position, WorkspacePoint { left: 0.0, top: 200.0 });
        assert_eq!(positions.iter().find(|item| item.item_id == "b").unwrap().position, WorkspacePoint { left: 80.0, top: 0.0 });
        assert_eq!(positions.iter().find(|item| item.item_id == "target").unwrap().position, WorkspacePoint { left: 80.0, top: 100.0 });
        assert_eq!(snapshot.grid_drop_placements_at(&ids, "screen", desired, true, Some(&insertion), None).unwrap().len(), 2);
        assert!(snapshot.grid_drop_placements_at(&ids, "screen", desired, false, Some(&("target".into(), "diagonal".into())), None).is_err());
    }
}
