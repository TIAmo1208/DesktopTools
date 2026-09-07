use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Mutex, MutexGuard};
use std::thread::{self, JoinHandle};

use crate::contracts::{
    DesktopItem, DesktopItemAvailability, DesktopItemError, DesktopItemSnapshot,
    ShellOperationKind, ShellOperationResult,
};
use crate::platform::windows::{
    NativeMenuCommand, NativeMenuTarget, ShellChangeEvent, ShellChangeWatcher,
    ShellCompensationAction, ShellMenuOwner, compensate_desktop_items, delete_desktop_items,
    enumerate_desktop_items, initialize_shell_apartment, open_desktop_items, paste_desktop_items,
    rename_desktop_item, set_desktop_item_clipboard, show_native_context_menu,
    start_desktop_item_drag, start_shell_change_watcher, uninitialize_shell_apartment,
};

/// 表示发送到专用 Shell STA 线程的请求。
enum ShellRequest {
    /// 枚举当前完整桌面项目集合。
    Enumerate(SyncSender<Result<Vec<DesktopItem>, String>>),
    /// 使用 Shell 默认动词打开完整项目选择集合。
    Open {
        /// 已通过快照校验的稳定项目引用。
        references: Vec<String>,
        /// 等待 Shell 操作结果的同步通道。
        reply: SyncSender<Result<(), String>>,
    },
    /// 使用 IFileOperation 重命名单个项目。
    Rename {
        /// 已通过快照校验的稳定项目引用。
        reference: String,
        /// 用户提交的新项目名称。
        new_name: String,
        /// 等待 Shell 实际结果的同步通道。
        reply: SyncSender<Result<ShellOperationResult, String>>,
    },
    /// 使用 IFileOperation 删除完整选择集合。
    Delete {
        /// 已通过快照校验的稳定项目引用。
        references: Vec<String>,
        /// 是否跳过回收站并明确永久删除。
        permanent: bool,
        /// 等待 Shell 实际结果的同步通道。
        reply: SyncSender<Result<ShellOperationResult, String>>,
    },
    /// 将完整选择集合写入系统 Shell 剪贴板。
    SetClipboard {
        /// 已通过快照校验的稳定项目引用。
        references: Vec<String>,
        /// 是否使用剪切而不是复制效果。
        cut: bool,
        /// 等待系统剪贴板结果的同步通道。
        reply: SyncSender<Result<ShellOperationResult, String>>,
    },
    /// 将系统 Shell 剪贴板粘贴到桌面命名空间。
    Paste(SyncSender<Result<ShellOperationResult, String>>),
    /// 使用 Shell IDataObject 启动系统 OLE 拖放。
    Drag {
        /// 已通过快照校验的稳定项目引用。
        references: Vec<String>,
        /// 等待系统拖放结果的同步通道。
        reply: SyncSender<Result<ShellOperationResult, String>>,
    },
    /// 使用新的 IFileOperation 执行统一时间线补偿。
    Compensate {
        /// 原始 Shell 操作类型。
        kind: ShellOperationKind,
        /// 本轮实际可执行的补偿项目集合。
        actions: Vec<ShellCompensationAction>,
        /// 等待 Shell 补偿结果的同步通道。
        reply: SyncSender<Result<ShellOperationResult, String>>,
    },
    /// 在专用 owner HWND 显示原生上下文菜单。
    ShowMenu {
        /// 当前项目、背景或产品菜单目标。
        target: NativeMenuTarget,
        /// 菜单锚点屏幕物理横坐标。
        screen_x: i32,
        /// 菜单锚点屏幕物理纵坐标。
        screen_y: i32,
        /// 当前目标支持的 Desktop Tools 产品命令。
        product_commands: Vec<NativeMenuCommand>,
        /// 等待产品命令选择结果的同步通道。
        reply: SyncSender<Result<Option<u32>, String>>,
    },
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
                if let Err(error) = initialize_shell_apartment() {
                    let _ = ready.send(Err(error));
                    return;
                }
                // 当前 STA 线程独占的专用菜单 owner HWND。
                let mut menu_owner = match ShellMenuOwner::create() {
                    Ok(owner) => owner,
                    Err(error) => {
                        uninitialize_shell_apartment();
                        let _ = ready.send(Err(error));
                        return;
                    }
                };
                let _ = ready.send(Ok(()));

