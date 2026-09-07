use std::path::Path;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};

use crate::contracts::{
    DESKTOP_HOST_SNAPSHOT_CHANGED_EVENT, VIRTUAL_DESKTOP_SNAPSHOT_CHANGED_EVENT,
    VirtualDesktopArchiveSummary, VirtualDesktopError, VirtualDesktopSnapshot,
    WORKSPACE_SNAPSHOT_CHANGED_EVENT, WorkspaceError, WorkspaceRestoreResult,
};
use crate::host::DesktopHostRuntime;
use crate::platform::windows::{
    VirtualDesktopChange, VirtualDesktopEnvironment, WindowsVirtualDesktopAdapter,
};
use crate::shell::DesktopItemState;
use crate::workspace::WorkspaceState;

/// 虚拟桌面切换开始事件名称。
pub const VIRTUAL_DESKTOP_SWITCHING_EVENT: &str = "virtual-desktop-switching";

/// 虚拟桌面适配器发布变化批次的唯一接收端。
type VirtualDesktopChangeReceiver = Receiver<Result<Vec<VirtualDesktopChange>, String>>;

/// 持有版本隔离适配器和最近一次 Windows 虚拟桌面环境。
pub struct VirtualDesktopState {
    /// Windows 10/11 私有 ABI 隔离适配器。
    adapter: WindowsVirtualDesktopAdapter,
    /// 当前活动桌面身份与完整桌面集合。
    environment: Mutex<VirtualDesktopEnvironment>,
}

/// 提供虚拟桌面命令、布局协调和生命周期监视。
impl VirtualDesktopState {
    /// 启动版本适配器并返回状态与唯一变化接收端。
    pub fn new() -> Result<(Self, VirtualDesktopChangeReceiver), String> {
        let adapter = WindowsVirtualDesktopAdapter::new()?; // 当前 Windows build 对应的隔离适配器。
        let environment = adapter.environment()?; // 应用启动时的完整虚拟桌面环境。
        let changes = adapter.watch(); // 创建、删除和切换变化轮询接收端。
        Ok((
            Self {
                adapter,
                environment: Mutex::new(environment),
            },
            changes,
        ))
    }

    /// 返回应用启动和恢复使用的当前 Windows 虚拟桌面环境。
    pub fn environment(&self) -> Result<VirtualDesktopEnvironment, String> {
        self.environment
            .lock()
            .map(|environment| environment.clone())
            .map_err(|_| "虚拟桌面状态暂时不可用".to_string())
    }

    /// 返回当前适配器环境与工作区归档摘要。
    pub fn snapshot(
        &self,
        workspace: &WorkspaceState,
    ) -> Result<VirtualDesktopSnapshot, VirtualDesktopError> {
        let environment = self
            .environment
            .lock()
            .map_err(|_| VirtualDesktopError::platform_failed(&"虚拟桌面状态暂时不可用"))?
            .clone(); // 当前受锁保护的 Windows 虚拟桌面环境。
        let archives = workspace
            .layout_archives()
            .map_err(|error| VirtualDesktopError::workspace_failed(&error))?
            .into_iter()
            .map(|archive| VirtualDesktopArchiveSummary {
                archive_id: archive.archive_id,
                source_virtual_desktop_id: archive.source_virtual_desktop_id,
                archived_at_unix: archive.archived_at_unix,
                expires_at_unix: archive.expires_at_unix,
            })
            .collect(); // 当前仍在 30 天保留期内的布局归档摘要。
        Ok(VirtualDesktopSnapshot {
            windows_build: environment.windows_build,
            current_virtual_desktop_id: environment.current_id,
            virtual_desktop_ids: environment.desktop_ids,
            archives,
        })
    }

    /// 使用版本适配器创建新桌面并建立默认布局。
    pub fn create(
        &self,
        workspace: &WorkspaceState,
        runtime: &DesktopHostRuntime,
        desktop_items: &DesktopItemState,
    ) -> Result<VirtualDesktopSnapshot, VirtualDesktopError> {
        let desktop_id = self
            .adapter
            .create()
            .map_err(|error| VirtualDesktopError::platform_failed(&error))?; // Windows 新分配的虚拟桌面身份。
        self.refresh_workspace(workspace, runtime, desktop_items)?;
        crate::log_info!("虚拟桌面：新桌面已创建，身份={desktop_id}");
        self.snapshot(workspace)
    }

    /// 使用版本适配器删除目标桌面并归档其布局。
    pub fn remove(
        &self,
        desktop_id: &str,
        fallback_id: &str,
        workspace: &WorkspaceState,
        runtime: &DesktopHostRuntime,
        desktop_items: &DesktopItemState,
    ) -> Result<VirtualDesktopSnapshot, VirtualDesktopError> {
        if desktop_id == fallback_id {
            return Err(VirtualDesktopError::invalid_command(
                "删除目标与回退虚拟桌面不能相同",
            ));
        }
        self.adapter
            .remove(desktop_id, fallback_id)
            .map_err(|error| VirtualDesktopError::platform_failed(&error))?;
        self.refresh_workspace(workspace, runtime, desktop_items)?;
        crate::log_info!(
            "虚拟桌面：桌面已删除并归档布局，身份={}，回退={}",
            desktop_id,
            fallback_id
        );
        self.snapshot(workspace)
    }

