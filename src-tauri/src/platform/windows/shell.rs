use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::null_mut;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, HGDIOBJ, SelectObject,
};
use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemAlloc, CoTaskMemFree, CoUninitialize,
};
use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    ILGetSize, SHCONTF_FOLDERS, SHCONTF_INCLUDEHIDDEN, SHCONTF_NONFOLDERS, SHFILEINFOW, SHGFI_PIDL,
    SHGFI_SYSICONINDEX, SHGetDesktopFolder, SHGetFileInfoW, SHGetImageList, SHGetNameFromIDList,
    SHIL_EXTRALARGE, SHParseDisplayName, SIGDN, SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_NORMALDISPLAY,
};
use windows::Win32::UI::WindowsAndMessaging::{DI_NORMAL, DestroyIcon, DrawIconEx, HICON};
use windows::core::HSTRING;

use crate::contracts::{DesktopItem, DesktopItemAvailability};

/// 持有由 Shell 或 COM 分配的 PIDL。
struct OwnedPidl(*mut ITEMIDLIST);

impl Drop for OwnedPidl {
    /// 释放 PIDL 使用的 COM 任务内存。
    fn drop(&mut self) {
        unsafe { CoTaskMemFree(Some(self.0.cast())) };
    }
}

/// 初始化专用 Shell 线程的 STA COM apartment。
pub fn initialize_shell_apartment() -> Result<(), String> {
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
        .ok()
        .map_err(|error| format!("初始化 Shell STA 失败: {error}"))
}

/// 释放专用 Shell 线程的 COM apartment。
pub fn uninitialize_shell_apartment() {
    unsafe { CoUninitialize() };
}

/// 枚举 Windows Shell 桌面命名空间中的完整项目集合。
pub fn enumerate_desktop_items() -> Result<Vec<DesktopItem>, String> {
    // Windows Shell 桌面根目录接口。
    let desktop = unsafe { SHGetDesktopFolder() }
        .map_err(|error| format!("获取 Shell 桌面命名空间失败: {error}"))?;
    // Shell 桌面项目枚举器。
    let mut enumerator = None;
    // 同时包含文件夹、普通项目和隐藏项目的枚举标志。
    let flags = (SHCONTF_FOLDERS.0 | SHCONTF_NONFOLDERS.0 | SHCONTF_INCLUDEHIDDEN.0) as u32;
    unsafe { desktop.EnumObjects(HWND::default(), flags, &mut enumerator) }
        .ok()
        .map_err(|error| format!("创建 Shell 桌面项目枚举器失败: {error}"))?;
    // 已创建的 Shell 桌面项目枚举器。
    let enumerator = enumerator.ok_or_else(|| "Shell 桌面项目枚举器为空".to_string())?;
    // 本次枚举观察到的完整项目集合。
    let mut items = Vec::new();

    loop {
        // Shell 为当前项目分配的 PIDL。
        let mut pidl: *mut ITEMIDLIST = null_mut();
        // 当前调用实际返回的项目数量。
        let mut fetched = 0;
        // 当前项目枚举结果。
        let result =
            unsafe { enumerator.Next(std::slice::from_mut(&mut pidl), Some(&mut fetched)) };
        result
            .ok()
            .map_err(|error| format!("读取 Shell 桌面项目失败: {error}"))?;
        if fetched == 0 {
            break;
        }
        if pidl.is_null() {
            continue;
        }

        // 当前项目的可重新解析稳定引用。
        let stable_reference = shell_name(pidl, SIGDN_DESKTOPABSOLUTEPARSING)
            .unwrap_or_else(|_| encoded_pidl_reference(pidl));
        // 从稳定引用重新解析出的 PIDL。
        let resolved_pidl = parse_desktop_item_reference(&stable_reference);
        // 当前项目的系统显示名称读取结果。
        let display_name = match &resolved_pidl {
            Ok(resolved) => shell_name(resolved.0, SIGDN_NORMALDISPLAY),
            Err(error) => Err(error.clone()),
        };
        // 当前项目的系统图像读取结果。
        let system_image = resolved_pidl
            .as_ref()
            .ok()
            .and_then(|resolved| system_image_data_url(resolved.0));
        // 当前项目元数据可解析状态。
        let availability = if display_name.is_ok() && system_image.is_some() {
            DesktopItemAvailability::Available
        } else {
            DesktopItemAvailability::Unavailable
        };
        // 元数据失败时仍可呈现并恢复的项目名称。
        let display_name = display_name.unwrap_or_else(|_| stable_reference.clone());

        items.push(DesktopItem {
            stable_reference,
            display_name,
            system_image,
            availability,
        });
        unsafe { CoTaskMemFree(Some(pidl.cast())) };
    }

    Ok(items)
}

