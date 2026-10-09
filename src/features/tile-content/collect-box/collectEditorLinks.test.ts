import assert from "node:assert/strict";
import test from "node:test";
import { createRequire } from "node:module";
import { pastedLink } from "./collectEditorLinks.ts";
import { configureCollectImageMarkdown, previewCollectImage } from "./collectEditorImages.ts";

test("放大复用 PNG 与 GIF 显示地址及原节点几何，不改写正文引用", () => {
    for (const currentSrc of ["data:image/png;base64,AAAA", "data:image/gif;base64,AAAA", 'https://example.com/a?x=1&y="2"']) {
        const image = {
            complete: true,
            naturalWidth: 96,
            naturalHeight: 40,
            width: 48,
            height: 20,
            currentSrc,
            getAttribute(name: string) {
                assert.equal(this, image);
                return name === "src" ? "目录/image.gif" : null;
            },
            getBoundingClientRect() {
                assert.equal(this, image);
                return { left: 12, top: 24 };
            },
        } as unknown as HTMLImageElement;
        let calls = 0;
        previewCollectImage(
            image,
            (displayed, lang, theme) => {
                calls++;
                assert.equal(displayed.getAttribute("src"), currentSrc.replace(/&/g, "&amp;").replace(/"/g, "&quot;"));
                assert.equal(displayed.naturalWidth, 96);
                assert.equal(displayed.width, 48);
                assert.deepEqual(displayed.getBoundingClientRect(), { left: 12, top: 24 });
                assert.equal(lang, "zh_CN");
                assert.equal(theme, "dark");
            },
            "dark",
        );
        assert.equal(calls, 1);
        assert.equal(image.getAttribute("src"), "目录/image.gif");
        for (const state of [{ complete: false }, { naturalWidth: 0 }, { currentSrc: "" }]) {
            previewCollectImage({ ...image, ...state }, () => assert.fail("未加载成功不得打开预览"), "classic");
        }
    }
});

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
