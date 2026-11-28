//! 把 [`super::draw::DrawCommand`] 落到 Direct2D / DirectWrite。
//!
//! 约定：
//! - 绘制坐标是 DIP；进入绘制前让 Direct2D 按该显示器 DPI 光栅化（文字才清晰），DIP→物理像素由 D2D 完成；
//! - 文本走系统界面字体（中文优先「Microsoft YaHei UI」，再退雅黑、宋体、Segoe UI）；
//! - 文本契约：`max_lines` 超过时按实际布局前 N 行高度裁剪，不补省略号，
//!   行数测量用 `IDWriteTextLayout::GetLineMetrics` 的实际结果；
//! - 文本格式缓存按 (字重, 字号×10 四舍五入, 对齐) 建键，**创建值也用同一个舍入后的字号**，
//!   缓存条目数有上界，避免任意字号常驻积累；
//! - 任何一步失败都必须把本帧已压入的裁剪/图层弹出去，不能给下一帧留脏状态。

use std::collections::HashMap;

use windows::{
    Win32::{
        Foundation::E_FAIL,
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
            Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
        },
    },
    core::{Error, HSTRING, Result, w},
};

use super::draw::{Color, DrawCommand, DrawList, Rect, TextAlign};
use super::geometry::DpiScale;

/// 文本格式缓存上限：超出就整体清空（字体档位本来就少，不做 LRU）。
const MAX_FORMATS: usize = 32;
/// 位图缓存上限：超出就整体清空（桌面项目数量有限，不做 LRU）。
const MAX_BITMAPS: usize = 512;

/// 单帧绘制器：持有 DirectWrite 工厂与按需缓存。
pub struct Painter
{
    write: IDWriteFactory,                               // 文本工厂，跨帧复用。
    family: String,                                      // 解析后的界面字体族。
    formats: HashMap<(u16, u32, u8), IDWriteTextFormat>, // (字重, 字号×10, 对齐) → 文本格式。
    brushes: HashMap<u32, ID2D1SolidColorBrush>,         // 颜色 → 画刷（每帧清空）。
    bitmaps: HashMap<usize, ID2D1Bitmap>,                // 图像身份 → 位图（跨帧复用，有上界）。
    layers: Vec<Option<ID2D1Layer>>,                     // 本帧实际压入的栈（None 是裁剪）。
}

impl Painter
{
    /// 创建文本工厂并解析可用的界面字体。
    pub fn new() -> Result<Self>
    {
        let write: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let family = resolve_ui_family(&write)?;
        Ok(Self { write, family, formats: HashMap::new(), brushes: HashMap::new(), bitmaps: HashMap::new(), layers: Vec::new() })
    }

    /// 实际使用的字体族（诊断与记录用）。
    pub fn family(&self) -> &str { &self.family }

    /// 当前缓存的文本格式数量（诊断与单测用，验证有上界）。
    pub fn cached_formats(&self) -> usize { self.formats.len() }

