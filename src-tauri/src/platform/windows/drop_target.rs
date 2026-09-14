use std::cell::RefCell;
use std::rc::Rc;

use windows::Win32::Foundation::{E_INVALIDARG, POINTL};
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::Ole::{DROPEFFECT, DROPEFFECT_NONE, IDropTarget, IDropTarget_Impl};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::core::{Ref, implement};

/// 原生已登记会话的最新渲染确认，不允许页面创建拖放会话。
static DROP_PREVIEWS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, Option<DropPreview>>>> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// 当前已渲染落点的版本、几何和滚动事实，释放时由原生重新命中。
#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DropPreview
{
    pub geometry_revision: u64,                                     // 同一指针下滚动与悬停展开的渲染帧版本。
    pub session: String,                                            // 原生登记的目标会话。
    pub sequence: u64,                                              // 指针或布局改变时递增的原生序号。
    pub revision: u64,                                              // Renderer 实际使用的布局版本。
    pub monitor_id: String,                                         // 几何所属显示器。
    pub pointer: crate::contracts::WorkspacePoint,                  // 已渲染的工作区 DIP 指针。
    pub regions: Vec<crate::contracts::HitRegion>,                  // 裁剪后的完整可见语义图。
    pub fence_scrolls: Vec<crate::contracts::WorkspaceFenceScroll>, // 自动滚动的最终位置。
}

/// 注册或清除一个原生目标的预览确认槽位。
pub(crate) fn set_preview_session(session: &str, active: bool)
{
    if let Ok(mut previews) = DROP_PREVIEWS.lock()
    {
        if active
        {
            previews.insert(session.to_string(), None);
        }
        else
        {
            previews.remove(session);
        }
    }
}

/// 接受页面异步确认；未知会话和过期帧不能覆盖正在使用的落点。
pub(crate) fn publish_preview(preview: DropPreview) -> Result<(), String>
{
    if !preview.pointer.left.is_finite() || !preview.pointer.top.is_finite() || preview.regions.len() > 4096 || preview.fence_scrolls.iter().any(|scroll| !scroll.scroll_top.is_finite() || scroll.scroll_top < 0.0)
    {
        return Err("拖放预览数据无效".into());
    }
    let mut previews = DROP_PREVIEWS.lock().map_err(|error| error.to_string())?; // 只保存已登记目标的确认。
    if let Some(current) = previews.get_mut(&preview.session)
    {
        if current.as_ref().is_none_or(|current| (current.sequence, current.geometry_revision) <= (preview.sequence, preview.geometry_revision))
        {
            *current = Some(preview);
        }
    }
    Ok(())
}

/// 释放不等待 Renderer；缺少匹配当前落点和布局的确认就拒绝。
pub(crate) fn confirmed_preview(session: &str, sequence: u64, revision: u64, monitor: &crate::contracts::DesktopHostMonitor, point: &POINTL) -> Option<DropPreview>
{
    let preview = DROP_PREVIEWS.lock().ok()?.get(session)?.clone()?; // 固定释放时的完整几何快照。
    let x = (f64::from(point.x) - f64::from(monitor.work_x)) / monitor.scale_factor; // 当前实际释放位置。
    let y = (f64::from(point.y) - f64::from(monitor.work_y)) / monitor.scale_factor;
    if preview.sequence != sequence || preview.revision != revision || preview.monitor_id != monitor.id || (preview.pointer.left - x).abs() > 0.01 || (preview.pointer.top - y).abs() > 0.01
    {
        return None;
    }
    crate::host::validate_hit_regions(&preview.regions, f64::from(monitor.work_width) / monitor.scale_factor, f64::from(monitor.work_height) / monitor.scale_factor).ok()?;
    Some(preview)
}

/// 当前源会话由进程登记，不从外部数据格式或项目路径推断内部身份。
static SOURCE_DRAG: std::sync::Mutex<Option<std::sync::Arc<SourceDragSession>>> = std::sync::Mutex::new(None);

