use chrono::Local;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
pub(crate) mod cleanup;
mod drop;
mod items;
mod references;
mod transfer;
pub(crate) use drop::{IncomingContent, receive};
pub(crate) use items::{CollectKind, get_item};
pub(crate) use transfer::{CollectFailure, PreparedExport, ReceiveResult, prepare_export, receive_items};

/// 三类收集项共享的显示快照；name 是操作身份，不是外部源路径。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectItem
{
    pub name: String,         // 文本文件名或新类型稳定 ID。
    pub kind: CollectKind,    // 业务类型。
    pub display_name: String, // 用户可读名称，与操作身份分离。
    pub path: Option<String>, // 当前实际文件路径，失效映射仍保留源路径。
    pub content: String,
    pub created_unix_ms: i64,
    pub pinned: bool,
    pub available: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeletedCollectInfo
{
    original_name: String,
    pinned: bool,
    deleted_at_unix_ms: i64,
}

/// 返回配置目录下共享的 collect 目录。
pub fn directory(config: &Path) -> PathBuf { config.join("collect") }

/// 读取本地图片原始字节，保留 GIF 动画并限制不可信文件的读取体积。
pub fn read_image(config: &Path, name: &str, reference: &str) -> io::Result<String>
{
    use base64::Engine;
    use std::io::Read;
    let root = directory(config); // 相对引用以 Markdown 所在目录为基准。
    let item = get_item(config, name)?; // 文件名或映射身份统一校验。
    let path = if !name.ends_with(".md")
    {
        if item.kind != CollectKind::Image
        {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "收集项不是图片"));
        }
        PathBuf::from(item.path.ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "图片路径不可用"))?)
    }
    else if reference.starts_with("file:")
    {
        tauri::Url::parse(reference).ok().and_then(|url| url.to_file_path().ok()).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "图片文件 URL 无效"))?
    }
    else
    {
        root.join(reference)
    }; // 绝对路径由 Path::join 保留，支持仅链接模式。
    let mut bytes = Vec::new(); // 有界读取避免文件在检查后增长突破限制。
    fs::File::open(path)?.take(32 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 32 * 1024 * 1024
    {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "图片超过 32 MiB"));
    }
    let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n")
    {
        "image/png"
    }
    else if bytes.starts_with(b"\xff\xd8\xff")
    {
        "image/jpeg"
    }
    else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")
    {
        "image/gif"
    }
    else if bytes.starts_with(b"BM")
    {
        "image/bmp"
    }
    else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP")
    {
        "image/webp"
    }
    else if bytes.starts_with(b"\0\0\x01\0")
    {
        "image/x-icon"
    }
    else
    {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "不支持的图片格式"));
    }; // 不将任意文件作为可执行页面返回。
    Ok(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)))
}

/// 读取当前收集项和置顶表。
pub fn list(config: &Path) -> io::Result<Vec<CollectItem>>
{
    let root = directory(config);
    fs::create_dir_all(&root)?;
    let pinned = read_pinned(&root)?;
    let mut items = fs::read_dir(&root)?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|value| value == "md"))
        .filter_map(|entry| {
            let path = entry.path();
            let content = fs::read_to_string(&path).ok()?;
            let metadata = entry.metadata().ok()?;
            let created_unix_ms = metadata.created().or_else(|_| metadata.modified()).ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64;
            let name = path.file_name()?.to_string_lossy().into_owned();
            Some(CollectItem { kind: CollectKind::Text, display_name: name.trim_end_matches(".md").to_string(), path: Some(path.to_string_lossy().into_owned()), pinned: pinned.iter().any(|item| item == &name), name, content, created_unix_ms, available: true })
        })
        .collect::<Vec<_>>();
    for entry in fs::read_dir(items::items_directory(config)).into_iter().flatten().filter_map(Result::ok)
    {
        if entry.path().is_dir()
            && let Some(name) = entry.file_name().to_str()
            && let Ok(item) = items::get_item(config, name)
        {
            items.push(item);
        }
    }
    for entry in fs::read_dir(&root)?.filter_map(Result::ok)
    {
        if items::is_image_path(&entry.path())
            && let Some(name) = entry.file_name().to_str()
        {
            items.push(get_item(config, name)?); // 只扫描根目录，不重复列出文档资源。
        }
    }
    items.sort_by(|a, b| b.pinned.cmp(&a.pinned).then(b.created_unix_ms.cmp(&a.created_unix_ms)).then(a.name.cmp(&b.name)));
    Ok(items)
}

