import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { islandGeometry } from "./islandGeometry.ts";
import {
    ISLAND_HOVER_DELAY_MS,
    islandLeaveSuppressed,
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
    islandExpandedBlank, beginIslandExpandedGrace, islandExpandedGraceActive, ISLAND_EXPAND_GRACE_MS,
    islandExpandedDwellBlocked,
    islandHitRegion,
    islandExpandedDwellStep,
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
test("展开态卡片本体不启动窗口拖动", () => {
    assert.equal(islandDragIntent("expanded", 0, false, true), undefined);
    assert.equal(islandDragIntent("expanded", 0, true, true), undefined);
    assert.equal(islandDragIntent("expanded", 1, false, true), undefined);
    assert.equal(islandDragIntent("expanded", 0, false, false), undefined);
});
test("键盘排序落点沿同一身份列表移动", () => {
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

test("展开态透明带命中：只有并集减目标内容盒的额外带退回，卡片外侧滚动条槽整条扣除", () => {
    for (const edge of islandEdges) for (const form of islandForms) for (const dpr of islandDprs) {
        const { window, card } = islandLayoutCase(edge, form, dpr);
        const label = `${edge}/${form}/DPR${dpr}`;
        // 展开态统一浮岛圆角、不绘制补面，命中轮廓只含四角圆角。
        const contour = islandHitContour([{ rect: card, radii: [12, 12, 12, 12] }], window, edge, "capsule", 0);
        const metrics = { mode: "expanded" as const, settled: true, scrollHeight: 900, clientHeight: card.height, scrollTop: 0 };
        const layout = islandScrollGeometry(islandFrame("expanded", window), islandFrame("expanded", card), edge, metrics);
        assert.equal(layout.visible, true, `${label} 溢出且已提交时必须出条`);
        assert.equal(layout.bands.length, islandHorizontalEdge(edge) ? 2 : 4, `${label} 退回带数量`); // 左右贴边同时有上下与左右透明列。
        const band = layout.bands[0]!;
        const bandPoint = islandHorizontalEdge(edge)
            ? { x: band.left + band.width / 2, y: window.top + window.height / 2 }
            : { x: window.left + window.width / 2, y: band.top + band.height / 2 };
        assert.equal(islandHitRegion("expanded", bandPoint, layout, contour), "extra-band", `${label} 额外带必须退回`);
        assert.equal(islandHitRegion("hover", bandPoint, layout, contour), "transparent", `${label} 非展开态同一点只是透明边距`);
        assert.equal(islandHitRegion("expanded", { x: card.left + card.width / 2, y: card.top + card.height / 2 }, layout, contour), "content", `${label} 内容盒内保持内容`);
        assert.equal(islandHitRegion("expanded", { x: layout.window.left - 0.5, y: layout.window.top + layout.window.height / 2 }, layout, contour), "outside", `${label} 视口外不归浮岛`);
        const { track, thumb } = layout;
        assert.ok(track && thumb, `${label} 出条时必须有 track 与 thumb`);
        // 滚动条矩形从退回带里扣除：右侧轨道落在额外带内，但不能退回。
        const trackPoint = { x: track.left + track.width / 2, y: track.top + track.height / 2 };
        assert.ok(islandPointInRect(trackPoint, layout.window), `${label} 轨道必须落在并集窗口内`);
        assert.equal(layout.bands.some((value) => islandPointInRect(trackPoint, value)), false, `${label} 轨道在内容列内，不属于透明带`); // 槽先判 scrollbar，不靠退回带扣除。
        assert.equal(islandHitRegion("expanded", trackPoint, layout, contour), "scrollbar", `${label} 滚动条必须可交互`);
        // 未完成提交或未溢出时同一点不再归滚动条。
        const hidden = islandScrollGeometry(islandFrame("expanded", window), islandFrame("expanded", card), edge, { ...metrics, settled: false });
        assert.equal(hidden.visible, false, `${label} 未提交前不出条`);
        // 滚动条槽在卡片外，整条槽（间距 + 轨道）都不触发退回；未出条时槽仍是展示区，不把指针判成退回。
        assert.ok(layout.gutter && islandPointInRect(trackPoint, layout.gutter), `${label} 轨道必须落在滚动条槽内`);
        assert.equal(islandHitRegion("expanded", trackPoint, hidden, contour), "scrollbar", `${label} 未出条时槽仍不触发退回`);
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

test("展开回退计时只响应跨边界，空白列移动不会重新开始", () => {
    assert.deepEqual(islandExpandedDwellStep("idle", "blank"), { state: "idle", timer: undefined });
    assert.deepEqual(islandExpandedDwellStep("idle", "content"), { state: "content", timer: undefined });
    assert.deepEqual(islandExpandedDwellStep("content", "blank"), { state: "blank", timer: "start" });
    for (let i = 0; i < 10; i += 1) assert.deepEqual(islandExpandedDwellStep("blank", "blank"), { state: "blank", timer: undefined });
    assert.deepEqual(islandExpandedDwellStep("blank", "content"), { state: "content", timer: "cancel" });
    assert.deepEqual(islandExpandedDwellStep("blank", "protected"), { state: "idle", timer: "cancel" });
    assert.deepEqual(islandExpandedDwellStep("content", "protected"), { state: "content", timer: undefined });
    assert.equal(ISLAND_MARGIN_DELAY_MS, 120);
});

test("全高展开列与工作区同高时不再伪造上下空白带，只保留列外透明区", () => {
    for (const edge of islandEdges) {
        const window = { left: 0, top: 0, width: 216, height: 900 };
        const frame = { ...islandFrame("expanded", window, window), cards: [{ rect: { left: 0, top: 0, width: 200, height: 900 }, radius: 12 }] };
        const metrics = { mode: "expanded" as const, settled: true, scrollHeight: 900, clientHeight: 900, scrollTop: 0 };
        const layout = islandScrollGeometry(frame, frame, edge, metrics);
        assert.deepEqual(layout.bands, [], `${edge} 全高内容列没有上下余量`);
        const contour = islandHitContour([{ rect: { left: 0, top: 0, width: 200, height: 900 }, radii: [12, 12, 12, 12] }], window, edge, "capsule", 0);
        const classify = (x: number, y: number) => islandHitRegion("expanded", { x, y }, layout, contour);
        assert.equal(classify(100, 500), "content");
        assert.equal(classify(layout.gutter!.left + 2, 500), "scrollbar");
        const input = { mode: "expanded" as const, region: "content" as const, now: 0, busy: false, outlineDistance: 1, coreDistance: 1 };
        const entered = islandMarginStep({ latch: "open" }, input);
        assert.equal(islandMarginStep(entered.state, { ...input, now: 1000 }).collapse, false); // 列内 1s 不收回。
    }
});

test("展开四边 + 非零 offset 使用同一工作区坐标系：列内非空白、列外空白、1s 不收回", () => {
    const work = { left: 0, top: 0, width: 2560, height: 1400 };
    const card = { left: 0, top: 0, width: 320, height: 108 };
    for (const edge of islandEdges) {
        const request = islandGeometry("expanded", [card], 12, work, edge, 120);
        const frame = { ...request, viewport: { left: 2140, top: 0, width: 420, height: 1400 } }; // 模拟仍拿旧窄视口 frame。
        const metrics = { mode: "expanded" as const, settled: true, scrollHeight: 1400, clientHeight: 1400, scrollTop: 0 };
        const layout = islandScrollGeometry(frame, request, edge, metrics, work);
        const columnX = request.windowBox.left - work.left;
        const inside = { x: columnX + 10, y: 20 };
        const outsideLeft = { x: Math.max(0, columnX - 10), y: 20 };
        const outsideRight = { x: columnX + request.windowBox.width + 10, y: 20 };
        assert.equal(islandExpandedBlank(layout, inside, false), false, `${edge} ${columnX} 列内`);
        assert.equal(islandExpandedBlank(layout, outsideLeft, false), columnX > 0, `${edge} 列左空白`);
        assert.equal(islandExpandedBlank(layout, outsideRight, false), outsideRight.x < work.width, `${edge} 列右空白`);
        assert.equal(layout.window.width, work.width, `${edge} 命中窗口=工作区宽`);
        const input = { mode: "expanded" as const, region: "content" as const, now: 0, busy: false, outlineDistance: 1, coreDistance: 1 };
        const entered = islandMarginStep({ latch: "open" }, input);
        assert.equal(islandMarginStep(entered.state, { ...input, now: 1000 }).collapse, false, `${edge} 1s 不收回`);
    }
});

test("展开滚动条只在溢出且已提交时出现，track/thumb 几何与滚动位置一致", () => {
    const window: IslandRect = { left: 100, top: 40, width: 200, height: 200 };
    const target: IslandRect = { left: 100, top: 40, width: 160, height: 120 };
    const frame = islandFrame("expanded", window, window), content = islandFrame("expanded", target, window);
    const metrics = { mode: "expanded" as const, settled: true, scrollHeight: 400, clientHeight: 200, scrollTop: 0 };
    const geometry = islandScrollGeometry(frame, content, "top", metrics);
    assert.equal(geometry.visible, true);
    // 三种非展开态即使内容溢出也不出条。
    for (const mode of ["hover", "collapsed"] as const) assert.equal(islandScrollGeometry(islandFrame(mode, window), content, "top", { ...metrics, mode }).visible, false);
    // 动画未结束或未提交、内容不溢出、视口过矮都不出条。
    assert.equal(islandScrollGeometry(frame, content, "top", { ...metrics, settled: false }).visible, false);
    assert.equal(islandScrollGeometry(frame, content, "top", { ...metrics, scrollHeight: 200 }).visible, false);
    assert.equal(islandScrollGeometry(frame, content, "top", { ...metrics, scrollHeight: 100 }).visible, false);
    assert.equal(islandScrollGeometry({ ...frame, windowBox: { ...window, height: 8 }, viewport: { ...window, height: 8 } }, content, "top", metrics).visible, false); // 视口过矮不出条。
    // 窗口与目标缺省时客户区坐标相对固定视口，不另读 root.right。
    const offset = islandScrollGeometry(islandFrame("expanded", window, { left: 90, top: 30, width: 420, height: 300 }), undefined, "top", metrics);
    assert.deepEqual(offset.window, { left: 0, top: 0, width: 420, height: 300 }); // 展开视口铺满工作区。
    assert.deepEqual(offset.content, { left: 10, top: 10, width: 200, height: 200 });
    assert.equal(offset.bands.length, 2); // 内容列左右两侧的透明区。
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

test("展开态与hover共用同一条120ms去抖：进带计时、回内容取消、满120ms才退回", () => {
    const base = { mode: "expanded" as const, region: "extra-band" as const, busy: false, outlineDistance: 10, coreDistance: 10 };
    let step = islandMarginStep({ latch: "open" }, { ...base, now: 0 });
    assert.equal(step.collapse, false, "刚进入额外带不能立即退回");
    assert.equal(step.state.since, 0, "进入额外带开始计时");
    step = islandMarginStep(step.state, { ...base, now: ISLAND_MARGIN_DELAY_MS - 1 });
    assert.equal(step.collapse, false, "未满120ms不退回");
    // 回到内容盒取消计时。
    step = islandMarginStep(step.state, { ...base, region: "content", now: ISLAND_MARGIN_DELAY_MS });
    assert.equal(step.collapse, false, "计时内回到内容盒取消");
    assert.equal(step.state.since, undefined);
    // 重新进带后从新的时间点计时；满 120ms 才退回。
    step = islandMarginStep(step.state, { ...base, now: 400 });
    assert.equal(step.state.since, 400);
    step = islandMarginStep(step.state, { ...base, now: 519 });
    assert.equal(step.collapse, false);
    step = islandMarginStep(step.state, { ...base, now: 520 });
    assert.equal(step.collapse, true, "连续停留满120ms后退回");
    assert.equal(step.state.since, undefined);
    // 其余透明边距与滚动条不触发也不计时；滚动条仍从判定区扣除。
    for (const region of ["content", "transparent", "scrollbar"] as const) {
        const other = islandMarginStep({ latch: "open" }, { ...base, region, now: 0 });
        assert.equal(other.collapse, false, `${region} 不应触发展开退回`);
        assert.equal(other.state.since, undefined, `${region} 不应开始计时`);
    }
    // 与 hover 共用同一常量，120ms 只有一处定义。
    assert.equal(ISLAND_MARGIN_DELAY_MS, 120);
});

test("常态关闭透明检测：detect=false 不写门闩、不产生 collapse/canEnter 副作用", () => {
    const state: IslandMarginState = { latch: "blocked", since: 5 };
    for (const region of ["transparent", "content", "extra-band", "scrollbar", "outside"] as const) {
        const step = islandMarginStep(state, { mode: "collapsed", region, now: 999, busy: false, outlineDistance: 99, coreDistance: -99, detect: false });
        assert.equal(step.collapse, false, `${region} 不得触发退回`);
        assert.equal(step.canEnter, false, `${region} 不得安排进入`);
        assert.deepEqual(step.state, { latch: "blocked" }, `${region} 不得写门闩或计时`);
    }
    // 关闭检测时离开窗口也不改门闩（常态不评估透明判定）。
    assert.deepEqual(
        islandMarginStep(state, { mode: "collapsed", region: "outside", now: 999, busy: false, outlineDistance: Infinity, coreDistance: Infinity, leave: true, detect: false }).state,
        { latch: "blocked" },
    );
    // 开启检测时逻辑保留：离开轮廓 4 DIP 解闩、回到内容内缩 4 DIP 才安排进入。
    const open = islandMarginStep({ latch: "blocked" }, { mode: "collapsed", region: "content", now: 0, busy: false, outlineDistance: 4, coreDistance: -4 });
    assert.equal(open.state.latch, "open");
    assert.equal(open.canEnter, true);
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

test("展开视口铺满工作区：透明区停留/点击回退，内容列与拖动例外", () => {
    const work = { left: 0, top: 0, width: 800, height: 600 };
    const column = { left: 220, top: 0, width: 360, height: 600 };
    const frame = { ...islandFrame("expanded", column, work), cards: [{ rect: { left: 0, top: 0, width: 360, height: 200 }, radius: 12 }] };
    const layout = islandScrollGeometry(frame, frame, "top", { mode: "expanded", settled: true, scrollHeight: 600, clientHeight: 600, scrollTop: 0 });
    assert.deepEqual(layout.window, work); // 命中窗口等于工作区，不再只有 420 带。
    assert.equal(islandExpandedBlank(layout, { x: 50, y: 300 }, false), true); // 内容列左侧透明区。
    assert.equal(islandExpandedBlank(layout, { x: 400, y: 300 }, false), false); // 内容列内不触发。
    assert.equal(islandExpandedBlank(layout, { x: 50, y: 300 }, true), false); // 卡片上的操作不触发。
    const input = { mode: "expanded" as const, region: "extra-band" as const, busy: false, outlineDistance: 1, coreDistance: 1 };
    const entered = islandMarginStep({ latch: "open" }, { ...input, now: 0 });
    assert.equal(islandMarginStep(entered.state, { ...input, now: 119 }).collapse, false);
    assert.equal(islandMarginStep(entered.state, { ...input, now: 120 }).collapse, true); // 连续停留 120ms 回胶囊。
    const source = readFileSync(new URL("./island.tsx", import.meta.url), "utf8");
    assert.match(source, /islandExpandedBlank\(layout, \{ x: event\.clientX, y: event\.clientY \}, onCard\)/); // 透明区单击直接回退。
    assert.match(source, /event\.type !== "pointerup" \|\| transitionMasked\.current \|\| modeRef\.current !== "expanded" \|\| cardSorting\.current/); // 拖动中不回退。
});

test("展开工作区左右透明列都可回退：左贴边、居中、右贴边", () => {
    const work = { left: 0, top: 0, width: 800, height: 600 };
    for (const [edge, column] of [["left", { left: 0, width: 360 }], ["top", { left: 220, width: 360 }], ["right", { left: 440, width: 360 }]] as const) {
        const content = { left: column.left, top: 0, width: column.width, height: 600 };
        const frame = { ...islandFrame("expanded", content, work), cards: [{ rect: { left: 0, top: 0, width: column.width, height: 200 }, radius: 12 }] };
        const layout = islandScrollGeometry(frame, frame, edge, { mode: "expanded", settled: true, scrollHeight: 600, clientHeight: 600, scrollTop: 0 });
        assert.ok(layout.bands.length >= 1, `${column.left} 至少一侧有透明带`);
        for (let x = 0; x < 800; x += 10) {
            const blank = islandExpandedBlank(layout, { x, y: 500 }, false);
            const inside = x >= column.left && x < column.left + column.width;
            assert.equal(blank, !inside, `x=${x} 内容列内不退回、列外可回退`);
        }
        assert.equal(islandExpandedBlank(layout, { x: column.left + 10, y: 20 }, true), false); // 卡片上不退回。
        assert.equal(islandExpandedBlank(layout, { x: layout.gutter!.left + 2, y: 500 }, false), false); // 滚动条槽不退回。
    }
});

test("拖卡排序期间透明区 119/120ms 都不回退，结束后恢复，Esc 仍取消", () => {
    const base = { mode: "expanded" as const, region: "extra-band" as const, outlineDistance: 1, coreDistance: 1 };
    const sorting = islandExpandedDwellBlocked(true, false);
    assert.equal(sorting, true);
    const during = islandMarginStep({ latch: "open" }, { ...base, busy: sorting, now: 0 });
    assert.equal(islandMarginStep(during.state, { ...base, busy: sorting, now: 119 }).collapse, false);
    assert.equal(islandMarginStep(during.state, { ...base, busy: sorting, now: 120 }).collapse, false); // 第二指针拖排中不回退。
    assert.equal(islandExpandedDwellBlocked(false, true), true); // 其它交互忙同样阻止。
    const after = islandMarginStep({ latch: "open" }, { ...base, busy: false, now: 0 });
    assert.equal(islandMarginStep(after.state, { ...base, busy: false, now: 119 }).collapse, false);
    assert.equal(islandMarginStep(after.state, { ...base, busy: false, now: 120 }).collapse, true); // 排序结束恢复 120ms。
    const source = readFileSync(new URL("./island.tsx", import.meta.url), "utf8");
    assert.match(source, /if \(cardSorting\.current\) \{\s*cancelExpandedReturn\(true\); \/\/ 拖卡排序期间第二指针/);
    assert.match(source, /islandExpandedDwellBlocked\(cardSorting\.current, interactionBusy\(\)\)/); // 到点再检查。
    assert.match(source, /suppressClick\.current = false;\s*cancelExpandedReturn\(true\); \/\/ 排序结束/);
    assert.match(source, /if \(cardSorting\.current\) \{ endCardSort\(\); return; \} \/\/ Esc/); // 不拦截取消。
});

test("触发展开后的同一次交互在 300ms/4 DIP 内不触发空白回退", () => {
    const start = beginIslandExpandedGrace(1000, { x: 400, y: 300 });
    assert.equal(ISLAND_EXPAND_GRACE_MS, 300);
    assert.equal(islandExpandedGraceActive(start, 1299, { x: 400, y: 300 }), true); // 同一指针未移动。
    assert.equal(islandExpandedGraceActive(start, 1300, { x: 400, y: 300 }), false); // 到点恢复。
    assert.equal(islandExpandedGraceActive(start, 1100, { x: 405, y: 300 }), false); // 移动超过 4 DIP 立即恢复。
    assert.equal(islandExpandedGraceActive(start, 1100, { x: 404, y: 300 }), true); // 4 DIP 内仍抑制。
    const keyboard = beginIslandExpandedGrace(1000); // Enter 展开没有指针起点，按时间窗抑制。
    assert.equal(islandExpandedGraceActive(keyboard, 1200, { x: 900, y: 900 }), true);
    assert.equal(islandExpandedGraceActive(keyboard, 1300, { x: 900, y: 900 }), false);

    const source = readFileSync(new URL("./island.tsx", import.meta.url), "utf8");
    assert.match(source, /expandedGrace\.current = beginIslandExpandedGrace\(performance\.now\(\), \{ x: event\.clientX, y: event\.clientY \}\);/); // 点击展开登记。
    assert.match(source, /expandedGrace\.current = beginIslandExpandedGrace\(performance\.now\(\)\);/); // Enter 展开登记。
    assert.match(source, /if \(islandExpandedGraceActive\(expandedGrace\.current, performance\.now\(\), gracePoint\)\) return;/); // 抑制 120ms 停留。
    assert.match(source, /if \(islandExpandedGraceActive\(expandedGrace\.current, performance\.now\(\), \{ x: event\.clientX, y: event\.clientY \}\)\) return;/); // 抑制空白单击。
    assert.match(source, /if \(cardSorting\.current\) \{\s*cancelExpandedReturn\(true\); \/\/ 拖卡排序期间第二指针/); // 拖排例外未改。
});

test("卡片单击展开与拖拽解耦，控件/移动/拖排不触发", () => {
    const source = readFileSync(new URL("./island.tsx", import.meta.url), "utf8");
    assert.match(source, /const tapCandidate = useRef<\{ id: number; x: number; y: number; mode: IslandMode \} \| undefined>\(undefined\);/);
    assert.match(source, /const control = target\.closest\("button, a, input, textarea, select, \[role='button'\], \[data-island-card-handle\]"\);/); // 控件不记候选。
    assert.match(source, /tapCandidate\.current = event\.button === 0 && !control \? \{ id: event\.pointerId, x: event\.screenX, y: event\.screenY, mode: modeRef\.current \} : undefined;/);
    assert.match(source, /const tap = tapCandidate\.current;/);
    assert.match(source, /const canExpand = event\.type === "pointerup" && tap && tap\.id === event\.pointerId && !suppressClick\.current && !moved && hit && root\.current\?\.contains\(hit\) && !onControl;/); // 卡片点击走 tap，不走 gesture。
    assert.match(source, /expandedGrace\.current = beginIslandExpandedGrace\(performance\.now\(\), \{ x: event\.clientX, y: event\.clientY \}\);/); // 保留 t126 grace。
    assert.match(source, /const intent = islandDragIntent\(mode, event\.button, Boolean\(target\.closest\(ISLAND_NO_DRAG\)\), Boolean\(target\.closest\("\.island-card"\)\)\);/); // 拖拽路径未改。
    assert.equal(islandDragMoved({ x: 0, y: 0 }, { x: 3, y: 0 }), false);
    assert.equal(islandDragMoved({ x: 0, y: 0 }, { x: 5, y: 0 }), true); // 超过 4 DIP 不展开。
});

test("工作区 2560、列 1092..1468、指针 1250,60 判为列内，300,60 为列外；1s 不收回", () => {
    const work = { left: 0, top: 0, width: 2560, height: 1400 };
    const column = { left: 1092, top: 0, width: 376, height: 1400 };
    const frame = { ...islandFrame("expanded", column, { left: 2140, top: 0, width: 420, height: 1400 }), cards: [{ rect: { left: 1149, top: 0, width: 262, height: 108 }, radius: 12 }] };
    const layout = islandScrollGeometry(frame, frame, "top", { mode: "expanded", settled: true, scrollHeight: 1400, clientHeight: 1400, scrollTop: 0 }, work);
    assert.equal(islandExpandedBlank(layout, { x: 1250, y: 60 }, false), false); // 指针在卡片/列内。
    assert.equal(islandExpandedBlank(layout, { x: 300, y: 60 }, false), true); // 列外空白。
    assert.equal(layout.column.left, 1092);
    assert.equal(layout.window.width, 2560);
    const input = { mode: "expanded" as const, region: "content" as const, now: 0, busy: false, outlineDistance: 1, coreDistance: 1 };
    const entered = islandMarginStep({ latch: "open" }, input);
    assert.equal(islandMarginStep(entered.state, { ...input, now: 1000 }).collapse, false);
});

test("假 pointerleave：遮蔽/grace/尺寸变化/坐标仍在窗口或列内时忽略，真离开才收", () => {
    const window = { left: 0, top: 0, width: 2560, height: 1400 };
    const column = { left: 1092, top: 0, width: 376, height: 1400 };
    const base = { masked: false, graceActive: false, resizedAt: 0, now: 1000 };
    assert.equal(islandLeaveSuppressed({ ...base, point: { x: 1180, y: 80 }, windowRect: window }), true); // 指针仍在岛内。
    assert.equal(islandLeaveSuppressed({ ...base, point: { x: 300, y: 80 }, windowRect: window, contentRect: column }), true); // 工作区内透明列也非离岛。
    assert.equal(islandLeaveSuppressed({ ...base, point: { x: 3000, y: 80 }, windowRect: window }), false); // 真移出窗口。
    assert.equal(islandLeaveSuppressed({ ...base, masked: true, point: { x: 3000, y: 80 } }), true);
    assert.equal(islandLeaveSuppressed({ ...base, graceActive: true, point: { x: 3000, y: 80 } }), true);
    assert.equal(islandLeaveSuppressed({ ...base, resizedAt: 900, now: 1100, point: { x: 3000, y: 80 } }), true); // 尺寸变化 300ms 内忽略。
    assert.equal(islandLeaveSuppressed({ ...base, resizedAt: 700, now: 1100, point: { x: 3000, y: 80 } }), false);

    const source = readFileSync(new URL("./island.tsx", import.meta.url), "utf8");
    assert.match(source, /const leaveIsFake = \(event\?: \{ screenX: number; screenY: number \}\): boolean =>/);
    assert.match(source, /if \(leaveIsFake\(event\)\) return; \/\/ 窗口放大\/刚展开时的 pointerleave 不是真正离岛；真实屏幕坐标在盒外仍放行。/);
    assert.match(source, /if \(leaveIsFake\(event\)\) return; \/\/ 假离开不撤触发者、不收起；屏幕坐标在盒外才继续。/);
    assert.match(source, /geometryResizeAt\.current = performance\.now\(\); \/\/ 原生窗口即将换尺寸/);
    assert.match(source, /lastPointerPoint\.current = \{ x: event\.clientX, y: event\.clientY \};/);
});

test("真离开用事件屏幕坐标：hover 窗口外 leave 收回，岛内/遮蔽/grace 忽略", () => {
    const hoverWindow = { left: 100, top: 100, width: 286, height: 108 };
    const base = { masked: false, graceActive: false, resizedAt: 0, now: 1000 };
    assert.equal(islandLeaveSuppressed({ ...base, point: { x: 300, y: 800 }, windowRect: hoverWindow }), false); // 屏幕别处真离开。
    assert.equal(islandLeaveSuppressed({ ...base, point: { x: 180, y: 150 }, windowRect: hoverWindow }), true); // 仍在 hover 窗口。
    assert.equal(islandLeaveSuppressed({ ...base, point: { x: 1180, y: 80 }, windowRect: { left: 0, top: 0, width: 2560, height: 1400 } }), true); // 展开工作区内。
    assert.equal(islandLeaveSuppressed({ ...base, masked: true, point: { x: 300, y: 800 }, windowRect: hoverWindow }), true);
    assert.equal(islandLeaveSuppressed({ ...base, graceActive: true, point: { x: 300, y: 800 }, windowRect: hoverWindow }), true);

    const source = readFileSync(new URL("./island.tsx", import.meta.url), "utf8");
    assert.match(source, /const eventPoint = event && Number\.isFinite\(event\.screenX\)/); // 事件屏幕坐标优先。
    assert.match(source, /const point = eventPoint \?\? lastPointerScreen\.current;/); // last 仅兜底。
    assert.match(source, /const windowRect = expanded \? expandedWork : frame\?\.windowBox;/); // 屏幕窗口盒。
    assert.match(source, /leaveWindow\(event\);/);
});

test("收起回胶囊后复位 latch/gate，离开再进入可再次 hover", () => {
    const source = readFileSync(new URL("./island.tsx", import.meta.url), "utf8");
    assert.match(source, /marginState\.current = \{ latch: next === "collapsed" \|\| !retainLatch \? "open" : "blocked" \}; \/\/ 回胶囊必须清 latch/);
    assert.match(source, /else if \(expandedDwellClosing\.current\) \{\s*expandedReentryGate\.current = true;/);
    assert.match(source, /\}, 600\); \/\/ 只在刚收起后的极短窗口阻止旧指针立即回 hover。/);
    assert.match(source, /expandedReentryGate\.current = false; \/\/ 真正离开后不再阻止下一次 hover。/);
    assert.match(source, /if \(modeRef\.current === "collapsed"\) marginState\.current = \{ latch: "open" \};/);
    assert.match(source, /expandedReentryGate\.current = false; \/\/ 重新进入说明已离开过，解除极短 gate。/);
    assert.match(source, /const hoverLatched = \(\) => modeRef\.current !== "collapsed" && marginState\.current\.latch !== "open";/); // hover 判定仍读同一 latch。
});
