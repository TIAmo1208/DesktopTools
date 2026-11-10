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
    ISLAND_MARGIN_DELAY_MS,
    ISLAND_REENTRY_MARGIN,
    ISLAND_SCROLLBAR_FADE_MS,
    islandPointInRect,
    islandCardDistance,
    islandHitContour,
    islandContourDistance,
    islandExtraBands,
    islandScrollGeometry,
    islandHitRegion,
    islandMarginStep,
    islandDebugCardCount,
    islandCollapseSettle,
    type IslandMarginState,
} from "./islandInteraction.ts";
import { moveIslandCard } from "./islandGeometry.ts";
import type { IslandRect, IslandStateRequest } from "../../contracts/island.ts";

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

// 非常规态透明命中与展开滚动条：全部判定是纯函数，旧实现没有这些函数，因此本组用例在旧代码上必然红。
type IslandEdge = "top" | "bottom" | "left" | "right";
const islandEdges: IslandEdge[] = ["top", "bottom", "left", "right"];
const islandForms = ["capsule", "notch"] as const;
const islandDprs = [1, 1.25, 1.5, 1.75] as const;
const islandHorizontalEdge = (edge: IslandEdge) => edge === "top" || edge === "bottom";
/** 设备像素量化后的 CSS 坐标：d 倍 DPR 下最小可表达步长是 1/d。 */
const snapDpr = (value: number, dpr: number) => Math.round(value * dpr) / dpr;
/** 小数几何按 1e-6 比较，分数 DPR 的累加误差不影响结论。 */
const near = (actual: number, expected: number, label: string) => assert.ok(Math.abs(actual - expected) < 1e-6, `${label}: ${actual} ≠ ${expected}`);
/** 按边与形态给出并集窗口、目标内容盒与四角半径：贴边侧卡片贴边，刘海在沿边方向各留 12 DIP 供补面。 */
const islandLayoutCase = (edge: IslandEdge, form: (typeof islandForms)[number], dpr: number) => {
    const horizontal = islandHorizontalEdge(edge);
    const window: IslandRect = horizontal
        ? { left: snapDpr(101, dpr), top: snapDpr(40, dpr), width: snapDpr(420, dpr), height: snapDpr(240, dpr) }
        : { left: snapDpr(101, dpr), top: snapDpr(40, dpr), width: snapDpr(200, dpr), height: snapDpr(320, dpr) };
    const alongPad = snapDpr(form === "notch" ? 12.5 : 9.25, dpr); // 刘海窗口沿边外扩 12 DIP，胶囊并集余量更小。
    // 卡片始终贴在自己那条边上：贴上下贴上下边、贴左右贴左右边，贴合实际的贴边锚点。
    const card: IslandRect = horizontal
        ? { left: window.left + alongPad, top: edge === "top" ? window.top : window.top + window.height - snapDpr(120, dpr), width: snapDpr(180, dpr), height: snapDpr(120, dpr) }
        : { left: edge === "left" ? window.left : window.left + window.width - snapDpr(120, dpr), top: window.top + alongPad, width: snapDpr(120, dpr), height: snapDpr(140, dpr) };
    // 贴边侧两角按形态取方形（刘海）或圆角（胶囊）；未贴边两角始终是浮岛圆角。
    const radii = form === "capsule" ? [12, 12, 12, 12]
        : edge === "top" ? [0, 0, 12, 12]
            : edge === "bottom" ? [12, 12, 0, 0]
                : edge === "left" ? [0, 12, 12, 0]
                    : [12, 0, 0, 12];
    return { window, card, radii, alongPad };
};
/** 构造一次提交的真实请求；viewport 固定在屏幕原点便于核对客户区坐标。 */
const islandFrame = (state: IslandStateRequest["state"], rect: IslandRect, viewport: IslandRect = { left: 0, top: 0, width: 1920, height: 1080 }): IslandStateRequest => ({ state, windowBox: rect, viewport, cards: [] });
/** 补面方形的圆弧中心与对角：中心所在象限是透明扇区，对角才是可见弧面。 */
const islandPatchCorners = (edge: IslandEdge, slot: 0 | 1, card: IslandRect, radius: number) => {
    const rect = {
        top: [{ left: card.left - radius, top: card.top }, { left: card.left + card.width, top: card.top }],
        bottom: [{ left: card.left - radius, top: card.top + card.height - radius }, { left: card.left + card.width, top: card.top + card.height - radius }],
        left: [{ left: card.left, top: card.top - radius }, { left: card.left, top: card.top + card.height }],
        right: [{ left: card.left + card.width - radius, top: card.top - radius }, { left: card.left + card.width - radius, top: card.top + card.height }],
    }[edge][slot]!;
    const [cx, cy] = { top: [[0, 1], [1, 1]], bottom: [[0, 0], [1, 0]], left: [[1, 0], [1, 1]], right: [[0, 0], [0, 1]] }[edge][slot]!;
    return { rect, center: { x: rect.left + cx * radius, y: rect.top + cy * radius }, opposite: { x: rect.left + (1 - cx) * radius, y: rect.top + (1 - cy) * radius } };
};

