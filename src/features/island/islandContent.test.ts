import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import {
    ISLAND_ANCHOR_GAP,
    ISLAND_ANCHOR_GROW_MS,
    ISLAND_ANCHOR_SHRINK_MS,
    ISLAND_ANCHOR_SIZE,
    ISLAND_ANCHOR_SLOTS,
    ISLAND_HIDDEN_ANCHOR_SOURCE,
    ISLAND_HOVER_CONTROL_LIMIT,
    ISLAND_HOVER_LINE_LIMIT,
    ISLAND_LAYER_ORDER,
    ISLAND_MESSAGE_ANCHOR_TTL_MS,
    ISLAND_STATUS_SPECS,
    islandAnchorDeadline,
    islandAnchorGlyph,
    islandAnchorId,
    islandAnchorRow,
    islandAnchorValue,
    islandArrive,
    islandCapsuleFace,
    islandClockLabel,
    islandDropFace,
    islandExpireAnchors,
    islandHoverContent,
    islandHoverEnter,
    islandHoverSelect,
    islandHoverStep,
    islandLayerRank,
    islandMergeHiddenAnchors,
    islandOverlayAction,
    islandStatusProgress,
    islandStatusSpec,
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

test("叠加分层：同来源滚动、不同来源等当前结束、更高层级立即切换", () => {
    assert.deepEqual([...ISLAND_LAYER_ORDER], ["drop", "confirm", "message", "event", "resident"]);
    assert.ok(islandLayerRank("drop") < islandLayerRank("confirm"));
    assert.ok(islandLayerRank("confirm") < islandLayerRank("message"));
    assert.ok(islandLayerRank("message") < islandLayerRank("event"));
    assert.ok(islandLayerRank("event") < islandLayerRank("resident"));
    const resident = anchor("resident", "music", 0, unit("running", "正在播放"));
    assert.equal(islandOverlayAction(resident, { source: "music", layer: "resident", nowMs: 1, unit: unit("running", "下一首") }), "scroll");
    assert.equal(islandOverlayAction(resident, { source: "toast", layer: "message", nowMs: 1, unit: unit("pending", "新消息") }), "switch");
    const message = anchor("message", "toast", 0, unit("pending", "新消息"));
    assert.equal(islandOverlayAction(message, { source: "collect", layer: "event", nowMs: 1, unit: unit("running", "复制中") }), "append");
    assert.equal(islandOverlayAction(message, { source: "confirm", layer: "confirm", nowMs: 1, unit: unit("awaiting", "允许？") }), "switch");
    assert.equal(islandOverlayAction(message, { source: "drop", layer: "drop", nowMs: 1 }), "switch");
    assert.equal(islandOverlayAction(undefined, { source: "drop", layer: "drop", nowMs: 1 }), "switch");
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

test("未读超时：普通消息保留 30s，确认与失败不超时，hover 与展开期间不消失", () => {
    const message = anchor("message", "toast", 1_000, unit("pending", "已同步 3 个文件"));
    assert.equal(islandAnchorDeadline(message), 31_000);
    assert.equal(islandExpireAnchors([message], 30_999, "collapsed").length, 1);
    assert.deepEqual(islandExpireAnchors([message], 31_000, "collapsed"), []);
    assert.equal(islandExpireAnchors([message], 90_000, "hover").length, 1);
    assert.equal(islandExpireAnchors([message], 90_000, "expanded").length, 1);
    const awaiting = anchor("confirm", "pair", 0, unit("awaiting", "允许配对吧？"));
    const failed = anchor("event", "upload", 0, unit("failed", "上传失败"));
    assert.equal(islandAnchorDeadline(awaiting), undefined);
    assert.equal(islandAnchorDeadline(failed), undefined);
    assert.equal(islandAnchorDeadline(anchor("message", "toast", 0, unit("failed", "发送失败"))), undefined);
    assert.equal(islandExpireAnchors([awaiting, failed], 10 * 60_000, "collapsed").length, 2);
    assert.equal(islandAnchorDeadline(anchor("resident", "music", 0, unit("running", "正在播放"))), undefined);
});

test("系统隐藏恢复：隐藏期间的消息合并成一个计数锚点，已过期的丢弃", () => {
    const now = 100_000;
    const hidden = [
        { source: "a", nowMs: now - 1_000, unit: unit("pending", "第一条") },
        { source: "b", nowMs: now - 2_000, unit: unit("pending", "第二条") },
        { source: "c", nowMs: now - ISLAND_MESSAGE_ANCHOR_TTL_MS, unit: unit("pending", "已过期") },
    ];
    const merged = islandMergeHiddenAnchors([], hidden, now);
    assert.equal(merged.length, 1);
    assert.equal(merged[0]!.count, 2);
    assert.equal(merged[0]!.unit, undefined); // 不逐个补播内容。
    assert.equal(merged[0]!.layer, "message");
    assert.equal(merged[0]!.source, ISLAND_HIDDEN_ANCHOR_SOURCE);
    assert.equal(islandMergeHiddenAnchors([], [hidden[2]!], now).length, 0); // 全部过期时不留锚点。
    const many = islandMergeHiddenAnchors(
        [],
        Array.from({ length: 120 }, (_, index) => ({ source: `s${index}`, nowMs: now, unit: unit("pending", "x") })),
        now,
    );
    assert.equal(many[0]!.count, 99); // 计数封顶 99，渲染层显示「99+」。
    assert.equal(islandExpireAnchors(merged, now + ISLAND_MESSAGE_ANCHOR_TTL_MS, "collapsed").length, 0);
});

test("拖放接收态：松手图标与固定文案，没有匹配插件时显示无法接收", () => {
    assert.deepEqual(islandDropFace("collect-box"), { accept: true, icon: "release", primary: "松开加入收集箱" });
    assert.deepEqual(islandDropFace("music"), { accept: true, icon: "release", primary: "交给音乐播放" });
    assert.deepEqual(islandDropFace(undefined), { accept: false, icon: "release", primary: "无法接收" });
});

test("内容模型不依赖 React、不持有定时器，渲染层可直接消费", () => {
    const source = readFileSync(new URL("./islandContent.ts", import.meta.url), "utf8");
    assert.doesNotMatch(source, /from "react"/);
    assert.doesNotMatch(source, /setTimeout|setInterval|requestAnimationFrame/);
    assert.match(source, /from "\.\.\/\.\.\/contracts\/island\.ts"/); // 只做类型引用。
});
