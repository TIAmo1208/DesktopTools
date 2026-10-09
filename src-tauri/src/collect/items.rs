use super::{CollectItem, directory};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CollectKind
{
    Text,
    Image,
    File,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ItemInfo
{
    pub kind: CollectKind,
    pub display_name: String,
    pub source_path: Option<String>,
    pub created_unix_ms: i64,
}

pub(crate) fn items_directory(config: &Path) -> PathBuf { directory(config).join("items") }
pub(crate) fn item_directory(config: &Path, name: &str) -> io::Result<PathBuf>
{
    if !name.strip_prefix("item-").is_some_and(|suffix| !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "收集项身份无效"));
    }
    Ok(items_directory(config).join(name))
}
pub(crate) fn get_item(config: &Path, name: &str) -> io::Result<CollectItem>
{
    let root = directory(config);
    if is_image_path(Path::new(name))
    {
        let path = image_file(config, name)?; // 独立图片仅位于 collect 根目录。
        let metadata = fs::metadata(&path)?; // 文件名作为身份，创建时间来自文件事实。
        if !metadata.is_file()
        {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "图片不是文件"));
        }
        return Ok(CollectItem { name: name.into(), kind: CollectKind::Image, display_name: name.into(), path: Some(path.to_string_lossy().into()), content: String::new(), created_unix_ms: metadata.created().or_else(|_| metadata.modified())?.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64, pinned: super::read_pinned(&root)?.iter().any(|item| item == name), available: true });
    }
    if name.ends_with(".md")
    {
        let path = super::safe_file(&root, name)?;
        let content = fs::read_to_string(&path)?;
        let metadata = fs::metadata(&path)?;
        let pinned = super::read_pinned(&root)?.iter().any(|item| item == name);
        return Ok(CollectItem { name: name.into(), kind: CollectKind::Text, display_name: name.trim_end_matches(".md").into(), path: Some(path.to_string_lossy().into()), content, created_unix_ms: metadata.created().or_else(|_| metadata.modified())?.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64, pinned, available: true });
    }
    let path = item_directory(config, name)?;
    let info: ItemInfo = serde_json::from_slice(&fs::read(path.join("info.json"))?).map_err(io::Error::other)?;
    if info.kind != CollectKind::File
    {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "映射记录类型无效"));
    }
    let actual = info.source_path.as_deref().map(PathBuf::from); // 映射只记录源路径。
    let available = actual.as_ref().is_some_and(|path| path.is_file());
    Ok(CollectItem { name: name.into(), kind: info.kind, display_name: info.display_name, path: actual.as_ref().map(|p| p.to_string_lossy().into()), content: String::new(), created_unix_ms: info.created_unix_ms, pinned: super::read_pinned(&root)?.iter().any(|item| item == name), available })
}

pub(crate) fn create_image(config: &Path, display_name: &str, bytes: &[u8]) -> io::Result<CollectItem>
{
    use std::io::Write;
    let root = directory(config); // 图片与 Markdown 平级，不放入文档资源目录。
    fs::create_dir_all(&root)?;
    let display = if is_image_path(Path::new(display_name))
    {
        display_name.to_string()
    }
    else
    {
        format!("{display_name}.png")
    }; // 无路径图像已统一转换为 PNG。
    let requested = image_file(config, &display)?; // 校验文件名且不接受外部路径。
    loop
    {
        let path = super::unique_resource_path(&root, requested.file_name().unwrap()); // 同名加后缀，保留现有文件。
        let mut file = match fs::OpenOptions::new().write(true).create_new(true).open(&path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }; // 独占创建覆盖检查到写入之间的竞态。
        let written = file.write_all(bytes).and_then(|()| file.sync_all()); // 完整写入后才发布条目。
        drop(file);
        if let Err(error) = written
        {
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        return get_item(config, &path.file_name().unwrap().to_string_lossy());
    }
}

pub(crate) fn create_mapping(config: &Path, display_name: &str, source_path: &str) -> io::Result<CollectItem>
{
    let name = new_item_name(config);
    let info = ItemInfo { kind: CollectKind::File, display_name: display_name.to_string(), source_path: Some(source_path.to_string()), created_unix_ms: super::now_ms() };
    write_item(config, &name, &info)
}

fn new_item_name(config: &Path) -> String
{
    let mut stamp = crate::tools::time::next_timeline_timestamp();
    loop
    {
        let name = format!("item-{stamp}");
        if !item_directory(config, &name).is_ok_and(|path| path.exists())
        {
            return name;
        }
        stamp += 1;
    }
}
fn write_item(config: &Path, name: &str, info: &ItemInfo) -> io::Result<CollectItem>
{
    let path = item_directory(config, name)?;
    fs::create_dir_all(items_directory(config))?;
    fs::create_dir(&path)?; // 独占创建，拒绝覆盖已有条目。
    fs::write(path.join("info.json"), serde_json::to_vec_pretty(info).map_err(io::Error::other)?)?;
    get_item(config, name)
}

/// 独立图片身份必须是根目录中的普通文件名，禁止路径与重定向。
pub(super) fn image_file(config: &Path, name: &str) -> io::Result<PathBuf>
{
    if name.trim() != name || name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|', '\0']) || !is_image_path(Path::new(name))
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "图片文件名无效"));
    }
    let path = directory(config).join(name); // 仅根目录直接子文件。
    if path.exists() && (path.symlink_metadata()?.file_type().is_symlink() || path.canonicalize()?.parent() != Some(directory(config).canonicalize()?.as_path()))
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "图片路径不能重定向"));
    }
    Ok(path)
}

