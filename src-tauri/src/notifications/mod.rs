mod state;
mod surface;

use crate::contracts::settings::{ApplicationPreferences, ApplicationTheme};
use serde::{Deserialize, Serialize};
use state::{ToastState, Visual};
use std::{
    cell::RefCell,
    ffi::c_void,
    sync::{
        Arc, Mutex,
        atomic::{AtomicIsize, Ordering},
    },
    time::Instant,
};
use surface::Surface;
use tauri::AppHandle;
use windows::{
    Win32::{
        Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM},
        Graphics::{
            Direct2D::Common::D2D_RECT_F,
            Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint, ValidateRect},
        },
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
            WindowsAndMessaging::*,
        },
    },
    core::{PCWSTR, w},
};

thread_local! {
    // COM 资源只在窗口线程创建和释放。
    static SURFACE: RefCell<Option<Surface>> = const { RefCell::new(None) };
}

/// 前端与原生共享的消息协议，不新增标题或动作字段。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ToastRequest
{
    pub r#type: String,  // 消息类型。
    pub content: String, // 正文。
    pub source: String,  // 来源。
    #[serde(rename = "dedupeKey")]
    pub dedupe_key: Option<String>, // 来源内去重键。
    pub loading: Option<bool>, // 加载状态图标。
    #[serde(rename = "durationMs")]
    pub duration_ms: u32, // 调用方明确传入的显示毫秒数，不含悬停暂停。
}

/// 从界面主题及系统设置解析出的绘图参数。
#[derive(Clone)]
struct Appearance
{
    dark: bool,     // 不使用桌面网格主题。
    animated: bool, // 有效动画开关。
    font: String,   // 已保存的字体。
    font_size: u8,  // 已保存的字号。
    radius: f32,    // 已保存的圆角。
}
impl Appearance
{
    /// 跟随系统时不读取保留的手动动画开关。
    fn resolve(preferences: &ApplicationPreferences, system_dark: bool, system_animated: bool) -> Self
    {
        Self {
            dark: match preferences.theme
            {
                ApplicationTheme::System => system_dark,
                ApplicationTheme::Dark => true,
                ApplicationTheme::Light => false,
            },
            animated: if preferences.follow_system_reduced_motion
            {
                system_animated
            }
            else
            {
                preferences.interface_animations
            },
            font: preferences.font_family.clone(),
            font_size: preferences.font_size,
            radius: preferences.corner_radius as f32,
        }
    }
}

/// 最后提交到窗口的消息画面，也是鼠标命中的唯一依据。
#[derive(Clone)]
struct Frame
{
    loading_phase: f32, // 从消息创建时刻计算的 loading 旋转角度。
    id: u64,            // 稳定身份。
    kind: String,       // 状态类型。
    loading: bool,      // 加载图标。
    rect: D2D_RECT_F,   // 当前画面几何。
    visual: Visual,     // 独立透明度与内容状态。
    closable: bool,     // 后层和离场项不参与关闭命中。
}
impl Frame
{
    /// 固定 32×32 DIP 关闭轨道。
    fn close_rect(&self) -> D2D_RECT_F { D2D_RECT_F { left: self.rect.right - 44.0, top: self.rect.top + 16.0, right: self.rect.right - 12.0, bottom: self.rect.top + 48.0 } }
    /// 输入已经由窗口物理坐标转换为 DIP。
    fn hit_close(&self, x: f32, y: f32) -> bool
    {
        let rect = self.close_rect(); // 当前可见按钮区域。
        self.closable && x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
    }
}

/// 单锁持有非 COM 状态，调用窗口 API 前释放，防止同步重入死锁。
struct Runtime
{
    queue: ToastState,                   // 有界消息与动画状态。
    preferences: ApplicationPreferences, // 已保存偏好。
    appearance: Appearance,              // 当前有效外观。
    frames: Vec<Frame>,                  // 上一帧呈现区域。
    scale: f32,                          // 主显示器 DPI。
    hover_id: Option<u64>,               // 关闭按钮悬停。
}

/// 持有主显示器顶层窗口和消息队列。
pub struct ToastManager
{
    hwnd: AtomicIsize,       // 退出时置零，允许重复清理。
    runtime: Mutex<Runtime>, // 唯一状态锁。
}
const SETTINGS_CHANGED: u32 = WM_APP + 41; // 设置变化的异步窗口通知。

