use super::*;

/// 原生回调已取得所有权的内容，一次拖放仅选择一种表示。
pub(crate) enum IncomingContent
{
    Text(String),       // 普通文本保持原样。
    Link(String),       // 明确 HTTP(S) URL。
    Files(Vec<String>), // 完整的本地图片集合。
    Image
    {
        bytes: Vec<u8>,
        dib: bool,
    }, // 有界原始图像，后台转换后收入。
}

/// 串行收入并读取磁盘最新正文，避免多个格子同时追加互相覆盖。
pub(crate) fn receive(config: &Path, target: Option<&str>, content: IncomingContent, mode: &str) -> Result<(), String>
{
    static RECEIVING: std::sync::Mutex<()> = std::sync::Mutex::new(()); // ponytail: 拖入低频，跨工作区吞吐有需求再拆分锁。
    let _guard = RECEIVING.lock().map_err(|_| "收集收入状态不可用")?; // 同次资源与正文保持顺序。
    let content = match content
    {
        IncomingContent::Text(text) if !text.trim().contains(char::is_whitespace) && tauri::Url::parse(text.trim()).is_ok_and(|url| matches!(url.scheme(), "http" | "https")) => IncomingContent::Link(text.trim().to_string()),
        content => content,
    }; // 单独 URL 是明确链接，含 URL 的普通正文不改写。
    let content = match content
    {
        IncomingContent::Image { bytes, dib } => IncomingContent::Image { bytes: crate::platform::windows::drop_content::image_png(&bytes, dib)?, dib: false },
        IncomingContent::Link(url) =>
        {
            let parsed = tauri::Url::parse(url.trim()).map_err(|_| "拖入链接无效")?; // 仅保存受支持的 URL，不访问网络。
            if !matches!(parsed.scheme(), "http" | "https")
            {
                return Err("只接收 HTTP(S) 链接".into());
            }
            IncomingContent::Text(format!("[{}](<{}>)", escape_label(url.trim()), parsed.as_str().replace('<', "%3C").replace('>', "%3E")))
        }
        IncomingContent::Files(paths) =>
        {
            if !crate::platform::windows::drop_content::file_targets(&paths).contains(&crate::platform::windows::drop_content::COLLECT)
            {
                return Err("收集箱只接收完整的图片文件集合".into());
            }
            IncomingContent::Files(paths)
        }
        content => content,
    }; // 文件与图像先验证，再创建条目。
    let name = if let Some(name) = target
    {
        let path = safe_file(&directory(config), name).map_err(|error| error.to_string())?; // 已消失或非法目标不得变成新建。
        fs::read_to_string(path).map_err(|error| format!("目标收集项不可用：{error}"))?;
        name.to_string()
    }
    else
    {
        create(config, "").map_err(|error| error.to_string())?.name
    }; // 仅格子空白位置新建。
    let append = |text: &str| -> Result<(), String> {
        let path = safe_file(&directory(config), &name).map_err(|error| error.to_string())?; // 每次提交前读取最新正文。
        let mut current = fs::read_to_string(path).map_err(|error| error.to_string())?; // 不使用前端过期条目快照。
        if !current.is_empty() && !current.ends_with('\n')
        {
            current.push_str("\n\n");
        }
        current.push_str(text);
        save(config, &name, &current).map_err(|error| error.to_string())
    }; // 原子正文提交失败保留之前的内容。
    match content
    {
        IncomingContent::Text(text) => append(&text),
        IncomingContent::Image { bytes, .. } =>
        {
            let resource = write_resource_bytes(config, &name, "拖入图片.png", &bytes).map_err(|error| error.to_string())?; // 无原始路径统一保存资源。
            append(&format!("![图片](<{}>)", escape_reference(&resource))).map_err(|error| format!("正文保存失败，图片已保留在 {resource}：{error}"))
        }
        IncomingContent::Files(paths) =>
        {
            for (index, source) in paths.iter().enumerate()
            {
                let resource = import_resource(config, &name, Path::new(source), mode).map_err(|error| format!("已收入 {index} 项，图片收入失败，来源={source}：{error}"))?; // 严格执行用户选择的模式。
                let label = Path::new(source).file_name().unwrap_or_default().to_string_lossy(); // 只作为 Markdown 文本。
                append(&format!("![{}](<{}>)", escape_label(&label), escape_reference(&resource))).map_err(|error| format!("已收入 {index} 项，正文保存失败，图片保留在 {resource}：{error}"))?; // 已移动资源保留可恢复路径，不再重复移动。
            }
            Ok(())
        }
        IncomingContent::Link(_) => unreachable!("链接已经转换为 Markdown"),
    }
}

