use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::mem::size_of;
use std::os::windows::fs::MetadataExt;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

static SHELL_DRAG_ACTIVE: AtomicBool = AtomicBool::new(false); // 宿主 UI STA 正在运行源端拖放，环境协调不得跨越本次会话。

/// 判断宿主是否正在运行源端 OLE 拖放。
pub(crate) fn shell_drag_active() -> bool { SHELL_DRAG_ACTIVE.load(Ordering::Acquire) }

use super::desktop_sources::{SYSTEM_ICONS, SourceConfigWatch, visible_icons};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use windows::Win32::Foundation::{DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, GetLastError, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, S_OK, SetLastError, WIN32_ERROR, WPARAM};
use windows::Win32::Graphics::Gdi::{BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, HGDIOBJ, SelectObject};
use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance, CoTaskMemAlloc, CoTaskMemFree, DVASPECT_CONTENT, FORMATETC, IDataObject, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::{DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, DROPEFFECT_NONE, DoDragDrop, IDropSource, IDropSource_Impl, OleGetClipboard, OleInitialize, OleSetClipboard, OleUninitialize, ReleaseStgMedium};
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
use windows::Win32::UI::Shell::CFSTR_LOGICALPERFORMEDDROPEFFECT;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::IShellFolder;
use windows::Win32::UI::Shell::{
    CFSTR_PREFERREDDROPEFFECT, CMF_NORMAL, CMINVOKECOMMANDINFO, CMINVOKECOMMANDINFOEX, CSIDL_DESKTOP, FOF_ALLOWUNDO, FOF_WANTNUKEWARNING, FOFX_ADDUNDORECORD, FOFX_RECYCLEONDELETE, FOLDERID_Desktop, FOLDERID_PublicDesktop, FileOperation, IContextMenu, IContextMenu2, IContextMenu3, IFileOperation, IFileOperationProgressSink, IFileOperationProgressSink_Impl, ILGetSize, IShellItem, SEE_MASK_IDLIST, SEE_MASK_UNICODE, SHCNE_ALLEVENTS, SHCNE_RENAMEFOLDER, SHCNE_RENAMEITEM, SHCNRF_NewDelivery, SHCNRF_ShellLevel, SHCONTF_FOLDERS, SHCONTF_INCLUDEHIDDEN, SHCONTF_NONFOLDERS, SHChangeNotification_Lock, SHChangeNotification_Unlock, SHChangeNotifyDeregister, SHChangeNotifyEntry, SHChangeNotifyRegister, SHCreateItemFromIDList, SHCreateItemFromParsingName, SHELLEXECUTEINFOW, SHFILEINFOW, SHGFI_PIDL, SHGFI_SYSICONINDEX, SHGetDesktopFolder, SHGetFileInfoW, SHGetImageList, SHGetKnownFolderIDList, SHGetNameFromIDList, SHGetSpecialFolderLocation, SHIL_EXTRALARGE, SHParseDisplayName, SIGDN,
    SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_FILESYSPATH, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEEDITING, ShellExecuteExW,
};
use windows::Win32::UI::Shell::{ILFindLastID, SHBindToParent};
use windows::Win32::UI::Shell::{KF_FLAG_DEFAULT, SHGetKnownFolderPath};
use windows::Win32::UI::WindowsAndMessaging::{AppendMenuW, CREATESTRUCTW, CreatePopupMenu, CreateWindowExW, DI_NORMAL, DefWindowProcW, DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW, DrawIconEx, GWLP_USERDATA, GetForegroundWindow, GetWindowLongPtrW, HICON, MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MSG, PostMessageW, PostQuitMessage, RegisterClassW, SW_SHOWNORMAL, SetForegroundWindow, SetWindowLongPtrW, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WINDOW_EX_STYLE, WM_APP, WM_CLOSE, WM_DESTROY, WM_DRAWITEM, WM_INITMENUPOPUP, WM_MEASUREITEM, WM_MENUCHAR, WM_NCCREATE, WM_NULL, WNDCLASSW, WS_POPUP};
use windows::Win32::UI::WindowsAndMessaging::{MsgWaitForMultipleObjects, PM_REMOVE, PeekMessageW, QS_ALLINPUT, WM_QUIT};
use windows::core::{BOOL, HRESULT, HSTRING, IUnknown, Interface, PCSTR, PCWSTR, Ref, implement, w};

use crate::contracts::{DesktopItem, DesktopItemAvailability, ShellOperationItemResult, ShellOperationKind, ShellOperationResult};

/// 持有由 Shell 或 COM 分配的 PIDL。
struct OwnedPidl(*mut ITEMIDLIST);

impl Drop for OwnedPidl
{
    /// 释放 PIDL 使用的 COM 任务内存。
    /// 释放适配器持有的原生资源。
    fn drop(&mut self) { unsafe { CoTaskMemFree(Some(self.0.cast())) }; }
}

/// 收集 IFileOperation 对每个项目回报的真实结果。
#[implement(IFileOperationProgressSink)]
struct FileOperationProgressSink
{
    expected: Option<super::file_proof::FileProof>, // 补偿专用的原始结果证据，Pre 回调再次核对。
    /// 跨 COM 回调与命令返回共享的项目结果集合。
    results: Arc<Mutex<Vec<ShellOperationItemResult>>>,
    /// 按 COM 对象身份保存操作前信息，目录回调交错时不混淆来源。
    pending_sources: Mutex<Vec<(IUnknown, ShellItemMetadata)>>,
}

/// 补偿操作需要的一个 Shell 项目身份、父级和编辑名称。
#[derive(Default)]
struct ShellItemMetadata
{
    compensation_blocked: Option<String>, // 在文件改变前确认是否涉及覆盖或目录合并。
    /// 项目自身的桌面绝对稳定引用。
    reference: String,
    /// 项目父级的桌面绝对稳定引用。
    parent_reference: Option<String>,
    /// 项目在父级中的可编辑名称。
    name: Option<String>,
}

/// 实现 IFileOperationProgressSink 的项目结果采集回调。
#[allow(non_snake_case)]
impl IFileOperationProgressSink_Impl for FileOperationProgressSink_Impl
{
    /// 接受 Shell 操作开始通知，不额外改变执行状态。
    fn StartOperations(&self) -> windows::core::Result<()> { Ok(()) }

    /// 接受 Shell 操作完成通知，具体结果由逐项回调保存。
    fn FinishOperations(&self, _hrresult: HRESULT) -> windows::core::Result<()> { Ok(()) }

    /// 在重命名前保存源项目身份与补偿信息。
    fn PreRenameItem(&self, _dwflags: u32, psiitem: Ref<'_, IShellItem>, _psznewname: &PCWSTR) -> windows::core::Result<()>
    {
        self.check_compensation(&psiitem, None, Some(_psznewname))?;
        self.record_source(psiitem);
        Ok(())
    }

    /// 记录重命名后的项目身份和执行结果。
    fn PostRenameItem(&self, _dwflags: u32, psiitem: Ref<'_, IShellItem>, _psznewname: &PCWSTR, hrrename: HRESULT, psinewlycreated: Ref<'_, IShellItem>) -> windows::core::Result<()>
    {
        self.record(psiitem, psinewlycreated, hrrename, hrrename == S_OK || hrrename == windows::Win32::UI::Shell::COPYENGINE_S_DONT_PROCESS_CHILDREN, _dwflags);
        Ok(())
    }

    /// 在移动前保存源项目身份与补偿信息。
    fn PreMoveItem(&self, dwflags: u32, psiitem: Ref<'_, IShellItem>, destination: Ref<'_, IShellItem>, new_name: &PCWSTR) -> windows::core::Result<()>
    {
        self.check_compensation(&psiitem, destination.as_ref(), Some(new_name))?;
        self.record_transfer_source(psiitem, destination, new_name, dwflags);
        Ok(())
    }

    /// 记录移动后的目标项目和执行结果。
    fn PostMoveItem(&self, _dwflags: u32, psiitem: Ref<'_, IShellItem>, _psidestinationfolder: Ref<'_, IShellItem>, _psznewname: &PCWSTR, hrmove: HRESULT, psinewlycreated: Ref<'_, IShellItem>) -> windows::core::Result<()>
    {
        self.record(psiitem, psinewlycreated, hrmove, hrmove == S_OK, _dwflags);
        Ok(())
    }

    /// 在复制前保存源项目元数据。
    fn PreCopyItem(&self, dwflags: u32, psiitem: Ref<'_, IShellItem>, destination: Ref<'_, IShellItem>, new_name: &PCWSTR) -> windows::core::Result<()>
    {
        self.record_transfer_source(psiitem, destination, new_name, dwflags);
        Ok(())
    }

    /// 记录复制生成的目标项目和执行结果。
    fn PostCopyItem(&self, _dwflags: u32, psiitem: Ref<'_, IShellItem>, _psidestinationfolder: Ref<'_, IShellItem>, _psznewname: &PCWSTR, hrcopy: HRESULT, psinewlycreated: Ref<'_, IShellItem>) -> windows::core::Result<()>
    {
        self.record(psiitem, psinewlycreated, hrcopy, hrcopy == S_OK, _dwflags);
        Ok(())
    }

    /// 在删除前保存源项目身份与补偿信息。
    fn PreDeleteItem(&self, _dwflags: u32, psiitem: Ref<'_, IShellItem>) -> windows::core::Result<()>
    {
        self.check_compensation(&psiitem, None, None)?;
        self.record_source(psiitem);
        Ok(())
    }

    /// 记录删除操作的项目结果和 HRESULT。
    fn PostDeleteItem(&self, _dwflags: u32, psiitem: Ref<'_, IShellItem>, hrdelete: HRESULT, psinewlycreated: Ref<'_, IShellItem>) -> windows::core::Result<()>
    {
        self.record(psiitem, psinewlycreated, hrdelete, hrdelete == S_OK, _dwflags);
        Ok(())
    }

    /// 允许 Shell 创建项目，不附加前置处理。
    fn PreNewItem(&self, _dwflags: u32, _psidestinationfolder: Ref<'_, IShellItem>, _psznewname: &PCWSTR) -> windows::core::Result<()> { Ok(()) }

    /// 记录新建项目及其执行结果。
    fn PostNewItem(&self, _dwflags: u32, psidestinationfolder: Ref<'_, IShellItem>, _psznewname: &PCWSTR, _psztemplatename: &PCWSTR, _dwfileattributes: u32, hrnew: HRESULT, psinewitem: Ref<'_, IShellItem>) -> windows::core::Result<()>
    {
        self.record(psidestinationfolder, psinewitem, hrnew, hrnew == S_OK, _dwflags);
        Ok(())
    }

    /// 接受进度通知，进度展示由 Shell 负责。
    fn UpdateProgress(&self, _iworktotal: u32, _iworksofar: u32) -> windows::core::Result<()> { Ok(()) }

    /// 接受 Shell 计时器重置通知，不维护独立计时器。
    fn ResetTimer(&self) -> windows::core::Result<()> { Ok(()) }

    /// 接受 Shell 计时暂停通知。
    fn PauseTimer(&self) -> windows::core::Result<()> { Ok(()) }

    /// 接受 Shell 计时恢复通知。
    fn ResumeTimer(&self) -> windows::core::Result<()> { Ok(()) }
}

impl FileOperationProgressSink_Impl
{
    /// 补偿不能覆盖新增冲突，也不能删除或移动已经变化的原结果。
    fn check_compensation(&self, source: &Ref<'_, IShellItem>, destination: Option<&IShellItem>, new_name: Option<&PCWSTR>) -> windows::core::Result<()>
    {
        let Some(expected) = &self.expected
        else
        {
            return Ok(());
        };
        let source = source.as_ref().ok_or_else(|| windows::core::Error::from_hresult(windows::Win32::Foundation::E_INVALIDARG))?;
        let path = shell_item_name(source, SIGDN_FILESYSPATH).map_err(|error| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, error))?;
        if !expected.subtree_unchanged(std::path::Path::new(&path))
        {
            return Err(windows::core::Error::new(windows::Win32::Foundation::E_FAIL, "补偿源身份或内容已经变化"));
        }
        if let Some(name) = new_name
        {
            let name = if name.is_null()
            {
                shell_item_name(source, SIGDN_PARENTRELATIVEEDITING)
            }
            else
            {
                unsafe { name.to_string() }.map_err(|error| error.to_string())
            }
            .map_err(|error| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, error))?;
            let parent = if let Some(destination) = destination
            {
                shell_item_name(destination, SIGDN_FILESYSPATH).map(std::path::PathBuf::from)
            }
            else
            {
                std::path::Path::new(&path).parent().map(std::path::Path::to_path_buf).ok_or_else(|| "补偿源没有父目录".to_string())
            }
            .map_err(|error| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, error))?;
            if parent.join(name).try_exists().unwrap_or(true)
            {
                return Err(windows::core::Error::new(windows::Win32::Foundation::E_FAIL, "补偿目标已存在，保留两侧文件"));
            }
        }
        Ok(())
    }

    /// 保留冲突决策前的目标存在性；不能把合并目录当作新建结果回收。
    fn record_transfer_source(&self, source: Ref<'_, IShellItem>, destination: Ref<'_, IShellItem>, new_name: &PCWSTR, flags: u32)
    {
        let identity = source.as_ref().and_then(|source| source.cast::<IUnknown>().ok()); // 当前 Pre 对象的规范身份。
        self.record_source(source);
        if let Ok(mut pending) = self.pending_sources.lock()
            && let Some((_, metadata)) = pending.iter_mut().rev().find(|(candidate, _)| Some(candidate) == identity.as_ref())
        {
            let name = if new_name.is_null() { metadata.name.clone() } else { unsafe { new_name.to_string() }.ok() }; // 使用 Shell 已协商的目标名称。
            let existing = destination.as_ref().and_then(|folder| shell_item_name(folder, SIGDN_FILESYSPATH).ok()).zip(name).map(|(parent, name)| std::path::Path::new(&parent).join(name).try_exists()); // 查询失败同样不能授权破坏性补偿。
            if flags & windows::Win32::UI::Shell::TSF_OVERWRITE_EXIST.0 as u32 != 0 || !matches!(existing, Some(Ok(false)))
            {
                metadata.compensation_blocked = Some("覆盖、目录合并或目标状态不明确，不能自动恢复".into());
            }
        }
    }

    /// 在 IFileOperation 改变项目身份前保存补偿所需元数据。
    fn record_source(&self, source: Ref<'_, IShellItem>)
    {
        let Some(source) = source.as_ref()
        else
        {
            return;
        };
        if let Ok(mut pending) = self.pending_sources.lock()
            && let Ok(identity) = source.cast::<IUnknown>()
        {
            pending.push((identity, shell_item_metadata(source)));
        }
    }

    /// 保存 Shell 回调提供的源引用、结果引用与 HRESULT。
    fn record(&self, source: Ref<'_, IShellItem>, result: Ref<'_, IShellItem>, status: HRESULT, succeeded: bool, flags: u32)
    {
        let identity = source.as_ref().and_then(|source| source.cast::<IUnknown>().ok()); // Post 回调实际来源的 COM 身份。
        let source = self
            .pending_sources
            .lock()
            .ok()
            .and_then(|mut pending| {
                let index = pending.iter().position(|(candidate, _)| Some(candidate) == identity.as_ref())?; // 只消费身份匹配的 Pre 元数据。
                Some(pending.remove(index).1)
            })
            .or_else(|| source.as_ref().map(shell_item_metadata))
            .unwrap_or_default(); // 无匹配时读取实际来源，不猜测队首项目。
        let result = result.as_ref().map(shell_item_metadata); // Post 回调提供的操作后结果项目元数据。
        let compensation_blocked = if flags & windows::Win32::UI::Shell::TSF_OVERWRITE_EXIST.0 as u32 != 0
        {
            Some("覆盖或目录合并不能自动恢复".into())
        }
        else if flags & windows::Win32::UI::Shell::TSF_RENAME_EXIST.0 as u32 != 0
        {
            None
        }
        else
        {
            source.compensation_blocked
        }; // 保留两者的原生重命名不等同于覆盖原目标。
        if let Ok(mut results) = self.results.lock()
        {
            // 非负 HRESULT 也可能表示跳过、等待或合并决策；完成状态由具体操作回调解释。
            results.push(ShellOperationItemResult { proof: None, compensation_blocked, source_reference: if source.reference.is_empty() { "未知项目".to_string() } else { source.reference }, result_reference: result.as_ref().map(|item| item.reference.clone()), succeeded, result_code: status.0, source_parent_reference: source.parent_reference, source_name: source.name, result_parent_reference: result.as_ref().and_then(|item| item.parent_reference.clone()), result_name: result.and_then(|item| item.name) });
        }
    }
}

/// 使用系统默认光标驱动 OLE 拖放生命周期。
#[implement(IDropSource)]
struct ShellDragSource
{
    drop_started: Arc<AtomicBool>, // QueryContinueDrag 确认投放后关闭释放丢失检查。
}

/// 仅观察原生按键和捕获，不访问或转移 Shell COM 对象。
fn watch_source_drag_release(source_thread: u32, drop_started: Arc<AtomicBool>) -> Result<(Sender<()>, JoinHandle<()>), String>
{
    let (stop, receiver) = mpsc::channel(); // OLE 返回后立即结束检查。
    let worker = thread::Builder::new()
        .name("desktop-tools-drag-release".into())
        .spawn(move || {
            use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE, VK_LBUTTON};
            use windows::Win32::UI::WindowsAndMessaging::{GUITHREADINFO, GetGUIThreadInfo, GetWindowThreadProcessId, WM_KEYDOWN};
            let mut released_at = None; // 给 OLE 正常 QueryContinueDrag 留出处理松键的时间。
            while matches!(receiver.recv_timeout(std::time::Duration::from_millis(50)), Err(mpsc::RecvTimeoutError::Timeout))
            {
                if drop_started.load(Ordering::Acquire)
                {
                    continue;
                }
                if unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) } < 0
                {
                    released_at = None;
                    continue;
                }
                let released = *released_at.get_or_insert_with(Instant::now);
                if released.elapsed() < Duration::from_millis(250)
                {
                    continue;
                }
                let mut info = GUITHREADINFO { cbSize: size_of::<GUITHREADINFO>() as u32, ..Default::default() }; // 当前源 STA 的 OLE 捕获窗口。
                if unsafe { GetGUIThreadInfo(source_thread, &mut info) }.is_err() || info.hwndCapture.is_invalid()
                {
                    continue;
                }
                if unsafe { GetWindowThreadProcessId(info.hwndCapture, None) } != source_thread || drop_started.load(Ordering::Acquire)
                {
                    continue;
                }
                // 只取消尚未进入 Drop 的失联源；窗口定向消息不会留给后续菜单，不在旧位置补做投放。
                match unsafe { PostMessageW(Some(info.hwndCapture), WM_KEYDOWN, WPARAM(VK_ESCAPE.0 as usize), LPARAM(0)) }
                {
                    Ok(()) =>
                    {
                        crate::log_warn!("拖放：源会话未收到鼠标释放，已请求取消");
                        break;
                    }
                    Err(error) => crate::log_error!("拖放：取消丢失释放消息的源会话失败，错误={error}"),
                }
            }
        })
        .map_err(|error| format!("启动拖放释放检查失败：{error}"))?; // 创建失败不启动无法恢复的 OLE 会话。
    Ok((stop, worker))
}

/// 使用 OLE 传入的按键决定正常释放，检查线程仅补充丢失消息的取消。
impl IDropSource_Impl for ShellDragSource_Impl
{
    /// 根据 Escape 和鼠标按键决定继续、取消或完成 OLE 拖动。
    fn QueryContinueDrag(&self, escape_pressed: BOOL, key_state: MODIFIERKEYS_FLAGS) -> HRESULT
    {
        if escape_pressed.as_bool()
        {
            return DRAGDROP_S_CANCEL;
        }
        if (key_state & MK_LBUTTON).0 == 0
        {
            self.drop_started.store(true, Ordering::Release);
            return DRAGDROP_S_DROP;
        }
        S_OK
    }