/// 创建一个带秒级时间名的空 Markdown 收集项。
pub fn create(config: &Path, content: &str) -> io::Result<CollectItem>
{
    let root = directory(config);
    fs::create_dir_all(&root)?;
    let base = Local::now().format("%Y-%m-%d-%H-%M-%S").to_string();
    let mut index = 1;
    let path = loop
    {
        let suffix = if index == 1 { String::new() } else { format!("-{index}") };
        let candidate = root.join(format!("{base}{suffix}.md"));
        if !candidate.exists()
        {
            break candidate;
        }
        index += 1;
    };
    fs::write(&path, content)?;
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    Ok(CollectItem { name, kind: CollectKind::Text, display_name: path.file_stem().unwrap_or_default().to_string_lossy().into_owned(), path: Some(path.to_string_lossy().into_owned()), content: content.to_string(), created_unix_ms: now_ms(), pinned: false, available: true })
}

/// 覆盖保存指定 Markdown 文件。
pub fn save(config: &Path, name: &str, content: &str) -> io::Result<()>
{
    use std::io::Write;
    let target = safe_file(&directory(config), name)?; // 复用路径边界。
    let temporary = target.with_extension(format!("md.{}.tmp", crate::tools::time::next_timeline_timestamp())); // 同卷独占临时文件。
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?; // 失败不截断正文。
    let result = file.write_all(content.as_bytes()).and_then(|()| file.sync_all()); // 完整写入后才替换。
    drop(file);
    let result = result.and_then(|()| crate::platform::windows::replace_file_atomic(temporary.as_os_str(), target.as_os_str())); // 沿用既有原子替换。
    if result.is_err()
    {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// 按当前收入方式处理一份外部图片，并返回可写入 Markdown 的引用路径。
pub fn import_resource(config: &Path, name: &str, source: &Path, mode: &str) -> io::Result<String>
{
    let resource_dir = resource_directory(config, name)?; // 先校验所属收集项，再执行源文件操作。
    let source = source.canonicalize()?;
    if !source.is_file()
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "图片路径无效"));
    }
    if mode == "link"
    {
        return Ok(source.to_string_lossy().into_owned());
    }
    if mode != "copy" && mode != "move"
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "图片收入方式无效"));
    }
    fs::create_dir_all(&resource_dir)?;
    let file_name = source.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "图片文件名无效"))?;
    let source_parent = source.parent().and_then(|parent| parent.canonicalize().ok()); // 外部图片的真实父目录。
    let resource_root = resource_dir.canonicalize()?; // 当前条目的真实资源目录。
    if source_parent.as_deref() == Some(resource_root.as_path())
    {
        return Ok(format!("{}/{}", resource_dir.file_name().unwrap().to_string_lossy(), file_name.to_string_lossy()));
    }
    let target = unique_resource_path(&resource_dir, file_name);
    if mode == "copy"
    {
        fs::copy(&source, &target)?;
    }
    else
    {
        crate::platform::windows::move_resource_file(&source, &target)?;
    }
    cleanup::record(&resource_dir, &target);
    Ok(format!("{}/{}", resource_dir.file_name().unwrap().to_string_lossy(), target.file_name().unwrap().to_string_lossy()))
}

/// 将 Markdown 图片引用解析为本地文件后收入当前条目的资源目录。
pub fn import_rendered_resource(config: &Path, name: &str, reference: &str, mode: &str) -> io::Result<String>
{
    let reference = reference.trim();
    let source = if Path::new(reference).is_absolute()
    {
        PathBuf::from(reference)
    }
    else
    {
        directory(config).join(reference)
    };
    import_resource(config, name, &source, mode)
}

