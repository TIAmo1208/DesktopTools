use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
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
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, ClientToScreen, CombineRgn, CreateCompatibleDC,
    CreateDIBSection, CreateRectRgn, DIB_RGB_COLORS, DeleteDC, DeleteObject, EnumDisplayMonitors,
    GetMonitorInfoW, HDC, HGDIOBJ, HMONITOR, MONITORINFO, MONITORINFOEXW, RDW_ALLCHILDREN,
    RDW_ERASE, RDW_FRAME, RDW_INVALIDATE, RDW_UPDATENOW, RGN_OR, RedrawWindow, ScreenToClient,
    SelectObject, SetWindowRgn,
};
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
use windows::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    IServiceProvider,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::{
    IDropTarget, OleInitialize, OleUninitialize, RegisterDragDrop, RevokeDragDrop,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Shell::{
    IFolderView, IShellBrowser, IShellFolder, IShellItem, IShellWindows, SHCreateItemWithParent,
    SHELLFLAGSTATE, SHGetDesktopFolder, SHGetSettings, SID_STopLevelBrowser,
    SIGDN_DESKTOPABSOLUTEPARSING, SSF_HIDEICONS, SVGIO_ALLVIEW, SWC_DESKTOP, SWFO_NEEDDISPATCH,
    ShellWindows,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, EnumChildWindows,
    EnumWindows, FindWindowExW, FindWindowW, GWL_STYLE, GetParent, GetWindowLongPtrW,
    GetWindowThreadProcessId, HWND_TOP, IsWindow, IsWindowVisible, MONITORINFOF_PRIMARY,
    PW_RENDERFULLCONTENT, RegisterClassExW, SW_HIDE, SW_SHOW, SWP_FRAMECHANGED, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SetParent, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, ShowWindowAsync, WINDOW_EX_STYLE, WNDCLASSEXW, WS_CHILD, WS_CLIPCHILDREN,
    WS_CLIPSIBLINGS, WS_POPUP, WS_VISIBLE,
};
use windows::core::{BOOL, Interface, PCWSTR, w};
use wry::http::{Request, Response, StatusCode};
use wry::raw_window_handle::{
    HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle,
};
use wry::{PageLoadEvent, Rect as WryRect, WebView, WebViewBuilder, WebViewExtWindows, dpi};

use crate::contracts::{DesktopHostMonitor, DesktopItem, HitRegion};

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
    /// 当前 Explorer 桌面 SysListView32 句柄。
    pub list_view: isize,
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

thread_local! {
    /// UI 线程持续持有的全部正式 Windowed WebView2 宿主。
    static NATIVE_HOSTS: RefCell<Vec<NativeHostResources>> = const { RefCell::new(Vec::new()) };
}

/// 最近一次前端发布的完整语义命中图。
static HIT_REGIONS: OnceLock<RwLock<HashMap<String, Vec<HitRegion>>>> = OnceLock::new();

/// 正式宿主窗口类名称。
const HOST_WINDOW_CLASS: windows::core::PCWSTR = w!("DesktopToolsFormalDesktopHost");

/// `SHELLFLAGSTATE` 中 `fHideIcons` 对应的位。
const SHELL_HIDE_ICONS_BIT: i32 = 1 << 12;

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
    "preferences-changed": "get_preferences",
    "windows-accent-color-changed": "get_windows_accent_color",
    "desktop-item-snapshot-changed": "get_current_desktop_item_snapshot",
    "desktop-host-snapshot-changed": "get_desktop_host_snapshot",
    "workspace-snapshot-changed": "get_current_workspace_snapshot",
    "component-runtime-snapshot-changed": "get_current_component_runtime_snapshot",
    "component-host-event": "get_current_component_host_event",
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
        if (payload == null) return;
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

/// 为正式宿主和 WebView2 子窗口注册 Shell 桌面 IDropTarget。
fn register_shell_drop_targets(hwnd: HWND) -> Result<(Vec<(isize, IDropTarget)>, bool), String> {
    unsafe { OleInitialize(None) }.map_err(|error| format!("初始化宿主 OLE 拖放失败: {error}"))?;
    let mut windows = vec![hwnd]; // 等待替换为 Shell 桌面投放目标的窗口集合。
    unsafe extern "system" fn collect_child(window: HWND, parameter: LPARAM) -> BOOL {
        let windows = unsafe { &mut *(parameter.0 as *mut Vec<HWND>) }; // 调用方持有的窗口集合。
        windows.push(window);
        BOOL(1)
    }
    let _ = unsafe {
        EnumChildWindows(
            Some(hwnd),
            Some(collect_child),
            LPARAM((&mut windows as *mut Vec<HWND>) as isize),
        )
    };
    let desktop = match unsafe { SHGetDesktopFolder() } {
        Ok(desktop) => desktop,
        Err(error) => {
            unsafe { OleUninitialize() };
            return Err(format!("创建宿主 Shell 投放目标失败: {error}"));
        }
    }; // 提供桌面命名空间 IDropTarget 的 Shell folder。
    let mut targets = Vec::new(); // 必须保持到宿主销毁的已注册 COM 投放目标。
    for target_window in windows {
        let target: IDropTarget = match unsafe { desktop.CreateViewObject(target_window) } {
            Ok(target) => target,
            Err(error) => {
                crate::log_warn!(
                    "Shell：跳过不支持桌面投放的宿主窗口，窗口句柄={:#x}，错误：{}",
                    target_window.0 as isize,
                    error
                );
                continue;
            }
        }; // 当前窗口对应的系统桌面投放目标。
        let _ = unsafe { RevokeDragDrop(target_window) };
        match unsafe { RegisterDragDrop(target_window, &target) } {
            Ok(()) => targets.push((target_window.0 as isize, target)),
            Err(error) => crate::log_warn!(
                "Shell：宿主窗口投放注册失败，窗口句柄={:#x}，错误：{}",
                target_window.0 as isize,
                error
            ),
        }
    }
    if targets.is_empty() {
        unsafe { OleUninitialize() };
        return Err("正式宿主没有可用的 Shell 桌面投放目标".to_string());
    }
    crate::log_info!(
        "Shell：宿主 OLE 投放目标已注册，窗口数量：{}",
        targets.len()
    );
    Ok((targets, true))
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

    let list_view = unsafe {
        FindWindowExW(Some(def_view), None, w!("SysListView32"), PCWSTR::null()).unwrap_or_default()
    }; // Explorer 桌面项目原始坐标所属的列表视图。
    if list_view.is_invalid() {
        return Err("未发现 Explorer 桌面项目列表".to_string());
    }

    Ok(ShellDesktopState {
        def_view: def_view.0 as isize,
        explorer_host: explorer_host.0 as isize,
        list_view: list_view.0 as isize,
        original_items_visible: desktop_items_setting_visible(),
    })
}

/// 读取 Explorer“显示桌面图标”用户设置，不受 Desktop Tools 临时隐藏窗口影响。
pub fn desktop_items_setting_visible() -> bool {
    let mut state = SHELLFLAGSTATE::default(); // Shell 返回的桌面图标设置位。
    unsafe {
        SHGetSettings(&mut state, SSF_HIDEICONS.0);
    }
    shell_flag_state_icons_visible(state._bitfield)
}

/// 将 Shell `fHideIcons` 位转换为用户可见性语义。
fn shell_flag_state_icons_visible(bitfield: i32) -> bool {
    bitfield & SHELL_HIDE_ICONS_BIT == 0
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

/// 枚举一台显示器并保存物理边界、工作区与有效 DPI。
unsafe extern "system" fn collect_monitor(
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
    } {
        return BOOL(1);
    }

    // 主显示器有效 DPI。
    let mut dpi_x = 96;
    // 主显示器垂直 DPI。
    let mut dpi_y = 96;
    unsafe {
        let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
    }
    let name_length = info
        .szDevice
        .iter()
        .position(|character| *character == 0)
        .unwrap_or(info.szDevice.len()); // Windows 显示设备名称的 UTF-16 有效长度。
    let name = String::from_utf16_lossy(&info.szDevice[..name_length]); // 显示器稳定设备名称。
    // 调用方持有的显示器结果集合。
    let result = unsafe { &mut *(parameter.0 as *mut Vec<DesktopHostMonitor>) };
    result.push(DesktopHostMonitor {
        id: name.clone(),
        name,
        primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        x: info.monitorInfo.rcMonitor.left,
        y: info.monitorInfo.rcMonitor.top,
        width: (info.monitorInfo.rcMonitor.right - info.monitorInfo.rcMonitor.left).max(0) as u32,
        height: (info.monitorInfo.rcMonitor.bottom - info.monitorInfo.rcMonitor.top).max(0) as u32,
        work_x: info.monitorInfo.rcWork.left,
        work_y: info.monitorInfo.rcWork.top,
        work_width: (info.monitorInfo.rcWork.right - info.monitorInfo.rcWork.left).max(0) as u32,
        work_height: (info.monitorInfo.rcWork.bottom - info.monitorInfo.rcWork.top).max(0) as u32,
        scale_factor: dpi_x as f64 / 96.0,
        dpi: dpi_x,
    });
    BOOL(1)
}

/// 返回当前全部显示器身份、物理边界、工作区与有效 DPI。
pub fn enumerate_desktop_monitors() -> Result<Vec<DesktopHostMonitor>, String> {
    let mut monitors: Vec<DesktopHostMonitor> = Vec::new(); // Windows 显示器枚举结果。
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect_monitor),
            LPARAM((&mut monitors as *mut Vec<DesktopHostMonitor>) as isize),
        );
    }
    if monitors.is_empty() {
        return Err("Windows 未返回显示器".to_string());
    }
    monitors.sort_by_key(|monitor| (!monitor.primary, monitor.x, monitor.y));
    Ok(monitors)
}

