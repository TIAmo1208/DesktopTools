import assert from "node:assert/strict";
import test from "node:test";
import { pastedLink } from "./collectEditorLinks.ts";

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
