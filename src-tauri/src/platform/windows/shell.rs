use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::null_mut;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use windows::Win32::Foundation::{
    DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, HANDLE, HINSTANCE, HWND,
    LPARAM, LRESULT, S_OK, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, HGDIOBJ, SelectObject,
};
use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
use windows::Win32::System::Com::{
    CLSCTX_ALL, CoCreateInstance, CoTaskMemAlloc, CoTaskMemFree, DVASPECT_CONTENT, FORMATETC,
    IDataObject, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL,
};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::{
    DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, DROPEFFECT_NONE, DoDragDrop,
    IDropSource, IDropSource_Impl, OleGetClipboard, OleInitialize, OleSetClipboard,
    OleUninitialize, ReleaseStgMedium,
};
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    CFSTR_PREFERREDDROPEFFECT, CMF_NORMAL, CMINVOKECOMMANDINFO, CSIDL_DESKTOP, FOF_ALLOWUNDO,
    FOF_WANTNUKEWARNING, FOFX_ADDUNDORECORD, FOFX_RECYCLEONDELETE, FOLDERID_Desktop,
    FOLDERID_PublicDesktop, FileOperation, IContextMenu, IContextMenu2, IContextMenu3,
    IFileOperation, IFileOperationProgressSink, IFileOperationProgressSink_Impl, ILGetSize,
    IShellItem, IShellView, SEE_MASK_IDLIST, SHCNE_ALLEVENTS, SHCNE_RENAMEFOLDER, SHCNE_RENAMEITEM,
    SHCNRF_NewDelivery, SHCNRF_ShellLevel, SHCONTF_FOLDERS, SHCONTF_INCLUDEHIDDEN,
    SHCONTF_NONFOLDERS, SHChangeNotification_Lock, SHChangeNotification_Unlock,
    SHChangeNotifyDeregister, SHChangeNotifyEntry, SHChangeNotifyRegister, SHCreateItemFromIDList,
    SHELLEXECUTEINFOW, SHFILEINFOW, SHGFI_PIDL, SHGFI_SYSICONINDEX, SHGetDesktopFolder,
    SHGetFileInfoW, SHGetImageList, SHGetKnownFolderIDList, SHGetNameFromIDList,
    SHGetSpecialFolderLocation, SHIL_EXTRALARGE, SHParseDisplayName, SIGDN,
    SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEEDITING,
    SVGIO_BACKGROUND, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CREATESTRUCTW, CreatePopupMenu, CreateWindowExW, DI_NORMAL, DefWindowProcW,
    DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW, DrawIconEx, GWLP_USERDATA,
    GetMessageW, GetWindowLongPtrW, HICON, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MSG,
    PostMessageW, PostQuitMessage, RegisterClassW, SW_SHOWNORMAL, SetForegroundWindow,
    SetWindowLongPtrW, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage,
    WINDOW_EX_STYLE, WM_APP, WM_CLOSE, WM_DESTROY, WM_DRAWITEM, WM_INITMENUPOPUP, WM_MEASUREITEM,
    WM_MENUCHAR, WM_NCCREATE, WNDCLASSW, WS_POPUP,
};
use windows::core::{
    BOOL, HRESULT, HSTRING, IUnknown, Interface, PCSTR, PCWSTR, Ref, implement, w,
};

use crate::contracts::{
    DesktopItem, DesktopItemAvailability, ShellOperationItemResult, ShellOperationKind,
    ShellOperationResult,
};

/// 持有由 Shell 或 COM 分配的 PIDL。
struct OwnedPidl(*mut ITEMIDLIST);

impl Drop for OwnedPidl {
    /// 释放 PIDL 使用的 COM 任务内存。
    fn drop(&mut self) {
        unsafe { CoTaskMemFree(Some(self.0.cast())) };
    }
}

/// 收集 IFileOperation 对每个项目回报的真实结果。
#[implement(IFileOperationProgressSink)]
struct FileOperationProgressSink {
    /// 跨 COM 回调与命令返回共享的项目结果集合。
    results: Arc<Mutex<Vec<ShellOperationItemResult>>>,
    /// Pre/Post 回调之间按执行顺序保存的源项目元数据。
    pending_sources: Arc<Mutex<VecDeque<ShellItemMetadata>>>,
}

/// 补偿操作需要的一个 Shell 项目身份、父级和编辑名称。
#[derive(Default)]
struct ShellItemMetadata {
    /// 项目自身的桌面绝对稳定引用。
    reference: String,
    /// 项目父级的桌面绝对稳定引用。
    parent_reference: Option<String>,
    /// 项目在父级中的可编辑名称。
    name: Option<String>,
}

/// 实现 IFileOperationProgressSink 的项目结果采集回调。
#[allow(non_snake_case)]
impl IFileOperationProgressSink_Impl for FileOperationProgressSink_Impl {
    fn StartOperations(&self) -> windows::core::Result<()> {
        Ok(())
    }

    fn FinishOperations(&self, _hrresult: HRESULT) -> windows::core::Result<()> {
        Ok(())
    }

    fn PreRenameItem(
        &self,
        _dwflags: u32,
        psiitem: Ref<'_, IShellItem>,
        _psznewname: &PCWSTR,
    ) -> windows::core::Result<()> {
        self.record_source(psiitem);
        Ok(())
    }

    fn PostRenameItem(
        &self,
        _dwflags: u32,
        psiitem: Ref<'_, IShellItem>,
        _psznewname: &PCWSTR,
        hrrename: HRESULT,
        psinewlycreated: Ref<'_, IShellItem>,
    ) -> windows::core::Result<()> {
        self.record(psiitem, psinewlycreated, hrrename);
        Ok(())
    }

    fn PreMoveItem(
        &self,
        _dwflags: u32,
        psiitem: Ref<'_, IShellItem>,
        _psidestinationfolder: Ref<'_, IShellItem>,
        _psznewname: &PCWSTR,
    ) -> windows::core::Result<()> {
        self.record_source(psiitem);
        Ok(())
    }

    fn PostMoveItem(
        &self,
        _dwflags: u32,
        psiitem: Ref<'_, IShellItem>,
        _psidestinationfolder: Ref<'_, IShellItem>,
        _psznewname: &PCWSTR,
        hrmove: HRESULT,
        psinewlycreated: Ref<'_, IShellItem>,
    ) -> windows::core::Result<()> {
        self.record(psiitem, psinewlycreated, hrmove);
        Ok(())
    }

    fn PreCopyItem(
        &self,
        _dwflags: u32,
        psiitem: Ref<'_, IShellItem>,
        _psidestinationfolder: Ref<'_, IShellItem>,
        _psznewname: &PCWSTR,
    ) -> windows::core::Result<()> {
        self.record_source(psiitem);
        Ok(())
    }

    fn PostCopyItem(
        &self,
        _dwflags: u32,
        psiitem: Ref<'_, IShellItem>,
        _psidestinationfolder: Ref<'_, IShellItem>,
        _psznewname: &PCWSTR,
        hrcopy: HRESULT,
        psinewlycreated: Ref<'_, IShellItem>,
    ) -> windows::core::Result<()> {
        self.record(psiitem, psinewlycreated, hrcopy);
        Ok(())
    }

    fn PreDeleteItem(
        &self,
        _dwflags: u32,
        psiitem: Ref<'_, IShellItem>,
    ) -> windows::core::Result<()> {
        self.record_source(psiitem);
        Ok(())
    }

    fn PostDeleteItem(
        &self,
        _dwflags: u32,
        psiitem: Ref<'_, IShellItem>,
        hrdelete: HRESULT,
        psinewlycreated: Ref<'_, IShellItem>,
    ) -> windows::core::Result<()> {
        self.record(psiitem, psinewlycreated, hrdelete);
        Ok(())
    }

    fn PreNewItem(
        &self,
        _dwflags: u32,
        _psidestinationfolder: Ref<'_, IShellItem>,
        _psznewname: &PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }

    fn PostNewItem(
        &self,
        _dwflags: u32,
        psidestinationfolder: Ref<'_, IShellItem>,
        _psznewname: &PCWSTR,
        _psztemplatename: &PCWSTR,
        _dwfileattributes: u32,
        hrnew: HRESULT,
        psinewitem: Ref<'_, IShellItem>,
    ) -> windows::core::Result<()> {
        self.record(psidestinationfolder, psinewitem, hrnew);
        Ok(())
    }

