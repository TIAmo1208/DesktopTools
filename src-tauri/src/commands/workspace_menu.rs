use crate::contracts::{DesktopHostMonitor, DesktopItemError, FENCE_MIN_HEIGHT, FENCE_MIN_WIDTH, WorkspaceCommand, WorkspaceContextMenuTarget, WorkspacePoint, WorkspaceRect, WorkspaceSnapshot};
use crate::platform::windows::{NativeMenuCommand, NativeMenuTarget};

/// 菜单返回的产品动作，原生 Shell 动词仍由 Windows 执行。
pub(super) enum WorkspaceMenuAction
{
    /// 取消或保持布局。
    None,
    /// 重新观察桌面。
    Refresh,
    /// 提交单次布局事务。
    Layout(WorkspaceCommand),
}

/// 集中菜单快照、可用项、坐标转换与选择结果解释。
pub(super) fn choose_workspace_menu(snapshot: &WorkspaceSnapshot, monitor: &DesktopHostMonitor, target: WorkspaceContextMenuTarget, client_x: f64, client_y: f64, select: impl FnOnce(NativeMenuTarget, i32, i32, Vec<NativeMenuCommand>) -> Result<Option<u32>, DesktopItemError>) -> Result<WorkspaceMenuAction, DesktopItemError>
{
    /// 第一个动态移动到围栏命令编号。
    const MOVE_TO_FENCE_FIRST: u32 = 0x7000;
    /// 将完整选择集合移出围栏命令编号。
    const REMOVE_FROM_FENCE: u32 = 0x7800;
    /// 切换围栏折叠状态命令编号。
    const TOGGLE_FENCE: u32 = 0x7801;
    /// 切换围栏排列方式命令编号。
    const TOGGLE_FENCE_LAYOUT: u32 = 0x7802;
    /// 解散围栏命令编号。
    const DISSOLVE_FENCE: u32 = 0x7803;
    /// 在菜单锚点创建围栏命令编号。
    const CREATE_FENCE: u32 = 0x7804;
    const CREATE_FOLDER_PORTAL: u32 = 0x7808;
    /// 当前工作区已经持续启用的编辑布局命令编号。
    const EDIT_LAYOUT: u32 = 0x7805;
    /// 重新枚举 Shell 项目并协调工作区命令编号。
    const REFRESH_ITEMS: u32 = 0x7806;
    /// 等待正式设置表面交付后启用的设置命令编号。
    const OPEN_SETTINGS: u32 = 0x7807;

    if !client_x.is_finite() || !client_y.is_finite()
    {
        return Err(DesktopItemError::invalid_selection("菜单锚点坐标无效"));
    }
    // 菜单锚点的屏幕物理横坐标。
    let screen_x = monitor.work_x + (client_x * monitor.scale_factor).round() as i32;
    // 菜单锚点的屏幕物理纵坐标。
    let screen_y = monitor.work_y + (client_y * monitor.scale_factor).round() as i32;
    // 当前菜单目标对应的原生 Shell 目标、产品命令和项目选择。
    let (native_target, product_commands, item_ids, fence_id) = match target
    {
        WorkspaceContextMenuTarget::Items { item_ids } =>
        {
            // 已通过当前 Shell 快照校验的多选稳定引用。
            let references = item_ids.clone();
            if item_ids.is_empty()
            {
                return Err(DesktopItemError::invalid_selection("菜单选择为空"));
            }
            // 当前选择集合全部成员共同支持的产品命令。
            let mut commands = snapshot.fences.iter().take((REMOVE_FROM_FENCE - MOVE_TO_FENCE_FIRST) as usize).enumerate().map(|(index, fence)| NativeMenuCommand { id: MOVE_TO_FENCE_FIRST.saturating_add(index as u32), label: format!("移动到围栏：{}", fence.title), enabled: item_ids.iter().any(|item_id| !fence.members.contains(item_id)) }).collect::<Vec<_>>();
            // 当前完整选择集合是否都具有移出围栏能力。
            let can_remove = item_ids.iter().all(|item_id| snapshot.fences.iter().any(|fence| fence.members.contains(item_id)));
            commands.push(NativeMenuCommand { id: REMOVE_FROM_FENCE, label: "移出围栏".to_string(), enabled: can_remove });
            commands.push(NativeMenuCommand { id: EDIT_LAYOUT, label: "编辑布局".to_string(), enabled: true });
            (NativeMenuTarget::Items(references), commands, item_ids, None)
        }
        WorkspaceContextMenuTarget::Fence { fence_id } =>
        {
            // 当前菜单目标围栏权威快照。
            let fence = snapshot.fences.iter().find(|fence| fence.id == fence_id).ok_or_else(|| DesktopItemError::invalid_selection("围栏不存在"))?;
            (
                NativeMenuTarget::ProductOnly,
                vec![
                    NativeMenuCommand { id: TOGGLE_FENCE, label: if fence.collapsed { "展开围栏" } else { "折叠围栏" }.to_string(), enabled: true },
                    NativeMenuCommand {
                        id: TOGGLE_FENCE_LAYOUT,
                        label: if fence.layout_mode == crate::contracts::FenceLayoutMode::Tile
                        {
                            "切换为列表"
                        }
                        else
                        {
                            "切换为平铺"
                        }
                        .to_string(),
                        enabled: !fence.collapsed,
                    },
                    NativeMenuCommand { id: DISSOLVE_FENCE, label: "解散围栏".to_string(), enabled: true },
                ],
                Vec::new(),
                Some(fence_id),
            )
        }
        WorkspaceContextMenuTarget::Background => (NativeMenuTarget::Background, vec![NativeMenuCommand { id: CREATE_FENCE, label: "新建围栏".to_string(), enabled: true }, NativeMenuCommand { id: CREATE_FOLDER_PORTAL, label: "新建文件夹门户".to_string(), enabled: true }, NativeMenuCommand { id: EDIT_LAYOUT, label: "编辑布局".to_string(), enabled: true }, NativeMenuCommand { id: REFRESH_ITEMS, label: "刷新桌面项目".to_string(), enabled: true }, NativeMenuCommand { id: OPEN_SETTINGS, label: "打开设置".to_string(), enabled: false }], Vec::new(), None),
    };
    // 原生菜单关闭后返回的可选产品命令编号。
    let enabled = product_commands.iter().filter(|command| command.enabled).map(|command| command.id).collect::<Vec<_>>(); // 本次菜单允许的动作。
    let selected = select(native_target, screen_x, screen_y, product_commands)?;
    let Some(selected) = selected
    else
    {
        return Ok(WorkspaceMenuAction::None);
    };
    if !enabled.contains(&selected)
    {
        return Err(DesktopItemError::invalid_selection("菜单返回了不可用的产品动作"));
    }
    if selected == EDIT_LAYOUT
    {
        crate::log_info!("工作区：编辑布局命令已激活");
        return Ok(WorkspaceMenuAction::None);
    }
    if selected == REFRESH_ITEMS
    {
        return Ok(WorkspaceMenuAction::Refresh);
    }

    // 产品菜单动作提交后的单一布局事务。
    let command = if (MOVE_TO_FENCE_FIRST..REMOVE_FROM_FENCE).contains(&selected)
    {
        // 动态移动命令编号对应的目标围栏。
        let target_fence = snapshot.fences.get((selected - MOVE_TO_FENCE_FIRST) as usize).ok_or_else(|| DesktopItemError::invalid_selection("目标围栏不存在"))?;
        WorkspaceCommand::CommitItemDrag { anchor_item_id: item_ids[0].clone(), item_ids, placements: Vec::new(), target_fence_id: Some(target_fence.id.clone()), before_item_id: None, fence_scrolls: Vec::new() }
    }
    else
    {
        match selected
        {
            REMOVE_FROM_FENCE => WorkspaceCommand::CommitItemDrag { anchor_item_id: item_ids[0].clone(), item_ids: item_ids.clone(), placements: snapshot.grid_drop_placements_at(&item_ids, &monitor.id, WorkspacePoint { left: client_x, top: client_y }, false, None, None).map_err(|error| DesktopItemError::workspace_reconciliation_failed(&error))?, target_fence_id: None, before_item_id: None, fence_scrolls: Vec::new() },
            TOGGLE_FENCE =>
            {
                let fence_id = fence_id.expect("围栏菜单命令包含目标围栏");
                let fence = snapshot.fences.iter().find(|fence| fence.id == fence_id).expect("围栏菜单目标已完成校验");
                WorkspaceCommand::SetFenceCollapsed { fence_id, collapsed: !fence.collapsed }
            }
            TOGGLE_FENCE_LAYOUT =>
            {
                let fence_id = fence_id.expect("围栏菜单命令包含目标围栏");
                let fence = snapshot.fences.iter().find(|fence| fence.id == fence_id).expect("围栏菜单目标已完成校验");
                WorkspaceCommand::SetFenceLayout {
                    fence_id,
                    layout_mode: if fence.layout_mode == crate::contracts::FenceLayoutMode::Tile
                    {
                        crate::contracts::FenceLayoutMode::List
                    }
                    else
                    {
                        crate::contracts::FenceLayoutMode::Tile
                    },
                }
            }
            DISSOLVE_FENCE => WorkspaceCommand::DissolveFence { fence_id: fence_id.expect("围栏菜单命令包含目标围栏") },
            CREATE_FENCE =>
            {
                // 当前 CSS 工作区可容纳的新围栏初始边界。
                let viewport_width = monitor.work_width as f64 / monitor.scale_factor;
                // 当前 CSS 工作区可容纳的新围栏初始高度。
                let viewport_height = monitor.work_height as f64 / monitor.scale_factor;
                let width = FENCE_MIN_WIDTH.max(320.0_f64.min(viewport_width));
                let height = FENCE_MIN_HEIGHT.max(240.0_f64.min(viewport_height));
                WorkspaceCommand::CreateFence { title: format!("新围栏 {}", snapshot.fences.len() + 1), bounds: WorkspaceRect { left: client_x.clamp(0.0, (viewport_width - width).max(0.0)), top: client_y.clamp(0.0, (viewport_height - height).max(0.0)), width, height } }
            }
            _ => return Ok(WorkspaceMenuAction::None),
        }
    };
    Ok(WorkspaceMenuAction::Layout(command))
}

#[cfg(test)]
#[path = "workspace_menu.test.rs"]
mod tests;
