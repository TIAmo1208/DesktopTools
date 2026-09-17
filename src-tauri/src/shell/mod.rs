use crate::contracts::{ShellDragPreparation, WorkspacePoint};
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError};
use std::sync::{Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::Ole::DROPEFFECT;
use windows::core::AgileReference;

use crate::contracts::{DesktopItem, DesktopItemAvailability, DesktopItemError, DesktopItemSnapshot, ShellOperationKind, ShellOperationResult};
use crate::platform::windows::{NativeMenuCommand, NativeMenuTarget, ShellChangeEvent, ShellChangeWatcher, ShellCompensationAction, ShellMenuOwner, compensate_desktop_items, delete_desktop_items, enumerate_desktop_items, initialize_shell_apartment, open_desktop_items, paste_desktop_items, paste_folder_portal_items, rename_desktop_item, set_desktop_item_clipboard, set_desktop_item_clipboard_for_paths, show_native_context_menu, start_shell_change_watcher, uninitialize_shell_apartment};

/// 表示发送到专用 Shell STA 线程的请求。
enum ShellRequest
{
    /// 在原指针拖动仍有效时准备完整数据对象，失败返回内部布局回退。
    PrepareDrag
    {
        references: Vec<String>, portal_source: bool, reply: SyncSender<Result<Option<ShellDragPreparation>, String>>
    },
    /// 释放未启动的单次准备，不影响其他窗口更新的准备。
    CancelPreparedDrag(u64),
    /// 枚举当前完整桌面项目集合。
    Enumerate(SyncSender<Result<Vec<DesktopItem>, String>>),
    /// 使用 Shell 默认动词打开完整项目选择集合。
    Open
    {
        /// 已通过快照校验的稳定项目引用。
        references: Vec<String>,
        /// 等待 Shell 操作结果的同步通道。
        reply: SyncSender<Result<(), String>>,
    },
    /// 使用 IFileOperation 重命名单个项目。
    Rename
    {
        /// 已通过快照校验的稳定项目引用。
        reference: String,
        /// 用户提交的新项目名称。
        new_name: String,
        /// 等待 Shell 实际结果的同步通道。
        reply: SyncSender<Result<ShellOperationResult, String>>,
    },
    /// 使用 IFileOperation 删除完整选择集合。
    Delete
    {
        /// 已通过快照校验的稳定项目引用。
        references: Vec<String>,
        /// 是否跳过回收站并明确永久删除。
        permanent: bool,
        /// 等待 Shell 实际结果的同步通道。
        reply: SyncSender<Result<ShellOperationResult, String>>,
    },
    /// 将完整选择集合写入系统 Shell 剪贴板。
    SetClipboard
    {
        /// 已通过快照校验的稳定项目引用。
        references: Vec<String>,
        /// 是否使用剪切而不是复制效果。
        cut: bool,
        /// 等待系统剪贴板结果的同步通道。
        reply: SyncSender<Result<ShellOperationResult, String>>,
    },
    /// 将门户绝对路径集合写入系统剪贴板。
    SetClipboardPaths
    {
        references: Vec<String>, cut: bool, reply: SyncSender<Result<ShellOperationResult, String>>
    },
    /// 将系统 Shell 剪贴板粘贴到桌面命名空间。
    Paste(SyncSender<Result<ShellOperationResult, String>>),
    /// 将系统剪贴板粘贴到指定门户目录。
    PasteFolder
    {
        directory: String, reply: SyncSender<Result<ShellOperationResult, String>>
    },
    /// 经 COM 封送在 Shell STA 执行原生投放，结果通道不阻塞工作线程。
    Transfer
    {
        references: Vec<String>,                             // 混合投放中明确需要真实传输的顶层来源。
        data: AgileReference<IDataObject>,                   // 在目标 apartment 解析的数据对象代理。
        effect: DROPEFFECT,                                  // 已协商的复制或移动效果。
        reply: Sender<Result<ShellOperationResult, String>>, // 宿主通过消息泵等待的结果通道。
    },
    /// 将已校验文件路径传输到门户绑定目录。
    TransferFolder
    {
        effect: DROPEFFECT, references: Vec<String>, directory: String, reply: Sender<Result<ShellOperationResult, String>>
    },
    /// 消费准备凭据并封送完整数据，真正拖放由接收输入的宿主 UI STA 执行。
    TakePreparedDrag
    {
        token: u64,                                           // 只能使用一次的准备凭据。
        reply: SyncSender<Result<PreparedShellDrag, String>>, // 不在 Shell STA 等待拖放结束。
    },
    /// 使用新的 IFileOperation 执行统一时间线补偿。
    Compensate
    {
        proofs: Vec<crate::platform::windows::file_proof::FileProof>, // 与动作一一对应的结果状态证据。
        /// 原始 Shell 操作类型。
        kind: ShellOperationKind,
        /// 本轮实际可执行的补偿项目集合。
        actions: Vec<ShellCompensationAction>,
        /// 等待 Shell 补偿结果的同步通道。
        reply: SyncSender<Result<ShellOperationResult, String>>,
    },
    /// 在专用 owner HWND 显示原生上下文菜单。
    ShowMenu
    {
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

/// 原子消费后的拖放输入，COM 引用必须在接收方 STA 重新解析。
pub(crate) struct PreparedShellDrag
{
    pub data: AgileReference<IDataObject>, // 原始完整数据对象的合法封送引用。
    pub references: Vec<String>,           // 与原数据对象一致的完整来源身份。
    pub portal_source: bool,               // 门户来源必须执行文件传输，不走内部布局提交。
}

/// 持有专用 Shell STA 线程及其请求通道。
struct ShellStaRuntime
{
    /// Shell 请求发送端。
    sender: Sender<ShellRequest>,
    /// 请求到达时唤醒 STA 联合等待的自动复位事件。
    request_event: isize,
    /// Shell STA 后台线程。
    worker: Option<JoinHandle<()>>,
}

/// 等待 Shell 请求事件或当前 STA 的 Windows/COM 消息。
fn wait_for_shell_activity(request_event: windows::Win32::Foundation::HANDLE) -> Result<(), String>
{
    use windows::Win32::Foundation::WAIT_OBJECT_0;
    use windows::Win32::System::Threading::INFINITE;
    use windows::Win32::UI::WindowsAndMessaging::{MsgWaitForMultipleObjects, QS_ALLINPUT};
    let result = unsafe { MsgWaitForMultipleObjects(Some(&[request_event]), false, INFINITE, QS_ALLINPUT) }; // 空闲时无限阻塞，请求或消息到达后立即返回。
    if result == WAIT_OBJECT_0 || result.0 == WAIT_OBJECT_0.0 + 1
    {
        return Ok(());
    }
    Err(format!("Shell STA 等待请求或消息失败，状态：{}", result.0))
}

impl ShellStaRuntime
{
    /// 创建并验证专用 Shell STA 线程。
    fn start() -> Result<Self, DesktopItemError>
    {
        // Shell 请求通道。
        let (sender, receiver) = mpsc::channel();
        // 请求发送后唤醒 STA 的自动复位事件。
        let request_event = unsafe { windows::Win32::System::Threading::CreateEventW(None, false, false, None) }.map_err(|error| DesktopItemError::shell_enumeration_failed(&format!("创建 Shell STA 请求事件失败：{error}")))?;
        // 线程和运行时共同引用的事件句柄值，由运行时在工作线程结束后关闭。
        let request_event_raw = request_event.0 as isize;
        // Shell STA 初始化结果通道。
        let (ready, initialized) = mpsc::sync_channel(0);
        // 专用 Shell STA 后台线程。
        let worker = thread::Builder::new()
            .name("desktop-tools-shell-sta".to_string())
            .spawn(move || {
                if let Err(error) = initialize_shell_apartment()
                {
                    let _ = ready.send(Err(error));
                    return;
                }
                // 当前 STA 线程独占的专用菜单 owner HWND。
                let mut menu_owner = match ShellMenuOwner::create()
                {
                    Ok(owner) => owner,
                    Err(error) =>
                    {
                        uninitialize_shell_apartment();
                        let _ = ready.send(Err(error));
                        return;
                    }
                };
                let _ = ready.send(Ok(()));
                let mut prepared_drag: Option<(u64, Vec<String>, IDataObject, bool)> = None; // 当前唯一未消费准备，COM 对象始终留在所属 STA。
                let mut next_drag_token = 0_u64; // 当前进程内递增的准备凭据。
                let mut transfers = Vec::<JoinHandle<()>>::new(); // 长文件操作使用独立 STA，菜单、打开和源拖动继续响应。

                'requests: loop
                {
                    use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage, WM_QUIT};
                    let mut message = MSG::default(); // 空闲时也必须处理 Shell COM 回调与剪贴板消息。
                    while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool()
                    {
                        if message.message == WM_QUIT
                        {
                            break 'requests;
                        }
                        unsafe {
                            let _ = TranslateMessage(&message);
                            DispatchMessageW(&message);
                        }
                    }
                    let request = match receiver.try_recv()
                    {
                        Ok(request) => request,
                        Err(TryRecvError::Empty) =>
                        {
                            let event = windows::Win32::Foundation::HANDLE(request_event_raw as *mut c_void); // 仅借用运行时持有的事件句柄。
                            if let Err(error) = wait_for_shell_activity(event)
                            {
                                crate::log_error!("Shell：{error}");
                                break;
                            }
                            continue;
                        }
                        Err(TryRecvError::Disconnected) => break,
                    }; // 已排队请求不等待；空队列才进入请求与消息的联合无限等待。
                    match request
                    {
                        ShellRequest::PrepareDrag { references, portal_source, reply } =>
                        {
                            prepared_drag = None;
                            let result = match crate::platform::windows::drag_data_object(windows::Win32::Foundation::HWND::default(), &references, portal_source)
                            {
                                Ok(data) =>
                                {
                                    next_drag_token += 1;
                                    prepared_drag = Some((next_drag_token, references, data, portal_source));
                                    use windows::Win32::UI::HiDpi::GetSystemMetricsForDpi;
                                    use windows::Win32::UI::WindowsAndMessaging::{SM_CXDRAG, SM_CYDRAG};
                                    Ok(Some(ShellDragPreparation { token: next_drag_token, threshold: WorkspacePoint { left: unsafe { GetSystemMetricsForDpi(SM_CXDRAG, 96) }.max(1) as f64, top: unsafe { GetSystemMetricsForDpi(SM_CYDRAG, 96) }.max(1) as f64 } }))
                                }
                                Err(error) =>
                                {
                                    crate::log_debug!("拖放：完整选择无法创建 Shell 数据对象，保留内部布局：{error}");
                                    Ok(None)
                                }
                            }; // 不缩减项目集合，也不改变原指针会话。
                            if reply.send(result).is_err()
                            {
                                prepared_drag = None;
                            }
                        }
                        ShellRequest::CancelPreparedDrag(token) =>
                        {
                            if prepared_drag.as_ref().is_some_and(|prepared| prepared.0 == token)
                            {
                                prepared_drag = None;
                            }
                        }
                        ShellRequest::Enumerate(reply) =>
                        {
                            let _ = reply.send(enumerate_desktop_items());
                        }
                        ShellRequest::Open { references, reply } =>
                        {
                            let _ = reply.send(open_desktop_items(&references));
                        }
                        ShellRequest::Rename { reference, new_name, reply } =>
                        {
                            let _ = reply.send(rename_desktop_item(&menu_owner, &reference, &new_name));
                        }
                        ShellRequest::Delete { references, permanent, reply } =>
                        {
                            let _ = reply.send(delete_desktop_items(&menu_owner, &references, permanent));
                        }
                        ShellRequest::SetClipboard { references, cut, reply } =>
                        {
                            let _ = reply.send(set_desktop_item_clipboard(&menu_owner, &references, cut));
                        }
                        ShellRequest::SetClipboardPaths { references, cut, reply } =>
                        {
                            let _ = reply.send(set_desktop_item_clipboard_for_paths(&references, cut));
                        }
                        ShellRequest::Paste(reply) =>
                        {
                            let _ = reply.send(paste_desktop_items(&menu_owner));
                        }
                        ShellRequest::PasteFolder { directory, reply } =>
                        {
                            let _ = reply.send(paste_folder_portal_items(&menu_owner, &directory));
                        }
                        ShellRequest::Transfer { data, effect, references, reply } =>
                        {
                            transfers.retain(|worker| !worker.is_finished());
                            let completed_reply = reply.clone(); // 线程建立失败仍能返回明确错误。
                            match thread::Builder::new().name("desktop-tools-file-transfer".into()).spawn(move || {
                                if let Err(error) = initialize_shell_apartment()
                                {
                                    let _ = completed_reply.send(Err(error));
                                    return;
                                }
                                let result = (|| {
                                    let owner = ShellMenuOwner::create()?;
                                    let object = data.resolve().map_err(|error| format!("解析投放数据代理失败：{error}"))?;
                                    crate::platform::windows::transfer_desktop_data(&owner, &object, effect, Some(&references))
                                })();
                                drop(data); // 发完成通知前释放所有跨 apartment 引用。
                                uninitialize_shell_apartment();
                                let _ = completed_reply.send(result);
                            })
                            {
                                Ok(worker) => transfers.push(worker),
                                Err(error) =>
                                {
                                    let _ = reply.send(Err(format!("启动文件传输线程失败：{error}")));
                                }
                            }
                        }
                        ShellRequest::TransferFolder { effect, references, directory, reply } =>
                        {
                            transfers.retain(|worker| !worker.is_finished());
                            let completed_reply = reply.clone();
                            match thread::Builder::new().name("desktop-tools-folder-transfer".into()).spawn(move || {
                                if let Err(error) = initialize_shell_apartment()
                                {
                                    let _ = completed_reply.send(Err(error));
                                    return;
                                }
                                let result = ShellMenuOwner::create().and_then(|owner| crate::platform::windows::transfer_paths_to_directory(&owner, effect, Some(&directory), &references));
                                uninitialize_shell_apartment();
                                let _ = completed_reply.send(result);
                            })
                            {
                                Ok(worker) => transfers.push(worker),
                                Err(error) =>
                                {
                                    let _ = reply.send(Err(format!("门户文件传输线程启动失败：{error}")));
                                }
                            }
                        }
                        ShellRequest::TakePreparedDrag { token, reply } =>
                        {
                            let result = if prepared_drag.as_ref().is_some_and(|prepared| prepared.0 == token)
                            {
                                let (_, references, data, portal_source) = prepared_drag.take().expect("准备凭据已核实"); // 启动之前消费，不能重放。
                                AgileReference::new(&data).map(|data| PreparedShellDrag { data, references, portal_source }).map_err(|error| format!("封送源拖放数据失败：{error}"))
                            }
                            else
                            {
                                Err("拖动准备已失效".to_string())
                            };
                            let _ = reply.send(result);
                        }
                        ShellRequest::Compensate { kind, actions, proofs, reply } =>
                        {
                            transfers.retain(|worker| !worker.is_finished());
                            let completed_reply = reply.clone(); // 创建失败也必须通知等待者。
                            match thread::Builder::new().name("desktop-tools-file-compensation".into()).spawn(move || {
                                if let Err(error) = initialize_shell_apartment()
                                {
                                    let _ = completed_reply.send(Err(error));
                                    return;
                                }
                                let result = ShellMenuOwner::create().and_then(|owner| compensate_desktop_items(&owner, kind, actions, proofs)); // owner 在所属 STA 内释放。
                                uninitialize_shell_apartment();
                                let _ = completed_reply.send(result);
                            })
                            {
                                Ok(worker) => transfers.push(worker),
                                Err(error) =>
                                {
                                    let _ = reply.send(Err(format!("启动文件补偿线程失败：{error}")));
                                }
                            }
                        }
                        ShellRequest::ShowMenu { target, screen_x, screen_y, product_commands, reply } =>
                        {
                            crate::log_info!("菜单：Shell STA 开始处理，位置=({screen_x},{screen_y})");
                            let _ = reply.send(show_native_context_menu(&mut menu_owner, target, screen_x, screen_y, &product_commands));
                            crate::log_info!("菜单：Shell STA 处理结束");
                        }
                        ShellRequest::Shutdown => break,
                    }
                }

                drop(prepared_drag);
                for worker in transfers
                {
                    let _ = worker.join();
                } // 退出时保留真实文件结果，不能中断一半后伪报取消。
                uninitialize_shell_apartment();
            })
            .map_err(|error| {
                let _ = unsafe { windows::Win32::Foundation::CloseHandle(request_event) };
                DesktopItemError::shell_enumeration_failed(&error)
            })?;
        let initialization = match initialized.recv()
        {
            Ok(initialization) => initialization,
            Err(error) =>
            {
                let _ = worker.join();
                let _ = unsafe { windows::Win32::Foundation::CloseHandle(request_event) };
                return Err(DesktopItemError::shell_enumeration_failed(&error));
            }
        }; // 初始化失败前工作线程已经结束，可以安全回收事件句柄。
        if let Err(error) = initialization
        {
            let _ = worker.join();
            let _ = unsafe { windows::Win32::Foundation::CloseHandle(request_event) };
            return Err(DesktopItemError::shell_enumeration_failed(&error));
        }

        crate::log_info!("Shell：专用 STA 运行时已启动");
        Ok(Self { sender, request_event: request_event_raw, worker: Some(worker) })
    }

    /// 将请求入队后设置事件，避免 STA 在空闲联合等待中遗漏唤醒。
    fn send(&self, request: ShellRequest) -> Result<(), String>
    {
        self.sender.send(request).map_err(|_| "Shell STA 线程已经停止".to_string())?;
        let event = windows::Win32::Foundation::HANDLE(self.request_event as *mut c_void); // 运行时持有到工作线程退出。
        unsafe { windows::Win32::System::Threading::SetEvent(event) }.map_err(|error| format!("唤醒 Shell STA 线程失败：{error}"))
    }

    /// 在专用 STA 线程枚举桌面项目。
    fn enumerate(&self) -> Result<Vec<DesktopItem>, String>
    {
        // 等待 Shell 枚举结果的同步通道。
        let (reply, response) = mpsc::sync_channel(0);
        self.send(ShellRequest::Enumerate(reply))?;
        response.recv().map_err(|_| "Shell STA 线程已经停止".to_string())?
    }

    /// 在专用 STA 线程使用 Shell 默认动词打开项目。
    fn open(&self, references: Vec<String>) -> Result<(), String>
    {
        // 等待 Shell 打开结果的同步通道。
        let (reply, response) = mpsc::sync_channel(0);
        self.send(ShellRequest::Open { references, reply })?;
        response.recv().map_err(|_| "Shell STA 线程已经停止".to_string())?
    }

    /// 在专用 STA 线程重命名单个 Shell 项目。
    fn rename(&self, reference: String, new_name: String) -> Result<ShellOperationResult, String>
    {
        let (reply, response) = mpsc::sync_channel(0); // 等待 Shell 重命名结果的同步通道。
        self.send(ShellRequest::Rename { reference, new_name, reply })?;
        response.recv().map_err(|_| "Shell STA 线程已经停止".to_string())?
    }

    /// 在专用 STA 线程删除完整选择集合。
    fn delete(&self, references: Vec<String>, permanent: bool) -> Result<ShellOperationResult, String>
    {
        let (reply, response) = mpsc::sync_channel(0); // 等待 Shell 删除结果的同步通道。
        self.send(ShellRequest::Delete { references, permanent, reply })?;
        response.recv().map_err(|_| "Shell STA 线程已经停止".to_string())?
    }

    /// 在专用 STA 线程设置系统 Shell 剪贴板。
    fn set_clipboard(&self, references: Vec<String>, cut: bool) -> Result<ShellOperationResult, String>
    {
        let (reply, response) = mpsc::sync_channel(0); // 等待系统剪贴板结果的同步通道。
        self.send(ShellRequest::SetClipboard { references, cut, reply })?;
        response.recv().map_err(|_| "Shell STA 线程已经停止".to_string())?
    }

    /// 在所属 STA 将门户路径集合写入剪贴板。
    fn set_clipboard_paths(&self, references: Vec<String>, cut: bool) -> Result<ShellOperationResult, String>
    {
        let (reply, response) = mpsc::sync_channel(0);
        self.send(ShellRequest::SetClipboardPaths { references, cut, reply })?;
        response.recv().map_err(|_| "Shell STA 线程已经停止".to_string())?
    }

    /// 在专用 STA 线程粘贴系统 Shell 剪贴板。
    fn paste(&self) -> Result<ShellOperationResult, String>
    {
        let (reply, response) = mpsc::sync_channel(0); // 等待 Shell 粘贴结果的同步通道。
        self.send(ShellRequest::Paste(reply))?;
        response.recv().map_err(|_| "Shell STA 线程已经停止".to_string())?
    }

    /// 在专用 STA 粘贴到门户当前目录。
    fn paste_folder(&self, directory: String) -> Result<ShellOperationResult, String>
    {
        let (reply, response) = mpsc::sync_channel(0);
        self.send(ShellRequest::PasteFolder { directory, reply })?;
        response.recv().map_err(|_| "Shell STA 线程已经停止".to_string())?
    }

    /// 在所属 STA 原子消费凭据并建立可跨 apartment 解析的引用。
    fn take_prepared_drag(&self, token: u64) -> Result<PreparedShellDrag, String>
    {
        let (reply, response) = mpsc::sync_channel(0); // 只等待准备，不占用整个拖放生命周期。
        self.send(ShellRequest::TakePreparedDrag { token, reply })?;
        response.recv().map_err(|_| "Shell STA 线程已经停止".to_string())?
    }

    /// 在专用 STA 线程执行一轮 Shell 补偿操作。
    fn compensate(&self, kind: ShellOperationKind, actions: Vec<ShellCompensationAction>, proofs: Vec<crate::platform::windows::file_proof::FileProof>) -> Result<ShellOperationResult, String>
    {
        let (reply, response) = mpsc::sync_channel(0); // 等待 Shell 补偿结果的同步通道。
        self.send(ShellRequest::Compensate { kind, actions, proofs, reply })?;
        response.recv().map_err(|_| "Shell STA 线程已经停止".to_string())?
    }

    /// 在专用 STA 线程显示原生上下文菜单。
    fn show_menu(&self, target: NativeMenuTarget, screen_x: i32, screen_y: i32, product_commands: Vec<NativeMenuCommand>) -> Result<Option<u32>, String>
    {
        // 等待原生菜单选择结果的同步通道。
        let (reply, response) = mpsc::sync_channel(0);
        self.send(ShellRequest::ShowMenu { target, screen_x, screen_y, product_commands, reply })?;
        response.recv().map_err(|_| "Shell STA 线程已经停止".to_string())?
    }
}

impl Drop for ShellStaRuntime
{
    /// 请求 Shell STA 线程释放 COM apartment 并退出。
    fn drop(&mut self)
    {
        let _ = self.send(ShellRequest::Shutdown);
        if let Some(worker) = self.worker.take()
        {
            let _ = worker.join();
        }
        let event = windows::Win32::Foundation::HANDLE(self.request_event as *mut c_void); // 工作线程退出后释放唯一事件句柄。
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(event) };
        self.request_event = 0;
    }
}

/// 持有桌面项目最近有效快照与专用 Shell STA 运行时。
pub struct DesktopItemState
{
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
enum ShellHistoryState
{
    /// 原始操作已应用，可以撤销。
    Applied,
    /// 原始操作已撤销，可以重做。
    Undone,
    /// 外部变化或缺失结果引用使记录不再可执行。
    Invalid,
}

/// 一个 Shell 项目在补偿链路中保留的原始目标和当前引用。
#[derive(Clone)]
struct ShellHistoryItem
{
    state: ShellHistoryState,                                       // 每项独立进入撤销或重做，部分失败不丢弃可重试项目。
    proof: Option<crate::platform::windows::file_proof::FileProof>, // 当前文件树的安全依据。
    blocked: Option<String>,                                        // 无法自动补偿的明确原因。
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
struct ShellHistoryEntry
{
    busy: bool,                                          // 从文件补偿到布局落盘保持占用，防止重复执行。
    layout: Option<crate::workspace::DropLayoutChange>,  // 外部投放的关联布局，不单独进入布局历史。
    layout_states: HashMap<String, ShellHistoryState>,   // 只改布局项目的独立方向。
    pending_layout: Vec<(String, String, String, bool)>, // 原结果、补偿前引用、补偿后引用、重做方向。
    /// 首次 Shell 操作公开结果。
    result: ShellOperationResult,
    /// 当前记录可以撤销、重做或已经失效。
    state: ShellHistoryState,
    /// 最近进入当前方向的 Unix 微秒时间。
    timeline_at_unix_micros: i64,
    /// 仍能明确匹配并参与后续补偿的项目集合。
    items: Vec<ShellHistoryItem>,
}

impl DesktopItemState
{
    /// 创建桌面项目状态与专用 Shell STA 运行时。
    pub fn new() -> Result<Self, DesktopItemError>
    {
        // 已注册桌面命名空间通知的变化监听线程与事件接收端。
        let (change_watcher, change_receiver) = start_shell_change_watcher().map_err(|error| DesktopItemError::shell_operation_failed(&error))?;
        Ok(Self { snapshot: Mutex::new(DesktopItemSnapshot::default()), runtime: ShellStaRuntime::start()?, _change_watcher: change_watcher, change_receiver: Mutex::new(Some(change_receiver)), operation_history: Mutex::new(Vec::new()), next_operation_id: AtomicU64::new(1) })
    }

    /// 将 Shell 变化事件接收端交给唯一应用协调线程。
    pub fn take_change_receiver(&self) -> Result<Receiver<ShellChangeEvent>, DesktopItemError> { self.change_receiver.lock().map_err(|_| DesktopItemError::snapshot_unavailable())?.take().ok_or_else(|| DesktopItemError::invalid_selection("Shell 变化协调线程已经启动")) }

    /// 枚举 Shell 桌面项目并发布新的不可变快照。
    pub fn refresh(&self) -> Result<DesktopItemSnapshot, DesktopItemError>
    {
        // 当前 Shell 枚举观察结果。
        let observation = self.runtime.enumerate();
        if let Err(error) = &observation
        {
            crate::log_error!("Shell：桌面项目枚举失败，错误：{error}");
        }

        // 等待提交观察结果的桌面项目快照。
        let mut snapshot = lock_snapshot(&self.snapshot)?;
        let snapshot = publish_observation(&mut snapshot, observation)?;
        // 当前快照中等待 Shell 恢复的项目数量。
        let unavailable_count = snapshot.items.iter().filter(|item| item.availability == DesktopItemAvailability::Unavailable).count();
        crate::log_debug!("Shell：桌面项目快照已发布，修订：{}，项目数量：{}，等待恢复：{}", snapshot.revision, snapshot.items.len(), unavailable_count);

        Ok(snapshot)
    }

    /// 返回最近一次有效桌面项目快照。
    pub fn snapshot(&self) -> Result<DesktopItemSnapshot, DesktopItemError> { Ok(lock_snapshot(&self.snapshot)?.clone()) }

    /// 使用 Shell 默认动词打开当前快照中的完整选择集合。
    pub fn open(&self, item_ids: &[String]) -> Result<(), DesktopItemError>
    {
        // 已通过当前快照可用性校验的稳定引用。
        let references = self.selection_references(item_ids)?;
        self.runtime.open(references).map_err(|error| {
            crate::log_error!("Shell：桌面项目打开失败，错误：{error}");
            DesktopItemError::shell_operation_failed(&error)
        })?;
        crate::log_debug!("Shell：桌面项目已打开，项目数量：{}", item_ids.len());
        Ok(())
    }

    /// 重命名单个当前可用项目并记录 Shell 确认结果。
    pub fn rename(&self, item_id: &str, new_name: String) -> Result<ShellOperationResult, DesktopItemError>
    {
        let references = self.selection_references(&[item_id.to_string()])?; // 已通过快照校验的单一稳定引用。
        let result = self.runtime.rename(references[0].clone(), new_name).map_err(|error| {
            crate::log_error!("Shell：桌面项目重命名失败，错误：{error}");
            DesktopItemError::shell_operation_failed(&error)
        })?; // Shell 确认的实际重命名结果。
        self.record_operation(result)
    }

    /// 删除当前完整选择集合并记录可逆或永久属性。
    pub fn delete(&self, item_ids: &[String], permanent: bool) -> Result<ShellOperationResult, DesktopItemError>
    {
        let references = self.selection_references(item_ids)?; // 已通过快照校验的稳定引用集合。
        let result = self.runtime.delete(references, permanent).map_err(|error| {
            crate::log_error!("Shell：桌面项目删除失败，永久={}，错误：{error}", u8::from(permanent));
            DesktopItemError::shell_operation_failed(&error)
        })?; // Shell 确认的实际删除结果。
        self.record_operation(result)
    }

    /// 门户边界校验后的路径复用同一 STA 和原生回收操作，保留部分成功及撤销记录。
    pub(crate) fn recycle_portal_items(&self, references: Vec<String>) -> Result<ShellOperationResult, DesktopItemError>
    {
        let result = self.runtime.delete(references, false).map_err(|error| DesktopItemError::shell_operation_failed(&error))?; // 普通 Delete 必须走系统回收行为。
        self.record_operation(result)
    }

    /// 门户已校验单项使用同一 STA 原生重命名与撤销记录。
    pub(crate) fn rename_portal_item(&self, reference: String, new_name: String) -> Result<ShellOperationResult, DesktopItemError>
    {
        let result = self.runtime.rename(reference, new_name).map_err(|error| DesktopItemError::shell_operation_failed(&error))?; // Shell 负责实际文件语义。
        self.record_operation(result)
    }

    /// 门户路径集合写入系统剪贴板。
    pub(crate) fn set_portal_clipboard(&self, references: Vec<String>, cut: bool) -> Result<ShellOperationResult, DesktopItemError> { self.runtime.set_clipboard_paths(references, cut).map_err(|error| DesktopItemError::shell_operation_failed(&error)) }

    /// 将当前完整选择集合复制或剪切到系统 Shell 剪贴板。
    pub fn set_clipboard(&self, item_ids: &[String], cut: bool) -> Result<ShellOperationResult, DesktopItemError>
    {
        let references = self.selection_references(item_ids)?; // 已通过快照校验的稳定引用集合。
        let mut result = self.runtime.set_clipboard(references, cut).map_err(|error| {
            crate::log_error!("Shell：系统剪贴板写入失败，剪切={}，错误：{error}", u8::from(cut));
            DesktopItemError::shell_operation_failed(&error)
        })?; // 系统剪贴板接受完整 IDataObject 后的结果。
        result.completed_at_unix_micros = crate::tools::time::next_timeline_timestamp();
        crate::log_debug!("Shell：系统剪贴板已更新，剪切={}，项目数量={}", u8::from(cut), item_ids.len());
        Ok(result)
    }

    /// 将系统 Shell 剪贴板粘贴到桌面命名空间并记录实际结果。
    pub fn paste(&self) -> Result<ShellOperationResult, DesktopItemError>
    {
        let result = self.runtime.paste().map_err(|error| {
            crate::log_error!("Shell：桌面项目粘贴失败，错误：{error}");
            DesktopItemError::shell_operation_failed(&error)
        })?; // Shell 确认的粘贴结果。
        self.record_operation(result)
    }

    /// 将系统剪贴板原生粘贴到门户当前目录。
    pub(crate) fn paste_portal(&self, directory: String) -> Result<ShellOperationResult, DesktopItemError> { self.runtime.paste_folder(directory).map_err(|error| DesktopItemError::shell_operation_failed(&error)).and_then(|result| self.record_operation(result)) }

    /// 原生宿主提交已封送的投放数据，返回后宿主继续处理消息直到结果完成。
    pub(crate) fn begin_drop_transfer(&self, data: &IDataObject, effect: DROPEFFECT, references: Vec<String>) -> Result<Receiver<Result<ShellOperationResult, String>>, DesktopItemError>
    {
        let data = AgileReference::new(data).map_err(|error| DesktopItemError::shell_operation_failed(&format!("封送投放数据失败：{error}")))?; // COM 提供跨 apartment 的引用，不传递裸接口指针。
        let (reply, result) = mpsc::channel(); // 无阻塞结果交接，宿主取消等待也不阻塞 Shell STA。
        self.runtime.send(ShellRequest::Transfer { data, effect, references, reply }).map_err(|error| DesktopItemError::shell_operation_failed(&error))?;
        Ok(result)
    }

    /// 在 Shell STA 将封送数据传输到门户目录。
    pub(crate) fn begin_folder_drop_transfer(&self, effect: DROPEFFECT, references: Vec<String>, directory: String) -> Result<Receiver<Result<ShellOperationResult, String>>, DesktopItemError>
    {
        let (reply, result) = mpsc::channel();
        self.runtime.send(ShellRequest::TransferFolder { effect, references, directory, reply }).map_err(|error| DesktopItemError::shell_operation_failed(&error))?;
        Ok(result)
    }

    /// 文件处理完成后登记可补偿事实，由宿主继续协调共享归属。
    pub(crate) fn record_drop_transfer(&self, result: ShellOperationResult, layout: Option<crate::workspace::DropLayoutChange>) -> Result<ShellOperationResult, DesktopItemError> { self.record_operation_with_layout(result, layout) }

    /// 向应用层交接完整源数据，Shell STA 继续泵送跨 apartment 调用。
    pub(crate) fn take_prepared_drag(&self, token: u64) -> Result<PreparedShellDrag, DesktopItemError> { self.runtime.take_prepared_drag(token).map_err(|error| DesktopItemError::shell_operation_failed(&error)) }

    /// 完整选择不可用时保留内部拖动；仅成功准备才返回可启动凭据。
    pub fn prepare_drag(&self, item_ids: &[String]) -> Result<Option<ShellDragPreparation>, DesktopItemError>
    {
        let references = match self.selection_references(item_ids)
        {
            Ok(references) => references,
            Err(_) => return Ok(None),
        }; // 不从选择中丢弃失效项目。
        let (reply, response) = mpsc::sync_channel(0); // 指针会话不等待，前端异步接收准备结果。
        self.runtime.send(ShellRequest::PrepareDrag { references, portal_source: false, reply }).map_err(|error| DesktopItemError::shell_operation_failed(&error))?;
        response.recv().map_err(|error| DesktopItemError::shell_operation_failed(&error))?.map_err(|error| DesktopItemError::shell_operation_failed(&error))
    }

    /// 门户路径已经通过命令层校验；完整集合必须成功准备，不能回退布局拖动。
    pub(crate) fn prepare_portal_drag(&self, references: Vec<String>) -> Result<ShellDragPreparation, DesktopItemError>
    {
        let (reply, response) = mpsc::sync_channel(0);
        self.runtime.send(ShellRequest::PrepareDrag { references, portal_source: true, reply }).map_err(|error| DesktopItemError::shell_operation_failed(&error))?;
        response.recv().map_err(|error| DesktopItemError::shell_operation_failed(&error))?.map_err(|error| DesktopItemError::shell_operation_failed(&error))?.ok_or_else(|| DesktopItemError::shell_operation_failed(&"无法准备门户文件拖放"))
    }

    /// 单击、取消或失去捕获时释放尚未消费的 Shell 数据对象。
    pub fn cancel_prepared_drag(&self, token: u64) -> Result<(), DesktopItemError> { self.runtime.send(ShellRequest::CancelPreparedDrag(token)).map_err(|error| DesktopItemError::shell_operation_failed(&error)) }

    /// 返回指定方向最近一个有效 Shell 历史单元的时间。
    pub fn latest_shell_history_time(&self, redo: bool) -> Result<Option<i64>, DesktopItemError>
    {
        let history = self.operation_history.lock().map_err(|_| DesktopItemError::snapshot_unavailable())?; // 正在补偿时不允许另一个请求越过该操作。
        if history.iter().any(|entry| entry.busy)
        {
            return Err(DesktopItemError::shell_operation_failed(&"文件与布局历史正在恢复"));
        }
        Ok(history.iter().filter(|entry| shell_history_has_direction(entry, redo)).map(|entry| entry.timeline_at_unix_micros).max())
    }

    /// 新布局或 Shell 操作提交后使当前统一时间线之后的 Shell 重做链失效。
    pub fn clear_redo_history(&self) -> Result<(), DesktopItemError>
    {
        for entry in self.operation_history.lock().map_err(|_| DesktopItemError::snapshot_unavailable())?.iter_mut()
        {
            invalidate_shell_redo(entry);
        }
        Ok(())
    }

    /// 完整工作区还原后清除不再对应当前状态的全部 Shell 历史。
    pub fn clear_operation_history(&self) -> Result<(), DesktopItemError>
    {
        self.operation_history.lock().map_err(|_| DesktopItemError::snapshot_unavailable())?.clear();
        Ok(())
    }

    /// 精确外部重命名只更新当前路径，执行时仍按原身份和内容证明核验。
    pub fn reconcile_operation_history(&self, _snapshot: &DesktopItemSnapshot, renames: &[(String, String)]) -> Result<(), DesktopItemError>
    {
        let mut history = self.operation_history.lock().map_err(|_| DesktopItemError::snapshot_unavailable())?; // 文件在桌面外也可参与补偿，不能根据桌面枚举推断丢失。
        for entry in history.iter_mut().filter(|entry| !entry.busy)
        {
            for item in &mut entry.items
            {
                if let Some((_, reference)) = renames.iter().find(|(old, _)| old == &item.current_reference)
                {
                    item.current_reference.clone_from(reference);
                    if let Some(proof) = &mut item.proof
                    {
                        proof.path = std::path::PathBuf::from(reference);
                    }
                }
            }
        }
        Ok(())
    }

    /// 撤销或重做一个联合单元；成功文件立即记账，布局阶段失败不会重放文件动作。
    pub fn restore_shell_history(&self, redo: bool) -> Result<Option<ShellOperationResult>, DesktopItemError>
    {
        let expected = if redo { ShellHistoryState::Undone } else { ShellHistoryState::Applied }; // 本轮可以执行的逐项方向。
        let entry = {
            let mut history = self.operation_history.lock().map_err(|_| DesktopItemError::snapshot_unavailable())?;
            if history.iter().any(|entry| entry.busy)
            {
                return Err(DesktopItemError::shell_operation_failed(&"文件与布局历史正在恢复"));
            }
            let Some(entry) = history.iter_mut().filter(|entry| shell_history_has_direction(entry, redo)).max_by_key(|entry| entry.timeline_at_unix_micros)
            else
            {
                return Ok(None);
            };
            entry.busy = true;
            entry.clone()
        }; // 释放锁后等待原生操作，其他 Shell 操作保持响应。
        let actions = if entry.pending_layout.is_empty() { shell_compensation_actions(&entry, redo) } else { Vec::new() }; // 布局保存重试不再次执行任何文件操作。
        let proofs = entry.items.iter().filter(|item| item.state == expected && item.proof.is_some()).filter_map(|item| item.proof.clone()).collect();
        let native = if actions.is_empty()
        {
            let mut result = entry.result.clone(); // 只有布局或待保存布局时不调用空文件操作。
            result.items.clear();
            result.aborted = false;
            result.operation_error = None;
            Ok(result)
        }
        else
        {
            self.runtime.compensate(entry.result.kind, actions, proofs)
        };
        let mut history = self.operation_history.lock().map_err(|_| DesktopItemError::snapshot_unavailable())?;
        let current = history.iter_mut().find(|current| current.result.operation_id == entry.result.operation_id).ok_or_else(DesktopItemError::snapshot_unavailable)?;
        let mut result = match native
        {
            Ok(result) => result,
            Err(error) =>
            {
                current.busy = false; // 没有逐项结果时保留证据，下一次明确请求必须重新验证。
                return Err(DesktopItemError::shell_operation_failed(&error));
            }
        };
        let mut restored_count = 0; // 仅统计有准确结果身份的完成项。
        for item in current.items.iter_mut().filter(|item| item.state == expected)
        {
            let Some(actual) = result.items.iter().find(|actual| actual.source_reference == item.current_reference)
            else
            {
                continue;
            };
            if actual.succeeded
            {
                if let Some(reference) = &actual.result_reference
                {
                    if current.layout.as_ref().is_some_and(|layout| layout.item_ids.contains(&item.result_reference))
                    {
                        current.pending_layout.push((item.result_reference.clone(), item.current_reference.clone(), reference.clone(), redo));
                    }
                    item.current_reference.clone_from(reference);
                    item.proof = actual.proof.clone();
                    item.blocked = actual.compensation_blocked.clone();
                    item.state = if item.proof.is_some()
                    {
                        if redo { ShellHistoryState::Applied } else { ShellHistoryState::Undone }
                    }
                    else
                    {
                        ShellHistoryState::Invalid
                    };
                    restored_count += 1;
                }
                else
                {
                    item.state = ShellHistoryState::Invalid;
                    item.blocked = Some("补偿缺少准确结果身份".into());
                }
            }
            else
            {
                item.proof = actual.proof.clone();
                item.blocked = actual.compensation_blocked.clone();
                if item.proof.is_none()
                {
                    item.state = ShellHistoryState::Invalid;
                }
            }
        }
        for actual in result.items.iter().filter(|actual| actual.succeeded && actual.proof.is_some())
        {
            if entry.items.iter().any(|item| item.current_reference == actual.source_reference)
            {
                continue;
            }
            let Some((root, suffix)) = entry.items.iter().filter(|item| item.state == expected).find_map(|item| {
                let suffix = std::path::Path::new(&actual.source_reference).strip_prefix(&item.proof.as_ref()?.path).ok()?; // 只拆分已证明属于原根的完成后代。
                (!suffix.as_os_str().is_empty()).then_some((item, suffix.to_path_buf()))
            })
            else
            {
                continue;
            };
            if result.items.iter().any(|item| item.source_reference == root.current_reference && item.succeeded)
            {
                continue;
            }
            let Some(reference) = &actual.result_reference
            else
            {
                continue;
            };
            let source = std::path::Path::new(&root.source_reference).join(&suffix);
            let destination = std::path::Path::new(&root.result_reference).join(&suffix);
            if !source.is_absolute() || !destination.is_absolute()
            {
                continue;
            } // Shell 虚拟身份不能用路径拼接推测。
            let mut child = root.clone(); // 部分成功后代仅参与文件补偿，不冒充独立围栏成员。
            child.source_reference = source.to_string_lossy().into_owned();
            child.result_reference = destination.to_string_lossy().into_owned();
            child.source_parent_reference = source.parent().map(|path| path.to_string_lossy().into_owned());
            child.source_name = source.file_name().map(|name| name.to_string_lossy().into_owned());
            child.result_parent_reference = destination.parent().map(|path| path.to_string_lossy().into_owned());
            child.result_name = destination.file_name().map(|name| name.to_string_lossy().into_owned());
            child.current_reference.clone_from(reference);
            child.proof = actual.proof.clone();
            child.blocked = None;
            child.state = if redo { ShellHistoryState::Applied } else { ShellHistoryState::Undone };
            current.items.push(child);
            restored_count += 1;
        }
        current.timeline_at_unix_micros = crate::tools::time::next_timeline_timestamp();
        current.state = ShellHistoryState::Invalid; // 不可补偿记录只提示一次，后续方向由逐项事实决定。
        result.operation_id = entry.result.operation_id;
        result.completed_at_unix_micros = current.timeline_at_unix_micros;
        let blocked = current.items.iter().filter(|item| item.state == ShellHistoryState::Invalid).count(); // 覆盖、合并和不明状态明确反馈。
        let retry = current.items.iter().filter(|item| item.state == expected).count();
        result.message = format!("文件{}完成 {restored_count} 项；{retry} 项未完成，{blocked} 项无法安全补偿", if redo { "重做" } else { "撤销" });
        Ok(Some(result))
    }

    /// 只恢复已完成文件和本轮布局项目，保存失败时保留待落盘动作供显式重试。
    pub(crate) fn restore_drop_history_layout(&self, workspace: &crate::workspace::WorkspaceState, snapshot: &DesktopItemSnapshot, operation_id: u64, redo: bool, cancelled: bool) -> Result<(usize, usize), DesktopItemError>
    {
        let entry = self.operation_history.lock().map_err(|_| DesktopItemError::snapshot_unavailable())?.iter().find(|entry| entry.result.operation_id == operation_id).cloned().ok_or_else(DesktopItemError::snapshot_unavailable)?;
        let Some(layout) = &entry.layout
        else
        {
            return Ok((0, 0));
        };
        let expected = if redo { ShellHistoryState::Undone } else { ShellHistoryState::Applied };
        let mut mappings = entry.pending_layout.iter().filter(|mapping| mapping.3 == redo).cloned().collect::<Vec<_>>(); // 不再次调用已完成文件动作。
        if !cancelled
        {
            mappings.extend(entry.layout_states.iter().filter(|(_, state)| **state == expected).map(|(id, _)| (id.clone(), id.clone(), id.clone(), redo)));
        }
        if mappings.is_empty()
        {
            return Ok((0, 0));
        }
        let restored = workspace.restore_drop_layout(layout, &mappings, snapshot).map_err(|error| DesktopItemError::shell_operation_failed(&error.message))?;
        let mut history = self.operation_history.lock().map_err(|_| DesktopItemError::snapshot_unavailable())?;
        let current = history.iter_mut().find(|entry| entry.result.operation_id == operation_id).ok_or_else(DesktopItemError::snapshot_unavailable)?;
        for (id, _, _, _) in &mappings
        {
            if let Some(state) = current.layout_states.get_mut(id)
            {
                *state = if restored.contains(id)
                {
                    if redo { ShellHistoryState::Applied } else { ShellHistoryState::Undone }
                }
                else
                {
                    ShellHistoryState::Invalid
                };
            }
        }
        current.pending_layout.retain(|mapping| mapping.3 != redo); // 明确歧义不自动重试；磁盘失败在此之前返回，队列保留。
        Ok((restored.len(), mappings.len().saturating_sub(restored.len())))
    }

    /// 无论刷新、保存还是事件发布是否成功，都释放联合历史执行占用。
    pub(crate) fn finish_shell_history(&self, operation_id: u64)
    {
        if let Ok(mut history) = self.operation_history.lock()
            && let Some(entry) = history.iter_mut().find(|entry| entry.result.operation_id == operation_id)
        {
            entry.busy = false;
        }
    }

    /// 为改变 Shell 事实的操作分配标识并加入会话记录。
    fn record_operation(&self, result: ShellOperationResult) -> Result<ShellOperationResult, DesktopItemError> { self.record_operation_with_layout(result, None) }

    /// 文件事实与已持久化归属形成同一条用户历史。
    fn record_operation_with_layout(&self, mut result: ShellOperationResult, layout: Option<crate::workspace::DropLayoutChange>) -> Result<ShellOperationResult, DesktopItemError>
    {
        if (result.aborted && !result.items.iter().any(|item| item.succeeded)) || !matches!(result.kind, ShellOperationKind::Rename | ShellOperationKind::Recycle | ShellOperationKind::PermanentDelete | ShellOperationKind::Paste)
        {
            return Ok(result);
        }
        result.operation_id = self.next_operation_id.fetch_add(1, Ordering::Relaxed);
        result.completed_at_unix_micros = crate::tools::time::next_timeline_timestamp();
        let items = shell_history_items(&result); // 当前操作中可以明确执行补偿的成功项目集合。
        let state = if !items.is_empty() || layout.is_some()
        {
            ShellHistoryState::Applied
        }
        else
        {
            ShellHistoryState::Invalid
        }; // 永久删除和缺少结果引用的记录不进入可执行时间线。
        let mut history = self.operation_history.lock().map_err(|_| DesktopItemError::snapshot_unavailable())?; // 等待提交新 Shell 分支的统一历史。
        for entry in history.iter_mut()
        {
            invalidate_shell_redo(entry);
        }
        let layout_states = layout.as_ref().map(|layout| layout.layout_only_ids.iter().map(|id| (id.clone(), ShellHistoryState::Applied)).collect()).unwrap_or_default();
        history.push(ShellHistoryEntry { busy: false, timeline_at_unix_micros: result.completed_at_unix_micros, result: result.clone(), state, items, layout, layout_states, pending_layout: Vec::new() });
        crate::log_info!("Shell：操作结果已记录，标识={}，类型={:?}，可逆={}，永久={}，取消={}", result.operation_id, result.kind, u8::from(result.reversible), u8::from(result.permanent), u8::from(result.aborted));
        Ok(result)
    }

    /// 在专用 owner HWND 显示当前语义目标的原生菜单。
    pub fn show_menu(&self, target: NativeMenuTarget, screen_x: i32, screen_y: i32, product_commands: Vec<NativeMenuCommand>) -> Result<Option<u32>, DesktopItemError>
    {
        let selected = self.runtime.show_menu(target, screen_x, screen_y, product_commands).map_err(|error| {
            crate::log_error!("Shell：原生上下文菜单失败，错误：{error}");
            DesktopItemError::shell_operation_failed(&error)
        })?;
        crate::log_debug!("Shell：原生上下文菜单已关闭，产品命令={}", selected.map(|command| command.to_string()).unwrap_or_else(|| "无".to_string()));
        Ok(selected)
    }

    /// 将当前选择集合校验并转换为可解析 Shell 稳定引用。
    pub fn selection_references(&self, item_ids: &[String]) -> Result<Vec<String>, DesktopItemError>
    {
        if item_ids.is_empty()
        {
            return Err(DesktopItemError::invalid_selection("桌面项目选择为空"));
        }
        // 当前请求中已经出现的项目引用。
        let mut selected = std::collections::HashSet::new();
        // 当前桌面项目权威快照。
        let snapshot = lock_snapshot(&self.snapshot)?;
        item_ids
            .iter()
            .map(|item_id| {
                if !selected.insert(item_id)
                {
                    return Err(DesktopItemError::invalid_selection("桌面项目选择包含重复项目"));
                }
                let item = snapshot.items.iter().find(|item| &item.stable_reference == item_id).ok_or_else(|| DesktopItemError::invalid_selection("桌面项目不存在"))?;
                if item.availability != DesktopItemAvailability::Available
                {
                    return Err(DesktopItemError::invalid_selection("桌面项目当前不可用"));
                }
                Ok(item.stable_reference.clone())
            })
            .collect()
    }
}

/// 从 Shell 实际结果筛选可以明确补偿的成功项目。
fn shell_history_items(result: &ShellOperationResult) -> Vec<ShellHistoryItem>
{
    result
        .items
        .iter()
        .filter(|item| item.succeeded)
        .filter_map(|item| {
            let current_reference = item.result_reference.clone()?; // 原始操作后下一次补偿使用的稳定引用。
            let actionable = match result.kind
            {
                ShellOperationKind::Rename => item.source_name.is_some() && item.result_name.is_some(),
                ShellOperationKind::Recycle => item.source_parent_reference.is_some() && item.source_name.is_some(),
                ShellOperationKind::Paste if result.effect.as_deref() == Some("复制") => item.result_parent_reference.is_some() && item.result_name.is_some(),
                ShellOperationKind::Paste if result.effect.as_deref() == Some("移动") => item.source_parent_reference.is_some() && item.source_name.is_some() && item.result_parent_reference.is_some() && item.result_name.is_some(),
                _ => false,
            }; // 当前操作类型完成撤销和重做所需的最小元数据集合。
            Some(ShellHistoryItem {
                state: if actionable && item.proof.is_some()
                {
                    ShellHistoryState::Applied
                }
                else
                {
                    ShellHistoryState::Invalid
                },
                proof: item.proof.clone(),
                blocked: item.compensation_blocked.clone().or_else(|| (!actionable || item.proof.is_none()).then(|| "缺少安全补偿依据".into())),
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
fn shell_compensation_actions(entry: &ShellHistoryEntry, redo: bool) -> Vec<ShellCompensationAction>
{
    entry
        .items
        .iter()
        .filter(|item| item.state == if redo { ShellHistoryState::Undone } else { ShellHistoryState::Applied })
        .filter(|item| item.proof.is_some())
        .map(|item| match entry.result.kind
        {
            ShellOperationKind::Rename => ShellCompensationAction::Rename { reference: item.current_reference.clone(), new_name: if redo { item.result_name.clone() } else { item.source_name.clone() }.expect("可逆重命名已保存目标名称") },
            ShellOperationKind::Recycle if redo => ShellCompensationAction::Recycle { reference: item.current_reference.clone() },
            ShellOperationKind::Recycle => ShellCompensationAction::Move { reference: item.current_reference.clone(), destination_parent_reference: item.source_parent_reference.clone().expect("可逆回收已保存来源父级"), new_name: item.source_name.clone().expect("可逆回收已保存来源名称") },
            ShellOperationKind::Paste if entry.result.effect.as_deref() == Some("复制") && !redo => ShellCompensationAction::Recycle { reference: item.current_reference.clone() },
            ShellOperationKind::Paste if entry.result.effect.as_deref() == Some("复制") => ShellCompensationAction::Move { reference: item.current_reference.clone(), destination_parent_reference: item.result_parent_reference.clone().expect("可逆复制已保存结果父级"), new_name: item.result_name.clone().expect("可逆复制已保存结果名称") },
            ShellOperationKind::Paste => ShellCompensationAction::Move { reference: item.current_reference.clone(), destination_parent_reference: if redo { item.result_parent_reference.clone() } else { item.source_parent_reference.clone() }.expect("可逆移动已保存目标父级"), new_name: if redo { item.result_name.clone() } else { item.source_name.clone() }.expect("可逆移动已保存目标名称") },
            _ => unreachable!("只有重命名、回收和粘贴进入 Shell 可逆历史"),
        })
        .collect()
}

/// 同一历史单元可以同时包含待重试项、已撤销项和待保存布局。
fn shell_history_has_direction(entry: &ShellHistoryEntry, redo: bool) -> bool
{
    if !entry.pending_layout.is_empty()
    {
        return entry.pending_layout.iter().any(|mapping| mapping.3 == redo);
    } // 先完成已经执行的方向，不逆转未提交布局。
    let expected = if redo { ShellHistoryState::Undone } else { ShellHistoryState::Applied }; // 当前方向逐项状态。
    entry.items.iter().any(|item| item.state == expected) || entry.layout_states.values().any(|state| *state == expected) || (!redo && entry.state == ShellHistoryState::Applied && !entry.items.is_empty())
}

/// 新分支仅使已撤销部分失效，保留失败后仍可明确重试的应用部分。
fn invalidate_shell_redo(entry: &mut ShellHistoryEntry)
{
    for item in &mut entry.items
    {
        if item.state == ShellHistoryState::Undone
        {
            item.state = ShellHistoryState::Invalid;
        }
    }
    for state in entry.layout_states.values_mut()
    {
        if *state == ShellHistoryState::Undone
        {
            *state = ShellHistoryState::Invalid;
        }
    }
}

/// 获取桌面项目快照锁。
fn lock_snapshot(snapshot: &Mutex<DesktopItemSnapshot>) -> Result<MutexGuard<'_, DesktopItemSnapshot>, DesktopItemError> { snapshot.lock().map_err(|_| DesktopItemError::snapshot_unavailable()) }

/// 按稳定引用协调本次 Shell 观察结果与最近有效元数据。
fn reconcile_desktop_items(mut observed: Vec<DesktopItem>, previous: &[DesktopItem]) -> Vec<DesktopItem>
{
    // 最近一次项目元数据的稳定引用索引。
    let previous_by_reference = previous.iter().map(|item| (item.stable_reference.as_str(), item)).collect::<HashMap<_, _>>();

    for item in &mut observed
    {
        if item.availability == DesktopItemAvailability::Available
        {
            continue;
        }

        if let Some(previous_item) = previous_by_reference.get(item.stable_reference.as_str())
        {
            item.display_name.clone_from(&previous_item.display_name);
            item.system_image.clone_from(&previous_item.system_image);
        }
    }

    observed.sort_by_cached_key(|item| item.display_name.to_lowercase());
    observed
}

/// 提交一次成功的 Shell 观察结果，并在失败时保留当前快照。
fn publish_observation(snapshot: &mut DesktopItemSnapshot, observation: Result<Vec<DesktopItem>, String>) -> Result<DesktopItemSnapshot, DesktopItemError>
{
    // 已成功枚举的完整桌面项目集合。
    let observed = observation.map_err(|error| DesktopItemError::shell_enumeration_failed(&error))?;
    snapshot.items = reconcile_desktop_items(observed, &snapshot.items);
    snapshot.revision += 1;

    Ok(snapshot.clone())
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
