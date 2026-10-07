use super::*;

/// 验证菜单展示、DPI 坐标和选择解释通过同一入口执行。
#[test]
fn menu_selection_preserves_target_and_rejects_disabled_actions()
{
    let monitor = DesktopHostMonitor { id: "A".into(), name: "主屏".into(), primary: true, x: -1920, y: 0, width: 1920, height: 1080, work_x: -1920, work_y: 30, work_width: 1920, work_height: 1050, scale_factor: 1.5, dpi: 144 }; // 混合 DPI 的菜单坐标来源。
    let mut snapshot = WorkspaceSnapshot::default(); // 当前独立布局。
    let choice = choose_workspace_menu(&snapshot, &monitor, WorkspaceContextMenuTarget::Background, 10.0, 20.0, |target, x, y, commands| {
        assert!(matches!(target, NativeMenuTarget::Background));
        assert_eq!((x, y), (-1905, 60));
        assert!(!commands.iter().any(|command| command.label == "编辑布局"));
        Ok(commands.iter().find(|command| command.label == "新建围栏").map(|command| command.id))
    })
    .unwrap(); // 真实菜单选择入口产生的布局动作。
    assert!(matches!(choice, WorkspaceMenuAction::Layout(WorkspaceCommand::CreateFence { .. })));
    for ids in [vec!["one".to_string()], vec!["one".to_string(), "two".to_string()]]
    {
        let created = choose_workspace_menu(&snapshot, &monitor, WorkspaceContextMenuTarget::Items { item_ids: ids.clone() }, 2000.0, 2000.0, |target, _, _, commands| {
            assert!(matches!(target, NativeMenuTarget::Items(ref selected) if selected == &ids));
            assert!(!commands.iter().any(|command| command.label == "编辑布局"));
            let command = commands.last().unwrap(); // 无已有围栏时仍保留创建入口。
            assert_eq!(command.label, "新建围栏并移入");
            assert!(command.enabled);
            Ok(Some(command.id))
        })
        .unwrap(); // 新建与移入必须解释为一个组合命令。
        assert!(matches!(created, WorkspaceMenuAction::Layout(WorkspaceCommand::CreateFenceWithItems { item_ids, bounds, .. }) if item_ids == ids && bounds.left == 960.0 && bounds.top == 460.0));
    }
    assert!(matches!(choose_workspace_menu(&snapshot, &monitor, WorkspaceContextMenuTarget::Background, 0.0, 0.0, |_, _, _, _| Ok(None)).unwrap(), WorkspaceMenuAction::None));
    assert!(choose_workspace_menu(&snapshot, &monitor, WorkspaceContextMenuTarget::Background, 0.0, 0.0, |_, _, _, commands| Ok(commands.iter().find(|command| !command.enabled).map(|command| command.id))).is_err());
    assert!(choose_workspace_menu(&snapshot, &monitor, WorkspaceContextMenuTarget::Background, f64::NAN, 0.0, |_, _, _, _| panic!("非法坐标不能打开菜单")).is_err());
    snapshot.fences.push(
        serde_json::from_value(serde_json::json!({
            "id": "f", "title": "工作", "monitorId": "A", "projectionMonitorId": null,
            "projectionBounds": null, "bounds": { "left": 0, "top": 0, "width": 320, "height": 240 },
            "members": [], "layoutMode": "tile", "scrollTop": 0, "collapsed": false,
            "lastExpandedHeight": 240, "zIndex": 1
        }))
        .unwrap(),
    ); // 菜单打开时固定的目标围栏。
    let moved = choose_workspace_menu(&snapshot, &monitor, WorkspaceContextMenuTarget::Items { item_ids: vec!["item".into()] }, 0.0, 0.0, |target, _, _, commands| {
        assert!(matches!(target, NativeMenuTarget::Items(ref ids) if ids == &["item"]));
        Ok(Some(commands[0].id))
    })
    .unwrap(); // 动态编号必须解析回稳定围栏身份。
    assert!(matches!(moved, WorkspaceMenuAction::Layout(WorkspaceCommand::CommitItemDrag { target_fence_id: Some(ref id), .. }) if id == "f"));
    let collapsed = choose_workspace_menu(&snapshot, &monitor, WorkspaceContextMenuTarget::Fence { fence_id: "f".into() }, 0.0, 0.0, |target, _, _, commands| {
        assert!(matches!(target, NativeMenuTarget::ProductOnly));
        Ok(Some(commands[0].id))
    })
    .unwrap(); // 围栏表面语义选择。
    assert!(matches!(collapsed, WorkspaceMenuAction::Layout(WorkspaceCommand::SetFenceCollapsed { collapsed: true, .. })));
}