test("展开退回带只取并集窗口与目标内容盒的沿贴边轴差值，四边两形态四档DPR一致", () => {
    for (const form of islandForms) for (const edge of islandEdges) for (const dpr of islandDprs) {
        const { window, card } = islandLayoutCase(edge, form, dpr);
        const label = `${edge}/${form}/DPR${dpr}`, horizontal = islandHorizontalEdge(edge);
        const bands = islandExtraBands(window, card, edge);
        assert.equal(bands.length, 2, `${label} 应恰有两条退回带`);
        const [first, second] = bands as [IslandRect, IslandRect];
        if (horizontal) {
            // 顶/底贴边只沿横向切分：带与并集窗口同高，贴边侧与正交余量不构成带。
            near(first.left, window.left, `${label} 首带左界`);
            near(first.width, card.left - window.left, `${label} 首带宽`);
            near(second.left, card.left + card.width, `${label} 次带左界`);
            near(second.width, window.left + window.width - card.left - card.width, `${label} 次带宽`);
            for (const band of bands) { near(band.top, window.top, `${label} 带顶`); near(band.height, window.height, `${label} 带高`); }
            const margin = { x: card.left + card.width / 2, y: window.top + window.height - 0.5 }; // 内容盒正下方的贴边余量。
            assert.equal(bands.some((band) => islandPointInRect(margin, band)), false, `${label} 正交贴边余量不触发展开退回`);
        } else {
            // 左/右贴边只沿纵向切分：带与并集窗口同宽。
            near(first.top, window.top, `${label} 首带上界`);
            near(first.height, card.top - window.top, `${label} 首带高`);
            near(second.top, card.top + card.height, `${label} 次带上界`);
            near(second.height, window.top + window.height - card.top - card.height, `${label} 次带高`);
            for (const band of bands) { near(band.left, window.left, `${label} 带左`); near(band.width, window.width, `${label} 带宽`); }
            const margin = { x: window.left + window.width - 0.5, y: card.top + card.height / 2 }; // 内容盒旁侧的未贴边余量。
            assert.equal(bands.some((band) => islandPointInRect(margin, band)), false, `${label} 正交侧余量不触发展开退回`);
        }
        const contentCenter = { x: card.left + card.width / 2, y: card.top + card.height / 2 };
        assert.equal(bands.some((band) => islandPointInRect(contentCenter, band)), false, `${label} 内容盒不在退回带内`);
        // 带在贴边轴上与内容盒相接，正好覆盖整条并集边长，不硬编码右侧。
        const span = horizontal ? window.width : window.height;
        const [firstSize, secondSize] = horizontal ? [first.width, second.width] : [first.height, second.height];
        near((horizontal ? card.width : card.height) + firstSize + secondSize, span, `${label} 带与内容盒覆盖整条边`);
    }
    // 目标内容盒已贴满该轴时没有退回带；未展开时目标缺省也不产生带。
    assert.deepEqual(islandExtraBands({ left: 0, top: 0, width: 100, height: 100 }, { left: 0, top: 0, width: 100, height: 100 }, "top"), []);
    // 四条边各自使用自己的轴：同一份几何在顶/左贴边下带的方向不同。
    const window = { left: 0, top: 0, width: 200, height: 100 };
    assert.equal(islandExtraBands(window, { left: 20, top: 0, width: 100, height: 100 }, "top")[0]!.height, 100);
    assert.equal(islandExtraBands(window, { left: 0, top: 20, width: 200, height: 60 }, "left")[0]!.width, 200);
});

