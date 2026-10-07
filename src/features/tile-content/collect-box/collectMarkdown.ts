import { parser, RuleType, type MarkdownToJSX } from "markdown-to-jsx";

/** 从 Markdown 语法树提取摘要文字，保留内容而不是删除格式标记。 */
export const collectExcerpt = (content: string) => {
    // 块级节点保留分隔，行内节点连续拼接，避免格式标记导致多余换行。
    const plain = (nodes: MarkdownToJSX.ASTNode[]): string =>
        nodes
            .map((node) => {
                switch (node.type) {
                    case RuleType.heading:
                        return "";
                    case RuleType.text:
                    case RuleType.codeInline:
                    case RuleType.frontmatter:
                        return node.text;
                    case RuleType.codeBlock:
                        return `${node.text}\n`;
                    case RuleType.image:
                        return `[图片${node.alt ? `：${node.alt}` : ""}]`;
                    case RuleType.link:
                    case RuleType.textFormatted:
                        return plain(node.children);
                    case RuleType.blockQuote:
                    case RuleType.paragraph:
                        return `${plain(node.children).trimEnd()}\n`;
                    case RuleType.orderedList:
                    case RuleType.unorderedList:
                        return `${node.items.map((item) => plain(item).trimEnd()).join("\n")}\n`;
                    case RuleType.table:
                        return `${[node.header, ...node.cells].map((row) => row.map((cell) => plain(cell).trimEnd()).join(" ")).join("\n")}\n`;
                    case RuleType.breakLine:
                        return "\n";
                    default:
                        return "";
                }
            })
            .join("");

    return plain(parser(content))
        .replace(/\r\n?/g, "\n")
        .replace(/[ \t]+/g, " ")
        .replace(/\n{3,}/g, "\n")
        .trim()
        .slice(0, 240);
};
