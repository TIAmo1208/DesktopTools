use crate::{app::settings::SettingsState, contracts::DesktopHostMonitor};
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    num::NonZeroIsize,
    rc::Rc,
    sync::{Arc, Mutex},
};
use tauri::{AppHandle, Emitter, Listener, Manager};
use windows::{
    Win32::{
        Foundation::{ERROR_CLASS_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SetLastError, WIN32_ERROR, WPARAM},
        Graphics::{
            Dwm::{DWM_BB_BLURREGION, DWM_BB_ENABLE, DWM_BLURBEHIND, DwmEnableBlurBehindWindow},
            Gdi::{CreateRectRgn, DeleteObject, ScreenToClient},
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Input::KeyboardAndMouse::{GetAsyncKeyState, ReleaseCapture, VK_ESCAPE, VK_LBUTTON},
            WindowsAndMessaging::*,
        },
    },
    core::{PCWSTR, w},
};
use wry::{
    Rect as WryRect, WebView, WebViewBuilder, dpi,
    http::{Request, Response},
    raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle},
};

/// 岛吸附的工作区边；几何均使用 DIP，与任务栏实际位置无关。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IslandEdge
{
    Top,
    Bottom,
    Left,
    Right,
}

impl IslandEdge
{
    /// 读取已校验的偏好值；空串表示顶部默认位置，未知值不静默回退。
    pub fn from_preference(value: &str) -> Option<Self>
    {
        match value
        {
            "" | "top" => Some(Self::Top),
            "bottom" => Some(Self::Bottom),
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            _ => None,
        }
    }

