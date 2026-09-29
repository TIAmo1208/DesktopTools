use std::path::Path;
use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, IDataObject, TYMED_HGLOBAL};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Ole::ReleaseStgMedium;
use windows::core::w;

pub(crate) const COLLECT: &str = "official.collect-box"; // 收集内容策略身份。
pub(crate) const MEDIA: &str = "official.music-control"; // 默认关联打开策略身份。
const IMAGE_LIMIT: usize = 32 * 1024 * 1024; // 外部图像原始字节上限。
const PIXEL_LIMIT: usize = 64 * 1024 * 1024; // 解码像素上限。
const TEXT_LIMIT: usize = 1024 * 1024; // UTF-16 原始正文上限。

/// 进入时固定的格式摘要，移动期间不读取外部正文。
#[derive(Clone, Default)]
pub(crate) struct ContentOffer
{
    pub files: bool,                // 完整集合包含文件表示，不能降级到文字绕过白名单。
    pub targets: Vec<&'static str>, // 可接收内容的业务类型。
    image: Option<u16>,             // PNG 优先，其次 DIB。
    link: Option<u16>,              // 明确的 Unicode URL。
    text: bool,                     // Unicode 普通文本。
}

/// 按已确认的业务白名单整体判断文件集合，不逐项静默筛选。
pub(crate) fn file_targets(paths: &[String]) -> Vec<&'static str>
{
    if paths.is_empty() || paths.len() > 512 || paths.iter().any(|path| !Path::new(path).is_file())
    {
        return Vec::new();
    }
    let extensions = paths.iter().map(|path| Path::new(path).extension().and_then(|extension| extension.to_str()).unwrap_or("").to_ascii_lowercase()).collect::<Vec<_>>(); // 所有路径的当前类型。
    let mut targets = Vec::new(); // 相同文件可以符合多个业务白名单。
    if extensions.iter().all(|extension| ["png", "jpg", "jpeg", "gif", "bmp", "webp", "ico"].contains(&extension.as_str()))
    {
        targets.push(COLLECT);
    }
    if extensions.len() == 1 && ["mp3", "wav", "flac", "m4a", "aac", "ogg", "opus", "wma", "mp4", "mkv", "avi", "mov", "wmv", "webm", "m4v"].contains(&extensions[0].as_str())
    {
        targets.push(MEDIA);
    }
    targets
}

/// 接收白名单与实际效果独立匹配，非文件动作只允许 COPY。
pub(crate) fn content_effect(offer: &ContentOffer, target: &str, mode: &str, allowed: windows::Win32::System::Ole::DROPEFFECT, keys: windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS) -> windows::Win32::System::Ole::DROPEFFECT
{
    use windows::Win32::System::Ole::{DROPEFFECT_COPY, DROPEFFECT_MOVE, DROPEFFECT_NONE};
    if !offer.targets.contains(&target)
    {
        return DROPEFFECT_NONE;
    }
    let expected = if target == COLLECT && offer.files && mode == "move" { DROPEFFECT_MOVE } else { DROPEFFECT_COPY }; // 路径链接不等于 OLE LINK。
    if super::drop_target::negotiated_effect(allowed, keys, expected, false) == expected
    {
        expected
    }
    else
    {
        DROPEFFECT_NONE
    }
}

/// 构造只读取 HGLOBAL 内容表示的 COM 格式描述。
fn format(id: u16) -> FORMATETC { FORMATETC { cfFormat: id, dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32, ..Default::default() } }

/// 探测能力不提取正文，释放时仍须处理 GetData 失败。
pub(crate) fn offer(data: &IDataObject, paths: &[String]) -> ContentOffer
{
    let supports = |id: u16| id != 0 && unsafe { data.QueryGetData(&format(id)) } == windows::Win32::Foundation::S_OK; // 只接受明确支持，不将 S_FALSE 当作可读取。
    let file_group = unsafe { RegisterClipboardFormatW(w!("FileGroupDescriptorW")) } as u16; // 虚拟文件不能降级到普通文字。
    let shell_items = unsafe { RegisterClipboardFormatW(w!("Shell IDList Array")) } as u16; // 无文件路径的 Shell 项目也不能降级收入。
    let files = !paths.is_empty() || supports(15) || supports(file_group) || supports(shell_items); // CF_HDROP 或完整解析的 Shell 文件集合。
    if files
    {
        return ContentOffer { files, targets: file_targets(paths), ..Default::default() };
    }
    let png = unsafe { RegisterClipboardFormatW(w!("PNG")) } as u16; // 常见浏览器与图像编辑器的 PNG 格式。
    let url = unsafe { RegisterClipboardFormatW(w!("UniformResourceLocatorW")) } as u16; // 明确的 URL 表示。
    let image = [png, 17, 8].into_iter().find(|id| supports(*id)); // PNG、CF_DIBV5、CF_DIB，保持固定选择优先级。
    let link = supports(url).then_some(url); // 不将任意 HTML 当作链接。
    let text = supports(13); // CF_UNICODETEXT。
    ContentOffer { files, targets: if image.is_some() || link.is_some() || text { vec![COLLECT] } else { Vec::new() }, image, link, text }
}

