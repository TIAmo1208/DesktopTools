import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import {
    islandHoverTravel, beginIslandMask, unmaskIslandTransition, releaseIslandMask, holdsIslandMask, beginIslandNativeSwap, finishIslandNativeSwap, canReleaseIslandMask,
    shouldMaskGeometryChange, islandGeometryCommitDecision, type IslandGeometrySeat,
    islandViewportMatched, awaitIslandViewport, islandFrameOrTimeout, islandRenderGate, revokeIslandReservation, ISLAND_NO_FRAME_TIMEOUT_MS, deferIslandMaskRelease, takeIslandMaskReleases, canResyncIslandGeometry, islandRetrySubmit, resetIslandGeometryBaseline,
    islandGeometryKey, islandFailedGeometryGate, islandWakingFrame,
    islandMotionFrame,
    islandMotionProgress,
    islandUnionBox,
    islandUnionFrame,
    islandAttachedCornerAt,
    ISLAND_MOTION_MS,
    ISLAND_EXPAND_STAGGER_MS,
    ISLAND_FRAME_MS,
    ISLAND_HOVER_DETAIL_EXIT_MS,
    ISLAND_HOVER_DETAIL_ENTER_MS,
} from "./islandMotion.ts";
import { islandExpandedColumn, islandFilletPatches, islandGeometry } from "./islandGeometry.ts";
import { parseIslandStateRequest, type IslandStateRequest } from "../../contracts/island.ts";

/** 读取被测源码：CI 的 Windows 检出是 CRLF，逐行定位与片段切分统一归一为 LF 再做。 */
const 读源码 = (relative: string) => readFileSync(new URL(relative, import.meta.url), "utf8").replace(/\r\n/g, "\n");
/** 提交门的源码：租约、在途提交、挂起解除与就绪/形变事实都在那个 module 里。 */
const 读提交门 = () => 读源码("./islandCommitGate.ts");

const work = { left: 0, top: 40, width: 1000, height: 800 };
const collapsed = [{ left: 0, top: 0, width: 160, height: 44 }];
const expanded = [
    { left: 0, top: 0, width: 360, height: 233 },
    { left: 0, top: 241, width: 360, height: 120 },
];
test("hover 详情按锚点顺序左右接力，不为失效或同一身份编排动画", () => {
    assert.equal(ISLAND_HOVER_DETAIL_EXIT_MS, 140);
    assert.equal(ISLAND_HOVER_DETAIL_ENTER_MS, 180);
    const ids = ["music", "notifications", "plugin"];
    assert.deepEqual(islandHoverTravel(ids, "music", "plugin"), { exitX: -4, enterX: 4 });
    assert.deepEqual(islandHoverTravel(ids, "plugin", "notifications"), { exitX: 4, enterX: -4 });
    assert.equal(islandHoverTravel(ids, "music", "music"), undefined);
    assert.equal(islandHoverTravel(ids, undefined, "plugin"), undefined);
    assert.equal(islandHoverTravel(ids, "gone", "music"), undefined);
    assert.equal(islandHoverTravel(ids, "music", "gone"), undefined);
    assert.equal(islandHoverTravel([], undefined, undefined), undefined);
});

test("形变时长与帧预算有界，曲线只在终点前轻微超出", () => {
    assert.equal(ISLAND_MOTION_MS, 240);
    assert.ok(ISLAND_FRAME_MS >= 1000 / 60);
    const samples = Array.from({ length: 1001 }, (_, i) => islandMotionProgress((i * ISLAND_MOTION_MS) / 1000));
    assert.equal(samples[0], 0);
    assert.equal(samples.at(-1), 1);
    assert.ok(Math.max(...samples) > 1.04);
    assert.ok(Math.max(...samples) <= 1.06);
    assert.ok(Math.min(...samples) >= 0);
    assert.equal(islandMotionProgress(-10), 0);
    assert.equal(islandMotionProgress(5000), 1);
});
test("所有贴边方向与默认中心采样均为合法完整原生请求，端点精确收敛", () => {
    for (const edge of ["", "top", "bottom", "left", "right"] as const) {
        const from = islandGeometry("collapsed", collapsed, 22, work, edge, 50);
        const to = islandGeometry("expanded", expanded, 12, work, edge, 50);
        for (const [a, b] of [
            [from, to],
            [to, from],
        ] as const) {
            for (let elapsed = 0; elapsed <= ISLAND_MOTION_MS; elapsed++) {
                const frame = islandMotionFrame(a, b, elapsed, edge, work);
                assert.deepEqual(parseIslandStateRequest(frame), frame);
                assert.ok(frame.windowBox.width <= Math.max(a.windowBox.width, b.windowBox.width) * 1.06);
                assert.ok(frame.windowBox.height <= Math.max(a.windowBox.height, b.windowBox.height) * 1.06);
                const epsilon = 1e-8;
                if (elapsed === ISLAND_MOTION_MS && b.state === "expanded") {
                    assert.equal(frame.windowBox.top, work.top);
                    assert.equal(frame.windowBox.height, work.height);
                    continue;
                }
                const horizontalEdge = edge === "left" || edge === "right";
                if (horizontalEdge) {
                    const desiredTop = b.windowBox.top + (b.windowBox.height - frame.windowBox.height) / 2;
                    assert.ok(
                        Math.abs(frame.windowBox.top - Math.max(work.top, Math.min(desiredTop, work.top + work.height - frame.windowBox.height))) < epsilon,
                        `${edge} 沿竖直方向居中并保持在工作区内`,
                    );
                    if (edge === "right") assert.ok(Math.abs(frame.windowBox.left + frame.windowBox.width - (b.windowBox.left + b.windowBox.width)) < epsilon);
                    else assert.equal(frame.windowBox.left, b.windowBox.left);
                } else {
                    assert.ok(
                        Math.abs(frame.windowBox.left + frame.windowBox.width / 2 - (b.windowBox.left + b.windowBox.width / 2)) < epsilon,
                        `${edge || "top"} 沿水平方向居中`,
                    );
                    if (edge === "bottom") assert.ok(Math.abs(frame.windowBox.top + frame.windowBox.height - Math.max(work.top + frame.windowBox.height, b.windowBox.top + b.windowBox.height)) < epsilon);
                    else assert.equal(frame.windowBox.top, Math.min(work.top + work.height - frame.windowBox.height, b.windowBox.top));
                }
            }
            assert.deepEqual(islandMotionFrame(a, b, ISLAND_MOTION_MS, edge, work), b);
        }
    }
});
test("并集动画期间原生窗口固定，内容只在并集内移动", () => {
    for (const edge of ["", "top", "bottom", "left", "right"] as const) {
        const from = islandGeometry("collapsed", collapsed, 22, work, edge, 50);
        const to = islandGeometry("expanded", expanded, 12, work, edge, 50);
        for (const [a, b] of [
            [from, to],
            [to, from],
        ] as const) {
            const union = islandUnionBox(a, b, work);
            for (let elapsed = 0; elapsed < ISLAND_MOTION_MS; elapsed += 10) {
                const frame = islandUnionFrame(a, b, elapsed, edge, work);
                assert.deepEqual(frame.windowBox, union); // 窗口盒在动画期间只取并集，不逐帧移动。
                for (const card of frame.cards) {
                    assert.ok(card.rect.left >= 0 && card.rect.top >= 0);
                    assert.ok(card.rect.left + card.rect.width <= union.width + 1e-6);
                    assert.ok(card.rect.top + card.rect.height <= union.height + 1e-6);
                }
                assert.deepEqual(parseIslandStateRequest(frame), frame);
            }
            const final = islandUnionFrame(a, b, ISLAND_MOTION_MS, edge, work);
            assert.deepEqual(final, b); // 展开终态也精确收口，不能保留起点的较宽盒。
            if (b.state === "expanded") assert.equal(final.windowBox.height, work.height);
        }
    }
});

test("四边两形态宽起点到窄展开先走并集，终态只保留全高目标列宽", () => {
    for (const edge of ["top", "bottom", "left", "right"] as const) {
        for (const form of ["capsule", "notch"] as const) {
            for (const state of ["collapsed", "hover"] as const) {
                const from = islandGeometry(state, [{ left: 0, top: 0, width: 300, height: 300 }], 16, work, edge, 0, form);
                const to = islandGeometry("expanded", [{ left: 0, top: 0, width: 180, height: 180 }], 12, work, edge, 0, form);
                const final = islandUnionFrame(from, to, ISLAND_MOTION_MS, edge, work);
                assert.deepEqual(islandUnionFrame(from, to, 0, edge, work).windowBox, islandUnionBox(from, to, work));
                assert.deepEqual(final.windowBox, to.windowBox, `${edge}/${form}/${state} 展开终态取目标列宽和工作区高度`);
                assert.deepEqual(parseIslandStateRequest(final), final);
                const card = final.cards[0]!;
                assert.equal(final.windowBox.left + card.rect.left, to.windowBox.left + to.cards[0]!.rect.left);
                assert.equal(final.windowBox.top + card.rect.top, to.windowBox.top + to.cards[0]!.rect.top);
                assert.equal(card.rect.width, to.cards[0]!.rect.width);
                assert.equal(card.rect.height, to.cards[0]!.rect.height);
                const next = islandUnionFrame(final, from, 0, edge, work);
                assert.deepEqual(next.windowBox, islandUnionBox(final, from, work), "退出形变只取一次并集，工作区高度不变");
                assert.equal(next.windowBox.left + next.cards[0]!.rect.left, final.windowBox.left + final.cards[0]!.rect.left);
                assert.equal(next.windowBox.top + next.cards[0]!.rect.top, final.windowBox.top + final.cards[0]!.rect.top);
                const firstMotion = islandUnionFrame(final, from, 1, edge, work);
                assert.ok(firstMotion.cards[0]!.rect.height < work.height / 2, "退出的首个形变帧不能误用全高宿主作为内容");
                assert.deepEqual(islandUnionFrame(final, from, ISLAND_MOTION_MS, edge, work), from);
            }
        }
    }
});

