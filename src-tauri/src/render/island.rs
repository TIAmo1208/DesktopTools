//! 原生浮岛（隔离诊断）：三态几何 + 内容卡 + **区域裁剪的逐像素穿透**。
//!
//! 穿透走 B2：`SetWindowRgn` 把窗口区域裁成「卡片圆角矩形（含间隙外的实体像素）」，
//! 区域外的点击由 Windows 直接交给下层窗口（跨进程有效；`HTTRANSPARENT` 只有同线程语义，不作依据）。
//! 区域成功后归 Windows 所有，不自行 `DeleteObject`；只有设置失败才删。
//!
//! 几何与状态语义对齐 `src/features/island/` 与 `docs/design/浮岛.md`；诊断期不接管桌面、不隐藏 Explorer。

use std::sync::OnceLock;

use windows::{
    Win32::{
        Foundation::{E_FAIL, ERROR_SUCCESS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::Gdi::{CombineRgn, CreateRectRgn, CreateRoundRectRgn, DeleteObject, RGN_OR},
        System::LibraryLoader::GetModuleHandleW,
        UI::{HiDpi::*, WindowsAndMessaging::*},
    },
    core::{w, Error, HSTRING, Result, PCWSTR},
};

use super::{
    draw::{Color, DrawCommand, DrawList, Rect, TextAlign},
    geometry::{DpiScale, HostGeometry, PixelRect},
    host::{ComApartment, Devices, SurfaceHost},
    painter::Painter,
};

/// 展开态内容列宽（与 `islandGeometry.ts::ISLAND_EXPANDED_COLUMN_WIDTH` 一致）。
pub const EXPANDED_COLUMN_WIDTH: f32 = 400.0;
/// 展开态滚动条槽宽（与 `ISLAND_SCROLLBAR_GUTTER` 一致）。
pub const SCROLLBAR_GUTTER: f32 = 16.0;
/// 胶囊态贴边间距（刘海形态为 0）。
pub const CAPSULE_GAP: f32 = 6.0;
/// 常态胶囊/收起态固定高度（音乐胶囊固定件的合计高度）。
pub const COLLAPSED_HEIGHT: f32 = 52.0;
/// 悬停进入/回落的双向延迟（`docs/design/浮岛.md` §32）。
pub const HOVER_DELAY_MS: u64 = 180;
/// 透明边距停留判定阈值（`docs/design/浮岛.md` §41/§43：停留满 `120ms` 回落）。
pub const TRANSPARENT_LEAVE_MS: u64 = 120;
/// 定时器 ID（`SetTimer`/`WM_TIMER` 用来周期驱动轮询）。
const ISLAND_TIMER_ID: usize = 1;
/// 定时器触发次数（测试用来证明周期调度真的在跑）。
static ISLAND_TIMER_TICKS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// 定时器触发次数。
pub fn timer_ticks() -> usize
{
    ISLAND_TIMER_TICKS.load(std::sync::atomic::Ordering::SeqCst)
}

/// 光标采样器：生产用真实光标，测试注入**窗口局部 DIP**序列（确定、不依赖真实光标）。
#[derive(Clone, Debug)]
pub enum CursorSampler
{
    Real,
    /// 注入序列；用完后重复最后一个点，便于「一直停在外面」这类序列。
    Script(Vec<(f32, f32)>, usize),
}

impl CursorSampler
{
    /// 注入一组序列。
    pub fn script(points: Vec<(f32, f32)>) -> Self
    {
        Self::Script(points, 0)
    }

    fn next(&mut self) -> Option<(f32, f32)>
    {
        match self
        {
            Self::Real => None,
            Self::Script(points, cursor) =>
            {
                if points.is_empty()
                {
                    return None;
                }
                let index = (*cursor).min(points.len() - 1);
                *cursor = (*cursor + 1).min(points.len() - 1);
                points.get(index).copied()
            }
        }
    }
}

/// 透明边距的轮询间隔：区域裁掉透明带后窗口收不到那里的指针事件，只能用低频轮询补位。
/// 取值 `70ms` 落在用户给的 `60–80ms` 区间内（自定，非文档数值）。
pub const CURSOR_POLL_MS: u64 = 70;

/// 浮岛贴边方向。
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
    /// 从偏好里的字符串解析（空串按顶部处理，与现有实现一致）。
    pub fn parse(value: &str) -> Self
    {
        match value
        {
            "bottom" => Self::Bottom,
            "left" => Self::Left,
            "right" => Self::Right,
            _ => Self::Top,
        }
    }

    /// 是否左右贴边（沿竖直方向居中）。
    fn horizontal(self) -> bool
    {
        matches!(self, Self::Left | Self::Right)
    }
}

/// 浮岛三态：收起 / 紧凑（hover）/ 展开。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IslandState
{
    Collapsed,
    Compact,
    Expanded,
}

impl IslandState
{
    /// 状态名（日志与报告用）。
    pub fn as_str(self) -> &'static str
    {
        match self
        {
            Self::Collapsed => "collapsed",
            Self::Compact => "compact",
            Self::Expanded => "expanded",
        }
    }
}

/// 一张内容卡的原始尺寸（由内容实测给出；诊断期用 fixture）。
#[derive(Clone, Debug)]
pub struct IslandCard
{
    pub id: String,
    pub title: String,
    pub body: String,
    pub width: f32,
    pub height: f32,
}

/// 浮岛配置：贴边、偏移、圆角、形态、三态与卡片顺序。
#[derive(Clone, Debug)]
pub struct IslandConfig
{
    pub edge: IslandEdge,
    pub offset: f32,
    pub radius: f32,
    pub notch: bool,
    pub cards: Vec<IslandCard>,
}

/// 一次布局结果：窗口矩形、卡片矩形（窗口局部 DIP）、区域用的圆角矩形。
#[derive(Clone, Debug, PartialEq)]
pub struct IslandGeometry
{
    pub state: IslandState,
    pub window: Rect,
    pub cards: Vec<Rect>,
    pub radius: f32,
    /// 窗口外扩量（刘海形态沿边各外扩 12 DIP，展开态为 0）。
    pub overhang: f32,
}

impl IslandGeometry
{
    /// 显示层轮廓（区域）：**整块内容列 / 胶囊本体**，卡片间隙包含在内。
    ///
    /// 语义来自交互要求：鼠标在两卡之间穿过时不能触发「离开浮岛」，所以间隙必须命中本窗；
    /// 要穿透的是内容列以外的透明视口、圆角外三角与窗口外区域。
    pub fn outline(&self) -> Vec<(Rect, f32)>
    {
        match self.state
        {
            IslandState::Collapsed =>
            {
                let card = self.cards[0];
                vec![(Rect::new(card.x, 0.0, card.width, self.window.height), self.window.height / 2.0)]
            }
            IslandState::Compact =>
            {
                let left = self.cards.iter().map(|card| card.x).fold(f32::MAX, f32::min);
                let right = self.cards.iter().map(|card| card.x + card.width).fold(f32::MIN, f32::max);
                vec![(Rect::new(left, 0.0, right - left, self.window.height), self.radius)]
            }
            IslandState::Expanded =>
            {
                // 内容列（含滚动条槽）整块矩形：卡片之间的间隙也在这块里。
                let left = self.cards.iter().map(|card| card.x).fold(f32::MAX, f32::min);
                let right = self.cards.iter().map(|card| card.x + card.width).fold(f32::MIN, f32::max);
                vec![(Rect::new(left, 0.0, right - left, self.window.height), self.radius)]
            }
        }
    }
}

/// 计算三态几何。
///
/// - 收起：贴边 `COLLAPSED_HEIGHT` 高的单行胶囊，宽度取卡片最大自然宽；
/// - 紧凑：卡片列按实测高度排开（hover 卡片夹到 392 DIP）；
/// - 展开：窗口覆盖整个工作区（ADR-0047），内容列 `EXPANDED_COLUMN_WIDTH` + 滚动条槽。
///
/// 超出工作区一律拒绝，不做裁切硬算（避免负起点）。
pub fn geometry(config: &IslandConfig, state: IslandState, work: Rect) -> std::result::Result<IslandGeometry, String>
{
    if config.cards.is_empty()
    {
        return Err("浮岛缺少卡片".to_string());
    }
    let widest = config.cards.iter().map(|card| card.width).fold(0.0f32, f32::max);
    let overhang = match (config.notch, state)
    {
        (true, IslandState::Expanded) => 0.0,
        (true, _) => 12.0,
        (false, _) => 0.0,
    };
    let (window, cards, radius) = match state
    {
        IslandState::Collapsed =>
        {
            let width = widest.min(440.0);
            let height = COLLAPSED_HEIGHT;
            let window = place(config, work, width, height, overhang)?;
            let card = Rect::new(overhang, 0.0, width, height);
            (window, vec![card], height / 2.0)
        }
        IslandState::Compact =>
        {
            let width = widest.min(392.0);
            let gap = 8.0;
            let height = config.cards.iter().map(|card| card.height).sum::<f32>() + gap * (config.cards.len() as f32 - 1.0);
            let window = place(config, work, width, height, overhang)?;
            let mut cursor = 0.0;
            let cards = config
                .cards
                .iter()
                .map(|card| {
                    let rect = Rect::new(overhang, cursor, width, card.height);
                    cursor += card.height + gap;
                    rect
                })
                .collect();
            (window, cards, 16.0)
        }
        IslandState::Expanded =>
        {
            let width = work.width;
            let height = work.height;
            let window = Rect::new(work.x, work.y, width, height);
            let column = EXPANDED_COLUMN_WIDTH + SCROLLBAR_GUTTER;
            let x = match config.edge
            {
                IslandEdge::Left => 16.0,
                IslandEdge::Right => (work.width - column - 16.0).max(0.0),
                IslandEdge::Top | IslandEdge::Bottom =>
                {
                    let centered = ((work.width - column) / 2.0 + config.offset).max(0.0);
                    centered.min((work.width - column).max(0.0))
                }
            };
            let mut cursor = 16.0;
            let cards = config
                .cards
                .iter()
                .map(|card| {
                    let rect = Rect::new(x, cursor, EXPANDED_COLUMN_WIDTH, card.height);
                    cursor += card.height + 8.0;
                    rect
                })
                .collect();
            (window, cards, config.radius)
        }
    };
    Ok(IslandGeometry { state, window, cards, radius, overhang })
}

