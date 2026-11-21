import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { pluginAssetUrl } from "../../bridge/pluginAssets.ts";
import { ISLAND_MESSAGE_CARD_ID } from "../../contracts/settings.ts";
import { islandCardCollapseDefault, islandCardRememberedCollapsed, islandMessageSlotIndex } from "./islandMessageCard.ts";
import { moveIslandVisibleCard } from "./islandGeometry.ts";

test("消息通知卡按 islandCardOrder 插入到实例卡之间，未初始化不渲染", () => {
    const order = ["official.music-control", "official.weather", ISLAND_MESSAGE_CARD_ID, "plugin.a", "plugin.b"];
    const instances = ["official.music-control", "official.weather", "plugin.a", "plugin.b"];
    assert.equal(islandMessageSlotIndex(order, instances), 2); // 天气之后、插件之前。
    assert.equal(islandMessageSlotIndex(["official.weather", ISLAND_MESSAGE_CARD_ID], ["official.weather"]), 1); // 末尾槽。
    assert.equal(islandMessageSlotIndex([ISLAND_MESSAGE_CARD_ID], []), 0);
    assert.equal(islandMessageSlotIndex([ISLAND_MESSAGE_CARD_ID, "plugin.a"], ["plugin.a"]), 0); // 首个插件之前。
    assert.equal(islandMessageSlotIndex(["plugin.a"], ["plugin.a"]), -1); // 顺序里没有消息卡时不渲染。
    assert.equal(islandMessageSlotIndex([], []), -1);
});

