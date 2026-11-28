//! 每显示器一块透明宿主窗口 + DirectComposition 视觉树 + Direct2D 表面。
//!
//! 结构：一个 `WS_POPUP` 透明窗口（`WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP`）
//! 覆盖该显示器工作区；窗口上挂一个 DirectComposition target，target 的根视觉内容是一块
//! `IDCompositionSurface`；每帧在该表面上用 Direct2D 画完再 `Commit`。
//!
//! 绘制事务的三条硬规则（复核 F1/F2 的直接对应）：
//! 1. **先校验再开事务**：绘制列表的栈配对、表面尺寸、窗口矩形都在 `BeginDraw` 之前检查；
//! 2. **任何失败都要收尾**：一旦 `BeginDraw` 成功，无论后续哪一步出错，都要按后进先出把
//!    Direct2D target 解绑、DComp 批次结束；**失败帧一律不 `Commit`**，错误里同时带上主
//!    HRESULT 与收尾 HRESULT；
//! 3. **按更新偏移绘制**：`BeginDraw` 会返回本次更新的左上角偏移（物理像素），整帧内容必须
//!    平移 `(offset - update.origin) / scale` 并裁剪到更新区域，否则局部更新会画错位置或破坏区域外像素。
//!
//! COM：诊断入口自己开公寓，用 [`ComApartment`] 做对称释放；所有图形资源必须比它先释放
//! （在 `run_demo` 里 guard 先声明，Rust 的局部变量反向析构保证了这一点）。

use std::sync::OnceLock;
use std::time::{Duration, Instant};

use windows::{
    Win32::{
        Foundation::{E_FAIL, E_INVALIDARG, HANDLE, HINSTANCE, HMODULE, HWND, LPARAM, LRESULT, POINT, RECT, RPC_E_CHANGED_MODE, WPARAM},
        Graphics::{
            Direct2D::{Common::*, *},
            Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0},
            Direct3D11::{D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device},
            DirectComposition::{DCompositionCreateDevice, IDCompositionDevice, IDCompositionSurface, IDCompositionTarget, IDCompositionVisual},
            Dxgi::{Common::DXGI_ALPHA_MODE_PREMULTIPLIED, Common::DXGI_FORMAT_B8G8R8A8_UNORM, IDXGIDevice, IDXGISurface},
            Gdi::ValidateRect,
        },
        System::{
            Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
            LibraryLoader::GetModuleHandleW,
        },
        UI::{HiDpi::*, WindowsAndMessaging::*},
    },
    core::{Error, HSTRING, Interface, PCWSTR, Result, w},
};

use super::draw::{Color, DrawCommand, DrawList, Rect};
use super::geometry::{DpiScale, HostGeometry, PixelRect, pixel_rect_to_dip};
use super::painter::Painter;

/// 宿主窗口类名。
const HOST_CLASS: PCWSTR = w!("DesktopToolsNativeSurfaceHost");

/// 给失败结果补上阶段名与 HRESULT：诊断时要把「哪一步 + 什么错误码」一起报出来。
pub(super) fn stage<T>(name: &str, result: Result<T>) -> Result<T> { result.map_err(|error| Error::new(error.code(), format!("{name} 失败（HRESULT=0x{:08X}）：{error}", error.code().0 as u32))) }

/// 只有已知的内容错误可沿用旧表面。未知设备错误及任何收尾失败都必须终止诊断。
#[derive(Debug)]
pub(super) enum FrameFailure
{
    Content(Error),
    Stop
    {
        primary: Error,
        cleanup: Vec<Error>,
    },
}
type FrameResult<T> = std::result::Result<T, FrameFailure>;

impl From<Error> for FrameFailure
{
    fn from(primary: Error) -> Self { Self::Stop { primary, cleanup: Vec::new() } }
}
impl From<FrameFailure> for Error
{
    fn from(value: FrameFailure) -> Self
    {
        match value
        {
            FrameFailure::Content(error) => error,
            FrameFailure::Stop { primary, cleanup } if cleanup.is_empty() => primary,
            FrameFailure::Stop { primary, cleanup } => Error::new(primary.code(), format!("{primary}；收尾：{}", cleanup.iter().map(Error::to_string).collect::<Vec<_>>().join("；"))),
        }
    }
}

/// EndDraw 失败即不能再用这条设备链；双重失败不把收尾 HRESULT 降为字符串。
fn finished<T>(body: FrameResult<T>, cleanup: Result<()>) -> FrameResult<T>
{
    let Err(end) = cleanup
    else
    {
        return body;
    };
    match body
    {
        Ok(_) => Err(end.into()),
        Err(FrameFailure::Content(primary)) => Err(FrameFailure::Stop { primary, cleanup: vec![end] }),
        Err(FrameFailure::Stop { primary, mut cleanup }) =>
        {
            cleanup.push(end);
            Err(FrameFailure::Stop { primary, cleanup })
        }
    }
}

/// 只有候选表面的内容无效时跳过发布；返回 Err 会立即退出宿主循环。
fn dpi_candidate<T>(candidate: FrameResult<T>) -> Result<Option<T>>
{
    match candidate
    {
        Ok(v) => Ok(Some(v)),
        Err(FrameFailure::Content(error)) =>
        {
            eprintln!("DPI 重绘内容无效，保留旧表面：{error}");
            Ok(None)
        }
        Err(fatal) => Err(fatal.into()),
    }
}
/// 禁止用 1×1 替代空窗口；计算边长也不能溢出 Win32 的有符号范围。
pub(super) fn surface_bounds(geometry: HostGeometry) -> Result<PixelRect>
{
    let r = geometry.surface;
    let width = r.right.checked_sub(r.left).filter(|x| *x > 0);
    let height = r.bottom.checked_sub(r.top).filter(|x| *x > 0);
    match (width, height)
    {
        (Some(w), Some(h)) if geometry.dpi.dpi > 0 => Ok(PixelRect::new(0, 0, w, h)),
        _ => Err(Error::new(E_FAIL, "宿主矩形或 DPI 无效")),
    }
}

