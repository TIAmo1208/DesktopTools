use super::{AUTO_SNAPSHOT_LIMIT, PersistedWorkspacePayload, WorkspaceState, decode_workspace_store, merge_layout_history_target, workspace_store_from_payload, write_auto_snapshot};
use crate::contracts::{ComponentInstanceSnapshot, DesktopHostMonitor, UnclassifiedItemSnapshot, WorkspaceCommand, WorkspaceFocusTarget, WorkspaceInteractionCommand, WorkspaceItemPlacement, WorkspacePoint, WorkspaceRect, WorkspaceSelectionMode, WorkspaceSnapshot};
use std::time::{SystemTime, UNIX_EPOCH};

/// 新增项目使用准确身份的坐标，已有项目不随 Explorer 布局移动。
#[test]
fn shell_positions_only_place_new_items()
{
    let path = snapshot_path("new-item-positions"); // 独立持久化文件。
    let state = workspace_state(path.clone()); // 真实工作区协调入口。
    let monitors = [DesktopHostMonitor { work_width: 78, work_height: 98, ..monitor("A", true, 0, 1.0) }, monitor("B", false, -1920, 1.25)]; // 主屏只有一个槽位，新项目必须直接分配到次屏。
    let initial = state.reconcile_host_monitors(&monitors, &["existing".into()], &[], &[]).unwrap(); // 已有布局不重新导入。
    let positions = [("existing".into(), "B".into(), WorkspacePoint { left: 500.0, top: 300.0 }), ("new".into(), "B".into(), WorkspacePoint { left: 156.0, top: 196.0 })]; // 同次读取的已有与新增坐标。
    let next = state.reconcile_environment_state(&monitors, &["existing".into(), "new".into()], &[], &[], None, false, &positions).unwrap();
    assert_eq!(next.unclassified_items[0], initial.unclassified_items[0]);
    let new = next.unclassified_items.iter().find(|item| item.stable_reference == "new").unwrap(); // 新项目的最终权威位置。
    assert_eq!(new.monitor_id, "B");
    let grid = next.monitor_grids.iter().find(|entry| entry.monitor_id == "B").unwrap().grid; // 目标屏实际网格。
    assert!((new.position.left - 156.0).abs() <= grid.column_width / 2.0);
    assert!((new.position.top - 196.0).abs() <= grid.row_height / 2.0);
    let repeated = state.reconcile_environment_state(&monitors, &["existing".into(), "new".into()], &[], &[], None, false, &positions).unwrap();
    assert_eq!(repeated, next);
    std::fs::remove_file(path).unwrap();
}

/// 新文件占用已释放的来源格后，撤销仍恢复原项目并保留新文件。
#[test]
fn undo_membership_keeps_a_new_item_in_the_released_cell()
{
    let path = snapshot_path("released-cell-undo"); // 独立测试存储。
    let state = workspace_state(path.clone()); // 真实事务和 Shell 协调路径。
    let initial = state.reconcile_items(400.0, 400.0, &["old".into()]).unwrap(); // 原项目在左上格。
    let fence = state.apply(WorkspaceCommand::CreateFence { title: "测试".into(), bounds: WorkspaceRect { left: 80.0, top: 80.0, width: 320.0, height: 240.0 } }).unwrap().fences[0].id.clone(); // 接收围栏。
    state.apply(WorkspaceCommand::MoveItemToFence { item_id: "old".into(), fence_id: fence, before_item_id: None }).unwrap();
    let added = state.reconcile_items(400.0, 400.0, &["old".into(), "new".into()]).unwrap(); // 外部新增不创建布局撤销事务。
    assert_eq!(added.unclassified_items.len(), 1);
    assert_eq!(added.unclassified_items[0].position, initial.unclassified_items[0].position);
    let undone = state.apply(WorkspaceCommand::UndoLayout).unwrap(); // 原位由历史恢复。
    assert_eq!(undone.unclassified_items.iter().find(|item| item.stable_reference == "old").unwrap().position, initial.unclassified_items[0].position);
    assert_eq!(undone.unclassified_items.len(), 2);
    assert_ne!(undone.unclassified_items[0].position, undone.unclassified_items[1].position);
    let redone = state.apply(WorkspaceCommand::RedoLayout).unwrap(); // 重做不丢失新文件，也不产生隐藏占位。
    assert_eq!(redone.unclassified_items.len(), 1);
    assert_eq!(redone.unclassified_items[0].stable_reference, "new");
    std::fs::remove_file(path).unwrap();
}

/// 相同路径的独立身份、几何、移除和重启恢复通过真实持久化路径验证。
#[test]
fn folder_portals_persist_independent_duplicate_paths()
{
    let path = snapshot_path("folder-portals"); // 独立测试布局文件。
    let root_path = std::env::temp_dir().to_string_lossy().into_owned(); // 已存在且不写入内容的目录。
    let bounds = WorkspaceRect { left: 24.0, top: 24.0, width: 360.0, height: 280.0 }; // 初始几何。
    let state = WorkspaceState::load(path.clone()).unwrap(); // 使用真实原子保存。
    state.apply(WorkspaceCommand::CreateFolderPortal { root_path: root_path.clone(), bounds }).unwrap();
    let created = state.apply(WorkspaceCommand::CreateFolderPortal { root_path, bounds }).unwrap(); // 同路径始终追加。
    assert_eq!(created.folder_portals.len(), 2);
    assert_ne!(created.folder_portals[0].id, created.folder_portals[1].id);
    let moved = WorkspaceRect { left: 100.0, ..bounds }; // 只修改第一个实例。
    state.apply(WorkspaceCommand::UpdateFolderPortalBounds { portal_id: created.folder_portals[0].id.clone(), bounds: moved }).unwrap();
    state.apply(WorkspaceCommand::SetFolderPortalAppearance { portal_id: created.folder_portals[0].id.clone(), change: crate::contracts::TileAppearanceChange::Collapsed(true) }).unwrap(); // 外观不占据布局撤销步骤。
    let undone = state.apply(WorkspaceCommand::UndoLayout).unwrap(); // 撤销移动，保留当前外观。
    assert_eq!(undone.folder_portals[0].bounds, bounds);
    assert!(undone.folder_portals[0].appearance.collapsed);
    state.apply(WorkspaceCommand::RedoLayout).unwrap();
    drop(state);
    let restored = WorkspaceState::load(path.clone()).unwrap(); // 重启恢复权威工作区。
    let snapshot = restored.snapshot().unwrap(); // 断言路径和几何均独立。
    assert_eq!(snapshot.folder_portals[0].bounds, moved);
    assert!(snapshot.folder_portals[0].appearance.collapsed);
    assert_eq!(snapshot.folder_portals[1].bounds, bounds);
    let remapped = restored.apply(WorkspaceCommand::RemapFolderPortal { portal_id: snapshot.folder_portals[0].id.clone(), root_path: std::env::current_dir().unwrap().to_string_lossy().into_owned() }).unwrap();
    assert_eq!(remapped.folder_portals[0].root_path, std::env::current_dir().unwrap().to_string_lossy());
    let removed = restored.apply(WorkspaceCommand::RemoveFolderPortal { portal_id: snapshot.folder_portals[0].id.clone() }).unwrap();
    assert_eq!(removed.folder_portals.len(), 1);
    assert!(std::path::Path::new(&removed.folder_portals[0].root_path).is_dir());
}

/// 创建当前测试独占的工作区快照路径。
fn snapshot_path(name: &str) -> std::path::PathBuf
{
    // 避免并发测试共享文件的纳秒时间标识。
    let unique = SystemTime::now().duration_since(UNIX_EPOCH).expect("系统时间应晚于 Unix epoch").as_nanos();
    std::env::temp_dir().join(format!("desktop-tools-{name}-{}-{unique}.json", std::process::id()))
}

/// 创建当前测试使用的工作区状态。
fn workspace_state(path: std::path::PathBuf) -> WorkspaceState { WorkspaceState::load(path).expect("测试工作区应可启动") }