/// 读取 PIDL 对应的 Shell 名称。
fn shell_name(pidl: *const ITEMIDLIST, kind: SIGDN) -> Result<String, String> {
    // Shell 分配的名称缓冲区。
    let name_pointer = unsafe { SHGetNameFromIDList(pidl, kind) }
        .map_err(|error| format!("读取 Shell 项目名称失败: {error}"))?;
    // 已转换的 UTF-8 Shell 名称。
    let name = unsafe { name_pointer.to_string() }
        .map_err(|error| format!("转换 Shell 项目名称失败: {error}"));
    unsafe { CoTaskMemFree(Some(name_pointer.0.cast())) };
    name
}

/// 将 PIDL 编码为可重新提交给 Shell 的稳定兜底引用。
fn encoded_pidl_reference(pidl: *const ITEMIDLIST) -> String {
    // Shell PIDL 的完整字节长度。
    let byte_length = unsafe { ILGetSize(Some(pidl)) } as usize;
    // 当前 PIDL 的二进制内容。
    let bytes = unsafe { std::slice::from_raw_parts(pidl.cast::<u8>(), byte_length) };
    format!("pidl:{}", STANDARD.encode(bytes))
}

/// 将稳定引用重新解析为 Shell PIDL。
fn parse_desktop_item_reference(reference: &str) -> Result<OwnedPidl, String> {
    if let Some(encoded) = reference.strip_prefix("pidl:") {
        // 从稳定引用解码并校验的 PIDL 字节。
        let bytes = decode_pidl_bytes(encoded)?;
        // COM 任务分配器提供的 PIDL 内存。
        let pidl = unsafe { CoTaskMemAlloc(bytes.len()) }.cast::<ITEMIDLIST>();
        if pidl.is_null() {
            return Err("分配 Shell PIDL 内存失败".to_string());
        }
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), pidl.cast::<u8>(), bytes.len()) };
        return Ok(OwnedPidl(pidl));
    }

    // 传给 Shell 的稳定解析名称。
    let display_name = HSTRING::from(reference);
    // Shell 分配的解析结果 PIDL。
    let mut pidl = null_mut();
    unsafe { SHParseDisplayName(&display_name, None, &mut pidl, 0, None) }
        .map_err(|error| format!("重新解析 Shell 项目引用失败: {error}"))?;
    if pidl.is_null() {
        return Err("Shell 项目引用解析结果为空".to_string());
    }

    Ok(OwnedPidl(pidl))
}

/// 解码并校验序列化 PIDL 的项目边界与终止项。
fn decode_pidl_bytes(encoded: &str) -> Result<Vec<u8>, String> {
    // Base64 解码后的 PIDL 字节。
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|error| format!("解码 Shell PIDL 引用失败: {error}"))?;
    // 当前 SHITEMID 的起始偏移。
    let mut offset = 0;

    loop {
        if offset + 2 > bytes.len() {
            return Err("Shell PIDL 引用缺少终止项".to_string());
        }

        // 当前 SHITEMID 的完整字节长度。
        let item_length = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]) as usize;
        if item_length == 0 {
            if offset + 2 == bytes.len() {
                return Ok(bytes);
            }
            return Err("Shell PIDL 引用包含终止项后的数据".to_string());
        }
        if item_length < 2 || offset + item_length > bytes.len() {
            return Err("Shell PIDL 引用包含无效项目长度".to_string());
        }

        offset += item_length;
    }
}

