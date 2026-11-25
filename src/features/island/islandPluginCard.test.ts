import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { pluginAssetUrl } from "../../bridge/pluginAssets.ts";

import {
    ISLAND_PLUGIN_SUMMARY_LIMIT,
    islandPluginMessageDeliverable,
    islandPluginMessageKey,
    islandPluginAnchorStatus,
    islandPluginIconSource,
    islandPluginMark,
    islandPluginMountSites,
    islandPluginStatusFromSummary,
    islandPluginSummaryCallbacks,
    islandPluginPrimary,
    islandPluginSummaries,
    islandPluginSummaryText,
} from "./islandPluginCard.ts";

test("摘要是单行文本：剥控制字符、拒空、按字符上限截断", () => {
    assert.equal(islandPluginSummaryText("  3 项待决\n"), "3 项待决");
    assert.equal(islandPluginSummaryText("构建\u0000完成\u0007"), "构建 完成"); // 控制字符折成空格。
    assert.equal(islandPluginSummaryText("   "), undefined);
    assert.equal(islandPluginSummaryText(undefined), undefined);
    assert.equal(islandPluginSummaryText("长".repeat(ISLAND_PLUGIN_SUMMARY_LIMIT + 20)), "长".repeat(ISLAND_PLUGIN_SUMMARY_LIMIT));
    // 胶囊一行放不下整段摘要，摘要入口只保留前 40 个字符。
    assert.equal(Array.from(islandPluginSummaryText("😀".repeat(50)) ?? "").length, ISLAND_PLUGIN_SUMMARY_LIMIT);
});

test("插件未调用摘要时回落清单名称，null 或空串显式隐藏", () => {
    assert.equal(islandPluginPrimary(undefined, "DSH 接入"), "DSH 接入"); // 从未调用才回落。
    assert.equal(islandPluginPrimary(null, "DSH 接入"), ""); // 显式隐藏不回落清单名。
    assert.equal(islandPluginPrimary("   ", "DSH 接入"), ""); // 空串与空白同显式隐藏。
    assert.equal(islandPluginPrimary("3 项待决", "DSH 接入"), "3 项待决"); // 插件调用 setIslandSummary 后生效。
});

test("摘要表按组件更新：null 与空串显式隐藏，未调用才回落", () => {
    const initial = {} as Readonly<Record<string, { text: string | null; status?: string }>>;
    const set = islandPluginSummaries(initial, "plugin.dsh", "3 项待决");
    assert.deepEqual(set, { "plugin.dsh": { text: "3 项待决" } });
    assert.equal(islandPluginSummaries(set, "plugin.dsh", "3 项待决"), set); // 内容相同不触发重绘。
    assert.deepEqual(islandPluginSummaries(set, "plugin.dsh", null), { "plugin.dsh": { text: null } }); // 显式隐藏锚点。
    assert.deepEqual(islandPluginSummaries(set, "plugin.dsh", ""), { "plugin.dsh": { text: null } }); // 空串同隐藏。
    assert.deepEqual(islandPluginSummaries(set, "plugin.other", "待处理"), { "plugin.dsh": { text: "3 项待决" }, "plugin.other": { text: "待处理" } });
    assert.deepEqual(islandPluginSummaries(set, "plugin.dsh", undefined), {}); // undefined 只用于从未调用，撤下条目。
    assert.equal(islandPluginSummaries(initial, "plugin.dsh", undefined), initial);
});

test("消息身份带来源，暂停时不投递", () => {
    assert.equal(islandPluginMessageKey("plugin.dsh", "pending:p1"), "plugin:plugin.dsh:pending:p1");
    assert.equal(islandPluginMessageKey("plugin.other", "pending:p1"), "plugin:plugin.other:pending:p1"); // 跨插件不冲突。
    assert.equal(islandPluginMessageDeliverable(false), true);
    assert.equal(islandPluginMessageDeliverable(true), false); // 锁屏、浮岛隐藏、全屏隐藏时不接收新提醒。
});

