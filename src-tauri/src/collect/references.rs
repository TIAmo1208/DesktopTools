use pulldown_cmark::{Event, LinkType, Parser, Tag};

/// 在单个原始 HTML 节点内只定位 `src`/`href` 属性值。
fn html_resource_edits(source: &str, old: &str, new: &str) -> Vec<(usize, usize, String)>
{
    let bytes = source.as_bytes(); // HTML 属性扫描只依赖 ASCII 标点。
    let mut edits = Vec::new(); // 相对 HTML 节点起点的替换范围。
    let mut index = 0;
    while index < bytes.len()
    {
        if !bytes[index].is_ascii()
        {
            index += source[index..].chars().next().map_or(1, char::len_utf8);
            continue;
        }
        let attribute = ["src", "href"].iter().find(|attribute| source.get(index..index + attribute.len()).is_some_and(|value| value.eq_ignore_ascii_case(attribute)) && (index == 0 || !bytes[index - 1].is_ascii_alphanumeric()) && (index + attribute.len() == bytes.len() || !bytes[index + attribute.len()].is_ascii_alphanumeric()));
        let Some(attribute) = attribute
        else
        {
            index += 1;
            continue;
        };
        let mut cursor = index + attribute.len(); // 属性名后的位置。
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace)
        {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'=')
        {
            index += attribute.len();
            continue;
        }
        cursor += 1;
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace)
        {
            cursor += 1;
        }
        let Some(&quote) = bytes.get(cursor)
        else
        {
            break;
        };
        if quote != b'\'' && quote != b'"'
        {
            index = cursor + 1;
            continue;
        }
        cursor += 1;
        let start = cursor; // 属性值起点。
        let Some(end_offset) = bytes[cursor..].iter().position(|byte| *byte == quote)
        else
        {
            break;
        };
        let end = cursor + end_offset;
        let value = &source[start..end];
        let explicit = value.starts_with("./"); // 保留显式相对前缀。
        let relative = value.strip_prefix("./").unwrap_or(value);
        if let Some(suffix) = relative.strip_prefix(&format!("{old}/"))
        {
            edits.push((start, end, format!("{}{new}/{suffix}", if explicit { "./" } else { "" })));
        }
        index = end + 1;
    }
    edits
}

