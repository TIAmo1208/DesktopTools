import assert from "node:assert/strict";
import test from "node:test";
import { islandCardMaskPath, islandCornerRadius, islandSeamMask, islandFilletPatches, islandGeometry, moveIslandCard, visibleIslandRect } from "./islandGeometry.ts";
import { parseIslandStateRequest } from "../../contracts/island.ts";

// 测试独立展开M/L/A/Z路径为细分多边形，不依赖生产的几何命中判断，避免把集合包含写成自证。
const pathPolygons = (path: string): [number, number][][] => {
    const tokens = path.match(/[MLAZ]|-?\d+(?:\.\d+)?(?:e[+-]?\d+)?/gi)!;
    const polygons: [number, number][][] = [];
    let index = 0, x = 0, y = 0, points: [number, number][] = [];
    const number = () => Number(tokens[index++]);
    while (index < tokens.length) {
        const command = tokens[index++];
        if (command === "M") { points = []; x = number(); y = number(); points.push([x, y]); }
        else if (command === "L") { x = number(); y = number(); points.push([x, y]); }
        else if (command === "Z") polygons.push(points);
        else if (command === "A") {
            const radius = number(); assert.equal(number(), radius); number(); number();
            const sweep = number(), nx = number(), ny = number();
            const dx = nx - x, dy = ny - y, length = Math.hypot(dx, dy), h = Math.sqrt(Math.max(0, radius ** 2 - length ** 2 / 4));
            let found = false;
            for (const sign of [-1, 1]) {
                const cx = (x + nx) / 2 - sign * dy * h / length, cy = (y + ny) / 2 + sign * dx * h / length;
                const start = Math.atan2(y - cy, x - cx);
                let delta = Math.atan2(ny - cy, nx - cx) - start;
                if (sweep && delta < 0) delta += Math.PI * 2;
                if (!sweep && delta > 0) delta -= Math.PI * 2;
                if (Math.abs(delta) > Math.PI + 1e-8) continue;
                for (let step = 1; step <= 64; step++) points.push([cx + radius * Math.cos(start + delta * step / 64), cy + radius * Math.sin(start + delta * step / 64)]);
                found = true; break;
            }
            assert.ok(found); x = nx; y = ny;
        } else assert.fail(`未知路径指令 ${command}`);
    }
    return polygons;
};
const polygonContains = (polygon: [number, number][], x: number, y: number) => {
    let inside = false;
    for (let i = 0, j = polygon.length - 1; i < polygon.length; j = i++) {
        const a = polygon[i]!, b = polygon[j]!;
        if ((a[1] > y) !== (b[1] > y) && x < (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0]) inside = !inside;
    }
    return inside;
};
test("单路径补采样为理想轮廓与接缝带的交集，四边两形态四档DPR无越界覆盖", () => {
    const card = { left: 20.125, top: 20.375, width: 160.25, height: 80.5 }, box = { left: 0, top: 0, width: 210, height: 140 };
    const inRect = (x: number, y: number, rect: typeof card) => x >= rect.left && y >= rect.top && x <= rect.left + rect.width && y <= rect.top + rect.height;
    for (const form of ["capsule", "notch"] as const) for (const edge of ["top", "bottom", "left", "right"] as const) for (const dpr of [1, 1.25, 1.5, 1.75]) {
        const radii = islandCornerRadius(16, form, edge, -12).split(' ').map(parseFloat);
        const mask = islandSeamMask(card, radii, box, edge, form, -12, dpr);
        if (form === "capsule") { assert.equal(mask, undefined); continue; }
        assert.ok(mask); assert.equal(mask.bands.length, 2);
        assert.ok(mask.path.startsWith(islandCardMaskPath(card.width, card.height, radii, card.left, card.top)));
        const polygons = pathPolygons(mask.path), patches = islandFilletPatches(card, box, edge, 12);
        for (const band of mask.bands) {
            assert.equal(edge === 'left' || edge === 'right' ? band.height : band.width, 2 + Math.max(1, 1 / dpr));
            for (let x = band.left + 0.123; x < band.left + band.width; x += 0.25) for (let y = band.top + 0.117; y < band.top + band.height; y += 0.25) {
                if (!polygons.some((polygon) => polygonContains(polygon, x, y))) continue;
                let ideal = inRect(x, y, card);
                if (!ideal) ideal = patches.some((patch) => {
                    if (!inRect(x, y, patch.rect)) return false;
                    const cx = edge === 'left' ? card.left + patch.radius : edge === 'right' ? card.left + card.width - patch.radius : patch.slot === 'start' ? card.left - patch.radius : card.left + card.width + patch.radius;
                    const cy = edge === 'top' ? card.top + patch.radius : edge === 'bottom' ? card.top + card.height - patch.radius : patch.slot === 'start' ? card.top - patch.radius : card.top + card.height + patch.radius;
                    return (x - cx) ** 2 + (y - cy) ** 2 >= patch.radius ** 2 - 0.03; // 64段四分之一圆离散误差<0.002 CSS px
                });
                assert.ok(ideal, `${edge} bandArea不能越出理想轮廓`);
            }
        }
        assert.equal(islandSeamMask(card, radii, box, edge, form, 12, dpr), undefined, '展开终态无补采样');
        assert.ok(islandSeamMask(card, radii, box, edge, form, -12, dpr), '稳态负角也补采样，不能遗留分数DPR内部接缝');
    }
});

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
    assert.equal(islandCornerRadius(12, "notch", "top", 12), "12px"); // 展开态贴边角等于岛圆角，四角一致。
});