test("偏移夹紧到边界时固定视口仍包含展开、常态与并集窗口", () => {
    const inside = (
        windowBox: { left: number; top: number; width: number; height: number },
        viewport: { left: number; top: number; width: number; height: number },
    ) =>
        windowBox.left >= viewport.left - 1e-6 &&
        windowBox.top >= viewport.top - 1e-6 &&
        windowBox.left + windowBox.width <= viewport.left + viewport.width + 1e-6 &&
        windowBox.top + windowBox.height <= viewport.top + viewport.height + 1e-6;
    for (const edge of ["", "top", "bottom", "left", "right"] as const) {
        for (const offset of [-999, -300, 0, 300, 999]) {
            for (const form of ["capsule", "notch"] as const) {
                const expandedState = islandGeometry("expanded", expanded, 12, work, edge, offset, form);
                const collapsedState = islandGeometry("collapsed", collapsed, 22, work, edge, offset, form);
                for (const [from, to] of [
                    [expandedState, collapsedState],
                    [collapsedState, expandedState],
                ] as const) {
                    // 展开页面视口铺满工作区，紧凑两态仍是 420 带；各自窗口落在自己的视口内。
                    assert.ok(inside(from.windowBox, from.viewport), "起点窗口必须在视口内");
                    assert.ok(inside(to.windowBox, to.viewport), "终点窗口必须在视口内");
                    for (let elapsed = 0; elapsed <= ISLAND_MOTION_MS; elapsed += 8) {
                        const unionFrame = islandUnionFrame(from, to, elapsed, edge, work);
                        assert.ok(inside(unionFrame.windowBox, unionFrame.viewport), `${edge || "top"} 偏移 ${offset} 的并集窗口必须落在固定视口内`);
                        assert.deepEqual(parseIslandStateRequest(unionFrame), unionFrame);
                        const motionFrame = islandMotionFrame(from, to, elapsed, edge, work);
                        assert.ok(inside(motionFrame.windowBox, motionFrame.viewport), `${edge || "top"} 偏移 ${offset} 的单帧窗口必须落在固定视口内`);
                    }
                }
            }
        }
    }
});

test("接近工作区边界时回弹不超工作区", () => {
    const from = islandGeometry("hover", [{ left: 0, top: 0, width: 360, height: 72 }], 16, work, "right", 0);
    const to = islandGeometry("expanded", [{ left: 0, top: 0, width: 360, height: 800 }], 12, work, "right", 0);
    for (let elapsed = 0; elapsed < 240; elapsed++) {
        const frame = islandMotionFrame(from, to, elapsed, "right", work);
        assert.ok(frame.windowBox.width <= work.width);
        assert.ok(frame.windowBox.height <= work.height);
        assert.deepEqual(parseIslandStateRequest(frame), frame);
    }
});

test("贴边侧圆角在形变中从实际绘制值连续过渡，补面随进度出现且不越出并集窗口", () => {
    const collapsedNotch = islandGeometry("collapsed", [{ left: 0, top: 0, width: 160, height: 44 }], 22, work, "top", 50, "notch");
    const hoverNotch = islandGeometry("hover", [{ left: 0, top: 0, width: 268, height: 72 }], 16, work, "top", 50, "notch");
    const expandedNotch = islandGeometry("expanded", [{ left: 0, top: 0, width: 360, height: 240 }], 12, work, "top", 50, "notch");
    // 常态与 hover 都是外凸负圆角，动画全程不回到普通圆角。
    for (const elapsed of [0, 60, 120, 180, 240, 300]) assert.equal(islandAttachedCornerAt(-12, hoverNotch, elapsed, "notch"), -12);
    // 展开态没有负圆角，进入刘海时贴边角先收成直角再长出补面，结束时精确落到 -12。
    assert.equal(islandAttachedCornerAt(12, hoverNotch, 0, "notch"), 12);
    assert.equal(islandAttachedCornerAt(12, hoverNotch, ISLAND_MOTION_MS, "notch"), -12);
    assert.ok(islandAttachedCornerAt(12, hoverNotch, 120, "notch") < 12);
    assert.equal(islandAttachedCornerAt(-12, expandedNotch, 0, "notch"), -12);
    assert.equal(islandAttachedCornerAt(-12, expandedNotch, ISLAND_MOTION_MS, "notch"), 12);
    // 中途被新切态打断时，起点是上一帧实际绘制的负圆角，而不是按目标 state 重建的正圆角。
    const interrupted = islandAttachedCornerAt(-9, expandedNotch, 20, "notch");
    assert.ok(interrupted < 0 && interrupted > -12);
    // 胶囊形态不参与负圆角，任何进度都不绘制补面。
    const capsule = islandGeometry("hover", [{ left: 0, top: 0, width: 268, height: 72 }], 16, work, "top", 50);
    assert.equal(islandAttachedCornerAt(-12, capsule, 0, "capsule"), 0);
    // 补面在每个采样帧都存在且完全落在并集窗口内。
    for (let elapsed = 0; elapsed <= ISLAND_MOTION_MS; elapsed += 10) {
        const frame = islandUnionFrame(expandedNotch, hoverNotch, elapsed, "top", work);
        const attached = islandAttachedCornerAt(12, hoverNotch, elapsed, "notch");
        if (attached >= 0) continue;
        const patches = islandFilletPatches(frame.cards[0]!.rect, frame.windowBox, "top", -attached);
        assert.equal(patches.length, 2);
        for (const patch of patches) {
            assert.ok(patch.rect.left >= -1e-6 && patch.rect.top >= -1e-6);
            assert.ok(patch.rect.left + patch.rect.width <= frame.windowBox.width + 1e-6);
            assert.ok(patch.rect.top + patch.rect.height <= frame.windowBox.height + 1e-6);
        }
    }
});

/** 展开两段：当前卡先完成，其他卡延迟从当前卡展开矩形弹出并落在各自位置。 */
test("极限 offset：源/目标中心不同时中心按进度连续插值，0ms 不跳（t88）", () => {
    // 顶部靠右、底部靠左各一组：源与目标中心相差数十 DIP，0ms 必须仍落在源中心。
    const cases = [
        { edge: "top" as const, offset: 400, from: [{ left: 0, top: 0, width: 280, height: 72 }], to: [{ left: 0, top: 0, width: 280, height: 72 }, { left: 0, top: 80, width: 360, height: 120 }] },
        { edge: "bottom" as const, offset: -400, from: [{ left: 0, top: 0, width: 140, height: 72 }], to: [{ left: 0, top: 0, width: 140, height: 72 }, { left: 0, top: 80, width: 200, height: 120 }] },
    ];
    for (const { edge, offset, from: fromCards, to: toCards } of cases) {
        const from = islandGeometry("hover", fromCards, 16, work, edge, offset);
        const to = islandGeometry("expanded", toCards, 12, work, edge, offset);
        for (const stagger of [0, ISLAND_EXPAND_STAGGER_MS]) {
            const first = islandUnionFrame(from, to, 0, edge, work, stagger);
            const 源偏 = from.windowBox.left - first.windowBox.left;
            const 源中心 = from.cards[0]!.rect.left + 源偏 + from.cards[0]!.rect.width / 2;
            const 首 = first.cards[0]!.rect;
            assert.ok(Math.abs(首.left + 首.width / 2 - 源中心) <= 1.5, `${edge} offset ${offset} 0ms 中心应与源连续：${首.left + 首.width / 2} vs ${源中心}`);
            const mid = islandUnionFrame(from, to, ISLAND_MOTION_MS / 2, edge, work, stagger);
            const 目标偏 = to.windowBox.left - mid.windowBox.left;
            const 目标中心 = to.cards[0]!.rect.left + 目标偏 + to.cards[0]!.rect.width / 2;
            const 中 = mid.cards[0]!.rect;
            const 段 = Math.abs(中.left + 中.width / 2 - 源中心) + Math.abs(中.left + 中.width / 2 - 目标中心);
            assert.ok(段 <= Math.abs(源中心 - 目标中心) + 2, `${edge} 中点应在源与目标中心之间`);
        }
    }
});

test("过渡首帧按贴边锚定：左贴固定左缘、右贴固定右缘、上下贴固定中心（t84）", () => {
    const single = [{ left: 0, top: 0, width: 280, height: 72 }];
    const multi = [{ left: 0, top: 0, width: 280, height: 72 }, { left: 0, top: 80, width: 360, height: 120 }];
    for (const edge of ["left", "right", "top"] as const) {
        const from = islandGeometry("hover", single, 16, work, edge, 0);
        const to = islandGeometry("expanded", multi, 12, work, edge, 0);
        for (const stagger of [0, ISLAND_EXPAND_STAGGER_MS]) {
            const first = islandUnionFrame(from, to, 0, edge, work, stagger);
            const 源偏 = from.windowBox.left - first.windowBox.left; // 源窗口→并集窗口的左向偏移。
            const 目标偏 = to.windowBox.left - first.windowBox.left; // 目标窗口→并集窗口的左向偏移。
            const 源 = from.cards[0]!.rect;
            const 首 = first.cards[0]!.rect;
            if (edge === "left") assert.ok(Math.abs(首.left - (源.left + 源偏)) <= 1, `左贴首帧左缘应与源连续：${首.left} vs ${源.left + 源偏}`);
            else if (edge === "right") assert.ok(Math.abs(首.left + 首.width - (源.left + 源.width + 源偏)) <= 1, `右贴首帧右缘应与源连续：${首.left + 首.width} vs ${源.left + 源.width + 源偏}`);
            else assert.ok(Math.abs(首.left + 首.width / 2 - (to.cards[0]!.rect.left + 目标偏 + to.cards[0]!.rect.width / 2)) <= 1, "顶部首帧中心应在目标中心");
        }
    }
});

test("展开两段：其他卡延迟后从当前卡展开矩形弹出", () => {
    const from = islandGeometry("hover", [expanded[0]!], 16, work, "", 0);
    const to = islandGeometry("expanded", expanded, 12, work, "", 0);
    const stagger = ISLAND_EXPAND_STAGGER_MS;
    const total = ISLAND_MOTION_MS + stagger;
    const start = islandUnionFrame(from, to, 0, "", work, stagger);
    const offsetX = to.windowBox.left - start.windowBox.left;
    const offsetY = to.windowBox.top - start.windowBox.top;
    assert.deepEqual(start.cards[1]!.rect, { ...to.cards[0]!.rect, left: to.cards[0]!.rect.left + offsetX, top: to.cards[0]!.rect.top + offsetY }); // 起点与当前卡重合。
    assert.equal(start.cards[0]!.rect.width, from.cards[0]!.rect.width); // 当前卡仍从 hover 尺寸起步。
    const mid = islandUnionFrame(from, to, stagger + ISLAND_MOTION_MS / 2, "", work, stagger);
    assert.ok(mid.cards[1]!.rect.top > start.cards[1]!.rect.top, "相位 2 中其他卡应离开当前卡向下弹出");
    const end = islandUnionFrame(from, to, total, "", work, stagger);
    assert.deepEqual(parseIslandStateRequest(end), end); // 终点仍是合法原生请求。
    for (const [index, card] of end.cards.entries()) {
        assert.equal(card.rect.left, to.cards[index]!.rect.left + offsetX);
        assert.equal(card.rect.top, to.cards[index]!.rect.top + offsetY);
        assert.ok(card.rect.left >= 0 && card.rect.top >= 0 && card.rect.left + card.rect.width <= end.windowBox.width + 0.001 && card.rect.top + card.rect.height <= end.windowBox.height + 0.001);
    }
    assert.equal(end.cards[1]!.rect.top, start.cards[1]!.rect.top + (to.cards[1]!.rect.top - to.cards[0]!.rect.top)); // 位移量等于槽位差。
});