/// 保持完整 Shell 数据对象身份及内部布局完成事实。
pub(crate) struct SourceDragSession
{
    pub pointer_offset: crate::contracts::WorkspacePoint, // 指针相对锚点表面的 DIP 偏移，跨屏保持。
    data: windows::core::AgileReference<IDataObject>,     // 在目标 apartment 重新解析真实 COM 身份。
    pub item_ids: Vec<String>,                            // 源端已经校验的完整选择，顺序以锚点开头。
    pub layout_committed: std::sync::atomic::AtomicBool,  // 内部完成不通过文件移动效果反馈。
}

/// 源端退出 OLE 时释放登记，包含原生错误和取消路径。
pub(crate) struct SourceDragGuard(std::sync::Arc<SourceDragSession>);

impl SourceDragGuard
{
    /// 开始唯一源会话；登记失败时不能启动无法识别的内部回落。
    pub(crate) fn register(data: &IDataObject, item_ids: &[String], pointer_offset: crate::contracts::WorkspacePoint) -> windows::core::Result<Self>
    {
        if !pointer_offset.left.is_finite() || !pointer_offset.top.is_finite() || pointer_offset.left < 0.0 || pointer_offset.top < 0.0
        {
            return Err(E_INVALIDARG.into());
        }
        if item_ids.is_empty() || item_ids.iter().any(|id| id.is_empty()) || item_ids.iter().collect::<std::collections::HashSet<_>>().len() != item_ids.len()
        {
            return Err(E_INVALIDARG.into());
        }
        let data = windows::core::AgileReference::new(data)?; // 合法跨 apartment 封送。
        let mut current = SOURCE_DRAG.lock().map_err(|_| windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL))?; // 只保护登记，不持锁调用 COM。
        if current.is_some()
        {
            return Err(windows::Win32::Foundation::E_UNEXPECTED.into());
        }
        let session = std::sync::Arc::new(SourceDragSession { data, pointer_offset, item_ids: item_ids.to_vec(), layout_committed: std::sync::atomic::AtomicBool::new(false) }); // 完整数据对象在本次 OLE 会话内有效。
        *current = Some(session.clone());
        Ok(Self(session))
    }

    /// 获取内部布局是否实际提交，独立于 DoDragDrop 的效果。
    pub(crate) fn layout_committed(&self) -> bool { self.0.layout_committed.load(std::sync::atomic::Ordering::Acquire) }
}

impl Drop for SourceDragGuard
{
    /// 源 OLE 结束即使会话失效，后续外部对象不能重放该会话。
    fn drop(&mut self)
    {
        if let Ok(mut current) = SOURCE_DRAG.lock()
        {
            if current.as_ref().is_some_and(|session| std::sync::Arc::ptr_eq(session, &self.0))
            {
                current.take();
            }
        }
    }
}

/// 只接受与当前源登记相同的 COM 对象，内容相同的外部对象仍是外部来源。
pub(crate) fn source_drag_session(data: &IDataObject) -> Option<std::sync::Arc<SourceDragSession>>
{
    use windows::core::Interface;
    let session = SOURCE_DRAG.lock().ok()?.clone()?; // COM 解析之前释放登记锁，避免重入死锁。
    let expected = session.data.resolve().ok()?.cast::<windows::core::IUnknown>().ok()?; // 目标 apartment 中的规范 IUnknown。
    let actual = data.cast::<windows::core::IUnknown>().ok()?; // OLE 回调实际给出的规范身份。
    (expected == actual).then_some(session)
}