    /// 使用系统默认光标反馈当前拖放效果。
    fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT { DRAGDROP_S_USEDEFAULTCURSORS }
}

/// Shell 原生菜单显示的语义目标。
pub enum NativeMenuTarget
{
    /// 使用当前完整选择集合构建多选 Shell 菜单。
    Items(Vec<String>),
    /// 门户当前目录中的真实文件系统项目，不通过桌面子项枚举。
    FolderItems(Vec<String>),
    /// 使用 Shell 桌面视图构建背景菜单。
    Background,
    /// 只显示 Desktop Tools 产品命令。
    ProductOnly,
}

/// 统一撤销时间线提交给新 IFileOperation 的一个补偿动作。
pub enum ShellCompensationAction
{
    /// 将当前项目重命名为指定编辑名称。
    Rename
    {
        /// 当前可解析项目稳定引用。
        reference: String,
        /// 补偿后项目编辑名称。
        new_name: String,
    },
    /// 将当前项目移动回指定父级和名称。
    Move
    {
        /// 当前可解析项目稳定引用。
        reference: String,
        /// 补偿目标父级稳定引用。
        destination_parent_reference: String,
        /// 补偿后项目编辑名称。
        new_name: String,
    },
    /// 将当前项目移入回收站。
    Recycle
    {
        /// 当前可解析项目稳定引用。
        reference: String,
    },
}

/// 追加到原生菜单中的一个 Desktop Tools 产品命令。
pub struct NativeMenuCommand
{
    /// 返回给命令层的稳定命令编号。
    pub id: u32,
    /// 用户可见的中文命令名称。
    pub label: String,
    /// 完整选择集合是否共同支持该命令。
    pub enabled: bool,
}

/// 当前 Shell 菜单需要转发消息的接口版本。
enum MenuMessageHandler
{
    /// 支持返回菜单字符结果的 IContextMenu3。
    ContextMenu3(IContextMenu3),
    /// 支持 owner-drawn 与动态子菜单消息的 IContextMenu2。
    ContextMenu2(IContextMenu2),
}

/// 专用菜单 owner HWND 持有的当前 Shell 菜单接口。
#[derive(Default)]
struct MenuOwnerState
{
    /// TrackPopupMenu 期间接收窗口消息的 Shell 菜单接口。
    handlers: Vec<MenuMessageHandler>,
}

/// 承载 Shell 菜单、owner-drawn 项和原生确认界面的专用 HWND。
pub struct ShellMenuOwner
{
    /// 专用原生 owner 窗口句柄。
    hwnd: HWND,
    /// 窗口过程通过 GWLP_USERDATA 访问的稳定状态地址。
    state: Box<MenuOwnerState>,
}

/// 将 Shell 菜单消息转发到当前 IContextMenu3/2。
unsafe extern "system" fn shell_menu_owner_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT
{
    if message == WM_NCCREATE
    {
        // CreateWindowExW 提交的专用菜单状态。
        let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize) };
    }
    // 当前 owner 窗口关联的专用菜单状态。
    let state = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut MenuOwnerState;
    if !state.is_null() && matches!(message, WM_DRAWITEM | WM_MEASUREITEM | WM_INITMENUPOPUP | WM_MENUCHAR)
    {
        // Shell 菜单处理后返回给窗口消息循环的结果。
        let mut result = LRESULT(0);
        // TrackPopupMenu 期间设置的当前 Shell 菜单接口。
        let handlers = &unsafe { &*state }.handlers;
        let mut handled = false; // 组合菜单需要将动态消息送达每个原生处理器。
        for handler in handlers
        {
            match handler
            {
                MenuMessageHandler::ContextMenu3(menu) =>
                {
                    if unsafe { menu.HandleMenuMsg2(message, wparam, lparam, Some(&mut result)) }.is_ok()
                    {
                        handled = true;
                        if message == WM_MENUCHAR && result.0 != 0
                        {
                            return result;
                        }
                    }
                }
                MenuMessageHandler::ContextMenu2(menu) if unsafe { menu.HandleMenuMsg(message, wparam, lparam) }.is_ok() =>
                {
                    handled = true;
                }
                MenuMessageHandler::ContextMenu2(_) =>
                {}
            }
        }
        if handled
        {
            return result;
        }
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

impl ShellMenuOwner
{
    /// 在 Shell STA 线程创建专用隐藏 owner HWND。
    pub fn create() -> Result<Self, String>
    {
        // 当前进程模块句柄。
        let module = unsafe { GetModuleHandleW(None) }.map_err(|error| format!("获取菜单 owner 模块失败: {error}"))?;
        // 专用 owner 窗口类。
        let class = WNDCLASSW { lpfnWndProc: Some(shell_menu_owner_proc), hInstance: HINSTANCE(module.0), lpszClassName: w!("DesktopToolsShellMenuOwner"), ..Default::default() };
        if unsafe { RegisterClassW(&class) } == 0 && unsafe { GetLastError() } != windows::Win32::Foundation::ERROR_CLASS_ALREADY_EXISTS
        {
            return Err(format!("注册 Shell 菜单 owner 窗口类失败：{}", windows::core::Error::from_win32()));
        }

        // 窗口过程使用的稳定菜单状态。
        let mut state = Box::<MenuOwnerState>::default();
        // 专用隐藏 owner HWND。
        let hwnd = unsafe { CreateWindowExW(WINDOW_EX_STYLE::default(), w!("DesktopToolsShellMenuOwner"), w!(""), WS_POPUP, 0, 0, 0, 0, None, None, Some(HINSTANCE(module.0)), Some((&mut *state as *mut MenuOwnerState).cast())) }.map_err(|error| format!("创建 Shell 菜单 owner HWND 失败: {error}"))?;
        Ok(Self { hwnd, state })
    }

    /// 设置 TrackPopupMenu 期间接收消息的 Shell 菜单接口。
    fn set_context_menu(&mut self, context_menu: &IContextMenu)
    {
        let handler = context_menu.cast::<IContextMenu3>().map(MenuMessageHandler::ContextMenu3).or_else(|_| context_menu.cast::<IContextMenu2>().map(MenuMessageHandler::ContextMenu2)).ok();
        self.state.handlers.extend(handler);
    }

    /// 清除已经关闭的 Shell 菜单接口。
    fn clear_context_menu(&mut self) { self.state.handlers.clear(); }
}

impl Drop for ShellMenuOwner
{
    /// 销毁专用菜单 owner HWND。
    /// 释放适配器持有的原生资源。
    fn drop(&mut self) { let _ = unsafe { DestroyWindow(self.hwnd) }; }
}

/// Shell 桌面变化通知携带的可选精确重命名事实。
#[derive(Clone, Debug)]
pub enum ShellChangeEvent
{
    /// 需要重新枚举桌面命名空间的普通变化。
    Invalidated,
    /// Shell 通知提供的旧、新稳定引用。
    Renamed
    {
        old_reference: String, new_reference: String
    },
}

/// 持有 Shell 变化通知窗口线程。
pub struct ShellChangeWatcher
{
    /// 接收 SHChangeNotify 消息的隐藏窗口句柄。
    hwnd: isize,
    /// 持续泵送窗口消息的后台线程。
    worker: Option<JoinHandle<()>>,
}

/// Shell 变化通知使用的窗口消息编号。
const SHELL_CHANGE_MESSAGE: u32 = WM_APP + 0x51;

/// 将 SHChangeNotify 消息转换为失效或精确重命名事件。
unsafe extern "system" fn shell_change_window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT
{
    if message == WM_NCCREATE
    {
        // CreateWindowExW 提交的变化事件发送端。
        let create = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize) };
    }
    if message == SHELL_CHANGE_MESSAGE
    {
        // 当前变化通知窗口关联的事件发送端。
        let sender = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const Sender<ShellChangeEvent>;
        // Shell 通知锁定后返回的两个绝对 PIDL。
        let mut pidls: *mut *mut ITEMIDLIST = null_mut();
        // Shell 通知事件掩码。
        let mut event = 0;
        // 当前 NewDelivery 通知的共享内存锁。
        let lock = unsafe { SHChangeNotification_Lock(HANDLE(wparam.0 as *mut c_void), lparam.0 as u32, Some(&mut pidls), Some(&mut event)) };
        if !lock.is_invalid() && !sender.is_null()
        {
            crate::log_debug!("Shell：收到原始变化通知，事件掩码=0x{:08X}", event as u32);
            // 当前通知能够提供的精确变化事实。
            let change = if (event as u32 & (SHCNE_RENAMEITEM.0 | SHCNE_RENAMEFOLDER.0)) != 0 && !pidls.is_null()
            {
                // 重命名前的绝对项目 PIDL。
                let old_pidl = unsafe { *pidls };
                // 重命名后的绝对项目 PIDL。
                let new_pidl = unsafe { *pidls.add(1) };
                match (shell_name(old_pidl, SIGDN_DESKTOPABSOLUTEPARSING), shell_name(new_pidl, SIGDN_DESKTOPABSOLUTEPARSING))
                {
                    (Ok(old_reference), Ok(new_reference)) => ShellChangeEvent::Renamed { old_reference, new_reference },
                    _ => ShellChangeEvent::Invalidated,
                }
            }
            else
            {
                ShellChangeEvent::Invalidated
            };
            let _ = unsafe { &*sender }.send(change);
            let _ = unsafe { SHChangeNotification_Unlock(lock) };
        }
        return LRESULT(0);
    }
    if message == WM_CLOSE
    {
        let _ = unsafe { DestroyWindow(hwnd) };
        return LRESULT(0);
    }
    if message == WM_DESTROY
    {
        unsafe { PostQuitMessage(0) };
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

/// 启动桌面命名空间 SHChangeNotify 监听线程。
pub fn start_shell_change_watcher() -> Result<(ShellChangeWatcher, Receiver<ShellChangeEvent>), String>
{
    // 变化事件发送到应用协调线程的通道。
    let (events, receiver) = mpsc::channel();
    // 变化通知窗口启动结果通道。
    let (ready, started) = mpsc::sync_channel(0);
    // 持续泵送 SHChangeNotify 消息的 STA 线程。
    let worker = thread::Builder::new()
        .name("desktop-tools-shell-change".to_string())
        .spawn(move || {
            if let Err(error) = initialize_shell_apartment()
            {
                let _ = ready.send(Err(error));
                return;
            }
            // 当前进程模块句柄。
            let module = match unsafe { GetModuleHandleW(None) }
            {
                Ok(module) => module,
                Err(error) =>
                {
                    uninitialize_shell_apartment();
                    let _ = ready.send(Err(format!("获取 Shell 变化窗口模块失败: {error}")));
                    return;
                }
            };
            // Shell 变化通知窗口类。
            let class = WNDCLASSW { lpfnWndProc: Some(shell_change_window_proc), hInstance: HINSTANCE(module.0), lpszClassName: w!("DesktopToolsShellChangeWatcher"), ..Default::default() };
            if unsafe { RegisterClassW(&class) } == 0
            {
                uninitialize_shell_apartment();
                let _ = ready.send(Err("注册 Shell 变化窗口类失败".to_string()));
                return;
            }
            // 当前窗口过程生命周期内保持稳定的事件发送端。
            let events = Box::new(events);
            // 接收桌面变化消息的隐藏窗口。
            let hwnd = match unsafe { CreateWindowExW(WINDOW_EX_STYLE::default(), w!("DesktopToolsShellChangeWatcher"), w!(""), WS_POPUP, 0, 0, 0, 0, None, None, Some(HINSTANCE(module.0)), Some((&*events as *const Sender<ShellChangeEvent>).cast())) }
            {
                Ok(hwnd) => hwnd,
                Err(error) =>
                {
                    uninitialize_shell_apartment();
                    let _ = ready.send(Err(format!("创建 Shell 变化窗口失败: {error}")));
                    return;
                }
            };
            // 桌面命名空间、用户桌面物理目录和公共桌面物理目录 PIDL。
            let roots = match (unsafe { SHGetSpecialFolderLocation(None, CSIDL_DESKTOP as i32) }, unsafe { SHGetKnownFolderIDList(&FOLDERID_Desktop, 0, None) }, unsafe { SHGetKnownFolderIDList(&FOLDERID_PublicDesktop, 0, None) })
            {
                (Ok(namespace), Ok(user), Ok(public)) => vec![OwnedPidl(namespace), OwnedPidl(user), OwnedPidl(public)],
                (namespace, user, public) =>
                {
                    let _ = unsafe { DestroyWindow(hwnd) };
                    uninitialize_shell_apartment();
                    let _ = ready.send(Err(format!("读取桌面变化根 PIDL 失败，命名空间={namespace:?}，用户桌面={user:?}，公共桌面={public:?}")));
                    return;
                }
            };
            let mut registrations = Vec::new(); // 三个根分别使用 cEntries=1 的 Shell 变化注册标识。
            for root in &roots
            {
                let entry = SHChangeNotifyEntry { pidl: root.0, fRecursive: false.into() }; // 当前根直接子项的变化注册项。
                let registration = unsafe { SHChangeNotifyRegister(hwnd, SHCNRF_ShellLevel | SHCNRF_NewDelivery, SHCNE_ALLEVENTS.0 as i32, SHELL_CHANGE_MESSAGE, 1, &entry) }; // 当前根独立的 Shell 变化注册标识。
                if registration == 0
                {
                    for registration in registrations
                    {
                        let _ = unsafe { SHChangeNotifyDeregister(registration) };
                    }
                    let _ = unsafe { DestroyWindow(hwnd) };
                    uninitialize_shell_apartment();
                    let _ = ready.send(Err("注册桌面 Shell 变化通知失败".to_string()));
                    return;
                }
                registrations.push(registration);
            }
            let mut config_watches = match SourceConfigWatch::create_all()
            {
                Ok(watches) => watches,
                Err(error) =>
                {
                    for registration in registrations
                    {
                        let _ = unsafe { SHChangeNotifyDeregister(registration) };
                    }
                    let _ = unsafe { DestroyWindow(hwnd) };
                    uninitialize_shell_apartment();
                    let _ = ready.send(Err(error));
                    return;
                }
            }; // 同一 Shell 消息线程持有全部注册表订阅。
            let handles = config_watches.iter().map(|watch| watch.event).collect::<Vec<_>>(); // 等待中的系统通知事件。
            let _ = ready.send(Ok(hwnd.0 as isize));
            crate::log_info!("桌面来源：显示配置与策略监听已启动");

            // Shell 变化线程持续运行的 Windows 消息。
            let mut message = MSG::default();
            'messages: loop
            {
                let result = unsafe { MsgWaitForMultipleObjects(Some(&handles), false, u32::MAX, QS_ALLINPUT) }; // 注册表事件与 Shell 消息共同唤醒。
                if (result.0 as usize) < config_watches.len()
                {
                    crate::log_debug!("桌面来源：收到注册表配置变化通知，订阅索引={}", result.0);
                    if let Err(error) = config_watches[result.0 as usize].rearm()
                    {
                        crate::log_error!("桌面来源：配置监听重新注册失败，错误：{error}");
                        break;
                    }
                    let _ = events.send(ShellChangeEvent::Invalidated);
                }
                else if result.0 as usize != config_watches.len()
                {
                    crate::log_error!("桌面来源：等待配置变化失败，状态：{}", result.0);
                    break;
                }
                while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool()
                {
                    if message.message == WM_QUIT
                    {
                        break 'messages;
                    }
                    unsafe {
                        let _ = TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                }
            }
            for registration in registrations
            {
                let _ = unsafe { SHChangeNotifyDeregister(registration) };
            }
            drop(roots);
            drop(events);
            uninitialize_shell_apartment();
        })
        .map_err(|error| format!("启动 Shell 变化通知线程失败: {error}"))?;
    // 已创建的变化通知窗口句柄。
    let hwnd = started.recv().map_err(|error| format!("等待 Shell 变化通知启动失败: {error}"))??;
    Ok((ShellChangeWatcher { hwnd, worker: Some(worker) }, receiver))
}

impl Drop for ShellChangeWatcher
{
    /// 请求变化通知窗口退出并等待线程释放注册。
    /// 释放适配器持有的原生资源。
    fn drop(&mut self)
    {
        let _ = unsafe { PostMessageW(Some(HWND(self.hwnd as *mut c_void)), WM_CLOSE, WPARAM(0), LPARAM(0)) };
        if let Some(worker) = self.worker.take()
        {
            let _ = worker.join();
        }
    }
}

/// 使用 Shell 默认动词打开一个或多个桌面项目。
pub fn open_desktop_items(references: &[String]) -> Result<(), String>
{
    if references.is_empty()
    {
        return Err("桌面项目选择为空".to_string());
    }
    for reference in references
    {
        // 当前稳定引用重新解析得到的绝对 PIDL。
        let pidl = parse_desktop_item_reference(reference)?;
        // 使用 PIDL 调用 Shell 默认打开动词的参数。
        let mut invocation = SHELLEXECUTEINFOW { cbSize: size_of::<SHELLEXECUTEINFOW>() as u32, fMask: SEE_MASK_IDLIST, lpVerb: w!("open"), nShow: SW_SHOWNORMAL.0, lpIDList: pidl.0.cast(), ..Default::default() };
        unsafe { ShellExecuteExW(&mut invocation) }.map_err(|error| format!("Shell 默认打开失败，项目={reference}，错误={error}"))?;
    }
    Ok(())
}

/// 保留 Shell 隐藏的快捷方式后缀，普通文件仍允许显式修改扩展名。
fn shortcut_rename_name(source_path: Option<&str>, new_name: &str) -> String
{
    let extension = source_path.and_then(|path| std::path::Path::new(path).extension()).and_then(|value| value.to_str()); // 真实文件类型。
    match extension
    {
        Some(extension) if extension.eq_ignore_ascii_case("lnk") || extension.eq_ignore_ascii_case("url") =>
        {
            let suffix = format!(".{extension}"); // 保留原始大小写的快捷方式后缀。
            if new_name.to_ascii_lowercase().ends_with(&suffix.to_ascii_lowercase())
            {
                new_name.to_string()
            }
            else
            {
                format!("{new_name}{suffix}")
            }
        }
        _ => new_name.to_string(),
    }
}

/// 使用 IFileOperation 重命名单个桌面项目并返回 Shell 实际结果。
pub fn rename_desktop_item(owner: &ShellMenuOwner, reference: &str, new_name: &str) -> Result<ShellOperationResult, String>
{
    let new_name = new_name.trim(); // 提交给 Shell 的用户可见新名称。
    if new_name.is_empty() || new_name.contains('\0')
    {
        return Err("桌面项目新名称无效".to_string());
    }
    let item = resolve_shell_item(reference)?; // 等待重命名的真实 Shell 项目。
    let operation = create_file_operation(owner, FOF_ALLOWUNDO | FOFX_ADDUNDORECORD)?; // 写入系统撤销记录的原生文件操作。
    let (sink, results) = create_operation_sink(); // 捕获实际重命名结果的进度回调。
    let source_path = shell_item_name(&item, SIGDN_FILESYSPATH).ok(); // 真实文件路径，不依赖隐藏后缀的显示名。
    let shell_name = HSTRING::from(shortcut_rename_name(source_path.as_deref(), new_name)); // 保留快捷方式类型。
    unsafe { operation.RenameItem(&item, &shell_name, &sink) }.map_err(|error| format!("提交 Shell 重命名失败: {error}"))?;
    perform_file_operation(&operation, ShellOperationKind::Rename, true, false, None, results)
}

/// 使用 IFileOperation 将选择集合移入回收站或永久删除。
pub fn delete_desktop_items(owner: &ShellMenuOwner, references: &[String], permanent: bool) -> Result<ShellOperationResult, String>
{
    validate_reference_selection(references)?;
    // 回收站删除写入撤销记录，永久删除保留系统原生警告。
    let flags = if permanent
    {
        FOF_WANTNUKEWARNING
    }
    else
    {
        FOF_ALLOWUNDO | FOFX_ADDUNDORECORD | FOFX_RECYCLEONDELETE
    };
    let operation = create_file_operation(owner, flags)?; // 承载完整删除选择集合的原生操作。
    let (sink, results) = create_operation_sink(); // 捕获每个项目实际删除状态的进度回调。
    for reference in references
    {
        let item = resolve_shell_item(reference)?; // 当前等待删除的真实 Shell 项目。
        unsafe { operation.DeleteItem(&item, &sink) }.map_err(|error| format!("提交 Shell 删除失败，项目={reference}，错误={error}"))?;
    }
    perform_file_operation(&operation, if permanent { ShellOperationKind::PermanentDelete } else { ShellOperationKind::Recycle }, !permanent, permanent, None, results)
}

/// 使用新的 IFileOperation 执行统一时间线的重命名、移动或回收补偿。
pub fn compensate_desktop_items(owner: &ShellMenuOwner, kind: ShellOperationKind, actions: Vec<ShellCompensationAction>, proofs: Vec<super::file_proof::FileProof>) -> Result<ShellOperationResult, String>
{
    if actions.is_empty() || actions.len() != proofs.len()
    {
        return Err("Shell 补偿集合或安全依据无效".into());
    }
    let mut completed_items = Vec::new(); // 每个独立顶层补偿立即加入结果，后续失败不会清空。
    let mut aborted = false;
    let mut operation_error = None;
    for (action, expected) in actions.into_iter().zip(proofs)
    {
        let reference = match &action
        {
            ShellCompensationAction::Rename { reference, .. } | ShellCompensationAction::Move { reference, .. } | ShellCompensationAction::Recycle { reference } => reference.clone(),
        };
        let completed = (|| {
            if !expected.unchanged()
            {
                return Err("文件身份、内容或后代已经变化，保留当前文件".to_string());
            }
            let flags = if matches!(&action, ShellCompensationAction::Recycle { .. })
            {
                FOF_ALLOWUNDO | FOFX_ADDUNDORECORD | FOFX_RECYCLEONDELETE
            }
            else
            {
                FOF_ALLOWUNDO | FOFX_ADDUNDORECORD
            };
            let operation = create_file_operation(owner, flags)?;
            let (sink, results) = create_operation_sink_with_proof(Some(expected.clone())); // Pre 回调再次验证源和目标冲突。
            let cookie = unsafe { operation.Advise(&sink) }.map_err(|error| error.to_string())?;
            let queued = (|| -> Result<(), String> {
                let item = resolve_shell_item(&reference)?;
                match action
                {
                    ShellCompensationAction::Rename { new_name, .. } =>
                    {
                        let source_path = shell_item_name(&item, SIGDN_FILESYSPATH).ok();
                        let name = HSTRING::from(shortcut_rename_name(source_path.as_deref(), &new_name));
                        unsafe { operation.RenameItem(&item, &name, None) }.map_err(|error| error.to_string())
                    }
                    ShellCompensationAction::Move { destination_parent_reference, new_name, .. } =>
                    {
                        let destination = resolve_shell_item(&destination_parent_reference)?;
                        unsafe { operation.MoveItem(&item, &destination, &HSTRING::from(new_name), None) }.map_err(|error| error.to_string())
                    }
                    ShellCompensationAction::Recycle { .. } => unsafe { operation.DeleteItem(&item, None) }.map_err(|error| error.to_string()),
                }
            })();
            if let Err(error) = queued
            {
                let _ = unsafe { operation.Unadvise(cookie) };
                return Err(error);
            }
            let completed = perform_file_operation(&operation, kind, true, false, None, results);
            let _ = unsafe { operation.Unadvise(cookie) };
            completed
        })();
        match completed
        {
            Ok(mut result) =>
            {
                aborted |= result.aborted;
                operation_error = operation_error.or(result.operation_error);
                if !result.items.iter().any(|item| item.source_reference == reference)
                {
                    result.items.push(ShellOperationItemResult { proof: None, compensation_blocked: Some("缺少根补偿完成反馈".into()), source_reference: reference.clone(), result_reference: None, succeeded: false, result_code: windows::Win32::Foundation::E_UNEXPECTED.0, source_parent_reference: None, source_name: None, result_parent_reference: None, result_name: None });
                }
                for item in &mut result.items
                {
                    if !item.succeeded && item.source_reference == reference && expected.unchanged()
                    {
                        item.proof = Some(expected.clone());
                    } // 已明确未变化的失败源才允许用户重试。
                }
                completed_items.extend(result.items);
            }
            Err(error) =>
            {
                operation_error.get_or_insert(windows::Win32::Foundation::E_FAIL.0);
                completed_items.push(ShellOperationItemResult { proof: expected.unchanged().then_some(expected), compensation_blocked: Some(error), source_reference: reference, result_reference: None, succeeded: false, result_code: windows::Win32::Foundation::E_FAIL.0, source_parent_reference: None, source_name: None, result_parent_reference: None, result_name: None });
            }
        }
        if aborted
        {
            break;
        }
    }
    let completion = operation_error.map_or(Ok(()), |code| Err(windows::core::Error::from_hresult(HRESULT(code))));
    Ok(completed_file_operation(kind, true, false, None, completion, Ok(BOOL::from(aborted)), completed_items))
}

/// 将完整 Shell IDataObject 放入系统剪贴板并保留复制或剪切效果。
pub fn set_desktop_item_clipboard(owner: &ShellMenuOwner, references: &[String], cut: bool) -> Result<ShellOperationResult, String>
{
    validate_reference_selection(references)?;
    let data_object = if references.iter().all(|reference| std::path::Path::new(reference).is_absolute())
    {
        filesystem_data_object(references)? // 门户使用绝对文件系统路径，绑定真实 Shell 项目。
    }
    else
    {
        desktop_data_object(owner.hwnd, references)? // 桌面项目继续使用桌面命名空间。
    };
    let effect = if cut { DROPEFFECT_MOVE } else { DROPEFFECT_COPY }; // 系统剪贴板使用的首选投放效果。
    set_drop_effect(&data_object, CFSTR_PREFERREDDROPEFFECT, effect)?;
    unsafe { OleSetClipboard(&data_object) }.map_err(|error| format!("写入系统 Shell 剪贴板失败: {error}"))?;
    // 已成功包含在系统数据对象中的选择项目结果。
    let items = references.iter().map(|reference| ShellOperationItemResult { proof: None, compensation_blocked: None, source_reference: reference.clone(), result_reference: Some(reference.clone()), succeeded: true, result_code: S_OK.0, source_parent_reference: None, source_name: None, result_parent_reference: None, result_name: None }).collect::<Vec<_>>();
    let kind = if cut { ShellOperationKind::Cut } else { ShellOperationKind::Copy }; // 当前系统剪贴板操作类型。
    Ok(ShellOperationResult {
        operation_id: 0,
        completed_at_unix_micros: 0,
        kind,
        aborted: false,
        operation_error: None, // 当前操作没有整体原生错误。
        reversible: false,
        permanent: false,
        effect: Some(drop_effect_name(effect).to_string()),
        message: format!("已{} {} 个项目到系统剪贴板", if cut { "剪切" } else { "复制" }, references.len()),
        items,
    })
}

/// 门户无桌面项目身份时直接使用绝对路径写入剪贴板。
pub fn set_desktop_item_clipboard_for_paths(references: &[String], cut: bool) -> Result<ShellOperationResult, String>
{
    validate_reference_selection(references)?;
    let data_object = filesystem_data_object(references)?;
    let effect = if cut { DROPEFFECT_MOVE } else { DROPEFFECT_COPY };
    set_drop_effect(&data_object, CFSTR_PREFERREDDROPEFFECT, effect)?;
    unsafe { OleSetClipboard(&data_object) }.map_err(|error| format!("写入门户系统剪贴板失败：{error}"))?;
    let kind = if cut { ShellOperationKind::Cut } else { ShellOperationKind::Copy };
    Ok(ShellOperationResult { operation_id: 0, completed_at_unix_micros: 0, kind, aborted: false, operation_error: None, reversible: false, permanent: false, effect: Some(drop_effect_name(effect).to_string()), message: format!("已{} {} 个项目到系统剪贴板", if cut { "剪切" } else { "复制" }, references.len()), items: references.iter().map(|reference| ShellOperationItemResult { proof: None, compensation_blocked: None, source_reference: reference.clone(), result_reference: Some(reference.clone()), succeeded: true, result_code: S_OK.0, source_parent_reference: None, source_name: None, result_parent_reference: None, result_name: None }).collect() })
}

/// 使用系统剪贴板 IDataObject 和 IFileOperation 粘贴到桌面命名空间。
pub fn paste_desktop_items(owner: &ShellMenuOwner) -> Result<ShellOperationResult, String>
{
    let data_object = unsafe { OleGetClipboard() }.map_err(|error| format!("读取系统 Shell 剪贴板失败: {error}"))?; // 当前系统剪贴板完整数据对象。
    let preferred_effect = read_drop_effect(&data_object, CFSTR_PREFERREDDROPEFFECT).unwrap_or(DROPEFFECT_COPY); // 剪切或复制来源声明的首选效果。
    let effect = if (preferred_effect & DROPEFFECT_MOVE).0 != 0 { DROPEFFECT_MOVE } else { DROPEFFECT_COPY }; // 本次粘贴交给 IFileOperation 的真实动作。
    transfer_desktop_data(owner, &data_object, effect, None)
}

/// 使用系统剪贴板粘贴到指定门户目录。
pub fn paste_folder_portal_items(owner: &ShellMenuOwner, directory: &str) -> Result<ShellOperationResult, String>
{
    let data_object = unsafe { OleGetClipboard() }.map_err(|error| format!("读取系统 Shell 剪贴板失败：{error}"))?;
    let preferred = read_drop_effect(&data_object, CFSTR_PREFERREDDROPEFFECT).unwrap_or(DROPEFFECT_COPY);
    let effect = if (preferred & DROPEFFECT_MOVE).0 != 0 { DROPEFFECT_MOVE } else { DROPEFFECT_COPY };
    transfer_data_to_directory(owner, &data_object, effect, Some(directory), None)
}

/// 在 Shell STA 将数据对象复制或移动到当前用户真实桌面，供剪贴板与原生投放共同执行。
pub(crate) fn default_drop_effect(data: &IDataObject, sources: &[String]) -> DROPEFFECT
{
    if let Some(preferred) = read_drop_effect(data, CFSTR_PREFERREDDROPEFFECT).filter(|effect| *effect == DROPEFFECT_COPY || *effect == DROPEFFECT_MOVE)
    {
        return preferred;
    }
    let same_volume = (|| {
        let path = unsafe { SHGetKnownFolderPath(&FOLDERID_Desktop, KF_FLAG_DEFAULT, None) }.ok()?;
        let value = unsafe { path.to_string() }.ok();
        unsafe { CoTaskMemFree(Some(path.0.cast())) };
        let volume = super::file_proof::volume_id(std::path::Path::new(&value?)).ok()?;
        Some(sources.iter().all(|source| super::file_proof::volume_id(std::path::Path::new(source)).ok() == Some(volume)))
    })()
    .unwrap_or(false); // 来源或目标卷无法确认时使用系统常规复制语义。
    if same_volume { DROPEFFECT_MOVE } else { DROPEFFECT_COPY }
}

/// 门户当前目录的默认效果按实际目标卷判断，保留数据对象的显式首选效果。
pub(crate) fn preferred_drop_effect(data: &IDataObject) -> Option<DROPEFFECT> { read_drop_effect(data, CFSTR_PREFERREDDROPEFFECT).filter(|effect| *effect == DROPEFFECT_COPY || *effect == DROPEFFECT_MOVE) }

/// 普通文件源与目标在同一卷默认移动，否则默认复制。
pub(crate) fn directory_drop_effect(sources: &[String], destination: &str) -> DROPEFFECT
{
    let volume = super::file_proof::volume_id(std::path::Path::new(destination)).ok();
    if volume.is_some() && sources.iter().all(|source| super::file_proof::volume_id(std::path::Path::new(source)).ok() == volume)
    {
        DROPEFFECT_MOVE
    }
    else
    {
        DROPEFFECT_COPY
    }
}

/// 在 Shell STA 将数据对象复制或移动到当前用户真实桌面，供剪贴板与原生投放共同执行。
pub(crate) fn transfer_desktop_data(owner: &ShellMenuOwner, data_object: &IDataObject, effect: DROPEFFECT, selected: Option<&[String]>) -> Result<ShellOperationResult, String> { transfer_data_to_directory(owner, data_object, effect, None, selected) }

/// 将 IDataObject 交给 IFileOperation，目标为指定文件系统目录。
pub(crate) fn transfer_data_to_directory(owner: &ShellMenuOwner, data_object: &IDataObject, effect: DROPEFFECT, directory: Option<&str>, selected: Option<&[String]>) -> Result<ShellOperationResult, String>
{
    if effect != DROPEFFECT_COPY && effect != DROPEFFECT_MOVE
    {
        return Err("桌面文件传输只接受复制或移动效果".to_string());
    }
    let destination: IShellItem = if let Some(directory) = directory
    {
        unsafe { SHCreateItemFromParsingName(&HSTRING::from(directory), None) }.map_err(|error| format!("创建门户 Shell 目标失败: {error}"))?
    }
    else
    {
        let desktop_pidl = unsafe { SHGetKnownFolderIDList(&FOLDERID_Desktop, KF_FLAG_DEFAULT.0 as u32, None) }.map(OwnedPidl).map_err(|error| format!("读取用户桌面目标失败: {error}"))?;
        unsafe { SHCreateItemFromIDList(desktop_pidl.0) }.map_err(|error| format!("创建桌面 Shell 目标失败: {error}"))?
    }; // IFileOperation 使用的真实目标目录。
    let filesystem_sources = super::drop_target::filesystem_drop_items(data_object); // 已存在文件的逐项传输路径；虚拟剪贴板仍交给 Shell 批量接口。
    if selected.is_some() && filesystem_sources.is_none()
    {
        return Err("投放数据不包含完整文件系统来源".into());
    }
    if let Some(all_sources) = filesystem_sources
    {
        let sources = selected.unwrap_or(&all_sources); // 混合投放只传输新来源，Ctrl 则由调用方选择整组。
        validate_reference_selection(sources)?;
        if sources.iter().any(|source| !all_sources.contains(source))
        {
            return Err("传输来源不属于本次拖放数据".into());
        }
        let mut completed_items = Vec::new(); // 已实际完成的顶层结果，不会被后续失败抹去。
        let mut aborted = false; // 用户取消停止剩余项目。
        let mut operation_error = None; // 逐项执行期间首个整体错误。
        for reference in sources
        {
            let before = shell_file_proof(reference).ok(); // 源完整内容只用于判断新结果是否完整，不保存副本。
            let completed = (|| {
                let source = resolve_shell_item(reference)?; // 每个顶层来源独立执行，目录后代仍由 Shell 管理。
                let operation = create_file_operation(owner, FOF_ALLOWUNDO | FOFX_ADDUNDORECORD)?;
                let (sink, results) = create_operation_sink();
                let cookie = unsafe { operation.Advise(&sink) }.map_err(|error| error.to_string())?; // 全局回调收集目录部分完成事实。
                let queued = if effect == DROPEFFECT_MOVE
                {
                    unsafe { operation.MoveItem(&source, &destination, PCWSTR::null(), None) }
                }
                else
                {
                    unsafe { operation.CopyItem(&source, &destination, PCWSTR::null(), None) }
                };
                if let Err(error) = queued
                {
                    let _ = unsafe { operation.Unadvise(cookie) };
                    return Err(error.to_string());
                }
                let completed = perform_file_operation(&operation, ShellOperationKind::Paste, true, false, Some(effect), results);
                let _ = unsafe { operation.Unadvise(cookie) };
                completed
            })();
            let mut item = match completed
            {
                Ok(mut completed) =>
                {
                    aborted |= completed.aborted;
                    operation_error = operation_error.or(completed.operation_error);
                    completed.items.iter().position(|item| &item.source_reference == reference).map(|index| completed.items.remove(index)).unwrap_or(ShellOperationItemResult { proof: None, compensation_blocked: Some("缺少顶层完成回调".into()), source_reference: reference.clone(), result_reference: None, succeeded: false, result_code: windows::Win32::Foundation::E_UNEXPECTED.0, source_parent_reference: None, source_name: None, result_parent_reference: None, result_name: None })
                }
                Err(error) =>
                {
                    operation_error.get_or_insert(windows::Win32::Foundation::E_FAIL.0);
                    ShellOperationItemResult { proof: None, compensation_blocked: Some(error), source_reference: reference.clone(), result_reference: None, succeeded: false, result_code: windows::Win32::Foundation::E_FAIL.0, source_parent_reference: None, source_name: None, result_parent_reference: None, result_name: None }
                }
            }; // 失败仍有明确来源，不把目录后代提升为根结果。
            if item.compensation_blocked.is_none()
            {
                let after = item.proof.take().or_else(|| item.result_reference.as_deref().and_then(|reference| shell_file_proof(reference).ok())); // DONT_PROCESS_CHILDREN 也需要完整结果证据。
                match (before, after)
                {
                    (Some(before), Some(after)) if before.same_contents(&after) =>
                    {
                        if item.result_code == windows::Win32::UI::Shell::COPYENGINE_S_DONT_PROCESS_CHILDREN.0
                        {
                            item.succeeded = true;
                        }
                        if item.succeeded
                        {
                            item.proof = Some(after);
                        }
                    }
                    (Some(_), Some(_)) =>
                    {
                        item.succeeded = false;
                        item.compensation_blocked = Some("结果内容与传输前来源不一致，保留文件并等待核对".into());
                    }
                    _ => item.compensation_blocked = Some("缺少完整来源或结果安全证据".into()),
                }
            }
            completed_items.push(item);
            if aborted
            {
                break;
            }
        }
        let completion = operation_error.map_or(Ok(()), |code| Err(windows::core::Error::from_hresult(HRESULT(code)))); // 整体错误与已完成部分同时返回。
        let reversible = completed_items.iter().any(|item| item.succeeded && item.proof.is_some());
        return Ok(completed_file_operation(ShellOperationKind::Paste, reversible, false, Some(effect), completion, Ok(BOOL::from(aborted)), completed_items));
    }
    let operation = create_file_operation(owner, FOF_ALLOWUNDO | FOFX_ADDUNDORECORD)?; // 支持系统撤销的粘贴操作。
    let (sink, results) = create_operation_sink(); // 收集粘贴产生的复制或移动实际结果。
    let cookie = unsafe { operation.Advise(&sink) }.map_err(|error| format!("监听 Shell 粘贴结果失败: {error}"))?; // 全局粘贴回调注册标识。
    let objects: IUnknown = data_object.cast().map_err(|error| format!("转换 Shell 剪贴板对象失败: {error}"))?; // IFileOperation 批量接口接收的数据对象。
    // 根据剪贴板首选效果提交移动或复制。
    let queued = if effect == DROPEFFECT_MOVE
    {
        unsafe { operation.MoveItems(&objects, &destination) }
    }
    else
    {
        unsafe { operation.CopyItems(&objects, &destination) }
    };
    if let Err(error) = queued
    {
        let _ = unsafe { operation.Unadvise(cookie) };
        return Err(format!("提交 Shell 粘贴失败: {error}"));
    }
    let result = perform_file_operation(&operation, ShellOperationKind::Paste, true, false, Some(effect), results); // 完成后返回的粘贴实际结果。
    let _ = unsafe { operation.Unadvise(cookie) };
    result
}

/// 在当前 Shell apartment 从已校验文件路径重建数据对象并执行目录传输。
pub(crate) fn transfer_paths_to_directory(owner: &ShellMenuOwner, effect: DROPEFFECT, directory: Option<&str>, references: &[String]) -> Result<ShellOperationResult, String>
{
    let data = filesystem_data_object(references)?;
    transfer_data_to_directory(owner, &data, effect, directory, Some(references))
}

/// 使用 Shell IDataObject 启动跨 Desktop Tools、Explorer 和其他程序的 OLE 拖放。
pub fn start_desktop_item_drag(data_object: IDataObject, references: &[String], pointer_offset: crate::contracts::WorkspacePoint, portal_source: bool) -> Result<ShellOperationResult, String>
{
    validate_reference_selection(references)?;
    let drop_started = Arc::new(AtomicBool::new(false)); // 目标已经开始 Drop 后不得取消真实文件操作。
    let drop_source: IDropSource = ShellDragSource { drop_started: Arc::clone(&drop_started) }.into(); // 使用系统默认光标的 OLE 拖动源。
    let allowed = DROPEFFECT_COPY | DROPEFFECT_MOVE | DROPEFFECT_LINK; // 外部目标可以协商的完整系统效果集合。
    let mut effect = DROPEFFECT_NONE; // DoDragDrop 返回的最终系统效果。
    let session = super::drop_target::SourceDragGuard::register(&data_object, references, pointer_offset, portal_source).map_err(|error| format!("登记系统拖动会话失败：{error}"))?; // 原生目标只认可本次完整数据对象。
    let source_thread = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() }; // COM 与 OLE 所属 STA。
    let (stop_release_watch, release_watch) = watch_source_drag_release(source_thread, Arc::clone(&drop_started))?; // 松键检查不依赖可能失去消息的源循环。
    SHELL_DRAG_ACTIVE.store(true, Ordering::Release);
    crate::log_info!("拖放：OLE 源会话开始，项目数量={}", references.len());
    let status = if unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(windows::Win32::UI::Input::KeyboardAndMouse::VK_LBUTTON.0 as i32) } < 0
    {
        unsafe { DoDragDrop(&data_object, &drop_source, allowed, &mut effect) }
    }
    else
    {
        DRAGDROP_S_CANCEL
    }; // 异步交接期间已经松开鼠标则取消，不能在新位置补做一次投放。
    drop_started.store(true, Ordering::Release);
    let _ = stop_release_watch.send(()); // 不让检查线程延续到下一个用户操作。
    if release_watch.join().is_err()
    {
        crate::log_error!("拖放：释放检查线程异常结束");
    }
    SHELL_DRAG_ACTIVE.store(false, Ordering::Release);
    crate::log_info!("拖放：OLE 源会话结束，状态={status:?}");
    if status.is_err() && !session.layout_committed()
    {
        return Err(format!("系统 OLE 拖放失败: {status:?}"));
    }
    let logical = read_drop_effect(&data_object, CFSTR_LOGICALPERFORMEDDROPEFFECT); // 优化移动的用户可见效果与删除责任分开解释。
    let (aborted, effect) = completed_drag_effect(status, effect, logical); // NONE 可以表示目标已完成优化移动。
    Ok(ShellOperationResult {
        operation_id: 0,
        completed_at_unix_micros: 0,
        kind: ShellOperationKind::Drag,
        aborted: aborted && !session.layout_committed(),
        operation_error: None, // 当前操作没有整体原生错误。
        reversible: false,
        permanent: false,
        effect: Some(drop_effect_name(effect).to_string()),
        items: Vec::new(), // 外部目标只返回整体反馈，不能伪造每项文件成功事实。
        message: if session.layout_committed()
        {
            "项目布局已完成".to_string()
        }
        else if aborted
        {
            "系统拖放已取消".to_string()
        }
        else if effect == DROPEFFECT_NONE
        {
            "目标未接受投放".to_string()
        }
        else
        {
            format!("系统拖放已完成，效果：{}", drop_effect_name(effect))
        },
    })
}