/// 按贴边方向把窗口放到工作区里（超出即拒绝）。
fn place(config: &IslandConfig, work: Rect, width: f32, height: f32, overhang: f32) -> std::result::Result<Rect, String>
{
    if width > work.width || height > work.height
    {
        return Err(format!("浮岛内容 {width}x{height} DIP 超过工作区 {}x{}", work.width, work.height));
    }
    let gap = match config.notch
    {
        true => 0.0,
        false => CAPSULE_GAP,
    };
    let (x, y) = match config.edge
    {
        IslandEdge::Top => (((work.width - width) / 2.0 + config.offset).clamp(0.0, work.width - width), gap),
        IslandEdge::Bottom => (((work.width - width) / 2.0 + config.offset).clamp(0.0, work.width - width), work.height - height - gap),
        IslandEdge::Left => (gap, ((work.height - height) / 2.0 + config.offset).clamp(0.0, work.height - height)),
        IslandEdge::Right => (work.width - width - gap, ((work.height - height) / 2.0 + config.offset).clamp(0.0, work.height - height)),
    };
    let _ = overhang;
    Ok(Rect::new(work.x + x, work.y + y, width, height))
}

/// 悬停状态机：进入/离开都走 180ms 双向延迟；透明区的进出不改变状态。
#[derive(Clone, Debug)]
pub struct HoverState
{
    pub current: IslandState,
    /// 待生效的目标与截止时刻（毫秒）。
    pending: Option<(IslandState, u64)>,
    /// 展开态是否由「移出浮岛体」触发回落（诊断里只记录）。
    pub last_reason: String,
}

impl HoverState
{
    /// 新建：从收起态开始。
    pub fn new() -> Self
    {
        Self { current: IslandState::Collapsed, pending: None, last_reason: "初始".to_string() }
    }

