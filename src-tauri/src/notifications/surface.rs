use super::{Appearance, Frame};
use std::collections::HashMap;
use windows::{
    Win32::{
        Foundation::{COLORREF, POINT, RECT, SIZE},
        Graphics::{
            Direct2D::{Common::*, *},
            DirectWrite::*,
            Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
            Gdi::*,
        },
        UI::WindowsAndMessaging::{ULW_ALPHA, UpdateLayeredWindow},
    },
    core::{HSTRING, w},
};

/// 拥有内存 DC 与预乘 Alpha 位图，包括初始化失败时的释放。
struct Bitmap
{
    dc: HDC,           // 内存绘图上下文。
    image: HBITMAP,    // 自有像素存储。
    previous: HGDIOBJ, // DC 原先选中的对象。
}

/// 使用正式 Direct2D 绘制路径检查主题、透明像素及 DPI，按需导出验收图。
#[cfg(test)]
#[test]
fn pixels_match_theme_alpha_and_dpi()
{
    use super::{Appearance, Frame, Visual};
    use crate::contracts::ThemeNotification;
    use crate::contracts::settings::ApplicationPreferences;
    for scale in [1.0, 1.5, 2.0]
    {
        for dark in [false, true]
        {
            let appearance = Appearance { dark, ..Appearance::resolve(&ApplicationPreferences::default(), false, true) }; // 精确的浅深主题。
            let mut surface = Surface::new(SIZE { cx: (300.0 * scale) as i32, cy: (350.0 * scale) as i32 }, scale, &appearance).unwrap(); // 真实软件渲染器。
            surface.measure(1, "设置已保存").unwrap();
            surface.measure(2, "正在加载").unwrap();
            let frames = vec![Frame { loading_phase: 0.0, id: 1, kind: "info".into(), loading: false, rect: D2D_RECT_F { left: 2.0, top: 140.0, right: 298.0, bottom: 204.0 }, visual: Visual { top: 140.0, height: 64.0, scale: 1.0, alpha: 1.0, content: 1.0 }, closable: true }, Frame { loading_phase: 0.0, id: 2, kind: "info".into(), loading: true, rect: D2D_RECT_F { left: 2.0, top: 218.0, right: 298.0, bottom: 282.0 }, visual: Visual { top: 218.0, height: 64.0, scale: 1.0, alpha: 1.0, content: 1.0 }, closable: true }]; // 展开样例包含真实 SVG loading 资源。
            surface.draw(&frames, &appearance, Some(2), true).unwrap();
            let mut section = DIBSECTION::default(); // 读取实际位图像素，不能以源码值代替渲染结果。
            unsafe {
                assert_ne!(GetObjectW(surface.bitmap.image.into(), std::mem::size_of::<DIBSECTION>() as i32, Some((&mut section as *mut DIBSECTION).cast())), 0);
            }
            let pixels = unsafe { std::slice::from_raw_parts(section.dsBm.bmBits.cast::<u8>(), (surface.size.cx * surface.size.cy * 4) as usize) }; // 预乘 BGRA。
            assert_eq!(pixels[3], 0, "堆栈外必须完全透明");
            let offset = (((150.0 * scale) as i32 * surface.size.cx + (140.0 * scale) as i32) * 4) as usize; // 正文之外的表面内部。
            assert_eq!(&pixels[offset..offset + 4], if dark { &[42, 37, 32, 255] } else { &[255, 255, 255, 255] });
            if let Some(directory) = std::env::var_os("DESKTOP_TOAST_CAPTURE_DIR")
            {
                let directory = std::path::PathBuf::from(directory); // 显式请求时才输出本地验收图。
                std::fs::create_dir_all(&directory).unwrap();
                let path = directory.join(format!("toast-{}-{}.png", if dark { "dark" } else { "light" }, (scale * 100.0) as u32)); // 主题及 DPI 标识。
                let mut rgba = pixels.to_vec(); // PNG 使用直通 Alpha。
                for pixel in rgba.chunks_exact_mut(4)
                {
                    pixel.swap(0, 2);
                    let alpha = u32::from(pixel[3]); // 还原预乘颜色。
                    for channel in &mut pixel[..3]
                    {
                        if let Some(value) = (u32::from(*channel) * 255).checked_div(alpha)
                        {
                            *channel = value.min(255) as u8;
                        }
                    }
                }
                let mut encoder = png::Encoder::new(std::fs::File::create(path).unwrap(), surface.size.cx as u32, surface.size.cy as u32); // 已安装的 PNG 编码器。
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                encoder.write_header().unwrap().write_image_data(&rgba).unwrap();
            }
            let mut fading = frames.clone(); // 每条消息单独透明，旧层不受影响。
            fading[1].visual.alpha = 0.5;
            surface.draw(&fading, &appearance, None, false).unwrap();
            let offset = (((228.0 * scale) as i32 * surface.size.cx + (140.0 * scale) as i32) * 4 + 3) as usize; // 淡入项内部透明度。
            assert!((i32::from(pixels[offset]) - 128).abs() <= 1);
            fading[1].loading = true;
            fading[1].loading_phase = 0.0;
            surface.draw(&fading, &appearance, None, false).unwrap();
            let first_rotation = pixels.to_vec(); // 正式画法的零角度像素。
            fading[1].loading_phase = std::f32::consts::FRAC_PI_2;
            surface.draw(&fading, &appearance, None, false).unwrap();
            assert_ne!(first_rotation, pixels, "loading 角度变化必须改变实际像素");
        }
        let theme = ThemeNotification { surface: Some("#0d1322".into()), border: Some("#00c2d9".into()), corner_style: Some("bevel".into()), corner_size: Some(12), ..Default::default() }; // 显式声明独立切角尺寸。
        let appearance = Appearance::resolve(&ApplicationPreferences::default(), false, false).with_palette(Some(theme));
        let mut surface = Surface::new(SIZE { cx: (300.0 * scale) as i32, cy: (350.0 * scale) as i32 }, scale, &appearance).unwrap();
        surface.measure(3, "主题通知").unwrap();
        let frame = Frame { loading_phase: 0.0, id: 3, kind: "info".into(), loading: false, rect: D2D_RECT_F { left: 2.0, top: 140.0, right: 298.0, bottom: 204.0 }, visual: Visual { top: 140.0, height: 64.0, scale: 1.0, alpha: 1.0, content: 1.0 }, closable: true };
        let pixel = |surface: &Surface, x: f32, y: f32| {
            let mut section = DIBSECTION::default();
            unsafe { assert_ne!(GetObjectW(surface.bitmap.image.into(), std::mem::size_of::<DIBSECTION>() as i32, Some((&mut section as *mut DIBSECTION).cast())), 0); }
            let bytes = unsafe { std::slice::from_raw_parts(section.dsBm.bmBits.cast::<u8>(), (surface.size.cx * surface.size.cy * 4) as usize) };
            let offset = (((y * scale) as i32 * surface.size.cx + (x * scale) as i32) * 4) as usize;
            [bytes[offset], bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]]
        }; // 真实预乘 BGRA 的指定 DIP 像素。
        surface.draw(&[frame.clone()], &appearance, None, false).unwrap();
        let surface_bgra = pixel(&surface, 140.0, 150.0); // 画板主题表面，正文轨道另取样。
        let text_region = (36..244).flat_map(|x| (140..204).map(move |y| (x, y))).filter(|(x, y)| {
            let color = pixel(&surface, *x as f32, *y as f32);
            color[3] > 200 && color != surface_bgra
        }).count();
        assert!(text_region > 8, "bevel 主题正文应明显区别于表面，DPI={scale}，像素={text_region}");
        let icon_region = (12..28).flat_map(|x| (165..181).map(move |y| (x, y))).filter(|(x, y)| pixel(&surface, *x as f32, *y as f32) != surface_bgra).count();
        assert!(icon_region > 4, "bevel 主题状态图标不应消失，DPI={scale}");
        let close_region = (260..286).flat_map(|x| (159..185).map(move |y| (x, y))).filter(|(x, y)| pixel(&surface, *x as f32, *y as f32) != surface_bgra).count();
        assert!(close_region > 4, "bevel 主题关闭图标不应消失，DPI={scale}");
        assert_eq!(pixel(&surface, 3.0, 141.0)[3], 0, "切角外必须透明，DPI={scale}");
        assert!(pixel(&surface, 10.0, 144.0)[3] > 0, "左上切边描线连续，DPI={scale}");
        assert!(pixel(&surface, 4.0, 150.0)[3] > 0, "左边框连续，DPI={scale}");
        assert_eq!(pixel(&surface, 296.0, 202.0)[3], 0, "右下切角外透明，DPI={scale}");
        assert!(pixel(&surface, 289.0, 199.0)[3] > 0, "右下切边描线连续，DPI={scale}");
        for visual_scale in [1.0, 0.94, 0.88]
        {
            let mut folded = frame.clone();
            folded.visual.scale = visual_scale;
            folded.rect.right = folded.rect.left + 296.0 * visual_scale;
            folded.rect.bottom = folded.rect.top + 64.0 * visual_scale;
            surface.draw(&[folded], &appearance, None, false).unwrap();
            assert_eq!(pixel(&surface, 2.0, 140.0)[3], 0, "切角应与 {visual_scale} 折叠比例同步，DPI={scale}");
        }
    }
}

