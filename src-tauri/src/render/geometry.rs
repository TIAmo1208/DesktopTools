//! 几何换算与命中判定的纯逻辑：不依赖任何 Win32 句柄，全部可单测。
//!
//! 约定：
//! - 物理像素矩形使用左闭右开区间，与 Win32 `PtInRect`、`GetMonitorInfoW` 的语义一致；
//! - DIP（96 DPI 下的逻辑像素）与物理像素之间只经 [`DpiScale`] 一处换算，避免各处自己乘除；
//! - DIP → 像素的矩形换算向外取整，宁可多画一个像素也不把内容裁掉。

use serde::{Deserialize, Serialize};

use super::draw::Rect;

/// 物理像素矩形（左闭右开）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PixelRect
{
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl PixelRect
{
    /// 按左上角与右下角构造。
    pub fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self
    {
        Self { left, top, right, bottom }
    }

    /// 按左上角与尺寸构造。
    pub fn from_xywh(x: i32, y: i32, width: i32, height: i32) -> Self
    {
        Self::new(x, y, x + width, y + height)
    }

    /// 宽度（可为 0，不为负）。
    pub fn width(&self) -> i32
    {
        (self.right - self.left).max(0)
    }

    /// 高度（可为 0，不为负）。
    pub fn height(&self) -> i32
    {
        (self.bottom - self.top).max(0)
    }

    /// 是否包含给定点（左闭右开），与 `PtInRect` 一致。
    pub fn contains(&self, x: i32, y: i32) -> bool
    {
        x >= self.left && x < self.right && y >= self.top && y < self.bottom
    }

    /// 是否与另一矩形有交叠（左闭右开，接触不算交叠）。
    pub fn intersects(&self, other: &PixelRect) -> bool
    {
        self.left < other.right && other.left < self.right && self.top < other.bottom && other.top < self.bottom
    }

    /// 平移。
    pub fn offset(&self, dx: i32, dy: i32) -> Self
    {
        Self::new(self.left + dx, self.top + dy, self.right + dx, self.bottom + dy)
    }

    /// 收进外层矩形；内层比外层大时对齐左上角并压到外层尺寸。
    pub fn clamp_into(&self, outer: &PixelRect) -> Self
    {
        let width = self.width().min(outer.width());
        let height = self.height().min(outer.height());
        let left = self.left.clamp(outer.left, (outer.right - width).max(outer.left));
        let top = self.top.clamp(outer.top, (outer.bottom - height).max(outer.top));
        Self::from_xywh(left, top, width, height)
    }
}

/// 显示器或其一部分的 DPI。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DpiScale
{
    /// 有效 DPI（96 的整数倍之外也可能是 120、144 等；125% 缩放即 120）。
    pub dpi: u32,
}

impl DpiScale
{
    /// 以 DPI 构造。
    pub fn new(dpi: u32) -> Self
    {
        Self { dpi: dpi.max(48) }
    }

    /// 以缩放比例构造（1.0 = 96 DPI）。
    pub fn from_scale(scale: f64) -> Self
    {
        Self::new((scale * 96.0).round().clamp(48.0, 960.0) as u32)
    }

    /// 缩放比例（1.0 = 100%）。
    pub fn scale(&self) -> f32
    {
        self.dpi as f32 / 96.0
    }

    /// DIP 长度 → 物理像素长度（四舍五入，正数至少 1 像素）。
    pub fn dip_to_px(&self, dip: f32) -> i32
    {
        let pixels = (dip * self.scale()).round() as i32;
        if dip > 0.0 { pixels.max(1) } else { pixels }
    }

    /// 物理像素长度 → DIP 长度。
    pub fn px_to_dip(&self, pixels: i32) -> f32
    {
        pixels as f32 / self.scale()
    }

    /// DIP 点 → 物理像素点。
    pub fn dip_point_to_px(&self, x: f32, y: f32) -> (i32, i32)
    {
        ((x * self.scale()).round() as i32, (y * self.scale()).round() as i32)
    }

    /// 物理像素点 → DIP 点。
    pub fn px_point_to_dip(&self, x: i32, y: i32) -> (f32, f32)
    {
        (x as f32 / self.scale(), y as f32 / self.scale())
    }
}

/// DIP 矩形 → 物理像素矩形：四边向外取整，避免把 0.5 像素的内容裁掉。
pub fn dip_rect_to_pixels(rect: &Rect, dpi: DpiScale) -> PixelRect
{
    let rect = rect.normalized();
    let scale = dpi.scale();
    let left = (rect.x * scale).floor() as i32;
    let top = (rect.y * scale).floor() as i32;
    let right = ((rect.right()) * scale).ceil() as i32;
    let bottom = ((rect.bottom()) * scale).ceil() as i32;
    PixelRect::new(left, top, right.max(left), bottom.max(top))
}

/// 物理像素矩形 → DIP 矩形。
pub fn pixel_rect_to_dip(rect: &PixelRect, dpi: DpiScale) -> Rect
{
    let scale = dpi.scale();
    Rect::new(rect.left as f32 / scale, rect.top as f32 / scale, rect.width() as f32 / scale, rect.height() as f32 / scale)
}

/// 命中结果：`index` 是传入切片里最上面那个矩形的位置。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit
{
    pub index: usize,
}

/// 命中判定：后绘制的在上层，因此从后往前找第一个包含该点的矩形。
pub fn hit_test_topmost(rects: &[PixelRect], x: i32, y: i32) -> Option<Hit>
{
    rects.iter().rposition(|rect| rect.contains(x, y)).map(|index| Hit { index })
}

/// 一块宿主表面的几何：工作区矩形 + 该处的 DPI。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct HostGeometry
{
    pub work_area: PixelRect,
    pub surface: PixelRect,
    pub dpi: DpiScale,
}

impl HostGeometry
{
    /// 由工作区与 DPI 构造；`surface` 与工作区一致（阶段 1 的宿主覆盖整块工作区）。
    pub fn for_work_area(work_area: PixelRect, dpi: DpiScale) -> Self
    {
        Self { work_area, surface: work_area, dpi }
    }

    /// 表面像素尺寸（DComp 表面必须用物理像素）。
    pub fn surface_size(&self) -> (u32, u32)
    {
        (self.surface.width().max(1) as u32, self.surface.height().max(1) as u32)
    }

    /// DIP 尺寸，供绘制层做布局。
    pub fn dip_size(&self) -> (f32, f32)
    {
        (self.dpi.px_to_dip(self.surface.width()), self.dpi.px_to_dip(self.surface.height()))
    }

    /// 屏幕像素点是否落在本宿主上。
    pub fn contains(&self, x: i32, y: i32) -> bool
    {
        self.surface.contains(x, y)
    }
}