test("刘海把贴边侧两角切成直角，圆弧由贴着卡片角的补面绘制", () => {
    assert.equal(islandCornerRadius(22, "capsule", "top"), "22px");
    assert.equal(islandCornerRadius(22, "notch", "top", 0), "0px 0px 22px 22px");
    assert.equal(islandCornerRadius(22, "notch", "bottom", 0), "22px 22px 0px 0px");
    assert.equal(islandCornerRadius(22, "notch", "left", 0), "0px 22px 22px 0px");
    assert.equal(islandCornerRadius(22, "notch", "right", 0), "22px 0px 0px 22px");
    // 贴边角从普通圆角平滑收成直角，再交给外凸补面。
    assert.equal(islandCornerRadius(22, "notch", "top", 6), "6px 6px 22px 22px");
    assert.equal(islandCornerRadius(22, "notch", "top", -12), "0px 0px 22px 22px");
});

test("外凸补面贴着卡片贴边角定位，四边方向各自正确", () => {
    // 顶部：窗口在卡片两侧各让 12 DIP。
    assert.deepEqual(islandFilletPatches({ left: 12, top: 0, width: 200, height: 60 }, { left: 0, top: 0, width: 224, height: 60 }, "top", 12), [
        { slot: "start", radius: 12, rect: { left: 0, top: 0, width: 12, height: 12 } },
        { slot: "end", radius: 12, rect: { left: 212, top: 0, width: 12, height: 12 } },
    ]);
    // 底部：补面在卡片下沿之上。
    assert.deepEqual(islandFilletPatches({ left: 12, top: 0, width: 200, height: 60 }, { left: 0, top: 0, width: 224, height: 60 }, "bottom", 12), [
        { slot: "start", radius: 12, rect: { left: 0, top: 48, width: 12, height: 12 } },
        { slot: "end", radius: 12, rect: { left: 212, top: 48, width: 12, height: 12 } },
    ]);
    // 左侧：窗口在卡片上下各让 12 DIP。
    assert.deepEqual(islandFilletPatches({ left: 0, top: 12, width: 200, height: 60 }, { left: 0, top: 0, width: 200, height: 84 }, "left", 12), [
        { slot: "start", radius: 12, rect: { left: 0, top: 0, width: 12, height: 12 } },
        { slot: "end", radius: 12, rect: { left: 0, top: 72, width: 12, height: 12 } },
    ]);
    // 右侧：补面在卡片右沿内侧。
    assert.deepEqual(islandFilletPatches({ left: 0, top: 12, width: 200, height: 60 }, { left: 0, top: 0, width: 200, height: 84 }, "right", 12), [
        { slot: "start", radius: 12, rect: { left: 188, top: 0, width: 12, height: 12 } },
        { slot: "end", radius: 12, rect: { left: 188, top: 72, width: 12, height: 12 } },
    ]);
});

test("补面半径按沿边余量收紧，半径为 0 时不绘制", () => {
    // 卡片离窗口左沿只有 5 DIP。
    assert.deepEqual(islandFilletPatches({ left: 5, top: 0, width: 200, height: 60 }, { left: 0, top: 0, width: 210, height: 60 }, "top", 12)[0], {
        slot: "start",
        radius: 5,
        rect: { left: 0, top: 0, width: 5, height: 5 },
    });
    assert.deepEqual(islandFilletPatches({ left: 12, top: 0, width: 200, height: 60 }, { left: 0, top: 0, width: 224, height: 60 }, "top", 0), []);
    // 卡片贴满窗口时没有可绘制空间，不能返回越界补面。
    assert.deepEqual(islandFilletPatches({ left: 0, top: 0, width: 200, height: 60 }, { left: 0, top: 0, width: 200, height: 60 }, "top", 12), []);
});
