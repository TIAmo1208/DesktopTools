import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { adoptPluginHover, adoptPluginShellOnce, mountPluginHover } from "../../contracts/plugin.ts";
import {
    ISLAND_ANCHOR_GAP,
    ISLAND_ANCHOR_GROW_MS,
    ISLAND_ANCHOR_SHRINK_MS,
    ISLAND_ANCHOR_SIZE,
    ISLAND_ANCHOR_SLOTS,
    ISLAND_HOVER_CONTROL_LIMIT,
    ISLAND_HOVER_LINE_LIMIT,
    ISLAND_LAYER_ORDER,
    ISLAND_MESSAGE_ANCHOR_TTL_MS,
    ISLAND_NOTICE_LAYER,
    ISLAND_STATUS_SPECS,
    islandAnchorGlyph,
    islandAnchorId,
    islandAnchorRow,
    islandAnchorValue,
    islandArrive,
    islandCapsuleFace,
    islandClockLabel,
    islandCollectArrival,
    islandDisplayAnchors,
    islandDropFace,
    islandHoverBody,
    islandHoverContent,
    islandHoverEnter,
    islandHoverInitialSelection,
    islandHoverProvider,
    islandHoverSelect,
    islandHoverSessionWidth,
    islandHoverStep,
    islandHoverInlineSize,
    islandLayerRank,
    islandMusicAnchorVisible,
    islandStatusProgress,
    islandStatusSpec,
    islandWeatherNotice,
    type IslandAnchor,
    type IslandContentLayer,
    type IslandContentUnit,
    type IslandStatus,
    type IslandStatusSpec,
} from "./islandContent.ts";

/** 测试用内容单元：一行主信息加可选附加项；状态既可以是内置十态，也可以是插件扩展值。 */
const unit = (status: string, primary: string, secondary?: string): IslandContentUnit =>
    secondary === undefined ? { status, primary } : { status, primary, secondary };

/** 测试用锚点：身份由层级与来源派生，与模型规则一致。 */
const anchor = (layer: IslandContentLayer, source: string, nowMs = 0, content?: IslandContentUnit): IslandAnchor => ({
    id: islandAnchorId(layer, source),
    source,
    layer,
    updatedAtMs: nowMs,
    ...(content === undefined ? {} : { unit: content }),
});

test("内容层复用视觉硬数值：锚点 28、间距 12、最多并排 3 枚，接力与超时取固定节奏", () => {
    assert.equal(ISLAND_ANCHOR_SIZE, 28);
    assert.equal(ISLAND_ANCHOR_GAP, 12);
    assert.equal(ISLAND_ANCHOR_SLOTS, 3);
    assert.ok(ISLAND_ANCHOR_GROW_MS >= 280 && ISLAND_ANCHOR_GROW_MS <= 320, "生长节奏在 280–320ms 内");
    assert.ok(ISLAND_ANCHOR_SHRINK_MS >= 200 && ISLAND_ANCHOR_SHRINK_MS <= 280, "收缩节奏在 200–280ms 内");
    assert.equal(ISLAND_MESSAGE_ANCHOR_TTL_MS, 30_000);
    assert.equal(ISLAND_HOVER_LINE_LIMIT, 2);
    assert.equal(ISLAND_HOVER_CONTROL_LIMIT, 3);
});

