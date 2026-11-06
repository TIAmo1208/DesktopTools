import assert from "node:assert/strict";
import test from "node:test";
import { islandMotionFrame, islandMotionProgress, ISLAND_MOTION_MS, ISLAND_FRAME_MS } from "./islandMotion.ts";
import { islandGeometry } from "./islandGeometry.ts";
import { parseIslandStateRequest } from "../../contracts/island.ts";

const work = { left: 0, top: 40, width: 1000, height: 800 };
const collapsed = [{ left: 0, top: 0, width: 160, height: 44 }];
const expanded = [
    { left: 0, top: 0, width: 360, height: 233 },
    { left: 0, top: 241, width: 360, height: 120 },
];
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
                if (edge === "bottom") assert.ok(Math.abs(frame.windowBox.top + frame.windowBox.height - (b.windowBox.top + b.windowBox.height)) < epsilon);
                else assert.equal(frame.windowBox.top, b.windowBox.top);
                if (edge === "right") assert.ok(Math.abs(frame.windowBox.left + frame.windowBox.width - (b.windowBox.left + b.windowBox.width)) < epsilon);
                else if (edge === "")
                    assert.ok(Math.abs(frame.windowBox.left + frame.windowBox.width / 2 - (b.windowBox.left + b.windowBox.width / 2)) < epsilon);
                else assert.equal(frame.windowBox.left, b.windowBox.left);
            }
            assert.deepEqual(islandMotionFrame(a, b, ISLAND_MOTION_MS, edge, work), b);
        }
    }
});
test("接近工作区边界时回弹不超工作区", () => {
    const from = islandGeometry("hover", [{ left: 0, top: 0, width: 900, height: 72 }], 16, work, "right", 0);
    const to = islandGeometry("expanded", [{ left: 0, top: 0, width: 1000, height: 800 }], 12, work, "right", 0);
    for (let elapsed = 0; elapsed < 240; elapsed++) {
        const frame = islandMotionFrame(from, to, elapsed, "right", work);
        assert.ok(frame.windowBox.width <= work.width);
        assert.ok(frame.windowBox.height <= work.height);
        assert.deepEqual(parseIslandStateRequest(frame), frame);
    }
});
