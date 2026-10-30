use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::size_of;
use std::num::NonZeroIsize;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tauri::{AppHandle, Listener, Manager};
use webview2_com::Microsoft::Web::WebView2::Win32::{COREWEBVIEW2_BOUNDS_MODE_USE_RAW_PIXELS, ICoreWebView2Controller3};
use windows::Win32::Foundation::{ERROR_CLASS_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{BI_RGB, BITMAPINFO, BITMAPINFOHEADER, ClientToScreen, CombineRgn, CreateCompatibleDC, CreateDIBSection, CreateRectRgn, DIB_RGB_COLORS, DeleteDC, DeleteObject, EnumDisplayMonitors, GetMonitorInfoW, HDC, HGDIOBJ, HMONITOR, MONITORINFO, MONITORINFOEXW, RDW_ALLCHILDREN, RDW_ERASE, RDW_FRAME, RDW_INVALIDATE, RDW_UPDATENOW, RGN_OR, RedrawWindow, ScreenToClient, SelectObject, SetWindowRgn};
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
use windows::Win32::System::Com::{CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, IServiceProvider};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::{IDropTarget, OleInitialize, OleUninitialize, RegisterDragDrop, RevokeDragDrop};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Shell::{IFolderView, IShellBrowser, IShellFolder, IShellItem, IShellWindows, SHCreateItemWithParent, SHELLFLAGSTATE, SHGetSettings, SID_STopLevelBrowser, SIGDN_DESKTOPABSOLUTEPARSING, SSF_HIDEICONS, SVGIO_ALLVIEW, SWC_DESKTOP, SWFO_NEEDDISPATCH, ShellWindows};
use windows::Win32::UI::WindowsAndMessaging::{CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, EnumChildWindows, EnumWindows, FindWindowExW, FindWindowW, GWL_STYLE, GetParent, GetWindowLongPtrW, GetWindowThreadProcessId, HWND_TOP, IsWindow, IsWindowVisible, MONITORINFOF_PRIMARY, PW_RENDERFULLCONTENT, RegisterClassExW, SW_HIDE, SW_SHOW, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SetParent, SetWindowLongPtrW, SetWindowPos, ShowWindow, ShowWindowAsync, WINDOW_EX_STYLE, WNDCLASSEXW, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_POPUP, WS_VISIBLE};
use windows::core::{BOOL, Interface, PCWSTR, w};
use wry::http::{Request, Response, StatusCode};
use wry::raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle};
use wry::{MemoryUsageLevel, PageLoadEvent, Rect as WryRect, WebView, WebViewBuilder, WebViewExtWindows, dpi};

use crate::contracts::{DesktopHostMonitor, DesktopItem, HitRegion};

/// 独立页面向正式应用共享状态发送命令的桥接接口。
pub type NativeCommandHandler = Arc<dyn Fn(&str, Value) -> Result<Value, Value> + Send + Sync + 'static>;

/// 接管前发现的 Explorer 桌面层状态。
#[derive(Clone, Copy, Debug)]
pub struct ShellDesktopState
{
    /// 当前 SHELLDLL_DefView 句柄。
    pub def_view: isize,
    /// 当前承载 SHELLDLL_DefView 的 Explorer 窗口。
    pub explorer_host: isize,
    /// 当前 Explorer 桌面 SysListView32 句柄。
    pub list_view: isize,
    /// 接管前 Explorer 桌面项目可见状态。
    pub original_items_visible: bool,
}

/// 单显示器正式宿主的原生记录。
#[derive(Clone, Debug)]
pub struct NativeDesktopHost
{
    /// Rust 持有的外层 HWND。
    pub hwnd: isize,
    /// 当前主显示器物理工作区。
    pub monitor: DesktopHostMonitor,
}

/// UI 线程持有的 Windowed WebView2 与宿主 HWND。
struct NativeHostResources
{
    /// 当前宿主对应的显示器稳定身份。
    monitor_id: String,
    /// Rust 持有的外层 HWND。
    hwnd: isize,
    /// Windowed WebView2 Controller。
    webview: WebView,
    /// 当前宿主页面的独立本地协议 URL。
    page_url: String,
    /// 外层窗口与 WebView2 子窗口注册的 Shell 桌面投放目标。
    drop_targets: Vec<(isize, IDropTarget)>,
    /// 当前 UI 线程是否需要配对释放一次 OleInitialize。
    ole_initialized: bool,
}

/// 为 Wry 暴露当前线程创建的 Win32 HWND。
struct NativeWindowHandle
{
    /// 当前正式宿主原始句柄。
    hwnd: NonZeroIsize,
}

/// 将正式宿主句柄交给 Wry 创建 Windowed WebView2。
impl HasWindowHandle for NativeWindowHandle
{
    /// 返回当前正式宿主的借用窗口句柄。
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError>
    {
        // Wry 接收的 Win32 原始窗口句柄。
        let handle = Win32WindowHandle::new(self.hwnd);
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(handle)) })
    }
}

/// 枚举 Explorer DefView 时使用的搜索状态。
struct DefViewSearch
{
    /// Progman 句柄。
    progman: HWND,
    /// 搜索得到的 DefView。
    def_view: HWND,
    /// 承载 DefView 的窗口。
    explorer_host: HWND,
}

thread_local! {
    /// UI 线程持续持有的全部正式 Windowed WebView2 宿主。
    static NATIVE_HOSTS: RefCell<Vec<NativeHostResources>> = const { RefCell::new(Vec::new()) };
}

/// 最近一次前端发布的完整语义命中图。
static HIT_REGIONS: OnceLock<RwLock<HashMap<String, Vec<HitRegion>>>> = OnceLock::new();

/// 逐屏页面最近一次导航完成时间，用于区分仍在加载与已加载但未就绪。
static PAGE_LOAD_FINISHED: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();

/// 返回逐屏页面加载完成记录表，首次使用时创建。
fn page_load_records() -> &'static Mutex<HashMap<String, Instant>> { PAGE_LOAD_FINISHED.get_or_init(|| Mutex::new(HashMap::new())) }

/// 记录逐屏页面导航完成时间，供恢复线程判断页面是否停滞。
pub fn mark_page_load_finished(monitor_id: &str)
{
    if let Ok(mut finished) = page_load_records().lock()
    {
        finished.insert(monitor_id.to_string(), Instant::now());
    }
}

/// 返回逐屏页面最近一次导航完成时间，从未完成时为空。
pub fn page_load_finished_at(monitor_id: &str) -> Option<Instant> { page_load_records().lock().ok().and_then(|finished| finished.get(monitor_id).copied()) }

/// 清除逐屏页面导航完成时间，新一次导航重新计时。
pub fn clear_page_load_finished(monitor_id: &str)
{
    if let Ok(mut finished) = page_load_records().lock()
    {
        finished.remove(monitor_id);
    }
}

/// 正式宿主窗口类名称。
const HOST_WINDOW_CLASS: windows::core::PCWSTR = w!("DesktopToolsFormalDesktopHost");

/// `SHELLFLAGSTATE` 中 `fHideIcons` 对应的位。
const SHELL_HIDE_ICONS_BIT: i32 = 1 << 12;

/// 独立宿主页面命令桥初始化脚本。
const NATIVE_BRIDGE_SCRIPT: &str = r#"
(() => {
  const nativeOrigin = __DESKTOP_NATIVE_ORIGIN__; // Rust 注入本屏原生命令地址，独立于开发页面来源。
  const reported = new Set(); // 同一页面生命周期内相同异常只记录一次。
  let pendingInteractions = 0; // 原生菜单和拖放命令返回前不暂停。
  const reportFailure = (stage, detail) => {
    const message = String(detail?.stack || detail?.message || detail || "未知错误").slice(0, 4096); // 限制日志大小。
    const key = `${stage}:${message}`; // 避免轮询失败刷屏。
    if (reported.has(key) || reported.size >= 40) return;
    reported.add(key);
    void fetch(`${nativeOrigin}/__desktop_tools_invoke/record_renderer_diagnostic`, {
      method: "POST", headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ origin: nativeOrigin, url: String(window.location?.href || ""), stage, message }),
    }).catch(() => {}); // 日志通道失败不递归上报。
  };
  if (window.addEventListener) {
    window.addEventListener("error", (event) => reportFailure(event.error ? "脚本异常" : "资源加载失败", event.error || `${event.message || ""} ${event.filename || event.target?.src || event.target?.href || ""}:${event.lineno || 0}:${event.colno || 0}`), true);
    window.addEventListener("unhandledrejection", (event) => reportFailure("未处理的异步异常", event.reason));
  }
  // 通过正式宿主本地协议调用 Rust 命令边界。
  const invoke = async (command, args = {}) => {
    const interaction = ["show_workspace_context_menu", "show_folder_portal_menu", "start_desktop_item_drag"].includes(command); // 会进入原生嵌套消息循环的命令。
    if (interaction) pendingInteractions++;
    try {
    // 当前命令对应的本地协议响应。
    const response = await fetch(`${nativeOrigin}/__desktop_tools_invoke/${encodeURIComponent(command)}`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(args),
    });
    // 已解析的统一命令响应载荷。
    const payload = await response.json();
    if (!payload.ok) throw payload.error;
    return payload.result;
    } catch (error) {
      if (["publish_desktop_host_readiness", "get_desktop_host_snapshot", "get_desktop_item_snapshot", "update_desktop_host_hit_map"].includes(command)) reportFailure(`命令失败：${command}，显示器=${args.monitorId || args.report?.monitorId || "未解析"}`, error);
      throw error;
    } finally {
      if (interaction) pendingInteractions--;
    }
  };

  // 页面从结构化事实与已保存偏好计算策略，后端只记录实际应用结果。
  let refreshPaused = false; // 非必要视觉暂停，健康心跳不变。
  let monitorId; // 本页显示器身份。
  let foreground; // 最新修订的共享快照。
  let pauseSince = 0; // 普通暂停稳定期。
  let interacting = false; // 指针交互保护。
  let policyTimer; // 仅候选暂停期间使用。
  let policyGeneration = 0; // 事件和读取的先后代次。
  const resumePolls = new Set(); // 恢复只读最新事实。
  const pausedAnimations = new Set(); // 只恢复本策略暂停的动画。
  const setPaused = (paused) => {
    if (refreshPaused === paused) return;
    refreshPaused = paused;
    for (const animation of document.getAnimations()) {
      if (paused && animation.playState === "running") { pausedAnimations.add(animation); animation.pause(); }
    }
    if (!paused) {
      for (const animation of pausedAnimations) animation.play();
      pausedAnimations.clear();
      for (const poll of resumePolls) void poll();
    }
    window.dispatchEvent(new CustomEvent("desktop-refresh-changed", { detail: paused }));
    if (monitorId) void invoke("record_desktop_refresh_state", { monitorId, paused }).catch(() => {});
  };
  const acceptForeground = (snapshot) => {
    if (!snapshot || !Number.isSafeInteger(snapshot.revision) || (foreground && snapshot.revision < foreground.revision)) return;
    foreground = snapshot;
    if (snapshot.locked === true) { pauseSince = 0; window.clearTimeout(policyTimer); setPaused(true); }
  };
  window.addEventListener("pointerdown", () => { interacting = true; if (foreground?.locked !== true) { pauseSince = 0; setPaused(false); } }, true);
  window.addEventListener("pointerup", () => { interacting = false; wakeRefresh(); }, true);
  window.addEventListener("pointercancel", () => { interacting = false; wakeRefresh(); }, true);
  window.addEventListener("focus", () => { interacting = false; if (foreground?.locked !== true) { pauseSince = 0; setPaused(false); } requestRefresh(); });
  const wakeRefresh = () => { policyGeneration++; window.clearTimeout(policyTimer); policyTimer = window.setTimeout(checkRefresh, 0); };
  const requestRefresh = () => { void invoke("refresh_foreground_snapshot").catch(() => {}); wakeRefresh(); };
  window.addEventListener("pageshow", requestRefresh);
  window.addEventListener("visibilitychange", () => { if (!document.hidden) requestRefresh(); });
  const checkRefresh = async () => {
    window.clearTimeout(policyTimer);
    const requested = ++policyGeneration; // 旧读取不得覆盖锁屏通知或更新的设置。
    try {
      if (monitorId) {
        const policy = await invoke("get_desktop_refresh_policy", { monitorId });
        if (requested !== policyGeneration) return;
        acceptForeground(policy.snapshot);
        if (foreground?.locked === true) { setPaused(true); return; }
        const covered = foreground?.coverage?.find((screen) => screen.monitorId === monitorId);
        const pause = policy.committed && (
          (policy.pauseOnFocus && foreground?.externalFocus === true) ||
          (policy.pauseOnFullscreen && foreground?.externalFocus === true &&
            (covered?.screen === true || (foreground.maximized === true && covered?.work === true)))
        );
        if (pause && !interacting && pendingInteractions === 0 && !document.querySelector('.desktop-tile-menu, [role="menu"], input:focus, textarea:focus')) {
          pauseSince ||= performance.now();
          if (performance.now() - pauseSince >= 300) setPaused(true);
        } else { pauseSince = 0; setPaused(false); }
      }
    } catch { if (requested === policyGeneration && foreground?.locked !== true) { pauseSince = 0; setPaused(false); } }
    finally {
      if (requested === policyGeneration && pauseSince && !refreshPaused) policyTimer = window.setTimeout(checkRefresh, 350);
    }
  };

  const eventCommands = {
    "music-session-changed": "get_current_music_session",
    "preferences-changed": "get_preferences",
    "windows-accent-color-changed": "get_windows_accent_color",
    "desktop-item-snapshot-changed": "get_current_desktop_item_snapshot",
    "desktop-host-snapshot-changed": "get_desktop_host_snapshot",
    "workspace-snapshot-changed": "get_current_workspace_snapshot",
    "component-runtime-snapshot-changed": "get_current_component_runtime_snapshot",
    "component-host-event": "get_current_component_host_event",
  };

  const eventSubscriptions = new Map(); // 已迁移事件的本页订阅，不保留历史消息。
  const dispatch = (name, payload) => {
    if (name === "desktop-refresh-policy-changed") acceptForeground(payload);
    if (name === "desktop-host-snapshot-changed") requestRefresh();
    if (name === "desktop-refresh-policy-changed" || name === "preferences-changed") {
      wakeRefresh();
    }
    for (const subscription of eventSubscriptions.get(name) || []) subscription.receive(payload);
  };

  // 权威布局、桌面项目及组件注册事件使用真实投递。
  const listen = async (eventName, handler) => {
    if (
      eventName === "folder-portal-changed" ||
      eventName === "collect-items-changed" ||
      eventName === "weather-configuration-changed" ||
      eventName === "weather-forecast-changed" ||
      eventName === "audio-spectrum-changed" ||
      eventName === "audio-spectrum-device-changed"
    ) {
      const subscription = { receive: (payload) => { if (!refreshPaused || eventName === "weather-configuration-changed" || payload?.error || payload?.deletedName) handler(payload); } }; // 主动删除即使暂停也必须移除会话缓存。
      if (!eventSubscriptions.has(eventName)) eventSubscriptions.set(eventName, new Set());
      eventSubscriptions.get(eventName).add(subscription);
      return () => eventSubscriptions.get(eventName)?.delete(subscription);
    }
    // 当前事件对应的只读快照命令。
    const command = eventCommands[eventName];
    if (!command) return () => {};
    {
      let active = true; // 退订后拒绝所有迟到结果。
      let generation = 0; // 事件先到时废弃旧的初始读取。
      const receive = (payload) => {
        generation++;
        if (!active || payload == null) return;
        if (eventName === "desktop-host-snapshot-changed" && !monitorId) {
          monitorId = payload.hosts?.[Number(new URLSearchParams(location.search).get("monitor") || 0)]?.monitor.id;
          if (monitorId) window.setTimeout(checkRefresh, 0);
        }
        if (refreshPaused) return;
        handler(payload);
      };
      const refresh = async () => {
        const requested = generation;
        try { const payload = await invoke(command); if (active && generation === requested) receive(payload); }
        catch (error) { reportFailure(`状态事件同步失败：${eventName}`, error); throw error; }
      };
      const subscription = { receive }; // 在读取快照前建立接收入口。
      if (!eventSubscriptions.has(eventName)) eventSubscriptions.set(eventName, new Set());
      eventSubscriptions.get(eventName).add(subscription);
      const resume = () => {
        if (eventName === "music-session-changed") void invoke("request_music_convergence").catch(() => {});
        void refresh().catch(() => {});
      }; // 音乐恢复同时唤醒有限收敛，快照读取不等待后续播放器事件。
      resumePolls.add(resume);
      try { await refresh(); } catch (error) { active = false; eventSubscriptions.get(eventName)?.delete(subscription); resumePolls.delete(resume); throw error; }
      return () => { active = false; generation++; eventSubscriptions.get(eventName)?.delete(subscription); resumePolls.delete(resume); };
    }
  };

  window.__DESKTOP_TOOLS_NATIVE__ = { invoke, listen, dispatch, isRefreshPaused: () => refreshPaused };
})();
"#;

