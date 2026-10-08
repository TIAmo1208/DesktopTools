use super::{CollectItem, CollectKind, directory, get_item};
use serde::Serialize;
use std::fs;
use std::path::Path;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CollectFailure
{
    pub name: String,
    pub error: String,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReceiveResult
{
    pub processed: usize,
    pub skipped: usize,
    pub failures: Vec<CollectFailure>,
    pub had_move: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreparedExport
{
    pub names: Vec<String>,
    pub paths: Vec<String>,
    pub text: String,
    pub result: ReceiveResult,
}

pub(crate) fn prepare_export(config: &Path, names: &[String]) -> Result<PreparedExport, String>
{
    let mut export = PreparedExport { names: Vec::new(), paths: Vec::new(), text: String::new(), result: ReceiveResult::default() };
    for name in names
    {
        let item = match get_item(config, name)
        {
            Ok(item) if item.available => item,
            Ok(_) =>
            {
                export.result.skipped += 1;
                continue;
            }
            Err(error) =>
            {
                export.result.failures.push(CollectFailure { name: name.clone(), error: error.to_string() });
                continue;
            }
        };
        export.names.push(item.name.clone());
        export.result.processed += 1;
        match item.kind
        {
            CollectKind::Text =>
            {
                let root = directory(config);
                let path = root.join(&item.name);
                export.paths.push(path.to_string_lossy().into_owned());
                let resource = root.join(item.name.trim_end_matches(".md"));
                if resource.is_dir()
                {
                    export.paths.push(resource.to_string_lossy().into_owned());
                }
                if !export.text.is_empty()
                {
                    export.text.push_str("\n\n");
                }
                export.text.push_str(&plain_text(&item.content));
            }
            CollectKind::Image | CollectKind::File =>
            {
                if let Some(path) = item.path
                {
                    export.paths.push(path);
                }
            }
        }
    }
    if export.names.is_empty() && export.result.failures.is_empty()
    {
        return Err("没有可拖出的收集项".into());
    }
    Ok(export)
}

pub(crate) fn receive_items(config: &Path, target_name: Option<&str>, names: &[String], editing: bool) -> Result<ReceiveResult, String>
{
    let mut result = ReceiveResult::default();
    let target_kind = target_name.map(|name| get_item(config, name).map(|item| item.kind).map_err(|error| error.to_string())).transpose()?; // 目标类型仅判断一次。
    for name in names
    {
        let item = match get_item(config, name)
        {
            Ok(item) if item.available => item,
            Ok(_) =>
            {
                result.skipped += 1;
                continue;
            }
            Err(error) =>
            {
                result.failures.push(CollectFailure { name: name.clone(), error: error.to_string() });
                continue;
            }
        };
        if target_kind.is_some_and(|kind| kind != CollectKind::Text || item.kind == CollectKind::File)
        {
            result.skipped += 1;
            continue;
        }
        let outcome = match target_name
        {
            Some(_) if editing => Err("收集项正在编辑，请先关闭编辑窗".into()),
            Some(target) if item.kind == CollectKind::Text => copy_text(config, &item, target).and_then(|content| super::drop::append_text(config, target, &content)),
            Some(target) => fs::read(item.path.as_deref().unwrap_or_default()).map_err(|error| error.to_string()).and_then(|bytes| super::drop::append_image(config, target, &item.display_name, &bytes)),
            None => copy_item(config, &item),
        };
        match outcome
        {
            Ok(()) => result.processed += 1,
            Err(error) => result.failures.push(CollectFailure { name: name.clone(), error }),
        }
    }
    Ok(result)
}

fn copy_item(config: &Path, item: &CollectItem) -> Result<(), String>
{
    match item.kind
    {
        CollectKind::Text =>
        {
            let created = super::create(config, "").map_err(|error| error.to_string())?;
            let content = copy_text(config, item, &created.name)?;
            super::save(config, &created.name, &content).map_err(|error| error.to_string())?;
        }
        CollectKind::Image | CollectKind::File =>
        {
            let path = item.path.as_deref().ok_or("收集项路径不可用")?; // 图片复制字节，映射只复制记录。
            if item.kind == CollectKind::Image
            {
                let bytes = fs::read(path).map_err(|error| error.to_string())?;
                super::items::create_image(config, &item.display_name, &bytes).map_err(|error| error.to_string())?;
            }
            else
            {
                super::items::create_mapping(config, &item.display_name, path).map_err(|error| error.to_string())?;
            }
        }
    }
    Ok(())
}

/// 复制自有资源到独占子目录，复用 Markdown 链接改写且不覆盖同名文件。
fn copy_text(config: &Path, item: &CollectItem, target: &str) -> Result<String, String>
{
    let source = super::resource_directory(config, &item.name).map_err(|error| error.to_string())?; // 复用资源归属与重定向校验。
    if !source.exists()
    {
        return Ok(item.content.clone());
    }
    let target_root = super::resource_directory(config, target).map_err(|error| error.to_string())?; // 不写到源条目的资源目录。
    fs::create_dir_all(&target_root).map_err(|error| error.to_string())?;
    let copied = target_root.join(format!("copy-{}", crate::tools::time::next_timeline_timestamp())); // 每次复制独占目的路径。
    let mut pending = vec![(source.clone(), copied.clone())]; // 先收集源树，避免同项追加遍历新建目录。
    let mut files = Vec::new(); // 拒绝符号链接，不跟随自有目录外的资源。
    let mut directories = Vec::new(); // 收集后再创建目的目录。
    while let Some((from, to)) = pending.pop()
    {
        directories.push(to.clone());
        for entry in fs::read_dir(&from).map_err(|error| error.to_string())?
        {
            let entry = entry.map_err(|error| error.to_string())?;
            let kind = entry.file_type().map_err(|error| error.to_string())?;
            if kind.is_symlink() || entry.path().canonicalize().map_err(|error| error.to_string())? != entry.path()
            {
                return Err("资源目录不能包含重定向".into());
            }
            if kind.is_dir()
            {
                pending.push((entry.path(), to.join(entry.file_name())));
            }
            else if kind.is_file()
            {
                files.push((entry.path(), to.join(entry.file_name())));
            }
        }
    }
    for path in directories
    {
        fs::create_dir(path).map_err(|error| error.to_string())?;
    }
    for (from, to) in files
    {
        fs::copy(from, to).map_err(|error| error.to_string())?;
    }
    let old = source.file_name().unwrap().to_string_lossy(); // 只替换源自有目录的链接。
    let new = copied.strip_prefix(directory(config).canonicalize().map_err(|error| error.to_string())?).map_err(|error| error.to_string())?.to_string_lossy().replace('\\', "/");
    Ok(super::references::rename_resources(&item.content, &old, &new))
}

fn plain_text(markdown: &str) -> String
{
    use pulldown_cmark::{Event, Parser, Tag, TagEnd};
    let mut text = String::new(); // 完整正文，无列表摘要长度截断。
    let mut links = Vec::new(); // 保留链接地址与嵌套结束次序。
    for event in Parser::new(markdown)
    {
        match event
        {
            Event::Text(value) | Event::Code(value) => text.push_str(&value),
            Event::SoftBreak | Event::HardBreak => text.push('\n'),
            Event::Start(Tag::Item) => text.push_str("- "),
            Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) => links.push(dest_url.into_string()),
            Event::End(TagEnd::Link | TagEnd::Image) =>
            {
                if let Some(url) = links.pop()
                {
                    text.push_str(&format!(" ({url})"));
                }
            }
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::CodeBlock | TagEnd::List(_)) => text.push_str("\n\n"),
            Event::End(TagEnd::Item) =>
            {
                if !text.ends_with('\n')
                {
                    text.push('\n');
                }
            }
            Event::Rule => text.push_str("\n\n"),
            _ =>
            {}
        }
    }
    text.trim_end().to_string()
}

