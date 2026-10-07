use std::mem::size_of;
use std::ptr::null_mut;

use base64::{Engine, engine::general_purpose::STANDARD};
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDIBits, GetObjectW, HBITMAP};
use windows::Win32::UI::Shell::{IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_THUMBNAILONLY};
use windows::core::HSTRING;

/// 跨线程只传递复制后的像素和 PNG，不共享 COM 或 GDI 对象。
pub(crate) struct ShellImage
{
    pub width: i32,       // 实际位图宽度。
    pub height: i32,      // 实际位图高度。
    pub bgra: Vec<u8>,    // 原生拖动使用的预乘 Alpha 像素。
    pub data_url: String, // WebView 使用的直通 Alpha PNG。
}

/// 在创建位图的线程中释放 GDI 所有权。
pub(crate) struct OwnedBitmap(pub HBITMAP);

impl Drop for OwnedBitmap
{
    /// 同时覆盖提取、编码和拖动初始化的错误退出。
    fn drop(&mut self) { let _ = unsafe { DeleteObject(self.0.into()) }; }
}

impl ShellImage
{
    /// 从复制像素重建拖动位图，固定逻辑外框内保持比例居中。
    pub fn drag_bitmap(&self) -> Result<OwnedBitmap, String>
    {
        let info = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: 48, biHeight: -48, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() }, ..Default::default() }; // 拖动沿用 48 像素外框。
        let mut bits = null_mut(); // DIB 拥有的可写像素。
        let bitmap = OwnedBitmap(unsafe { CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) }.map_err(|error| error.to_string())?); // 返回前保持句柄所有权。
        let output = unsafe { std::slice::from_raw_parts_mut(bits.cast::<u8>(), 48 * 48 * 4) }; // 顶向下 BGRA。
        output.fill(0);
        let scale = 48.0 / self.width.max(self.height) as f64; // 保持宽高比，不裁切。
        let width = (self.width as f64 * scale).round().max(1.0) as usize; // 居中目标宽度。
        let height = (self.height as f64 * scale).round().max(1.0) as usize; // 居中目标高度。
        for y in 0..height
        {
            for x in 0..width
            {
                let source = ((y * self.height as usize / height) * self.width as usize + x * self.width as usize / width) * 4; // 最近邻采样，沿用小尺寸系统图像。
                let target = (((48 - height) / 2 + y) * 48 + (48 - width) / 2 + x) * 4; // 不透明像素之外保持透明。
                output[target..target + 4].copy_from_slice(&self.bgra[source..source + 4]);
            }
        }
        Ok(bitmap)
    }
}

