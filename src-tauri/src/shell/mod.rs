use std::collections::HashMap;
use std::sync::mpsc::{self, Sender, SyncSender};
use std::sync::{Mutex, MutexGuard};
use std::thread::{self, JoinHandle};

use crate::contracts::{
    DesktopItem, DesktopItemAvailability, DesktopItemError, DesktopItemSnapshot,
};
use crate::platform::windows::{
    enumerate_desktop_items, initialize_shell_apartment, uninitialize_shell_apartment,
};

/// 表示发送到专用 Shell STA 线程的请求。
enum ShellRequest {
    /// 枚举当前完整桌面项目集合。
    Enumerate(SyncSender<Result<Vec<DesktopItem>, String>>),
    /// 关闭专用 Shell STA 线程。
    Shutdown,
}

/// 持有专用 Shell STA 线程及其请求通道。
struct ShellStaRuntime {
    /// Shell 请求发送端。
    sender: Sender<ShellRequest>,
    /// Shell STA 后台线程。
    worker: Option<JoinHandle<()>>,
}

impl ShellStaRuntime {
    /// 创建并验证专用 Shell STA 线程。
    fn start() -> Result<Self, DesktopItemError> {
        // Shell 请求通道。
        let (sender, receiver) = mpsc::channel();
        // Shell STA 初始化结果通道。
        let (ready, initialized) = mpsc::sync_channel(0);
        // 专用 Shell STA 后台线程。
        let worker = thread::Builder::new()
            .name("desktop-tools-shell-sta".to_string())
            .spawn(move || {
                // 当前线程的 STA 初始化结果。
                let initialization = initialize_shell_apartment();
                let can_run = initialization.is_ok();
                let _ = ready.send(initialization);
                if !can_run {
                    return;
                }

                while let Ok(request) = receiver.recv() {
                    match request {
                        ShellRequest::Enumerate(reply) => {
                            let _ = reply.send(enumerate_desktop_items());
                        }
                        ShellRequest::Shutdown => break,
                    }
                }

                uninitialize_shell_apartment();
            })
            .map_err(|error| DesktopItemError::shell_enumeration_failed(&error))?;
        initialized
            .recv()
            .map_err(|error| DesktopItemError::shell_enumeration_failed(&error))?
            .map_err(|error| DesktopItemError::shell_enumeration_failed(&error))?;

        crate::log_info!("Shell：专用 STA 运行时已启动");
        Ok(Self {
            sender,
            worker: Some(worker),
        })
    }

    /// 在专用 STA 线程枚举桌面项目。
    fn enumerate(&self) -> Result<Vec<DesktopItem>, String> {
        // 等待 Shell 枚举结果的同步通道。
        let (reply, response) = mpsc::sync_channel(0);
        self.sender
            .send(ShellRequest::Enumerate(reply))
            .map_err(|_| "Shell STA 线程已经停止".to_string())?;
        response
            .recv()
            .map_err(|_| "Shell STA 线程已经停止".to_string())?
    }
}

