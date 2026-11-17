import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { parseIslandSnapshot, parseIslandStateRequest, parseIslandDragResult, parseIslandDropFact } from "./island.ts";

const instance = {
    instanceId: "island.plugin.clock",
    componentId: "plugin.clock",
    monitorId: "",
    projectionMonitorId: null,
    projectionBounds: null,
    bounds: { left: 0, top: 0, width: 0, height: 0 },
    stateVersion: 1,
    state: {},
}; // 后端合成实例形状。
const registration = {
    instanceId: instance.instanceId,
    componentId: instance.componentId,
    entry: "plugin:slot",
    authorizedCapabilities: ["read-host-time"],
    status: "ready",
    error: null,
}; // 不进入桌面注册集合。
const snapshot = {
    instances: [instance],
    runtime: {
        revision: 1,
        manifests: [
            {
                id: "plugin.clock",
                name: "时钟",
                description: "岛卡",
                entry: "plugin:slot",
                script: "clock/main.js",
                stylesheet: "",
                requiredCapabilities: ["read-host-time"],
                removable: true,
                island: true,
                persistenceVersion: 1,
            },
        ],
        instances: [registration],
        failures: [],
    },
};

test("拖动命令只接受严格命中事实与新偏移，缺失偏移不回退到旧值", () => {
    assert.deepEqual(parseIslandDragResult({ pointerInside: true, offset: 12.5 }), { pointerInside: true, offset: 12.5 });
    assert.deepEqual(parseIslandDragResult({ pointerInside: false, offset: null }), { pointerInside: false, offset: null });
    for (const value of [
        undefined,
        null,
        false,
        true,
        [],
        {},
        { pointerInside: true },
        { pointer_inside: true },
        { pointerInside: 1, offset: null },
        { pointerInside: "false", offset: null },
        { pointerInside: null, offset: null },
        { pointerInside: true, offset: "1" },
        { pointerInside: true, offset: Number.NaN },
        { pointerInside: true, offset: Number.POSITIVE_INFINITY },
        { pointerInside: true, offset: 1, other: false },
    ])
        assert.throws(() => parseIslandDragResult(value));
});

