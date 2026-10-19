use super::*;

/// 桌面背景菜单在“新建围栏”“新建文件夹门户”同组内新增“添加天气”，并返回该动作的产品命令编号。
#[test]
fn background_menu_offers_weather_creation_next_to_fence_and_folder_portal()
{
    let monitor = DesktopHostMonitor { id: "A".into(), name: "主屏".into(), primary: true, x: 0, y: 0, width: 1920, height: 1080, work_x: 0, work_y: 0, work_width: 1920, work_height: 1040, scale_factor: 1.0, dpi: 96 }; // 单屏菜单坐标来源。
    let snapshot = WorkspaceSnapshot::default(); // 空白桌面只需要菜单项本身。
    let choice = choose_workspace_menu(&snapshot, &monitor, WorkspaceContextMenuTarget::Background, 10.0, 20.0, |target, _, _, commands| {
        assert!(matches!(target, NativeMenuTarget::Background));
        let labels = commands.iter().map(|command| command.label.as_str()).collect::<Vec<_>>(); // 本次菜单展示的全部项目文案。
        assert_eq!(labels, vec!["新建围栏", "新建文件夹门户", "添加天气", "刷新桌面项目", "打开设置"], "添加天气必须与创建类动作同组且顺序稳定");
        let weather = commands.iter().find(|command| command.label == "添加天气").unwrap(); // 新增的创建入口项。
        assert!(weather.enabled);
        Ok(Some(weather.id))
    })
    .unwrap(); // 只解释选择结果，创建参数由统一入口补齐。
    assert!(matches!(choice, WorkspaceMenuAction::Weather), "添加天气必须由统一创建入口执行，不在这里构造布局命令");
}

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
