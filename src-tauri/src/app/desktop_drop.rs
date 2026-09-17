use tauri::{AppHandle, Emitter, Manager};

use crate::contracts::{HitRegion, ShellOperationResult, WorkspaceFenceScroll, WorkspacePoint, WorkspaceSnapshot};
use crate::workspace::{DropTarget, WorkspaceState};

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
    target: DropTarget,              // 共享归属与原视图空间意图。
    snapshot: WorkspaceSnapshot,     // 预检时的布局，用于隔离同期新增项目。
    sources: Vec<String>,            // 原始顺序。
    existing: Vec<String>,           // 只改变布局的已知项目。
    pub transfers: Vec<String>,      // 平台需要执行真实传输的完整来源。
    pub portal_root: Option<String>, // 门户目标绑定根目录。
}

/// 向原生会话提供当前布局事实，应用状态访问集中于应用层。
pub(crate) fn snapshot(app: &AppHandle) -> Result<WorkspaceSnapshot, String> { app.state::<WorkspaceState>().snapshot().map_err(|error| error.message) }

/// 从已确认几何解析共享目标，组件与失效围栏不得穿透到背景。
fn target_at(input: DropInput, snapshot: &WorkspaceSnapshot, dragged: &[String]) -> Result<Option<DropTarget>, String>
{
    let fence_id = drop_fence_at(&input.pointer, &input.regions, snapshot); // 共享围栏身份。
    let position = WorkspacePoint { left: input.pointer.left.max(0.0), top: input.pointer.top.max(0.0) }; // 保存位置不允许越过工作区原点。
    let region = input.regions.iter().filter(|region| position.left >= region.left && position.left < region.left + region.width && position.top >= region.top && position.top < region.top + region.height).max_by_key(|region| region.priority); // 最高语义接收者。
    let portal = region.filter(|region| region.kind == "component").and_then(|region| snapshot.folder_portals.iter().find(|portal| portal.id == region.id));
    let portal_path = region.and_then(|region| region.target_path.clone());
    if fence_id.is_none() && portal.is_none() && !region.is_some_and(|region| region.kind == "workspace-background" || (region.kind == "desktop-item" && snapshot.unclassified_items.iter().any(|item| item.stable_reference == region.id)))
    {
        return Ok(None);
    }
    let before_item_id = fence_id.as_ref().and_then(|id| snapshot.fences.iter().find(|fence| &fence.id == id)).and_then(|fence| drop_before_item_at(&input.pointer, &input.regions, fence, dragged)); // 手动排序的稳定后继。
    let insertion = if fence_id.is_none()
    {
        drop_grid_insertion_at(&input.pointer, &input.regions, dragged)
    }
    else
    {
        None
    }; // 桌面插入与围栏插入互斥。
    Ok(Some(DropTarget { monitor_id: input.monitor_id, revision: snapshot.revision, fence_id, portal_id: portal.map(|portal| portal.id.clone()), portal_path, before_item_id, position, free: false, insertion, scrolls: input.scrolls }))
}

/// 提交内部布局投放，不请求正被 DoDragDrop 占用的 Shell STA。
pub(crate) fn commit_internal(app: &AppHandle, snapshot: WorkspaceSnapshot, input: DropInput, item_ids: Vec<String>, pointer_offset: &WorkspacePoint, free: bool, source_positions: &[WorkspacePoint]) -> Result<bool, String>
{
    let Some(anchor_item_id) = item_ids.first().cloned()
    else
    {
        return Ok(false);
    }; // 空来源不能构造布局事务。
    let Some(mut target) = target_at(input, &snapshot, &item_ids)?
    else
    {
        return Ok(false);
    }; // 遮挡或失效目标拒绝投放。
    target.position = WorkspacePoint { left: (target.position.left - pointer_offset.left).max(0.0), top: (target.position.top - pointer_offset.top).max(0.0) };
    let placements = if target.fence_id.is_some()
    {
        Vec::new()
    }
    else
    {
        snapshot.grid_drop_placements_at(&item_ids, &target.monitor_id, target.position, free, target.insertion.as_ref(), Some(source_positions)).map_err(|error| error.message)?
    }; // 完整组按原有自由或网格规则落位。
    let next = app.state::<WorkspaceState>().apply_on_monitor_at_revision(Some(&target.monitor_id), crate::contracts::WorkspaceCommand::CommitItemDrag { anchor_item_id, item_ids, placements, target_fence_id: target.fence_id, before_item_id: target.before_item_id, fence_scrolls: target.scrolls }, Some(snapshot.revision)).map_err(|error| error.message)?; // 持久化成功才报告内部提交。
    let _ = app.state::<crate::shell::DesktopItemState>().clear_redo_history();
    let _ = app.emit(crate::contracts::WORKSPACE_SNAPSHOT_CHANGED_EVENT, &next);
    Ok(true)
}