/// 环境重复观察不改变统一布局、选择或撤销，保存和还原只包含同一布局。
#[test]
fn unified_workspace_preserves_layout_interaction_and_history()
{
    let path = snapshot_path("unified-workspace"); // 独占持久文件。
    let backup = snapshot_path("unified-backup"); // 独占手动备份。
    let mut state = WorkspaceState::load(path.clone()).unwrap(); // 唯一工作区不接收系统桌面身份。
    let monitors = [monitor("DISPLAY1", true, 0, 1.0)]; // 固定物理显示器。
    let items = ["one".to_string(), "two".to_string()]; // 固定 Shell 事实。
    state.reconcile_host_monitors(&monitors, &items, &[], &[]).unwrap();
    let bounds = WorkspaceRect { left: 40.0, top: 40.0, width: 320.0, height: 240.0 }; // 初始围栏位置。
    let created = state.apply(WorkspaceCommand::CreateFence { title: "统一".into(), bounds }).unwrap(); // 创建唯一围栏。
    let fence_id = created.fences[0].id.clone(); // 后续事务使用稳定身份。
    state.apply(WorkspaceCommand::MoveItemToFence { item_id: "one".into(), fence_id: fence_id.clone(), before_item_id: None }).unwrap();
    let moved_bounds = WorkspaceRect { left: 500.0, ..bounds }; // 成员操作后的空间事务。
    state.apply(WorkspaceCommand::UpdateFenceBounds { fence_id: fence_id.clone(), bounds: moved_bounds }).unwrap();
    state.apply_interaction_on_monitor("DISPLAY1", WorkspaceInteractionCommand::FocusItem { item_id: "two".into(), selection_mode: WorkspaceSelectionMode::Replace }).unwrap();
    let before = state.snapshot().unwrap(); // 选择、位置、成员共同保留。
    let time = state.latest_layout_history_time(false).unwrap(); // 环境重复观察不产生历史。
    for _ in 0..3
    {
        assert_eq!(state.reconcile_host_monitors(&monitors, &items, &[], &[]).unwrap(), before);
    }
    assert_eq!(state.latest_layout_history_time(false).unwrap(), time);
    let undone = state.apply(WorkspaceCommand::UndoLayout).unwrap(); // 空间先于成员撤销。
    assert_eq!(undone.fences[0].bounds, bounds);
    assert_eq!(undone.fences[0].members, ["one"]);
    assert_eq!(undone.interaction.selected_item_ids, ["two"]);
    assert!(state.apply(WorkspaceCommand::UndoLayout).unwrap().fences[0].members.is_empty());
    state.apply(WorkspaceCommand::RedoLayout).unwrap();
    let restored = state.apply(WorkspaceCommand::RedoLayout).unwrap(); // 同一时间线重做恢复布局。
    assert_eq!(restored.fences, before.fences);
    state.write_manual_backup(&backup).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap(); // 磁盘唯一布局。
    assert_eq!(value["storeVersion"], 5);
    assert_eq!(value["payload"]["layout"]["fences"][0]["members"], serde_json::json!(["one"]));
    assert!(value["payload"].get("layouts").is_none());
    assert!(value["payload"].get("currentVirtualDesktopId").is_none());
    state.apply(WorkspaceCommand::UpdateFenceBounds { fence_id, bounds }).unwrap();
    let modified = state.snapshot().unwrap(); // 注入保存失败后应完整保留。
    let history_time = state.latest_layout_history_time(false).unwrap(); // 失败不能清空历史。
    state.persistence_path = path.join("blocked.json");
    assert!(state.restore_backup(&backup, &monitors, &items).is_err());
    assert_eq!(state.snapshot().unwrap(), modified);
    assert_eq!(state.latest_layout_history_time(false).unwrap(), history_time);
    state.persistence_path = path.clone();
    assert_eq!(state.restore_backup(&backup, &monitors, &items).unwrap().snapshot.fences, before.fences);
    assert!(state.latest_layout_history_time(false).unwrap().is_none());
    drop(state);
    let reopened = WorkspaceState::load(path.clone()).unwrap(); // 重启不依赖系统桌面身份。
    assert_eq!(reopened.snapshot().unwrap().fences, before.fences);
    assert!(reopened.snapshot().unwrap().interaction.selected_item_ids.is_empty());
    drop(reopened);
    std::fs::remove_file(path).unwrap();
    std::fs::remove_file(backup).unwrap();
}

/// 验证环境观察失败时名称、布局和磁盘保持原状态，重试可整体提交。
#[test]
fn environment_observation_rolls_back_names_with_layout()
{
    let path = snapshot_path("environment-transaction"); // 测试独占持久文件。
    let mut state = workspace_state(path.clone()); // 可注入保存失败的工作区。
    let monitors = [monitor("DISPLAY1", true, 0, 1.0)]; // 当前宿主事实。
    let items = vec![crate::contracts::DesktopItem { stable_reference: "a".into(), display_name: "Before".into(), system_image: None, availability: crate::contracts::DesktopItemAvailability::Available }]; // 首次 Shell 观察。
    state.reconcile_environment(super::WorkspaceEnvironment { new_item_positions: &[], transfer_assignments: false, monitors: &monitors, items: &items, assignments: &[], renames: &[] }).unwrap();
    let before = state.snapshot().unwrap(); // 原始权威布局。
    let disk = std::fs::read(&path).unwrap(); // 已保存的原始快照。
    let mut next_items = items.clone(); // Shell 确认的重命名结果。
    next_items[0].stable_reference = "b".into();
    next_items[0].display_name = "After".into();
    let renames = [("a".into(), "b".into())]; // 精确身份变化。
    state.persistence_path = path.join("blocked.json");
    assert!(state.reconcile_environment(super::WorkspaceEnvironment { new_item_positions: &[], transfer_assignments: false, monitors: &monitors, items: &next_items, assignments: &[], renames: &renames }).is_err());
    assert_eq!(state.snapshot().unwrap(), before);
    assert_eq!(state.item_names.lock().unwrap().get("a").unwrap(), "before");
    assert_eq!(std::fs::read(&path).unwrap(), disk);
    state.persistence_path = path.clone();
    let result = state.reconcile_environment(super::WorkspaceEnvironment { new_item_positions: &[], transfer_assignments: false, monitors: &monitors, items: &next_items, assignments: &[], renames: &renames }).unwrap(); // 同一观察重试成功。
    assert_eq!(result.unclassified_items[0].stable_reference, "b");
    assert_eq!(state.item_names.lock().unwrap().get("b").unwrap(), "after");
    drop(state);
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir_all(path.with_extension("snapshots")).unwrap();
}

/// 共享标题和悬停策略持久化，撤销空间移动不得恢复旧外观。
#[test]
fn fence_title_and_hover_policy_persist_and_undo()
{
    let path = snapshot_path("fence-options"); // 独占配置。
    let state = workspace_state(path.clone()); // 测试工作区。
    let snapshot = state.apply(WorkspaceCommand::CreateFence { title: "旧标题".into(), bounds: WorkspaceRect { left: 10.0, top: 10.0, width: 320.0, height: 240.0 } }).unwrap(); // 初始围栏。
    let id = snapshot.fences[0].id.clone(); // 稳定围栏身份。
    state.apply(WorkspaceCommand::UpdateFenceBounds { fence_id: id.clone(), bounds: WorkspaceRect { left: 100.0, ..snapshot.fences[0].bounds } }).unwrap(); // 建立独立空间历史。
    let rename = serde_json::from_value(serde_json::json!({"type":"rename-fence", "fenceId":id, "title":"新标题"})).unwrap(); // 前端实际命令格式。
    state.apply(rename).unwrap();
    let enable = serde_json::from_value(serde_json::json!({"type":"set-fence-auto-expand", "fenceId":id, "enabled":true})).unwrap(); // 前端开关命令。
    state.apply(enable).unwrap();
    assert!(state.snapshot().unwrap().fences[0].auto_expand);
    assert_eq!(workspace_state(path.clone()).snapshot().unwrap().fences[0].title, "新标题");
    state.apply(WorkspaceCommand::UndoLayout).unwrap();
    assert!(state.snapshot().unwrap().fences[0].auto_expand);
    assert_eq!(state.snapshot().unwrap().fences[0].bounds, snapshot.fences[0].bounds);
    assert_eq!(state.snapshot().unwrap().fences[0].title, "新标题");
    let empty = state.apply(WorkspaceCommand::RenameFence { fence_id: id.clone(), title: "  ".into() }).unwrap(); // 空白标题规范化为空标题。
    assert_eq!(empty.fences[0].title, "");
    assert!(state.apply(WorkspaceCommand::RenameFence { fence_id: id, title: "\0".into() }).is_err());
    drop(state);
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir_all(path.with_extension("snapshots")).unwrap();
}