/// 将保存的原始整数恢复为 HWND。
fn raw_hwnd(raw: isize) -> HWND { HWND(raw as *mut c_void) }

/// 将现有宿主事件转交 UI 线程的原生 WebView，页面重载后自动使用新的订阅入口。
pub(crate) fn install_host_events(app: &AppHandle)
{
    super::refresh_policy::start_events(app);
    for name in [crate::contracts::DESKTOP_HOST_SNAPSHOT_CHANGED_EVENT, crate::contracts::WORKSPACE_SNAPSHOT_CHANGED_EVENT, crate::contracts::DESKTOP_ITEM_SNAPSHOT_CHANGED_EVENT, crate::contracts::COMPONENT_RUNTIME_SNAPSHOT_CHANGED_EVENT, "music-session-changed", "preferences-changed", "windows-accent-color-changed", "component-host-event", "desktop-refresh-policy-changed", "folder-portal-changed", "collect-items-changed", "weather-configuration-changed", "weather-forecast-changed", "audio-spectrum-changed", "audio-spectrum-device-changed", crate::contracts::PLUGIN_SETTINGS_CHANGED_EVENT]
    {
        let handle = app.clone(); // 原生窗口所属的 UI 调度器。
        app.listen(name, move |event| {
            let payload = event.payload().to_string(); // Tauri 已序列化的结构化快照。
            let watcher_failed = name == "folder-portal-changed" && serde_json::from_str::<Value>(&payload).is_ok_and(|value| value.get("error").is_some()); // 暂停仍保留监听故障恢复通道。
            let spectrum_event = name == crate::platform::windows::audio::SPECTRUM_EVENT || name == crate::platform::windows::audio::SPECTRUM_DEVICE_EVENT; // 唯一按帧发布的高频事件。
            let task_handle = handle.clone(); // 主线任务持有独立句柄，监听回调仍可重复执行。
            if let Err(error) = handle.run_on_main_thread(move || {
                let script = format!("window.__DESKTOP_TOOLS_NATIVE__?.dispatch('{name}', {payload});"); // 内部白名单事件名，不执行外部脚本输入。
                // 频谱只在同时开启音乐控件与频谱时才有订阅者，每帧读一次设置即可。
                let spectrum_active = spectrum_event && task_handle.state::<crate::app::settings::SettingsState>().preferences().is_ok_and(|preferences| preferences.music_enabled && preferences.music_spectrum_enabled);
                let workspace = task_handle.state::<crate::workspace::WorkspaceState>(); // 按显示器判断音乐控件归属，避免整份快照拷贝。
                NATIVE_HOSTS.with(|hosts| {
                    for host in hosts.borrow().iter()
                    {
                        if spectrum_event && (!spectrum_active || !workspace.has_component_on_monitor(&host.monitor_id, "official.music-control"))
                        {
                            continue; // 该屏没有正在显示频谱的音乐控件。
                        }
                        if name == "component-host-event" && !crate::component_runtime::ComponentRuntimeState::has_time_consumers(&host.monitor_id)
                        {
                            continue;
                        }
                        if !matches!(name, "desktop-refresh-policy-changed" | "preferences-changed" | "collect-items-changed" | "weather-configuration-changed") && !watcher_failed && super::refresh_policy::is_paused(&host.monitor_id)
                        {
                            continue;
                        }
                        if name == "desktop-refresh-policy-changed"
                        {
                            let level = if super::refresh_policy::is_paused(&host.monitor_id)
                            {
                                MemoryUsageLevel::Low
                            }
                            else
                            {
                                MemoryUsageLevel::Normal
                            };
                            if let Err(error) = host.webview.set_memory_usage_level(level)
                            {
                                crate::log_warn!("宿主：设置 WebView2 内存目标失败，显示器={}，错误={error}", host.monitor_id);
                            }
                        }
                        if let Err(error) = host.webview.evaluate_script(&script)
                        {
                            crate::log_warn!("宿主：事件投递失败，显示器={}，错误={error}", host.monitor_id);
                        }
                    }
                });
            })
            {
                crate::log_warn!("宿主：事件调度失败，错误={error}");
            }
        });
    }
}

