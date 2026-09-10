use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::{WorkspaceError, WorkspaceSnapshot, grid_capacity, grid_position};

/// 在当前空间历史与全局内容历史之间选择最近的可执行单元。
pub(super) fn history_key(histories: &std::collections::HashMap<String, super::WorkspaceHistory>, current: &str, redo: bool) -> String
{
    [current, super::SHARED_HISTORY_KEY]
        .into_iter()
        .filter_map(|key| {
            let history = histories.get(key)?; // 当前候选历史。
            let entry = if redo { history.redo.last() } else { history.undo.last() }?; // 当前方向的最新单元。
            Some((key, entry.timeline_at_unix_micros))
        })
        .max_by_key(|(_, timestamp)| *timestamp)
        .map(|(key, _)| key)
        .unwrap_or(current)
        .to_string()
}

/// 更新共享权威及其他活动桌面的投影，调用方在保存成功后原子提交副本。
pub(super) fn publish(source: &WorkspaceSnapshot, desktops: &mut super::WorkspaceVirtualDesktops) -> Result<(), WorkspaceError>
{
    desktops.shared_fences = contents(source);
    for layout in desktops.layouts.values_mut()
    {
        project(source, layout)?;
    }
    Ok(())
}

/// 全局围栏身份与成员，独立于虚拟桌面的空间投影。
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SharedFence
{
    pub id: String,           // 全局稳定围栏身份。
    pub members: Vec<String>, // 全局成员与手动顺序。
}

/// 共享撤销恢复成员及原操作改动的空间字段，保留其他桌面后续的独立调整。
pub(super) fn restore(before: &WorkspaceSnapshot, after: &WorkspaceSnapshot, current: &WorkspaceSnapshot) -> Result<WorkspaceSnapshot, WorkspaceError>
{
    let mut desired = before.clone(); // 历史目标只保留仍存在的项目事实。
    let ids = current.unclassified_items.iter().map(|item| item.stable_reference.as_str()).collect::<HashSet<_>>(); // 当前有效来源集合。
    desired.unclassified_items.retain(|item| ids.contains(item.stable_reference.as_str()));
    for fence in &mut desired.fences
    {
        fence.members.retain(|id| ids.contains(id.as_str()));
    }
    let mut restored = current.clone(); // 空间状态以当前视图为基础。
    project(&desired, &mut restored)?;
    for fence in &mut restored.fences
    {
        let Some(previous) = before.fences.iter().find(|previous| previous.id == fence.id)
        else
        {
            continue;
        };
        let Some(applied) = after.fences.iter().find(|applied| applied.id == fence.id)
        else
        {
            continue;
        };
        if previous.bounds != applied.bounds && fence.bounds == applied.bounds
        {
            fence.bounds = previous.bounds;
        }
        if previous.monitor_id != applied.monitor_id && fence.monitor_id == applied.monitor_id
        {
            fence.monitor_id.clone_from(&previous.monitor_id);
        }
        if previous.z_index != applied.z_index && fence.z_index == applied.z_index
        {
            fence.z_index = previous.z_index;
        }
        if previous.sort_mode != applied.sort_mode && fence.sort_mode == applied.sort_mode
        {
            fence.sort_mode = previous.sort_mode;
        }
    }
    for item in &mut restored.unclassified_items
    {
        let Some(previous) = before.unclassified_items.iter().find(|previous| previous.stable_reference == item.stable_reference)
        else
        {
            continue;
        };
        let Some(applied) = after.unclassified_items.iter().find(|applied| applied.stable_reference == item.stable_reference)
        else
        {
            continue;
        };
        if previous.position != applied.position && item.position == applied.position
        {
            item.position = previous.position;
        }
        if previous.monitor_id != applied.monitor_id && item.monitor_id == applied.monitor_id
        {
            item.monitor_id.clone_from(&previous.monitor_id);
        }
        if previous.z_index != applied.z_index && item.z_index == applied.z_index
        {
            item.z_index = previous.z_index;
        }
    }
    restored.validate()?;
    Ok(restored)
}

/// 从已经提交的工作区投影提取唯一共享内容。
pub(super) fn contents(snapshot: &WorkspaceSnapshot) -> Vec<SharedFence> { snapshot.fences.iter().map(|fence| SharedFence { id: fence.id.clone(), members: fence.members.clone() }).collect() }

