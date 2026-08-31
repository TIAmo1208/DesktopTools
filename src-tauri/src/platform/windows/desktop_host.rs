use std::borrow::Cow;
use std::cell::RefCell;
use std::ffi::c_void;
use std::mem::size_of;
use std::num::NonZeroIsize;
use std::sync::{Arc, OnceLock, RwLock};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};
use tauri::AppHandle;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_BOUNDS_MODE_USE_RAW_PIXELS, ICoreWebView2Controller3,
};
use windows::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, POINT, RECT,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CombineRgn, CreateCompatibleDC, CreateDIBSection,
    CreateRectRgn, DIB_RGB_COLORS, DeleteDC, DeleteObject, EnumDisplayMonitors, GetMonitorInfoW,
    HDC, HGDIOBJ, HMONITOR, MONITORINFO, MONITORINFOEXW, RDW_ALLCHILDREN, RDW_ERASE, RDW_FRAME,
    RDW_INVALIDATE, RDW_UPDATENOW, RGN_OR, RedrawWindow, ScreenToClient, SelectObject,
    SetWindowRgn,
};
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, EnumWindows,
    FindWindowExW, FindWindowW, GWL_STYLE, GetParent, GetWindowLongPtrW, GetWindowThreadProcessId,
    HWND_TOP, IsWindow, IsWindowVisible, MONITORINFOF_PRIMARY, PW_RENDERFULLCONTENT,
    RegisterClassExW, SW_HIDE, SW_SHOW, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOOWNERZORDER, SWP_NOSIZE, SetParent, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    ShowWindowAsync, WINDOW_EX_STYLE, WNDCLASSEXW, WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS,
    WS_POPUP, WS_VISIBLE,
};
use windows::core::{BOOL, Interface, PCWSTR, w};
use wry::http::{Request, Response, StatusCode};
use wry::raw_window_handle::{
    HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle,
};
use wry::{PageLoadEvent, Rect as WryRect, WebView, WebViewBuilder, WebViewExtWindows, dpi};

use crate::contracts::{DesktopHostMonitor, HitRegion};

/// 独立页面向正式应用共享状态发送命令的桥接接口。
pub type NativeCommandHandler =
    Arc<dyn Fn(&str, Value) -> Result<Value, Value> + Send + Sync + 'static>;

/// 接管前发现的 Explorer 桌面层状态。
#[derive(Clone, Copy, Debug)]
pub struct ShellDesktopState {
    /// 当前 SHELLDLL_DefView 句柄。
    pub def_view: isize,
    /// 当前承载 SHELLDLL_DefView 的 Explorer 窗口。
    pub explorer_host: isize,
    /// 接管前 Explorer 桌面项目可见状态。
    pub original_items_visible: bool,
}

/// 单显示器正式宿主的原生记录。
#[derive(Clone, Debug)]
pub struct NativeDesktopHost {
    /// Rust 持有的外层 HWND。
    pub hwnd: isize,
    /// 当前主显示器物理工作区。
    pub monitor: DesktopHostMonitor,
}

/// UI 线程持有的 Windowed WebView2 与宿主 HWND。
struct NativeHostResources {
    /// Rust 持有的外层 HWND。
    hwnd: isize,
    /// Windowed WebView2 Controller。
    webview: WebView,
}

/// 为 Wry 暴露当前线程创建的 Win32 HWND。
struct NativeWindowHandle {
    /// 当前正式宿主原始句柄。
    hwnd: NonZeroIsize,
}

/// 将正式宿主句柄交给 Wry 创建 Windowed WebView2。
impl HasWindowHandle for NativeWindowHandle {
    /// 返回当前正式宿主的借用窗口句柄。
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        // Wry 接收的 Win32 原始窗口句柄。
        let handle = Win32WindowHandle::new(self.hwnd);
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(handle)) })
    }
}

/// 枚举 Explorer DefView 时使用的搜索状态。
struct DefViewSearch {
    /// Progman 句柄。
    progman: HWND,
    /// 搜索得到的 DefView。
    def_view: HWND,
    /// 承载 DefView 的窗口。
    explorer_host: HWND,
}