test("浮岛常态挂载隐藏宿主、接通摘要入口并按运行状态暂停", () => {
    const source = readFileSync(new URL("./island.tsx", import.meta.url), "utf8");
    const css = readFileSync(new URL("./island.css", import.meta.url), "utf8");
    assert.match(source, /instance\.componentId\.startsWith\("plugin\.\"\) && manifest\?\.island === true/); // 只挂已声明 island 的插件。
    assert.match(source, /mode !== "expanded" && islandRunning \? \(/); // 常态与 hover 也挂载。
    assert.match(source, /<div className="island-plugin-runtime" aria-hidden="true" inert>/);
    assert.doesNotMatch(source, /island-plugin-runtime[\s\S]{0,80}display:\s*none/); // 不靠 display:none 隐藏宿主。
    assert.match(source, /const islandRunning = !islandPaused && preferences\?\.islandEnabled === true/); // 关闭、锁屏、隐藏时卸载。
    assert.doesNotMatch(source, /onIslandSummary=\{\(text\) =>/); // 不再用内联回调，避免重绘换函数身份。
    assert.match(source, /islandPluginMessageDeliverable\(!islandRunningRef\.current\)/); // 暂停或关闭期间不接收新提醒。
    assert.match(css, /\.island-plugin-runtime \{[\s\S]*?inset-inline-start: -10000px;[\s\S]*?overflow: hidden;/);
});

test("暂停时三种挂载位都不存在，恢复后每个实例只有一处", () => {
    const instanceIds = ["island.plugin.a", "island.plugin.b"];
    for (const mode of ["collapsed", "hover", "expanded"] as const) {
        const paused = islandPluginMountSites({ running: false, mode, instanceIds });
        assert.equal(paused.size, 0); // 锁屏、全屏隐藏、关岛时不挂任何插件卡。
    }
    const collapsed = islandPluginMountSites({ running: true, mode: "collapsed", instanceIds });
    assert.deepEqual([...collapsed.values()], ["hidden", "hidden"]); // 常态只有隐藏宿主。

    const hover = islandPluginMountSites({ running: true, mode: "hover", instanceIds });
    assert.deepEqual([...hover.values()], ["hidden", "hidden"]); // 悬停期间 renderTile 宿主不断开轮询。

    const expanded = islandPluginMountSites({ running: true, mode: "expanded", instanceIds });
    assert.deepEqual([...expanded.values()], ["expanded", "expanded"]); // 展开态不再留隐藏宿主。
    for (const sites of [collapsed, hover, expanded]) {
        assert.equal(sites.size, instanceIds.length); // 同一实例任一时刻只登记一处。
    }
});

test("摘要回调按 componentId 保持稳定，多次调用只更新摘要", () => {
    const calls: string[] = [];
    const callbackFor = islandPluginSummaryCallbacks((componentId, text) => void calls.push(`${componentId}:${text}`));
    const first = callbackFor("plugin.a");
    const again = callbackFor("plugin.a");
    const other = callbackFor("plugin.b");
    assert.equal(first, again); // 同一实例拿到同一函数，不会触发插件内容重挂。
    assert.notEqual(first, other); // 不同实例各自独立。
    first("1 项待决");
    again("2 项待决");
    other("运行中");
    assert.deepEqual(calls, ["plugin.a:1 项待决", "plugin.a:2 项待决", "plugin.b:运行中"]);
});

test("摘要更新不重建插件实例：稳定回调进入上下文，挂载位使用快照实例身份", () => {
    const source = readFileSync(new URL("./island.tsx", import.meta.url), "utf8");
    const slot = readFileSync(new URL("../components/PluginSlot.tsx", import.meta.url), "utf8");
    assert.match(source, /const summaryCallback = useMemo\(\(\) => islandPluginSummaryCallbacks\(updatePluginSummary\), \[updatePluginSummary\]\)/);
    assert.match(source, /onIslandSummary=\{summaryCallback\(instance\.componentId\)\}/); // 常态宿主。
    assert.match(source, /onIslandSummary=\{summaryCallback\(plugin\.componentId\)\}/); // hover 详情。
    assert.match(source, /const pluginSites = islandPluginMountSites\(\{/); // 三处挂载统一按计划。
    assert.match(source, /pluginSites\.get\(instance\.instanceId\) !== "hidden"/);
    assert.match(source, /hoverPluginInstance\?\.instanceId === plugin\.instanceId/); // hover 视图与隐藏宿主并存。
    assert.match(source, /pluginSites\.get\(instance\.instanceId\) === "expanded"/);
    assert.match(slot, /const islandContext = useMemo\(\(\) => \(onIslandSummary \? \{ setIslandSummary: onIslandSummary \} : \{\}\), \[onIslandSummary\]\)/); // 只在回调身份变化时改上下文。
    assert.doesNotMatch(slot, /pluginSummaries/); // 摘要状态不进入插件挂载依赖。
    assert.doesNotMatch(source, /island\.\$\{/); // 复挂沿用快照的 island.<pluginId> 实例身份，不由前端另造。
});

test("业务状态映射到既有符号，显式 status 优先于短词", () => {
    for (const [text, status] of [
        ["未授权", "unauthorized"],
        ["未连接", "disconnected"],
        ["读取失败", "failed"],
        ["数据陈旧", "paused"],
        ["3 项待决", "awaiting"],
        ["99+ 项待决", "awaiting"],
        ["账号未登录", "unauthorized"],
        ["余额未授权", "unauthorized"],
        ["余额读取失败", "failed"],
        ["已连接", "pending"],
    ] as const)
        assert.equal(islandPluginStatusFromSummary(text), status); // 与规格状态表一致。
    assert.equal(islandPluginStatusFromSummary("CNY 12.34"), undefined); // 普通读数不改变符号。
    assert.equal(islandPluginStatusFromSummary(null), undefined);
    assert.equal(islandPluginAnchorStatus("failed", "3 项待决", "pending"), "failed"); // 插件显式状态优先。
    assert.equal(islandPluginAnchorStatus(undefined, "3 项待决", "pending"), "awaiting"); // 旧插件短词兼容。
    assert.equal(islandPluginAnchorStatus(undefined, "未知文案", "disconnected"), "disconnected"); // 无法映射回落注册状态。
});

test("悬停期间保留 renderTile 宿主，hover 只读视图单独清理", () => {
    const source = readFileSync(new URL("./island.tsx", import.meta.url), "utf8");
    const slot = readFileSync(new URL("../components/PluginSlot.tsx", import.meta.url), "utf8");
    assert.match(source, /mode !== "expanded" && islandRunning \?/); // hover 也挂隐藏宿主。
    assert.match(source, /hoverPluginInstance\?\.instanceId === plugin\.instanceId \?/); // hover 只读视图另挂。
    assert.match(slot, /const hoverCleanups = new Map/); // hover 视图独立清理表。
    assert.match(slot, /if \(hover\) releasePluginHoverNow\(context\.instanceId\)/); // hover 只结束自己的会话。
    assert.match(slot, /else releasePluginTileNow\(context\.instanceId\)/); // renderTile 先同步释放旧会话。
    assert.match(slot, /releasePluginTileSession\(instanceId, sessionRef\.current\)/); // 清理只结束自己创建的会话。
});

test("状态符号优先于身份 Logo：六种状态各自映射到正确图形", () => {
    assert.equal(islandPluginMark("pending"), "logo"); // 仅正常连接/余额读数用身份 Logo。
    assert.equal(islandPluginMark("awaiting"), "question");
    assert.equal(islandPluginMark("paused"), "pause");
    assert.equal(islandPluginMark("unauthorized"), "lock");
    assert.equal(islandPluginMark("failed"), "alert");
    assert.equal(islandPluginMark("disconnected"), "link-off");
    assert.equal(islandPluginMark("running"), "ring");
    assert.equal(islandPluginMark("plugin.custom"), undefined); // 未知扩展状态仍走插件自绘。
    const source = readFileSync(new URL("./island.tsx", import.meta.url), "utf8");
    assert.match(source, /islandPluginMark\(status\) === "logo" \? <PluginIdentityIcon/); // 锚点只对 logo 档渲染身份图标。
    assert.match(source, /<PluginIdentityMark status=\{cardStatus\} iconPath=\{manifest\.iconPath\} size=\{16\} \/>/); // 展开标题同源。
});

test("插件图标源：iconPath 存在用原图，缺失/非法/加载失败回落", () => {
    assert.deepEqual(islandPluginIconSource("DSH 接入/icon.svg", undefined, pluginAssetUrl), {
        kind: "image",
        url: "/plugins/DSH 接入/icon.svg",
    }); // 官方原色图直接走资产地址。
    for (const missing of [undefined, "", "../icon.svg", "/abs/icon.svg"])
        assert.equal(islandPluginIconSource(missing, undefined, pluginAssetUrl).kind, "fallback"); // 缺失或非法路径回落。
    assert.equal(islandPluginIconSource("DSH 接入/icon.svg", "/plugins/DSH 接入/icon.svg", pluginAssetUrl).kind, "fallback"); // 加载失败按 URL 记录。
    assert.equal(islandPluginIconSource("DSH 接入/icon.svg", "/plugins/other.svg", pluginAssetUrl).kind, "image"); // 换图标后重新尝试。
});

test("插件摘要可声明导航，回调原样传递 uri", async () => {
    const withNavigation = islandPluginSummaries({}, "plugin.dsh-status", "待批准", "awaiting", { uri: "dsh://open" });
    assert.deepEqual(withNavigation, {
        "plugin.dsh-status": { text: "待批准", status: "awaiting", navigation: { uri: "dsh://open" } },
    }); // 导航随摘要保存，锚点据此可点。
    assert.deepEqual(islandPluginSummaries({}, "plugin.dsh-status", "待批准", "awaiting", { uri: "   " }), {
        "plugin.dsh-status": { text: "待批准", status: "awaiting" },
    }); // 空地址等于未声明。
    const calls: unknown[] = [];
    const callbackFor = islandPluginSummaryCallbacks((componentId, text, status, navigation) => calls.push([componentId, text, status, navigation]));
    callbackFor("plugin.dsh-status")("待批准", "awaiting", { uri: "dsh://open" });
    assert.deepEqual(calls, [["plugin.dsh-status", "待批准", "awaiting", { uri: "dsh://open" }]]);
});
