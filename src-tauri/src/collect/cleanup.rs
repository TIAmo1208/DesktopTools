use super::{directory, items, resource_directory, safe_file};
use crate::platform::windows::file_proof::FileProof;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

static RECORDS: Mutex<()> = Mutex::new(()); // 登记和清理串行更新归属证据。
const MANIFEST: &str = ".managed-images.json"; // 随文档资源目录重命名，不收录历史未知文件。

#[cfg(test)]
#[path = "cleanup.test.rs"]
mod tests;

/// 读取应用实际创建的图片证据；损坏或重定向记录不能授权删除。
fn records(root: &Path) -> Result<BTreeMap<String, FileProof>, String>
{
    let path = root.join(MANIFEST); // 仅访问当前文档资源目录。
    if path.exists() && path.canonicalize().map_err(|e| e.to_string())? != path
    {
        return Err("附件归属记录不能重定向".into());
    }
    crate::persistence::load_json(&path).map(|value| value.unwrap_or_default()).map_err(|e| format!("读取附件归属失败，路径={}，原因={e}", path.display()))
}

/// 仅在新图片完整创建后登记；失败保留图片但不将其纳入自动回收。
pub(super) fn record(root: &Path, path: &Path)
{
    if !items::is_image_path(path)
    {
        return;
    }
    let result = (|| {
        let _guard = RECORDS.lock().map_err(|_| "附件归属锁不可用")?; // 串行更新原子文件。
        let mut entries = records(root)?; // 不覆盖损坏的旧记录。
        let relative = path.strip_prefix(root).map_err(|_| "附件不属于文档资源目录")?; // 复制形成的嵌套目录也登记真实归属。
        entries.insert(relative.to_string_lossy().into_owned(), FileProof::capture(path)?);
        crate::persistence::save_json_atomic(&root.join(MANIFEST), &entries).map_err(|e| e.to_string())
    })();
    if let Err(error) = result
    {
        crate::log_warn!("收集箱：附件未登记，保留文件，路径={}，原因={error}", path.display());
    }
}

/// 编辑窗开始前固定归属集合，放弃草稿时只考虑集合之外的新附件。
pub(crate) fn baseline(config: &Path, name: &str) -> Result<BTreeSet<String>, String>
{
    let root = resource_directory(config, name).map_err(|e| e.to_string())?; // 校验现存文档与资源目录。
    Ok(records(&root)?.into_keys().collect())
}

/// 保留 Markdown 内联、引用式链接和可可靠识别的 HTML 图片引用。
fn destinations(content: &str) -> Result<Vec<String>, String>
{
    use pulldown_cmark::{Event, Parser, Tag};
    let mut uncertain = false; // 未解析引用定义不能推断为已删除。
    let callback = |_: pulldown_cmark::BrokenLink<'_>| {
        uncertain = true;
        None
    };
    let parser = Parser::new_with_broken_link_callback(content, pulldown_cmark::Options::all(), Some(callback));
    let mut result = parser.reference_definitions().iter().map(|(_, value)| value.dest.to_string()).collect::<Vec<_>>(); // 未使用定义也保守保留。
    for event in parser
    {
        match event
        {
            Event::Start(Tag::Image { dest_url, .. } | Tag::Link { dest_url, .. }) => result.push(dest_url.to_string()),
            Event::Html(html) | Event::InlineHtml(html) => result.extend(super::references::html_destinations(&html)?),
            Event::Text(text) if text.contains("![") => return Err("图片标记未完整解析，保留附件".into()),
            _ => (),
        }
    }
    if uncertain
    {
        return Err("Markdown 存在未解析的引用，保留附件".into());
    }
    Ok(result)
}