/// 区分用户取消和优化移动；逻辑效果仅用于呈现，不用于删除源文件。
fn completed_drag_effect(status: HRESULT, returned: DROPEFFECT, logical: Option<DROPEFFECT>) -> (bool, DROPEFFECT)
{
    if status == DRAGDROP_S_CANCEL
    {
        return (true, DROPEFFECT_NONE);
    }
    let effect = logical.filter(|effect| *effect == DROPEFFECT_COPY || *effect == DROPEFFECT_MOVE || *effect == DROPEFFECT_LINK).unwrap_or(returned); // 只接受明确单一效果。
    (false, effect)
}

/// 在当前 STA 结合桌面菜单类与真实目录上下文，目录对象保持到命令执行结束。
#[cfg(test)]
fn folder_background_menu(owner: HWND, directory: &str) -> Result<(IShellFolder, IContextMenu, IContextMenu), String>
{
    let pidl = parse_desktop_item_reference(directory)?; // 真实文件系统目录的绝对 PIDL。
    let desktop = unsafe { SHGetDesktopFolder() }.map_err(|error| format!("获取本地 Shell 根失败: {error}"))?; // 本进程 Shell 根。
    let folder: IShellFolder = unsafe { desktop.BindToObject(pidl.0, None) }.map_err(|error| format!("绑定背景菜单目录失败: {error}"))?; // 明确的新建目标。
    use windows::Win32::System::Registry::{HKEY, HKEY_CLASSES_ROOT, KEY_READ, RegCloseKey, RegOpenKeyExW};
    use windows::Win32::UI::Shell::{DEFCONTEXTMENU, SHCreateDefaultContextMenu};
    let menu: IContextMenu = unsafe { folder.CreateViewObject(owner) }.map_err(|error| format!("创建本地目录背景菜单失败: {error}"))?; // 保留粘贴、新建及目录扩展。
    let mut key = HKEY::default(); // 仅追加桌面专属动词，避免重复加载目录扩展。
    unsafe { RegOpenKeyExW(HKEY_CLASSES_ROOT, w!("DesktopBackground"), None, KEY_READ, &mut key) }.ok().map_err(|error| format!("读取桌面菜单注册类失败: {error}"))?;
    let mut definition = DEFCONTEXTMENU { hwnd: owner, pidlFolder: pidl.0, psf: std::mem::ManuallyDrop::new(Some(folder.clone())), cKeys: 1, aKeys: &key, ..Default::default() }; // 菜单接口在本 STA 创建，文件系统目录仍是命令目标。
    let result = unsafe { SHCreateDefaultContextMenu::<IContextMenu>(&definition) }; // Shell 加载本机注册的原生菜单扩展。
    unsafe { std::mem::ManuallyDrop::drop(&mut definition.psf) };
    let _ = unsafe { RegCloseKey(key) };
    let desktop_menu = result.map_err(|error| format!("创建本地桌面背景菜单失败: {error}"))?;
    Ok((folder, menu, desktop_menu))
}