/// 扫描和收入共用当前图片文件白名单。
pub(super) fn is_image_path(path: &Path) -> bool { matches!(path.extension().and_then(|value| value.to_str()).unwrap_or("").to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "ico") }

#[cfg(test)]
mod tests
{
    use super::*;

    #[test]
    fn item_identity_rejects_path_escape()
    {
        let root = Path::new("collect-test");
        assert_eq!(item_directory(root, "item-123").unwrap(), directory(root).join("items/item-123"));
        for name in ["", ".", "..", "item-", "item-../outside", "C:\\outside", "item-1 ", "item-1."]
        {
            assert!(item_directory(root, name).is_err(), "{name}");
        }
        for name in ["../outside.md", "C:\\outside.md", "a/b.md"]
        {
            assert_eq!(get_item(root, name).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        }
    }

    #[test]
    fn deleted_mapping_source_remains_unavailable()
    {
        let root = std::env::temp_dir().join(format!("collect-mapping-{}", crate::tools::time::next_timeline_timestamp())); // 本测试独占目录。
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.txt"); // 模拟外部删除的映射目标。
        fs::write(&source, "content").unwrap();
        let item = create_mapping(&root, "source.txt", source.to_str().unwrap()).unwrap(); // 映射记录保留独立身份。
        assert!(item.available);
        fs::remove_file(&source).unwrap();
        let error = crate::platform::windows::settings::open_path(&source).unwrap_err(); // 真实 Shell 错误必须有原因和路径。
        assert!(error.contains("文件不存在或已被删除"), "{error}");
        assert!(error.contains(source.to_str().unwrap()), "{error}");
        let items = super::super::list(&root).unwrap(); // 失败后的刷新仍保留映射。
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, item.name);
        assert!(!items[0].available);
        fs::write(&source, "restored").unwrap();
        assert!(get_item(&root, &item.name).unwrap().available);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn root_images_keep_identity_and_do_not_scan_document_resources()
    {
        let root = std::env::temp_dir().join(format!("collect-root-images-{}", crate::tools::time::next_timeline_timestamp())); // 本测试独占目录。
        let bytes = b"\x89PNG\r\n\x1a\n"; // 图片读取的 PNG 签名。
        let note = super::super::create(&root, "正文").unwrap(); // 插图仍归属同名文档目录。
        super::super::write_resource_bytes(&root, &note.name, "reference.png", bytes).unwrap();
        let image = create_image(&root, "reference.png", bytes).unwrap();
        let second = create_image(&root, "reference.png", bytes).unwrap(); // 同名收入保留两份。
        assert_eq!(image.name, "reference.png");
        assert_eq!(second.name, "reference-2.png");
        assert_eq!(Path::new(image.path.as_ref().unwrap()).parent(), Some(directory(&root).as_path()));
        assert_eq!(super::super::list(&root).unwrap().len(), 3);
        assert!(super::super::read_image(&root, &image.name, "ignored.png").unwrap().starts_with("data:image/png;base64,"));
        assert!(get_item(&root, "../reference.png").is_err());
        super::super::set_pinned(&root, &image.name, true).unwrap();
        super::super::delete(&root, &image.name).unwrap();
        fs::write(directory(&root).join(&image.name), b"conflict").unwrap();
        assert_eq!(super::super::restore_latest(&root).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(directory(&root).join(&image.name)).unwrap(), b"conflict");
        fs::remove_file(directory(&root).join(&image.name)).unwrap();
        let restored = super::super::restore_latest(&root).unwrap().unwrap();
        assert_eq!(restored.name, image.name);
        assert!(restored.pinned);
        assert_eq!(fs::read(restored.path.unwrap()).unwrap(), bytes);
        assert!(directory(&root).join(note.name.trim_end_matches(".md")).join("reference.png").is_file());
        fs::remove_dir_all(root).unwrap();
    }
}