/// 在 OLE 有效生命周期内复制外部内存，所有路径配对释放存储介质。
fn read_bytes(data: &IDataObject, id: u16, limit: usize) -> Result<Vec<u8>, String>
{
    let mut medium = unsafe { data.GetData(&format(id)) }.map_err(|error| format!("读取拖入数据失败：{error}"))?; // GetData 交付的有界外部介质。
    let result = (|| {
        if medium.tymed != TYMED_HGLOBAL.0 as u32
        {
            return Err("拖入数据介质不受支持".into());
        }
        let memory = unsafe { medium.u.hGlobal }; // 仅在介质类型验证后访问联合体。
        let size = unsafe { GlobalSize(memory) }; // 在构造切片前验证长度。
        if size == 0 || size > limit
        {
            return Err("拖入数据为空或超过大小限制".into());
        }
        let pointer = unsafe { GlobalLock(memory) }.cast::<u8>(); // 有效期仅到 GlobalUnlock。
        if pointer.is_null()
        {
            return Err("无法读取拖入数据内存".into());
        }
        let bytes = unsafe { std::slice::from_raw_parts(pointer, size) }.to_vec(); // 业务只接收自有数据。
        let _ = unsafe { GlobalUnlock(memory) };
        Ok(bytes)
    })();
    unsafe { ReleaseStgMedium(&mut medium) };
    result
}

/// 提取一种最高优先级表示，格式读取失败不回退到其他内容。
pub(crate) fn extract(data: &IDataObject, paths: Vec<String>, offer: &ContentOffer) -> Result<crate::collect::IncomingContent, String>
{
    use crate::collect::IncomingContent;
    if offer.files
    {
        return Ok(IncomingContent::Files(paths));
    }
    if let Some(id) = offer.image
    {
        let bytes = read_bytes(data, id, IMAGE_LIMIT)?; // 在回调内取得字节，后台再验证、转换图像。
        return Ok(IncomingContent::Image { bytes, dib: id == 8 || id == 17 });
    }
    let bytes = read_bytes(data, offer.link.unwrap_or(13), TEXT_LIMIT)?; // URL 优先于普通文字。
    if !bytes.len().is_multiple_of(2)
    {
        return Err("拖入文本编码无效".into());
    }
    let mut words = bytes.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).collect::<Vec<_>>(); // 对齐安全的 UTF-16 解码。
    let end = words.iter().position(|word| *word == 0).ok_or("拖入文本缺少终止符")?; // 不把尾部内存收入正文。
    words.truncate(end);
    let text = String::from_utf16(&words).map_err(|_| "拖入文本编码无效")?; // 拒绝损坏的代理项。
    if text.trim().is_empty()
    {
        return Err("拖入文本为空".into());
    }
    if offer.link.is_some()
    {
        Ok(IncomingContent::Link(text))
    }
    else if offer.text
    {
        Ok(IncomingContent::Text(text))
    }
    else
    {
        Err("拖入内容不受支持".into())
    }
}

