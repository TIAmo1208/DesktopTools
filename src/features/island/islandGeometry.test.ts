import assert from "node:assert/strict";
import test from "node:test";
import { islandCardMaskPath, islandContentOffset, islandCornerRadius, islandSeamMask, islandFilletPatches, islandGeometry, islandWindowKey, moveIslandCard, visibleIslandRect, ISLAND_COMPACT_MAX_WIDTH, ISLAND_EXPANDED_COLUMN_WIDTH, ISLAND_SCROLLBAR_GUTTER } from "./islandGeometry.ts";
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
const card = { left: 0, top: 0, width: 400, height: 240 };
test("四边展开统一覆盖工作区高度，水平落位与滚动槽遵循原规则", () => {
    for (const [edge, left, measuredLeft] of [
        ["top", 860, 0],
        ["bottom", 860, 0],
        ["left", 6, 0],
        ["right", 1498, 16],
    ] as const) {
        // 展开态窗口在卡片外侧多一条 16 DIP 滚动条槽；贴右边时槽在卡片左侧，实测卡片带 16 起始内缩。
        const request = islandGeometry("expanded", [{ ...card, left: measuredLeft }], 12, work, edge, 100);
        assert.deepEqual(request.windowBox, { left, top: work.top, width: 416, height: work.height });
        assert.deepEqual(parseIslandStateRequest(request), request);
    }
    assert.equal(islandGeometry("hover", [card], 16, work, "right", 2000).windowBox.top, 840);
    assert.equal(islandGeometry("collapsed", [card], 22, work, "", 999).windowBox.left, 1520); // 空边按顶部应用偏移。
    assert.equal(islandGeometry("collapsed", [card], 22, work, "", 0).windowBox.left, 760); // 空边偏移 0 仍居中。
});

test("三态尺寸变化时贴边侧固定，沿边以中心对称扩展", () => {
    const collapsed = islandGeometry("collapsed", [{ ...card, width: 44, height: 44 }], 22, work, "top", 0);
    const expanded = islandGeometry("expanded", [card], 12, work, "top", 0);
    const center = (rect: { left: number; width: number }) => rect.left + rect.width / 2;
    // 卡片内容按自身宽度沿边居中；滚动条槽只让窗口向一侧长出去，不参与居中。
    assert.equal(center(collapsed.windowBox), expanded.windowBox.left + expanded.cards[0]!.rect.width / 2);
    assert.equal(expanded.windowBox.width, expanded.cards[0]!.rect.width + 16); // 窗口仍比内容宽一条槽。
    assert.equal(collapsed.windowBox.top, work.top + 6); // 胶囊仍离开边缘 6 DIP。
    assert.equal(expanded.windowBox.top, work.top);
    assert.equal(expanded.windowBox.height, work.height);
    assert.deepEqual(expanded.viewport, work); // 展开页面视口铺满工作区，透明区可收到指针。
    assert.equal(collapsed.viewport.width, 440); // 胶囊仍使用 440 视口带，几何不变。
});
test("实测统一宽度、各自高度，间隙透明但仍归岛命中", () => {
    const request = islandGeometry("expanded", [card, { ...card, top: 248, height: 120 }], 12, work, "bottom", 0);
    assert.equal(request.windowBox.height, work.height);
    assert.deepEqual(request.cards[2], { rect: { left: 0, top: 240, width: 400, height: 8 }, radius: 0 });
    assert.deepEqual(parseIslandStateRequest(request), request);
    const tall = islandGeometry("expanded", [{ ...card, height: 2000 }, { ...card, top: 2008 }], 12, work, "bottom", 0);
    assert.deepEqual(tall.windowBox, request.windowBox);
    assert.deepEqual(tall.cards.map(({ rect }) => rect.height), [work.height]);
    assert.deepEqual(parseIslandStateRequest(tall), tall);
    assert.throws(() => islandGeometry("expanded", [card], 12, { ...work, width: 400 }, "top", 0), /超过主屏工作区/); // 列宽固定 400 后，只能靠窄工作区触发。
    assert.throws(() => islandGeometry("hover", [{ ...card, height: work.height + 1 }], 16, work, "top", 0), /超过主屏工作区/);
});
test("展开窗口键仅随实际列宽、工作区或三态改变，不随内部卡片变化", () => {
    const primary = { left: 0, top: 0, width: 2560, height: 1400 };
    const before = islandGeometry("expanded", [card, { ...card, top: 248 }], 12, primary, "top", 0);
    const folded = islandGeometry("expanded", [{ ...card, height: 44 }, { ...card, top: 52 }], 12, primary, "top", 0);
    const scrolled = islandGeometry("expanded", [{ ...card, top: -120 }, { ...card, top: 128 }], 12, primary, "top", 0);
    assert.deepEqual(before.windowBox, { left: 1080, top: 0, width: 416, height: 1400 });
    assert.notDeepEqual(folded.cards, before.cards);
    assert.equal(islandWindowKey(folded), islandWindowKey(before));
    assert.equal(islandWindowKey(scrolled), islandWindowKey(before));
    const wider = islandGeometry("expanded", [card], 12, { ...primary, width: 2400 }, "top", 0); // 列宽固定后只有工作区变化能改窗口键。
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
    const size = { left: 0, top: 0, width: 400, height: 240 };
    const expanded = islandGeometry("expanded", [size], 12, work, "top", 0, "notch");
    assert.deepEqual(expanded.windowBox, { left: 760, top: 40, width: 416, height: work.height });
    assert.deepEqual(expanded.cards[0]!.rect, { left: 0, top: 0, width: 400, height: 240 });
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
    assert.equal(islandContentOffset(islandGeometry("expanded", [card], 12, work, "right", 0) as never).x, 1514);
});