impl ToastManager
{
    /// 创建无激活原生窗口并应用初始偏好。
    pub fn install(preferences: ApplicationPreferences) -> Result<Arc<Self>, String>
    {
        let appearance = Appearance::resolve(&preferences, false, true); // 系统值随后在窗口线程解析。
        let manager = Arc::new(Self { hwnd: AtomicIsize::new(0), runtime: Mutex::new(Runtime { queue: ToastState::default(), preferences, appearance, frames: Vec::new(), scale: 1.0, hover_id: None }) }); // HWND 引用的稳定地址。
        let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.map_err(|error| error.to_string())?; // 当前模块。
        let class = WNDCLASSEXW { cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32, lpfnWndProc: Some(window_proc), hInstance: HINSTANCE(module.0), lpszClassName: w!("DesktopToolsNativeToast"), ..Default::default() }; // 无不透明背景刷。
        if unsafe { RegisterClassExW(&class) } == 0 && unsafe { windows::Win32::Foundation::GetLastError() } != windows::Win32::Foundation::ERROR_CLASS_ALREADY_EXISTS
        {
            return Err("注册 Toast 窗口类失败".into());
        }
        let hwnd = unsafe { CreateWindowExW(WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, w!("DesktopToolsNativeToast"), w!("Desktop Tools"), WS_POPUP, 0, 0, 300, 420, None, None, Some(HINSTANCE(module.0)), Some(Arc::as_ptr(&manager) as *const c_void)) }.map_err(|error| error.to_string())?; // 独立顶层窗口。
        manager.hwnd.store(hwnd.0 as isize, Ordering::Release);
        Ok(manager)
    }