    fn UpdateProgress(&self, _iworktotal: u32, _iworksofar: u32) -> windows::core::Result<()> {
        Ok(())
    }

    fn ResetTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }

    fn PauseTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }

    fn ResumeTimer(&self) -> windows::core::Result<()> {
        Ok(())
    }
}

impl FileOperationProgressSink_Impl {
    /// 在 IFileOperation 改变项目身份前保存补偿所需元数据。
    fn record_source(&self, source: Ref<'_, IShellItem>) {
        let Some(source) = source.as_ref() else {
            return;
        };
        if let Ok(mut pending) = self.pending_sources.lock() {
            pending.push_back(shell_item_metadata(source));
        }
    }

    /// 保存 Shell 回调提供的源引用、结果引用与 HRESULT。
    fn record(&self, source: Ref<'_, IShellItem>, result: Ref<'_, IShellItem>, status: HRESULT) {
        let source = self
            .pending_sources
            .lock()
            .ok()
            .and_then(|mut pending| pending.pop_front())
            .or_else(|| source.as_ref().map(shell_item_metadata))
            .unwrap_or_default(); // Pre 回调优先提供的操作前源项目元数据。
        let result = result.as_ref().map(shell_item_metadata); // Post 回调提供的操作后结果项目元数据。
        if let Ok(mut results) = self.results.lock() {
            results.push(ShellOperationItemResult {
                source_reference: if source.reference.is_empty() {
                    "未知项目".to_string()
                } else {
                    source.reference
                },
                result_reference: result.as_ref().map(|item| item.reference.clone()),
                succeeded: status.is_ok(),
                result_code: status.0,
                source_parent_reference: source.parent_reference,
                source_name: source.name,
                result_parent_reference: result
                    .as_ref()
                    .and_then(|item| item.parent_reference.clone()),
                result_name: result.and_then(|item| item.name),
            });
        }
    }
}

/// 使用系统默认光标驱动 OLE 拖放生命周期。
#[implement(IDropSource)]
struct ShellDragSource;

/// 实现 OLE 拖动继续、取消和释放规则。
#[allow(non_snake_case)]
impl IDropSource_Impl for ShellDragSource_Impl {
    fn QueryContinueDrag(&self, escape_pressed: BOOL, key_state: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape_pressed.as_bool() {
            return DRAGDROP_S_CANCEL;
        }
        if (key_state & MK_LBUTTON).0 == 0 {
            return DRAGDROP_S_DROP;
        }
        S_OK
    }

    fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

/// Shell 原生菜单显示的语义目标。
pub enum NativeMenuTarget {
    /// 使用当前完整选择集合构建多选 Shell 菜单。
    Items(Vec<String>),
    /// 使用 Shell 桌面视图构建背景菜单。
    Background,
    /// 只显示 Desktop Tools 产品命令。
    ProductOnly,
}

/// 统一撤销时间线提交给新 IFileOperation 的一个补偿动作。
pub enum ShellCompensationAction {
    /// 将当前项目重命名为指定编辑名称。
    Rename {
        /// 当前可解析项目稳定引用。
        reference: String,
        /// 补偿后项目编辑名称。
        new_name: String,
    },
    /// 将当前项目移动回指定父级和名称。
    Move {
        /// 当前可解析项目稳定引用。
        reference: String,
        /// 补偿目标父级稳定引用。
        destination_parent_reference: String,
        /// 补偿后项目编辑名称。
        new_name: String,
    },
    /// 将当前项目移入回收站。
    Recycle {
        /// 当前可解析项目稳定引用。
        reference: String,
    },
}

/// 追加到原生菜单中的一个 Desktop Tools 产品命令。
pub struct NativeMenuCommand {
    /// 返回给命令层的稳定命令编号。
    pub id: u32,
    /// 用户可见的中文命令名称。
    pub label: String,
    /// 完整选择集合是否共同支持该命令。
    pub enabled: bool,
}

/// 当前 Shell 菜单需要转发消息的接口版本。
enum MenuMessageHandler {
    /// 支持返回菜单字符结果的 IContextMenu3。
    ContextMenu3(IContextMenu3),
    /// 支持 owner-drawn 与动态子菜单消息的 IContextMenu2。
    ContextMenu2(IContextMenu2),
}

/// 专用菜单 owner HWND 持有的当前 Shell 菜单接口。
#[derive(Default)]
struct MenuOwnerState {
    /// TrackPopupMenu 期间接收窗口消息的 Shell 菜单接口。
    handler: Option<MenuMessageHandler>,
}

/// 承载 Shell 菜单、owner-drawn 项和原生确认界面的专用 HWND。
pub struct ShellMenuOwner {
    /// 专用原生 owner 窗口句柄。
    hwnd: HWND,
    /// 窗口过程通过 GWLP_USERDATA 访问的稳定状态地址。
    state: Box<MenuOwnerState>,
}

/// 将 Shell 菜单消息转发到当前 IContextMenu3/2。
unsafe extern "system" fn shell_menu_owner_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        // CreateWindowExW 提交的专用菜单状态。
        let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize) };
    }
    // 当前 owner 窗口关联的专用菜单状态。
    let state = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut MenuOwnerState;
    if !state.is_null()
        && matches!(
            message,
            WM_DRAWITEM | WM_MEASUREITEM | WM_INITMENUPOPUP | WM_MENUCHAR
        )
    {
        // Shell 菜单处理后返回给窗口消息循环的结果。
        let mut result = LRESULT(0);
        // TrackPopupMenu 期间设置的当前 Shell 菜单接口。
        let handler = unsafe { &mut *state }.handler.as_ref();
        match handler {
            Some(MenuMessageHandler::ContextMenu3(menu)) => {
                if unsafe { menu.HandleMenuMsg2(message, wparam, lparam, Some(&mut result)) }
                    .is_ok()
                {
                    return result;
                }
            }
            Some(MenuMessageHandler::ContextMenu2(menu))
                if unsafe { menu.HandleMenuMsg(message, wparam, lparam) }.is_ok() =>
            {
                return LRESULT(0);
            }
            Some(MenuMessageHandler::ContextMenu2(_)) => {}
            None => {}
        }
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

impl ShellMenuOwner {
    /// 在 Shell STA 线程创建专用隐藏 owner HWND。
    pub fn create() -> Result<Self, String> {
        // 当前进程模块句柄。
        let module = unsafe { GetModuleHandleW(None) }
            .map_err(|error| format!("获取菜单 owner 模块失败: {error}"))?;
        // 专用 owner 窗口类。
        let class = WNDCLASSW {
            lpfnWndProc: Some(shell_menu_owner_proc),
            hInstance: HINSTANCE(module.0),
            lpszClassName: w!("DesktopToolsShellMenuOwner"),
            ..Default::default()
        };
        if unsafe { RegisterClassW(&class) } == 0 {
            return Err("注册 Shell 菜单 owner 窗口类失败".to_string());
        }

        // 窗口过程使用的稳定菜单状态。
        let mut state = Box::<MenuOwnerState>::default();
        // 专用隐藏 owner HWND。
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("DesktopToolsShellMenuOwner"),
                w!(""),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(HINSTANCE(module.0)),
                Some((&mut *state as *mut MenuOwnerState).cast()),
            )
        }
        .map_err(|error| format!("创建 Shell 菜单 owner HWND 失败: {error}"))?;
        Ok(Self { hwnd, state })
    }

    /// 设置 TrackPopupMenu 期间接收消息的 Shell 菜单接口。
    fn set_context_menu(&mut self, context_menu: &IContextMenu) {
        self.state.handler = context_menu
            .cast::<IContextMenu3>()
            .map(MenuMessageHandler::ContextMenu3)
            .or_else(|_| {
                context_menu
                    .cast::<IContextMenu2>()
                    .map(MenuMessageHandler::ContextMenu2)
            })
            .ok();
    }

    /// 清除已经关闭的 Shell 菜单接口。
    fn clear_context_menu(&mut self) {
        self.state.handler = None;
    }
}

impl Drop for ShellMenuOwner {
    /// 销毁专用菜单 owner HWND。
    fn drop(&mut self) {
        let _ = unsafe { DestroyWindow(self.hwnd) };
    }
}

/// Shell 桌面变化通知携带的可选精确重命名事实。
#[derive(Clone, Debug)]
pub enum ShellChangeEvent {
    /// 需要重新枚举桌面命名空间的普通变化。
    Invalidated,
    /// Shell 通知提供的旧、新稳定引用。
    Renamed {
        old_reference: String,
        new_reference: String,
    },
}

