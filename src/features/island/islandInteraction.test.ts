import assert from "node:assert/strict";
import test from "node:test";
import {
    ISLAND_HOVER_DELAY_MS,
    ISLAND_NO_DRAG,
    islandDragIntent,
    islandSortTarget,
    islandDragMoved,
    islandAfterDrag,
    cancelIslandHover,
    islandHoverTarget,
} from "./islandInteraction.ts";
import { moveIslandCard } from "./islandGeometry.ts";

const order = ["official.music-control", "plugin.one", "plugin.two"];
test("常态与hover的歌词、封面和卡片空白均走窗口拖动", () => {
    for (const mode of ["collapsed", "hover"] as const) {
        for (const region of ["歌词", "媒体图", "卡片空白"]) assert.equal(islandDragIntent(mode, 0, false, true), "window", region);
        assert.equal(islandDragIntent(mode, 0, true, true), undefined);
        assert.equal(islandDragIntent(mode, 2, false, true), undefined);
        assert.equal(islandDragIntent(mode, 0, false, false), undefined);
    }
    assert.equal(ISLAND_NO_DRAG.includes("island-music__line"), false);
    assert.equal(ISLAND_NO_DRAG.includes("island-music__artwork"), false);
    for (const selector of ["button", "[data-plugin-container]", "[data-island-plugin]", "[data-island-no-drag]"]) assert.ok(ISLAND_NO_DRAG.includes(selector));
    assert.equal(ISLAND_HOVER_DELAY_MS, 180);
});
test("hover单击与拖动只按4 DIP位移区分，不按时间", () => {
    const start = { x: 100, y: 80 };
    assert.equal(islandDragMoved(start, start), false);
    assert.equal(islandDragMoved(start, { x: 104, y: 80 }), false);
    assert.equal(islandDragMoved(start, { x: 103, y: 82 }), false);
    assert.equal(islandDragMoved(start, { x: 104.01, y: 80 }), true);
    assert.equal(islandDragMoved(start, { x: 103, y: 83 }), true);
});
test("双向hover延时在反向边界、拖动或展开后取消原目标", () => {
    assert.equal(islandHoverTarget("collapsed", true, false), "hover");
    assert.equal(islandHoverTarget("collapsed", false, false), undefined);
    assert.equal(islandHoverTarget("hover", false, false), "collapsed");
    assert.equal(islandHoverTarget("hover", true, false), undefined);
    assert.equal(islandHoverTarget("hover", false, true), undefined);
    assert.equal(islandHoverTarget("expanded", false, false), undefined);
});
test("原生释放事实重置状态前取消旧定时器，岛外不会被旧回调重新拉起", (context) => {
    context.mock.timers.enable({ apis: ["setTimeout"] });
    for (const pointerInside of [true, false]) {
        let mode: "hover" | "collapsed" = "hover";
        const timer = {
            current: setTimeout(() => {
                mode = "hover";
            }, ISLAND_HOVER_DELAY_MS) as ReturnType<typeof setTimeout> | undefined,
        };
        cancelIslandHover(timer);
        mode = islandAfterDrag(pointerInside);
        context.mock.timers.tick(ISLAND_HOVER_DELAY_MS * 2);
        assert.equal(timer.current, undefined);
        assert.equal(mode, pointerInside ? "hover" : "collapsed");
        assert.equal(islandHoverTarget(mode, !pointerInside, true), undefined); // 原生拖动仍在进行时不切态。
        assert.equal(islandHoverTarget(mode, !pointerInside, false), pointerInside ? "collapsed" : "hover"); // 释放后普通移入移出恢复消抖。
    }
});

test("原生拖动释放后按最新命中结果重判，不沿用开始态", () => {
    assert.equal(islandAfterDrag(true), "hover");
    assert.equal(islandAfterDrag(false), "collapsed");
});
test("展开态卡片本体只排序，不启动窗口拖动", () => {
    assert.equal(islandDragIntent("expanded", 0, false, true), "sort");
    assert.equal(islandDragIntent("expanded", 0, true, true), undefined);
    assert.equal(islandDragIntent("expanded", 1, false, true), undefined);
    assert.equal(islandDragIntent("expanded", 0, false, false), undefined);
});
test("无需句柄，合法卡片落点沿同一身份列表排序", () => {
    const target = islandSortTarget("expanded", order, order[0], order[2]!, false);
    assert.equal(target, "plugin.two");
    assert.deepEqual(moveIslandCard(order, order[0]!, target!), ["plugin.one", "plugin.two", "official.music-control"]);
    for (const mode of ["collapsed", "hover"] as const) assert.equal(islandSortTarget(mode, order, order[0], order[2]!, false), undefined);
    for (const [from, to, blocked] of [
        [undefined, "plugin.one", false],
        ["external", "plugin.one", false],
        [order[0], "external", false],
        [order[0], order[0], false],
        [order[0], "plugin.one", true],
    ] as const)
        assert.equal(islandSortTarget("expanded", order, from, to!, blocked), undefined);
});