/// 在任何文件传输前拆分准确来源并完成布局容量预检。
pub(crate) fn prepare_external(app: &AppHandle, snapshot: WorkspaceSnapshot, input: DropInput, sources: Vec<String>, copy_all: bool) -> Result<Option<ExternalDrop>, String>
{
    let layout_only = sources.iter().filter(|id| !copy_all && snapshot.contains_item(id)).cloned().collect::<Vec<_>>(); // 非门户目标可直接调整已有桌面项目。
    let Some(target) = target_at(input, &snapshot, &layout_only)?
    else
    {
        return Ok(None);
    }; // 无效目标不启动文件操作。
    let portal_root = if let Some(id) = &target.portal_id
    {
        let portal = snapshot.folder_portals.iter().find(|portal| &portal.id == id).ok_or("门户目标不存在")?;
        let path = target.portal_path.as_deref().ok_or("门户当前目录未确认")?;
        let (_, current) = crate::commands::folder_portal::checked_directory(&portal.root_path, path)?;
        Some(current.to_string_lossy().into_owned())
    }
    else
    {
        None
    }; // 页面目录只作为候选，释放前再次校验绑定范围。
    let existing = if portal_root.is_some() { Vec::new() } else { layout_only }; // 门户投放总是实际 Shell 文件操作。
    let transfers = sources.iter().filter(|id| !existing.contains(id)).cloned().collect::<Vec<_>>();
    if portal_root.is_none()
    {
        app.state::<WorkspaceState>().preflight_drop(&target, &existing, transfers.len()).map_err(|error| error.message)?;
    }
    Ok(Some(ExternalDrop { target, snapshot, sources, existing, transfers, portal_root }))
}

/// 消费文件事实，协调环境、共享归属和同一次撤销记录；不重新执行文件传输。
pub(crate) fn complete_external(app: &AppHandle, plan: ExternalDrop, mut completed: ShellOperationResult) -> (bool, bool, Option<ShellOperationResult>)
{
    let ExternalDrop { target: drop_target, snapshot, sources, existing, transfers, portal_root } = plan; // 释放时冻结的业务输入。
    let workspace = app.state::<WorkspaceState>(); // 当前权威工作区。
    let state = app.state::<crate::shell::DesktopItemState>(); // Shell 快照与补偿记录入口。
    let mut committed = false; // 文件成功不等于布局提交成功。
    let succeeded = completed.items.iter().any(|item| item.succeeded); // 只有文件实际成功才返回文件传输效果。
    if portal_root.is_some()
    {
        return (succeeded, false, state.record_drop_transfer(completed, None).ok());
    } // 门户拖放只改变真实目录，不改变桌面布局。
    let mut layout_change = None;
    let disposition = (|| -> Result<(), String> {
        let items = if transfers.is_empty() { state.snapshot() } else { state.refresh() }.map_err(|error| error.message)?;
        if !transfers.is_empty() && (drop_target.fence_id.is_none() || completed.aborted)
        {
            let runtime = app.state::<std::sync::Arc<crate::host::DesktopHostRuntime>>();
            let monitors = runtime.snapshot().map_err(|error| error.message)?.hosts.into_iter().map(|host| host.monitor).collect::<Vec<_>>();
            let placement_monitor = drop_target.fence_id.as_ref().and_then(|id| snapshot.fences.iter().find(|fence| &fence.id == id)).map(|fence| &fence.monitor_id).unwrap_or(&drop_target.monitor_id);
            let assignments = completed.items.iter().filter(|item| transfers.contains(&item.source_reference)).filter_map(|item| item.result_reference.as_ref()).filter(|id| !snapshot.unclassified_items.iter().any(|item| &item.stable_reference == *id)).map(|id| (id.clone(), placement_monitor.clone())).collect::<Vec<_>>(); // 只分配此次准确结果，不收走并发创建。
            workspace.reconcile_environment(crate::workspace::WorkspaceEnvironment { new_item_positions: &[], transfer_assignments: true, monitors: &monitors, items: &items.items, assignments: &assignments, renames: &[] }).map_err(|error| error.message)?;
        }
        let mut ids = Vec::new(); // 保持来源相对顺序，合并结果身份只提交一次。
        if !completed.aborted
        {
            for source in &sources
            {
                let result = if existing.contains(source)
                {
                    Some(source.clone())
                }
                else
                {
                    completed.items.iter().find(|item| &item.source_reference == source && item.succeeded).and_then(|item| item.result_reference.clone()).filter(|id| items.items.iter().any(|item| &item.stable_reference == id))
                };
                if let Some(id) = result
                    && !ids.contains(&id)
                {
                    ids.push(id);
                }
            }
        } // 用户取消不提交已有归属，已完成文件保留在未归类。
        let final_snapshot = if ids.is_empty()
        {
            workspace.snapshot().map_err(|error| error.message)?
        }
        else
        {
            let (snapshot, change) = workspace.commit_external_drop(&drop_target, &ids, &existing, &items).map_err(|error| error.message)?;
            layout_change = Some(change);
            committed = true;
            snapshot
        };
        let _ = app.emit(crate::contracts::DESKTOP_ITEM_SNAPSHOT_CHANGED_EVENT, &items);
        let _ = app.emit(crate::contracts::WORKSPACE_SNAPSHOT_CHANGED_EVENT, &final_snapshot);
        Ok(())
    })();
    if let Err(error) = disposition
    {
        completed.message.push_str(&format!("；文件保持实际状态，布局未提交：{error}"));
        crate::log_error!("拖放：{}", completed.message);
        if let Ok(items) = state.snapshot()
        {
            let _ = app.emit(crate::contracts::DESKTOP_ITEM_SNAPSHOT_CHANGED_EVENT, &items);
        }
        if let Ok(snapshot) = workspace.snapshot()
        {
            let _ = app.emit(crate::contracts::WORKSPACE_SNAPSHOT_CHANGED_EVENT, &snapshot);
        }
    }

    let result = match state.record_drop_transfer(completed, layout_change)
    {
        Ok(result) =>
        {
            crate::log_info!("拖放：{}", result.message);
            Some(result)
        }
        Err(error) =>
        {
            crate::log_error!("拖放：记录结果失败：{}", error.message);
            None
        }
    }; // 保持文件结果与布局补偿在同一条历史中。
    (succeeded, committed, result)
}