/// 持有 Shell 变化通知窗口线程。
pub struct ShellChangeWatcher {
    /// 接收 SHChangeNotify 消息的隐藏窗口句柄。
    hwnd: isize,
    /// 持续泵送窗口消息的后台线程。
    worker: Option<JoinHandle<()>>,
}

/// Shell 变化通知使用的窗口消息编号。
const SHELL_CHANGE_MESSAGE: u32 = WM_APP + 0x51;

/// 将 SHChangeNotify 消息转换为失效或精确重命名事件。
unsafe extern "system" fn shell_change_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        // CreateWindowExW 提交的变化事件发送端。
        let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize) };
    }
    if message == SHELL_CHANGE_MESSAGE {
        // 当前变化通知窗口关联的事件发送端。
        let sender =
            unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const Sender<ShellChangeEvent>;
        // Shell 通知锁定后返回的两个绝对 PIDL。
        let mut pidls: *mut *mut ITEMIDLIST = null_mut();
        // Shell 通知事件掩码。
        let mut event = 0;
        // 当前 NewDelivery 通知的共享内存锁。
        let lock = unsafe {
            SHChangeNotification_Lock(
                HANDLE(wparam.0 as *mut c_void),
                lparam.0 as u32,
                Some(&mut pidls),
                Some(&mut event),
            )
        };
        if !lock.is_invalid() && !sender.is_null() {
            // 当前通知能够提供的精确变化事实。
            let change = if (event as u32 & (SHCNE_RENAMEITEM.0 | SHCNE_RENAMEFOLDER.0)) != 0
                && !pidls.is_null()
            {
                // 重命名前的绝对项目 PIDL。
                let old_pidl = unsafe { *pidls };
                // 重命名后的绝对项目 PIDL。
                let new_pidl = unsafe { *pidls.add(1) };
                match (
                    shell_name(old_pidl, SIGDN_DESKTOPABSOLUTEPARSING),
                    shell_name(new_pidl, SIGDN_DESKTOPABSOLUTEPARSING),
                ) {
                    (Ok(old_reference), Ok(new_reference)) => ShellChangeEvent::Renamed {
                        old_reference,
                        new_reference,
                    },
                    _ => ShellChangeEvent::Invalidated,
                }
            } else {
                ShellChangeEvent::Invalidated
            };
            let _ = unsafe { &*sender }.send(change);
            let _ = unsafe { SHChangeNotification_Unlock(lock) };
        }
        return LRESULT(0);
    }
    if message == WM_CLOSE {
        let _ = unsafe { DestroyWindow(hwnd) };
        return LRESULT(0);
    }
    if message == WM_DESTROY {
        unsafe { PostQuitMessage(0) };
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

/// 启动桌面命名空间 SHChangeNotify 监听线程。
pub fn start_shell_change_watcher()
-> Result<(ShellChangeWatcher, Receiver<ShellChangeEvent>), String> {
    // 变化事件发送到应用协调线程的通道。
    let (events, receiver) = mpsc::channel();
    // 变化通知窗口启动结果通道。
    let (ready, started) = mpsc::sync_channel(0);
    // 持续泵送 SHChangeNotify 消息的 STA 线程。
    let worker = thread::Builder::new()
        .name("desktop-tools-shell-change".to_string())
        .spawn(move || {
            if let Err(error) = initialize_shell_apartment() {
                let _ = ready.send(Err(error));
                return;
            }
            // 当前进程模块句柄。
            let module = match unsafe { GetModuleHandleW(None) } {
                Ok(module) => module,
                Err(error) => {
                    uninitialize_shell_apartment();
                    let _ = ready.send(Err(format!("获取 Shell 变化窗口模块失败: {error}")));
                    return;
                }
            };
            // Shell 变化通知窗口类。
            let class = WNDCLASSW {
                lpfnWndProc: Some(shell_change_window_proc),
                hInstance: HINSTANCE(module.0),
                lpszClassName: w!("DesktopToolsShellChangeWatcher"),
                ..Default::default()
            };
            if unsafe { RegisterClassW(&class) } == 0 {
                uninitialize_shell_apartment();
                let _ = ready.send(Err("注册 Shell 变化窗口类失败".to_string()));
                return;
            }
            // 当前窗口过程生命周期内保持稳定的事件发送端。
            let events = Box::new(events);
            // 接收桌面变化消息的隐藏窗口。
            let hwnd = match unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("DesktopToolsShellChangeWatcher"),
                    w!(""),
                    WS_POPUP,
                    0,
                    0,
                    0,
                    0,
                    None,
                    None,
                    Some(HINSTANCE(module.0)),
                    Some((&*events as *const Sender<ShellChangeEvent>).cast()),
                )
            } {
                Ok(hwnd) => hwnd,
                Err(error) => {
                    uninitialize_shell_apartment();
                    let _ = ready.send(Err(format!("创建 Shell 变化窗口失败: {error}")));
                    return;
                }
            };
            // 桌面命名空间、用户桌面物理目录和公共桌面物理目录 PIDL。
            let roots = match (
                unsafe { SHGetSpecialFolderLocation(None, CSIDL_DESKTOP as i32) },
                unsafe { SHGetKnownFolderIDList(&FOLDERID_Desktop, 0, None) },
                unsafe { SHGetKnownFolderIDList(&FOLDERID_PublicDesktop, 0, None) },
            ) {
                (Ok(namespace), Ok(user), Ok(public)) => vec![
                    OwnedPidl(namespace),
                    OwnedPidl(user),
                    OwnedPidl(public),
                ],
                (namespace, user, public) => {
                    let _ = unsafe { DestroyWindow(hwnd) };
                    uninitialize_shell_apartment();
                    let _ = ready.send(Err(format!(
                        "读取桌面变化根 PIDL 失败，命名空间={namespace:?}，用户桌面={user:?}，公共桌面={public:?}"
                    )));
                    return;
                }
            };
            let mut registrations = Vec::new(); // 三个根分别使用 cEntries=1 的 Shell 变化注册标识。
            for root in &roots {
                let entry = SHChangeNotifyEntry {
                    pidl: root.0,
                    fRecursive: false.into(),
                }; // 当前根直接子项的变化注册项。
                let registration = unsafe {
                    SHChangeNotifyRegister(
                        hwnd,
                        SHCNRF_ShellLevel | SHCNRF_NewDelivery,
                        SHCNE_ALLEVENTS.0 as i32,
                        SHELL_CHANGE_MESSAGE,
                        1,
                        &entry,
                    )
                }; // 当前根独立的 Shell 变化注册标识。
                if registration == 0 {
                    for registration in registrations {
                        let _ = unsafe { SHChangeNotifyDeregister(registration) };
                    }
                    let _ = unsafe { DestroyWindow(hwnd) };
                    uninitialize_shell_apartment();
                    let _ = ready.send(Err("注册桌面 Shell 变化通知失败".to_string()));
                    return;
                }
                registrations.push(registration);
            }
            let _ = ready.send(Ok(hwnd.0 as isize));

            // Shell 变化线程持续运行的 Windows 消息。
            let mut message = MSG::default();
            while unsafe { GetMessageW(&mut message, None, 0, 0) }.as_bool() {
                unsafe {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            for registration in registrations {
                let _ = unsafe { SHChangeNotifyDeregister(registration) };
            }
            drop(roots);
            drop(events);
            uninitialize_shell_apartment();
        })
        .map_err(|error| format!("启动 Shell 变化通知线程失败: {error}"))?;
    // 已创建的变化通知窗口句柄。
    let hwnd = started
        .recv()
        .map_err(|error| format!("等待 Shell 变化通知启动失败: {error}"))??;
    Ok((
        ShellChangeWatcher {
            hwnd,
            worker: Some(worker),
        },
        receiver,
    ))
}

impl Drop for ShellChangeWatcher {
    /// 请求变化通知窗口退出并等待线程释放注册。
    fn drop(&mut self) {
        let _ = unsafe {
            PostMessageW(
                Some(HWND(self.hwnd as *mut c_void)),
                WM_CLOSE,
                WPARAM(0),
                LPARAM(0),
            )
        };
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// 使用 Shell 默认动词打开一个或多个桌面项目。
pub fn open_desktop_items(references: &[String]) -> Result<(), String> {
    if references.is_empty() {
        return Err("桌面项目选择为空".to_string());
    }
    for reference in references {
        // 当前稳定引用重新解析得到的绝对 PIDL。
        let pidl = parse_desktop_item_reference(reference)?;
        // 使用 PIDL 调用 Shell 默认打开动词的参数。
        let mut invocation = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_IDLIST,
            lpVerb: w!("open"),
            nShow: SW_SHOWNORMAL.0,
            lpIDList: pidl.0.cast(),
            ..Default::default()
        };
        unsafe { ShellExecuteExW(&mut invocation) }
            .map_err(|error| format!("Shell 默认打开失败，项目={reference}，错误={error}"))?;
    }
    Ok(())
}

/// 使用 IFileOperation 重命名单个桌面项目并返回 Shell 实际结果。
pub fn rename_desktop_item(
    owner: &ShellMenuOwner,
    reference: &str,
    new_name: &str,
) -> Result<ShellOperationResult, String> {
    let new_name = new_name.trim(); // 提交给 Shell 的用户可见新名称。
    if new_name.is_empty() || new_name.contains('\0') {
        return Err("桌面项目新名称无效".to_string());
    }
    let item = resolve_shell_item(reference)?; // 等待重命名的真实 Shell 项目。
    let operation = create_file_operation(owner, FOF_ALLOWUNDO | FOFX_ADDUNDORECORD)?; // 写入系统撤销记录的原生文件操作。
    let (sink, results) = create_operation_sink(); // 捕获实际重命名结果的进度回调。
    let shell_name = HSTRING::from(new_name); // IFileOperation 接收的 UTF-16 新名称。
    unsafe { operation.RenameItem(&item, &shell_name, &sink) }
        .map_err(|error| format!("提交 Shell 重命名失败: {error}"))?;
    perform_file_operation(
        &operation,
        ShellOperationKind::Rename,
        true,
        false,
        None,
        results,
    )
}

/// 使用 IFileOperation 将选择集合移入回收站或永久删除。
pub fn delete_desktop_items(
    owner: &ShellMenuOwner,
    references: &[String],
    permanent: bool,
) -> Result<ShellOperationResult, String> {
    validate_reference_selection(references)?;
    // 回收站删除写入撤销记录，永久删除保留系统原生警告。
    let flags = if permanent {
        FOF_WANTNUKEWARNING
    } else {
        FOF_ALLOWUNDO | FOFX_ADDUNDORECORD | FOFX_RECYCLEONDELETE
    };
    let operation = create_file_operation(owner, flags)?; // 承载完整删除选择集合的原生操作。
    let (sink, results) = create_operation_sink(); // 捕获每个项目实际删除状态的进度回调。
    for reference in references {
        let item = resolve_shell_item(reference)?; // 当前等待删除的真实 Shell 项目。
        unsafe { operation.DeleteItem(&item, &sink) }
            .map_err(|error| format!("提交 Shell 删除失败，项目={reference}，错误={error}"))?;
    }
    perform_file_operation(
        &operation,
        if permanent {
            ShellOperationKind::PermanentDelete
        } else {
            ShellOperationKind::Recycle
        },
        !permanent,
        permanent,
        None,
        results,
    )
}

/// 使用新的 IFileOperation 执行统一时间线的重命名、移动或回收补偿。
pub fn compensate_desktop_items(
    owner: &ShellMenuOwner,
    kind: ShellOperationKind,
    actions: Vec<ShellCompensationAction>,
) -> Result<ShellOperationResult, String> {
    if actions.is_empty() {
        return Err("Shell 补偿项目为空".to_string());
    }
    let recycle_only = actions
        .iter()
        .all(|action| matches!(action, ShellCompensationAction::Recycle { .. })); // 全部动作是否需要回收站语义。
    let flags = if recycle_only {
        FOF_ALLOWUNDO | FOFX_ADDUNDORECORD | FOFX_RECYCLEONDELETE
    } else {
        FOF_ALLOWUNDO | FOFX_ADDUNDORECORD
    }; // 每次补偿操作独立写入 Explorer 会话撤销记录。
    let operation = create_file_operation(owner, flags)?; // 当前统一时间线补偿使用的原生操作。
    let (sink, results) = create_operation_sink(); // 捕获补偿操作的每项实际结果。
    for action in actions {
        match action {
            ShellCompensationAction::Rename {
                reference,
                new_name,
            } => {
                let item = resolve_shell_item(&reference)?; // 当前等待补偿重命名的项目。
                let new_name = HSTRING::from(new_name); // IFileOperation 接收的 UTF-16 编辑名称。
                unsafe { operation.RenameItem(&item, &new_name, &sink) }
                    .map_err(|error| format!("提交 Shell 补偿重命名失败: {error}"))?;
            }
            ShellCompensationAction::Move {
                reference,
                destination_parent_reference,
                new_name,
            } => {
                let item = resolve_shell_item(&reference)?; // 当前等待补偿移动的项目。
                let destination = resolve_shell_item(&destination_parent_reference)?; // 补偿移动目标父级。
                let new_name = HSTRING::from(new_name); // 补偿后恢复的编辑名称。
                unsafe { operation.MoveItem(&item, &destination, &new_name, &sink) }
                    .map_err(|error| format!("提交 Shell 补偿移动失败: {error}"))?;
            }
            ShellCompensationAction::Recycle { reference } => {
                let item = resolve_shell_item(&reference)?; // 当前等待补偿回收的项目。
                unsafe { operation.DeleteItem(&item, &sink) }
                    .map_err(|error| format!("提交 Shell 补偿回收失败: {error}"))?;
            }
        }
    }
    perform_file_operation(&operation, kind, true, false, None, results)
}

/// 将完整 Shell IDataObject 放入系统剪贴板并保留复制或剪切效果。
pub fn set_desktop_item_clipboard(
    owner: &ShellMenuOwner,
    references: &[String],
    cut: bool,
) -> Result<ShellOperationResult, String> {
    validate_reference_selection(references)?;
    let data_object = desktop_data_object(owner.hwnd, references)?; // Shell handler 提供的完整多格式数据对象。
    let effect = if cut {
        DROPEFFECT_MOVE
    } else {
        DROPEFFECT_COPY
    }; // 系统剪贴板使用的首选投放效果。
    set_preferred_drop_effect(&data_object, effect)?;
    unsafe { OleSetClipboard(&data_object) }
        .map_err(|error| format!("写入系统 Shell 剪贴板失败: {error}"))?;
    // 已成功包含在系统数据对象中的选择项目结果。
    let items = references
        .iter()
        .map(|reference| ShellOperationItemResult {
            source_reference: reference.clone(),
            result_reference: Some(reference.clone()),
            succeeded: true,
            result_code: S_OK.0,
            source_parent_reference: None,
            source_name: None,
            result_parent_reference: None,
            result_name: None,
        })
        .collect::<Vec<_>>();
    let kind = if cut {
        ShellOperationKind::Cut
    } else {
        ShellOperationKind::Copy
    }; // 当前系统剪贴板操作类型。
    Ok(ShellOperationResult {
        operation_id: 0,
        completed_at_unix_micros: 0,
        kind,
        aborted: false,
        reversible: false,
        permanent: false,
        effect: Some(drop_effect_name(effect).to_string()),
        message: format!(
            "已{} {} 个项目到系统剪贴板",
            if cut { "剪切" } else { "复制" },
            references.len()
        ),
        items,
    })
}

/// 使用系统剪贴板 IDataObject 和 IFileOperation 粘贴到桌面命名空间。
pub fn paste_desktop_items(owner: &ShellMenuOwner) -> Result<ShellOperationResult, String> {
    let data_object = unsafe { OleGetClipboard() }
        .map_err(|error| format!("读取系统 Shell 剪贴板失败: {error}"))?; // 当前系统剪贴板完整数据对象。
    let preferred_effect = read_preferred_drop_effect(&data_object).unwrap_or(DROPEFFECT_COPY); // 剪切或复制来源声明的首选效果。
    let effect = if (preferred_effect & DROPEFFECT_MOVE).0 != 0 {
        DROPEFFECT_MOVE
    } else {
        DROPEFFECT_COPY
    }; // 本次粘贴交给 IFileOperation 的真实动作。
    let desktop_pidl = unsafe { SHGetSpecialFolderLocation(None, CSIDL_DESKTOP as i32) }
        .map(OwnedPidl)
        .map_err(|error| format!("读取桌面 Shell 目标失败: {error}"))?; // 桌面命名空间绝对 PIDL。
    let destination: IShellItem = unsafe { SHCreateItemFromIDList(desktop_pidl.0) }
        .map_err(|error| format!("创建桌面 Shell 目标失败: {error}"))?; // IFileOperation 使用的桌面目标项目。
    let operation = create_file_operation(owner, FOF_ALLOWUNDO | FOFX_ADDUNDORECORD)?; // 支持系统撤销的粘贴操作。
    let (sink, results) = create_operation_sink(); // 收集粘贴产生的复制或移动实际结果。
    let cookie = unsafe { operation.Advise(&sink) }
        .map_err(|error| format!("监听 Shell 粘贴结果失败: {error}"))?; // 全局粘贴回调注册标识。
    let objects: IUnknown = data_object
        .cast()
        .map_err(|error| format!("转换 Shell 剪贴板对象失败: {error}"))?; // IFileOperation 批量接口接收的数据对象。
    // 根据剪贴板首选效果提交移动或复制。
    let queued = if effect == DROPEFFECT_MOVE {
        unsafe { operation.MoveItems(&objects, &destination) }
    } else {
        unsafe { operation.CopyItems(&objects, &destination) }
    };
    if let Err(error) = queued {
        let _ = unsafe { operation.Unadvise(cookie) };
        return Err(format!("提交 Shell 粘贴失败: {error}"));
    }
    let result = perform_file_operation(
        &operation,
        ShellOperationKind::Paste,
        true,
        false,
        Some(effect),
        results,
    ); // 完成后返回的粘贴实际结果。
    let _ = unsafe { operation.Unadvise(cookie) };
    result
}

/// 使用 Shell IDataObject 启动跨 Desktop Tools、Explorer 和其他程序的 OLE 拖放。
pub fn start_desktop_item_drag(
    owner: &ShellMenuOwner,
    references: &[String],
) -> Result<ShellOperationResult, String> {
    validate_reference_selection(references)?;
    let data_object = desktop_data_object(owner.hwnd, references)?; // 保留 Shell handler 全部格式的拖动数据对象。
    let drop_source: IDropSource = ShellDragSource.into(); // 使用系统默认光标的 OLE 拖动源。
    let allowed = DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK; // 外部目标可以协商的完整系统效果集合。
    let mut effect = DROPEFFECT_NONE; // DoDragDrop 返回的最终系统效果。
    let status = unsafe { DoDragDrop(&data_object, &drop_source, allowed, &mut effect) }; // 持续到系统完成、取消或拒绝的拖放结果。
    if status.is_err() {
        return Err(format!("系统 OLE 拖放失败: {status:?}"));
    }
    let aborted = status == DRAGDROP_S_CANCEL || effect == DROPEFFECT_NONE; // 用户取消或目标拒绝的拖动结果。
    Ok(ShellOperationResult {
        operation_id: 0,
        completed_at_unix_micros: 0,
        kind: ShellOperationKind::Drag,
        aborted,
        reversible: false,
        permanent: false,
        effect: Some(drop_effect_name(effect).to_string()),
        items: references
            .iter()
            .map(|reference| ShellOperationItemResult {
                source_reference: reference.clone(),
                result_reference: None,
                succeeded: !aborted,
                result_code: status.0,
                source_parent_reference: None,
                source_name: None,
                result_parent_reference: None,
                result_name: None,
            })
            .collect(),
        message: if aborted {
            "系统拖放已取消".to_string()
        } else {
            format!("系统拖放已完成，效果：{}", drop_effect_name(effect))
        },
    })
}

/// 显示 Shell 原生上下文菜单并返回选中的产品命令编号。
pub fn show_native_context_menu(
    owner: &mut ShellMenuOwner,
    target: NativeMenuTarget,
    screen_x: i32,
    screen_y: i32,
    product_commands: &[NativeMenuCommand],
) -> Result<Option<u32>, String> {
    /// Shell 菜单命令起始编号。
    const SHELL_COMMAND_FIRST: u32 = 1;
    /// Shell 菜单命令结束编号。
    const SHELL_COMMAND_LAST: u32 = 0x6fff;

    // Windows Shell 桌面根目录接口。
    let desktop = unsafe { SHGetDesktopFolder() }
        .map_err(|error| format!("获取 Shell 桌面命名空间失败: {error}"))?;
    // 保持项目子 PIDL 生命周期覆盖菜单显示的集合。
    let item_pidls = match &target {
        NativeMenuTarget::Items(references) => resolve_desktop_child_pidls(references)?,
        NativeMenuTarget::Background | NativeMenuTarget::ProductOnly => Vec::new(),
    };
    // 当前语义目标对应的可选 Shell 上下文菜单。
    let context_menu: Option<IContextMenu> = match target {
        NativeMenuTarget::Items(_) => {
            // 传给桌面根目录的相对项目 PIDL 集合。
            let pidl_pointers = item_pidls
                .iter()
                .map(|pidl| pidl.0.cast_const())
                .collect::<Vec<_>>();
            Some(
                unsafe { desktop.GetUIObjectOf(owner.hwnd, &pidl_pointers, None) }
                    .map_err(|error| format!("创建多选 Shell 项目菜单失败: {error}"))?,
            )
        }
        NativeMenuTarget::Background => {
            let view: IShellView = unsafe { desktop.CreateViewObject(owner.hwnd) }
                .map_err(|error| format!("创建 Shell 桌面视图失败: {error}"))?;
            Some(
                unsafe { view.GetItemObject(SVGIO_BACKGROUND) }
                    .map_err(|error| format!("获取 Shell 桌面背景菜单失败: {error}"))?,
            )
        }
        NativeMenuTarget::ProductOnly => None,
    };
    // 承载 Shell 与 Desktop Tools 命令的原生菜单。
    let menu =
        unsafe { CreatePopupMenu() }.map_err(|error| format!("创建原生上下文菜单失败: {error}"))?;
    if let Some(context_menu) = &context_menu {
        unsafe {
            context_menu
                .QueryContextMenu(menu, 0, SHELL_COMMAND_FIRST, SHELL_COMMAND_LAST, CMF_NORMAL)
                .ok()
        }
        .map_err(|error| format!("填充 Shell 上下文菜单失败: {error}"))?;
        owner.set_context_menu(context_menu);
    }

    if !product_commands.is_empty() {
        // Desktop Tools 产品命令子菜单。
        let product_menu = unsafe { CreatePopupMenu() }
            .map_err(|error| format!("创建 Desktop Tools 菜单失败: {error}"))?;
        for command in product_commands {
            // 当前产品命令的 UTF-16 用户可见名称。
            let label = HSTRING::from(&command.label);
            // 当前完整选择集合对应的菜单启用状态。
            let flags = if command.enabled {
                MF_STRING
            } else {
                MF_STRING | MF_GRAYED
            };
            unsafe {
                AppendMenuW(
                    product_menu,
                    flags,
                    command.id as usize,
                    PCWSTR(label.as_ptr()),
                )
            }
            .map_err(|error| format!("添加 Desktop Tools 菜单项失败: {error}"))?;
        }
        if context_menu.is_some() {
            unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()) }
                .map_err(|error| format!("添加菜单分隔线失败: {error}"))?;
        }
        unsafe { AppendMenuW(menu, MF_POPUP, product_menu.0 as usize, w!("Desktop Tools")) }
            .map_err(|error| format!("添加 Desktop Tools 子菜单失败: {error}"))?;
    }

    let _ = unsafe { SetForegroundWindow(owner.hwnd) };
    // 用户在原生菜单中选择的命令编号。
    let selected = unsafe {
        TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            screen_x,
            screen_y,
            None,
            owner.hwnd,
            None,
        )
        .0 as u32
    };
    owner.clear_context_menu();
    let _ = unsafe { DestroyMenu(menu) };

    if (SHELL_COMMAND_FIRST..=SHELL_COMMAND_LAST).contains(&selected) {
        // IContextMenu 要求的零基命令偏移指针。
        let verb = (selected - SHELL_COMMAND_FIRST) as usize as *const u8;
        // Shell 菜单命令调用参数。
        let invocation = CMINVOKECOMMANDINFO {
            cbSize: size_of::<CMINVOKECOMMANDINFO>() as u32,
            hwnd: owner.hwnd,
            lpVerb: PCSTR(verb),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        unsafe {
            context_menu
                .as_ref()
                .expect("Shell 命令只来自 Shell 上下文菜单")
                .InvokeCommand(&invocation)
        }
        .map_err(|error| format!("执行 Shell 菜单命令失败: {error}"))?;
        return Ok(None);
    }
    Ok(product_commands
        .iter()
        .find(|command| command.enabled && command.id == selected)
        .map(|command| command.id))
}

