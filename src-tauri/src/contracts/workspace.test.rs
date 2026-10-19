use super::{WorkspaceCommand, WorkspaceContextMenuTarget, WorkspaceFocusTarget, WorkspaceInteractionCommand};

/// 验证排序与滚动 transport 名称独立，并拒绝非法排序状态。
#[test]
fn sorting_command_decodes_without_shadowing_scroll()
{
    for mode in ["none", "name-ascending", "name-descending"]
    {
        let command: WorkspaceCommand = serde_json::from_value(serde_json::json!({ "type": "set-fence-sort", "fenceId": "fence-1", "sortMode": mode })).unwrap(); // 模拟 React 的真实命令载荷。
        assert!(matches!(command, WorkspaceCommand::SetFenceSort { .. }));
    }
    let scroll: WorkspaceCommand = serde_json::from_value(serde_json::json!({ "type": "set-fence-scroll", "fenceId": "fence-1", "scrollTop": 12 })).unwrap(); // 原有滚动命令必须继续可用。
    assert!(matches!(scroll, WorkspaceCommand::SetFenceScroll { scroll_top: 12.0, .. }));
    assert!(serde_json::from_value::<WorkspaceCommand>(serde_json::json!({ "type": "set-fence-sort", "fenceId": "fence-1", "sortMode": "unknown" })).is_err());
}

/// 验证前端多选项目菜单目标通过公开 Rust 契约反序列化。
#[test]
fn workspace_context_menu_decodes_selected_items()
{
    let target: WorkspaceContextMenuTarget = serde_json::from_value(serde_json::json!({
        "type": "items",
        "itemIds": ["a", "b"]
    }))
    .expect("工作区项目菜单目标应可反序列化");

    assert_eq!(target, WorkspaceContextMenuTarget::Items { item_ids: vec!["a".to_string(), "b".to_string()] });
}

/// 验证前端组拖动命令通过公开 Rust 契约反序列化。
#[test]
fn workspace_command_decodes_group_drag_transaction()
{
    let command: WorkspaceCommand = serde_json::from_value(serde_json::json!({
        "type": "commit-item-drag",
        "anchorItemId": "a",
        "itemIds": ["a", "b"],
        "placements": [],
        "targetFenceId": "fence-1",
        "beforeItemId": null,
        "fenceScrolls": [{"fenceId": "fence-1", "scrollTop": 24}]
    }))
    .expect("工作区组拖动命令应可反序列化");

    assert!(matches!(
        command,
        WorkspaceCommand::CommitItemDrag {
            anchor_item_id,
            item_ids,
            target_fence_id,
            ..
        } if anchor_item_id == "a" && item_ids == ["a", "b"] && target_fence_id.as_deref() == Some("fence-1")
    ));
}

/// 验证前端 camelCase 交互命令通过公开 Rust 契约反序列化。
#[test]
fn workspace_interaction_decodes_camel_case_focus_surface()
{
    let command: WorkspaceInteractionCommand = serde_json::from_value(serde_json::json!({
        "type": "focus-surface",
        "focus": {"type": "fence", "fenceId": "fence-1"}
    }))
    .expect("工作区公开交互命令应可反序列化");

    assert_eq!(command, WorkspaceInteractionCommand::FocusSurface { focus: WorkspaceFocusTarget::Fence { fence_id: "fence-1".to_string() } });
}

/// 验证前端 camelCase 命令通过公开 Rust 契约反序列化。
#[test]
fn workspace_command_decodes_the_public_contract_shape()
{
    // React bridge 提交的项目归属命令。
    let command: WorkspaceCommand = serde_json::from_value(serde_json::json!({
        "type": "move-item-to-fence",
        "itemId": "::{project}",
        "fenceId": "fence-1",
        "beforeItemId": null
    }))
    .expect("工作区公开命令应可反序列化");

    assert_eq!(command, WorkspaceCommand::MoveItemToFence { item_id: "::{project}".to_string(), fence_id: "fence-1".to_string(), before_item_id: None });
}

/// 创建组件命令继续接受省略初始状态的载荷，并保留调用方给出的初始状态。
#[test]
fn create_component_command_keeps_an_optional_initial_state()
{
    let bounds = serde_json::json!({ "left": 24.0, "top": 24.0, "width": 360.0, "height": 280.0 }); // 两种载荷共用的新实例边界。
    let without_state: WorkspaceCommand = serde_json::from_value(serde_json::json!({ "type": "create-component", "componentId": "official.collect-box", "bounds": bounds.clone() })).expect("省略初始状态的创建命令应可反序列化");
    assert!(matches!(without_state, WorkspaceCommand::CreateComponent { state, .. } if state == serde_json::json!({})));

    let with_state: WorkspaceCommand = serde_json::from_value(serde_json::json!({ "type": "create-component", "componentId": "official.weather", "bounds": bounds, "state": { "weatherSource": "qweather", "weatherCityId": "101010100" } })).expect("带初始状态的创建命令应可反序列化");
    assert!(matches!(with_state, WorkspaceCommand::CreateComponent { state, .. } if state["weatherSource"] == "qweather" && state["weatherCityId"] == "101010100"));
}
