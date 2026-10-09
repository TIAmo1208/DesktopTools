use std::cell::RefCell;
use std::rc::Rc;

use windows::Win32::Foundation::{E_INVALIDARG, POINTL};
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::Ole::{DROPEFFECT, DROPEFFECT_NONE, IDropTarget, IDropTarget_Impl};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::core::{Ref, implement};

/// 原生已登记会话的最新渲染确认，不允许页面创建拖放会话。
static DROP_PREVIEWS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, Option<DropPreview>>>> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

thread_local! {
    // 每个原生会话只在结束时输出统计，覆盖 OLE 未调用 Drop 的禁止放置路径。
    static PREVIEW_CHECKS: RefCell<std::collections::HashMap<String, [u64; 7]>> = RefCell::new(std::collections::HashMap::new());
}

/// 当前已渲染落点的版本、几何和滚动事实，释放时由原生重新命中。
#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DropPreview
{
    #[serde(default)]
    pub collect_items: Vec<CollectDropRegion>, // 已裁剪收集项，只有命中所属组件才参与分流。
    pub geometry_revision: u64,                                     // 同一指针下滚动与悬停展开的渲染帧版本。
    pub session: String,                                            // 原生登记的目标会话。
    pub sequence: u64,                                              // 指针或布局改变时递增的原生序号。
    pub revision: u64,                                              // Renderer 实际使用的布局版本。
    pub monitor_id: String,                                         // 几何所属显示器。
    pub pointer: crate::contracts::WorkspacePoint,                  // 已渲染的工作区 DIP 指针。
    pub regions: Vec<crate::contracts::HitRegion>,                  // 裁剪后的完整可见语义图。
    pub fence_scrolls: Vec<crate::contracts::WorkspaceFenceScroll>, // 自动滚动的最终位置。
}

/// 当前实际可见收集项的目标身份和有界几何。
#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CollectDropRegion
{
    pub instance_id: String, // 归属组件实例。
    pub name: String,        // 收集项相对文件名。
    #[serde(flatten)]
    pub bounds: crate::contracts::WorkspaceRect, // 内容视口内的可见区域。
}

/// 注册或清除一个原生目标的预览确认槽位。
pub(crate) fn set_preview_session(session: &str, active: bool)
{
    if let Ok(mut previews) = DROP_PREVIEWS.lock()
    {
        if active
        {
            previews.insert(session.to_string(), None);
            PREVIEW_CHECKS.with(|checks| {
                checks.borrow_mut().insert(session.to_string(), [0; 7]);
            });
        }
        else
        {
            previews.remove(session);
            PREVIEW_CHECKS.with(|checks| {
                if let Some(counts) = checks.borrow_mut().remove(session)
                {
                    crate::log_info!("拖放：预览校验统计，会话={}，匹配={}，无确认={}，序号不符={}，布局版本不符={}，显示器不符={}，坐标不符={}，几何无效={}（拒绝原因可重叠）", session, counts[0], counts[1], counts[2], counts[3], counts[4], counts[5], counts[6]);
                }
            });
        }
    }
}

/// 接受页面异步确认；未知会话和过期帧不能覆盖正在使用的落点。
pub(crate) fn publish_preview(preview: DropPreview) -> Result<(), String>
{
    if preview.collect_items.len() > 4096 || preview.collect_items.iter().any(|item| item.name.len() > 1024 || item.instance_id.is_empty() || ![item.bounds.left, item.bounds.top, item.bounds.width, item.bounds.height].iter().all(|value| value.is_finite() && *value >= 0.0))
    {
        return Err("收集项投放几何无效".into());
    }
    if !preview.pointer.left.is_finite() || !preview.pointer.top.is_finite() || preview.regions.len() > 4096 || preview.fence_scrolls.iter().any(|scroll| !scroll.scroll_top.is_finite() || scroll.scroll_top < 0.0)
    {
        return Err("拖放预览数据无效".into());
    }
    let mut previews = DROP_PREVIEWS.lock().map_err(|error| error.to_string())?; // 只保存已登记目标的确认。
    if let Some(current) = previews.get_mut(&preview.session)
        && current.as_ref().is_none_or(|current| (current.sequence, current.geometry_revision) <= (preview.sequence, preview.geometry_revision))
    {
        *current = Some(preview);
    }
    Ok(())
}

/// 释放不等待 Renderer；缺少当前布局有效的确认就拒绝，实时指针负责重新命中区域。
pub(crate) fn confirmed_preview(session: &str, sequence: u64, revision: u64, monitor: &crate::contracts::DesktopHostMonitor, _point: &POINTL) -> Option<DropPreview>
{
    let preview = DROP_PREVIEWS.lock().ok()?.get(session)?.clone(); // 未登记会话不创建统计，固定本次校验的完整几何快照。
    let mut failures = preview.as_ref().map_or([true, false, false, false, false, false], |preview| [false, preview.sequence > sequence, preview.revision != revision, preview.monitor_id != monitor.id, false, false]); // 允许确认帧落后当前序号，实时指针不再与旧帧坐标比较。
    if !failures.iter().any(|failed| *failed)
        && let Some(preview) = preview.as_ref()
    {
        failures[5] = crate::host::validate_hit_regions(&preview.regions, f64::from(monitor.work_width) / monitor.scale_factor, f64::from(monitor.work_height) / monitor.scale_factor).is_err();
    } // 保持原来的短路行为，过期帧不重复扫描完整几何。
    let accepted = !failures.iter().any(|failed| *failed); // 不改变原有安全门槛。
    PREVIEW_CHECKS.with(|checks| {
        let mut checks = checks.borrow_mut(); // 原生线程本地计数，无日志 IO 和跨线程锁。
        let counts = checks.entry(session.to_string()).or_default(); // 本次会话的聚合统计。
        counts[0] += u64::from(accepted);
        for (index, failed) in failures.iter().enumerate()
        {
            counts[index + 1] += u64::from(*failed);
        }
    });
    if accepted { preview } else { None }
}

