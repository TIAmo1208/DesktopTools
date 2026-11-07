import assert from "node:assert/strict";
import test from "node:test";

import { matchTileType, pluginMenuItems, pluginTileMinimum, pluginTileTitle } from "./pluginTile.ts";
import type { ComponentManifest } from "../../contracts/component.ts";

/** 一份带类型级尺寸的插件清单。 */
const manifest = {
    id: "plugin.example",
    name: "示例",
    entry: "index.js",
    script: "plugin/index.js",
    role: "tile",
    stylesheet: "",
    themeAppearance: null,
    themeFonts: [],
    requiredCapabilities: [],
    creatable: true,
    removable: true,
    maxTiles: 1,
    tileTypes: [
        { key: "status", name: "状态", defaultSize: { width: 168, height: 170 }, minimumSize: { width: 168, height: 114 } },
        { key: "logs", name: "日志", defaultSize: null, minimumSize: null },
    ],
    persistenceVersion: 1,
} as ComponentManifest;

// 空类型键与未知键回落到首个类型；未声明尺寸时由公共底线兜底。
test("matches tile type and type-level minimum", () => {
    assert.equal(matchTileType(manifest, "logs")?.key, "logs");
    assert.equal(matchTileType(manifest, "")?.key, "status");
    assert.equal(matchTileType(manifest, "unknown")?.key, "status");
    assert.deepEqual(pluginTileMinimum(manifest, ""), { width: 168, height: 114 });
    assert.equal(pluginTileMinimum(manifest, "logs"), undefined);
    assert.equal(pluginTileMinimum(undefined, "status"), undefined);
});

// 声明了类型就用该类型的显示名作标题；隐式单类型沿用插件名。
test("tile title follows the declared type name", () => {
    assert.equal(pluginTileTitle(manifest, "status", "示例"), "状态");
    assert.equal(pluginTileTitle(manifest, "logs", "示例"), "日志");
    assert.equal(pluginTileTitle(manifest, "", "示例"), "状态");
    assert.equal(pluginTileTitle({ ...manifest, tileTypes: [] }, "", "示例"), "示例");
    assert.equal(pluginTileTitle(undefined, "status", "示例"), "示例");
});

// 插件菜单项按注册顺序映射到公共菜单，分组与禁用原样保留。
test("maps plugin menu entries to shell menu items", () => {
    const selected: string[] = [];
    const items = pluginMenuItems([
        { id: "a", label: "打开", onSelect: () => selected.push("a") },
        { id: "b", label: "刷新", disabled: true, separatorBefore: true, description: "进行中", onSelect: () => selected.push("b") },
    ]);
    assert.deepEqual(
        items.map((item) => [item.label, item.disabled, item.separatorBefore, item.description]),
        [
            ["打开", false, false, undefined],
            ["刷新", true, true, "进行中"],
        ],
    );
    items[0].onSelect?.();
    assert.deepEqual(selected, ["a"]);
});