/// 将剪贴板图片字节写入条目资源目录。
pub fn write_resource_bytes(config: &Path, name: &str, file_name: &str, bytes: &[u8]) -> io::Result<String>
{
    let resource_dir = resource_directory(config, name)?; // 剪贴板和文件导入采用同一归属校验。
    let safe_name = Path::new(file_name).file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "图片文件名无效"))?;
    if safe_name.to_string_lossy().contains([':', '\0'])
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "图片文件名无效"));
    }
    fs::create_dir_all(&resource_dir)?;
    let target = unique_resource_path(&resource_dir, safe_name);
    fs::write(&target, bytes)?;
    cleanup::record(&resource_dir, &target);
    Ok(format!("{}/{}", resource_dir.file_name().unwrap().to_string_lossy(), target.file_name().unwrap().to_string_lossy()))
}

/// 资源必须归属现存收集项的同名旁侧目录，拒绝路径越界与删除包目录冲突。
fn resource_directory(config: &Path, name: &str) -> io::Result<PathBuf>
{
    let root = directory(config).canonicalize()?; // 当前共享目录的真实路径。
    let item = safe_file(&root, name)?; // 复用 Markdown 文件名校验。
    let real_item = item.canonicalize()?; // 失效条目不能产生孤立资源。
    if !real_item.is_file() || real_item.parent() != Some(root.as_path())
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "收集项资源归属无效"));
    }
    let stem = item.file_stem().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "收集项名称无效"))?; // 只移除末尾的 Markdown 扩展名。
    if stem == "." || stem == ".." || stem.to_string_lossy().eq_ignore_ascii_case("backup")
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "收集项资源目录名称无效"));
    }
    let resource = root.join(stem); // 受限的直接子目录。
    if resource.exists() && (!resource.is_dir() || resource.canonicalize()? != resource)
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "收集项资源目录不能重定向"));
    }
    Ok(resource)
}

fn unique_resource_path(root: &Path, name: &std::ffi::OsStr) -> PathBuf
{
    let stem = Path::new(name).file_stem().and_then(|value| value.to_str()).unwrap_or("image");
    let extension = Path::new(name).extension().and_then(|value| value.to_str()).map(|value| format!(".{value}")).unwrap_or_default();
    let mut index = 1;
    loop
    {
        let suffix = if index == 1 { String::new() } else { format!("-{index}") };
        let path = root.join(format!("{stem}{suffix}{extension}"));
        if !path.exists()
        {
            return path;
        }
        index += 1;
    }
}

/// 修改文件名并保留置顶关系。
pub fn rename(config: &Path, old: &str, new: &str, draft: Option<&str>) -> io::Result<String>
{
    let root = directory(config);
    let source = safe_file(&root, old)?;
    let target_name = sanitize_name(new)?;
    let target = root.join(target_name);
    let old_resource_name = source.file_stem().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "原收集项名称无效"))?.to_string_lossy().into_owned();
    let new_resource_name = target.file_stem().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "新收集项名称无效"))?.to_string_lossy().into_owned();
    if target.exists()
    {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "收集项名称已存在"));
    }
    let resource = root.join(&old_resource_name);
    let target_resource = root.join(&new_resource_name);
    if resource.exists() && target_resource.exists()
    {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "收集项资源目录已存在"));
    }
    let content = fs::read_to_string(&source)?;
    let resource_exists = resource.exists(); // 磁盘内容和草稿只随实际资源目录移动而改写。
    fs::rename(&source, &target)?;
    if resource_exists
    {
        if let Err(error) = fs::rename(&resource, &target_resource)
        {
            let _ = fs::rename(&target, &source);
            return Err(error);
        }
        let new_content = references::rename_resources(&content, &old_resource_name, &new_resource_name); // 只修改真实链接目标。
        let temporary = target.with_extension("md.rename.tmp"); // 同卷临时文件，不截断已有正文。
        let written = (|| -> io::Result<()> {
            use std::io::Write;
            let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?; // 不覆盖其他操作的临时文件。
            let result = (|| -> io::Result<()> {
                file.write_all(new_content.as_bytes())?;
                file.sync_all()
            })(); // 完整写入后才允许替换。
            drop(file);
            let result = result.and_then(|()| crate::platform::windows::replace_file_atomic(temporary.as_os_str(), target.as_os_str())); // 复用现有原子替换能力。
            if result.is_err()
            {
                let _ = fs::remove_file(&temporary);
            }
            result
        })(); // 写入失败时原 Markdown 保持完整。
        if let Err(error) = written
        {
            let _ = fs::rename(&target_resource, &resource);
            let _ = fs::rename(&target, &source);
            return Err(error);
        }
    }
    let mut pinned = read_pinned(&root)?;
    if let Some(item) = pinned.iter_mut().find(|item| *item == old)
    {
        *item = target.file_name().unwrap().to_string_lossy().into_owned();
        fs::write(root.join("pinned.txt"), pinned.join("\n"))?;
    }
    Ok(
        if resource_exists
        {
            references::rename_resources(draft.unwrap_or(&content), &old_resource_name, &new_resource_name)
        }
        else
        {
            draft.unwrap_or(&content).to_string()
        },
    )
}