/// 在宿主 OLE 线程等待 Shell STA 结果，同时持续处理窗口和 COM 消息。
pub(crate) fn wait_transfer(result: std::sync::mpsc::Receiver<Result<crate::contracts::ShellOperationResult, String>>) -> Result<crate::contracts::ShellOperationResult, String>
{
    use std::sync::mpsc::TryRecvError;
    use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, MSG, MsgWaitForMultipleObjects, PM_REMOVE, PeekMessageW, PostQuitMessage, QS_ALLINPUT, TranslateMessage, WM_QUIT};
    let mut quit = None; // 文件执行完成前保留退出请求，不能提前声明失败并丢掉实际结果。
    let completed = loop
    {
        match result.try_recv()
        {
            Ok(completed) => break completed,
            Err(TryRecvError::Disconnected) => break Err("Shell 投放结果通道已经关闭".to_string()),
            Err(TryRecvError::Empty) =>
            {}
        }
        let mut message = MSG::default(); // 当前宿主线程消息。
        unsafe {
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool()
            {
                if message.message == WM_QUIT
                {
                    quit = Some(message.wParam.0 as i32);
                    continue;
                }
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            MsgWaitForMultipleObjects(None, false, 15, QS_ALLINPUT);
        }
    };
    if let Some(code) = quit
    {
        unsafe { PostQuitMessage(code) };
    }
    completed
}

/// 宿主提供的原生拖放阶段，屏幕坐标不经 WebView 同步查询。
pub enum DropPhase<'a>
{
    Enter(&'a IDataObject, MODIFIERKEYS_FLAGS, &'a POINTL, DROPEFFECT), // 检查源格式并创建会话。
    Over(MODIFIERKEYS_FLAGS, &'a POINTL, DROPEFFECT),                   // 根据最新快照更新投放效果与预览。
    Leave,                                                              // 清理原生会话与前端预览。
    Drop(&'a IDataObject, MODIFIERKEYS_FLAGS, &'a POINTL, DROPEFFECT),  // 按释放点执行投放并反馈实际效果。
}

/// 仅在注册窗口所属 OLE 线程调用的投放处理函数。
pub type DropHandler = Rc<dyn for<'a> Fn(DropPhase<'a>) -> windows::core::Result<DROPEFFECT>>;

/// 只接受 Shell 能解析为现有文件系统项目的完整集合，虚拟文件不降级猜测路径。
pub(crate) fn filesystem_drop(data: &IDataObject) -> bool { filesystem_drop_items(data).is_some() }

/// 显式修饰键不能静默降级；内部布局的 Alt 不参与文件链接协商。
pub(crate) fn negotiated_effect(allowed: DROPEFFECT, keys: MODIFIERKEYS_FLAGS, preferred: DROPEFFECT, internal: bool) -> DROPEFFECT
{
    use windows::Win32::System::Ole::{DROPEFFECT_COPY, DROPEFFECT_MOVE, MK_ALT};
    use windows::Win32::System::SystemServices::{MK_CONTROL, MK_RBUTTON, MK_SHIFT};
    if keys.0 & MK_RBUTTON.0 != 0
    {
        return DROPEFFECT_NONE;
    }
    if internal
    {
        return if (allowed & DROPEFFECT_MOVE).0 != 0 { DROPEFFECT_MOVE } else { allowed & DROPEFFECT_COPY };
    }
    let control = keys.0 & MK_CONTROL.0 != 0;
    let shift = keys.0 & MK_SHIFT.0 != 0;
    if keys.0 & MK_ALT != 0 || (control && shift)
    {
        return DROPEFFECT_NONE;
    }
    if control
    {
        return allowed & DROPEFFECT_COPY;
    }
    if shift
    {
        return allowed & DROPEFFECT_MOVE;
    }
    if (allowed & preferred).0 != 0
    {
        return preferred;
    }
    if (allowed & DROPEFFECT_COPY).0 != 0 { DROPEFFECT_COPY } else { allowed & DROPEFFECT_MOVE }
}