test("插件图标走 /plugins 资产地址，缺失或加载失败回落通用图标", () => {
    assert.equal(pluginAssetUrl("DSH 接入/icon.svg"), "/plugins/DSH 接入/icon.svg");
    for (const bad of ["", "/abs/icon.svg", "../icon.svg", "sub/../icon.svg", "sub//icon.svg", "./icon.svg"])
        assert.equal(pluginAssetUrl(bad), undefined); // 与脚本共用同一条相对路径边界。
    const source = readFileSync(new URL("./island.tsx", import.meta.url), "utf8");
    assert.match(source, /const PluginIdentityIcon = \(\{ iconPath, size = 20 \}/); // 锚点 20、展开标题 16。
    assert.match(source, /const source = islandPluginIconSource\(iconPath, failedUrl, pluginAssetUrl\)/);
    assert.match(source, /if \(source\.kind === "fallback"\) return <IslandFunctionIcon kind="plugin" \/>/); // 缺失/失败不出现破图。
    assert.match(source, /<img[\s\S]{0,240}className="island-plugin-identity"[\s\S]{0,240}src=\{source\.url\}[\s\S]{0,240}onError=\{\(\) => setFailedUrl\(source\.url\)\}/); // 原色 img。
    const css = readFileSync(new URL("./island.css", import.meta.url), "utf8");
    const start = css.indexOf(".island-plugin-identity {");
    const rule = css.slice(start, css.indexOf("}", start));
    assert.match(rule, /object-fit: contain;/);
    assert.match(rule, /background: transparent;/);
    assert.doesNotMatch(rule, /mask|currentColor/); // 不再压成单色剪影。
    assert.match(source, /<PluginIdentityMark status=\{cardStatus\} iconPath=\{manifest\.iconPath\} size=\{16\} \/>/); // 标题位也先判状态。
    assert.match(source, /islandPluginMark\(status\) === "logo" \? <PluginIdentityIcon/); // 仅正常连接用身份图标。
});

test("消息通知卡按序渲染，固定首位已移除", () => {
    const source = readFileSync(new URL("./island.tsx", import.meta.url), "utf8");
    assert.match(source, /messageSlotIndexFor\(preferences\?\.islandCardOrder/);
    assert.match(source, /index === messageSlotIndex \? renderMessageCard\(\) : null/);
    assert.match(source, /messageSlotIndex === instances\.length \? renderMessageCard\(\) : null/);
    assert.doesNotMatch(source, /aria-label="消息" data-island-message="true"/); // 不再固定首位。
    assert.match(source, /if \(!visibleMessages\.length && !messageFailure\) return null/); // 无消息且无故障不画空卡。
    assert.match(source, /if \(summary\?\.text === null\) return \[\];/); // 显式隐藏锚点不回落清单名。
});

test("键盘排序按完整保存顺序交换可见卡，消息槽不丢且自身可移动", () => {
    const order = ["official.weather", ISLAND_MESSAGE_CARD_ID, "plugin.dsh", "official.music-control"];
    assert.deepEqual(moveIslandVisibleCard(order, "official.weather", "plugin.dsh"), [
        "plugin.dsh",
        ISLAND_MESSAGE_CARD_ID,
        "official.weather",
        "official.music-control",
    ]); // 其它卡相邻移动保留消息槽。
    assert.deepEqual(moveIslandVisibleCard(order, ISLAND_MESSAGE_CARD_ID, "plugin.dsh"), [
        "official.weather",
        "plugin.dsh",
        ISLAND_MESSAGE_CARD_ID,
        "official.music-control",
    ]); // 消息卡自身可上下移动。
    assert.deepEqual(moveIslandVisibleCard(order, "official.weather", "official.weather"), order); // 原地不动。
    const source = readFileSync(new URL("./island.tsx", import.meta.url), "utf8");
    assert.match(source, /const order = preferences\?\.islandCardOrder \?\? instances\.map/); // 用完整保存顺序，不用实例集合重建。
    assert.match(source, /const sortableIds = \(preferences\?\.islandCardOrder/);
    assert.match(source, /void handleReorder\(ISLAND_MESSAGE_CARD_ID, to\)/); // 消息卡外壳参与方向键排序。
    assert.doesNotMatch(source, /const order = sortable\.map/);
});

test("消息槽初始化只在保存成功后置标记，失败下次重试", () => {
    const source = readFileSync(new URL("./island.tsx", import.meta.url), "utf8");
    assert.match(source, /savePreferences\(\{ \.\.\.preferences, \.\.\.initialization \}\)/); // 走权威保存命令。
    assert.match(source, /\.then\(\(saved\) => \{\s*if \(active\) onPreferences\(saved\);/); // 成功才回写完整偏好。
    assert.match(source, /消息通知卡初始化保存失败，下次重试/); // 失败不置标记。
});

test("消息卡默认收起到最近一条，用户显式操作后按记忆恢复", () => {
    assert.equal(islandCardCollapseDefault(ISLAND_MESSAGE_CARD_ID, true), true); // 首次默认收起。
    assert.equal(islandCardCollapseDefault("official.music-control", true), false); // 其它卡默认展开。
    assert.equal(islandCardCollapseDefault("official.collect-box", true), true); // 收集箱沿用偏好默认。
    assert.equal(islandCardCollapseDefault("official.collect-box", false), false);

    assert.equal(islandCardRememberedCollapsed({}, {}, ISLAND_MESSAGE_CARD_ID, true), true); // 从未操作过：默认收起。
    assert.equal(islandCardRememberedCollapsed({ [ISLAND_MESSAGE_CARD_ID]: false }, {}, ISLAND_MESSAGE_CARD_ID, true), false); // 用户展开后写入记忆并生效。
    assert.equal(islandCardRememberedCollapsed({ [ISLAND_MESSAGE_CARD_ID]: false }, { [ISLAND_MESSAGE_CARD_ID]: false }, ISLAND_MESSAGE_CARD_ID, true), true); // 关闭记忆开关回到默认收起。
    assert.equal(islandCardRememberedCollapsed({ [ISLAND_MESSAGE_CARD_ID]: true }, {}, ISLAND_MESSAGE_CARD_ID, true), true); // 用户主动收起也记住。
    assert.equal(islandCardRememberedCollapsed({}, {}, "official.music-control", true), false); // 其它卡行为不变。
    assert.equal(islandCardRememberedCollapsed({ "official.music-control": true }, {}, "official.music-control", true), true);
    assert.equal(islandCardRememberedCollapsed({ "official.music-control": true }, { "official.music-control": false }, "official.music-control", true), false); // 记忆关闭回默认展开。
});

test("消息面板收起态由宿主统一记忆驱动，不再用局部 useState", () => {
    const source = readFileSync(new URL("./island.tsx", import.meta.url), "utf8");
    assert.match(source, /const collapseDefault = \(componentId: string\) => islandCardCollapseDefault\(/); // 默认收起规则走纯模型。
    assert.doesNotMatch(source, /const \[open, setOpen\] = useState\(false\)/); // 不再维护局部折叠状态。
    assert.match(source, /collapsed=\{cardCollapsed\(ISLAND_MESSAGE_CARD_ID\)\}/);
    assert.match(source, /onToggle=\{\(card\) => \{[\s\S]*?handleToggleCard\(ISLAND_MESSAGE_CARD_ID, card\);\s*\}/); // 卡头写入统一收起记忆。
    assert.match(source, /aria-expanded=\{!collapsed\}/);
});
