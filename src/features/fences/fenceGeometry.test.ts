import assert from "node:assert/strict";
import test from "node:test";

import { moveFence, resizeFence } from "./fenceGeometry.ts";

// 验证围栏移动保持完整边界位于当前工作区。
test("moves a fence within workspace bounds", () => {
    assert.deepEqual(
        moveFence(
            { left: 100, top: 80, width: 320, height: 240 },
            { x: -180, y: 900 },
            { width: 1280, height: 720 },
        ),
        { left: 0, top: 480, width: 320, height: 240 },
    );
});

// 验证原型八方向缩放规则保持最小尺寸与工作区边界。
test("resizes a fence from its north-west edge", () => {
    assert.deepEqual(
        resizeFence(
            "nw",
            { left: 100, top: 80, width: 320, height: 240 },
            { x: -180, y: -120 },
            { width: 1280, height: 720 },
        ),
        { left: 0, top: 0, width: 420, height: 320 },
    );
});

// 验证围栏缩放收敛到公开契约定义的最小尺寸。
test("clamps a fence to the minimum contract size", () => {
    assert.deepEqual(
        resizeFence("se", { left: 100, top: 80, width: 320, height: 240 }, { x: -1000, y: -1000 }, { width: 1280, height: 720 }),
        { left: 100, top: 80, width: 164, height: 154 },
    );
});
