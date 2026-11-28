//! 桌面层右键菜单的原生弹层（诊断期实现，不接真实桌面）。
//!
//! 结构：一个 `WS_POPUP` 弹层窗口（`WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP`），
//! 置顶显示、不抢焦点；表面用 DirectComposition + Direct2D 自绘（圆角 6 DIP、1 DIP 描边、
//! 容器内距 4 DIP、行高 28 DIP、左侧 16 DIP 图标列），配色取偏好里的强调色与前景色。
//!
//! 与 ADR-0040 的关系：ADR-0040 的正式弹层**需要键盘**，因此不设 `WS_EX_NOACTIVATE`；
//! 本诊断按任务要求用非激活窗口，键盘靠定向消息投递到菜单窗口（与 t44/t45 验证过的消息链路同源）。
//! 这条差异写在 `20-原生桌面菜单.md` 里，不当作正式方案。

use std::sync::OnceLock;

use windows::{
    Win32::{
        Foundation::{E_FAIL, ERROR_SUCCESS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::ValidateRect,
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            HiDpi::*,
            WindowsAndMessaging::*,
            Input::KeyboardAndMouse::*,
        },
    },
    core::{w, Error, HSTRING, Result, PCWSTR},
};

use super::{
    draw::{Color, DrawCommand, DrawList, Rect, TextAlign},
    geometry::{DpiScale, HostGeometry, PixelRect},
    host::{ComApartment, Devices, SurfaceHost},
    painter::Painter,
};

/// 菜单样式：数值全部取自 `docs/design/视觉规范.md`（菜单宽 232、圆角 6、行高 28、内距 4、图标 16、正文 13）。
#[derive(Clone, Copy, Debug)]
pub struct MenuStyle
{
    pub width: f32,
    pub radius: f32,
    pub padding: f32,
    pub item_height: f32,
    pub font_size: f32,
    pub icon_size: f32,
    pub icon_column: f32,
    pub separator_height: f32,
}

impl Default for MenuStyle
{
    fn default() -> Self
    {
        Self { width: 232.0, radius: 6.0, padding: 4.0, item_height: 28.0, font_size: 13.0, icon_size: 16.0, icon_column: 24.0, separator_height: 9.0 }
    }
}

/// 一个菜单条目：项或分组分隔线。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuEntry
{
    Item { id: String, label: String, enabled: bool, danger: bool },
    Separator,
}

impl MenuEntry
{
    /// 便捷构造：可点击项。
    pub fn item(id: &str, label: &str) -> Self
    {
        Self::Item { id: id.to_string(), label: label.to_string(), enabled: true, danger: false }
    }

    /// 便捷构造：危险项（使用 `--color-error`）。
    pub fn danger(id: &str, label: &str) -> Self
    {
        Self::Item { id: id.to_string(), label: label.to_string(), enabled: true, danger: true }
    }
}

/// 一行（项或分隔线）在菜单内的相对矩形（菜单局部 DIP）。
#[derive(Clone, Debug, PartialEq)]
pub struct MenuRow
{
    /// 条目下标。
    pub index: usize,
    /// 相对菜单左上角的矩形。
    pub rect: Rect,
}

/// 菜单布局结果。
#[derive(Clone, Debug, PartialEq)]
pub struct MenuLayout
{
    /// 菜单在屏幕上的矩形（工作区局部 DIP，含窗口原点）。
    pub rect: Rect,
    /// 每行相对菜单的矩形。
    pub rows: Vec<MenuRow>,
    /// 是否向左展开（右侧空间不足）。
    pub flipped_x: bool,
    /// 是否向上展开（底部空间不足）。
    pub flipped_y: bool,
    /// 是否被整体推回工作区。
    pub pushed: bool,
    /// 因为工作区放不下而没有绘制的行数（这些行不参与命中与键盘候选）。
    pub hidden_rows: usize,
}

/// 一行的实际高度。
fn row_height(entry: &MenuEntry, style: &MenuStyle) -> f32
{
    match entry
    {
        MenuEntry::Item { .. } => style.item_height,
        MenuEntry::Separator => style.separator_height,
    }
}

/// 计算菜单尺寸与位置：贴触发格子的右侧向下展开，放不下就反向，最后整体推回工作区。
///
/// 工作区容不下「内距 ×2 + 至少一行」或宽度小于「图标列 + 内距 ×2 + 最小文字区」时**拒绝**，
/// 不做负数起点的硬算（`work` 太小时 `.max(28)` 这类兜底会把起点推到工作区外）。
/// 高度被工作区截断时，超出的行不绘制、也不进入命中与键盘候选。
pub fn layout(anchor: Rect, entries: &[MenuEntry], style: &MenuStyle, work: Rect) -> std::result::Result<MenuLayout, String>
{
    // 最小可用宽度：图标列 + 内距×2 + 至少 80 DIP 文字区；最小高度：内距×2 + 一行。
    let minimum_width = style.icon_column + style.padding * 2.0 + 80.0;
    let minimum_height = style.padding * 2.0 + style.item_height;
    if work.width < minimum_width || work.height < minimum_height
    {
        return Err(format!("工作区 {}x{} DIP 太小，放不下一个菜单（至少需要 {minimum_width:.0}x{minimum_height:.0}）", work.width, work.height));
    }
    let width = style.width.min(work.width);
    let full_height: f32 = entries.iter().map(|entry| row_height(entry, style)).sum::<f32>() + style.padding * 2.0;
    let height = full_height.min(work.height);
    // 右侧放不下 → 向左展开（贴格子左边），仍放不下再推回工作区。
    let mut flipped_x = false;
    let mut x = anchor.x + anchor.width + 2.0;
    if x + width > work.right()
    {
        x = anchor.x - width - 2.0;
        flipped_x = true;
    }
    // 底部放不下 → 向上展开（贴格子底边）。
    let mut flipped_y = false;
    let mut y = anchor.y;
    if y + height > work.bottom()
    {
        y = anchor.bottom() - height;
        flipped_y = true;
    }
    let unclamped = (x, y);
    x = x.max(work.x).min(work.right() - width);
    y = y.max(work.y).min(work.bottom() - height);
    let pushed = (x, y) != unclamped;
    let rect = Rect::new(x, y, width, height);
    // 只排能完整放进菜单高度的行；被截断的行记在 hidden_rows，永不参与命中与键盘候选。
    let available = (height - style.padding * 2.0).max(0.0);
    let mut rows = Vec::new();
    let mut hidden_rows = 0usize;
    let mut cursor = style.padding;
    for (index, entry) in entries.iter().enumerate()
    {
        let item_height = row_height(entry, style);
        if cursor + item_height > style.padding + available
        {
            hidden_rows += 1;
            continue;
        }
        rows.push(MenuRow { index, rect: Rect::new(style.padding, cursor, (width - style.padding * 2.0).max(0.0), item_height) });
        cursor += item_height;
    }
    Ok(MenuLayout { rect, rows, flipped_x, flipped_y, pushed, hidden_rows })
}

