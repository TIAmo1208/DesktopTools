//! 桌面层置顶弹层窗口：承载格子菜单、确认框与天气弹窗。
//!
//! 桌面宿主挂在 Explorer 的桌面 z 带里，宿主 WebView 内的 `position: fixed` 浮层只能画在桌面层。
//! 本模块按需创建一个独立置顶窗口承载这些浮层，窗口透明、无边框、不进任务栏；
//! 关闭后先隐藏并保留窗口，空闲超时再销毁以释放 WebView2 进程。

use crate::contracts::DesktopHostMonitor;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{
    LazyLock, Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use windows::Win32::Foundation::{GetLastError, HWND, SetLastError, WIN32_ERROR};
use windows::Win32::Graphics::Dwm::{DWM_BB_BLURREGION, DWM_BB_ENABLE, DWM_BLURBEHIND, DwmEnableBlurBehindWindow, DwmExtendFrameIntoClientArea, DwmFlush};
use windows::Win32::Graphics::Gdi::{CreateRectRgn, DeleteObject, RDW_ALLCHILDREN, RDW_ERASE, RDW_FRAME, RDW_INVALIDATE, RDW_UPDATENOW, RedrawWindow};
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::WindowsAndMessaging::{GWL_EXSTYLE, GWL_STYLE, GetClassNameW, GetForegroundWindow, GetWindowLongPtrW, GetWindowThreadProcessId, IsWindow, IsWindowVisible, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, WS_CAPTION, WS_EX_WINDOWEDGE, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME};

/// 记下“下次显示需要重绘一次”，由显示流程取走。
fn mark_repaint_pending()
{
    if let Ok(mut state) = STATE.lock()
    {
        state.needs_repaint = true;
    }
}

/// 隐藏失败时的兜底：直接销毁窗口，避免一个可见的置顶空窗口挡住桌面点击。
/// 销毁也失败时只记日志并交给空闲回收重试。
fn force_destroy(app: &AppHandle, window: &WebviewWindow)
{
    match window.destroy()
    {
        Ok(()) =>
        {
            if let Ok(mut state) = STATE.lock()
                && state.window.as_ref().is_some_and(|current| current.label() == window.label())
            {
                state.window = None;
            }
        }
        Err(error) =>
        {
            crate::log_error!("弹层：隐藏失败后销毁窗口仍失败，保留句柄等待空闲回收，错误={error}");
            schedule_idle_destroy(app);
        }
    }
}

/// 判断这次定位是否仍属于当前请求：状态代次与请求身份都不变才算有效。
fn frame_is_current(request_id: u64, generation: u64) -> Result<bool, String>
{
    let state = STATE.lock().map_err(|_| "弹层状态不可用")?;
    Ok(state.generation == generation && state.request.as_ref().is_some_and(|pending| pending.view.request_id == request_id))
}

/// 弹层关闭后保留窗口的空闲时长（毫秒）；超时销毁并释放 WebView2 进程。
pub(crate) const POPUP_IDLE_TIMEOUT_MS: u64 = 90_000;
/// 弹层窗口标识；同一时刻只存在一个弹层。
const POPUP_LABEL: &str = "popup-layer";
/// 宿主可监听的弹层结果事件名；必须同时登记在注入桥与转发白名单里。
pub(crate) const POPUP_RESULT_EVENT: &str = "popup-layer-result";
/// 复用已创建窗口时通知页面换请求的事件名。
const POPUP_REQUEST_EVENT: &str = "popup-layer-request-changed";
/// 单个弹层请求的序列化上限；菜单与对话框载荷远小于此值。
const POPUP_PAYLOAD_LIMIT: usize = 256 * 1024;
/// 页面上报尺寸的上限（毫秒）：超过就按打开失败收束，避免宿主永远等不到结果。
pub(crate) const POPUP_MEASURE_TIMEOUT_MS: u64 = 5_000;
/// 上报超时的轮询步长：等待期只有几秒，按固定步长检查是否已经显示。
const POPUP_MEASURE_POLL_MS: u64 = 100;

/// 弹层种类；决定页面装配哪个浮层组件。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum PopupLayerKind
{
    /// 桌面格子菜单：返回被选择项的 `indexPath`。
    Menu,
    /// 确认框：返回 `confirmed`。
    Confirm,
    /// 天气城市弹窗：返回所选城市。
    WeatherCity,
    /// 天气来源选择：返回所选来源。
    WeatherSource,
}

/// 宿主发起的弹层请求；坐标是宿主客户区 DIP，由 Rust 结合显示器换算成屏幕物理像素。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PopupLayerRequest
{
    /// 本次请求身份；结果必须回传同一值。
    pub request_id: u64,
    /// 浮层种类。
    pub kind: PopupLayerKind,
    /// 宿主页面的显示器枚举索引，与逐屏宿主创建顺序一致。
    pub monitor: usize,
    /// 锚点横坐标（宿主客户区 DIP）。
    pub anchor_x: f64,
    /// 锚点纵坐标（宿主客户区 DIP）。
    pub anchor_y: f64,
    /// 组件载荷；结构与种类对应，由页面按契约校验。
    pub payload: Value,
}

/// 锚点四周可用的 DIP 空间，页面据此决定二级菜单展开方向与最大高度。
#[derive(Clone, Copy, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PopupLayerSpace
{
    /// 锚点左侧可用宽度。
    pub left: f64,
    /// 锚点右侧可用宽度。
    pub right: f64,
    /// 锚点上方可用高度。
    pub top: f64,
    /// 锚点下方可用高度。
    pub bottom: f64,
}

/// 交给弹层页面装配的请求视图。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PopupLayerView
{
    /// 本次请求身份。
    pub request_id: u64,
    /// 浮层种类。
    pub kind: PopupLayerKind,
    /// 锚点四周可用空间。
    pub available: PopupLayerSpace,
    /// 组件载荷。
    pub payload: Value,
}

/// 页面量出内容后上报的窗口摆放：尺寸为 DIP，偏移是锚点在弹层文档中的位置。
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PopupLayerFrame
{
    /// 本次请求身份；过期上报直接丢弃。
    pub request_id: u64,
    /// 弹层内容宽度（DIP）。
    pub width: f64,
    /// 弹层内容高度（DIP）。
    pub height: f64,
    /// 锚点在文档中的横坐标（DIP）。
    pub anchor_offset_x: f64,
    /// 锚点在文档中的纵坐标（DIP）。
    pub anchor_offset_y: f64,
}

/// 弹层回传结果；宿主按 `indexPath` 找到原命令闭包执行，业务逻辑不迁进弹层。
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PopupLayerResult
{
    /// 本次请求身份。
    pub request_id: u64,
    /// 浮层种类。
    pub kind: PopupLayerKind,
    /// 菜单命令路径；非菜单结果为空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index_path: Option<Vec<usize>>,
    /// 确认框结论；非确认结果为空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirmed: Option<bool>,
    /// 弹窗确认结果；天气类弹窗使用。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    /// 用户取消或弹层失活，未选择任何命令。
    pub dismissed: bool,
    /// 菜单命令保持打开：只回传本次选择，当前请求继续有效。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_open: Option<bool>,
}

impl PopupLayerResult
{
    /// 以取消结果收束一次请求。
    fn cancelled(kind: PopupLayerKind, request_id: u64) -> Self { Self { request_id, kind, index_path: None, confirmed: None, value: None, dismissed: true, keep_open: None } }
}

/// 已校验的进行中请求；保留原始锚点与显示器索引用于定位。
#[derive(Clone, Debug)]
struct PendingRequest
{
    /// 交给页面的请求视图。
    view: PopupLayerView,
    /// 宿主页面显示器枚举索引。
    monitor: usize,
    /// 锚点横坐标（宿主客户区 DIP）。
    anchor_x: f64,
    /// 锚点纵坐标（宿主客户区 DIP）。
    anchor_y: f64,
    /// 打开前的前台窗口，关闭时按它还原。
    foreground_before: isize,
}

/// 当前弹层窗口与请求；同一时刻只有一个弹层。
#[derive(Default)]
struct PopupLayerState
{
    /// 已创建的弹层窗口；空闲超时后清空。
    window: Option<WebviewWindow>,
    /// 进行中的请求；结束时取出，重复结束不会重复发事件。
    request: Option<PendingRequest>,
    /// 弹层是否已经显示并接管焦点。
    shown: bool,
    /// 弹出前的前台窗口，用于关闭后还原。
    foreground_before: isize,
    /// 状态代次：每次换请求或收束都自增，用于识别迟到的定位与显示。
    generation: u64,
    /// 上次隐藏后窗口需要重绘一次，避免复用窗口时留白。
    needs_repaint: bool,
}

/// 窗口创建与销毁的串行锁：两者会争夺同一个窗口 label，不能交错执行。
static WINDOW_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// 弹层状态；窗口操作与请求切换都在同一把锁下串行。
static STATE: LazyLock<Mutex<PopupLayerState>> = LazyLock::new(Default::default);
/// 空闲销毁代次；任何新请求或新计时都推进它，旧计时醒来即失效。
static IDLE_GENERATION: AtomicU64 = AtomicU64::new(0);

/// 取出显示器枚举索引对应的宿主显示器。
fn monitor_at(index: usize) -> Result<DesktopHostMonitor, String> { super::enumerate_desktop_monitors()?.into_iter().nth(index).ok_or_else(|| "弹层缺少对应的显示器".to_string()) }

/// 显示器工作区的 DIP 尺寸。
fn monitor_work_dip(monitor: &DesktopHostMonitor) -> (f64, f64)
{
    let scale = if monitor.scale_factor > 0.0 { monitor.scale_factor } else { 1.0 }; // 缩放缺失时按 1 处理，不借用其他屏。
    (f64::from(monitor.work_width) / scale, f64::from(monitor.work_height) / scale)
}

/// 校验请求并换算锚点四周可用空间。
fn pending(request: PopupLayerRequest) -> Result<PendingRequest, String>
{
    if request.request_id == 0
    {
        return Err("弹层请求身份无效".to_string());
    }
    if !request.anchor_x.is_finite() || !request.anchor_y.is_finite() || request.anchor_x < 0.0 || request.anchor_y < 0.0
    {
        return Err("弹层锚点坐标无效".to_string());
    }
    if serde_json::to_vec(&request.payload).map_err(|error| format!("弹层载荷序列化失败：{error}"))?.len() > POPUP_PAYLOAD_LIMIT
    {
        return Err("弹层载荷超过上限".to_string());
    }
    let monitor = monitor_at(request.monitor)?; // 宿主客户区原点等于显示器工作区原点。
    let (width, height) = monitor_work_dip(&monitor);
    if request.anchor_x > width || request.anchor_y > height
    {
        return Err("弹层锚点超出显示器工作区".to_string());
    }
    Ok(PendingRequest {
        view: PopupLayerView { request_id: request.request_id, kind: request.kind, available: PopupLayerSpace { left: request.anchor_x, right: width - request.anchor_x, top: request.anchor_y, bottom: height - request.anchor_y }, payload: request.payload },
        monitor: request.monitor,
        anchor_x: request.anchor_x,
        anchor_y: request.anchor_y,
        foreground_before: unsafe { GetForegroundWindow() }.0 as isize, // 打开前的真实前台，测量期间的前台变化不影响还原目标。
    })
}

/// 弹层页面地址；与设置窗同源，才能复用内嵌资源与 `/plugins/` 资产。
fn popup_page_url(app: &AppHandle) -> tauri::Url
{
    let mut url = super::settings_page_url(); // 同一自定义协议来源。
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("popup-layer", "1");
        if let Some(stylesheet) = super::active_theme_stylesheet(app)
        {
            pairs.append_pair("theme-css", &stylesheet); // 首帧就挂主题样式表，避免弹层先闪默认皮肤。
        }
    }
    url
}