    /// 创建新 Windows 桌面并把指定删除归档绑定到新身份。
    pub fn restore_archive(
        &self,
        archive_id: &str,
        workspace: &WorkspaceState,
        runtime: &DesktopHostRuntime,
        desktop_items: &DesktopItemState,
    ) -> Result<VirtualDesktopSnapshot, VirtualDesktopError> {
        let new_desktop_id = self
            .adapter
            .create()
            .map_err(|error| VirtualDesktopError::platform_failed(&error))?; // 恢复归档使用的新 Windows 桌面身份。
        let environment = self.refresh_workspace(workspace, runtime, desktop_items)?; // 创建后当前完整虚拟桌面环境。
        let monitors = runtime
            .snapshot()
            .map_err(|error| VirtualDesktopError::platform_failed(&error.message))?
            .hosts
            .into_iter()
            .map(|host| host.monitor)
            .collect::<Vec<_>>(); // 恢复归档使用的当前逐屏环境。
        let item_ids = desktop_items
            .snapshot()
            .map_err(|error| VirtualDesktopError::workspace_failed(&error.message))?
            .items
            .into_iter()
            .map(|item| item.stable_reference)
            .collect::<Vec<_>>(); // 恢复布局需要保留的完整 Shell 项目集合。
        workspace
            .restore_layout_archive(archive_id, &new_desktop_id, &monitors, &item_ids)
            .map_err(|error| VirtualDesktopError::workspace_failed(&error))?;
        crate::log_info!(
            "虚拟桌面：布局归档已绑定到新桌面，归档={}，身份={}，活动桌面={}",
            archive_id,
            new_desktop_id,
            environment.current_id
        );
        self.snapshot(workspace)
    }