/// 没有 Shell 坐标的新增项目逐列填补空位，保留已有布局位置。
#[test]
fn new_desktop_items_fill_columns_and_reuse_holes()
{
    let path = snapshot_path("new-items-row-order"); // 测试独占布局文件。
    let state = workspace_state(path.clone()); // 当前权威工作区。
    let first = state.reconcile_items(400.0, 400.0, &["a".into(), "b".into(), "c".into()]).unwrap(); // 首次枚举。
    let positions = first.unclassified_items.iter().map(|item| item.position).collect::<Vec<_>>(); // 初始逐列位置。
    assert_eq!(positions[0].left, positions[1].left);
    assert_eq!(positions[1].top - positions[0].top, first.grid.row_height);
    state.reconcile_items(400.0, 400.0, &["a".into(), "c".into()]).unwrap();
    let next = state.reconcile_items(400.0, 400.0, &["a".into(), "c".into(), "d".into()]).unwrap(); // 补充新项目。
    assert_eq!(next.unclassified_items.iter().find(|item| item.stable_reference == "d").unwrap().position, positions[1]);
    assert_eq!(next.unclassified_items.iter().find(|item| item.stable_reference == "c").unwrap().position, positions[2]);
    drop(state);
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir_all(path.with_extension("snapshots")).unwrap();
}

/// 独立网格设置覆盖运行、重启及容量不足不提交的完整路径。
#[test]
fn grid_preferences_reflow_and_reject_insufficient_capacity()
{
    let path = snapshot_path("grid-preferences"); // 测试独占布局文件。
    let settings_path = snapshot_path("grid-settings"); // 测试独占偏好文件。
    let state = workspace_state(path.clone()); // 原子持久化工作区。
    let settings = crate::app::settings::SettingsState::load(settings_path.clone()).unwrap(); // 首次启动配置。
    let monitors = [monitor("DISPLAY1", true, 0, 1.5)]; // 混合 DPI 计算中的 150% 显示器。
    let items = (0..30).map(|index| format!("item-{index}")).collect::<Vec<_>>(); // 新大网格不能全部容纳的项目数。
    state.reconcile_host_monitors(&monitors, &items, &[], &[]).unwrap();
    let mut preferences = settings.preferences().unwrap(); // 等待修改的配置。
    preferences.desktop_grid.width = 100;
    state.save_preferences(&settings, preferences.clone(), &monitors).unwrap();
    let snapshot = state.snapshot().unwrap(); // 已应用的权威布局。
    assert_eq!(snapshot.grid.column_width, 110.0);
    assert_eq!(snapshot.grid.row_height, 98.0);
    assert_eq!(snapshot.unclassified_items[0].position.left, 0.0);
    assert_eq!(settings.preferences().unwrap().fence_grid, crate::contracts::settings::ApplicationPreferences::default().fence_grid);
    preferences.desktop_grid = crate::contracts::settings::GridPreferences { width: 160, height: 180, column_gap: 48, row_gap: 48 };
    assert!(state.save_preferences(&settings, preferences, &monitors).is_err());
    assert_eq!(state.snapshot().unwrap(), snapshot);
    assert_eq!(settings.preferences().unwrap().desktop_grid.width, 100);
    let blocked_path = snapshot_path("blocked-grid-settings"); // 写入时才变成目录的配置目标。
    let blocked_settings = crate::app::settings::SettingsState::load(blocked_path.clone()).unwrap(); // 可读取但随后无法替换目标的配置状态。
    std::fs::create_dir(&blocked_path).unwrap();
    let mut rejected = settings.preferences().unwrap(); // 会触发重排但最终保存失败的配置。
    rejected.desktop_grid.width = 110;
    assert!(state.save_preferences(&blocked_settings, rejected, &monitors).is_err());
    assert_eq!(state.snapshot().unwrap(), snapshot);
    std::fs::remove_dir(blocked_path).unwrap();
    drop(state);
    let restored = workspace_state(path.clone()); // 重启恢复旧格位与新步进。
    restored.initialize_grid_preferences(settings.preferences().unwrap().desktop_grid).unwrap();
    let next = restored.reconcile_host_monitors(&monitors, &items, &[], &[]).unwrap(); // 环境变化不回退默认网格。
    assert_eq!(next.grid.column_width, 110.0);
    assert_eq!(next.unclassified_items, snapshot.unclassified_items);
    drop(restored);
    std::fs::remove_file(path).unwrap();
    std::fs::remove_file(settings_path).unwrap();
}

/// 创建逐屏工作区测试使用的显示器物理状态。
fn monitor(id: &str, primary: bool, x: i32, scale_factor: f64) -> DesktopHostMonitor { DesktopHostMonitor { id: id.to_string(), name: id.to_string(), primary, x, y: 0, width: 1920, height: 1080, work_x: x, work_y: 0, work_width: 1920, work_height: 1040, scale_factor, dpi: (96.0 * scale_factor).round() as u32 } }

/// 验证官方组件边界、状态和显示器归属可恢复，并在原显示器缺失时仅临时投射。
#[test]
fn component_layout_persists_and_projects_without_losing_ownership()
{
    let path = snapshot_path("component-layout"); // 当前测试使用的持久工作区文件。
    let snapshot_directory = path.with_extension("snapshots"); // 当前测试可能生成的自动快照目录。
    let default_component = ComponentInstanceSnapshot { instance_id: "official.local-clock.default".to_string(), component_id: "official.local-clock".to_string(), monitor_id: String::new(), projection_monitor_id: None, projection_bounds: None, bounds: WorkspaceRect { left: 0.0, top: 0.0, width: 220.0, height: 116.0 }, state_version: 1, state: serde_json::json!({"format": "24-hour"}) }; // 组件运行时注册的官方默认实例。
    let state = WorkspaceState::load_with_components(path.clone(), std::slice::from_ref(&default_component)).expect("包含官方组件的工作区应可启动");
    state.reconcile_host_monitors(&[monitor("DISPLAY1", true, 0, 1.0), monitor("DISPLAY2", false, 1920, 1.25)], &[], &[], &[]).expect("官方组件应可进入主显示器默认布局");
    state.apply_on_monitor(Some("DISPLAY2"), WorkspaceCommand::UpdateComponentBounds { instance_id: default_component.instance_id.clone(), bounds: WorkspaceRect { left: 140.0, top: 96.0, width: 240.0, height: 128.0 } }).expect("组件应可迁移到第二显示器");
    state.apply_on_monitor(Some("DISPLAY2"), WorkspaceCommand::UpdateComponentState { instance_id: default_component.instance_id.clone(), state_version: 1, state: serde_json::json!({"format": "12-hour"}) }).expect("组件版本化状态应可保存");
    let undone = state.apply(WorkspaceCommand::UndoLayout).unwrap(); // 只撤销空间移动。
    assert_eq!(undone.components[0].monitor_id, "DISPLAY1");
    assert_eq!(undone.components[0].state["format"], "12-hour");
    let redone = state.apply(WorkspaceCommand::RedoLayout).unwrap(); // 同一历史恢复位置，不覆盖私有状态。
    assert_eq!(redone.components[0].monitor_id, "DISPLAY2");
    assert_eq!(redone.components[0].state["format"], "12-hour");
    drop(state);

    let restored = WorkspaceState::load_with_components(path.clone(), std::slice::from_ref(&default_component)).expect("组件布局应可从工作区文件恢复");
    let restored_snapshot = restored.snapshot().expect("恢复后的组件快照应可读取");
    assert_eq!(restored_snapshot.components[0].monitor_id, "DISPLAY2");
    assert_eq!(restored_snapshot.components[0].bounds.width, 240.0);
    assert_eq!(restored_snapshot.components[0].state["format"], "12-hour");

    let projected = restored.reconcile_host_monitors(&[monitor("DISPLAY1", true, 0, 1.0)], &[], &[], &[]).expect("缺失显示器上的组件应可临时投射");
    assert_eq!(projected.components[0].monitor_id, "DISPLAY2");
    assert_eq!(projected.components[0].projection_monitor_id.as_deref(), Some("DISPLAY1"));
    assert!(projected.components[0].projection_bounds.is_some());
    drop(restored);

    std::fs::remove_file(path).expect("组件测试快照应可清理");
    if snapshot_directory.exists()
    {
        std::fs::remove_dir_all(snapshot_directory).expect("组件测试自动快照目录应可清理");
    }
}