/// 首次请求时创建隐藏的置顶弹层窗口；页面地址只在这里生成一次，建好后记一条可诊断的来源日志。
fn create_window(app: &AppHandle, kind: PopupLayerKind) -> Result<WebviewWindow, String>
{
    let monitors = super::enumerate_desktop_monitors()?;
    let monitor = monitors.iter().find(|monitor| monitor.primary).or_else(|| monitors.first()).ok_or("弹层缺少显示器")?; // 占位尺寸只服务首帧测量。
    let scale = if monitor.scale_factor > 0.0 { monitor.scale_factor } else { 1.0 };
    let (work_width, work_height) = monitor_work_dip(monitor);
    let page_url = popup_page_url(app); // 只生成一次：boot 序号只推进一格，日志与建窗用同一地址。
    let mut builder = WebviewWindowBuilder::new(app, POPUP_LABEL, WebviewUrl::CustomProtocol(page_url.clone()))
        .title("Desktop Tools 弹层")
        .decorations(false)
        .shadow(false)
        .transparent(true)
        .background_color(tauri::window::Color(0, 0, 0, 0)) // webview 背景透明，圆角与子菜单预留区不显示底色。
        .skip_taskbar(true)
        .always_on_top(true)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .focused(false) // 页面量好尺寸后才显示并接管焦点。
        .visible(false)
        .inner_size(work_width, work_height) // 占位视口足够大，页面按真实内容再收敛。
        .position(f64::from(monitor.work_x) / scale, f64::from(monitor.work_y) / scale);
    // 复用宿主的 WebView2 环境：共享渲染进程与调试端口，也避免每屏多开一个环境。
    if let Some(environment) = super::desktop_host::shared_webview_environment()
    {
        builder = builder.with_environment(environment);
    }
    let window = builder.build().map_err(|error| format!("创建弹层窗口失败：{error}"))?;
    // 让 tao 的窗口状态记录“无边框”：显示时它会按记录重新套用样式，状态不对就会闪一帧标题栏。
    if let Err(error) = window.set_decorations(false)
    {
        crate::log_warn!("弹层：声明无边框失败，显示时可能闪一帧标题栏，错误={error}");
    }
    if let Ok(hwnd) = window.hwnd()
    {
        // 先去掉系统标题栏与关闭按钮，再启用透明合成。
        if let Err(error) = strip_system_frame(hwnd)
        {
            crate::log_warn!("弹层：去除系统边框失败，可能出现关闭按钮，错误={error}");
        }
        if let Err(error) = enable_transparent_composition(hwnd)
        {
            crate::log_warn!("弹层：透明合成启用失败，浮层可能带底色，错误={error}");
        }
    }
    let handle = app.clone(); // 失活关闭需要应用句柄。
    let event_window = window.clone(); // 事件回调里读回真实焦点，迟到事件不得关掉新请求。
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::Focused(false) = event
        {
            // 同一把锁里同时取“是否已显示”和当前请求身份，避免两次检查之间换了请求。
            let (shown, request_id) = STATE
                .lock()
                .map(|state| (state.shown, state.request.as_ref().map(|pending| pending.view.request_id)))
                .unwrap_or((false, None));
            let request_id = match (shown, request_id)
            {
                (true, Some(request_id)) => request_id,
                _ => return, // 隐藏、替换请求或尚未显示时没有可取消的请求。
            };
            if event_window.is_focused().unwrap_or(false)
            {
                return; // 窗口已经重新取得焦点：这是上一次隐藏留下的迟到事件。
            }
            // 记录拿走前台的窗口，便于回查"菜单一出现就关"一类问题。
            let foreground = unsafe { GetForegroundWindow() };
            let mut process = 0_u32;
            unsafe { GetWindowThreadProcessId(foreground, Some(&mut process)) };
            let mut class = [0_u16; 128];
            let length = unsafe { GetClassNameW(foreground, &mut class) };
            crate::log_debug!(
                "弹层：失活时前台窗口类={}，进程={}，本进程={}",
                String::from_utf16_lossy(&class[..length as usize]),
                process,
                std::process::id()
            );
            // 带上请求身份：即使这次失活属于上一次显示，也不会收束新请求。
            if let Err(error) = execute_dismiss(&handle, Some(request_id))
            {
                crate::log_warn!("弹层：失活关闭失败，错误={error}");
            }
        }
    });
    // 建好窗口就把页面来源记进日志：弹层页面取自 build/frontend，产物没重建时这条是唯一线索。
    crate::log_info!("弹层：窗口已创建，页面来源={page_url}，等待页面上报尺寸，种类={kind:?}");
    Ok(window)
}

