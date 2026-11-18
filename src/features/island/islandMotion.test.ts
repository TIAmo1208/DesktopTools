import assert from "node:assert/strict";
import test from "node:test";
import {
    islandHoverTravel,
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
import { islandFilletPatches, islandGeometry } from "./islandGeometry.ts";
import { parseIslandStateRequest, type IslandStateRequest } from "../../contracts/island.ts";

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
                    assert.deepEqual(from.viewport, to.viewport, "同一偏移下两态必须共用视口");
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
    assert.equal(start.cards[0]!.rect.left, from.cards[0]!.rect.left + (from.windowBox.left - start.windowBox.left));
    const single: IslandStateRequest = { ...to, cards: [to.cards[0]!] };
    assert.deepEqual(islandUnionFrame(from, single, 120, "", work, ISLAND_EXPAND_STAGGER_MS), islandUnionFrame(from, single, 120, "", work)); // 单卡不启用两段。
});