impl Drop for Bitmap
{
    /// 先恢复选择对象，随后释放位图和 DC。
    fn drop(&mut self)
    {
        unsafe {
            let _ = SelectObject(self.dc, self.previous);
            let _ = DeleteObject(self.image.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

/// 缓存窗口线程的 Direct2D、DirectWrite 与逐条文字布局。
pub(super) struct Surface
{
    loading_mask: ID2D1Bitmap,                       // loading.svg 的 DPI 相关透明度缓存，主题色由画刷提供。
    stroke: ID2D1StrokeStyle,                        // Lucide 圆头与圆角连接样式，创建后复用。
    factory: ID2D1Factory,                           // 复用工厂创建当前帧轮廓。
    pub target: ID2D1DCRenderTarget,                 // 可反复清空和重绘的软件目标。
    write: IDWriteFactory,                           // 文字布局工厂。
    format: IDWriteTextFormat,                       // 当前字体格式。
    layouts: HashMap<u64, (IDWriteTextLayout, f32)>, // 仅保留当前队列的文字测量。
    pub size: SIZE,                                  // 物理像素尺寸。
    pub scale: f32,                                  // 主显示器 DPI 比例。
    font: (String, u8, i32),                         // 格式缓存键：字体、字号、字重。
    bitmap: Bitmap,                                  // 最后释放像素存储。
}

impl Surface
{
    /// 为当前主显示器创建一次软件表面，窗口尺寸或 DPI 改变时重建。
    pub fn new(size: SIZE, scale: f32, appearance: &Appearance) -> windows::core::Result<Self>
    {
        unsafe {
            let dc = CreateCompatibleDC(None); // 与屏幕独立的内存 DC。
            if dc.is_invalid()
            {
                return Err(windows::core::Error::from_win32());
            }
            let mut bitmap = Bitmap { dc, image: HBITMAP::default(), previous: HGDIOBJ::default() }; // 初始化中也拥有已创建资源。
            let info = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: size.cx, biHeight: -size.cy, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() }, ..Default::default() }; // 自顶向下 BGRA。
            let mut pixels = std::ptr::null_mut(); // Windows 分配的位图数据。
            bitmap.image = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut pixels, None, 0)?;
            bitmap.previous = SelectObject(dc, bitmap.image.into());
            let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?; // 窗口线程软件工厂。
            let stroke = factory.CreateStrokeStyle(&D2D1_STROKE_STYLE_PROPERTIES { startCap: D2D1_CAP_STYLE_ROUND, endCap: D2D1_CAP_STYLE_ROUND, dashCap: D2D1_CAP_STYLE_ROUND, lineJoin: D2D1_LINE_JOIN_ROUND, ..Default::default() }, None)?; // 与 Lucide stroke-linecap/linejoin 一致。
            let target = factory.CreateDCRenderTarget(&D2D1_RENDER_TARGET_PROPERTIES { r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE, pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED }, dpiX: 96.0 * scale, dpiY: 96.0 * scale, ..Default::default() })?; // 支持透明像素。
            target.BindDC(dc, &RECT { left: 0, top: 0, right: size.cx, bottom: size.cy })?;
            target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let tree = resvg::usvg::Tree::from_data(include_bytes!("../../../resources/loading.svg"), &resvg::usvg::Options::default()).map_err(|error| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, format!("解析 loading.svg 失败：{error}")))?; // 直接使用资源，不复制其几何。
            let icon_size = (16.0 * scale).ceil() as u32; // 当前 DPI 下的实际图标像素。
            let mut icon = resvg::tiny_skia::Pixmap::new(icon_size, icon_size).ok_or_else(|| windows::core::Error::from_hresult(windows::Win32::Foundation::E_OUTOFMEMORY))?; // 只在表面创建时栅格化。
            resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(icon_size as f32 / tree.size().width(), icon_size as f32 / tree.size().height()), &mut icon.as_mut());
            let alpha = icon.data().chunks_exact(4).map(|pixel| pixel[3]).collect::<Vec<_>>(); // 保留 SVG 每段透明度，黑色填充作为单色主题蒙版。
            let loading_mask = target.CreateBitmap(D2D_SIZE_U { width: icon_size, height: icon_size }, Some(alpha.as_ptr().cast()), icon_size, &D2D1_BITMAP_PROPERTIES { pixelFormat: D2D1_PIXEL_FORMAT { format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED }, dpiX: 96.0 * scale, dpiY: 96.0 * scale })?;
            let write: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?; // 复用系统文本工厂。
            let format = write.CreateTextFormat(&HSTRING::from(&appearance.font), None, DWRITE_FONT_WEIGHT(appearance.weight), DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, appearance.font_size as f32, w!("zh-CN"))?; // 已保存的界面字体与主题字重。
            Ok(Self { loading_mask, stroke, factory, target, write, format, layouts: HashMap::new(), size, scale, font: (appearance.font.clone(), appearance.font_size, appearance.weight), bitmap })
        }
    }

    /// 字体变化时失效文字缓存，主题换色不重建图形设备。
    pub fn configure(&mut self, appearance: &Appearance) -> windows::core::Result<()>
    {
        if self.font == (appearance.font.clone(), appearance.font_size, appearance.weight)
        {
            return Ok(());
        }
        self.format = unsafe { self.write.CreateTextFormat(&HSTRING::from(&appearance.font), None, DWRITE_FONT_WEIGHT(appearance.weight), DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, appearance.font_size as f32, w!("zh-CN"))? };
        self.font = (appearance.font.clone(), appearance.font_size, appearance.weight);
        self.layouts.clear();
        Ok(())
    }

    /// 按真实文字测量高度，超出五条可用空间时省略，关闭按钮始终可见。
    pub fn measure(&mut self, id: u64, content: &str) -> windows::core::Result<f32>
    {
        if let Some((_, height)) = self.layouts.get(&id)
        {
            return Ok(*height);
        }
        let max_height = ((self.size.cy as f32 / self.scale - 32.0 - 56.0) / 5.0).max(64.0); // 五条消息与间距的最大单项高度。
        let text = content.encode_utf16().collect::<Vec<_>>(); // 原始正文。
        unsafe {
            let layout = self.write.CreateTextLayout(&text, &self.format, 208.0, max_height - 24.0)?; // 为图标和关闭按钮预留各自轨道。
            let ellipsis = self.write.CreateEllipsisTrimmingSign(&self.format)?; // 超长正文不越过面板。
            layout.SetTrimming(&DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, ..Default::default() }, &ellipsis)?;
            let mut metrics = DWRITE_TEXT_METRICS::default(); // DirectWrite 实际行高。
            layout.GetMetrics(&mut metrics)?;
            let height = (metrics.height + 24.0).clamp(64.0, max_height); // 短正文保持稳定高度。
            self.layouts.insert(id, (layout, height));
            Ok(height)
        }
    }

    /// 将颜色转换为 Direct2D 预合成输入，复用前端主题的精确色值。
    fn color(rgb: u32, alpha: f32) -> D2D1_COLOR_F { D2D1_COLOR_F { r: ((rgb >> 16) & 255) as f32 / 255.0, g: ((rgb >> 8) & 255) as f32 / 255.0, b: (rgb & 255) as f32 / 255.0, a: alpha } }

    /// 按主题独立尺寸生成两个 45° 切角，右上与左下继续使用外观圆角。
    fn bevel_geometry(&self, rect: D2D_RECT_F, radius: f32, corner_size: f32, visual_scale: f32) -> windows::core::Result<ID2D1PathGeometry>
    {
        let geometry = unsafe { self.factory.CreatePathGeometry()? };
        let sink = unsafe { geometry.Open()? };
        let radius = radius.max(0.0).min((rect.right - rect.left) / 2.0).min((rect.bottom - rect.top) / 2.0); // 其余两角仍使用外观圆角。
        let cut = (corner_size * visual_scale).min((rect.right - rect.left) / 2.0).min((rect.bottom - rect.top) / 2.0); // 独立切角随折叠层同比缩放。
        let point = |x, y| {
            let mut value = D2D1_ELLIPSE::default().point; // 已有 Windows Vector2 类型，不添加依赖。
            value.X = x;
            value.Y = y;
            value
        };
        unsafe {
            sink.BeginFigure(point(rect.left + cut, rect.top), D2D1_FIGURE_BEGIN_FILLED);
            sink.AddLine(point(rect.right - radius, rect.top));
            sink.AddArc(&D2D1_ARC_SEGMENT { point: point(rect.right, rect.top + radius), size: D2D_SIZE_F { width: radius, height: radius }, rotationAngle: 0.0, sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE, arcSize: D2D1_ARC_SIZE_SMALL });
            sink.AddLine(point(rect.right, rect.bottom - cut));
            sink.AddLine(point(rect.right - cut, rect.bottom));
            sink.AddLine(point(rect.left + radius, rect.bottom));
            sink.AddArc(&D2D1_ARC_SEGMENT { point: point(rect.left, rect.bottom - radius), size: D2D_SIZE_F { width: radius, height: radius }, rotationAngle: 0.0, sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE, arcSize: D2D1_ARC_SIZE_SMALL });
            sink.AddLine(point(rect.left, rect.top + cut));
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            sink.Close()?;
        }
        Ok(geometry)
    }

    /// 独立绘制每条消息的背景、文本及 Lucide 路径，不改变整个窗口透明度。
    pub fn draw(&mut self, frames: &[Frame], appearance: &Appearance, hover_id: Option<u64>, expanded: bool) -> windows::core::Result<()>
    {
        self.layouts.retain(|id, _| frames.iter().any(|frame| frame.id == *id));
        let (surface, border, text, muted, subtle) = appearance.palette_colors(); // 主题声明了消息配色时逐项覆盖内置两套。
        unsafe {
            let brush = self.target.CreateSolidColorBrush(&Self::color(text, 1.0), None)?; // 单帧复用画刷。
            self.target.BeginDraw();
            self.target.Clear(Some(&Self::color(0, 0.0)));
            if expanded && frames.len() > 1
            {
                brush.SetColor(&Self::color(surface, 1.0 / 255.0));
                let top = frames.iter().map(|frame| frame.rect.top).fold(f32::MAX, f32::min); // 堆栈内间隙保留悬停，外部仍完全透明。
                let bottom = frames.iter().map(|frame| frame.rect.bottom).fold(f32::MIN, f32::max); // 不延伸到最后可见消息之外。
                self.target.FillRectangle(&D2D_RECT_F { left: 2.0, top, right: 298.0, bottom }, &brush);
            }
            for frame in frames
            {
                let rect = frame.rect; // 绘制和命中共用的当前可见几何。
                let alpha = frame.visual.alpha; // 当前消息独立透明度。
                if alpha <= 0.0
                {
                    continue;
                }
                let rounded = D2D1_ROUNDED_RECT { rect, radiusX: appearance.radius * frame.visual.scale, radiusY: appearance.radius * frame.visual.scale }; // 未声明 bevel 时保持原圆角绘制。
                let bevel = appearance.bevel_size(); // 只有主题显式声明 bevel 与独立尺寸才改变轮廓。
                let geometry = bevel.map(|corner_size| self.bevel_geometry(rect, rounded.radiusX, corner_size, frame.visual.scale)).transpose()?; // 填充与描边共用轮廓。
                brush.SetColor(&Self::color(surface, alpha));
                if let Some(geometry) = &geometry
                {
                    self.target.FillGeometry(geometry, &brush, None);
                }
                else
                {
                    self.target.FillRoundedRectangle(&rounded, &brush);
                }
                brush.SetColor(&Self::color(border, alpha));
                if let Some(geometry) = &geometry
                {
                    self.target.DrawGeometry(geometry, &brush, 1.0, None);
                }
                else
                {
                    self.target.DrawRoundedRectangle(&rounded, &brush, 1.0, None);
                }
                let content_alpha = alpha * frame.visual.content; // 后层内容随让位淡出。
                if content_alpha <= 0.0
                {
                    continue;
                }
                self.target.PushAxisAlignedClip(&rect, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE); // 正文与图标都在轮廓内侧；不让几何 layer 掩膜吃掉文字。
                self.draw_content(frame, appearance, hover_id, &brush, text, muted, subtle, content_alpha)?;
                self.target.PopAxisAlignedClip();
            }
            self.target.EndDraw(None, None)
        }
    }

    /// 绘制正文、关闭与状态图标，外层负责当前轮廓裁剪。
    fn draw_content(&self, frame: &Frame, appearance: &Appearance, hover_id: Option<u64>, source_brush: &ID2D1SolidColorBrush, text: u32, muted: u32, subtle: u32, content_alpha: f32) -> windows::core::Result<()>
    {
        let brush = source_brush; // 绘制借用同一画刷，避免二次引用传入 COM 参数。
        let rect = frame.rect;
        let point = |x, y| {
            let mut position = D2D1_ELLIPSE::default().point;
            position.X = x;
            position.Y = y;
            position
        };
        unsafe {
            brush.SetColor(&Self::color(text, content_alpha));
            if let Some((layout, _)) = self.layouts.get(&frame.id)
            {
                let mut metrics = DWRITE_TEXT_METRICS::default(); // 垂直居中当前多行正文。
                layout.GetMetrics(&mut metrics)?;
                self.target.DrawTextLayout(point(rect.left + 36.0, rect.top + ((rect.bottom - rect.top - metrics.height) / 2.0).max(12.0)), layout, brush, D2D1_DRAW_TEXT_OPTIONS_CLIP);
            }
            if hover_id == Some(frame.id)
            {
                brush.SetColor(&Self::color(subtle, content_alpha));
                self.target.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: frame.close_rect(), radiusX: 4.0, radiusY: 4.0 }, brush);
            }
            brush.SetColor(&Self::color(muted, content_alpha));
            let center = point(rect.right - 28.0, rect.top + 32.0); // 18px Lucide circle-x，24 viewBox 中半径 10。
            self.target.DrawEllipse(&D2D1_ELLIPSE { point: center, radiusX: 7.5, radiusY: 7.5 }, brush, 1.5, &self.stroke);
            self.target.DrawLine(point(center.X - 2.25, center.Y - 2.25), point(center.X + 2.25, center.Y + 2.25), brush, 1.5, &self.stroke);
            self.target.DrawLine(point(center.X + 2.25, center.Y - 2.25), point(center.X - 2.25, center.Y + 2.25), brush, 1.5, &self.stroke);
            let status_color = appearance.status_color(frame.kind.as_str(), muted); // 主题声明了状态色时覆盖内置语义色。
            brush.SetColor(&Self::color(status_color, content_alpha));
            let center = point(rect.left + 20.0, rect.top + 32.0); // 左侧 16px 状态图标。
            if frame.loading
            {
                let mut original = Default::default(); // 保存原变换，旋转只作用于资源图标。
                    self.target.GetTransform(&mut original);
                    let mut rotation = original; // 以图标中心旋转缓存蒙版。
                    let (sine, cosine) = frame.loading_phase.sin_cos();
                    rotation.M11 = cosine;
                    rotation.M12 = sine;
                    rotation.M21 = -sine;
                    rotation.M22 = cosine;
                    rotation.M31 = center.X * (1.0 - cosine) + center.Y * sine;
                    rotation.M32 = center.Y * (1.0 - cosine) - center.X * sine;
                    self.target.SetTransform(&rotation);
                    self.target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED); // FillOpacityMask 要求此模式，边缘 Alpha 已来自 SVG 栅格化。
                    self.target.FillOpacityMask(&self.loading_mask, brush, D2D1_OPACITY_MASK_CONTENT_GRAPHICS, Some(&D2D_RECT_F { left: center.X - 8.0, top: center.Y - 8.0, right: center.X + 8.0, bottom: center.Y + 8.0 }), None);
                    self.target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                    self.target.SetTransform(&original);
                }
                else if frame.kind == "success"
                {
                    self.target.DrawLine(point(center.X - 5.0, center.Y), point(center.X - 1.0, center.Y + 4.0), brush, 1.5, &self.stroke);
                    self.target.DrawLine(point(center.X - 1.0, center.Y + 4.0), point(center.X + 6.0, center.Y - 4.0), brush, 1.5, &self.stroke);
                }
                else
                {
                    self.target.DrawEllipse(&D2D1_ELLIPSE { point: center, radiusX: 6.5, radiusY: 6.5 }, brush, 1.3, None);
                    let inverted = frame.kind == "warning" || frame.kind == "error"; // 信息与警告使用不同的点线方向。
                    self.target.DrawLine(point(center.X, center.Y + if inverted { -3.0 } else { 0.0 }), point(center.X, center.Y + if inverted { 1.0 } else { 4.0 }), brush, 1.5, &self.stroke);
                    self.target.FillEllipse(&D2D1_ELLIPSE { point: point(center.X, center.Y + if inverted { 3.5 } else { -3.0 }), radiusX: 0.8, radiusY: 0.8 }, brush);
                }
        }
        Ok(())
    }

    /// 以固定整窗 Alpha 提交逐像素透明，消息之间的透明度互不影响。
    pub fn present(&self, hwnd: windows::Win32::Foundation::HWND) -> windows::core::Result<()>
    {
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 }; // Alpha 已在每条消息绘制时合成。
        unsafe { UpdateLayeredWindow(hwnd, None, None, Some(&self.size), Some(self.bitmap.dc), Some(&POINT::default()), COLORREF(0), Some(&blend), ULW_ALPHA) }
    }
}