/// 从桌面根目录重新解析多选菜单使用的相对子 PIDL。
fn resolve_desktop_child_pidls(references: &[String]) -> Result<Vec<OwnedPidl>, String> {
    if references.is_empty() {
        return Err("桌面项目菜单选择为空".to_string());
    }
    // 当前请求中需要解析的稳定引用集合。
    let requested = references.iter().cloned().collect::<HashSet<_>>();
    if requested.len() != references.len() {
        return Err("桌面项目菜单选择包含重复项目".to_string());
    }
    // Windows Shell 桌面根目录接口。
    let desktop = unsafe { SHGetDesktopFolder() }
        .map_err(|error| format!("获取 Shell 桌面命名空间失败: {error}"))?;
    // Shell 桌面项目枚举器。
    let mut enumerator = None;
    // 同时包含文件夹、普通项目和隐藏项目的枚举标志。
    let flags = (SHCONTF_FOLDERS.0 | SHCONTF_NONFOLDERS.0 | SHCONTF_INCLUDEHIDDEN.0) as u32;
    unsafe { desktop.EnumObjects(HWND::default(), flags, &mut enumerator) }
        .ok()
        .map_err(|error| format!("创建 Shell 桌面项目枚举器失败: {error}"))?;
    // 已创建的 Shell 桌面项目枚举器。
    let enumerator = enumerator.ok_or_else(|| "Shell 桌面项目枚举器为空".to_string())?;
    // 已按稳定引用解析的相对子 PIDL。
    let mut resolved = HashMap::new();
    loop {
        // Shell 为当前项目分配的相对子 PIDL。
        let mut pidl: *mut ITEMIDLIST = null_mut();
        // 当前调用实际返回的项目数量。
        let mut fetched = 0;
        unsafe { enumerator.Next(std::slice::from_mut(&mut pidl), Some(&mut fetched)) }
            .ok()
            .map_err(|error| format!("读取 Shell 桌面项目失败: {error}"))?;
        if fetched == 0 {
            break;
        }
        if pidl.is_null() {
            continue;
        }
        // 当前相对子 PIDL 对应的稳定引用。
        let reference = shell_name(pidl, SIGDN_DESKTOPABSOLUTEPARSING)
            .unwrap_or_else(|_| encoded_pidl_reference(pidl));
        if requested.contains(&reference) {
            resolved.insert(reference, OwnedPidl(pidl));
        } else {
            unsafe { CoTaskMemFree(Some(pidl.cast())) };
        }
    }
    references
        .iter()
        .map(|reference| {
            resolved
                .remove(reference)
                .ok_or_else(|| format!("Shell 桌面项目已无法解析: {reference}"))
        })
        .collect()
}