/// 返回包含指定虚拟桌面物理坐标的显示器稳定身份。
fn monitor_id_at_point(monitors: &[DesktopHostMonitor], point: POINT) -> Option<&str> {
    monitors
        .iter()
        .find(|monitor| {
            let right = i64::from(monitor.x) + i64::from(monitor.width);
            let bottom = i64::from(monitor.y) + i64::from(monitor.height);
            i64::from(point.x) >= i64::from(monitor.x)
                && i64::from(point.x) < right
                && i64::from(point.y) >= i64::from(monitor.y)
                && i64::from(point.y) < bottom
        })
        .map(|monitor| monitor.id.as_str())
}

/// 返回 Explorer 已附着的桌面视图，调用线程必须已初始化 COM。
pub(super) fn explorer_desktop_view() -> Result<windows::Win32::UI::Shell::IShellView, String> {
    let shell_windows: IShellWindows = unsafe {
        CoCreateInstance(&ShellWindows, None, CLSCTX_LOCAL_SERVER)
            .map_err(|error| format!("创建 IShellWindows 失败: {error}"))?
    }; // Explorer Shell 窗口集合。
    let location = VARIANT::default(); // FindWindowSW 使用的空位置参数。
    let mut desktop_hwnd = 0; // Explorer 桌面自动化窗口句柄。
    let desktop_dispatch = unsafe {
        shell_windows
            .FindWindowSW(
                &location,
                &location,
                SWC_DESKTOP,
                &mut desktop_hwnd,
                SWFO_NEEDDISPATCH,
            )
            .map_err(|error| format!("查找 Explorer 桌面窗口失败: {error}"))?
    }; // Explorer 桌面窗口自动化接口。
    let service_provider = desktop_dispatch
        .cast::<IServiceProvider>()
        .map_err(|error| format!("获取 IServiceProvider 失败: {error}"))?; // 桌面窗口服务查询接口。
    let shell_browser: IShellBrowser = unsafe {
        service_provider
            .QueryService(&SID_STopLevelBrowser)
            .map_err(|error| format!("获取 IShellBrowser 失败: {error}"))?
    }; // 桌面顶层浏览器接口。
    unsafe { shell_browser.QueryActiveShellView() }
        .map_err(|error| format!("获取 Explorer 桌面视图失败: {error}"))
}

