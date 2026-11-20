import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { parseComponentRuntimeSnapshot } from "../../../contracts/component.ts";

import { islandCardAvailability, moveIslandCard, normalizeIslandCardOrder, sortIslandCardManifests, updateIslandCardOrder } from "./islandSettings.ts";

test("真实官方音乐清单进入岛设置列表，勾选才写入顺序", () => {
    const official = JSON.parse(readFileSync(new URL("../../../../src-tauri/components/official.music-control.json", import.meta.url), "utf8")); // 与 Rust include_str 使用同一清单。
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

test("收集箱与天气清单声明为浮岛卡且带卡片说明", () => {
    for (const file of ["official.collect-box.json", "official.weather.json"]) {
        const manifest = JSON.parse(readFileSync(new URL(`../../../../src-tauri/components/${file}`, import.meta.url), "utf8"));
        assert.equal(manifest.island, true, `${file} 必须在设置页可开启`);
        assert.equal(typeof manifest.description, "string");
        assert.ok(manifest.description.length > 0);
    }
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

test("锚点显示二级页绑定三枚开关，音乐面板读写同一偏好，卸载入口只对插件卡渲染", () => {
    const page = readFileSync(new URL("./IslandSettingsPage.tsx", import.meta.url), "utf8");
    const music = readFileSync(new URL("../features/FeatureSettings.tsx", import.meta.url), "utf8");
    // 一级页是二级入口，不再保留全局胶囊消息开关。
    assert.match(page, /锚点显示/);
    assert.match(page, /setAnchorPage\(true\)/);
    assert.doesNotMatch(page, /islandCapsuleMessages/);
    assert.doesNotMatch(page, /胶囊消息通知/);
    // 三枚开关经同一读取函数与同一补丁写入，缺键默认由契约常量决定，并沿用 unavailable 停用。
    for (const key of ["music", "message", "plugin"]) assert.match(page, new RegExp(`"${key}", "[^"]*上浮岛显示"`));
    assert.match(page, /id=\{`island-anchor-\$\{key\}`\}/);
    assert.match(page, /checked=\{islandAnchorVisible\(preferences, key\)\}/);
    assert.match(page, /onPreferences\(\{ islandAnchorVisible: \{ \.\.\.\(preferences\?\.islandAnchorVisible \?\? \{\}\), \[key\]: checked \} \}\)/);
    assert.match(page, /disabled=\{unavailable\}/);
    assert.match(page, /控制该功能锚点在胶囊与 hover 同时出现或撤下/);
    assert.match(page, /展开卡仍按卡片开关/);
    // 功能格子音乐二级页与浮岛二级页读同一函数、写同一字段，默认值不在页面里再写一份。
    assert.match(music, /id="feature-music-island-anchor"/);
    assert.match(music, /label="音乐上浮岛显示"/);
    assert.match(music, /checked=\{islandAnchorVisible\(preferences, "music"\)\}/);
    assert.match(music, /onPreferences\(\{ islandAnchorVisible: \{ \.\.\.\(preferences\?\.islandAnchorVisible \?\? \{\}\), music: checked \} \}\)/);
    assert.doesNotMatch(music, /ISLAND_ANCHOR_MUSIC_VISIBLE_DEFAULT/);
    assert.match(music, /MusicLyricsRow/);
    assert.match(page, /歌词与频谱在「功能格子 → 音乐控制」中设置/);
    assert.doesNotMatch(page, /MusicLyricsRow|MusicSpectrumRow/);
    // 卸载入口只在插件卡分支渲染；官方卡仍保留上浮岛开关与列表菜单的停用项。
    assert.match(page, /\{detail\.id\.startsWith\("plugin\."\) \? \(/);
    assert.match(page, /island-settings__trash/);
    assert.match(page, /上浮岛/);
    assert.match(page, /label: "停用此卡片"/);
    // 详情页命令区不再无条件渲染卸载按钮：按钮必须落在插件判断之后。
    const commands = page.slice(page.indexOf('className="island-settings__commands"'));
    assert.ok(commands.indexOf('startsWith("plugin.")') < commands.indexOf("island-settings__trash"));
});

test("一级分组标题走共享式样、功能行距交给容器 gap、二级页返回后焦点回入口", () => {
    const 提取规则 = (css: string) =>
        [...css.replace(/\/\*[\s\S]*?\*\//g, "").matchAll(/([^{}]+)\{([^{}]*)\}/g)].map(([, selector, body]) => ({ selector: selector.trim(), body }));
    const 岛页 = readFileSync(new URL("./IslandSettingsPage.tsx", import.meta.url), "utf8");
    const 岛样式 = 提取规则(readFileSync(new URL("./IslandSettingsPage.css", import.meta.url), "utf8"));
    const 功能页 = readFileSync(new URL("../features/FeatureSettings.tsx", import.meta.url), "utf8");
    const 功能样式源 = readFileSync(new URL("../features/FeatureSettings.css", import.meta.url), "utf8");
    const 功能样式 = 提取规则(功能样式源);
    const 插件页 = readFileSync(new URL("../plugins/PluginSettingsPage.tsx", import.meta.url), "utf8");
    // 浮岛页的 h2 规则只允许二级页标题一层（18px/600/38px）；一级分组标题不得有页内覆盖。
    const 岛内标题规则 = 岛样式.filter(({ selector }) => selector.includes("h2"));
    assert.deepEqual(
        岛内标题规则.map(({ selector }) => selector),
        [".island-settings__heading h2"],
    );
    assert.match(岛内标题规则[0]?.body ?? "", /font-size:\s*18px/);
    assert.match(岛内标题规则[0]?.body ?? "", /line-height:\s*38px/);
    const 卡片标签 = 岛样式.find(({ selector }) => selector === ".island-settings__label");
    assert.match(卡片标签?.body ?? "", /margin:\s*0;/);
    assert.match(卡片标签?.body ?? "", /font-size:\s*12px/);
    assert.match(卡片标签?.body ?? "", /font-weight:\s*600/);
    for (const id of ["island-look-title", "island-behavior-title", "island-anchor-entry-title"]) {
        assert.match(岛页, new RegExp(`<h2 className="settings__group-title" id="${id}"`));
    }
    // 功能格子页：格子标题走共享类且挂在分组容器里；页内没有泛型 h2 覆盖，二级页标题的 18px 规则仅限 detail-title。
    assert.match(功能页, /className="feature-settings__list settings__group"/);
    assert.equal(功能样式.find(({ selector }) => selector === ".feature-settings h2"), undefined);
    // 功能行距由所在列表容器的 gap 提供，行自身不再留下外边距。
    const 功能行规则 = 功能样式.find(({ selector }) => selector === ".feature-settings__row");
    assert.doesNotMatch(功能行规则?.body ?? "", /margin-bottom/);
    // 频谱下拉停用时文字换次要色，与共享 .settings__select:disabled 同口径。
    const 下拉停用 = 功能样式.find(({ selector }) => selector === ".feature-settings select:disabled");
    assert.match(下拉停用?.body ?? "", /color:\s*var\(--color-muted\)/);
    // 三个二级页都记录返回焦点并在回到一级页时复焦入口。
    for (const 源码 of [岛页, 功能页, 插件页]) {
        assert.match(源码, /returnFocusSelector/);
        assert.match(源码, /\.focus\(\)/);
    }
});