test("十种基础状态各自给出视觉差异、一句话短词与结束后的去向", () => {
    const specs = Object.values(ISLAND_STATUS_SPECS);
    assert.equal(specs.length, 10);
    assert.equal(new Set(specs.map((spec) => spec.status)).size, 10);
    assert.equal(new Set(specs.map((spec) => spec.word)).size, 10); // 短词固定且互不重复。
    const visuals = specs.map((spec) => (spec.visual.kind === "glyph" ? `glyph:${spec.visual.symbol}` : spec.visual.kind === "ring" ? `ring:${spec.visual.source}` : "sweep"));
    assert.equal(new Set(visuals).size, 10); // 弧长或符号两两不同。
    for (const spec of specs) {
        assert.ok(spec.word.length > 0 && spec.word.length <= 4, spec.word);
        assert.ok(["linger", "dot", "vanish"].includes(spec.disposition), spec.status);
    }
    // 结束后去向：完成收成圆点，取消直接消失，确认与失败停留。
    assert.equal(ISLAND_STATUS_SPECS.completed.disposition, "dot");
    assert.equal(ISLAND_STATUS_SPECS.cancelled.disposition, "vanish");
    assert.equal(ISLAND_STATUS_SPECS.awaiting.disposition, "linger");
    assert.equal(ISLAND_STATUS_SPECS.failed.disposition, "linger");
    assert.deepEqual(Object.keys(ISLAND_STATUS_SPECS).sort(), [
        "awaiting",
        "cancelled",
        "completed",
        "disconnected",
        "failed",
        "indeterminate",
        "paused",
        "pending",
        "running",
        "unauthorized",
    ]);
    // 状态不封闭：插件按同一形状补充状态，内置状态不可被改写。
    const uploading: IslandStatusSpec = { status: "uploading", word: "上传中", visual: { kind: "ring", source: "live" }, disposition: "linger" };
    assert.equal(islandStatusSpec("uploading"), undefined);
    assert.deepEqual(islandStatusSpec("uploading", [uploading]), uploading);
    assert.equal(islandStatusSpec("running", [{ ...uploading, status: "running", word: "改写" }])?.word, "进行中");
    // 扩展状态可以进入内容单元，内置图形解析仍只覆盖内置十态。
    const extended = anchor("event", "upload", 0, unit("uploading", "同步相册"));
    assert.equal(islandStatusSpec(extended.unit!.status, [uploading])?.word, "上传中");
});

test("状态变化直接落终态：归一化进度与锚点图形都不保留未完成动画", () => {
    assert.equal(islandStatusProgress("running", 0.42), 0.42);
    assert.equal(islandStatusProgress("paused", 0.42), 0.42);
    assert.equal(islandStatusProgress("pending", 0.42), 0);
    assert.equal(islandStatusProgress("completed", 0.42), 1);
    assert.equal(islandStatusProgress("failed", 0.42), 1);
    assert.equal(islandStatusProgress("cancelled", 0.42), 0);
    assert.equal(islandStatusProgress("unauthorized", 0.9), 0);
    assert.equal(islandStatusProgress("indeterminate", 0.9), undefined); // 无总量没有归一化进度。
    assert.equal(islandStatusProgress("running", Number.NaN), 0);
    assert.equal(islandStatusProgress("running", -1), 0);
    assert.equal(islandStatusProgress("running", 2), 1);
    assert.deepEqual(islandAnchorGlyph("running", 0.42), { kind: "ring", arc: 0.42 });
    assert.deepEqual(islandAnchorGlyph("pending", 0.42), { kind: "ring", arc: 0 });
    assert.deepEqual(islandAnchorGlyph("completed", 0.42), { kind: "glyph", symbol: "check" });
    assert.deepEqual(islandAnchorGlyph("cancelled"), { kind: "glyph", symbol: "cross" });
    assert.deepEqual(islandAnchorGlyph("indeterminate"), { kind: "sweep" });
});

test("锚点身份与顺序：同来源原地换内容，不同来源追加到右侧，换层级另起锚点", () => {
    const message = islandAnchorId("message", "a");
    const start = [anchor("message", "a", 1_000, unit("running", "同步中"))];
    const sameSource = islandArrive(start, { source: "a", layer: "message", nowMs: 2_000, unit: unit("completed", "已同步") });
    assert.equal(sameSource.length, 1);
    assert.equal(sameSource[0]!.id, message);
    assert.equal(sameSource[0]!.updatedAtMs, 2_000); // 30s 从换内容时刻重算。
    assert.equal(sameSource[0]!.unit?.primary, "已同步");
    const otherSource = islandArrive(sameSource, { source: "b", layer: "message", nowMs: 3_000, unit: unit("pending", "排队") });
    assert.deepEqual(
        otherSource.map((item) => item.id),
        [message, islandAnchorId("message", "b")],
    );
    const otherLayer = islandArrive(otherSource, { source: "a", layer: "confirm", nowMs: 4_000, unit: unit("awaiting", "允许连接？") });
    assert.equal(otherLayer.length, 3);
    assert.equal(otherLayer.at(-1)!.id, islandAnchorId("confirm", "a"));
    // 收起再展开不换位：插入只追加在末尾，已有锚点位置不动。
    assert.deepEqual(
        otherLayer.slice(0, 2).map((item) => item.id),
        [message, islandAnchorId("message", "b")],
    );
});