/// 为正式宿主和 WebView2 子窗口注册 Shell 桌面 IDropTarget。
fn register_shell_drop_targets(hwnd: HWND, app: &AppHandle, monitor: &DesktopHostMonitor) -> Result<(Vec<(isize, IDropTarget)>, bool), String>
{
    unsafe { OleInitialize(None) }.map_err(|error| format!("初始化宿主 OLE 拖放失败: {error}"))?;
    let mut windows = vec![hwnd]; // 等待替换为 Shell 桌面投放目标的窗口集合。
    unsafe extern "system" fn collect_child(window: HWND, parameter: LPARAM) -> BOOL
    {
        let windows = unsafe { &mut *(parameter.0 as *mut Vec<HWND>) }; // 调用方持有的窗口集合。
        windows.push(window);
        BOOL(1)
    }
    let _ = unsafe { EnumChildWindows(Some(hwnd), Some(collect_child), LPARAM((&mut windows as *mut Vec<HWND>) as isize)) };
    let mut targets = Vec::new(); // 必须保持到宿主销毁的已注册 COM 投放目标。
    for target_window in windows
    {
        let app = app.clone(); // 原生回调访问当前 Shell 操作代理。
        let monitor = monitor.clone(); // 释放坐标所属宿主的物理空间与 DPI。
        let accepted = std::cell::Cell::new(false); // Enter 对完整来源集合的接受性结果。
        let content_offer = std::cell::RefCell::new(super::drop_content::ContentOffer::default()); // 进入时探测的有界业务内容摘要。
        let preferred_effect = std::cell::Cell::new(windows::Win32::System::Ole::DROPEFFECT_COPY); // 源首选效果或实际卷关系决定的默认值。
        let explicit_effect = std::cell::Cell::new(None); // 来源明确声明的首选效果不受目标卷重算覆盖。
        let source_paths = std::cell::RefCell::new(Vec::<String>::new()); // 当前完整文件来源，用于门户目标卷判定。
        let portal_effect = std::cell::RefCell::new(None::<(String, windows::Win32::System::Ole::DROPEFFECT)>); // 目标目录不变时复用卷判断。
        let internal_source = std::cell::Cell::new(false); // Enter 已核实的进程内 COM 身份。
        let preview = std::cell::RefCell::new(None::<serde_json::Value>); // 复用已有跨宿主预览事件，文件来源仍由原生决定。
        let target = super::drop_target::create_drop_target(
            Some(target_window),
            std::rc::Rc::new(move |phase| {
                use super::drop_target::DropPhase;
                use windows::Win32::System::Ole::{DROPEFFECT_COPY, DROPEFFECT_MOVE, DROPEFFECT_NONE};
                match phase
                {
                    DropPhase::Enter(data, keys, point, _effect) =>
                    {
                        crate::log_info!("拖放：原生进入，显示器={}，位置=({},{})", monitor.id, point.x, point.y);
                        internal_source.set(super::drop_target::source_drag_session(data).is_some());
                        *source_paths.borrow_mut() = super::drop_target::filesystem_drop_items(data).unwrap_or_default();
                        *content_offer.borrow_mut() = super::drop_content::offer(data, &super::drop_content::collect_file_paths(data).unwrap_or_default());
                        accepted.set((internal_source.get() || !source_paths.borrow().is_empty() || !content_offer.borrow().targets.is_empty()) && (keys & windows::Win32::System::SystemServices::MK_RBUTTON).0 == 0);
                        if !accepted.get()
                        {
                            return Ok(DROPEFFECT_NONE);
                        }
                        let source_session = super::drop_target::source_drag_session(data); // 只有进程登记的完整对象能使用内部来源位置。
                        let item_ids = source_session.as_ref().map(|session| session.item_ids.clone()).unwrap_or_else(|| source_paths.borrow().clone());
                        explicit_effect.set(super::shell::preferred_drop_effect(data));
                        *portal_effect.borrow_mut() = None;
                        if source_session.as_ref().is_none_or(|session| session.portal_source || session.collect_names.is_some())
                        {
                            preferred_effect.set(super::shell::default_drop_effect(data, &source_paths.borrow()));
                        }
                        let revision = crate::app::desktop_drop::snapshot(&app).map_err(|error| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, error))?.revision; // 原生要求页面确认相同布局版本。
                        let payload = serde_json::json!({"session": format!("ole-{}", crate::tools::time::next_timeline_timestamp()), "sequence": 1, "revision": revision, "modifiers": keys.0, "source": "ole", "target": monitor.id, "phase": "move", "kind": "items", "itemIds": item_ids, "visuals": [], "origin": {"left": 0, "top": 0}, "pointer": {"left": (f64::from(point.x) - f64::from(monitor.work_x)) / monitor.scale_factor, "top": (f64::from(point.y) - f64::from(monitor.work_y)) / monitor.scale_factor}}); // 原生坐标只用于预览，Drop 不等待页面回传。
                        let mut payload = payload;
                        payload["contentTargets"] = serde_json::json!(content_offer.borrow().targets);
                        payload["fileContent"] = serde_json::json!(!source_paths.borrow().is_empty() || internal_source.get());
                        payload["external"] = serde_json::json!(source_session.as_ref().is_none_or(|session| session.portal_source || session.collect_names.is_some()));
                        if let Some(source) = source_session
                            && !source.portal_source
                            && source.collect_names.is_none()
                        {
                            let cached = HIT_REGIONS.get().and_then(|regions| regions.read().ok()); // 复用源宿主已经发布的实际项目几何。
                            let regions = cached.as_ref().map(|regions| regions.values().flatten().filter(|region| region.kind == "desktop-item").collect::<Vec<_>>()).unwrap_or_default();
                            let anchor = regions.iter().find(|region| source.item_ids.first() == Some(&region.id));
                            let visuals = source
                                .item_ids
                                .iter()
                                .enumerate()
                                .map(|(index, id)| {
                                    let bounds = regions.iter().find(|region| &region.id == id);
                                    let left = bounds.zip(anchor).map(|(bounds, anchor)| bounds.left - anchor.left).unwrap_or(index as f64 * 12.0) - source.pointer_offset.left;
                                    let top = bounds.zip(anchor).map(|(bounds, anchor)| bounds.top - anchor.top).unwrap_or(index as f64 * 12.0) - source.pointer_offset.top;
                                    serde_json::json!({"itemId": id, "bounds": {"left": left, "top": top, "width": bounds.map(|region| region.width).unwrap_or(78.0), "height": bounds.map(|region| region.height).unwrap_or(98.0)}})
                                })
                                .collect::<Vec<_>>(); // 缺少可见节点时仍保留完整集合，以紧凑组图像呈现。
                            payload["visuals"] = serde_json::json!(visuals);
                        }
                        super::drop_target::set_preview_session(payload["session"].as_str().expect("原生会话字符串"), true);
                        let _ = relay_workspace_drag(payload.clone());
                        *preview.borrow_mut() = Some(payload);
                        crate::log_info!("拖放：原生进入处理完成，显示器={}", monitor.id);
                        Ok(DROPEFFECT_NONE) // 首个几何确认前不猜测当前目标允许的动作。
                    }
                    DropPhase::Over(keys, point, effect) =>
                    {
                        if !accepted.get()
                        {
                            return Ok(DROPEFFECT_NONE);
                        }
                        let mut portal_target = false; // 门户即使接收本程序桌面项目也执行真实文件传输。
                        let mut portal_path = None;
                        if let Some(payload) = preview.borrow_mut().as_mut()
                        {
                            let pointer = serde_json::json!({"left": (f64::from(point.x) - f64::from(monitor.work_x)) / monitor.scale_factor, "top": (f64::from(point.y) - f64::from(monitor.work_y)) / monitor.scale_factor}); // 当前工作区指针。
                            let snapshot = crate::app::desktop_drop::snapshot(&app).map_err(|error| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, error))?; // 同一次移动共用领域快照。
                            let revision = serde_json::json!(snapshot.revision); // 当前领域版本。
                            if payload["pointer"] != pointer || payload["revision"] != revision || payload["modifiers"] != serde_json::json!(keys.0)
                            {
                                payload["sequence"] = serde_json::json!(payload["sequence"].as_u64().unwrap_or(0).saturating_add(1));
                            }
                            payload["pointer"] = pointer;
                            payload["revision"] = revision;
                            payload["modifiers"] = serde_json::json!(keys.0);
                            let _ = relay_workspace_drag(payload.clone());
                            let Some(geometry) = super::drop_target::confirmed_preview(payload["session"].as_str().unwrap_or_default(), payload["sequence"].as_u64().unwrap_or_default(), payload["revision"].as_u64().unwrap_or_default(), &monitor, point)
                            else
                            {
                                return Ok(DROPEFFECT_NONE);
                            }; // 新帧确认前不显示可投放光标。
                            let top = super::drop_target::hit_region_at_point(&geometry, &monitor, point);
                            if let Some(region) = top.as_ref()
                            {
                                if let Some(target) = crate::app::desktop_drop::content_target(&app, &snapshot, region, &monitor.id).map_err(|error| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, error))?
                                {
                                    let preferences = app.state::<crate::app::settings::SettingsState>().preferences().map_err(|error| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, error))?; // 当前资源模式决定实际效果。
                                    let x = (f64::from(point.x) - f64::from(monitor.work_x)) / monitor.scale_factor; // 以当前指针命中正文目标。
                                    let y = (f64::from(point.y) - f64::from(monitor.work_y)) / monitor.scale_factor;
                                    let appending = geometry.collect_items.iter().any(|item| item.instance_id == region.id && item.name.ends_with(".md") && x >= item.bounds.left && x < item.bounds.left + item.bounds.width && y >= item.bounds.top && y < item.bounds.top + item.bounds.height);
                                    let mode = if appending { preferences.collect_image_import_mode.as_str() } else { "copy" }; // 独立图片始终复制，文档插图沿用旧偏好。
                                    return Ok(super::drop_content::content_effect(&content_offer.borrow(), &target, mode, effect, keys));
                                }
                            }
                            if source_paths.borrow().is_empty() && !internal_source.get()
                            {
                                return Ok(DROPEFFECT_NONE);
                            } // 文本不能进入桌面文件或布局路径。
                            portal_target = top.as_ref().is_some_and(|region| region.kind == "component" && region.target_path.is_some());
                            portal_path = top.as_ref().filter(|_| portal_target).and_then(|region| region.target_path.clone());
                            if !top.as_ref().is_some_and(|region| matches!(region.kind.as_str(), "workspace-background" | "fence" | "desktop-item") || (region.kind == "component" && region.target_path.is_some()))
                            {
                                return Ok(DROPEFFECT_NONE);
                            }
                        }
                        let preferred = if let Some(path) = portal_path.as_deref()
                        {
                            explicit_effect.get().unwrap_or_else(|| {
                                let mut cache = portal_effect.borrow_mut();
                                if cache.as_ref().is_none_or(|(current, _)| current != path)
                                {
                                    *cache = Some((path.to_string(), super::shell::directory_drop_effect(&source_paths.borrow(), path)));
                                }
                                cache.as_ref().expect("门户目标效果已缓存").1
                            })
                        }
                        else
                        {
                            preferred_effect.get()
                        };
                        Ok(super::drop_target::negotiated_effect(effect, keys, preferred, internal_source.get() && !portal_target && !preview.borrow().as_ref().is_some_and(|payload| payload["external"] == true)))
                    }
                    DropPhase::Drop(data, keys, point, mut effect) =>
                    {
                        if !accepted.get()
                        {
                            return Ok(DROPEFFECT_NONE);
                        }
                        let source_session = super::drop_target::source_drag_session(data);
                        if internal_source.get() && source_session.is_none()
                        {
                            return Ok(DROPEFFECT_NONE);
                        } // 释放对象身份变化不能进入业务路径。
                        let current = crate::app::desktop_drop::snapshot(&app).map_err(|error| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, error))?; // 释放时最新工作区。
                        let content_geometry = preview.borrow().as_ref().and_then(|payload| super::drop_target::confirmed_preview(payload["session"].as_str()?, payload["sequence"].as_u64()?, current.revision, &monitor, point)); // 与 Shell 路径共用版本化几何。
                        if let Some(geometry) = content_geometry.as_ref()
                        {
                            if let Some(region) = super::drop_target::hit_region_at_point(geometry, &monitor, point)
                            {
                                if let Some(target) = crate::app::desktop_drop::content_target(&app, &current, &region, &monitor.id).map_err(|error| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, error))?
                                {
                                    let paths = if target == super::drop_content::COLLECT && source_session.as_ref().is_some_and(|session| session.collect_names.is_some())
                                    {
                                        source_session.as_ref().expect("来源已校验").item_ids.clone()
                                    }
                                    else if target == super::drop_content::COLLECT
                                    {
                                        match super::drop_content::collect_file_paths(data)
                                        {
                                            Ok(paths) => paths,
                                            Err(error) =>
                                            {
                                                crate::app::desktop_drop::report_content_error(&app, error);
                                                return Ok(DROPEFFECT_NONE);
                                            }
                                        }
                                    }
                                    else
                                    {
                                        super::drop_target::filesystem_drop_items(data).unwrap_or_default()
                                    }; // 收集保留失效路径，其他目标仍整体校验。
                                    let offer = super::drop_content::offer(data, &paths); // 重新探测仍有效的源格式。
                                    let preferences = app.state::<crate::app::settings::SettingsState>().preferences().map_err(|error| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, error))?; // 固定本次资源模式。
                                    let x = (f64::from(point.x) - f64::from(monitor.work_x)) / monitor.scale_factor; // 释放实时横坐标。
                                    let y = (f64::from(point.y) - f64::from(monitor.work_y)) / monitor.scale_factor; // 释放实时纵坐标。
                                    let name = geometry.collect_items.iter().find(|item| item.instance_id == region.id && x >= item.bounds.left && x < item.bounds.left + item.bounds.width && y >= item.bounds.top && y < item.bounds.top + item.bounds.height).map(|item| item.name.clone()); // 命中条目才追加。
                                    let mode = if name.as_deref().is_some_and(|name| name.ends_with(".md"))
                                    {
                                        preferences.collect_image_import_mode
                                    }
                                    else
                                    {
                                        "copy".into()
                                    }; // 空白处复制图片并保留来源。
                                    let selected = super::drop_content::content_effect(&offer, &target, &mode, effect, keys); // 先目标业务，后内部布局。
                                    if selected == DROPEFFECT_NONE
                                    {
                                        return Ok(DROPEFFECT_NONE);
                                    }
                                    let content = match super::drop_content::extract(data, paths, &offer)
                                    {
                                        Ok(content) => content,
                                        Err(error) =>
                                        {
                                            crate::app::desktop_drop::report_content_error(&app, error);
                                            return Ok(DROPEFFECT_NONE);
                                        }
                                    }; // 数据对象仅在原生回调生命周期内访问。
                                    if let Some(payload) = preview.borrow_mut().as_mut()
                                    {
                                        payload["phase"] = serde_json::json!("pending");
                                        let _ = relay_workspace_drag(payload.clone());
                                    }
                                    let (sender, receiver) = std::sync::mpsc::channel(); // 仅传输自有内容和结果，不跨线程保存 COM 对象。
                                    let worker_app = app.clone(); // 后台业务拥有应用句柄。
                                    let monitor_id = monitor.id.clone(); // 固定释放显示器。
                                    let collect_names = source_session.as_ref().and_then(|session| session.collect_names.clone()).filter(|_| target == super::drop_content::COLLECT); // 外部对象不能伪造内部条目。
                                    tauri::async_runtime::spawn_blocking(move || {
                                        let _ = sender.send(crate::app::desktop_drop::receive_content(&worker_app, region, monitor_id, name, content, mode, collect_names));
                                    });
                                    let result = super::drop_target::wait_transfer(receiver); // 沿用 OLE 消息泵等待真实执行结果。
                                    if selected == DROPEFFECT_MOVE || result.as_ref().is_ok_and(|result| result.had_move)
                                    {
                                        let _ = super::shell::set_drop_effect(data, windows::Win32::UI::Shell::CFSTR_PERFORMEDDROPEFFECT, DROPEFFECT_NONE);
                                    } // 本地逐项移动不能报告整批 MOVE，来源绝不再次删除。
                                    match result
                                    {
                                        Ok(received) =>
                                        {
                                            crate::log_info!("拖放：业务内容接收，目标类型={target}，已处理={}，跳过={}，失败={}", received.processed, received.skipped, received.failures.len());
                                            if received.processed > 0
                                            {
                                                if let Some(payload) = preview.borrow_mut().as_mut()
                                                {
                                                    payload["committed"] = serde_json::json!(true);
                                                }
                                            }
                                            return Ok(
                                                if received.processed > 0 && selected == DROPEFFECT_COPY && !received.had_move
                                                {
                                                    DROPEFFECT_COPY
                                                }
                                                else
                                                {
                                                    DROPEFFECT_NONE
                                                },
                                            );
                                        }
                                        Err(error) =>
                                        {
                                            crate::app::desktop_drop::report_content_error(&app, error);
                                            return Ok(DROPEFFECT_NONE);
                                        }
                                    }
                                }
                            }
                        }
                        let portal_target = preview.borrow().as_ref().and_then(|payload| super::drop_target::confirmed_preview(payload["session"].as_str()?, payload["sequence"].as_u64()?, payload["revision"].as_u64()?, &monitor, point)).and_then(|geometry| super::drop_target::hit_region_at_point(&geometry, &monitor, point)).is_some_and(|region| region.kind == "component" && region.target_path.is_some());
                        if let Some(session) = source_session.as_ref().filter(|session| !session.portal_source && session.collect_names.is_none() && !portal_target)
                        {
                            let snapshot = crate::app::desktop_drop::snapshot(&app).map_err(|error| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, error))?; // 应用层提供释放时布局版本。
                            let geometry = preview.borrow().as_ref().and_then(|payload| super::drop_target::confirmed_preview(payload["session"].as_str()?, payload["sequence"].as_u64()?, snapshot.revision, &monitor, point)); // 不同步等待页面，拒绝未确认或过期落点。
                            let Some(geometry) = geometry
                            else
                            {
                                crate::log_warn!("拖放：内部释放拒绝，缺少匹配渲染确认，显示器={}，位置=({},{})，布局版本={}，原生预览={}", monitor.id, point.x, point.y, snapshot.revision, preview.borrow().as_ref().map(|payload| serde_json::json!({"session": payload["session"], "sequence": payload["sequence"], "revision": payload["revision"], "pointer": payload["pointer"]})).unwrap_or_default());
                                if let Some(session) = preview.borrow().as_ref().and_then(|payload| payload["session"].as_str())
                                {
                                    super::drop_target::trace_rejected_preview(session);
                                }
                                return Ok(DROPEFFECT_NONE);
                            };
                            let source_positions = preview.borrow().as_ref().and_then(|payload| payload["visuals"].as_array()).map(|visuals| visuals.iter().filter_map(|visual| Some(crate::contracts::WorkspacePoint { left: visual["bounds"]["left"].as_f64()?, top: visual["bounds"]["top"].as_f64()? })).collect::<Vec<_>>()).unwrap_or_default(); // Enter 已冻结的真实来源排列，不使用旧桌面占位。
                            let committed = crate::app::desktop_drop::commit_internal(&app, snapshot, crate::app::desktop_drop::DropInput { monitor_id: monitor.id.clone(), pointer: crate::contracts::WorkspacePoint { left: (f64::from(point.x) - f64::from(monitor.work_x)) / monitor.scale_factor, top: (f64::from(point.y) - f64::from(monitor.work_y)) / monitor.scale_factor }, regions: geometry.regions, scrolls: geometry.fence_scrolls }, session.item_ids.clone(), &session.pointer_offset, keys.0 & windows::Win32::System::Ole::MK_ALT != 0, &source_positions).map_err(|error| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, error))?; // 应用层只提交布局，原生会话保留独立完成标记。
                            crate::log_info!("拖放：内部释放布局结果，显示器={}，已提交={}", monitor.id, committed);
                            if committed
                            {
                                session.layout_committed.store(true, std::sync::atomic::Ordering::Release);
                                if let Some(payload) = preview.borrow_mut().as_mut()
                                {
                                    payload["committed"] = serde_json::json!(true);
                                }
                            }
                            return Ok(DROPEFFECT_NONE);
                        }
                        if internal_source.get() && source_session.is_none()
                        {
                            return Ok(DROPEFFECT_NONE);
                        } // Drop 对象与 Enter 身份不一致时拒绝，不改走外部路径。
                        if !super::shell::shell_drag_active() || source_session.is_some()
                        {
                            let preferred = portal_effect.borrow().as_ref().map(|(_, effect)| *effect).unwrap_or(preferred_effect.get());
                            effect = super::drop_target::negotiated_effect(effect, keys, explicit_effect.get().unwrap_or(preferred), false);
                            if effect != DROPEFFECT_COPY && effect != DROPEFFECT_MOVE
                            {
                                return Ok(DROPEFFECT_NONE);
                            }
                            let Some(sources) = super::drop_target::filesystem_drop_items(data).or_else(|| source_session.as_ref().map(|session| session.item_ids.clone()))
                            else
                            {
                                return Ok(DROPEFFECT_NONE);
                            }; // 完整来源必须在释放时仍可解析。
                            let state = app.state::<crate::shell::DesktopItemState>();
                            let snapshot = crate::app::desktop_drop::snapshot(&app).map_err(|error| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, error))?;
                            let geometry = preview.borrow().as_ref().and_then(|payload| super::drop_target::confirmed_preview(payload["session"].as_str()?, payload["sequence"].as_u64()?, snapshot.revision, &monitor, point));
                            let Some(geometry) = geometry
                            else
                            {
                                return Ok(DROPEFFECT_NONE);
                            }; // 缺少当前渲染确认不开始文件传输。
                            let Some(plan) = crate::app::desktop_drop::prepare_external(&app, snapshot, crate::app::desktop_drop::DropInput { monitor_id: monitor.id.clone(), pointer: crate::contracts::WorkspacePoint { left: (f64::from(point.x) - f64::from(monitor.work_x)) / monitor.scale_factor, top: (f64::from(point.y) - f64::from(monitor.work_y)) / monitor.scale_factor }, regions: geometry.regions, scrolls: geometry.fence_scrolls }, sources, keys.0 & windows::Win32::System::SystemServices::MK_CONTROL.0 != 0).map_err(|error| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, error))?
                            else
                            {
                                return Ok(DROPEFFECT_NONE);
                            }; // 应用层完成准确来源拆分与传输前预检。
                            let has_transfers = !plan.transfers.is_empty(); // OLE 完成反馈只反映真实文件传输。
                            if let Some(payload) = preview.borrow_mut().as_mut()
                            {
                                payload["phase"] = serde_json::json!("pending");
                                let _ = relay_workspace_drag(payload.clone());
                            }
                            let completed = if !has_transfers
                            {
                                crate::contracts::ShellOperationResult { operation_id: 0, completed_at_unix_micros: 0, kind: crate::contracts::ShellOperationKind::Paste, aborted: false, operation_error: None, reversible: true, permanent: false, effect: None, items: Vec::new(), message: "已有桌面项目仅调整布局".into() }
                            }
                            else
                            {
                                let result = if let Some(directory) = plan.portal_root.clone()
                                {
                                    state.begin_folder_drop_transfer(effect, plan.transfers.clone(), directory)
                                }
                                else
                                {
                                    state.begin_drop_transfer(data, effect, plan.transfers.clone())
                                }
                                .map_err(|error| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, error.message))?;
                                super::drop_target::wait_transfer(result).unwrap_or_else(|error| crate::contracts::ShellOperationResult { operation_id: 0, completed_at_unix_micros: 0, kind: crate::contracts::ShellOperationKind::Paste, aborted: false, operation_error: Some(windows::Win32::Foundation::E_FAIL.0), reversible: false, permanent: false, effect: Some(if effect == DROPEFFECT_MOVE { "移动" } else { "复制" }.into()), items: Vec::new(), message: format!("文件结果无法完整确认：{error}") })
                            };
                            let (succeeded, committed, result) = crate::app::desktop_drop::complete_external(&app, plan, completed); // 原生只交付文件事实，不解释布局结果归属。
                            if committed && let Some(payload) = preview.borrow_mut().as_mut()
                            {
                                payload["committed"] = serde_json::json!(true);
                            }
                            if effect == DROPEFFECT_MOVE && has_transfers
                            {
                                let _ = super::shell::set_drop_effect(data, windows::Win32::UI::Shell::CFSTR_PERFORMEDDROPEFFECT, DROPEFFECT_NONE);
                                if succeeded
                                {
                                    let _ = super::shell::set_drop_effect(data, windows::Win32::UI::Shell::CFSTR_LOGICALPERFORMEDDROPEFFECT, DROPEFFECT_MOVE);
                                }
                            }
                            if let Some(result) = result
                            {
                                let _ = relay_workspace_drag(serde_json::json!({"phase": "operation-result", "target": monitor.id, "result": result}));
                            }
                            return Ok(if succeeded && effect == DROPEFFECT_COPY { DROPEFFECT_COPY } else { DROPEFFECT_NONE });
                        }
                        Ok(DROPEFFECT_NONE) // 源 STA 正被占用时拒绝其他传输，不能重入同一请求队列。
                    }
                    DropPhase::Leave =>
                    {
                        if let Some(mut payload) = preview.borrow_mut().take()
                        {
                            super::drop_target::set_preview_session(payload["session"].as_str().expect("原生会话字符串"), false);
                            payload["phase"] = serde_json::json!("end");
                            let _ = relay_workspace_drag(payload);
                        }
                        internal_source.set(false);
                        accepted.set(false);
                        *content_offer.borrow_mut() = super::drop_content::ContentOffer::default();
                        source_paths.borrow_mut().clear();
                        *portal_effect.borrow_mut() = None;
                        Ok(DROPEFFECT_NONE)
                    }
                }
            }),
        ); // 宿主拥有 OLE 生命周期与文件协议，布局业务由应用层组织。
        let _ = unsafe { RevokeDragDrop(target_window) };
        match unsafe { RegisterDragDrop(target_window, &target) }
        {
            Ok(()) => targets.push((target_window.0 as isize, target)),
            Err(error) => crate::log_warn!("Shell：宿主窗口投放注册失败，窗口句柄={:#x}，错误：{}", target_window.0 as isize, error),
        }
    }
    if targets.is_empty()
    {
        unsafe { OleUninitialize() };
        return Err("正式宿主没有可用的 Shell 桌面投放目标".to_string());
    }
    crate::log_info!("Shell：宿主 OLE 投放目标已注册，窗口数量：{}", targets.len());
    Ok((targets, true))
}