/// 在后台把支持的 DIB 转为 PNG，并对现有 PNG 做有界完整解码验证。
pub(crate) fn image_png(bytes: &[u8], dib: bool) -> Result<Vec<u8>, String>
{
    if bytes.len() > IMAGE_LIMIT
    {
        return Err("图像超过大小限制".into());
    }
    if !dib
    {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes)); // 已安装的 PNG 解码器。
        decoder.set_limits(png::Limits { bytes: PIXEL_LIMIT });
        let mut reader = decoder.read_info().map_err(|error| format!("PNG 数据无效：{error}"))?; // 验证文件头与尺寸。
        let size = reader.output_buffer_size(); // 实际解码像素内存。
        if size > PIXEL_LIMIT
        {
            return Err("图像像素超过大小限制".into());
        }
        reader.next_frame(&mut vec![0; size]).map_err(|error| format!("PNG 数据损坏：{error}"))?;
        return Ok(bytes.to_vec());
    }
    if bytes.len() < 40
    {
        return Err("DIB 头不完整".into());
    }
    let u32_at = |offset| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap()); // 固定头字段已检查边界。
    let header = u32_at(0) as usize; // BITMAPINFOHEADER 或 V4/V5 头长度。
    let width = u32_at(4) as i32; // DIB 横向像素数。
    let height = u32_at(8) as i32; // 负高度表示从上到下存储。
    let bits = u16::from_le_bytes([bytes[14], bytes[15]]); // 仅支持 BGR 24/32 位未压缩像素。
    if ![40, 108, 124].contains(&header) || header > bytes.len() || width <= 0 || height == 0 || height == i32::MIN || bytes[12..14] != [1, 0] || ![24, 32].contains(&bits) || u32_at(16) != 0 || u32_at(32) != 0
    {
        return Err("仅支持未压缩的 24/32 位 DIB".into());
    }
    let width = width as usize; // 正宽度已经验证。
    let rows = height.unsigned_abs() as usize; // 实际行数。
    let stride = (width * usize::from(bits)).div_ceil(32) * 4; // DWORD 对齐的源行跨度。
    let size = width.checked_mul(rows).and_then(|size| size.checked_mul(3)).filter(|size| *size <= PIXEL_LIMIT).ok_or("图像像素超过大小限制")?; // 有界 RGB 输出。
    if stride.checked_mul(rows).and_then(|size| size.checked_add(header)).is_none_or(|end| end > bytes.len())
    {
        return Err("DIB 像素不完整".into());
    }
    let mut pixels = vec![0; size]; // PNG 使用 RGB，BI_RGB 高字节不视为有效透明度。
    for row in 0..rows
    {
        let source_row = if height > 0 { rows - row - 1 } else { row }; // 修正底向上的位图。
        for column in 0..width
        {
            let source = header + source_row * stride + column * usize::from(bits / 8); // 已验证完整像素边界。
            let target = (row * width + column) * 3; // RGB 输出位置。
            pixels[target..target + 3].copy_from_slice(&[bytes[source + 2], bytes[source + 1], bytes[source]]);
        }
    }
    let mut output = Vec::new(); // 交付收集业务的 PNG 字节。
    {
        let mut encoder = png::Encoder::new(&mut output, width as u32, rows as u32); // 复用既有 PNG 依赖。
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().map_err(|error| error.to_string())?.write_image_data(&pixels).map_err(|error| error.to_string())?;
    }
    Ok(output)
}

#[cfg(test)]
mod tests
{
    use super::*;
    use windows::Win32::System::Ole::{DROPEFFECT_COPY, DROPEFFECT_MOVE, DROPEFFECT_NONE};
    use windows::Win32::System::SystemServices::{MK_CONTROL, MK_SHIFT, MODIFIERKEYS_FLAGS};