impl Drop for ShellStaRuntime {
    /// 请求 Shell STA 线程释放 COM apartment 并退出。
    fn drop(&mut self) {
        let _ = self.sender.send(ShellRequest::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// 持有桌面项目最近有效快照与专用 Shell STA 运行时。
pub struct DesktopItemState {
    /// 当前不可变桌面项目快照。
    snapshot: Mutex<DesktopItemSnapshot>,
    /// 专用 Shell STA 运行时。
    runtime: ShellStaRuntime,
}

impl DesktopItemState {
    /// 创建桌面项目状态与专用 Shell STA 运行时。
    pub fn new() -> Result<Self, DesktopItemError> {
        Ok(Self {
            snapshot: Mutex::new(DesktopItemSnapshot::default()),
            runtime: ShellStaRuntime::start()?,
        })
    }

    /// 枚举 Shell 桌面项目并发布新的不可变快照。
    pub fn refresh(&self) -> Result<DesktopItemSnapshot, DesktopItemError> {
        // 当前 Shell 枚举观察结果。
        let observation = self.runtime.enumerate();
        if let Err(error) = &observation {
            crate::log_error!("Shell：桌面项目枚举失败，错误：{error}");
        }

        // 等待提交观察结果的桌面项目快照。
        let mut snapshot = lock_snapshot(&self.snapshot)?;
        let snapshot = publish_observation(&mut snapshot, observation)?;
        // 当前快照中等待 Shell 恢复的项目数量。
        let unavailable_count = snapshot
            .items
            .iter()
            .filter(|item| item.availability == DesktopItemAvailability::Unavailable)
            .count();
        crate::log_info!(
            "Shell：桌面项目快照已发布，修订：{}，项目数量：{}，等待恢复：{}",
            snapshot.revision,
            snapshot.items.len(),
            unavailable_count
        );

        Ok(snapshot)
    }
}

/// 获取桌面项目快照锁。
fn lock_snapshot(
    snapshot: &Mutex<DesktopItemSnapshot>,
) -> Result<MutexGuard<'_, DesktopItemSnapshot>, DesktopItemError> {
    snapshot
        .lock()
        .map_err(|_| DesktopItemError::snapshot_unavailable())
}

/// 按稳定引用协调本次 Shell 观察结果与最近有效元数据。
fn reconcile_desktop_items(
    mut observed: Vec<DesktopItem>,
    previous: &[DesktopItem],
) -> Vec<DesktopItem> {
    // 最近一次项目元数据的稳定引用索引。
    let previous_by_reference = previous
        .iter()
        .map(|item| (item.stable_reference.as_str(), item))
        .collect::<HashMap<_, _>>();

    for item in &mut observed {
        if item.availability == DesktopItemAvailability::Available {
            continue;
        }

        if let Some(previous_item) = previous_by_reference.get(item.stable_reference.as_str()) {
            item.display_name.clone_from(&previous_item.display_name);
            item.system_image.clone_from(&previous_item.system_image);
        }
    }

    observed.sort_by_cached_key(|item| item.display_name.to_lowercase());
    observed
}

/// 提交一次成功的 Shell 观察结果，并在失败时保留当前快照。
fn publish_observation(
    snapshot: &mut DesktopItemSnapshot,
    observation: Result<Vec<DesktopItem>, String>,
) -> Result<DesktopItemSnapshot, DesktopItemError> {
    // 已成功枚举的完整桌面项目集合。
    let observed =
        observation.map_err(|error| DesktopItemError::shell_enumeration_failed(&error))?;
    snapshot.items = reconcile_desktop_items(observed, &snapshot.items);
    snapshot.revision += 1;

    Ok(snapshot.clone())
}

#[cfg(test)]
mod tests {
    use super::{publish_observation, reconcile_desktop_items};
    use crate::contracts::{DesktopItem, DesktopItemAvailability, DesktopItemSnapshot};

    /// 验证暂时解析失败的项目保留最近一次有效元数据。
    #[test]
    fn unavailable_item_preserves_last_valid_metadata() {
        // 最近一次有效项目集合。
        let previous = vec![DesktopItem {
            stable_reference: "::{project}".to_string(),
            display_name: "Project".to_string(),
            system_image: Some("data:image/png;base64,AA==".to_string()),
            availability: DesktopItemAvailability::Available,
        }];
        // 本次 Shell 枚举观察到的暂时不可用项目。
        let observed = vec![DesktopItem {
            stable_reference: "::{project}".to_string(),
            display_name: "::{project}".to_string(),
            system_image: None,
            availability: DesktopItemAvailability::Unavailable,
        }];

        // 按稳定引用协调后的项目集合。
        let reconciled = reconcile_desktop_items(observed, &previous);

        assert_eq!(reconciled[0].display_name, "Project");
        assert_eq!(
            reconciled[0].system_image.as_deref(),
            Some("data:image/png;base64,AA==")
        );
        assert_eq!(
            reconciled[0].availability,
            DesktopItemAvailability::Unavailable
        );
    }

    /// 验证枚举失败保留最近一次有效快照。
    #[test]
    fn enumeration_failure_preserves_last_valid_snapshot() {
        // 最近一次有效桌面项目快照。
        let mut snapshot = DesktopItemSnapshot {
            revision: 8,
            items: vec![DesktopItem {
                stable_reference: "::{project}".to_string(),
                display_name: "Project".to_string(),
                system_image: None,
                availability: DesktopItemAvailability::Available,
            }],
        };
        // Shell 枚举失败结果。
        let result = publish_observation(&mut snapshot, Err("Explorer 暂时不可用".to_string()));

        assert_eq!(
            result.expect_err("Shell 枚举失败应返回错误").code,
            "shell-enumeration-failed"
        );
        assert_eq!(snapshot.revision, 8);
        assert_eq!(snapshot.items[0].display_name, "Project");
    }
}