/// 切换置顶状态。
pub fn set_pinned(config: &Path, name: &str, pinned: bool) -> io::Result<()>
{
    let root = directory(config);
    if items::is_image_path(Path::new(name))
    {
        let _ = items::image_file(config, name)?;
    }
    else if name.ends_with(".md")
    {
        let _ = safe_file(&root, name)?;
    }
    else
    {
        let _ = items::item_directory(config, name)?;
    }
    let mut values = read_pinned(&root)?;
    values.retain(|item| item != name);
    if pinned
    {
        values.push(name.to_string());
    }
    fs::write(root.join("pinned.txt"), values.join("\n"))
}

/// 将文件和同名资源目录移动到 backup 包，并清理最旧删除项至最多十份。
pub fn delete(config: &Path, name: &str) -> io::Result<()>
{
    let root = directory(config);
    let source = if items::is_image_path(Path::new(name))
    {
        items::image_file(config, name)?
    }
    else if name.ends_with(".md")
    {
        safe_file(&root, name)?
    }
    else
    {
        items::item_directory(config, name)?
    };
    if !source.exists()
    {
        return Err(io::Error::new(io::ErrorKind::NotFound, "收集项不存在"));
    }
    let backup = root.join("backup");
    fs::create_dir_all(&backup)?;
    let stamp = Local::now().format("%Y-%m-%d-%H-%M-%S").to_string();
    let package = unique_backup_package(&backup, &stamp);
    fs::create_dir_all(&package)?;
    let pinned = read_pinned(&root)?.iter().any(|item| item == name);
    fs::rename(&source, package.join(name))?;
    let resource = root.join(name.trim_end_matches(".md"));
    if name.ends_with(".md") && resource.is_dir()
    {
        fs::rename(resource, package.join(name.trim_end_matches(".md")))?;
    }
    let info = DeletedCollectInfo { original_name: name.to_string(), pinned, deleted_at_unix_ms: now_ms() };
    fs::write(package.join("info.json"), serde_json::to_vec_pretty(&info).map_err(io::Error::other)?)?;
    if name.ends_with(".md")
    {
        set_pinned(config, name, false)?;
    }
    else
    {
        let mut values = read_pinned(&root)?;
        values.retain(|item| item != name);
        fs::write(root.join("pinned.txt"), values.join("\n"))?;
    }
    let mut entries = fs::read_dir(&backup)?.filter_map(Result::ok).filter(|entry| entry.path().is_dir() && entry.path().join("info.json").is_file()).collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.metadata().and_then(|m| m.modified()).ok());
    while entries.len() > 10
    {
        let entry = entries.remove(0);
        fs::remove_dir_all(entry.path())?;
    }
    Ok(())
}