/// Windows 返回的主显示器物理状态。
struct PrimaryMonitor {
    /// 主显示器工作区。
    monitor: DesktopHostMonitor,
}

thread_local! {
    /// UI 线程持续持有的正式 Windowed WebView2 宿主。
    static NATIVE_HOST: RefCell<Option<NativeHostResources>> = const { RefCell::new(None) };
}

/// 最近一次前端发布的完整语义命中图。
static HIT_REGIONS: OnceLock<RwLock<Vec<HitRegion>>> = OnceLock::new();

/// 正式宿主窗口类名称。
const HOST_WINDOW_CLASS: windows::core::PCWSTR = w!("DesktopToolsFormalDesktopHost");

/// 独立宿主页面命令桥初始化脚本。
const NATIVE_BRIDGE_SCRIPT: &str = r#"
(() => {
  // 通过正式宿主本地协议调用 Rust 命令边界。
  const invoke = async (command, args = {}) => {
    // 当前命令对应的本地协议响应。
    const response = await fetch(`/__desktop_tools_invoke/${encodeURIComponent(command)}`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(args),
    });
    // 已解析的统一命令响应载荷。
    const payload = await response.json();
    if (!payload.ok) throw payload.error;
    return payload.result;
  };

  // 快照事件对应的只读轮询命令。
  const eventCommands = {
    "desktop-item-snapshot-changed": "get_current_desktop_item_snapshot",
    "desktop-host-snapshot-changed": "get_desktop_host_snapshot",
    "workspace-snapshot-changed": "get_current_workspace_snapshot",
  };

  // 通过只读快照轮询模拟正式宿主事件订阅。
  const listen = async (eventName, handler) => {
    // 当前事件对应的只读快照命令。
    const command = eventCommands[eventName];
    if (!command) return () => {};
    // 当前订阅是否仍需继续轮询。
    let active = true;
    // 上一次已发布的序列化快照。
    let previous = "";
    // 执行一轮快照读取与变化发布。
    const poll = async () => {
      if (!active) return;
      try {
        // 当前轮次返回的类型化快照载荷。
        const payload = await invoke(command);
        // 用于变化比较的稳定 JSON 文本。
        const serialized = JSON.stringify(payload);
        if (serialized !== previous) {
          previous = serialized;
          handler(payload);
        }
      } finally {
        if (active) window.setTimeout(poll, 250);
      }
    };
    window.setTimeout(poll, 0);
    return () => { active = false; };
  };

  window.__DESKTOP_TOOLS_NATIVE__ = { invoke, listen };
})();
"#;

/// 将保存的原始整数恢复为 HWND。
fn raw_hwnd(raw: isize) -> HWND {
    HWND(raw as *mut c_void)
}

