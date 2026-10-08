import assert from "node:assert/strict";
import test from "node:test";
import { createRequire } from "node:module";
import { collectExcerpt } from "./collectMarkdown.ts";

test("摘要保留 Markdown 实际文字与图片占位", () => {
    const summary = collectExcerpt(
        "# 标题\n正文 **强调** [链接](https://example.com)\n\n```ts\nconst value = 1;\n```\n\n![说明](image.png)\n\n|字段|值|\n|-|-|\n|名称|测试|",
    );
    assert.match(summary, /正文 强调 链接/);
    assert.match(summary, /const value = 1;/);
    assert.match(summary, /\[图片：说明\]/);
    assert.match(summary, /字段 值\n名称 测试/);
    assert.doesNotMatch(summary, /\*\*|\[链接\]\(/);
});

test("摘要保留原文换行", () => {
    assert.equal(collectExcerpt("第一行\n第二行"), "第一行\n第二行");
});

test("摘要压缩段落空行，保留列表与代码行且不修改 Markdown 原文", () => {
    createRequire(import.meta.url)("vditor/dist/js/lute/lute.min.js");
    const lute = globalThis.Lute.New(); // 与编辑器保存正文使用同一解析器。
    const markdown = lute.VditorDOM2Md('<p data-block="0">第一行</p><p data-block="0">第二行</p>'); // 模拟编辑器中的两个段落。
    assert.equal(collectExcerpt(markdown), "第一行\n第二行");
    assert.equal(collectExcerpt("正文 **强调** [链接](https://example.com)\n\n下一段"), "正文 强调 链接\n下一段");
    assert.equal(collectExcerpt("- 第一项\n- 第二项\n\n后文"), "第一项\n第二项\n后文");
    assert.equal(collectExcerpt("1. 第一项\n2. 第二项"), "第一项\n第二项");
    assert.equal(collectExcerpt("前文\n\n```text\n代码一\n代码二\n```\n\n后文"), "前文\n代码一\n代码二\n后文");
    assert.equal(collectExcerpt("第一行\r\n\r\n第二行"), "第一行\n第二行");
    assert.equal(lute.VditorDOM2Md('<p data-block="0">第一行</p><p data-block="0">第二行</p>'), markdown);
});