/// 验证排序事务覆盖归属、拖动、重命名、历史和持久化边界。
#[test]
fn fence_sorting_is_persisted_and_applies_to_every_membership_path()
{
    use crate::contracts::{DesktopItem, DesktopItemAvailability, FenceSortMode};
    let path = snapshot_path("fence-sorting"); // 本测试独占的持久化文件。
    let state = workspace_state(path.clone()); // 使用真实布局事务和原子持久化的工作区。
    let mut items = [("a", "Zulu"), ("b", "alpha"), ("c", "Bravo"), ("d", "ALPHA")].map(|(id, name)| DesktopItem { stable_reference: id.to_string(), display_name: name.to_string(), system_image: None, availability: DesktopItemAvailability::Available }); // 显示名称与引用顺序故意不同，并包含忽略大小写的同名项目。
    let ids = items.iter().map(|item| item.stable_reference.clone()).collect::<Vec<_>>(); // 当前完整 Shell 引用集合。
    state.update_item_names(&items).unwrap();
    state.reconcile_items(1280.0, 720.0, &ids).unwrap();
    let fence_id = state.apply(WorkspaceCommand::CreateFence { title: "排序验收".to_string(), bounds: WorkspaceRect { left: 100.0, top: 80.0, width: 320.0, height: 240.0 } }).unwrap().fences[0].id.clone(); // 本测试目标围栏。
    for id in ["a", "b"]
    {
        state.apply(WorkspaceCommand::MoveItemToFence { item_id: id.to_string(), fence_id: fence_id.clone(), before_item_id: None }).unwrap();
    }
    let ascending = state.apply(WorkspaceCommand::SetFenceSort { fence_id: fence_id.clone(), sort_mode: FenceSortMode::NameAscending }).unwrap(); // 首次名称排序结果。
    assert_eq!(ascending.fences[0].members, ["b", "a"]);
    let grouped = state.apply(WorkspaceCommand::CommitItemDrag { anchor_item_id: "c".to_string(), item_ids: vec!["c".to_string(), "d".to_string()], placements: vec![], target_fence_id: Some(fence_id.clone()), before_item_id: Some("stale-target".to_string()), fence_scrolls: vec![] }).unwrap(); // 排序模式忽略手动插入位置，包括旧前端快照中的失效目标。
    assert_eq!(grouped.fences[0].members, ["b", "d", "c", "a"]);
    assert_eq!(state.apply(WorkspaceCommand::UndoLayout).unwrap().fences[0].members, ["b", "a"]);
    assert_eq!(state.apply(WorkspaceCommand::RedoLayout).unwrap().fences[0].members, ["b", "d", "c", "a"]);
    let descending = state.apply(WorkspaceCommand::SetFenceSort { fence_id: fence_id.clone(), sort_mode: FenceSortMode::NameDescending }).unwrap(); // 再次名称排序结果。
    assert_eq!(descending.fences[0].members, ["a", "c", "b", "d"]);
    let disabled = state.apply(WorkspaceCommand::SetFenceSort { fence_id: fence_id.clone(), sort_mode: FenceSortMode::None }).unwrap(); // 关闭自动排序保留已有排列。
    assert_eq!(disabled.fences[0].members, descending.fences[0].members);
    let manual = state.apply(WorkspaceCommand::MoveItemToFence { item_id: "d".to_string(), fence_id: fence_id.clone(), before_item_id: Some("a".to_string()) }).unwrap(); // 手动插入继续生效。
    assert_eq!(manual.fences[0].members, ["d", "a", "c", "b"]);
    state.apply(WorkspaceCommand::SetFenceSort { fence_id: fence_id.clone(), sort_mode: FenceSortMode::NameAscending }).unwrap();
    items[0].display_name = "Aardvark".to_string();
    state.update_item_names(&items).unwrap();
    let renamed = state.reconcile_items(1280.0, 720.0, &ids).unwrap(); // 名称变化但引用不变也应重新排序。
    assert_eq!(renamed.fences[0].members, ["a", "d", "b", "c"]);
    drop(state);
    let restored = workspace_state(path.clone()); // 验证重启后保留排序状态及成员顺序。
    assert_eq!(restored.snapshot().unwrap().fences, renamed.fences);
    drop(restored);
    std::fs::remove_file(path).unwrap();
}

/// 验证围栏成员随唯一布局保存，并通过当前格式完整性校验。
#[test]
fn current_fence_store_round_trips()
{
    let path = snapshot_path("shared-fence-store"); // 本测试独占的当前格式备份。
    let state = workspace_state(path.clone()); // 创建默认无排序围栏的工作区。
    state.apply(WorkspaceCommand::CreateFence { title: "旧围栏".to_string(), bounds: WorkspaceRect { left: 0.0, top: 0.0, width: 320.0, height: 240.0 } }).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap(); // 当前规范化备份载荷。
    assert!(!serde_json::to_string(&value).unwrap().contains("sortMode"));
    assert!(value["payload"].get("sharedFences").is_none());
    assert_eq!(value["payload"]["layout"]["fences"][0]["members"], serde_json::json!([]));
    let store = decode_workspace_store(value).unwrap(); // 当前格式校验并读取完整成员。
    assert_eq!(store.payload.layout.fences[0].sort_mode, crate::contracts::FenceSortMode::None);
    drop(state);
    std::fs::remove_file(path).unwrap();
}

/// 验证创建围栏命令持久化完整初始状态并可在重启后恢复。
#[test]
fn create_fence_command_persists_and_restores_the_snapshot()
{
    // 当前测试使用的持久化文件。
    let path = snapshot_path("create-fence");
    // 模拟首次应用启动的工作区状态。
    let state = workspace_state(path.clone());
    // 创建围栏命令返回的权威快照。
    let snapshot = state.apply(WorkspaceCommand::CreateFence { title: "项目资料".to_string(), bounds: WorkspaceRect { left: 120.0, top: 80.0, width: 360.0, height: 280.0 } }).expect("有效围栏应可创建");

    assert_eq!(snapshot.revision, 1);
    assert_eq!(snapshot.fences[0].id, "fence-1");
    assert_eq!(snapshot.fences[0].last_expanded_height, 280.0);
    drop(state);

    // 模拟应用重启后从磁盘恢复的工作区状态。
    let restored = WorkspaceState::load(path.clone()).expect("已保存工作区应可恢复");
    assert_eq!(restored.snapshot().expect("恢复快照应可读取"), snapshot);

    std::fs::remove_file(path).expect("测试快照应可清理");
}

/// 验证单项选择通过公开快照发布且应用重启后回到空运行期状态。
#[test]
fn item_selection_is_runtime_state()
{
    // 当前测试使用的持久化文件。
    let path = snapshot_path("item-selection");
    // 已协调一个真实桌面项目身份的工作区状态。
    let state = workspace_state(path.clone());
    state.reconcile_items(1280.0, 720.0, &["::{project}".to_string()]).expect("Shell 项目应可协调到工作区");

    // 单击项目后发布的权威交互快照。
    let snapshot = state.apply_interaction(WorkspaceInteractionCommand::FocusItem { item_id: "::{project}".to_string(), selection_mode: WorkspaceSelectionMode::Replace }).expect("项目应可成为唯一选择和键盘焦点");
    assert_eq!(snapshot.interaction.revision, 1);
    assert_eq!(snapshot.interaction.selected_item_ids, ["::{project}"]);
    assert_eq!(snapshot.interaction.selection_anchor_id.as_deref(), Some("::{project}"));

    drop(state);
    // 重启后只恢复布局，选择与焦点使用新的运行期初始状态。
    let restored = WorkspaceState::load(path.clone()).expect("工作区布局应可恢复");
    assert_eq!(restored.snapshot().expect("恢复快照应可读取").interaction.revision, 0);
    assert!(restored.snapshot().expect("恢复快照应可读取").interaction.selected_item_ids.is_empty());

    std::fs::remove_file(path).expect("测试快照应可清理");
}