/// 去掉系统标题栏、边框与关闭按钮。
///
/// Tauri 的 `decorations(false)` 只去掉可见标题栏，仍保留 `WS_CAPTION | WS_SYSMENU`：
/// Windows 11 会在客户区右上角画出系统关闭按钮，边框也按标题栏配色绘制。
fn strip_system_frame(hwnd: HWND) -> Result<(), String>
{
    /// 需要去掉的普通窗口样式位。
    const REMOVED: u32 = WS_CAPTION.0 | WS_THICKFRAME.0 | WS_MINIMIZEBOX.0 | WS_MAXIMIZEBOX.0 | WS_SYSMENU.0;
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
    let trimmed = style & !REMOVED;
    if trimmed != style
    {
        // SetWindowLongPtrW 返回 0 既可能成功也可能失败，必须配合 GetLastError 与读回判断。
        unsafe { SetLastError(WIN32_ERROR(0)) };
        let previous = unsafe { SetWindowLongPtrW(hwnd, GWL_STYLE, trimmed as isize) };
        let error = unsafe { GetLastError() };
        let applied = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
        if (previous == 0 && error.0 != 0) || applied != trimmed
        {
            return Err(format!("设置弹层窗口样式失败：错误={}，当前={applied:#010x}", error.0));
        }
    }
    // 扩展样式里的凸起边框同样去掉，避免边缘多一条系统描边。
    let extended = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    let extended_trimmed = extended & !WS_EX_WINDOWEDGE.0;
    if extended_trimmed != extended
    {
        unsafe { SetLastError(WIN32_ERROR(0)) };
        unsafe { SetWindowLongPtrW(hwnd, GWL_EXSTYLE, extended_trimmed as isize) };
        let error = unsafe { GetLastError() };
        let applied = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
        if applied != extended_trimmed
        {
            crate::log_warn!("弹层：去除扩展边框失败，错误={}，当前={applied:#010x}", error.0);
        }
    }
    // 边框变化必须让系统重新计算非客户区，否则关闭按钮的绘制区域仍然保留。
    unsafe { SetWindowPos(hwnd, None, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED) }.map_err(|error| error.to_string())
}