/// 判断两个 HWND 是否属于同一个 Explorer 进程。
unsafe fn same_process(left: HWND, right: HWND) -> bool {
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
unsafe extern "system" fn find_def_view(window: HWND, parameter: LPARAM) -> BOOL {
    // 调用方传入的搜索状态。
    let search = unsafe { &mut *(parameter.0 as *mut DefViewSearch) };
    // 当前顶层窗口中的 DefView。
    let def_view = unsafe {
        FindWindowExW(Some(window), None, w!("SHELLDLL_DefView"), PCWSTR::null())
            .unwrap_or_default()
    };
    if !def_view.is_invalid() && unsafe { same_process(search.progman, window) } {
        search.def_view = def_view;
        search.explorer_host = window;
        return BOOL(0);
    }
    BOOL(1)
}

/// 发现当前 Explorer 桌面项目视图与宿主。
pub fn discover_shell_desktop() -> Result<ShellDesktopState, String> {
    // Windows 桌面 Progman 窗口。
    let progman = unsafe { FindWindowW(w!("Progman"), PCWSTR::null()).unwrap_or_default() };
    if progman.is_invalid() {
        return Err("未发现 Explorer Progman".to_string());
    }

    // Progman 直属的 DefView。
    let mut def_view = unsafe {
        FindWindowExW(Some(progman), None, w!("SHELLDLL_DefView"), PCWSTR::null())
            .unwrap_or_default()
    };
    // 当前 DefView 所属 Explorer 窗口。
    let mut explorer_host = if def_view.is_invalid() {
        HWND::default()
    } else {
        progman
    };
    if def_view.is_invalid() {
        // 顶层 WorkerW 搜索状态。
        let mut search = DefViewSearch {
            progman,
            def_view: HWND::default(),
            explorer_host: HWND::default(),
        };
        unsafe {
            let _ = EnumWindows(
                Some(find_def_view),
                LPARAM((&mut search as *mut DefViewSearch) as isize),
            );
        }
        def_view = search.def_view;
        explorer_host = search.explorer_host;
    }
    if def_view.is_invalid() || explorer_host.is_invalid() {
        return Err("未发现完整 Explorer 桌面层级".to_string());
    }

    Ok(ShellDesktopState {
        def_view: def_view.0 as isize,
        explorer_host: explorer_host.0 as isize,
        original_items_visible: desktop_items_visible(def_view.0 as isize),
    })
}

/// 设置 Explorer 桌面项目列表可见性并等待状态稳定。
pub fn set_desktop_items_visible(def_view_raw: isize, visible: bool) -> bool {
    // Explorer 桌面项目列表窗口。
    let list_view = unsafe {
        FindWindowExW(
            Some(raw_hwnd(def_view_raw)),
            None,
            w!("SysListView32"),
            PCWSTR::null(),
        )
        .unwrap_or_default()
    };
    if list_view.is_invalid() {
        return false;
    }

    unsafe {
        let _ = ShowWindowAsync(list_view, if visible { SW_SHOW } else { SW_HIDE });
    }
    for _ in 0..20 {
        if unsafe { IsWindowVisible(list_view).as_bool() } == visible {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    false
}

/// 返回 Explorer 桌面项目列表当前可见性。
pub fn desktop_items_visible(def_view_raw: isize) -> bool {
    // Explorer 桌面项目列表窗口。
    let list_view = unsafe {
        FindWindowExW(
            Some(raw_hwnd(def_view_raw)),
            None,
            w!("SysListView32"),
            PCWSTR::null(),
        )
        .unwrap_or_default()
    };
    !list_view.is_invalid() && unsafe { IsWindowVisible(list_view).as_bool() }
}

/// 请求 Explorer 重绘壁纸与桌面项目层。
pub fn redraw_shell_desktop(shell: ShellDesktopState) {
    for raw in [shell.explorer_host, shell.def_view] {
        if unsafe { IsWindow(Some(raw_hwnd(raw))).as_bool() } {
            unsafe {
                let _ = RedrawWindow(
                    Some(raw_hwnd(raw)),
                    None,
                    None,
                    RDW_INVALIDATE | RDW_ERASE | RDW_FRAME | RDW_UPDATENOW | RDW_ALLCHILDREN,
                );
            }
        }
    }
}

/// 枚举主显示器并返回物理工作区与有效 DPI。
unsafe extern "system" fn find_primary_monitor(
    monitor: HMONITOR,
    _device_context: HDC,
    _monitor_rect: *mut RECT,
    parameter: LPARAM,
) -> BOOL {
    // Windows 显示器信息。
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
    if !unsafe {
        GetMonitorInfoW(
            monitor,
            (&mut info as *mut MONITORINFOEXW).cast::<MONITORINFO>(),
        )
        .as_bool()
    } || info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY == 0
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
    // 调用方持有的主显示器结果。
    let result = unsafe { &mut *(parameter.0 as *mut Option<PrimaryMonitor>) };
    *result = Some(PrimaryMonitor {
        monitor: DesktopHostMonitor {
            work_x: info.monitorInfo.rcWork.left,
            work_y: info.monitorInfo.rcWork.top,
            work_width: (info.monitorInfo.rcWork.right - info.monitorInfo.rcWork.left).max(0)
                as u32,
            work_height: (info.monitorInfo.rcWork.bottom - info.monitorInfo.rcWork.top).max(0)
                as u32,
            scale_factor: dpi_x as f64 / 96.0,
            dpi: dpi_x,
        },
    });
    BOOL(0)
}

/// 返回当前主显示器物理工作区。
fn primary_monitor() -> Result<DesktopHostMonitor, String> {
    // Windows 主显示器枚举结果。
    let mut primary: Option<PrimaryMonitor> = None;
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(find_primary_monitor),
            LPARAM((&mut primary as *mut Option<PrimaryMonitor>) as isize),
        );
    }
    primary
        .map(|value| value.monitor)
        .ok_or_else(|| "Windows 未返回主显示器".to_string())
}

/// 正式宿主窗口过程。
unsafe extern "system" fn host_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

/// 注册正式宿主窗口类。
fn register_host_class() -> Result<(), String> {
    // 当前模块句柄。
    let module = unsafe { GetModuleHandleW(PCWSTR::null()) }
        .map_err(|error| format!("读取正式宿主模块句柄失败: {error}"))?;
    // 正式宿主窗口类描述。
    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(host_window_proc),
        hInstance: HINSTANCE(module.0),
        lpszClassName: HOST_WINDOW_CLASS,
        ..Default::default()
    };
    // 正式宿主窗口类注册结果。
    let atom = unsafe { RegisterClassExW(&class) };
    if atom == 0 && unsafe { GetLastError() } != ERROR_CLASS_ALREADY_EXISTS {
        return Err(format!(
            "注册正式宿主窗口类失败: {}",
            windows::core::Error::from_win32()
        ));
    }
    Ok(())
}