test("叠加分层：拖放、确认、合并通知、活动音乐、常驻按优先级排列", () => {
    assert.deepEqual([...ISLAND_LAYER_ORDER], ["drop", "confirm", "message", "event", "resident"]);
    assert.ok(islandLayerRank("drop") < islandLayerRank("confirm"));
    assert.ok(islandLayerRank("confirm") < islandLayerRank("message"));
    assert.ok(islandLayerRank("message") < islandLayerRank("event"));
    assert.ok(islandLayerRank("event") < islandLayerRank("resident"));
});

test("＋N 折叠：最多并排三枚，第 4 枚起折成计数圆，顺序不换位", () => {
    const anchors = Array.from({ length: 5 }, (_, index) => anchor("message", `s${index}`, index, unit("pending", `第 ${index + 1} 条`)));
    const row = islandAnchorRow(anchors);
    assert.deepEqual(
        row.visible.map((item) => item.source),
        ["s0", "s1", "s2"],
    );
    assert.equal(row.overflow, 2);
    assert.deepEqual(
        row.folded.map((item) => item.source),
        ["s3", "s4"],
    );
    assert.deepEqual(islandAnchorRow(anchors.slice(0, 3)), { visible: anchors.slice(0, 3), overflow: 0, folded: [] });
    // 折叠只决定露出几枚：可见行与折叠项合起来仍是原顺序。
    assert.deepEqual(
        [...row.visible, ...row.folded].map((item) => item.source),
        ["s0", "s1", "s2", "s3", "s4"],
    );
});

test("零锚点显示 hh:mm 空态仍可展开；只剩锚点不自动隐藏", () => {
    const face = islandCapsuleFace([], new Date(2026, 0, 2, 9, 5).getTime());
    assert.deepEqual(face, { kind: "clock", text: "09:05", expandable: true });
    assert.equal(islandClockLabel(new Date(2026, 0, 2, 23, 59).getTime()), "23:59");
    const bare = anchor("event", "copy", 0); // 只剩圆，没有内容单元。
    const anchorsFace = islandCapsuleFace([bare], 0);
    assert.equal(anchorsFace.kind, "anchors");
    assert.equal(anchorsFace.expandable, true);
    assert.deepEqual(islandHoverContent(bare), { lines: [], controls: [], expandable: false });
});

test("附加项白名单：只接受整数百分比、99+ 与 mm:ss，裸整数与 N+ 被拒绝", () => {
    assert.deepEqual(islandAnchorValue("40%"), { kind: "percent", value: 40 });
    assert.deepEqual(islandAnchorValue("0%"), { kind: "percent", value: 0 });
    assert.deepEqual(islandAnchorValue("100%"), { kind: "percent", value: 100 });
    assert.deepEqual(islandAnchorValue("99+"), { kind: "count" });
    assert.deepEqual(islandAnchorValue("12:30"), { kind: "time", seconds: 750 });
    assert.deepEqual(islandAnchorValue("00:00"), { kind: "time", seconds: 0 });
    for (const rejected of ["42", "5+", "9+", "99 +", "100+", "0", "3:05", "12:60", "12:30:00", "40.5%", "101%", "042%", "40 %", " 40%", "", "—", "1h20m"])
        assert.equal(islandAnchorValue(rejected), undefined, rejected);
});

test("hover 宽度取所有锚点内容最大值，至少保留胶囊宽度", () => {
    assert.equal(islandHoverInlineSize([]), 0);
    assert.equal(islandHoverInlineSize([], 92), 92);
    assert.equal(islandHoverInlineSize([132.5]), 132.5);
    assert.equal(islandHoverInlineSize([80, 250, 120], 100), 250);
    assert.equal(islandHoverInlineSize([80, 120], 250), 250);
    assert.equal(islandHoverInlineSize([Number.NaN, Infinity, -Infinity, -90]), 0);
    assert.equal(islandHoverInlineSize([-20, Number.NaN, 72], -15), 72);
});