                while let Ok(request) = receiver.recv() {
                    match request {
                        ShellRequest::Enumerate(reply) => {
                            let _ = reply.send(enumerate_desktop_items());
                        }
                        ShellRequest::Open { references, reply } => {
                            let _ = reply.send(open_desktop_items(&references));
                        }
                        ShellRequest::Rename {
                            reference,
                            new_name,
                            reply,
                        } => {
                            let _ =
                                reply.send(rename_desktop_item(&menu_owner, &reference, &new_name));
                        }
                        ShellRequest::Delete {
                            references,
                            permanent,
                            reply,
                        } => {
                            let _ = reply.send(delete_desktop_items(
                                &menu_owner,
                                &references,
                                permanent,
                            ));
                        }
                        ShellRequest::SetClipboard {
                            references,
                            cut,
                            reply,
                        } => {
                            let _ = reply.send(set_desktop_item_clipboard(
                                &menu_owner,
                                &references,
                                cut,
                            ));
                        }
                        ShellRequest::Paste(reply) => {
                            let _ = reply.send(paste_desktop_items(&menu_owner));
                        }
                        ShellRequest::Drag { references, reply } => {
                            let _ = reply.send(start_desktop_item_drag(&menu_owner, &references));
                        }
                        ShellRequest::Compensate {
                            kind,
                            actions,
                            reply,
                        } => {
                            let _ =
                                reply.send(compensate_desktop_items(&menu_owner, kind, actions));
                        }
                        ShellRequest::ShowMenu {
                            target,
                            screen_x,
                            screen_y,
                            product_commands,
                            reply,
                        } => {
                            let _ = reply.send(show_native_context_menu(
                                &mut menu_owner,
                                target,
                                screen_x,
                                screen_y,
                                &product_commands,
                            ));
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

    /// 在专用 STA 线程使用 Shell 默认动词打开项目。
    fn open(&self, references: Vec<String>) -> Result<(), String> {
        // 等待 Shell 打开结果的同步通道。
        let (reply, response) = mpsc::sync_channel(0);
        self.sender
            .send(ShellRequest::Open { references, reply })
            .map_err(|_| "Shell STA 线程已经停止".to_string())?;
        response
            .recv()
            .map_err(|_| "Shell STA 线程已经停止".to_string())?
    }

    /// 在专用 STA 线程重命名单个 Shell 项目。
    fn rename(&self, reference: String, new_name: String) -> Result<ShellOperationResult, String> {
        let (reply, response) = mpsc::sync_channel(0); // 等待 Shell 重命名结果的同步通道。
        self.sender
            .send(ShellRequest::Rename {
                reference,
                new_name,
                reply,
            })
            .map_err(|_| "Shell STA 线程已经停止".to_string())?;
        response
            .recv()
            .map_err(|_| "Shell STA 线程已经停止".to_string())?
    }

    /// 在专用 STA 线程删除完整选择集合。
    fn delete(
        &self,
        references: Vec<String>,
        permanent: bool,
    ) -> Result<ShellOperationResult, String> {
        let (reply, response) = mpsc::sync_channel(0); // 等待 Shell 删除结果的同步通道。
        self.sender
            .send(ShellRequest::Delete {
                references,
                permanent,
                reply,
            })
            .map_err(|_| "Shell STA 线程已经停止".to_string())?;
        response
            .recv()
            .map_err(|_| "Shell STA 线程已经停止".to_string())?
    }

    /// 在专用 STA 线程设置系统 Shell 剪贴板。
    fn set_clipboard(
        &self,
        references: Vec<String>,
        cut: bool,
    ) -> Result<ShellOperationResult, String> {
        let (reply, response) = mpsc::sync_channel(0); // 等待系统剪贴板结果的同步通道。
        self.sender
            .send(ShellRequest::SetClipboard {
                references,
                cut,
                reply,
            })
            .map_err(|_| "Shell STA 线程已经停止".to_string())?;
        response
            .recv()
            .map_err(|_| "Shell STA 线程已经停止".to_string())?
    }

    /// 在专用 STA 线程粘贴系统 Shell 剪贴板。
    fn paste(&self) -> Result<ShellOperationResult, String> {
        let (reply, response) = mpsc::sync_channel(0); // 等待 Shell 粘贴结果的同步通道。
        self.sender
            .send(ShellRequest::Paste(reply))
            .map_err(|_| "Shell STA 线程已经停止".to_string())?;
        response
            .recv()
            .map_err(|_| "Shell STA 线程已经停止".to_string())?
    }

    /// 在专用 STA 线程启动系统 OLE 拖放。
    fn drag(&self, references: Vec<String>) -> Result<ShellOperationResult, String> {
        let (reply, response) = mpsc::sync_channel(0); // 等待系统 OLE 拖放结果的同步通道。
        self.sender
            .send(ShellRequest::Drag { references, reply })
            .map_err(|_| "Shell STA 线程已经停止".to_string())?;
        response
            .recv()
            .map_err(|_| "Shell STA 线程已经停止".to_string())?
    }

    /// 在专用 STA 线程执行一轮 Shell 补偿操作。
    fn compensate(
        &self,
        kind: ShellOperationKind,
        actions: Vec<ShellCompensationAction>,
    ) -> Result<ShellOperationResult, String> {
        let (reply, response) = mpsc::sync_channel(0); // 等待 Shell 补偿结果的同步通道。
        self.sender
            .send(ShellRequest::Compensate {
                kind,
                actions,
                reply,
            })
            .map_err(|_| "Shell STA 线程已经停止".to_string())?;
        response
            .recv()
            .map_err(|_| "Shell STA 线程已经停止".to_string())?
    }

    /// 在专用 STA 线程显示原生上下文菜单。
    fn show_menu(
        &self,
        target: NativeMenuTarget,
        screen_x: i32,
        screen_y: i32,
        product_commands: Vec<NativeMenuCommand>,
    ) -> Result<Option<u32>, String> {
        // 等待原生菜单选择结果的同步通道。
        let (reply, response) = mpsc::sync_channel(0);
        self.sender
            .send(ShellRequest::ShowMenu {
                target,
                screen_x,
                screen_y,
                product_commands,
                reply,
            })
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
    /// 持有 SHChangeNotify 注册和窗口线程。
    _change_watcher: ShellChangeWatcher,
    /// 供应用协调线程独占取得的变化事件接收端。
    change_receiver: Mutex<Option<Receiver<ShellChangeEvent>>>,
    /// 当前应用会话中确认完成的 Shell 文件操作记录。
    operation_history: Mutex<Vec<ShellHistoryEntry>>,
    /// 下一个会话内 Shell 文件操作标识。
    next_operation_id: AtomicU64,
}

/// 一个可逆 Shell 操作在统一时间线中的当前位置。
#[derive(Clone, Copy, Eq, PartialEq)]
enum ShellHistoryState {
    /// 原始操作已应用，可以撤销。
    Applied,
    /// 原始操作已撤销，可以重做。
    Undone,
    /// 外部变化或缺失结果引用使记录不再可执行。
    Invalid,
}

/// 一个 Shell 项目在补偿链路中保留的原始目标和当前引用。
#[derive(Clone)]
struct ShellHistoryItem {
    /// 原始操作前源项目稳定引用。
    source_reference: String,
    /// 原始操作后结果项目稳定引用。
    result_reference: String,
    /// 原始操作前源项目父级稳定引用。
    source_parent_reference: Option<String>,
    /// 原始操作前源项目编辑名称。
    source_name: Option<String>,
    /// 原始操作后结果项目父级稳定引用。
    result_parent_reference: Option<String>,
    /// 原始操作后结果项目编辑名称。
    result_name: Option<String>,
    /// 当前状态下等待下一次补偿的实际稳定引用。
    current_reference: String,
}

/// 当前应用会话中的一个可逆 Shell 操作日志单元。
#[derive(Clone)]
struct ShellHistoryEntry {
    /// 首次 Shell 操作公开结果。
    result: ShellOperationResult,
    /// 当前记录可以撤销、重做或已经失效。
    state: ShellHistoryState,
    /// 最近进入当前方向的 Unix 微秒时间。
    timeline_at_unix_micros: i64,
    /// 仍能明确匹配并参与后续补偿的项目集合。
    items: Vec<ShellHistoryItem>,
}

impl DesktopItemState {
    /// 创建桌面项目状态与专用 Shell STA 运行时。
    pub fn new() -> Result<Self, DesktopItemError> {
        // 已注册桌面命名空间通知的变化监听线程与事件接收端。
        let (change_watcher, change_receiver) = start_shell_change_watcher()
            .map_err(|error| DesktopItemError::shell_operation_failed(&error))?;
        Ok(Self {
            snapshot: Mutex::new(DesktopItemSnapshot::default()),
            runtime: ShellStaRuntime::start()?,
            _change_watcher: change_watcher,
            change_receiver: Mutex::new(Some(change_receiver)),
            operation_history: Mutex::new(Vec::new()),
            next_operation_id: AtomicU64::new(1),
        })
    }

    /// 将 Shell 变化事件接收端交给唯一应用协调线程。
    pub fn take_change_receiver(&self) -> Result<Receiver<ShellChangeEvent>, DesktopItemError> {
        self.change_receiver
            .lock()
            .map_err(|_| DesktopItemError::snapshot_unavailable())?
            .take()
            .ok_or_else(|| DesktopItemError::invalid_selection("Shell 变化协调线程已经启动"))
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

    /// 返回最近一次有效桌面项目快照。
    pub fn snapshot(&self) -> Result<DesktopItemSnapshot, DesktopItemError> {
        Ok(lock_snapshot(&self.snapshot)?.clone())
    }

    /// 使用 Shell 默认动词打开当前快照中的完整选择集合。
    pub fn open(&self, item_ids: &[String]) -> Result<(), DesktopItemError> {
        // 已通过当前快照可用性校验的稳定引用。
        let references = self.selection_references(item_ids)?;
        self.runtime.open(references).map_err(|error| {
            crate::log_error!("Shell：桌面项目打开失败，错误：{error}");
            DesktopItemError::shell_operation_failed(&error)
        })?;
        crate::log_info!("Shell：桌面项目已打开，项目数量：{}", item_ids.len());
        Ok(())
    }

    /// 重命名单个当前可用项目并记录 Shell 确认结果。
    pub fn rename(
        &self,
        item_id: &str,
        new_name: String,
    ) -> Result<ShellOperationResult, DesktopItemError> {
        let references = self.selection_references(&[item_id.to_string()])?; // 已通过快照校验的单一稳定引用。
        let result = self
            .runtime
            .rename(references[0].clone(), new_name)
            .map_err(|error| {
                crate::log_error!("Shell：桌面项目重命名失败，错误：{error}");
                DesktopItemError::shell_operation_failed(&error)
            })?; // Shell 确认的实际重命名结果。
        self.record_operation(result)
    }

    /// 删除当前完整选择集合并记录可逆或永久属性。
    pub fn delete(
        &self,
        item_ids: &[String],
        permanent: bool,
    ) -> Result<ShellOperationResult, DesktopItemError> {
        let references = self.selection_references(item_ids)?; // 已通过快照校验的稳定引用集合。
        let result = self
            .runtime
            .delete(references, permanent)
            .map_err(|error| {
                crate::log_error!(
                    "Shell：桌面项目删除失败，永久={}，错误：{error}",
                    u8::from(permanent)
                );
                DesktopItemError::shell_operation_failed(&error)
            })?; // Shell 确认的实际删除结果。
        self.record_operation(result)
    }

    /// 将当前完整选择集合复制或剪切到系统 Shell 剪贴板。
    pub fn set_clipboard(
        &self,
        item_ids: &[String],
        cut: bool,
    ) -> Result<ShellOperationResult, DesktopItemError> {
        let references = self.selection_references(item_ids)?; // 已通过快照校验的稳定引用集合。
        let mut result = self
            .runtime
            .set_clipboard(references, cut)
            .map_err(|error| {
                crate::log_error!(
                    "Shell：系统剪贴板写入失败，剪切={}，错误：{error}",
                    u8::from(cut)
                );
                DesktopItemError::shell_operation_failed(&error)
            })?; // 系统剪贴板接受完整 IDataObject 后的结果。
        result.completed_at_unix_micros = crate::tools::time::next_timeline_timestamp();
        crate::log_info!(
            "Shell：系统剪贴板已更新，剪切={}，项目数量={}",
            u8::from(cut),
            item_ids.len()
        );
        Ok(result)
    }

    /// 将系统 Shell 剪贴板粘贴到桌面命名空间并记录实际结果。
    pub fn paste(&self) -> Result<ShellOperationResult, DesktopItemError> {
        let result = self.runtime.paste().map_err(|error| {
            crate::log_error!("Shell：桌面项目粘贴失败，错误：{error}");
            DesktopItemError::shell_operation_failed(&error)
        })?; // Shell 确认的粘贴结果。
        self.record_operation(result)
    }

    /// 使用当前完整选择集合启动系统 OLE 拖放。
    pub fn drag(&self, item_ids: &[String]) -> Result<ShellOperationResult, DesktopItemError> {
        let references = self.selection_references(item_ids)?; // 已通过快照校验的稳定引用集合。
        let mut result = self.runtime.drag(references).map_err(|error| {
            crate::log_error!("Shell：系统 OLE 拖放失败，错误：{error}");
            DesktopItemError::shell_operation_failed(&error)
        })?; // 系统目标协商后的最终拖放结果。
        result.completed_at_unix_micros = crate::tools::time::next_timeline_timestamp();
        crate::log_info!(
            "Shell：系统 OLE 拖放结束，项目数量={}，效果={}，取消={}",
            item_ids.len(),
            result.effect.as_deref().unwrap_or("无"),
            u8::from(result.aborted)
        );
        Ok(result)
    }

    /// 返回指定方向最近一个有效 Shell 历史单元的时间。
    pub fn latest_shell_history_time(&self, redo: bool) -> Result<Option<i64>, DesktopItemError> {
        let expected_state = if redo {
            ShellHistoryState::Undone
        } else {
            ShellHistoryState::Applied
        }; // 当前方向可以执行的历史状态。
        Ok(self
            .operation_history
            .lock()
            .map_err(|_| DesktopItemError::snapshot_unavailable())?
            .iter()
            .rev()
            .find(|entry| entry.state == expected_state && !entry.items.is_empty())
            .map(|entry| entry.timeline_at_unix_micros))
    }

    /// 新布局或 Shell 操作提交后使当前统一时间线之后的 Shell 重做链失效。
    pub fn clear_redo_history(&self) -> Result<(), DesktopItemError> {
        for entry in self
            .operation_history
            .lock()
            .map_err(|_| DesktopItemError::snapshot_unavailable())?
            .iter_mut()
            .filter(|entry| entry.state == ShellHistoryState::Undone)
        {
            entry.state = ShellHistoryState::Invalid;
        }
        Ok(())
    }

    /// 完整工作区还原后清除不再对应当前状态的全部 Shell 历史。
    pub fn clear_operation_history(&self) -> Result<(), DesktopItemError> {
        self.operation_history
            .lock()
            .map_err(|_| DesktopItemError::snapshot_unavailable())?
            .clear();
        Ok(())
    }

    /// 使用精确重命名和最终桌面项目集合更新或失效 Shell 补偿记录。
    pub fn reconcile_operation_history(
        &self,
        snapshot: &DesktopItemSnapshot,
        renames: &[(String, String)],
    ) -> Result<(), DesktopItemError> {
        let active_references = snapshot
            .items
            .iter()
            .map(|item| item.stable_reference.as_str())
            .collect::<std::collections::HashSet<_>>(); // 当前桌面命名空间最终存在的稳定引用。
        let mut history = self
            .operation_history
            .lock()
            .map_err(|_| DesktopItemError::snapshot_unavailable())?; // 等待外部事实协调的完整 Shell 历史。
        let mut invalidated = false; // 本轮是否发现无法明确匹配的记录。
        for entry in history.iter_mut() {
            if entry.state == ShellHistoryState::Invalid {
                continue;
            }
            let previous_state = entry.state; // 精确变化协调前的历史方向。
            for item in &mut entry.items {
                let Some((_, new_reference)) = renames
                    .iter()
                    .find(|(old_reference, _)| old_reference == &item.current_reference)
                else {
                    continue;
                };
                item.current_reference.clone_from(new_reference);
            }
            let next_state = if previous_state == ShellHistoryState::Applied
                && entry
                    .items
                    .iter()
                    .all(|item| item.current_reference == item.source_reference)
            {
                Some(ShellHistoryState::Undone)
            } else if previous_state == ShellHistoryState::Undone
                && entry
                    .items
                    .iter()
                    .all(|item| item.current_reference == item.result_reference)
            {
                Some(ShellHistoryState::Applied)
            } else {
                None
            }; // 全部项目精确到达相反状态时才确认 Explorer 外部撤销或重做。
            if let Some(next_state) = next_state {
                entry.state = next_state;
                entry.timeline_at_unix_micros = crate::tools::time::next_timeline_timestamp();
            }
            let outside_desktop = (entry.result.kind == ShellOperationKind::Recycle
                && entry.state == ShellHistoryState::Applied)
                || (entry.result.kind == ShellOperationKind::Paste
                    && entry.state == ShellHistoryState::Undone); // 回收站项目或粘贴来源不要求出现在桌面枚举中。
            if !outside_desktop
                && entry
                    .items
                    .iter()
                    .any(|item| !active_references.contains(item.current_reference.as_str()))
            {
                entry.state = ShellHistoryState::Invalid;
                invalidated = true;
            }
        }
        if invalidated {
            for entry in history
                .iter_mut()
                .filter(|entry| entry.state == ShellHistoryState::Undone)
            {
                entry.state = ShellHistoryState::Invalid;
            }
            crate::log_warn!("Shell：外部状态变化存在歧义，相关记录及重做链已失效");
        }
        Ok(())
    }

    /// 撤销或重做最近一个有效 Shell 补偿单元。
    pub fn restore_shell_history(
        &self,
        redo: bool,
    ) -> Result<Option<ShellOperationResult>, DesktopItemError> {
        let expected_state = if redo {
            ShellHistoryState::Undone
        } else {
            ShellHistoryState::Applied
        }; // 当前方向可以执行的历史状态。
        let selected = self
            .operation_history
            .lock()
            .map_err(|_| DesktopItemError::snapshot_unavailable())?
            .iter()
            .enumerate()
            .rev()
            .find(|(_, entry)| entry.state == expected_state && !entry.items.is_empty())
            .map(|(index, entry)| (index, entry.clone())); // 锁外执行原生 Shell 补偿的历史副本。
        let Some((history_index, entry)) = selected else {
            return Ok(None);
        };
        let actions = shell_compensation_actions(&entry, redo); // 当前有效项目对应的原生补偿动作。
        let mut result = match self.runtime.compensate(entry.result.kind, actions) {
            Ok(result) => result,
            Err(error) => {
                if let Ok(mut history) = self.operation_history.lock()
                    && let Some(current) = history.get_mut(history_index)
                    && current.result.operation_id == entry.result.operation_id
                {
                    current.state = ShellHistoryState::Invalid;
                }
                crate::log_error!(
                    "Shell：统一历史补偿失败，标识={}，方向={}，错误={error}",
                    entry.result.operation_id,
                    if redo { "重做" } else { "撤销" }
                );
                return Err(DesktopItemError::shell_operation_failed(&error));
            }
        }; // 原生补偿失败时失效歧义记录，避免重复执行未知部分状态。
        let total = entry.items.len(); // 本轮补偿前仍明确有效的项目数量。
        let mut restored_items = Vec::new(); // Shell 确认成功且仍获得稳定结果引用的项目。
        for item in &entry.items {
            let Some(actual) = result.items.iter().find(|actual| {
                actual.source_reference == item.current_reference && actual.succeeded
            }) else {
                continue;
            };
            let Some(reference) = actual.result_reference.clone() else {
                continue;
            };
            let mut restored = item.clone();
            restored.current_reference = reference;
            restored_items.push(restored);
        }
        let restored_count = restored_items.len(); // 可以继续进入相反方向的实际恢复数量。
        let changed_count = total.saturating_sub(restored_count); // Shell 外部状态变化或缺失结果引用数量。
        let completed_at = crate::tools::time::next_timeline_timestamp(); // 本轮补偿进入统一时间线的新完成时间。
        result.operation_id = entry.result.operation_id;
        result.completed_at_unix_micros = completed_at;
        if changed_count > 0 {
            result.message = if redo {
                format!("已重做 {restored_count} 项，{changed_count} 项状态已变化")
            } else {
                format!("已恢复 {restored_count} 项，{changed_count} 项状态已变化")
            };
        }
        let mut history = self
            .operation_history
            .lock()
            .map_err(|_| DesktopItemError::snapshot_unavailable())?; // 原生补偿完成后等待提交的 Shell 历史。
        let current = history
            .get_mut(history_index)
            .filter(|current| {
                current.result.operation_id == entry.result.operation_id
                    && current.state == expected_state
            })
            .ok_or_else(DesktopItemError::snapshot_unavailable)?;
        if restored_count > 0 {
            current.items = restored_items;
            current.state = if redo {
                ShellHistoryState::Applied
            } else {
                ShellHistoryState::Undone
            };
            current.timeline_at_unix_micros = completed_at;
        } else if !result.aborted {
            current.state = ShellHistoryState::Invalid;
        }
        crate::log_info!(
            "Shell：统一历史补偿完成，标识={}，方向={}，恢复={}，变化={}",
            entry.result.operation_id,
            if redo { "重做" } else { "撤销" },
            restored_count,
            changed_count
        );
        Ok(Some(result))
    }

    /// 为改变 Shell 事实的操作分配标识并加入会话记录。
    fn record_operation(
        &self,
        mut result: ShellOperationResult,
    ) -> Result<ShellOperationResult, DesktopItemError> {
        if (result.aborted && !result.items.iter().any(|item| item.succeeded))
            || !matches!(
                result.kind,
                ShellOperationKind::Rename
                    | ShellOperationKind::Recycle
                    | ShellOperationKind::PermanentDelete
                    | ShellOperationKind::Paste
            )
        {
            return Ok(result);
        }
        result.operation_id = self.next_operation_id.fetch_add(1, Ordering::Relaxed);
        result.completed_at_unix_micros = crate::tools::time::next_timeline_timestamp();
        let items = shell_history_items(&result); // 当前操作中可以明确执行补偿的成功项目集合。
        let state = if result.reversible && !items.is_empty() {
            ShellHistoryState::Applied
        } else {
            ShellHistoryState::Invalid
        }; // 永久删除和缺少结果引用的记录不进入可执行时间线。
        let mut history = self
            .operation_history
            .lock()
            .map_err(|_| DesktopItemError::snapshot_unavailable())?; // 等待提交新 Shell 分支的统一历史。
        for entry in history
            .iter_mut()
            .filter(|entry| entry.state == ShellHistoryState::Undone)
        {
            entry.state = ShellHistoryState::Invalid;
        }
        history.push(ShellHistoryEntry {
            timeline_at_unix_micros: result.completed_at_unix_micros,
            result: result.clone(),
            state,
            items,
        });
        crate::log_info!(
            "Shell：操作结果已记录，标识={}，类型={:?}，可逆={}，永久={}，取消={}",
            result.operation_id,
            result.kind,
            u8::from(result.reversible),
            u8::from(result.permanent),
            u8::from(result.aborted)
        );
        Ok(result)
    }

    /// 在专用 owner HWND 显示当前语义目标的原生菜单。
    pub fn show_menu(
        &self,
        target: NativeMenuTarget,
        screen_x: i32,
        screen_y: i32,
        product_commands: Vec<NativeMenuCommand>,
    ) -> Result<Option<u32>, DesktopItemError> {
        let selected = self
            .runtime
            .show_menu(target, screen_x, screen_y, product_commands)
            .map_err(|error| {
                crate::log_error!("Shell：原生上下文菜单失败，错误：{error}");
                DesktopItemError::shell_operation_failed(&error)
            })?;
        crate::log_debug!(
            "Shell：原生上下文菜单已关闭，产品命令={}",
            selected
                .map(|command| command.to_string())
                .unwrap_or_else(|| "无".to_string())
        );
        Ok(selected)
    }

    /// 将当前选择集合校验并转换为可解析 Shell 稳定引用。
    pub fn selection_references(
        &self,
        item_ids: &[String],
    ) -> Result<Vec<String>, DesktopItemError> {
        if item_ids.is_empty() {
            return Err(DesktopItemError::invalid_selection("桌面项目选择为空"));
        }
        // 当前请求中已经出现的项目引用。
        let mut selected = std::collections::HashSet::new();
        // 当前桌面项目权威快照。
        let snapshot = lock_snapshot(&self.snapshot)?;
        item_ids
            .iter()
            .map(|item_id| {
                if !selected.insert(item_id) {
                    return Err(DesktopItemError::invalid_selection(
                        "桌面项目选择包含重复项目",
                    ));
                }
                let item = snapshot
                    .items
                    .iter()
                    .find(|item| &item.stable_reference == item_id)
                    .ok_or_else(|| DesktopItemError::invalid_selection("桌面项目不存在"))?;
                if item.availability != DesktopItemAvailability::Available {
                    return Err(DesktopItemError::invalid_selection("桌面项目当前不可用"));
                }
                Ok(item.stable_reference.clone())
            })
            .collect()
    }
}

/// 从 Shell 实际结果筛选可以明确补偿的成功项目。
fn shell_history_items(result: &ShellOperationResult) -> Vec<ShellHistoryItem> {
    result
        .items
        .iter()
        .filter(|item| item.succeeded)
        .filter_map(|item| {
            let current_reference = item.result_reference.clone()?; // 原始操作后下一次补偿使用的稳定引用。
            let actionable = match result.kind {
                ShellOperationKind::Rename => {
                    item.source_name.is_some() && item.result_name.is_some()
                }
                ShellOperationKind::Recycle => {
                    item.source_parent_reference.is_some() && item.source_name.is_some()
                }
                ShellOperationKind::Paste if result.effect.as_deref() == Some("复制") => {
                    item.result_parent_reference.is_some() && item.result_name.is_some()
                }
                ShellOperationKind::Paste if result.effect.as_deref() == Some("移动") => {
                    item.source_parent_reference.is_some()
                        && item.source_name.is_some()
                        && item.result_parent_reference.is_some()
                        && item.result_name.is_some()
                }
                _ => false,
            }; // 当前操作类型完成撤销和重做所需的最小元数据集合。
            actionable.then(|| ShellHistoryItem {
                source_reference: item.source_reference.clone(),
                result_reference: current_reference.clone(),
                source_parent_reference: item.source_parent_reference.clone(),
                source_name: item.source_name.clone(),
                result_parent_reference: item.result_parent_reference.clone(),
                result_name: item.result_name.clone(),
                current_reference,
            })
        })
        .collect()
}

/// 将一个 Shell 历史单元转换为本轮新的 IFileOperation 动作。
fn shell_compensation_actions(
    entry: &ShellHistoryEntry,
    redo: bool,
) -> Vec<ShellCompensationAction> {
    entry
        .items
        .iter()
        .map(|item| match entry.result.kind {
            ShellOperationKind::Rename => ShellCompensationAction::Rename {
                reference: item.current_reference.clone(),
                new_name: if redo {
                    item.result_name.clone()
                } else {
                    item.source_name.clone()
                }
                .expect("可逆重命名已保存目标名称"),
            },
            ShellOperationKind::Recycle if redo => ShellCompensationAction::Recycle {
                reference: item.current_reference.clone(),
            },
            ShellOperationKind::Recycle => ShellCompensationAction::Move {
                reference: item.current_reference.clone(),
                destination_parent_reference: item
                    .source_parent_reference
                    .clone()
                    .expect("可逆回收已保存来源父级"),
                new_name: item.source_name.clone().expect("可逆回收已保存来源名称"),
            },
            ShellOperationKind::Paste
                if entry.result.effect.as_deref() == Some("复制") && !redo =>
            {
                ShellCompensationAction::Recycle {
                    reference: item.current_reference.clone(),
                }
            }
            ShellOperationKind::Paste if entry.result.effect.as_deref() == Some("复制") => {
                ShellCompensationAction::Move {
                    reference: item.current_reference.clone(),
                    destination_parent_reference: item
                        .result_parent_reference
                        .clone()
                        .expect("可逆复制已保存结果父级"),
                    new_name: item.result_name.clone().expect("可逆复制已保存结果名称"),
                }
            }
            ShellOperationKind::Paste => ShellCompensationAction::Move {
                reference: item.current_reference.clone(),
                destination_parent_reference: if redo {
                    item.result_parent_reference.clone()
                } else {
                    item.source_parent_reference.clone()
                }
                .expect("可逆移动已保存目标父级"),
                new_name: if redo {
                    item.result_name.clone()
                } else {
                    item.source_name.clone()
                }
                .expect("可逆移动已保存目标名称"),
            },
            _ => unreachable!("只有重命名、回收和粘贴进入 Shell 可逆历史"),
        })
        .collect()
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
    use super::{
        ShellHistoryEntry, ShellHistoryState, publish_observation, reconcile_desktop_items,
        shell_compensation_actions, shell_history_items,
    };
    use crate::contracts::{
        DesktopItem, DesktopItemAvailability, DesktopItemSnapshot, ShellOperationItemResult,
        ShellOperationKind, ShellOperationResult,
    };
    use crate::platform::windows::ShellCompensationAction;

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

    /// 验证补偿日志只接纳同时具有实际结果引用和所需父级名称的项目。
    #[test]
    fn reversible_shell_history_requires_complete_native_metadata() {
        let result = ShellOperationResult {
            operation_id: 3,
            completed_at_unix_micros: 10,
            kind: ShellOperationKind::Recycle,
            aborted: false,
            reversible: true,
            permanent: false,
            effect: None,
            items: vec![
                ShellOperationItemResult {
                    source_reference: "desktop-old".to_string(),
                    result_reference: Some("recycle-item".to_string()),
                    succeeded: true,
                    result_code: 0,
                    source_parent_reference: Some("desktop-parent".to_string()),
                    source_name: Some("old.txt".to_string()),
                    result_parent_reference: Some("recycle-parent".to_string()),
                    result_name: Some("old.txt".to_string()),
                },
                ShellOperationItemResult {
                    source_reference: "missing-result".to_string(),
                    result_reference: None,
                    succeeded: true,
                    result_code: 0,
                    source_parent_reference: Some("desktop-parent".to_string()),
                    source_name: Some("missing.txt".to_string()),
                    result_parent_reference: None,
                    result_name: None,
                },
            ],
            message: "移入回收站完成：2 项".to_string(),
        }; // 一个可明确恢复和一个缺少结果引用的 Shell 实际结果。

        let items = shell_history_items(&result);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].current_reference, "recycle-item");
    }

    /// 验证回收站撤销和重做分别生成移动恢复与再次回收动作。
    #[test]
    fn recycle_history_builds_inverse_native_actions() {
        let result = ShellOperationResult {
            operation_id: 4,
            completed_at_unix_micros: 20,
            kind: ShellOperationKind::Recycle,
            aborted: false,
            reversible: true,
            permanent: false,
            effect: None,
            items: vec![ShellOperationItemResult {
                source_reference: "desktop-old".to_string(),
                result_reference: Some("recycle-item".to_string()),
                succeeded: true,
                result_code: 0,
                source_parent_reference: Some("desktop-parent".to_string()),
                source_name: Some("old.txt".to_string()),
                result_parent_reference: Some("recycle-parent".to_string()),
                result_name: Some("old.txt".to_string()),
            }],
            message: "移入回收站完成：1 项".to_string(),
        }; // 已具备完整补偿元数据的回收结果。
        let entry = ShellHistoryEntry {
            items: shell_history_items(&result),
            result,
            state: ShellHistoryState::Applied,
            timeline_at_unix_micros: 20,
        };

        let undo = shell_compensation_actions(&entry, false);
        let redo = shell_compensation_actions(&entry, true);

        assert!(matches!(
            &undo[0],
            ShellCompensationAction::Move {
                reference,
                destination_parent_reference,
                new_name
            } if reference == "recycle-item"
                && destination_parent_reference == "desktop-parent"
                && new_name == "old.txt"
        ));
        assert!(matches!(
            &redo[0],
            ShellCompensationAction::Recycle { reference } if reference == "recycle-item"
        ));
    }
}