/// 将已确认的工作区 DIP 指针映射到实际存在的共享围栏。
fn drop_fence_at(point: &WorkspacePoint, regions: &[HitRegion], snapshot: &crate::contracts::WorkspaceSnapshot) -> Option<String>
{
    let x = point.left; // 已确认的工作区 DIP 横坐标。
    let y = point.top; // 已确认的工作区 DIP 纵坐标。
    let region = regions.iter().filter(|region| x >= region.left && x < region.left + region.width && y >= region.top && y < region.top + region.height).max_by_key(|region| region.priority)?; // 与宿主一致选择顶层语义区域。
    if region.kind == "fence"
    {
        snapshot.fences.iter().find(|fence| fence.id == region.id).map(|fence| fence.id.clone())
    }
    else if region.kind == "desktop-item"
    {
        snapshot.fences.iter().find(|fence| fence.members.contains(&region.id)).map(|fence| fence.id.clone())
    }
    else
    {
        None
    }
}

/// 桌面项目按当前主导轴和中心死区规则确定四向插入。
fn drop_grid_insertion_at(point: &WorkspacePoint, regions: &[HitRegion], dragged: &[String]) -> Option<(String, String)>
{
    let x = point.left; // 已确认的工作区 DIP 横坐标。
    let y = point.top; // 已确认的工作区 DIP 纵坐标。
    let region = regions.iter().filter(|region| x >= region.left && x < region.left + region.width && y >= region.top && y < region.top + region.height).max_by_key(|region| region.priority)?;
    if region.kind != "desktop-item" || dragged.contains(&region.id)
    {
        return None;
    }
    let dx = x - region.left - region.width / 2.0;
    let dy = y - region.top - region.height / 2.0;
    let direction = if dx.abs() >= dy.abs()
    {
        if dx.abs() <= region.width * 0.25
        {
            return None;
        }
        if dx < 0.0 { "left" } else { "right" }
    }
    else
    {
        if dy.abs() <= region.height * 0.25
        {
            return None;
        }
        if dy < 0.0 { "up" } else { "down" }
    };
    Some((region.id.clone(), direction.into()))
}

/// 按围栏布局轴解析当前可见成员的前后插入点。
fn drop_before_item_at(point: &WorkspacePoint, regions: &[HitRegion], fence: &crate::contracts::FenceSnapshot, dragged: &[String]) -> Option<String>
{
    if fence.sort_mode != crate::contracts::FenceSortMode::None
    {
        return None;
    }
    let x = point.left; // 已确认的工作区 DIP 横坐标。
    let y = point.top; // 已确认的工作区 DIP 纵坐标。
    let region = regions.iter().filter(|region| x >= region.left && x < region.left + region.width && y >= region.top && y < region.top + region.height).max_by_key(|region| region.priority)?; // 只使用实际顶层可见目标。
    if region.kind != "desktop-item" || dragged.contains(&region.id)
    {
        return None;
    }
    let members = fence.members.iter().filter(|id| !dragged.contains(id)).collect::<Vec<_>>(); // 插入次序先移除完整拖动组。
    let index = members.iter().position(|id| **id == region.id)?; // 目标必须仍属于当前围栏。
    let (distance, span) = if fence.layout_mode == crate::contracts::FenceLayoutMode::List
    {
        (y - region.top - region.height / 2.0, region.height)
    }
    else
    {
        (x - region.left - region.width / 2.0, region.width)
    }; // 列表使用纵轴，平铺使用横轴。
    if distance.abs() <= span * 0.25
    {
        return None;
    }
    members.get(index + usize::from(distance > 0.0)).map(|id| (*id).clone())
}

#[cfg(test)]
#[path = "desktop_drop.test.rs"]
mod tests;
