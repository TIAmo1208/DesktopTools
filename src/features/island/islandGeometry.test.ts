import assert from "node:assert/strict";
import test from "node:test";
import { islandGeometry, moveIslandCard, visibleIslandRect } from "./islandGeometry.ts";
import { parseIslandStateRequest } from "../../contracts/island.ts";

const work = { left: 0, top: 40, width: 1920, height: 1040 }; // 顶部任务栏工作区。
const card = { left: 0, top: 0, width: 320, height: 240 };
test("四边展开向内，沿边锚点不变并且偏移夹紧", () => {
    for (const [edge, left, top] of [
        ["top", 100, 40],
        ["bottom", 100, 840],
        ["left", 0, 140],
        ["right", 1600, 140],
    ] as const) {
        const request = islandGeometry("expanded", [card], 12, work, edge, 100);
        assert.deepEqual(request.windowBox, { left, top, width: 320, height: 240 });
        assert.deepEqual(parseIslandStateRequest(request), request);
    }
    assert.equal(islandGeometry("hover", [card], 16, work, "right", 2000).windowBox.top, 840);
    assert.equal(islandGeometry("collapsed", [card], 22, work, "", 999).windowBox.left, 800);
});
test("实测统一宽度、各自高度，间隙透明但仍归岛命中", () => {
    const request = islandGeometry("expanded", [card, { ...card, top: 248, height: 120 }], 12, work, "bottom", 0);
    assert.equal(request.windowBox.height, 368);
    assert.deepEqual(request.cards[2], { rect: { left: 0, top: 240, width: 320, height: 8 }, radius: 0 });
    assert.deepEqual(parseIslandStateRequest(request), request);
    assert.throws(() => islandGeometry("expanded", [{ ...card, height: 2000 }], 12, work, "top", 0));
});
test("负坐标工作区、竖胶囊和圆角上限", () => {
    const request = islandGeometry(
        "collapsed",
        [{ left: 0, top: 0, width: 44, height: 176 }],
        22,
        { left: -1920, top: -40, width: 1920, height: 1080 },
        "left",
        0,
    );
    assert.deepEqual(request.windowBox, { left: -1920, top: -40, width: 44, height: 176 });
    assert.equal(request.cards[0]!.radius, 22);
    assert.deepEqual(parseIslandStateRequest(request), request);
});
test("滚动视口裁剪卡片，不向原生提交越界区域", () => {
    assert.deepEqual(visibleIslandRect({ ...card, top: -20 }, 200), [{ ...card, top: 0, height: 200 }]);
    assert.deepEqual(visibleIslandRect({ ...card, top: 180 }, 200), [{ ...card, top: 180, height: 20 }]);
    assert.deepEqual(visibleIslandRect({ ...card, top: 200 }, 200), []);
});

test("卡片顺序只移动已有功能身份，不生成第二份列表", () => {
    const order = ["official.music-control", "plugin.one", "plugin.two"];
    assert.deepEqual(moveIslandCard(order, "official.music-control", "plugin.two"), ["plugin.one", "plugin.two", "official.music-control"]);
    assert.deepEqual(moveIslandCard(order, "unknown", "plugin.two"), order);
    assert.deepEqual(moveIslandCard(order, "plugin.one", "plugin.one"), order);
    assert.deepEqual(order, ["official.music-control", "plugin.one", "plugin.two"]);
});