    /// 按指令序列绘制一帧。
    ///
    /// `clip`：把绘制限制在更新区域内（DIP）；调用方负责在这块区域里先擦出底色。
    /// 栈校验在**任何 GPU 调用之前**完成；绘制中出错时本帧的图层与裁剪都会弹出。
    pub fn paint(&mut self, target: &ID2D1RenderTarget, list: &DrawList, dpi: DpiScale, clip: Option<Rect>) -> Result<()>
    {
        // 1) 提前校验：列表本身不成立就不碰设备。
        list.validate().map_err(|error| Error::new(E_FAIL, format!("绘制列表栈未配对：{error:?}")))?;
        let scale = dpi.scale();
        unsafe {
            target.SetDpi(96.0 * scale, 96.0 * scale);
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        }

        // 2) 必要时压入更新区域裁剪。
        let mut clip_open = false;
        if let Some(rect) = clip
        {
            let d2d = to_d2d(rect);
            unsafe { target.PushAxisAlignedClip(&d2d, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
            clip_open = true;
        }

        // 3) 绘制；失败时仍然要执行第 4 步的收尾。
        let outcome = self.draw(target, list);

        // 4) 统一收尾：图层与裁剪按后进先出弹出，画刷缓存清空。
        while let Some(entry) = self.layers.pop()
        {
            unsafe { if entry.is_some() { target.PopLayer() } else { target.PopAxisAlignedClip() } };
        }
        if clip_open
        {
            unsafe { target.PopAxisAlignedClip() };
        }
        self.brushes.clear();
        outcome
    }

    /// 逐条绘制；错误直接向上抛，收尾由 [`Painter::paint`] 统一负责。
    fn draw(&mut self, target: &ID2D1RenderTarget, list: &DrawList) -> Result<()>
    {
        for command in list.commands()
        {
            match command
            {
                DrawCommand::FillRoundRect { rect, radius, color } =>
                {
                    let brush = self.brush(target, *color)?;
                    let rounded = rounded(rect, *radius);
                    unsafe { target.FillRoundedRectangle(&rounded, &brush) };
                }
                DrawCommand::StrokeRoundRect { rect, radius, width, color } =>
                {
                    let brush = self.brush(target, *color)?;
                    let rounded = rounded(rect, *radius);
                    unsafe { target.DrawRoundedRectangle(&rounded, &brush, (*width).max(0.0), None) };
                }
                DrawCommand::Caption { rect, text, size, weight, color, align } =>
                {
                    if rect.width <= 0. || rect.height <= 0.
                    {
                        continue;
                    }
                    let format = self.format_mode(*size, *weight, *align, true)?;
                    let brush = self.brush(target, *color)?;
                    let utf16: Vec<_> = text.encode_utf16().collect();
                    unsafe {
                        target.DrawText(&utf16, &format, &to_d2d(*rect), &brush, D2D1_DRAW_TEXT_OPTIONS_CLIP, DWRITE_MEASURING_MODE_NATURAL);
                    }
                }
                DrawCommand::Text { rect, text, size, weight, color, align, max_lines } =>
                {
                    if text.is_empty() || rect.width <= 0.0 || rect.height <= 0.0
                    {
                        continue;
                    }
                    let format = self.format(*size, *weight, *align)?;
                    let utf16: Vec<u16> = text.encode_utf16().collect();
                    let layout = unsafe { self.write.CreateTextLayout(&utf16, &format, rect.width, rect.height)? };
                    let height = visible_height(&layout, rect.height, *max_lines)?;
                    let brush = self.brush(target, *color)?;
                    let clip = to_d2d(Rect::new(rect.x, rect.y, rect.width, height));
                    unsafe {
                        target.PushAxisAlignedClip(&clip, D2D1_ANTIALIAS_MODE_ALIASED);
                        target.DrawText(&utf16, &format, &to_d2d(*rect), &brush, D2D1_DRAW_TEXT_OPTIONS_CLIP, DWRITE_MEASURING_MODE_NATURAL);
                        target.PopAxisAlignedClip();
                    }
                }
                DrawCommand::PushClip { rect } =>
                {
                    let clip = to_d2d(*rect);
                    unsafe { target.PushAxisAlignedClip(&clip, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
                    self.layers.push(None);
                }
                DrawCommand::PopClip =>
                {
                    unsafe { target.PopAxisAlignedClip() };
                    self.layers.pop();
                }
                DrawCommand::PushOpacity { opacity } =>
                {
                    let layer = unsafe { target.CreateLayer(None)? };
                    let parameters = D2D1_LAYER_PARAMETERS { contentBounds: infinite_bounds(), opacity: opacity.clamp(0.0, 1.0), ..Default::default() };
                    unsafe { target.PushLayer(&parameters, &layer) };
                    self.layers.push(Some(layer));
                }
                DrawCommand::PopOpacity =>
                {
                    unsafe { target.PopLayer() };
                    self.layers.pop();
                }
                DrawCommand::Image { rect, image } =>
                {
                    if rect.width <= 0.0 || rect.height <= 0.0
                    {
                        continue;
                    }
                    let bitmap = self.bitmap(target, image)?;
                    let destination = to_d2d(*rect);
                    unsafe { target.DrawBitmap(&bitmap, Some(&destination), 1.0, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, None) };
                }
            }
        }
        Ok(())
    }

    /// 取（或创建）颜色画刷。
    fn brush(&mut self, target: &ID2D1RenderTarget, color: Color) -> Result<ID2D1SolidColorBrush>
    {
        if let Some(brush) = self.brushes.get(&color.0)
        {
            return Ok(brush.clone());
        }
        let [red, green, blue, alpha] = color.to_unit();
        let brush = unsafe { target.CreateSolidColorBrush(&D2D1_COLOR_F { r: red, g: green, b: blue, a: alpha }, None)? };
        self.brushes.insert(color.0, brush.clone());
        Ok(brush)
    }

    /// 取（或创建）图像位图：按图像身份缓存，像素只上传一次。
    ///
    /// 缓存有上界（超出整体清空，与文本格式同一策略）；像素长度与宽高不符时拒绝，不上传坏数据。
    fn bitmap(&mut self, target: &ID2D1RenderTarget, image: &super::draw::ImageRef) -> Result<ID2D1Bitmap>
    {
        let identity = image.identity();
        if let Some(bitmap) = self.bitmaps.get(&identity)
        {
            return Ok(bitmap.clone());
        }
        let expected = image.width() as usize * image.height() as usize * 4;
        if image.width() == 0 || image.height() == 0 || image.pixels().len() != expected
        {
            return Err(Error::new(E_FAIL, format!("图像像素与尺寸不符：{}x{} / {} 字节", image.width(), image.height(), image.pixels().len())));
        }
        let properties = D2D1_BITMAP_PROPERTIES { pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED }, dpiX: 96.0, dpiY: 96.0 };
        let size = D2D_SIZE_U { width: image.width(), height: image.height() };
        let bitmap = unsafe { target.CreateBitmap(size, Some(image.pixels().as_ptr().cast()), image.width() * 4, &properties) }?;
        if self.bitmaps.len() >= MAX_BITMAPS
        {
            self.bitmaps.clear();
        }
        self.bitmaps.insert(identity, bitmap.clone());
        Ok(bitmap)
    }

    /// 取（或创建）文本格式：键与创建值都用四舍五入到 0.1 DIP 的字号。
    fn format(&mut self, size: f32, weight: u16, align: TextAlign) -> Result<IDWriteTextFormat> { self.format_mode(size, weight, align, false) }
    fn format_mode(&mut self, size: f32, weight: u16, align: TextAlign, caption: bool) -> Result<IDWriteTextFormat>
    {
        let size = round_tenth(size);
        let alignment = match align
        {
            TextAlign::Start => 0u8,
            TextAlign::Center => 1,
            TextAlign::End => 2,
        };
        let key = (weight, (size * 10.0).round() as u32, alignment | (u8::from(caption) << 2));
        if let Some(format) = self.formats.get(&key)
        {
            return Ok(format.clone());
        }
        if self.formats.len() >= MAX_FORMATS
        {
            self.formats.clear();
        }
        let format = unsafe { self.write.CreateTextFormat(&HSTRING::from(self.family.as_str()), None, DWRITE_FONT_WEIGHT(weight as i32), DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size, w!("zh-CN"))? };
        unsafe {
            if caption
            {
                format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                let sign = self.write.CreateEllipsisTrimmingSign(&format)?;
                format.SetTrimming(&DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, ..Default::default() }, &sign)?;
            }
            else
            {
                format.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
            }
            format.SetTextAlignment(match align
            {
                TextAlign::Start => DWRITE_TEXT_ALIGNMENT_LEADING,
                TextAlign::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                TextAlign::End => DWRITE_TEXT_ALIGNMENT_TRAILING,
            })?;
        }
        self.formats.insert(key, format.clone());
        Ok(format)
    }
}

/// 按同一份布局的真实行高裁剪；不限行时只用调用方的高度，不估算字体高度。
fn visible_height(layout: &IDWriteTextLayout, height: f32, limit: u16) -> Result<f32>
{
    if limit == 0
    {
        return Ok(height);
    }
    let mut metrics = DWRITE_TEXT_METRICS::default();
    unsafe {
        layout.GetMetrics(&mut metrics)?;
    }
    if metrics.lineCount <= u32::from(limit)
    {
        return Ok(height);
    }
    let mut lines = vec![DWRITE_LINE_METRICS::default(); metrics.lineCount as usize];
    let mut count = 0;
    unsafe {
        layout.GetLineMetrics(Some(&mut lines), &mut count)?;
    }
    Ok(height.min(lines.iter().take(limit as usize).map(|line| line.height).sum()))
}
/// 字号四舍五入到 0.1 DIP：缓存键与创建值必须一致，否则 12.01 与 12.04 会共用首个格式。
pub fn round_tenth(size: f32) -> f32 { (size * 10.0).round() / 10.0 }

/// DIP 矩形 → Direct2D 矩形。
fn to_d2d(rect: Rect) -> D2D_RECT_F { D2D_RECT_F { left: rect.x, top: rect.y, right: rect.right(), bottom: rect.bottom() } }

/// 圆角矩形；半径先夹到半宽半高内，避免 Direct2D 拒绝非法半径。
fn rounded(rect: &Rect, radius: f32) -> D2D1_ROUNDED_RECT
{
    let limit = radius.max(0.0).min(rect.width.abs() / 2.0).min(rect.height.abs() / 2.0);
    D2D1_ROUNDED_RECT { rect: to_d2d(*rect), radiusX: limit, radiusY: limit }
}

/// 图层内容边界：不裁剪。
fn infinite_bounds() -> D2D_RECT_F { D2D_RECT_F { left: f32::NEG_INFINITY * 0.5, top: f32::NEG_INFINITY * 0.5, right: f32::INFINITY * 0.5, bottom: f32::INFINITY * 0.5 } }

/// 在系统字体集合里挑一个能显示中文的界面字体。
fn resolve_ui_family(write: &IDWriteFactory) -> Result<String>
{
    let mut collection = None;
    unsafe { write.GetSystemFontCollection(&mut collection, false)? };
    let collection = collection.ok_or_else(|| Error::new(E_FAIL, "取不到系统字体集合"))?;
    for candidate in ["Microsoft YaHei UI", "Microsoft YaHei", "SimSun", "Segoe UI"]
    {
        let mut index = 0u32;
        let mut exists = windows::core::BOOL(0);
        unsafe { collection.FindFamilyName(&HSTRING::from(candidate), &mut index, &mut exists)? };
        if exists.as_bool()
        {
            return Ok(candidate.to_string());
        }
    }
    Ok("Segoe UI".to_string())
}

#[cfg(test)]
mod text_tests
{
    use super::*;
    #[test]
    fn directwrite_clips_at_real_line_boundary_and_bounds_formats() -> Result<()>
    {
        let _apt = super::super::host::ComApartment::enter()?;
        let mut painter = Painter::new()?;
        let format = painter.format(12.04, 400, TextAlign::Start)?;
        assert_eq!(unsafe { format.GetFontSize() }, 12.0);
        let text: Vec<_> = "甲\n乙\n丙".encode_utf16().collect();
        let layout = unsafe { painter.write.CreateTextLayout(&text, &format, 96.0, 160.0)? };
        let one = visible_height(&layout, 160.0, 1)?;
        let two = visible_height(&layout, 160.0, 2)?;
        assert!(one > 0.0 && one < two && two < 160.0);
        assert_eq!(visible_height(&layout, 4.0, 1)?, 4.0);
        for size in 10..80
        {
            painter.format(size as f32, 400, TextAlign::Start)?;
            assert!(painter.cached_formats() <= MAX_FORMATS);
        }
        Ok(())
    }
    /// 字号统一舍入到 0.1 DIP，缓存键与创建值因此不会各自漂移。
    #[test]
    fn font_size_rounds_to_one_tenth()
    {
        assert_eq!(round_tenth(12.01), 12.0);
        assert_eq!(round_tenth(12.04), 12.0);
        assert_eq!(round_tenth(12.06), 12.1);
        assert_eq!(round_tenth(13.0), 13.0);
        assert_eq!(round_tenth(0.04), 0.0);
    }
}