/// 校验 Shell 项目选择非空且不包含重复引用。
fn validate_reference_selection(references: &[String]) -> Result<(), String> {
    if references.is_empty() {
        return Err("桌面项目选择为空".to_string());
    }
    let unique = references.iter().collect::<HashSet<_>>(); // 当前请求中唯一的稳定引用集合。
    if unique.len() != references.len() {
        return Err("桌面项目选择包含重复项目".to_string());
    }
    Ok(())
}

/// 从桌面根目录创建保留全部 Shell 格式的多选 IDataObject。
fn desktop_data_object(hwnd: HWND, references: &[String]) -> Result<IDataObject, String> {
    let item_pidls = resolve_desktop_child_pidls(references)?; // 保持到 IDataObject 创建完成的相对子 PIDL 集合。
    let pidl_pointers = item_pidls
        .iter()
        .map(|pidl| pidl.0.cast_const())
        .collect::<Vec<_>>(); // 传给桌面 Shell folder 的相对子 PIDL 指针。
    let desktop = unsafe { SHGetDesktopFolder() }
        .map_err(|error| format!("获取 Shell 桌面命名空间失败: {error}"))?; // IDataObject 所属的桌面根接口。
    unsafe { desktop.GetUIObjectOf(hwnd, &pidl_pointers, None) }
        .map_err(|error| format!("创建 Shell 多选数据对象失败: {error}"))
}