test("展开根铺满工作区，卡片列按贴边偏移定位且首帧不在 0,0", () => {
    const cases = [
        ["left", 100, 6], ["right", 100, 1514], ["top", 100, 860], ["bottom", 100, 860], ["top", 0, 760],
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

test("展开滚动条槽：极端偏移只夹紧内容起点，窗口仍在工作区内", () => {
    for (const edge of ["top", "bottom"] as const) {
        const pushed = islandGeometry("expanded", [card], 12, work, edge, 1e6);
        assert.equal(pushed.windowBox.left, work.left + work.width - card.width - 16, `${edge} 正偏移夹紧后内容贴工作区右缘、窗口补槽`);
        assert.ok(pushed.windowBox.left + pushed.windowBox.width <= work.left + work.width + 0.001, `${edge} 窗口不得越出工作区`);
        assert.equal(pushed.cards[0]!.rect.left, 0); // 卡片仍在内容列原点，槽不计入卡片坐标。
        const pulled = islandGeometry("expanded", [card], 12, work, edge, -1e6);
        assert.equal(pulled.windowBox.left, work.left, `${edge} 负偏移夹紧到工作区左缘`);
        assert.equal(pulled.cards[0]!.rect.left, 0);
    }
    // 同内容宽下窗口盒与卡片坐标恒定：滚动条显隐不参与几何。
    const a = islandGeometry("expanded", [card], 12, work, "top", 0);
    const b = islandGeometry("expanded", [{ ...card }], 12, work, "top", 0);
    assert.deepEqual(a.windowBox, b.windowBox);
    assert.deepEqual(a.cards[0]!.rect, b.cards[0]!.rect);
});

test("anchorStart：自然宽切换锚定左缘，宽度向右扩展；展开态不锚定", () => {
    const work = { left: 0, top: 0, width: 1920, height: 1080 };
    const 卡 = (width) => [{ left: 0, top: 0, width, height: 40 }];
    const 居中 = islandGeometry("collapsed", 卡(320), 22, work, "top", 0, "capsule");
    assert.equal(居中.windowBox.left, 800, `居中胶囊 left 应为 780`);
    const 锚定 = islandGeometry("collapsed", 卡(320), 22, work, "top", 0, "capsule", { anchorStart: 100 });
    assert.equal(锚定.windowBox.left, 100);
    // 消息切换到自然宽更小的内容：左缘不变，宽度向右扩展/收缩。
    const 变窄 = islandGeometry("collapsed", 卡(280), 22, work, "top", 0, "capsule", { anchorStart: 100 });
    assert.equal(变窄.windowBox.left, 100);
    assert.equal(变窄.windowBox.width, 280);
    assert.equal(锚定.windowBox.width, 320);
    // 越界时夹在工作区内。
    const 越界 = islandGeometry("collapsed", 卡(320), 22, work, "top", 0, "capsule", { anchorStart: 5000 });
    assert.equal(越界.windowBox.left, 1920 - 越界.windowBox.width);
    // 展开态固定列（不锚定）：列偏移与窗口盒不随锚点变化。
    const 展开 = islandGeometry("expanded", 卡(320), 12, work, "top", 0, "capsule", { anchorStart: 100 });
    const 展开对照 = islandGeometry("expanded", 卡(320), 12, work, "top", 0, "capsule");
    assert.equal(展开.windowBox.left, 展开对照.windowBox.left);
    assert.equal(ISLAND_EXPANDED_COLUMN_WIDTH, 400);
});

test("hover 卡宽上限 392、窗口不超过展开窗 416，切换选中不变（t119/t123）", () => {
    assert.equal(ISLAND_COMPACT_MAX_WIDTH, ISLAND_EXPANDED_COLUMN_WIDTH + ISLAND_SCROLLBAR_GUTTER - 24, "hover 卡宽上限 = 展开窗 416 − 两端缺口 24");
    assert.equal(ISLAND_COMPACT_MAX_WIDTH, 392);
    assert.ok(ISLAND_COMPACT_MAX_WIDTH + 24 <= ISLAND_EXPANDED_COLUMN_WIDTH + 16, "hover 窗口（卡宽 + 两端缺口）不得超过展开窗口 416");
    const work = { left: 0, top: 0, width: 1920, height: 1080 };
    // 极端长内容也只能取到上限：卡片与窗口盒都不超过展开窗。
    const 超长 = islandGeometry("hover", [{ left: 0, top: 0, width: ISLAND_COMPACT_MAX_WIDTH, height: 72 }], 16, work, "top", 0, "capsule");
    assert.ok(超长.cards[0]!.rect.width <= ISLAND_COMPACT_MAX_WIDTH + 0.01, "hover 卡片不得超过 392");
    assert.ok(超长.windowBox.width <= ISLAND_EXPANDED_COLUMN_WIDTH + 16 + 0.01, `hover 窗口不得超过展开窗 416，实际 ${超长.windowBox.width}`);
    const 展开 = islandGeometry("expanded", [{ left: 0, top: 0, width: ISLAND_COMPACT_MAX_WIDTH, height: 72 }], 12, work, "top", 0, "capsule");
    assert.equal(展开.cards[0]!.rect.width, ISLAND_EXPANDED_COLUMN_WIDTH, "展开仍固定 400");
    // hover 窗口左右缘不越过展开窗口：边缘点击落点仍在展开窗内。
    assert.ok(超长.windowBox.left >= 展开.windowBox.left - 0.01, `hover 左缘 ${超长.windowBox.left} 不应越过展开左缘 ${展开.windowBox.left}`);
    assert.ok(超长.windowBox.left + 超长.windowBox.width <= 展开.windowBox.left + 展开.windowBox.width + 0.01, "hover 右缘不应越过展开右缘");
});

test("hover 窗盒在四种贴边与两种形态下都被展开窗盒包含（t126）", () => {
    const work = { left: 0, top: 0, width: 1920, height: 1080 };
    const 卡 = [{ left: 0, top: 0, width: ISLAND_COMPACT_MAX_WIDTH, height: 72 }];
    for (const form of ["capsule", "notch"] as const) {
        for (const edge of ["top", "bottom", "left", "right"] as const) {
            for (const offset of [0, 120, -120]) {
                const hover = islandGeometry("hover", 卡, 16, work, edge, offset, form);
                const expanded = islandGeometry("expanded", 卡, 12, work, edge, offset, form);
                const h = hover.windowBox;
                const e = expanded.windowBox;
                assert.ok(h.left >= e.left - 0.01, `${form}/${edge}/${offset}: hover 左缘 ${h.left} 越出展开窗左缘 ${e.left}`);
                assert.ok(h.left + h.width <= e.left + e.width + 0.01, `${form}/${edge}/${offset}: hover 右缘 ${h.left + h.width} 越出展开窗右缘 ${e.left + e.width}`);
                assert.ok(h.top >= e.top - 0.01 && h.top + h.height <= e.top + e.height + 0.01, `${form}/${edge}/${offset}: hover 上下越出展开窗`);
                // 可点击卡体仍在展开窗内，且上下贴边时左右各留 12 DIP 缺口。
                const 卡体 = hover.cards[0]!.rect; // 卡体坐标相对窗口原点。
                assert.ok(卡体.left >= -0.01 && 卡体.left + 卡体.width <= h.width + 0.01, `${form}/${edge}/${offset}: 卡体越出 hover 窗`);
                if (form === "notch" && (edge === "top" || edge === "bottom")) assert.ok(Math.abs(卡体.left - 12) <= 0.5, `${form}/${edge}/${offset}: 刘海卡片应左右各让出 12 DIP 圆弧区，实际 ${卡体.left}`);
            }
        }
    }
});

test("极限偏移（±760）与工作区边缘下 hover 窗仍被展开窗包含（t128）", () => {
    const work = { left: 0, top: 0, width: 1920, height: 1080 };
    const 卡 = [{ left: 0, top: 0, width: ISLAND_COMPACT_MAX_WIDTH, height: 72 }];
    for (const form of ["capsule", "notch"] as const) {
        for (const edge of ["top", "bottom", "left", "right"] as const) {
            for (const offset of [0, 120, -120, 760, -760]) {
                const hover = islandGeometry("hover", 卡, 16, work, edge, offset, form);
                const expanded = islandGeometry("expanded", 卡, 12, work, edge, offset, form);
                const h = hover.windowBox;
                const e = expanded.windowBox;
                // 窗口按实际宽度夹紧：既不越出工作区，也不越出展开窗（左右与上下四端）。
                assert.ok(h.left >= work.left - 0.01 && h.left + h.width <= work.left + work.width + 0.01, `${form}/${edge}/${offset}: hover 越出工作区 [${h.left},${h.left + h.width}]`);
                assert.ok(h.left >= e.left - 0.01 && h.left + h.width <= e.left + e.width + 0.01, `${form}/${edge}/${offset}: hover [${h.left},${h.left + h.width}] 未被展开窗 [${e.left},${e.left + e.width}] 包含`);
                assert.ok(h.top >= e.top - 0.01 && h.top + h.height <= e.top + e.height + 0.01, `${form}/${edge}/${offset}: hover 上下越出展开窗`);
            }
        }
    }
    // 窄卡仍按自身宽度自然居中，不被上限夹紧。
    const 窄 = islandGeometry("hover", [{ left: 0, top: 0, width: 200, height: 72 }], 16, work, "top", 0, "capsule");
    assert.equal(窄.windowBox.width, 200);
    assert.ok(Math.abs(窄.windowBox.left + 窄.windowBox.width / 2 - 960) < 1, "窄卡 hover 应居中");
});

test("刘海 hover 窄卡按实际窗宽居中、并夹在展开盒区间内（t130）", () => {
    const work = { left: 0, top: 0, width: 1920, height: 1080 };
    for (const edge of ["top", "bottom"] as const) {
        for (const width of [200, 280]) {
            const hover = islandGeometry("hover", [{ left: 0, top: 0, width, height: 72 }], 16, work, edge, 0, "notch");
            const expanded = islandGeometry("expanded", [{ left: 0, top: 0, width, height: 72 }], 12, work, edge, 0, "notch");
            const h = hover.windowBox;
            const 卡 = hover.cards[0]!.rect;
            // 窗口 = 卡宽 + 两端各 12；卡片中心与窗口中心都要落在工作区中心。
            assert.equal(h.width, width + 24, `${edge}/${width}: 刘海窗宽应为卡宽 + 24`);
            assert.ok(Math.abs(h.left + h.width / 2 - 960) < 1, `${edge}/${width}: 窗中心应在工作区中心，实际 ${h.left + h.width / 2}`);
            assert.ok(Math.abs((卡.left + 卡.width / 2) - h.width / 2) < 1, `${edge}/${width}: 卡片应在窗口内居中`);
            assert.ok(h.left >= expanded.windowBox.left - 0.01 && h.left + h.width <= expanded.windowBox.left + expanded.windowBox.width + 0.01, `${edge}/${width}: 窄卡窗应落在展开盒内`);
        }
    }
    // 392 宽仍与展开窗端点对齐（t126 不回归）。
    const 宽 = islandGeometry("hover", [{ left: 0, top: 0, width: ISLAND_COMPACT_MAX_WIDTH, height: 72 }], 16, work, "top", 0, "notch");
    assert.deepEqual(宽.windowBox, { left: 760, top: 0, width: 416, height: 72 });
});

test("侧贴高卡与旧锚点 anchorStart 都按最终窗尺寸限界（t132）", () => {
    const work = { left: 0, top: 0, width: 1920, height: 1080 };
    // 1) 侧贴高卡：沿边是高度，展开窗覆盖整段工作区，夹紧不能用 400 列宽求出的 590。
    for (const edge of ["left", "right"] as const) {
        for (const offset of [250, -250]) {
            const hover = islandGeometry("hover", [{ left: 0, top: 0, width: 200, height: 780 }], 16, work, edge, offset, "capsule");
            const expanded = islandGeometry("expanded", [{ left: 0, top: 0, width: 200, height: 780 }], 12, work, edge, offset, "capsule");
            const h = hover.windowBox;
            assert.equal(h.height, 780);
            assert.ok(h.top >= -0.01 && h.top + h.height <= 1080 + 0.01, `${edge}/${offset}: 高卡窗越出工作区 top=${h.top}`);
            assert.ok(h.top >= expanded.windowBox.top - 0.01 && h.top + h.height <= expanded.windowBox.top + expanded.windowBox.height + 0.01, `${edge}/${offset}: 高卡窗未被展开窗包含`);
        }
    }
    // 2) 旧锚点（200 宽胶囊 left=848）不得把 392 hover 窗推出展开盒。
    const 锚定 = islandGeometry("hover", [{ left: 0, top: 0, width: ISLAND_COMPACT_MAX_WIDTH, height: 72 }], 16, work, "top", 0, "notch", { anchorStart: 848 });
    const 展开 = islandGeometry("expanded", [{ left: 0, top: 0, width: ISLAND_COMPACT_MAX_WIDTH, height: 72 }], 12, work, "top", 0, "notch");
    assert.deepEqual(锚定.windowBox, { left: 760, top: 0, width: 416, height: 72 }, `旧锚点应被夹回展开盒，实际 ${JSON.stringify(锚定.windowBox)}`);
    assert.ok(锚定.windowBox.left >= 展开.windowBox.left && 锚定.windowBox.left + 锚定.windowBox.width <= 展开.windowBox.left + 展开.windowBox.width);
});

test("横向侧贴的旧 anchorStart 按展开列盒 X 边界夹紧（t134）", () => {
    const work = { left: 0, top: 0, width: 1920, height: 1080 };
    const 卡 = [{ left: 0, top: 0, width: ISLAND_COMPACT_MAX_WIDTH, height: 72 }];
    for (const [edge, anchor] of [["right", 1714], ["left", 0], ["left", 1900]] as const) {
        const hover = islandGeometry("hover", 卡, 16, work, edge, 0, "capsule", { anchorStart: anchor });
        const expanded = islandGeometry("expanded", 卡, 12, work, edge, 0, "capsule");
        const h = hover.windowBox;
        const e = expanded.windowBox;
        assert.ok(h.left >= e.left - 0.01 && h.left + h.width <= e.left + e.width + 0.01, `${edge}/anchor${anchor}: hover [${h.left},${h.left + h.width}] 越出展开盒 [${e.left},${e.left + e.width}]`);
        // 右贴边时右缘必须落在展开盒右缘内（工作区右缘还要让出胶囊 6 DIP）。
        if (edge === "right") assert.ok(h.left + h.width <= work.width - 6 + 0.01, `右贴 hover 右缘 ${h.left + h.width} 越出 1914`);
    }
});
