use crate::{app::settings::SettingsState, contracts::DesktopHostMonitor};
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    num::NonZeroIsize,
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering},
    },
};
use tauri::{AppHandle, Emitter, Listener, Manager};
use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_CLASS_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SetLastError, WIN32_ERROR, WPARAM},
        Graphics::{
            Dwm::{DWM_BB_BLURREGION, DWM_BB_ENABLE, DWM_BLURBEHIND, DwmEnableBlurBehindWindow},
            Gdi::{CreateRectRgn, DeleteObject, ScreenToClient},
        },
        System::{
            LibraryLoader::GetModuleHandleW,
            Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW},
        },
        UI::{
            Input::KeyboardAndMouse::{GetAsyncKeyState, ReleaseCapture, VK_ESCAPE, VK_LBUTTON},
            WindowsAndMessaging::*,
        },
    },
    core::{PCWSTR, PWSTR, w},
};
use wry::{
    Rect as WryRect, WebView, WebViewBuilder, WebViewBuilderExtWindows, dpi,
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

    /// 返回显式贴边偏好；空串由偏好调用方按顶部处理，拖动不强制固化。
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
#[allow(dead_code)] // 贴边方向改由设置决定，保留纯函数供几何测试。
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

/// 沿边方向的中心偏移夹紧：0 表示居中，范围是相对居中可移动的半幅距离。
pub fn clamp_offset(edge: IslandEdge, offset: f64, size: (f64, f64), work: IslandRect) -> f64
{
    let span = match edge
    {
        IslandEdge::Top | IslandEdge::Bottom => (work.width - size.0).max(0.0),
        IslandEdge::Left | IslandEdge::Right => (work.height - size.1).max(0.0),
    }; // 沿边可移动总距离；负值表示岛比工作区长。
    offset.clamp(-span / 2.0, span / 2.0)
}

/// 岛形态到贴边方向的间隔（DIP）：胶囊离边 6，刘海贴边 0。
pub fn island_margin(form: &str) -> f64 { if form == "notch" { 0.0 } else { 6.0 } }

/// 将岛贴到指定工作区边并在沿边方向居中；`offset` 是相对居中的偏移，`margin` 是贴边方向的间隔。
pub fn place(edge: IslandEdge, offset: f64, size: (f64, f64), work: IslandRect, margin: f64) -> IslandRect
{
    let offset = clamp_offset(edge, offset, size, work); // 统一夹紧持久化与实时偏移。
    let center_x = (work.width - size.0).max(0.0) / 2.0; // 尺寸超限时不居中，保持工作区起点。
    let center_y = (work.height - size.1).max(0.0) / 2.0;
    let margin = margin.max(0.0);
    let (left, top) = match edge
    {
        IslandEdge::Top => (work.left + center_x + offset, work.top + margin),
        IslandEdge::Bottom => (work.left + center_x + offset, work.top + (work.height - size.1).max(0.0) - margin),
        IslandEdge::Left => (work.left + margin, work.top + center_y + offset),
        IslandEdge::Right => (work.left + (work.width - size.0).max(0.0) - margin, work.top + center_y + offset),
    }; // 工作区绝对坐标，允许多显示器负坐标；贴边侧固定，另一侧向屏幕中心展开。
    IslandRect { left: left.max(work.left), top: top.max(work.top), width: size.0, height: size.1 }
}

/// 按偏好放置；空串按顶部应用同一偏移，不把空串固化为显式 top。
pub fn place_from_preference(edge: &str, offset: f64, size: (f64, f64), work: IslandRect, margin: f64) -> Option<IslandRect>
{
    let parsed = IslandEdge::from_preference(edge)?; // 未知字符串由边界调用方处理错误。
    Some(place(parsed, offset, size, work, margin))
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

/// 一次状态切换的完整几何；window_box 为屏幕绝对 DIP 窗口盒，viewport 为固定页面原点视口。
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct IslandStateRequest
{
    pub state: String,
    pub window_box: IslandRect,
    pub viewport: IslandRect,
    pub cards: Vec<IslandCard>,
}

impl IslandStateRequest
{
    /// IPC 不接受非有限数、无界区域、盒外卡片、超出视口的窗口或未知状态。
    pub(crate) fn validate(&self) -> Result<(), String>
    {
        if !matches!(self.state.as_str(), "collapsed" | "hover" | "expanded")
        {
            return Err("未知岛状态".into());
        }
        if !valid_rect(self.window_box) || !valid_rect(self.viewport) || self.viewport.width > 16_384.0 || self.viewport.height > 16_384.0 || self.cards.is_empty() || self.cards.len() > 128
        {
            return Err("岛窗口盒、视口或卡片数量无效".into());
        }
        if !window_in_viewport(self.window_box, self.viewport)
        {
            return Err("岛窗口盒超出固定视口".into());
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

/// 规范化只允许量化级位移；原点被真实移动时必须拒绝，让前端按最新工作区重测。
fn accept_normalized_window(requested: IslandRect, normalized: IslandRect, viewport: IslandRect) -> Result<(), String>
{
    if (normalized.left - requested.left).abs() > 0.5 || (normalized.top - requested.top).abs() > 0.5
    {
        return Err("岛窗口盒被原生规范化移动，需要按最新工作区重测".into());
    }
    if !window_in_viewport(normalized, viewport)
    {
        return Err("岛窗口盒在落位后超出固定视口".into());
    }
    Ok(())
}

/// 工作区规范化：窗口左上夹到工作区内；尺寸已先行检查不超过工作区。
fn clamp_window(window: IslandRect, area: IslandRect) -> IslandRect
{
    IslandRect { left: window.left.clamp(area.left, area.left + area.width - window.width), top: window.top.clamp(area.top, area.top + area.height - window.height), width: window.width, height: window.height }
}

/// 窗口盒必须落在固定视口内（0.5 DIP 容差），页面坐标才不会因落位/夹紧错位。
fn window_in_viewport(window: IslandRect, viewport: IslandRect) -> bool
{
    window.left >= viewport.left - 0.5 && window.top >= viewport.top - 0.5 && window.left + window.width <= viewport.left + viewport.width + 0.5 && window.top + window.height <= viewport.top + viewport.height + 0.5
}

/// 防止浮点到 Win32 整数转换溢出；实际工作区边界在 UI 线程进一步检查。
fn valid_rect(rect: IslandRect) -> bool { [rect.left, rect.top, rect.width, rect.height].iter().all(|value| value.is_finite() && value.abs() <= 1_000_000.0) && rect.width > 0.0 && rect.height > 0.0 }

/// 最近一次真正下发的原生尺寸；同一窗口盒的动画帧不能重复触发 SetWindowPos/SetBounds。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct AppliedSurface
{
    window: Option<(i32, i32, i32, i32)>, // 外层窗口物理位置与尺寸。
    child: Option<(i32, i32, i32, i32)>,  // WebView 子窗口相对父窗口的物理位置与尺寸；固定视口随窗口反向偏移。
}

impl AppliedSurface
{
    /// 计算本次需要下发的窗口/子窗口尺寸并返回下一份已应用快照；重复几何不重复调用原生。
    fn plan(self, window: (i32, i32, i32, i32), child: (i32, i32, i32, i32)) -> (Self, bool, bool)
    {
        (Self { window: Some(window), child: Some(child) }, self.window != Some(window), self.child != Some(child))
    }

    /// 原生移动循环会改变窗口位置：失效窗口缓存，子窗口在下次提交按固定视口重新计算偏移。
    fn forget_window(&mut self) { self.window = None; }
}

/// 固定视口的子窗口局部矩形：屏幕位置始终等于 viewport 原点，父窗口移动时用偏移抵消。
fn child_bounds(window: IslandRect, viewport: IslandRect, scale: f64) -> (i32, i32, i32, i32)
{
    let (viewport_left, viewport_width) = px_span(viewport.left, viewport.width, scale);
    let (viewport_top, viewport_height) = px_span(viewport.top, viewport.height, scale);
    (viewport_left - px(window.left, scale), viewport_top - px(window.top, scale), viewport_width, viewport_height)
}

/// 一次取整得到物理起止边；宽高由同一对取整边界相减，避免左右/上下各自四舍五入产生 1px 缝。
fn px_span(start: f64, length: f64, scale: f64) -> (i32, i32)
{
    let first = px(start, scale);
    (first, (px(start + length, scale) - first).max(1))
}

/// UI 线程唯一持有的岛资源，不把 WebView 或 HWND 跨线程执行。
struct IslandWindow
{
    hwnd: HWND,                           // 外层不激活工具窗口。
    webview: Rc<WebView>,                 // UI 线程可克隆快照，在借用期外调整 Controller。
    app: AppHandle,                       // 松手保存偏好所需应用状态。
    monitor: DesktopHostMonitor,          // 主屏当前工作区与缩放。
    geometry: Option<IslandStateRequest>, // 首次状态到达前禁止显示。
    drag: DragProgress,                   // 几何修订使旧移动循环失效。
    surface: AppliedSurface,              // 最近一次实际下发的原生尺寸，重复帧只更新页面内容。
    screenshot_hidden: bool,              // 本次隐藏是否因系统截图工具在前台；显示事实直接读 Win32。
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
            if !(preferences.island_edge.is_empty() && edge == IslandEdge::Top)
            {
                preferences.island_edge = edge.as_preference().into(); // 默认空边拖动后仍保留空串，不固化为显式 top。
            }
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
static ISLAND_HWND: AtomicIsize = AtomicIsize::new(0); // 键盘钩子线程只读取 HWND 数值，不跨线程碰 WebView。
static SCREENSHOT_SEEN: AtomicBool = AtomicBool::new(false); // 热键后是否已观察到截图工具真正进入前台。
static SCREENSHOT_PENDING: AtomicBool = AtomicBool::new(false); // 热键已提前隐藏，等待截图工具出现或超时。
static SCREENSHOT_STARTED_MS: AtomicU64 = AtomicU64::new(0); // 热键触发的毫秒时间，供计时器判断是否超时。
const SCREENSHOT_TIMER_ID: usize = 0x4953; // 截图隐藏期间的兜底恢复计时器。
const SCREENSHOT_TIMEOUT_MS: u64 = 1200; // 热键后等待截图工具出现的上限。
const WM_ISLAND_SCREENSHOT_HOTKEY: u32 = WM_APP + 39; // 钩子线程请求 UI 线程安装截图计时器。

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
                // 关闭只隐藏并保留窗口：关闭后立刻重开会重复创建 WebView2，环境交接会卡住 UI 线程；进程退出时统一释放。
                apply_policy(false);
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
    crate::log_debug!("岛：开始创建原生窗口");
    let monitor = primary_monitor()?; // 从原生 rcWork 获取当前主屏。
    crate::log_debug!("岛：主屏工作区已读取，显示器={}", monitor.id);
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
        let mut builder = WebViewBuilder::new()
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
            .with_url(island_page_url(super::desktop_host::boot_epoch(), super::desktop_host::active_theme_stylesheet(app).as_deref()).as_str());
        // 复用正式宿主的环境：关闭后再次开启岛时不重复创建 WebView2 环境，避免环境交接卡住 UI 线程。
        let environment = super::desktop_host::shared_webview_environment();
        crate::log_debug!("岛：开始创建 WebView2，复用环境={}", environment.is_some());
        if let Some(environment) = environment
        {
            builder = builder.with_environment(environment);
        }
        let webview = builder.build_as_child(&raw).map_err(|error| format!("创建岛 WebView 失败：{error}"))?;
        crate::log_debug!("岛：WebView2 已创建");
        Ok::<WebView, String>(webview)
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
    ISLAND.with(|slot| *slot.borrow_mut() = Some(IslandWindow { hwnd, webview: Rc::new(webview), app: app.clone(), monitor, geometry: None, drag: DragProgress::default(), surface: AppliedSurface::default(), screenshot_hidden: false }));
    ISLAND_HWND.store(hwnd.0 as isize, Ordering::Release);
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
    ISLAND_HWND.store(0, Ordering::Release);
    SCREENSHOT_PENDING.store(false, Ordering::Release);
    SCREENSHOT_SEEN.store(false, Ordering::Release);
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

/// 系统截图工具的进程名；Windows 10/11 的截图浮层分属不同进程。
fn is_screenshot_process(name: &str) -> bool { matches!(name, "screenclipper.exe" | "snippingtool.exe" | "screensketch.exe") }

/// 热键隐藏是否应结束：工具已出现后离开前台，或等待超时仍未出现。
fn screenshot_hotkey_timeout(started_ms: u64, now_ms: u64, seen: bool) -> bool { started_ms != 0 && (seen || now_ms.saturating_sub(started_ms) > SCREENSHOT_TIMEOUT_MS) }

/// 截图热键在系统冻结画面之前先隐藏岛；键盘钩子线程只写原子状态并做一次隐藏。
pub(crate) fn hide_for_screenshot_hotkey()
{
    SCREENSHOT_PENDING.store(true, Ordering::Release);
    SCREENSHOT_STARTED_MS.store(now_millis(), Ordering::Release);
    SCREENSHOT_SEEN.store(false, Ordering::Release);
    let hwnd = ISLAND_HWND.load(Ordering::Acquire);
    if hwnd != 0
    {
        let hwnd = HWND(hwnd as *mut _);
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
            // SetTimer 只能由窗口所属 UI 线程安装；这里只请求 UI 线程接管隐藏与计时器。
            if let Err(error) = PostMessageW(Some(hwnd), WM_ISLAND_SCREENSHOT_HOTKEY, WPARAM(0), LPARAM(0))
            {
                crate::log_warn!("岛：截图热键调度 UI 线程失败，等待下一次策略刷新恢复：{error}");
            }
        }
    }
}

/// 当前毫秒时间；只用于截图热键超时判断，不参与业务时间。
fn now_millis() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|elapsed| elapsed.as_millis() as u64).unwrap_or(0) }

/// 截图热键是否仍处于提前隐藏状态。
fn screenshot_hotkey_pending() -> bool { SCREENSHOT_PENDING.load(Ordering::Acquire) }

/// 截图隐藏后的兜底恢复：不依赖漏发的前台事件，截图工具未出现或离开前台即恢复。
fn screenshot_timer_tick()
{
    let foreground = screenshot_foreground();
    if foreground
    {
        SCREENSHOT_SEEN.store(true, Ordering::Release);
        apply_policy(true);
        return;
    }
    let started = SCREENSHOT_STARTED_MS.load(Ordering::Acquire);
    if screenshot_hotkey_timeout(started, now_millis(), SCREENSHOT_SEEN.load(Ordering::Acquire))
    {
        SCREENSHOT_PENDING.store(false, Ordering::Release);
        SCREENSHOT_SEEN.store(false, Ordering::Release);
        crate::log_info!("岛：截图工具已离开前台或未出现，按当前偏好恢复岛窗口");
        apply_policy(true);
        return;
    }
    // 手动打开截图工具时没有热键 started；screenshot_hidden 仍由前台检测设置，计时器同样要兜底。
    let hidden_for_screenshot = ISLAND.with(|slot| slot.borrow().as_ref().is_some_and(|resource| resource.screenshot_hidden));
    if !SCREENSHOT_PENDING.load(Ordering::Acquire) && hidden_for_screenshot
    {
        apply_policy(true);
    }
}

/// 前台是系统截图工具时隐藏岛，避免截图工具触发的置顶刷新把岛拍进截图。
fn screenshot_foreground() -> bool
{
    let window = unsafe { GetForegroundWindow() };
    if window.0.is_null()
    {
        return false;
    }
    let mut process = 0u32;
    unsafe { GetWindowThreadProcessId(window, Some(&mut process)) };
    if process == 0
    {
        return false;
    }
    let handle = match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process) }
    {
        Ok(handle) => handle,
        Err(_) => return false,
    };
    let mut buffer = [0u16; 512]; // 截图工具可执行文件路径足够放下的缓冲。
    let mut size = buffer.len() as u32;
    let named = unsafe { QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut size) };
    let _ = unsafe { CloseHandle(handle) };
    if named.is_err()
    {
        return false;
    }
    let path = String::from_utf16_lossy(&buffer[..size as usize]);
    let name = path.rsplit('\\').next().unwrap_or_default().to_ascii_lowercase();
    is_screenshot_process(&name)
}