/// 验证切换、范围和保留选择模式遵循焦点容器逻辑顺序。
#[test]
fn selection_modes_follow_the_focus_container()
{
    // 当前测试使用的持久化文件。
    let path = snapshot_path("selection-modes");
    // 已协调四个桌面项目的工作区状态。
    let state = workspace_state(path.clone());
    state.reconcile_items(1280.0, 720.0, &["a".to_string(), "b".to_string(), "c".to_string(), "d".to_string()]).expect("Shell 项目应可协调到工作区");
    // 当前测试用于范围选择的围栏标识。
    let fence_id = state.apply(WorkspaceCommand::CreateFence { title: "范围".to_string(), bounds: WorkspaceRect { left: 100.0, top: 80.0, width: 320.0, height: 240.0 } }).expect("围栏应可创建").fences[0].id.clone();
    for item_id in ["b", "c", "d"]
    {
        state.apply(WorkspaceCommand::MoveItemToFence { item_id: item_id.to_string(), fence_id: fence_id.clone(), before_item_id: None }).expect("项目应可加入范围选择围栏");
    }

    state.apply_interaction(WorkspaceInteractionCommand::FocusItem { item_id: "b".to_string(), selection_mode: WorkspaceSelectionMode::Replace }).expect("范围锚点应可建立");
    // 同一围栏中的 Shift 范围选择结果。
    let range = state.apply_interaction(WorkspaceInteractionCommand::FocusItem { item_id: "d".to_string(), selection_mode: WorkspaceSelectionMode::Range }).expect("同一焦点容器应可执行范围选择");
    assert_eq!(range.interaction.selected_item_ids, ["b", "c", "d"]);
    assert_eq!(range.interaction.selection_anchor_id.as_deref(), Some("b"));

    // Ctrl 切换目标后的选择集合。
    let toggled = state.apply_interaction(WorkspaceInteractionCommand::FocusItem { item_id: "c".to_string(), selection_mode: WorkspaceSelectionMode::Toggle }).expect("焦点项目应可切换选择状态");
    assert_eq!(toggled.interaction.selected_item_ids, ["b", "d"]);
    assert_eq!(toggled.interaction.selection_anchor_id.as_deref(), Some("c"));

    // Ctrl+方向键只移动焦点并保留现有选择。
    let preserved = state.apply_interaction(WorkspaceInteractionCommand::FocusItem { item_id: "d".to_string(), selection_mode: WorkspaceSelectionMode::Preserve }).expect("项目焦点应可独立移动");
    assert_eq!(preserved.interaction.selected_item_ids, ["b", "d"]);

    // 项目焦点上的 Ctrl+A 保留当前项目作为选择锚点。
    let selected_container = state.apply_interaction(WorkspaceInteractionCommand::SelectFocusContainer).expect("当前项目焦点容器应可全选");
    assert_eq!(selected_container.interaction.selected_item_ids, ["b", "c", "d"]);
    assert_eq!(selected_container.interaction.selection_anchor_id.as_deref(), Some("d"));

    // 跨焦点容器的 Shift 选择收束为目标单项。
    let reset = state.apply_interaction(WorkspaceInteractionCommand::FocusItem { item_id: "a".to_string(), selection_mode: WorkspaceSelectionMode::Range }).expect("跨容器范围选择应建立新锚点");
    assert_eq!(reset.interaction.selected_item_ids, ["a"]);
    assert_eq!(reset.interaction.selection_anchor_id.as_deref(), Some("a"));

    std::fs::remove_file(path).expect("测试快照应可清理");
}

/// 验证框选、高层焦点、容器全选和围栏折叠协调公开交互状态。
#[test]
fn surface_focus_and_fence_collapse_coordinate_selection()
{
    // 当前测试使用的持久化文件。
    let path = snapshot_path("surface-focus");
    // 已协调三个桌面项目的工作区状态。
    let state = workspace_state(path.clone());
    state.reconcile_items(1280.0, 720.0, &["a".to_string(), "b".to_string(), "c".to_string()]).expect("Shell 项目应可协调到工作区");
    // 当前测试使用的围栏标识。
    let fence_id = state.apply(WorkspaceCommand::CreateFence { title: "焦点".to_string(), bounds: WorkspaceRect { left: 100.0, top: 80.0, width: 320.0, height: 240.0 } }).expect("围栏应可创建").fences[0].id.clone();
    for item_id in ["b", "c"]
    {
        state.apply(WorkspaceCommand::MoveItemToFence { item_id: item_id.to_string(), fence_id: fence_id.clone(), before_item_id: None }).expect("项目应可加入围栏");
    }

    // 框选按视觉顺序发布的选择、锚点和焦点。
    let marquee = state.apply_interaction(WorkspaceInteractionCommand::SelectItems { item_ids: vec!["a".to_string(), "c".to_string()] }).expect("框选结果应可提交");
    assert_eq!(marquee.interaction.selected_item_ids, ["a", "c"]);
    assert_eq!(marquee.interaction.selection_anchor_id.as_deref(), Some("a"));

    // 围栏标题焦点清空项目选择。
    let fence_focus = state.apply_interaction(WorkspaceInteractionCommand::FocusSurface { focus: WorkspaceFocusTarget::Fence { fence_id: fence_id.clone() } }).expect("围栏标题应可获得高层焦点");
    assert!(fence_focus.interaction.selected_item_ids.is_empty());
    // 围栏标题上的 Ctrl+A 选择全部成员。
    let selected_fence = state.apply_interaction(WorkspaceInteractionCommand::SelectFocusContainer).expect("围栏焦点容器应可全选");
    assert_eq!(selected_fence.interaction.selected_item_ids, ["b", "c"]);

    state.apply_interaction(WorkspaceInteractionCommand::FocusItem { item_id: "b".to_string(), selection_mode: WorkspaceSelectionMode::Replace }).expect("围栏成员应可获得焦点");
    // 折叠围栏后的权威交互快照。
    let collapsed = state.apply(WorkspaceCommand::SetFenceCollapsed { fence_id: fence_id.clone(), collapsed: true }).expect("围栏应可折叠");
    assert!(collapsed.interaction.selected_item_ids.is_empty());
    assert!(collapsed.interaction.selection_anchor_id.is_none());
    assert_eq!(collapsed.interaction.keyboard_focus, WorkspaceFocusTarget::Fence { fence_id: fence_id.clone() });

    // 桌面背景焦点上的 Ctrl+A 只选择未归类区域。
    state.apply_interaction(WorkspaceInteractionCommand::FocusSurface { focus: WorkspaceFocusTarget::Background }).expect("桌面背景应可获得焦点");
    let selected_background = state.apply_interaction(WorkspaceInteractionCommand::SelectFocusContainer).expect("未归类焦点容器应可全选");
    assert_eq!(selected_background.interaction.selected_item_ids, ["a"]);

    std::fs::remove_file(path).expect("测试快照应可清理");
}

/// 验证旧位置只由撤销恢复，普通移出和解散使用当前围栏位置。
#[test]
fn membership_positions_exist_only_in_undo_history()
{
    // 当前测试使用的持久化文件。
    let path = snapshot_path("membership");
    // 模拟当前应用会话的工作区状态。
    let state = workspace_state(path.clone());
    // 宿主网格与 Shell 项目形成的单次原子协调事务。
    let reconciled = state.reconcile_items(1280.0, 720.0, &["::{occupied}".to_string(), "::{project}".to_string(), "::{document}".to_string()]).expect("Shell 项目应可协调到未归类网格");
    assert_eq!(reconciled.revision, 1);
    assert_eq!(reconciled.grid.rows, 7);
    assert_eq!(reconciled.unclassified_items.len(), 3);
    // 新围栏的权威标识。
    let fence_id = state.apply(WorkspaceCommand::CreateFence { title: "资料".to_string(), bounds: WorkspaceRect { left: 100.0, top: 80.0, width: 320.0, height: 240.0 } }).expect("围栏应可创建").fences[0].id.clone();
    // 桌面项目进入围栏前的来源位置。
    let source_position = state.snapshot().expect("协调快照应可读取").unclassified_items.iter().find(|item| item.stable_reference == "::{project}").expect("项目应拥有未归类位置").position;

    // 首次加入围栏后的权威快照。
    let snapshot = state.apply(WorkspaceCommand::MoveItemToFence { item_id: "::{project}".to_string(), fence_id: fence_id.clone(), before_item_id: None }).expect("桌面项目应可加入围栏");
    assert_eq!(snapshot.fences[0].members, ["::{project}"]);
    assert!(snapshot.unclassified_items.iter().all(|item| item.stable_reference != "::{project}"), "旧桌面位置只存在于撤销历史");
    let undone = state.apply(WorkspaceCommand::UndoLayout).expect("进入围栏应可撤销"); // 历史恢复实际来源位置。
    assert_eq!(undone.unclassified_items.iter().find(|item| item.stable_reference == "::{project}").unwrap().position, source_position);
    let snapshot = state.apply(WorkspaceCommand::RedoLayout).expect("进入围栏应可重做"); // 重做再次释放来源格。
    // 项目释放到自身时保持当前成员顺序和修订。
    let unchanged = state.apply(WorkspaceCommand::MoveItemToFence { item_id: "::{project}".to_string(), fence_id: fence_id.clone(), before_item_id: Some("::{project}".to_string()) }).expect("释放到自身应保持当前归属");
    assert_eq!(unchanged, snapshot);

    // 项目移出围栏后的权威快照。
    let snapshot = state.apply(WorkspaceCommand::RemoveItemFromFence { item_id: "::{project}".to_string() }).expect("桌面项目应可移出围栏");
    assert!(snapshot.fences[0].members.is_empty());
    assert_eq!(snapshot.unclassified_items.iter().find(|item| item.stable_reference == "::{project}").expect("移出项目应靠近当前围栏").position, WorkspacePoint { left: 78.0, top: 98.0 });

    state.apply(WorkspaceCommand::MoveItemToFence { item_id: "::{project}".to_string(), fence_id: fence_id.clone(), before_item_id: None }).expect("桌面项目应可再次加入围栏");
    state.apply(WorkspaceCommand::MoveItemToFence { item_id: "::{document}".to_string(), fence_id: fence_id.clone(), before_item_id: None }).expect("第二个桌面项目应可加入围栏");
    // 围栏解散并避让已有未归类项目后的权威快照。
    let snapshot = state.apply(WorkspaceCommand::DissolveFence { fence_id }).expect("围栏应可解散");
    assert!(snapshot.fences.is_empty());
    assert_eq!(snapshot.unclassified_items.iter().find(|item| item.stable_reference == "::{project}").expect("解散后项目应回到未归类区域").position, crate::contracts::WorkspacePoint { left: 78.0, top: 98.0 });
    assert_eq!(snapshot.unclassified_items.iter().find(|item| item.stable_reference == "::{document}").expect("解散后第二个项目应回到未归类区域").position, crate::contracts::WorkspacePoint { left: 78.0, top: 196.0 });

    std::fs::remove_file(path).expect("测试快照应可清理");
}

