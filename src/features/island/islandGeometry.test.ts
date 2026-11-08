import assert from "node:assert/strict";
import test from "node:test";
import { islandCornerRadius, islandGeometry, moveIslandCard, visibleIslandRect } from "./islandGeometry.ts";
import { parseIslandStateRequest } from "../../contracts/island.ts";

const work = { left: 0, top: 40, width: 1920, height: 1040 }; // 顶部任务栏工作区。
const card = { left: 0, top: 0, width: 320, height: 240 };
test("四边贴边固定，沿边居中并应用持久偏移，偏移夹紧到居中半幅", () => {
    for (const [edge, left, top] of [
        ["top", 900, 46],
        ["bottom", 900, 834],
        ["left", 6, 540],
        ["right", 1594, 540],
    ] as const) {
        const request = islandGeometry("expanded", [card], 12, work, edge, 100);
        assert.deepEqual(request.windowBox, { left, top, width: 320, height: 240 });
        assert.deepEqual(parseIslandStateRequest(request), request);
    }
    assert.equal(islandGeometry("hover", [card], 16, work, "right", 2000).windowBox.top, 840);
    assert.equal(islandGeometry("collapsed", [card], 22, work, "", 999).windowBox.left, 1600); // 空边按顶部应用偏移。
    assert.equal(islandGeometry("collapsed", [card], 22, work, "", 0).windowBox.left, 800); // 空边偏移 0 仍居中。
});

test("三态尺寸变化时贴边侧固定，沿边以中心对称扩展", () => {
    const collapsed = islandGeometry("collapsed", [{ ...card, width: 44, height: 44 }], 22, work, "top", 0);
    const expanded = islandGeometry("expanded", [card], 12, work, "top", 0);
    const center = (rect: { left: number; width: number }) => rect.left + rect.width / 2;
    assert.equal(center(collapsed.windowBox), center(expanded.windowBox)); // 水平中心不动。
    assert.equal(collapsed.windowBox.top, expanded.windowBox.top); // 顶边固定。
    assert.ok(expanded.windowBox.top + expanded.windowBox.height > collapsed.windowBox.top + collapsed.windowBox.height); // 向屏幕中心方向长。
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
    assert.deepEqual(request.windowBox, { left: -1914, top: 412, width: 44, height: 176 });
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

test("刘海窗口沿贴边方向外扩 12 DIP，卡片本体保持贴边并向内让出圆弧区", () => {
    const size = { left: 0, top: 0, width: 200, height: 60 };
    for (const [edge, left, top, width, height, cardLeft, cardTop] of [
        ["top", 848, 40, 224, 60, 12, 0],
        ["bottom", 848, 1020, 224, 60, 12, 0],
        ["left", 0, 518, 200, 84, 0, 12],
        ["right", 1720, 518, 200, 84, 0, 12],
    ] as const) {
        const notch = islandGeometry("collapsed", [size], 22, work, edge, 0, "notch");
        assert.deepEqual(notch.windowBox, { left, top, width, height }); // 只在沿边方向外扩，贴边方向仍为零间距。
        assert.deepEqual(notch.cards[0]!.rect, { left: cardLeft, top: cardTop, width: 200, height: 60 });
    }
    const capsule = islandGeometry("collapsed", [size], 22, work, "top", 0);
    assert.equal(capsule.windowBox.top, 46); // 胶囊离边 6。
});

test("刘海展开态不再外扩窗口，卡片仍按岛圆角绘制", () => {
    const size = { left: 0, top: 0, width: 320, height: 240 };
    const expanded = islandGeometry("expanded", [size], 12, work, "top", 0, "notch");
    assert.deepEqual(expanded.windowBox, { left: 800, top: 40, width: 320, height: 240 });
    assert.deepEqual(expanded.cards[0]!.rect, { left: 0, top: 0, width: 320, height: 240 });
    assert.equal(islandCornerRadius(12, "notch", "top", true), "12px");
});

test("刘海把贴边侧两角切成直角，圆弧由窗口角部补面绘制，展开态统一", () => {
    assert.equal(islandCornerRadius(22, "capsule", "top"), "22px");
    assert.equal(islandCornerRadius(22, "notch", "top"), "0 0 22px 22px");
    assert.equal(islandCornerRadius(22, "notch", "bottom"), "22px 22px 0 0");
    assert.equal(islandCornerRadius(22, "notch", "left"), "0 22px 22px 0");
    assert.equal(islandCornerRadius(22, "notch", "right"), "22px 0 0 22px");
    assert.equal(islandCornerRadius(22, "notch", "top", true), "22px"); // 展开态统一，不区分贴边方向。
});