/// UI 线程使用相同快照隐藏或无激活恢复；未收首态绝不显示占位窗。
fn apply_policy(reassert_topmost: bool)
{
    let target = ISLAND.with(|slot| {
        let slot = slot.borrow(); // 仅计算副作用所需的 Copy 数据，不调用窗口 API。
        let resource = slot.as_ref()?;
        let preferences = resource.app.state::<SettingsState>().preferences().ok()?;
        let foreground = screenshot_foreground();
        if foreground
        {
            SCREENSHOT_SEEN.store(true, Ordering::Release);
        }
        else if SCREENSHOT_PENDING.load(Ordering::Acquire) && screenshot_hotkey_timeout(SCREENSHOT_STARTED_MS.load(Ordering::Acquire), now_millis(), SCREENSHOT_SEEN.load(Ordering::Acquire))
        {
            // 任何一次策略刷新都能自愈：工具未出现且已超时，清掉提前隐藏状态。
            SCREENSHOT_PENDING.store(false, Ordering::Release);
            SCREENSHOT_SEEN.store(false, Ordering::Release);
        }
        let screenshot = foreground || screenshot_hotkey_pending();
        let hide = !preferences.island_enabled || resource.geometry.is_none() || suppressed(&super::refresh_policy::foreground_snapshot(), preferences.island_hide_on_fullscreen) || screenshot;
        // 钩子线程可能已直接隐藏窗口，显示判定必须读取 Win32 实际状态而不是缓存布尔。
        let was_visible = unsafe { IsWindowVisible(resource.hwnd).as_bool() };
        Some((resource.hwnd, hide, was_visible, screenshot, resource.screenshot_hidden, foreground))
    }); // RefCell 借用到此结束，隐藏中重入 finish_drag 也可以取得可变借用。
    if let Some((hwnd, hide, was_visible, screenshot, screenshot_hidden, foreground)) = target
    {
        if screenshot && !screenshot_hidden && hide
        {
            if foreground
            {
                crate::log_info!("岛：系统截图工具进入前台，隐藏岛窗口");
            }
            else
            {
                crate::log_info!("岛：截图热键触发，提前隐藏岛窗口");
            }
        }
        else if !screenshot && screenshot_hidden
        {
            crate::log_info!("岛：系统截图工具离开前台，按当前偏好恢复岛窗口");
        }
        unsafe {
            // 已可见时不再重复 ShowWindow：截图工具等外部事件只刷新前台事实，不能制造一次重绘。
            if hide && was_visible
            {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            else if !hide && !was_visible
            {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            }
            if !hide && reassert_topmost
            {
                let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
            if screenshot
            {
                let _ = SetTimer(Some(hwnd), SCREENSHOT_TIMER_ID, 300, None);
            }
            else
            {
                let _ = KillTimer(Some(hwnd), SCREENSHOT_TIMER_ID);
            }
        }
        // ShowWindow 可能同步重入，显示状态在原生调用结束后再回写。
        ISLAND.with(|slot| {
            if let Some(resource) = slot.borrow_mut().as_mut().filter(|resource| resource.hwnd == hwnd)
            {
                resource.screenshot_hidden = screenshot;
            }
        });
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
                resource.surface = AppliedSurface::default();
                resource.screenshot_hidden = false;
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
    let requested = request.window_box; // 页面已使用的权威窗口位置；规范化不能静默移动它，否则页面 margin 与父窗口裁剪错位。
    let (hwnd, webview, revision, state_changed, surface) = ISLAND.with(|slot| {
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
            // 按页面提交的窗口盒尺寸落位：刘海窗口已含沿边圆弧区，不能再用卡片联合盒尺寸。
            let placed = place_from_preference(&preferences.island_edge, preferences.island_offset, (request.window_box.width, request.window_box.height), area, island_margin(&preferences.island_form)).ok_or("岛贴边偏好无效")?;
            request.window_box.left = placed.left;
            request.window_box.top = placed.top;
        }
        if request.window_box.width > area.width || request.window_box.height > area.height
        {
            return Err("岛窗口盒超过主屏工作区".into());
        }
        if px(request.window_box.width, scale) < 1 || px(request.window_box.height, scale) < 1 || request.cards.iter().any(|card| px(card.rect.left + card.rect.width, scale) <= px(card.rect.left, scale) || px(card.rect.top + card.rect.height, scale) <= px(card.rect.top, scale))
        {
            return Err("岛区域不足一个物理像素".into());
        }
        request.window_box = clamp_window(request.window_box, area);
        // 首次落位或工作区夹紧改变了窗口盒时，固定视口可能不再包含它；拒绝而不是让页面坐标错位。
        accept_normalized_window(requested, request.window_box, request.viewport)?;
        let state_changed = resource.geometry.as_ref().is_none_or(|previous| previous.state != request.state); // 动画同态帧不重复记录 INFO。
        resource.drag.invalidate(); // 先使旧拖动失效，再执行可能重入的窗口操作。
        resource.geometry = None; // 提交完成前也拒绝按上一态开始拖动。
        Ok::<_, String>((resource.hwnd, Rc::clone(&resource.webview), resource.drag.revision, state_changed, resource.surface))
    })?; // 从此开始不持有 ISLAND 的任何借用。
    let scale = monitor.scale_factor;
    // 本次提交的真实物理尺寸；窗口盒相同的动画帧不再重复下发。
    let (physical_left, physical_width) = px_span(request.window_box.left, request.window_box.width, scale);
    let (physical_top, physical_height) = px_span(request.window_box.top, request.window_box.height, scale);
    let physical = (physical_left, physical_top, physical_width, physical_height);
    let viewport = request.viewport; // 固定页面原点：子窗口屏幕位置不随父窗口移动。
    let child = child_bounds(request.window_box, viewport, scale); // 子窗口相对父窗口的位置；父窗口移动时反向偏移。
    let (next_surface, resize_window, resize_child) = surface.plan(physical, child);
    let stage = std::cell::Cell::new(StateStage::WindowBox); // 多闭包共享阶段，不持有 RefCell 借用。
    let result = run_transition(
        request,
        |request| {
            let rect = request.window_box; // 本次静默态或动画帧对应的物理窗口盒。
            stage.set(StateStage::WindowBox);
            if resize_window
            {
                unsafe { SetWindowPos(hwnd, Some(HWND_TOPMOST), physical.0, physical.1, physical.2, physical.3, SWP_NOACTIVATE) }.map_err(|error| format!("设置岛窗口盒失败：{error}"))?;
            }
            stage.set(StateStage::WebView);
            if resize_child
            {
                let bounds = next_surface.child.unwrap_or(child);
                webview.set_bounds(WryRect { position: dpi::PhysicalPosition::new(bounds.0, bounds.1).into(), size: dpi::PhysicalSize::new(bounds.2.max(1) as u32, bounds.3.max(1) as u32).into() }).map_err(|error| format!("设置岛 WebView 尺寸失败：{error}"))?;
            }
            // 每帧提交日志刷屏，排查时再打开。
            // crate::log_debug!("岛：提交帧 状态={}，窗口盒DIP=[{}, {}, {}, {}]，卡片DIP=[{}]", request.state, rect.left, rect.top, rect.width, rect.height, request.cards.iter().map(|card| format!("[{}, {}, {}, {}]圆角{}", card.rect.left, card.rect.top, card.rect.width, card.rect.height, card.radius)).collect::<Vec<_>>().join("；"));
            if state_changed
            {
                crate::log_debug!("岛：切换状态={}，显示器={}，窗口盒DIP=[{}, {}, {}, {}]，缩放={}", request.state, monitor.id, rect.left, rect.top, rect.width, rect.height, scale);
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
                    resource.surface = next_surface;
                    resource.monitor = monitor.clone();
                }
                else
                {
                    resource.surface = AppliedSurface::default(); // 原生操作可能已部分生效，失败后不能沿用缓存尺寸。
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
    pub offset: Option<f64>,  // 本次拖动后的沿边居中偏移；取消、超时或未吸附时为 None。
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
        resource.surface.forget_window(); // 原生移动循环会改窗口位置，旧缓存盒立即失效，下次提交必须重新定位。
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
    let active = geometry.cards.iter().map(|card| card.rect).reduce(union_box).ok_or("岛区域为空")?;
    let area = work_dip(&monitor);
    let mut window = RECT::default(); // 拖动后的窗口位置；松手不再以鼠标为中心重新定位，避免跳变。
    unsafe { GetWindowRect(hwnd, &mut window) }.map_err(|error| error.to_string())?;
    let window_left = f64::from(window.left) / scale; // 物理像素统一换算到 DIP。
    let window_top = f64::from(window.top) / scale;
    let center_x = window_left + active.left + active.width / 2.0; // 卡片中心跟随窗口，不跟随鼠标按下点。
    let center_y = window_top + active.top + active.height / 2.0;
    let preferences = app.state::<SettingsState>().preferences()?; // 贴边方向只由设置决定，拖动只沿当前边移动。
    let edge = IslandEdge::from_preference(&preferences.island_edge).unwrap_or(IslandEdge::Top);
    let raw = match edge
    {
        IslandEdge::Top | IslandEdge::Bottom => center_x - (area.left + area.width / 2.0), // 沿边偏移 = 卡片中心相对工作区中心。
        IslandEdge::Left | IslandEdge::Right => center_y - (area.top + area.height / 2.0),
    };
    let card = geometry.cards.first().map(|card| card.rect).ok_or("岛区域为空")?;
    let notch_extra = if preferences.island_form == "notch" && geometry.state != "expanded" { 24.0 } else { 0.0 };
    let clamp_size = match edge
    {
        IslandEdge::Top | IslandEdge::Bottom => (card.width + notch_extra, card.height),
        IslandEdge::Left | IslandEdge::Right => (card.width, card.height + notch_extra),
    };
    let offset = clamp_offset(edge, raw, clamp_size, area); // 与页面 islandGeometry 的卡片+刘海夹紧口径一致。
    // 不在这里移动窗口：跨边时旧态尺寸算出的位置会和页面按新边重测的结果打架，吸附位置统一由页面提交。
    let committed = ISLAND.with(|slot| {
        let mut slot = slot.borrow_mut(); // 不把过期原生操作提交给重建后的新窗口。
        let Some(resource) = slot.as_mut().filter(|resource| Rc::ptr_eq(&resource.webview, &webview) && resource.drag.revision == revision)
        else
        {
            return false;
        };
        resource.monitor = monitor.clone();
        true
    });
    if !committed
    {
        return Ok(());
    }
    let pointer_inside = cursor_inside_client(hwnd); // 释放事实先采样，后台磁盘等待期间不重读鼠标。
    crate::log_info!("岛：拖动落点，显示器={}，边={}，偏移DIP={}", monitor.id, edge.as_preference(), offset);
    if let Some(done) = done
    {
        let _ = done.send(IslandDragResult { pointer_inside, offset: Some(offset) }); // 先释放命令屏障，页面按同一落点重测。
    }
    tauri::async_runtime::spawn_blocking(move || {
        let result = (|| {
            let state = app.state::<SettingsState>();
            let saved = LANDINGS.lock().map_err(|_| "岛落点状态不可用")?.save(&state, landing_seq, edge, offset)?; // 同锁检查最新序号并写盘，不再重存整份旧偏好。
            if let Some(saved) = saved
            {
                app.emit("preferences-changed", &saved).map_err(|error| format!("岛落点已保存，但偏好事件发布失败：{error}"))?;
            }
            Ok::<(), String>(())
        })();
        if let Err(error) = &result
        {
            crate::log_warn!("岛：保存拖动位置失败，错误：{error}");
        }
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
    if message == WM_ISLAND_SCREENSHOT_HOTKEY
    {
        apply_policy(true); // UI 线程安装计时器并回写实际可见性。
        return LRESULT(0);
    }
    if message == WM_TIMER && wparam.0 == SCREENSHOT_TIMER_ID
    {
        screenshot_timer_tick();
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
        let previous = IslandStateRequest { state: "collapsed".into(), window_box: IslandRect { left: 0.0, top: 0.0, width: 100.0, height: 40.0 }, viewport: IslandRect { left: -10.0, top: -10.0, width: 120.0, height: 60.0 }, cards: vec![] }; // 不创建真实 HWND。
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
