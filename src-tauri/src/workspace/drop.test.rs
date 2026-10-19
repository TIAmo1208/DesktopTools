use super::*;

/// 原生内部投放按围栏可见排列落位，不恢复分散的旧桌面坐标。
#[test]
fn internal_drop_uses_rendered_group_geometry()
{
    let snapshot = WorkspaceSnapshot { grid: super::super::WorkspaceGrid { left: 0.0, top: 0.0, column_width: 80.0, row_height: 100.0, rows: 10, columns: 10 }, unclassified_items: [("a", 0.0, 0.0), ("b", 640.0, 800.0)].into_iter().map(|(id, left, top)| super::super::UnclassifiedItemSnapshot { stable_reference: id.into(), monitor_id: "screen".into(), projection_monitor_id: None, projection_position: None, position: WorkspacePoint { left, top }, z_index: 0 }).collect(), ..WorkspaceSnapshot::default() }; // 最小目标工作区。
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
        let mut state = super::super::WorkspaceState::load_with_components(path.clone(), &[], &[]).unwrap();
        state.reconcile_host_monitors(&monitors, &["existing".into()], &[], &[]).unwrap();
        let created = state.apply(WorkspaceCommand::CreateFence { title: " ".into(), bounds: WorkspaceRect { left: 40.0, top: 40.0, width: 320.0, height: 240.0 } }).unwrap();
        let fence = created.fences[0].id.clone();
        let target = DropTarget { monitor_id: "screen".into(), revision: created.revision, fence_id: Some(fence.clone()), portal_id: None, portal_path: None, before_item_id: None, position: WorkspacePoint { left: 0.0, top: 0.0 }, free: false, insertion: None, scrolls: Vec::new() };
        let disk = std::fs::read(&path).unwrap();
        state.preflight_drop(&target, &["existing".into()], 1).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), disk);
        assert!(state.preflight_drop(&target, &["existing".into()], 1000).is_ok()); // 围栏容量不受桌面空格限制。
        assert!(state.preflight_drop(&DropTarget { fence_id: None, ..target.clone() }, &["existing".into()], 1000).is_err()); // 桌面投放仍检查实际容量。
        let ids = ["existing".into(), "new".into(), "concurrent".into()]; // 新文件成功和其他程序并发新增使用不同准确身份。
        state.reconcile_host_monitors(&monitors, &ids, &[], &[]).unwrap();
        assert!(state.preflight_drop(&target, &["existing".into()], 1).is_err()); // 释放前布局版本变化必须拒绝。
        let observed = DesktopItemSnapshot { revision: 1, items: ["existing", "new", "concurrent"].into_iter().map(|id| DesktopItem { stable_reference: id.into(), display_name: id.into(), system_image: None, availability: DesktopItemAvailability::Available }).collect() }; // 完成时的准确文件身份。
        let (committed, change) = state.commit_external_drop(&target, &["existing".into(), "new".into()], &["existing".into()], &observed).unwrap(); // 释放后完成可以使用固定共享目标。
        assert_eq!(committed.fences[0].members, ["existing", "new"]);
        let committed = state.apply(WorkspaceCommand::CommitItemDrag { anchor_item_id: "concurrent".into(), item_ids: vec!["concurrent".into()], placements: vec![super::WorkspaceItemPlacement { item_id: "concurrent".into(), position: WorkspacePoint { left: 0.0, top: 0.0 } }], target_fence_id: None, before_item_id: None, fence_scrolls: Vec::new() }).unwrap(); // 后续独立操作使用旧来源格。
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
        assert_eq!(undone.unclassified_items.iter().find(|item| item.stable_reference == "concurrent").unwrap().position, WorkspacePoint { left: 0.0, top: 0.0 }); // Shell 补偿不覆盖后续独立落位。
        assert_ne!(undone.unclassified_items.iter().find(|item| item.stable_reference == "existing").unwrap().position, WorkspacePoint { left: 0.0, top: 0.0 });
        assert!(!undone.unclassified_items.iter().any(|item| item.stable_reference == "new"));
        drop(state);
        std::fs::remove_dir_all(root).unwrap();
    }
}

/// 与前端同一 L 形组实例核对原生落位，并验证自由定位和输入拒绝。
#[test]
fn native_grid_drop_preserves_group_geometry()
{
    let mut snapshot = WorkspaceSnapshot { grid: super::super::WorkspaceGrid { left: 0.0, top: 0.0, column_width: 80.0, row_height: 100.0, rows: 3, columns: 3 }, unclassified_items: [("anchor", 0.0, 0.0), ("right", 80.0, 0.0), ("down", 0.0, 100.0)].into_iter().map(|(id, left, top)| super::super::UnclassifiedItemSnapshot { stable_reference: id.into(), monitor_id: "screen".into(), projection_monitor_id: None, projection_position: None, position: WorkspacePoint { left, top }, z_index: 0 }).collect(), ..WorkspaceSnapshot::default() }; // 测试只使用布局，不访问文件系统。
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
    let snapshot = WorkspaceSnapshot { grid: super::super::WorkspaceGrid { left: 0.0, top: 0.0, column_width: 78.0, row_height: 98.0, rows: 7, columns: 2 }, unclassified_items: [("dragged", 0.0, 0.0), ("later", 0.0, 196.0), ("legacy", 20.0, 396.0)].into_iter().map(|(id, left, top)| super::super::UnclassifiedItemSnapshot { stable_reference: id.into(), monitor_id: "screen".into(), projection_monitor_id: None, projection_position: None, position: WorkspacePoint { left, top }, z_index: 0 }).collect(), ..WorkspaceSnapshot::default() }; // 对应前端离网来源回归。
    let result = snapshot.grid_drop_placements(&["dragged".into()], "screen", WorkspacePoint { left: 0.0, top: 98.0 }, false).unwrap(); // 只返回实际拖动项目的提交位置。
    assert_eq!(result, [WorkspaceItemPlacement { item_id: "dragged".into(), position: WorkspacePoint { left: 0.0, top: 98.0 } }]);
    assert_eq!(snapshot.unclassified_items[1].position.top, 196.0);
}

/// 使用前端同一传播实例核对原生组插入，且 Alt 不执行挤位。
#[test]
fn native_insertion_matches_frontend_group_propagation()
{
    let snapshot = WorkspaceSnapshot { grid: super::super::WorkspaceGrid { left: 0.0, top: 0.0, column_width: 80.0, row_height: 100.0, rows: 3, columns: 2 }, unclassified_items: [("a", 0.0, 0.0), ("b", 0.0, 100.0), ("target", 0.0, 200.0), ("tail", 80.0, 0.0)].into_iter().map(|(id, left, top)| super::super::UnclassifiedItemSnapshot { stable_reference: id.into(), monitor_id: "screen".into(), projection_monitor_id: None, projection_position: None, position: WorkspacePoint { left, top }, z_index: 0 }).collect(), ..WorkspaceSnapshot::default() }; // 最小插入布局。
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