/// 只改写 Markdown 解析器确认的链接目标，保持正文、代码和其他格式不变。
pub(super) fn rename_resources(content: &str, old: &str, new: &str) -> String
{
    let parser = Parser::new(content); // 源码位置用于局部替换。
    let mut targets = Vec::new(); // 目标定义或内联链接的范围与解析后地址。
    let mut edits = Vec::new(); // 从后向前应用，源码位置不随替换失效。
    for (_, definition) in parser.reference_definitions().iter()
    {
        targets.push((definition.span.clone(), definition.dest.to_string(), true));
    }
    for (event, span) in parser.into_offset_iter()
    {
        if let Event::Html(html) | Event::InlineHtml(html) = &event
        {
            for (start, end, replacement) in html_resource_edits(html, old, new)
            {
                edits.push((span.start + start..span.start + end, replacement));
            }
        }
        if let Event::Start(Tag::Image { link_type: LinkType::Inline, dest_url, .. } | Tag::Link { link_type: LinkType::Inline, dest_url, .. }) = event
        {
            targets.push((span, dest_url.to_string(), false));
        }
    }
    for (span, destination, definition) in targets
    {
        let relative = destination.strip_prefix("./").unwrap_or(&destination); // 保留显式相对路径语义。
        let Some(suffix) = relative.strip_prefix(&format!("{old}/"))
        else
        {
            continue;
        }; // 不修改外链和同名片段。
        if !source_is_markdown_span(content, &span)
        {
            edits.push((span, destination));
            continue;
        }
        let source = &content[span.clone()]; // 解析器确认的单个链接源码。
        let bytes = source.as_bytes(); // Markdown 标点均为 ASCII。
        let Some(mut cursor) = source.find('[')
        else
        {
            continue;
        };
        let mut depth = 0; // 匹配完整标签，跳过转义和行内代码。
        while cursor < bytes.len()
        {
            match bytes[cursor]
            {
                b'\\' =>
                {
                    cursor += 2;
                    continue;
                }
                b'`' =>
                {
                    let count = bytes[cursor..].iter().take_while(|value| **value == b'`').count(); // 当前代码界定符长度。
                    let marker = "`".repeat(count); // 只跳过实际闭合的代码片段。
                    if let Some(end) = source[cursor + count..].find(&marker)
                    {
                        cursor += count + end + count;
                        continue;
                    }
                }
                b'[' => depth += 1,
                b']' =>
                {
                    depth -= 1;
                    if depth == 0
                    {
                        cursor += 1;
                        break;
                    }
                }
                _ =>
                {}
            }
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&if definition { b':' } else { b'(' })
        {
            continue;
        }
        cursor += 1;
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace)
        {
            cursor += 1;
        }
        let start = cursor; // 包含尖括号时一并替换。
        let angular = bytes.get(cursor) == Some(&b'<'); // 带空格的合法链接地址。
        if angular
        {
            cursor += 1;
        }
        let mut parentheses = 0; // 普通地址允许平衡括号。
        while cursor < bytes.len()
        {
            let byte = bytes[cursor]; // 当前地址字节。
            if byte == b'\\'
            {
                cursor += 2;
                continue;
            }
            if angular && byte == b'>'
            {
                cursor += 1;
                break;
            }
            if !angular
            {
                if byte.is_ascii_whitespace() || (byte == b')' && parentheses == 0)
                {
                    break;
                }
                if byte == b'('
                {
                    parentheses += 1;
                }
                if byte == b')'
                {
                    parentheses -= 1;
                }
            }
            cursor += 1;
        }
        if cursor > bytes.len()
        {
            continue;
        }
        let prefix = if destination.starts_with("./") { "./" } else { "" }; // 保持相对路径前缀。
        let renamed = format!("{prefix}{new}/{suffix}"); // 仅变更资源目录。
        let escaped = renamed.replace('&', "&amp;").replace('<', "%3C").replace('>', "%3E").replace('\\', "%5C").replace('\n', "%0A").replace('\r', "%0D"); // 尖括号目标中的特殊字符。
        edits.push((span.start + start..span.start + cursor, format!("<{escaped}>")));
    }
    edits.sort_by_key(|(span, _)| span.start);
    edits.dedup_by(|a, b| a.0 == b.0);
    let mut result = content.to_string(); // 没有链接变化时逐字保留原内容。
    for (span, replacement) in edits.into_iter().rev()
    {
        result.replace_range(span, &replacement);
    }
    result
}

/// HTML 属性已经携带精确替换范围；Markdown 链接需要解析括号边界。
fn source_is_markdown_span(content: &str, span: &std::ops::Range<usize>) -> bool { !content[span.clone()].trim_start().starts_with('<') }

#[cfg(test)]
mod tests
{
    use super::*;
    #[test]
    fn changes_only_link_destinations()
    {
        let source = "旧/a.png <img src=\"旧/html.png\" data-title=\"旧/plain\"> `![示例](旧/a.png)`\n\n```md\n![示例](旧/a.png)\n```\n\n![旧/a.png](旧/a.png \"旧/标题\") [下载](./旧/doc.pdf)\n![引用][id]\n\n[id]: <旧/a b.png> \"标题\"\n<a href='./旧/page.html'>页面</a>\n[外链](https://example.com/旧/a.png)\n"; // 混合正文、代码、Markdown 与原始 HTML 链接。
        let result = rename_resources(source, "旧", "新"); // 保留非地址内容。
        assert_eq!(result, source.replace("src=\"旧/html.png\"", "src=\"新/html.png\"").replace("](旧/a.png \"", "](<新/a.png> \"").replace("](./旧/doc.pdf)", "](<./新/doc.pdf>)").replace("[id]: <旧/a b.png>", "[id]: <新/a b.png>").replace("href='./旧/page.html'", "href='./新/page.html'"));
        assert_eq!(rename_resources("草稿追加", "旧", "新"), "草稿追加");
    }
}