    /// 返回显式贴边偏好；拖动落点始终写具体四边，默认居中空串由偏好调用方保留。
    pub fn as_preference(self) -> &'static str
    {
        match self
        {
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

/// 岛或工作区的 DIP 矩形；调用方保证坐标有限、尺寸非负。
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
pub struct IslandRect
{
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

/// 取中心到工作区边界直线的最近距离；等距固定按上、下、左、右优先。
pub fn snap_edge(center_x: f64, center_y: f64, work: IslandRect) -> IslandEdge
{
    // 严格小于才替换，保证等距时保留较早的边。
    let distances = [(IslandEdge::Top, (center_y - work.top).abs()), (IslandEdge::Bottom, (center_y - work.top - work.height).abs()), (IslandEdge::Left, (center_x - work.left).abs()), (IslandEdge::Right, (center_x - work.left - work.width).abs())];
    let mut nearest = distances[0]; // 当前最近边与距离。
    for candidate in distances.into_iter().skip(1)
    {
        if candidate.1 < nearest.1
        {
            nearest = candidate;
        }
    }
    nearest.0
}

/// 将距工作区左端（水平边）或顶端（垂直边）的偏移夹紧；尺寸超限时落到起点。
pub fn clamp_offset(edge: IslandEdge, offset: f64, size: (f64, f64), work: IslandRect) -> f64
{
    let available = match edge
    {
        IslandEdge::Top | IslandEdge::Bottom => work.width - size.0,
        IslandEdge::Left | IslandEdge::Right => work.height - size.1,
    }; // 沿边可移动距离；负值表示岛比工作区长。
    offset.clamp(0.0, available.max(0.0))
}

/// 将岛贴到指定工作区边；超出工作区的尺寸不缩小，只将对应轴放到起点。
pub fn place(edge: IslandEdge, offset: f64, size: (f64, f64), work: IslandRect) -> IslandRect
{
    let offset = clamp_offset(edge, offset, size, work); // 统一夹紧持久化与实时偏移。
    let (left, top) = match edge
    {
        IslandEdge::Top => (work.left + offset, work.top),
        IslandEdge::Bottom => (work.left + offset, work.top + (work.height - size.1).max(0.0)),
        IslandEdge::Left => (work.left, work.top + offset),
        IslandEdge::Right => (work.left + (work.width - size.0).max(0.0), work.top + offset),
    }; // 工作区绝对坐标，允许多显示器负坐标。
    IslandRect { left, top, width: size.0, height: size.1 }
}

/// 按偏好放置；空串忽略旧偏移，始终在顶部居中。
pub fn place_from_preference(edge: &str, offset: f64, size: (f64, f64), work: IslandRect) -> Option<IslandRect>
{
    let parsed = IslandEdge::from_preference(edge)?; // 未知字符串由边界调用方处理错误。
    let offset = if edge.is_empty() { (work.width - size.0) / 2.0 } else { offset }; // 默认位置不持久化为显式 top。
    Some(place(parsed, offset, size, work))
}

/// 返回指向工作区内部的展开单位向量。
#[allow(dead_code)] // 岛页面按态选择展开方向后接入。
pub fn expand_direction(edge: IslandEdge) -> (f64, f64)
{
    match edge
    {
        IslandEdge::Top => (0.0, 1.0),
        IslandEdge::Bottom => (0.0, -1.0),
        IslandEdge::Left => (1.0, 0.0),
        IslandEdge::Right => (-1.0, 0.0),
    }
}

/// 状态切换只调整一次窗口盒，覆盖当前态与目标态的完整区域。
pub fn union_box(a: IslandRect, b: IslandRect) -> IslandRect
{
    let left = a.left.min(b.left); // 联合盒左边界。
    let top = a.top.min(b.top); // 联合盒上边界。
    IslandRect { left, top, width: (a.left + a.width).max(b.left + b.width) - left, height: (a.top + a.height).max(b.top + b.height) - top }
}

/// 独立岛协议只允许自己的来源调用既有命令通道。
pub(crate) const ISLAND_PROTOCOL: &str = "desktop-tools-island";
const ISLAND_CLASS: PCWSTR = w!("DesktopTools.Island"); // 唯一岛窗口类。

/// 岛始终使用独立协议与内嵌页面；主题地址沿用桌面宿主的缓存键派生规则。
pub(crate) fn island_page_url(boot: u64, theme_css: Option<&str>) -> tauri::Url
{
    let mut url = tauri::Url::parse(&format!("http://{ISLAND_PROTOCOL}.localhost/index.html")).expect("岛协议地址无效"); // 不接受开发服务器来源。
    url.query_pairs_mut().append_pair("island", "1").append_pair("boot", &boot.to_string());
    if let Some(stylesheet) = theme_css
    {
        url.query_pairs_mut().append_pair("theme-css", stylesheet);
    }
    url
}

/// 页面提交的窗口内局部卡片区域，圆角半径也使用 DIP。
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct IslandCard
{
    pub rect: IslandRect,
    pub radius: f64,
}

/// 一次状态切换的完整几何；window_box 为屏幕绝对 DIP 联合盒。
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct IslandStateRequest
{
    pub state: String,
    pub window_box: IslandRect,
    pub cards: Vec<IslandCard>,
}

impl IslandStateRequest
{
    /// IPC 不接受非有限数、无界区域、盒外卡片或未知状态。
    pub(crate) fn validate(&self) -> Result<(), String>
    {
        if !matches!(self.state.as_str(), "collapsed" | "hover" | "expanded")
        {
            return Err("未知岛状态".into());
        }
        if !valid_rect(self.window_box) || self.cards.is_empty() || self.cards.len() > 128
        {
            return Err("岛窗口盒或卡片数量无效".into());
        }
        for card in &self.cards
        {
            if !valid_rect(card.rect) || card.rect.left < 0.0 || card.rect.top < 0.0 || card.rect.left + card.rect.width > self.window_box.width || card.rect.top + card.rect.height > self.window_box.height || !card.radius.is_finite() || card.radius < 0.0 || card.radius > card.rect.width.min(card.rect.height) / 2.0
            {
                return Err("岛卡片区域或圆角无效".into());
            }
        }
        if self.state == "collapsed" && self.cards.len() != 1
        {
            return Err("常态岛必须只有一个胶囊区域".into());
        }
        Ok(())
    }
}

/// 防止浮点到 Win32 整数转换溢出；实际工作区边界在 UI 线程进一步检查。
fn valid_rect(rect: IslandRect) -> bool { [rect.left, rect.top, rect.width, rect.height].iter().all(|value| value.is_finite() && value.abs() <= 1_000_000.0) && rect.width > 0.0 && rect.height > 0.0 }

/// UI 线程唯一持有的岛资源，不把 WebView 或 HWND 跨线程执行。
struct IslandWindow
{
    hwnd: HWND,                           // 外层不激活工具窗口。
    webview: Rc<WebView>,                 // UI 线程可克隆快照，在借用期外调整 Controller。
    app: AppHandle,                       // 松手保存偏好所需应用状态。
    monitor: DesktopHostMonitor,          // 主屏当前工作区与缩放。
    geometry: Option<IslandStateRequest>, // 首次状态到达前禁止显示。
    drag: DragProgress,                   // 几何修订使旧移动循环失效。
}

/// 一个移动循环最多消费一次当前几何修订；切态会取消尚未完成的落点。
#[derive(Default)]
struct DragProgress
{
    revision: u64,                                                       // 当前几何代次。
    pending: Option<u64>,                                                // 进行中移动循环的几何代次。
    landing_seq: u64,                                                    // 开始拖动时分配，跨窗口重建也不复用。
    landing_done: Option<std::sync::mpsc::SyncSender<IslandDragResult>>, // 本次后台保存与命令完成的单次屏障。
}
impl DragProgress
{
    fn begin(&mut self) -> Result<(), String>
    {
        if self.pending.is_some()
        {
            return Err("岛正在拖动".into());
        }
        self.pending = Some(self.revision);
        Ok(())
    }
    fn invalidate(&mut self)
    {
        self.revision = self.revision.wrapping_add(1);
        self.pending = None;
        self.landing_done = None; // 取消且未调度保存时断开等待者，不报保存失败。
    }
    fn take_commit(&mut self) -> bool { self.pending.take() == Some(self.revision) }
}
/// 落点开始与保存共用一把序号锁，旧后台任务不能覆盖更新的拖动。
#[derive(Default)]
struct LandingOrder
{
    latest: u64,    // 最新开始的拖动序号。
    committed: u64, // 已落盘的落点序号。
}
impl LandingOrder
{
    fn begin(&mut self) -> Result<u64, String>
    {
        self.latest = self.latest.checked_add(1).ok_or("岛拖动序号已耗尽")?;
        Ok(self.latest)
    }

    /// 锁覆盖序号检查和原子字段更新；关闭后的落点不写盘。
    fn save(&mut self, state: &SettingsState, sequence: u64, edge: IslandEdge, offset: f64) -> Result<Option<crate::contracts::settings::ApplicationPreferences>, String>
    {
        if sequence != self.latest || sequence <= self.committed
        {
            crate::log_debug!("岛：丢弃过期落点，序号={}，最新={}，已提交={}", sequence, self.latest, self.committed);
            return Ok(None);
        }
        let mut disabled = false; // patch 内确认真实开关，不能依赖拖动前的快照。
        let saved = state.update(|preferences| {
            if !preferences.island_enabled
            {
                disabled = true;
                return Err("岛已关闭，不保存落点".into());
            }
            preferences.island_edge = edge.as_preference().into();
            preferences.island_offset = offset;
            Ok(())
        });
        if disabled
        {
            crate::log_debug!("岛：关闭后丢弃落点，序号={}", sequence);
            return Ok(None);
        }
        let saved = saved?;
        self.committed = sequence; // 仅磁盘与内存均成功后推进已提交序号。
        Ok(Some(saved))
    }
}
// ponytail: 单岛串行落盘锁；扩展多岛时再按岛身份拆锁。
static LANDINGS: Mutex<LandingOrder> = Mutex::new(LandingOrder { latest: 0, committed: 0 });

thread_local! {
    static ISLAND: RefCell<Option<IslandWindow>> = const { RefCell::new(None) }; // 仅 Tauri UI 线程访问。
}

/// Wry 复用已有 Win32 HWND 的标准包装。
struct IslandHandle(NonZeroIsize);
impl HasWindowHandle for IslandHandle
{
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> { Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(Win32WindowHandle::new(self.0))) }) }
}

/// 将全屏和锁屏事实映射为岛可见性；未知状态不伪造已确认覆盖。
fn suppressed(snapshot: &super::refresh_policy::ForegroundSnapshot, hide_fullscreen: bool) -> bool { snapshot.locked == Some(true) || (hide_fullscreen && snapshot.exclusive_fullscreen == Some(true)) }

/// 在既有前台事件上重新断言置顶，不启动轮询。
pub(crate) fn install(app: &AppHandle)
{
    let handle = app.clone(); // 事件闭包不持有任何 Win32 资源。
    app.listen("desktop-refresh-policy-changed", move |_| {
        let _ = handle.run_on_main_thread(|| apply_policy(true));
    });
    reconcile(app);
}

/// 保存偏好后调度 UI 更新；执行时读取最新值，过期排队不会重新打开已关闭的岛。
pub(crate) fn reconcile(app: &AppHandle)
{
    let handle = app.clone(); // UI 回调所需的应用状态。
    if let Err(error) = app.run_on_main_thread(move || {
        let result = (|| {
            let preferences = handle.state::<SettingsState>().preferences()?; // 保存后的最新事实。
            if !preferences.island_enabled
            {
                destroy();
                return Ok(());
            }
            if ISLAND.with(|slot| slot.borrow().is_none())
            {
                create(&handle)?;
            }
            apply_policy(true);
            Ok::<(), String>(())
        })();
        if let Err(error) = result
        {
            crate::log_warn!("岛：应用偏好失败，错误：{error}");
        }
    })
    {
        crate::log_warn!("岛：无法调度偏好更新，错误：{error}");
    }
}

/// 主屏是本期唯一的目标屏，物理 rcWork 转换成同一绝对 DIP 空间。
fn primary_monitor() -> Result<DesktopHostMonitor, String> { super::enumerate_desktop_monitors()?.into_iter().find(|monitor| monitor.primary).ok_or_else(|| "岛缺少主显示器".into()) }
fn work_dip(monitor: &DesktopHostMonitor) -> IslandRect { IslandRect { left: f64::from(monitor.work_x) / monitor.scale_factor, top: f64::from(monitor.work_y) / monitor.scale_factor, width: f64::from(monitor.work_width) / monitor.scale_factor, height: f64::from(monitor.work_height) / monitor.scale_factor } }
fn px(value: f64, scale: f64) -> i32 { (value * scale).round() as i32 }

const WM_ISLAND_RESTORE_FOREGROUND: u32 = WM_APP + 38; // WebView2 绕过鼠标激活消息时的一次性补偿。
const WM_ISLAND_CHILD_NOACTIVATE: u32 = WM_APP + 37; // 合并子窗口创建与导航完成后的样式修复时机。

/// 一次点击最多恢复一次；任何条件失效也消费记录，不能留到下次激活使用。
#[derive(Default)]
struct ClickForeground
{
    target: Option<isize>, // 鼠标激活前的外部根窗口。
}
impl ClickForeground
{
    fn take(&mut self, island: isize, foreground: isize, valid: bool, dragging: bool) -> Option<isize>
    {
        let target = self.target.take()?; // 在任何 Win32 调用前清空，允许消息同步重入。
        (valid && !dragging && foreground == island && target != 0 && target != island).then_some(target)
    }
}
thread_local! {
    static CLICK_FOREGROUND: RefCell<ClickForeground> = RefCell::new(ClickForeground::default()); // 不与岛资源 RefCell 共借用，避免窗口消息重入冲突。
    static NATIVE_DRAGGING: Cell<bool> = const { Cell::new(false) }; // 原生移动循环内禁止恢复前台。
}

/// 同时检查已接受的拖动请求与实际原生移动循环；借用冲突时保守放弃补偿。
fn native_dragging() -> bool { NATIVE_DRAGGING.get() || ISLAND.with(|slot| slot.try_borrow().map_or(true, |slot| slot.as_ref().is_some_and(|window| window.drag.pending.is_some()))) }

/// 仅补偿尚未被用户主动切走的本次点击激活，不保证系统级禁止显式激活。
fn restore_click_foreground(hwnd: HWND)
{
    let target = CLICK_FOREGROUND.with(|state| state.borrow().target); // 只快照数值，Win32 调用不持有借用。
    let valid = target.is_some_and(|target| unsafe { IsWindow(Some(HWND(target as *mut _))).as_bool() && IsWindowVisible(HWND(target as *mut _)).as_bool() });
    let foreground = unsafe { GetForegroundWindow() }.0 as isize;
    let target = CLICK_FOREGROUND.with(|state| state.borrow_mut().take(hwnd.0 as isize, foreground, valid, native_dragging()));
    if let Some(target) = target
        && !unsafe { SetForegroundWindow(HWND(target as *mut _)) }.as_bool()
    {
        crate::log_warn!("岛：点击激活后的前台恢复被系统拒绝，目标={target}");
    }
}

/// 仅合并不激活位，不能覆盖 WebView2 已设置的透明、布局及其他扩展样式。
fn noactivate_style(style: isize) -> isize { style | WS_EX_NOACTIVATE.0 as isize }

/// EnumChildWindows 自身递归后代；只改样式，不子类化、不吞鼠标或键盘消息。
fn apply_child_noactivate(hwnd: HWND)
{
    unsafe extern "system" fn apply(child: HWND, _: LPARAM) -> windows::core::BOOL
    {
        let style = unsafe { GetWindowLongPtrW(child, GWL_EXSTYLE) }; // 保留 Chromium/WebView2 自有位。
        let merged = noactivate_style(style);
        if merged != style
        {
            unsafe {
                SetLastError(WIN32_ERROR(0));
            }
            let previous = unsafe { SetWindowLongPtrW(child, GWL_EXSTYLE, merged) }; // 返回零也可能是合法旧样式，须配合 LastError 判断。
            let error = unsafe { GetLastError() };
            if previous == 0 && error.0 != 0
            {
                let actual = unsafe { GetWindowLongPtrW(child, GWL_EXSTYLE) }; // WebView2 建窗钩子可能留下 LastError，以读回的目标位为准。
                if unsafe { IsWindow(Some(child)) }.as_bool() && actual != noactivate_style(actual)
                {
                    crate::log_warn!("岛：子窗口不激活样式读回未生效，窗口={}，错误={}", child.0 as isize, error.0);
                } // 已销毁的创建期窗口无需修复；后续创建/导航事件会处理仍存活的后代。
            }
        }
        windows::core::BOOL(1)
    }
    let _ = unsafe { EnumChildWindows(Some(hwnd), Some(apply), LPARAM(0)) }; // 枚举期间新建窗口由通知或导航完成补齐。
}

/// 延后到子窗口创建流程退出后再枚举，避免在 WebView2 的建窗栈中修改样式。
fn schedule_child_noactivate(hwnd: HWND)
{
    if let Err(error) = unsafe { PostMessageW(Some(hwnd), WM_ISLAND_CHILD_NOACTIVATE, WPARAM(0), LPARAM(0)) }
    {
        crate::log_warn!("岛：子窗口样式更新调度失败，错误：{error}");
    }
}

/// 建窗失败也释放 HWND；开启但页面尚未提交状态时保持隐藏。
fn create(app: &AppHandle) -> Result<(), String>
{
    let monitor = primary_monitor()?; // 从原生 rcWork 获取当前主屏。
    let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.map_err(|error| error.to_string())?; // 当前模块身份。
    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(window_proc),
        hInstance: HINSTANCE(module.0),
        lpszClassName: ISLAND_CLASS,
        hbrBackground: Default::default(), // 空类画刷，不绘制不透明窗口底色。
        ..Default::default()
    }; // 透明矩形 HWND，可见圆角交给页面 CSS。
    if unsafe { RegisterClassExW(&class) } == 0 && unsafe { GetLastError() } != ERROR_CLASS_ALREADY_EXISTS
    {
        return Err(format!("注册岛窗口类失败：{}", windows::core::Error::from_win32()));
    }
    let hwnd = unsafe { CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, ISLAND_CLASS, w!("Desktop Tools Island"), WS_POPUP | WS_CLIPCHILDREN | WS_CLIPSIBLINGS, monitor.work_x, monitor.work_y, 1, 1, None, None, Some(HINSTANCE(module.0)), None) }.map_err(|error| format!("创建岛窗口失败：{error}"))?; // 占位像素不是三态视觉尺寸。
    let build = (|| {
        let raw = IslandHandle(NonZeroIsize::new(hwnd.0 as isize).ok_or("岛 HWND 为空")?); // Wry 句柄借用只限本线程。
        let protocol_app = app.clone(); // 协议请求后台分派。
        let island_hwnd = hwnd.0 as isize; // 页面回调仅捕获数值，不跨线程持有原生窗口对象。
        WebViewBuilder::new()
            .with_id("island")
            .with_transparent(true)
            .with_focused(false) // Wry 默认主动 MoveFocus；岛创建时不能抢走前台。
            .with_bounds(WryRect { position: dpi::PhysicalPosition::new(0, 0).into(), size: dpi::PhysicalSize::new(1, 1).into() })
            .with_asynchronous_custom_protocol(ISLAND_PROTOCOL.into(), move |_id, request, responder| {
                dispatch_protocol(request, protocol_app.clone(), move |response| responder.respond(response));
            })
            .with_on_page_load_handler(move |event, _| {
                if matches!(event, wry::PageLoadEvent::Finished)
                {
                    schedule_child_noactivate(HWND(island_hwnd as *mut _)); // 某些 WebView2 后代禁用父通知，导航完成再补齐一次。
                }
            })
            .with_initialization_script(&super::desktop_host::native_bridge_script(&format!("http://{ISLAND_PROTOCOL}.localhost")))
            .with_url(island_page_url(super::desktop_host::boot_epoch(), super::desktop_host::active_theme_stylesheet(app).as_deref()).as_str())
            .build_as_child(&raw)
            .map_err(|error| format!("创建岛 WebView 失败：{error}"))
    })(); // 任何 WebView 构造失败均销毁占位 HWND。
    let webview = match build
    {
        Ok(webview) => webview,
        Err(error) =>
        {
            let _ = unsafe { DestroyWindow(hwnd) };
            return Err(error);
        }
    };
    apply_child_noactivate(hwnd); // 创建后的全部后代都必须保持点击不激活，根窗口样式不变。
    let empty = unsafe { CreateRectRgn(0, 0, 0, 0) }; // 空模糊区域，只启用合成透明，不铺系统材质。
    if empty.is_invalid()
    {
        drop(webview);
        let _ = unsafe { DestroyWindow(hwnd) };
        return Err("创建岛透明合成空区域失败".into());
    }
    let blur = DWM_BLURBEHIND { dwFlags: DWM_BB_ENABLE | DWM_BB_BLURREGION, fEnable: true.into(), hRgnBlur: empty, ..Default::default() };
    let result = unsafe { DwmEnableBlurBehindWindow(hwnd, &blur).and_then(|_| windows::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea(hwnd, &windows::Win32::UI::Controls::MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 })) }; // 无系统材质，让整个客户区参与 DWM 透明合成。
    let _ = unsafe { DeleteObject(empty.into()) }; // DWM调用复制区域，不转移HRGN所有权。
    if let Err(error) = result
    {
        drop(webview);
        let _ = unsafe { DestroyWindow(hwnd) };
        return Err(format!("启用岛透明合成失败：{error}"));
    }
    crate::log_info!("岛：透明矩形窗口已启用，CSS 绘制圆角，透明角落仍属于岛命中区");
    if let Err(error) = unsafe { SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) }
    {
        drop(webview);
        let _ = unsafe { DestroyWindow(hwnd) };
        return Err(format!("岛置顶失败：{error}"));
    }
    crate::log_info!("岛：已创建隐藏窗口，显示器={}，工作区=[{}, {}, {}, {}]，缩放={}", monitor.id, monitor.work_x, monitor.work_y, monitor.work_width, monitor.work_height, monitor.scale_factor);
    ISLAND.with(|slot| *slot.borrow_mut() = Some(IslandWindow { hwnd, webview: Rc::new(webview), app: app.clone(), monitor, geometry: None, drag: DragProgress::default() }));
    Ok(())
}