/// 判断两个 HWND 是否属于同一个 Explorer 进程。
unsafe fn same_process(left: HWND, right: HWND) -> bool
{
    // 左侧窗口所属进程标识。
    let mut left_process_id = 0;
    // 右侧窗口所属进程标识。
    let mut right_process_id = 0;
    unsafe {
        GetWindowThreadProcessId(left, Some(&mut left_process_id));
        GetWindowThreadProcessId(right, Some(&mut right_process_id));
    }
    left_process_id != 0 && left_process_id == right_process_id
}

/// 枚举顶层窗口并定位 Explorer 的 SHELLDLL_DefView。
unsafe extern "system" fn find_def_view(window: HWND, parameter: LPARAM) -> BOOL
{
    // 调用方传入的搜索状态。
    let search = unsafe { &mut *(parameter.0 as *mut DefViewSearch) };
    // 当前顶层窗口中的 DefView。
    let def_view = unsafe { FindWindowExW(Some(window), None, w!("SHELLDLL_DefView"), PCWSTR::null()).unwrap_or_default() };
    if !def_view.is_invalid() && unsafe { same_process(search.progman, window) }
    {
        search.def_view = def_view;
        search.explorer_host = window;
        return BOOL(0);
    }
    BOOL(1)
}

/// 发现当前 Explorer 桌面项目视图与宿主。
pub fn discover_shell_desktop() -> Result<ShellDesktopState, String>
{
    // Windows 桌面 Progman 窗口。
    let progman = unsafe { FindWindowW(w!("Progman"), PCWSTR::null()).unwrap_or_default() };
    if progman.is_invalid()
    {
        return Err("未发现 Explorer Progman".to_string());
    }

    // Progman 直属的 DefView。
    let mut def_view = unsafe { FindWindowExW(Some(progman), None, w!("SHELLDLL_DefView"), PCWSTR::null()).unwrap_or_default() };
    // 当前 DefView 所属 Explorer 窗口。
    let mut explorer_host = if def_view.is_invalid() { HWND::default() } else { progman };
    if def_view.is_invalid()
    {
        // 顶层 WorkerW 搜索状态。
        let mut search = DefViewSearch { progman, def_view: HWND::default(), explorer_host: HWND::default() };
        unsafe {
            let _ = EnumWindows(Some(find_def_view), LPARAM((&mut search as *mut DefViewSearch) as isize));
        }
        def_view = search.def_view;
        explorer_host = search.explorer_host;
    }
    if def_view.is_invalid() || explorer_host.is_invalid()
    {
        return Err("未发现完整 Explorer 桌面层级".to_string());
    }

    let list_view = unsafe { FindWindowExW(Some(def_view), None, w!("SysListView32"), PCWSTR::null()).unwrap_or_default() }; // Explorer 桌面项目原始坐标所属的列表视图。
    if list_view.is_invalid()
    {
        return Err("未发现 Explorer 桌面项目列表".to_string());
    }

    Ok(ShellDesktopState { def_view: def_view.0 as isize, explorer_host: explorer_host.0 as isize, list_view: list_view.0 as isize, original_items_visible: desktop_items_setting_visible() })
}

/// 读取 Explorer“显示桌面图标”用户设置，不受 Desktop Tools 临时隐藏窗口影响。
pub fn desktop_items_setting_visible() -> bool
{
    let mut state = SHELLFLAGSTATE::default(); // Shell 返回的桌面图标设置位。
    unsafe {
        SHGetSettings(&mut state, SSF_HIDEICONS.0);
    }
    shell_flag_state_icons_visible(state._bitfield)
}

/// 将 Shell `fHideIcons` 位转换为用户可见性语义。
fn shell_flag_state_icons_visible(bitfield: i32) -> bool { bitfield & SHELL_HIDE_ICONS_BIT == 0 }

