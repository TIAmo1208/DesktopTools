use super::items::is_image_path;
use super::*;

/// 原生回调已取得所有权的内容，一次拖放仅选择一种表示。
pub(crate) enum IncomingContent
{
    Text(String),
    Link(String),
    Files(Vec<String>),
    Image
    {
        bytes: Vec<u8>,
        dib: bool,
    },
}

/// 串行收入并读取磁盘最新正文，逐项保留处理、跳过和失败结果。
pub(crate) fn receive(config: &Path, target: Option<&str>, content: IncomingContent, mode: &str, editing: bool) -> Result<super::ReceiveResult, String>
{
    static RECEIVING: std::sync::Mutex<()> = std::sync::Mutex::new(()); // ponytail: 拖入低频，跨工作区吞吐有需求再拆分锁。
    let _guard = RECEIVING.lock().map_err(|_| "收集收入状态不可用")?;
    let content = match content
    {
        IncomingContent::Text(text) if !text.trim().contains(char::is_whitespace) && tauri::Url::parse(text.trim()).is_ok_and(|url| matches!(url.scheme(), "http" | "https")) => IncomingContent::Link(text.trim().to_string()),
        content => content,
    }; // 自动识别的链接继续经过统一转换，不能落入不可达分支。
    let content = match content
    {
        IncomingContent::Image { bytes, dib } => IncomingContent::Image { bytes: crate::platform::windows::drop_content::image_png(&bytes, dib)?, dib: false },
        IncomingContent::Link(url) =>
        {
            let parsed = tauri::Url::parse(url.trim()).map_err(|_| "拖入链接无效")?;
            if !matches!(parsed.scheme(), "http" | "https")
            {
                return Err("只接收 HTTP(S) 链接".into());
            }
            IncomingContent::Text(format!("[{}](<{}>)", escape_label(url.trim()), parsed.as_str().replace('<', "%3C").replace('>', "%3E")))
        }
        IncomingContent::Files(paths) =>
        {
            if paths.is_empty() || paths.len() > 512
            {
                return Err("拖入文件数量无效".into());
            }
            IncomingContent::Files(paths)
        }
        content => content,
    };
    let mut result = super::ReceiveResult::default();
    let target_kind = target.map(|name| get_item(config, name).map(|item| item.kind).map_err(|error| error.to_string())).transpose()?; // 一次操作只判断一次实际目标类型。
    if target_kind.is_some_and(|kind| kind != CollectKind::Text)
    {
        result.skipped = match &content
        {
            IncomingContent::Files(paths) => paths.len(),
            _ => 1,
        };
        return Ok(result);
    }
    match content
    {
        IncomingContent::Text(text) =>
        {
            let outcome = if target.is_some() && editing
            {
                Err("收集项正在编辑，请先关闭编辑窗".into())
            }
            else if text.len() > 1024 * 1024
            {
                Err("拖入文本超过 1 MiB".into())
            }
            else if let Some(name) = target
            {
                append_text(config, name, &text)
            }
            else
            {
                super::create(config, &text).map(|_| ()).map_err(|error| error.to_string())
            };
            match outcome
            {
                Ok(()) => result.processed = 1,
                Err(error) => result.failures.push(CollectFailure { name: target.unwrap_or("拖入文本").into(), error }),
            }
        }
        IncomingContent::Image { bytes, .. } =>
        {
            if let Some(name) = target
            {
                if editing
                {
                    result.failures.push(CollectFailure { name: name.into(), error: "收集项正在编辑，请先关闭编辑窗".into() });
                    return Ok(result);
                }
                append_image(config, name, "拖入图片.png", &bytes)?;
            }
            else
            {
                items::create_image(config, "图片", &bytes).map_err(|error| error.to_string())?;
            }
            result.processed = 1;
        }
        IncomingContent::Files(paths) =>
        {
            for source in paths
            {
                let path = Path::new(&source);
                let display = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                if path.is_dir()
                {
                    result.skipped += 1;
                    continue;
                }
                if !path.is_file()
                {
                    result.failures.push(CollectFailure { name: source, error: "拖入文件不可用".into() });
                    continue;
                }
                if is_image_path(path)
                {
                    if let Some(target) = target
                    {
                        let outcome = if editing
                        {
                            Err("收集项正在编辑，请先关闭编辑窗".into())
                        }
                        else
                        {
                            import_resource(config, target, path, mode).map_err(|error| error.to_string()).and_then(|reference| {
                                if mode == "move"
                                {
                                    result.had_move = true;
                                }
                                append_text(config, target, &format!("![{}](<{}>)", escape_label(&display), escape_reference(&reference))).map_err(|error| format!("正文保存失败，图片保留在 {reference}：{error}"))
                            })
                        }; // 沿用图片原始格式及既有复制、移动、链接语义。
                        match outcome
                        {
                            Ok(()) => result.processed += 1,
                            Err(error) => result.failures.push(super::CollectFailure { name: source, error }),
                        }
                    }
                    else
                    {
                        use std::io::Read;
                        let mut bytes = Vec::new(); // 对外部文件实施有界读取。
                        if let Err(error) = fs::File::open(path).and_then(|file| file.take(32 * 1024 * 1024 + 1).read_to_end(&mut bytes))
                        {
                            result.failures.push(CollectFailure { name: source, error: error.to_string() });
                            continue;
                        }
                        if bytes.len() > 32 * 1024 * 1024
                        {
                            result.failures.push(CollectFailure { name: source, error: "图片超过 32 MiB".into() });
                            continue;
                        }
                        match items::create_image(config, &display, &bytes)
                        {
                            Ok(_) =>
                            {
                                result.processed += 1;
                            }
                            Err(error) => result.failures.push(super::CollectFailure { name: source, error: error.to_string() }),
                        }
                    }
                }
                else if target.is_some()
                {
                    result.skipped += 1;
                }
                else
                {
                    match items::create_mapping(config, &display, &source)
                    {
                        Ok(_) => result.processed += 1,
                        Err(error) => result.failures.push(super::CollectFailure { name: source, error: error.to_string() }),
                    }
                }
            }
        }
        IncomingContent::Link(_) => unreachable!("链接已经转换为 Markdown"),
    }
    Ok(result)
}

