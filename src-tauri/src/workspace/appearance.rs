use super::*;
use crate::contracts::{TileAppearance, TileAppearanceChange, TileTitleMode};

/// 在同一事务中校验外观字段并更新公共投影。
pub(super) fn apply(snapshot: &mut WorkspaceSnapshot, target: WorkspaceFocusTarget, change: TileAppearanceChange) -> Result<(), WorkspaceError>
{
    let (key, initial) = match &target
    {
        WorkspaceFocusTarget::Fence { fence_id } =>
        {
            let fence = snapshot.fence(fence_id)?; // 使用仍存在的围栏身份和初始外观。
            (format!("fence:{fence_id}"), TileAppearance { title: Some(fence.title.clone()), collapsed: fence.collapsed, auto_expand: fence.auto_expand, ..Default::default() })
        }
        WorkspaceFocusTarget::Component { component_id } =>
        {
            snapshot.component_mut(component_id)?;
            (format!("component:{component_id}"), TileAppearance::default())
        }
        _ => return Err(WorkspaceError::invalid_command("外观目标必须是桌面格子")),
    }; // 业务身份不因公共外壳而合并。
    let mut appearance = snapshot.tile_appearances.get(&key).cloned().unwrap_or(initial); // 校验失败时不修改原值。
    match change
    {
        TileAppearanceChange::Title(title) =>
        {
            validate_title(title.trim())?;
            appearance.title = Some(title.trim().to_string());
        }
        TileAppearanceChange::TitleMode(mode) => appearance.title_mode = mode,
        TileAppearanceChange::ButtonMode(mode) =>
        {
            if appearance.title_mode == TileTitleMode::Hover
            {
                return Err(WorkspaceError::invalid_command("悬浮标题下不能修改功能选项显示偏好"));
            }
            appearance.button_mode = mode;
        }
        TileAppearanceChange::Collapsed(collapsed) =>
        {
            if let WorkspaceFocusTarget::Fence { fence_id } = &target
            {
                snapshot.apply(WorkspaceCommand::SetFenceCollapsed { fence_id: fence_id.clone(), collapsed }, None)?;
            }
            appearance.collapsed = collapsed;
        }
        TileAppearanceChange::AutoExpand(enabled) => appearance.auto_expand = enabled,
    }
    snapshot.tile_appearances.insert(key, appearance);
    project(snapshot);
    Ok(())
}

/// 共享外观覆盖围栏快照中的兼容投影字段，保留全部空间数据。
pub(super) fn project(snapshot: &mut WorkspaceSnapshot)
{
    for fence in &mut snapshot.fences
    {
        if let Some(appearance) = snapshot.tile_appearances.get(&format!("fence:{}", fence.id))
        {
            if let Some(title) = &appearance.title
            {
                fence.title.clone_from(title);
            }
            fence.collapsed = appearance.collapsed;
            fence.auto_expand = appearance.auto_expand;
        }
    }
}