/// 解析目录中所有文档，跨文档使用的附件同样受保护；任何读取不完整均停止回收。
fn references(config: &Path) -> Result<BTreeSet<PathBuf>, String>
{
    let root = directory(config).canonicalize().map_err(|e| e.to_string())?; // Markdown 相对地址的唯一基准。
    let base = tauri::Url::from_directory_path(&root).map_err(|_| "收集箱目录无法转为文件 URL")?;
    let mut referenced = BTreeSet::new(); // Windows 路径大小写统一后比较。
    for entry in fs::read_dir(&root).map_err(|e| e.to_string())?
    {
        let path = entry.map_err(|e| e.to_string())?.path(); // 不扫描其他文档资源子目录。
        if !path.extension().is_some_and(|value| value.eq_ignore_ascii_case("md"))
        {
            continue;
        }
        let content = fs::read_to_string(&path).map_err(|e| format!("正文读取失败，路径={}，原因={e}", path.display()))?;
        for destination in destinations(&content).map_err(|e| format!("引用解析不确定，路径={}，原因={e}", path.display()))?
        {
            // 原始路径和 URL 解码路径都保留，兼容字面百分号及编码引用。
            let raw = PathBuf::from(&destination);
            let raw = if raw.is_absolute() { raw } else { root.join(raw) };
            if let Ok(path) = raw.canonicalize()
            {
                referenced.insert(PathBuf::from(path.to_string_lossy().to_lowercase()));
            }
            let normalized = destination.replace('\\', "/"); // Windows 路径分隔符与编辑器输出统一。
            let url = if normalized.as_bytes().get(1) == Some(&b':')
            {
                tauri::Url::parse(&format!("file:///{normalized}"))
            }
            else
            {
                base.join(&normalized)
            }
            .map_err(|e| e.to_string())?;
            if matches!(url.scheme(), "http" | "https" | "mailto" | "data")
            {
                continue;
            }
            if url.scheme() != "file"
            {
                return Err(format!("引用协议无法确认：{}", url.scheme()));
            }
            let path = url.to_file_path().map_err(|_| "本地图片引用无法解析")?;
            // 失效引用不授权删除任何可能同名的附件。
            let path = path.canonicalize().map_err(|e| format!("引用路径无法确认，路径={}，原因={e}", path.display()))?;
            referenced.insert(PathBuf::from(path.to_string_lossy().to_lowercase()));
        }
    }
    Ok(referenced)
}

/// 关闭阶段回收明确归属且无引用的图片；recycle 必须使用系统回收站。
pub(crate) fn cleanup(config: &Path, name: &str, keep: Option<&BTreeSet<String>>, mut recycle: impl FnMut(&FileProof) -> Result<(), String>) -> Result<(), String>
{
    let _guard = RECORDS.lock().map_err(|_| "附件归属锁不可用")?; // 登记与清理不并发修改记录。
    let root = resource_directory(config, name).map_err(|e| e.to_string())?;
    let document = safe_file(&directory(config), name).map_err(|e| e.to_string())?; // 不接受前端传入正文或附件路径。
    let saved = fs::read_to_string(&document).map_err(|e| format!("正文读取失败，路径={}，原因={e}", document.display()))?;
    let mut entries = records(&root)?;
    if entries.is_empty()
    {
        return Ok(());
    }
    for (name, mut proof) in entries.clone()
    {
        if keep.is_some_and(|keep| keep.contains(&name)) || !items::is_image_path(Path::new(&name))
        {
            continue;
        }
        if Path::new(&name).components().any(|part| !matches!(part, std::path::Component::Normal(_))) || name.contains(':') || name.is_empty()
        {
            continue;
        }
        let path = root.join(&name); // 忽略记录中的绝对路径，仅接受归属目录内的普通相对路径。
        if !path.canonicalize().is_ok_and(|actual| actual == path)
        {
            continue;
        } // 嵌套目录也不能通过重解析点跳转。
        proof.path = path.clone(); // 应用内重命名移动目录，但文件身份和摘要不变。
        if !proof.unchanged()
        {
            crate::log_warn!("收集箱：保留身份或内容已变化的附件，路径={}", path.display());
            continue;
        }
        if fs::read_to_string(&document).map_err(|e| e.to_string())? != saved
        {
            return Err("正文在清理期间变化，停止回收".into());
        }
        if references(config)?.contains(&PathBuf::from(path.to_string_lossy().to_lowercase()))
        {
            continue;
        }
        if !proof.unchanged()
        {
            continue;
        }
        proof.path = PathBuf::from(crate::platform::windows::shell_parsing_path(&path.to_string_lossy())); // Shell 不接受 Win32 扩展前缀，Pre 回调使用同一种路径表示。
        match recycle(&proof)
        {
            Ok(()) =>
            {
                entries.remove(&name);
                crate::log_info!("收集箱：未引用图片已移入系统回收站，路径={}", path.display());
            }
            Err(error) => crate::log_warn!("收集箱：图片回收失败，正文保存不受影响，路径={}，原因={error}", path.display()),
        }
    }
    crate::persistence::save_json_atomic(&root.join(MANIFEST), &entries).map_err(|e| e.to_string())
}