test("透明轮廓：胶囊圆角外四角与刘海补面透明扇区都算透明，可见弧面算内容", () => {
    for (const form of islandForms) for (const edge of islandEdges) for (const dpr of islandDprs) {
        const { window, card, radii } = islandLayoutCase(edge, form, dpr);
        const label = `${edge}/${form}/DPR${dpr}`;
        const contour = islandHitContour([{ rect: card, radii }], window, edge, form, form === "notch" ? -12 : 0);
        assert.equal(contour.arcs.length, form === "notch" ? 2 : 0, `${label} 补面数量`);
        // 未贴边侧两角始终是圆角外透明像素；贴边侧在刘海里是方形（补面外侧），在胶囊里仍是圆角。
        const corners = [
            { x: card.left + 0.25, y: card.top + 0.25 },
            { x: card.left + card.width - 0.25, y: card.top + 0.25 },
            { x: card.left + card.width - 0.25, y: card.top + card.height - 0.25 },
            { x: card.left + 0.25, y: card.top + card.height - 0.25 },
        ];
        const attached = edge === "top" ? [0, 1] : edge === "bottom" ? [2, 3] : edge === "left" ? [0, 3] : [1, 2];
        corners.forEach((point, index) => {
            const distance = islandContourDistance(point, contour);
            const square = form === "notch" && attached.includes(index);
            if (square) assert.ok(distance <= 1e-8, `${label} 贴边侧方形角在可见轮廓内`);
            else assert.ok(distance > 0, `${label} 第${index + 1}角必须算透明`);
        });
        if (form !== "notch") continue;
        // 每块补面：透明扇区（圆弧中心所在方形角）不算内容，对角附近的可见弧面算内容。
        for (const slot of [0, 1] as const) {
            const patch = islandPatchCorners(edge, slot, card, 12);
            const inward = { x: patch.center.x + (patch.opposite.x - patch.center.x) * 0.18, y: patch.center.y + (patch.opposite.y - patch.center.y) * 0.18 };
            const visible = { x: patch.opposite.x + (patch.center.x - patch.opposite.x) * 0.18, y: patch.opposite.y + (patch.center.y - patch.opposite.y) * 0.18 };
            const arc = contour.arcs[slot]!;
            near(arc.radius, 12, `${label} 补面半径`);
            near(arc.center.x, patch.center.x, `${label} 第${slot + 1}块补面圆弧中心 x`);
            near(arc.center.y, patch.center.y, `${label} 第${slot + 1}块补面圆弧中心 y`);
            assert.ok(islandContourDistance(inward, contour) > 0, `${label} 第${slot + 1}块补面的透明扇区不算内容`);
            assert.ok(islandContourDistance(visible, contour) <= 1e-8, `${label} 第${slot + 1}块补面的可见弧面算内容`);
            assert.ok(islandCardDistance(visible, { rect: card, radii }) > 0, `${label} 可见弧面在卡片方框之外`);
        }
    }
    // 胶囊在负圆角/展开态没有补面，不能凭空多出轮廓。
    assert.equal(islandHitContour([{ rect: { left: 0, top: 0, width: 120, height: 60 }, radii: [6, 6, 6, 6] }], { left: 0, top: 0, width: 200, height: 100 }, "top", "notch", 0).arcs.length, 0);
});