/// 将稳定引用解析为 IFileOperation 使用的 IShellItem。
fn resolve_shell_item(reference: &str) -> Result<IShellItem, String> {
    let pidl = parse_desktop_item_reference(reference)?; // 当前稳定引用重新解析的绝对 PIDL。
    unsafe { SHCreateItemFromIDList(pidl.0) }
        .map_err(|error| format!("创建 Shell 项目失败，项目={reference}，错误={error}"))
}

/// 创建带 owner HWND 和指定系统行为标志的 IFileOperation。
fn create_file_operation(
    owner: &ShellMenuOwner,
    flags: windows::Win32::UI::Shell::FILEOPERATION_FLAGS,
) -> Result<IFileOperation, String> {
    let operation: IFileOperation =
        unsafe { CoCreateInstance(&FileOperation, None, CLSCTX_ALL) }
            .map_err(|error| format!("创建 IFileOperation 失败: {error}"))?; // 当前用户操作独占的系统文件操作对象。
    unsafe { operation.SetOwnerWindow(owner.hwnd) }
        .map_err(|error| format!("设置 Shell 操作 owner 失败: {error}"))?;
    unsafe { operation.SetOperationFlags(flags) }
        .map_err(|error| format!("设置 Shell 操作属性失败: {error}"))?;
    Ok(operation)
}

/// 创建 IFileOperation 进度回调和共享结果集合。
fn create_operation_sink() -> (
    IFileOperationProgressSink,
    Arc<Mutex<Vec<ShellOperationItemResult>>>,
) {
    let results = Arc::new(Mutex::new(Vec::new())); // COM 回调与命令返回共享的项目结果集合。
    let sink: IFileOperationProgressSink = FileOperationProgressSink {
        results: Arc::clone(&results),
        pending_sources: Arc::new(Mutex::new(VecDeque::new())),
    }
    .into(); // 当前 IFileOperation 使用的实际结果采集器。
    (sink, results)
}

/// 执行已经排队的 IFileOperation 并生成统一公开结果。
fn perform_file_operation(
    operation: &IFileOperation,
    kind: ShellOperationKind,
    reversible: bool,
    permanent: bool,
    effect: Option<DROPEFFECT>,
    results: Arc<Mutex<Vec<ShellOperationItemResult>>>,
) -> Result<ShellOperationResult, String> {
    unsafe { operation.PerformOperations() }
        .map_err(|error| format!("执行 Shell 文件操作失败: {error}"))?;
    let aborted = unsafe { operation.GetAnyOperationsAborted() }
        .map_err(|error| format!("读取 Shell 操作取消状态失败: {error}"))?
        .as_bool(); // Shell 是否报告用户取消或跳过操作。
    let items = results
        .lock()
        .map_err(|_| "读取 Shell 项目操作结果失败".to_string())?
        .clone(); // 每个项目由进度回调确认的实际结果。
    let succeeded = items.iter().filter(|item| item.succeeded).count(); // Shell 确认成功的项目数量。
    let failed = items.len().saturating_sub(succeeded); // Shell 回报失败的项目数量。
    let operation_name = match kind {
        ShellOperationKind::Rename => "重命名",
        ShellOperationKind::Recycle => "移入回收站",
        ShellOperationKind::PermanentDelete => "永久删除",
        ShellOperationKind::Paste => "粘贴",
        ShellOperationKind::Copy | ShellOperationKind::Cut | ShellOperationKind::Drag => {
            "Shell 操作"
        }
    }; // 用户可见的操作名称。
    let message = if aborted && succeeded == 0 {
        format!("{operation_name}已取消")
    } else if failed > 0 {
        format!("{operation_name}完成：成功 {succeeded} 项，失败 {failed} 项")
    } else if items.is_empty() {
        format!("{operation_name}已由 Windows Shell 完成")
    } else {
        format!("{operation_name}完成：{succeeded} 项")
    }; // 按实际成功、失败和取消状态生成的用户说明。
    Ok(ShellOperationResult {
        operation_id: 0,
        completed_at_unix_micros: 0,
        kind,
        aborted,
        reversible,
        permanent,
        effect: effect.map(|value| drop_effect_name(value).to_string()),
        items,
        message,
    })
}

