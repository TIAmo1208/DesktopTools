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
                    if alpha > 0
                    {
                        for channel in &mut pixel[..3]
                        {
                            *channel = (u32::from(*channel) * 255 / alpha).min(255) as u8;
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
    pub target: ID2D1DCRenderTarget,                 // 可反复清空和重绘的软件目标。
    write: IDWriteFactory,                           // 文字布局工厂。
    format: IDWriteTextFormat,                       // 当前字体格式。
    layouts: HashMap<u64, (IDWriteTextLayout, f32)>, // 仅保留当前队列的文字测量。
    pub size: SIZE,                                  // 物理像素尺寸。
    pub scale: f32,                                  // 主显示器 DPI 比例。
    font: (String, u8),                              // 格式缓存键。
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
            let format = write.CreateTextFormat(&HSTRING::from(&appearance.font), None, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, appearance.font_size as f32, w!("zh-CN"))?; // 已保存的界面字体。
            Ok(Self { loading_mask, stroke, target, write, format, layouts: HashMap::new(), size, scale, font: (appearance.font.clone(), appearance.font_size), bitmap })
        }
    }

    /// 字体变化时失效文字缓存，主题换色不重建图形设备。
    pub fn configure(&mut self, appearance: &Appearance) -> windows::core::Result<()>
    {
        if self.font == (appearance.font.clone(), appearance.font_size)
        {
            return Ok(());
        }
        self.format = unsafe { self.write.CreateTextFormat(&HSTRING::from(&appearance.font), None, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, appearance.font_size as f32, w!("zh-CN"))? };
        self.font = (appearance.font.clone(), appearance.font_size);
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

    /// 独立绘制每条消息的背景、文本及 Lucide 路径，不改变整个窗口透明度。
    pub fn draw(&mut self, frames: &[Frame], appearance: &Appearance, hover_id: Option<u64>, expanded: bool) -> windows::core::Result<()>
    {
        let point = |x, y| {
            let mut position = D2D1_ELLIPSE::default().point;
            position.X = x;
            position.Y = y;
            position
        }; // 使用 Windows 数值类型而不增加直接依赖。
        self.layouts.retain(|id, _| frames.iter().any(|frame| frame.id == *id));
        let (surface, border, text, muted, subtle) = if appearance.dark
        {
            (0x20252A, 0x394149, 0xF2F5F7, 0xA9B2BB, 0x272D33)
        }
        else
        {
            (0xFFFFFF, 0xD7DCE2, 0x17202A, 0x687481, 0xF7F8FA)
        }; // 与 global.css 同源色值。
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
                let rounded = D2D1_ROUNDED_RECT { rect, radiusX: appearance.radius * frame.visual.scale, radiusY: appearance.radius * frame.visual.scale }; // 按外观设置缩放圆角。
                brush.SetColor(&Self::color(surface, alpha));
                self.target.FillRoundedRectangle(&rounded, &brush);
                brush.SetColor(&Self::color(border, alpha));
                self.target.DrawRoundedRectangle(&rounded, &brush, 1.0, None);
                let content_alpha = alpha * frame.visual.content; // 后层内容随让位淡出。
                if content_alpha <= 0.0
                {
                    continue;
                }
                self.target.PushAxisAlignedClip(&rect, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE); // 缩小旧层时正文不能画到当前表面外。
                brush.SetColor(&Self::color(text, content_alpha));
                if let Some((layout, _)) = self.layouts.get(&frame.id)
                {
                    let mut metrics = DWRITE_TEXT_METRICS::default(); // 垂直居中当前多行正文。
                    layout.GetMetrics(&mut metrics)?;
                    self.target.DrawTextLayout(point(rect.left + 36.0, rect.top + ((rect.bottom - rect.top - metrics.height) / 2.0).max(12.0)), layout, &brush, D2D1_DRAW_TEXT_OPTIONS_CLIP);
                }
                if hover_id == Some(frame.id)
                {
                    brush.SetColor(&Self::color(subtle, content_alpha));
                    self.target.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: frame.close_rect(), radiusX: 4.0, radiusY: 4.0 }, &brush);
                }
                brush.SetColor(&Self::color(muted, content_alpha));
                let center = point(rect.right - 28.0, rect.top + 32.0); // 18px Lucide circle-x，24 viewBox 中半径 10。
                self.target.DrawEllipse(&D2D1_ELLIPSE { point: center, radiusX: 7.5, radiusY: 7.5 }, &brush, 1.5, &self.stroke);
                self.target.DrawLine(point(center.X - 2.25, center.Y - 2.25), point(center.X + 2.25, center.Y + 2.25), &brush, 1.5, &self.stroke);
                self.target.DrawLine(point(center.X + 2.25, center.Y - 2.25), point(center.X - 2.25, center.Y + 2.25), &brush, 1.5, &self.stroke);
                let status_color = match frame.kind.as_str()
                {
                    "success" =>
                    {
                        if appearance.dark
                        {
                            0x34D399
                        }
                        else
                        {
                            0x059669
                        }
                    }
                    "error" =>
                    {
                        if appearance.dark
                        {
                            0xFDA29B
                        }
                        else
                        {
                            0xB42318
                        }
                    }
                    _ => muted,
                }; // 状态配色沿用现有语义色。
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
                    self.target.FillOpacityMask(&self.loading_mask, &brush, D2D1_OPACITY_MASK_CONTENT_GRAPHICS, Some(&D2D_RECT_F { left: center.X - 8.0, top: center.Y - 8.0, right: center.X + 8.0, bottom: center.Y + 8.0 }), None);
                    self.target.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                    self.target.SetTransform(&original);
                }
                else if frame.kind == "success"
                {
                    self.target.DrawLine(point(center.X - 5.0, center.Y), point(center.X - 1.0, center.Y + 4.0), &brush, 1.5, &self.stroke);
                    self.target.DrawLine(point(center.X - 1.0, center.Y + 4.0), point(center.X + 6.0, center.Y - 4.0), &brush, 1.5, &self.stroke);
                }
                else
                {
                    self.target.DrawEllipse(&D2D1_ELLIPSE { point: center, radiusX: 6.5, radiusY: 6.5 }, &brush, 1.3, None);
                    let inverted = frame.kind == "warning" || frame.kind == "error"; // 信息与警告使用不同的点线方向。
                    self.target.DrawLine(point(center.X, center.Y + if inverted { -3.0 } else { 0.0 }), point(center.X, center.Y + if inverted { 1.0 } else { 4.0 }), &brush, 1.5, &self.stroke);
                    self.target.FillEllipse(&D2D1_ELLIPSE { point: point(center.X, center.Y + if inverted { 3.5 } else { -3.0 }), radiusX: 0.8, radiusY: 0.8 }, &brush);
                }
                self.target.PopAxisAlignedClip();
            }
            self.target.EndDraw(None, None)
        }
    }

    /// 以固定整窗 Alpha 提交逐像素透明，消息之间的透明度互不影响。
    pub fn present(&self, hwnd: windows::Win32::Foundation::HWND) -> windows::core::Result<()>
    {
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 }; // Alpha 已在每条消息绘制时合成。
        unsafe { UpdateLayeredWindow(hwnd, None, None, Some(&self.size), Some(self.bitmap.dc), Some(&POINT::default()), COLORREF(0), Some(&blend), ULW_ALPHA) }
    }
}