/// 将正式宿主附着到 Explorer 图标宿主并定位到工作区。
fn attach_host(
    hwnd: isize,
    shell: ShellDesktopState,
    monitor: &DesktopHostMonitor,
) -> Result<(), String> {
    // 工作区左上角在 Explorer 宿主中的客户区坐标。
    let mut top_left = POINT {
        x: monitor.work_x,
        y: monitor.work_y,
    };
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
        SetWindowPos(
            raw_hwnd(hwnd),
            Some(raw_hwnd(shell.def_view)),
            top_left.x,
            top_left.y,
            monitor.work_width as i32,
            monitor.work_height as i32,
            SWP_NOOWNERZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        )
        .map_err(|error| format!("定位正式宿主失败: {error}"))?;
    }
    if unsafe { GetParent(raw_hwnd(hwnd)).unwrap_or_default() } != raw_hwnd(shell.explorer_host) {
        return Err("正式宿主父级校验失败".to_string());
    }
    Ok(())
}

/// 为目标显示器固定 WebView2 raw-pixel Bounds 与栅格化比例。
fn configure_controller(
    controller: &webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Controller,
    monitor: &DesktopHostMonitor,
) -> Result<(), String> {
    // 支持显式 BoundsMode 与 RasterizationScale 的 Controller3。
    let controller3 = controller
        .cast::<ICoreWebView2Controller3>()
        .map_err(|error| format!("读取 WebView2 Controller3 失败: {error}"))?;
    unsafe {
        controller3
            .SetBoundsMode(COREWEBVIEW2_BOUNDS_MODE_USE_RAW_PIXELS)
            .map_err(|error| format!("设置 WebView2 raw-pixel Bounds 失败: {error}"))?;
        controller3
            .SetShouldDetectMonitorScaleChanges(false)
            .map_err(|error| format!("关闭 WebView2 自动显示器缩放失败: {error}"))?;
        controller3
            .SetRasterizationScale(monitor.scale_factor)
            .map_err(|error| format!("设置 WebView2 栅格化比例失败: {error}"))?;
    }
    Ok(())
}

/// 生成本地页面资产或命令响应。
fn protocol_response(
    status: StatusCode,
    content_type: &str,
    csp: Option<&str>,
    body: Cow<'static, [u8]>,
) -> Response<Cow<'static, [u8]>> {
    // 当前响应构建器。
    let mut builder = Response::builder()
        .status(status)
        .header("Content-Type", content_type);
    if let Some(csp) = csp {
        builder = builder.header("Content-Security-Policy", csp);
    }
    builder.body(body).expect("FATAL 正式宿主协议响应构造失败")
}