/// 执行原生菜单命令并保留 Unicode 新建目标目录。
fn invoke_menu_command(menu: &IContextMenu, owner: HWND, command_offset: u32, directory: Option<&str>) -> Result<(), String>
{
    let directory = directory.map(HSTRING::from); // 持有到 InvokeCommand 返回的目录字符串。
    let invocation = CMINVOKECOMMANDINFOEX {
        cbSize: size_of::<CMINVOKECOMMANDINFOEX>() as u32,
        fMask: SEE_MASK_UNICODE, // Windows SDK 中 CMIC_MASK_UNICODE 与此标志同值。
        hwnd: owner,
        lpVerb: PCSTR(command_offset as usize as *const u8),
        lpVerbW: PCWSTR(command_offset as usize as *const u16),
        lpDirectoryW: directory.as_ref().map_or(PCWSTR::null(), |value| PCWSTR(value.as_ptr())),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    }; // 扩展结构通过兼容首部传给 Shell。
    unsafe { menu.InvokeCommand((&invocation as *const CMINVOKECOMMANDINFOEX).cast::<CMINVOKECOMMANDINFO>()) }.map_err(|error| format!("执行 Shell 菜单命令失败: {error}"))
}

/// 按 WM_CONTEXTMENU 协议打包坐标，保留位于主屏左侧或上方的负坐标。
fn desktop_menu_coordinates(x: i32, y: i32) -> Result<LPARAM, String>
{
    let x = i16::try_from(x).map_err(|_| "桌面菜单横坐标超出 Win32 消息范围")?; // 防止静默截断到其他屏幕。
    let y = i16::try_from(y).map_err(|_| "桌面菜单纵坐标超出 Win32 消息范围")?; // 防止静默截断到其他屏幕。
    Ok(LPARAM((u32::from(x as u16) | (u32::from(y as u16) << 16)) as isize))
}