/// 提取完整顶层来源身份，结果关联不使用目录变化通知或文件名猜测。
pub(crate) fn filesystem_drop_items(data: &IDataObject) -> Option<Vec<String>>
{
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{IShellItemArray, SHCreateShellItemArrayFromDataObject, SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_FILESYSPATH};
    let items: IShellItemArray = unsafe { SHCreateShellItemArrayFromDataObject(data) }.ok()?; // Shell 统一解析真实数据格式。
    let count = unsafe { items.GetCount() }.ok()?; // 顶层传输项目总数。
    if count == 0
    {
        return None;
    }
    let references = (0..count)
        .map(|index| {
            let item = unsafe { items.GetItemAt(index) }.ok()?; // 当前源项目。
            let path = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }.ok()?; // 排除虚拟文件。
            let value = unsafe { path.to_string() }; // 验证现有文件路径。
            unsafe { CoTaskMemFree(Some(path.0.cast())) };
            if std::fs::metadata(value.ok()?).is_err()
            {
                return None;
            }
            let reference = unsafe { item.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING) }.ok()?; // 与逐项回调相同的 Shell 解析名。
            let value = unsafe { reference.to_string() }; // 全局源身份。
            unsafe { CoTaskMemFree(Some(reference.0.cast())) };
            value.ok()
        })
        .collect::<Option<Vec<_>>>()?;
    let paths = references.iter().map(|reference| reference.trim_end_matches(['\\', '/']).to_lowercase()).collect::<Vec<_>>(); // 拒绝重复根或父子重叠，避免同一内容执行两次。
    if paths.iter().enumerate().any(|(index, path)| paths.iter().enumerate().any(|(other, parent)| index != other && (path == parent || path.starts_with(&format!("{parent}\\")))))
    {
        return None;
    }
    Some(references)
}

/// 管理系统拖放回调和会话生命周期，不承担文件或布局决策。
#[implement(IDropTarget)]
struct DesktopDropTarget
{
    window: windows::Win32::Foundation::HWND,                     // 原生拖动图像所属目标窗口。
    helper: Option<windows::Win32::UI::Shell::IDropTargetHelper>, // Shell 提供外部源的完整拖动图像。
    handler: DropHandler,                                         // 当前窗口所属的原生决策入口。
    active: RefCell<bool>,                                        // 防止 Leave 和 Drop 重复结束会话。
}

/// 创建由宿主线程持有并交给 RegisterDragDrop 的原生投放目标。
pub fn create_drop_target(window: Option<windows::Win32::Foundation::HWND>, handler: DropHandler) -> IDropTarget
{
    let helper = window.and_then(|_| unsafe { windows::Win32::System::Com::CoCreateInstance(&windows::Win32::UI::Shell::CLSID_DragDropHelper, None, windows::Win32::System::Com::CLSCTX_INPROC_SERVER) }.ok()); // 图像服务不可用不改变文件接收能力。
    DesktopDropTarget { window: window.unwrap_or_default(), helper, handler, active: RefCell::new(false) }.into()
}