/** 并集大于目标窗口时，其他卡锚点必须换算到并集坐标；单卡展开不启用两段。 */
test("展开两段的并集坐标换算与单卡退化", () => {
    const viewport = { left: 0, top: 0, width: 1000, height: 800 };
    const to: IslandStateRequest = {
        state: "expanded",
        windowBox: { left: 100, top: 40, width: 360, height: 360 },
        viewport,
        cards: [
            { rect: { left: 0, top: 0, width: 360, height: 232 }, radius: 12 },
            { rect: { left: 0, top: 240, width: 360, height: 120 }, radius: 12 },
        ],
    };
    const from: IslandStateRequest = {
        state: "hover",
        windowBox: { left: 0, top: 40, width: 200, height: 72 },
        viewport,
        cards: [{ rect: { left: 20, top: 0, width: 160, height: 72 }, radius: 16 }],
    };
    assert.deepEqual(parseIslandStateRequest(from), from);
    assert.deepEqual(parseIslandStateRequest(to), to);
    const start = islandUnionFrame(from, to, 0, "", work, ISLAND_EXPAND_STAGGER_MS);
    const dx = to.windowBox.left - start.windowBox.left;
    assert.ok(dx > 0, "该用例必须产生非零并集偏移");
    assert.equal(start.windowBox.left, from.windowBox.left); // 并集向左扩到入态窗口。
    assert.equal(start.cards[1]!.rect.left, to.cards[0]!.rect.left + dx); // 锚点已换算到并集坐标。
    // 首帧中心与源连续（源/目标中心不同则按进度插值，t88）：宽度仍用入态宽度。
    const 源中心 = from.cards[0]!.rect.left + (from.windowBox.left - start.windowBox.left) + from.cards[0]!.rect.width / 2;
    assert.equal(start.cards[0]!.rect.width, from.cards[0]!.rect.width);
    assert.ok(Math.abs(start.cards[0]!.rect.left + start.cards[0]!.rect.width / 2 - 源中心) <= 1.5, "首帧中心应与源中心一致");
    const single: IslandStateRequest = { ...to, cards: [to.cards[0]!] };
    assert.deepEqual(islandUnionFrame(from, single, 120, "", work, ISLAND_EXPAND_STAGGER_MS), islandUnionFrame(from, single, 120, "", work)); // 单卡不启用两段。
});

test("过渡帧保持旧坐标系：旧原点与新原点下屏幕位置都不越界、不出现负局部坐标", () => {
    const viewport = { left: 0, top: 0, width: 1920, height: 1080 };
    const hover = {
        state: "hover" as const,
        windowBox: { left: 1694, top: 0, width: 220, height: 600 },
        viewport: { left: 1500, top: 0, width: 420, height: 600 },
        cards: [{ rect: { left: 0, top: 0, width: 220, height: 200 }, radius: 12 }],
    };
    const expanded = {
        state: "expanded" as const,
        windowBox: { left: 1694, top: 0, width: 336, height: 600 },
        viewport,
        cards: [{ rect: { left: 0, top: 0, width: 320, height: 600 }, radius: 12 }],
    };
    const screen = (frame: IslandStateRequest) => frame.windowBox.left + frame.cards[0]!.rect.left;
    // 过渡起点按旧坐标系绘制：屏幕 x 仍是 1694，而不是把局部坐标预写成新原点后的 3194。
    assert.equal(screen(hover), 1694);
    for (const [from, to] of [[hover, expanded], [expanded, hover]] as const) {
        for (const elapsed of [0, ISLAND_MOTION_MS]) {
            const frame = islandUnionFrame(from, to, elapsed, "right", viewport);
            const x = screen(frame);
            assert.ok(x >= 0 && x <= viewport.width, `过渡帧屏幕 x=${x} 必须在视口内`);
            for (const card of frame.cards) assert.ok(card.rect.left >= 0, "过渡帧不得出现负局部坐标");
        }
    }
    const source = 读源码("./island.tsx");
    const paintIndex = source.indexOf("paint(from, 0, attachedCorner");
    const commitIndex = source.indexOf("await commit(unionFirst)");
    assert.ok(paintIndex > 0 && commitIndex > paintIndex);
    assert.doesNotMatch(source.slice(paintIndex, commitIndex), /applyViewportMargin\(request\)/); // await 前不再改根坐标。
    assert.match(source, /if \(!\(await commit\(unionFirst\)\)\) return; \/\/ 回包作废时直接丢弃/);
    assert.match(source, /remeasure\.current\(true\); \/\/ 动画结束按实际几何复核一次/);
    assert.doesNotMatch(source, /islandExpandedStartFrame/); // 退回也走同一旧坐标 paint，不用展开助手。
});

test("视口切换的并集帧保持旧坐标系，终帧精确落回目标态", () => {
    // 回包前后所有可见帧都用真实几何检查：屏幕位置不越出工作区。
    const viewport = { left: 0, top: 0, width: 1920, height: 1080 };
    const from = {
        state: "expanded" as const,
        windowBox: { left: 1594, top: 0, width: 336, height: 1080 },
        viewport,
        cards: [{ rect: { left: 0, top: 0, width: 320, height: 300 }, radius: 12 }],
    };
    const to = {
        state: "hover" as const,
        windowBox: { left: 1694, top: 0, width: 220, height: 600 },
        viewport: { left: 1500, top: 0, width: 420, height: 600 },
        cards: [{ rect: { left: 0, top: 0, width: 220, height: 200 }, radius: 12 }],
    };
    for (const elapsed of [0, 60, 120, 180, ISLAND_MOTION_MS]) {
        const frame = islandUnionFrame(from, to, elapsed, "right", viewport);
        for (const card of frame.cards) {
            const x = frame.windowBox.left + card.rect.left;
            assert.ok(x >= 0 && x <= viewport.width, `elapsed=${elapsed} 屏幕 x=${x} 必须在 0..1920`);
        }
    }
    const done = islandUnionFrame(from, to, ISLAND_MOTION_MS, "right", viewport);
    assert.deepEqual(done, to); // 解除遮蔽时已经落回目标态。
});