test("展开态透明带命中：只有并集减目标内容盒的额外带退回，滚动条矩形先行扣除", () => {
    for (const edge of islandEdges) for (const form of islandForms) for (const dpr of islandDprs) {
        const { window, card } = islandLayoutCase(edge, form, dpr);
        const label = `${edge}/${form}/DPR${dpr}`;
        // 展开态统一浮岛圆角、不绘制补面，命中轮廓只含四角圆角。
        const contour = islandHitContour([{ rect: card, radii: [12, 12, 12, 12] }], window, edge, "capsule", 0);
        const metrics = { mode: "expanded" as const, settled: true, scrollHeight: 900, clientHeight: card.height, scrollTop: 0 };
        const layout = islandScrollGeometry(islandFrame("expanded", window), islandFrame("expanded", card), edge, metrics);
        assert.equal(layout.visible, true, `${label} 溢出且已提交时必须出条`);
        assert.equal(layout.bands.length, 2, `${label} 退回带数量`);
        const band = layout.bands[0]!;
        const bandPoint = islandHorizontalEdge(edge)
            ? { x: band.left + band.width / 2, y: window.top + window.height / 2 }
            : { x: window.left + window.width / 2, y: band.top + band.height / 2 };
        assert.equal(islandHitRegion("expanded", bandPoint, layout, contour), "extra-band", `${label} 额外带必须退回`);
        assert.equal(islandHitRegion("hover", bandPoint, layout, contour), "transparent", `${label} 非展开态同一点只是透明边距`);
        assert.equal(islandHitRegion("expanded", { x: card.left + card.width / 2, y: card.top + card.height / 2 }, layout, contour), "content", `${label} 内容盒内保持内容`);
        assert.equal(islandHitRegion("expanded", { x: window.left - 0.5, y: window.top + window.height / 2 }, layout, contour), "outside", `${label} 窗口外不归浮岛`);
        const { track, thumb } = layout;
        assert.ok(track && thumb, `${label} 出条时必须有 track 与 thumb`);
        // 滚动条矩形从退回带里扣除：右侧轨道落在额外带内，但不能退回。
        const trackPoint = { x: track.left + track.width / 2, y: track.top + track.height / 2 };
        assert.ok(islandPointInRect(trackPoint, layout.window), `${label} 轨道必须落在并集窗口内`);
        if (islandHorizontalEdge(edge)) assert.ok(layout.bands.some((value) => islandPointInRect(trackPoint, value)), `${label} 轨道落在退回带内，扣除才有意义`);
        assert.equal(islandHitRegion("expanded", trackPoint, layout, contour), "scrollbar", `${label} 滚动条必须可交互`);
        // 未完成提交或未溢出时同一点不再归滚动条。
        const hidden = islandScrollGeometry(islandFrame("expanded", window), islandFrame("expanded", card), edge, { ...metrics, settled: false });
        assert.equal(hidden.visible, false, `${label} 未提交前不出条`);
        assert.equal(islandHitRegion("expanded", trackPoint, hidden, contour) === "scrollbar", false, `${label} 未出条时不得占用命中`);
        // 轨道与滑块都在并集窗口内，滑块不越出轨道，滚动位置线性映射。
        for (const point of [trackPoint, { x: thumb.left + thumb.width / 2, y: thumb.top + thumb.height / 2 }]) assert.ok(islandPointInRect(point, window), `${label} 滚动条必须落在并集窗口内`);
        assert.ok(thumb.height >= 24 - 1e-6 && thumb.height <= track.height + 1e-6, `${label} 滑块高度`);
        assert.ok(thumb.top >= track.top - 1e-6 && thumb.top + thumb.height <= track.top + track.height + 1e-6, `${label} 滑块不越出轨道`);
        const middle = islandScrollGeometry(islandFrame("expanded", window), islandFrame("expanded", card), edge, { ...metrics, scrollTop: layout.maxScroll / 2 });
        near(middle.thumb!.top, track.top + layout.travel / 2, `${label} 滑块行程按滚动比例`);
    }
    // 透明角落（圆角外）不触发退回：展开态仍是透明像素，不进入额外带。
    const window = { left: 100, top: 40, width: 420, height: 240 }, card = { left: 101, top: 40, width: 180, height: 120 };
    const layout = islandScrollGeometry(islandFrame("expanded", window), islandFrame("expanded", card), "top", { mode: "expanded", settled: true, scrollHeight: 300, clientHeight: 120, scrollTop: 0 });
    const contour = islandHitContour([{ rect: card, radii: [12, 12, 12, 12] }], window, "top", "capsule", 0);
    assert.equal(islandHitRegion("expanded", { x: card.left + 0.25, y: card.top + 0.25 }, layout, contour), "transparent");
});