/// 源更新矩形的左上角映射到 DXGI 位图中的 offset，而不是位图原点。
fn update_mapping(update: PixelRect, offset: POINT, dpi: DpiScale) -> (f32, f32, Rect)
{
    let source = pixel_rect_to_dip(&update, dpi);
    let (x, y) = dpi.px_point_to_dip(offset.x, offset.y);
    (x - source.x, y - source.y, Rect::new(x, y, source.width, source.height))
}
/// 一次绘制的结果读数。
#[derive(Clone, Debug, Default)]
pub struct PaintOutcome
{
    /// DirectComposition 返回的更新偏移（物理像素）。
    pub offset: POINT,
    /// 本次更新区域（物理像素，已按表面裁剪）。
    pub update: PixelRect,
    /// 是否整面更新。
    pub full: bool,
}

/// 一块宿主表面的诊断读数（只描述事实，不含性能结论）。
#[derive(Clone, Debug, serde::Serialize)]
pub struct HostReport
{
    pub monitor: String,
    pub primary: bool,
    pub work_area: String,
    pub surface: String,
    pub dpi: u32,
    pub scale: f64,
    pub create_ms: f64,
    pub paint_ms: f64,
    /// 表面绘制路径（BeginDraw 拿到的接口）。
    pub surface_path: String,
    /// 首帧的更新偏移与是否整面更新，验证非零偏移分支是否被走到。
    pub first_offset: String,
    pub first_full_update: bool,
}

/// 一次演示运行的汇总读数。
#[derive(Clone, Debug, serde::Serialize)]
pub struct DemoReport
{
    pub pid: u32,
    pub monitors: usize,
    pub font_family: String,
    pub dpi_awareness: String,
    pub create_total_ms: f64,
    pub hosts: Vec<HostReport>,
    pub windows_after_exit: usize,
    pub exit_code: i32,
}

/// 进程级图形设备集合：D3D11 设备、它的 DXGI 接口、由同一设备建出的 Direct2D 设备与上下文，
/// 以及 DirectComposition 设备。绘制上下文与合成表面必须同源，否则表面画上去不显示。
pub struct Devices
{
    #[allow(dead_code)] // 只用来创建并持有上下文，必须比 context 活得久。
    d2d: ID2D1Device, // 由同一 DXGI 设备建出的 Direct2D 设备。
    pub context: ID2D1DeviceContext,      // 复用的绘制上下文。
    pub composition: IDCompositionDevice, // 合成设备。
}

impl Devices
{
    /// 建一条同源设备链：D3D11(BGRA) → DXGI → D2D → DComp。
    pub fn new() -> Result<Self>
    {
        let levels = [D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_10_0];
        let mut device: Option<ID3D11Device> = None;
        let mut level = D3D_FEATURE_LEVEL_10_0;
        unsafe {
            // 不用 SINGLETHREADED：DirectComposition 内部会为该设备建 Direct2D 设备，单线程标志会让它拿不到
            // 线程保护；多线程保护由我们自己的串行绘制保证。
            D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, Some(&levels), D3D11_SDK_VERSION, Some(&mut device), Some(&mut level), None)?;
        }
        let device = device.ok_or_else(|| Error::new(E_FAIL, "D3D11CreateDevice 未返回设备"))?;
        let dxgi: IDXGIDevice = stage("ID3D11Device→IDXGIDevice", device.cast())?;
        let composition: IDCompositionDevice = stage("DCompositionCreateDevice(dxgi)", unsafe { DCompositionCreateDevice(&dxgi) })?;
        let factory: ID2D1Factory1 = stage("D2D1CreateFactory(ID2D1Factory1)", unsafe { D2D1CreateFactory::<ID2D1Factory1>(D2D1_FACTORY_TYPE_MULTI_THREADED, None) })?;
        let d2d: ID2D1Device = stage("ID2D1Factory1::CreateDevice", unsafe { factory.CreateDevice(&dxgi) })?;
        let context: ID2D1DeviceContext = stage("ID2D1Device::CreateDeviceContext", unsafe { d2d.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE) })?;
        Ok(Self { d2d, context, composition })
    }
}

/// 同线程 COM 公寓：成功初始化才负责反初始化，且必须比所有 COM 资源活得久。
pub struct ComApartment
{
    _thread: std::marker::PhantomData<std::rc::Rc<()>>, // 禁止跨线程析构。
    owned: bool,                                        // 是否由本 guard 初始化（RPC_E_CHANGED_MODE 时为 false）。
    note: String,                                       // 诊断用的结果说明。
}

impl ComApartment
{
    /// 进入 STA 公寓；只有真正的失败才返回 Err。
    pub fn enter() -> Result<Self>
    {
        let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if result.is_ok()
        {
            return Ok(Self { _thread: Default::default(), owned: true, note: "apartment_threaded".to_string() });
        }
        if result == RPC_E_CHANGED_MODE
        {
            // 线程已在别的公寓里：沿用现状，不接管释放。
            return Ok(Self { _thread: Default::default(), owned: false, note: "沿用既有 MTA 单元".to_string() });
        }
        Err(Error::new(result, format!("CoInitializeEx 失败（HRESULT=0x{:08X}）", result.0 as u32)))
    }

    /// 诊断说明。
    pub fn note(&self) -> &str { &self.note }
}

impl Drop for ComApartment
{
    fn drop(&mut self)
    {
        if self.owned
        {
            unsafe { CoUninitialize() };
        }
    }
}

/// 单显示器的原生宿主。
pub struct SurfaceHost
{
    hwnd: HWND,
    geometry: HostGeometry,
    _target: IDCompositionTarget,
    visual: IDCompositionVisual,
    current: Option<IDCompositionSurface>,
}