/// 路由 Tauri 内嵌资产和正式宿主命令。
fn handle_protocol_request(
    request: Request<Vec<u8>>,
    app: &AppHandle,
    command_handler: &NativeCommandHandler,
) -> Response<Cow<'static, [u8]>> {
    // 当前本地协议请求路径。
    let path = request.uri().path();
    if let Some(command) = path.strip_prefix("/__desktop_tools_invoke/") {
        // 前端提交的命令参数。
        let arguments = serde_json::from_slice(request.body()).unwrap_or_else(|_| json!({}));
        // 正式宿主命令处理结果。
        let payload = match command_handler(command, arguments) {
            Ok(result) => json!({ "ok": true, "result": result }),
            Err(error) => {
                crate::log_error!("宿主：本地命令执行失败，命令={command}，错误={error}");
                json!({ "ok": false, "error": error })
            }
        };
        return protocol_response(
            StatusCode::OK,
            "application/json; charset=utf-8",
            None,
            Cow::Owned(serde_json::to_vec(&payload).expect("FATAL 正式宿主命令响应序列化失败")),
        );
    }

    // Tauri 构建时嵌入的 React 资产路径。
    let asset_path = path.trim_start_matches('/');
    // 空路径映射到 React 入口页面。
    let asset_path = if asset_path.is_empty() {
        "index.html"
    } else {
        asset_path
    };
    if let Some(asset) = app.asset_resolver().get(asset_path.to_string()) {
        // 资产元数据在移动字节缓冲区前转为独立值。
        let content_type = asset.mime_type().to_string();
        // 当前资产附加的内容安全策略。
        let csp = asset.csp_header().map(str::to_string);
        return protocol_response(
            StatusCode::OK,
            &content_type,
            csp.as_deref(),
            Cow::Owned(asset.bytes),
        );
    }

    protocol_response(
        StatusCode::NOT_FOUND,
        "text/plain; charset=utf-8",
        None,
        Cow::Borrowed(b"not found"),
    )
}

/// 创建主显示器对应的正式透明 Windowed WebView2 宿主。
pub fn create_primary_desktop_host(
    app: &AppHandle,
    shell: ShellDesktopState,
    command_handler: NativeCommandHandler,
) -> Result<NativeDesktopHost, String> {
    // 当前主显示器物理工作区。
    let monitor = primary_monitor()?;
    register_host_class()?;
    // 当前模块句柄。
    let module = unsafe { GetModuleHandleW(PCWSTR::null()) }
        .map_err(|error| format!("读取正式宿主模块句柄失败: {error}"))?;
    // 正式宿主外层 HWND。
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            HOST_WINDOW_CLASS,
            w!("Desktop Tools"),
            WS_POPUP | WS_CLIPCHILDREN | WS_CLIPSIBLINGS | WS_VISIBLE,
            monitor.work_x,
            monitor.work_y,
            monitor.work_width as i32,
            monitor.work_height as i32,
            None,
            None,
            Some(HINSTANCE(module.0)),
            None,
        )
    }
    .map_err(|error| format!("创建正式宿主 HWND 失败: {error}"))?;
    // Wry 使用的正式宿主句柄包装。
    let raw_window = NativeWindowHandle {
        hwnd: NonZeroIsize::new(hwnd.0 as isize).ok_or("正式宿主 HWND 为空")?,
    };
    // 本地协议使用的 AppHandle。
    let protocol_app = app.clone();
    // 本地协议使用的命令处理器。
    let protocol_handler = Arc::clone(&command_handler);
    // 正式 Windowed WebView2。
    let webview = WebViewBuilder::new()
        .with_id("desktop-host-primary")
        .with_transparent(true)
        .with_visible(true)
        .with_bounds(WryRect {
            position: dpi::PhysicalPosition::new(0, 0).into(),
            size: dpi::PhysicalSize::new(monitor.work_width, monitor.work_height).into(),
        })
        .with_initialization_script(NATIVE_BRIDGE_SCRIPT)
        .with_custom_protocol("desktop-tools-host".to_string(), move |_id, request| {
            handle_protocol_request(request, &protocol_app, &protocol_handler)
        })
        .with_on_page_load_handler(|event, url| {
            // 当前 React 页面加载阶段的中文名称。
            let stage = match event {
                PageLoadEvent::Started => "开始",
                PageLoadEvent::Finished => "完成",
            };
            crate::log_info!("宿主：React 页面加载{}，URL：{}", stage, url);
        })
        .with_url("about:blank")
        .build_as_child(&raw_window)
        .map_err(|error| format!("创建正式 Windowed WebView2 失败: {error}"))?;

    // 正式 WebView2 的原生 Controller。
    let controller = webview.controller();
    attach_host(hwnd.0 as isize, shell, &monitor)?;
    configure_controller(&controller, &monitor)?;
    unsafe {
        controller
            .NotifyParentWindowPositionChanged()
            .map_err(|error| format!("通知 WebView2 宿主位置变化失败: {error}"))?;
    }
    crate::log_info!(
        "宿主：主显示器透明宿主已创建，工作区：[{}, {}, {}, {}]，DPI：{}，栅格化比例：{}，BoundsMode：USE_RAW_PIXELS",
        monitor.work_x,
        monitor.work_y,
        monitor.work_width,
        monitor.work_height,
        monitor.dpi,
        monitor.scale_factor
    );

    NATIVE_HOST.with(|host| {
        *host.borrow_mut() = Some(NativeHostResources {
            hwnd: hwnd.0 as isize,
            webview,
        });
    });
    Ok(NativeDesktopHost {
        hwnd: hwnd.0 as isize,
        monitor,
    })
}

