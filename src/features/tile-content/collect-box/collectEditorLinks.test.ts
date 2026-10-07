import assert from "node:assert/strict";
import test from "node:test";
import { createRequire } from "node:module";
import { pastedLink } from "./collectEditorLinks.ts";
import { configureCollectImageMarkdown } from "./collectEditorImages.ts";

test("图片说明的转义方括号不生成属性内 HTML，正文转义标记保持原样", () => {
    createRequire(import.meta.url)("vditor/dist/js/lute/lute.min.js"); // 复用产品 Lute。
    const lute = globalThis.Lute.New(); // 独立配置实例。
    const body = String.raw`\[正文\]`; // 图片之外仍保留所见即所得标记。
    const before = lute.Md2VditorDOM(body);
    configureCollectImageMarkdown(lute);
    assert.equal(lute.Md2VditorDOM(body), before);
    const html = lute.Md2VditorDOM(String.raw`![\[deadman\] 116147787.png](<2026-09-29-17-52-28/[deadman] 116147787-4.png>)`);
    assert.ok(html.includes('alt="[deadman] 116147787.png"'));
    assert.ok(!html.includes("<span"));
    assert.ok(html.includes("%5Bdeadman%5D%20116147787-4.png"));
});

test("图片显示候选不污染 Vditor 保存的 Markdown 引用", () => {
    const require = createRequire(import.meta.url); // 使用与产品相同的 Lute 解析器。
    require("vditor/dist/js/lute/lute.min.js"); // Lute 包将 API 注册到全局。
    const lute = globalThis.Lute.New(); // 独立解析实例。
    const markdown = "![图片](<目录/[deadman] 116147787.png>)\n"; // 覆盖中文、空格和方括号路径。
    const dom = lute.Md2VditorDOM(markdown); // 实际编辑器 DOM。
    const displayed = dom.replace("<img ", '<img srcset="data:image/png;base64,AAAA" '); // 只改变显示候选。
    assert.equal(lute.VditorDOM2Md(displayed), lute.VditorDOM2Md(dom));
    assert.ok(!lute.VditorDOM2Md(displayed).includes("base64"));
});

test("识别普通 URL 并使用 URL 作为名称", () => {
    assert.deepEqual(pastedLink("https://example.com/a"), { label: "https://example.com/a", url: "https://example.com/a" });
});

test("保留 Markdown 链接名称", () => {
    assert.deepEqual(pastedLink("[文档](https://example.com/docs)"), { label: "文档", url: "https://example.com/docs" });
});

test("从带外围片段的 HTML 剪贴板内容提取链接", () => {
    assert.deepEqual(pastedLink('<meta charset="utf-8"><a href="https://example.com">文档</a>'), { label: "文档", url: "https://example.com" });
});

test("普通文本保持原样粘贴", () => {
    assert.equal(pastedLink("普通文本"), undefined);
});