impl SurfaceHost
{
    pub(super) fn attach(hwnd: HWND, geometry: HostGeometry, device: &IDCompositionDevice) -> Result<Self>
    {
        surface_bounds(geometry)?;
        let visual = stage("CreateVisual", unsafe { device.CreateVisual() })?;
        let target = stage("CreateTargetForHwnd", unsafe { device.CreateTargetForHwnd(hwnd, true) })?;
        stage("SetRoot", unsafe { target.SetRoot(&visual) })?;
        Ok(Self { hwnd, geometry, _target: target, visual, current: None })
    }

    /// EndDraw 会把更新排进设备的下一次提交。只写未发布表面，避免另一窗口 Commit 时夹带失败帧。
    pub(super) fn prepare(geometry: HostGeometry, painter: &mut Painter, devices: &Devices, list: &DrawList) -> FrameResult<(IDCompositionSurface, PaintOutcome)>
    {
        let bounds = surface_bounds(geometry).map_err(FrameFailure::Content)?;
        validate_list(list).map_err(FrameFailure::Content)?;
        let surface = stage("CreateSurface", unsafe { devices.composition.CreateSurface(bounds.width() as u32, bounds.height() as u32, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_ALPHA_MODE_PREMULTIPLIED) })?;
        let outcome = paint_update(&surface, bounds, bounds, geometry.dpi, painter, devices, list)?;
        Ok((surface, outcome))
    }

    /// 只发布已完整绘制的候选表面；提交失败则回滚并由诊断入口终止设备使用。
    pub(super) fn publish(&mut self, geometry: HostGeometry, surface: IDCompositionSurface, devices: &Devices) -> Result<()>
    {
        stage("SetContent", unsafe { self.visual.SetContent(&surface) })?;
        if let Err(error) = stage("Commit", unsafe { devices.composition.Commit() })
        {
            // 移除未提交的替换，不让下一次设备提交发布它。
            let restore = stage("恢复旧 SetContent", unsafe {
                match self.current.as_ref()
                {
                    Some(previous) => self.visual.SetContent(previous),
                    None => self.visual.SetContent(None),
                }
            });
            return finished(Err(error.into()), restore).map_err(Error::from);
        }
        self.geometry = geometry;
        self.current = Some(surface);
        Ok(())
    }
}

fn validate_list(list: &DrawList) -> Result<()> { list.validate().map_err(|e| Error::new(E_FAIL, format!("绘制列表栈未配对：{e:?}"))) }

/// 只更新未发布表面；两个事务各自把所有可能失败的操作放进内层 Result，再无条件收尾。
fn paint_update(surface: &IDCompositionSurface, bounds: PixelRect, update: PixelRect, dpi: DpiScale, painter: &mut Painter, devices: &Devices, list: &DrawList) -> FrameResult<PaintOutcome>
{
    validate_list(list).map_err(FrameFailure::Content)?;
    if update.left < 0 || update.top < 0 || update.right > bounds.right || update.bottom > bounds.bottom || update.right <= update.left || update.bottom <= update.top
    {
        return Err(FrameFailure::Content(Error::new(E_INVALIDARG, "更新矩形无效")));
    }
    let rect = RECT { left: update.left, top: update.top, right: update.right, bottom: update.bottom };
    let mut offset = POINT::default();
    let dxgi: IDXGISurface = stage("DComp BeginDraw(IDXGISurface)", unsafe { surface.BeginDraw(Some(&rect), &mut offset) })?;
    let ctx = &devices.context;
    let body = (|| {
        let props = D2D1_BITMAP_PROPERTIES1 { pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED }, dpiX: dpi.dpi as f32, dpiY: dpi.dpi as f32, bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW, ..Default::default() };
        let bmp = stage("CreateBitmapFromDxgiSurface", unsafe { ctx.CreateBitmapFromDxgiSurface(&dxgi, Some(&props)) })?;
        let (dx, dy, clip) = update_mapping(update, offset, dpi);
        let clip_rect = D2D_RECT_F { left: clip.x, top: clip.y, right: clip.right(), bottom: clip.bottom() };
        unsafe {
            ctx.SetTarget(&bmp);
            ctx.SetDpi(dpi.dpi as f32, dpi.dpi as f32);
            ctx.BeginDraw();
            ctx.PushAxisAlignedClip(&clip_rect, D2D1_ANTIALIAS_MODE_ALIASED);
            ctx.Clear(Some(&D2D1_COLOR_F::default()));
        }
        let body = stage("Painter::paint", painter.paint(ctx, &list.translated(dx, dy), dpi, None)).map_err(|e| if e.code() == E_INVALIDARG { FrameFailure::Content(e) } else { e.into() });
        unsafe {
            ctx.PopAxisAlignedClip();
        }
        let end = stage("D2D EndDraw", unsafe { ctx.EndDraw(None, None) });
        unsafe {
            ctx.SetTarget(None);
        }
        finished(body, end)
    })();
    drop(dxgi);
    finished(body, stage("DComp EndDraw", unsafe { surface.EndDraw() }))?;
    Ok(PaintOutcome { offset, update, full: update == bounds })
}
/// 从窗口矩形与 DPI 求宿主几何；任一查询失败都返回 Err（不拿零矩形兜底）。
fn host_geometry(hwnd: HWND) -> Result<HostGeometry>
{
    let mut rect = RECT::default();
    stage("GetWindowRect", unsafe { GetWindowRect(hwnd, &mut rect) })?;
    if rect.right <= rect.left || rect.bottom <= rect.top
    {
        return Err(Error::new(E_FAIL, "窗口矩形无效"));
    }
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0
    {
        return Err(Error::new(E_FAIL, "GetDpiForWindow 返回 0"));
    }
    Ok(HostGeometry::for_work_area(PixelRect::new(rect.left, rect.top, rect.right, rect.bottom), DpiScale::new(dpi)))
}

/// 一次演示里创建的宿主窗口集合，离开作用域时按创建顺序销毁。
pub(super) struct DemoWindows
{
    pub(super) hwnds: Vec<HWND>, // 已创建的宿主窗口。
}