/// 读取 IShellItem 的桌面绝对解析名称并释放 Shell 内存。
fn shell_item_reference(item: &IShellItem) -> Result<String, String> {
    shell_item_name(item, SIGDN_DESKTOPABSOLUTEPARSING)
}

/// 读取补偿日志需要的项目稳定引用、父级引用和编辑名称。
fn shell_item_metadata(item: &IShellItem) -> ShellItemMetadata {
    ShellItemMetadata {
        reference: shell_item_reference(item).unwrap_or_default(),
        parent_reference: unsafe { item.GetParent() }
            .ok()
            .and_then(|parent| shell_item_reference(&parent).ok()),
        name: shell_item_name(item, SIGDN_PARENTRELATIVEEDITING).ok(),
    }
}

/// 读取 IShellItem 指定语义的名称并释放 Shell 内存。
fn shell_item_name(item: &IShellItem, kind: SIGDN) -> Result<String, String> {
    let name = unsafe { item.GetDisplayName(kind) }
        .map_err(|error| format!("读取 Shell 项目名称失败: {error}"))?; // Shell 分配的指定语义名称。
    let reference =
        unsafe { name.to_string() }.map_err(|error| format!("转换 Shell 项目名称失败: {error}")); // 已转换的 UTF-8 名称。
    unsafe { CoTaskMemFree(Some(name.0.cast())) };
    reference
}

/// 将复制或剪切首选效果写入 Shell IDataObject。
fn set_preferred_drop_effect(data_object: &IDataObject, effect: DROPEFFECT) -> Result<(), String> {
    let format_id = unsafe { RegisterClipboardFormatW(CFSTR_PREFERREDDROPEFFECT) }; // 系统首选投放效果剪贴板格式。
    if format_id == 0 {
        return Err("注册 Shell 首选投放效果格式失败".to_string());
    }
    let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, size_of::<u32>()) }
        .map_err(|error| format!("分配 Shell 剪贴板效果内存失败: {error}"))?; // IDataObject 接管的可移动全局内存。
    let value = unsafe { GlobalLock(memory) }.cast::<u32>(); // 写入首选效果的 DWORD 地址。
    if value.is_null() {
        let mut medium = STGMEDIUM {
            tymed: TYMED_HGLOBAL.0 as u32,
            u: STGMEDIUM_0 { hGlobal: memory },
            ..Default::default()
        }; // 用于释放失败分配的存储介质。
        unsafe { ReleaseStgMedium(&mut medium) };
        return Err("锁定 Shell 剪贴板效果内存失败".to_string());
    }
    unsafe { value.write(effect.0) };
    let _ = unsafe { GlobalUnlock(memory) };
    let format = FORMATETC {
        cfFormat: format_id as u16,
        ptd: null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }; // Shell 首选投放效果的数据格式描述。
    let mut medium = STGMEDIUM {
        tymed: TYMED_HGLOBAL.0 as u32,
        u: STGMEDIUM_0 { hGlobal: memory },
        ..Default::default()
    }; // 将所有权转交给 IDataObject 的效果存储介质。
    if let Err(error) = unsafe { data_object.SetData(&format, &medium, true) } {
        unsafe { ReleaseStgMedium(&mut medium) };
        return Err(format!("写入 Shell 首选投放效果失败: {error}"));
    }
    Ok(())
}

/// 从 Shell IDataObject 读取复制或剪切首选效果。
fn read_preferred_drop_effect(data_object: &IDataObject) -> Option<DROPEFFECT> {
    let format_id = unsafe { RegisterClipboardFormatW(CFSTR_PREFERREDDROPEFFECT) }; // 系统首选投放效果剪贴板格式。
    if format_id == 0 {
        return None;
    }
    let format = FORMATETC {
        cfFormat: format_id as u16,
        ptd: null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }; // 请求首选投放效果的格式描述。
    let mut medium = unsafe { data_object.GetData(&format) }.ok()?; // IDataObject 返回的效果存储介质。
    let memory = unsafe { medium.u.hGlobal }; // 存放 DROPEFFECT DWORD 的全局内存。
    let value = unsafe { GlobalLock(memory) }.cast::<u32>(); // 只读首选效果 DWORD 地址。
    let effect = (!value.is_null()).then(|| DROPEFFECT(unsafe { value.read() })); // 已读取的可选系统投放效果。
    if !value.is_null() {
        let _ = unsafe { GlobalUnlock(memory) };
    }
    unsafe { ReleaseStgMedium(&mut medium) };
    effect
}

/// 将系统 DROPEFFECT 转换为用户可见的稳定名称。
fn drop_effect_name(effect: DROPEFFECT) -> &'static str {
    if (effect & DROPEFFECT_MOVE).0 != 0 {
        "移动"
    } else if (effect & DROPEFFECT_COPY).0 != 0 {
        "复制"
    } else if (effect & DROPEFFECT_LINK).0 != 0 {
        "创建链接"
    } else {
        "无"
    }
}

/// 初始化专用 Shell 线程的 STA COM apartment。
pub fn initialize_shell_apartment() -> Result<(), String> {
    unsafe { OleInitialize(None) }.map_err(|error| format!("初始化 Shell STA/OLE 失败: {error}"))
}

/// 释放专用 Shell 线程的 COM apartment。
pub fn uninitialize_shell_apartment() {
    unsafe { OleUninitialize() };
}

/// 枚举 Windows Shell 桌面命名空间中的完整项目集合。
pub fn enumerate_desktop_items() -> Result<Vec<DesktopItem>, String> {
    // Windows Shell 桌面根目录接口。
    let desktop = unsafe { SHGetDesktopFolder() }
        .map_err(|error| format!("获取 Shell 桌面命名空间失败: {error}"))?;
    // Shell 桌面项目枚举器。
    let mut enumerator = None;
    // 同时包含文件夹、普通项目和隐藏项目的枚举标志。
    let flags = (SHCONTF_FOLDERS.0 | SHCONTF_NONFOLDERS.0 | SHCONTF_INCLUDEHIDDEN.0) as u32;
    unsafe { desktop.EnumObjects(HWND::default(), flags, &mut enumerator) }
        .ok()
        .map_err(|error| format!("创建 Shell 桌面项目枚举器失败: {error}"))?;
    // 已创建的 Shell 桌面项目枚举器。
    let enumerator = enumerator.ok_or_else(|| "Shell 桌面项目枚举器为空".to_string())?;
    // 本次枚举观察到的完整项目集合。
    let mut items = Vec::new();

    loop {
        // Shell 为当前项目分配的 PIDL。
        let mut pidl: *mut ITEMIDLIST = null_mut();
        // 当前调用实际返回的项目数量。
        let mut fetched = 0;
        // 当前项目枚举结果。
        let result =
            unsafe { enumerator.Next(std::slice::from_mut(&mut pidl), Some(&mut fetched)) };
        result
            .ok()
            .map_err(|error| format!("读取 Shell 桌面项目失败: {error}"))?;
        if fetched == 0 {
            break;
        }
        if pidl.is_null() {
            continue;
        }

        // 当前项目的可重新解析稳定引用。
        let stable_reference = shell_name(pidl, SIGDN_DESKTOPABSOLUTEPARSING)
            .unwrap_or_else(|_| encoded_pidl_reference(pidl));
        // 从稳定引用重新解析出的 PIDL。
        let resolved_pidl = parse_desktop_item_reference(&stable_reference);
        // 当前项目的系统显示名称读取结果。
        let display_name = match &resolved_pidl {
            Ok(resolved) => shell_name(resolved.0, SIGDN_NORMALDISPLAY),
            Err(error) => Err(error.clone()),
        };
        // 当前项目的系统图像读取结果。
        let system_image = resolved_pidl
            .as_ref()
            .ok()
            .and_then(|resolved| system_image_data_url(resolved.0));
        // 当前项目元数据可解析状态。
        let availability = if display_name.is_ok() && system_image.is_some() {
            DesktopItemAvailability::Available
        } else {
            DesktopItemAvailability::Unavailable
        };
        // 元数据失败时仍可呈现并恢复的项目名称。
        let display_name = display_name.unwrap_or_else(|_| stable_reference.clone());

        items.push(DesktopItem {
            stable_reference,
            display_name,
            system_image,
            availability,
        });
        unsafe { CoTaskMemFree(Some(pidl.cast())) };
    }

    Ok(items)
}