/// 菜单交互事件。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuEvent
{
    /// 高亮变化（旧下标 → 新下标）。
    Highlight(Option<usize>, usize),
    /// 触发某一项。
    Activated(String),
    /// 关闭（原因）。
    Closed(&'static str),
}

/// 菜单状态机：高亮、按键、鼠标、关闭。
#[derive(Clone, Debug)]
pub struct MenuState
{
    pub entries: Vec<MenuEntry>,
    pub layout: MenuLayout,
    highlight: Option<usize>,
}

impl MenuState
{
    /// 打开菜单。
    pub fn open(entries: Vec<MenuEntry>, anchor: Rect, style: &MenuStyle, work: Rect) -> std::result::Result<Self, String>
    {
        let layout = layout(anchor, &entries, style, work)?;
        let state = Self { entries, layout, highlight: None };
        let first = state.focusable().first().copied();
        Ok(Self { highlight: first, ..state })
    }

    /// 当前高亮项。
    pub fn highlight(&self) -> Option<usize>
    {
        self.highlight
    }

    /// 当前高亮项的 id。
    pub fn highlighted_id(&self) -> Option<&str>
    {
        match self.highlight.map(|index| &self.entries[index])
        {
            Some(MenuEntry::Item { id, .. }) => Some(id.as_str()),
            _ => None,
        }
    }

    /// 可聚焦项下标（跳过禁用项与分隔线）。
    fn focusable(&self) -> Vec<usize>
    {
        // 只含**实际绘制出来**的行：工作区装不下的行既不可见，也不能被键盘选中。
        self.layout
            .rows
            .iter()
            .map(|row| row.index)
            .filter(|index| matches!(self.entries[*index], MenuEntry::Item { enabled: true, .. }))
            .collect()
    }

    /// 方向键/确认/取消。返回产生的事件（`None` 表示状态没变）。
    pub fn key(&mut self, virtual_key: u16) -> Option<MenuEvent>
    {
        let focusable = self.focusable();
        if focusable.is_empty()
        {
            return None;
        }
        match virtual_key
        {
            0x26 | 0x28 =>
            {
                let position = self.highlight.and_then(|index| focusable.iter().position(|value| *value == index));
                let next = match (virtual_key, position)
                {
                    (0x26, Some(0)) => focusable.len() - 1, // VK_UP 首项回绕到末项。
                    (0x26, Some(value)) => value - 1,
                    (0x26, None) => focusable.len() - 1,
                    (_, Some(value)) => (value + 1) % focusable.len(),
                    (_, None) => 0,
                };
                let before = self.highlight;
                self.highlight = Some(focusable[next]);
                Some(MenuEvent::Highlight(before, focusable[next]))
            }
            0x0D =>
            {
                let index = self.highlight?;
                match &self.entries[index]
                {
                    MenuEntry::Item { id, enabled: true, .. } => Some(MenuEvent::Activated(id.clone())),
                    _ => None,
                }
            }
            0x1B => Some(MenuEvent::Closed("Escape")),
            _ => None,
        }
    }

    /// 鼠标悬停：传入菜单局部 DIP 坐标。
    pub fn hover(&mut self, x: f32, y: f32) -> Option<MenuEvent>
    {
        let index = self.row_at(x, y)?;
        if !matches!(self.entries[index], MenuEntry::Item { enabled: true, .. })
        {
            return None;
        }
        if self.highlight == Some(index)
        {
            return None;
        }
        let before = self.highlight;
        self.highlight = Some(index);
        Some(MenuEvent::Highlight(before, index))
    }

    /// 鼠标点击：命中有项返回触发事件；点在菜单内空白处返回 `None`；点在菜单外由调用方关闭。
    pub fn click(&mut self, x: f32, y: f32) -> Option<MenuEvent>
    {
        let index = self.row_at(x, y)?;
        match &self.entries[index]
        {
            MenuEntry::Item { id, enabled: true, .. } => Some(MenuEvent::Activated(id.clone())),
            _ => None,
        }
    }

    /// 点命中哪一行（菜单局部 DIP）。
    pub fn row_at(&self, x: f32, y: f32) -> Option<usize>
    {
        self.layout.rows.iter().find(|row| row.rect.contains(x, y)).map(|row| row.index)
    }

    /// 菜单局部点是否在菜单内。
    pub fn contains(&self, x: f32, y: f32) -> bool
    {
        x >= 0.0 && y >= 0.0 && x < self.layout.rect.width && y < self.layout.rect.height
    }

    /// 生成绘制列表（菜单局部坐标）。
    pub fn draw(&self, style: &MenuStyle, dark: bool, accent: Color) -> DrawList
    {
        let mut list = DrawList::new();
        let surface = if dark { Color::rgba(0x20, 0x26, 0x28, 0xF2) } else { Color::rgba(0xB9, 0xB9, 0xB9, 0xF2) };
        let border = if dark { Color::rgba(0xD2, 0xE0, 0xE3, 0x3D) } else { Color::rgba(0x37, 0x42, 0x45, 0x2E) };
        let text = if dark { Color::rgba(0xE8, 0xEF, 0xF0, 0xFF) } else { Color::rgba(0x20, 0x25, 0x26, 0xFF) };
        let muted = if dark { Color::rgba(0xA8, 0xB4, 0xB6, 0xFF) } else { Color::rgba(0x5C, 0x66, 0x68, 0xFF) };
        let danger = if dark { Color::rgba(0xF0, 0x8A, 0x80, 0xFF) } else { Color::rgba(0xA3, 0x21, 0x18, 0xFF) };
        let rect = Rect::new(0.0, 0.0, self.layout.rect.width, self.layout.rect.height);
        list.push(DrawCommand::FillRoundRect { rect, radius: style.radius, color: surface });
        list.push(DrawCommand::StrokeRoundRect { rect: Rect::new(0.5, 0.5, rect.width - 1.0, rect.height - 1.0), radius: (style.radius - 0.5).max(0.0), width: 1.0, color: border });
        for row in &self.layout.rows
        {
            match &self.entries[row.index]
            {
                MenuEntry::Separator =>
                {
                    let y = row.rect.y + row.rect.height / 2.0;
                    list.push(DrawCommand::FillRoundRect { rect: Rect::new(row.rect.x + 4.0, y, (row.rect.width - 8.0).max(0.0), 1.0), radius: 0.0, color: border });
                }
                MenuEntry::Item { label, enabled, danger: is_danger, .. } =>
                {
                    let highlighted = self.highlight == Some(row.index);
                    if highlighted && *enabled
                    {
                        // 选中/悬停：强调色 18% 底（视觉规范「选中使用强调色 18% 底色及边框」）。
                        let wash = accent.with_alpha(0.18);
                        list.push(DrawCommand::FillRoundRect { rect: row.rect, radius: 4.0, color: wash });
                    }
                    // 左侧固定图标列：16 DIP 图标 + 24 DIP 列宽（规范取值），图标本体先用占位方块。
                    let icon = Rect::new(row.rect.x + (style.icon_column - style.icon_size) / 2.0, row.rect.y + (style.item_height - style.icon_size) / 2.0, style.icon_size, style.icon_size);
                    list.push(DrawCommand::StrokeRoundRect { rect: icon, radius: 3.0, width: 1.0, color: if *enabled { muted } else { muted.with_alpha(0.38) } });
                    let color = match (*enabled, *is_danger)
                    {
                        (false, _) => muted.with_alpha(0.38),
                        (true, true) => danger,
                        (true, false) => text,
                    };
                    let label_x = row.rect.x + style.icon_column + 4.0;
                    list.push(DrawCommand::Text {
                        rect: Rect::new(label_x, row.rect.y + (style.item_height - style.font_size * 1.5).max(0.0) / 2.0, (row.rect.width - style.icon_column - 8.0).max(0.0), style.item_height),
                        text: label.clone(),
                        size: style.font_size,
                        weight: 400,
                        color,
                        align: TextAlign::Start,
                        max_lines: 1,
                    });
                }
            }
        }
        list
    }
}


/// 菜单（屏幕坐标，物理像素）是否包含某个屏幕点：左闭右开，与命中矩形同一口径。
pub fn menu_contains_screen_point(menu: RECT, x: i32, y: i32) -> bool
{
    x >= menu.left && x < menu.right && y >= menu.top && y < menu.bottom
}

/// owner 客户区点 → 屏幕点（纯函数，便于用非零原点做回归）。
pub fn client_to_screen_point(owner_origin: (i32, i32), client: (i32, i32)) -> (i32, i32)
{
    (owner_origin.0 + client.0, owner_origin.1 + client.1)
}

/// 菜单的隔离 owner：菜单归它所有；它收到「菜单矩形外的指针点击」或自身失焦时请求关闭菜单。
///
/// 非激活弹层收不到窗口外的点击（点击落在别的窗口上），所以「外部点击关闭」只能由 owner 判定；
/// 这里用隐藏 owner + 定向消息演示这条路径，日志里明确标为合成消息，不冒充物理输入。
struct OwnerRuntime
{
    menu: HWND,
    close_requested: bool,
    events: Vec<String>,
}

const OWNER_CLASS: PCWSTR = w!("DesktopToolsMenuOwner");

unsafe extern "system" fn owner_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT
{
    let runtime = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut OwnerRuntime;
    if runtime.is_null()
    {
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }
    let runtime = unsafe { &mut *runtime };
    match message
    {
        WM_LBUTTONDOWN | WM_RBUTTONDOWN =>
        {
            // 协议：owner 消息里的坐标是**客户区**坐标，必须先用 ClientToScreen 换成屏幕坐标，
            // 再与菜单的屏幕矩形（GetWindowRect）比较；两者绝不可直接比。
            let client = POINT { x: (lparam.0 & 0xFFFF) as i16 as i32, y: ((lparam.0 >> 16) & 0xFFFF) as i16 as i32 };
            let mut screen = client;
            let mut owner_rect = RECT::default();
            unsafe {
                let _ = windows::Win32::Graphics::Gdi::ClientToScreen(hwnd, &mut screen);
                let _ = GetWindowRect(hwnd, &mut owner_rect);
            }
            let mut rect = RECT::default();
            let inside = unsafe { GetWindowRect(runtime.menu, &mut rect) }.is_ok() && menu_contains_screen_point(rect, screen.x, screen.y);
            runtime.events.push(format!("[owner] 合成点击 client=({},{}) screen=({},{}) owner_origin=({},{}) -> 菜单矩形内={inside}", client.x, client.y, screen.x, screen.y, owner_rect.left, owner_rect.top));
            if !inside
            {
                runtime.close_requested = true;
                runtime.events.push("[owner] 点击落在菜单矩形外 -> 请求关闭菜单".to_string());
            }
        }
        WM_KILLFOCUS =>
        {
            runtime.close_requested = true;
            runtime.events.push("[owner] 自身失焦 -> 请求关闭菜单".to_string());
        }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

/// 注册 owner 窗口类（进程内一次）。
fn register_owner_class(instance: HINSTANCE) -> Result<()>
{
    static REGISTERED: OnceLock<std::result::Result<(), String>> = OnceLock::new();
    let outcome = REGISTERED.get_or_init(|| {
        let class = WNDCLASSEXW { cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32, lpfnWndProc: Some(owner_proc), hInstance: instance, lpszClassName: OWNER_CLASS, ..Default::default() };
        match unsafe { RegisterClassExW(&class) } == 0
        {
            true => Err("RegisterClassExW(owner) 失败".to_string()),
            false => Ok(()),
        }
    });
    match outcome
    {
        Ok(()) => Ok(()),
        Err(message) => Err(Error::new(E_FAIL, message.clone())),
    }
}

/// 建窗/附着/显示失败时的回收器（RAII）：**同时**托管两个 HWND 与两处运行时 Box。
///
/// drop 顺序写死为「清 USERDATA → 释放 Box → 销毁窗口」，原因：`DestroyWindow` 会重入本进程的 WndProc，
/// 若先销毁窗口或先释放 Box，WndProc 可能通过 `GWLP_USERDATA` 读到已释放的运行时；先把 USERDATA 清空，
/// 重入时拿到的是空指针，会走安全分支。
struct WindowGuard
{
    owner: HWND,
    menu: HWND,
    owner_runtime: *mut OwnerRuntime,
    menu_runtime: *mut MenuRuntime,
    armed: bool,
}

impl WindowGuard
{
    fn new(owner: HWND, owner_runtime: *mut OwnerRuntime) -> Self
    {
        Self { owner, menu: HWND::default(), owner_runtime, menu_runtime: std::ptr::null_mut(), armed: true }
    }
}

impl Drop for WindowGuard
{
    fn drop(&mut self)
    {
        if !self.armed
        {
            return;
        }
        unsafe {
            if !self.menu.is_invalid()
            {
                SetWindowLongPtrW(self.menu, GWLP_USERDATA, 0);
            }
            if !self.owner.is_invalid()
            {
                SetWindowLongPtrW(self.owner, GWLP_USERDATA, 0);
            }
            if !self.menu_runtime.is_null()
            {
                drop(Box::from_raw(self.menu_runtime));
                self.menu_runtime = std::ptr::null_mut();
            }
            if !self.owner_runtime.is_null()
            {
                drop(Box::from_raw(self.owner_runtime));
                self.owner_runtime = std::ptr::null_mut();
            }
            if !self.menu.is_invalid() && IsWindow(Some(self.menu)).as_bool()
            {
                let _ = DestroyWindow(self.menu);
            }
            if !self.owner.is_invalid() && IsWindow(Some(self.owner)).as_bool()
            {
                let _ = DestroyWindow(self.owner);
            }
            self.menu = HWND::default();
            self.owner = HWND::default();
        }
    }
}

/// 运行时存活计数：只用于证明失败路径把 Box 真的释放了（建一个 +1、Drop 一次 -1）。
static LIVE_RUNTIMES: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// 当前仍存活的菜单/owner 运行时数量（测试与诊断读）。
pub fn live_runtimes() -> isize
{
    LIVE_RUNTIMES.load(std::sync::atomic::Ordering::SeqCst)
}

impl Drop for MenuRuntime
{
    fn drop(&mut self)
    {
        LIVE_RUNTIMES.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Drop for OwnerRuntime
{
    fn drop(&mut self)
    {
        LIVE_RUNTIMES.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// 打开过程中的故障注入点（只用于诊断/测试，证明失败路径会回收资源）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenFault
{
    /// 正常运行。
    None,
    /// 建窗成功后让附着之后的一步失败：模拟 `SetWindowPos` 返回错误。
    Show,
}

/// 菜单窗口运行时：WndProc 通过 `GWLP_USERDATA` 拿到它，用它重绘并记录事件。
pub struct MenuRuntime
{
    pub state: MenuState,
    /// 关闭请求（由 WndProc 置位，真正销毁交给消息泵之后的 `MenuWindow::pump`）。
    pub close_requested: bool,
    pub close_reason: Option<String>,
    pub style: MenuStyle,
    pub dark: bool,
    pub accent: Color,
    pub host: SurfaceHost,
    pub painter: *mut Painter,
    pub devices: *mut Devices,
    pub geometry: HostGeometry,
    pub events: Vec<String>,
    pub closed: bool,
}

const MENU_CLASS: PCWSTR = w!("DesktopToolsNativeMenu");

/// 菜单弹层窗口：创建、显示、按状态重绘、销毁。
pub struct MenuWindow
{
    pub hwnd: HWND,
    runtime: *mut MenuRuntime,
    /// 隔离 owner 窗口及其运行时（外部点击/失焦的关闭入口）。
    pub owner: HWND,
    owner_runtime: *mut OwnerRuntime,
}

impl MenuWindow
{
    /// 在给定屏幕矩形上创建置顶非激活弹层，并把第一帧画出来。
    ///
    /// 资源边界：owner/菜单的 HWND 与两处运行时 Box 从创建那一刻就交给 `WindowGuard`；
    /// 之后 `SurfaceHost::attach`、`SetWindowPos(SWP_SHOWWINDOW)`、可见性后验任一步失败，
    /// guard 都会把 Box 与窗口一起回收，不留半建 HWND，也不漏 Box。
    pub fn open(initial: MenuState, style: MenuStyle, dark: bool, accent: Color, painter: &mut Painter, devices: &mut Devices, work_area: PixelRect, dpi: DpiScale) -> Result<Self>
    {
        Self::open_with_fault(initial, style, dark, accent, painter, devices, work_area, dpi, OpenFault::None)
    }

    /// 带故障注入的打开（诊断/测试用）：`OpenFault::Show` 会在窗口建好之后让显示这一步失败。
    #[allow(clippy::too_many_arguments)]
    pub fn open_with_fault(initial: MenuState, style: MenuStyle, dark: bool, accent: Color, painter: &mut Painter, devices: &mut Devices, work_area: PixelRect, dpi: DpiScale, fault: OpenFault) -> Result<Self>
    {
        let instance: HINSTANCE = unsafe { GetModuleHandleW(None) }.map(|value| HINSTANCE(value.0))?;
        register_menu_class(instance)?;
        register_owner_class(instance)?;
        // owner 先建：菜单必须归它所有（外部点击/失焦由 owner 判定）。
        let owner = unsafe { CreateWindowExW(WINDOW_EX_STYLE(0), OWNER_CLASS, &HSTRING::from("菜单 owner"), WS_POPUP, 0, 0, 0, 0, None, None, Some(instance), None) }?;
        let owner_runtime = Box::into_raw(Box::new(OwnerRuntime { menu: HWND::default(), close_requested: false, events: Vec::new() }));
        LIVE_RUNTIMES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        unsafe {
            SetWindowLongPtrW(owner, GWLP_USERDATA, owner_runtime as isize);
        }
        // guard 在**任何可失败调用之前**接管：owner 一定被回收。
        let mut guard = WindowGuard::new(owner, owner_runtime);
        // 菜单几何按 DIP → 物理像素换算，窗口矩形取菜单自身大小。
        let x = work_area.left + (initial.layout.rect.x * dpi.scale()).round() as i32;
        let y = work_area.top + (initial.layout.rect.y * dpi.scale()).round() as i32;
        let width = (initial.layout.rect.width * dpi.scale()).round().max(1.0) as i32;
        let height = (initial.layout.rect.height * dpi.scale()).round().max(1.0) as i32;
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP,
                MENU_CLASS,
                &HSTRING::from("原生菜单"),
                WS_POPUP,
                x,
                y,
                width,
                height,
                Some(owner),
                None,
                Some(instance),
                None,
            )
        }?;
        guard.menu = hwnd;
        let geometry = HostGeometry::for_work_area(PixelRect::new(x, y, x + width, y + height), dpi);
        let host = SurfaceHost::attach(hwnd, geometry, &devices.composition)?; // 失败 → guard 回收窗口与 owner
        let runtime = Box::into_raw(Box::new(MenuRuntime { state: initial, close_requested: false, close_reason: None, style, dark, accent, host, painter: painter as *mut Painter, devices: devices as *mut Devices, geometry, events: Vec::new(), closed: false }));
        LIVE_RUNTIMES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        guard.menu_runtime = runtime;
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, runtime as isize);
            (*owner_runtime).menu = hwnd;
        }
        if fault == OpenFault::Show
        {
            return Err(Error::new(E_FAIL, "注入故障：显示阶段失败（用于验证资源回收）"));
        }
        // 显示用 SetWindowPos(SWP_SHOWWINDOW)：它的返回值带 GetLastError，能真正判失败；
        // 不用 ShowWindow 的返回值判断——那个值表示「此前是否可见」，新建隐藏窗首次显示正常也返回 0。
        unsafe {
            SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW).map_err(|error| Error::new(E_FAIL, format!("SetWindowPos(TOPMOST|SHOWWINDOW) 失败：{error}")))?;
        }
        if !unsafe { IsWindowVisible(hwnd).as_bool() }
        {
            return Err(Error::new(E_FAIL, "窗口显示后仍不可见（IsWindowVisible=false）"));
        }
        guard.armed = false; // 成功交接：后续由 MenuWindow 负责回收
        let window = Self { hwnd, runtime, owner, owner_runtime };
        window.repaint()?;
        Ok(window)
    }

    /// 重绘当前状态（状态变化后调用）。
    pub fn repaint(&self) -> Result<()>
    {
        let runtime = unsafe { &mut *self.runtime };
        let list = runtime.state.draw(&runtime.style, runtime.dark, runtime.accent);
        let painter = unsafe { &mut *runtime.painter };
        let devices = unsafe { &mut *runtime.devices };
        let (surface, _) = SurfaceHost::prepare(runtime.geometry, painter, devices, &list)?;
        runtime.host.publish(runtime.geometry, surface, devices)?;
        Ok(())
    }

    /// 事件日志（WndProc 里记录）。
    pub fn events(&self) -> Vec<String>
    {
        unsafe { (*self.runtime).events.clone() }
    }

    /// 只泵消息：把投递进来的按键/鼠标交给 WndProc（不销毁窗口）。
    pub fn pump(&self)
    {
        let mut message = MSG::default();
        for _ in 0..64
        {
            let mut dispatched = false;
            unsafe {
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool()
                {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                    dispatched = true;
                }
            }
            if !dispatched
            {
                break;
            }
        }
    }

    /// WndProc 请求的关闭原因（`None` 表示菜单还开着）。
    pub fn close_reason(&self) -> Option<String>
    {
        unsafe { (*self.runtime).close_reason.clone() }
    }

    /// owner 记录的事件（外部点击/失焦）。
    pub fn owner_events(&self) -> Vec<String>
    {
        if self.owner_runtime.is_null()
        {
            return Vec::new();
        }
        unsafe { (*self.owner_runtime).events.clone() }
    }

    /// owner 是否请求关闭菜单。
    pub fn owner_close_requested(&self) -> bool
    {
        !self.owner_runtime.is_null() && unsafe { (*self.owner_runtime).close_requested }
    }

    /// 取出并清空至今记录的事件（便于按时间顺序写日志，不必等窗口关闭）。
    pub fn take_events(&self) -> Vec<String>
    {
        unsafe { std::mem::take(&mut (*self.runtime).events) }
    }

    /// 当前菜单状态（供诊断读取高亮与布局）。
    pub fn state(&self) -> &MenuState
    {
        unsafe { &(*self.runtime).state }
    }

    /// 关闭并销毁窗口，释放运行时。
    pub fn close(mut self) -> Vec<String>
    {
        let mut events = self.events();
        events.extend(unsafe { (*self.owner_runtime).events.clone() });
        // 先让所有 &mut 借用结束，再销毁窗口（DestroyWindow 会重入 WndProc）。
        let runtime = self.runtime;
        let owner = self.owner;
        let owner_runtime = self.owner_runtime;
        self.runtime = std::ptr::null_mut();
        self.owner_runtime = std::ptr::null_mut();
        unsafe {
            let _ = DestroyWindow(self.hwnd);
            drop(Box::from_raw(runtime));
            let _ = DestroyWindow(owner);
            drop(Box::from_raw(owner_runtime));
        }
        events
    }
}

impl Drop for MenuWindow
{
    fn drop(&mut self)
    {
        if !self.runtime.is_null()
        {
            let runtime = self.runtime;
            let owner = self.owner;
            let owner_runtime = self.owner_runtime;
            self.runtime = std::ptr::null_mut();
            self.owner_runtime = std::ptr::null_mut();
            unsafe {
                let _ = DestroyWindow(self.hwnd);
                drop(Box::from_raw(runtime));
                if !owner.is_invalid()
                {
                    let _ = DestroyWindow(owner);
                }
                if !owner_runtime.is_null()
                {
                    drop(Box::from_raw(owner_runtime));
                }
            }
        }
    }
}

/// 菜单窗口过程：方向键/回车/Esc、鼠标悬停与点击、失活即关闭。
unsafe extern "system" fn menu_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT
{
    let runtime = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut MenuRuntime;
    if runtime.is_null()
    {
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    }
    let runtime = unsafe { &mut *runtime };
    match message
    {
        WM_KEYDOWN =>
        {
            let key = wparam.0 as u16;
            match runtime.state.key(key)
            {
                Some(MenuEvent::Activated(id)) =>
                {
                    runtime.events.push(format!("key {key:#06x} -> activated {id}"));
                    request_close(runtime, "Enter 触发");
                }
                Some(MenuEvent::Closed(reason)) =>
                {
                    runtime.events.push(format!("key {key:#06x} -> closed {reason}"));
                    request_close(runtime, reason);
                }
                Some(MenuEvent::Highlight(_, next)) =>
                {
                    runtime.events.push(format!("key {key:#06x} -> highlight {}", runtime.state.highlighted_id().unwrap_or("-")));
                    let _ = next;
                    repaint_runtime(runtime);
                }
                None => {}
            }
            return LRESULT(0);
        }
        WM_MOUSEMOVE =>
        {
            let (x, y) = cursor_in_menu(hwnd, lparam);
            if let Some(MenuEvent::Highlight(_, _)) = runtime.state.hover(x, y)
            {
                runtime.events.push(format!("hover ({x:.1},{y:.1}) -> highlight {}", runtime.state.highlighted_id().unwrap_or("-")));
                repaint_runtime(runtime);
            }
            return LRESULT(0);
        }
        WM_LBUTTONDOWN =>
        {
            let (x, y) = cursor_in_menu(hwnd, lparam);
            match runtime.state.click(x, y)
            {
                Some(MenuEvent::Activated(id)) =>
                {
                    runtime.events.push(format!("click ({x:.1},{y:.1}) -> activated {id}"));
                    request_close(runtime, "菜单项点击");
                }
                _ =>
                {
                    runtime.events.push(format!("click ({x:.1},{y:.1}) -> no item"));
                    request_close(runtime, "菜单内非条目区点击");
                }
            }
            return LRESULT(0);
        }
        WM_ACTIVATE =>
        {
            // 只有 WA_INACTIVE（低 16 位 == 0）才代表本窗口失去激活，才允许关闭。
            let code = (wparam.0 & 0xFFFF) as u32;
            if activation_should_close(code)
            {
                runtime.events.push(format!("[menu] WM_ACTIVATE WA_INACTIVE(0x{code:04X}) -> close"));
                request_close(runtime, "失活(WA_INACTIVE)");
            }
            else
            {
                runtime.events.push(format!("[menu] WM_ACTIVATE 0x{code:04X} -> 不关闭"));
            }
            return LRESULT(0);
        }
        WM_KILLFOCUS | WM_CAPTURECHANGED =>
        {
            runtime.events.push(format!("[menu] message {message:#06x} -> close 焦点移出"));
            request_close(runtime, "焦点移出");
            return LRESULT(0);
        }
        WM_PAINT =>
        {
            unsafe {
                let _ = ValidateRect(Some(hwnd), None);
            }
            return LRESULT(0);
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_DESTROY =>
        {
            runtime.closed = true;
            runtime.events.push("[menu] WM_DESTROY".to_string());
            return LRESULT(0);
        }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

/// 客户端坐标（物理像素）→ 菜单局部 DIP。
fn cursor_in_menu(hwnd: HWND, lparam: LPARAM) -> (f32, f32)
{
    // WM_MOUSEMOVE / WM_LBUTTONDOWN 的 lparam 已经是客户端坐标（物理像素），只需按 DPI 换成 DIP。
    let x = (lparam.0 & 0xFFFF) as i16 as i32;
    let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96) as f32 / 96.0;
    (x as f32 / dpi, y as f32 / dpi)
}

/// 重绘（忽略错误：状态变化不该把进程带崩）。
fn repaint_runtime(runtime: &mut MenuRuntime)
{
    let list = runtime.state.draw(&runtime.style, runtime.dark, runtime.accent);
    let painter = unsafe { &mut *runtime.painter };
    let devices = unsafe { &mut *runtime.devices };
    if let Ok((surface, _)) = SurfaceHost::prepare(runtime.geometry, painter, devices, &list)
    {
        let _ = runtime.host.publish(runtime.geometry, surface, devices);
    }
}

/// 请求关闭：只在运行时上置位，真正 `DestroyWindow` 由 `MenuWindow::pump` 在借用结束后执行
/// （`DestroyWindow` 会重入本 WndProc，不能在还持有 `&mut MenuRuntime` 时调用）。
fn request_close(runtime: &mut MenuRuntime, reason: &str)
{
    if !runtime.close_requested
    {
        runtime.close_requested = true;
        runtime.close_reason = Some(reason.to_string());
        runtime.events.push(format!("[menu] close requested: {reason}"));
    }
}

/// 激活消息是否应关闭菜单：只有 `WA_INACTIVE`(0) 是失活；`WA_ACTIVE`(1)/`WA_CLICKACTIVE`(2) 都不关。
pub fn activation_should_close(code: u32) -> bool
{
    code == WA_INACTIVE
}

/// 注册菜单窗口类（进程内一次）。
fn register_menu_class(instance: HINSTANCE) -> Result<()>
{
    static REGISTERED: OnceLock<std::result::Result<(), String>> = OnceLock::new();
    let outcome = REGISTERED.get_or_init(|| {
        let class = WNDCLASSEXW { cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32, style: CS_HREDRAW | CS_VREDRAW, lpfnWndProc: Some(menu_proc), hInstance: instance, hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(), lpszClassName: MENU_CLASS, ..Default::default() };
        match unsafe { RegisterClassExW(&class) } == 0
        {
            true => Err("RegisterClassExW 失败".to_string()),
            false => Ok(()),
        }
    });
    match outcome
    {
        Ok(()) => Ok(()),
        Err(message) => Err(Error::new(E_FAIL, message.clone())),
    }
}

/// 读菜单窗口的 EXSTYLE 与 STYLE 实际位（取证用）。
pub fn window_styles(hwnd: HWND) -> (i64, i64)
{
    unsafe { (GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as i64, GetWindowLongPtrW(hwnd, GWL_STYLE) as i64) }
}

#[cfg(test)]
mod tests
{
    use super::*;

    fn entries() -> Vec<MenuEntry>
    {
        vec![MenuEntry::item("detail", "查看详情"), MenuEntry::item("api", "切换 API"), MenuEntry::Separator, MenuEntry::item("pin", "固定显示模式"), MenuEntry::danger("remove", "移除")]
    }

    /// 布局：行高 28、内距 4、分隔线 9，整体高度与行位置都要对得上。
    #[test]
    fn layout_uses_spec_metrics()
    {
        let style = MenuStyle::default();
        let work = Rect::new(0.0, 0.0, 1920.0, 1080.0);
        let layout = layout(Rect::new(100.0, 100.0, 360.0, 280.0), &entries(), &style, work).expect("常规工作区应通过");
        assert_eq!(layout.rect.width, 232.0, "菜单宽度取规范 232");
        let expected: f32 = 28.0 * 4.0 + 9.0 + 4.0 * 2.0;
        assert_eq!(layout.rect.height, expected);
        assert!(!layout.flipped_x && !layout.flipped_y && !layout.pushed);
        assert_eq!(layout.rows[0].rect.y, 4.0);
        assert_eq!(layout.rows[1].rect.y, 32.0);
        assert_eq!(layout.rows[2].rect.height, 9.0, "分隔线高度按 9 DIP");
        // 贴格子右侧 2 DIP。
        assert_eq!(layout.rect.x, 100.0 + 360.0 + 2.0);
    }

    /// 定位：右侧放不下向左翻、底部放不下向上翻、两端都放不下再推回工作区。
    #[test]
    fn layout_flips_and_clamps_into_work_area()
    {
        let style = MenuStyle::default();
        let work = Rect::new(0.0, 0.0, 800.0, 600.0);
        let right_edge = layout(Rect::new(600.0, 100.0, 180.0, 200.0), &entries(), &style, work).expect("应通过");
        assert!(right_edge.flipped_x);
        assert_eq!(right_edge.rect.right(), 600.0 - 2.0, "向左展开贴格子左边");
        let bottom = layout(Rect::new(100.0, 500.0, 200.0, 80.0), &entries(), &style, work).expect("应通过");
        assert!(bottom.flipped_y);
        assert_eq!(bottom.rect.bottom(), 580.0, "向上展开贴格子底边");
        // 格子几乎占满整屏：两个方向都翻不动，必须整体推回工作区且完整可见。
        let huge = layout(Rect::new(10.0, 10.0, 780.0, 580.0), &entries(), &style, work).expect("应通过");
        assert!(huge.rect.x >= work.x && huge.rect.right() <= work.right());
        assert!(huge.rect.y >= work.y && huge.rect.bottom() <= work.bottom());
    }



    /// F3：guard 同时托管 HWND 与 Box —— 显示阶段故障必须回滚窗口并释放两个运行时。
    #[test]
    fn open_fault_after_window_creation_reclaims_everything()
    {
        let apartment = ComApartment::enter().expect("需要 COM 线程");
        let mut devices = Devices::new().expect("设备");
        let mut painter = Painter::new().expect("画笔");
        let style = MenuStyle::default();
        let work = Rect::new(0.0, 0.0, 1200.0, 800.0);
        let state = MenuState::open(menu_entries(), Rect::new(10.0, 10.0, 100.0, 60.0), &style, work).expect("应能打开");
        let before_runtimes = live_runtimes();
        let before_windows = count_menu_windows();
        let pixel = PixelRect::new(0, 0, 1200, 800);
        let outcome = MenuWindow::open_with_fault(state, style, true, Color::rgba(0x25, 0x63, 0xEB, 0xFF), &mut painter, &mut devices, pixel, DpiScale::new(96), OpenFault::Show).map(|window| {
            let _ = window.close();
        });
        assert!(outcome.is_err(), "注入故障必须返回错误");
        let error = outcome.unwrap_err().to_string();
        assert!(error.contains("注入故障"), "错误应来自显示阶段：{error}");
        assert_eq!(count_menu_windows(), before_windows, "菜单窗口必须被回收，不能残留");
        assert_eq!(live_runtimes(), before_runtimes, "两个运行时 Box 必须被释放，不能泄漏");
        let _ = apartment;
    }

    /// F1：owner 客户区坐标必须先转屏幕坐标；用**非零原点**证明不是靠 (0,0) 巧合。
    #[test]
    fn owner_client_point_is_converted_to_screen_before_hit_test()
    {
        // 菜单屏幕矩形 x 762..994、y 80..274；owner 原点 (700,0)。
        let menu = RECT { left: 762, top: 80, right: 994, bottom: 274 };
        let owner_origin = (700, 0);
        // owner 本地 x=100 → 屏幕 800 → 落在菜单内，不能判为外部点击。
        let inside = client_to_screen_point(owner_origin, (100, 120));
        assert_eq!(inside, (800, 120));
        assert!(menu_contains_screen_point(menu, inside.0, inside.1), "屏幕 800 落在 762..994，应判为菜单内");
        // owner 本地 x=-30 → 屏幕 670 → 在菜单左侧外部，应关闭。
        let outside = client_to_screen_point(owner_origin, (-30, 120));
        assert_eq!(outside, (670, 120));
        assert!(!menu_contains_screen_point(menu, outside.0, outside.1), "屏幕 670 在菜单外，应判为外部点击");
        // 反例：若错误地拿客户区坐标直接比（不做转换），100 会被判成外部——这正是修复前的行为。
        assert!(!menu_contains_screen_point(menu, 100, 120), "未转换的客户区坐标会被误判为外部（旧缺陷）");
    }

    /// F1：只有 WA_INACTIVE 才关闭；WA_ACTIVE / WA_CLICKACTIVE 都不关。
    #[test]
    fn activation_only_closes_on_inactive()
    {
        assert!(activation_should_close(WA_INACTIVE), "WA_INACTIVE(0) 必须关闭");
        assert!(!activation_should_close(WA_ACTIVE), "WA_ACTIVE(1) 不得关闭（旧实现在这里误杀）");
        assert!(!activation_should_close(WA_CLICKACTIVE), "WA_CLICKACTIVE(2) 不得关闭");
    }

    /// F2：工作区太小放不下一个菜单时拒绝打开，不硬算负数起点。
    #[test]
    fn layout_rejects_tiny_work_area()
    {
        let style = MenuStyle::default();
        let anchor = Rect::new(0.0, 0.0, 10.0, 10.0);
        let tiny = layout(anchor, &entries(), &style, Rect::new(0.0, 0.0, 60.0, 40.0));
        assert!(tiny.is_err(), "60x40 的工作区必须拒绝");
        assert!(tiny.unwrap_err().contains("太小"));
        let narrow = layout(anchor, &entries(), &style, Rect::new(0.0, 0.0, 100.0, 600.0));
        assert!(narrow.is_err(), "宽度不足以放下图标列与文字时必须拒绝");
        assert!(MenuState::open(entries(), anchor, &style, Rect::new(0.0, 0.0, 60.0, 40.0)).is_err(), "状态机也不能在拒绝的布局上打开");
    }

    /// F2：菜单比工作区高时只排能完整放下的行；未绘出的行不可键盘命中。
    #[test]
    fn tall_menu_truncates_and_hidden_rows_are_not_reachable()
    {
        let style = MenuStyle::default();
        let anchor = Rect::new(10.0, 10.0, 100.0, 60.0);
        let many: Vec<MenuEntry> = (0..30).map(|index| MenuEntry::item(&format!("i{index}"), "项")).collect();
        // 工作区只够两行多一点：内距 4×2 + 28×2 = 64。
        let work = Rect::new(0.0, 0.0, 600.0, 70.0);
        let layout = layout(anchor, &many, &style, work).expect("应能放下至少一行");
        assert_eq!(layout.rect.height, 70.0, "菜单高度被工作区截断");
        assert!(layout.hidden_rows >= 27, "绝大多数行被截断：{}", layout.hidden_rows);
        assert_eq!(layout.rows.len(), 2, "只排得下两行");
        let mut state = MenuState::open(many.clone(), anchor, &style, work).expect("应打开");
        // 键盘只能在两行之间移动，绝不会走到未绘出的行。
        for _ in 0..8
        {
            state.key(0x28);
            let index = state.highlight().expect("应有高亮");
            assert!(layout.rows.iter().any(|row| row.index == index), "高亮行必须实际被绘制");
        }
        // 超窄工作区：拒绝。
        assert!(layout_of(&many, &style, Rect::new(0.0, 0.0, 110.0, 800.0)).is_err());
    }

    fn layout_of(entries: &[MenuEntry], style: &MenuStyle, work: Rect) -> std::result::Result<MenuLayout, String>
    {
        layout(Rect::new(0.0, 0.0, 40.0, 40.0), entries, style, work)
    }

    /// 键盘：方向键在可点击项间移动（跳过分隔线与禁用项）、回车触发、Esc 关闭。
    #[test]
    fn keyboard_navigation_and_activation()
    {
        let style = MenuStyle::default();
        let work = Rect::new(0.0, 0.0, 1920.0, 1080.0);
        let mut state = MenuState::open(entries(), Rect::new(0.0, 0.0, 100.0, 100.0), &style, work).expect("应打开");
        assert_eq!(state.highlighted_id(), Some("detail"), "打开时高亮第一个可点击项");
        state.key(0x28);
        assert_eq!(state.highlighted_id(), Some("api"));
        state.key(0x28);
        assert_eq!(state.highlighted_id(), Some("pin"), "分隔线不接收焦点");
        state.key(0x26);
        assert_eq!(state.highlighted_id(), Some("api"));
        assert_eq!(state.key(0x0D), Some(MenuEvent::Activated("api".into())));
        assert_eq!(state.key(0x1B), Some(MenuEvent::Closed("Escape")));
    }

    /// 鼠标：悬停高亮、点击触发、点在行外不触发。
    #[test]
    fn mouse_hover_and_click()
    {
        let style = MenuStyle::default();
        let work = Rect::new(0.0, 0.0, 1920.0, 1080.0);
        let mut state = MenuState::open(entries(), Rect::new(0.0, 0.0, 100.0, 100.0), &style, work).expect("应打开");
        // 第三行（固定显示模式）中心：4 + 28 + 28 + 9 + 14 = 83。
        assert!(matches!(state.hover(20.0, 83.0), Some(MenuEvent::Highlight(_, 3))));
        assert_eq!(state.highlighted_id(), Some("pin"));
        assert_eq!(state.click(20.0, 83.0), Some(MenuEvent::Activated("pin".into())));
        assert_eq!(state.click(20.0, 5.0), Some(MenuEvent::Activated("detail".into())), "第一行点击直接触发");
        assert_eq!(state.click(500.0, 5.0), None, "菜单外坐标不算命中行");
    }

    /// 绘制：圆角 6、1 DIP 描边、每条可点击项都有图标列与文字，危险项用错误色。
    #[test]
    fn draw_list_matches_spec_shape()
    {
        let style = MenuStyle::default();
        let work = Rect::new(0.0, 0.0, 1920.0, 1080.0);
        let state = MenuState::open(entries(), Rect::new(0.0, 0.0, 100.0, 100.0), &style, work).expect("应打开");
        let list = state.draw(&style, true, Color::rgba(0xFF, 0xB0, 0x3A, 0xFF));
        assert!(list.is_balanced());
        match list.commands()[0]
        {
            DrawCommand::FillRoundRect { radius, .. } => assert_eq!(radius, 6.0),
            ref other => panic!("首条应是菜单底，实际 {other:?}"),
        }
        match list.commands()[1]
        {
            DrawCommand::StrokeRoundRect { width, .. } => assert_eq!(width, 1.0),
            ref other => panic!("第二条应是描边，实际 {other:?}"),
        }
        let texts: Vec<&str> = list
            .commands()
            .iter()
            .filter_map(|command| match command
            {
                DrawCommand::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["查看详情", "切换 API", "固定显示模式", "移除"]);
        // 图标列：每个可点击项一个 16 DIP 方块描边。
        let icons = list
            .commands()
            .iter()
            .filter(|command| matches!(command, DrawCommand::StrokeRoundRect { width, .. } if (*width - 1.0).abs() < f32::EPSILON))
            .count();
        assert_eq!(icons, 5, "菜单描边 + 4 个图元方块");
    }
}

/// 一次菜单诊断的结果。
#[derive(Clone, Debug, serde::Serialize)]
pub struct MenuReport
{
    pub pid: u32,
    pub monitors: usize,
    pub style_ex: String,
    pub style: String,
    pub foreground_before: i64,
    pub foreground_while_open: i64,
    pub foreground_unchanged_while_open: bool,
    pub layout_cases: Vec<String>,
    pub events: Vec<String>,
    pub activated: Option<String>,
    pub residual_windows: usize,
    pub exit_code: i32,
    pub notes: Vec<String>,
}

/// 打开菜单状态：布局被拒绝（工作区放不下）时把原因转成 windows 错误返回，不建窗。
fn open_state(entries: &[MenuEntry], anchor: Rect, style: &MenuStyle, work: Rect) -> Result<MenuState>
{
    MenuState::open(entries.to_vec(), anchor, style, work).map_err(|reason| Error::new(E_FAIL, reason))
}

/// 自包含场景的三块格子（不读持久布局），返回 (id, 矩形)。
fn fixture_tiles(work: Rect) -> Vec<(String, Rect)>
{
    let tile = |index: usize| Rect::new(work.x + 80.0 + index as f32 * 320.0, work.y + 80.0, 360.0, 280.0);
    vec![("fixture-a".to_string(), tile(0)), ("fixture-b".to_string(), tile(1)), ("fixture-c".to_string(), tile(2))]
}

/// 跑一次菜单诊断：自包含场景 → 点击格子 → 打开菜单 → 键盘/鼠标/失活各路径 → 关闭。
pub fn run_menu() -> Result<MenuReport>
{
    let apartment = ComApartment::enter()?;
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let monitors = crate::platform::windows::enumerate_desktop_monitors().map_err(|message| Error::new(E_FAIL, message))?;
    let monitor = monitors.first().ok_or_else(|| Error::new(E_FAIL, "没有活动显示器"))?;
    let work_px = PixelRect::new(monitor.work_x, monitor.work_y, monitor.work_x + monitor.work_width as i32, monitor.work_y + monitor.work_height as i32);
    let dpi = DpiScale::new(monitor.dpi);
    let geometry = HostGeometry::for_work_area(work_px, dpi);
    let (dip_width, dip_height) = geometry.dip_size();
    let work = Rect::new(0.0, 0.0, dip_width, dip_height);

    let mut devices = Devices::new()?;
    let mut painter = Painter::new()?;
    let style = MenuStyle::default();
    // 明确固定的**演示调色**：不读偏好也不读布局（诊断期自包含）。
    // 强调色取视觉规范默认令牌 `--color-accent: #2563eb`，其余表面色取围栏令牌的深色默认值。
    // 这不是「与用户主题等价」，正式接入时须换成主题令牌。
    let accent = super::input::focus_border_color("#2563EB");
    let dark = true;

    // 定位与越界用例（纯逻辑，逐条记进报告）。
    let tiles = fixture_tiles(work);
    let layout_cases = vec![
        match layout(tiles[0].1, &menu_entries(), &style, work)
        {
            Ok(case) => format!("贴右侧正常展开：({:.0},{:.0}) {:.0}x{:.0} flip=({},{}) push={} hidden={}", case.rect.x, case.rect.y, case.rect.width, case.rect.height, case.flipped_x, case.flipped_y, case.pushed, case.hidden_rows),
            Err(reason) => format!("拒绝：{reason}"),
        },
        match layout(Rect::new(work.right() - 40.0, work.y + 100.0, 60.0, 120.0), &menu_entries(), &style, work)
        {
            Ok(case) => format!("右侧不足向左翻：({:.0},{:.0}) flip=({},{}) right<=work.right {}", case.rect.x, case.rect.y, case.flipped_x, case.flipped_y, case.rect.right() <= work.right()),
            Err(reason) => format!("拒绝：{reason}"),
        },
        match layout(Rect::new(work.x + 100.0, work.bottom() - 60.0, 200.0, 50.0), &menu_entries(), &style, work)
        {
            Ok(case) => format!("底部不足向上翻：({:.0},{:.0}) flip=({},{}) bottom<=work.bottom {}", case.rect.x, case.rect.y, case.flipped_x, case.flipped_y, case.rect.bottom() <= work.bottom()),
            Err(reason) => format!("拒绝：{reason}"),
        },
        // 极小工作区：拒绝打开（不硬算负数起点）。
        match layout(tiles[0].1, &menu_entries(), &style, Rect::new(0.0, 0.0, 60.0, 40.0))
        {
            Ok(case) => format!("极小工作区竟然通过了：{:?}", case.rect),
            Err(reason) => format!("极小工作区拒绝打开：{reason}"),
        },
        // 超长菜单在常规工作区里被截断：记录未绘出的行数。
        match layout(tiles[0].1, &(0..30).map(|index| MenuEntry::item(&format!("i{index}"), "菜单项")).collect::<Vec<_>>(), &style, work)
        {
            Ok(case) => format!("超长菜单截断：高 {:.0} hidden_rows={}", case.rect.height, case.hidden_rows),
            Err(reason) => format!("拒绝：{reason}"),
        },
    ];

    let mut events: Vec<String> = Vec::new();
    let mut activated: Option<String> = None;
    let foreground_before = unsafe { GetForegroundWindow() }.0 as i64;

    // ① 自包含场景里命中第二块格子（与生产命中函数同一套判定）。
    let entries = {
        let mut scene = super::input::HitScene::new(
            tiles
                .iter()
                .enumerate()
                .map(|(index, (id, rect))| super::input::HitEntry::new(id.clone(), super::input::TargetKind::Item, *rect, 0.0, Vec::new(), 70, index as i32, Some(work)))
                .collect(),
            None,
        );
        let point = (tiles[1].1.x + 10.0, tiles[1].1.y + 10.0);
        match scene.hit(point.0, point.1)
        {
            Some(hit) =>
            {
                events.push(format!("click tile at ({:.0},{:.0}) -> hit {}", point.0, point.1, hit.id));
                hit.id
            }
            None =>
            {
                events.push("click tile -> 未命中".to_string());
                scene = super::input::HitScene::new(Vec::new(), None);
                String::new()
            }
        }
    };
    let _ = entries;

    // ② 打开菜单（在第二块格子旁展开），记录打开前后的前台窗口。
    let anchor = tiles[1].1;
    let mut window = MenuWindow::open(open_state(&menu_entries(), anchor, &style, work)?, style, dark, accent, &mut painter, &mut devices, work_px, dpi)?;
    events.push(format!("menu opened at ({:.0},{:.0}) {:.0}x{:.0}", window.state().layout.rect.x, window.state().layout.rect.y, window.state().layout.rect.width, window.state().layout.rect.height));
    let (ex_style, window_style) = window_styles(window.hwnd);
    let foreground_while_open = unsafe { GetForegroundWindow() }.0 as i64;
    let foreground_unchanged_while_open = foreground_while_open == foreground_before;
    events.push(format!("foreground {foreground_before} -> {foreground_while_open} (unchanged={foreground_unchanged_while_open})"));

    // 泵消息：PostMessage 的按键/鼠标要由本线程的消息循环分发给菜单 WndProc。
    // 消息泵：把投递进来的合成消息交给菜单/owner 的 WndProc（日志里标为「合成」）。

    // ③ 激活语义：WA_ACTIVE 不得关闭；WA_INACTIVE 才关闭。
    unsafe {
        let _ = PostMessageW(Some(window.hwnd), WM_ACTIVATE, WPARAM(WA_ACTIVE as usize), LPARAM(0));
    }
    window.pump();
    events.extend(window.take_events());
    let active_reason = window.close_reason();
    events.push(format!("[script] 合成 WM_ACTIVATE(WA_ACTIVE) -> close_reason={active_reason:?}（应为 None）"));
    // ④ 键盘：下移一格 → 回车触发。先记录按键后是否关闭。
    unsafe {
        let _ = PostMessageW(Some(window.hwnd), WM_KEYDOWN, WPARAM(0x28), LPARAM(0));
    }
    window.pump();
    events.extend(window.take_events());
    events.extend(window.owner_events());
    events.push(format!("after VK_DOWN highlight={}", window.state().highlighted_id().unwrap_or("-")));
    let target = window.state().highlighted_id().map(str::to_string);
    unsafe {
        let _ = PostMessageW(Some(window.hwnd), WM_KEYDOWN, WPARAM(0x0D), LPARAM(0));
    }
    window.pump();
    events.extend(window.take_events());
    events.extend(window.owner_events());
    activated = target.clone();
    events.push(format!("after VK_RETURN activated={target:?} is_window={}", unsafe { IsWindow(Some(window.hwnd)).as_bool() }));
    let _ = window.close();

    // ④ 鼠标悬停 + 点击触发。
    let mut window = MenuWindow::open(open_state(&menu_entries(), anchor, &style, work)?, style, dark, accent, &mut painter, &mut devices, work_px, dpi)?;
    let row = window.state().layout.rows[2].rect;
    let scale = dpi.scale();
    let lparam = (((((row.y + row.height / 2.0) * scale) as i32) << 16) | (((row.x + 30.0) * scale) as i32 & 0xFFFF)) as isize;
    unsafe {
        let _ = PostMessageW(Some(window.hwnd), WM_MOUSEMOVE, WPARAM(0), LPARAM(lparam));
    }
    window.pump();
    events.extend(window.take_events());
    events.extend(window.owner_events());
    events.push(format!("after hover highlight={}", window.state().highlighted_id().unwrap_or("-")));
    unsafe {
        let _ = PostMessageW(Some(window.hwnd), WM_LBUTTONDOWN, WPARAM(0), LPARAM(lparam));
    }
    window.pump();
    events.extend(window.take_events());
    events.extend(window.owner_events());
    let _ = window.close();

    // ⑤ Esc 关闭 + 失活关闭。
    let mut window = MenuWindow::open(open_state(&menu_entries(), anchor, &style, work)?, style, dark, accent, &mut painter, &mut devices, work_px, dpi)?;
    unsafe {
        let _ = PostMessageW(Some(window.hwnd), WM_KEYDOWN, WPARAM(0x1B), LPARAM(0));
    }
    window.pump();
    events.extend(window.take_events());
    events.extend(window.owner_events());
    let _ = window.close();
    let mut window = MenuWindow::open(open_state(&menu_entries(), anchor, &style, work)?, style, dark, accent, &mut painter, &mut devices, work_px, dpi)?;
    unsafe {
        let _ = PostMessageW(Some(window.hwnd), WM_ACTIVATE, WPARAM(0), LPARAM(0));
    }
    window.pump();
    events.extend(window.take_events());
    events.extend(window.owner_events());
    let _ = window.close();

    // ⑤-1 owner 收到菜单矩形外的合成点击 → 关闭（真正的「外部点击」入口）。
    let mut window = MenuWindow::open(open_state(&menu_entries(), anchor, &style, work)?, style, dark, accent, &mut painter, &mut devices, work_px, dpi)?;
    let outside_x = (window.state().layout.rect.x * scale) as i32 + work_px.left - 200;
    let outside_y = (window.state().layout.rect.y * scale) as i32 + work_px.top - 100;
    let owner_param = (((outside_y as i16 as i32) << 16) | (outside_x as i16 as i32 & 0xFFFF)) as isize;
    unsafe {
        let _ = PostMessageW(Some(window.owner), WM_LBUTTONDOWN, WPARAM(0), LPARAM(owner_param));
    }
    window.pump();
    events.extend(window.owner_events());
    events.push(format!("[script] 合成点击发给 owner ({outside_x},{outside_y})（菜单矩形外）-> owner_close_requested={}", window.owner_close_requested()));
    let _ = window.close();

    // ⑤-2 owner 自身失焦 → 关闭。
    let mut window = MenuWindow::open(open_state(&menu_entries(), anchor, &style, work)?, style, dark, accent, &mut painter, &mut devices, work_px, dpi)?;
    unsafe {
        let _ = PostMessageW(Some(window.owner), WM_KILLFOCUS, WPARAM(0), LPARAM(0));
    }
    window.pump();
    events.extend(window.owner_events());
    events.push(format!("[script] 合成 owner WM_KILLFOCUS -> owner_close_requested={}", window.owner_close_requested()));
    let _ = window.close();

    // ⑤-3 非零原点：把 owner 挪到 (700,0)，用两个客户区点证明先 ClientToScreen 再比较。
    let mut window = MenuWindow::open(open_state(&menu_entries(), anchor, &style, work)?, style, dark, accent, &mut painter, &mut devices, work_px, dpi)?;
    unsafe {
        let _ = SetWindowPos(window.owner, None, 700, 0, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }
    window.pump();
    let menu_screen = {
        let mut rect = RECT::default();
        let ok = unsafe { GetWindowRect(window.hwnd, &mut rect) }.is_ok();
        (rect, ok)
    };
    events.push(format!("[script] owner 挪到 (700,0)，菜单屏幕矩形 {}-{}（ok={}）", menu_screen.0.left, menu_screen.0.right, menu_screen.1));
    for (label, client_x) in [("客户区 x=100（屏幕 800，应在菜单内）", 100i32), ("客户区 x=-30（屏幕 670，应在菜单外）", -30i32)]
    {
        let param = (((120i32) << 16) | (client_x as i32 & 0xFFFF)) as isize;
        unsafe {
            let _ = PostMessageW(Some(window.owner), WM_LBUTTONDOWN, WPARAM(0), LPARAM(param));
        }
        window.pump();
        events.extend(window.owner_events());
        events.push(format!("[script] 非零原点 {label} -> owner_close_requested={}", window.owner_close_requested()));
    }
    let _ = window.close();

    // ⑥ 点击落在菜单窗口内、但不在任何条目上：走「菜单内非条目区点击」的关闭分支。
    let mut window = MenuWindow::open(open_state(&menu_entries(), anchor, &style, work)?, style, dark, accent, &mut painter, &mut devices, work_px, dpi)?;
    let lparam = ((((window.state().layout.rect.height - 1.0) * scale) as i32) << 16 | 20) as isize;
    unsafe {
        let _ = PostMessageW(Some(window.hwnd), WM_LBUTTONDOWN, WPARAM(0), LPARAM(lparam));
    }
    window.pump();
    events.extend(window.take_events());
    events.extend(window.owner_events());
    let _ = window.close();

    // ⑦ 失败注入：极小工作区必须在建窗前拒绝，且不留窗口。
    let tiny = PixelRect::new(work_px.left, work_px.top, work_px.left + 60, work_px.top + 40);
    let rejected = match open_state(&menu_entries(), anchor, &style, Rect::new(0.0, 0.0, 10.0, 10.0))
        .and_then(|state| MenuWindow::open(state, style, dark, accent, &mut painter, &mut devices, tiny, dpi))
    {
        Ok(window) =>
        {
            let _ = window.close();
            "竟然打开了（不符合预期）".to_string()
        }
        Err(error) => format!("已拒绝：{error}"),
    };
    events.push(format!("[script] 极小工作区打开注入 -> {rejected}"));

    let residual_windows = count_menu_windows();
    let foreground_after = unsafe { GetForegroundWindow() }.0 as i64;
    events.push(format!("foreground after close {foreground_after} (unchanged={})", foreground_after == foreground_before));
    let _ = apartment;

    Ok(MenuReport {
        pid: std::process::id(),
        monitors: monitors.len(),
        style_ex: format!("0x{ex_style:08X}"),
        style: format!("0x{window_style:08X}"),
        foreground_before,
        foreground_while_open,
        foreground_unchanged_while_open,
        layout_cases,
        events,
        activated,
        residual_windows,
        exit_code: 0,
        notes: vec![
            "诊断期菜单窗口用 WS_EX_NOACTIVATE：键盘靠定向消息投递（与 t44/t45 同源的 PostMessage→WndProc 链路）；ADR-0040 的正式弹层需要键盘，不设该位，两者差异见记录".to_string(),
            "图标列用 16 DIP 占位方块，尚未接入 resources/ 的图标资源".to_string(),
            "未接真实桌面、未接真实输入路由；多屏/混合 DPI、IME、无障碍未验证".to_string(),
        ],
    })
}

/// 一组示例菜单项（分组、危险项与分隔线各一处，用来验证排版规则）。
fn menu_entries() -> Vec<MenuEntry>
{
    vec![
        MenuEntry::item("detail", "查看详情"),
        MenuEntry::item("api", "切换 API"),
        MenuEntry::item("city", "选择城市"),
        MenuEntry::Separator,
        MenuEntry::item("pin", "固定显示模式"),
        MenuEntry::item("mode", "切换显示模式"),
        MenuEntry::Separator,
        MenuEntry::danger("remove", "移除"),
    ]
}

/// 统计本进程仍存活的菜单窗口数量。
fn count_menu_windows() -> usize
{
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    COUNT.store(0, Ordering::SeqCst);
    unsafe extern "system" fn callback(hwnd: HWND, _lparam: LPARAM) -> windows::core::BOOL
    {
        let mut name = [0u16; 64];
        let length = unsafe { GetClassNameW(hwnd, &mut name) };
        if String::from_utf16_lossy(&name[..length.max(0) as usize]) == "DesktopToolsNativeMenu"
        {
            COUNT.fetch_add(1, Ordering::SeqCst);
        }
        windows::core::BOOL(1)
    }
    unsafe {
        let _ = EnumWindows(Some(callback), LPARAM(0));
    }
    COUNT.load(Ordering::SeqCst)
}