impl Drop for DemoWindows
{
    fn drop(&mut self)
    {
        for hwnd in self.hwnds.drain(..)
        {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
        }
    }
}

// 窗口过程和消息泵同线程。第一项移动失败须保留到主循环读取，不能被后续成功覆盖。
thread_local! { static DPI_EVENT: std::cell::Cell<std::result::Result<bool,(HWND,windows::core::HRESULT)>> = const {std::cell::Cell::new(Ok(false))}; }

fn record_dpi_move(hwnd: HWND, result: Result<()>)
{
    if DPI_EVENT.get().is_ok()
    {
        DPI_EVENT.set(result.map(|()| true).map_err(|e| (hwnd, e.code())));
    }
}

fn take_dpi_event() -> Result<bool> { DPI_EVENT.replace(Ok(false)).map_err(|(hwnd, code)| Error::new(code, format!("WM_DPICHANGED 调整窗口失败，停用宿主：HWND={hwnd:?} HRESULT=0x{:08X}", code.0 as u32))) }
/// 宿主窗口过程：只处理 DPI 变化与销毁，绘制全部走合成表面。
unsafe extern "system" fn host_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT
{
    match message
    {
        WM_DESTROY | WM_DISPLAYCHANGE =>
        {
            crate::platform::windows::environment_watch::signal_environment();
        }
        WM_DPICHANGED =>
        {
            crate::platform::windows::environment_watch::signal_environment();
            // 按系统建议的矩形调整窗口；建议矩形无效时保留原矩形，不把 0 尺寸传下去。
            let Some(suggested) = (unsafe { (lparam.0 as *const RECT).as_ref() })
            else
            {
                record_dpi_move(hwnd, Err(Error::new(E_INVALIDARG, "无建议矩形")));
                return LRESULT(0);
            };
            let width = suggested.right.checked_sub(suggested.left).unwrap_or(0);
            let height = suggested.bottom.checked_sub(suggested.top).unwrap_or(0);
            let applied = if width > 0 && height > 0
            {
                unsafe { SetWindowPos(hwnd, None, suggested.left, suggested.top, width, height, SWP_NOZORDER | SWP_NOACTIVATE) }
            }
            else
            {
                Err(Error::new(E_INVALIDARG, "无效建议矩形"))
            };
            record_dpi_move(hwnd, applied);
            return LRESULT(0);
        }
        WM_ERASEBKGND => return LRESULT(1), // 合成表面自己负责像素。
        WM_PAINT =>
        {
            unsafe {
                let _ = ValidateRect(Some(hwnd), None);
            }
            return LRESULT(0);
        }
        _ =>
        {}
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

/// 注册宿主窗口类（进程内只注册一次）。
fn register_host_class(instance: HINSTANCE) -> Result<()>
{
    static REGISTERED: OnceLock<std::result::Result<(), String>> = OnceLock::new();
    let outcome = REGISTERED.get_or_init(|| {
        let class = WNDCLASSEXW { cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32, style: CS_HREDRAW | CS_VREDRAW, lpfnWndProc: Some(host_proc), hInstance: instance, hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(), lpszClassName: HOST_CLASS, ..Default::default() };
        let atom = unsafe { RegisterClassExW(&class) };
        if atom == 0
        {
            let code = unsafe { windows::Win32::Foundation::GetLastError() };
            return Err(format!("RegisterClassExW 失败：{code:?}"));
        }
        Ok(())
    });
    match outcome
    {
        Ok(()) => Ok(()),
        Err(message) => Err(Error::new(E_FAIL, message.clone())),
    }
}

/// 创建一块覆盖工作区的透明宿主窗口（不激活、不占任务栏、无重定向位图）。
pub(super) fn create_host_window(instance: HINSTANCE, geometry: HostGeometry, title: &str) -> Result<HWND>
{
    register_host_class(instance)?;
    let area = geometry.work_area;
    let style = WS_POPUP;
    let ex_style = WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP;
    unsafe { CreateWindowExW(ex_style, HOST_CLASS, &HSTRING::from(title), style, area.left, area.top, area.width(), area.height(), None, None, Some(instance), None) }
}

/// 演示用的绘制内容：一块半透明圆角卡片 + 中文标题与说明 + 一个描边矩形。
fn demo_list(geometry: HostGeometry, index: usize, family: &str) -> DrawList
{
    let (width, _height) = geometry.dip_size();
    let card = Rect::new(36.0, 36.0 + 24.0 * index as f32, (width - 72.0).min(420.0).max(240.0), 148.0);
    let mut list = DrawList::new();
    // 卡片底：半透明深色 + 1px 描边（半透明混合与描边都在这一帧里验证）。
    list.push(DrawCommand::FillRoundRect { rect: card, radius: 16.0, color: Color::rgba(0x1E, 0x1E, 0x24, 0xC0) });
    list.push(DrawCommand::StrokeRoundRect { rect: card, radius: 16.0, width: 1.0, color: Color::rgba(0xFF, 0xFF, 0xFF, 0x33) });
    // 内部再叠一个更亮的描边矩形，专门用来看圆角与线宽是否随 DPI 正确缩放。
    let inner = Rect::new(card.x + 16.0, card.y + 88.0, card.width - 32.0, 44.0);
    list.push(DrawCommand::StrokeRoundRect { rect: inner, radius: 10.0, width: 1.5, color: Color::rgba(0x3D, 0x7A, 0xEA, 0xCC) });
    list.push(DrawCommand::PushClip { rect: inner });
    list.push(DrawCommand::Text { rect: Rect::new(inner.x + 10.0, inner.y + 12.0, inner.width - 20.0, 20.0), text: format!("原生渲染核心 · 显示器 {} · {}", index + 1, family), size: 13.0, weight: 400, color: Color::rgba(0xE8, 0xF0, 0xFF, 0xFF), align: super::draw::TextAlign::Start, max_lines: 1 });
    list.push(DrawCommand::PopClip);
    list.push(DrawCommand::Text { rect: Rect::new(card.x + 20.0, card.y + 18.0, card.width - 40.0, 28.0), text: "原生宿主首测".to_string(), size: 20.0, weight: 600, color: Color::rgba(0xFF, 0xFF, 0xFF, 0xFF), align: super::draw::TextAlign::Start, max_lines: 1 });
    list.push(DrawCommand::PushOpacity { opacity: 0.85 });
    list.push(DrawCommand::Text { rect: Rect::new(card.x + 20.0, card.y + 50.0, card.width - 40.0, 40.0), text: "DirectComposition 表面 + Direct2D 绘制，中文按系统界面字体回退。".to_string(), size: 13.0, weight: 400, color: Color::rgba(0xD8, 0xDC, 0xE4, 0xFF), align: super::draw::TextAlign::Start, max_lines: 2 });
    list.push(DrawCommand::PopOpacity);
    list
}

/// 原生首帧不是接管凭据。真实 Shell 图像与可用输入未接入，只建隐藏子窗，不显示或隐藏 Explorer。
/// 线程内资源由局部变量逆序析构：视觉树 → HWND → 图形设备 → guardian → 公寓。
pub(crate) fn run_protected_desktop() -> std::result::Result<(), String>
{
    use crate::platform::windows::{
        desktop_items_visible, discover_shell_desktop, enumerate_desktop_monitors,
        environment_watch::{DesktopEnvironmentWatch, EnvironmentNotification},
    };
    use windows::Win32::{Foundation::CloseHandle, System::Threading::CreateEventW};
    let _apartment = ComApartment::enter().map_err(|e| e.to_string())?;
    let (shell, _guardian) = crate::host::open_guarded_desktop(crate::recovery::GuardianMode::MonitorOnly)?;
    let monitors = enumerate_desktop_monitors()?;
    if monitors.is_empty()
    {
        return Err("无活动显示器；Explorer 未接管".into());
    }
    let (scene, settings) = super::workspace_scene::read_scene().map_err(|e| e.to_string())?;
    let stop = unsafe { CreateEventW(None, true, false, None) }.map_err(|e| e.to_string())?;
    let mut watch = DesktopEnvironmentWatch::new(stop).inspect_err(|_| {
        let _ = unsafe { CloseHandle(stop) };
    })?;
    let devices = Devices::new().map_err(|e| e.to_string())?;
    let mut painter = Painter::new().map_err(|e| e.to_string())?;
    let module = unsafe { GetModuleHandleW(None) }.map_err(|e| e.to_string())?;
    let mut windows = DemoWindows { hwnds: Vec::new() };
    let mut hosts = Vec::new();
    for monitor in &monitors
    {
        let geometry = HostGeometry::for_work_area(PixelRect::from_xywh(monitor.work_x, monitor.work_y, monitor.work_width as i32, monitor.work_height as i32), DpiScale::new(monitor.dpi));
        surface_bounds(geometry).map_err(|e| e.to_string())?;
        let hwnd = create_host_window(HINSTANCE(module.0), geometry, &format!("NativeDesktop·{}", monitor.id)).map_err(|e| e.to_string())?;
        windows.hwnds.push(hwnd);
        shell.attach_hidden(hwnd, monitor)?;
        let mut host = SurfaceHost::attach(hwnd, geometry, &devices.composition).map_err(|e| e.to_string())?;
        let list = super::workspace_scene::draw_snapshot(&scene, &settings, &monitor.id, geometry);
        let (surface, _) = SurfaceHost::prepare(geometry, &mut painter, &devices, &list).map_err(|e| Error::from(e).to_string())?;
        host.publish(geometry, surface, &devices).map_err(|e| e.to_string())?;
        crate::log_info!("原生宿主：已附着未显示，显示器={}，HWND={:?}，Explorer父窗=0x{:X}，布局修订={}，指令数={}", monitor.id, hwnd, shell.explorer_host, scene.revision, list.len());
        hosts.push(host);
    }
    // 最后安装退出接收器。初始化任一步失败均沿 RAII 撤下子窗并停止 guardian。
    let _exit = crate::app::desktop_menu::ExitReceiver::install()?;
    crate::log_info!("原生宿主：保护态未接管，Shell 图像与交互未就绪；Explorer项目可见={}，退出命令已就绪", desktop_items_visible(shell.def_view));
    loop
    {
        match watch.wait()?
        {
            EnvironmentNotification::Stop => break,
            EnvironmentNotification::Visibility => continue, // 用户的图标开关由 Explorer 与 guardian 持有，不干预。
            _ =>
            {
                let now = discover_shell_desktop()?;
                if now.explorer_host != shell.explorer_host || now.def_view != shell.def_view || now.list_view != shell.list_view || enumerate_desktop_monitors()? != monitors || hosts.iter().any(|h| unsafe { GetParent(h.hwnd) }.ok().map(|p| p.0 as isize) != Some(shell.explorer_host)) || take_dpi_event().map_err(|e| e.to_string())?
                {
                    return Err("原生宿主环境变化，停止而不接管 Explorer；请重新启动".into());
                }
            }
        }
    }
    crate::log_info!("原生宿主：正常停止，Explorer 未接管；释放 {} 个隐藏宿主及 guardian", hosts.len());
    Ok(())
}
/// 跑一次演示：每显示器一块透明宿主，画一帧，保持 `hold` 后退出。
pub fn run_demo(hold: Duration) -> Result<DemoReport> { run_scene(hold, |geometry, _, index, family| demo_list(geometry, index, family), None) }

/// 共用的逐屏绘制与消息泵。场景只消费当前屏的局部几何。
pub(super) fn run_scene(hold: Duration, scene: impl Fn(HostGeometry, &crate::contracts::DesktopHostMonitor, usize, &str) -> DrawList, thumbs: Option<&super::thumbs::Thumbs>) -> Result<DemoReport>
{
    // 公寓 guard 先声明 → 最后析构：所有 COM 资源都比它早释放。
    let apartment = ComApartment::enter()?;
    let awareness = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let awareness_note = match awareness
    {
        Ok(()) => "per_monitor_v2".to_string(),
        Err(error) => format!("沿用现有设置（HRESULT=0x{:08X}）", error.code().0 as u32),
    };

    let monitors = crate::platform::windows::enumerate_desktop_monitors().map_err(|message| Error::new(E_FAIL, message))?;
    let instance: HINSTANCE = stage("GetModuleHandleW", unsafe { GetModuleHandleW(None) }).map(|h| HINSTANCE(h.0))?;
    let devices = Devices::new()?;
    let mut painter = stage("Painter::new", Painter::new())?;
    let family = painter.family().to_string();

    let started = Instant::now();
    let mut windows = DemoWindows { hwnds: Vec::new() };
    let mut hosts: Vec<SurfaceHost> = Vec::new();
    let mut reports: Vec<HostReport> = Vec::new();
    for (index, monitor) in monitors.iter().enumerate()
    {
        let work_area = PixelRect::new(monitor.work_x, monitor.work_y, monitor.work_x + monitor.work_width as i32, monitor.work_y + monitor.work_height as i32);
        if monitor.dpi == 0
        {
            return Err(Error::new(E_FAIL, "显示器 DPI 无效"));
        }
        let geometry = HostGeometry::for_work_area(work_area, DpiScale::new(monitor.dpi));
        surface_bounds(geometry)?;
        let create_started = Instant::now();
        let hwnd = stage("create_host_window", create_host_window(instance, geometry, &format!("NativeSurface·{}", monitor.name)))?;
        windows.hwnds.push(hwnd);
        let mut host = SurfaceHost::attach(hwnd, geometry, &devices.composition)?;
        let create_ms = create_started.elapsed().as_secs_f64() * 1000.0;
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            // 不抢焦点的弹层默认会落到 z 序底部，演示里显式抬到非 topmost 层顶部，
            // 否则用户窗口会把它整个盖住（正式宿主由产品自己的层级管理接管这一步）。
            stage("SetWindowPos", SetWindowPos(hwnd, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW))?;
        }
        let list = scene(geometry, &monitors[index], index, &family);
        let paint_started = Instant::now();
        let (surface, outcome) = SurfaceHost::prepare(geometry, &mut painter, &devices, &list)?;
        host.publish(geometry, surface, &devices)?;
        let paint_ms = paint_started.elapsed().as_secs_f64() * 1000.0;
        let (width, height) = geometry.surface_size();
        reports.push(HostReport { monitor: monitor.name.clone(), primary: monitor.primary, work_area: format!("{}x{}+{}+{}", work_area.width(), work_area.height(), work_area.left, work_area.top), surface: format!("{width}x{height}"), dpi: monitor.dpi, scale: monitor.scale_factor, create_ms, paint_ms, surface_path: "BeginDraw(IDXGISurface)+CreateBitmapFromDxgiSurface".to_string(), first_offset: format!("({},{})", outcome.offset.x, outcome.offset.y), first_full_update: outcome.full });
        hosts.push(host);
    }
    let create_total_ms = started.elapsed().as_secs_f64() * 1000.0;

    // 保持一段时间，期间泵消息并处理 DPI 变化。
    let deadline = Instant::now() + hold;
    // 已绘制到的取图代次：0 表示保持阶段还没有一帧反映取图结果，首轮先补一帧。
    let mut drawn = 0u64;
    let mut message = MSG::default();
    while Instant::now() < deadline
    {
        unsafe {
            while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool()
            {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        if take_dpi_event()?
        {
            // 先查矩形与 DPI，查询失败就保留旧表面，不拿零矩形重建；成功也只发布完整画好的表面。
            repaint_scene(&mut hosts, &monitors, &scene, &mut painter, &devices, &family)?;
        }
        // 有取图来源：等「结果落定」事件或消息，不轮询；超时只用来守住 hold 截止。
        let remaining = deadline.saturating_duration_since(Instant::now()).as_millis().min(u32::MAX as u128) as u32;
        match thumbs
        {
            Some(thumbs) =>
            {
                // 结果可能在创建宿主期间就已落定（事件已置位），先补一帧再等事件，避免漏掉。
                if thumbs.settled() != drawn
                {
                    repaint_scene(&mut hosts, &monitors, &scene, &mut painter, &devices, &family)?;
                    drawn = thumbs.settled();
                }
                let wake = HANDLE(thumbs.handle() as *mut _);
                unsafe {
                    let _ = MsgWaitForMultipleObjectsEx(Some(&[wake]), remaining.max(1), QS_ALLINPUT, MSG_WAIT_FOR_MULTIPLE_OBJECTS_EX_FLAGS(0));
                }
            }
            None =>
            {
                unsafe {
                    let _ = MsgWaitForMultipleObjectsEx(None, remaining.min(50).max(1), QS_ALLINPUT, MSG_WAIT_FOR_MULTIPLE_OBJECTS_EX_FLAGS(0));
                }
            }
        }
    }

    // 先撤掉合成目标与表面，再销毁窗口；最后才轮到 COM guard。
    drop(hosts);
    drop(windows);
    let windows_after_exit = count_own_windows();

    Ok(DemoReport { pid: std::process::id(), monitors: monitors.len(), font_family: family, dpi_awareness: format!("{awareness_note} / com:{}", apartment.note()), create_total_ms, hosts: reports, windows_after_exit, exit_code: 0 })
}

/// 用同一个场景重画所有屏：先在新表面画，成功后再发布；单屏失败保留旧表面，不中止本次会话。
fn repaint_scene(
    hosts: &mut [SurfaceHost],
    monitors: &[crate::contracts::DesktopHostMonitor],
    scene: &impl Fn(HostGeometry, &crate::contracts::DesktopHostMonitor, usize, &str) -> DrawList,
    painter: &mut Painter,
    devices: &Devices,
    family: &str,
) -> Result<()>
{
    for (index, host) in hosts.iter_mut().enumerate()
    {
        let geometry = match host_geometry(host.hwnd)
        {
            Ok(geometry) => geometry,
            Err(error) =>
            {
                eprintln!("重画前查询窗口几何失败，保留旧表面：{error}");
                continue;
            }
        };
        let list = scene(geometry, &monitors[index], index, family);
        if let Some((surface, _)) = dpi_candidate(SurfaceHost::prepare(geometry, painter, devices, &list))?
        {
            host.publish(geometry, surface, devices)?;
        }
    }
    Ok(())
}

/// 逐帧诊断：按 `steps` 次重建并发布表面，每次由 `frame` 给出该帧绘制列表。
///
/// 与 [`run_scene`] 共用设备、宿主、DPI 处理与收尾；区别只是每帧重新 `prepare + publish`，
/// 让调用方（命中和焦点的诊断）能在同一块真实表面上演示状态变化。
pub(super) fn run_scene_steps(mut frame: impl FnMut(usize, HostGeometry, &crate::contracts::DesktopHostMonitor, usize, &str) -> DrawList, steps: usize, step_hold: Duration) -> Result<DemoReport>
{
    let apartment = ComApartment::enter()?;
    let awareness = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let awareness_note = match awareness
    {
        Ok(()) => "per_monitor_v2".to_string(),
        Err(error) => format!("沿用现有设置（HRESULT=0x{:08X}）", error.code().0 as u32),
    };
    let monitors = crate::platform::windows::enumerate_desktop_monitors().map_err(|message| Error::new(E_FAIL, message))?;
    let instance: HINSTANCE = stage("GetModuleHandleW", unsafe { GetModuleHandleW(None) }).map(|h| HINSTANCE(h.0))?;
    let devices = Devices::new()?;
    let mut painter = stage("Painter::new", Painter::new())?;
    let family = painter.family().to_string();

    let started = Instant::now();
    let mut windows = DemoWindows { hwnds: Vec::new() };
    let mut hosts: Vec<SurfaceHost> = Vec::new();
    let mut reports: Vec<HostReport> = Vec::new();
    for (index, monitor) in monitors.iter().enumerate()
    {
        let work_area = PixelRect::new(monitor.work_x, monitor.work_y, monitor.work_x + monitor.work_width as i32, monitor.work_y + monitor.work_height as i32);
        if monitor.dpi == 0
        {
            return Err(Error::new(E_FAIL, "显示器 DPI 无效"));
        }
        let geometry = HostGeometry::for_work_area(work_area, DpiScale::new(monitor.dpi));
        surface_bounds(geometry)?;
        let create_started = Instant::now();
        let hwnd = stage("create_host_window", create_host_window(instance, geometry, &format!("NativeSurface·{}", monitor.name)))?;
        windows.hwnds.push(hwnd);
        let mut host = SurfaceHost::attach(hwnd, geometry, &devices.composition)?;
        let create_ms = create_started.elapsed().as_secs_f64() * 1000.0;
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            stage("SetWindowPos", SetWindowPos(hwnd, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW))?;
        }
        let list = frame(0, geometry, &monitors[index], index, &family);
        let paint_started = Instant::now();
        let (surface, outcome) = SurfaceHost::prepare(geometry, &mut painter, &devices, &list)?;
        host.publish(geometry, surface, &devices)?;
        let paint_ms = paint_started.elapsed().as_secs_f64() * 1000.0;
        let (width, height) = geometry.surface_size();
        reports.push(HostReport { monitor: monitor.name.clone(), primary: monitor.primary, work_area: format!("{}x{}+{}+{}", work_area.width(), work_area.height(), work_area.left, work_area.top), surface: format!("{width}x{height}"), dpi: monitor.dpi, scale: monitor.scale_factor, create_ms, paint_ms, surface_path: "BeginDraw(IDXGISurface)+CreateBitmapFromDxgiSurface".to_string(), first_offset: format!("({},{})", outcome.offset.x, outcome.offset.y), first_full_update: outcome.full });
        hosts.push(host);
    }
    let create_total_ms = started.elapsed().as_secs_f64() * 1000.0;

    for step in 1..steps
    {
        let mut message = MSG::default();
        let deadline = Instant::now() + step_hold;
        while Instant::now() < deadline
        {
            unsafe {
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool()
                {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            if take_dpi_event()?
            {
                break;
            }
            unsafe {
                let _ = MsgWaitForMultipleObjectsEx(None, 20, QS_ALLINPUT, MSG_WAIT_FOR_MULTIPLE_OBJECTS_EX_FLAGS(0));
            }
        }
        for (index, host) in hosts.iter_mut().enumerate()
        {
            let geometry = host_geometry(host.hwnd)?;
            let list = frame(step, geometry, &monitors[index], index, &family);
            let (surface, _) = SurfaceHost::prepare(geometry, &mut painter, &devices, &list)?;
            host.publish(geometry, surface, &devices)?;
        }
    }

    drop(hosts);
    drop(windows);
    let windows_after_exit = count_own_windows();
    Ok(DemoReport { pid: std::process::id(), monitors: monitors.len(), font_family: family, dpi_awareness: format!("{awareness_note} / com:{}", apartment.note()), create_total_ms, hosts: reports, windows_after_exit, exit_code: 0 })
}

/// 统计本进程仍存活的宿主窗口数量（收尾自检）。
fn count_own_windows() -> usize
{
    let mut count = 0usize;
    unsafe {
        let _ = EnumWindows(Some(count_callback), LPARAM((&mut count as *mut usize) as isize));
    }
    count
}

/// 只统计本进程的宿主窗口类。
unsafe extern "system" fn count_callback(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL
{
    let mut process = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut process));
    }
    if process != std::process::id()
    {
        return windows::core::BOOL(1);
    }
    let mut name = [0u16; 64];
    let length = unsafe { GetClassNameW(hwnd, &mut name) };
    let class = String::from_utf16_lossy(&name[..length.max(0) as usize]);
    if class == "DesktopToolsNativeSurfaceHost"
    {
        let counter = unsafe { &mut *(lparam.0 as *mut usize) };
        *counter += 1;
    }
    windows::core::BOOL(1)
}

#[cfg(test)]
mod host_tests
{
    use super::*;
    /// 在真实 HWND 已创建后注入无效 Explorer 层，验证失败回滚不遗留窗口、不显示输入面。
    #[test]
    fn protected_attach_failure_releases_hidden_window() -> Result<()>
    {
        use crate::platform::windows::{ShellDesktopState, enumerate_desktop_monitors};
        let monitor = enumerate_desktop_monitors().unwrap().remove(0);
        let geometry = HostGeometry::for_work_area(PixelRect::from_xywh(0, 0, 32, 32), DpiScale::new(96));
        let hwnd = create_host_window(HINSTANCE(unsafe { GetModuleHandleW(None) }?.0), geometry, "ProtectedAttachTest")?;
        {
            let _owned = DemoWindows { hwnds: vec![hwnd] };
            assert!(!unsafe { IsWindowVisible(hwnd) }.as_bool());
            let shell = ShellDesktopState { def_view: 0, explorer_host: 0, list_view: 0, original_items_visible: true };
            assert!(shell.attach_hidden(hwnd, &monitor).is_err());
        }
        assert!(!unsafe { IsWindow(Some(hwnd)) }.as_bool());
        Ok(())
    }

    /// 注入设备丢失及双重收尾失败：主循环不得再处理下一块表面。
    #[test]
    fn fatal_dpi_errors_stop_before_next_surface()
    {
        let removed = windows::Win32::Graphics::Dxgi::DXGI_ERROR_DEVICE_REMOVED;
        let recreate = windows::Win32::Foundation::D2DERR_RECREATE_TARGET;
        let bad_content = || FrameFailure::Content(Error::new(E_INVALIDARG, "注入坏内容"));
        assert!(dpi_candidate::<()>(Err(bad_content())).unwrap().is_none());
        let end1 = Error::new(E_FAIL, "D2D EndDraw 注入失败");
        let end2 = Error::new(removed, "DComp EndDraw 注入失败");
        let both = finished::<()>(finished(Err(bad_content()), Err(end1)), Err(end2));
        assert!(matches!(&both, Err(FrameFailure::Stop{primary,cleanup}) if primary.code()==E_INVALIDARG && cleanup.iter().map(Error::code).collect::<Vec<_>>()==[E_FAIL,removed]));
        for bad in [both, Err(Error::new(removed, "device lost").into()), Err(Error::new(recreate, "target lost").into())]
        {
            let mut attempts = 0;
            let result = [bad, Ok(())].into_iter().try_for_each(|candidate| {
                attempts += 1;
                dpi_candidate(candidate).map(|_| ())
            });
            assert!(result.is_err());
            assert_eq!(attempts, 1);
        }
    }
    /// 窗口过程失败后再来成功消息也不会吞掉原始 HWND 和 HRESULT。
    #[test]
    fn dpi_move_failure_reaches_message_pump()
    {
        let hwnd = HWND(0x1234usize as *mut _);
        record_dpi_move(hwnd, Err(Error::new(E_FAIL, "注入 SetWindowPos 失败")));
        record_dpi_move(HWND::default(), Ok(()));
        let error = take_dpi_event().unwrap_err();
        assert_eq!(error.code(), E_FAIL);
        assert!(error.to_string().contains(&format!("HWND={hwnd:?}")));
        assert!(error.to_string().contains("HRESULT=0x80004005"));
        assert!(!take_dpi_event().unwrap());
    }
    #[test]
    fn nonzero_atlas_offset_maps_update_not_surface_origin()
    {
        let (dx, dy, r) = update_mapping(PixelRect::new(24, 48, 72, 96), POINT { x: 120, y: 12 }, DpiScale::new(144));
        assert_eq!((dx, dy), (64., -24.));
        assert_eq!(r, Rect::new(80., 8., 32., 32.));
    }
    #[test]
    fn failed_frame_does_not_poison_next_frame() -> Result<()>
    {
        let _apt = ComApartment::enter()?;
        let devices = Devices::new()?;
        let mut painter = Painter::new()?;
        let make = || unsafe { devices.composition.CreateSurface(64, 64, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_ALPHA_MODE_PREMULTIPLIED) };
        let first = make()?;
        let other = make()?;
        let bounds = PixelRect::new(0, 0, 64, 64);
        let dpi = DpiScale::new(144);
        let good = DrawList::new();
        let mut bad = DrawList::new();
        bad.push(DrawCommand::PushClip { rect: Rect::new(0., 0., 8., 8.) });
        bad.push(DrawCommand::PushOpacity { opacity: 0.5 });
        bad.push(DrawCommand::Text { rect: Rect::new(0., 0., 8., 8.), text: "坏字号".into(), size: -1., weight: 400, color: Color::TRANSPARENT, align: super::super::draw::TextAlign::Start, max_lines: 1 });
        bad.push(DrawCommand::PopOpacity);
        bad.push(DrawCommand::PopClip);
        let err = paint_update(&first, bounds, bounds, dpi, &mut painter, &devices, &bad).unwrap_err();
        assert!(matches!(&err,FrameFailure::Content(e) if e.to_string().contains("Painter::paint")), "{err:?}");
        paint_update(&first, bounds, bounds, dpi, &mut painter, &devices, &good)?;
        paint_update(&other, bounds, bounds, dpi, &mut painter, &devices, &good)?;
        paint_update(&first, bounds, PixelRect::new(12, 6, 36, 24), dpi, &mut painter, &devices, &good)?;
        Ok(())
    }
}