/// 共享事件白名单在 UI 线程调用；借用结束后执行脚本，允许原生消息重入。
pub(crate) fn dispatch_script(script: &str)
{
    let webview = ISLAND.with(|slot| slot.borrow().as_ref().map(|window| Rc::clone(&window.webview))); // 岛关闭时没有投递目标。
    if let Some(webview) = webview
        && let Err(error) = webview.evaluate_script(script)
    {
        crate::log_warn!("岛：事件投递失败，错误：{error}");
    }
}

/// UI 线程幂等清理，先释放 WebView 再销毁外层 HWND。
pub(crate) fn destroy()
{
    CLICK_FOREGROUND.with(|state| state.borrow_mut().target = None); // 关闭后不保留旧 HWND 的补偿任务。
    NATIVE_DRAGGING.set(false);
    let resource = ISLAND.with(|slot| slot.borrow_mut().take()); // 先移出，窗口销毁重入时不会重复释放。
    if let Some(resource) = resource
    {
        let hwnd = resource.hwnd; // WebView drop 后仍需关闭原生窗口。
        drop(resource);
        if let Err(error) = unsafe { DestroyWindow(hwnd) }
        {
            crate::log_warn!("岛：销毁窗口失败，错误：{error}");
        }
        crate::log_info!("岛：窗口及 WebView 已释放");
    }
}