/// Markdown 文本转义，文件名与 URL 不能注入新的语法节点。
fn escape_label(text: &str) -> String { text.replace('\\', "\\\\").replace('[', "\\[").replace(']', "\\]").replace('<', "\\<").replace('>', "\\>").replace(['\r', '\n'], " ") }

#[cfg(test)]
mod tests
{
    use super::*;

    /// 实际磁盘验证新建、追加、链接、三种图片模式与失效目标不改写正文。
    #[test]
    fn collect_drop_preserves_content_and_resource_modes()
    {
        assert_eq!(escape_reference(r"\\?\UNC\server\share\图片.png"), "//server/share/图片.png");
        assert_eq!(escape_reference(r"\\?\C:\图片.png"), "C:/图片.png");
        assert_eq!(escape_label("<b>[图片]</b>"), r"\<b\>\[图片\]\</b\>");
        let root = std::env::temp_dir().join(format!("desktop-tools-collect-drop-{}", crate::tools::time::next_timeline_timestamp())); // 独占验证目录。
        fs::create_dir_all(&root).unwrap();
        let item = create(&root, "原正文").unwrap(); // 已存在的目标条目。
        receive(&root, Some(&item.name), IncomingContent::Text("追加文字".into()), "copy").unwrap();
        receive(&root, Some(&item.name), IncomingContent::Link("https://example.com/a?q=b".into()), "copy").unwrap();
        let content = fs::read_to_string(directory(&root).join(&item.name)).unwrap(); // 查看实际 Markdown。
        assert!(content.starts_with("原正文\n\n追加文字"));
        assert_eq!(content.matches("[https://example.com/a?q=b](<https://example.com/a?q=b>)").count(), 1);
        let source = root.join("图片[1].png"); // 文件内容不解码，沿现有图片文件路径收入。
        fs::write(&source, b"source image").unwrap();
        for mode in ["copy", "link", "move"]
        {
            receive(&root, Some(&item.name), IncomingContent::Files(vec![source.to_string_lossy().into_owned()]), mode).unwrap();
            assert_eq!(source.exists(), mode != "move");
        }
        let saved = fs::read_to_string(directory(&root).join(&item.name)).unwrap(); // 已提交正文保持三次独立投放。
        assert_eq!(saved.matches("![图片\\[1\\].png]").count(), 3);
        assert!(receive(&root, Some(&item.name), IncomingContent::Link("javascript:alert(1)".into()), "copy").is_err());
        assert!(receive(&root, Some(&item.name), IncomingContent::Image { bytes: vec![0; 8], dib: false }, "copy").is_err());
        assert!(receive(&root, Some("../outside.md"), IncomingContent::Text("拒绝".into()), "copy").is_err());
        assert!(receive(&root, Some("missing.md"), IncomingContent::Text("拒绝".into()), "copy").is_err());
        assert_eq!(fs::read_to_string(directory(&root).join(&item.name)).unwrap(), saved);
        receive(&root, None, IncomingContent::Text("新条目".into()), "copy").unwrap();
        assert_eq!(list(&root).unwrap().len(), 2);
        std::fs::remove_dir_all(root).unwrap();
    }
}

/// 将本地链接写成安全的 Markdown 尖括号目标。
fn escape_reference(reference: &str) -> String
{
    let reference = reference.strip_prefix(r"\\?\UNC\").map_or_else(|| reference.strip_prefix(r"\\?\").unwrap_or(reference).to_string(), |unc| format!("//{unc}")); // 保留 UNC 共享身份，去掉 Win32 扩展路径前缀。
    reference.replace('\\', "/").replace('<', "%3C").replace('>', "%3E").replace('\r', "%0D").replace('\n', "%0A")
}