test("hover 会话宽度只增不减，退出后下一次从新内容测起", () => {
    let width = islandHoverSessionWidth(true, 0, 160);
    assert.equal(width, 160);
    width = islandHoverSessionWidth(true, width, 260);
    assert.equal(width, 260);
    assert.equal(islandHoverSessionWidth(true, width, 180), 260);
    assert.equal(islandHoverSessionWidth(true, width, 260), 260);
    assert.equal(islandHoverSessionWidth(true, width, Number.NaN), 260);
    width = islandHoverSessionWidth(false, width, 180);
    assert.equal(width, 0);
    assert.equal(islandHoverSessionWidth(true, width, 180), 180);
});

test("从胶囊进入 hover 优先选中当前正文，有效性仍由可见锚点决定", () => {
    const music = anchor("event", "music");
    const bell = anchor("message", "notifications");
    const plugin = anchor("resident", "plugin");
    const visible = [music, bell, plugin];
    assert.equal(islandHoverInitialSelection(visible, bell.id, music.id), bell.id); // 故意让消息既不是首项也不是上次选择。
    assert.equal(islandHoverInitialSelection(visible, bell.id, undefined), bell.id);
    assert.equal(islandHoverInitialSelection([music, plugin], bell.id, plugin.id), plugin.id); // 切态同时撤下通知时不选不可见项。
    assert.equal(islandHoverInitialSelection(visible, undefined, plugin.id), islandHoverSelect(visible, plugin.id));
    assert.equal(islandHoverInitialSelection(visible, undefined, "gone"), music.id);
    assert.equal(islandHoverInitialSelection([], undefined, music.id), undefined); // 时钟不产生可交互身份。
});

test("hover 选中项、左右键与 Enter：列表与内容一一对应，行与控件有上限", () => {
    const anchors = Array.from({ length: 4 }, (_, index) => anchor("event", `s${index}`, index, unit("running", `任务 ${index}`, "42%")));
    const ids = anchors.map((item) => item.id);
    assert.equal(islandHoverSelect(anchors, undefined), ids[0]);
    assert.equal(islandHoverSelect(anchors, "gone"), ids[0]); // 选中项失效时回落。
    assert.equal(islandHoverSelect(anchors, ids[2]), ids[2]);
    assert.equal(islandHoverStep(anchors, ids[0], 1), ids[1]);
    assert.equal(islandHoverStep(anchors, ids[1], -1), ids[0]);
    assert.equal(islandHoverStep(anchors, ids[0], -1), ids[0]); // 端点停住，不循环。
    assert.equal(islandHoverStep(anchors, ids[3], 1), ids[3]);
    assert.equal(islandHoverStep([], ids[0], 1), undefined);
    const content = islandHoverContent(anchors[1], ["打开", "暂停", "更多", "危险"]);
    assert.deepEqual(content.lines, ["任务 1", "42%"]);
    assert.deepEqual(content.controls, ["打开", "暂停", "更多"]);
    assert.equal(content.expandable, true);
    assert.equal(islandHoverEnter(anchors[1]), ids[1]);
    assert.equal(islandHoverEnter(anchor("event", "bare", 0)), undefined);
    assert.equal(islandHoverContent(undefined).expandable, false);
    // 展开再收起保留当前选中项：身份仍在就沿用，顺序也不变。
    const grown = islandArrive(anchors, { source: "late", layer: "event", nowMs: 9, unit: unit("pending", "新任务") });
    assert.equal(islandHoverSelect(grown, ids[2]), ids[2]);
    assert.deepEqual(
        grown.slice(0, 4).map((item) => item.id),
        ids,
    );
});

test("拖放接收态：松手图标与固定文案，没有匹配插件时显示无法接收", () => {
    assert.deepEqual(islandDropFace("collect-box"), { accept: true, icon: "release", primary: "松开加入收集箱" });
    assert.deepEqual(islandDropFace("music"), { accept: true, icon: "release", primary: "交给音乐播放" });
    assert.deepEqual(islandDropFace(undefined), { accept: false, icon: "release", primary: "无法接收" });
});

