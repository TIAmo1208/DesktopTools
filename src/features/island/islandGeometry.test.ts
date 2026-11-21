import assert from "node:assert/strict";
import test from "node:test";
import { islandCardMaskPath, islandContentOffset, islandCornerRadius, islandSeamMask, islandFilletPatches, islandGeometry, islandWindowKey, moveIslandCard, visibleIslandRect } from "./islandGeometry.ts";
import { parseIslandStateRequest } from "../../contracts/island.ts";
import { readFileSync } from "node:fs";

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
test("四边展开统一覆盖工作区高度，水平落位与滚动槽遵循原规则", () => {
    for (const [edge, left, measuredLeft] of [
        ["top", 892, 0],
        ["bottom", 892, 0],
        ["left", 6, 0],
        ["right", 1578, 16],
    ] as const) {
        // 展开态窗口在卡片外侧多一条 16 DIP 滚动条槽；贴右边时槽在卡片左侧，实测卡片带 16 起始内缩。
        const request = islandGeometry("expanded", [{ ...card, left: measuredLeft }], 12, work, edge, 100);
        assert.deepEqual(request.windowBox, { left, top: work.top, width: 336, height: work.height });
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
    assert.equal(center(collapsed.windowBox), center(expanded.windowBox)); // 展开窗口含滚动条槽整体沿边居中，槽不会把窗口推出工作区。
    assert.equal(collapsed.windowBox.top, work.top + 6); // 胶囊仍离开边缘 6 DIP。
    assert.equal(expanded.windowBox.top, work.top);
    assert.equal(expanded.windowBox.height, work.height);
    assert.deepEqual(expanded.viewport, work); // 展开页面视口铺满工作区，透明区可收到指针。
    assert.equal(collapsed.viewport.width, 420); // 胶囊仍使用 420 视口带，几何不变。
});
test("实测统一宽度、各自高度，间隙透明但仍归岛命中", () => {
    const request = islandGeometry("expanded", [card, { ...card, top: 248, height: 120 }], 12, work, "bottom", 0);
    assert.equal(request.windowBox.height, work.height);
    assert.deepEqual(request.cards[2], { rect: { left: 0, top: 240, width: 320, height: 8 }, radius: 0 });
    assert.deepEqual(parseIslandStateRequest(request), request);
    const tall = islandGeometry("expanded", [{ ...card, height: 2000 }, { ...card, top: 2008 }], 12, work, "bottom", 0);
    assert.deepEqual(tall.windowBox, request.windowBox);
    assert.deepEqual(tall.cards.map(({ rect }) => rect.height), [work.height]);
    assert.deepEqual(parseIslandStateRequest(tall), tall);
    assert.throws(() => islandGeometry("expanded", [{ ...card, width: work.width }], 12, work, "top", 0), /超过主屏工作区/);
    assert.throws(() => islandGeometry("hover", [{ ...card, height: work.height + 1 }], 16, work, "top", 0), /超过主屏工作区/);
});
test("展开窗口键仅随实际列宽、工作区或三态改变，不随内部卡片变化", () => {
    const primary = { left: 0, top: 0, width: 2560, height: 1400 };
    const before = islandGeometry("expanded", [card, { ...card, top: 248 }], 12, primary, "top", 0);
    const folded = islandGeometry("expanded", [{ ...card, height: 44 }, { ...card, top: 52 }], 12, primary, "top", 0);
    const scrolled = islandGeometry("expanded", [{ ...card, top: -120 }, { ...card, top: 128 }], 12, primary, "top", 0);
    assert.deepEqual(before.windowBox, { left: 1112, top: 0, width: 336, height: 1400 });
    assert.notDeepEqual(folded.cards, before.cards);
    assert.equal(islandWindowKey(folded), islandWindowKey(before));
    assert.equal(islandWindowKey(scrolled), islandWindowKey(before));
    const wider = islandGeometry("expanded", [{ ...card, width: 360 }], 12, primary, "top", 0);
    assert.notEqual(islandWindowKey(wider), islandWindowKey(before));
    assert.notEqual(islandWindowKey({ ...before, state: "hover" }), islandWindowKey(before));
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
    assert.deepEqual(expanded.windowBox, { left: 792, top: 40, width: 336, height: work.height });
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

test("展开首帧内容偏移按贴边与偏移落在目标列，不是 (0,0)", () => {
    const cases = [
        ["top", 100], ["bottom", 100], ["left", 100], ["right", 100], ["top", -300], ["", 0],
    ] as const;
    for (const [edge, offset] of cases) {
        const request = islandGeometry("expanded", [card], 12, work, edge, offset);
        const content = islandContentOffset(request);
        assert.equal(content.y, 0); // 展开内容列从工作区顶部开始。
        assert.equal(content.x, request.windowBox.left - work.left);
        assert.ok(content.x > 0, `${edge || "top"} 偏移 ${offset} 不应落在页面原点`);
        assert.deepEqual(request.viewport, work);
    }
    assert.equal(islandContentOffset(islandGeometry("expanded", [card], 12, work, "right", 0) as never).x, 1594);
});

test("展开根铺满工作区，卡片列按贴边偏移定位且首帧不在 0,0", () => {
    const cases = [
        ["left", 100, 6], ["right", 100, 1594], ["top", 100, 892], ["bottom", 100, 892], ["top", 0, 792],
    ] as const;
    for (const [edge, offset, expected] of cases) {
        const request = islandGeometry("expanded", [{ ...card, left: 0 }], 12, work, edge, offset);
        const columnX = request.windowBox.left - work.left;
        assert.equal(columnX, expected, `${edge}/${offset} 列左边距`);
        assert.equal(columnX + request.cards[0]!.rect.left, expected); // 卡片屏幕 x 自然带列偏移。
        assert.notEqual(columnX + request.cards[0]!.rect.top, 0); // 首帧不是左上角 (0,0)。
        assert.deepEqual(request.viewport, work);
    }
    const source = readFileSync(new URL("./island.tsx", import.meta.url), "utf8");
    assert.match(source, /inlineSize: `\$\{expandedPaint \? work\.width : value\.windowBox\.width\}px`/); // 根宽=工作区。
    assert.match(source, /left: `\$\{shape\.rect\.left \+ columnX\}px`/); // 卡片列绝对偏移。
    assert.match(source, /applyViewportMargin = \(value: IslandStateRequest\) => \{\s*const offset = value\.state === "expanded"/);
    const css = readFileSync(new URL("./island.css", import.meta.url), "utf8");
    assert.match(css, /padding-inline-start: var\(--island-expanded-column, 0px\);/);
    assert.match(css, /padding-inline-start: calc\(var\(--island-expanded-column, 0px\) \+ 16px\);/);
});