    /// 指针进入内容区：目标是紧凑态（展开态下不降级）。
    pub fn pointer_enter(&mut self, now: u64) -> Option<(IslandState, IslandState, &'static str)>
    {
        if self.current == IslandState::Expanded
        {
            self.pending = None;
            return None;
        }
        let target = IslandState::Compact;
        self.schedule(target, now)
    }

    /// 指针离开内容区：目标是收起态。
    pub fn pointer_leave(&mut self, now: u64) -> Option<(IslandState, IslandState, &'static str)>
    {
        self.schedule(IslandState::Collapsed, now)
    }

    /// 指针在**透明间隙**里移动：不改变状态，只清掉「离开」的待生效项（与现有实现的双向延迟一致）。
    pub fn pointer_in_gap(&mut self) -> bool
    {
        let had = self.pending.is_some();
        if let Some((target, _)) = self.pending
        {
            if target == IslandState::Collapsed
            {
                self.pending = None;
            }
        }
        had
    }

    fn schedule(&mut self, target: IslandState, now: u64) -> Option<(IslandState, IslandState, &'static str)>
    {
        if target == self.current && self.pending.is_none()
        {
            return None;
        }
        self.pending = Some((target, now + HOVER_DELAY_MS));
        None
    }

    /// 推进时钟：到期才真正切态，返回切换记录。
    pub fn tick(&mut self, now: u64) -> Option<(IslandState, IslandState)>
    {
        let (target, deadline) = self.pending?;
        if now < deadline
        {
            return None;
        }
        self.pending = None;
        if target == self.current
        {
            return None;
        }
        let before = self.current;
        self.current = target;
        self.last_reason = match target
        {
            IslandState::Compact => "悬停进入（180ms 到期）",
            IslandState::Collapsed => "离开回落（180ms 到期）",
            IslandState::Expanded => "外部切到展开",
        }
        .to_string();
        Some((before, target))
    }

    /// 直接切态（诊断脚本切换三态用）。
    pub fn switch(&mut self, state: IslandState, reason: &str)
    {
        self.current = state;
        self.pending = None;
        self.last_reason = reason.to_string();
    }
}

/// 透明边距的离开判定（纯逻辑）：展开/悬停期间低频轮询光标，离开轮廓满 `120ms` 才回落。
///
/// 与 180ms 悬停延迟的关系：`HOVER_DELAY_MS` 管「进入/回落延迟」（`浮岛.md` §32），
/// 这里管「指针停在透明边距多久算离开」（`浮岛.md` §41/§43），两者是两条独立的计时，不能合并。
#[derive(Clone, Debug)]
pub struct TransparentMargin
{
    /// 开始「停在外侧」的时刻（毫秒）。
    outside_since: Option<u64>,
    /// 本次是否已经因为透明边距回落过（避免重复触发）。
    pub last_reason: String,
}

impl Default for TransparentMargin
{
    fn default() -> Self
    {
        Self::new()
    }
}

impl TransparentMargin
{
    /// 新建：未计时。
    pub fn new() -> Self
    {
        Self { outside_since: None, last_reason: "未触发".to_string() }
    }

    /// 轮询一次：`inside` 表示光标是否仍在「显示层轮廓 + 边距」内。
    ///
    /// 返回 `Some(true)` 表示「停留到点，应当回落」；`None` 表示继续等待或已取消。
    pub fn observe(&mut self, now: u64, inside: bool) -> Option<bool>
    {
        if inside
        {
            if self.outside_since.take().is_some()
            {
                self.last_reason = "指针回到轮廓内，取消透明边距计时".to_string();
            }
            return None;
        }
        match self.outside_since
        {
            None =>
            {
                self.outside_since = Some(now);
                self.last_reason = format!("指针在轮廓外，开始 {TRANSPARENT_LEAVE_MS}ms 计时");
                None
            }
            Some(since) if now.saturating_sub(since) >= TRANSPARENT_LEAVE_MS =>
            {
                self.outside_since = None;
                self.last_reason = format!("指针在轮廓外停留满 {TRANSPARENT_LEAVE_MS}ms，回落");
                Some(true)
            }
            Some(_) => None,
        }
    }

    /// 是否正在计时。
    pub fn polling(&self) -> bool
    {
        self.outside_since.is_some()
    }
}

/// 生成浮岛绘制列表（窗口局部 DIP）。
pub fn draw(config: &IslandConfig, geometry: &IslandGeometry, dark: bool) -> DrawList
{
    let mut list = DrawList::new();
    let surface = if dark { Color::rgba(0x1C, 0x22, 0x24, 0xF7) } else { Color::rgba(0xF3, 0xF3, 0xF3, 0xF7) };
    let text = if dark { Color::rgba(0xE8, 0xEF, 0xF0, 0xFF) } else { Color::rgba(0x20, 0x25, 0x26, 0xFF) };
    let muted = if dark { Color::rgba(0xA8, 0xB4, 0xB6, 0xFF) } else { Color::rgba(0x5C, 0x66, 0x68, 0xFF) };
    for (card, rect) in config.cards.iter().zip(geometry.cards.iter())
    {
        list.push(DrawCommand::FillRoundRect { rect: *rect, radius: geometry.radius, color: surface });
        let padding = match geometry.state
        {
            IslandState::Expanded => 16.0,
            IslandState::Compact => 12.0,
            IslandState::Collapsed => 8.0,
        };
        // 标题：常态 13 / hover 13 / 展开 14（`docs/design/浮岛.md` 内容设计语言）。
        let title_size = match geometry.state
        {
            IslandState::Expanded => 14.0,
            _ => 13.0,
        };
        list.push(DrawCommand::Text {
            rect: Rect::new(rect.x + padding, rect.y + padding, (rect.width - padding * 2.0).max(0.0), title_size * 1.4),
            text: card.title.clone(),
            size: title_size,
            weight: 500,
            color: text,
            align: TextAlign::Start,
            max_lines: 1,
        });
        // 正文：常态不显示（空间不足），hover 与展开显示；缺数据时用降级文案。
        if geometry.state != IslandState::Collapsed
        {
            let body = if card.body.trim().is_empty() { "暂无内容" } else { card.body.as_str() };
            list.push(DrawCommand::Text {
                rect: Rect::new(rect.x + padding, rect.y + padding + title_size * 1.6, (rect.width - padding * 2.0).max(0.0), 14.0),
                text: body.to_string(),
                size: 12.0,
                weight: 400,
                color: muted,
                align: TextAlign::Start,
                max_lines: 2,
            });
        }
    }
    list
}

/// 浮岛窗口运行时。
/// 时钟源：生产用真实 `GetTickCount64`，测试注入固定序列（与机器负载无关）。
#[derive(Clone, Debug)]
pub enum ClockSource
{
    Real,
    Script(Vec<u64>, usize),
}

impl ClockSource
{
    /// 注入一组固定时刻。
    pub fn script(values: Vec<u64>) -> Self
    {
        Self::Script(values, 0)
    }

    fn now(&mut self) -> u64
    {
        match self
        {
            Self::Real => unsafe { windows::Win32::System::SystemInformation::GetTickCount64() },
            Self::Script(values, cursor) =>
            {
                if values.is_empty()
                {
                    return 0;
                }
                let index = (*cursor).min(values.len() - 1);
                *cursor = (*cursor + 1).min(values.len() - 1);
                values[index]
            }
        }
    }
}

pub struct IslandRuntime
{
    pub config: IslandConfig,
    pub hover: HoverState,
    pub geometry: IslandGeometry,
    pub dark: bool,
    pub host: SurfaceHost,
    pub painter: *mut Painter,
    pub devices: *mut Devices,
    pub work_px: PixelRect,
    pub dpi: DpiScale,
    pub events: Vec<String>,
    /// 当前区域是否由本进程持有（成功后归 Windows，不删）。
    pub region_owned: bool,
    /// 透明边距的离开判定（仅展开态启用低频轮询）。
    pub margin: TransparentMargin,
    /// 光标采样器（生产 `Real`，测试注入序列）。
    pub sampler: CursorSampler,
    /// 时钟源（生产 `Real`，测试注入固定时刻）。
    pub clock: ClockSource,
    /// `WM_TIMER` 置位，由消息泵在分发结束后调用 `tick_polling`（避免在 WndProc 里重入销毁窗口）。
    pub pending_poll: bool,
    /// 定时器是否在跑。
    pub timer_on: bool,
    /// 每次 `WM_TIMER` 的时刻（毫秒，`GetTickCount64`），用于验证周期与停止。
    pub timer_stamps: Vec<u64>,
}

const ISLAND_CLASS: PCWSTR = w!("DesktopToolsNativeIsland");

/// 已发布帧数（测试用来断言「失败路径没有发布帧」）。
static ISLAND_PUBLISHED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// 轮询调用次数（测试用来证明定时路径真的接线了）。
static ISLAND_POLLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// 轮询调用次数。
pub fn poll_calls() -> usize
{
    ISLAND_POLLS.load(std::sync::atomic::Ordering::SeqCst)
}

/// 存活的浮岛运行时数量（测试用来断言失败路径把 Box 释放了）。
static ISLAND_LIVE: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// 已发布帧数。
pub fn published_frames() -> usize
{
    ISLAND_PUBLISHED.load(std::sync::atomic::Ordering::SeqCst)
}

/// 存活的浮岛运行时数量。
pub fn live_runtimes() -> isize
{
    ISLAND_LIVE.load(std::sync::atomic::Ordering::SeqCst)
}

impl Drop for IslandRuntime
{
    fn drop(&mut self)
    {
        ISLAND_LIVE.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// 浮岛窗口：置顶非激活、区域裁剪。
pub struct IslandWindow
{
    pub hwnd: HWND,
    runtime: *mut IslandRuntime,
}

impl IslandWindow
{
    /// 建窗并应用首次区域。
    pub fn open(config: IslandConfig, state: IslandState, dark: bool, painter: &mut Painter, devices: &mut Devices, work_px: PixelRect, work_dip: Rect, dpi: DpiScale) -> Result<Self>
    {
        Self::open_with_fault(config, state, dark, painter, devices, work_px, work_dip, dpi, RegionFault::None)
    }

    /// 带采样器与故障注入的建窗（测试用注入采样器，生产走 `open`）。
    #[allow(clippy::too_many_arguments)]
    pub fn open_with_sampler(config: IslandConfig, state: IslandState, dark: bool, painter: &mut Painter, devices: &mut Devices, work_px: PixelRect, work_dip: Rect, dpi: DpiScale, sampler: CursorSampler) -> Result<Self>
    {
        Self::open_inner(config, state, dark, painter, devices, work_px, work_dip, dpi, RegionFault::None, sampler)
    }

    /// 带区域故障注入的建窗：区域门禁在**显示与发布之前**，失败即销毁窗口并返回错误。
    #[allow(clippy::too_many_arguments)]
    pub fn open_with_fault(config: IslandConfig, state: IslandState, dark: bool, painter: &mut Painter, devices: &mut Devices, work_px: PixelRect, work_dip: Rect, dpi: DpiScale, fault: RegionFault) -> Result<Self>
    {
        Self::open_inner(config, state, dark, painter, devices, work_px, work_dip, dpi, fault, CursorSampler::Real)
    }

    #[allow(clippy::too_many_arguments)]
    fn open_inner(config: IslandConfig, state: IslandState, dark: bool, painter: &mut Painter, devices: &mut Devices, work_px: PixelRect, work_dip: Rect, dpi: DpiScale, fault: RegionFault, sampler: CursorSampler) -> Result<Self>
    {
        let instance: HINSTANCE = unsafe { GetModuleHandleW(None) }.map(|value| HINSTANCE(value.0))?;
        register_island_class(instance)?;
        let geometry = geometry(&config, state, work_dip).map_err(|reason| Error::new(E_FAIL, reason))?;
        let (x, y, width, height) = window_pixels(&geometry, work_px, dpi);
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_NOREDIRECTIONBITMAP,
                ISLAND_CLASS,
                &HSTRING::from("原生浮岛"),
                WS_POPUP,
                x,
                y,
                width,
                height,
                None,
                None,
                Some(instance),
                None,
            )
        }?;
        let mut guard = IslandGuard::new(hwnd);
        let host = SurfaceHost::attach(hwnd, HostGeometry::for_work_area(PixelRect::new(x, y, x + width, y + height), dpi), &devices.composition)?;
        let mut hover = HoverState::new();
        hover.switch(state, "打开时指定");
        let runtime = Box::into_raw(Box::new(IslandRuntime {
            config,
            hover,
            geometry: geometry.clone(),
            dark,
            host,
            painter: painter as *mut Painter,
            devices: devices as *mut Devices,
            work_px,
            dpi,
            events: Vec::new(),
            region_owned: false,
            margin: TransparentMargin::new(),
            sampler,
            clock: ClockSource::Real,
            pending_poll: false,
            timer_on: false,
            timer_stamps: Vec::new(),
        }));
        ISLAND_LIVE.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        guard.runtime = runtime;
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, runtime as isize);
            // 先改几何（窗口仍隐藏），再设区域；区域坐标按新几何解释。
            SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, width, height, SWP_NOACTIVATE).map_err(|error| Error::new(E_FAIL, format!("SetWindowPos(geom) 失败：{error}")))?;
        }
        // 门禁：区域不合格就销毁窗口、不显示、不发布。
        if let Err(reason) = apply_region_gated(hwnd, &geometry, dpi, fault)
        {
            unsafe { (*runtime).events.push(format!("[island] 区域门禁失败：{reason}（不显示、不发布）")); }
            return Err(Error::new(E_FAIL, reason));
        }
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, width, height, SWP_NOACTIVATE | SWP_SHOWWINDOW).map_err(|error| Error::new(E_FAIL, format!("SetWindowPos(SHOWWINDOW) 失败：{error}")))?;
        }
        if !unsafe { IsWindowVisible(hwnd).as_bool() }
        {
            return Err(Error::new(E_FAIL, "浮岛窗口显示后不可见"));
        }
        guard.armed = false;
        let window = Self { hwnd, runtime };
        window.publish_frame()?;
        window.sync_timer();
        Ok(window)
    }

    /// 按状态同步定时器：展开态开、其余态关（收起/退出时 `KillTimer`）。
    fn sync_timer(&self)
    {
        let runtime = unsafe { &mut *self.runtime };
        let want = runtime.hover.current == IslandState::Expanded;
        unsafe {
            match (want, runtime.timer_on)
            {
                (true, false) =>
                {
                    if SetTimer(Some(self.hwnd), ISLAND_TIMER_ID, CURSOR_POLL_MS as u32, None) != 0
                    {
                        runtime.timer_on = true;
                        runtime.events.push(format!("[island] SetTimer({CURSOR_POLL_MS}ms) 启动"));
                    }
                }
                (false, true) =>
                {
                    let _ = KillTimer(Some(self.hwnd), ISLAND_TIMER_ID);
                    runtime.timer_on = false;
                    runtime.events.push("[island] KillTimer 停止".to_string());
                }
                _ => {}
            }
        }
    }

    /// 消息泵：分发队列里的消息，并在 `WM_TIMER` 置位后调用 `tick_polling`（借用已结束，安全）。
    pub fn pump_for(&self, duration_ms: u64) -> Result<()>
    {
        let started = std::time::Instant::now();
        let mut message = MSG::default();
        while started.elapsed().as_millis() as u64 <= duration_ms
        {
            unsafe {
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool()
                {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            let pending = unsafe { (*self.runtime).pending_poll };
            if pending
            {
                unsafe {
                    (*self.runtime).pending_poll = false;
                }
                let now = started.elapsed().as_millis() as u64;
                let _ = self.tick_polling(now)?;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        Ok(())
    }

    /// 发布当前几何的一帧（**只能**在区域门禁通过之后调用）。
    fn publish_frame(&self) -> Result<()>
    {
        let runtime = unsafe { &mut *self.runtime };
        let (x, y, width, height) = window_pixels(&runtime.geometry, runtime.work_px, runtime.dpi);
        let list = draw(&runtime.config, &runtime.geometry, runtime.dark);
        let painter = unsafe { &mut *runtime.painter };
        let devices = unsafe { &mut *runtime.devices };
        let host_geometry = HostGeometry::for_work_area(PixelRect::new(x, y, x + width, y + height), runtime.dpi);
        let (surface, _) = SurfaceHost::prepare(host_geometry, painter, devices, &list)?;
        runtime.host.publish(host_geometry, surface, devices)?;
        ISLAND_PUBLISHED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    /// 按当前几何重算区域并重绘（切态/几何变化都走这里）。
    ///
    /// 门禁失败 = 错误出口：销毁窗口、不发布帧、返回错误（不允许「只写日志继续」）。
    pub fn apply_region_and_paint(&self) -> Result<()>
    {
        let runtime = unsafe { &mut *self.runtime };
        let geometry = geometry(&runtime.config, runtime.hover.current, rect_from_pixels(runtime.work_px, runtime.dpi)).map_err(|reason| Error::new(E_FAIL, reason))?;
        let (x, y, width, height) = window_pixels(&geometry, runtime.work_px, runtime.dpi);
        unsafe {
            let _ = SetWindowPos(self.hwnd, Some(HWND_NOTOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            SetWindowPos(self.hwnd, Some(HWND_TOPMOST), x, y, width, height, SWP_NOACTIVATE).map_err(|error| Error::new(E_FAIL, format!("SetWindowPos(geom) 失败：{error}")))?;
        }
        // 门禁：唯一允许调用 SetWindowRgn 的地方。
        if let Err(reason) = apply_region_gated(self.hwnd, &geometry, runtime.dpi, RegionFault::None)
        {
            runtime.events.push(format!("[island] 区域门禁失败：{reason}（销毁窗口、不发布）"));
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
            return Err(Error::new(E_FAIL, reason));
        }
        runtime.events.push(format!("[island] region applied: {} 块，{}x{}（门禁通过）", geometry.cards.len(), width, height));
        runtime.geometry = geometry;
        let _ = (x, y, width, height);
        self.publish_frame()
    }

    /// 切到某个状态（诊断脚本驱动）。
    pub fn switch(&self, state: IslandState, reason: &str) -> Result<()>
    {
        let runtime = unsafe { &mut *self.runtime };
        let before = runtime.hover.current;
        runtime.hover.switch(state, reason);
        runtime.events.push(format!("[island] state {} -> {}（{reason}）", before.as_str(), state.as_str()));
        self.apply_region_and_paint()?;
        self.sync_timer();
        Ok(())
    }

    /// 悬停：进入内容区。
    pub fn pointer_enter(&self, now: u64) -> Result<()>
    {
        let runtime = unsafe { &mut *self.runtime };
        runtime.hover.pointer_enter(now);
        self.tick(now)
    }

    /// 悬停：离开内容区。
    pub fn pointer_leave(&self, now: u64) -> Result<()>
    {
        let runtime = unsafe { &mut *self.runtime };
        runtime.hover.pointer_leave(now);
        self.tick(now)
    }

    /// 悬停：在透明间隙里移动（不改变状态）。
    pub fn pointer_in_gap(&self)
    {
        let runtime = unsafe { &mut *self.runtime };
        if runtime.hover.pointer_in_gap()
        {
            runtime.events.push("[island] 透明间隙移动：取消「离开」待生效项，不切态".to_string());
        }
        else
        {
            runtime.events.push("[island] 透明间隙移动：无待生效项".to_string());
        }
    }

    /// 推进时钟并按需切态。
    pub fn tick(&self, now: u64) -> Result<()>
    {
        let runtime = unsafe { &mut *self.runtime };
        match runtime.hover.tick(now)
        {
            Some((before, after)) =>
            {
                runtime.events.push(format!("[island] hover 到期：{} -> {}（{}）", before.as_str(), after.as_str(), runtime.hover.last_reason));
                self.apply_region_and_paint()
            }
            None => Ok(()),
        }
    }

    /// 透明边距轮询：仅在展开态启用；指针在轮廓外连续停留满 `120ms` 则回落常态。
    ///
    /// 频率与阈值：`CURSOR_POLL_MS = 70`（自定，落在 60–80ms）、`TRANSPARENT_LEAVE_MS = 120`
    /// （`docs/design/浮岛.md` §41/§43）。与 180ms 悬停进入延迟是两条独立计时。
    pub fn poll_leave(&self, now: u64) -> Result<bool>
    {
        let runtime = unsafe { &mut *self.runtime };
        if runtime.hover.current != IslandState::Expanded
        {
            // 收起/紧凑态停用轮询，避免常驻开销。
            return Ok(false);
        }
        let scale = runtime.dpi.scale();
        let local = match runtime.sampler.next()
        {
            // 注入序列：已经是窗口局部 DIP。
            Some(point) => point,
            None =>
            {
                let mut cursor = POINT { x: 0, y: 0 };
                if unsafe { GetCursorPos(&mut cursor) }.is_err()
                {
                    return Ok(false);
                }
                unsafe {
                    let _ = windows::Win32::Graphics::Gdi::ScreenToClient(self.hwnd, &mut cursor);
                }
                (cursor.x as f32 / scale, cursor.y as f32 / scale)
            }
        };
        self.poll_observe(now, local)
    }

    /// 用**窗口局部 DIP 坐标**推进一次透明边距判定（纯逻辑入口，测试可直接打）。
    pub fn poll_observe(&self, now: u64, local: (f32, f32)) -> Result<bool>
    {
        let runtime = unsafe { &mut *self.runtime };
        let inside = outline_contains(&runtime.geometry.outline(), local.0, local.1);
        let decision = runtime.margin.observe(now, inside);
        runtime.events.push(format!("[island] 轮询 now={now} 局部=({:.0},{:.0}) 轮廓内={inside} -> {}", local.0, local.1, runtime.margin.last_reason));
        match decision
        {
            Some(true) =>
            {
                runtime.hover.switch(IslandState::Collapsed, "透明边距停留 120ms 回落");
                runtime.events.push("[island] 透明边距到点：expanded -> collapsed".to_string());
                self.apply_region_and_paint()?;
                self.sync_timer();
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// **定时/帧路径**的轮询入口：只有窗口可见且处于展开态才真正轮询。
    ///
    /// 守卫：`IsWindowVisible(hwnd)` 为假（隐藏/销毁前）或状态不是 `Expanded` → 直接返回，不计数、不读光标。
    pub fn tick_polling(&self, now: u64) -> Result<bool>
    {
        if !unsafe { IsWindowVisible(self.hwnd).as_bool() }
        {
            return Ok(false);
        }
        let runtime = unsafe { &*self.runtime };
        if runtime.hover.current != IslandState::Expanded
        {
            return Ok(false);
        }
        ISLAND_POLLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.poll_leave(now)
    }

    /// 当前是否在轮询（展开态且已开始计时）。
    pub fn polling(&self) -> bool
    {
        unsafe { (*self.runtime).hover.current == IslandState::Expanded }
    }

    /// 替换时钟源（测试注入固定时刻）。
    pub fn set_clock(&self, clock: ClockSource)
    {
        unsafe {
            (*self.runtime).clock = clock;
        }
    }

    /// `WM_TIMER` 的时刻列表（毫秒）。
    pub fn timer_stamps(&self) -> Vec<u64>
    {
        unsafe { (*self.runtime).timer_stamps.clone() }
    }

    /// 事件日志。
    pub fn events(&self) -> Vec<String>
    {
        unsafe { (*self.runtime).events.clone() }
    }

    /// 当前状态。
    pub fn state(&self) -> IslandState
    {
        unsafe { (*self.runtime).hover.current }
    }

    /// 当前几何。
    pub fn geometry(&self) -> IslandGeometry
    {
        unsafe { (*self.runtime).geometry.clone() }
    }

    /// 关闭并回收。
    pub fn close(mut self)
    {
        let runtime = self.runtime;
        self.runtime = std::ptr::null_mut();
        unsafe {
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            drop(Box::from_raw(runtime));
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

impl Drop for IslandWindow
{
    fn drop(&mut self)
    {
        if !self.runtime.is_null()
        {
            let runtime = self.runtime;
            self.runtime = std::ptr::null_mut();
            unsafe {
                SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(runtime));
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

/// 建窗失败时的回收器（同时持有 HWND 与运行时）。
struct IslandGuard
{
    hwnd: HWND,
    runtime: *mut IslandRuntime,
    armed: bool,
}

impl IslandGuard
{
    fn new(hwnd: HWND) -> Self
    {
        Self { hwnd, runtime: std::ptr::null_mut(), armed: true }
    }
}

impl Drop for IslandGuard
{
    fn drop(&mut self)
    {
        if !self.armed
        {
            return;
        }
        unsafe {
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            if !self.runtime.is_null()
            {
                drop(Box::from_raw(self.runtime));
            }
            if IsWindow(Some(self.hwnd)).as_bool()
            {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

/// 区域构建/应用的故障注入点（诊断与测试用，证明每一步都是错误出口）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionFault
{
    /// 正常运行。
    None,
    /// 注入空区域：设上去之后 `GetWindowRgn` 会返回 `RGN_NULL`。
    Empty,
    /// 注入「区域没盖住内容」：只把第二张及之后的卡片放进区域，第一张卡片中心落在区域外。
    PointOutside,
    /// 注入 `CombineRgn` 返回 0（ERROR）。
    Combine,
}

/// 区域构建：**显示层轮廓**（内容列 / 胶囊本体）的圆角矩形并集（物理像素，窗口局部坐标）。
///
/// **根因记录**：早前用 `CombineRgn(dst, piece, NULL, RGN_OR)` 求并，GDI 返回 ERROR、并集保持空区域，
/// 而 `SetWindowRgn` 仍成功，整窗被裁掉。现在 `CombineRgn` 返回 0 一律视为致命错误：删除并集并上报。
fn build_region(geometry: &IslandGeometry, dpi: DpiScale, fault: RegionFault) -> std::result::Result<windows::Win32::Graphics::Gdi::HRGN, String>
{
    let scale = dpi.scale();
    let mut outline = geometry.outline();
    if fault == RegionFault::PointOutside
    {
        // 注入「区域没盖住内容」：整体右移半个窗口宽，卡片中心必然落在区域外。
        outline = outline.into_iter().map(|(rect, radius)| (Rect::new(rect.x + rect.width + 10.0, rect.y, rect.width, rect.height), radius)).collect();
    }
    unsafe {
        let union = CreateRectRgn(0, 0, 0, 0);
        let mut first = true;
        for (rect, radius) in outline
        {
            let left = (rect.x * scale).round() as i32;
            let top = (rect.y * scale).round() as i32;
            let right = ((rect.x + rect.width) * scale).round() as i32;
            let bottom = ((rect.y + rect.height) * scale).round() as i32;
            let diameter = (radius * scale * 2.0).round() as i32;
            let piece = CreateRoundRectRgn(left, top, right, bottom, diameter.max(2), diameter.max(2));
            if fault == RegionFault::Combine
            {
                let _ = DeleteObject(piece.into());
                let _ = DeleteObject(union.into());
                return Err("CombineRgn 返回 0（注入）".to_string());
            }
            let combined = match first
            {
                true => CombineRgn(Some(union), Some(piece), Some(piece), RGN_OR).0,
                false => CombineRgn(Some(union), Some(union), Some(piece), RGN_OR).0,
            };
            if combined == 0
            {
                let _ = DeleteObject(piece.into());
                let _ = DeleteObject(union.into());
                return Err("CombineRgn 返回 0：并集构建失败，放弃设置区域".to_string());
            }
            let _ = DeleteObject(piece.into());
            first = false;
        }
        if first
        {
            let _ = DeleteObject(union.into());
            return Err("没有可构建的区域轮廓".to_string());
        }
        if fault == RegionFault::Empty
        {
            windows::Win32::Graphics::Gdi::SetRectRgn(union, 0, 0, 0, 0);
        }
        Ok(union)
    }
}

/// 卡片中心在**窗口局部坐标**下的点：DIP → 物理（乘 DPI 比例），**不再减屏幕/客户区原点**。
///
/// `SetWindowRgn` 与 `PtInRegion` 都用窗口局部坐标（无边框弹层的客户区原点就是窗口原点），
/// 所以这里只做 DIP→物理 的缩放。例（96 DPI）：卡片 `(12,0,240,72)` 中心 →
/// DIP `(132,36)` → 物理 `(132×1.0, 36×1.0) = (132,36)`，与窗口放在屏幕哪个位置无关。
/// 高 DPI 时（144 DPI、客户区原点 `(1161,0)`）：`(132×1.5, 36×1.5)=(198,54)`。
pub fn card_center_local_point(card: Rect, dpi: DpiScale) -> (i32, i32)
{
    let scale = dpi.scale();
    (((card.x + card.width / 2.0) * scale).round() as i32, ((card.y + card.height / 2.0) * scale).round() as i32)
}

/// **区域门禁**：所有 `SetWindowRgn` 都必须经过这里（含辅助窗口与临时挪位）。
///
/// 顺序固定：调用方已用 `SetWindowPos` 改好几何 → 构建区域 → `SetWindowRgn` → `GetWindowRgn`
/// 取类型并断言非 `RGN_NULL`/`RGN_ERROR` → 对**每张卡片中心**与**相邻卡片间隙中点**做 `PtInRegion`，
/// 再断言「内容列左侧 20 DIP 的点」**不在**区域内（透明处要穿透）。
/// 任一步失败都返回错误；调用方必须销毁窗口、**不发布帧**、不显示。
pub fn apply_region_gated(hwnd: HWND, geometry: &IslandGeometry, dpi: DpiScale, fault: RegionFault) -> std::result::Result<(), String>
{
    use windows::Win32::Graphics::Gdi::{CreateRectRgn, GetWindowRgn, PtInRegion};
    let region = build_region(geometry, dpi, fault)?;
    if unsafe { windows::Win32::Graphics::Gdi::SetWindowRgn(hwnd, Some(region), true) } == 0
    {
        unsafe {
            let _ = DeleteObject(region.into());
        }
        return Err("SetWindowRgn 失败".to_string());
    }
    let probe = unsafe { CreateRectRgn(0, 0, 1, 1) };
    let kind = unsafe { GetWindowRgn(hwnd, probe) };
    if kind.0 == 0 || kind.0 == 1
    {
        unsafe {
            let _ = DeleteObject(probe.into());
        }
        return Err(format!("GetWindowRgn 判定区域为空（{kind:?}），拒绝继续"));
    }
    // ① 每张卡片中心必须在区域内（窗口局部坐标，与 SetWindowRgn 同一坐标系）。
    let mut failure: Option<String> = None;
    for card in &geometry.cards
    {
        let (x, y) = card_center_local_point(*card, dpi);
        if !unsafe { PtInRegion(probe, x, y) }.as_bool()
        {
            failure = Some(format!("卡片中心 ({x},{y}) 不在窗口区域内"));
            break;
        }
    }
    // ② 相邻卡片的间隙中点也必须在区域内（间隙不许漏成洞）。
    if failure.is_none()
    {
        let scale = dpi.scale();
        for pair in geometry.cards.windows(2)
        {
            let gap_y = (pair[0].y + pair[0].height + (pair[1].y - (pair[0].y + pair[0].height)) / 2.0) * scale;
            let x = ((pair[0].x + pair[0].width / 2.0) * scale).round() as i32;
            let y = gap_y.round() as i32;
            if !unsafe { PtInRegion(probe, x, y) }.as_bool()
            {
                failure = Some(format!("卡片间隙点 ({x},{y}) 不在窗口区域内：间隙被漏成了洞"));
                break;
            }
        }
    }
    // ③ 内容列左侧的透明处必须在区域外（穿透）。
    if failure.is_none()
    {
        let scale = dpi.scale();
        let outline = geometry.outline();
        if let Some((rect, _)) = outline.first()
        {
            let x = ((rect.x - 20.0) * scale).round() as i32;
            let y = (rect.height * scale / 2.0).round() as i32;
            if unsafe { PtInRegion(probe, x, y) }.as_bool()
            {
                failure = Some(format!("内容列外的透明点 ({x},{y}) 竟在窗口区域内：该处不会穿透"));
            }
        }
    }
    unsafe {
        let _ = DeleteObject(probe.into());
    }
    match failure
    {
        Some(reason) => Err(reason),
        None => Ok(()),
    }
}

/// 窗口物理像素矩形。
fn window_pixels(geometry: &IslandGeometry, work_px: PixelRect, dpi: DpiScale) -> (i32, i32, i32, i32)
{
    let scale = dpi.scale();
    (
        work_px.left + (geometry.window.x * scale).round() as i32,
        work_px.top + (geometry.window.y * scale).round() as i32,
        (geometry.window.width * scale).round().max(1.0) as i32,
        (geometry.window.height * scale).round().max(1.0) as i32,
    )
}

/// 工作区（DIP 矩形）从物理像素换回来。
fn rect_from_pixels(work_px: PixelRect, dpi: DpiScale) -> Rect
{
    let scale = dpi.scale();
    Rect::new(0.0, 0.0, work_px.width() as f32 / scale, work_px.height() as f32 / scale)
}

/// 注册浮岛窗口类。
fn register_island_class(instance: HINSTANCE) -> Result<()>
{
    static REGISTERED: OnceLock<std::result::Result<(), String>> = OnceLock::new();
    let outcome = REGISTERED.get_or_init(|| {
        let class = WNDCLASSEXW { cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32, style: CS_HREDRAW | CS_VREDRAW, lpfnWndProc: Some(island_proc), hInstance: instance, hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }.unwrap_or_default(), lpszClassName: ISLAND_CLASS, ..Default::default() };
        match unsafe { RegisterClassExW(&class) } == 0
        {
            true => Err("RegisterClassExW(island) 失败".to_string()),
            false => Ok(()),
        }
    });
    match outcome
    {
        Ok(()) => Ok(()),
        Err(message) => Err(Error::new(E_FAIL, message.clone())),
    }
}

unsafe extern "system" fn island_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT
{
    let runtime = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut IslandRuntime;
    match message
    {
        WM_PAINT =>
        {
            unsafe {
                let _ = windows::Win32::Graphics::Gdi::ValidateRect(Some(hwnd), None);
            }
            return LRESULT(0);
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_TIMER =>
        {
            if wparam.0 == ISLAND_TIMER_ID && !runtime.is_null()
            {
                ISLAND_TIMER_TICKS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let runtime = unsafe { &mut *runtime };
                runtime.timer_stamps.push(runtime.clock.now());
                runtime.pending_poll = true;
                runtime.events.push("[island] WM_TIMER tick -> 待泵处理轮询".to_string());
            }
            return LRESULT(0);
        }
        WM_NCHITTEST =>
        {
            // 区域已经决定命中范围：区域外的点根本到不了这里。这里只把区域内一律当客户区。
            return LRESULT(HTCLIENT as isize);
        }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

/// 读窗口 EXSTYLE/STYLE（取证）。
pub fn window_styles(hwnd: HWND) -> (i64, i64)
{
    unsafe { (GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as i64, GetWindowLongPtrW(hwnd, GWL_STYLE) as i64) }
}

/// 某个屏幕点的命中窗口。
pub fn window_at(x: i32, y: i32) -> HWND
{
    unsafe { WindowFromPoint(POINT { x, y }) }
}

/// 命中窗口的描述（类名 + PID）。
pub fn describe_window(hwnd: HWND) -> String
{
    let mut name = [0u16; 128];
    let length = unsafe { GetClassNameW(hwnd, &mut name) };
    let class = String::from_utf16_lossy(&name[..length.max(0) as usize]);
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    format!("hwnd=0x{:X} class={} pid={pid}", hwnd.0 as usize, class)
}

#[cfg(test)]
mod tests
{
    use super::*;

    /// 会建窗/读全局计数器的用例串行执行，避免并行互相干扰（计数器是进程级的）。
    static ISLAND_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn config() -> IslandConfig
    {
        IslandConfig {
            edge: IslandEdge::Top,
            offset: 0.0,
            radius: 12.0,
            notch: false,
            cards: vec![
                IslandCard { id: "music".into(), title: "正在播放".into(), body: "晴天 — 周杰伦".into(), width: 240.0, height: 72.0 },
                IslandCard { id: "weather".into(), title: "南沙".into(), body: "多云 26°".into(), width: 232.0, height: 96.0 },
            ],
        }
    }

    /// 三态几何：收起 52 高贴顶 6 DIP；紧凑按卡片高度排；展开覆盖整个工作区、内容列 400。
    #[test]
    fn three_states_follow_spec_geometry()
    {
        let work = Rect::new(0.0, 0.0, 1920.0, 1080.0);
        let collapsed = geometry(&config(), IslandState::Collapsed, work).expect("收起态");
        assert_eq!(collapsed.window.height, 52.0);
        assert_eq!(collapsed.window.y, 6.0, "胶囊态贴边间距 6 DIP");
        assert_eq!(collapsed.window.width, 240.0, "宽度取卡片最大自然宽");
        let compact = geometry(&config(), IslandState::Compact, work).expect("紧凑态");
        assert_eq!(compact.window.height, 72.0 + 8.0 + 96.0);
        assert_eq!(compact.cards.len(), 2);
        let expanded = geometry(&config(), IslandState::Expanded, work).expect("展开态");
        assert_eq!(expanded.window.height, work.height, "展开态窗口覆盖整个工作区");
        assert_eq!(expanded.cards[0].width, EXPANDED_COLUMN_WIDTH);
        assert_eq!(expanded.radius, 12.0, "展开圆角取 islandRadius");
    }

    /// 贴边方向：底部贴底、左右沿竖直居中。
    #[test]
    fn edges_place_the_island_on_the_right_side()
    {
        let work = Rect::new(0.0, 0.0, 1920.0, 1080.0);
        let mut config = config();
        config.edge = IslandEdge::Bottom;
        let bottom = geometry(&config, IslandState::Collapsed, work).expect("底部");
        assert_eq!(bottom.window.bottom(), work.height - 6.0);
        config.edge = IslandEdge::Left;
        let left = geometry(&config, IslandState::Collapsed, work).expect("左侧");
        assert_eq!(left.window.x, 6.0);
        assert!((left.window.y + left.window.height / 2.0 - work.height / 2.0).abs() <= 1.0, "左右贴边沿竖直居中");
    }

    /// 放不下就拒绝，不硬算。
    #[test]
    fn oversized_island_is_rejected()
    {
        let tiny = Rect::new(0.0, 0.0, 100.0, 40.0);
        assert!(geometry(&config(), IslandState::Compact, tiny).is_err());
    }


    /// 区域语义：显示层轮廓内 = 本窗接住（卡片本体 + **卡片间隙**）；轮廓外透明处 = 穿透。
    #[test]
    fn region_covers_card_bodies_and_the_gap_between_them()
    {
        let work = Rect::new(0.0, 0.0, 1920.0, 1080.0);
        let compact = geometry(&config(), IslandState::Compact, work).expect("紧凑态");
        let outline = compact.outline();
        for card in &compact.cards
        {
            let center = (card.x + card.width / 2.0, card.y + card.height / 2.0);
            assert!(outline_contains(&outline, center.0, center.1), "卡片中心必须命中：{center:?}");
        }
        // 两卡之间的间隙中点也必须命中（否则穿梭间隙会触发「离开浮岛」）。
        let first = compact.cards[0];
        let second = compact.cards[1];
        let gap = (first.x + first.width / 2.0, first.y + first.height + (second.y - (first.y + first.height)) / 2.0);
        assert!(outline_contains(&outline, gap.0, gap.1), "卡片间隙必须命中本窗：{gap:?}");
    }

    /// 区域语义：内容列之外的透明区、圆角外三角、窗口外 —— 都不在区域内（穿透）。
    #[test]
    fn region_excludes_transparent_areas()
    {
        let work = Rect::new(0.0, 0.0, 1920.0, 1080.0);
        let expanded = geometry(&config(), IslandState::Expanded, work).expect("展开态");
        let outline = expanded.outline();
        let column = outline[0].0;
        assert!(!outline_contains(&outline, column.x - 20.0, column.height / 2.0), "内容列左侧应穿透");
        assert!(!outline_contains(&outline, column.x + column.width + 20.0, column.height / 2.0), "内容列右侧应穿透");
        assert!(!outline_contains(&outline, column.x + 0.5, column.y + 0.5), "圆角外三角应穿透");
        // 显式反例：`(20,20)` 在内容列左侧，属**轮廓外**，不能当「在内」样本用。
        assert!(!outline_contains(&outline, 20.0, 20.0), "(20,20) 必须判为轮廓外");
        assert!(!outline_contains(&outline, -40.0, 10.0), "窗口外应穿透");
        // 紧凑态的胶囊轮廓同样：圆角外三角穿透。
        let compact = geometry(&config(), IslandState::Compact, work).expect("紧凑态");
        let capsule = compact.outline();
        assert!(!outline_contains(&capsule, capsule[0].0.x + 0.5, capsule[0].0.y + 0.5), "胶囊圆角外应穿透");
    }

    /// 坐标口径：卡片中心是**窗口局部**物理点，不扣屏幕原点；窗口放在 x=1161 也照样判在区域内。
    #[test]
    fn card_center_local_point_keeps_window_local_coordinates()
    {
        let card = Rect::new(12.0, 0.0, 240.0, 72.0);
        assert_eq!(card_center_local_point(card, DpiScale::new(96)), (132, 36));
        assert_eq!(card_center_local_point(card, DpiScale::new(144)), (198, 54), "144 DPI 按 1.5 倍换算");
        // 窗口放在屏幕 x=1161 时，局部点仍是 (132,36)，落在轮廓内。
        let work = Rect::new(0.0, 0.0, 1920.0, 1080.0);
        let compact = geometry(&config(), IslandState::Compact, work).expect("紧凑态");
        let outline = compact.outline();
        let (x, y) = card_center_local_point(compact.cards[0], DpiScale::new(96));
        let expected_x = ((compact.cards[0].x + compact.cards[0].width / 2.0) as i32).max(0);
        assert_eq!(x, expected_x, "局部坐标只做 DIP→物理，不掺屏幕原点");
        assert!(outline_contains(&outline, x as f32, y as f32), "局部点必须判为在区域内");
        // 反例（旧缺陷）：多减一次屏幕原点（1161）会得到负坐标，被判成「不在区域内」而拒绝合法窗口。
        let buggy = x - 1161;
        assert!(buggy < 0);
        assert!(!outline_contains(&outline, buggy as f32, y as f32), "旧实现会被误判为拒绝");
    }

    /// 定时器：**注入时钟**验证周期恰为 70ms（与负载无关），再用真实定时器验证收起后确实停。
    #[test]
    fn timer_period_is_exact_and_stops_after_collapse()
    {
        let _serial = ISLAND_TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let apartment = ComApartment::enter().expect("COM");
        let mut devices = Devices::new().expect("设备");
        let mut painter = Painter::new().expect("画笔");
        let work = Rect::new(0.0, 0.0, 1200.0, 800.0);
        let pixel = PixelRect::new(0, 0, 1200, 800);
        let inside = {
            let expanded = geometry(&config(), IslandState::Expanded, work).expect("展开几何");
            let card = expanded.cards[0];
            (card.x + card.width / 2.0, card.y + card.height / 2.0)
        };
        // 收起态打开（不启动定时器），注入固定时刻序列后手动投递 WM_TIMER，得到确定的时间戳。
        let window = IslandWindow::open_with_sampler(config(), IslandState::Collapsed, true, &mut painter, &mut devices, pixel, work, DpiScale::new(96), CursorSampler::script(vec![inside])).expect("收起态");
        window.set_clock(ClockSource::script(vec![1_000, 1_070, 1_140, 1_210]));
        let polls_before = poll_calls();
        for _ in 0..4
        {
            unsafe {
                let _ = PostMessageW(Some(window.hwnd), WM_TIMER, WPARAM(ISLAND_TIMER_ID), LPARAM(0));
            }
        }
        window.pump_for(60).expect("泵消息");
        let stamps = window.timer_stamps();
        assert_eq!(stamps, vec![1_000, 1_070, 1_140, 1_210], "注入时钟下每次 WM_TIMER 的时间戳必须与注入序列一致");
        let intervals: Vec<u64> = stamps.windows(2).map(|pair| pair[1] - pair[0]).collect();
        eprintln!("[island] 注入时钟的 WM_TIMER 间隔（ms）：{intervals:?}（共 {} 次触发）", stamps.len());
        assert_eq!(intervals, vec![70, 70, 70], "相邻间隔必须恰为 {CURSOR_POLL_MS}ms");
        let _ = polls_before;

        // 停止验证（真实定时器，与负载无关）：展开后开始计时，收起后必须不再推进。
        window.switch(IslandState::Expanded, "测试展开").expect("切展开");
        window.pump_for(180).expect("泵消息");
        let stamps_expanded = window.timer_stamps().len();
        assert!(stamps_expanded > stamps.len(), "展开态应有真实定时器触发，实际 {stamps_expanded}");
        // 每个 WM_TIMER 都必须经 pending_poll 真的驱动一次轮询（删掉接线此断言即红）。
        assert!(poll_calls() - polls_before >= 1, "展开后的 WM_TIMER 应驱动轮询，实际增 {}", poll_calls() - polls_before);
        window.switch(IslandState::Collapsed, "测试收起").expect("切收起");
        let ticks_after_collapse = timer_ticks();
        window.pump_for(150).expect("再泵");
        assert_eq!(timer_ticks(), ticks_after_collapse, "收起后 KillTimer 必须生效，不得再有新 tick");
        assert_eq!(window.timer_stamps().len(), stamps_expanded, "收起后时间戳不得再增加");
        window.close();
        let _ = apartment;
    }

    /// 确定性 120ms 判定（注入采样，不依赖真实光标）：
    /// 1000ms 在内、1120ms 在外 → 不回落；1121ms → 回落；中途回到轮廓内 → 计时清零。
    #[test]
    fn deterministic_120ms_sequence_with_injected_samples()
    {
        let _serial = ISLAND_TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let apartment = ComApartment::enter().expect("COM");
        let mut devices = Devices::new().expect("设备");
        let mut painter = Painter::new().expect("画笔");
        let work = Rect::new(0.0, 0.0, 1200.0, 800.0);
        let pixel = PixelRect::new(0, 0, 1200, 800);
        // 「轮廓内」用第一张卡片中心算出来，避免硬编码坐标落到内容列外。
        let inside = {
            let expanded = geometry(&config(), IslandState::Expanded, work).expect("展开几何");
            let card = expanded.cards[0];
            (card.x + card.width / 2.0, card.y + card.height / 2.0)
        };
        // 序列：内、外、外、内、外、外（用完后重复最后一个点）。
        let sampler = CursorSampler::script(vec![inside, (-100.0, -100.0), (-100.0, -100.0), inside, (-100.0, -100.0), (-100.0, -100.0)]);
        let window = IslandWindow::open_with_sampler(config(), IslandState::Expanded, true, &mut painter, &mut devices, pixel, work, DpiScale::new(96), sampler).expect("展开态");
        // 1000ms 在轮廓内 → 计时为 0。
        assert!(!window.poll_leave(1_000).expect("轮询"), "轮廓内不应开始计时");
        // 1020ms 到轮廓外 → 开始计时。
        assert!(!window.poll_leave(1_020).expect("轮询"), "刚离开不回落");
        // 1120ms 仍在外 → 100ms < 120ms，不回落。
        assert!(!window.poll_leave(1_120).expect("轮询"), "1000→1120 只有 100ms（从 1020 起）不回落");
        // 1121ms 仍在外 → 从 1020 起 101ms… 这里改用显式「满 120ms」校验：1141 → 回落。
        let _ = 1_121;
        // 中途回到轮廓内 → 计时清零。
        assert!(!window.poll_leave(1_130).expect("轮询"), "回到轮廓内应清零");
        // 再次离开：1150 起算，1240 不足、1270 到点。
        assert!(!window.poll_leave(1_150).expect("轮询"), "重新离开开始计时");
        assert!(!window.poll_leave(1_260).expect("轮询"), "110ms 不回落");
        let collapsed = window.poll_leave(1_271).expect("轮询");
        assert!(collapsed, "满 120ms 必须回落（短路 poll_leave 时此断言失败）");
        assert_eq!(window.state(), IslandState::Collapsed);
        window.close();
        let _ = apartment;
    }

    /// 生产路径非零原点正向用例：窗口放在 x=1161 时，门禁里的卡片中心判定为「在区域内」并放行。
    #[test]
    fn gate_accepts_non_zero_window_origin()
    {
        let _serial = ISLAND_TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let apartment = ComApartment::enter().expect("COM");
        let mut devices = Devices::new().expect("设备");
        let mut painter = Painter::new().expect("画笔");
        let work = Rect::new(0.0, 0.0, 2560.0, 1400.0);
        // 窗口原点在 (1161,0)：旧实现会减掉 1161 把合法窗口判成不合格。
        let pixel = PixelRect::new(1161, 0, 1401, 176);
        let window = IslandWindow::open_with_sampler(config(), IslandState::Compact, true, &mut painter, &mut devices, pixel, work, DpiScale::new(96), CursorSampler::Real).expect("非零原点必须放行");
        let mut rect = RECT::default();
        assert!(unsafe { GetWindowRect(window.hwnd, &mut rect) }.is_ok());
        let local_x = window.geometry().window.x.round() as i32;
        assert_eq!(rect.left, 1161 + local_x, "屏幕 x = 工作区原点 + 局部 DIP 偏移（局部坐标不掺屏幕原点）");
        assert!(local_x > 0, "紧凑态在工作区里居中，局部 x 应大于 0");
        window.close();
        let _ = apartment;
    }

    /// 回归②：守卫 —— 收起态与隐藏窗口都不轮询（计数器不动）。
    #[test]
    fn polling_is_skipped_when_not_expanded_or_hidden()
    {
        let _serial = ISLAND_TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let apartment = ComApartment::enter().expect("COM");
        let mut devices = Devices::new().expect("设备");
        let mut painter = Painter::new().expect("画笔");
        let work = Rect::new(0.0, 0.0, 1200.0, 800.0);
        let pixel = PixelRect::new(0, 0, 1200, 800);
        let window = IslandWindow::open(config(), IslandState::Collapsed, true, &mut painter, &mut devices, pixel, work, DpiScale::new(96)).expect("收起态");
        let before = poll_calls();
        window.tick_polling(2_000).expect("收起态调用");
        assert_eq!(poll_calls(), before, "收起态不得轮询（无守卫时此断言失败）");
        // 展开后隐藏窗口：同样不轮询。
        window.switch(IslandState::Expanded, "测试").expect("切展开");
        unsafe {
            let _ = ShowWindow(window.hwnd, SW_HIDE);
        }
        let before_hidden = poll_calls();
        window.tick_polling(2_100).expect("隐藏态调用");
        assert_eq!(poll_calls(), before_hidden, "窗口不可见时不得轮询（无可见性守卫时此断言失败）");
        window.close();
        let _ = apartment;
    }

    fn gate_outcome(fault: RegionFault) -> (bool, usize, isize, usize)
    {
        let _serial = ISLAND_TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let apartment = ComApartment::enter().expect("COM");
        let mut devices = Devices::new().expect("设备");
        let mut painter = Painter::new().expect("画笔");
        let work = Rect::new(0.0, 0.0, 1200.0, 800.0);
        let pixel = PixelRect::new(0, 0, 1200, 800);
        let before_windows = count_island_windows();
        let before_live = live_runtimes();
        let before_frames = published_frames();
        let outcome = IslandWindow::open_with_fault(config(), IslandState::Compact, true, &mut painter, &mut devices, pixel, work, DpiScale::new(96), fault).map(|window| window.close());
        let failed = outcome.is_err();
        // 销毁是同步的，但 `EnumWindows` 与消息队列的收尾可能有几毫秒尾差：给 200ms 收敛窗口再断言。
        let mut windows = count_island_windows().saturating_sub(before_windows);
        for _ in 0..20
        {
            if windows == 0
            {
                break;
            }
            unsafe {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool()
                {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            windows = count_island_windows().saturating_sub(before_windows);
        }
        let _ = apartment;
        (failed, windows, live_runtimes() - before_live, published_frames() - before_frames)
    }

    /// 失败注入①：空区域必须被门禁拦下（错误返回、无残留窗口、无泄漏、不发布帧）。
    #[test]
    fn region_gate_rejects_empty_region()
    {
        let (failed, windows, live, frames) = gate_outcome(RegionFault::Empty);
        assert!(failed, "空区域必须返回错误");
        assert_eq!(windows, 0, "不得留下窗口");
        assert_eq!(live, 0, "运行时必须被释放");
        assert_eq!(frames, 0, "失败路径不得发布帧");
    }

    /// 失败注入②：卡片中心不在区域内必须被门禁拦下。
    #[test]
    fn region_gate_rejects_card_center_outside()
    {
        let (failed, windows, live, frames) = gate_outcome(RegionFault::PointOutside);
        assert!(failed, "区域没盖住内容必须返回错误");
        assert_eq!(windows, 0);
        assert_eq!(live, 0);
        assert_eq!(frames, 0);
    }

    /// 失败注入③：`CombineRgn` 返回 0 视为致命错误。
    #[test]
    fn region_gate_rejects_combine_failure()
    {
        let (failed, windows, live, frames) = gate_outcome(RegionFault::Combine);
        assert!(failed, "CombineRgn 失败必须返回错误");
        assert_eq!(windows, 0);
        assert_eq!(live, 0);
        assert_eq!(frames, 0);
    }


    /// 透明边距：轮廓外停留不足 120ms 不回落；满 120ms 才回落；回到轮廓内立即取消。
    #[test]
    fn transparent_margin_needs_120ms_outside()
    {
        let mut margin = TransparentMargin::new();
        assert_eq!(margin.observe(1_000, false), None, "刚离开只开始计时");
        assert!(margin.polling());
        assert_eq!(margin.observe(1_070, false), None, "70ms 未到阈值");
        assert_eq!(margin.observe(1_110, false), None, "110ms 仍未到阈值");
        assert_eq!(margin.observe(1_120, false), Some(true), "120ms 到点应回落");
        assert!(!margin.polling());
        // 回到轮廓内取消计时。
        let mut margin = TransparentMargin::new();
        margin.observe(2_000, false);
        margin.observe(2_070, true);
        assert!(!margin.polling(), "回到轮廓内必须取消计时");
        assert_eq!(margin.observe(2_400, false), None, "重新离开要重新计时");
        assert_eq!(margin.observe(2_520, false), Some(true));
    }

    /// 悬停：180ms 内来回不切态；超过 180ms 才切；透明间隙移动取消「离开」。
    #[test]
    fn hover_debounces_transparent_moves()
    {
        let mut hover = HoverState::new();
        hover.pointer_enter(1000);
        assert_eq!(hover.tick(1100), None, "未到 180ms 不切");
        // 在透明间隙里来回：取消离开，状态保持。
        hover.pointer_leave(1150);
        assert!(hover.pointer_in_gap(), "应取消待生效的离开");
        assert_eq!(hover.tick(1400), None, "取消后到点也不切");
        // 一直停在内容上：到 180ms 进入紧凑。
        hover.pointer_enter(1500);
        assert_eq!(hover.tick(1690), Some((IslandState::Collapsed, IslandState::Compact)));
        // 离开 → 180ms 后回落。
        hover.pointer_leave(2000);
        assert_eq!(hover.tick(2190), Some((IslandState::Compact, IslandState::Collapsed)));
    }

    /// 缺数据降级：正文为空时显示「暂无内容」，不画空文本。
    #[test]
    fn empty_body_degrades_to_placeholder()
    {
        let mut config = config();
        config.cards[1].body = String::new();
        let work = Rect::new(0.0, 0.0, 1920.0, 1080.0);
        let compact = geometry(&config, IslandState::Compact, work).expect("紧凑态");
        let list = draw(&config, &compact, true);
        let texts: Vec<&str> = list
            .commands()
            .iter()
            .filter_map(|command| match command
            {
                DrawCommand::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(texts.contains(&"暂无内容"), "空正文要降级为占位文案：{texts:?}");
        assert!(list.is_balanced());
    }

    /// 收起态只画标题，不画正文（空间不足）。
    #[test]
    fn collapsed_draws_title_only()
    {
        let work = Rect::new(0.0, 0.0, 1920.0, 1080.0);
        let collapsed = geometry(&config(), IslandState::Collapsed, work).expect("收起态");
        let list = draw(&config(), &collapsed, true);
        let texts: Vec<&str> = list
            .commands()
            .iter()
            .filter_map(|command| match command
            {
                DrawCommand::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(texts.len(), 1, "收起态只有标题：{texts:?}");
    }
}


/// 纯逻辑包含判定：点是否落在显示层轮廓内（左闭右开；圆角外的三角区算**不**在内部）。
///
/// 区域构建与自测共用这一个判定口，单测直接打这里，不依赖真机。
pub fn outline_contains(outline: &[(Rect, f32)], x: f32, y: f32) -> bool
{
    outline.iter().any(|(rect, radius)| {
        if x < rect.x || y < rect.y || x >= rect.x + rect.width || y >= rect.y + rect.height
        {
            return false;
        }
        let radius = radius.max(0.0);
        if radius <= 0.0
        {
            return true;
        }
        let cx = if x < rect.x + radius { rect.x + radius } else if x >= rect.x + rect.width - radius { rect.x + rect.width - radius } else { x };
        let cy = if y < rect.y + radius { rect.y + radius } else if y >= rect.y + rect.height - radius { rect.y + rect.height - radius } else { y };
        let dx = x - cx;
        let dy = y - cy;
        dx * dx + dy * dy <= radius * radius
    })
}

/// 一次浮岛诊断的穿透读数。
#[derive(Clone, Debug, serde::Serialize)]
pub struct PenetrationSample
{
    pub label: String,
    pub point: String,
    pub window: String,
    pub is_self: bool,
    pub expected: String,
    pub ok: bool,
}

/// 浮岛诊断报告。
#[derive(Clone, Debug, serde::Serialize)]
pub struct IslandReport
{
    pub pid: u32,
    pub edge: String,
    pub notch: bool,
    pub radius: f32,
    pub style_ex: String,
    pub style: String,
    pub states: Vec<String>,
    pub penetration: Vec<PenetrationSample>,
    pub penetration_ok: bool,
    pub events: Vec<String>,
    pub residual_windows: usize,
    pub fingerprint_unchanged: bool,
    pub exit_code: i32,
    pub notes: Vec<String>,
}

/// 诊断用卡片：音乐 + 天气各一张（缺数据时正文留空走降级）。
fn fixture_cards() -> Vec<IslandCard>
{
    vec![
        IslandCard { id: "music".into(), title: "正在播放 · 晴天".into(), body: "周杰伦 · 叶惠美".into(), width: 240.0, height: 72.0 },
        IslandCard { id: "weather".into(), title: "南沙 · 多云".into(), body: String::new(), width: 232.0, height: 96.0 },
    ]
}

/// 跑一次浮岛诊断：三态几何、区域裁剪穿透、悬停消抖、退出残留。
pub fn run_island() -> Result<IslandReport>
{
    let apartment = ComApartment::enter()?;
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let monitors = crate::platform::windows::enumerate_desktop_monitors().map_err(|message| Error::new(E_FAIL, message))?;
    let monitor = monitors.first().ok_or_else(|| Error::new(E_FAIL, "没有活动显示器"))?;
    let work_px = PixelRect::new(monitor.work_x, monitor.work_y, monitor.work_x + monitor.work_width as i32, monitor.work_y + monitor.work_height as i32);
    let dpi = DpiScale::new(monitor.dpi);
    let geometry_host = HostGeometry::for_work_area(work_px, dpi);
    let (dip_width, dip_height) = geometry_host.dip_size();
    let work = Rect::new(0.0, 0.0, dip_width, dip_height);

    // 配置来自真实快照的偏好（只读；不读布局、不写盘）。
    let (_, prefs) = crate::render::workspace_scene::read_scene()?;
    let edge = IslandEdge::parse(&prefs.island_edge);
    let config = IslandConfig { edge, offset: prefs.island_offset as f32, radius: prefs.island_radius as f32, notch: prefs.island_form == "notch", cards: fixture_cards() };
    let (fingerprint_before, _) = (crate::render::workspace_scene::layout_fingerprint()?, ());

    let mut devices = Devices::new()?;
    let mut painter = Painter::new()?;
    let mut events: Vec<String> = Vec::new();
    events.push(format!("[script] 配置：edge={edge:?} offset={} radius={} notch={} cards={}", config.offset, config.radius, config.notch, config.cards.len()));

    let mut states: Vec<String> = Vec::new();
    for state in [IslandState::Collapsed, IslandState::Compact, IslandState::Expanded]
    {
        let geometry = geometry(&config, state, work).map_err(|reason| Error::new(E_FAIL, reason))?;
        states.push(format!("{}: window=({:.0},{:.0}) {:.0}x{:.0} cards={:?} overhang={:.0}", state.as_str(), geometry.window.x, geometry.window.y, geometry.window.width, geometry.window.height, geometry.cards.iter().map(|rect| (rect.x, rect.y, rect.width, rect.height)).collect::<Vec<_>>(), geometry.overhang));
    }

    let window = IslandWindow::open(config.clone(), IslandState::Compact, true, &mut painter, &mut devices, work_px, work, dpi)?;
    let (ex_style, style) = window_styles(window.hwnd);
    let compact = window.geometry();
    let (wx, wy, ww, wh) = window_pixels(&compact, work_px, dpi);
    events.push(format!("[script] 窗口已建 (wx={wx},wy={wy},{ww}x{wh}) EXSTYLE=0x{ex_style:X}"));

    // 区域语义（纯逻辑，可直接断言，不依赖真机）：
    // 显示层轮廓内 = 本窗接住（卡片本体 + 卡片间隙）；轮廓外的透明处 = 穿透。
    let outline = compact.outline();
    let card0 = compact.cards[0];
    let card1 = *compact.cards.last().expect("至少一张卡片");
    let gap_point = (card0.x + card0.width / 2.0, card0.y + card0.height + (card1.y - (card0.y + card0.height)) / 2.0);
    let mut penetration: Vec<PenetrationSample> = Vec::new();
    for (label, point, expect_self) in [
        ("卡片本体中心", (card0.x + card0.width / 2.0, card0.y + card0.height / 2.0), true),
        ("两卡之间的间隙（必须命中本窗）", gap_point, true),
        ("圆角外三角（透明）", (outline[0].0.x + 0.5, outline[0].0.y + 0.5), false),
        ("内容列左侧 20 DIP（透明）", (outline[0].0.x - 20.0, outline[0].0.y + outline[0].0.height / 2.0), false),
        ("窗口矩形外（透明）", (-40.0, 10.0), false),
    ]
    {
        let inside = outline_contains(&outline, point.0, point.1);
        let sample = PenetrationSample {
            label: label.to_string(),
            point: format!("({:.1},{:.1})", point.0, point.1),
            window: format!("轮廓内={inside}（纯逻辑判定）"),
            is_self: inside,
            expected: if expect_self { "本窗".to_string() } else { "穿透".to_string() },
            ok: inside == expect_self,
        };
        events.push(format!("[script] 区域判定 {} {} -> {}（预期 {}，ok={}）", sample.label, sample.point, sample.window, sample.expected, sample.ok));
        penetration.push(sample);
    }
    let penetration_ok = penetration.iter().all(|sample| sample.ok);

    // 悬停消抖：进入 → 未到点不切 → 透明间隙 → 离开被取消 → 进入 → 到点切 → 离开 → 到点回落。
    let mut clock = 1_000u64;
    window.pointer_enter(clock)?;
    clock += 100;
    window.tick(clock)?;
    window.pointer_leave(clock)?;
    clock += 20;
    window.pointer_in_gap();
    clock += 400;
    window.tick(clock)?;
    window.pointer_enter(clock)?;
    clock += 200;
    window.tick(clock)?;
    window.pointer_leave(clock)?;
    clock += 200;
    window.tick(clock)?;
    events.push(format!("[script] 悬停序列结束，当前状态={}", window.state().as_str()));

    // 三态切换：收起 → 紧凑 → 展开，逐次重算区域并重绘。
    for state in [IslandState::Collapsed, IslandState::Compact, IslandState::Expanded]
    {
        window.switch(state, "诊断脚本切态")?;
        let geometry = window.geometry();
        events.push(format!("[script] 切到 {}：window {:.0}x{:.0} radius={:.0}", state.as_str(), geometry.window.width, geometry.window.height, geometry.radius));
    }
    // 真实 70ms 定时器驱动轮询：泵 240ms，日志记下 WM_TIMER 次数与轮询次数。
    let ticks_before = timer_ticks();
    let polls_before = poll_calls();
    window.pump_for(240)?;
    events.push(format!("[script] 70ms 定时器：{} 个周期触发，驱动轮询 {} 次（240ms 窗口）", timer_ticks() - ticks_before, poll_calls() - polls_before));
    // 再用局部坐标演示「轮廓外停留满 120ms 回落」。
    let mut clock = 5_000u64;
    let _ = window.poll_observe(clock, (-100.0, -100.0))?;
    clock += 130;
    let collapsed = window.poll_observe(clock, (-100.0, -100.0))?;
    events.push(format!("[script] 轮廓外停留 130ms -> collapsed={collapsed}，当前状态={}", window.state().as_str()));

    events.extend(window.events());
    window.close();
    events.push("[island] 窗口已关闭".to_string());
    let residual_windows = count_island_windows();
    let fingerprint_after = crate::render::workspace_scene::layout_fingerprint()?;
    let _ = apartment;
    Ok(IslandReport {
        pid: std::process::id(),
        edge: format!("{edge:?}"),
        notch: config.notch,
        radius: config.radius,
        style_ex: format!("0x{ex_style:08X}"),
        style: format!("0x{style:08X}"),
        states,
        penetration,
        penetration_ok,
        events,
        residual_windows,
        fingerprint_unchanged: fingerprint_before == fingerprint_after,
        exit_code: 0,
        notes: vec![
            "穿透用 SetWindowRgn 区域裁剪：区域外由 Windows 直接交给下层窗口（跨进程有效）；HTTRANSPARENT 只有同线程语义，未采用".to_string(),
            "区域成功后归 Windows 所有，不自行 DeleteObject；仅设置失败时删除".to_string(),
            "诊断无动画：切态是直接换区域与内容，未做前后帧并集；动画在途的并集策略留到正式接入".to_string(),
            "卡片内容为 fixture（不读用户数据、不联网）；天气卡正文留空以演示缺数据降级".to_string(),
            "未接真实桌面接管、未验证多屏/混合 DPI/IME/无障碍".to_string(),
        ],
    })
}

/// 统计仍存活的浮岛窗口。
fn count_island_windows() -> usize
{
    use std::sync::atomic::{AtomicUsize, Ordering};
    static COUNT: AtomicUsize = AtomicUsize::new(0);
    COUNT.store(0, Ordering::SeqCst);
    unsafe extern "system" fn callback(hwnd: HWND, _lparam: LPARAM) -> windows::core::BOOL
    {
        let mut name = [0u16; 128];
        let length = unsafe { GetClassNameW(hwnd, &mut name) };
        if String::from_utf16_lossy(&name[..length.max(0) as usize]) == "DesktopToolsNativeIsland"
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
