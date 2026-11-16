import assert from "node:assert/strict";
import test from "node:test";
import { islandCollectItems, islandCollectLimit, islandCollectSummary, islandCollectTitle } from "./islandCollect.ts";
import type { CollectItem } from "../../bridge/collect.ts";

const item = (name: string, createdUnixMs: number, options: Partial<CollectItem> = {}): CollectItem => ({
    name,
    kind: "text",
    displayName: name,
    path: null,
    content: "",
    createdUnixMs,
    pinned: false,
    available: true,
    ...options,
});

test("浮岛收集箱沿用置顶与时间排序，并按三态限制条数", () => {
    const items = [item("a.md", 3, { content: "# 标题\n正文" }), item("b.md", 1), item("c.md", 2, { pinned: true })];
    assert.deepEqual(islandCollectItems(items, islandCollectLimit("collapsed")).map((entry) => entry.name), ["c.md"]);
    assert.deepEqual(islandCollectItems(items, islandCollectLimit("hover")).map((entry) => entry.name), ["c.md", "a.md"]);
    assert.deepEqual(islandCollectItems(items, 2).map((entry) => entry.name), ["c.md", "a.md"]);
    assert.equal(islandCollectLimit("expanded"), 50);
});

test("浮岛收集箱标题与摘要复用桌面文本规则", () => {
    const text = item("note.md", 1, { content: "# 真实标题\n正文内容" });
    assert.equal(islandCollectTitle(text), "真实标题");
    assert.equal(islandCollectSummary(text), "正文内容");
    const file = item("file-1.json", 1, { kind: "file", displayName: "配置.json" });
    assert.equal(islandCollectTitle(file), "file-1.json");
    assert.equal(islandCollectSummary(file), "配置.json");
});
