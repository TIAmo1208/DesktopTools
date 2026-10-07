/** 将剪贴板中的 URL、Markdown 链接或 HTML 链接转换为编辑器链接数据。 */
export const pastedLink = (value: string) => {
    const text = value.trim();
    const markdown = text.match(/^\[([^\]]*)\]\((https?:\/\/[^\s)]+)\)$/i);
    if (markdown) return { label: markdown[1] || markdown[2], url: markdown[2] };
    const html = text.match(/<a\b[^>]*\bhref=["'](https?:\/\/[^"']+)["'][^>]*>([\s\S]*?)<\/a>/i);
    if (html) return { label: html[2].replace(/<[^>]*>/g, "").trim() || html[1], url: html[1] };
    if (/^https?:\/\/\S+$/i.test(text)) return { label: text, url: text };
    return undefined;
};