test("连续几何提交：同视口窗口盒变化不遮蔽，回锚与换视口/切态/贴边/形态仍遮蔽", () => {
    const viewport = { left: 0, top: 0, width: 420, height: 1080 };
    const leftViewport = { left: 0, top: 0, width: 420, height: 1040 }; // 贴边时视口带只随工作区与贴边方向变化。
    const seat = { edge: "", form: "capsule" };
    const collapsedBase = parseIslandStateRequest({
        state: "collapsed",
        windowBox: { left: 130, top: 0, width: 160, height: 44 },
        viewport,
        cards: [{ rect: { left: 0, top: 0, width: 160, height: 44 }, radius: 22 }],
    });
    // 新消息到达：胶囊加宽并因居中左移，视口、状态、贴边与形态都不变。
    const collapsedWider = parseIslandStateRequest({
        state: "collapsed",
        windowBox: { left: 110, top: 0, width: 200, height: 44 },
        viewport,
        cards: [{ rect: { left: 0, top: 0, width: 200, height: 44 }, radius: 22 }],
    });
    assert.equal(shouldMaskGeometryChange(collapsedBase, collapsedWider, seat, seat), false); // 加宽不遮蔽，交接动画留在屏幕上。
    assert.equal(shouldMaskGeometryChange(collapsedWider, collapsedBase, seat, seat), false); // 收窄同样只改窗口盒。
    // 窗口盒不变、只换锚点行：没有尺寸变化可藏，也不遮蔽。
    const collapsedAnchors = parseIslandStateRequest({ ...collapsedBase, anchors: [{ rect: { left: 4, top: 4, width: 28, height: 28 }, radius: 14 }] });
    assert.equal(shouldMaskGeometryChange(collapsedBase, collapsedAnchors, seat, seat), false);

    const hoverSeat = { edge: "top", form: "capsule" };
    const hoverBase = parseIslandStateRequest({
        state: "hover",
        windowBox: { left: 110, top: 6, width: 200, height: 420 },
        viewport,
        cards: [{ rect: { left: 0, top: 0, width: 200, height: 120 }, radius: 16 }],
    });
    const hoverWider = parseIslandStateRequest({
        state: "hover",
        windowBox: { left: 100, top: 6, width: 220, height: 420 },
        viewport,
        cards: [{ rect: { left: 0, top: 0, width: 220, height: 120 }, radius: 16 }],
    });
    assert.equal(shouldMaskGeometryChange(hoverBase, hoverWider, hoverSeat, hoverSeat), false); // hover 宽度变化同样只改窗口盒。

    // 切态换内容坐标系：collapsed↔hover 与 collapsed↔expanded 都遮蔽。
    assert.equal(shouldMaskGeometryChange(collapsedBase, hoverBase, seat, hoverSeat), true);
    assert.equal(shouldMaskGeometryChange(hoverBase, collapsedBase, hoverSeat, seat), true);
    const expanded = parseIslandStateRequest({
        state: "expanded",
        windowBox: { left: 0, top: 0, width: 1920, height: 1080 },
        viewport: { left: 0, top: 0, width: 1920, height: 1080 },
        cards: [{ rect: { left: 780, top: 0, width: 360, height: 600 }, radius: 12 }],
    });
    assert.equal(shouldMaskGeometryChange(collapsedBase, expanded, seat, hoverSeat), true);
    assert.equal(shouldMaskGeometryChange(expanded, collapsedBase, hoverSeat, seat), true);

    // 换贴边或换形态：窗口仍落同一视口，但内容锚点变了。
    assert.equal(shouldMaskGeometryChange(collapsedBase, collapsedWider, seat, { edge: "top", form: "capsule" }), true);
    assert.equal(shouldMaskGeometryChange(collapsedBase, collapsedWider, seat, { edge: "", form: "notch" }), true);
    // 换视口：显示器、DPI、工作区或沿边偏移变化都会改视口带。
    assert.equal(shouldMaskGeometryChange(collapsedBase, { ...collapsedBase, viewport: { left: 60, top: 0, width: 420, height: 1080 } }, seat, seat), true);
    assert.equal(shouldMaskGeometryChange(collapsedBase, { ...collapsedBase, viewport: { ...viewport, left: 0.4 } }, seat, seat), false); // 半个 DIP 的量化不算换视口。

    // 不换尺寸的纯移动（偏移变化但没有回锚标志）仍遮蔽。
    const moved = parseIslandStateRequest({ ...collapsedBase, windowBox: { left: 260, top: 0, width: 160, height: 44 } });
    assert.equal(shouldMaskGeometryChange(collapsedBase, moved, seat, seat), true);

    // 没有基准时保守遮蔽：首次提交、换边/换形态后清空几何都落在这一支。
    assert.equal(shouldMaskGeometryChange(undefined, collapsedBase, seat, seat), true);
    assert.equal(shouldMaskGeometryChange(collapsedBase, collapsedBase, undefined, seat), true);

    // M1：左贴边 hover 垂直拖动 150 同时内容变宽 5，视口不变；回锚提交必须遮蔽一次。
    const dragSeat = { edge: "left", form: "capsule" };
    const dragBase = parseIslandStateRequest({
        state: "hover",
        windowBox: { left: 0, top: 300, width: 200, height: 420 },
        viewport: leftViewport,
        cards: [{ rect: { left: 0, top: 0, width: 200, height: 120 }, radius: 16 }],
    });
    const dragLanded = parseIslandStateRequest({
        state: "hover",
        windowBox: { left: 0, top: 450, width: 205, height: 420 },
        viewport: leftViewport,
        cards: [{ rect: { left: 0, top: 0, width: 205, height: 120 }, radius: 16 }],
    });
    assert.equal(shouldMaskGeometryChange(dragBase, dragLanded, dragSeat, dragSeat), false); // 只靠 resize 判定会放过同时位移与改尺寸的落点。
    assert.equal(shouldMaskGeometryChange(dragBase, dragLanded, dragSeat, dragSeat, true), true); // 回锚标志强制遮蔽这一次。

    // M2：基准随提交前移，换边后同一 effect 内第二次普通宽变不再遮蔽。
    const leftSeat = { edge: "left", form: "capsule" };
    const leftBase = parseIslandStateRequest({
        state: "collapsed",
        windowBox: { left: 6, top: 498, width: 180, height: 44 },
        viewport: leftViewport,
        cards: [{ rect: { left: 0, top: 0, width: 180, height: 44 }, radius: 22 }],
    });
    const leftWider = parseIslandStateRequest({
        state: "collapsed",
        windowBox: { left: 6, top: 488, width: 220, height: 44 },
        viewport: leftViewport,
        cards: [{ rect: { left: 0, top: 0, width: 220, height: 44 }, radius: 22 }],
    });
    let baseline: IslandGeometrySeat | undefined;
    const 换边首次 = islandGeometryCommitDecision(undefined, leftBase, baseline, leftSeat); // 换边已清空几何，没有可比基准。
    assert.equal(换边首次.mask, true);
    assert.deepEqual(换边首次.seat, leftSeat); // 基准随本次提交前移。
    baseline = 换边首次.seat;
    assert.equal(islandGeometryCommitDecision(leftBase, leftWider, baseline, leftSeat).mask, false); // 同一 effect 内的下一次普通宽变不再整层隐藏。
    // 基准固化成旧的贴边身份（修复前的写法）会把第二次提交也遮蔽。
    assert.equal(islandGeometryCommitDecision(leftBase, leftWider, seat, leftSeat).mask, true);
    // 回锚提交只遮蔽它自己那一次，随后的同视口宽变照常放行。
    const 回锚后 = parseIslandStateRequest({ ...leftWider, windowBox: { left: 6, top: 448, width: 220, height: 44 } });
    const 回锚 = islandGeometryCommitDecision(leftWider, 回锚后, baseline, leftSeat, true);
    assert.equal(回锚.mask, true);
    const 回锚后更宽 = parseIslandStateRequest({ ...回锚后, windowBox: { left: 6, top: 448, width: 240, height: 44 }, cards: [{ rect: { left: 0, top: 0, width: 240, height: 44 }, radius: 22 }] });
    assert.equal(islandGeometryCommitDecision(回锚后, 回锚后更宽, 回锚.seat, leftSeat).mask, false);
});

test("遮蔽租约：取消即恢复、新任务接管旧回包不解除、连续切换与无后续都最终可见", () => {
    const lease = { seq: 0, owner: undefined as number | undefined, masked: false };
    const first = beginIslandMask(lease);
    assert.equal(lease.owner, first);
    assert.equal(lease.masked, true);
    assert.equal(releaseIslandMask(lease, first), true); // 队列前取消也释放。
    assert.deepEqual(lease, { seq: 1, owner: undefined, masked: false }); // 不会一直隐藏。

    const stale = beginIslandMask(lease);
    const current = beginIslandMask(lease);
    assert.equal(holdsIslandMask(lease, stale), false);
    assert.equal(releaseIslandMask(lease, stale), false); // 旧回包不能解除新遮蔽。
    assert.equal(unmaskIslandTransition(lease, stale), false);
    assert.deepEqual(lease, { seq: 3, owner: current, masked: true });
    assert.equal(unmaskIslandTransition(lease, current), true);
    assert.equal(lease.owner, current); // 保留所有权供本任务动画绘制。
    assert.equal(releaseIslandMask(lease, current), true);
    assert.deepEqual(lease, { seq: 3, owner: undefined, masked: false });

    const one = beginIslandMask(lease);
    const two = beginIslandMask(lease);
    assert.equal(releaseIslandMask(lease, one), false);
    assert.equal(unmaskIslandTransition(lease, two), true);
    assert.equal(releaseIslandMask(lease, two), true); // 连续两次后最终可见、可交互。

    const source = 读源码("./island.tsx");
    assert.match(source, /if \(!active \|\| !mounted\.current\) \{ settleMaskRelease\(maskToken\); return; \}/); // 队列前取消走唯一结算。
    assert.match(source, /settleMaskRelease\(ownerToken, \{ resynced: gate\.state\(\)\.applied, alreadySynced: true \}\)/);
    assert.match(source, /if \(!active \|\| !gate.owns\(maskToken\)\) return; \/\/ 旧回包不得绘制或解除新遮蔽/);
    assert.match(source, /if \(gate.owns\(maskToken\)\) settleMaskRelease\(maskToken, \{ alreadySynced: settled, failed: commitFailed \}\);/);
});