/// 设置 Explorer 桌面项目列表可见性并等待状态稳定。
pub fn set_desktop_items_visible(def_view_raw: isize, visible: bool) -> bool
{
    // Explorer 桌面项目列表窗口。
    let list_view = unsafe { FindWindowExW(Some(raw_hwnd(def_view_raw)), None, w!("SysListView32"), PCWSTR::null()).unwrap_or_default() };
    if list_view.is_invalid()
    {
        return false;
    }

    unsafe {
        let _ = ShowWindowAsync(list_view, if visible { SW_SHOW } else { SW_HIDE });
    }
    for _ in 0..20
    {
        if unsafe { IsWindowVisible(list_view).as_bool() } == visible
        {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    false
}

/// 返回 Explorer 桌面项目列表当前可见性。
pub fn desktop_items_visible(def_view_raw: isize) -> bool
{
    // Explorer 桌面项目列表窗口。
    let list_view = unsafe { FindWindowExW(Some(raw_hwnd(def_view_raw)), None, w!("SysListView32"), PCWSTR::null()).unwrap_or_default() };
    !list_view.is_invalid() && unsafe { IsWindowVisible(list_view).as_bool() }
}

/// 请求 Explorer 重绘壁纸与桌面项目层。
pub fn redraw_shell_desktop(shell: ShellDesktopState)
{
    for raw in [shell.explorer_host, shell.def_view]
    {
        if unsafe { IsWindow(Some(raw_hwnd(raw))).as_bool() }
        {
            unsafe {
                let _ = RedrawWindow(Some(raw_hwnd(raw)), None, None, RDW_INVALIDATE | RDW_ERASE | RDW_FRAME | RDW_UPDATENOW | RDW_ALLCHILDREN);
            }
        }
    }
}

/// 枚举一台显示器并保存物理边界、工作区与有效 DPI。
unsafe extern "system" fn collect_monitor(monitor: HMONITOR, _device_context: HDC, _monitor_rect: *mut RECT, parameter: LPARAM) -> BOOL
{
    // Windows 显示器信息。
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if !unsafe { GetMonitorInfoW(monitor, (&mut info as *mut MONITORINFOEXW).cast::<MONITORINFO>()).as_bool() }
    {
        return BOOL(1);
    }

    // 主显示器有效 DPI。
    let mut dpi_x = 96;
    // 主显示器垂直 DPI。
    let mut dpi_y = 96;
    unsafe {
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
    }
    let name_length = info.szDevice.iter().position(|character| *character == 0).unwrap_or(info.szDevice.len()); // Windows 显示设备名称的 UTF-16 有效长度。
    let name = String::from_utf16_lossy(&info.szDevice[..name_length]); // 显示器稳定设备名称。
    // 调用方持有的显示器结果集合。
    let result = unsafe { &mut *(parameter.0 as *mut Vec<DesktopHostMonitor>) };
    result.push(DesktopHostMonitor { id: name.clone(), name, primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0, x: info.monitorInfo.rcMonitor.left, y: info.monitorInfo.rcMonitor.top, width: (info.monitorInfo.rcMonitor.right - info.monitorInfo.rcMonitor.left).max(0) as u32, height: (info.monitorInfo.rcMonitor.bottom - info.monitorInfo.rcMonitor.top).max(0) as u32, work_x: info.monitorInfo.rcWork.left, work_y: info.monitorInfo.rcWork.top, work_width: (info.monitorInfo.rcWork.right - info.monitorInfo.rcWork.left).max(0) as u32, work_height: (info.monitorInfo.rcWork.bottom - info.monitorInfo.rcWork.top).max(0) as u32, scale_factor: dpi_x as f64 / 96.0, dpi: dpi_x });
    BOOL(1)
}

/// 返回当前全部显示器身份、物理边界、工作区与有效 DPI。
pub fn enumerate_desktop_monitors() -> Result<Vec<DesktopHostMonitor>, String>
{
    let mut monitors: Vec<DesktopHostMonitor> = Vec::new(); // Windows 显示器枚举结果。
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(collect_monitor), LPARAM((&mut monitors as *mut Vec<DesktopHostMonitor>) as isize));
    }
    if monitors.is_empty()
    {
        return Err("Windows 未返回显示器".to_string());
    }
    monitors.sort_by_key(|monitor| (!monitor.primary, monitor.x, monitor.y));
    Ok(monitors)
}

/// 返回包含指定虚拟桌面物理坐标的显示器稳定身份。
fn monitor_id_at_point(monitors: &[DesktopHostMonitor], point: POINT) -> Option<&str>
{
    monitors
        .iter()
        .find(|monitor| {
            let right = i64::from(monitor.x) + i64::from(monitor.width);
            let bottom = i64::from(monitor.y) + i64::from(monitor.height);
            i64::from(point.x) >= i64::from(monitor.x) && i64::from(point.x) < right && i64::from(point.y) >= i64::from(monitor.y) && i64::from(point.y) < bottom
        })
        .map(|monitor| monitor.id.as_str())
}

/// 返回 Explorer 已附着的桌面视图，调用线程必须已初始化 COM。
pub(super) fn explorer_desktop_view() -> Result<windows::Win32::UI::Shell::IShellView, String>
{
    let shell_windows: IShellWindows = unsafe { CoCreateInstance(&ShellWindows, None, CLSCTX_LOCAL_SERVER).map_err(|error| format!("创建 IShellWindows 失败: {error}"))? }; // Explorer Shell 窗口集合。
    let location = VARIANT::default(); // FindWindowSW 使用的空位置参数。
    let mut desktop_hwnd = 0; // Explorer 桌面自动化窗口句柄。
    let desktop_dispatch = unsafe { shell_windows.FindWindowSW(&location, &location, SWC_DESKTOP, &mut desktop_hwnd, SWFO_NEEDDISPATCH).map_err(|error| format!("查找 Explorer 桌面窗口失败: {error}"))? }; // Explorer 桌面窗口自动化接口。
    let service_provider = desktop_dispatch.cast::<IServiceProvider>().map_err(|error| format!("获取 IServiceProvider 失败: {error}"))?; // 桌面窗口服务查询接口。
    let shell_browser: IShellBrowser = unsafe { service_provider.QueryService(&SID_STopLevelBrowser).map_err(|error| format!("获取 IShellBrowser 失败: {error}"))? }; // 桌面顶层浏览器接口。
    unsafe { shell_browser.QueryActiveShellView() }.map_err(|error| format!("获取 Explorer 桌面视图失败: {error}"))
}

/// 通过 Shell 路径读取 Explorer 桌面项目解析名称和物理屏幕坐标。
fn explorer_desktop_item_positions(list_view_raw: isize) -> Result<HashMap<String, POINT>, String>
{
    if list_view_raw == 0
    {
        return Err("Explorer SysListView32 句柄为空".to_string());
    }
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let folder_view = explorer_desktop_view()?.cast::<IFolderView>().map_err(|error| format!("获取 IFolderView 失败: {error}"))?; // 当前 Explorer 桌面文件夹视图。
    let folder: IShellFolder = unsafe { folder_view.GetFolder().map_err(|error| format!("获取 IShellFolder 失败: {error}"))? }; // 当前桌面文件夹接口。
    let item_count = unsafe { folder_view.ItemCount(SVGIO_ALLVIEW).map_err(|error| format!("读取 Explorer 桌面项目数量失败: {error}"))? }; // Explorer 当前显示的桌面项目数量。
    let mut positions = HashMap::new(); // 解析名称到物理坐标的映射。
    for item_index in 0..item_count
    {
        let pidl = unsafe { folder_view.Item(item_index).map_err(|error| format!("读取 Explorer 桌面项目 PIDL 失败: {error}"))? }; // Explorer 分配的桌面子项目 PIDL。
        let item_result = unsafe {
            (|| -> windows::core::Result<(String, POINT)> {
                let shell_item: IShellItem = SHCreateItemWithParent(None, &folder, pidl)?; // 当前子项目的 ShellItem。
                let name_pointer = shell_item.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING)?; // Shell 分配的绝对解析名称。
                let parsing_name = name_pointer.to_string(); // UTF-16 解析名称转换结果。
                CoTaskMemFree(Some(name_pointer.0.cast()));
                let mut point = folder_view.GetItemPosition(pidl)?; // Explorer 视图中的项目坐标。
                if !ClientToScreen(raw_hwnd(list_view_raw), &mut point).as_bool()
                {
                    return Err(windows::core::Error::from_win32());
                }
                Ok((parsing_name?, point))
            })()
        };
        unsafe { CoTaskMemFree(Some(pidl.cast())) };
        if let Ok((parsing_name, point)) = item_result
        {
            positions.insert(parsing_name.to_lowercase(), point);
        }
    }
    Ok(positions)
}

/// 按准确项目身份读取 Explorer 显示器归属及工作区 DIP 坐标，仅用于首次导入与新增项目。
pub fn desktop_item_monitor_assignments(items: &[DesktopItem], monitors: &[DesktopHostMonitor], shell: ShellDesktopState) -> Vec<(String, String, crate::contracts::WorkspacePoint)>
{
    let positions = match explorer_desktop_item_positions(shell.list_view)
    {
        Ok(positions) => positions,
        Err(error) =>
        {
            crate::log_warn!("工作区：读取 Explorer 项目显示器归属失败，项目={}，显示器={}，错误={error}", items.len(), monitors.len());
            return Vec::new();
        }
    }; // Explorer 当前项目物理坐标。
    let assignments = match_desktop_item_monitor_assignments(items, monitors, &positions); // Explorer 坐标与当前显示器边界的匹配结果。
    crate::log_debug!("工作区：Explorer 项目显示器归属已读取，项目={}，匹配={}，显示器={}", items.len(), assignments.len(), monitors.len());
    assignments
}

/// 将 Explorer 解析名称坐标映射为 Workspace 使用的稳定显示器归属。
fn match_desktop_item_monitor_assignments(items: &[DesktopItem], monitors: &[DesktopHostMonitor], positions: &HashMap<String, POINT>) -> Vec<(String, String, crate::contracts::WorkspacePoint)>
{
    items
        .iter()
        .filter_map(|item| {
            let point = positions.get(&item.stable_reference.to_lowercase())?; // 当前项目的 Explorer 物理坐标。
            let monitor_id = monitor_id_at_point(monitors, *point)?; // 当前物理坐标所属显示器。
            let monitor = monitors.iter().find(|monitor| monitor.id == monitor_id)?; // 物理坐标转换到目标屏工作区 DIP。
            Some((item.stable_reference.clone(), monitor_id.to_string(), crate::contracts::WorkspacePoint { left: (f64::from(point.x) - f64::from(monitor.work_x)) / monitor.scale_factor, top: (f64::from(point.y) - f64::from(monitor.work_y)) / monitor.scale_factor }))
        })
        .collect()
}

