import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { parseIslandSnapshot, parseIslandStateRequest, parseIslandDragResult } from "./island.ts";

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

test("拖动命令只接受严格的camelCase布尔命中结果", () => {
    assert.deepEqual(parseIslandDragResult({ pointerInside: true }), { pointerInside: true });
    assert.deepEqual(parseIslandDragResult({ pointerInside: false }), { pointerInside: false });
    for (const value of [
        undefined,
        null,
        false,
        true,
        [],
        {},
        { pointer_inside: true },
        { pointerInside: 1 },
        { pointerInside: "false" },
        { pointerInside: null },
        { pointerInside: true, other: false },
    ])
        assert.throws(() => parseIslandDragResult(value));
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
    cards: [{ rect: { left: 0, top: 0, width: 200, height: 44 }, radius: 22 }],
}; // 合法常态屏幕 DIP。
test("三态请求与原生几何边界一致，拒绝非有限值、越界和非法数量", () => {
    for (const state of ["collapsed", "hover", "expanded"]) assert.deepEqual(parseIslandStateRequest({ ...request, state }), { ...request, state });
    for (const patch of [
        { state: "other" },
        { cards: [] },
        { cards: [request.cards[0], request.cards[0]] },
        { windowBox: { ...request.windowBox, left: Infinity } },
        { windowBox: { ...request.windowBox, width: 0 } },
        { windowBox: { ...request.windowBox, top: 1_000_001 } },
        { cards: [{ ...request.cards[0], radius: 23 }] },
        { cards: [{ ...request.cards[0], radius: NaN }] },
        { cards: [{ rect: { ...request.cards[0].rect, left: 1 }, radius: 22 }] },
        { unknown: true },
    ])
        assert.throws(() => parseIslandStateRequest({ ...request, ...patch }));
    assert.equal(parseIslandStateRequest({ ...request, state: "expanded", cards: [request.cards[0], request.cards[0]] }).cards.length, 2);
    assert.throws(() => parseIslandStateRequest({ ...request, state: "expanded", cards: Array(129).fill(request.cards[0]) }));
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