/// 两套协议注册都走既有命令边界，来源白名单只允许岛协议自身。
pub(crate) fn dispatch_protocol<F>(request: Request<Vec<u8>>, app: AppHandle, respond: F)
where
    F: FnOnce(Response<Cow<'static, [u8]>>) + Send + 'static,
{
    let invoke = request.uri().path().starts_with("/__desktop_tools_invoke/"); // 只有命令可能阻塞。
    let run = move || {
        let handler: super::NativeCommandHandler = Arc::new({
            let app = app.clone(); // 分派时读取同一运行时。
            move |command, arguments| {
                let runtime = app.try_state::<Arc<crate::host::DesktopHostRuntime>>().ok_or_else(|| serde_json::json!({"code": "desktop-host-starting", "message": "桌面宿主正在启动"}))?;
                crate::commands::dispatch_native_command(&app, &runtime, command, arguments)
            }
        });
        respond(super::desktop_host::handle_desktop_request(request, &app, &handler, &format!("http://{ISLAND_PROTOCOL}.localhost"), None));
    };
    if invoke
    {
        tauri::async_runtime::spawn_blocking(run);
    }
    else
    {
        run();
    }
}

/// UI 线程使用相同快照隐藏或无激活恢复；未收首态绝不显示占位窗。
fn apply_policy(reassert_topmost: bool)
{
    let target = ISLAND.with(|slot| {
        let slot = slot.borrow(); // 仅计算副作用所需的 Copy 数据，不调用窗口 API。
        let resource = slot.as_ref()?;
        let preferences = resource.app.state::<SettingsState>().preferences().ok()?;
        let hide = !preferences.island_enabled || resource.geometry.is_none() || suppressed(&super::refresh_policy::foreground_snapshot(), preferences.island_hide_on_fullscreen);
        Some((resource.hwnd, hide))
    }); // RefCell 借用到此结束，隐藏中重入 finish_drag 也可以取得可变借用。
    if let Some((hwnd, hide)) = target
    {
        unsafe {
            let _ = ShowWindow(hwnd, if hide { SW_HIDE } else { SW_SHOWNOACTIVATE });
            if !hide && reassert_topmost
            {
                let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
    }
}

/// 切态原生事务的失败阶段，全部采用清空并隐藏而非多步回滚。
#[derive(Clone, Copy, Debug)]
enum StateStage
{
    WindowBox,
    WebView,
    Commit,
}

/// 生产与失败注入共用事务收口；访问状态的闭包只在原生副作用结束后运行。
fn run_transition(request: IslandStateRequest, apply: impl FnOnce(&IslandStateRequest) -> Result<(), String>, mut with_state: impl FnMut(bool, &mut dyn FnMut(&mut Option<IslandStateRequest>, &mut DragProgress)) -> Result<(), String>, on_failure: impl FnOnce()) -> Result<(), String>
{
    let applied = apply(&request); // 不持有任何 RefCell 借用，可同步重入消息循环。
    let success = applied.is_ok();
    let mut request = Some(request); // 闭包只会提交一次，失败不保留上一态。
    let committed = with_state(success, &mut |geometry, drag| {
        if success
        {
            *geometry = request.take();
        }
        else
        {
            *geometry = None;
            drag.invalidate();
        }
    });
    if success && committed.is_err()
    {
        let _ = with_state(false, &mut |geometry, drag| {
            *geometry = None;
            drag.invalidate();
        }); // 提交过期也走同一清空分支。
    }
    let result = applied.and(committed);
    if result.is_err()
    {
        on_failure();
    } // 隐藏在状态访问结束后执行，避免重入借用。
    result
}

/// 未提交首态或切态失败后的岛一律不能开始拖动。
fn can_drag(geometry: Option<&IslandStateRequest>) -> bool { geometry.is_some_and(|geometry| matches!(geometry.state.as_str(), "collapsed" | "hover")) }

/// 每次提交都完整校验；静默态一次提交，动画帧复用同一入口，不另设队列或节流。
pub(crate) fn set_state(request: IslandStateRequest) -> Result<(), String>
{
    finish_state_submission(apply_state(request), || {
        let hwnd = ISLAND.with(|slot| {
            let mut slot = slot.borrow_mut(); // 包括参数和工作区预检失败，绝不保留上一动画中间态。
            slot.as_mut().map(|resource| {
                resource.geometry = None;
                resource.drag.invalidate();
                resource.hwnd
            })
        });
        if let Some(hwnd) = hwnd
        {
            let _ = unsafe { ShowWindow(hwnd, SW_HIDE) }; // 状态借用释放后隐藏，允许消息重入。
        }
    })
}

/// 任意失败只触发一次外层清理；成功帧和最终态均不隐藏。
fn finish_state_submission(result: Result<(), String>, on_failure: impl FnOnce()) -> Result<(), String>
{
    if result.is_err()
    {
        on_failure();
    }
    result
}

/// UI 线程应用一份完整几何；调用方串行按帧提交，原生错误统一交给外层隐藏。
fn apply_state(mut request: IslandStateRequest) -> Result<(), String>
{
    request.validate()?;
    let monitor = primary_monitor()?; // 枚举窗口环境也在 RefCell 借用期外。
    let (hwnd, webview, revision, state_changed) = ISLAND.with(|slot| {
        let mut slot = slot.borrow_mut(); // 只校验并取出快照，不调用会泵消息的 API。
        let resource = slot.as_mut().ok_or("岛未开启")?;
        if !resource.app.state::<SettingsState>().preferences()?.island_enabled
        {
            return Err("岛未开启".into());
        }
        let area = work_dip(&monitor); // 实测主屏 rcWork 的 DIP 表达。
        let scale = monitor.scale_factor; // 所有 HWND 和 HRGN 坐标只在这里转像素。
        if resource.geometry.is_none()
        {
            let preferences = resource.app.state::<SettingsState>().preferences()?; // 首次状态按偏好落位。
            let active = request.cards.iter().map(|card| card.rect).reduce(union_box).ok_or("岛区域为空")?;
            let placed = place_from_preference(&preferences.island_edge, preferences.island_offset, (active.width, active.height), area).ok_or("岛贴边偏好无效")?;
            request.window_box.left = placed.left - active.left;
            request.window_box.top = placed.top - active.top;
        }
        if request.window_box.width > area.width || request.window_box.height > area.height
        {
            return Err("岛窗口盒超过主屏工作区".into());
        }
        if px(request.window_box.width, scale) < 1 || px(request.window_box.height, scale) < 1 || request.cards.iter().any(|card| px(card.rect.left + card.rect.width, scale) <= px(card.rect.left, scale) || px(card.rect.top + card.rect.height, scale) <= px(card.rect.top, scale))
        {
            return Err("岛区域不足一个物理像素".into());
        }
        request.window_box.left = request.window_box.left.clamp(area.left, area.left + area.width - request.window_box.width);
        request.window_box.top = request.window_box.top.clamp(area.top, area.top + area.height - request.window_box.height);
        let state_changed = resource.geometry.as_ref().is_none_or(|previous| previous.state != request.state); // 动画同态帧不重复记录 INFO。
        resource.drag.invalidate(); // 先使旧拖动失效，再执行可能重入的窗口操作。
        resource.geometry = None; // 提交完成前也拒绝按上一态开始拖动。
        Ok::<_, String>((resource.hwnd, Rc::clone(&resource.webview), resource.drag.revision, state_changed))
    })?; // 从此开始不持有 ISLAND 的任何借用。
    let scale = monitor.scale_factor;
    let stage = std::cell::Cell::new(StateStage::WindowBox); // 多闭包共享阶段，不持有 RefCell 借用。
    let result = run_transition(
        request,
        |request| {
            let rect = request.window_box; // 本次静默态或动画帧对应的物理窗口盒。
            stage.set(StateStage::WindowBox);
            unsafe { SetWindowPos(hwnd, Some(HWND_TOPMOST), px(rect.left, scale), px(rect.top, scale), px(rect.width, scale), px(rect.height, scale), SWP_NOACTIVATE) }.map_err(|error| format!("设置岛窗口盒失败：{error}"))?;
            stage.set(StateStage::WebView);
            webview.set_bounds(WryRect { position: dpi::PhysicalPosition::new(0, 0).into(), size: dpi::PhysicalSize::new(px(rect.width, scale) as u32, px(rect.height, scale) as u32).into() }).map_err(|error| format!("设置岛 WebView 尺寸失败：{error}"))?;
            if state_changed
            {
                crate::log_info!("岛：切换状态={}，显示器={}，窗口盒DIP=[{}, {}, {}, {}]，缩放={}", request.state, monitor.id, rect.left, rect.top, rect.width, rect.height, scale);
            }
            stage.set(StateStage::Commit);
            Ok(())
        },
        |success, update| {
            ISLAND.with(|slot| {
                let mut slot = slot.borrow_mut(); // 状态收口只操作 Rust 数据，绝不泵送原生消息。
                let resource = slot.as_mut().filter(|resource| Rc::ptr_eq(&resource.webview, &webview)).ok_or("岛窗口已销毁")?;
                if success && resource.drag.revision != revision
                {
                    return Err("岛状态提交已失效".into());
                }
                update(&mut resource.geometry, &mut resource.drag);
                if success
                {
                    resource.monitor = monitor.clone();
                }
                Ok(())
            })
        },
        || {
            crate::log_warn!("岛：几何提交失败，外层将清空几何并隐藏，阶段={:?}", stage.get()); // 前置校验与原生步骤共用外层 fail-closed。
        },
    );
    if result.is_ok()
    {
        apply_policy(false); // 切态的 SetWindowPos 已同时断言置顶，不重复 resize/定位。
    }
    result
}

/// 释放并吸附后的指针事实，不依赖 WebView2 是否补发 pointerleave。
#[derive(Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct IslandDragResult
{
    pub pointer_inside: bool, // 未确认、取消或超时时保守返回 false。
}

/// Win32 客户区命中规则：左上含、右下不含，圆角不参与矩形内外事实。
fn point_inside_client(point: POINT, rect: RECT) -> bool { point.x >= rect.left && point.y >= rect.top && point.x < rect.right && point.y < rect.bottom }

/// 仅在成功吸附并提交最终几何后调用，使用同一物理客户区坐标空间。
fn cursor_inside_client(hwnd: HWND) -> bool
{
    let mut point = POINT::default(); // 当前屏幕物理坐标。
    let mut rect = RECT::default(); // 最终窗口客户区物理矩形。
    unsafe { GetAsyncKeyState(VK_ESCAPE.0 as i32) >= 0 && GetCursorPos(&mut point).is_ok() && ScreenToClient(hwnd, &mut point).as_bool() && GetClientRect(hwnd, &mut rect).is_ok() && point_inside_client(point, rect) }
}

/// 常态与 hover 均可进入原生移动循环，展开态只允许卡片排序，不移动窗口。
pub(crate) fn begin_drag() -> Result<std::sync::mpsc::Receiver<IslandDragResult>, String>
{
    let (done, saved) = std::sync::mpsc::sync_channel(1); // UI线程只传递完成信号，不等待磁盘。
    let (hwnd, sequence) = ISLAND.with(|slot| {
        let mut slot = slot.borrow_mut(); // 开始一次当前修订的移动循环，随后立即释放借用。
        let resource = slot.as_mut().ok_or("岛未开启")?;
        if !can_drag(resource.geometry.as_ref())
        {
            return Err("展开态不能拖动岛窗口，请先收起卡片".to_string());
        }
        let mut order = LANDINGS.lock().map_err(|_| "岛落点状态不可用")?; // 序号分配与后台写盘线性化，不跨 Win32 调用持锁。
        resource.drag.begin()?;
        resource.drag.landing_seq = order.begin()?;
        resource.drag.landing_done = Some(done);
        Ok((resource.hwnd, resource.drag.landing_seq))
    })?;
    CLICK_FOREGROUND.with(|state| state.borrow_mut().target = None); // 原生移动循环不能执行排队的点击补偿。
    NATIVE_DRAGGING.set(true);
    unsafe {
        let _ = ReleaseCapture();
        SendMessageW(hwnd, WM_NCLBUTTONDOWN, Some(WPARAM(HTCAPTION as usize)), Some(LPARAM(0)));
    }
    NATIVE_DRAGGING.set(false);
    ISLAND.with(|slot| {
        if let Some(resource) = slot.borrow_mut().as_mut().filter(|resource| resource.hwnd == hwnd && resource.drag.landing_seq == sequence)
        {
            resource.drag.landing_done = None; // 没有成功调度保存的取消/失败移动不让命令空等。
        }
    });
    Ok(saved)
}

/// 仅命令后台线程等待保存；超时不改变已完成的原生拖动结果。
pub(crate) fn wait_for_landing(saved: std::sync::mpsc::Receiver<IslandDragResult>, timeout: std::time::Duration) -> IslandDragResult
{
    match saved.recv_timeout(timeout)
    {
        Ok(result) => result,
        Err(error) =>
        {
            match error
            {
                std::sync::mpsc::RecvTimeoutError::Timeout => crate::log_warn!("岛：等待拖动落点保存超时，后台任务继续，不改变拖动结果"),
                std::sync::mpsc::RecvTimeoutError::Disconnected => crate::log_debug!("岛：本次拖动未调度落点保存或已取消，无需等待"),
            }
            IslandDragResult::default()
        }
    }
}

/// 原生移动循环结束后按胶囊而非联合盒中心吸附，并异步保存偏好。
fn finish_drag(hwnd: HWND) -> Result<(), String>
{
    let snapshot = ISLAND.with(|slot| {
        let mut slot = slot.borrow_mut(); // 消费一次落点资格，再释放借用执行原生操作。
        let resource = slot.as_mut().filter(|resource| resource.hwnd == hwnd)?;
        if !resource.drag.take_commit()
        {
            return None;
        } // 重复退出、非拖动态或旧修订均无提交。
        Some((resource.app.clone(), Rc::clone(&resource.webview), resource.geometry.clone()?, resource.drag.revision, resource.drag.landing_seq, resource.drag.landing_done.take()))
    });
    let Some((app, webview, geometry, revision, landing_seq, done)) = snapshot
    else
    {
        return Ok(());
    };
    let monitor = primary_monitor()?; // 读取最新工作区也不持有资源借用。
    let scale = monitor.scale_factor;
    let mut rect = RECT::default(); // HWND 实际屏幕物理坐标。
    unsafe { GetWindowRect(hwnd, &mut rect) }.map_err(|error| error.to_string())?;
    let active = geometry.cards.iter().map(|card| card.rect).reduce(union_box).ok_or("岛区域为空")?;
    let left = f64::from(rect.left) / scale + active.left;
    let top = f64::from(rect.top) / scale + active.top;
    let area = work_dip(&monitor);
    let edge = snap_edge(left + active.width / 2.0, top + active.height / 2.0, area);
    let offset = clamp_offset(edge, if matches!(edge, IslandEdge::Top | IslandEdge::Bottom) { left - area.left } else { top - area.top }, (active.width, active.height), area);
    let placed = place(edge, offset, (active.width, active.height), area);
    let x = placed.left - active.left; // 联合盒原点随胶囊平移。
    let y = placed.top - active.top;
    unsafe { SetWindowPos(hwnd, Some(HWND_TOPMOST), px(x, scale), px(y, scale), 0, 0, SWP_NOSIZE | SWP_NOACTIVATE) }.map_err(|error| error.to_string())?; // 无借用，重复 WM_EXITSIZEMOVE 会直接返回。
    let committed = ISLAND.with(|slot| {
        let mut slot = slot.borrow_mut(); // 不把过期原生操作提交给重建后的新窗口。
        let Some(resource) = slot.as_mut().filter(|resource| Rc::ptr_eq(&resource.webview, &webview) && resource.drag.revision == revision)
        else
        {
            return false;
        };
        let Some(geometry) = resource.geometry.as_mut()
        else
        {
            return false;
        };
        geometry.window_box.left = x;
        geometry.window_box.top = y;
        resource.monitor = monitor.clone();
        true
    });
    if !committed
    {
        return Ok(());
    }
    let pointer_inside = cursor_inside_client(hwnd); // 吸附成功后的释放事实先采样，后台磁盘等待期间不重读鼠标。
    crate::log_info!("岛：拖动吸附，显示器={}，边={}，偏移DIP={}，窗口原点DIP=[{}, {}]", monitor.id, edge.as_preference(), offset, x, y);
    tauri::async_runtime::spawn_blocking(move || {
        let result = (|| {
            let state = app.state::<SettingsState>();
            let saved = LANDINGS.lock().map_err(|_| "岛落点状态不可用")?.save(&state, landing_seq, edge, offset)?; // 同锁检查最新序号并写盘，不再重存整份旧偏好。
            if let Some(saved) = saved
            {
                app.emit("preferences-changed", &saved).map_err(|error| format!("岛落点已保存，但偏好事件发布失败：{error}"))?;
                return Ok(pointer_inside);
            }
            Ok::<bool, String>(false) // 过期序号或关闭后丢弃，不误报指针仍在岛内。
        })();
        if let Err(error) = &result
        {
            crate::log_warn!("岛：保存拖动位置失败，错误：{error}");
        }
        if let Some(done) = done
        {
            let _ = done.send(IslandDragResult { pointer_inside: result.unwrap_or(false) });
        } // 保存/广播成功、过期丢弃或错误均释放命令屏障，错误不改变拖动结果。
    });
    Ok(())
}

/// 不抢焦点；松手吸附由 Windows 原生移动循环触发。
unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT
{
    if message == WM_ERASEBKGND
    {
        return LRESULT(1);
    } // 不覆盖 WebView2 的透明像素。
    if message == WM_ISLAND_CHILD_NOACTIVATE
    {
        apply_child_noactivate(hwnd);
        return LRESULT(0);
    }
    if message == WM_PARENTNOTIFY && matches!((wparam.0 & 0xffff) as u32, WM_CREATE | WM_DESTROY)
    {
        schedule_child_noactivate(hwnd); // 后代重建后重做，不只修复首次创建的子窗口。
    }
    if message == WM_MOUSEACTIVATE
    {
        let previous = unsafe { GetForegroundWindow() }.0 as isize; // 某些 WebView2 路径不发此消息，激活时另用 lparam 兜底。
        CLICK_FOREGROUND.with(|state| state.borrow_mut().target = (previous != 0 && previous != hwnd.0 as isize).then_some(previous));
        return LRESULT(MA_NOACTIVATE as isize);
    }
    if message == WM_ACTIVATE
    {
        let activation = (wparam.0 & 0xffff) as u32;
        let clicking = unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) } < 0; // 不补偿程序切换、键盘激活或过期记录。
        if matches!(activation, WA_ACTIVE | WA_CLICKACTIVE) && clicking && !native_dragging()
        {
            CLICK_FOREGROUND.with(|state| {
                let mut state = state.borrow_mut();
                if state.target.is_none()
                {
                    state.target = (lparam.0 != 0 && lparam.0 != hwnd.0 as isize).then_some(lparam.0);
                }
            });
            if let Err(error) = unsafe { PostMessageW(Some(hwnd), WM_ISLAND_RESTORE_FOREGROUND, WPARAM(0), LPARAM(0)) }
            {
                CLICK_FOREGROUND.with(|state| state.borrow_mut().target = None);
                crate::log_warn!("岛：点击前台恢复调度失败，错误：{error}");
            }
        }
        else
        {
            CLICK_FOREGROUND.with(|state| state.borrow_mut().target = None);
        }
    }
    if message == WM_ISLAND_RESTORE_FOREGROUND
    {
        restore_click_foreground(hwnd);
        return LRESULT(0);
    }
    if message == WM_ENTERSIZEMOVE
    {
        NATIVE_DRAGGING.set(true);
        CLICK_FOREGROUND.with(|state| state.borrow_mut().target = None);
    }
    if message == WM_EXITSIZEMOVE
    {
        NATIVE_DRAGGING.set(false);
    }
    if message == WM_EXITSIZEMOVE
        && let Err(error) = finish_drag(hwnd)
    {
        crate::log_warn!("岛：拖动吸附失败，错误：{error}");
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

#[cfg(test)]
mod reentrancy_tests
{
    use super::*;

    #[test]
    fn island_state_failure_injection_always_clears_geometry_and_disables_drag()
    {
        let previous = IslandStateRequest { state: "collapsed".into(), window_box: IslandRect { left: 0.0, top: 0.0, width: 100.0, height: 40.0 }, cards: vec![] }; // 不创建真实 HWND。
        for stage in [StateStage::WindowBox, StateStage::WebView, StateStage::Commit]
        {
            let mut geometry = Some(previous.clone());
            let mut drag = DragProgress::default();
            drag.begin().unwrap();
            let revision = drag.revision;
            let mut hidden = 0;
            let result = run_transition(
                previous.clone(),
                |_| Err(format!("注入 {stage:?} 失败")),
                |_, update| {
                    update(&mut geometry, &mut drag);
                    Ok(())
                },
                || hidden += 1,
            );
            assert!(result.is_err());
            assert!(geometry.is_none(), "生产错误分支必须清空几何");
            assert_ne!(drag.revision, revision, "生产错误分支必须 invalidate");
            assert!(drag.pending.is_none());
            assert_eq!(hidden, 1, "生产错误分支必须且只能隐藏一次");
            assert!(!can_drag(geometry.as_ref()));
        }
        let mut geometry = None;
        let mut drag = DragProgress::default();
        let mut hidden = 0;
        run_transition(
            previous.clone(),
            |_| Ok(()),
            |_, update| {
                update(&mut geometry, &mut drag);
                Ok(())
            },
            || hidden += 1,
        )
        .unwrap();
        assert_eq!(geometry.as_ref().unwrap().window_box, previous.window_box);
        assert!(can_drag(geometry.as_ref()));
        assert_eq!(hidden, 0, "生产成功分支不能隐藏");
    }

    #[test]
    fn island_landing_order_drops_old_jobs_and_preserves_other_preferences()
    {
        let directory = std::env::temp_dir().join(format!("desktop-tools-island-order-{}", std::process::id())); // 本测试独占磁盘状态。
        let path = directory.join("settings.json");
        let state = SettingsState::load(path.clone()).unwrap();
        state
            .update(|preferences| {
                preferences.island_enabled = true;
                preferences.weather_enabled = true;
                Ok(())
            })
            .unwrap();
        let mut order = LandingOrder::default(); // 独立序号器，不污染运行时全局锁。
        let old = order.begin().unwrap();
        let new = order.begin().unwrap();
        assert!(order.save(&state, old, IslandEdge::Left, 1.0).unwrap().is_none());
        assert!(order.save(&state, new, IslandEdge::Right, 22.0).unwrap().is_some());
        let bytes = std::fs::read(&path).unwrap();
        assert!(order.save(&state, old, IslandEdge::Left, 1.0).unwrap().is_none());
        assert!(order.save(&state, new, IslandEdge::Left, 1.0).unwrap().is_none());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        let saved = state.preferences().unwrap();
        assert!(saved.weather_enabled);
        assert_eq!(saved.island_edge, "right");
        assert_eq!(saved.island_offset, 22.0);
        let disabled_job = order.begin().unwrap();
        state
            .update(|preferences| {
                preferences.island_enabled = false;
                preferences.weather_enabled = false;
                Ok(())
            })
            .unwrap();
        let disabled_bytes = std::fs::read(&path).unwrap();
        assert!(order.save(&state, disabled_job, IslandEdge::Bottom, 40.0).unwrap().is_none());
        assert_eq!(std::fs::read(&path).unwrap(), disabled_bytes);
        assert!(!state.preferences().unwrap().weather_enabled);
        assert_eq!(SettingsState::load(path).unwrap().preferences().unwrap(), state.preferences().unwrap());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn island_drag_snapshot_releases_borrow_and_commits_only_once()
    {
        let progress = RefCell::new(DragProgress::default()); // 不创建 HWND，也不需要 Windows 消息循环。
        assert!(!progress.borrow_mut().take_commit());
        progress.borrow_mut().begin().unwrap();
        assert!(progress.borrow_mut().begin().is_err());
        let snapshot = {
            let mut progress = progress.borrow_mut();
            progress.take_commit().then_some(progress.revision)
        }; // 与 finish_drag 相同：先消费资格并返回值，再执行副作用。
        let mut commits = 0; // 模拟 SetWindowPos 同步重入 WM_EXITSIZEMOVE。
        if let Some(revision) = snapshot
        {
            assert!(!progress.borrow_mut().take_commit()); // 能再次可变借用，且重复退出没有第二次提交资格。
            if progress.borrow().revision == revision
            {
                commits += 1;
            }
        }
        assert!(!progress.borrow_mut().take_commit());
        assert_eq!(commits, 1);
        progress.borrow_mut().begin().unwrap();
        progress.borrow_mut().invalidate(); // 切态使旧移动循环失效。
        assert!(!progress.borrow_mut().take_commit());
        progress.borrow_mut().begin().unwrap();
        let revision = {
            let mut progress = progress.borrow_mut();
            assert!(progress.take_commit());
            progress.revision
        };
        progress.borrow_mut().invalidate(); // 模拟副作用期间重入新的几何提交。
        assert_ne!(progress.borrow().revision, revision); // 旧修订不能提交到新状态。
    }
}

#[cfg(test)]
#[path = "island.test.rs"]
mod tests;