/// 正式宿主窗口过程。
unsafe extern "system" fn host_window_proc(hwnd: HWND, message: u32, wparam: windows::Win32::Foundation::WPARAM, lparam: windows::Win32::Foundation::LPARAM) -> windows::Win32::Foundation::LRESULT
{
    if matches!(message, windows::Win32::UI::WindowsAndMessaging::WM_DISPLAYCHANGE | windows::Win32::UI::WindowsAndMessaging::WM_DPICHANGED)
    {
        super::environment_watch::signal_environment();
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

/// 注册正式宿主窗口类。
fn register_host_class() -> Result<(), String>
{
    // 当前模块句柄。
    let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.map_err(|error| format!("读取正式宿主模块句柄失败: {error}"))?;
    // 正式宿主窗口类描述。
    let class = WNDCLASSEXW { cbSize: size_of::<WNDCLASSEXW>() as u32, style: CS_HREDRAW | CS_VREDRAW, lpfnWndProc: Some(host_window_proc), hInstance: HINSTANCE(module.0), lpszClassName: HOST_WINDOW_CLASS, ..Default::default() };
    // 正式宿主窗口类注册结果。
    let atom = unsafe { RegisterClassExW(&class) };
    if atom == 0 && unsafe { GetLastError() } != ERROR_CLASS_ALREADY_EXISTS
    {
        return Err(format!("注册正式宿主窗口类失败: {}", windows::core::Error::from_win32()));
    }
    Ok(())
}

/// 将正式宿主附着到 Explorer 图标宿主并定位到工作区。
fn attach_host(hwnd: isize, shell: ShellDesktopState, monitor: &DesktopHostMonitor) -> Result<(), String>
{
    // 工作区左上角在 Explorer 宿主中的客户区坐标。
    let mut top_left = POINT { x: monitor.work_x, y: monitor.work_y };
    unsafe {
        let _ = ScreenToClient(raw_hwnd(shell.explorer_host), &mut top_left);
    }
    // 正式宿主当前窗口样式。
    let style = unsafe { GetWindowLongPtrW(raw_hwnd(hwnd), GWL_STYLE) } as u32;
    // Explorer 子窗口样式。
    let child_style = (style & !WS_POPUP.0) | WS_CHILD.0;
    unsafe {
        SetWindowLongPtrW(raw_hwnd(hwnd), GWL_STYLE, child_style as isize);
        let _ = SetParent(raw_hwnd(hwnd), Some(raw_hwnd(shell.explorer_host)));
        SetWindowPos(raw_hwnd(hwnd), Some(raw_hwnd(shell.def_view)), top_left.x, top_left.y, monitor.work_width as i32, monitor.work_height as i32, SWP_NOOWNERZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED).map_err(|error| format!("定位正式宿主失败: {error}"))?;
    }
    if unsafe { GetParent(raw_hwnd(hwnd)).unwrap_or_default() } != raw_hwnd(shell.explorer_host)
    {
        return Err("正式宿主父级校验失败".to_string());
    }
    Ok(())
}

/// 为目标显示器固定 WebView2 raw-pixel Bounds 与栅格化比例。
fn configure_controller(controller: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller, monitor: &DesktopHostMonitor) -> Result<(), String>
{
    // 支持显式 BoundsMode 与 RasterizationScale 的 Controller3。
    let controller3 = controller.cast::<ICoreWebView2Controller3>().map_err(|error| format!("读取 WebView2 Controller3 失败: {error}"))?;
    unsafe {
        controller3.SetBoundsMode(COREWEBVIEW2_BOUNDS_MODE_USE_RAW_PIXELS).map_err(|error| format!("设置 WebView2 raw-pixel Bounds 失败: {error}"))?;
        controller3.SetShouldDetectMonitorScaleChanges(false).map_err(|error| format!("关闭 WebView2 自动显示器缩放失败: {error}"))?;
        controller3.SetRasterizationScale(monitor.scale_factor).map_err(|error| format!("设置 WebView2 栅格化比例失败: {error}"))?;
    }
    Ok(())
}

/// 生成本地页面资产或命令响应。
fn protocol_response(status: StatusCode, content_type: &str, csp: Option<&str>, body: Cow<'static, [u8]>) -> Response<Cow<'static, [u8]>>
{
    // 当前响应构建器。
    let mut builder = Response::builder().status(status).header("Content-Type", content_type);
    if let Some(csp) = csp
    {
        builder = builder.header("Content-Security-Policy", csp);
    }
    builder.body(body).expect("FATAL 正式宿主协议响应构造失败")
}

/// 按路径段做百分号解码。段内的 `%2F`、`%5C` 不会变成分隔符。
fn decode_plugin_relative(relative: &str) -> Option<String>
{
    if relative.is_empty() || relative.contains(['\\', '\0'])
    {
        return None;
    }
    let mut decoded = String::new(); // 解码后的相对路径，仍用 `/` 分段。
    for (index, segment) in relative.split('/').enumerate()
    {
        if index > 0
        {
            decoded.push('/');
        }
        let segment = percent_decode_segment(segment)?; // 非法 %xx 或非 UTF-8 直接拒绝。
        if segment.is_empty() || segment == "." || segment == ".." || segment.contains(['/', '\\', ':', '\0'])
        {
            return None;
        }
        decoded.push_str(&segment);
    }
    Some(decoded)
}

/// 把单个路径段从百分号编码还原成 UTF-8。`+` 保持原样。
fn percent_decode_segment(segment: &str) -> Option<String>
{
    let bytes = segment.as_bytes(); // 百分号序列按字节解释。
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len()
    {
        if bytes[index] == b'%'
        {
            if index + 2 >= bytes.len()
            {
                return None;
            }
            let high = hex_value(bytes[index + 1])?;
            let low = hex_value(bytes[index + 2])?;
            decoded.push((high << 4) | low);
            index += 3;
        }
        else
        {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

/// 十六进制数字，大小写均可。
fn hex_value(byte: u8) -> Option<u8>
{
    match byte
    {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// 只读取插件根目录内的 JS/CSS，任何链接、重解析点或非普通路径均拒绝。
fn read_plugin_asset(directory: &std::path::Path, relative: &str) -> Option<(&'static str, Vec<u8>)>
{
    use std::{
        fs,
        os::windows::fs::MetadataExt,
        path::{Component, Path},
    };
    let relative = decode_plugin_relative(relative)?; // 先按段解码，再做穿越校验。
    let relative_path = Path::new(&relative);
    if relative_path.components().any(|part| !matches!(part, Component::Normal(_)))
    {
        return None;
    }
    let content_type = match relative_path.extension()?.to_str()?
    {
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        _ => return None,
    }; // 其他格式不能通过插件协议读取。
    let candidate = directory.join(relative_path); // 尚未跟随链接的请求目标。
    for ancestor in candidate.ancestors().take_while(|ancestor| ancestor.starts_with(directory))
    {
        let metadata = fs::symlink_metadata(ancestor).ok()?; // 仅检查插件根及其内部，不约束用户目录重定向。
        if metadata.file_type().is_symlink() || metadata.file_attributes() & 0x400 != 0
        {
            return None;
        }
    }
    let root = fs::canonicalize(directory).ok()?; // 实际插件根。
    let path = fs::canonicalize(candidate).ok()?; // 实际资产路径必须仍在插件根内。
    if !path.starts_with(&root) || !fs::metadata(&path).ok()?.is_file()
    {
        return None;
    }
    Some((content_type, fs::read(path).ok()?))
}

/// 路由插件资产、Tauri 内嵌资产和正式宿主命令。
fn handle_protocol_request(request: Request<Vec<u8>>, app: &AppHandle, command_handler: &NativeCommandHandler) -> Response<Cow<'static, [u8]>>
{
    // 当前本地协议请求路径。
    let path = request.uri().path();
    if let Some(relative) = path.strip_prefix("/plugins/")
    {
        if let Some(runtime) = app.try_state::<crate::component_runtime::ComponentRuntimeState>()
            && let Some((content_type, bytes)) = read_plugin_asset(runtime.plugin_directory(), relative)
        {
            return protocol_response(StatusCode::OK, content_type, None, Cow::Owned(bytes));
        }
        crate::log_warn!("插件：资产不存在或路径不受支持，相对路径={relative}");
        return protocol_response(StatusCode::NOT_FOUND, "text/plain; charset=utf-8", None, Cow::Borrowed(b"not found"));
    }
    if let Some(command) = path.strip_prefix("/__desktop_tools_invoke/")
    {
        // 前端提交的命令参数。
        let arguments = serde_json::from_slice(request.body()).unwrap_or_else(|_| json!({}));
        // 正式宿主命令处理结果。
        let payload = match command_handler(command, arguments)
        {
            Ok(result) => json!({ "ok": true, "result": result }),
            Err(error) =>
            {
                crate::log_error!("宿主：本地命令执行失败，命令={command}，错误={error}");
                json!({ "ok": false, "error": error })
            }
        };
        return protocol_response(StatusCode::OK, "application/json; charset=utf-8", None, Cow::Owned(serde_json::to_vec(&payload).expect("FATAL 正式宿主命令响应序列化失败")));
    }

    // Tauri 构建时嵌入的 React 资产路径。
    let asset_path = path.trim_start_matches('/');
    // 空路径映射到 React 入口页面。
    let asset_path = if asset_path.is_empty() { "index.html" } else { asset_path };
    if let Some(asset) = app.asset_resolver().get(asset_path.to_string())
    {
        // 资产元数据在移动字节缓冲区前转为独立值。
        let content_type = asset.mime_type().to_string();
        // 当前资产附加的内容安全策略。
        let csp = asset.csp_header().map(str::to_string);
        return protocol_response(StatusCode::OK, &content_type, csp.as_deref(), Cow::Owned(asset.bytes));
    }

    protocol_response(StatusCode::NOT_FOUND, "text/plain; charset=utf-8", None, Cow::Borrowed(b"not found"))
}

/// 设置窗口自定义协议名。Windows 上页面来源为 `http://desktop-tools-settings.localhost`。
pub(crate) const SETTINGS_PROTOCOL: &str = "desktop-tools-settings";

/// 设置窗口页面地址，与协议名对应，供同源 `import("/plugins/...")`。
pub(crate) fn settings_page_url() -> tauri::Url { tauri::Url::parse(&format!("http://{SETTINGS_PROTOCOL}.localhost/")).expect("设置窗口协议地址无效") }

/// 设置窗口协议入口。页面、插件资产和原生命令都交给现有处理函数。
pub(crate) fn handle_settings_protocol_request(request: Request<Vec<u8>>, app: &AppHandle) -> Response<Cow<'static, [u8]>>
{
    // 请求到达时再取宿主运行时；设置窗口打开时运行时已经放进应用状态。
    let handler: NativeCommandHandler = Arc::new({
        let app = app.clone(); // 命令闭包独立持有应用句柄。
        move |command, arguments| {
            let Some(runtime) = app.try_state::<Arc<crate::host::DesktopHostRuntime>>()
            else
            {
                return Err(json!({ "code": "desktop-host-starting", "message": "桌面宿主正在启动" }));
            }; // 与逐屏宿主同一条命令边界。
            crate::commands::dispatch_native_command(&app, &runtime, command, arguments)
        }
    });
    handle_protocol_request(request, app, &handler)
}

/// 静态资产在协议线程直接返回；原生命令改到阻塞池，避免占用 WebView UI 线程。
pub(crate) fn dispatch_settings_protocol<F>(request: Request<Vec<u8>>, app: AppHandle, respond: F)
where
    F: FnOnce(Response<Cow<'static, [u8]>>) + Send + 'static,
{
    let invoke = request.uri().path().starts_with("/__desktop_tools_invoke/"); // 只有命令可能等待 Shell STA。
    let run = move || respond(handle_settings_protocol_request(request, &app));
    if invoke
    {
        tauri::async_runtime::spawn_blocking(run);
    }
    else
    {
        run();
    }
}

/// 保留开发 URL 的路径与查询字段，并设置宿主身份；正式模式使用内嵌资源。
fn desktop_page_url(native_origin: &str, dev_url: Option<&tauri::Url>, index: usize) -> tauri::Url
{
    let mut url = dev_url.cloned().unwrap_or_else(|| tauri::Url::parse(&format!("{native_origin}/index.html")).expect("宿主协议地址无效")); // 当前模式的页面来源。
    let pairs: Vec<(String, String)> = url.query_pairs().filter(|(key, _)| key != "desktop-host" && key != "monitor").map(|(key, value)| (key.into_owned(), value.into_owned())).collect(); // 保留非宿主查询字段。
    url.set_query(None);
    url.query_pairs_mut().extend_pairs(pairs).append_pair("desktop-host", "1").append_pair("monitor", &index.to_string());
    url
}

/// 检查命令来源；开发来源只由调用方在 Tauri 开发模式下注入。
fn allowed_desktop_origin(origin: &str, native_origin: &str, dev_origin: Option<&str>) -> bool { origin == native_origin || dev_origin == Some(origin) }

/// 为开发页面提供受限跨域命令访问，预检不执行任何原生命令。
fn handle_desktop_request(request: Request<Vec<u8>>, app: &AppHandle, handler: &NativeCommandHandler, native_origin: &str, dev_origin: Option<&str>) -> Response<Cow<'static, [u8]>>
{
    if !request.uri().path().starts_with("/__desktop_tools_invoke/")
    {
        return handle_protocol_request(request, app, handler);
    }
    let origin = request.headers().get("Origin").and_then(|value| value.to_str().ok()).unwrap_or(""); // 不允许缺失或不可信来源调用命令。
    if !allowed_desktop_origin(origin, native_origin, dev_origin)
    {
        return protocol_response(StatusCode::FORBIDDEN, "text/plain", None, Cow::Borrowed(b"forbidden origin"));
    }
    let cors_origin = (dev_origin == Some(origin)).then(|| origin.to_string()); // 仅开发页面需要 CORS 响应。
    let mut response = if request.method() == wry::http::Method::OPTIONS
    {
        protocol_response(StatusCode::NO_CONTENT, "text/plain", None, Cow::Borrowed(b""))
    }
    else if request.method() == wry::http::Method::POST
    {
        handle_protocol_request(request, app, handler)
    }
    else
    {
        protocol_response(StatusCode::METHOD_NOT_ALLOWED, "text/plain", None, Cow::Borrowed(b"POST required"))
    }; // 只执行 POST，OPTIONS 仅回应预检。
    if let Some(origin) = cors_origin
    {
        let headers = response.headers_mut(); // 仅精确放行配置的开发服务器 Origin。
        headers.insert("Access-Control-Allow-Origin", origin.parse().expect("开发 Origin 无效"));
        headers.insert("Access-Control-Allow-Methods", "POST".parse().unwrap());
        headers.insert("Access-Control-Allow-Headers", "Content-Type".parse().unwrap());
        headers.insert("Vary", "Origin".parse().unwrap());
    }
    response
}

#[cfg(test)]
mod desktop_dev_tests
{
    use super::{allowed_desktop_origin, desktop_page_url};

    /// 插件资产仅允许根内 JS/CSS，拒绝穿越、设备流和目录联接。
    #[test]
    fn plugin_assets_reject_unsafe_paths_and_reparse_points()
    {
        let root = std::env::temp_dir().join(format!("desktop-tools-plugin-assets-{}", std::process::id())); // 独占资产目录。
        let directory = root.join("plugin"); // 插件来源根。
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("plugin.test.js"), "window.test = true;").unwrap();
        std::fs::write(directory.join("plugin.test.css"), "body {}").unwrap();
        std::fs::write(directory.join("private.json"), "{}").unwrap();
        assert_eq!(super::read_plugin_asset(&directory, "plugin.test.js").unwrap().0, "text/javascript; charset=utf-8");
        assert_eq!(super::read_plugin_asset(&directory, "plugin.test.css").unwrap().1, b"body {}");
        for path in ["", "../outside.js", "/plugin.test.js", "private.json", "missing.js", "plugin.test.js:stream", "%2e%2e/outside.js", r"..\outside.js"]
        {
            assert!(super::read_plugin_asset(&directory, path).is_none(), "不安全路径应拒绝: {path}");
        }
        std::fs::create_dir(directory.join("nested")).unwrap();
        std::fs::write(directory.join("nested/plugin.test.js"), "redirected").unwrap();
        let junction = directory.join("linked"); // 目录联接不需要创建符号链接权限。
        assert!(std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(&junction).arg(&directory).output().unwrap().status.success());
        assert!(super::read_plugin_asset(&directory, "linked/plugin.test.js").is_none());
        assert_eq!(super::read_plugin_asset(&junction.join("nested"), "plugin.test.js").unwrap().1, b"redirected"); // 插件根以上的目录联接允许使用。
        std::fs::remove_dir(&junction).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// 中文目录名按百分号解码后能读到脚本；穿越和非法编码仍是 404。
    #[test]
    fn percent_encoded_plugin_paths_decode_and_stay_inside()
    {
        let root = std::env::temp_dir().join(format!("desktop-tools-plugin-encoded-{}", std::process::id())); // 独占资产目录。
        let directory = root.join("plugin"); // 插件来源根。
        let folder = directory.join("演示插件-正常"); // 与实机请求同一文件夹名。
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("main.js"), "encoded").unwrap();
        let encoded = "%E6%BC%94%E7%A4%BA%E6%8F%92%E4%BB%B6-%E6%AD%A3%E5%B8%B8/main.js"; // 浏览器对非 ASCII 段的编码。
        assert_eq!(super::read_plugin_asset(&directory, encoded).unwrap().1, b"encoded");
        assert_eq!(super::read_plugin_asset(&directory, "演示插件-正常/main.js").unwrap().1, b"encoded");
        for path in ["%2e%2e/main.js", "ok/%2e%2e/main.js", "%2e%2e%2fmain.js", "%2E/main.js", "%5cmain.js", "%00main.js"]
        {
            assert!(super::read_plugin_asset(&directory, path).is_none(), "解码后仍应拒绝: {path}");
        }
        for path in ["%ZZ/main.js", "%E6/main.js", "%FF/main.js", "main%2Ejs%"]
        {
            assert!(super::read_plugin_asset(&directory, path).is_none(), "非法编码应 404: {path}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 完整围栏图像超过普通消息上限时，两层必须接受相同的有界载荷。
    #[test]
    fn tile_drag_payload_uses_shared_budget()
    {
        let payload = serde_json::json!({"phase": "move", "kind": "fence", "visuals": [{"content": "x".repeat(70000)}]}); // 超过旧 64 KB 门槛的围栏预览。
        assert!(payload.to_string().len() > 65536);
        assert!(payload.to_string().len() < super::drag_payload_limit(&payload));
        assert_eq!(super::drag_payload_limit(&serde_json::json!({"kind": "items"})), 65536);
        assert_eq!(super::drag_payload_limit(&serde_json::json!({"phase": "ole-preview"})), 1048576);
    }

    /// 开发路径与原生命令来源独立，正式模式不接收开发来源。
    #[test]
    fn development_url_and_origin_are_scoped()
    {
        let native = "http://desktop-tools-host-2.localhost"; // 第二个显示器的协议来源。
        let dev = tauri::Url::parse("http://127.0.0.1:1420/app/?theme=dark&monitor=9").unwrap(); // 带路径和已有查询的开发入口。
        let page = desktop_page_url(native, Some(&dev), 2); // 开发页面保留原路径，替换显示器字段。
        assert_eq!(page.path(), "/app/");
        assert_eq!(page.query(), Some("theme=dark&desktop-host=1&monitor=2"));
        assert_eq!(desktop_page_url(native, None, 2).as_str(), "http://desktop-tools-host-2.localhost/index.html?desktop-host=1&monitor=2");
        assert!(allowed_desktop_origin(native, native, None));
        assert!(allowed_desktop_origin("http://127.0.0.1:1420", native, Some("http://127.0.0.1:1420")));
        for origin in ["http://127.0.0.1:1420", "http://desktop-tools-host-1.localhost", "null", ""]
        {
            assert!(!allowed_desktop_origin(origin, native, None));
        }
        assert!(!allowed_desktop_origin("http://127.0.0.1:14200", native, Some("http://127.0.0.1:1420")));
    }

    /// 设置窗口来源必须等于协议名加 `.localhost`，否则 Tauri 不把它当本地 IPC 来源。
    #[test]
    fn settings_page_url_matches_protocol_name()
    {
        let url = super::settings_page_url(); // 设置窗口实际导航地址。
        assert_eq!(url.as_str(), "http://desktop-tools-settings.localhost/");
        assert_eq!(url.host_str().and_then(|host| host.strip_suffix(".localhost")), Some(super::SETTINGS_PROTOCOL));
    }
}

/// 创建一台显示器对应的正式透明 Windowed WebView2 宿主。
fn create_desktop_host(app: &AppHandle, shell: ShellDesktopState, command_handler: NativeCommandHandler, monitor: &DesktopHostMonitor, index: usize) -> Result<NativeDesktopHost, String>
{
    // 当前模块句柄。
    let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.map_err(|error| format!("读取正式宿主模块句柄失败: {error}"))?;
    // 正式宿主外层 HWND。
    let hwnd = unsafe { CreateWindowExW(WINDOW_EX_STYLE::default(), HOST_WINDOW_CLASS, w!("Desktop Tools"), WS_POPUP | WS_CLIPCHILDREN | WS_CLIPSIBLINGS | WS_VISIBLE, monitor.work_x, monitor.work_y, monitor.work_width as i32, monitor.work_height as i32, None, None, Some(HINSTANCE(module.0)), None) }.map_err(|error| format!("创建正式宿主 HWND 失败: {error}"))?;
    // Wry 使用的正式宿主句柄包装。
    let raw_window = NativeWindowHandle { hwnd: NonZeroIsize::new(hwnd.0 as isize).ok_or("正式宿主 HWND 为空")? };
    // 本地协议使用的 AppHandle。
    let protocol_app = app.clone();
    // 本地协议使用的命令处理器。
    let protocol_handler = Arc::clone(&command_handler);
    let label = format!("desktop-host-{index}"); // Wry 内部使用的逐屏宿主标识。
    // 页面导航进度记录使用的显示器身份。
    let load_monitor = monitor.id.clone();
    let protocol = format!("desktop-tools-host-{index}"); // 避免多 WebView 协议处理器冲突的逐屏 scheme。
    let native_origin = format!("http://{protocol}.localhost"); // 本屏桥接始终使用自己的协议。
    let dev_url = if tauri::is_dev() { app.config().build.dev_url.as_ref() } else { None }; // Debug 构建不等同于 Tauri 开发模式。
    let dev_origin = dev_url.map(|url| url.origin().ascii_serialization()); // 正式模式不会放行开发 Origin。
    let page_url = desktop_page_url(&native_origin, dev_url, index).to_string(); // 页面与刷新共享当前模式地址。
    let initialization_script = NATIVE_BRIDGE_SCRIPT.replace("__DESKTOP_NATIVE_ORIGIN__", &serde_json::to_string(&native_origin).expect("宿主地址序列化失败")).replace("__REFRESH_POLICY_FALLBACK__", if super::refresh_policy::fallback_enabled() { "true" } else { "false" }); // JSON 编码避免脚本字符串转义错误，Hook 状态在宿主创建前确定。
    super::refresh_policy::set_paused(&monitor.id, false);
    crate::log_info!("宿主：页面来源={}，原生命令来源={}", page_url, native_origin);
    // 正式 Windowed WebView2。
    let webview = WebViewBuilder::new()
        .with_id(&label)
        .with_transparent(true)
        .with_visible(true)
        .with_bounds(WryRect { position: dpi::PhysicalPosition::new(0, 0).into(), size: dpi::PhysicalSize::new(monitor.work_width, monitor.work_height).into() })
        .with_initialization_script(&initialization_script)
        .with_asynchronous_custom_protocol(protocol, move |_id, request, responder| {
            if request.uri().path().starts_with("/__desktop_tools_invoke/")
            {
                let app = protocol_app.clone(); // 命令可以等待 Shell STA，UI 线程必须继续泵送 OLE 与渲染确认。
                let handler = Arc::clone(&protocol_handler); // 复用同一正式命令边界。
                let native_origin = native_origin.clone(); // 后台命令独立持有所需的来源白名单。
                let dev_origin = dev_origin.clone(); // 开发来源只在开发构建可用。
                tauri::async_runtime::spawn_blocking(move || responder.respond(handle_desktop_request(request, &app, &handler, &native_origin, dev_origin.as_deref())));
            }
            else
            {
                responder.respond(handle_protocol_request(request, &protocol_app, &protocol_handler));
            } // 已嵌入的静态资产不需要工作线程。
        })
        .with_on_page_load_handler(move |event, url| {
            // 初始占位页不代表真实页面的加载进度，不能作为就绪等待基准。
            let placeholder = url == "about:blank";
            // 记录导航进度，恢复线程据此区分仍在加载与已加载但未就绪。
            let stage = match event
            {
                PageLoadEvent::Started =>
                {
                    if !placeholder
                    {
                        clear_page_load_finished(&load_monitor);
                    }
                    "开始"
                }
                PageLoadEvent::Finished =>
                {
                    if !placeholder
                    {
                        mark_page_load_finished(&load_monitor);
                    }
                    "完成"
                }
            };
            crate::log_debug!("宿主：React 页面加载{}，URL：{}", stage, url);
        })
        .with_url("about:blank")
        .build_as_child(&raw_window)
        .map_err(|error| format!("创建正式 Windowed WebView2 失败: {error}"))?;

    // 正式 WebView2 的原生 Controller。
    let controller = webview.controller();
    attach_host(hwnd.0 as isize, shell, monitor)?;
    configure_controller(&controller, monitor)?;
    unsafe {
        controller.NotifyParentWindowPositionChanged().map_err(|error| format!("通知 WebView2 宿主位置变化失败: {error}"))?;
    }
    let (drop_targets, ole_initialized) = register_shell_drop_targets(hwnd, app, monitor)?; // Desktop Tools 接收外部 Shell IDataObject 的窗口注册。
    crate::log_debug!("宿主：逐屏透明宿主已创建，显示器={}，主显示器={}，工作区：[{}, {}, {}, {}]，DPI：{}，栅格化比例：{}，BoundsMode：USE_RAW_PIXELS", monitor.id, u8::from(monitor.primary), monitor.work_x, monitor.work_y, monitor.work_width, monitor.work_height, monitor.dpi, monitor.scale_factor);

    NATIVE_HOSTS.with(|hosts| {
        hosts.borrow_mut().push(NativeHostResources { monitor_id: monitor.id.clone(), hwnd: hwnd.0 as isize, webview, page_url, drop_targets, ole_initialized });
    });
    Ok(NativeDesktopHost { hwnd: hwnd.0 as isize, monitor: monitor.clone() })
}

/// 为每台显示器创建独立正式宿主。
pub fn create_desktop_hosts(app: &AppHandle, shell: ShellDesktopState, command_handler: NativeCommandHandler, monitors: &[DesktopHostMonitor]) -> Result<Vec<NativeDesktopHost>, String>
{
    if monitors.is_empty()
    {
        return Err("正式宿主缺少显示器".to_string());
    }
    register_host_class()?;
    let mut hosts = Vec::with_capacity(monitors.len()); // 已完整创建的逐屏原生宿主记录。
    for (index, monitor) in monitors.iter().enumerate()
    {
        match create_desktop_host(app, shell, Arc::clone(&command_handler), monitor, index)
        {
            Ok(host) => hosts.push(host),
            Err(error) =>
            {
                destroy_desktop_hosts();
                return Err(error);
            }
        }
    }
    Ok(hosts)
}

/// 保留未变化且页面索引一致的宿主，只创建并导航发生变化的显示器。
pub fn reconcile_desktop_hosts(app: &AppHandle, shell: ShellDesktopState, command_handler: NativeCommandHandler, monitors: &[DesktopHostMonitor], previous: &[NativeDesktopHost]) -> Result<Vec<NativeDesktopHost>, String>
{
    let retained = previous.iter().enumerate().filter(|(index, host)| monitors.get(*index) == Some(&host.monitor)).map(|(_, host)| host.clone()).collect::<Vec<_>>(); // 索引变化会改变页面身份，不能复用。
    destroy_matching_hosts(|id| !retained.iter().any(|host| host.monitor.id == id));
    let mut result = Vec::with_capacity(monitors.len()); // 最新顺序的完整宿主集合。
    for (index, monitor) in monitors.iter().enumerate()
    {
        if let Some(host) = retained.iter().find(|host| host.monitor.id == monitor.id)
        {
            if unsafe { GetParent(raw_hwnd(host.hwnd)).unwrap_or_default() } != raw_hwnd(shell.explorer_host)
            {
                attach_host(host.hwnd, shell, monitor)?;
            } // Win+D 可以迁移 Explorer 父窗口，仍有效的 WebView 与 OLE 生命周期不重建。
            result.push(host.clone());
            continue;
        }
        let host = create_desktop_host(app, shell, Arc::clone(&command_handler), monitor, index)?; // 仅新建受影响的宿主。
        NATIVE_HOSTS.with(|hosts| {
            let hosts = hosts.borrow(); // 同线程持有的新 WebView。
            let resource = hosts.iter().find(|resource| resource.hwnd == host.hwnd).ok_or("新宿主资源不存在")?; // 待导航资源。
            resource.webview.load_url(&resource.page_url).map_err(|error| error.to_string())
        })?;
        result.push(host);
    }
    Ok(result)
}

/// 导航全部正式宿主到 Tauri 内嵌 React 页面。
pub fn load_desktop_hosts() -> Result<(), String>
{
    NATIVE_HOSTS.with(|hosts| {
        for host in hosts.borrow().iter()
        {
            host.webview.load_url(&host.page_url).map_err(|error| format!("加载正式宿主 React 页面失败，显示器={}，错误={error}", host.monitor_id))?;
        }
        Ok(())
    })
}

/// 按当前轻量恢复路径刷新一个 Renderer，并保留外层 HWND 与命中区域。
pub fn reload_desktop_host(monitor_id: &str) -> Result<(), String>
{
    super::directory_watch::clear_monitor(monitor_id);
    crate::component_runtime::ComponentRuntimeState::clear_time_monitor(monitor_id);
    crate::weather::activity::clear_monitor(monitor_id);
    super::refresh_policy::set_paused(monitor_id, false);
    clear_page_load_finished(monitor_id);
    NATIVE_HOSTS.with(|hosts| {
        let hosts = hosts.borrow(); // 当前 UI 线程持有的全部正式 WebView2 宿主。
        let host = hosts.iter().find(|host| host.monitor_id == monitor_id).ok_or_else(|| format!("Renderer 重载目标不存在，显示器={monitor_id}"))?;
        if host.webview.evaluate_script("window.location.reload()").is_ok()
        {
            return Ok(());
        }
        host.webview.load_url(&host.page_url).map_err(|error| format!("Renderer 轻量重载失败，显示器={monitor_id}，错误={error}"))
    })
}

/// 在宿主 UI 线程转发有界拖动消息；内容只作为 CustomEvent 数据，不插入 HTML。
pub(crate) fn drag_payload_limit(payload: &serde_json::Value) -> usize
{
    if payload.get("phase").and_then(serde_json::Value::as_str) == Some("ole-preview") || payload.get("kind").and_then(serde_json::Value::as_str) == Some("fence")
    {
        1024 * 1024
    }
    else
    {
        65536
    }
}

/// 在宿主线程转发通过同一大小限制的拖动消息。
pub fn relay_workspace_drag(payload: serde_json::Value) -> Result<(), String>
{
    if payload.get("phase").and_then(serde_json::Value::as_str) == Some("ole-preview")
    {
        return super::drop_target::publish_preview(serde_json::from_value(payload).map_err(|error| format!("拖放预览格式无效：{error}"))?);
    }
    let encoded = serde_json::to_string(&payload).map_err(|error| error.to_string())?; // 安全编码的事件数据。
    let limit = drag_payload_limit(&payload); // 命令入口与实际转发共用同一个载荷预算。
    if encoded.len() > limit
    {
        return Err("跨屏拖动消息过大".into());
    }
    let script = format!("window.dispatchEvent(new CustomEvent('desktop-tools-drag',{{detail:{encoded}}}))"); // 仅分发给本应用宿主。
    NATIVE_HOSTS.with(|hosts| {
        for host in hosts.borrow().iter()
        {
            host.webview.evaluate_script(&script).map_err(|error| error.to_string())?;
        }
        Ok(())
    })
}

/// 显示或撤下正式宿主 HWND。
pub fn set_host_visible(hwnd: isize, visible: bool) -> bool
{
    unsafe {
        let _ = ShowWindow(raw_hwnd(hwnd), if visible { SW_SHOW } else { SW_HIDE });
    }
    for _ in 0..20
    {
        if unsafe { IsWindowVisible(raw_hwnd(hwnd)).as_bool() } == visible
        {
            return true;
        }
        thread::sleep(Duration::from_millis(25));
    }
    false
}

/// 原子替换并应用正式宿主的完整语义命中图。
pub fn apply_host_hit_regions(host: &NativeDesktopHost, regions: Vec<HitRegion>) -> Result<usize, String>
{
    // 合并全部语义矩形的最终窗口区域。
    let combined_region = unsafe { CreateRectRgn(0, 0, 0, 0) };
    if combined_region.is_invalid()
    {
        return Err("创建正式宿主命中区域失败".to_string());
    }

    for region in &regions
    {
        // CSS 坐标转换后的物理像素矩形。
        let left = (region.left * host.monitor.scale_factor).floor() as i32;
        // 物理像素矩形顶边。
        let top = (region.top * host.monitor.scale_factor).floor() as i32;
        // 物理像素矩形右边。
        let right = ((region.left + region.width) * host.monitor.scale_factor).ceil() as i32;
        // 物理像素矩形底边。
        let bottom = ((region.top + region.height) * host.monitor.scale_factor).ceil() as i32;
        // 当前语义对象对应的临时 GDI 区域。
        let item_region = unsafe { CreateRectRgn(left, top, right, bottom) };
        if item_region.is_invalid()
        {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(combined_region.0));
            }
            return Err("创建语义对象命中区域失败".to_string());
        }
        unsafe {
            let _ = CombineRgn(Some(combined_region), Some(combined_region), Some(item_region), RGN_OR);
            let _ = DeleteObject(HGDIOBJ(item_region.0));
        }
    }
    if unsafe { SetWindowRgn(raw_hwnd(host.hwnd), Some(combined_region), true) } == 0
    {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(combined_region.0));
        }
        return Err("应用正式宿主命中区域失败".to_string());
    }
    unsafe {
        SetWindowPos(raw_hwnd(host.hwnd), Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE).map_err(|error| format!("提升正式宿主输入层级失败: {error}"))?;
    }
    HIT_REGIONS.get_or_init(|| RwLock::new(HashMap::new())).write().map_err(|error| error.to_string())?.insert(host.monitor.id.clone(), regions);
    Ok(HIT_REGIONS.get().expect("命中图已初始化").read().map_err(|error| error.to_string())?.get(&host.monitor.id).map(Vec::len).unwrap_or(0))
}

/// 在连续拖动期间临时恢复正式宿主完整窗口区域。
pub fn suspend_host_hit_regions(host: &NativeDesktopHost) -> Result<(), String>
{
    if unsafe { SetWindowRgn(raw_hwnd(host.hwnd), None, true) } == 0
    {
        return Err("暂停正式宿主命中区域失败".to_string());
    }
    Ok(())
}

/// 拖动期间更新 OLE 语义命中图，同时保持窗口裁剪暂停。
pub(crate) fn cache_host_hit_regions(monitor_id: &str, regions: Vec<HitRegion>) -> Result<(), String>
{
    HIT_REGIONS.get_or_init(|| RwLock::new(HashMap::new())).write().map_err(|error| error.to_string())?.insert(monitor_id.to_string(), regions);
    Ok(())
}

/// 判断语义区域是否提供正式宿主自身的可见像素证据。
fn is_pixel_probe_region(region: &HitRegion) -> bool { region.kind != "workspace-background" }

/// 捕获正式宿主窗口并确认命中区域内存在真实像素。
pub fn probe_host_pixels(host: &NativeDesktopHost) -> Result<usize, String>
{
    // 主显示器工作区像素尺寸。
    let width = host.monitor.work_width as i32;
    // 主显示器工作区物理像素高度。
    let height = host.monitor.work_height as i32;
    // 当前完整语义命中图。
    let hit_regions = HIT_REGIONS.get().ok_or_else(|| "正式宿主命中图尚未发布".to_string())?.read().map_err(|error| error.to_string())?.get(&host.monitor.id).cloned().ok_or_else(|| format!("显示器 {} 的正式宿主命中图尚未发布", host.monitor.id))?.into_iter().filter(is_pixel_probe_region).collect::<Vec<_>>();
    // 用于接收 PrintWindow 内容的内存设备上下文。
    let device_context = unsafe { CreateCompatibleDC(None) };
    if device_context.is_invalid()
    {
        return Err("创建像素探针设备上下文失败".to_string());
    }
    // 自顶向下 32 位 DIB 描述。
    let bitmap_info = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: width, biHeight: -height, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() }, ..Default::default() };
    // DIB 像素内存首地址。
    let mut bits = std::ptr::null_mut();
    // 承载正式宿主像素的 DIB。
    let bitmap = unsafe { CreateDIBSection(Some(device_context), &bitmap_info, DIB_RGB_COLORS, &mut bits, None, 0) }.map_err(|error| format!("创建像素探针 DIB 失败: {error}"))?;
    // 设备上下文之前选中的 GDI 对象。
    let previous_bitmap = unsafe { SelectObject(device_context, HGDIOBJ(bitmap.0)) };
    // PrintWindow 绘制结果。
    let rendered = unsafe { PrintWindow(raw_hwnd(host.hwnd), device_context, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)).as_bool() };
    // 捕获到的完整 BGRA 字节。
    let pixels = unsafe { std::slice::from_raw_parts(bits.cast::<u8>(), (width * height * 4).max(0) as usize) };
    // 命中区域内出现的非黑 RGB 像素数量。
    let visible_pixels = if rendered
    {
        // 全部命中区域累计的真实渲染像素数量。
        let mut count = 0;
        for region in &hit_regions
        {
            // 当前语义区域转换后的物理像素边界。
            let left = (region.left * host.monitor.scale_factor).floor() as i32;
            // 当前语义区域物理顶边。
            let top = (region.top * host.monitor.scale_factor).floor() as i32;
            // 当前语义区域物理右边。
            let right = ((region.left + region.width) * host.monitor.scale_factor).ceil() as i32;
            // 当前语义区域物理底边。
            let bottom = ((region.top + region.height) * host.monitor.scale_factor).ceil() as i32;
            for y in top.clamp(0, height)..bottom.clamp(0, height)
            {
                for x in left.clamp(0, width)..right.clamp(0, width)
                {
                    // 当前物理像素在 BGRA 缓冲区中的字节偏移。
                    let offset = ((y * width + x) * 4) as usize;
                    if pixels[offset..offset + 3].iter().any(|channel| *channel != 0)
                    {
                        count += 1;
                    }
                }
            }
        }
        count
    }
    else
    {
        0
    };
    unsafe {
        SelectObject(device_context, previous_bitmap);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(device_context);
    }
    (visible_pixels > 0).then_some(visible_pixels).ok_or_else(|| "正式宿主像素探针未检测到渲染内容".to_string())
}

/// 销毁全部正式 Windowed WebView2 与外层 HWND。
pub fn destroy_desktop_hosts() { destroy_matching_hosts(|_| true); }

/// 释放匹配宿主的 OLE、WebView 与 HWND，其他宿主不受影响。
fn destroy_matching_hosts(matches: impl Fn(&str) -> bool)
{
    NATIVE_HOSTS.with(|hosts| {
        let mut hosts = hosts.borrow_mut(); // 当前线程独占原生资源集合。
        let removed = (0..hosts.len()).rev().filter(|index| matches(&hosts[*index].monitor_id)).collect::<Vec<_>>(); // 逆序移除避免索引变化。
        for index in removed
        {
            let host = hosts.remove(index); // 仅释放本次选中的宿主。
            super::refresh_policy::set_paused(&host.monitor_id, false); // 销毁宿主时退出暂停集合，避免刷新策略永久判为暂停。
            super::directory_watch::clear_monitor(&host.monitor_id);
            crate::component_runtime::ComponentRuntimeState::clear_time_monitor(&host.monitor_id);
            crate::weather::activity::clear_monitor(&host.monitor_id);
            for (hwnd, _) in &host.drop_targets
            {
                let _ = unsafe { RevokeDragDrop(raw_hwnd(*hwnd)) };
            }
            drop(host.drop_targets);
            if host.ole_initialized
            {
                unsafe { OleUninitialize() };
            }
            // Controller 释放后销毁的正式宿主 HWND。
            let hwnd = host.hwnd;
            drop(host.webview);
            unsafe {
                let _ = DestroyWindow(raw_hwnd(hwnd));
            }
        }
    });
}

#[cfg(test)]
#[path = "desktop_host.test.rs"]
mod tests;