/// 在独立 STA 提取文件内容，不请求 Shell 装饰或快捷方式缩略图。
pub(crate) fn extract_thumbnail(reference: &str, pixels: u32) -> Result<Option<ShellImage>, String>
{
    let path = std::path::Path::new(reference); // 桌面引用保留原始文件身份。
    if !path.is_absolute() || path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("lnk"))
    {
        return Ok(None);
    }
    let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?; // 目录和虚拟对象保留已有系统图标。
    if !metadata.is_file()
    {
        return Ok(None);
    }
    let factory: IShellItemImageFactory = unsafe { SHCreateItemFromParsingName(&HSTRING::from(reference), None) }.map_err(|error| error.to_string())?; // COM 接口不离开所属线程。
    let bitmap = match unsafe { factory.GetImage(SIZE { cx: pixels as i32, cy: pixels as i32 }, SIIGBF_THUMBNAILONLY) }
    {
        Ok(bitmap) => OwnedBitmap(bitmap),
        Err(_) => return Ok(None), // 无处理器或无缩略图是正常的图标回退。
    };
    let mut description = BITMAP::default(); // GetImage 返回的真实尺寸。
    if unsafe { GetObjectW(bitmap.0.into(), size_of::<BITMAP>() as i32, Some((&mut description as *mut BITMAP).cast())) } == 0
    {
        return Err("读取缩略图位图描述失败".into());
    }
    let width = description.bmWidth; // 不能假设返回正方形。
    let height = description.bmHeight.abs(); // 统一使用顶向下读取。
    if width <= 0 || height <= 0 || width > 1024 || height > 1024
    {
        return Err("系统缩略图尺寸超出读取边界".into());
    }
    let mut bgra = vec![0; width as usize * height as usize * 4]; // 有界像素副本。
    let mut info = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: width, biHeight: -height, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() }, ..Default::default() }; // 标准 BGRA DIB。
    let dc = unsafe { CreateCompatibleDC(None) }; // GetDIBits 的临时设备上下文。
    if dc.is_invalid()
    {
        return Err("创建缩略图读取上下文失败".into());
    }
    let lines = unsafe { GetDIBits(dc, bitmap.0, 0, height as u32, Some(bgra.as_mut_ptr().cast()), &mut info, DIB_RGB_COLORS) }; // 读取全部行。
    let _ = unsafe { DeleteDC(dc) };
    if lines != height
    {
        return Err("系统缩略图像素读取不完整".into());
    }
    let mut rgba = bgra.clone(); // Shell 内容位图使用直通 Alpha，PNG 仅调整通道顺序。
    for pixel in rgba.chunks_exact_mut(4)
    {
        pixel.swap(0, 2);
    }
    for pixel in bgra.chunks_exact_mut(4)
    {
        let alpha = u32::from(pixel[3]); // 仅原生拖动副本需要预乘，透明像素归零。
        for channel in &mut pixel[..3]
        {
            *channel = ((u32::from(*channel) * alpha + 127) / 255) as u8;
        }
    }
    let mut encoded = Vec::new(); // 编码后的 PNG。
    {
        let mut encoder = png::Encoder::new(&mut encoded, width as u32, height as u32); // 不强制变成 48×48。
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().map_err(|error| error.to_string())?.write_image_data(&rgba).map_err(|error| error.to_string())?;
    }
    Ok(Some(ShellImage { width, height, bgra, data_url: format!("data:image/png;base64,{}", STANDARD.encode(encoded)) }))
}

#[cfg(test)]
mod tests
{
    use super::*;

    /// 用隔离图片验证真实 Shell 提取、透明像素和原生拖动居中，不触碰用户文件。
    #[test]
    fn shell_thumbnail_preserves_aspect_and_drag_padding()
    {
        crate::platform::windows::initialize_shell_apartment().unwrap();
        let path = std::env::temp_dir().join(format!("desktop-tools-thumbnail-{}.png", std::process::id())); // 本测试独占的本地样本。
        {
            let file = std::fs::File::create(&path).unwrap(); // 无缓存的新样本。
            let mut encoder = png::Encoder::new(file, 96, 40); // 宽图可暴露方形拉伸。
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().unwrap().write_image_data(&[200, 40, 10, 128].repeat(96 * 40)).unwrap();
        }
        let image = extract_thumbnail(path.to_str().unwrap(), 48).unwrap().expect("PNG 应有系统缩略图"); // 正式提取和编码路径。
        std::fs::remove_file(&path).unwrap();
        assert_eq!((image.width, image.height), (48, 20));
        let encoded = STANDARD.decode(image.data_url.strip_prefix("data:image/png;base64,").unwrap()).unwrap(); // 解码正式 PNG 结果。
        let mut reader = png::Decoder::new(std::io::Cursor::new(encoded)).read_info().unwrap(); // 直通 Alpha 检查。
        let mut pixels = vec![0; reader.output_buffer_size()]; // 完整输出。
        reader.next_frame(&mut pixels).unwrap();
        assert!((pixels[0] as i32 - 200).abs() <= 3, "颜色应避免重复预乘：{:?}", &pixels[..4]);
        assert!((pixels[3] as i32 - 128).abs() <= 2, "应保留半透明 Alpha");
        {
            let bitmap = image.drag_bitmap().unwrap(); // 线程内重建的拖动位图。
            let mut description = BITMAP::default(); // DIB 可直接观察其像素。
            assert_ne!(unsafe { GetObjectW(bitmap.0.into(), size_of::<BITMAP>() as i32, Some((&mut description as *mut BITMAP).cast())) }, 0);
            let drag = unsafe { std::slice::from_raw_parts(description.bmBits.cast::<u8>(), 48 * 48 * 4) }; // 48×48 固定外框。
            assert_eq!(drag[3], 0);
            assert_eq!(drag[(24 * 48 + 24) * 4 + 3], image.bgra[3]);
        }
        crate::platform::windows::uninitialize_shell_apartment();
    }
}