/// 导航正式宿主到 Tauri 内嵌 React 页面。
pub fn load_primary_desktop_host() -> Result<(), String> {
    NATIVE_HOST.with(|host| {
        host.borrow()
            .as_ref()
            .ok_or_else(|| "正式宿主尚未创建".to_string())?
            .webview
            .load_url("http://desktop-tools-host.localhost/index.html?desktop-host=1")
            .map_err(|error| format!("加载正式宿主 React 页面失败: {error}"))
    })
}

/// 显示或撤下正式宿主 HWND。
pub fn set_host_visible(hwnd: isize, visible: bool) -> bool {
    unsafe {
        let _ = ShowWindow(raw_hwnd(hwnd), if visible { SW_SHOW } else { SW_HIDE });
    }
    for _ in 0..20 {
        if unsafe { IsWindowVisible(raw_hwnd(hwnd)).as_bool() } == visible {
            return true;
        }
        thread::sleep(Duration::from_millis(25));
    }
    false
}

/// 原子替换并应用正式宿主的完整语义命中图。
pub fn apply_host_hit_regions(
    host: &NativeDesktopHost,
    regions: Vec<HitRegion>,
) -> Result<usize, String> {
    // 合并全部语义矩形的最终窗口区域。
    let combined_region = unsafe { CreateRectRgn(0, 0, 0, 0) };
    if combined_region.is_invalid() {
        return Err("创建正式宿主命中区域失败".to_string());
    }

    for region in &regions {
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
        if item_region.is_invalid() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(combined_region.0));
            }
            return Err("创建语义对象命中区域失败".to_string());
        }
        unsafe {
            let _ = CombineRgn(
                Some(combined_region),
                Some(combined_region),
                Some(item_region),
                RGN_OR,
            );
            let _ = DeleteObject(HGDIOBJ(item_region.0));
        }
    }
    if unsafe { SetWindowRgn(raw_hwnd(host.hwnd), Some(combined_region), true) } == 0 {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(combined_region.0));
        }
        return Err("应用正式宿主命中区域失败".to_string());
    }
    unsafe {
        SetWindowPos(
            raw_hwnd(host.hwnd),
            Some(HWND_TOP),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
        .map_err(|error| format!("提升正式宿主输入层级失败: {error}"))?;
    }
    *HIT_REGIONS
        .get_or_init(|| RwLock::new(Vec::new()))
        .write()
        .map_err(|error| error.to_string())? = regions;
    Ok(HIT_REGIONS
        .get()
        .expect("命中图已初始化")
        .read()
        .map_err(|error| error.to_string())?
        .len())
}