    /// 白名单整体判断，媒体仅单项；复制动作不得误报移动或链接。
    #[test]
    fn content_policy_checks_whole_batch_and_effects()
    {
        let root = std::env::temp_dir().join(format!("desktop-tools-drop-policy-{}", crate::tools::time::next_timeline_timestamp())); // 独占测试目录。
        std::fs::create_dir_all(&root).unwrap();
        let image = root.join("图片.PNG"); // 大小写不影响图片类型。
        let media = root.join("音频.MP3"); // 单个媒体文件。
        std::fs::write(&image, []).unwrap();
        std::fs::write(&media, []).unwrap();
        let image = image.to_string_lossy().into_owned(); // 真实现有路径。
        let media = media.to_string_lossy().into_owned(); // 真实现有路径。
        assert_eq!(file_targets(std::slice::from_ref(&image)), [COLLECT]);
        assert_eq!(file_targets(std::slice::from_ref(&media)), [MEDIA]);
        assert!(file_targets(&[image.clone(), media.clone()]).is_empty());
        assert!(file_targets(&[media.clone(), media]).is_empty());
        assert!(file_targets(&[root.to_string_lossy().into_owned()]).is_empty());
        let files = ContentOffer { files: true, targets: vec![COLLECT], ..Default::default() }; // 文件收入策略。
        let text = ContentOffer { text: true, targets: vec![COLLECT], ..Default::default() }; // 文字不能让来源删除。
        let allowed = DROPEFFECT_COPY | DROPEFFECT_MOVE; // 来源提供的可用效果。
        assert_eq!(content_effect(&files, COLLECT, "move", allowed, MODIFIERKEYS_FLAGS(0)), DROPEFFECT_MOVE);
        assert_eq!(content_effect(&files, COLLECT, "move", allowed, MK_CONTROL), DROPEFFECT_NONE);
        assert_eq!(content_effect(&files, COLLECT, "link", allowed, MODIFIERKEYS_FLAGS(0)), DROPEFFECT_COPY);
        assert_eq!(content_effect(&text, COLLECT, "move", allowed, MODIFIERKEYS_FLAGS(0)), DROPEFFECT_COPY);
        assert_eq!(content_effect(&text, COLLECT, "copy", allowed, MK_SHIFT), DROPEFFECT_NONE);
        assert_eq!(content_effect(&text, COLLECT, "copy", DROPEFFECT_MOVE, MODIFIERKEYS_FLAGS(0)), DROPEFFECT_NONE);
        assert_eq!(content_effect(&files, "official.local-clock", "copy", allowed, MODIFIERKEYS_FLAGS(0)), DROPEFFECT_NONE);
        std::fs::remove_dir_all(root).unwrap();
    }