/// 复用窗口重新显示后强制重绘：隐藏期间 WebView2 可能停止出帧，直接显示会留下空白窗口。
fn refresh_composition(hwnd: HWND)
{
    if let Err(error) = enable_transparent_composition(hwnd)
    {
        crate::log_warn!("弹层：重新启用透明合成失败，错误={error}");
    }
    let _ = unsafe { RedrawWindow(Some(hwnd), None, None, RDW_INVALIDATE | RDW_ERASE | RDW_FRAME | RDW_UPDATENOW | RDW_ALLCHILDREN) }; // 返回值只表示是否入队重绘，不影响后续流程。
    let _ = unsafe { DwmFlush() };
}

/// 空模糊区域加整客户区扩展帧，让页面 CSS 圆角以外的像素参与 DWM 合成。
fn enable_transparent_composition(hwnd: HWND) -> Result<(), String>
{
    let region = unsafe { CreateRectRgn(0, 0, 0, 0) }; // 空区域：只启用合成透明，不铺系统材质。
    if region.is_invalid()
    {
        return Err("创建弹层透明合成区域失败".to_string());
    }
    let blur = DWM_BLURBEHIND { dwFlags: DWM_BB_ENABLE | DWM_BB_BLURREGION, fEnable: true.into(), hRgnBlur: region, ..Default::default() };
    let result = unsafe { DwmEnableBlurBehindWindow(hwnd, &blur) }.and_then(|()| unsafe { DwmExtendFrameIntoClientArea(hwnd, &MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 }) });
    let _ = unsafe { DeleteObject(region.into()) }; // DWM 复制区域，不转移 HRGN 所有权。
    result.map_err(|error| error.to_string())
}