/// 投影共享内容，保留目标桌面的已有位置、尺寸、滚动与组件。
pub(super) fn project(source: &WorkspaceSnapshot, target: &mut WorkspaceSnapshot) -> Result<(), WorkspaceError>
{
    let previous = target.clone(); // 判断投影是否改变目标的可见状态。
    target.tile_appearances.clone_from(&source.tile_appearances);
    if target.monitor_grids.is_empty() && target.unclassified_items.is_empty()
    {
        target.grid = source.grid;
        target.monitor_grids.clone_from(&source.monitor_grids);
    } // 新桌面尚未得到宿主几何时使用已有空间网格，避免默认单格限制共享项目恢复。
    target.fences.retain(|fence| source.fences.iter().any(|shared| shared.id == fence.id));
    for fence in &source.fences
    {
        if let Some(existing) = target.fences.iter_mut().find(|existing| existing.id == fence.id)
        {
            existing.members.clone_from(&fence.members);
        }
        else
        {
            let mut added = fence.clone(); // 新桌面缺少表面时以来源表面初始化，后续各自调整。
            added.scroll_top = 0.0;
            target.fences.push(added);
        }
    }
    // 补齐成员的来源占位，避免恢复新视图时产生悬空成员或重复坐标。
    let mut ids = target.unclassified_items.iter().map(|item| item.stable_reference.clone()).collect::<HashSet<_>>(); // 已有来源引用。
    let mut occupied = target.unclassified_items.iter().map(|item| (item.monitor_id.clone(), item.position.left.to_bits(), item.position.top.to_bits())).collect::<HashSet<_>>(); // 已有来源占位。
    for item in &source.unclassified_items
    {
        if !ids.insert(item.stable_reference.clone())
        {
            continue;
        }
        let mut added = item.clone(); // 仅为目标缺少的项目生成初始位置。
        let grid = target.monitor_grids.iter().find(|entry| entry.monitor_id == item.monitor_id).map(|entry| entry.grid).unwrap_or(target.grid); // 目标桌面对应网格。
        let position = (0..grid_capacity(grid)?).map(|index| grid_position(grid, index)).find(|position| !occupied.contains(&(added.monitor_id.clone(), position.left.to_bits(), position.top.to_bits()))).ok_or_else(|| WorkspaceError::invalid_command("共享项目来源位置容量不足"))?; // 当前目标可用的来源占位。
        added.position = position;
        added.projection_position = None;
        added.projection_monitor_id = None;
        occupied.insert((added.monitor_id.clone(), position.left.to_bits(), position.top.to_bits()));
        target.unclassified_items.push(added);
    }
    super::appearance::project(target);
    target.normalize_interaction();
    if *target != previous
    {
        target.revision = previous.revision.saturating_add(1);
    }
    target.validate()
}

/// 磁盘空间快照不存成员；只接受这种结构并由全局内容显式重建投影。
pub(super) mod spatial
{
    use crate::contracts::WorkspaceSnapshot;
    use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error};

    /// 将投影序列化为空间信息，成员只写入共享表。
    pub fn serialize<S: Serializer>(snapshot: &WorkspaceSnapshot, serializer: S) -> Result<S::Ok, S::Error>
    {
        let mut value = serde_json::to_value(snapshot).map_err(serde::ser::Error::custom)?; // 可移除成员字段的空间载荷。
        value.as_object_mut().ok_or_else(|| serde::ser::Error::custom("工作区空间载荷无效"))?.remove("tileAppearances");
        for fence in value["fences"].as_array_mut().ok_or_else(|| serde::ser::Error::custom("围栏空间载荷无效"))?
        {
            fence.as_object_mut().ok_or_else(|| serde::ser::Error::custom("围栏空间载荷无效"))?.remove("members");
        }
        value.serialize(serializer)
    }

    /// 读取新空间结构，拒绝在逐桌面数据中保存成员。
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<WorkspaceSnapshot, D::Error>
    {
        let mut value = serde_json::Value::deserialize(deserializer)?; // 当前版本空间载荷。
        for fence in value["fences"].as_array_mut().ok_or_else(|| D::Error::custom("围栏空间载荷无效"))?
        {
            let fields = fence.as_object_mut().ok_or_else(|| D::Error::custom("围栏空间载荷无效"))?; // 围栏空间字段。
            if fields.contains_key("members")
            {
                return Err(D::Error::custom("围栏成员必须存储在共享内容中"));
            }
            fields.insert("members".to_string(), serde_json::json!([]));
        }
        serde_json::from_value(value).map_err(D::Error::custom)
    }
}
