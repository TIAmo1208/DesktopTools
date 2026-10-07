import assert from "node:assert/strict";
import test from "node:test";

import { moveTile, resizeTile, resolveTileMinimum, snapTileEdges } from "./tileGeometry.ts";

// 功能格子沿用八方向算法，同时拥有内容所需的最小尺寸。
test("component resize and snap respect their own minimum bounds", () => {
    const start = { left: 100, top: 100, width: 220, height: 116 }; // 既有时钟边界。
    const viewport = { width: 1280, height: 720 }; // 当前逻辑工作区。
    const minimum = { width: 164, height: 90 }; // 功能格子的最小可读尺寸。
    for (const direction of ["n", "ne", "e", "se", "s", "sw", "w", "nw"] as const) {
        const result = resizeTile(direction, start, { x: -1000, y: -1000 }, viewport, minimum); // 极端指针位移。
        assert.ok(result.width >= minimum.width && result.height >= minimum.height);
        assert.ok(result.left >= 0 && result.top >= 0);
    }
    assert.equal(resizeTile("se", start, { x: -500, y: -500 }, viewport, minimum).height, 90);
    assert.equal(snapTileEdges({ ...start, height: 93 }, [{ ...start, top: 190 }], viewport, 8, { bottom: true }, minimum).height, 90);
});

// 验证围栏移动保持完整边界位于当前工作区。
test("moves a fence within workspace bounds", () => {
    assert.deepEqual(moveTile({ left: 100, top: 80, width: 320, height: 240 }, { x: -180, y: 900 }, { width: 1280, height: 720 }), {
        left: 0,
        top: 480,
        width: 320,
        height: 240,
    });
});

// 验证原型八方向缩放规则保持最小尺寸与工作区边界。
test("resizes a fence from its north-west edge", () => {
    assert.deepEqual(resizeTile("nw", { left: 100, top: 80, width: 320, height: 240 }, { x: -180, y: -120 }, { width: 1280, height: 720 }), {
        left: 0,
        top: 0,
        width: 420,
        height: 320,
    });
});

// 未覆盖时使用公共底线，不依赖围栏内容大小。
test("clamps a tile to the shared minimum contract size", () => {
    assert.deepEqual(resizeTile("se", { left: 100, top: 80, width: 320, height: 240 }, { x: -1000, y: -1000 }, { width: 1280, height: 720 }), {
        left: 100,
        top: 80,
        width: 80,
        height: 48,
    });
});

// 有效覆盖不得降低持久底线，无效数字不能流入几何运算。
test("validates minimum overrides at the shared geometry boundary", () => {
    assert.deepEqual(resolveTileMinimum(), { width: 80, height: 48 });
    assert.deepEqual(resolveTileMinimum({ width: 1, height: 1 }), { width: 80, height: 48 });
    for (const size of [0, -1, NaN, Infinity]) {
        assert.throws(() => resolveTileMinimum({ width: size, height: 110 }));
        assert.throws(() => resolveTileMinimum({ width: 164, height: size }));
    }
    assert.deepEqual(resolveTileMinimum({ width: 168, height: 142 }), { width: 168, height: 142 });
});

// 验证围栏移动会吸附到工作区和相邻围栏边缘。
test("snaps a moved fence to nearby edges", () => {
    assert.deepEqual(
        snapTileEdges({ left: 103, top: 202, width: 164, height: 154 }, [{ left: 100, top: 200, width: 200, height: 154 }], { width: 1280, height: 720 }, 8),
        { left: 100, top: 200, width: 164, height: 154 },
    );
});

// 验证围栏缩放会将受控边缘吸附到相邻围栏边缘。
test("snaps a resized fence edge without moving the fixed edge", () => {
    assert.deepEqual(
        snapTileEdges({ left: 100, top: 80, width: 197, height: 154 }, [{ left: 300, top: 80, width: 200, height: 154 }], { width: 1280, height: 720 }, 8, {
            right: true,
        }),
        { left: 100, top: 80, width: 200, height: 154 },
    );
});
