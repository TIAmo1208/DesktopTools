import assert from "node:assert/strict";
import test from "node:test";
import { mergeCollectItems, orderCollectItems, filterCollectItems, readCollectBoxState } from "./collectItemState.ts";
import type { CollectItem } from "../../../bridge/collect.ts";

test("混排置顶优先，分离保持类型顺序且组内置顶优先", () => {
    const make = (name: string, kind: CollectItem["kind"], time: number, pinned = false): CollectItem => ({
        name,
        kind,
        displayName: name,
        path: null,
        content: "",
        createdUnixMs: time,
        pinned,
        available: true,
    });
    const items = [make("f", "file", 4, true), make("t", "text", 3), make("i", "image", 2), make("p", "text", 1, true)];
    assert.deepEqual(
        orderCollectItems(items, "mixed", "time").map((item) => item.name),
        ["f", "p", "t", "i"],
    );
    assert.deepEqual(
        orderCollectItems(items, "separated", "time").map((item) => item.name),
        ["p", "t", "i", "f"],
    );
    assert.deepEqual(filterCollectItems(items, { text: false, image: false, file: false }), []);
    assert.deepEqual(
        filterCollectItems(items, { text: false, image: true, file: false }).map((item) => item.name),
        ["i"],
    );
    assert.deepEqual(
        items.map((item) => item.name),
        ["f", "t", "i", "p"],
    );
    assert.deepEqual(readCollectBoxState(null), { layout: "mixed", filters: { text: true, image: true, file: true } });
    assert.deepEqual(readCollectBoxState({ layout: "separated", filters: { text: false, image: false, file: false } }).filters, {
        text: false,
        image: false,
        file: false,
    });
});

test("主动删除移除缓存，外部删除保留不可用项，恢复文件重新可用", () => {
    const item = { name: "测试.md", kind: "text" as const, displayName: "测试", path: null, content: "内容", createdUnixMs: 1, pinned: false, available: true };
    assert.deepEqual(mergeCollectItems([item], [], item.name), []);
    assert.deepEqual(mergeCollectItems([item], []), [{ ...item, available: false }]);
    assert.deepEqual(mergeCollectItems([{ ...item, available: false }], [item]), [item]);
    assert.deepEqual(mergeCollectItems([item], [item], item.name), []);
});