/// 桌面背景交给 Explorer；项目和围栏仍使用当前应用的上下文菜单。
pub fn show_native_context_menu(owner: &mut ShellMenuOwner, target: NativeMenuTarget, screen_x: i32, screen_y: i32, product_commands: &[NativeMenuCommand]) -> Result<Option<u32>, String>
{
    if matches!(&target, NativeMenuTarget::Background)
    {
        use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, GetWindowThreadProcessId, WM_CONTEXTMENU};
        let desktop = super::desktop_host::discover_shell_desktop()?; // 每次重新发现真实 Explorer 视图，兼容 Explorer 重启。
        let view = HWND(desktop.def_view as *mut c_void); // 由 Explorer 自己处理背景菜单及全部 Shell 动词。
        let coordinates = desktop_menu_coordinates(screen_x, screen_y)?; // WM_CONTEXTMENU 使用有符号 16 位屏幕坐标。
        let mut process_id = 0; // 允许真实菜单所属进程取得前台。
        unsafe { GetWindowThreadProcessId(view, Some(&mut process_id)) };
        if let Err(error) = unsafe { AllowSetForegroundWindow(process_id) }
        {
            crate::log_debug!("Shell：授权 Explorer 菜单前台失败，进程={process_id}，错误={error}");
        }
        unsafe { PostMessageW(Some(view), WM_CONTEXTMENU, WPARAM(view.0 as usize), coordinates) }.map_err(|error| format!("请求 Explorer 桌面背景菜单失败: {error}"))?;
        crate::log_info!("Shell：背景菜单已交给 Explorer，视图={view:?}，位置=({screen_x},{screen_y})");
        return Ok(None); // 只确认消息投递；菜单与命令执行完全由 Explorer 持有。
    }
    /// Shell 菜单命令起始编号。
    const SHELL_COMMAND_FIRST: u32 = 1;
    /// Shell 菜单命令结束编号。
    const SHELL_COMMAND_LAST: u32 = 0x6fff;

    // Windows Shell 桌面根目录接口。
    let desktop = unsafe { SHGetDesktopFolder() }.map_err(|error| format!("获取 Shell 桌面命名空间失败: {error}"))?;
    // 保持项目子 PIDL 生命周期覆盖菜单显示的集合。
    let item_pidls = match &target
    {
        NativeMenuTarget::Items(references) => resolve_desktop_child_pidls(references)?,
        NativeMenuTarget::FolderItems(references) => references.iter().map(|reference| parse_desktop_item_reference(reference)).collect::<Result<Vec<_>, _>>()?,
        NativeMenuTarget::Background | NativeMenuTarget::ProductOnly => Vec::new(),
    };
    // 当前语义目标对应的可选 Shell 上下文菜单。
    let context_menu: Option<IContextMenu> = match target
    {
        NativeMenuTarget::Items(_) =>
        {
            // 传给桌面根目录的相对项目 PIDL 集合。
            let pidl_pointers = item_pidls.iter().map(|pidl| pidl.0.cast_const()).collect::<Vec<_>>();
            Some(unsafe { desktop.GetUIObjectOf(owner.hwnd, &pidl_pointers, None) }.map_err(|error| format!("创建多选 Shell 项目菜单失败: {error}"))?)
        }
        NativeMenuTarget::FolderItems(ref references) =>
        {
            validate_reference_selection(references)?;
            let parent = std::path::Path::new(&references[0]).parent(); // 多选菜单必须属于同一真实目录。
            if parent.is_none() || references.iter().any(|path| std::path::Path::new(path).parent() != parent)
            {
                return Err("门户菜单选择不属于同一目录".to_string());
            }
            let folder: IShellFolder = unsafe { SHBindToParent(item_pidls[0].0, None) }.map_err(|error| format!("绑定门户菜单目录失败：{error}"))?; // 父目录接口与绝对 PIDL 在菜单关闭前保持有效。
            let children = item_pidls.iter().map(|pidl| unsafe { ILFindLastID(pidl.0).cast_const() }).collect::<Vec<_>>(); // 目录内相对子项 PIDL。
            Some(unsafe { folder.GetUIObjectOf(owner.hwnd, &children, None) }.map_err(|error| format!("创建门户原生菜单失败：{error}"))?)
        }
        NativeMenuTarget::Background => unreachable!("背景菜单已交给 Explorer"),
        NativeMenuTarget::ProductOnly => None,
    };
    // 承载 Shell 与 Desktop Tools 命令的原生菜单。
    let menu = unsafe { CreatePopupMenu() }.map_err(|error| format!("创建原生上下文菜单失败: {error}"))?;
    // 填充失败也释放处理器和 HMENU，避免影响后续菜单请求。
    let prepared: Result<(), String> = (|| {
        if let Some(context_menu) = &context_menu
        {
            unsafe { context_menu.QueryContextMenu(menu, 0, SHELL_COMMAND_FIRST, SHELL_COMMAND_LAST, CMF_NORMAL).ok() }.map_err(|error| format!("填充 Shell 上下文菜单失败: {error}"))?;
            owner.set_context_menu(context_menu);
        }

        if !product_commands.is_empty()
        {
            // Desktop Tools 产品命令子菜单。
            let product_menu = unsafe { CreatePopupMenu() }.map_err(|error| format!("创建 Desktop Tools 菜单失败: {error}"))?;
            for command in product_commands
            {
                // 当前产品命令的 UTF-16 用户可见名称。
                let label = HSTRING::from(&command.label);
                // 当前完整选择集合对应的菜单启用状态。
                let flags = if command.enabled { MF_STRING } else { MF_STRING | MF_GRAYED };
                unsafe { AppendMenuW(product_menu, flags, command.id as usize, PCWSTR(label.as_ptr())) }.map_err(|error| format!("添加 Desktop Tools 菜单项失败: {error}"))?;
            }
            if context_menu.is_some()
            {
                unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null()) }.map_err(|error| format!("添加菜单分隔线失败: {error}"))?;
            }
            unsafe { AppendMenuW(menu, MF_POPUP, product_menu.0 as usize, w!("Desktop Tools")) }.map_err(|error| format!("添加 Desktop Tools 子菜单失败: {error}"))?;
        }

        Ok(())
    })();
    if let Err(error) = prepared
    {
        owner.clear_context_menu();
        let _ = unsafe { DestroyMenu(menu) };
        return Err(error);
    }

    let foreground_before = unsafe { GetForegroundWindow() }; // 记录前台归属，区分激活失败与用户取消。
    let activated = unsafe { SetForegroundWindow(owner.hwnd) }.as_bool(); // 系统可能拒绝前台切换。
    let started = std::time::Instant::now(); // 弹出菜单持续时间。
    crate::log_debug!("Shell：开始弹出菜单，位置=({screen_x},{screen_y})，原前台={foreground_before:?}，所有者={:?}，激活={activated}", owner.hwnd);
    unsafe { SetLastError(WIN32_ERROR(0)) };
    // 用户在原生菜单中选择的命令编号。
    let selected = unsafe { TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON, screen_x, screen_y, None, owner.hwnd, None).0 as u32 };
    let tracking_error = unsafe { GetLastError() }; // 必须在其他 Win32 调用前读取。
    // Win32 弹出菜单协议要求向 owner 投递消息，使下次点击能正常激活菜单。
    let _ = unsafe { PostMessageW(Some(owner.hwnd), WM_NULL, WPARAM(0), LPARAM(0)) };
    crate::log_debug!("Shell：菜单跟踪结束，命令={selected}，耗时={}ms，错误=0x{:08X}，当前前台={:?}", started.elapsed().as_millis(), tracking_error.0, unsafe { GetForegroundWindow() });
    let result = if selected == 0 && tracking_error.0 != 0
    {
        Err(format!("显示 Shell 菜单失败: 0x{:08X}", tracking_error.0))
    }
    else if (SHELL_COMMAND_FIRST..=SHELL_COMMAND_LAST).contains(&selected)
    {
        invoke_menu_command(context_menu.as_ref().expect("Shell 命令只来自 Shell 上下文菜单"), owner.hwnd, selected - SHELL_COMMAND_FIRST, None).map(|()| None)
    }
    else
    {
        Ok(product_commands.iter().find(|command| command.enabled && command.id == selected).map(|command| command.id))
    }; // 动态菜单及消息处理器保持到执行完成；失败也执行清理。
    owner.clear_context_menu();
    let _ = unsafe { DestroyMenu(menu) };
    result
}