/// 验证未归类项目可以占用围栏成员释放出的视觉空网格。
#[test]
fn free_drop_uses_the_cell_released_by_a_fence_member()
{
    let path = snapshot_path("fence-source-collision");
    let state = workspace_state(path.clone());
    state.reconcile_items(1280.0, 720.0, &["a".to_string(), "b".to_string()]).expect("Shell 项目应可协调到工作区");
    let fence_id = state.apply(WorkspaceCommand::CreateFence { title: "资料".to_string(), bounds: WorkspaceRect { left: 100.0, top: 80.0, width: 320.0, height: 240.0 } }).expect("围栏应可创建").fences[0].id.clone();
    let source_position = state.snapshot().expect("快照应可读取").unclassified_items.iter().find(|item| item.stable_reference == "a").expect("围栏成员应拥有来源位置").position;
    state.apply(WorkspaceCommand::MoveItemToFence { item_id: "a".to_string(), fence_id, before_item_id: None }).expect("项目应可进入围栏");
    let snapshot = state.apply(WorkspaceCommand::CommitItemDrag { anchor_item_id: "b".to_string(), item_ids: vec!["b".to_string()], placements: vec![WorkspaceItemPlacement { item_id: "b".to_string(), position: source_position }], target_fence_id: None, before_item_id: None, fence_scrolls: Vec::new() }).expect("空白网格应可放置");
    let positions = snapshot.unclassified_items.iter().map(|item| (item.position.left.to_bits(), item.position.top.to_bits())).collect::<std::collections::HashSet<_>>();
    assert_eq!(positions.len(), snapshot.unclassified_items.len());
    assert_eq!(snapshot.unclassified_items.len(), 1); // 原成员没有隐藏来源位置。
    assert_eq!(snapshot.unclassified_items.iter().find(|item| item.stable_reference == "b").expect("项目 b 应存在").position, source_position);
    std::fs::remove_file(path).expect("测试快照应可清理");
}

/// 验证围栏几何、折叠、布局和滚动状态形成完整持久化事务。
#[test]
fn fence_state_commands_persist_the_complete_visual_state()
{
    // 当前测试使用的持久化文件。
    let path = snapshot_path("fence-state");
    // 模拟当前应用会话的工作区状态。
    let state = workspace_state(path.clone());
    // 新围栏的权威标识。
    let fence_id = state.apply(WorkspaceCommand::CreateFence { title: "任务".to_string(), bounds: WorkspaceRect { left: 40.0, top: 40.0, width: 320.0, height: 240.0 } }).expect("围栏应可创建").fences[0].id.clone();
    // 移动和缩放后提交的围栏边界。
    let bounds = WorkspaceRect { left: 180.0, top: 120.0, width: 420.0, height: 300.0 };
    state.apply(WorkspaceCommand::UpdateFenceBounds { fence_id: fence_id.clone(), bounds }).expect("围栏边界应可更新");
    state.apply(WorkspaceCommand::SetFenceLayout { fence_id: fence_id.clone(), layout_mode: crate::contracts::FenceLayoutMode::List }).expect("围栏布局应可更新");
    state.apply(WorkspaceCommand::SetFenceScroll { fence_id: fence_id.clone(), scroll_top: 72.0 }).expect("围栏滚动应可更新");
    // 围栏全部视觉状态提交后的权威快照。
    let snapshot = state.apply(WorkspaceCommand::SetFenceCollapsed { fence_id, collapsed: true }).expect("围栏应可折叠");

    assert_eq!(snapshot.fences[0].bounds, bounds);
    assert_eq!(snapshot.fences[0].last_expanded_height, 300.0);
    assert_eq!(snapshot.fences[0].layout_mode, crate::contracts::FenceLayoutMode::List);
    assert_eq!(snapshot.fences[0].scroll_top, 72.0);
    assert!(snapshot.fences[0].collapsed);

    std::fs::remove_file(path).expect("测试快照应可清理");
}

/// 验证组拖动形成一个可完整撤销和重做的布局事务。
#[test]
fn group_drag_is_one_undoable_layout_transaction()
{
    // 当前测试使用的持久化文件。
    let path = snapshot_path("group-drag-history");
    // 已协调三个项目的当前应用会话工作区。
    let state = workspace_state(path.clone());
    state.reconcile_items(1280.0, 720.0, &["a".to_string(), "b".to_string(), "c".to_string()]).expect("Shell 项目应可协调到工作区");
    // 当前组拖动使用的目标围栏标识。
    let fence_id = state.apply(WorkspaceCommand::CreateFence { title: "拖动".to_string(), bounds: WorkspaceRect { left: 120.0, top: 80.0, width: 320.0, height: 240.0 } }).expect("目标围栏应可创建").fences[0].id.clone();
    state.apply_interaction(WorkspaceInteractionCommand::SelectItems { item_ids: vec!["a".to_string(), "b".to_string()] }).expect("组拖动选择应可建立");

    // 两个项目原子进入围栏后的权威快照。
    let committed = state.apply(WorkspaceCommand::CommitItemDrag { anchor_item_id: "a".to_string(), item_ids: vec!["a".to_string(), "b".to_string()], placements: Vec::new(), target_fence_id: Some(fence_id.clone()), before_item_id: None, fence_scrolls: Vec::new() }).expect("组拖动应可原子提交");
    assert_eq!(committed.fences[0].members, ["a", "b"]);
    assert_eq!(committed.interaction.selected_item_ids, ["a", "b"]);
    let stale = state.apply_on_monitor_at_revision(None, WorkspaceCommand::CommitItemDrag { anchor_item_id: "c".into(), item_ids: vec!["c".into()], placements: Vec::new(), target_fence_id: Some(fence_id.clone()), before_item_id: None, fence_scrolls: Vec::new() }, Some(committed.revision - 1)); // 校验后发生其他布局事务时，旧几何不能再提交。
    assert!(stale.is_err());
    assert_eq!(state.snapshot().unwrap(), committed);

    // 撤销组拖动后的完整布局。
    let undone = state.apply(WorkspaceCommand::UndoLayout).expect("组拖动应可撤销");
    assert!(undone.fences[0].members.is_empty());
    assert_eq!(undone.interaction.selected_item_ids, ["a", "b"]);

    // 重做组拖动后的完整布局。
    let redone = state.apply(WorkspaceCommand::RedoLayout).expect("组拖动应可重做");
    assert_eq!(redone.fences[0].members, ["a", "b"]);
    assert_eq!(redone.interaction.keyboard_focus, WorkspaceFocusTarget::Item { item_id: "a".to_string() });

    let appended = state.apply(WorkspaceCommand::CommitItemDrag { anchor_item_id: "c".into(), item_ids: vec!["c".into()], placements: Vec::new(), target_fence_id: Some(fence_id), before_item_id: Some("removed-successor".into()), fence_scrolls: Vec::new() }).expect("释放后后继消失应追加而不丢掉整组"); // 并发后继失效仍按实际提交顺序追加。
    assert_eq!(appended.fences[0].members, ["a", "b", "c"]);
    assert_eq!(state.apply(WorkspaceCommand::UndoLayout).unwrap().fences[0].members, ["a", "b"]);

    std::fs::remove_file(path).expect("测试快照应可清理");
}

