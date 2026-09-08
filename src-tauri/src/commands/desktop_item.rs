use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, State};

use crate::contracts::{
    DESKTOP_ITEM_SNAPSHOT_CHANGED_EVENT, DesktopItemError, DesktopItemSnapshot, FENCE_MIN_HEIGHT,
    FENCE_MIN_WIDTH, ShellOperationResult, WORKSPACE_SNAPSHOT_CHANGED_EVENT, WorkspaceCommand,
    WorkspaceContextMenuTarget, WorkspaceItemPlacement, WorkspaceRect, WorkspaceSnapshot,
};
use crate::host::DesktopHostRuntime;
use crate::platform::windows::ShellChangeEvent;
use crate::platform::windows::{NativeMenuCommand, NativeMenuTarget};
use crate::shell::DesktopItemState;
use crate::workspace::WorkspaceState;

use super::workspace::{reconcile_workspace_environment, reconcile_workspace_items};

/// 枚举真实 Shell 桌面项目并发布新的不可变快照。
#[tauri::command]
pub fn get_desktop_item_snapshot(
    app: AppHandle,
    state: State<'_, DesktopItemState>,
    workspace: State<'_, WorkspaceState>,
    runtime: State<'_, Arc<DesktopHostRuntime>>,
) -> Result<DesktopItemSnapshot, DesktopItemError> {
    // 已发布到内存状态的桌面项目快照。
    let snapshot = refresh_desktop_item_snapshot(&state)?;
    app.emit(DESKTOP_ITEM_SNAPSHOT_CHANGED_EVENT, &snapshot)
        .map_err(|error| {
            crate::log_error!("Shell：桌面项目快照事件发布失败，错误：{error}");
            DesktopItemError::event_publish_failed(&error)
        })?;
    // 已按正式宿主网格协调的工作区快照。
    let workspace_snapshot = reconcile_workspace_items(&workspace, &runtime, &snapshot)
        .map_err(|error| DesktopItemError::workspace_reconciliation_failed(&error))?;
    app.emit(WORKSPACE_SNAPSHOT_CHANGED_EVENT, &workspace_snapshot)
        .map_err(|error| DesktopItemError::event_publish_failed(&error))?;

    Ok(snapshot)
}

/// 使用 Shell 默认动词打开一个或多个桌面项目。
#[tauri::command]
pub fn open_desktop_items(
    state: State<'_, DesktopItemState>,
    item_ids: Vec<String>,
) -> Result<(), DesktopItemError> {
    state.open(&item_ids)
}

/// 使用 IFileOperation 重命名单个桌面项目。
#[tauri::command]
pub fn rename_desktop_item(
    state: State<'_, DesktopItemState>,
    workspace: State<'_, WorkspaceState>,
    item_id: String,
    new_name: String,
) -> Result<ShellOperationResult, DesktopItemError> {
    let result = state.rename(&item_id, new_name)?; // 已记录统一时间线的实际 Shell 结果。
    clear_layout_redo_for_shell_operation(&workspace, &result)?;
    Ok(result)
}

/// 将完整选择集合移入回收站或永久删除。
#[tauri::command]
pub fn delete_desktop_items(
    state: State<'_, DesktopItemState>,
    workspace: State<'_, WorkspaceState>,
    item_ids: Vec<String>,
    permanent: bool,
) -> Result<ShellOperationResult, DesktopItemError> {
    let result = state.delete(&item_ids, permanent)?; // 已记录统一时间线的实际 Shell 结果。
    clear_layout_redo_for_shell_operation(&workspace, &result)?;
    Ok(result)
}

/// 将完整选择集合复制或剪切到系统 Shell 剪贴板。
#[tauri::command]
pub fn set_desktop_item_clipboard(
    state: State<'_, DesktopItemState>,
    item_ids: Vec<String>,
    cut: bool,
) -> Result<ShellOperationResult, DesktopItemError> {
    state.set_clipboard(&item_ids, cut)
}

/// 将系统 Shell 剪贴板内容粘贴到桌面命名空间。
#[tauri::command]
pub fn paste_desktop_items(
    state: State<'_, DesktopItemState>,
    workspace: State<'_, WorkspaceState>,
) -> Result<ShellOperationResult, DesktopItemError> {
    let result = state.paste()?; // 已记录统一时间线的实际 Shell 粘贴结果。
    clear_layout_redo_for_shell_operation(&workspace, &result)?;
    Ok(result)
}

/// 使用完整选择集合启动系统 OLE 拖放。
#[tauri::command]
pub fn start_desktop_item_drag(
    state: State<'_, DesktopItemState>,
    item_ids: Vec<String>,
) -> Result<ShellOperationResult, DesktopItemError> {
    state.drag(&item_ids)
}