/// 读取 PIDL 对应的 Shell 名称。
fn shell_name(pidl: *const ITEMIDLIST, kind: SIGDN) -> Result<String, String> {
    // Shell 分配的名称缓冲区。
    let name_pointer = unsafe { SHGetNameFromIDList(pidl, kind) }
        .map_err(|error| format!("读取 Shell 项目名称失败: {error}"))?;
    // 已转换的 UTF-8 Shell 名称。
    let name = unsafe { name_pointer.to_string() }
        .map_err(|error| format!("转换 Shell 项目名称失败: {error}"));
    unsafe { CoTaskMemFree(Some(name_pointer.0.cast())) };
    name
}

/// 将 PIDL 编码为可重新提交给 Shell 的稳定兜底引用。
fn encoded_pidl_reference(pidl: *const ITEMIDLIST) -> String {
    // Shell PIDL 的完整字节长度。
    let byte_length = unsafe { ILGetSize(Some(pidl)) } as usize;
    // 当前 PIDL 的二进制内容。
    let bytes = unsafe { std::slice::from_raw_parts(pidl.cast::<u8>(), byte_length) };
    format!("pidl:{}", STANDARD.encode(bytes))
}

/// 将稳定引用重新解析为 Shell PIDL。
fn parse_desktop_item_reference(reference: &str) -> Result<OwnedPidl, String> {
    if let Some(encoded) = reference.strip_prefix("pidl:") {
        // 从稳定引用解码并校验的 PIDL 字节。
        let bytes = decode_pidl_bytes(encoded)?;
        // COM 任务分配器提供的 PIDL 内存。
        let pidl = unsafe { CoTaskMemAlloc(bytes.len()) }.cast::<ITEMIDLIST>();
        if pidl.is_null() {
            return Err("分配 Shell PIDL 内存失败".to_string());
        }
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), pidl.cast::<u8>(), bytes.len()) };
        return Ok(OwnedPidl(pidl));
    }

    // 传给 Shell 的稳定解析名称。
    let display_name = HSTRING::from(reference);
    // Shell 分配的解析结果 PIDL。
    let mut pidl = null_mut();
    unsafe { SHParseDisplayName(&display_name, None, &mut pidl, 0, None) }
        .map_err(|error| format!("重新解析 Shell 项目引用失败: {error}"))?;
    if pidl.is_null() {
        return Err("Shell 项目引用解析结果为空".to_string());
    }

    Ok(OwnedPidl(pidl))
}

/// 解码并校验序列化 PIDL 的项目边界与终止项。
fn decode_pidl_bytes(encoded: &str) -> Result<Vec<u8>, String> {
    // Base64 解码后的 PIDL 字节。
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|error| format!("解码 Shell PIDL 引用失败: {error}"))?;
    // 当前 SHITEMID 的起始偏移。
    let mut offset = 0;

    loop {
        if offset + 2 > bytes.len() {
            return Err("Shell PIDL 引用缺少终止项".to_string());
        }

        // 当前 SHITEMID 的完整字节长度。
        let item_length = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as usize;
        if item_length == 0 {
            if offset + 2 == bytes.len() {
                return Ok(bytes);
            }
            return Err("Shell PIDL 引用包含终止项后的数据".to_string());
        }
        if item_length < 2 || offset + item_length > bytes.len() {
            return Err("Shell PIDL 引用包含无效项目长度".to_string());
        }

        offset += item_length;
    }
}

/// 读取 PIDL 对应的 48 像素 Shell 系统图像。
fn system_image_data_url(pidl: *const ITEMIDLIST) -> Option<String> {
    // Shell 系统图像索引结果。
    let mut file_info = SHFILEINFOW::default();
    // PIDL 查询结果标志。
    let query_result = unsafe {
        SHGetFileInfoW(
            windows::core::PCWSTR(pidl.cast::<u16>()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut file_info),
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_PIDL | SHGFI_SYSICONINDEX,
        )
    };
    if query_result == 0 {
        return None;
    }

    // 与 Explorer 桌面大图标模式一致的 HICON。
    let icon = unsafe {
        SHGetImageList::<IImageList>(SHIL_EXTRALARGE as i32)
            .ok()?
            .GetIcon(file_info.iIcon, ILD_TRANSPARENT.0)
            .ok()?
    };
    // 供浏览器直接呈现的 PNG data URL。
    let encoded = unsafe { icon_data_url(icon) };
    unsafe {
        let _ = DestroyIcon(icon);
    }
    encoded
}

/// 将 Shell HICON 编码为浏览器可直接渲染的 PNG data URL。
unsafe fn icon_data_url(icon: HICON) -> Option<String> {
    /// Desktop Tools 桌面项目图标边长。
    const ICON_SIZE: i32 = 48;
    // 用于绘制图标的内存设备上下文。
    let device_context = unsafe { CreateCompatibleDC(None) };
    if device_context.is_invalid() {
        return None;
    }

    // 自顶向下的 32 位 BGRA DIB 描述。
    let bitmap_info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: ICON_SIZE,
            biHeight: -ICON_SIZE,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    // DIB 像素内存首地址。
    let mut bits: *mut c_void = null_mut();
    // 承载系统图标的 DIB 句柄。
    let bitmap = unsafe {
        match CreateDIBSection(
            Some(device_context),
            &bitmap_info,
            DIB_RGB_COLORS,
            &mut bits,
            None,
            0,
        ) {
            Ok(value) => value,
            Err(_) => {
                let _ = DeleteDC(device_context);
                return None;
            }
        }
    };
    // 设备上下文之前选中的 GDI 对象。
    let previous_bitmap = unsafe { SelectObject(device_context, HGDIOBJ(bitmap.0)) };
    // DIB 像素字节长度。
    let byte_length = (ICON_SIZE * ICON_SIZE * 4) as usize;
    unsafe { std::ptr::write_bytes(bits, 0, byte_length) };
    // 系统图标绘制结果。
    let draw_result = unsafe {
        DrawIconEx(
            device_context,
            0,
            0,
            icon,
            ICON_SIZE,
            ICON_SIZE,
            0,
            None,
            DI_NORMAL,
        )
    };
    // 从 DIB 复制出的 BGRA 数据。
    let bgra = unsafe { std::slice::from_raw_parts(bits.cast::<u8>(), byte_length) };
    // 当前图标是否已经携带有效 alpha。
    let has_alpha = bgra.chunks_exact(4).any(|pixel| pixel[3] != 0);
    // PNG 编码器需要的 RGBA 数据。
    let mut rgba = Vec::with_capacity(byte_length);
    for pixel in bgra.chunks_exact(4) {
        // 旧式 HICON 缺少 alpha 时按非黑像素补全不透明度。
        let alpha = if has_alpha {
            pixel[3]
        } else if pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0 {
            255
        } else {
            0
        };
        rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], alpha]);
    }

    unsafe {
        SelectObject(device_context, previous_bitmap);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(device_context);
    }
    if draw_result.is_err() {
        return None;
    }

    // PNG 文件内容。
    let mut encoded = Vec::new();
    {
        // 48x48 RGBA PNG 编码器。
        let mut encoder = png::Encoder::new(&mut encoded, ICON_SIZE as u32, ICON_SIZE as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        // PNG 数据写入器。
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&rgba).ok()?;
    }

    Some(format!(
        "data:image/png;base64,{}",
        STANDARD.encode(encoded)
    ))
}

#[cfg(test)]
mod tests {
    use super::{encoded_pidl_reference, parse_desktop_item_reference};

    /// 验证 PIDL 稳定引用可解码并保持原始字节。
    #[test]
    fn encoded_pidl_reference_round_trips() {
        // 表示桌面根节点的终止 PIDL。
        let bytes = [0_u8, 0];
        // 编码后的 PIDL 稳定引用。
        let reference = encoded_pidl_reference(bytes.as_ptr().cast());
        // 从稳定引用重新解析出的 PIDL。
        let parsed = parse_desktop_item_reference(&reference).expect("PIDL 稳定引用应可重新解析");

        assert_eq!(encoded_pidl_reference(parsed.0), reference);
    }
}