test("回包先验权、终态先于释放，前置/在途取消时序正确", () => {
    const lease = { seq: 0, owner: undefined as number | undefined, masked: false };
    const a = beginIslandMask(lease);
    const b = beginIslandMask(lease);
    assert.equal(holdsIslandMask(lease, a), false); // A 回包到达时 B 已持有。
    assert.equal(releaseIslandMask(lease, a), false); // A 的释放幂等丢弃，不影响 B。
    assert.deepEqual(lease, { seq: 2, owner: b, masked: true });

    const source = 读源码("./island.tsx");
    const awaitAt = source.indexOf("await setIslandState(value);");
    const guardAt = source.indexOf("if (!active || !gate.owns(maskToken))", awaitAt);
    const writeAt = source.indexOf("currentGeometry.current = value;", awaitAt);
    assert.ok(guardAt > awaitAt && writeAt > guardAt); // 回包后先验权，再写几何/置 inputReady。
    assert.match(source, /settleMaskRelease\(maskToken, \{ resynced: response, failed: !response \}\); \/\/ 回包作废也走唯一结算入口/);

    const finallyAt = source.indexOf("animating = false;\n                                committing = false;");
    const releaseAt = source.indexOf("settleMaskRelease(maskToken, { alreadySynced: settled, failed: commitFailed })", finallyAt);
    const terminal = source.slice(finallyAt, releaseAt);
    assert.match(terminal, /if \(settled && active && gate.owns\(maskToken\) && request\.state !== "expanded"\)/);
    assert.match(terminal, /inputGeometryKey\.current = key;/);
    assert.match(terminal, /remeasure\.current\(true\);/);
    assert.ok(releaseAt > finallyAt); // 成功终态在释放前完成。

    assert.match(读提交门(), /if \(swap\.pending > 0\) \{/); // pending 时挂续延。
    assert.match(source, /settleMaskRelease\(ownerToken, \{ resynced: gate\.state\(\)\.applied, alreadySynced: true \}\)/);
});

test("共享原生物理状态：A 在途时 B 取消不提前揭开，回包重同步后才解除", () => {
    const swap = { pending: 0, appliedToken: null as string | null };
    const lease = { seq: 0, owner: undefined as number | undefined, masked: false };
    const a = beginIslandMask(lease);
    beginIslandNativeSwap(swap);
    const b = beginIslandMask(lease); // B 抢到显示租约。
    assert.equal(canReleaseIslandMask(swap, String(b), false), false); // A 在途，B 不能解除。
    assert.equal(canReleaseIslandMask(swap, String(b), true), false); // pending 未清空，重同步也不行。
    finishIslandNativeSwap(swap, String(a), true);
    assert.equal(canReleaseIslandMask(swap, String(b), false), false); // 落地的是 A，不是 B。
    assert.equal(canReleaseIslandMask(swap, String(b), true), true); // 按实际几何重同步后可释放。

    const swapA = { pending: 0, appliedToken: null as string | null };
    const leaseA = { seq: 0, owner: undefined as number | undefined, masked: false };
    const onlyA = beginIslandMask(leaseA);
    beginIslandNativeSwap(swapA);
    assert.equal(canReleaseIslandMask(swapA, String(onlyA), false), false); // A 在途取消也保持遮蔽。
    finishIslandNativeSwap(swapA, String(onlyA), true);
    assert.equal(canReleaseIslandMask(swapA, String(onlyA), false), true); // 回包落地后才解除。

    const rejected = { pending: 0, appliedToken: null as string | null };
    beginIslandNativeSwap(rejected);
    finishIslandNativeSwap(rejected, "stale", false);
    assert.equal(rejected.appliedToken, null);
    assert.equal(canReleaseIslandMask(rejected, "stale", false), false); // 过期 reject 不构成解除依据。

    const source = 读源码("./island.tsx");
    assert.match(读提交门(), /const swap: IslandNativeSwap = \{ pending: 0, appliedToken: null, appliedGeometry: undefined \};/); // 跨 effect 共享。
    assert.match(source, /gate\.nativeStarted\(\);/);
    assert.match(source, /gate\.nativeFinished\(maskToken, response, value\);/);
    assert.match(读提交门(), /if \(swap\.pending > 0\) \{/); // pending 时挂续延。
    assert.match(source, /settleMaskRelease\(ownerToken, \{ resynced: gate\.state\(\)\.applied, alreadySynced: true \}\)/); // cleanup 走唯一结算。
    assert.match(source, /if \(gate.owns\(maskToken\)\) \{\s*currentGeometry\.current = undefined; \/\/ 只有当前租约持有者能清共享几何/);
    assert.match(source, /onResync: \(value\) => \{\s*if \(!active \|\| !root\.current \|\| !resyncPaint\) return false;/); // 旧实例或已同步的回包不重画。
});

test("遮蔽释放唯一入口：三条时序与 800ms 失败兜底", () => {
    // 1) B 取消后 A 成功：A 回包重同步到 1694 才解除 B 的遮蔽。
    const swap = { pending: 0, appliedToken: null as string | null };
    const lease = { seq: 0, owner: undefined as number | undefined, masked: false };
    const a = beginIslandMask(lease);
    beginIslandNativeSwap(swap);
    beginIslandMask(lease); // B 抢租约后取消。
    finishIslandNativeSwap(swap, String(a), true);
    const geometry = { state: "expanded" as const, windowBox: { left: 1694, top: 0, width: 336, height: 1080 }, viewport: { left: 0, top: 0, width: 1920, height: 1080 }, cards: [{ rect: { left: 0, top: 0, width: 320, height: 300 }, radius: 12 }] };
    assert.equal(geometry.windowBox.left + geometry.cards[0]!.rect.left, 1694);

    // 2) A 单独在途取消后成功：回包后 appliedToken 指向 A，才允许解除。
    const swapA = { pending: 0, appliedToken: null as string | null };
    const leaseA = { seq: 0, owner: undefined as number | undefined, masked: false };
    const onlyA = beginIslandMask(leaseA);
    beginIslandNativeSwap(swapA);
    assert.equal(canReleaseIslandMask(swapA, String(onlyA), false), false);
    finishIslandNativeSwap(swapA, String(onlyA), true);
    assert.equal(canReleaseIslandMask(swapA, String(onlyA), false), true);

    // 3) 过期 reject + B 取消：appliedToken 为空时 fail-closed，800ms 后才允许重同步解除。
    const rejected = { pending: 0, appliedToken: null as string | null };
    const stale = beginIslandMask(leaseA);
    beginIslandNativeSwap(rejected);
    finishIslandNativeSwap(rejected, String(stale), false);
    assert.equal(canReleaseIslandMask(rejected, String(stale), false), false);
    assert.equal(canReleaseIslandMask(rejected, String(stale), true), true);

    const source = 读源码("./island.tsx");
    assert.match(读提交门(), /const settle = \(token: number, options: IslandMaskSettleOptions = \{\}\): void =>/); // 唯一入口。
    assert.match(读提交门(), /if \(swap\.pending > 0\) \{/);
    assert.match(读提交门(), /scheduler\.setTimer\(\(\) => \{[\s\S]*?settle\(token, \{ keepOwner: options\.keepOwner, alreadySynced: options\.alreadySynced, resynced: true \}\);/);
    assert.match(读提交门(), /if \(!alreadySynced\) \{[\s\S]*?const alive = host\?\.onRestore\(\) === true;[\s\S]*?host\?\.onResync\(actual\);/); // 死实例不还原样式；restore 后再重绘。
    assert.match(读提交门(), /if \(keepOwner\) unmask\(token, true\);\s*else release\(token, true\);/); // 帧到后按是否保留所有权解除。
    assert.match(source, /drainMaskReleases\(response, !response\)/); // 成功与失败都 drain。
    assert.match(source, /settleMaskRelease\(maskToken, \{ resynced: response, failed: !response \}\)/);
    assert.equal((source.match(/releaseMask\(maskToken/g) ?? []).length, 0); // finally 不再各自解罩。
});

test("reject 必 drain、有界补偿重提交、失败 800ms 只解除 CSS", () => {
    const swap = { pending: 0, appliedToken: null as string | null };
    const lease = { seq: 0, owner: undefined as number | undefined, masked: false };
    const token = beginIslandMask(lease);
    beginIslandNativeSwap(swap);
    finishIslandNativeSwap(swap, String(token), false);
    assert.equal(swap.pending, 0); // 失败也成对归零。
    assert.equal(swap.appliedToken, null);
    assert.equal(canReleaseIslandMask(swap, String(token), false), false); // 失败不立即解除。

    const recovered = { pending: 0, appliedToken: null as string | null };
    beginIslandNativeSwap(recovered);
    finishIslandNativeSwap(recovered, String(token), true); // 补偿成功。
    assert.equal(recovered.pending, 0);
    assert.equal(canReleaseIslandMask(recovered, String(token), false), true); // 补偿成功后可解除并可见。

    const failed = { pending: 0, appliedToken: null as string | null };
    beginIslandNativeSwap(failed);
    finishIslandNativeSwap(failed, String(token), false);
    assert.equal(canReleaseIslandMask(failed, String(token), true), true); // 800ms 兜底只解除 CSS。
    assert.equal(failed.pending, 0);

    const source = 读源码("./island.tsx");
    const finallyAt = source.indexOf("gate.nativeFinished(maskToken, response, value); // 任何返回路径都成对归零 pending。");
    const drainAt = source.indexOf("drainMaskReleases(response, !response); // reject 也一定会 drain。", finallyAt);
    assert.ok(finallyAt > 0 && drainAt > finallyAt); // reject 的 finally 仍然 drain。
    assert.match(source, /response = await islandRetrySubmit\(\{/); // 有界补偿重提交走可测助手。
    assert.match(source, /current: \(\) => gate.owns\(maskToken\)/);
    assert.match(source, /islandNativeFailClosed = true;/);
    assert.match(source, /岛原生几何提交失败，已标记 fail-closed/);
    assert.match(source, /settleMaskRelease\(maskToken, \{ alreadySynced: settled, failed: commitFailed \}\)/); // 失败走 800ms。
});

test("fail-closed 后同一窗口盒只由外部状态变化重提一次，成功后恢复去重", () => {
    const source = 读源码("./island.tsx");
    assert.match(source, /let failClosedResubmit = islandNativeFailClosed; \/\/ 仅在本次外部状态变化后的首个测量允许绕过一次去重/);
    assert.match(source, /const deduped = key === geometryKey\.current && \(!expanded \|\| \(from && islandWindowKey\(from\) === key\)\);/);
    assert.match(source, /if \(deduped && !failClosedResubmit\) \{/); // 只有标记允许绕过。
    assert.match(source, /if \(failClosedResubmit\) failClosedResubmit = false; \/\/ 一次外部变化只强制重提一次，防自激/);
    assert.match(source, /islandNativeFailClosed = false;\s*failedGeometry\.current = "";[^\n]*\n\s*commitFailed = false;/); // 成功后清标记、恢复去重。
    assert.equal((source.match(/islandNativeFailClosed = true/g) ?? []).length, 1); // 只有补偿失败会置真，无外部变化不自行置真。
    assert.match(source, /islandNativeFailClosed = true;\s*commitFailed = true;\s*console\.warn/);
});

test("展开揭幕改为视口条件等待：inner 到位后再等一帧才解除遮蔽", async () => {
    // 条件与容差：目标视口 2560×1400，inner 差 1 DIP 内视为到位。
    assert.equal(islandViewportMatched({ width: 2560, height: 1400 }, { width: 2560, height: 1400 }), true);
    assert.equal(islandViewportMatched({ width: 2561, height: 1399 }, { width: 2560, height: 1400 }), true);
    assert.equal(islandViewportMatched({ width: 2563, height: 1400 }, { width: 2560, height: 1400 }), false);
    assert.equal(islandViewportMatched({ width: 420, height: 1400 }, { width: 2560, height: 1400 }), false);
    // 条件滞后两帧才到位：只在到位后再等一帧就返回 true。
    let inner = { width: 420, height: 1400 };
    let 时钟 = 0;
    const 帧 = [];
    const 到位 = await awaitIslandViewport({
        viewport: { width: 2560, height: 1400 },
        inner: () => inner,
        wait: async () => {
            帧.push(inner.width);
            时钟 += 16;
            if (帧.length === 3) inner = { width: 2560, height: 1400 };
        },
        now: () => 时钟,
    });
    assert.equal(到位, true);
    assert.equal(帧.length, 4); // 两次滞后帧 + 到位帧 + 提交帧。
    // 一直不到位：有界超时收口（返回 false），只再等一次，不会无限等。
    let 次数 = 0;
    const 超时到位 = await awaitIslandViewport({
        viewport: { width: 2560, height: 1400 },
        inner: () => ({ width: 420, height: 1400 }),
        wait: async (剩余) => {
            次数 += 1;
            时钟 += Math.max(1, 剩余);
        },
        now: () => 时钟,
        timeoutMs: 300,
    });
    assert.equal(超时到位, false);
    assert.equal(次数, 2); // 一次超时帧 + 一次收口帧。
    // 源码：并集与终态都走视口条件；双 rAF 时间屏障已退役。
    const source = 读源码("./island.tsx");
    const 并集 = source.slice(source.indexOf("paint(unionFirst, 0, startAttached, staggerMs);"), source.indexOf("settleMaskRelease(maskToken, { keepOwner: true, alreadySynced: true })"));
    assert.match(并集, /await awaitViewport\(request\.viewport\)/);
    assert.match(并集, /islandTrace\("frame-barrier", \{ where: "union", matched: 已到位 \}\)/);
    assert.ok(并集.indexOf("awaitViewport") < 并集.indexOf("frame-barrier"));
    assert.ok(并集.indexOf("frame-barrier") < 并集.indexOf("mask-release-union"));
    assert.match(source, /if \(gate\.state\(\)\.masked && islandViewportChanged\(from, request\)\) \{/); // 终态同样走视口条件。
    assert.match(source, /islandTrace\("frame-barrier", \{ where: "final", matched: 已到位 \}\)/);
    assert.doesNotMatch(source, /islandCommittedFrame/); // 时间屏障（双 rAF）已退役。
    // 遮蔽窗口拉长后，期间按下的 Esc 不能丢。
    assert.match(source, /if \(gate\.state\(\)\.masked\) \{ if \(event\.key === "Escape"\) pendingEscape\.current = true; return; \}/);
    assert.match(source, /if \(pendingEscape\.current\) \{ pendingEscape\.current = false; changeMode\("collapsed", false\); \}/);
});

test("挂起提交→同几何替换实例→旧回包：只结算租约、不重画", () => {
    const queue = new Map();
    // 旧实例卸载时把 alreadySynced 一起挂起；旧实现只存 token，语义丢失后会用旧 paint 重画固定尺寸。
    deferIslandMaskRelease(queue, 7, { keepOwner: false, alreadySynced: true });
    deferIslandMaskRelease(queue, 8, { keepOwner: true, alreadySynced: false });
    const 取出 = takeIslandMaskReleases(queue);
    assert.deepEqual(取出, [
        { token: 7, keepOwner: false, alreadySynced: true },
        { token: 8, keepOwner: true, alreadySynced: false },
    ]);
    assert.equal(queue.size, 0); // 取出即清空。
    // 旧回包：实例已替换（active=false）或已同步，任一条件都不得重画几何。
    assert.equal(canResyncIslandGeometry({ active: false, applied: true, alreadySynced: false }), false);
    assert.equal(canResyncIslandGeometry({ active: false, applied: true, alreadySynced: 取出[0].alreadySynced }), false);
    assert.equal(canResyncIslandGeometry({ active: true, applied: true, alreadySynced: 取出[0].alreadySynced }), false);
    assert.equal(canResyncIslandGeometry({ active: true, applied: true, alreadySynced: false }), true); // 同一实例正常重同步仍保留。
    assert.equal(canResyncIslandGeometry({ active: true, applied: false, alreadySynced: false }), false); // 原生未落地不画。

    const source = 读源码("./island.tsx");
    assert.match(读提交门(), /const deferred = new Map<number, /);
    assert.match(source, /resyncPaint = \(value\) => \{ if \(!active \|\| !root\.current\) return; paint\(value, 0\); \}/);
});

test("IPC 重试耗尽：有界重试返回 false，下一次提交按无可信基准重新遮蔽", async () => {
    const 等待记录 = [];
    const 等待 = (毫秒) => {
        等待记录.push(毫秒);
        return Promise.resolve();
    };
    // 连续拒绝：两次重试后失败，不抛异常。
    assert.equal(await islandRetrySubmit({ submit: async () => { throw new Error("reject"); }, wait: 等待, current: () => true }), false);
    assert.deepEqual(等待记录, [100, 100]);
    // 迟延后成功：第二次提交成功。
    let 次 = 0;
    assert.equal(await islandRetrySubmit({ submit: async () => { 次 += 1; if (次 === 1) throw new Error("delay"); }, wait: 等待, current: () => true }), true);
    // 租约被接管：不再提交。
    let 提交次数 = 0;
    assert.equal(await islandRetrySubmit({ submit: async () => { 提交次数 += 1; }, wait: 等待, current: () => false }), false);
    assert.equal(提交次数, 0);
    // 基准重置：几何/贴边/去重键全部失效 → 下一次提交按首次提交遮蔽。
    const reset = resetIslandGeometryBaseline();
    assert.equal(reset.geometry, undefined);
    assert.equal(reset.seat, undefined);
    assert.equal(reset.key, "");
    const next = parseIslandStateRequest({
        state: "collapsed",
        windowBox: { left: 130, top: 0, width: 160, height: 44 },
        viewport: { left: 0, top: 0, width: 420, height: 1080 },
        cards: [{ rect: { left: 0, top: 0, width: 160, height: 44 }, radius: 22 }],
    });
    assert.equal(islandGeometryCommitDecision(reset.geometry, next, reset.seat, { edge: "", form: "capsule" }).mask, true); // 无可信基准 → 重新遮蔽。

    const source = 读源码("./island.tsx");
    assert.match(source, /const baseline = resetIslandGeometryBaseline\(\);/);
    assert.match(source, /currentGeometry\.current = baseline\.geometry;/);
    assert.match(source, /seatBaseline\.current = baseline\.seat;/);
});

test("展开终帧先写最终列：恢复相对流之前补 --island-expanded-column", () => {
    const request = parseIslandStateRequest({
        state: "expanded",
        windowBox: { left: 900, top: 40, width: 336, height: 1040 },
        viewport: { left: 0, top: 40, width: 1920, height: 1040 },
        cards: [{ rect: { left: 0, top: 0, width: 320, height: 240 }, radius: 12 }],
    });
    assert.equal(islandExpandedColumn(request), 900); // 列左边距 = 窗口左 − 视口左，逐帧绘制与终帧共用同一算式。
    const 右贴 = parseIslandStateRequest({ ...request, windowBox: { ...request.windowBox, left: 1578 } });
    assert.equal(islandExpandedColumn(右贴), 1578);

    const source = 读源码("./island.tsx");
    const 展开分支 = source.slice(source.indexOf('if (request.state === "expanded") {'), source.indexOf("} else if (!animate && grows) {"));
    const 列下标 = 展开分支.indexOf("--island-expanded-column");
    const 恢复下标 = 展开分支.indexOf("restoreStyles();");
    assert.ok(列下标 >= 0 && 恢复下标 > 列下标, "终帧 restore 之前必须按最终 request 写一次列偏移");
    assert.match(展开分支, /islandExpandedColumn\(request\)/); // 终帧列与请求同源，不沿用上一帧并集列。
    assert.match(source, /const columnX = expandedPaint \? islandExpandedColumn\(value\) : 0;/); // 逐帧绘制也复用同一算式。
});

test("持续拒绝不无限重试：失败后同几何不再重发，只有外部变化或几何改变才放行", () => {
    // 模拟「提交失败 → 清基准 → 释放时强制重测」的循环：失败门拦住重复几何。
    let failedKey = "";
    let submits = 0;
    const 强制重测 = (key, resubmit = false) => {
        if (islandFailedGeometryGate({ failedKey, key, resubmit })) return "skip";
        submits += 1; // 一次提交内部含 1 次首发 + 2 次有界重试。
        failedKey = key; // 重试耗尽：记下未落地的目标几何。
        return "failed";
    };
    for (let index = 0; index < 12; index += 1) 强制重测("k1"); // 800ms 兜底释放触发的强制重测。
    assert.equal(submits, 1); // 持续拒绝只发过一次，不再 1+2 次无限重启。
    assert.equal(强制重测("k1", true), "failed"); // 外部状态变化后的 fail-closed 重提放行一次。
    assert.equal(submits, 2);
    assert.equal(强制重测("k2"), "failed"); // 几何改变（新窗口盒/视口）照常提交。
    assert.equal(submits, 3);
    assert.equal(强制重测("k2"), "skip");
    assert.equal(submits, 3);
    // 成功提交会清空失败门：同一几何之后还能正常提交。
    assert.equal(islandFailedGeometryGate({ failedKey: "", key: "k2", resubmit: false }), false);

    const source = 读源码("./island.tsx");
    assert.match(source, /if \(islandFailedGeometryGate\(\{ failedKey: failedGeometry\.current, key, resubmit: failClosedResubmit \}\)\) return;/);
    assert.match(source, /failedGeometry\.current = islandGeometryKey\(request\);/); // 失败时记录未落地几何。
    assert.match(source, /failedGeometry\.current = "";/); // 成功时清门。
    // 去重键与失败门共用同一算式：展开只看窗口盒/视口/态。
    const expanded = parseIslandStateRequest({ state: "expanded", windowBox: { left: 10, top: 0, width: 336, height: 400 }, viewport: { left: 0, top: 0, width: 900, height: 400 }, cards: [{ rect: { left: 0, top: 0, width: 320, height: 200 }, radius: 12 }] });
    assert.equal(islandGeometryKey(expanded), JSON.stringify([expanded.state, expanded.windowBox, expanded.viewport]));
    assert.match(islandGeometryKey({ ...expanded, state: "collapsed", windowBox: { left: 10, top: 0, width: 44, height: 44 } }), /"state":"collapsed"/); // 紧凑态看整份请求。
});

test("并集揭幕等待可被 cleanup/finishMotion 唤醒，submission 必定收口", async () => {
    let 帧回调;
    const 等待 = islandWakingFrame((callback) => {
        帧回调 = callback;
        return 7;
    });
    let 收口 = false;
    void 等待.done.then(() => {
        收口 = true;
    });
    await Promise.resolve();
    assert.equal(收口, false); // 帧未到、未唤醒时仍在等。
    等待.wake(); // cleanup/finishMotion 取消帧：必须收口，否则 submission 永不结束。
    await 等待.done;
    assert.equal(收口, true);
    // 正常帧路径同样收口。
    let 正常帧;
    const 正常 = islandWakingFrame((callback) => {
        正常帧 = callback;
        return 8;
    });
    let 正常收口 = false;
    void 正常.done.then(() => {
        正常收口 = true;
    });
    await Promise.resolve();
    assert.equal(正常收口, false);
    正常帧();
    await 正常.done;
    assert.equal(正常收口, true);

    const source = 读源码("./island.tsx");
    assert.match(source, /const reveal = islandWakingFrame\(/); // 揭幕等待本身可唤醒（并带无帧超时）。
    assert.match(source, /wakeReveal = reveal\.wake;/);
    const 取消处 = source.match(/cancelAnimationFrame\(motionFrame\);\s*wakeMotion\?\.\(\);\s*wakeReveal\?\.\(\);/g) ?? [];
    assert.equal(取消处.length, 2); // finishMotion 与 cleanup 都唤醒揭幕等待。
});

test("终态视口屏障按需触发：同视口或已揭幕不等，trace 关闭不读布局", () => {
    const source = 读源码("./island.tsx");
    // 终态屏障只在「仍要揭幕 + 换了视口」时触发，覆盖无动效展开与紧凑态跨视口。
    assert.match(source, /if \(gate\.state\(\)\.masked && islandViewportChanged\(from, request\)\) \{/);
    const 终态块 = source.slice(source.indexOf("if (gate.state().masked && islandViewportChanged(from, request)) {"), source.indexOf("settled = true;"));
    assert.ok(终态块.includes("await awaitViewport(request.viewport)"), "终态按视口条件等，不按时间硬等");
    assert.ok(终态块.includes('islandTrace("frame-barrier", { where: "final"'));
    // 并集路径顺序：paint → 视口条件 → 屏障 trace → mask-release。
    const 并集 = source.slice(source.indexOf("paint(unionFirst, 0, startAttached, staggerMs);"), source.indexOf("settleMaskRelease(maskToken, { keepOwner: true, alreadySynced: true })"));
    assert.ok(并集.indexOf("awaitViewport") < 并集.indexOf("frame-barrier"));
    assert.ok(并集.indexOf("frame-barrier") < 并集.indexOf("mask-release-union"));
    // trace 关闭时不为诊断构造 detail（不读 getBoundingClientRect）。
    assert.match(source, /const islandTraceLazy = \(stage: string, detail: \(\) => Record<string, unknown>\) => \{\s*if \(!islandTracing\(\)\) return;/);
    assert.match(source, /islandTraceLazy\("paint", \(\) => \(\{/);
    assert.doesNotMatch(source, /islandTrace\("paint", \{/); // 不再有立即求值的 paint 诊断。
});

test("失败门用外层最终目标 key：展开失败的并集不会多打一轮 IPC", () => {
    const 工作区 = { left: 0, top: 0, width: 1920, height: 1080 };
    const 折叠 = islandGeometry("collapsed", [{ left: 0, top: 0, width: 160, height: 44 }], 22, 工作区, "top", 0);
    const 展开 = islandGeometry("expanded", [{ left: 0, top: 0, width: 320, height: 900 }], 12, 工作区, "top", 0);
    const 并集 = islandUnionFrame(折叠, 展开, 0, "top", 工作区, 0);
    assert.notEqual(islandGeometryKey(并集), islandGeometryKey(展开)); // 真实并集的 key 与最终目标不同。
    // 提交链：并集提交重试耗尽 = 1 次首发 + 2 次有界重试；随后 800ms 兜底触发的强制重测拿的是最终目标。
    let ipc = 0;
    let failedKey = "";
    const 测量 = (request) => {
        if (islandFailedGeometryGate({ failedKey, key: islandGeometryKey(request), resubmit: false })) return "skip";
        ipc += 3; // 1 + 2 次 IPC
        failedKey = islandGeometryKey(展开); // 修法：始终记外层最终目标 request。
        return "failed";
    };
    assert.equal(测量(并集), "failed");
    assert.equal(ipc, 3);
    assert.equal(测量(展开), "skip"); // 最终目标被失败门拦住，不再多打一轮 1+2。
    assert.equal(ipc, 3);
    assert.equal(测量(展开), "skip");
    assert.equal(ipc, 3);
    // 旧做法（登记中间帧 key）拦不住最终目标：这条断言保证测试能抓住该回归。
    assert.equal(islandFailedGeometryGate({ failedKey: islandGeometryKey(并集), key: islandGeometryKey(展开), resubmit: false }), false);

    const source = 读源码("./island.tsx");
    assert.match(source, /failedGeometry\.current = islandGeometryKey\(request\);/); // 记外层最终目标。
    assert.doesNotMatch(source, /failedGeometry\.current = islandGeometryKey\(value\);/); // 不记动画中间帧。
});

test("隐藏页无帧：解除遮蔽与揭幕等待都能被计时器推进收口", async () => {
    // islandFrameOrTimeout：只推进计时器、不派发帧也必须执行一次。
    let 帧回调;
    let 定时回调;
    let 已取消帧 = 0;
    let 已清定时 = 0;
    let 执行次数 = 0;
    const 取消 = islandFrameOrTimeout({
        act: () => { 执行次数 += 1; },
        requestFrame: (callback) => { 帧回调 = callback; return 11; },
        cancelFrame: () => { 已取消帧 += 1; },
        setTimer: (callback) => { 定时回调 = callback; return 22; },
        clearTimer: () => { 已清定时 += 1; },
        timeoutMs: ISLAND_NO_FRAME_TIMEOUT_MS,
    });
    定时回调(); // 只推进计时器，帧永不派发。
    assert.equal(执行次数, 1);
    assert.equal(已清定时, 1); // 定时器执行后清掉自己。
    assert.equal(已取消帧, 1); // 帧也一并取消。
    帧回调();
    定时回调();
    assert.equal(执行次数, 1); // 只执行一次。
    取消(); // 取消后不再触发。
    assert.equal(执行次数, 1);
    // 帧先到：取消定时器，不再等超时。
    let 第二轮定时 = 0;
    let 第二轮执行 = 0;
    islandFrameOrTimeout({
        act: () => { 第二轮执行 += 1; },
        requestFrame: (callback) => { callback(); return 33; },
        cancelFrame: () => {},
        setTimer: () => { 第二轮定时 += 1; return 44; },
        clearTimer: () => {},
        timeoutMs: ISLAND_NO_FRAME_TIMEOUT_MS,
    });
    assert.equal(第二轮执行, 1);
    assert.equal(第二轮定时, 0); // 帧先到就不再排计时器。

    // islandWakingFrame + timeout：没有帧、没有外部 wake，计时器也要让 done 收口。
    let 等待定时;
    let 收口 = false;
    const 等待 = islandWakingFrame(
        () => { /* 帧永不派发 */ },
        { timeoutMs: ISLAND_NO_FRAME_TIMEOUT_MS, setTimer: (callback) => { 等待定时 = callback; return 55; }, clearTimer: () => {} },
    );
    void 等待.done.then(() => { 收口 = true; });
    await Promise.resolve();
    assert.equal(收口, false);
    等待定时();
    await 等待.done;
    assert.equal(收口, true);

    // 源码：settleMaskRelease 的解除与动画的揭幕/帧等待都走计时器兜底。
    const source = 读源码("./island.tsx");
    const 结算起 = source.indexOf("const settleMaskRelease = ");
    const 解除 = source.slice(结算起, source.indexOf("const observer = new ResizeObserver", 结算起));
    assert.ok(解除.length > 0, "解除遮蔽必须用帧/超时竞速");
    assert.match(读提交门(), /if \(keepOwner\) unmask\(token, true\);/);
    assert.match(读提交门(), /timeoutMs: ISLAND_NO_FRAME_TIMEOUT_MS/);
    assert.match(source, /\{ timeoutMs: ISLAND_NO_FRAME_TIMEOUT_MS, setTimer: \(callback\) => setTimeout\(callback, ISLAND_NO_FRAME_TIMEOUT_MS\)/); // 揭幕等待
    assert.match(source, /const 无帧计时 = setTimeout\(\(\) => \{/); // 动画帧等待兜底
    assert.match(source, /if \(!\(await commit\(value\)\)\) return; \/\/ 中间帧重试耗尽/); // 中间帧失败直接进 finally
    assert.doesNotMatch(source, /if \(!\(await commit\(value\)\)\) break;/);
});

test("中间帧失败不再提交最终目标：用可拒绝的 setIslandState 桩统计真实调用次数", async () => {
    // 真正会拒绝的提交桩（等价 setIslandState）：首次 + 两次有界重试，全部由桩自己计数。
    let 调用 = 0;
    const 拒绝提交 = async () => {
        调用 += 1;
        throw new Error("native reject");
    };
    // 模拟 commit(value)：首发一次，失败后走 islandRetrySubmit 的两次重试。
    const 提交 = async () => {
        try {
            await 拒绝提交();
            return true;
        } catch {
            return islandRetrySubmit({ submit: 拒绝提交, wait: async () => {}, current: () => true });
        }
    };
    assert.equal(await 提交(), false);
    assert.equal(调用, 3); // 1 次首发 + 2 次重试，由桩实测，不手写 ipc。
    // 失败门登记外层最终目标后，同一目标不再重发；中间帧 key 拦不住目标（旧缺陷）。
    const 目标key = islandGeometryKey(islandGeometry("expanded", [{ left: 0, top: 0, width: 320, height: 900 }], 12, { left: 0, top: 0, width: 1920, height: 1080 }, "top", 0));
    assert.equal(islandFailedGeometryGate({ failedKey: 目标key, key: 目标key, resubmit: false }), true);
    const 调用前 = 调用;
    if (!islandFailedGeometryGate({ failedKey: 目标key, key: 目标key, resubmit: false })) await 提交();
    assert.equal(调用, 调用前); // 被门拦住：一次 IPC 都没发。
});

test("带遮蔽的视口切换：隐藏帧先提交一帧，再发首次原生提交", () => {
    const source = 读源码("./island.tsx");
    const 提交起 = source.indexOf("const commit = async (value: IslandStateRequest)");
    const 提交 = source.slice(提交起, source.indexOf("currentGeometry.current = value; // 只在持有当前显示租约时写共享几何。", 提交起));
    assert.ok(提交.length > 0);
    // 顺序：租约校验 → （仅遮蔽 + 换视口时）等隐藏帧提交 → 发原生提交。
    assert.match(提交, /if \(!隐藏帧已等\) \{/);
    assert.match(提交, /if \(from && hadPaintedFrame\.current && gate\.state\(\)\.masked && islandViewportChanged\(from, request\) && gate\.state\(\)\.needsPaint\) \{/); // 已有画面才等（from 为空时不等，见代码注释）。
    assert.match(提交, /隐藏帧门 \?\?= islandRenderGate\(/); // 共享隐藏帧门（一帧 + 帧后宏任务 + 总超时）。
    assert.match(提交, /count: 1, \/\/ 一帧 \+ 帧后宏任务/);
    assert.match(提交, /收口 = await 门\.done;/);
    assert.match(提交, /islandTrace\("mask-frame", \{ settled: 收口 \}\);/); // 区分 timeout 与真实 frame 收口。
    assert.ok(提交.indexOf("await 隐藏帧.done") < 提交.indexOf("await setIslandState(value)"), "等帧必须在首次原生提交之前");
    // 取消时唤醒：cleanup 先 wakeMaskFrame 再取消帧。
    assert.match(source, /wakeMaskFrame\?\.\(\); \/\/ 先唤醒等待方/);
    assert.match(source, /wakeMaskFrame = 门\.wake;/);
    // 「已有画面」由绘制标记维护，换边清几何后仍为 true。
    assert.match(source, /const hadPaintedFrame = useRef\(false\);/);
    assert.match(source, /hadPaintedFrame\.current = true;/);
    assert.ok(提交.indexOf("if (!gate.owns(maskToken)) { settleMaskRelease") < 提交.indexOf("if (!隐藏帧已等)"), "先校验租约再等帧");
    // 首次提交才等：同一任务后续提交不重复等。
    assert.match(提交, /隐藏帧已等 = true;/);
    // 同视口/无遮蔽不等待：条件必须同时包含 transitionMasked 与 islandViewportChanged。
    assert.doesNotMatch(提交, /if \(islandViewportChanged\(from, request\)\) \{\s*const 隐藏帧/);
    // cleanup 取消隐藏帧等待句柄（等待本身另有 300ms 超时兜底）。
    assert.match(source, /cancelAnimationFrame\(maskFrame\);/);
});

test("收起不淡出内容：只有展开相位 2 的其他卡做淡入，终帧清空内联值", () => {
    const source = 读源码("./island.tsx");
    const 绘制 = source.slice(source.indexOf("// 内容只在展开相位 2 的其他卡上淡入"), source.indexOf("paintAttached(attached, value, elapsed < ISLAND_MOTION_MS);"));
    assert.ok(绘制.length > 0);
    assert.match(绘制, /const 相位淡入 = staggerMs > 0 && index > 0;/); // 只有相位 2 的其他卡淡入。
    assert.match(绘制, /const reveal = 相位淡入 && localElapsed < ISLAND_MOTION_MS \?/); // 非相位卡 reveal 恒为 1（内容保持可见）。
    assert.match(绘制, /content\.style\.removeProperty\("opacity"\);/);
    assert.match(绘制, /content\.style\.removeProperty\("filter"\);/);
    assert.doesNotMatch(绘制, /const reveal = localElapsed >= ISLAND_MOTION_MS \? 1 :/); // 旧逻辑：任何切态都从 0 淡入。
    assert.match(绘制, /if \(staggerMs > 0 && index > 0\) card\.style\.opacity = `\$\{reveal\}`;/); // 卡片级淡出仍只给相位 2。
    // 取消/清理路径按属性名清掉内容上的 opacity/filter。
    const 列表 = source.slice(source.indexOf("const PAINTED_STYLE_PROPERTIES"), source.indexOf("] as const;"));
    assert.match(列表, /"opacity"/);
    assert.match(列表, /"filter"/);
});

test("隐藏帧等两次渲染机会：帧收口/超时收口/取消唤醒三种收口原因", async () => {
    const 造门 = (count) => {
        const 帧回调 = [];
        let 定时回调;
        let 取消帧 = 0;
        let 清定时 = 0;
        const 门 = islandRenderGate({
            count,
            timeoutMs: ISLAND_NO_FRAME_TIMEOUT_MS,
            requestFrame: (callback) => {
                帧回调.push(callback);
                return 帧回调.length;
            },
            cancelFrame: () => { 取消帧 += 1; },
            setTimer: (callback) => { 定时回调 = callback; return 77; },
            clearTimer: () => { 清定时 += 1; },
        });
        return { 门, 帧回调, 推帧: () => 帧回调.shift()?.(), 推定时: () => 定时回调?.(), 取取消帧: () => 取消帧, 取清定时: () => 清定时 };
    };

    // 两次渲染机会才收口为 frame。
    const 甲 = 造门(2);
    甲.推帧();
    甲.推帧();
    assert.equal(await 甲.门.done, "frame");
    assert.equal(甲.取取消帧(), 1); // 最后一次帧句柄也被取消。
    assert.equal(甲.取清定时(), 1);

    // 没有帧只有计时器：timeout 收口。
    const 乙 = 造门(2);
    乙.推定时();
    assert.equal(await 乙.门.done, "timeout");

    // 取消唤醒：立刻 wake 收口，不等超时。
    const 丙 = 造门(2);
    丙.门.wake();
    assert.equal(await 丙.门.done, "wake");
    assert.equal(丙.取清定时(), 1);

    // 一次帧不够：count=2 时仍是待定。
    const 丁 = 造门(2);
    let 已收口 = false;
    void 丁.门.done.then(() => { 已收口 = true; });
    丁.推帧();
    await Promise.resolve();
    assert.equal(已收口, false);
    丁.推帧();
    assert.equal(await 丁.门.done, "frame");

    // 源码：commit 用两次渲染机会，cleanup 取消时先唤醒。
    const source = 读源码("./island.tsx");
    const 提交 = source.slice(source.indexOf("const commit = async (value: IslandStateRequest)"), source.indexOf("currentGeometry.current = value;"));
    assert.match(提交, /count: 1, \/\/ 一帧 \+ 帧后宏任务/);
    assert.match(提交, /timeoutMs: ISLAND_NO_FRAME_TIMEOUT_MS, \/\/ 两次机会共享同一个总超时。/);
    assert.match(source, /if \(收口 === "frame"\) await new Promise<void>/, "帧后要有宏任务：证明这一帧确实画完");
    assert.match(source, /wakeMaskFrame\?\.\(\); \/\/ 先唤醒等待方，避免取消后空等 300ms 超时。/);
});

test("同边同 key 接管：未首发的预留随 cleanup 撤销，继任 effect 必须发出收起提交", () => {
    const 跑 = (撤销: boolean) => {
        // 与 island.tsx 同形的共享状态：几何键 + 预留 + 原生提交计数。
        const 几何键 = { current: "" };
        const 预留 = { current: undefined };
        let 提交次数 = 0;
        // 旧任务：collapsed 目标同 key，写键并预留，随后在等隐藏帧（尚未首发）。
        几何键.current = "collapsed@同边";
        预留.current = { key: "collapsed@同边", token: 7, submitted: false, submittedFinal: false };
        if (撤销) {
            const 撤销键 = revokeIslandReservation(预留.current, 7, 几何键.current);
            if (撤销键 !== undefined) { 几何键.current = ""; 预留.current = undefined; }
        }
        // cleanup 后旧任务 inactive；继任 effect 按同 key 去重判断。
        const 去重 = 几何键.current === "collapsed@同边";
        if (!去重) {
            几何键.current = "collapsed@同边";
            预留.current = { key: "collapsed@同边", token: 8, submitted: true, submittedFinal: true };
            提交次数 += 1; // 继任 effect 真的发出收起 IPC。
        }
        return { 提交次数, 键: 几何键.current, 预留: 预留.current };
    };
    assert.equal(跑(false).提交次数, 0); // 旧行为：cleanup 不撤销 → 继任去重 → 两边都没发。
    const 修复后 = 跑(true);
    assert.equal(修复后.提交次数, 1); // 修复后：继任 effect 发出一次收起提交。
    assert.equal(修复后.预留?.token, 8); // 预留归属已换成继任者。

    // 首发后 cleanup 不撤销：键保留，继任去重不会重复提交。
    assert.equal(revokeIslandReservation({ key: "k", token: 7, submitted: true, submittedFinal: true }, 7, "k"), undefined);
    // token 不匹配（已被接管）不撤销。
    assert.equal(revokeIslandReservation({ key: "k", token: 7, submitted: false, submittedFinal: false }, 8, "k"), undefined);
    // 键已被改写也不撤销。
    assert.equal(revokeIslandReservation({ key: "k", token: 7, submitted: false, submittedFinal: false }, 7, "k2"), undefined);
    // 正常撤销返回键本身。
    assert.equal(revokeIslandReservation({ key: "k", token: 7, submitted: false, submittedFinal: false }, 7, "k"), "k");

    // 源码：cleanup 用身份撤销；maskPainted 在 owner/active 校验之后；submitted 在首发前标记。
    const source = 读源码("./island.tsx");
    assert.match(source, /const 撤销键 = revokeIslandReservation\(pendingSubmit\.current, gate\.state\(\)\.owner \?\? -1, geometryKey\.current\);/);
    assert.match(source, /pendingSubmit\.current = \{ key, token: maskToken, submitted: false, submittedFinal: false \};/);
    assert.match(source, /if \(value === request\) pendingSubmit\.current\.submittedFinal = true;/);
    const 等帧块 = source.slice(source.indexOf('if (!隐藏帧已等) {'), source.indexOf("let response = false;", source.indexOf('if (!隐藏帧已等) {')));
    const 校验位 = 等帧块.indexOf("if (!active || !gate.owns(maskToken))");
    const 绘制位 = 等帧块.indexOf("gate.markPainted();");
    assert.ok(校验位 > 0 && 绘制位 > 校验位, "maskPainted 必须记录在 owner/active 校验通过之后");
    assert.ok(等帧块.indexOf("wakeMaskFrame = undefined;") < 绘制位, "wake 路径不得提前记已绘制");
});

test("过渡帧已发、最终请求未发：cleanup 撤销后继任 effect 恰好补发一次最终收起", () => {
    // 可控 setIslandState 桩（记录每次提交的值）+ 可替换 effect 的时序模型。
    const 跑 = (场景: "等帧中" | "union已发" | "最终已发") => {
        const 键 = { current: "" };
        const 预留 = { current: undefined as { key: string; token: number; submitted: boolean; submittedFinal: boolean } | undefined };
        const 提交记录: string[] = [];
        const 提交 = (值: string) => { 提交记录.push(值); }; // setIslandState 桩：真实请求序列。
        // 旧任务：等帧期间还没有预留；等帧结束后才登记键，union 首帧之后才轮到最终目标。
        if (场景 !== "等帧中") {
            键.current = "collapsed@同边";
            预留.current = { key: "collapsed@同边", token: 7, submitted: false, submittedFinal: false };
            提交("union"); // 并集过渡帧。
            预留.current.submitted = true;
            if (场景 === "最终已发") {
                提交("collapsed"); // 最终目标（胶囊 windowBox）。
                预留.current.submittedFinal = true;
            }
        }
        // 旧 effect 被替换：cleanup 按身份撤销未收口的预留。
        const 撤销键 = revokeIslandReservation(预留.current, 7, 键.current);
        if (撤销键 !== undefined) { 键.current = ""; 预留.current = undefined; }
        // 继任 effect：同 key 去重判断，未去重则补发最终收起。
        const 去重 = 键.current === "collapsed@同边";
        if (!去重) {
            键.current = "collapsed@同边";
            预留.current = { key: "collapsed@同边", token: 8, submitted: true, submittedFinal: true };
            提交("collapsed");
        }
        return 提交记录;
    };
    // 等帧期间被替换：没有键可去重 → 继任发一次收起。
    assert.deepEqual(跑("等帧中"), ["collapsed"]);
    // union 已发但最终目标没发：继任补一次最终收起，整体恰好一次 collapsed。
    const union已发 = 跑("union已发");
    assert.deepEqual(union已发, ["union", "collapsed"]);
    assert.equal(union已发.filter((项) => 项 === "collapsed").length, 1);
    // 最终目标已发：保留 key，继任去重，不重复。
    const 最终已发 = 跑("最终已发");
    assert.deepEqual(最终已发, ["union", "collapsed"]);
    assert.equal(最终已发.filter((项) => 项 === "collapsed").length, 1);

    // 源码：键与预留在隐藏帧等待之后登记；submittedFinal 只在最终 request 发出时置真。
    const source = 读源码("./island.tsx");
    const 等帧位 = source.indexOf("if (!隐藏帧已等) {");
    const 登记位 = source.indexOf("if (!pendingSubmit.current || pendingSubmit.current.token !== maskToken) {");
    const 首发位 = source.indexOf("gate.nativeStarted();", 登记位);
    assert.ok(等帧位 > 0 && 登记位 > 等帧位 && 首发位 > 登记位, "预留必须落在等帧之后、首发之前");
    assert.match(source, /if \(value === request\) pendingSubmit\.current\.submittedFinal = true;/);
    assert.match(source, /const 撤销键 = revokeIslandReservation\(pendingSubmit\.current, gate\.state\(\)\.owner \?\? -1, geometryKey\.current\);/);
});