/// 在正确语义目标显示原生菜单并提交选中的产品命令。
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn show_workspace_context_menu(
    app: AppHandle,
    state: State<'_, DesktopItemState>,
    workspace: State<'_, WorkspaceState>,
    runtime: State<'_, Arc<DesktopHostRuntime>>,
    monitor_id: String,
    target: WorkspaceContextMenuTarget,
    client_x: f64,
    client_y: f64,
) -> Result<WorkspaceSnapshot, DesktopItemError> {
    // 已应用 Shell 或产品菜单动作的权威工作区快照。
    let snapshot = show_workspace_context_menu_state(
        &state,
        &workspace,
        &runtime,
        &monitor_id,
        target,
        client_x,
        client_y,
    )?;
    app.emit(WORKSPACE_SNAPSHOT_CHANGED_EVENT, &snapshot)
        .map_err(|error| DesktopItemError::event_publish_failed(&error))?;
    Ok(snapshot)
}

/// 为 Tauri 与本地 transport 共享原生菜单领域协调。
pub(crate) fn show_workspace_context_menu_state(
    state: &DesktopItemState,
    workspace: &WorkspaceState,
    runtime: &DesktopHostRuntime,
    monitor_id: &str,
    target: WorkspaceContextMenuTarget,
    client_x: f64,
    client_y: f64,
) -> Result<WorkspaceSnapshot, DesktopItemError> {
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
    /// 当前工作区已经持续启用的编辑布局命令编号。
    const EDIT_LAYOUT: u32 = 0x7805;
    /// 重新枚举 Shell 项目并协调工作区命令编号。
    const REFRESH_ITEMS: u32 = 0x7806;
    /// 等待正式设置表面交付后启用的设置命令编号。
    const OPEN_SETTINGS: u32 = 0x7807;

    if !client_x.is_finite() || !client_y.is_finite() {
        return Err(DesktopItemError::invalid_selection("菜单锚点坐标无效"));
    }
    // 当前宿主物理工作区与 CSS 缩放信息。
    let host_snapshot = runtime
        .snapshot()
        .map_err(|error| DesktopItemError::shell_operation_failed(&error.message))?;
    let monitor = host_snapshot
        .hosts
        .iter()
        .find(|host| host.monitor.id == monitor_id)
        .map(|host| &host.monitor)
        .ok_or_else(|| DesktopItemError::shell_operation_failed(&"当前显示器宿主不存在"))?;
    // 菜单锚点的屏幕物理横坐标。
    let screen_x = monitor.work_x + (client_x * monitor.scale_factor).round() as i32;
    // 菜单锚点的屏幕物理纵坐标。
    let screen_y = monitor.work_y + (client_y * monitor.scale_factor).round() as i32;
    // 菜单打开前的当前权威工作区快照。
    let snapshot = workspace
        .snapshot()
        .map_err(|error| DesktopItemError::shell_operation_failed(&error))?;

    // 当前菜单目标对应的原生 Shell 目标、产品命令和项目选择。
    let (native_target, product_commands, item_ids, fence_id) = match target {
        WorkspaceContextMenuTarget::Items { item_ids } => {
            // 已通过当前 Shell 快照校验的多选稳定引用。
            let references = state.selection_references(&item_ids)?;
            // 当前选择集合全部成员共同支持的产品命令。
            let mut commands = snapshot
                .fences
                .iter()
                .enumerate()
                .map(|(index, fence)| NativeMenuCommand {
                    id: MOVE_TO_FENCE_FIRST.saturating_add(index as u32),
                    label: format!("移动到围栏：{}", fence.title),
                    enabled: item_ids
                        .iter()
                        .any(|item_id| !fence.members.contains(item_id)),
                })
                .collect::<Vec<_>>();
            // 当前完整选择集合是否都具有移出围栏能力。
            let can_remove = item_ids.iter().all(|item_id| {
                snapshot
                    .fences
                    .iter()
                    .any(|fence| fence.members.contains(item_id))
            });
            commands.push(NativeMenuCommand {
                id: REMOVE_FROM_FENCE,
                label: "移出围栏".to_string(),
                enabled: can_remove,
            });
            commands.push(NativeMenuCommand {
                id: EDIT_LAYOUT,
                label: "编辑布局".to_string(),
                enabled: true,
            });
            (
                NativeMenuTarget::Items(references),
                commands,
                item_ids,
                None,
            )
        }
        WorkspaceContextMenuTarget::Fence { fence_id } => {
            // 当前菜单目标围栏权威快照。
            let fence = snapshot
                .fences
                .iter()
                .find(|fence| fence.id == fence_id)
                .ok_or_else(|| DesktopItemError::invalid_selection("围栏不存在"))?;
            (
                NativeMenuTarget::ProductOnly,
                vec![
                    NativeMenuCommand {
                        id: TOGGLE_FENCE,
                        label: if fence.collapsed {
                            "展开围栏"
                        } else {
                            "折叠围栏"
                        }
                        .to_string(),
                        enabled: true,
                    },
                    NativeMenuCommand {
                        id: TOGGLE_FENCE_LAYOUT,
                        label: if fence.layout_mode == crate::contracts::FenceLayoutMode::Tile {
                            "切换为列表"
                        } else {
                            "切换为平铺"
                        }
                        .to_string(),
                        enabled: !fence.collapsed,
                    },
                    NativeMenuCommand {
                        id: DISSOLVE_FENCE,
                        label: "解散围栏".to_string(),
                        enabled: true,
                    },
                ],
                Vec::new(),
                Some(fence_id),
            )
        }
        WorkspaceContextMenuTarget::Background => (
            NativeMenuTarget::Background,
            vec![
                NativeMenuCommand {
                    id: CREATE_FENCE,
                    label: "新建围栏".to_string(),
                    enabled: true,
                },
                NativeMenuCommand {
                    id: EDIT_LAYOUT,
                    label: "编辑布局".to_string(),
                    enabled: true,
                },
                NativeMenuCommand {
                    id: REFRESH_ITEMS,
                    label: "刷新桌面项目".to_string(),
                    enabled: true,
                },
                NativeMenuCommand {
                    id: OPEN_SETTINGS,
                    label: "打开设置".to_string(),
                    enabled: false,
                },
            ],
            Vec::new(),
            None,
        ),
    };
    // 原生菜单关闭后返回的可选产品命令编号。
    let selected = state.show_menu(native_target, screen_x, screen_y, product_commands)?;
    let Some(selected) = selected else {
        return Ok(snapshot);
    };
    if selected == EDIT_LAYOUT {
        crate::log_info!("工作区：编辑布局命令已激活");
        return Ok(snapshot);
    }
    if selected == REFRESH_ITEMS {
        let item_snapshot = state.refresh()?; // 用户显式刷新得到的当前 Shell 项目集合。
        return reconcile_workspace_environment(workspace, runtime, &item_snapshot, true, &[])
            .map_err(|error| DesktopItemError::workspace_reconciliation_failed(&error));
    }

    // 产品菜单动作提交后的单一布局事务。
    let command = if (MOVE_TO_FENCE_FIRST..REMOVE_FROM_FENCE).contains(&selected) {
        // 动态移动命令编号对应的目标围栏。
        let target_fence = snapshot
            .fences
            .get((selected - MOVE_TO_FENCE_FIRST) as usize)
            .ok_or_else(|| DesktopItemError::invalid_selection("目标围栏不存在"))?;
        WorkspaceCommand::CommitItemDrag {
            anchor_item_id: item_ids[0].clone(),
            item_ids,
            placements: Vec::new(),
            target_fence_id: Some(target_fence.id.clone()),
            before_item_id: None,
            fence_scrolls: Vec::new(),
        }
    } else {
        match selected {
            REMOVE_FROM_FENCE => WorkspaceCommand::CommitItemDrag {
                anchor_item_id: item_ids[0].clone(),
                item_ids: item_ids.clone(),
                placements: snapshot
                    .unclassified_items
                    .iter()
                    .filter(|item| item_ids.contains(&item.stable_reference))
                    .map(|item| WorkspaceItemPlacement {
                        item_id: item.stable_reference.clone(),
                        position: item.position,
                    })
                    .collect(),
                target_fence_id: None,
                before_item_id: None,
                fence_scrolls: Vec::new(),
            },
            TOGGLE_FENCE => {
                let fence_id = fence_id.expect("围栏菜单命令包含目标围栏");
                let fence = snapshot
                    .fences
                    .iter()
                    .find(|fence| fence.id == fence_id)
                    .expect("围栏菜单目标已完成校验");
                WorkspaceCommand::SetFenceCollapsed {
                    fence_id,
                    collapsed: !fence.collapsed,
                }
            }
            TOGGLE_FENCE_LAYOUT => {
                let fence_id = fence_id.expect("围栏菜单命令包含目标围栏");
                let fence = snapshot
                    .fences
                    .iter()
                    .find(|fence| fence.id == fence_id)
                    .expect("围栏菜单目标已完成校验");
                WorkspaceCommand::SetFenceLayout {
                    fence_id,
                    layout_mode: if fence.layout_mode == crate::contracts::FenceLayoutMode::Tile {
                        crate::contracts::FenceLayoutMode::List
                    } else {
                        crate::contracts::FenceLayoutMode::Tile
                    },
                }
            }
            DISSOLVE_FENCE => WorkspaceCommand::DissolveFence {
                fence_id: fence_id.expect("围栏菜单命令包含目标围栏"),
            },
            CREATE_FENCE => {
                // 当前 CSS 工作区可容纳的新围栏初始边界。
                let viewport_width = monitor.work_width as f64 / monitor.scale_factor;
                // 当前 CSS 工作区可容纳的新围栏初始高度。
                let viewport_height = monitor.work_height as f64 / monitor.scale_factor;
                let width = FENCE_MIN_WIDTH.max(320.0_f64.min(viewport_width));
                let height = FENCE_MIN_HEIGHT.max(240.0_f64.min(viewport_height));
                WorkspaceCommand::CreateFence {
                    title: format!("新围栏 {}", snapshot.fences.len() + 1),
                    bounds: WorkspaceRect {
                        left: client_x.clamp(0.0, (viewport_width - width).max(0.0)),
                        top: client_y.clamp(0.0, (viewport_height - height).max(0.0)),
                        width,
                        height,
                    },
                }
            }
            _ => return Ok(snapshot),
        }
    };
    let before_revision = snapshot.revision; // 判断产品菜单动作是否创建新统一时间线分支的修订号。
    let next = workspace
        .apply_on_monitor(Some(monitor_id), command)
        .map_err(|error| DesktopItemError::shell_operation_failed(&error))?;
    if next.revision != before_revision {
        state.clear_redo_history()?;
    }
    Ok(next)
}