#[cfg(test)]
mod tests
{
    use super::*;

    #[test]
    fn copies_keep_sources_resources_and_partial_outcomes()
    {
        let root = std::env::temp_dir().join(format!("collect-transfer-{}", crate::tools::time::next_timeline_timestamp())); // 独立磁盘事实，验证实际资源复制。
        let text = super::super::create(&root, "# 标题\n\n**正文** [网站](https://example.com)").unwrap();
        let reference = super::super::write_resource_bytes(&root, &text.name, "photo.png", b"image").unwrap();
        super::super::save(&root, &text.name, &format!("{}\n\n![图片](<{reference}>)", text.content)).unwrap();
        let target = super::super::create(&root, "目标\n").unwrap();
        let image = super::super::items::create_image(&root, "photo.png", b"image").unwrap();
        let file_path = root.join("file.txt"); // 映射源文件在删除记录后仍存在。
        fs::write(&file_path, b"file").unwrap();
        let file = super::super::items::create_mapping(&root, "file.txt", file_path.to_str().unwrap()).unwrap();
        let names = vec![text.name.clone(), image.name.clone(), file.name.clone()]; // 文本、图片匹配，文件映射跳过。
        let blocked = receive_items(&root, Some(&target.name), &names, true).unwrap();
        assert_eq!((blocked.processed, blocked.skipped, blocked.failures.len()), (0, 1, 2));
        let result = receive_items(&root, Some(&target.name), &names, false).unwrap();
        assert_eq!((result.processed, result.skipped, result.failures.len()), (2, 1, 0));
        let content = get_item(&root, &target.name).unwrap().content;
        assert!(content.starts_with("目标\n\n# 标题"));
        assert!(!content.contains(&reference));
        assert!(content.contains("/copy-"));
        let copied = receive_items(&root, None, &names, false).unwrap();
        assert_eq!((copied.processed, copied.failures.len()), (3, 0));
        assert!(names.iter().all(|name| get_item(&root, name).unwrap().available));
        let export = prepare_export(&root, &names).unwrap();
        assert!(export.text.contains("标题\n\n正文 网站 (https://example.com)"));
        assert!(!export.text.contains("**"));
        assert_eq!(export.paths.len(), 4);
        super::super::delete(&root, &file.name).unwrap();
        assert!(file_path.is_file());
        assert_eq!(super::super::restore_latest(&root).unwrap().unwrap().name, file.name);
        fs::remove_dir_all(root).unwrap();
    }
}