/// 验证 Shell 精确重命名同步布局成员、选择、锚点和键盘焦点。
#[test]
fn shell_rename_remaps_all_runtime_item_references()
{
    let path = snapshot_path("shell-rename"); // 当前测试使用的持久化文件。
    let state = workspace_state(path.clone()); // 模拟当前应用会话的工作区状态。
    state.reconcile_items(1280.0, 720.0, &["old".to_string()]).expect("原项目应可协调到工作区");
    let fence_id = state.apply(WorkspaceCommand::CreateFence { title: "重命名".to_string(), bounds: WorkspaceRect { left: 100.0, top: 80.0, width: 320.0, height: 240.0 } }).expect("围栏应可创建").fences[0].id.clone(); // 当前项目所属围栏标识。
    state.apply(WorkspaceCommand::MoveItemToFence { item_id: "old".to_string(), fence_id, before_item_id: None }).expect("原项目应可加入围栏");
    state.apply_interaction(WorkspaceInteractionCommand::FocusItem { item_id: "old".to_string(), selection_mode: WorkspaceSelectionMode::Replace }).expect("原项目应可建立选择和焦点");

    let renamed = state.reconcile_shell_items(1280.0, 720.0, &["new".to_string()], &[("old".to_string(), "new".to_string())]).expect("Shell 重命名事实应可协调"); // 精确重命名和最终枚举共同产生的快照。
    assert_eq!(renamed.fences[0].members, ["new"]);
    assert_eq!(renamed.interaction.selected_item_ids, ["new"]);
    assert_eq!(renamed.interaction.selection_anchor_id.as_deref(), Some("new"));
    assert_eq!(renamed.interaction.keyboard_focus, WorkspaceFocusTarget::Item { item_id: "new".to_string() });

    std::fs::remove_file(path).expect("测试快照应可清理");
}

/// Shell 新项目协调不能使用遗留选择或围栏焦点改变逻辑归属。
#[test]
fn shell_new_item_does_not_inherit_fence_selection()
{
    let path = snapshot_path("shell-new-item-fence-selection"); // 隔离本次持久状态。
    let state = workspace_state(path.clone()); // 使用真实工作区协调入口。
    state.reconcile_items(1280.0, 720.0, &["old".into()]).unwrap();
    let fence_id = state.apply(WorkspaceCommand::CreateFence { title: "测试".into(), bounds: WorkspaceRect { left: 100.0, top: 80.0, width: 320.0, height: 240.0 } }).unwrap().fences[0].id.clone(); // 已有投放目标。
    state.apply(WorkspaceCommand::MoveItemToFence { item_id: "old".into(), fence_id: fence_id.clone(), before_item_id: None }).unwrap();
    state.apply_interaction(WorkspaceInteractionCommand::FocusItem { item_id: "old".into(), selection_mode: WorkspaceSelectionMode::Replace }).unwrap();
    let next = state.reconcile_shell_items(1280.0, 720.0, &["old".into(), "new".into()], &[]).unwrap(); // 无真实投放、无重命名映射的 Shell 新建通知。
    assert!(next.unclassified_items.iter().any(|item| item.stable_reference == "new"));
    assert!(next.fences.iter().all(|fence| !fence.members.iter().any(|item| item == "new")));
    assert_eq!(next.fences.iter().find(|fence| fence.id == fence_id).unwrap().members, ["old"]);
    drop(state);
    std::fs::remove_file(&path).unwrap();
    if path.with_extension("snapshots").exists()
    {
        std::fs::remove_dir_all(path.with_extension("snapshots")).unwrap();
    }
}

/// 已确认离开桌面的旧成员不能让同路径新文件继承围栏归属。
#[test]
fn recreated_shell_item_does_not_restore_deleted_membership()
{
    let path = snapshot_path("shell-recreated-membership"); // 独立回归快照。
    let state = workspace_state(path.clone()); // 实际工作区协调入口。
    let item_id = "新建文本文档.txt".to_string(); // Windows 默认名称会重复使用的稳定路径。
    state.reconcile_items(1280.0, 720.0, std::slice::from_ref(&item_id)).unwrap();
    let fence_id = state.apply(WorkspaceCommand::CreateFence { title: "测试".into(), bounds: WorkspaceRect { left: 100.0, top: 80.0, width: 320.0, height: 240.0 } }).unwrap().fences[0].id.clone(); // 原文件所属围栏。
    state.apply(WorkspaceCommand::MoveItemToFence { item_id: item_id.clone(), fence_id, before_item_id: None }).unwrap();
    let removed = state.reconcile_shell_items(1280.0, 720.0, &[], &[]).unwrap(); // 成功完整枚举确认原文件已离开桌面。
    assert!(removed.fences.iter().all(|fence| !fence.members.contains(&item_id)), "已删除的成员必须退出活动围栏");
    assert!(removed.unclassified_items.iter().all(|item| item.stable_reference != item_id));
    let recreated = state.reconcile_shell_items(1280.0, 720.0, std::slice::from_ref(&item_id), &[]).unwrap(); // 不含重命名或投放命令的同名新建。
    assert!(recreated.fences.iter().all(|fence| !fence.members.contains(&item_id)));
    assert!(recreated.unclassified_items.iter().any(|item| item.stable_reference == item_id));
    drop(state);
    std::fs::remove_file(&path).unwrap();
    if path.with_extension("snapshots").exists()
    {
        std::fs::remove_dir_all(path.with_extension("snapshots")).unwrap();
    }
}

/// 验证重命名补偿后的精确引用仍能撤销先前的布局移动。
#[test]
fn rename_compensation_preserves_prior_layout_undo()
{
    let path = snapshot_path("rename-layout-undo"); // 当前测试布局。
    let state = workspace_state(path.clone()); // 同一会话的历史集合。
    let initial = state.reconcile_items(1280.0, 720.0, &["old".into()]).unwrap(); // 原始来源位置。
    state.apply(WorkspaceCommand::CommitItemDrag { anchor_item_id: "old".into(), item_ids: vec!["old".into()], placements: vec![WorkspaceItemPlacement { item_id: "old".into(), position: WorkspacePoint { left: 300.0, top: 200.0 } }], target_fence_id: None, before_item_id: None, fence_scrolls: vec![] }).unwrap();
    state.reconcile_shell_items(1280.0, 720.0, &["new".into()], &[("old".into(), "new".into())]).unwrap();
    state.reconcile_shell_items(1280.0, 720.0, &["old".into()], &[("new".into(), "old".into())]).unwrap();
    let restored = state.restore_layout_history(false).unwrap(); // 撤销重命名后再撤销布局。
    assert_eq!(restored.snapshot.unclassified_items[0].position, initial.unclassified_items[0].position);
    assert_eq!(restored.snapshot.unclassified_items[0].stable_reference, "old");
    drop(state);
    std::fs::remove_file(&path).unwrap();
    std::fs::remove_dir_all(path.with_extension("snapshots")).unwrap();
}