test("胶囊与 hover 共用显示投影：三种开关独立，选择与方向键跳过已撤下锚点", () => {
    const music = { ...anchor("event", "music"), kind: "music" } satisfies IslandAnchor;
    const message = anchor("message", "notifications");
    const plugin = { ...anchor("resident", "plugin"), kind: "plugin" } satisfies IslandAnchor;
    const confirm = anchor("confirm", "confirm");
    const anchors = [music, message, plugin, confirm];
    assert.deepEqual(islandDisplayAnchors(anchors, undefined), anchors);
    for (const [key, removed] of [["music", music], ["message", message], ["plugin", plugin]] as const) {
        const visible = islandDisplayAnchors(anchors, { islandAnchorVisible: { [key]: false } });
        assert.deepEqual(visible, anchors.filter((item) => item !== removed));
        assert.equal(islandHoverSelect(visible, removed.id), visible[0].id);
        assert.equal(islandHoverStep(visible, visible[0].id, 1), visible[1].id);
    }
    assert.deepEqual(islandDisplayAnchors(anchors, { islandAnchorVisible: { music: false, message: false, plugin: false } }), [confirm]);
    assert.equal(anchors.length, 4, "过滤只生成显示投影，不删除输入事实");
});

test("音乐锚点只在有媒体会话时出现：播放与暂停算会话，停止、无会话或未启用都不产生", () => {
    assert.equal(islandMusicAnchorVisible({ enabled: true, playback: "playing" }), true);
    assert.equal(islandMusicAnchorVisible({ enabled: true, playback: "paused" }), true);
    for (const playback of ["stopped", "unknown", undefined] as const) {
        for (const enabled of [true, false]) assert.equal(islandMusicAnchorVisible({ enabled, playback }), false);
    }
    assert.equal(islandMusicAnchorVisible({ enabled: false, playback: "playing" }), false);
    assert.equal(ISLAND_NOTICE_LAYER, "event"); // 音乐是活动型事件锚点。
});

test("天气变化通知：摘要变化投递一次，首次、缺失和未变化都不通知", () => {
    assert.equal(islandWeatherNotice(undefined, "晴 22°"), false);
    assert.equal(islandWeatherNotice("晴 22°", "晴 22°"), false);
    assert.equal(islandWeatherNotice("晴 22°", "多云 19°"), true);
    assert.equal(islandWeatherNotice("晴 22°", undefined), false);
});