/// 读取 PIDL 对应的 48 像素 Shell 系统图像。
fn system_image_data_url(pidl: *const ITEMIDLIST) -> Option<String> {
    // Shell 系统图像索引结果。
    let mut file_info = SHFILEINFOW::default();
    // PIDL 查询结果标志。
    let query_result = unsafe {
        SHGetFileInfoW(
            windows::core::PCWSTR(pidl.cast::<u16>()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut file_info),
            size_of::<SHFILEINFOW>() as u32,
            SHGFI_PIDL | SHGFI_SYSICONINDEX,
        )
    };
    if query_result == 0 {
        return None;
    }

    // 与 Explorer 桌面大图标模式一致的 HICON。
    let icon = unsafe {
        SHGetImageList::<IImageList>(SHIL_EXTRALARGE as i32)
            .ok()?
            .GetIcon(file_info.iIcon, ILD_TRANSPARENT.0)
            .ok()?
    };
    // 供浏览器直接呈现的 PNG data URL。
    let encoded = unsafe { icon_data_url(icon) };
    unsafe {
        let _ = DestroyIcon(icon);
    }
    encoded
}

/// 将 Shell HICON 编码为浏览器可直接渲染的 PNG data URL。
unsafe fn icon_data_url(icon: HICON) -> Option<String> {
    /// Desktop Tools 桌面项目图标边长。
    const ICON_SIZE: i32 = 48;
    // 用于绘制图标的内存设备上下文。
    let device_context = unsafe { CreateCompatibleDC(None) };
    if device_context.is_invalid() {
        return None;
    }

    // 自顶向下的 32 位 BGRA DIB 描述。
    let bitmap_info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: ICON_SIZE,
            biHeight: -ICON_SIZE,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    // DIB 像素内存首地址。
    let mut bits: *mut c_void = null_mut();
    // 承载系统图标的 DIB 句柄。
    let bitmap = unsafe {
        match CreateDIBSection(
            Some(device_context),
            &bitmap_info,
            DIB_RGB_COLORS,
            &mut bits,
            None,
            0,
        ) {
            Ok(value) => value,
            Err(_) => {
                let _ = DeleteDC(device_context);
                return None;
            }
        }
    };
    // 设备上下文之前选中的 GDI 对象。
    let previous_bitmap = unsafe { SelectObject(device_context, HGDIOBJ(bitmap.0)) };
    // DIB 像素字节长度。
    let byte_length = (ICON_SIZE * ICON_SIZE * 4) as usize;
    unsafe { std::ptr::write_bytes(bits, 0, byte_length) };
    // 系统图标绘制结果。
    let draw_result = unsafe {
        DrawIconEx(
            device_context,
            0,
            0,
            icon,
            ICON_SIZE,
            ICON_SIZE,
            0,
            None,
            DI_NORMAL,
        )
    };
    // 从 DIB 复制出的 BGRA 数据。
    let bgra = unsafe { std::slice::from_raw_parts(bits.cast::<u8>(), byte_length) };
    // 当前图标是否已经携带有效 alpha。
    let has_alpha = bgra.chunks_exact(4).any(|pixel| pixel[3] != 0);
    // PNG 编码器需要的 RGBA 数据。
    let mut rgba = Vec::with_capacity(byte_length);
    for pixel in bgra.chunks_exact(4) {
        // 旧式 HICON 缺少 alpha 时按非黑像素补全不透明度。
        let alpha = if has_alpha {
            pixel[3]
        } else if pixel[0] != 0 || pixel[1] != 0 || pixel[2] != 0 {
            255
        } else {
            0
        };
        rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], alpha]);
    }

    unsafe {
        SelectObject(device_context, previous_bitmap);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(device_context);
    }
    if draw_result.is_err() {
        return None;
    }

    // PNG 文件内容。
    let mut encoded = Vec::new();
    {
        // 48x48 RGBA PNG 编码器。
        let mut encoder = png::Encoder::new(&mut encoded, ICON_SIZE as u32, ICON_SIZE as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        // PNG 数据写入器。
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&rgba).ok()?;
    }

    Some(format!(
        "data:image/png;base64,{}",
        STANDARD.encode(encoded)
    ))
}

#[cfg(test)]
mod tests {
    use super::{encoded_pidl_reference, parse_desktop_item_reference};

    /// 验证 PIDL 稳定引用可解码并保持原始字节。
    #[test]
    fn encoded_pidl_reference_round_trips() {
        // 表示桌面根节点的终止 PIDL。
        let bytes = [0_u8, 0];
        // 编码后的 PIDL 稳定引用。
        let reference = encoded_pidl_reference(bytes.as_ptr().cast());
        // 从稳定引用重新解析出的 PIDL。
        let parsed = parse_desktop_item_reference(&reference).expect("PIDL 稳定引用应可重新解析");

        assert_eq!(encoded_pidl_reference(parsed.0), reference);
    }
}