/// 按原型公开 Shell 路径读取 Explorer 桌面项目解析名称和物理屏幕坐标。
fn explorer_desktop_item_positions(list_view_raw: isize) -> Result<HashMap<String, POINT>, String> {
    if list_view_raw == 0 {
        return Err("Explorer SysListView32 句柄为空".to_string());
    }
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let folder_view = explorer_desktop_view()?
        .cast::<IFolderView>()
        .map_err(|error| format!("获取 IFolderView 失败: {error}"))?; // 当前 Explorer 桌面文件夹视图。
    let folder: IShellFolder = unsafe {
        folder_view
            .GetFolder()
            .map_err(|error| format!("获取 IShellFolder 失败: {error}"))?
    }; // 当前桌面文件夹接口。
    let item_count = unsafe {
        folder_view
            .ItemCount(SVGIO_ALLVIEW)
            .map_err(|error| format!("读取 Explorer 桌面项目数量失败: {error}"))?
    }; // Explorer 当前显示的桌面项目数量。
    let mut positions = HashMap::new(); // 解析名称到物理坐标的映射。
    for item_index in 0..item_count {
        let pidl = unsafe {
            folder_view
                .Item(item_index)
                .map_err(|error| format!("读取 Explorer 桌面项目 PIDL 失败: {error}"))?
        }; // Explorer 分配的桌面子项目 PIDL。
        let item_result = unsafe {
            (|| -> windows::core::Result<(String, POINT)> {
                let shell_item: IShellItem = SHCreateItemWithParent(None, &folder, pidl)?; // 当前子项目的 ShellItem。
                let name_pointer = shell_item.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING)?; // Shell 分配的绝对解析名称。
                let parsing_name = name_pointer.to_string(); // UTF-16 解析名称转换结果。
                CoTaskMemFree(Some(name_pointer.0.cast()));
                let mut point = folder_view.GetItemPosition(pidl)?; // Explorer 视图中的项目坐标。
                if !ClientToScreen(raw_hwnd(list_view_raw), &mut point).as_bool() {
                    return Err(windows::core::Error::from_win32());
                }
                Ok((parsing_name?, point))
            })()
        };
        unsafe { CoTaskMemFree(Some(pidl.cast())) };
        if let Ok((parsing_name, point)) = item_result {
            positions.insert(parsing_name.to_lowercase(), point);
        }
    }
    Ok(positions)
}