/// 打开或复用弹层窗口；窗口首次创建时由页面读取请求并上报尺寸。
pub(crate) fn execute_open(app: &AppHandle, request: PopupLayerRequest) -> Result<(), String>
{
    let pending = pending(request)?;
    // 复用的窗口需要通知页面换请求；新窗口由页面加载后自行读取。
    let (reused, replaced) = {
        let mut state = STATE.lock().map_err(|_| "弹层状态不可用")?;
        if state.request.as_ref().is_some_and(|current| current.view.request_id == pending.view.request_id)
        {
            return Err("弹层请求身份重复".to_string());
        }
        let replaced = state.request.replace(pending.clone()); // 上一个未结束的请求要按取消收束。
        state.shown = false;
        state.generation = state.generation.wrapping_add(1); // 让迟到的定位与显示失效。
        if state.window.as_ref().is_some_and(|window| app.get_webview_window(window.label()).is_none())
        {
            state.window = None; // 窗口已被外部销毁，按冷启动重建。
        }
        (state.window.clone(), replaced)
    };
    if let Some(previous) = replaced
    {
        let result = PopupLayerResult::cancelled(previous.view.kind, previous.view.request_id);
        if let Err(error) = app.emit(POPUP_RESULT_EVENT, &result)
        {
            crate::log_warn!("弹层：替换请求时取消旧请求失败，请求={}，错误={error}", result.request_id);
        }
    }
    IDLE_GENERATION.fetch_add(1, Ordering::AcqRel); // 取消尚未触发的空闲销毁。
    let Some(window) = reused
    else
    {
        // 建窗与空闲销毁互斥：同一 label 不能被两次创建/销毁交错。
        let window_guard = WINDOW_LOCK.lock();
        let window = match create_window(app, pending.view.kind)
        {
            Ok(window) => window,
            Err(error) =>
            {
                // 建窗失败时撤回本次请求，避免状态里留一个没有窗口可承载的请求。
                if let Ok(mut state) = STATE.lock()
                    && state.request.as_ref().is_some_and(|current| current.view.request_id == pending.view.request_id)
                {
                    state.request = None;
                }
                return Err(error);
            }
        };
        STATE.lock().map_err(|_| "弹层状态不可用")?.window = Some(window);
        drop(window_guard); // 创建段结束，释放销毁互斥。
        start_measure_watchdog(app, pending.view.request_id);
        return Ok(());
    };
    mark_repaint_pending();
    let result = window.hide().map_err(|error| error.to_string()).and_then(|()| notify_or_reload(app, &window, &pending.view));
    if let Err(error) = result
    {
        // 页面收不到换请求就永远不会上报尺寸，必须把请求撤回并让宿主按失败收尾。
        if let Ok(mut state) = STATE.lock()
            && state.request.as_ref().is_some_and(|current| current.view.request_id == pending.view.request_id)
        {
            state.request = None;
        }
        // 失败可能停在 hide 之前：再隐藏一次；隐藏不掉就销毁，绝不留下可见的置顶空窗口挡住桌面点击。
        if window.hide().is_err()
        {
            crate::log_warn!("弹层：复用失败后仍无法隐藏，改为销毁窗口，错误={error}");
            match window.destroy()
            {
                Ok(()) =>
                {
                    if let Ok(mut state) = STATE.lock()
                        && state.window.as_ref().is_some_and(|current| current.label() == window.label())
                    {
                        state.window = None;
                    }
                }
                Err(destroy_error) =>
                {
                    // 销毁也失败：保留句柄，交给空闲回收再试；不能在窗口仍可见时把它从状态里丢掉。
                    crate::log_error!("弹层：复用失败后销毁窗口仍失败，保留句柄等待空闲回收，错误={destroy_error}");
                    schedule_idle_destroy(app);
                }
            }
        }
        else
        {
            schedule_idle_destroy(app); // 已隐藏但没有请求：按空闲时序回收。
        }
        return Err(format!("复用弹层窗口失败：{error}"));
    }
    start_measure_watchdog(app, pending.view.request_id);
    Ok(())
}

/// 复用窗口时的换请求方式。
///
/// 调试构建下前端会被反复重建，复用窗口若只发事件会继续跑旧 bundle；这里改为重新导航到
/// 带新 `boot` 序号的入口页，保证页面就是当前构建产物。发布构建只发事件，省掉一次页面加载。
fn notify_or_reload(app: &AppHandle, window: &WebviewWindow, view: &PopupLayerView) -> Result<(), String>
{
    if cfg!(debug_assertions)
    {
        return window.navigate(popup_page_url(app)).map_err(|error| format!("重新加载弹层页面失败：{error}"));
    }
    let _ = view;
    window.emit(POPUP_REQUEST_EVENT, view).map_err(|error| error.to_string())
}

/// 页面读取当前请求；窗口刚创建时用它拿到首批载荷。
pub(crate) fn execute_get_request(app: &AppHandle) -> Result<Option<PopupLayerView>, String>
{
    if app.get_webview_window(POPUP_LABEL).is_none()
    {
        return Ok(None); // 窗口已销毁，页面不会再被请求。
    }
    let state = STATE.lock().map_err(|_| "弹层状态不可用")?;
    Ok(state.request.as_ref().map(|pending| pending.view.clone()))
}

/// 页面量出内容尺寸后定位、缩放并显示弹层窗口。
pub(crate) fn execute_set_frame(app: &AppHandle, frame: PopupLayerFrame) -> Result<(), String>
{
    if !frame.width.is_finite() || !frame.height.is_finite() || !frame.anchor_offset_x.is_finite() || !frame.anchor_offset_y.is_finite() || frame.width <= 0.0 || frame.height <= 0.0
    {
        return Err("弹层尺寸无效".to_string());
    }
    let (pending, window) = {
        let state = STATE.lock().map_err(|_| "弹层状态不可用")?;
        let pending = state.request.clone().ok_or("弹层请求已结束")?;
        if pending.view.request_id != frame.request_id
        {
            return Err("弹层尺寸不属于当前请求".to_string());
        }
        let window = state.window.clone().ok_or("弹层窗口已销毁")?;
        (pending, window)
    };
    let monitor = monitor_at(pending.monitor)?;
    apply_frame(app, &window, &pending, &monitor, &frame)
}