#[allow(non_snake_case)]
impl IDropTarget_Impl for DesktopDropTarget_Impl
{
    /// 检查数据对象后开始会话，失败时确保系统收到拒绝效果。
    fn DragEnter(&self, data: Ref<'_, IDataObject>, keys: MODIFIERKEYS_FLAGS, point: &POINTL, effect: *mut DROPEFFECT) -> windows::core::Result<()>
    {
        if effect.is_null()
        {
            return Err(E_INVALIDARG.into());
        }
        let allowed = unsafe { *effect }; // 源允许的效果，回调前先将输出设为拒绝。
        unsafe { *effect = DROPEFFECT_NONE };
        self.DragLeave()?;
        let data = data.as_ref().ok_or_else(|| windows::core::Error::from_hresult(E_INVALIDARG))?; // Enter 必须提供有效数据对象。
        *self.active.borrow_mut() = true;
        match (self.handler)(DropPhase::Enter(data, keys, point, allowed))
        {
            Ok(selected) =>
            {
                unsafe { *effect = selected & allowed };
                if source_drag_session(data).is_none()
                    && let Some(helper) = &self.helper
                {
                    let _ = unsafe { helper.DragEnter(self.window, data, &windows::Win32::Foundation::POINT { x: point.x, y: point.y }, selected & allowed) };
                }
                Ok(())
            }
            Err(error) =>
            {
                let _ = self.DragLeave();
                Err(error)
            }
        }
    }

    /// 原生命中处理返回当前允许效果，不持有可重入借用。
    fn DragOver(&self, keys: MODIFIERKEYS_FLAGS, point: &POINTL, effect: *mut DROPEFFECT) -> windows::core::Result<()>
    {
        if effect.is_null()
        {
            return Err(E_INVALIDARG.into());
        }
        let allowed = unsafe { *effect }; // 系统提供的当前源效果集合。
        unsafe { *effect = DROPEFFECT_NONE };
        if !*self.active.borrow()
        {
            return Ok(());
        }
        let selected = (self.handler)(DropPhase::Over(keys, point, allowed))?; // 使用宿主最新命中状态。
        unsafe { *effect = selected & allowed };
        if let Some(helper) = &self.helper
        {
            let _ = unsafe { helper.DragOver(&windows::Win32::Foundation::POINT { x: point.x, y: point.y }, selected & allowed) };
        }
        Ok(())
    }

    /// 离开或取消时清理会话，重复调用不重复通知。
    fn DragLeave(&self) -> windows::core::Result<()>
    {
        let active = self.active.replace(false); // 回调前结束会话，允许安全重入。
        if active
        {
            if let Some(helper) = &self.helper
            {
                let _ = unsafe { helper.DragLeave() };
            }
            (self.handler)(DropPhase::Leave)?;
        }
        Ok(())
    }

    /// 投放结束无论成功或失败都清理一次，输出只来自实际处理结果。
    fn Drop(&self, data: Ref<'_, IDataObject>, keys: MODIFIERKEYS_FLAGS, point: &POINTL, effect: *mut DROPEFFECT) -> windows::core::Result<()>
    {
        if effect.is_null()
        {
            return Err(E_INVALIDARG.into());
        }
        let allowed = unsafe { *effect }; // 源端允许的释放效果。
        unsafe { *effect = DROPEFFECT_NONE };
        if !self.active.replace(false)
        {
            return Ok(());
        }
        if let Some(helper) = &self.helper
        {
            let _ = unsafe { helper.Show(false) };
        } // 文件操作等待期间移除浮动图像。
        let result = data.as_ref().ok_or_else(|| windows::core::Error::from_hresult(E_INVALIDARG)).and_then(|data| (self.handler)(DropPhase::Drop(data, keys, point, allowed))); // Drop 的数据对象由系统提供，不缓存过期接口。
        if let Some(helper) = &self.helper
            && let Some(data) = data.as_ref()
        {
            let _ = unsafe { helper.Drop(data, &windows::Win32::Foundation::POINT { x: point.x, y: point.y }, result.as_ref().copied().unwrap_or(DROPEFFECT_NONE)) };
        }
        let cleanup = (self.handler)(DropPhase::Leave); // 即使文件处理失败也释放所有临时状态。
        let selected = result?; // 文件执行错误优先于预览清理错误。
        unsafe { *effect = selected & allowed };
        if let Err(error) = cleanup
        {
            crate::log_warn!("拖放：投放已处理，清理预览失败：{error}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests
{
    use super::*;
    use windows::Win32::System::Ole::DROPEFFECT_COPY;

    /// 验证未知、过期、跨屏及错误坐标的渲染确认不能授权投放。
    #[test]
    fn preview_requires_live_session_and_matching_view()
    {
        let monitor = crate::contracts::DesktopHostMonitor { id: "preview-screen".into(), name: "preview-screen".into(), primary: false, x: -1000, y: 0, width: 1000, height: 800, work_x: -1000, work_y: 0, work_width: 1000, work_height: 800, scale_factor: 2.0, dpi: 192 }; // 负坐标与混合 DPI。
        let point = POINTL { x: -970, y: 60 }; // 对应 DIP (15, 30)。
        let preview = DropPreview { session: "preview-test".into(), sequence: 2, geometry_revision: 1, revision: 7, monitor_id: monitor.id.clone(), pointer: crate::contracts::WorkspacePoint { left: 15.0, top: 30.0 }, regions: vec![crate::contracts::HitRegion { id: "background".into(), kind: "workspace-background".into(), left: 0.0, top: 0.0, width: 500.0, height: 400.0, priority: 1, target_path: None }], fence_scrolls: Vec::new() }; // 已呈现的合法几何。
        publish_preview(preview.clone()).unwrap();
        assert!(confirmed_preview(&preview.session, 2, 7, &monitor, &point).is_none());
        set_preview_session(&preview.session, true);
        publish_preview(preview.clone()).unwrap();
        assert!(confirmed_preview(&preview.session, 2, 7, &monitor, &point).is_some());
        assert!(confirmed_preview(&preview.session, 3, 7, &monitor, &point).is_none());
        assert!(confirmed_preview(&preview.session, 2, 8, &monitor, &point).is_none());
        assert!(confirmed_preview(&preview.session, 2, 7, &monitor, &POINTL { x: -968, y: 60 }).is_none());
        let mut older = preview.clone(); // 迟到帧不能覆盖已确认的新序号。
        older.sequence = 1;
        publish_preview(older).unwrap();
        assert!(confirmed_preview(&preview.session, 2, 7, &monitor, &point).is_some());
        let mut invalid = preview.clone(); // 有效序号也不能授权越界几何。
        invalid.regions[0].left = -1.0;
        publish_preview(invalid).unwrap();
        assert!(confirmed_preview(&preview.session, 2, 7, &monitor, &point).is_none());
        set_preview_session(&preview.session, false);
        publish_preview(preview.clone()).unwrap();
        assert!(confirmed_preview(&preview.session, 2, 7, &monitor, &point).is_none());
    }

    /// 真实 COM 回调验证无 Enter 的释放被拒绝，重复 Leave 不创建虚假会话。
    #[test]
    fn inactive_target_rejects_drop_and_ignores_leave()
    {
        let calls = Rc::new(RefCell::new(0)); // 记录业务处理调用次数。
        let recorded = Rc::clone(&calls); // 回调持有的计数器。
        let target = create_drop_target(
            None,
            Rc::new(move |_| {
                *recorded.borrow_mut() += 1;
                Ok(DROPEFFECT_COPY)
            }),
        ); // 没有启动会话的真实 COM 对象。
        let mut effect = DROPEFFECT_COPY; // 模拟源允许复制。
        unsafe {
            target.DragOver(MODIFIERKEYS_FLAGS(0), POINTL::default(), &mut effect).unwrap();
            assert_eq!(effect, DROPEFFECT_NONE);
            target.DragLeave().unwrap();
            target.DragLeave().unwrap();
            target.Drop(None, MODIFIERKEYS_FLAGS(0), POINTL::default(), &mut effect).unwrap();
        }
        assert_eq!(*calls.borrow(), 0);
    }

    /// 真实数据对象验证效果协商、释放失败与一次性清理。
    #[test]
    fn failed_drop_cleans_active_session_once()
    {
        use windows::Win32::Foundation::E_FAIL;
        use windows::Win32::System::Ole::{DROPEFFECT_MOVE, OleInitialize, OleUninitialize};
        use windows::Win32::UI::Shell::{BHID_DataObject, IShellItem, SHCreateItemFromParsingName};
        unsafe { OleInitialize(None) }.unwrap();
        {
            let item: IShellItem = unsafe { SHCreateItemFromParsingName(&windows::core::HSTRING::from(std::env::temp_dir().to_string_lossy().as_ref()), None) }.unwrap(); // 只读外部目录的数据提供方。
            let data: IDataObject = unsafe { item.BindToHandler(None, &BHID_DataObject) }.unwrap(); // 真实 Shell 数据对象。
            assert!(filesystem_drop(&data));
            let pointer_offset = crate::contracts::WorkspacePoint { left: 12.0, top: 24.0 }; // 指针偏移同样跟随可信源会话。
            let session = SourceDragGuard::register(&data, &["source".to_string()], pointer_offset).unwrap(); // 真实源登记同时验证跨 STA 身份和内部反馈。
            assert!(SourceDragGuard::register(&data, &["second".to_string()], pointer_offset).is_err());
            let other_data: IDataObject = unsafe { item.BindToHandler(None, &BHID_DataObject) }.unwrap(); // 相同来源内容不能代替已登记 COM 身份。
            assert!(source_drag_session(&other_data).is_none());
            let agile = windows::core::AgileReference::new(&data).unwrap(); // 真实跨 apartment 代理。
            let (sender, receiver) = std::sync::mpsc::channel(); // 使用与正式宿主相同的消息泵等待。
            let worker = std::thread::spawn(move || {
                unsafe { OleInitialize(None) }.unwrap();
                {
                    let proxy = agile.resolve().unwrap(); // 只在目标 STA 使用解析出的接口。
                    assert!(filesystem_drop(&proxy));
                    let session = source_drag_session(&proxy).expect("合法跨 STA 代理应匹配源身份"); // 必须在当前 apartment 比较 IUnknown。
                    assert_eq!(session.item_ids, ["source"]);
                    assert_eq!(session.pointer_offset, pointer_offset);
                    session.layout_committed.store(true, std::sync::atomic::Ordering::Release);
                }
                drop(agile); // 完成通知前释放所有代理，主线程仍持续泵消息。
                unsafe { OleUninitialize() };
                sender.send(Err("封送已验证".to_string())).unwrap();
            }); // 不执行任何文件传输。
            assert_eq!(wait_transfer(receiver).unwrap_err(), "封送已验证");
            worker.join().unwrap();
            assert!(session.layout_committed());
            assert!(source_drag_session(&data).is_some());
            drop(session);
            assert!(source_drag_session(&data).is_none());
            let phases = Rc::new(RefCell::new(Vec::new())); // 验证每阶段只通知一次。
            let recorded = Rc::clone(&phases); // 回调共享的检查记录。
            let target = create_drop_target(
                None,
                Rc::new(move |phase| match phase
                {
                    DropPhase::Enter(data, keys, point, allowed) =>
                    {
                        assert!(!windows::core::Interface::as_raw(data).is_null());
                        assert_eq!(keys.0, 0);
                        assert_eq!(point.x, -100);
                        assert_eq!(allowed, DROPEFFECT_MOVE);
                        recorded.borrow_mut().push("enter");
                        Ok(DROPEFFECT_COPY)
                    }
                    DropPhase::Over(keys, point, allowed) =>
                    {
                        assert_eq!(keys.0, 0);
                        assert_eq!(point.x, -100);
                        recorded.borrow_mut().push("over");
                        Ok(allowed)
                    }
                    DropPhase::Drop(data, keys, point, allowed) =>
                    {
                        assert!(!windows::core::Interface::as_raw(data).is_null());
                        assert_eq!(keys.0, 0);
                        assert_eq!(point.x, -100);
                        assert_eq!(allowed, DROPEFFECT_MOVE);
                        recorded.borrow_mut().push("drop");
                        Err(E_FAIL.into())
                    }
                    DropPhase::Leave =>
                    {
                        recorded.borrow_mut().push("leave");
                        Ok(DROPEFFECT_NONE)
                    }
                }),
            ); // 业务失败模拟，不执行磁盘操作。
            let point = POINTL { x: -100, y: 0 }; // 保留负屏幕坐标。
            let mut effect = DROPEFFECT_MOVE; // 源不允许复制。
            unsafe {
                target.DragEnter(&data, MODIFIERKEYS_FLAGS(0), point, &mut effect).unwrap();
                assert_eq!(effect, DROPEFFECT_NONE);
                effect = DROPEFFECT_MOVE;
                target.DragOver(MODIFIERKEYS_FLAGS(0), point, &mut effect).unwrap();
                assert_eq!(effect, DROPEFFECT_MOVE);
                assert!(target.Drop(&data, MODIFIERKEYS_FLAGS(0), point, &mut effect).is_err());
                assert_eq!(effect, DROPEFFECT_NONE);
                target.DragLeave().unwrap();
            }
            assert_eq!(*phases.borrow(), ["enter", "over", "drop", "leave"]);
        }
        unsafe { OleUninitialize() };
    }
}