    /// 最小真实位图验证行方向、像素转换、截断和尺寸限制。
    #[test]
    fn image_data_is_bounded_and_decodable()
    {
        let mut dib = vec![0u8; 44]; // 单个 BGR 像素含行填充。
        dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&1i32.to_le_bytes());
        dib[8..12].copy_from_slice(&1i32.to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&24u16.to_le_bytes());
        dib[40..43].copy_from_slice(&[3, 2, 1]);
        let png = image_png(&dib, true).unwrap(); // 真实转换结果。
        assert_eq!(image_png(&png, false).unwrap(), png);
        let mut reader = png::Decoder::new(std::io::Cursor::new(&png)).read_info().unwrap(); // 解码核对 RGB。
        let mut pixels = vec![0; reader.output_buffer_size()]; // 精确输出大小。
        reader.next_frame(&mut pixels).unwrap();
        assert_eq!(pixels, [1, 2, 3]);
        assert!(image_png(&dib[..42], true).is_err());
        dib[8..12].copy_from_slice(&(-1i32).to_le_bytes());
        assert_eq!(image_png(&dib, true).unwrap(), png);
        dib[4..8].copy_from_slice(&i32::MAX.to_le_bytes());
        assert!(image_png(&dib, true).is_err());
        assert!(image_png(b"not an image", false).is_err());
    }

    /// 模拟浏览器只暴露内容格式；存储、提取和释放仍使用真实 Shell COM 对象。
    #[windows::core::implement(IDataObject)]
    struct ContentData
    {
        inner: IDataObject,
        formats: std::rc::Rc<std::cell::RefCell<Vec<u16>>>,
    } // 声明测试源实际提供的表示。
    impl windows::Win32::System::Com::IDataObject_Impl for ContentData_Impl
    {
        fn GetData(&self, format: *const FORMATETC) -> windows::core::Result<windows::Win32::System::Com::STGMEDIUM> { unsafe { self.inner.GetData(format) } }
        fn QueryGetData(&self, value: *const FORMATETC) -> windows::core::HRESULT
        {
            let id = unsafe { (*value).cfFormat }; // COM 测试调用提供有效描述。
            if self.formats.borrow().contains(&id)
            {
                windows::Win32::Foundation::S_OK
            }
            else
            {
                windows::Win32::Foundation::E_NOTIMPL
            }
        }
        fn GetDataHere(&self, _: *const FORMATETC, _: *mut windows::Win32::System::Com::STGMEDIUM) -> windows::core::Result<()> { Err(windows::Win32::Foundation::E_NOTIMPL.into()) }
        fn GetCanonicalFormatEtc(&self, _: *const FORMATETC, _: *mut FORMATETC) -> windows::core::HRESULT { windows::Win32::Foundation::E_NOTIMPL }
        fn SetData(&self, _: *const FORMATETC, _: *const windows::Win32::System::Com::STGMEDIUM, _: windows::core::BOOL) -> windows::core::Result<()> { Err(windows::Win32::Foundation::E_NOTIMPL.into()) }
        fn EnumFormatEtc(&self, _: u32) -> windows::core::Result<windows::Win32::System::Com::IEnumFORMATETC> { Err(windows::Win32::Foundation::E_NOTIMPL.into()) }
        fn DAdvise(&self, _: *const FORMATETC, _: u32, _: windows::core::Ref<'_, windows::Win32::System::Com::IAdviseSink>) -> windows::core::Result<u32> { Err(windows::Win32::Foundation::E_NOTIMPL.into()) }
        fn DUnadvise(&self, _: u32) -> windows::core::Result<()> { Err(windows::Win32::Foundation::E_NOTIMPL.into()) }
        fn EnumDAdvise(&self) -> windows::core::Result<windows::Win32::System::Com::IEnumSTATDATA> { Err(windows::Win32::Foundation::E_NOTIMPL.into()) }
    }

    /// 真实 COM 数据对象验证格式优先级、UTF-16、介质释放与失败不回退。
    #[test]
    fn ole_content_selects_one_representation()
    {
        use windows::Win32::System::Com::{STGMEDIUM, STGMEDIUM_0};
        use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc};
        use windows::Win32::System::Ole::{OleInitialize, OleUninitialize};
        use windows::Win32::UI::Shell::SHCreateDataObject;
        unsafe { OleInitialize(None) }.unwrap();
        {
            let data: IDataObject = unsafe { SHCreateDataObject(None, None, None) }.unwrap(); // 无来源文件的真实数据容器。
            let formats = std::rc::Rc::new(std::cell::RefCell::new(Vec::new())); // 只声明已成功提供的格式。
            let put = |id: u16, bytes: &[u8]| {
                let memory = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) }.unwrap(); // IDataObject 接管存储。
                let pointer = unsafe { GlobalLock(memory) }.cast::<u8>(); // 测试格式字节。
                assert!(!pointer.is_null());
                unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer, bytes.len()) };
                let _ = unsafe { GlobalUnlock(memory) };
                let medium = STGMEDIUM { tymed: TYMED_HGLOBAL.0 as u32, u: STGMEDIUM_0 { hGlobal: memory }, ..Default::default() }; // SetData 成功后不再自行释放。
                unsafe { data.SetData(&format(id), &medium, true) }.unwrap();
                formats.borrow_mut().push(id);
            }; // 只替换源数据，运行真实格式探测和提取。
            let text = "中文与 emoji 📥\0".encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<_>>(); // Unicode 正文。
            put(13, &text);
            let source: IDataObject = ContentData { inner: data.clone(), formats: formats.clone() }.into(); // 内容源不带 Shell 文件表示。
            let offered = offer(&source, &[]); // 只有文字时接收收集箱。
            assert_eq!(offered.targets, [COLLECT]);
            assert!(matches!(extract(&data, vec![], &offered).unwrap(), crate::collect::IncomingContent::Text(text) if text == "中文与 emoji 📥"));
            assert!(read_bytes(&data, 13, 2).is_err());
            let url = unsafe { RegisterClipboardFormatW(w!("UniformResourceLocatorW")) } as u16; // 多表示链接。
            put(url, &"https://example.com/\0".encode_utf16().flat_map(u16::to_le_bytes).collect::<Vec<_>>());
            assert!(matches!(extract(&source, vec![], &offer(&source, &[])).unwrap(), crate::collect::IncomingContent::Link(_)));
            let png = unsafe { RegisterClipboardFormatW(w!("PNG")) } as u16; // 图像比链接和文字优先。
            put(png, b"broken PNG");
            assert!(matches!(extract(&source, vec![], &offer(&source, &[])).unwrap(), crate::collect::IncomingContent::Image { .. }));
            let paths = vec!["missing.png".to_string()]; // 文件表示存在时不得回退到文字。
            let offered = offer(&data, &paths); // 不完整文件来源。
            assert!(offered.files && offered.targets.is_empty());
        }
        unsafe { OleUninitialize() };
    }
}