/// 在连续拖动期间临时恢复正式宿主完整窗口区域。
pub fn suspend_host_hit_regions(host: &NativeDesktopHost) -> Result<(), String> {
    if unsafe { SetWindowRgn(raw_hwnd(host.hwnd), None, true) } == 0 {
        return Err("暂停正式宿主命中区域失败".to_string());
    }
    Ok(())
}

/// 捕获正式宿主窗口并确认命中区域内存在真实像素。
pub fn probe_host_pixels(host: &NativeDesktopHost) -> Result<usize, String> {
    // 主显示器工作区像素尺寸。
    let width = host.monitor.work_width as i32;
    // 主显示器工作区物理像素高度。
    let height = host.monitor.work_height as i32;
    // 当前完整语义命中图。
    let hit_regions = HIT_REGIONS
        .get()
        .ok_or_else(|| "正式宿主命中图尚未发布".to_string())?
        .read()
        .map_err(|error| error.to_string())?
        .clone();
    // 用于接收 PrintWindow 内容的内存设备上下文。
    let device_context = unsafe { CreateCompatibleDC(None) };
    if device_context.is_invalid() {
        return Err("创建像素探针设备上下文失败".to_string());
    }
    // 自顶向下 32 位 DIB 描述。
    let bitmap_info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    // DIB 像素内存首地址。
    let mut bits = std::ptr::null_mut();
    // 承载正式宿主像素的 DIB。
    let bitmap = unsafe {
        CreateDIBSection(
            Some(device_context),
            &bitmap_info,
            DIB_RGB_COLORS,
            &mut bits,
            None,
            0,
        )
    }
    .map_err(|error| format!("创建像素探针 DIB 失败: {error}"))?;
    // 设备上下文之前选中的 GDI 对象。
    let previous_bitmap = unsafe { SelectObject(device_context, HGDIOBJ(bitmap.0)) };
    // PrintWindow 绘制结果。
    let rendered = unsafe {
        PrintWindow(
            raw_hwnd(host.hwnd),
            device_context,
            PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT),
        )
        .as_bool()
    };
    // 捕获到的完整 BGRA 字节。
    let pixels = unsafe {
        std::slice::from_raw_parts(bits.cast::<u8>(), (width * height * 4).max(0) as usize)
    };
    // 命中区域内出现的非黑 RGB 像素数量。
    let visible_pixels = if rendered {
        // 全部命中区域累计的真实渲染像素数量。
        let mut count = 0;
        for region in &hit_regions {
            // 当前语义区域转换后的物理像素边界。
            let left = (region.left * host.monitor.scale_factor).floor() as i32;
            // 当前语义区域物理顶边。
            let top = (region.top * host.monitor.scale_factor).floor() as i32;
            // 当前语义区域物理右边。
            let right = ((region.left + region.width) * host.monitor.scale_factor).ceil() as i32;
            // 当前语义区域物理底边。
            let bottom = ((region.top + region.height) * host.monitor.scale_factor).ceil() as i32;
            for y in top.clamp(0, height)..bottom.clamp(0, height) {
                for x in left.clamp(0, width)..right.clamp(0, width) {
                    // 当前物理像素在 BGRA 缓冲区中的字节偏移。
                    let offset = ((y * width + x) * 4) as usize;
                    if pixels[offset..offset + 3]
                        .iter()
                        .any(|channel| *channel != 0)
                    {
                        count += 1;
                    }
                }
            }
        }
        count
    } else {
        0
    };
    unsafe {
        SelectObject(device_context, previous_bitmap);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(device_context);
    }
    (visible_pixels > 0)
        .then_some(visible_pixels)
        .ok_or_else(|| "正式宿主像素探针未检测到渲染内容".to_string())
}

/// 销毁正式 Windowed WebView2 与外层 HWND。
pub fn destroy_primary_desktop_host() {
    NATIVE_HOST.with(|host| {
        if let Some(host) = host.borrow_mut().take() {
            // Controller 释放后销毁的正式宿主 HWND。
            let hwnd = host.hwnd;
            drop(host.webview);
            unsafe {
                let _ = DestroyWindow(raw_hwnd(hwnd));
            }
        }
    });
}