/// 枚举 transport 共享的桌面项目快照。
pub(crate) fn refresh_desktop_item_snapshot(
    state: &DesktopItemState,
) -> Result<DesktopItemSnapshot, DesktopItemError> {
    state.refresh()
}

/// 返回本地事件轮询使用的当前桌面项目快照。
pub(crate) fn read_desktop_item_snapshot(
    state: &DesktopItemState,
) -> Result<DesktopItemSnapshot, DesktopItemError> {
    state.snapshot()
}

/// 启动 250 ms 合并窗口和拖动结束门槛后的 Shell 变化协调。
pub fn start_desktop_item_change_monitor(app: AppHandle, changes: Receiver<ShellChangeEvent>) {
    thread::spawn(move || {
        while let Ok(first_change) = changes.recv() {
            // 当前合并窗口内收集的精确重命名事实。
            let mut renames = Vec::new();
            collect_shell_change(first_change, &mut renames);
            loop {
                match changes.recv_timeout(Duration::from_millis(250)) {
                    Ok(change) => collect_shell_change(change, &mut renames),
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            // 拖动期间到达的变化等待布局事务完成或取消。
            while app
                .state::<Arc<DesktopHostRuntime>>()
                .is_input_drag_active()
            {
                thread::sleep(Duration::from_millis(25));
                for change in changes.try_iter() {
                    collect_shell_change(change, &mut renames);
                }
            }

            // Shell 最终事实重新枚举后的桌面项目快照。
            let item_snapshot = match app.state::<DesktopItemState>().refresh() {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    crate::log_error!("Shell：变化后桌面项目协调失败，错误：{}", error.message);
                    continue;
                }
            };
            if let Err(error) = app
                .state::<DesktopItemState>()
                .reconcile_operation_history(&item_snapshot, &renames)
            {
                crate::log_error!("Shell：操作历史对账失败，错误：{}", error.message);
            }
            // 精确重命名和最终枚举共同产生的权威工作区快照。
            let workspace_snapshot = match reconcile_workspace_environment(
                &app.state::<WorkspaceState>(),
                &app.state::<Arc<DesktopHostRuntime>>(),
                &item_snapshot,
                true,
                &renames,
            ) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    crate::log_error!("Shell：变化协调更新工作区失败，错误：{}", error.message);
                    continue;
                }
            };
            let _ = app.emit(DESKTOP_ITEM_SNAPSHOT_CHANGED_EVENT, &item_snapshot);
            let _ = app.emit(WORKSPACE_SNAPSHOT_CHANGED_EVENT, &workspace_snapshot);
            crate::log_debug!(
                "Shell：桌面变化已协调，重命名={}，项目={}，布局修订={}",
                renames.len(),
                item_snapshot.items.len(),
                workspace_snapshot.revision
            );
        }
    });
}

/// 新 Shell 文件操作提交后清除当前布局重做分支。
pub(crate) fn clear_layout_redo_for_shell_operation(
    workspace: &WorkspaceState,
    result: &ShellOperationResult,
) -> Result<(), DesktopItemError> {
    if result.operation_id == 0 {
        return Ok(());
    }
    workspace
        .clear_redo_history()
        .map_err(|error| DesktopItemError::shell_operation_failed(&error))
}

/// 将一个 Shell 变化事件归并到精确重命名事实集合。
fn collect_shell_change(change: ShellChangeEvent, renames: &mut Vec<(String, String)>) {
    if let ShellChangeEvent::Renamed {
        old_reference,
        new_reference,
    } = change
    {
        renames.push((old_reference, new_reference));
    }
}