/// 按 Explorer 原始图标坐标返回首次导入使用的项目显示器归属。
pub fn desktop_item_monitor_assignments(
    items: &[DesktopItem],
    monitors: &[DesktopHostMonitor],
    shell: ShellDesktopState,
) -> Vec<(String, String)> {
    let positions = match explorer_desktop_item_positions(shell.list_view) {
        Ok(positions) => positions,
        Err(error) => {
            crate::log_warn!(
                "工作区：读取 Explorer 项目显示器归属失败，项目={}，显示器={}，错误={error}",
                items.len(),
                monitors.len()
            );
            return Vec::new();
        }
    }; // Explorer 当前项目物理坐标。
    let assignments = match_desktop_item_monitor_assignments(items, monitors, &positions); // Explorer 坐标与当前显示器边界的匹配结果。
    crate::log_debug!(
        "工作区：Explorer 项目显示器归属已读取，项目={}，匹配={}，显示器={}",
        items.len(),
        assignments.len(),
        monitors.len()
    );
    assignments
}

/// 将 Explorer 解析名称坐标映射为 Workspace 使用的稳定显示器归属。
fn match_desktop_item_monitor_assignments(
    items: &[DesktopItem],
    monitors: &[DesktopHostMonitor],
    positions: &HashMap<String, POINT>,
) -> Vec<(String, String)> {
    items
        .iter()
        .filter_map(|item| {
            let point = positions.get(&item.stable_reference.to_lowercase())?; // 当前项目的 Explorer 物理坐标。
            let monitor_id = monitor_id_at_point(monitors, *point)?; // 当前物理坐标所属显示器。
            Some((item.stable_reference.clone(), monitor_id.to_string()))
        })
        .collect()
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

/// 创建一台显示器对应的正式透明 Windowed WebView2 宿主。
fn create_desktop_host(
    app: &AppHandle,
    shell: ShellDesktopState,
    command_handler: NativeCommandHandler,
    monitor: &DesktopHostMonitor,
    index: usize,
) -> Result<NativeDesktopHost, String> {
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
    let label = format!("desktop-host-{index}"); // Wry 内部使用的逐屏宿主标识。
    let protocol = format!("desktop-tools-host-{index}"); // 避免多 WebView 协议处理器冲突的逐屏 scheme。
    let page_url = format!("http://{protocol}.localhost/index.html?desktop-host=1&monitor={index}"); // 当前宿主页面携带的显示器枚举索引。
    // 正式 Windowed WebView2。
    let webview = WebViewBuilder::new()
        .with_id(&label)
        .with_transparent(true)
        .with_visible(true)
        .with_bounds(WryRect {
            position: dpi::PhysicalPosition::new(0, 0).into(),
            size: dpi::PhysicalSize::new(monitor.work_width, monitor.work_height).into(),
        })
        .with_initialization_script(NATIVE_BRIDGE_SCRIPT)
        .with_custom_protocol(protocol, move |_id, request| {
            handle_protocol_request(request, &protocol_app, &protocol_handler)
        })
        .with_on_page_load_handler(|event, url| {
            // 当前 React 页面加载阶段的中文名称。
            let stage = match event {
                PageLoadEvent::Started => "开始",
                PageLoadEvent::Finished => "完成",
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
        controller
            .NotifyParentWindowPositionChanged()
            .map_err(|error| format!("通知 WebView2 宿主位置变化失败: {error}"))?;
    }
    let (drop_targets, ole_initialized) = register_shell_drop_targets(hwnd)?; // Desktop Tools 接收外部 Shell IDataObject 的窗口注册。
    crate::log_debug!(
        "宿主：逐屏透明宿主已创建，显示器={}，主显示器={}，工作区：[{}, {}, {}, {}]，DPI：{}，栅格化比例：{}，BoundsMode：USE_RAW_PIXELS",
        monitor.id,
        u8::from(monitor.primary),
        monitor.work_x,
        monitor.work_y,
        monitor.work_width,
        monitor.work_height,
        monitor.dpi,
        monitor.scale_factor
    );

    NATIVE_HOSTS.with(|hosts| {
        hosts.borrow_mut().push(NativeHostResources {
            monitor_id: monitor.id.clone(),
            hwnd: hwnd.0 as isize,
            webview,
            page_url,
            drop_targets,
            ole_initialized,
        });
    });
    Ok(NativeDesktopHost {
        hwnd: hwnd.0 as isize,
        monitor: monitor.clone(),
    })
}

/// 为每台显示器创建独立正式宿主。
pub fn create_desktop_hosts(
    app: &AppHandle,
    shell: ShellDesktopState,
    command_handler: NativeCommandHandler,
    monitors: &[DesktopHostMonitor],
) -> Result<Vec<NativeDesktopHost>, String> {
    if monitors.is_empty() {
        return Err("正式宿主缺少显示器".to_string());
    }
    register_host_class()?;
    let mut hosts = Vec::with_capacity(monitors.len()); // 已完整创建的逐屏原生宿主记录。
    for (index, monitor) in monitors.iter().enumerate() {
        match create_desktop_host(app, shell, Arc::clone(&command_handler), monitor, index) {
            Ok(host) => hosts.push(host),
            Err(error) => {
                destroy_desktop_hosts();
                return Err(error);
            }
        }
    }
    Ok(hosts)
}

/// 保留未变化且页面索引一致的宿主，只创建并导航发生变化的显示器。
pub fn reconcile_desktop_hosts(
    app: &AppHandle,
    shell: ShellDesktopState,
    command_handler: NativeCommandHandler,
    monitors: &[DesktopHostMonitor],
    previous: &[NativeDesktopHost],
) -> Result<Vec<NativeDesktopHost>, String> {
    let retained = previous
        .iter()
        .enumerate()
        .filter(|(index, host)| monitors.get(*index) == Some(&host.monitor))
        .map(|(_, host)| host.clone())
        .collect::<Vec<_>>(); // 索引变化会改变页面身份，不能复用。
    destroy_matching_hosts(|id| !retained.iter().any(|host| host.monitor.id == id));
    let mut result = Vec::with_capacity(monitors.len()); // 最新顺序的完整宿主集合。
    for (index, monitor) in monitors.iter().enumerate() {
        if let Some(host) = retained.iter().find(|host| host.monitor.id == monitor.id) {
            result.push(host.clone());
            continue;
        }
        let host = create_desktop_host(app, shell, Arc::clone(&command_handler), monitor, index)?; // 仅新建受影响的宿主。
        NATIVE_HOSTS.with(|hosts| {
            let hosts = hosts.borrow(); // 同线程持有的新 WebView。
            let resource = hosts
                .iter()
                .find(|resource| resource.hwnd == host.hwnd)
                .ok_or("新宿主资源不存在")?; // 待导航资源。
            resource
                .webview
                .load_url(&resource.page_url)
                .map_err(|error| error.to_string())
        })?;
        result.push(host);
    }
    Ok(result)
}

/// 导航全部正式宿主到 Tauri 内嵌 React 页面。
pub fn load_desktop_hosts() -> Result<(), String> {
    NATIVE_HOSTS.with(|hosts| {
        for host in hosts.borrow().iter() {
            host.webview.load_url(&host.page_url).map_err(|error| {
                format!(
                    "加载正式宿主 React 页面失败，显示器={}，错误={error}",
                    host.monitor_id
                )
            })?;
        }
        Ok(())
    })
}

/// 按原型轻量重载路径刷新一个 Renderer，并保留外层 HWND 与命中区域。
pub fn reload_desktop_host(monitor_id: &str) -> Result<(), String> {
    NATIVE_HOSTS.with(|hosts| {
        let hosts = hosts.borrow(); // 当前 UI 线程持有的全部正式 WebView2 宿主。
        let host = hosts
            .iter()
            .find(|host| host.monitor_id == monitor_id)
            .ok_or_else(|| format!("Renderer 重载目标不存在，显示器={monitor_id}"))?;
        if host
            .webview
            .evaluate_script("window.location.reload()")
            .is_ok()
        {
            return Ok(());
        }
        host.webview
            .load_url(&host.page_url)
            .map_err(|error| format!("Renderer 轻量重载失败，显示器={monitor_id}，错误={error}"))
    })
}

/// 在宿主 UI 线程转发有界拖动消息；内容只作为 CustomEvent 数据，不插入 HTML。
pub fn relay_workspace_drag(payload: serde_json::Value) -> Result<(), String> {
    let encoded = serde_json::to_string(&payload).map_err(|error| error.to_string())?; // 安全编码的事件数据。
    if encoded.len() > 65536 {
        return Err("跨屏拖动消息过大".into());
    }
    let script =
        format!("window.dispatchEvent(new CustomEvent('desktop-tools-drag',{{detail:{encoded}}}))"); // 仅分发给本应用宿主。
    NATIVE_HOSTS.with(|hosts| {
        for host in hosts.borrow().iter() {
            host.webview
                .evaluate_script(&script)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
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
    HIT_REGIONS
        .get_or_init(|| RwLock::new(HashMap::new()))
        .write()
        .map_err(|error| error.to_string())?
        .insert(host.monitor.id.clone(), regions);
    Ok(HIT_REGIONS
        .get()
        .expect("命中图已初始化")
        .read()
        .map_err(|error| error.to_string())?
        .get(&host.monitor.id)
        .map(Vec::len)
        .unwrap_or(0))
}

/// 在连续拖动期间临时恢复正式宿主完整窗口区域。
pub fn suspend_host_hit_regions(host: &NativeDesktopHost) -> Result<(), String> {
    if unsafe { SetWindowRgn(raw_hwnd(host.hwnd), None, true) } == 0 {
        return Err("暂停正式宿主命中区域失败".to_string());
    }
    Ok(())
}

/// 判断语义区域是否提供正式宿主自身的可见像素证据。
fn is_pixel_probe_region(region: &HitRegion) -> bool {
    region.kind != "workspace-background"
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
        .get(&host.monitor.id)
        .cloned()
        .ok_or_else(|| format!("显示器 {} 的正式宿主命中图尚未发布", host.monitor.id))?
        .into_iter()
        .filter(is_pixel_probe_region)
        .collect::<Vec<_>>();
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

/// 销毁全部正式 Windowed WebView2 与外层 HWND。
pub fn destroy_desktop_hosts() {
    destroy_matching_hosts(|_| true);
}

/// 释放匹配宿主的 OLE、WebView 与 HWND，其他宿主不受影响。
fn destroy_matching_hosts(matches: impl Fn(&str) -> bool) {
    NATIVE_HOSTS.with(|hosts| {
        let mut hosts = hosts.borrow_mut(); // 当前线程独占原生资源集合。
        let removed = (0..hosts.len())
            .rev()
            .filter(|index| matches(&hosts[*index].monitor_id))
            .collect::<Vec<_>>(); // 逆序移除避免索引变化。
        for index in removed {
            let host = hosts.remove(index); // 仅释放本次选中的宿主。
            for (hwnd, _) in &host.drop_targets {
                let _ = unsafe { RevokeDragDrop(raw_hwnd(*hwnd)) };
            }
            drop(host.drop_targets);
            if host.ole_initialized {
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
mod tests {
    use super::{
        SHELL_HIDE_ICONS_BIT, is_pixel_probe_region, match_desktop_item_monitor_assignments,
        shell_flag_state_icons_visible,
    };
    use crate::contracts::{DesktopHostMonitor, DesktopItem, DesktopItemAvailability, HitRegion};
    use std::collections::HashMap;
    use windows::Win32::Foundation::POINT;

    /// 验证透明桌面背景保留输入能力且不参与渲染像素门槛。
    #[test]
    fn pixel_probe_excludes_the_workspace_background() {
        let region = HitRegion {
            id: "workspace-background".to_string(),
            kind: "workspace-background".to_string(),
            left: 0.0,
            top: 0.0,
            width: 1920.0,
            height: 1080.0,
            priority: 1,
        };

        assert!(!is_pixel_probe_region(&region));
    }

    /// 验证 Shell `fHideIcons` 用户设置与恢复可见性语义相反。
    #[test]
    fn shell_hide_icons_setting_maps_to_restore_visibility() {
        assert!(shell_flag_state_icons_visible(0));
        assert!(!shell_flag_state_icons_visible(SHELL_HIDE_ICONS_BIT));
    }

    /// 验证 Explorer 物理图标坐标按显示器边界匹配稳定项目引用。
    #[test]
    fn explorer_positions_assign_items_to_monitors() {
        let monitors = [
            DesktopHostMonitor {
                id: "DISPLAY1".to_string(),
                name: "DISPLAY1".to_string(),
                primary: true,
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                work_x: 0,
                work_y: 0,
                work_width: 1920,
                work_height: 1040,
                scale_factor: 1.0,
                dpi: 96,
            },
            DesktopHostMonitor {
                id: "DISPLAY2".to_string(),
                name: "DISPLAY2".to_string(),
                primary: false,
                x: -1280,
                y: 0,
                width: 1280,
                height: 1024,
                work_x: -1280,
                work_y: 0,
                work_width: 1280,
                work_height: 984,
                scale_factor: 1.25,
                dpi: 120,
            },
        ]; // 当前测试的负坐标混合 DPI 显示器集合。
        let items = [DesktopItem {
            stable_reference: r"C:\Users\User\Desktop\Project".to_string(),
            display_name: "Project".to_string(),
            system_image: None,
            availability: DesktopItemAvailability::Available,
        }]; // 当前等待 Explorer 坐标匹配的项目。
        let positions = HashMap::from([(
            r"c:\users\user\desktop\project".to_string(),
            POINT { x: -400, y: 120 },
        )]); // Explorer 返回的绝对解析名称和物理坐标。

        assert_eq!(
            match_desktop_item_monitor_assignments(&items, &monitors, &positions),
            [(items[0].stable_reference.clone(), "DISPLAY2".to_string())]
        );
    }
}