/// 从桌面根目录重新解析多选菜单使用的相对子 PIDL。
fn resolve_desktop_child_pidls(references: &[String]) -> Result<Vec<OwnedPidl>, String>
{
    if references.is_empty()
    {
        return Err("桌面项目菜单选择为空".to_string());
    }
    // 当前请求中需要解析的稳定引用集合。
    let requested = references.iter().cloned().collect::<HashSet<_>>();
    if requested.len() != references.len()
    {
        return Err("桌面项目菜单选择包含重复项目".to_string());
    }
    // Windows Shell 桌面根目录接口。
    let desktop = unsafe { SHGetDesktopFolder() }.map_err(|error| format!("获取 Shell 桌面命名空间失败: {error}"))?;
    // Shell 桌面项目枚举器。
    let mut enumerator = None;
    // 同时包含文件夹、普通项目和隐藏项目的枚举标志。
    let flags = (SHCONTF_FOLDERS.0 | SHCONTF_NONFOLDERS.0 | SHCONTF_INCLUDEHIDDEN.0) as u32;
    unsafe { desktop.EnumObjects(HWND::default(), flags, &mut enumerator) }.ok().map_err(|error| format!("创建 Shell 桌面项目枚举器失败: {error}"))?;
    // 已创建的 Shell 桌面项目枚举器。
    let enumerator = enumerator.ok_or_else(|| "Shell 桌面项目枚举器为空".to_string())?;
    // 已按稳定引用解析的相对子 PIDL。
    let mut resolved = HashMap::new();
    loop
    {
        // Shell 为当前项目分配的相对子 PIDL。
        let mut pidl: *mut ITEMIDLIST = null_mut();
        // 当前调用实际返回的项目数量。
        let mut fetched = 0;
        unsafe { enumerator.Next(std::slice::from_mut(&mut pidl), Some(&mut fetched)) }.ok().map_err(|error| format!("读取 Shell 桌面项目失败: {error}"))?;
        if fetched == 0
        {
            break;
        }
        if pidl.is_null()
        {
            continue;
        }
        // 当前相对子 PIDL 对应的稳定引用。
        let reference = shell_name(pidl, SIGDN_DESKTOPABSOLUTEPARSING).unwrap_or_else(|_| encoded_pidl_reference(pidl));
        if requested.contains(&reference)
        {
            resolved.insert(reference, OwnedPidl(pidl));
        }
        else
        {
            unsafe { CoTaskMemFree(Some(pidl.cast())) };
        }
    }
    references.iter().map(|reference| resolved.remove(reference).ok_or_else(|| format!("Shell 桌面项目已无法解析: {reference}"))).collect()
}

/// 校验 Shell 项目选择非空且不包含重复引用。
fn validate_reference_selection(references: &[String]) -> Result<(), String>
{
    if references.is_empty()
    {
        return Err("桌面项目选择为空".to_string());
    }
    let unique = references.iter().collect::<HashSet<_>>(); // 当前请求中唯一的稳定引用集合。
    if unique.len() != references.len()
    {
        return Err("桌面项目选择包含重复项目".to_string());
    }
    Ok(())
}

/// 从桌面根目录创建保留全部 Shell 格式的多选 IDataObject。
pub(crate) fn desktop_data_object(hwnd: HWND, references: &[String]) -> Result<IDataObject, String>
{
    let item_pidls = resolve_desktop_child_pidls(references)?; // 保持到 IDataObject 创建完成的相对子 PIDL 集合。
    let pidl_pointers = item_pidls.iter().map(|pidl| pidl.0.cast_const()).collect::<Vec<_>>(); // 传给桌面 Shell folder 的相对子 PIDL 指针。
    let desktop = unsafe { SHGetDesktopFolder() }.map_err(|error| format!("获取 Shell 桌面命名空间失败: {error}"))?; // IDataObject 所属的桌面根接口。
    unsafe { desktop.GetUIObjectOf(hwnd, &pidl_pointers, None) }.map_err(|error| format!("创建 Shell 多选数据对象失败: {error}"))
}