/// 按当前请求的锚点把弹层摆到屏幕物理像素位置。
fn apply_frame(app: &AppHandle, window: &WebviewWindow, pending: &PendingRequest, monitor: &DesktopHostMonitor, frame: &PopupLayerFrame) -> Result<(), String>
{
    let scale = if monitor.scale_factor > 0.0 { monitor.scale_factor } else { 1.0 };
    let width = (frame.width * scale).round().max(1.0) as i32;
    let height = (frame.height * scale).round().max(1.0) as i32;
    let anchor_x = f64::from(monitor.work_x) + (pending.anchor_x * scale).round();
    let anchor_y = f64::from(monitor.work_y) + (pending.anchor_y * scale).round();
    let left = (anchor_x - (frame.anchor_offset_x * scale).round()) as i32;
    let top = (anchor_y - (frame.anchor_offset_y * scale).round()) as i32;
    // 只把窗口整体推回工作区，不改变窗口内部的菜单布局。
    let work_right = monitor.work_x + monitor.work_width as i32;
    let work_bottom = monitor.work_y + monitor.work_height as i32;
    let left = left.clamp(monitor.work_x, (work_right - width).max(monitor.work_x));
    let top = top.clamp(monitor.work_y, (work_bottom - height).max(monitor.work_y));
    let (generation, repaint) = {
        let mut state = STATE.lock().map_err(|_| "弹层状态不可用")?;
        match state.request.as_ref()
        {
            Some(current) if current.view.request_id == frame.request_id => (state.generation, std::mem::take(&mut state.needs_repaint)),
            _ => return Err("弹层请求已结束".to_string()),
        }
    };
    // 先在屏幕外就位：系统在“显示”这一帧会按标题栏配色绘制窗口边框，
    // 放到屏幕外可以让这一帧完全不可见，再移到目标位置（移动不会重画边框）。
    let staging = -width - 200;
    window
        .set_size(tauri::PhysicalSize::new(width as u32, height as u32))
        .and_then(|()| window.set_position(tauri::PhysicalPosition::new(staging, top)))
        .map_err(|error| format!("摆放弹层窗口失败：{error}"))?;
    // 定位期间请求可能已被取消或结束：显示前再验一次，避免让已收束的弹层重新出现。
    if !frame_is_current(frame.request_id, generation)?
    {
        let _ = window.hide();
        return Err("弹层请求已结束".to_string());
    }
    if let Ok(hwnd) = window.hwnd()
    {
        // 显示前再收敛一次样式：窗口在这之前可能被重新套上系统边框。
        let _ = strip_system_frame(hwnd);
    }
    window.show().and_then(|()| window.set_focus()).map_err(|error| format!("显示弹层窗口失败：{error}"))?;
    if !frame_is_current(frame.request_id, generation)?
    {
        let _ = window.hide();
        return Err("弹层请求已结束".to_string());
    }
    if let Ok(hwnd) = window.hwnd()
    {
        // 只有隐藏过的窗口重新显示才需要强制重绘；普通尺寸变化（展开/收起子菜单）走轻量路径。
        if repaint
        {
            refresh_composition(hwnd);
        }
        // 显示流程可能重新套用默认窗口样式，这里再收敛一次，确保没有系统标题栏与关闭按钮。
        if let Err(error) = strip_system_frame(hwnd)
        {
            crate::log_warn!("弹层：显示后去除系统边框失败，错误={error}");
        }
    }
    // 内容已经在屏幕外画好，移到目标位置后整窗一次出现。
    window.set_position(tauri::PhysicalPosition::new(left, top)).map_err(|error| format!("摆放弹层窗口失败：{error}"))?;
    if let Ok(hwnd) = window.hwnd()
    {
        let _ = strip_system_frame(hwnd); // 移动同样可能重新套用样式，出现后再收敛一次。
    }
    // 显示成功后才置 shown：中途任何一步失败都留给上报超时兜底，不会留下一个等不到结果的请求。
    {
        let mut state = STATE.lock().map_err(|_| "弹层状态不可用")?;
        let current_request = state.request.as_ref().map(|current| current.view.request_id);
        if state.generation != generation || current_request != Some(frame.request_id)
        {
            let _ = window.hide(); // 显示期间请求已被收束，别让这次浮层留在屏幕上。
            return Err("弹层请求已结束".to_string());
        }
        state.shown = true;
        state.foreground_before = pending.foreground_before; // 关闭时还原到打开前的窗口。
    }
    crate::log_debug!("弹层：已显示，种类={:?}，屏幕=[{left},{top},{width},{height}]，显示器={}，缩放={scale}", pending.view.kind, monitor.id);
    let _ = app; // 与其他窗口入口保持同一签名。
    Ok(())
}

/// 页面回传结果：隐藏窗口、还原前台，并把结果转发给宿主。
///
/// 菜单命令声明保持打开（`keepOpen`）时只转发本次选择，请求与窗口都继续有效，
/// 由宿主推送新载荷刷新勾选状态。
pub(crate) fn execute_finish(app: &AppHandle, result: PopupLayerResult) -> Result<(), String>
{
    if result.keep_open == Some(true) && !result.dismissed
    {
        {
            let state = STATE.lock().map_err(|_| "弹层状态不可用")?;
            let current = state.request.as_ref().ok_or("弹层请求已结束")?;
            if current.view.request_id != result.request_id || current.view.kind != result.kind
            {
                return Err("弹层结果不属于当前请求".to_string());
            }
        }
        return app.emit(POPUP_RESULT_EVENT, &result).map_err(|error| format!("弹层结果转发失败：{error}"));
    }
    let (window, previous) = {
        let mut state = STATE.lock().map_err(|_| "弹层状态不可用")?;
        let current = state.request.take().ok_or("弹层请求已结束")?;
        if current.view.request_id != result.request_id || current.view.kind != result.kind
        {
            state.request = Some(current); // 不属于当前请求，保持原请求可用。
            return Err("弹层结果不属于当前请求".to_string());
        }
        let window = state.window.clone();
        state.shown = false;
        state.generation = state.generation.wrapping_add(1); // 收束后迟到的显示必须失效。
        let previous = std::mem::take(&mut state.foreground_before);
        (window, previous)
    };
    if let Some(window) = window
    {
        mark_repaint_pending();
        if let Err(error) = window.hide()
        {
            // 隐藏失败时窗口还在屏幕上：直接销毁，绝不留一个挡住桌面点击的置顶窗口。
            crate::log_warn!("弹层：隐藏窗口失败，改为销毁，错误={error}");
            force_destroy(app, &window);
        }
    }
    restore_foreground(previous);
    if let Err(error) = app.emit(POPUP_RESULT_EVENT, &result)
    {
        crate::log_warn!("弹层：结果转发失败，请求={}，错误={error}", result.request_id);
    }
    schedule_idle_destroy(app);
    Ok(())
}

