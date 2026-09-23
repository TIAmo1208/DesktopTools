import assert from "node:assert/strict";
import test from "node:test";
import { mergeCollectItems } from "./collectItemState.ts";

test("主动删除移除缓存，外部删除保留不可用项，恢复文件重新可用", () => {
    const item = { name: "测试.md", content: "内容", createdUnixMs: 1, pinned: false, available: true };
    assert.deepEqual(mergeCollectItems([item], [], item.name), []);
    assert.deepEqual(mergeCollectItems([item], []), [{ ...item, available: false }]);
    assert.deepEqual(mergeCollectItems([{ ...item, available: false }], [item]), [item]);
    assert.deepEqual(mergeCollectItems([item], [item], item.name), []);
});