test("投放事实带类型并严格解析：未知字段、未知 kind 与非布尔 active 都拒绝", () => {
    assert.deepEqual(parseIslandDropFact({ active: true, kind: "collect" }), { active: true, kind: "collect" });
    assert.deepEqual(parseIslandDropFact({ active: false, kind: "reject" }), { active: false, kind: "reject" }); // 离开/放下的回落事实。
    for (const value of [
        undefined,
        null,
        true,
        false,
        [],
        {},
        { active: true },
        { kind: "collect" },
        { active: true, kind: "music" }, // 音乐投放未实现，未知 kind 一律拒绝。
        { active: "true", kind: "collect" },
        { active: 1, kind: "reject" },
        { active: null, kind: "reject" },
        { active: true, kind: "COLLECT" },
        { active: true, kind: "collect", extra: 1 },
    ])
        assert.throws(() => parseIslandDropFact(value));
    const contract = readFileSync(new URL("./island.ts", import.meta.url), "utf8");
    assert.match(contract, /kind: "collect" \| "reject"/); // 事实类型只有收集箱与不可接收。
    assert.doesNotMatch(contract, /from "\.\.\/features\//); // 契约不得反向依赖功能目录。
    assert.match(readFileSync(new URL("../bridge/island.ts", import.meta.url), "utf8"), /parseIslandDropFact/); // 桥用同一解析，坏载荷不进入页面。
});

test("岛快照复用清单与注册契约，保留零布局实例及顺序", () => {
    const parsed = parseIslandSnapshot(snapshot);
    assert.deepEqual(parsed.instances, [instance]);
    assert.equal(parsed.runtime.manifests[0].island, true);
    assert.equal(parsed.runtime.manifests[0].description, "岛卡");
    assert.equal(parsed.runtime.manifests[0].script, "clock/main.js");
    for (const patch of [
        { instanceId: "desktop.clock" },
        { monitorId: "main" },
        { bounds: { left: 0, top: 0, width: 80, height: 48 } },
        { componentId: "plugin.other" },
        { stateVersion: 2 },
    ])
        assert.throws(() => parseIslandSnapshot({ ...snapshot, instances: [{ ...instance, ...patch }] }));
    assert.throws(() => parseIslandSnapshot({ ...snapshot, instances: [instance, instance] }));
    assert.throws(() => parseIslandSnapshot({ ...snapshot, runtime: { ...snapshot.runtime, instances: [] } }));
    assert.throws(() =>
        parseIslandSnapshot({ ...snapshot, runtime: { ...snapshot.runtime, manifests: [{ ...snapshot.runtime.manifests[0], island: false }] } }),
    );
    assert.deepEqual(parseIslandSnapshot({ instances: [], runtime: { revision: 0, manifests: [], instances: [], failures: [] } }).instances, []);
});

const request = {
    state: "collapsed",
    windowBox: { left: -100, top: 40, width: 200, height: 44 },
    viewport: { left: -112, top: 28, width: 224, height: 68 },
    cards: [{ rect: { left: 0, top: 0, width: 200, height: 44 }, radius: 22 }],
}; // 合法常态屏幕 DIP 与固定视口。
test("三态请求与原生几何边界一致，拒绝非有限值、越界和非法数量", () => {
    for (const state of ["collapsed", "hover", "expanded"]) assert.deepEqual(parseIslandStateRequest({ ...request, state }), { ...request, state });
    for (const patch of [
        { state: "other" },
        { cards: [] },
        { windowBox: { ...request.windowBox, left: Infinity } },
        { windowBox: { ...request.windowBox, width: 0 } },
        { windowBox: { ...request.windowBox, top: 1_000_001 } },
        { windowBox: { ...request.windowBox, left: -120 } },
        { viewport: { ...request.viewport, width: 0 } },
        { viewport: { ...request.viewport, width: 20_000 } },
        { viewport: { ...request.viewport, top: 100 } },
        { cards: [{ ...request.cards[0], radius: 23 }] },
        { cards: [{ ...request.cards[0], radius: NaN }] },
        { cards: [{ rect: { ...request.cards[0].rect, left: 1 }, radius: 22 }] },
        { unknown: true },
    ])
        assert.throws(() => parseIslandStateRequest({ ...request, ...patch }));
    assert.equal(parseIslandStateRequest({ ...request, state: "expanded", cards: [request.cards[0], request.cards[0]] }).cards.length, 2);
    assert.throws(() => parseIslandStateRequest({ ...request, state: "expanded", cards: Array(129).fill(request.cards[0]) }));
});

/** 28 × 28 圆形锚点区域，位置即窗口局部坐标。 */
const anchorRegion = (left: number, top: number) => ({ rect: { left, top, width: 28, height: 28 }, radius: 14 });
const anchorRow = [0, 1, 2].map((index) => anchorRegion(8 + index * 40, 8)); // 三枚并排，间距 12。
test("常态必须恰好一个胶囊区域，锚点行只由常态提交并按圆形与窗口盒校验", () => {
    // 常态只提交一张胶囊卡片：多张会把原生拖动中心与窗口夹紧算到错误的区域上。
    assert.equal(parseIslandStateRequest(request).cards.length, 1);
    assert.equal(parseIslandStateRequest(request).anchors, undefined); // 未提交锚点时不补默认值，过渡帧形状不变。
    for (const count of [2, 3, 4]) {
        const cards = Array.from({ length: count }, (_, index) => ({ rect: { left: index * 40, top: 0, width: 40, height: 44 }, radius: 20 }));
        assert.throws(() => parseIslandStateRequest({ ...request, state: "collapsed", cards }), /常态必须恰好一个胶囊区域/);
    }
    // hover 与展开仍是多区域：内容卡与展开卡片语义不变。
    const hover = {
        state: "hover",
        windowBox: { left: -100, top: 28, width: 200, height: 68 },
        viewport: { left: -112, top: 28, width: 224, height: 68 },
        cards: [{ rect: { left: 0, top: 36, width: 200, height: 32 }, radius: 16 }],
    }; // hover 的内容卡；悬停锚点列表由渲染层绘制，不提交几何。
    assert.equal(parseIslandStateRequest(hover).cards.length, 1);
    assert.equal(parseIslandStateRequest({ ...request, state: "expanded", cards: [request.cards[0], request.cards[0]] }).cards.length, 2);
    // 常态锚点行：3 枚并排 + 1 枚「＋N」折叠圆是上限，第 5 枚拒绝。
    const collapsedAnchors = { ...request, anchors: anchorRow };
    assert.deepEqual(parseIslandStateRequest(collapsedAnchors).anchors, anchorRow);
    assert.equal(parseIslandStateRequest({ ...collapsedAnchors, anchors: [...anchorRow, anchorRegion(128, 8)] }).anchors?.length, 4);
    for (const patch of [
        { anchors: [...anchorRow, anchorRegion(128, 8), anchorRegion(168, 8)] },
        { anchors: [{ rect: { left: 180, top: 8, width: 28, height: 28 }, radius: 14 }] },
        { anchors: [{ rect: { left: 8, top: 32, width: 28, height: 28 }, radius: 14 }] },
        { anchors: [{ rect: { left: 8, top: 8, width: 28, height: 20 }, radius: 14 }] },
        { anchors: [{ rect: { left: 8, top: 8, width: 28, height: 28 }, radius: 12 }] },
        { anchors: [{ rect: { left: 8, top: 8, width: 28, height: 28 }, radius: 14, extra: 1 }] },
        { anchors: [{ rect: { left: 8, top: 8, width: 28, height: 28 } }] },
    ])
        assert.throws(() => parseIslandStateRequest({ ...collapsedAnchors, ...patch }));
    // 锚点行只由常态提交：悬停与展开携带锚点一律拒绝，避免契约与渲染层实测不一致。
    for (const state of ["hover", "expanded"]) {
        assert.throws(() => parseIslandStateRequest({ ...request, state, anchors: anchorRow }), /锚点行只由常态提交/);
        assert.equal(parseIslandStateRequest({ ...request, state }).anchors, undefined);
    }
});

test("锚点契约与原生共用同一上限、常态门禁与文案", () => {
    const contract = readFileSync(new URL("./island.ts", import.meta.url), "utf8");
    const native = readFileSync(new URL("../../src-tauri/src/platform/windows/island.rs", import.meta.url), "utf8");
    assert.match(contract, /ISLAND_ANCHOR_REGION_LIMIT = 4/);
    assert.match(native, /ISLAND_ANCHOR_REGION_LIMIT: usize = 4/);
    for (const message of ["岛卡片数量无效", "岛锚点数量无效", "岛锚点区域或圆形无效", "常态必须恰好一个胶囊区域", "锚点行只由常态提交"])
        assert.ok(native.includes(message) && contract.includes(message), message);
    // 常态门禁两端同形：TS 用收窄后的 state 与 cards，Rust 用 self.state 与 cards.len()。
    assert.match(contract, /state === "collapsed" && cards\.length !== 1/);
    assert.match(native, /self\.state == "collapsed" && self\.cards\.len\(\) != 1/);
    assert.doesNotMatch(contract, /不再要求「恰好一张」/);
    assert.doesNotMatch(native, /不再要求「恰好一张」/);
    assert.doesNotMatch(native, /常态岛必须只有一个胶囊区域/); // 旧文案已由显式门禁取代。
    assert.match(native, /#\[serde\(default\)\]\s+pub anchors/);
});

test("岛与桌面入口并列，桥仅复用既有通知并且读取不发布快照", () => {
    const app = readFileSync(new URL("../app/App.tsx", import.meta.url), "utf8"); // 路由不把岛嵌进工作区。
    assert.match(app, /query.get\("island"\) === "1"/);
    const main = readFileSync(new URL("../main.tsx", import.meta.url), "utf8");
    assert.match(main, /isDesktopHost \|\| isIsland/);
    const bridge = readFileSync(new URL("../bridge/island.ts", import.meta.url), "utf8");
    assert.match(bridge, /COMPONENT_RUNTIME_SNAPSHOT_CHANGED_EVENT, "preferences-changed"/);
    assert.doesNotMatch(bridge, /setInterval|setTimeout/);
    assert.match(bridge, /getPreferences\(\)/);
});