    /// 入队、测量及绘制均交回窗口线程。
    pub fn publish(self: &Arc<Self>, app: &AppHandle, request: ToastRequest) -> Result<(), String>
    {
        if request.content.trim().is_empty() || request.source.trim().is_empty() || request.duration_ms == 0 || !matches!(request.r#type.as_str(), "info" | "success" | "warning" | "error")
        {
            return Err("Toast 消息参数无效".into());
        }
        let manager = Arc::clone(self); // 主线程任务执行完之前保持有效。
        app.run_on_main_thread(move || {
            if manager.hwnd().is_invalid()
            {
                return;
            }
            if let Ok(mut runtime) = manager.runtime.lock()
            {
                let animated = runtime.appearance.animated; // 当前有效动画。
                runtime.queue.enqueue(request, 64.0, Instant::now(), animated);
            }
            manager.refresh();
        })
        .map_err(|error| error.to_string())
    }

    /// 只接受已保存偏好，异步更新现有消息。
    pub fn update_preferences(&self, preferences: ApplicationPreferences)
    {
        if let Ok(mut runtime) = self.runtime.lock()
        {
            runtime.preferences = preferences;
        }
        self.settings_changed();
    }

    /// 系统颜色监听和保存偏好共用同一唤醒入口。
    pub fn settings_changed(&self)
    {
        let hwnd = self.hwnd(); // 退出后不再发送。
        if !hwnd.is_invalid()
            && let Err(error) = unsafe { PostMessageW(Some(hwnd), SETTINGS_CHANGED, WPARAM(0), LPARAM(0)) }
        {
            crate::log_warn!("Toast：设置更新通知失败，错误={error}");
        }
    }

    /// 接收程序现有主题入口的解析结果；过期页面偏好不能覆盖新设置。
    pub fn sync_appearance(&self, preferences: ApplicationPreferences, dark: bool, animations_enabled: bool)
    {
        if let Ok(mut runtime) = self.runtime.lock()
        {
            if runtime.preferences != preferences
            {
                return;
            }
            let next = Appearance::resolve(&preferences, dark, animations_enabled); // 不读取或监听系统颜色。
            if runtime.appearance.animated && !next.animated
            {
                for item in &mut runtime.queue.items
                {
                    item.motion.from = item.motion.to;
                }
            }
            runtime.appearance = next;
        }
        self.settings_changed();
    }

    /// 正常退出、重复退出都至多销毁一次窗口。
    pub fn cleanup(&self)
    {
        let hwnd = HWND(self.hwnd.swap(0, Ordering::AcqRel) as *mut c_void); // 先阻止迟到任务。
        if !hwnd.is_invalid()
            && let Err(error) = unsafe { DestroyWindow(hwnd) }
        {
            crate::log_warn!("Toast：销毁窗口失败，错误={error}");
        }
    }
    /// 返回当前 HWND。
    fn hwnd(&self) -> HWND { HWND(self.hwnd.load(Ordering::Acquire) as *mut c_void) }

    /// 绘制失败停止计时器，避免错误刷屏。
    fn refresh(&self)
    {
        if let Err(error) = self.paint()
        {
            unsafe {
                let _ = KillTimer(Some(self.hwnd()), 1);
            }
            crate::log_error!("Toast：刷新失败，错误={error}");
        }
    }

    /// 采样逐条动画，提交窗口前释放状态锁。
    fn paint(&self) -> Result<(), String>
    {
        let hwnd = self.hwnd(); // 当前 UI 线程拥有。
        if hwnd.is_invalid()
        {
            return Ok(());
        }
        if self.runtime.lock().map_err(|_| "Toast 状态不可用")?.queue.items.is_empty()
        {
            unsafe {
                let _ = KillTimer(Some(hwnd), 1);
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            SURFACE.with(|surface| surface.borrow_mut().take());
            return Ok(());
        } // 空闲主题变化不分配绘图设备或像素存储。
        let monitor = unsafe { MonitorFromPoint(POINT::default(), MONITOR_DEFAULTTOPRIMARY) }; // 固定主屏。
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() }; // 工作区边界。
        if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool()
        {
            return Err("读取主显示器工作区失败".into());
        }
        let mut dpi_x = 96; // 横向 DPI。
        let mut dpi_y = 96; // 纵向 DPI。
        unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) }.map_err(|error| error.to_string())?;
        let scale = dpi_x as f32 / 96.0; // DIP 转物理像素。
        let size = SIZE { cx: (300.0 * scale).ceil() as i32, cy: info.rcWork.bottom - info.rcWork.top }; // 未绘制区 Alpha 为零且鼠标穿透。
        let now = Instant::now(); // 帧时钟。
        let (frames, appearance, hover_id, expanded, delay) = {
            let mut runtime = self.runtime.lock().map_err(|_| "Toast 状态不可用")?; // 不嵌套加锁。
            let appearance = runtime.appearance.clone(); // 绘图快照。
            let animated = appearance.animated; // 本帧是否插值。
            runtime.scale = scale;
            let mut changed = false; // 文字高度变化才重算目标。
            SURFACE.with(|surface| -> Result<(), String> {
                let mut surface = surface.borrow_mut(); // 窗口线程 COM 资源。
                if surface.as_ref().is_none_or(|surface| surface.size != size || surface.scale != scale)
                {
                    *surface = Some(Surface::new(size, scale, &appearance).map_err(|error| error.to_string())?);
                }
                let surface = surface.as_mut().expect("表面已创建"); // 缓存绘图设备。
                surface.configure(&appearance).map_err(|error| error.to_string())?;
                for item in &mut runtime.queue.items
                {
                    let height = surface.measure(item.id, &item.request.content).map_err(|error| error.to_string())?; // 按消息身份缓存测量。
                    if height != item.height
                    {
                        item.height = height;
                        changed = true;
                    }
                }
                Ok(())
            })?;
            if changed
            {
                runtime.queue.reflow(now, animated);
            }
            let delay = runtime.queue.tick(now, animated); // 自动关闭与有限动画。
            let frames = runtime
                .queue
                .items
                .iter()
                .map(|item| {
                    let visual = item.motion.sample(now, item.closing, animated); // 独立透明度。
                    let width = 296.0 * visual.scale; // 居中缩小后层。
                    let top = size.cy as f32 / scale - 16.0 + visual.top; // 底部预留退出空间。
                    Frame {
                        loading_phase: if animated
                        {
                            now.saturating_duration_since(item.created_at).as_secs_f32() * std::f32::consts::TAU
                        }
                        else
                        {
                            0.0
                        },
                        id: item.id,
                        kind: item.request.r#type.clone(),
                        loading: item.request.loading.unwrap_or(false),
                        rect: D2D_RECT_F { left: (300.0 - width) / 2.0, right: (300.0 + width) / 2.0, top, bottom: top + visual.height * visual.scale },
                        visual,
                        closable: !item.closing && visual.content >= 0.999 && visual.alpha >= 0.999,
                    }
                })
                .collect::<Vec<_>>(); // 旧层先绘制。
            runtime.frames = frames.clone();
            (frames, appearance, runtime.hover_id, runtime.queue.hovered_since.is_some(), delay)
        };
        if frames.is_empty()
        {
            unsafe {
                let _ = KillTimer(Some(hwnd), 1);
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            SURFACE.with(|surface| surface.borrow_mut().take()); // 空队列无常驻绘图资源。
            return Ok(());
        }
        unsafe { SetWindowPos(hwnd, Some(HWND_TOPMOST), info.rcWork.right - size.cx - (14.0 * scale).round() as i32, info.rcWork.top, size.cx, size.cy, SWP_NOACTIVATE | SWP_NOOWNERZORDER) }.map_err(|error| error.to_string())?;
        SURFACE.with(|surface| -> Result<(), String> {
            let mut surface = surface.borrow_mut(); // 绘制时不持消息锁。
            let surface = surface.as_mut().expect("非空队列表面已准备");
            surface.draw(&frames, &appearance, hover_id, expanded).and_then(|()| surface.present(hwnd)).map_err(|error| error.to_string())
        })?;
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            if let Some(delay) = delay
            {
                SetTimer(Some(hwnd), 1, delay, None);
            }
            else
            {
                let _ = KillTimer(Some(hwnd), 1);
            }
        }
        Ok(())
    }

    /// 依据最后实际呈现的几何处理关闭和悬停。
    fn pointer(&self, point: Option<POINT>, close: bool)
    {
        let mut changed = close; // 点击后刷新离场帧。
        if let Ok(mut runtime) = self.runtime.lock()
        {
            let hit = point.and_then(|point| runtime.frames.iter().rev().find(|frame| frame.hit_close(point.x as f32 / runtime.scale, point.y as f32 / runtime.scale)).map(|frame| frame.id)); // 当前可见按钮。
            let animated = runtime.appearance.animated; // 动画开关。
            let now = Instant::now(); // 输入时钟。
            if close
            {
                if let Some(id) = hit
                {
                    runtime.queue.close(id, now, animated);
                }
            }
            else
            {
                changed = runtime.hover_id != hit || runtime.queue.hovered_since.is_some() != point.is_some();
                runtime.hover_id = hit;
                runtime.queue.hover(point.is_some(), now, animated);
            }
        }
        if changed
        {
            self.refresh();
        }
    }
}