/// 使用当前原生指针在已确认几何上命中最高优先级区域。
pub(crate) fn hit_region_at_point(preview: &DropPreview, monitor: &crate::contracts::DesktopHostMonitor, point: &POINTL) -> Option<crate::contracts::HitRegion>
{
    let x = (f64::from(point.x) - f64::from(monitor.work_x)) / monitor.scale_factor;
    let y = (f64::from(point.y) - f64::from(monitor.work_y)) / monitor.scale_factor;
    preview.regions.iter().filter(|region| x >= region.left && x < region.left + region.width && y >= region.top && y < region.top + region.height).max_by_key(|region| region.priority).cloned()
}

/// 只在实际释放失败时记录 Renderer 确认，避免移动回调刷屏。
pub(crate) fn trace_rejected_preview(session: &str)
{
    if let Ok(previews) = DROP_PREVIEWS.lock()
    {
        match previews.get(session).and_then(Option::as_ref)
        {
            Some(preview) => crate::log_warn!("拖放：释放时页面确认，会话={}，序号={}，布局版本={}，几何版本={}，显示器={}，位置=({},{})，区域数={}", session, preview.sequence, preview.revision, preview.geometry_revision, preview.monitor_id, preview.pointer.left, preview.pointer.top, preview.regions.len()),
            None => crate::log_warn!("拖放：释放时尚无页面确认，会话={session}"),
        }
    }
}

/// 当前源会话由进程登记，不从外部数据格式或项目路径推断内部身份。
static SOURCE_DRAG: std::sync::Mutex<Option<std::sync::Arc<SourceDragSession>>> = std::sync::Mutex::new(None);

/// 保持完整 Shell 数据对象身份及内部布局完成事实。
pub(crate) struct SourceDragSession
{
    pub pointer_offset: crate::contracts::WorkspacePoint, // 指针相对锚点表面的 DIP 偏移，跨屏保持。
    data: windows::core::AgileReference<IDataObject>,     // 在目标 apartment 重新解析真实 COM 身份。
    pub item_ids: Vec<String>,                            // 源端已经校验的完整选择，顺序以锚点开头。
    pub collect_names: Option<Vec<String>>, // 收集条目身份只从本进程登记读取。
    pub portal_source: bool,                              // 门户来源跨本程序目标也必须执行真实文件传输。
    pub layout_committed: std::sync::atomic::AtomicBool,  // 内部完成不通过文件移动效果反馈。
}

/// 源端退出 OLE 时释放登记，包含原生错误和取消路径。
pub(crate) struct SourceDragGuard(std::sync::Arc<SourceDragSession>);

impl SourceDragGuard
{
    /// 开始唯一源会话；登记失败时不能启动无法识别的内部回落。
    pub(crate) fn register(data: &IDataObject, item_ids: &[String], pointer_offset: crate::contracts::WorkspacePoint, portal_source: bool, collect_names: Option<Vec<String>>) -> windows::core::Result<Self>
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
        let session = std::sync::Arc::new(SourceDragSession { data, pointer_offset, item_ids: item_ids.to_vec(), portal_source, collect_names, layout_committed: std::sync::atomic::AtomicBool::new(false) }); // 完整数据对象在本次 OLE 会话内有效。
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
            && current.as_ref().is_some_and(|session| std::sync::Arc::ptr_eq(session, &self.0))
        {
            current.take();
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
pub(crate) fn wait_transfer<T>(result: std::sync::mpsc::Receiver<Result<T, String>>) -> Result<T, String>
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
pub(crate) fn filesystem_drop_items(data: &IDataObject) -> Option<Vec<String>> { shell_drop_items(data, true) }

/// 收集箱把失效路径交给逐项业务反馈，不提前丢弃整个 Shell 集合。
pub(crate) fn collect_shell_paths(data: &IDataObject) -> Option<Vec<String>> { shell_drop_items(data, false) }

/// 提取 Shell 文件系统路径，桌面目标保留完整选择校验。
fn shell_drop_items(data: &IDataObject, strict: bool) -> Option<Vec<String>>
{
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{IShellItemArray, SHCreateShellItemArrayFromDataObject, SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_FILESYSPATH};
    let items: IShellItemArray = unsafe { SHCreateShellItemArrayFromDataObject(data) }.ok()?; // Shell 统一解析真实数据格式。
    let count = unsafe { items.GetCount() }.ok()?; // 顶层传输项目总数。
    if count == 0 || count > 512
    {
        return None;
    }
    let references = (0..count)
        .map(|index| {
            let item = unsafe { items.GetItemAt(index) }.ok()?; // 当前源项目。
            let path = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }.ok()?; // 排除虚拟文件。
            let value = unsafe { path.to_string() }; // 验证现有文件路径。
            unsafe { CoTaskMemFree(Some(path.0.cast())) };
            if strict && std::fs::metadata(value.ok()?).is_err()
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
    if strict && paths.iter().enumerate().any(|(index, path)| paths.iter().enumerate().any(|(other, parent)| index != other && (path == parent || path.starts_with(&format!("{parent}\\")))))
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
                if let Some(helper) = &self.helper
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
        if let Err(error) = &result
        {
            crate::log_error!("拖放：释放处理失败，错误={error}");
        }
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
#[path = "drop_target.test.rs"]
mod tests;