/// 验证 Explorer 初始归属、断开投射、重连恢复和 DPI 变化保持持久显示器。
#[test]
fn monitor_reconciliation_preserves_persistent_assignment()
{
    let path = snapshot_path("monitor-reconciliation"); // 当前测试使用的持久化文件。
    let state = workspace_state(path.clone()); // 模拟首次逐屏启动的工作区状态。
    let primary = monitor("DISPLAY1", true, 0, 1.0); // 当前主显示器。
    let secondary = monitor("DISPLAY2", false, 1920, 1.5); // 当前混合 DPI 次显示器。
    let initial = state.reconcile_host_monitors(&[primary.clone(), secondary.clone()], &["primary-item".to_string(), "secondary-item".to_string()], &[("secondary-item".to_string(), "DISPLAY2".to_string())], &[]).expect("Explorer 项目归属应可首次导入");
    let secondary_position = initial.unclassified_items.iter().find(|item| item.stable_reference == "secondary-item").expect("次屏项目应存在").position; // 次屏项目持久 DIP 位置。
    assert_eq!(initial.unclassified_items[1].monitor_id, "DISPLAY2");

    let projected = state.reconcile_host_monitors(std::slice::from_ref(&primary), &["primary-item".to_string(), "secondary-item".to_string()], &[], &[]).expect("显示器断开后项目应可临时投射");
    let projected_item = projected.unclassified_items.iter().find(|item| item.stable_reference == "secondary-item").expect("投射项目应存在");
    assert_eq!(projected_item.monitor_id, "DISPLAY2");
    assert_eq!(projected_item.projection_monitor_id.as_deref(), Some("DISPLAY1"));
    assert_eq!(projected_item.position, secondary_position);

    let mut scaled_secondary = secondary; // DPI-only 重建后的次显示器。
    scaled_secondary.scale_factor = 2.0;
    scaled_secondary.dpi = 192;
    let restored = state.reconcile_host_monitors(&[primary, scaled_secondary], &["primary-item".to_string(), "secondary-item".to_string()], &[], &[]).expect("显示器重连和 DPI 变化应可协调");
    let restored_item = restored.unclassified_items.iter().find(|item| item.stable_reference == "secondary-item").expect("恢复项目应存在");
    assert_eq!(restored_item.monitor_id, "DISPLAY2");
    assert_eq!(restored_item.projection_monitor_id, None);
    assert_eq!(restored_item.position, secondary_position);

    std::fs::remove_file(path).expect("测试快照应可清理");
}

/// 验证跨屏围栏和项目拖动在一个布局事务中改写目标显示器归属。
#[test]
fn cross_monitor_layout_commands_use_the_target_monitor()
{
    let path = snapshot_path("cross-monitor-command"); // 当前测试使用的持久化文件。
    let state = workspace_state(path.clone()); // 当前逐屏布局状态。
    let monitors = [monitor("DISPLAY1", true, 0, 1.0), monitor("DISPLAY2", false, 1920, 1.5)]; // 当前双屏混合 DPI 环境。
    state.reconcile_host_monitors(&monitors, &["item".to_string()], &[], &[]).expect("项目应可初始化");
    let fence = state.apply_on_monitor(Some("DISPLAY2"), WorkspaceCommand::CreateFence { title: "次屏围栏".to_string(), bounds: WorkspaceRect { left: 80.0, top: 80.0, width: 320.0, height: 240.0 } }).expect("围栏应在目标显示器创建");
    assert_eq!(fence.fences[0].monitor_id, "DISPLAY2");

    let moved = state.apply_on_monitor(Some("DISPLAY2"), WorkspaceCommand::CommitItemDrag { anchor_item_id: "item".to_string(), item_ids: vec!["item".to_string()], placements: vec![WorkspaceItemPlacement { item_id: "item".to_string(), position: WorkspacePoint { left: 112.0, top: 172.0 } }], target_fence_id: None, before_item_id: None, fence_scrolls: Vec::new() }).expect("项目应可跨屏提交");
    assert_eq!(moved.unclassified_items[0].monitor_id, "DISPLAY2");
    assert_eq!(moved.unclassified_items[0].position.left, 112.0);

    std::fs::remove_file(path).expect("测试快照应可清理");
}

/// 验证当前备份格式拒绝被修改但未更新校验值的载荷。
#[test]
fn current_backup_rejects_tampered_payload()
{
    let store = workspace_store_from_payload(PersistedWorkspacePayload {
        layout: WorkspaceSnapshot::default(), // 唯一布局。
        explorer_layout_imported: true,
    })
    .expect("当前备份应可创建"); // 已计算完整性校验值的原始备份。
    let mut value = serde_json::to_value(store).expect("备份应可序列化"); // 等待模拟损坏的 JSON 载荷。
    value["payload"]["layout"]["revision"] = serde_json::json!(9);

    let error = decode_workspace_store(value).expect_err("校验值未同步的备份应被拒绝");

    assert_eq!(error.code, "workspace-restore-failed");
    assert!(error.message.contains("完整性校验失败"));
}

/// 验证旧多布局与单布局格式均被拒绝，不执行兼容读取。
#[test]
fn legacy_workspace_formats_are_rejected()
{
    for version in 1..5
    {
        let value = serde_json::json!({"storeVersion": version}); // 所有旧版本直接拒绝。
        assert!(decode_workspace_store(value).is_err());
    }
    assert!(decode_workspace_store(serde_json::to_value(WorkspaceSnapshot::default()).unwrap()).is_err());
}

/// 验证统一工作区的自动快照只保留最近 20 份。
#[test]
fn automatic_snapshots_keep_the_latest_twenty()
{
    let directory = snapshot_path("auto-snapshot-limit").with_extension("snapshots"); // 当前测试独占的自动快照目录。
    let store = workspace_store_from_payload(PersistedWorkspacePayload {
        layout: WorkspaceSnapshot::default(), // 唯一布局。
        explorer_layout_imported: true,
    })
    .expect("自动快照载荷应可创建");
    for _ in 0..AUTO_SNAPSHOT_LIMIT + 3
    {
        write_auto_snapshot(&directory, &store).expect("自动快照应可写入");
    }

    let count = std::fs::read_dir(&directory).expect("自动快照目录应可读取").count();
    assert_eq!(count, AUTO_SNAPSHOT_LIMIT);

    std::fs::remove_dir_all(directory).expect("自动快照目录应可清理");
}

/// 验证损坏备份在还原前被拒绝且当前工作区保持完整。
#[test]
fn corrupted_restore_preserves_the_current_workspace()
{
    let path = snapshot_path("restore-rollback"); // 当前测试使用的权威工作区文件。
    let backup_path = snapshot_path("restore-rollback-backup"); // 当前测试使用的手动备份文件。
    let state = workspace_state(path.clone());
    state.reconcile_items(1280.0, 720.0, &["project".to_string()]).expect("当前工作区项目应可初始化");
    state.write_manual_backup(&backup_path).expect("手动备份应可写入");
    let before = state.snapshot().expect("还原前工作区应可读取"); // 损坏还原必须保持的当前状态。
    let mut value: serde_json::Value = serde_json::from_slice(&std::fs::read(&backup_path).expect("手动备份应可读取")).expect("手动备份应为 JSON");
    value["payload"]["layout"]["revision"] = serde_json::json!(77);
    std::fs::write(&backup_path, serde_json::to_vec_pretty(&value).expect("损坏备份应可序列化")).expect("损坏备份应可写入");

    let result = state.restore_backup(&backup_path, &[monitor("DISPLAY1", true, 0, 1.0)], &["project".to_string()]);

    assert_eq!(result.expect_err("损坏备份应被拒绝").code, "workspace-restore-failed");
    assert_eq!(state.snapshot().expect("失败后工作区应可读取"), before);

    drop(state);
    std::fs::remove_file(path).expect("测试工作区应可清理");
    std::fs::remove_file(backup_path).expect("测试备份应可清理");
}

/// 验证布局历史跳过已离开项目并保留历史提交后新增的项目。
#[test]
fn layout_history_merge_reports_partial_restore_and_keeps_external_items()
{
    let item = |stable_reference: &str, left: f64| UnclassifiedItemSnapshot { stable_reference: stable_reference.to_string(), monitor_id: "DISPLAY1".to_string(), projection_monitor_id: None, projection_position: None, position: WorkspacePoint { left, top: 60.0 }, z_index: left as u32 }; // 当前测试构造稳定项目位置的最小工厂。
    let target = WorkspaceSnapshot { unclassified_items: vec![item("existing", 20.0), item("removed", 112.0)], ..WorkspaceSnapshot::default() }; // 历史事务原本覆盖的布局。
    let current = WorkspaceSnapshot { unclassified_items: vec![item("existing", 204.0), item("created", 296.0)], ..WorkspaceSnapshot::default() }; // 外部删除和新增后的当前布局。

    let (merged, restored_count, changed_count) = merge_layout_history_target(target, &current);

    assert_eq!(restored_count, 1);
    assert_eq!(changed_count, 1);
    assert!(merged.unclassified_items.iter().any(|item| item.stable_reference == "created"));
    assert!(merged.unclassified_items.iter().all(|item| item.stable_reference != "removed"));
}