/// 宿主更新进行中弹层的载荷；复选菜单保持打开时用它刷新勾选状态。
pub(crate) fn execute_update_payload(app: &AppHandle, request_id: u64, payload: Value) -> Result<(), String>
{
    if serde_json::to_vec(&payload).map_err(|error| format!("弹层载荷序列化失败：{error}"))?.len() > POPUP_PAYLOAD_LIMIT
    {
        return Err("弹层载荷超过上限".to_string());
    }
    let (window, view) = {
        let mut state = STATE.lock().map_err(|_| "弹层状态不可用")?;
        let current = state.request.as_mut().ok_or("弹层请求已结束")?;
        if current.view.request_id != request_id
        {
            return Err("弹层请求身份不匹配".to_string());
        }
        current.view.payload = payload;
        let view = current.view.clone();
        (state.window.clone().ok_or("弹层窗口已销毁")?, view)
    };
    if let Err(error) = window.emit(POPUP_REQUEST_EVENT, &view)
    {
        crate::log_warn!("弹层：载荷更新投递失败，请求={request_id}，错误={error}");
    }
    let _ = app; // 与其他弹层入口保持同一签名。
    Ok(())
}

/// 这次取消是否可以作用到当前请求：带身份时必须与进行中的请求一致，过期取消不得结束新请求。
fn dismiss_applies_to(expected: Option<u64>, current: Option<u64>) -> bool
{
    match (expected, current)
    {
        (Some(expected), Some(current)) => expected == current,
        _ => true, // 没有指定身份，或已经没有进行中的请求：只按隐藏窗口处理。
    }
}

/// 宿主或失活触发的取消；没有进行中的请求时只隐藏窗口。
pub(crate) fn execute_dismiss(app: &AppHandle, request_id: Option<u64>) -> Result<(), String>
{
    let (window, current, previous) = {
        let mut state = STATE.lock().map_err(|_| "弹层状态不可用")?;
        if !dismiss_applies_to(request_id, state.request.as_ref().map(|current| current.view.request_id))
        {
            return Ok(()); // 过期取消不得结束新请求。
        }
        let window = state.window.clone();
        state.shown = false;
        state.generation = state.generation.wrapping_add(1); // 取消后迟到的显示必须失效。
        let previous = std::mem::take(&mut state.foreground_before);
        (window, state.request.take(), previous)
    };
    if let Some(window) = window
    {
        mark_repaint_pending();
        if let Err(error) = window.hide()
        {
            // 取消路径同理：隐藏失败立即销毁，不留可见空窗口。
            crate::log_warn!("弹层：取消时隐藏窗口失败，改为销毁，错误={error}");
            force_destroy(app, &window);
        }
    }
    restore_foreground(previous);
    if let Some(current) = current
    {
        let result = PopupLayerResult::cancelled(current.view.kind, current.view.request_id);
        if let Err(error) = app.emit(POPUP_RESULT_EVENT, &result)
        {
            crate::log_warn!("弹层：取消结果转发失败，请求={}，错误={error}", result.request_id);
        }
    }
    schedule_idle_destroy(app);
    Ok(())
}

/// 能还原就还原到打开前的程序，不能就落到桌面；失败只记日志。
fn restore_foreground(previous: isize)
{
    if previous == 0
    {
        return;
    }
    let target = HWND(previous as *mut _);
    if !unsafe { IsWindow(Some(target)).as_bool() } || !unsafe { IsWindowVisible(target).as_bool() }
    {
        crate::log_debug!("弹层：打开前的窗口已不可用，前台不还原"); // 焦点自然落到下一个窗口。
        return;
    }
    if !unsafe { SetForegroundWindow(target) }.as_bool()
    {
        crate::log_debug!("弹层：前台还原被系统拒绝，目标={previous}");
    }
}

/// 这次等待是否仍然有效：请求没被替换、代次没推进、也没有显示。
fn wait_is_pending(state: &PopupLayerState, request_id: u64, generation: u64) -> bool { !state.shown && state.generation == generation && state.request.as_ref().is_some_and(|pending| pending.view.request_id == request_id) }

/// 为当前请求启动上报超时看门狗；请求已显示或已结束时不启动。
fn start_measure_watchdog(app: &AppHandle, request_id: u64)
{
    let generation = match STATE.lock()
    {
        Ok(state) if wait_is_pending(&state, request_id, state.generation) => state.generation,
        _ => return,
    };
    watch_measure_timeout(app, request_id, generation);
}