    /// 补齐备份缺失的 Windows 桌面并事务式还原完整工作区。
    pub fn restore_workspace_backup(
        &self,
        path: &Path,
        workspace: &WorkspaceState,
        runtime: &DesktopHostRuntime,
        desktop_items: &DesktopItemState,
    ) -> Result<WorkspaceRestoreResult, WorkspaceError> {
        let original_environment = self
            .adapter
            .environment()
            .map_err(|error| WorkspaceError::restore_failed(&error))?; // 还原失败回滚使用的原始 Windows 桌面环境。
        let monitors = runtime
            .snapshot()
            .map_err(|error| WorkspaceError::restore_failed(&error.message))?
            .hosts
            .into_iter()
            .map(|host| host.monitor)
            .collect::<Vec<_>>(); // 备份布局恢复使用的当前显示器环境。
        let item_ids = desktop_items
            .snapshot()
            .map_err(|error| WorkspaceError::restore_failed(&error.message))?
            .items
            .into_iter()
            .map(|item| item.stable_reference)
            .collect::<Vec<_>>(); // 备份恢复时可以立即解析的 Shell 项目集合。
        let mut environment_state = self
            .environment
            .lock()
            .map_err(|_| WorkspaceError::state_unavailable())?; // 工作区提交前确认虚拟桌面内存状态可以同步替换。
        let missing_ids = workspace.missing_backup_virtual_desktop_ids(
            path,
            &original_environment.current_id,
            &original_environment.desktop_ids,
        )?; // 备份中需要创建新 Windows 桌面承接的布局身份。
        let mut mappings = Vec::with_capacity(missing_ids.len()); // 备份身份到实际新桌面身份的映射。
        for source_id in missing_ids {
            match self.adapter.create() {
                Ok(target_id) => mappings.push((source_id, target_id)),
                Err(error) => {
                    rollback_created_virtual_desktops(
                        &self.adapter,
                        &mappings,
                        &original_environment.current_id,
                    );
                    return Err(WorkspaceError::restore_failed(&error));
                }
            }
        }
        let restored_environment = match self.adapter.environment() {
            Ok(environment) => environment,
            Err(error) => {
                rollback_created_virtual_desktops(
                    &self.adapter,
                    &mappings,
                    &original_environment.current_id,
                );
                return Err(WorkspaceError::restore_failed(&error));
            }
        }; // 新桌面创建后的最终 Windows 环境。
        let result = workspace.restore_backup(
            path,
            &restored_environment.current_id,
            &restored_environment.desktop_ids,
            &mappings,
            &monitors,
            &item_ids,
        );
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                rollback_created_virtual_desktops(
                    &self.adapter,
                    &mappings,
                    &original_environment.current_id,
                );
                return Err(error);
            }
        }; // 工作区原子文件和内存状态均已替换的还原结果。
        *environment_state = restored_environment;
        crate::log_info!(
            "虚拟桌面：完整工作区备份已协调，新建桌面={}，路径={}",
            mappings.len(),
            path.display()
        );
        Ok(result)
    }

    /// 重新读取 Windows 环境并协调全部独立布局。
    fn refresh_workspace(
        &self,
        workspace: &WorkspaceState,
        runtime: &DesktopHostRuntime,
        desktop_items: &DesktopItemState,
    ) -> Result<VirtualDesktopEnvironment, VirtualDesktopError> {
        let mut environment_state = self
            .environment
            .lock()
            .map_err(|_| VirtualDesktopError::platform_failed(&"虚拟桌面状态暂时不可用"))?; // 与完整备份还原共用的环境到工作区锁顺序。
        let environment = self
            .adapter
            .environment()
            .map_err(|error| VirtualDesktopError::platform_failed(&error))?; // 当前最终 Windows 虚拟桌面环境。
        let monitors = runtime
            .snapshot()
            .map_err(|error| VirtualDesktopError::platform_failed(&error.message))?
            .hosts
            .into_iter()
            .map(|host| host.monitor)
            .collect::<Vec<_>>(); // 当前全部逐屏宿主显示器状态。
        let item_ids = desktop_items
            .snapshot()
            .map_err(|error| VirtualDesktopError::workspace_failed(&error.message))?
            .items
            .into_iter()
            .map(|item| item.stable_reference)
            .collect::<Vec<_>>(); // 每个活动布局共享的完整 Shell 项目集合。
        workspace
            .reconcile_virtual_desktops(
                &environment.current_id,
                &environment.desktop_ids,
                &monitors,
                &item_ids,
            )
            .map_err(|error| VirtualDesktopError::workspace_failed(&error))?;
        *environment_state = environment.clone();
        Ok(environment)
    }

    /// 启动虚拟桌面生命周期协调线程。
    pub fn start_monitor(
        app: AppHandle,
        changes: Receiver<Result<Vec<VirtualDesktopChange>, String>>,
    ) {
        thread::spawn(move || {
            while let Ok(changes) = changes.recv() {
                let changes = match changes {
                    Ok(changes) => changes,
                    Err(error) => {
                        crate::log_fatal!("虚拟桌面：版本适配器失效，错误={error}");
                        app.state::<Arc<DesktopHostRuntime>>().cleanup();
                        app.exit(12);
                        return;
                    }
                }; // 当前轮询产生的虚拟桌面生命周期事实。
                if let Some((previous_id, current_id)) = changes.iter().find_map(|change| {
                    if let VirtualDesktopChange::Switched {
                        previous_id,
                        current_id,
                    } = change
                    {
                        Some((previous_id, current_id))
                    } else {
                        None
                    }
                }) {
                    let _ = app.state::<Arc<DesktopHostRuntime>>().cancel_input_drags();
                    let _ = app.emit(
                        VIRTUAL_DESKTOP_SWITCHING_EVENT,
                        json!({"previousId": previous_id, "currentId": current_id}),
                    );
                }
                let state = app.state::<VirtualDesktopState>(); // 当前虚拟桌面应用状态。
                let workspace = app.state::<WorkspaceState>(); // 当前全部独立布局状态。
                let runtime = app.state::<Arc<DesktopHostRuntime>>(); // 当前逐屏宿主运行时。
                let desktop_items = app.state::<DesktopItemState>(); // 当前共享 Shell 项目状态。
                if let Err(error) = state.refresh_workspace(&workspace, &runtime, &desktop_items) {
                    crate::log_fatal!("虚拟桌面：生命周期协调失败，错误={}", error.message);
                    runtime.cleanup();
                    app.exit(12);
                    return;
                }
                let workspace_snapshot = match workspace.snapshot() {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        crate::log_error!("虚拟桌面：读取活动布局失败，错误={}", error.message);
                        continue;
                    }
                }; // 切换后当前活动虚拟桌面布局。
                if let Ok(snapshot) = state.snapshot(&workspace) {
                    let _ = app.emit(VIRTUAL_DESKTOP_SNAPSHOT_CHANGED_EVENT, snapshot);
                }
                let _ = app.emit(WORKSPACE_SNAPSHOT_CHANGED_EVENT, workspace_snapshot);
                if let Ok(snapshot) = runtime.snapshot() {
                    let _ = app.emit(DESKTOP_HOST_SNAPSHOT_CHANGED_EVENT, snapshot);
                }
                crate::log_info!("虚拟桌面：生命周期变化已协调，事件数量={}", changes.len());
            }
        });
    }
}

/// 备份还原失败时删除本轮新建的 Windows 虚拟桌面。
fn rollback_created_virtual_desktops(
    adapter: &WindowsVirtualDesktopAdapter,
    mappings: &[(String, String)],
    fallback_id: &str,
) {
    for (_, desktop_id) in mappings.iter().rev() {
        if let Err(error) = adapter.remove(desktop_id, fallback_id) {
            crate::log_error!(
                "虚拟桌面：备份失败回滚新桌面失败，身份={}，错误={error}",
                desktop_id
            );
        }
    }
}