test("展开滚动条只在溢出且已提交时出现，track/thumb 几何与滚动位置一致", () => {
    const window: IslandRect = { left: 100, top: 40, width: 200, height: 200 };
    const target: IslandRect = { left: 100, top: 40, width: 160, height: 120 };
    const frame = islandFrame("expanded", window), content = islandFrame("expanded", target);
    const metrics = { mode: "expanded" as const, settled: true, scrollHeight: 400, clientHeight: 200, scrollTop: 0 };
    const geometry = islandScrollGeometry(frame, content, "top", metrics);
    assert.equal(geometry.visible, true);
    // 三种非展开态即使内容溢出也不出条。
    for (const mode of ["hover", "collapsed"] as const) assert.equal(islandScrollGeometry(islandFrame(mode, window), content, "top", { ...metrics, mode }).visible, false);
    // 动画未结束或未提交、内容不溢出、视口过矮都不出条。
    assert.equal(islandScrollGeometry(frame, content, "top", { ...metrics, settled: false }).visible, false);
    assert.equal(islandScrollGeometry(frame, content, "top", { ...metrics, scrollHeight: 200 }).visible, false);
    assert.equal(islandScrollGeometry(frame, content, "top", { ...metrics, scrollHeight: 100 }).visible, false);
    assert.equal(islandScrollGeometry({ ...frame, windowBox: { ...window, height: 8 } }, content, "top", metrics).visible, false);
    // 窗口与目标缺省时客户区坐标相对固定视口，不另读 root.right。
    const offset = islandScrollGeometry(islandFrame("expanded", window, { left: 90, top: 30, width: 420, height: 300 }), undefined, "top", metrics);
    assert.deepEqual(offset.window, { left: 10, top: 10, width: 200, height: 200 });
    assert.deepEqual(offset.content, offset.window);
    assert.deepEqual(offset.bands, []);
    // 滑块到两端时严格贴轨道两端。
    const start = islandScrollGeometry(frame, content, "top", { ...metrics, scrollTop: 0 });
    near(start.thumb!.top, start.track!.top, "滑块顶端贴轨道顶端");
    const end = islandScrollGeometry(frame, content, "top", { ...metrics, scrollTop: 200 });
    near(end.thumb!.top + end.thumb!.height, end.track!.top + end.track!.height, "滑块底端贴轨道底端");
    // 滚动位置超出范围时夹紧，不越出轨道。
    assert.equal(islandScrollGeometry(frame, content, "top", { ...metrics, scrollTop: 9999 }).thumb!.top, end.thumb!.top);
    assert.equal(islandScrollGeometry(frame, content, "top", { ...metrics, scrollTop: -9999 }).thumb!.top, start.thumb!.top);
});

test("hover透明边距：连续停留满120ms才收起，途中回到内容盒立即取消", () => {
    const input = { mode: "hover" as const, region: "transparent" as const, busy: false, outlineDistance: 10, coreDistance: 10, now: 1000 };
    let step = islandMarginStep({ latch: "open" }, input);
    assert.equal(step.collapse, false, "刚进入透明区不能立即收起");
    assert.equal(step.state.since, 1000);
    step = islandMarginStep(step.state, { ...input, now: 1119 });
    assert.equal(step.collapse, false, "未满120ms不收起");
    step = islandMarginStep(step.state, { ...input, region: "content", now: 1120 });
    assert.equal(step.collapse, false, "回到内容盒必须取消计时");
    assert.equal(step.state.since, undefined);
    step = islandMarginStep(step.state, { ...input, now: 1300 });
    assert.equal(step.collapse, false, "取消后重新计时");
    step = islandMarginStep(step.state, { ...input, now: 1419 });
    assert.equal(step.collapse, false);
    step = islandMarginStep(step.state, { ...input, now: 1420 });
    assert.equal(step.collapse, true, "连续停留满120ms后回到胶囊");
    assert.equal(step.state.latch, "blocked", "收起同时落门闩");
    assert.equal(step.state.since, undefined);
    // 忙碌（原生拖动、排序、滚动条拖动）不推进也不收起。
    const busy = islandMarginStep({ latch: "open", since: 500 }, { ...input, busy: true });
    assert.equal(busy.collapse, false);
    assert.equal(busy.state.latch, "open");
});

test("展开态只有额外带触发退回，其余透明边距与滚动条不退回", () => {
    const base = { mode: "expanded" as const, now: 0, busy: false, outlineDistance: 10, coreDistance: 10 };
    assert.equal(islandMarginStep({ latch: "open" }, { ...base, region: "extra-band" }).collapse, true);
    for (const region of ["content", "transparent", "scrollbar"] as const) {
        const step = islandMarginStep({ latch: "open" }, { ...base, region });
        assert.equal(step.collapse, false, `${region} 不应触发展开退回`);
        assert.equal(step.state.latch, "open");
    }
    // 展开态不保留hover计时，收起后再进入也不沿用旧 since。
    assert.equal(islandMarginStep({ latch: "open", since: 10 }, { ...base, region: "scrollbar" }).state.since, undefined);
});