/// 页面必须在 `POPUP_MEASURE_TIMEOUT_MS` 内上报尺寸，否则这次浮层永远不会出现。
///
/// 弹层页面与设置窗同源，取自 `build/frontend`。产物没重建、页面脚本出错或加载失败时，
/// 窗口建得出来却没有人上报尺寸；没有这道看门狗，宿主那次请求既不显示也不结束，
/// 用户只看到「右键没有菜单」，日志里也找不到线索。超时后按取消收束，让宿主恢复状态。
fn watch_measure_timeout(app: &AppHandle, request_id: u64, generation: u64)
{
    let handle = app.clone();
    std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_millis(POPUP_MEASURE_TIMEOUT_MS);
        // 分片等待：显示或换请求后立即退出，不把线程占满整个上限。
        while std::time::Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(POPUP_MEASURE_POLL_MS));
            match STATE.lock()
            {
                Ok(state) if wait_is_pending(&state, request_id, generation) =>
                {}
                _ => return, // 已显示、换了请求或状态不可用。
            }
        }
        let target = handle.clone(); // 收束要在主线程做窗口操作。
        if let Err(error) = handle.run_on_main_thread(move || {
            // 收束前再验一次：超时与显示可能只差一帧。
            let (kind, page_url) = match STATE.lock()
            {
                Ok(state) if wait_is_pending(&state, request_id, generation) => match state.request.as_ref()
                {
                    Some(pending) => (pending.view.kind, state.window.as_ref().and_then(|window| window.url().ok()).map(|url| url.to_string()).unwrap_or_default()),
                    None => return,
                },
                _ => return,
            };
            // 产物没重建只发生在开发运行，安装版拿到这条提示只会被误导。
            let hint = if cfg!(debug_assertions)
            {
                "；开发运行请确认前端产物已重建（npm run build，弹层页面取自 build/frontend）"
            }
            else
            {
                ""
            };
            crate::log_error!("弹层：页面 {}ms 内没有上报尺寸，本次浮层按取消收束，请求={request_id}，种类={kind:?}，页面={page_url}{hint}", POPUP_MEASURE_TIMEOUT_MS);
            if let Err(error) = execute_dismiss(&target, Some(request_id))
            {
                crate::log_warn!("弹层：上报超时收束失败，请求={request_id}，错误={error}");
            }
        })
        {
            // 调度不上主线程时请求会继续悬挂，必须留下线索。
            crate::log_warn!("弹层：上报超时无法调度主线程收束，请求={request_id}，错误={error}");
        }
    });
}

/// 关闭后按空闲时长销毁窗口；新请求推进代次即可取消。
fn schedule_idle_destroy(app: &AppHandle)
{
    let generation = IDLE_GENERATION.fetch_add(1, Ordering::AcqRel) + 1;
    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(POPUP_IDLE_TIMEOUT_MS));
        if IDLE_GENERATION.load(Ordering::Acquire) != generation
        {
            return; // 期间已有新请求或新计时。
        }
        let target = handle.clone(); // 调度闭包持有独立句柄，调用方仍可移动原句柄。
        let _ = handle.run_on_main_thread(move || destroy(&target));
    });
}

/// 空闲超时销毁窗口并释放 WebView2；下次请求走冷启动。
fn destroy(app: &AppHandle)
{
    // 销毁期间不允许并发创建：否则新请求会撞上尚未销毁的旧窗口。
    let _window_guard = WINDOW_LOCK.lock();
    let window = STATE.lock().ok().and_then(|mut state| {
        if state.request.is_some()
        {
            return None; // 销毁期间出现新请求，保留窗口。
        }
        state.window.take()
    });
    if let Some(window) = window
    {
        match window.destroy()
        {
            Ok(()) => crate::log_info!("弹层：空闲 {}s，窗口与 WebView2 已释放，下次请求冷启动", POPUP_IDLE_TIMEOUT_MS / 1000),
            Err(error) =>
            {
                // 销毁失败时窗口可能仍在屏幕上：放回状态，下一次空闲计时或退出流程还能再试。
                crate::log_warn!("弹层：空闲销毁失败，保留句柄等待重试，错误={error}");
                if let Ok(mut state) = STATE.lock()
                    && state.window.is_none()
                {
                    state.window = Some(window);
                }
            }
        }
    }
    let _ = app; // 窗口销毁走 Tauri 内部调度，句柄保留以对齐其他窗口入口。
}

/// 弹层页面读取当前请求；只有弹层窗口本身可以调用。
#[tauri::command]
pub fn get_popup_layer_request(window: tauri::WebviewWindow, app: AppHandle) -> Result<Option<PopupLayerView>, String>
{
    require_popup_window(&window)?;
    execute_get_request(&app)
}

/// 弹层页面上报内容尺寸；只有弹层窗口本身可以调用。
#[tauri::command]
pub fn set_popup_layer_frame(window: tauri::WebviewWindow, app: AppHandle, frame: PopupLayerFrame) -> Result<(), String>
{
    require_popup_window(&window)?;
    execute_set_frame(&app, frame)
}

/// 弹层页面回传结果；只有弹层窗口本身可以调用。
#[tauri::command]
pub fn finish_popup_layer(window: tauri::WebviewWindow, app: AppHandle, result: PopupLayerResult) -> Result<(), String>
{
    require_popup_window(&window)?;
    execute_finish(&app, result)
}

/// 只接受弹层窗口自身的调用；宿主命令只走原生通道，不占用 Tauri IPC 表面。
fn require_popup_window(window: &tauri::WebviewWindow) -> Result<(), String>
{
    if window.label() == POPUP_LABEL
    {
        Ok(())
    }
    else
    {
        crate::log_warn!("弹层：拒绝非弹层窗口的请求，窗口={}", window.label());
        Err("弹层命令只接受弹层窗口调用".to_string())
    }
}

#[cfg(test)]
#[path = "popup_layer.test.rs"]
mod tests;
