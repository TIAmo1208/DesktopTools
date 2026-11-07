import assert from "node:assert/strict";
import test from "node:test";

import { clearPluginMenuItems, normalizePluginMenuItems, readPluginMenuItems, setPluginMenuItems, subscribePluginMenuItems } from "./pluginMenu.ts";

// 非法项丢弃、重复标识保留首项，点击回调必须存在。
test("normalizes plugin menu entries", () => {
    const warnings: string[] = [];
    const entries = normalizePluginMenuItems(
        [
            { id: "refresh", label: "刷新", onSelect: () => undefined, disabled: true },
            { id: "refresh", label: "重复", onSelect: () => undefined },
            { id: "", label: "无标识", onSelect: () => undefined },
            { id: "broken", label: "无回调" },
            { id: "copy", label: "复制", onSelect: () => undefined, separatorBefore: true },
        ],
        (message) => warnings.push(message),
    );
    assert.deepEqual(
        entries.map((entry) => entry.id),
        ["refresh", "copy"],
    );
    assert.equal(entries[0].disabled, true);
    assert.equal(entries[1].separatorBefore, true);
    assert.equal(warnings.length, 3);
});

// 注册替换当前菜单并通知宿主；清理后回到空列表。
test("stores menu items per instance", () => {
    const seen: number[] = [];
    const stop = subscribePluginMenuItems("instance-a", () => seen.push(readPluginMenuItems("instance-a").length));
    setPluginMenuItems("instance-a", [{ id: "refresh", label: "刷新", onSelect: () => undefined }]);
    assert.equal(readPluginMenuItems("instance-a").length, 1);
    setPluginMenuItems("instance-a", [
        { id: "refresh", label: "刷新", onSelect: () => undefined },
        { id: "copy", label: "复制", onSelect: () => undefined },
    ]);
    assert.equal(readPluginMenuItems("instance-a").length, 2);
    assert.deepEqual(seen, [1, 2]);
    clearPluginMenuItems("instance-a");
    assert.deepEqual(readPluginMenuItems("instance-a"), []);
    assert.deepEqual(seen, [1, 2, 0]);
    stop();
    setPluginMenuItems("instance-a", []);
    assert.deepEqual(seen, [1, 2, 0]);
    clearPluginMenuItems("instance-a");
});

// 实例之间互不影响。
test("keeps instances isolated", () => {
    setPluginMenuItems("instance-a", [{ id: "a", label: "A", onSelect: () => undefined }]);
    setPluginMenuItems("instance-b", []);
    assert.equal(readPluginMenuItems("instance-a").length, 1);
    assert.equal(readPluginMenuItems("instance-b").length, 0);
    clearPluginMenuItems("instance-a");
    clearPluginMenuItems("instance-b");
});