test("门闩：离开不透明轮廓≥4 DIP才解闩，回到内容需内缩4 DIP才允许再hover", () => {
    let state: IslandMarginState = { latch: "blocked" };
    const step = (over: Partial<Parameters<typeof islandMarginStep>[1]> = {}) => {
        const result = islandMarginStep(state, { mode: "collapsed", region: "content", now: 0, busy: false, outlineDistance: 0, coreDistance: 0, ...over });
        state = result.state;
        return result;
    };
    let result = step({ outlineDistance: ISLAND_REENTRY_MARGIN - 0.01 });
    assert.equal(result.state.latch, "blocked", "透明边距内移动不解闩");
    assert.equal(result.canEnter, false);
    result = step({ outlineDistance: ISLAND_REENTRY_MARGIN });
    assert.equal(result.state.latch, "outside", "离开不透明轮廓4 DIP后解闩");
    assert.equal(result.canEnter, false, "解闩不等于直接进入hover");
    result = step({ coreDistance: -ISLAND_REENTRY_MARGIN + 0.01 });
    assert.equal(result.state.latch, "outside", "仅贴轮廓不够，需内缩4 DIP");
    assert.equal(result.canEnter, false);
    result = step({ coreDistance: -ISLAND_REENTRY_MARGIN });
    assert.equal(result.state.latch, "open");
    assert.equal(result.canEnter, true, "解闩且进入内容盒后交给180ms计时");
    // 同一份指针输入若落在透明边距，即使解闩也不安排进入。
    state = { latch: "outside" };
    assert.equal(step({ region: "transparent", coreDistance: -6 }).canEnter, false);
    // 离开窗口：从未收起的门闩保持开放，已锁定门闩落回窗外，计时清除。
    assert.deepEqual(islandMarginStep({ latch: "open", since: 3 }, { mode: "collapsed", region: "outside", now: 9, busy: false, outlineDistance: 99, coreDistance: 99, leave: true }).state, { latch: "open" });
    assert.deepEqual(islandMarginStep({ latch: "blocked", since: 3 }, { mode: "collapsed", region: "outside", now: 9, busy: false, outlineDistance: 99, coreDistance: 99, leave: true }).state, { latch: "outside" });
});

test("门闩期间不安排hover进入，既有180ms进入消抖保持不变", () => {
    assert.equal(islandHoverTarget("collapsed", true, false, true), undefined);
    assert.equal(islandHoverTarget("collapsed", true, false, false), "hover");
    // hover 自身的回落不受门闩影响：只有收起后（collapsed + latched）才拦截。
    assert.equal(islandHoverTarget("hover", false, false, true), "collapsed");
    assert.equal(islandHoverTarget("expanded", true, false, true), undefined);
    assert.equal(ISLAND_HOVER_DELAY_MS, 180);
    assert.equal(ISLAND_MARGIN_DELAY_MS, 120);
    assert.equal(ISLAND_REENTRY_MARGIN, 4);
    assert.equal(ISLAND_SCROLLBAR_FADE_MS, 150);
});

test("debug临时卡片只在开发构建按查询参数注入且不超过8张", () => {
    assert.equal(islandDebugCardCount(false, "?island-debug=6"), 0, "发布构建不得注入");
    assert.equal(islandDebugCardCount(true, ""), 0, "默认不注入");
    assert.equal(islandDebugCardCount(true, "?island-debug=6"), 6);
    assert.equal(islandDebugCardCount(true, "?island-debug=99"), 8, "上限8张撑出溢出");
    assert.equal(islandDebugCardCount(true, "?island-debug=-3"), 0);
    assert.equal(islandDebugCardCount(true, "?island-debug=abc"), 0);
    assert.equal(islandDebugCardCount(true, "?island-debug=2.9"), 2);
    assert.equal(islandDebugCardCount(true, "?other=1"), 0);
});

test("收起收口先瞬时隐藏再提交，取消后两者都不执行", () => {
    const events: string[] = [];
    const apply = { hide: () => events.push("hide"), commit: () => events.push("commit") };
    // 正常 transitionend 与兜底计时共用同一次收口：隐藏无条件先于提交。
    assert.equal(islandCollapseSettle(7, 7, apply), true);
    assert.deepEqual(events, ["hide", "commit"], "隐藏必须先于提交状态");
    events.length = 0;
    // 已被反向请求或卸载取消：迟到回调既不得隐藏也不得提交。
    assert.equal(islandCollapseSettle(6, 7, apply), false, "token 失效后不得执行");
    assert.deepEqual(events, [], "取消后不得隐藏也不得提交");
    // 兜底路径不假定计时器到时条已透明：注入的 hide 是提交前的强制归零步骤。
    const fallback: string[] = [];
    let opacity = 0.5;
    const settle = { hide: () => { opacity = 0; fallback.push(`hide:${opacity}`); }, commit: () => fallback.push(`commit:${opacity}`) };
    assert.equal(islandCollapseSettle(1, 1, settle), true);
    assert.deepEqual(fallback, ["hide:0", "commit:0"], "兜底必须先瞬时隐藏到 0 再提交");
});