/// 按来源身份创建完整拖放数据对象；门户路径不经过桌面命名空间解析。
pub(crate) fn drag_data_object(hwnd: HWND, references: &[String], portal_source: bool) -> Result<IDataObject, String>
{
    if portal_source
    {
        filesystem_data_object(references)
    }
    else
    {
        desktop_data_object(hwnd, references)
    }
}

/// 从绝对文件系统路径创建可复制/剪切的 Shell 数据对象。
fn filesystem_data_object(references: &[String]) -> Result<IDataObject, String>
{
    let pidls = references.iter().map(|reference| parse_desktop_item_reference(reference)).collect::<Result<Vec<_>, _>>()?; // 绝对 PIDL 生命周期覆盖数据对象创建。
    let parent_path = std::path::Path::new(&references[0]).parent().ok_or("门户项目父目录无效")?;
    if references.iter().any(|reference| std::path::Path::new(reference).parent() != Some(parent_path))
    {
        return Err("门户选择必须来自同一目录".to_string());
    }
    let parent: IShellFolder = unsafe { SHBindToParent(pidls[0].0, None) }.map_err(|error| format!("绑定门户剪贴板目录失败：{error}"))?;
    let children = pidls.iter().map(|pidl| unsafe { ILFindLastID(pidl.0).cast_const() }).collect::<Vec<_>>(); // 同一父目录下的相对子 PIDL。
    unsafe { parent.GetUIObjectOf(HWND::default(), &children, None) }.map_err(|error| format!("创建门户多选数据对象失败：{error}"))
}

/// 将稳定引用解析为 IFileOperation 使用的 IShellItem。
fn resolve_shell_item(reference: &str) -> Result<IShellItem, String>
{
    let pidl = parse_desktop_item_reference(reference)?; // 当前稳定引用重新解析的绝对 PIDL。
    unsafe { SHCreateItemFromIDList(pidl.0) }.map_err(|error| format!("创建 Shell 项目失败，项目={reference}，错误={error}"))
}

/// 创建带 owner HWND 和指定系统行为标志的 IFileOperation。
fn create_file_operation(owner: &ShellMenuOwner, flags: windows::Win32::UI::Shell::FILEOPERATION_FLAGS) -> Result<IFileOperation, String>
{
    let operation: IFileOperation = unsafe { CoCreateInstance(&FileOperation, None, CLSCTX_ALL) }.map_err(|error| format!("创建 IFileOperation 失败: {error}"))?; // 当前用户操作独占的系统文件操作对象。
    unsafe { operation.SetOwnerWindow(owner.hwnd) }.map_err(|error| format!("设置 Shell 操作 owner 失败: {error}"))?;
    unsafe { operation.SetOperationFlags(flags) }.map_err(|error| format!("设置 Shell 操作属性失败: {error}"))?;
    Ok(operation)
}

/// 创建 IFileOperation 进度回调和共享结果集合。
fn create_operation_sink() -> (IFileOperationProgressSink, Arc<Mutex<Vec<ShellOperationItemResult>>>) { create_operation_sink_with_proof(None) }

/// 使用同一结果采集器为补偿操作加入根身份检查。
fn create_operation_sink_with_proof(expected: Option<super::file_proof::FileProof>) -> (IFileOperationProgressSink, Arc<Mutex<Vec<ShellOperationItemResult>>>)
{
    let results = Arc::new(Mutex::new(Vec::new())); // COM 回调与命令返回共享的项目结果集合。
    let sink: IFileOperationProgressSink = FileOperationProgressSink { results: Arc::clone(&results), pending_sources: Mutex::new(Vec::new()), expected }.into(); // 当前 IFileOperation 使用的实际结果采集器。
    (sink, results)
}

/// 执行已经排队的 IFileOperation 并生成统一公开结果。
fn perform_file_operation(operation: &IFileOperation, kind: ShellOperationKind, reversible: bool, permanent: bool, effect: Option<DROPEFFECT>, results: Arc<Mutex<Vec<ShellOperationItemResult>>>) -> Result<ShellOperationResult, String>
{
    let completion = unsafe { operation.PerformOperations() }; // 整体失败不丢弃此前已完成的项目事实。
    let cancellation = unsafe { operation.GetAnyOperationsAborted() }; // 单独读取用户取消状态。
    let mut items = results.lock().map_err(|_| "读取 Shell 项目操作结果失败".to_string())?.clone(); // 每个项目由进度回调确认的实际结果。
    let observed = items.clone(); // 先保留后代失败事实，再移除非顶层回调。
    let complete_parent = |parent: &ShellOperationItemResult| parent.succeeded && !observed.iter().any(|child| child.source_reference != parent.source_reference && !child.succeeded && std::path::Path::new(&child.source_reference).starts_with(&parent.source_reference)); // 部分目录保留实际成功子树供补偿重做。
    items.retain(|item| !observed.iter().any(|parent| complete_parent(parent) && parent.source_reference != item.source_reference && std::path::Path::new(&parent.source_reference).is_absolute() && std::path::Path::new(&item.source_reference).starts_with(&parent.source_reference)));
    for item in &mut items
    {
        let descendant_failed = observed.iter().any(|child| child.source_reference != item.source_reference && !child.succeeded && std::path::Path::new(&child.source_reference).starts_with(&item.source_reference)); // 根成功不能掩盖目录后代的跳过或失败。
        if descendant_failed
        {
            item.succeeded = false;
            item.compensation_blocked = Some("目录仅部分完成，不能自动恢复整个目标".into());
        }
        if let Some(reference) = &item.result_reference
            && item.compensation_blocked.is_none()
        {
            match shell_file_proof(reference)
            {
                Ok(proof) =>
                {
                    if !descendant_failed && item.result_code == windows::Win32::UI::Shell::COPYENGINE_S_DONT_PROCESS_CHILDREN.0 && !std::path::Path::new(&item.source_reference).exists()
                    {
                        item.succeeded = true;
                    }
                    if item.succeeded
                    {
                        item.proof = Some(proof);
                    }
                }
                Err(error) => item.compensation_blocked = Some(format!("无法确认结果安全状态：{error}")),
            }
        }
    }
    let completed_roots = items.iter().filter(|item| item.succeeded).map(|item| item.source_reference.clone()).collect::<Vec<_>>(); // 原子目录移动确认后移除其重复后代。
    items.retain(|item| !completed_roots.iter().any(|parent| parent != &item.source_reference && std::path::Path::new(parent).is_absolute() && std::path::Path::new(&item.source_reference).starts_with(parent)));
    Ok(completed_file_operation(kind, reversible, permanent, effect, completion, cancellation, items))
}

/// Shell 引用先解析为真实路径，再采集文件树的安全补偿依据。
fn shell_file_proof(reference: &str) -> Result<super::file_proof::FileProof, String>
{
    let item = resolve_shell_item(reference)?; // 不将 PIDL 文本当作磁盘路径。
    let path = shell_item_name(&item, SIGDN_FILESYSPATH)?; // 当前真实结果路径。
    super::file_proof::FileProof::capture(std::path::Path::new(&path))
}

/// 汇总原生完成状态与逐项事实，保持部分成功在整体错误后仍可观察。
fn completed_file_operation(kind: ShellOperationKind, reversible: bool, permanent: bool, effect: Option<DROPEFFECT>, completion: windows::core::Result<()>, cancellation: windows::core::Result<BOOL>, items: Vec<ShellOperationItemResult>) -> ShellOperationResult
{
    let operation_error = completion.as_ref().err().or_else(|| cancellation.as_ref().err()).map(|error| error.code().0); // 优先保留执行失败的原生错误码。
    let cancel_code = |code: i32| [windows::Win32::UI::Shell::COPYENGINE_E_USER_CANCELLED.0, windows::Win32::UI::Shell::COPYENGINE_E_CANCELLED.0, windows::Win32::Foundation::E_ABORT.0, HRESULT::from_win32(windows::Win32::Foundation::ERROR_CANCELLED.0).0].contains(&code); // 只有明确取消码或没有其他失败事实的中断才解释为用户取消。
    let aborted = operation_error.is_some_and(cancel_code) || items.iter().any(|item| cancel_code(item.result_code)) || (cancellation.as_ref().is_ok_and(|value| value.as_bool()) && operation_error.is_none() && items.iter().all(|item| item.result_code >= 0)); // 明确取消码不依赖第二次查询；普通文件失败不抑制已有项目布局。
    let succeeded = items.iter().filter(|item| item.succeeded).count(); // Shell 确认成功的项目数量。
    let failed = items.len().saturating_sub(succeeded); // Shell 回报失败的项目数量。
    let operation_name = match kind
    {
        ShellOperationKind::Rename => "重命名",
        ShellOperationKind::Recycle => "移入回收站",
        ShellOperationKind::PermanentDelete => "永久删除",
        ShellOperationKind::Paste => "粘贴",
        ShellOperationKind::Copy | ShellOperationKind::Cut | ShellOperationKind::Drag => "Shell 操作",
    }; // 用户可见的操作名称。
    let message = if let Some(error) = operation_error
    {
        format!("{operation_name}未全部完成：已确认成功 {succeeded} 项，失败 {failed} 项，错误：{error:#010x}")
    }
    else if aborted && succeeded == 0
    {
        format!("{operation_name}已取消")
    }
    else if failed > 0
    {
        format!("{operation_name}完成：成功 {succeeded} 项，失败 {failed} 项")
    }
    else if items.is_empty()
    {
        format!("{operation_name}未收到逐项完成反馈，文件状态待核对")
    }
    else
    {
        format!("{operation_name}完成：{succeeded} 项")
    }; // 按实际成功、失败和取消状态生成的用户说明。
    let reversible = reversible && items.iter().any(|item| item.succeeded && item.proof.is_some()); // Shell 请求撤销记录不等于本应用拥有安全补偿能力。
    ShellOperationResult { operation_id: 0, completed_at_unix_micros: 0, kind, aborted, operation_error, reversible, permanent, effect: effect.map(|value| drop_effect_name(value).to_string()), items, message }
}

/// 读取 IShellItem 的桌面绝对解析名称并释放 Shell 内存。
fn shell_item_reference(item: &IShellItem) -> Result<String, String> { shell_item_name(item, SIGDN_DESKTOPABSOLUTEPARSING) }

/// 读取补偿日志需要的项目稳定引用、父级引用和编辑名称。
fn shell_item_metadata(item: &IShellItem) -> ShellItemMetadata
{
    ShellItemMetadata {
        compensation_blocked: None,
        reference: shell_item_reference(item).unwrap_or_default(),
        parent_reference: unsafe { item.GetParent() }.ok().and_then(|parent| shell_item_reference(&parent).ok()),
        name: shell_item_name(item, SIGDN_FILESYSPATH).ok().and_then(|path| std::path::Path::new(&path).file_name().map(|name| name.to_string_lossy().into_owned())).or_else(|| shell_item_name(item, SIGDN_PARENTRELATIVEEDITING).ok()), // 文件系统历史保存完整文件名，虚拟项目沿用 Shell 编辑名。
    }
}

/// 读取 IShellItem 指定语义的名称并释放 Shell 内存。
fn shell_item_name(item: &IShellItem, kind: SIGDN) -> Result<String, String>
{
    let name = unsafe { item.GetDisplayName(kind) }.map_err(|error| format!("读取 Shell 项目名称失败: {error}"))?; // Shell 分配的指定语义名称。
    let reference = unsafe { name.to_string() }.map_err(|error| format!("转换 Shell 项目名称失败: {error}")); // 已转换的 UTF-8 名称。
    unsafe { CoTaskMemFree(Some(name.0.cast())) };
    reference
}

/// 将复制或剪切首选效果写入 Shell IDataObject。
pub(crate) fn set_drop_effect(data_object: &IDataObject, format_name: PCWSTR, effect: DROPEFFECT) -> Result<(), String>
{
    let format_id = unsafe { RegisterClipboardFormatW(format_name) }; // 指定的 Shell 效果格式。
    if format_id == 0
    {
        return Err("注册 Shell 首选投放效果格式失败".to_string());
    }
    let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, size_of::<u32>()) }.map_err(|error| format!("分配 Shell 剪贴板效果内存失败: {error}"))?; // IDataObject 接管的可移动全局内存。
    let value = unsafe { GlobalLock(memory) }.cast::<u32>(); // 写入首选效果的 DWORD 地址。
    if value.is_null()
    {
        let mut medium = STGMEDIUM { tymed: TYMED_HGLOBAL.0 as u32, u: STGMEDIUM_0 { hGlobal: memory }, ..Default::default() }; // 用于释放失败分配的存储介质。
        unsafe { ReleaseStgMedium(&mut medium) };
        return Err("锁定 Shell 剪贴板效果内存失败".to_string());
    }
    unsafe { value.write(effect.0) };
    let _ = unsafe { GlobalUnlock(memory) };
    let format = FORMATETC { cfFormat: format_id as u16, ptd: null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 }; // Shell 首选投放效果的数据格式描述。
    let mut medium = STGMEDIUM { tymed: TYMED_HGLOBAL.0 as u32, u: STGMEDIUM_0 { hGlobal: memory }, ..Default::default() }; // 将所有权转交给 IDataObject 的效果存储介质。
    if let Err(error) = unsafe { data_object.SetData(&format, &medium, true) }
    {
        unsafe { ReleaseStgMedium(&mut medium) };
        return Err(format!("写入 Shell 首选投放效果失败: {error}"));
    }
    Ok(())
}