/// 恢复 backup 中最近删除的一项，并保留原文件名、资源目录和置顶状态。
pub fn restore_latest(config: &Path) -> io::Result<Option<CollectItem>>
{
    let root = directory(config);
    let backup = root.join("backup");
    let mut entries = fs::read_dir(&backup).ok().into_iter().flatten().filter_map(Result::ok).filter(|entry| entry.path().is_dir() && entry.path().join("info.json").is_file()).collect::<Vec<_>>();
    entries.sort_by_key(|entry| entry.metadata().and_then(|m| m.modified()).ok());
    let Some(entry) = entries.pop()
    else
    {
        return Ok(None);
    };
    let package = entry.path();
    let info: DeletedCollectInfo = serde_json::from_slice(&fs::read(package.join("info.json"))?).map_err(io::Error::other)?;
    if items::is_image_path(Path::new(&info.original_name))
    {
        let target = items::image_file(config, &info.original_name)?; // 恢复根目录图片，冲突拒绝覆盖。
        if target.exists()
        {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, "恢复目标已存在"));
        }
        fs::rename(package.join(&info.original_name), target)?;
        if info.pinned
        {
            set_pinned(config, &info.original_name, true)?;
        }
        fs::remove_dir_all(package)?;
        return get_item(config, &info.original_name).map(Some);
    }
    if info.original_name.ends_with(".md")
    {
        let target = root.join(&info.original_name);
        if target.exists() || root.join(info.original_name.trim_end_matches(".md")).exists()
        {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, "恢复目标已存在"));
        }
        fs::rename(package.join(&info.original_name), &target)?;
        let resource_name = info.original_name.trim_end_matches(".md");
        if package.join(resource_name).is_dir()
        {
            fs::rename(package.join(resource_name), root.join(resource_name))?;
        }
        fs::remove_dir_all(package)?;
        if info.pinned
        {
            set_pinned(config, &info.original_name, true)?;
        }
        let content = fs::read_to_string(&target)?;
        let metadata = fs::metadata(&target)?;
        return Ok(Some(CollectItem { name: info.original_name.clone(), kind: CollectKind::Text, display_name: info.original_name.trim_end_matches(".md").into(), path: Some(target.to_string_lossy().into()), content, created_unix_ms: metadata.created().or_else(|_| metadata.modified())?.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64, pinned: info.pinned, available: true }));
    }
    let target = items::item_directory(config, &info.original_name)?;
    if target.exists()
    {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "恢复目标已存在"));
    }
    fs::rename(package.join(&info.original_name), &target)?;
    fs::remove_dir_all(package)?;
    if info.pinned
    {
        set_pinned(config, &info.original_name, true)?;
    }
    Ok(Some(items::get_item(config, &info.original_name)?))
}

fn unique_backup_package(root: &Path, stamp: &str) -> PathBuf
{
    let mut index = 1;
    loop
    {
        let suffix = if index == 1 { String::new() } else { format!("-{index}") };
        let path = root.join(format!("{stamp}{suffix}"));
        if !path.exists()
        {
            return path;
        }
        index += 1;
    }
}