pub(super) fn append_text(config: &Path, name: &str, text: &str) -> Result<(), String>
{
    let path = safe_file(&directory(config), name).map_err(|error| error.to_string())?;
    let mut current = fs::read_to_string(path).map_err(|error| format!("目标收集项不可用：{error}"))?;
    if !current.is_empty()
    {
        if !current.ends_with('\n')
        {
            current.push('\n');
        }
        if !current.ends_with("\n\n")
        {
            current.push('\n');
        }
    }
    current.push_str(text);
    save(config, name, &current).map_err(|error| error.to_string())
}

pub(super) fn append_image(config: &Path, name: &str, display: &str, bytes: &[u8]) -> Result<(), String>
{
    let resource = write_resource_bytes(config, name, display, bytes).map_err(|error| error.to_string())?;
    append_text(config, name, &format!("![图片](<{}>)", escape_reference(&resource))).map_err(|error| format!("正文保存失败，图片已保留在 {resource}：{error}"))
}

fn escape_label(text: &str) -> String { text.replace('\\', "\\\\").replace('[', "\\[").replace(']', "\\]").replace('<', "\\<").replace('>', "\\>").replace(['\r', '\n'], " ") }
fn escape_reference(reference: &str) -> String
{
    let reference = reference.strip_prefix(r"\\?\UNC\").map_or_else(|| reference.strip_prefix(r"\\?\").unwrap_or(reference).to_string(), |unc| format!("//{unc}"));
    reference.replace('\\', "/").replace('<', "%3C").replace('>', "%3E").replace('\r', "%0D").replace('\n', "%0A")
}

#[cfg(test)]
mod tests
{
    use super::*;
    #[test]
    fn labels_and_references_are_escaped()
    {
        assert_eq!(escape_reference(r"\\?\UNC\server\share\图片.png"), "//server/share/图片.png");
        assert_eq!(escape_label("<b>[图片]</b>"), r"\<b\>\[图片\]\</b\>");
    }

    #[test]
    fn income_preserves_partial_results_and_editor_content()
    {
        let root = std::env::temp_dir().join(format!("collect-income-{}", crate::tools::time::next_timeline_timestamp())); // 本测试独占目录。
        fs::create_dir_all(&root).unwrap();
        let image = root.join("sample.jpg"); // 验证原格式与移动语义。
        let file = root.join("sample.txt"); // 普通文件只创建映射。
        fs::write(&image, b"image-bytes").unwrap();
        fs::write(&file, b"source").unwrap();
        let result = receive(&root, None, IncomingContent::Text("https://example.com/".into()), "copy", false).unwrap();
        assert_eq!(result.processed, 1);
        let text = list(&root).unwrap().pop().unwrap(); // 链接必须转换而不能 panic。
        assert_eq!(text.content, "[https://example.com/](<https://example.com/>)");
        let paths = vec![root.join("missing.png").to_string_lossy().into_owned(), image.to_string_lossy().into_owned(), file.to_string_lossy().into_owned(), root.to_string_lossy().into_owned()]; // 失效项不阻止后续同类型项。
        let result = receive(&root, None, IncomingContent::Files(paths), "move", false).unwrap();
        assert_eq!((result.processed, result.skipped, result.failures.len(), result.had_move), (2, 1, 1, false));
        assert!(image.exists()); // 独立收入即使设置为移动也保留源文件。
        assert!(file.exists());
        let image_item = list(&root).unwrap().into_iter().find(|item| item.kind == CollectKind::Image).unwrap();
        assert!(image_item.path.as_ref().unwrap().ends_with(".jpg"));
        assert_eq!(Path::new(image_item.path.as_ref().unwrap()).parent(), Some(directory(&root).as_path()));
        let result = receive(&root, Some(&text.name), IncomingContent::Text("不可覆盖".into()), "copy", true).unwrap();
        assert_eq!(result.failures.len(), 1);
        assert_eq!(get_item(&root, &text.name).unwrap().content, text.content);
        fs::write(&image, b"image-bytes").unwrap();
        let result = receive(&root, Some(&text.name), IncomingContent::Files(vec![image.to_string_lossy().into_owned(), file.to_string_lossy().into_owned()]), "copy", true).unwrap();
        assert_eq!((result.processed, result.skipped, result.failures.len()), (0, 1, 1));
        let result = receive(&root, Some(&text.name), IncomingContent::Files(vec![image.to_string_lossy().into_owned()]), "move", false).unwrap();
        assert_eq!((result.processed, result.had_move), (1, true));
        assert!(!image.exists());
        assert!(get_item(&root, &text.name).unwrap().content.contains("sample.jpg"));
        fs::remove_dir_all(root).unwrap();
    }
}
