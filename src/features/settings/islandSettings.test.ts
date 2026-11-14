import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { parseComponentRuntimeSnapshot } from "../../contracts/component.ts";

import { islandCardAvailability, moveIslandCard, normalizeIslandCardOrder, sortIslandCardManifests, updateIslandCardOrder } from "./islandSettings.ts";

test("真实官方音乐清单进入岛设置列表，勾选才写入顺序", () => {
    const official = JSON.parse(readFileSync(new URL("../../../src-tauri/components/official.music-control.json", import.meta.url), "utf8")); // 与 Rust include_str 使用同一清单。
    const parse = (manifest: unknown) =>
        parseComponentRuntimeSnapshot({ revision: 1, manifests: [{ script: "", ...(manifest as object) }], instances: [], failures: [] }).manifests;
    assert.deepEqual(sortIslandCardManifests(parse({ ...official, island: false }), []), []); // 缺少声明会重现「暂无可用岛卡」。
    const cards = sortIslandCardManifests(parse(official), []);
    assert.deepEqual(
        cards.map((card) => card.id),
        ["official.music-control"],
    );
    assert.equal(cards[0].name, "音乐控制");
    assert.equal(cards[0].entry, "official:music-control");
    assert.equal(islandCardAvailability([], cards[0].id).enabled, false);
    assert.deepEqual(updateIslandCardOrder([], cards[0].id, true), ["official.music-control"]);
});

test("岛卡顺序规范化、去重并限制为 32 项", () => {
    const cards = ["bad", "official.clock", "official.clock", ...Array.from({ length: 40 }, (_, index) => `plugin.card-${index}`)];
    const result = normalizeIslandCardOrder(cards);
    assert.equal(result.length, 32);
    assert.deepEqual(result.slice(0, 2), ["official.clock", "plugin.card-0"]);
});

test("开关与移动都修改同一份顺序", () => {
    const enabled = updateIslandCardOrder(["official.a"], "plugin.b", true);
    assert.deepEqual(enabled, ["official.a", "plugin.b"]);
    assert.deepEqual(moveIslandCard(enabled, "plugin.b", -1), ["plugin.b", "official.a"]);
    assert.deepEqual(updateIslandCardOrder(enabled, "official.a", false), ["plugin.b"]);
});

test("行可用性在边界正确禁用排序", () => {
    assert.deepEqual(islandCardAvailability(["official.a", "plugin.b"], "official.a"), { enabled: true, canMoveUp: false, canMoveDown: true });
    assert.deepEqual(islandCardAvailability(["official.a", "plugin.b"], "plugin.b"), { enabled: true, canMoveUp: true, canMoveDown: false });
    assert.deepEqual(islandCardAvailability(["official.a"], "plugin.missing"), { enabled: false, canMoveUp: false, canMoveDown: false });
});