fn now_ms() -> i64 { Local::now().timestamp_millis() }
fn sanitize_name(value: &str) -> io::Result<String>
{
    let value = value.trim();
    if value.is_empty() || value.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|'])
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "收集项文件名无效"));
    }
    Ok(if value.ends_with(".md") { value.to_string() } else { format!("{value}.md") })
}
pub(crate) fn safe_file(root: &Path, name: &str) -> io::Result<PathBuf>
{
    let name = sanitize_name(name)?;
    let path = root.join(&name);
    if path.parent() != Some(root)
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "收集项路径无效"));
    }
    Ok(path)
}
fn read_pinned(root: &Path) -> io::Result<Vec<String>>
{
    match fs::read_to_string(root.join("pinned.txt"))
    {
        Ok(value) => Ok(value.lines().map(str::trim).filter(|value| !value.is_empty()).map(ToOwned::to_owned).collect()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests
{
    /// 真实本地引用保留中文与空格，拒绝缺失项目和非图片文件。
    #[test]
    fn local_images_preserve_references()
    {
        use super::*;
        let config = std::env::temp_dir().join(format!("collect-images-{}", std::process::id())); // 独立测试目录。
        fs::create_dir_all(directory(&config).join("资源")).unwrap();
        fs::write(directory(&config).join("note.md"), "").unwrap();
        let image = directory(&config).join("资源/[deadman] a.png"); // 包含需 URL 编码的文件名。
        fs::write(&image, b"\x89PNG\r\n\x1a\n").unwrap();
        assert!(read_image(&config, "note.md", "资源/[deadman] a.png").unwrap().starts_with("data:image/png;base64,"));
        assert!(read_image(&config, "note.md", image.to_str().unwrap()).is_ok());
        assert!(read_image(&config, "note.md", tauri::Url::from_file_path(&image).unwrap().as_str()).is_ok());
        assert!(read_image(&config, "missing.md", image.to_str().unwrap()).is_err());
        assert!(read_image(&config, "../note.md", image.to_str().unwrap()).is_err());
        assert!(read_image(&config, "note.md", "note.md").is_err());
        fs::remove_dir_all(config).unwrap();
    }
    use super::*;
    /// 两条资源入口都拒绝非法归属，失败不得移动源文件或在 collect 外创建目录。
    #[test]
    fn resources_reject_invalid_item_paths()
    {
        let root = std::env::temp_dir().join(format!("collect-path-validation-{}", std::process::id())); // 本测试独占配置目录。
        let item = create(&root, "").unwrap(); // 合法资源归属。
        let source = root.join("source.png"); // 失败后必须保留的源文件。
        fs::write(&source, b"source").unwrap();
        for name in ["../outside.md", "..\\outside.md", "C:\\outside.md", "missing.md"]
        {
            assert!(import_resource(&root, name, &source, "move").is_err());
            assert!(write_resource_bytes(&root, name, "image.png", b"image").is_err());
            assert!(source.is_file());
        }
        fs::write(directory(&root).join("...md"), "").unwrap();
        assert!(write_resource_bytes(&root, "...md", "image.png", b"image").is_err());
        fs::write(directory(&root).join("backup.md"), "").unwrap();
        assert!(write_resource_bytes(&root, "backup.md", "image.png", b"image").is_err());
        assert!(!root.join("outside").exists());
        assert!(write_resource_bytes(&root, &item.name, "image.png:stream", b"image").is_err());
        let resource = write_resource_bytes(&root, &item.name, "image.png", b"image").unwrap(); // 合法入口仍然可用。
        assert_eq!(fs::read(directory(&root).join(resource)).unwrap(), b"image");
        fs::remove_dir_all(root).unwrap();
    }
    /// 收入方式保持源文件语义；目标放在构建目录，便于与系统临时目录跨卷验证。
    #[test]
    fn resource_import_preserves_copy_link_and_move_semantics()
    {
        let source_root = std::env::temp_dir().join(format!("collect-resource-source-{}", std::process::id())); // 只包含本测试创建的图片。
        let target_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("build").join(format!("collect-resource-target-{}", std::process::id())); // 当前构建卷中的独立目标。
        fs::create_dir_all(&source_root).unwrap();
        let source = source_root.join("图片.png"); // 带中文名称的原始资源。
        fs::write(&source, b"resource-content").unwrap();
        let item = create(&target_root, "").unwrap(); // 收入资源的收集项。
        let linked = import_resource(&target_root, &item.name, &source, "link").unwrap(); // 链接保留原文件。
        assert_eq!(Path::new(&linked), source.canonicalize().unwrap());
        assert!(source.is_file());
        let copied = import_resource(&target_root, &item.name, &source, "copy").unwrap(); // 复制生成首个资源名。
        assert_eq!(fs::read(directory(&target_root).join(&copied)).unwrap(), b"resource-content");
        assert!(source.is_file());
        assert!(import_resource(&target_root, &item.name, &source, "invalid").is_err());
        assert!(source.is_file());
        let moved = import_resource(&target_root, &item.name, &source, "move").unwrap(); // 同名冲突生成新名，不覆盖复制结果。
        assert_ne!(copied, moved);
        assert!(!source.exists());
        assert_eq!(fs::read(directory(&target_root).join(&moved)).unwrap(), b"resource-content");
        assert_eq!(fs::read(directory(&target_root).join(&copied)).unwrap(), b"resource-content");
        let local = directory(&target_root).join(&moved); // 已在目标资源目录中的图片。
        assert_eq!(import_resource(&target_root, &item.name, &local, "move").unwrap(), moved); // 移动命令幂等返回原引用。
        fs::remove_dir_all(source_root).unwrap();
        fs::remove_dir_all(target_root).unwrap();
    }
    /// 重启扫描只返回磁盘现有文件，不恢复上一会话的不可用条目。
    #[test]
    fn fresh_scan_uses_only_current_files()
    {
        let root = std::env::temp_dir().join(format!("desktop-tools-collect-scan-{}", std::process::id())); // 独立扫描目录。
        let first = create(&root, "content").unwrap(); // 初次扫描存在的文件。
        assert_eq!(list(&root).unwrap().len(), 1);
        fs::rename(directory(&root).join(first.name), directory(&root).join("renamed.md")).unwrap();
        let scanned = list(&root).unwrap(); // 新会话直接重新扫描磁盘。
        assert_eq!(scanned.len(), 1);
        assert_eq!(scanned[0].name, "renamed.md");
        assert!(scanned[0].available);
        fs::remove_file(directory(&root).join("renamed.md")).unwrap();
        assert!(list(&root).unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn creates_conflicting_names_and_limits_backups()
    {
        let root = std::env::temp_dir().join(format!("desktop-tools-collect-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let first = create(&root, "a").unwrap();
        let second = create(&root, "b").unwrap();
        assert_ne!(first.name, second.name);
        save(&root, &first.name, "updated").unwrap();
        set_pinned(&root, &first.name, true).unwrap();
        let resource = root.join("collect").join(first.name.trim_end_matches(".md"));
        fs::create_dir_all(&resource).unwrap();
        fs::write(resource.join("image.png"), b"image").unwrap();
        save(&root, &first.name, &format!("![图片]({}/image.png)", first.name.trim_end_matches(".md"))).unwrap();
        assert!(list(&root).unwrap()[0].pinned);
        let draft = format!("未保存的草稿\n![图片]({}/image.png)", first.name.trim_end_matches(".md")); // 与磁盘内容不同的编辑器草稿。
        let occupied = directory(&root).join("renamed.md.rename.tmp"); // 模拟无法独占临时目标的写入失败。
        fs::write(&occupied, "已有临时文件").unwrap();
        assert!(rename(&root, &first.name, "renamed.md", Some(&draft)).is_err());
        assert!(resource.join("image.png").is_file());
        assert_eq!(fs::read_to_string(directory(&root).join(&first.name)).unwrap(), format!("![图片]({}/image.png)", first.name.trim_end_matches(".md")));
        assert_eq!(fs::read_to_string(&occupied).unwrap(), "已有临时文件");
        fs::remove_file(occupied).unwrap();
        let renamed_draft = rename(&root, &first.name, "renamed.md", Some(&draft)).unwrap();
        assert_eq!(renamed_draft, "未保存的草稿\n![图片](<renamed/image.png>)");
        assert!(!root.join("collect").join(&first.name).exists());
        assert!(root.join("collect").join("renamed.md").exists());
        assert!(root.join("collect").join("renamed").is_dir());
        assert_eq!(fs::read_to_string(root.join("collect").join("renamed.md")).unwrap(), "![图片](<renamed/image.png>)");
        delete(&root, "renamed.md").unwrap();
        let restored = restore_latest(&root).unwrap().unwrap();
        assert_eq!(restored.name, "renamed.md");
        assert!(restored.pinned);
        assert!(root.join("collect").join("renamed.md").exists());
        let _ = fs::remove_dir_all(root);
    }
}