/// 顶层窗口只处理有限计时、输入与系统广播。
unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT
{
    unsafe {
        if message == WM_NCCREATE
        {
            let create = &*(lparam.0 as *const CREATESTRUCTW); // 稳定 Arc 地址。
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const ToastManager; // HWND 不独立拥有 Arc。
        if pointer.is_null()
        {
            return DefWindowProcW(hwnd, message, wparam, lparam);
        }
        let manager = &*pointer; // manager 活到窗口销毁之后。
        match message
        {
            WM_PAINT =>
            {
                let _ = ValidateRect(Some(hwnd), None);
                LRESULT(0)
            }
            WM_TIMER =>
            {
                manager.refresh();
                LRESULT(0)
            }
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_MOUSEMOVE =>
            {
                use windows::Win32::UI::Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent};
                let mut tracking = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 }; // 仅订阅离开。
                let _ = TrackMouseEvent(&mut tracking);
                manager.pointer(Some(POINT { x: (lparam.0 as u16 as i16) as i32, y: ((lparam.0 >> 16) as u16 as i16) as i32 }), false);
                LRESULT(0)
            }
            windows::Win32::UI::Controls::WM_MOUSELEAVE =>
            {
                manager.pointer(None, false);
                LRESULT(0)
            }
            WM_LBUTTONUP =>
            {
                manager.pointer(Some(POINT { x: (lparam.0 as u16 as i16) as i32, y: ((lparam.0 >> 16) as u16 as i16) as i32 }), true);
                LRESULT(0)
            }
            SETTINGS_CHANGED =>
            {
                manager.refresh();
                LRESULT(0)
            }
            WM_DISPLAYCHANGE | WM_DPICHANGED =>
            {
                manager.refresh();
                LRESULT(0)
            }
            WM_NCDESTROY =>
            {
                manager.hwnd.store(0, Ordering::Release);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                let _ = KillTimer(Some(hwnd), 1);
                SURFACE.with(|surface| surface.borrow_mut().take());
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

#[cfg(test)]
#[path = "notifications.test.rs"]
mod tests;

#[cfg(test)]
#[path = "resource.test.rs"]
mod resource_tests;