/// 从 Shell IDataObject 读取指定效果格式，并验证外部内存大小。
fn read_drop_effect(data_object: &IDataObject, format_name: PCWSTR) -> Option<DROPEFFECT>
{
    let format_id = unsafe { RegisterClipboardFormatW(format_name) }; // 指定投放效果剪贴板格式。
    if format_id == 0
    {
        return None;
    }
    let format = FORMATETC { cfFormat: format_id as u16, ptd: null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 }; // 请求首选投放效果的格式描述。
    let mut medium = unsafe { data_object.GetData(&format) }.ok()?; // IDataObject 返回的效果存储介质。
    if medium.tymed != TYMED_HGLOBAL.0 as u32 || unsafe { GlobalSize(medium.u.hGlobal) } < size_of::<u32>()
    {
        unsafe { ReleaseStgMedium(&mut medium) };
        return None;
    }
    let memory = unsafe { medium.u.hGlobal }; // 存放 DROPEFFECT DWORD 的全局内存。
    let value = unsafe { GlobalLock(memory) }.cast::<u32>(); // 只读首选效果 DWORD 地址。
    let effect = (!value.is_null()).then(|| DROPEFFECT(unsafe { value.read() })); // 已读取的可选系统投放效果。
    if !value.is_null()
    {
        let _ = unsafe { GlobalUnlock(memory) };
    }
    unsafe { ReleaseStgMedium(&mut medium) };
    effect
}

/// 将系统 DROPEFFECT 转换为用户可见的稳定名称。
fn drop_effect_name(effect: DROPEFFECT) -> &'static str
{
    if (effect & DROPEFFECT_MOVE).0 != 0
    {
        "移动"
    }
    else if (effect & DROPEFFECT_COPY).0 != 0
    {
        "复制"
    }
    else if (effect & DROPEFFECT_LINK).0 != 0
    {
        "创建链接"
    }
    else
    {
        "无"
    }
}

/// 初始化专用 Shell 线程的 STA COM apartment。
pub fn initialize_shell_apartment() -> Result<(), String> { unsafe { OleInitialize(None) }.map_err(|error| format!("初始化 Shell STA/OLE 失败: {error}")) }

/// 释放专用 Shell 线程的 COM apartment。
pub fn uninitialize_shell_apartment() { unsafe { OleUninitialize() }; }

/// 仅读取两个真实桌面目录的直接子项及显式开启的五类系统对象。
pub fn enumerate_desktop_items() -> Result<Vec<DesktopItem>, String>
{
    let visible = visible_icons()?; // 先完整校验配置，读取失败不发布部分结果。
    let mut references = Vec::new(); // 可重新解析的真实目录子项及系统对象引用。
    let mut roots = HashSet::new(); // 重定向到相同目录时只读取一次。
    for folder in [FOLDERID_Desktop, FOLDERID_PublicDesktop]
    {
        let raw = unsafe { SHGetKnownFolderPath(&folder, KF_FLAG_DEFAULT, None) }.map_err(|error| format!("读取实际桌面目录失败：{error}"))?; // Known Folder 返回当前重定向路径。
        let path = unsafe { raw.to_string() }.map_err(|error| format!("转换桌面目录路径失败：{error}")); // 在释放 COM 字符串前复制路径。
        unsafe { CoTaskMemFree(Some(raw.0.cast())) };
        let path = path?; // 完整 Unicode 桌面目录。
        if !roots.insert(path.to_lowercase())
        {
            continue;
        }
        let entries = std::fs::read_dir(&path).map_err(|error| format!("读取桌面目录失败，路径：{path}，错误：{error}"))?; // 不递归，不展开快捷方式目标。
        for entry in entries
        {
            let entry = entry.map_err(|error| format!("读取桌面子项失败，路径：{path}，错误：{error}"))?; // 当前实际目录的直接子项。
            let metadata = entry.metadata().map_err(|error| format!("读取桌面子项属性失败，路径：{}，错误：{error}", entry.path().display()))?; // Windows 原始隐藏属性。
            if metadata.file_attributes() & 0x2 != 0
            {
                continue;
            }
            let reference = entry.path().into_os_string().into_string().map_err(|_| "桌面项目路径包含无效 Unicode".to_string())?; // 保持原始文件与快捷方式身份。
            references.push(reference);
        }
    }
    for ((folder, _), enabled) in SYSTEM_ICONS.iter().zip(visible)
    {
        if !enabled
        {
            continue;
        }
        let pidl = OwnedPidl(unsafe { SHGetKnownFolderIDList(folder, 0, None) }.map_err(|error| format!("读取已开启系统图标 PIDL 失败：{error}"))?); // 独立获取系统对象，不枚举桌面根。
        references.push(shell_name(pidl.0, SIGDN_DESKTOPABSOLUTEPARSING).unwrap_or_else(|_| encoded_pidl_reference(pidl.0)));
    }
    // 本次枚举观察到的完整项目集合。
    let mut items = Vec::new();
    let mut identities = HashSet::new(); // 同一稳定身份去重，同名不同路径仍保留。
    for reference in references
    {
        let resolved = parse_desktop_item_reference(&reference); // 元数据解析失败仍保留真实目录项目。
        let stable_reference = resolved.as_ref().ok().and_then(|pidl| shell_name(pidl.0, SIGDN_DESKTOPABSOLUTEPARSING).ok()).unwrap_or(reference); // 复用原来的 Shell 稳定名称。
        if !identities.insert(stable_reference.to_lowercase())
        {
            continue;
        }
        // 从稳定引用重新解析出的 PIDL。
        let resolved_pidl = parse_desktop_item_reference(&stable_reference);
        // 当前项目的系统显示名称读取结果。
        let display_name = match &resolved_pidl
        {
            Ok(resolved) => shell_name(resolved.0, SIGDN_NORMALDISPLAY),
            Err(error) => Err(error.clone()),
        };
        // 当前项目的系统图像读取结果。
        let system_image = resolved_pidl.as_ref().ok().and_then(|resolved| system_image_data_url(resolved.0));
        // 当前项目元数据可解析状态。
        let availability = if display_name.is_ok() && system_image.is_some()
        {
            DesktopItemAvailability::Available
        }
        else
        {
            DesktopItemAvailability::Unavailable
        };
        // 元数据失败时仍可呈现并恢复的项目名称。
        let display_name = display_name.unwrap_or_else(|_| stable_reference.clone());

        items.push(DesktopItem { stable_reference, display_name, system_image, availability });
    }

    Ok(items)
}

/// 读取 PIDL 对应的 Shell 名称。
fn shell_name(pidl: *const ITEMIDLIST, kind: SIGDN) -> Result<String, String>
{
    // Shell 分配的名称缓冲区。
    let name_pointer = unsafe { SHGetNameFromIDList(pidl, kind) }.map_err(|error| format!("读取 Shell 项目名称失败: {error}"))?;
    // 已转换的 UTF-8 Shell 名称。
    let name = unsafe { name_pointer.to_string() }.map_err(|error| format!("转换 Shell 项目名称失败: {error}"));
    unsafe { CoTaskMemFree(Some(name_pointer.0.cast())) };
    name
}

/// 将 PIDL 编码为可重新提交给 Shell 的稳定兜底引用。
fn encoded_pidl_reference(pidl: *const ITEMIDLIST) -> String
{
    // Shell PIDL 的完整字节长度。
    let byte_length = unsafe { ILGetSize(Some(pidl)) } as usize;
    // 当前 PIDL 的二进制内容。
    let bytes = unsafe { std::slice::from_raw_parts(pidl.cast::<u8>(), byte_length) };
    format!("pidl:{}", STANDARD.encode(bytes))
}

/// 将稳定引用重新解析为 Shell PIDL。
fn parse_desktop_item_reference(reference: &str) -> Result<OwnedPidl, String>
{
    if let Some(encoded) = reference.strip_prefix("pidl:")
    {
        // 从稳定引用解码并校验的 PIDL 字节。
        let bytes = decode_pidl_bytes(encoded)?;
        // COM 任务分配器提供的 PIDL 内存。
        let pidl = unsafe { CoTaskMemAlloc(bytes.len()) }.cast::<ITEMIDLIST>();
        if pidl.is_null()
        {
            return Err("分配 Shell PIDL 内存失败".to_string());
        }
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), pidl.cast::<u8>(), bytes.len()) };
        return Ok(OwnedPidl(pidl));
    }

    // 传给 Shell 的稳定解析名称。
    let display_name = HSTRING::from(reference);
    // Shell 分配的解析结果 PIDL。
    let mut pidl = null_mut();
    unsafe { SHParseDisplayName(&display_name, None, &mut pidl, 0, None) }.map_err(|error| format!("重新解析 Shell 项目引用失败: {error}"))?;
    if pidl.is_null()
    {
        return Err("Shell 项目引用解析结果为空".to_string());
    }

    Ok(OwnedPidl(pidl))
}

/// 解码并校验序列化 PIDL 的项目边界与终止项。
fn decode_pidl_bytes(encoded: &str) -> Result<Vec<u8>, String>
{
    // Base64 解码后的 PIDL 字节。
    let bytes = STANDARD.decode(encoded).map_err(|error| format!("解码 Shell PIDL 引用失败: {error}"))?;
    // 当前 SHITEMID 的起始偏移。
    let mut offset = 0;

    loop
    {
        if offset + 2 > bytes.len()
        {
            return Err("Shell PIDL 引用缺少终止项".to_string());
        }

        // 当前 SHITEMID 的完整字节长度。
        let item_length = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as usize;
        if item_length == 0
        {
            if offset + 2 == bytes.len()
            {
                return Ok(bytes);
            }
            return Err("Shell PIDL 引用包含终止项后的数据".to_string());
        }
        if item_length < 2 || offset + item_length > bytes.len()
        {
            return Err("Shell PIDL 引用包含无效项目长度".to_string());
        }

        offset += item_length;
    }
}

/// 读取 PIDL 对应的 48 像素 Shell 系统图像。
fn system_image_data_url(pidl: *const ITEMIDLIST) -> Option<String>
{
    // Shell 系统图像索引结果。
    let mut file_info = SHFILEINFOW::default();
    // PIDL 查询结果标志。
    let query_result = unsafe { SHGetFileInfoW(windows::core::PCWSTR(pidl.cast::<u16>()), FILE_FLAGS_AND_ATTRIBUTES(0), Some(&mut file_info), size_of::<SHFILEINFOW>() as u32, SHGFI_PIDL | SHGFI_SYSICONINDEX) };
    if query_result == 0
    {
        return None;
    }

    // 与 Explorer 桌面大图标模式一致的 HICON。
    let icon = unsafe { SHGetImageList::<IImageList>(SHIL_EXTRALARGE as i32).ok()?.GetIcon(file_info.iIcon, ILD_TRANSPARENT.0).ok()? };
    // 供浏览器直接呈现的 PNG data URL。
    let encoded = unsafe { icon_data_url(icon) };
    unsafe {
        let _ = DestroyIcon(icon);
    }
    encoded
}

/// 将 Shell HICON 编码为浏览器可直接渲染的 PNG data URL。
unsafe fn icon_data_url(icon: HICON) -> Option<String>
{
    /// Desktop Tools 桌面项目图标边长。
    const ICON_SIZE: i32 = 48;
    // 用于绘制图标的内存设备上下文。
    let device_context = unsafe { CreateCompatibleDC(None) };
    if device_context.is_invalid()
    {
        return None;
    }

    // 自顶向下的 32 位 BGRA DIB 描述。
    let bitmap_info = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: ICON_SIZE, biHeight: -ICON_SIZE, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() }, ..Default::default() };
    // DIB 像素内存首地址。
    let mut bits: *mut c_void = null_mut();
    // 承载系统图标的 DIB 句柄。
    let bitmap = unsafe {
        match CreateDIBSection(Some(device_context), &bitmap_info, DIB_RGB_COLORS, &mut bits, None, 0)
        {
            Ok(value) => value,
            Err(_) =>
            {
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
    let draw_result = unsafe { DrawIconEx(device_context, 0, 0, icon, ICON_SIZE, ICON_SIZE, 0, None, DI_NORMAL) };
    // 从 DIB 复制出的 BGRA 数据。
    let bgra = unsafe { std::slice::from_raw_parts(bits.cast::<u8>(), byte_length) };
    // 当前图标是否已经携带有效 alpha。
    let has_alpha = bgra.chunks_exact(4).any(|pixel| pixel[3] != 0);
    // PNG 编码器需要的 RGBA 数据。
    let mut rgba = Vec::with_capacity(byte_length);
    for pixel in bgra.chunks_exact(4)
    {
        // 旧式 HICON 缺少 alpha 时按非黑像素补全不透明度。
        let alpha = if has_alpha
        {
            pixel[3]
        }
        else if pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0
        {
            255
        }
        else
        {
            0
        };
        let mut color = [pixel[2], pixel[1], pixel[0], alpha]; // PNG 使用直通 Alpha 的 RGBA 像素。
        // GDI 绘制结果已预乘 Alpha，编码前还原颜色，避免半透明边缘被重复压暗。
        if has_alpha && alpha > 0 && alpha < 255
        {
            for channel in &mut color[..3]
            {
                *channel = ((u32::from(*channel) * 255 + u32::from(alpha) / 2) / u32::from(alpha)).min(255) as u8;
            }
        }
        rgba.extend_from_slice(&color);
    }

    unsafe {
        SelectObject(device_context, previous_bitmap);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(device_context);
    }
    if draw_result.is_err()
    {
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

    Some(format!("data:image/png;base64,{}", STANDARD.encode(encoded)))
}

#[cfg(test)]
#[path = "shell.test.rs"]
mod tests;