test("通知保留时长只有一处，旧天气与收录提醒的独立定时已移除", () => {
    const model = readFileSync(new URL("./islandContent.ts", import.meta.url), "utf8");
    const messages = readFileSync(new URL("./islandMessages.ts", import.meta.url), "utf8");
    const renderer = readFileSync(new URL("./island.tsx", import.meta.url), "utf8");
    const css = readFileSync(new URL("./island.css", import.meta.url), "utf8");
    const constants = new Set([...model.matchAll(/\b(ISLAND_[A-Z_]*_MS)\b/g)].map((match) => match[1]));
    assert.deepEqual([...constants].sort(), ["ISLAND_ANCHOR_GROW_MS", "ISLAND_ANCHOR_SHRINK_MS", "ISLAND_MESSAGE_ANCHOR_TTL_MS"]);
    assert.equal([...`${model}\n${messages}\n${renderer}\n${css}`.matchAll(/\b30_000\b|\b30000\b/g)].length, 1);
    assert.match(renderer, /islandNotificationDeadline\(/);
    const cssDurations = new Set([...css.matchAll(/\b[0-9]+(?:\.[0-9]+)?ms\b/g)].map((match) => match[0]));
    assert.deepEqual([...cssDurations].sort(), ["0ms", "140ms", "150ms", "200ms", "80ms"]);
});

test("收集快照比较：只有新条目才给完成态，重命名/置顶/删除/无变化都不通知", () => {
    const entry = (name: string, createdUnixMs: number) => ({ name, createdUnixMs });
    const title = (item: { name: string }) => item.name; // 渲染层会把既有标题格式化函数传进来。
    const base = [entry("a.md", 1), entry("b.md", 2)];
    // 新增：数量增加且出现新身份（名称与创建时刻都不在旧快照），取创建时刻最新的一条。
    assert.deepEqual(islandCollectArrival(base, [...base, entry("c.md", 3)], title), { activity: "completed", title: "c.md" });
    assert.deepEqual(islandCollectArrival(base, [entry("c.md", 3), ...base], title), { activity: "completed", title: "c.md" }); // 置顶新条目也一样。
    // 重命名：名称变、创建时刻未变，不算新收录。
    assert.deepEqual(islandCollectArrival(base, [entry("a-改名.md", 1), entry("b.md", 2)], title), { activity: "idle" });
    // 置顶：只改顺序。
    assert.deepEqual(islandCollectArrival(base, [entry("b.md", 2), entry("a.md", 1)], title), { activity: "idle" });
    // 删除：数量减少。
    assert.deepEqual(islandCollectArrival(base, [entry("a.md", 1)], title), { activity: "idle" });
    // 无变化与首次读取都不通知。
    assert.deepEqual(islandCollectArrival(base, [...base], title), { activity: "idle" });
    assert.deepEqual(islandCollectArrival(undefined, base, title), { activity: "idle" });
    // 数量增加但没有新身份（旧条目重复出现）也不通知。
    assert.deepEqual(islandCollectArrival(base, [...base, entry("a.md", 1)], title), { activity: "idle" });
});

test("hover 提供者：音乐、消息、插件与默认身份分发，通用详情控件最多三个", () => {
    const kindAnchor = (kind: IslandAnchor["kind"]): IslandAnchor => ({ id: `a-${kind}`, source: "s", layer: "event", updatedAtMs: 1_000, kind, unit: unit("pending", "主信息", "补充") });
    const provider = (kind: IslandAnchor["kind"]) => islandHoverProvider(kindAnchor(kind));
    assert.equal(islandHoverProvider(undefined), "default");
    assert.equal(provider("music"), "music");
    assert.equal(provider("message"), "message");
    assert.equal(provider("plugin"), "plugin");
    assert.equal(provider("event"), "default");
    assert.deepEqual(islandHoverBody(kindAnchor("music"), ["上一首", "播放", "下一首", "删除"]), {
        provider: "music", lines: ["主信息", "补充"], controls: ["上一首", "播放", "下一首"], expandable: true,
    });
});

test("插件 hover 入口：采纳函数、拒绝非函数、清理安全、重复绑定保留首份", () => {
    assert.deepEqual(adoptPluginHover(undefined), {}); // 未提供入口不算错误。
    assert.deepEqual(adoptPluginHover("nope"), { reason: "插件 hover 入口不是函数" });
    const render = () => () => undefined;
    assert.equal(adoptPluginHover(render).render, render); // 只有函数被采纳。
    const container = {} as HTMLElement;
    // 返回清理函数：调用时执行。
    let cleaned = 0;
    const cleanup = mountPluginHover(() => () => { cleaned += 1; }, container, { pluginId: "p" });
    cleanup();
    cleanup();
    assert.equal(cleaned, 2);
    // 入口抛错：只回报，调用方拿到的仍是可安全调用的清理函数。
    const failures: unknown[] = [];
    const safe = mountPluginHover(() => { throw new Error("boom"); }, container, { pluginId: "p" }, (error) => failures.push(error));
    assert.equal(typeof safe, "function");
    assert.doesNotThrow(() => safe());
    assert.equal(failures.length, 1);
    // 返回值不是函数：忽略脏值，不把数字当清理函数。
    const dirty = mountPluginHover((() => 42) as unknown as () => void, container, { pluginId: "p" });
    assert.equal(typeof dirty, "function");
    assert.doesNotThrow(() => dirty());
    // 同一标识只保留首份外壳。
    const registry = new Map<string, { name: string }>();
    assert.equal(adoptPluginShellOnce(registry, "p", { name: "首份" }), true);
    assert.equal(adoptPluginShellOnce(registry, "p", { name: "第二次" }), false);
    assert.equal(registry.get("p")?.name, "首份");
});

test("内容模型不依赖 React、不持有定时器，渲染层可直接消费", () => {
    for (const file of ["./islandContent.ts", "./islandMessages.ts"]) {
        const source = readFileSync(new URL(file, import.meta.url), "utf8");
        assert.doesNotMatch(source, /from "react"|setTimeout|setInterval|requestAnimationFrame/);
    }
});
